use std::{
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use serde::Serialize;

use crate::{
    Args,
    db_probe::{self, DbProbeDelta, DbProbeSnapshot},
    process_metrics::{ProcessSnapshot, capture_snapshot},
    progress::{ProgressCounters, ProgressSnapshot},
    summary::{LatencySummary, latency_summary},
};

pub(crate) struct TraceReporter {
    stop_sender: mpsc::Sender<()>,
    handle: JoinHandle<()>,
}

#[derive(Debug, Serialize)]
struct TraceSample {
    event: &'static str,
    phase: &'static str,
    sequence: u64,
    run_id: String,
    backend: crate::Backend,
    scenario: crate::Scenario,
    child_index: Option<usize>,
    target: String,
    elapsed_ms: u128,
    attempted: u64,
    succeeded: u64,
    failed: u64,
    recent_attempted: u64,
    recent_failed: u64,
    recent_ops_sec: f64,
    throughput_ops_sec: f64,
    interval_latency: IntervalLatencySummary,
    process: ProcessSnapshot,
    #[serde(skip_serializing_if = "Option::is_none")]
    db_probe: Option<DbProbeSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    db_delta_from_start: Option<DbProbeDelta>,
}

#[derive(Debug, Serialize)]
struct IntervalLatencySummary {
    count: u64,
    latency: LatencySummary,
}

pub(crate) fn spawn_trace_reporter(
    args: &Args,
    target: &str,
    progress: Arc<ProgressCounters>,
) -> Option<TraceReporter> {
    let path = args.trace_jsonl.clone()?;
    if args.trace_interval_seconds == 0 || args.warmup_phase {
        return None;
    }

    let runtime = tokio::runtime::Handle::try_current().ok();
    let args = args.clone();
    let target = target.to_string();
    let (stop_sender, stop_receiver) = mpsc::channel();
    let handle = thread::Builder::new()
        .name("ironclaw-stress-trace".to_string())
        .spawn(move || {
            let run_id = args
                .run_id
                .clone()
                .unwrap_or_else(|| "unknown-run".to_string());
            let interval = Duration::from_secs(args.trace_interval_seconds);
            let started = Instant::now();
            let mut last_snapshot = ProgressSnapshot::default();
            let mut last_report = Instant::now();
            let mut sequence = 0;
            let db_start = capture_db_snapshot(&args, runtime.as_ref());

            loop {
                match stop_receiver.recv_timeout(interval) {
                    Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                        emit_trace_sample(
                            &path,
                            &args,
                            &run_id,
                            &target,
                            &progress,
                            runtime.as_ref(),
                            &db_start,
                            started,
                            &mut last_snapshot,
                            &mut last_report,
                            &mut sequence,
                            "final",
                        );
                        break;
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        emit_trace_sample(
                            &path,
                            &args,
                            &run_id,
                            &target,
                            &progress,
                            runtime.as_ref(),
                            &db_start,
                            started,
                            &mut last_snapshot,
                            &mut last_report,
                            &mut sequence,
                            "interval",
                        );
                    }
                }
            }
        })
        .ok()?;

    Some(TraceReporter {
        stop_sender,
        handle,
    })
}

#[allow(clippy::too_many_arguments)]
fn emit_trace_sample(
    path: &Path,
    args: &Args,
    run_id: &str,
    target: &str,
    progress: &ProgressCounters,
    runtime: Option<&tokio::runtime::Handle>,
    db_start: &Option<DbProbeSnapshot>,
    started: Instant,
    last_snapshot: &mut ProgressSnapshot,
    last_report: &mut Instant,
    sequence: &mut u64,
    phase: &'static str,
) {
    let snapshot = progress.snapshot();
    let recent_attempted = snapshot.attempted.saturating_sub(last_snapshot.attempted);
    let recent_failed = snapshot.failed.saturating_sub(last_snapshot.failed);
    let recent_elapsed = last_report.elapsed().as_secs_f64().max(f64::MIN_POSITIVE);
    let elapsed = started.elapsed();
    let elapsed_secs = elapsed.as_secs_f64().max(f64::MIN_POSITIVE);
    let mut interval_latencies = progress.drain_interval_latencies_us();
    interval_latencies.sort_unstable();
    let db_probe = capture_db_snapshot(args, runtime);
    let db_delta_from_start = match (db_start, &db_probe) {
        (Some(start), Some(current)) => {
            Some(db_probe::summarize(start.clone(), current.clone()).delta)
        }
        _ => None,
    };
    let sample = TraceSample {
        event: "trace_sample",
        phase,
        sequence: *sequence,
        run_id: run_id.to_string(),
        backend: args.backend,
        scenario: args.scenario,
        child_index: args.child_index,
        target: target.to_string(),
        elapsed_ms: elapsed.as_millis(),
        attempted: snapshot.attempted,
        succeeded: snapshot.attempted.saturating_sub(snapshot.failed),
        failed: snapshot.failed,
        recent_attempted,
        recent_failed,
        recent_ops_sec: recent_attempted as f64 / recent_elapsed,
        throughput_ops_sec: snapshot.attempted as f64 / elapsed_secs,
        interval_latency: IntervalLatencySummary {
            count: interval_latencies.len() as u64,
            latency: latency_summary(&interval_latencies),
        },
        process: capture_snapshot(),
        db_probe,
        db_delta_from_start,
    };
    if let Err(error) = append_json_line(path, &sample) {
        eprintln!(
            "{} failed to write trace sample to {}: {error}",
            crate::log_prefix(args),
            path.display()
        );
    }
    *last_snapshot = snapshot;
    *last_report = Instant::now();
    *sequence = (*sequence).saturating_add(1);
}

pub(crate) fn stop_trace_reporter(trace_reporter: Option<TraceReporter>) {
    if let Some(trace_reporter) = trace_reporter {
        let _ = trace_reporter.stop_sender.send(());
        if let Err(payload) = trace_reporter.handle.join() {
            eprintln!(
                "trace reporter panicked: {}",
                panic_payload_to_string(&payload)
            );
        }
    }
}

pub(crate) async fn prepare_trace_outputs(args: &Args) -> Result<(), String> {
    let Some(path) = &args.trace_jsonl else {
        return Ok(());
    };
    if args.child_index.is_some() {
        return Ok(());
    }
    remove_trace_file(path).await?;
    if args.processes > 1 {
        for child_index in 0..args.processes {
            remove_trace_file(&child_trace_path(path, child_index)).await?;
        }
    }
    Ok(())
}

pub(crate) fn child_trace_path(path: &Path, child_index: usize) -> PathBuf {
    let mut child_path = path.to_path_buf();
    let file_name = path
        .file_name()
        .map(|file_name| file_name.to_string_lossy().to_string())
        .unwrap_or_else(|| "trace.jsonl".to_string());
    child_path.set_file_name(format!("{file_name}.child-{child_index}.jsonl"));
    child_path
}

fn capture_db_snapshot(
    args: &Args,
    runtime: Option<&tokio::runtime::Handle>,
) -> Option<DbProbeSnapshot> {
    if args.scenario.is_process_local() {
        return None;
    }
    match runtime {
        Some(handle) => Some(handle.block_on(db_probe::capture(args))),
        None => Some(DbProbeSnapshot {
            error: Some("trace db probe failed: no Tokio runtime handle".to_string()),
            ..DbProbeSnapshot::default()
        }),
    }
}

fn append_json_line(path: &Path, sample: &TraceSample) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| format!("create trace dir: {error}"))?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| format!("open trace file: {error}"))?;
    let encoded = serde_json::to_string(sample).map_err(|error| error.to_string())?;
    writeln!(file, "{encoded}").map_err(|error| format!("write trace file: {error}"))
}

async fn remove_trace_file(path: &Path) -> Result<(), String> {
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "remove stale trace file {}: {error}",
            path.display()
        )),
    }
}

fn panic_payload_to_string(payload: &Box<dyn std::any::Any + Send + 'static>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        return (*message).to_string();
    }
    if let Some(message) = payload.downcast_ref::<String>() {
        return message.clone();
    }
    "non-string panic payload".to_string()
}
