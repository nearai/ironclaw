mod capture;
mod child_io;
mod human;
mod process_metrics;
mod process_pressure;
mod progress;
mod redaction;
mod report;
mod resource_ops;
mod summary;
mod sweep;
mod synthetic;
#[cfg(test)]
mod tests;
mod user_turn;

use std::{
    any::Any,
    collections::BTreeMap,
    env::{self, VarError},
    io::ErrorKind,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, mpsc},
    thread,
    thread::JoinHandle,
    time::{Duration, Instant},
};

use crate::{
    capture::CapturedRun,
    child_io::{join_child_stderr_reader, spawn_child_stderr_reader},
    process_metrics::{ProcessMetrics, ProcessMetricsSampler},
    progress::{ProgressCounters, spawn_progress_reporter, stop_progress_reporter},
    redaction::{redact_libsql_path, redact_postgres_url},
    summary::{
        FailureCause, FailureCauseSummary, LatencySummary, latency_summary,
        summarize_failure_causes, summarize_user_turn_stages,
    },
    synthetic::SyntheticIds,
    user_turn::{
        UserTurnStageDurations, UserTurnStageLatencySummary, build_user_turn_workload,
        run_user_turn_tasks,
    },
};
use clap::{Parser, ValueEnum};
use ironclaw_filesystem::{RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{
    MountAlias, MountGrant, MountPermissions, MountView, TenantId, VirtualPath,
};
use ironclaw_resources::{
    FilesystemResourceGovernorStore, PersistentResourceGovernor, ResourceAccount, ResourceGovernor,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Parser)]
#[command(
    name = "ironclaw_stress",
    about = "Stress IronClaw infrastructure workloads"
)]
pub(crate) struct Args {
    #[arg(long, value_enum)]
    pub(crate) backend: Backend,

    /// OS processes to run against the same snapshot path. Use >1 to exercise
    /// cross-process CAS contention that the in-process lock cannot serialize.
    #[arg(long, default_value_t = 1)]
    pub(crate) processes: usize,

    /// Threads per process.
    #[arg(long, default_value_t = 8)]
    pub(crate) concurrency: usize,

    /// Operations per thread.
    #[arg(long, default_value_t = 200)]
    pub(crate) operations: usize,

    /// Synthetic users distributed across operations.
    #[arg(long, default_value_t = 50)]
    pub(crate) users: usize,

    /// Synthetic tenants distributed across users.
    #[arg(long, default_value_t = 1)]
    pub(crate) tenants: usize,

    #[arg(long, value_enum, default_value_t = Scenario::ReserveRelease)]
    pub(crate) scenario: Scenario,

    /// Shared run id. Defaults to a fresh UUID.
    #[arg(long)]
    pub(crate) run_id: Option<String>,

    /// libSQL database path. Defaults to a temp-file path printed in output.
    #[arg(long)]
    pub(crate) libsql_path: Option<PathBuf>,

    /// Postgres URL. Defaults to IRONCLAW_FILESYSTEM_POSTGRES_URL, then DATABASE_URL.
    #[arg(long)]
    pub(crate) postgres_url: Option<String>,

    /// Postgres pool size per process.
    #[arg(long, default_value_t = 4)]
    pub(crate) postgres_pool_size: usize,

    /// Emit live progress to stderr every N seconds. Set to 0 to disable.
    #[arg(long, default_value_t = 1)]
    pub(crate) progress_interval_seconds: u64,

    /// Emit a human-readable summary table to stderr after the JSON summary.
    #[arg(long, default_value_t = false)]
    pub(crate) human_read: bool,

    /// Comma-separated concurrency values to sweep.
    #[arg(long, value_delimiter = ',')]
    pub(crate) sweep_concurrency: Vec<usize>,

    /// Comma-separated synthetic user counts to sweep.
    #[arg(long, value_delimiter = ',')]
    pub(crate) sweep_users: Vec<usize>,

    /// Comma-separated model latency values to sweep for mixed-user-session.
    #[arg(long, value_delimiter = ',')]
    pub(crate) sweep_model_latency_ms: Vec<u64>,

    /// Repetitions per sweep point.
    #[arg(long, default_value_t = 1)]
    pub(crate) repetitions: usize,

    /// Write one JSON object per sweep run to this file.
    #[arg(long)]
    pub(crate) output_jsonl: Option<PathBuf>,

    /// Fail when any run's failure rate is above this value, e.g. 0.01.
    #[arg(long)]
    pub(crate) max_failure_rate: Option<f64>,

    /// Fail when any run's p95 latency is above this many milliseconds.
    #[arg(long)]
    pub(crate) max_p95_ms: Option<u64>,

    /// Fail when any run's throughput is below this operations/sec value.
    #[arg(long)]
    pub(crate) min_throughput: Option<f64>,

    /// Fail when any run's peak RSS is above this many MiB.
    #[arg(long)]
    pub(crate) max_rss_mb: Option<u64>,

    /// Fail when any run's CPU time is above this many milliseconds.
    #[arg(long)]
    pub(crate) max_cpu_ms: Option<u128>,

    /// Synthetic model latency for mixed-user-session operations.
    #[arg(long, default_value_t = 0)]
    pub(crate) model_latency_ms: u64,

    /// Emit structured stderr spans for failed user-turn operations.
    #[arg(long, default_value_t = false)]
    pub(crate) span_log_failures: bool,

    /// Emit structured stderr spans for user-turn operations at or above this latency.
    /// Set to 0 to disable.
    #[arg(long, default_value_t = 0)]
    pub(crate) slow_span_threshold_ms: u64,

    /// Max structured spans to emit per process. Set to 0 for unlimited.
    #[arg(long, default_value_t = 100)]
    pub(crate) span_sample_limit: usize,

    /// CPU loop iterations per cpu-burn operation.
    #[arg(long, default_value_t = 250_000)]
    pub(crate) cpu_work_units: u64,

    /// Bytes allocated and touched per memory-churn operation.
    #[arg(long, default_value_t = 1_048_576)]
    pub(crate) memory_bytes: usize,

    /// Milliseconds to hold each memory allocation before dropping it.
    #[arg(long, default_value_t = 0)]
    pub(crate) memory_hold_ms: u64,

    #[arg(long, hide = true)]
    pub(crate) child_index: Option<usize>,
}

#[derive(Debug, Clone, Copy, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Backend {
    Libsql,
    Postgres,
}

impl Backend {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Libsql => "libsql",
            Self::Postgres => "postgres",
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Scenario {
    ReserveRelease,
    ReserveReconcile,
    ChatTurn,
    MixedUserSession,
    CpuBurn,
    MemoryChurn,
}

impl Scenario {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ReserveRelease => "reserve-release",
            Self::ReserveReconcile => "reserve-reconcile",
            Self::ChatTurn => "chat-turn",
            Self::MixedUserSession => "mixed-user-session",
            Self::CpuBurn => "cpu-burn",
            Self::MemoryChurn => "memory-churn",
        }
    }

    pub(crate) fn is_resource_governor(self) -> bool {
        matches!(self, Self::ReserveRelease | Self::ReserveReconcile)
    }

    pub(crate) fn is_user_turn(self) -> bool {
        matches!(self, Self::ChatTurn | Self::MixedUserSession)
    }

    pub(crate) fn is_process_local(self) -> bool {
        matches!(self, Self::CpuBurn | Self::MemoryChurn)
    }
}

struct BackendHandle {
    governor: Arc<dyn ResourceGovernor>,
    target: String,
}

#[derive(Debug, Clone)]
pub(crate) struct Sample {
    pub(crate) latency: Duration,
    pub(crate) error: Option<String>,
    pub(crate) failure: Option<FailureCause>,
    pub(crate) stages: Option<UserTurnStageDurations>,
}

#[derive(Debug, Serialize, Deserialize)]
struct RunSummary {
    backend: Backend,
    scenario: Scenario,
    run_id: String,
    target: String,
    child_index: Option<usize>,
    processes: usize,
    concurrency: usize,
    operations_per_thread: usize,
    users: usize,
    tenants: usize,
    attempted: u64,
    succeeded: u64,
    failed: u64,
    duration_ms: u128,
    throughput_ops_sec: f64,
    latency: LatencySummary,
    process: ProcessMetrics,
    #[serde(skip_serializing_if = "Option::is_none")]
    stage_latency: Option<UserTurnStageLatencySummary>,
    errors: BTreeMap<String, u64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    failure_causes: BTreeMap<String, FailureCauseSummary>,
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let mut args = Args::parse();
    validate_args(&args)?;

    let run_id = args
        .run_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string());
    args.run_id = Some(run_id.clone());
    let generated_libsql_path = if args.child_index.is_none()
        && matches!(args.backend, Backend::Libsql)
        && args.libsql_path.is_none()
    {
        let path = default_libsql_path();
        args.libsql_path = Some(path.clone());
        Some(path)
    } else {
        None
    };

    let result = if args.child_index.is_none() && sweep::is_enabled(&args) {
        sweep::run(&args, &run_id).await
    } else {
        run_once(&args, &run_id)
            .await
            .and_then(|captured| {
                report::print_captured_run(&args, &run_id, &captured).map(|_| captured)
            })
            .and_then(|captured| {
                let metrics = captured.metrics();
                sweep::enforce_thresholds(&args, &[("run".to_string(), metrics)])
            })
    };

    if let Some(path) = generated_libsql_path {
        cleanup_generated_libsql_path(&path).await;
    }

    result
}

pub(crate) async fn run_once(args: &Args, run_id: &str) -> Result<CapturedRun, String> {
    if args.child_index.is_none() && args.processes > 1 {
        prewarm(args, run_id)
            .await
            .and_then(|_| run_child_processes(args, run_id))
            .map(|summaries| CapturedRun::Parent {
                aggregate: report::parent_summary_value(args, run_id, &summaries),
                summaries,
            })
    } else {
        run_in_process(args, run_id)
            .await
            .map(|summary| CapturedRun::Single(Box::new(summary)))
    }
}

fn validate_args(args: &Args) -> Result<(), String> {
    if args.processes == 0 {
        return Err("--processes must be greater than 0".to_string());
    }
    if args.concurrency == 0 {
        return Err("--concurrency must be greater than 0".to_string());
    }
    if args.operations == 0 {
        return Err("--operations must be greater than 0".to_string());
    }
    if args.users == 0 {
        return Err("--users must be greater than 0".to_string());
    }
    if args.tenants == 0 {
        return Err("--tenants must be greater than 0".to_string());
    }
    if args.postgres_pool_size == 0 {
        return Err("--postgres-pool-size must be greater than 0".to_string());
    }
    if args.repetitions == 0 {
        return Err("--repetitions must be greater than 0".to_string());
    }
    if args.sweep_concurrency.contains(&0) {
        return Err("--sweep-concurrency values must be greater than 0".to_string());
    }
    if args.sweep_users.contains(&0) {
        return Err("--sweep-users values must be greater than 0".to_string());
    }
    if let Some(max_failure_rate) = args.max_failure_rate
        && !(0.0..=1.0).contains(&max_failure_rate)
    {
        return Err("--max-failure-rate must be between 0.0 and 1.0".to_string());
    }
    if let Some(min_throughput) = args.min_throughput
        && min_throughput < 0.0
    {
        return Err("--min-throughput must be greater than or equal to 0".to_string());
    }
    if matches!(args.max_rss_mb, Some(0)) {
        return Err("--max-rss-mb must be greater than 0".to_string());
    }
    if matches!(args.max_cpu_ms, Some(0)) {
        return Err("--max-cpu-ms must be greater than 0".to_string());
    }
    if args.cpu_work_units == 0 {
        return Err("--cpu-work-units must be greater than 0".to_string());
    }
    if args.memory_bytes == 0 {
        return Err("--memory-bytes must be greater than 0".to_string());
    }
    if args.scenario.is_user_turn() && args.processes > 1 {
        return Err(format!(
            "--scenario {} requires --processes 1",
            args.scenario.as_str()
        ));
    }
    Ok(())
}

async fn prewarm(args: &Args, run_id: &str) -> Result<(), String> {
    eprintln!(
        "{} prewarming backend={} scenario={} run_id={}",
        log_prefix(args),
        args.backend.as_str(),
        args.scenario.as_str(),
        run_id
    );
    if args.scenario.is_resource_governor() {
        let backend = build_backend(args, run_id).await?;
        let account =
            ResourceAccount::tenant(TenantId::new("stress-prewarm").map_err(display_err)?);
        backend
            .governor
            .account_snapshot(&account)
            .map_err(|error| format!("prewarm failed: {error:?}"))?;
    } else {
        let workload = build_user_turn_workload(args, run_id).await?;
        eprintln!(
            "{} prewarmed target={}",
            log_prefix(args),
            workload.target()
        );
    }
    Ok(())
}

fn run_child_processes(args: &Args, run_id: &str) -> Result<Vec<RunSummary>, String> {
    let current_exe =
        std::env::current_exe().map_err(|error| format!("resolve current executable: {error}"))?;
    let libsql_path = args.libsql_path.clone().unwrap_or_else(default_libsql_path);

    eprintln!(
        "{} spawning {} child processes total_operations_per_child={} progress_interval_seconds={}",
        log_prefix(args),
        args.processes,
        args.concurrency.saturating_mul(args.operations),
        args.progress_interval_seconds
    );

    let mut children = Vec::with_capacity(args.processes);
    for child_index in 0..args.processes {
        let mut command = Command::new(&current_exe);
        command
            .arg("--backend")
            .arg(args.backend.as_str())
            .arg("--processes")
            .arg("1")
            .arg("--concurrency")
            .arg(args.concurrency.to_string())
            .arg("--operations")
            .arg(args.operations.to_string())
            .arg("--users")
            .arg(args.users.to_string())
            .arg("--tenants")
            .arg(args.tenants.to_string())
            .arg("--scenario")
            .arg(args.scenario.as_str())
            .arg("--postgres-pool-size")
            .arg(args.postgres_pool_size.to_string())
            .arg("--progress-interval-seconds")
            .arg(args.progress_interval_seconds.to_string())
            .arg("--model-latency-ms")
            .arg(args.model_latency_ms.to_string())
            .arg("--slow-span-threshold-ms")
            .arg(args.slow_span_threshold_ms.to_string())
            .arg("--span-sample-limit")
            .arg(args.span_sample_limit.to_string())
            .arg("--run-id")
            .arg(run_id)
            .arg("--child-index")
            .arg(child_index.to_string())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if args.span_log_failures {
            command.arg("--span-log-failures");
        }
        if matches!(args.backend, Backend::Libsql) {
            command.arg("--libsql-path").arg(&libsql_path);
        }
        if let Some(url) = &args.postgres_url {
            command.env("IRONCLAW_FILESYSTEM_POSTGRES_URL", url);
        }
        match command.spawn() {
            Ok(mut child) => {
                let stderr_reader = child
                    .stderr
                    .take()
                    .and_then(|stderr| spawn_child_stderr_reader(child_index, stderr));
                children.push((child_index, child, stderr_reader));
            }
            Err(error) => {
                terminate_children(&mut children);
                return Err(format!("spawn child {child_index}: {error}"));
            }
        }
    }

    let mut summaries = Vec::with_capacity(children.len());
    while !children.is_empty() {
        let (child_index, child, stderr_reader) = children.remove(0);
        let output = match child.wait_with_output() {
            Ok(output) => output,
            Err(error) => {
                join_child_stderr_reader(child_index, stderr_reader);
                terminate_children(&mut children);
                return Err(format!("wait for child {child_index}: {error}"));
            }
        };
        join_child_stderr_reader(child_index, stderr_reader);
        if !output.status.success() {
            terminate_children(&mut children);
            return Err(format!(
                "child {child_index} failed with status {}; see stderr above for child logs",
                output.status
            ));
        }
        let stdout = match String::from_utf8(output.stdout) {
            Ok(stdout) => stdout,
            Err(error) => {
                terminate_children(&mut children);
                return Err(format!(
                    "child {child_index} emitted non-utf8 stdout: {error}"
                ));
            }
        };
        let summary: RunSummary = match serde_json::from_str(stdout.trim()) {
            Ok(summary) => summary,
            Err(error) => {
                terminate_children(&mut children);
                return Err(format!(
                    "parse child {child_index} summary: {error}: {stdout}"
                ));
            }
        };
        summaries.push(summary);
    }
    summaries.sort_by_key(|summary| summary.child_index.unwrap_or(usize::MAX));
    Ok(summaries)
}

async fn run_in_process(args: &Args, run_id: &str) -> Result<RunSummary, String> {
    eprintln!(
        "{} preparing backend={} scenario={} run_id={}",
        log_prefix(args),
        args.backend.as_str(),
        args.scenario.as_str(),
        run_id
    );
    let total_operations = args.concurrency.saturating_mul(args.operations);
    if args.scenario.is_process_local() {
        return run_process_pressure_in_process(args, run_id, total_operations).await;
    }
    let identities = Arc::new(SyntheticIds::new(args)?);

    if args.scenario.is_resource_governor() {
        return run_resource_governor_in_process(args, run_id, total_operations, identities).await;
    }

    run_user_turn_in_process(args, run_id, total_operations, identities).await
}

async fn run_process_pressure_in_process(
    args: &Args,
    run_id: &str,
    total_operations: usize,
) -> Result<RunSummary, String> {
    eprintln!(
        "{} running target=process://local concurrency={} operations_per_thread={} total_operations={} progress_interval_seconds={}",
        log_prefix(args),
        args.concurrency,
        args.operations,
        total_operations,
        args.progress_interval_seconds
    );
    let metrics = ProcessMetricsSampler::start(Duration::from_millis(100));
    let started = Instant::now();
    let args_clone = args.clone();
    let samples = tokio::task::spawn_blocking(move || process_pressure::run(&args_clone))
        .await
        .map_err(|error| {
            if error.is_panic() {
                eprintln!("process pressure task panicked: {error:?}");
                "process pressure task panicked".to_string()
            } else {
                eprintln!("process pressure task cancelled: {error:?}");
                "process pressure task cancelled".to_string()
            }
        })??;
    let elapsed = started.elapsed();
    let process = metrics.finish();
    let summary = summarize(
        args,
        run_id,
        "process://local".to_string(),
        elapsed,
        samples,
        process,
    );
    eprintln!(
        "{} finished attempted={} succeeded={} failed={} duration_ms={} throughput_ops_sec={:.1}",
        log_prefix(args),
        summary.attempted,
        summary.succeeded,
        summary.failed,
        summary.duration_ms,
        summary.throughput_ops_sec
    );
    Ok(summary)
}

async fn run_resource_governor_in_process(
    args: &Args,
    run_id: &str,
    total_operations: usize,
    identities: Arc<SyntheticIds>,
) -> Result<RunSummary, String> {
    let backend = build_backend(args, run_id).await?;
    eprintln!(
        "{} running target={} concurrency={} operations_per_thread={} total_operations={} users={} tenants={} progress_interval_seconds={}",
        log_prefix(args),
        backend.target,
        args.concurrency,
        args.operations,
        total_operations,
        args.users,
        args.tenants,
        args.progress_interval_seconds
    );
    let metrics = ProcessMetricsSampler::start(Duration::from_millis(100));
    let started = Instant::now();
    let governor = Arc::clone(&backend.governor);
    let args_clone = args.clone();
    let samples =
        tokio::task::spawn_blocking(move || run_threads(&governor, &args_clone, &identities))
            .await
            .map_err(|error| {
                if error.is_panic() {
                    eprintln!("run_threads task panicked: {error:?}");
                    "run_threads task panicked".to_string()
                } else {
                    eprintln!("run_threads task cancelled: {error:?}");
                    "run_threads task cancelled".to_string()
                }
            })??;
    let elapsed = started.elapsed();
    let process = metrics.finish();
    let summary = summarize(args, run_id, backend.target, elapsed, samples, process);
    eprintln!(
        "{} finished attempted={} succeeded={} failed={} duration_ms={} throughput_ops_sec={:.1}",
        log_prefix(args),
        summary.attempted,
        summary.succeeded,
        summary.failed,
        summary.duration_ms,
        summary.throughput_ops_sec
    );
    Ok(summary)
}

async fn run_user_turn_in_process(
    args: &Args,
    run_id: &str,
    total_operations: usize,
    identities: Arc<SyntheticIds>,
) -> Result<RunSummary, String> {
    let workload = Arc::new(build_user_turn_workload(args, run_id).await?);
    eprintln!(
        "{} running target={} concurrency={} operations_per_task={} total_operations={} users={} tenants={} progress_interval_seconds={}",
        log_prefix(args),
        workload.target(),
        args.concurrency,
        args.operations,
        total_operations,
        args.users,
        args.tenants,
        args.progress_interval_seconds
    );
    let metrics = ProcessMetricsSampler::start(Duration::from_millis(100));
    let started = Instant::now();
    let target = workload.target().to_string();
    let samples = run_user_turn_tasks(workload, args, identities).await?;
    let elapsed = started.elapsed();
    let process = metrics.finish();
    let summary = summarize(args, run_id, target, elapsed, samples, process);
    eprintln!(
        "{} finished attempted={} succeeded={} failed={} duration_ms={} throughput_ops_sec={:.1}",
        log_prefix(args),
        summary.attempted,
        summary.succeeded,
        summary.failed,
        summary.duration_ms,
        summary.throughput_ops_sec
    );
    Ok(summary)
}

fn run_threads(
    governor: &Arc<dyn ResourceGovernor>,
    args: &Args,
    identities: &Arc<SyntheticIds>,
) -> Result<Vec<Sample>, String> {
    let total_operations = args.concurrency.saturating_mul(args.operations);
    let progress = Arc::new(ProgressCounters::default());
    let progress_reporter = spawn_progress_reporter(
        log_prefix(args),
        args.backend.as_str(),
        args.scenario.as_str(),
        args.progress_interval_seconds,
        total_operations,
        Arc::clone(&progress),
    );
    let result = run_threads_inner(governor, args, identities, &progress);
    stop_progress_reporter(progress_reporter);
    result
}

fn run_threads_inner(
    governor: &Arc<dyn ResourceGovernor>,
    args: &Args,
    identities: &Arc<SyntheticIds>,
    progress: &Arc<ProgressCounters>,
) -> Result<Vec<Sample>, String> {
    let (sender, receiver) = mpsc::channel();
    let mut handles = Vec::with_capacity(args.concurrency);

    for worker_index in 0..args.concurrency {
        let governor = Arc::clone(governor);
        let identities = Arc::clone(identities);
        let sender = sender.clone();
        let progress = Arc::clone(progress);
        let args = args.clone();
        let handle = match thread::Builder::new()
            .name(format!("ironclaw-stress-{worker_index}"))
            .spawn(move || -> Result<(), String> {
                let mut samples = Vec::with_capacity(args.operations);
                for operation_index in 0..args.operations {
                    let sample = run_one_operation(
                        &governor,
                        &args,
                        &identities,
                        worker_index,
                        operation_index,
                    );
                    progress.record(sample.error.is_some());
                    samples.push(sample);
                }
                sender
                    .send(samples)
                    .map_err(|_| "sample receiver dropped".to_string())
            }) {
            Ok(handle) => handle,
            Err(error) => {
                join_workers(handles)?;
                return Err(format!("spawn worker {worker_index}: {error}"));
            }
        };
        handles.push((worker_index, handle));
    }
    drop(sender);

    let mut samples = Vec::with_capacity(args.concurrency * args.operations);
    for worker_samples in receiver {
        samples.extend(worker_samples);
    }
    join_workers(handles)?;
    let expected = args.concurrency * args.operations;
    if samples.len() != expected {
        return Err(format!(
            "collected {} samples but expected {expected}",
            samples.len()
        ));
    }
    Ok(samples)
}

fn run_one_operation(
    governor: &Arc<dyn ResourceGovernor>,
    args: &Args,
    identities: &SyntheticIds,
    worker_index: usize,
    operation_index: usize,
) -> Sample {
    let scope = identities.scope(args, worker_index, operation_index);
    let estimate = resource_ops::estimate();
    let usage = resource_ops::usage();

    let started = Instant::now();
    let outcome = match args.scenario {
        Scenario::ReserveRelease => governor
            .reserve(scope, estimate)
            .and_then(|reservation| governor.release(reservation.id).map(|_| ())),
        Scenario::ReserveReconcile => governor
            .reserve(scope, estimate)
            .and_then(|reservation| governor.reconcile(reservation.id, usage).map(|_| ())),
        Scenario::ChatTurn => unreachable!("chat-turn uses the async user-turn workload"),
        Scenario::MixedUserSession => {
            unreachable!("mixed-user-session uses the async user-turn workload")
        }
        Scenario::CpuBurn | Scenario::MemoryChurn => {
            unreachable!("process-only scenarios use the local pressure workload")
        }
    };
    let latency = started.elapsed();
    let failure = outcome
        .err()
        .map(|error| resource_ops::failure(args.scenario, error));
    let error = failure.as_ref().map(|cause| cause.bucket.clone());
    Sample {
        latency,
        error,
        failure,
        stages: None,
    }
}

fn summarize(
    args: &Args,
    run_id: &str,
    target: String,
    elapsed: Duration,
    samples: Vec<Sample>,
    process: ProcessMetrics,
) -> RunSummary {
    let mut errors = BTreeMap::new();
    let mut latencies: Vec<u128> = samples
        .iter()
        .map(|sample| sample.latency.as_micros())
        .collect();
    latencies.sort_unstable();
    let failed = samples
        .iter()
        .filter_map(|sample| sample.error.as_ref())
        .map(|error| {
            *errors.entry(error.clone()).or_insert(0) += 1;
        })
        .count() as u64;
    let attempted = samples.len() as u64;
    let succeeded = attempted.saturating_sub(failed);
    let elapsed_secs = elapsed.as_secs_f64().max(f64::MIN_POSITIVE);

    RunSummary {
        backend: args.backend,
        scenario: args.scenario,
        run_id: run_id.to_string(),
        target,
        child_index: args.child_index,
        processes: args.processes,
        concurrency: args.concurrency,
        operations_per_thread: args.operations,
        users: args.users,
        tenants: args.tenants,
        attempted,
        succeeded,
        failed,
        duration_ms: elapsed.as_millis(),
        throughput_ops_sec: attempted as f64 / elapsed_secs,
        latency: latency_summary(&latencies),
        process,
        stage_latency: summarize_user_turn_stages(&samples),
        errors,
        failure_causes: summarize_failure_causes(&samples),
    }
}

pub(crate) fn log_prefix(args: &Args) -> String {
    match args.child_index {
        Some(child_index) => format!("[ironclaw-stress child={child_index}]"),
        None => "[ironclaw-stress]".to_string(),
    }
}

async fn build_backend(args: &Args, run_id: &str) -> Result<BackendHandle, String> {
    match args.backend {
        Backend::Libsql => build_libsql_backend(args, run_id).await,
        Backend::Postgres => build_postgres_backend(args, run_id).await,
    }
}

#[cfg(feature = "libsql")]
async fn build_libsql_backend(args: &Args, run_id: &str) -> Result<BackendHandle, String> {
    let (filesystem, target) = build_libsql_root(args).await?;
    Ok(BackendHandle {
        governor: governor_from_root(filesystem, run_id)?,
        target,
    })
}

#[cfg(feature = "libsql")]
pub(crate) async fn build_libsql_root(
    args: &Args,
) -> Result<(Arc<ironclaw_filesystem::LibSqlRootFilesystem>, String), String> {
    use ironclaw_filesystem::LibSqlRootFilesystem;

    let path = args.libsql_path.clone().unwrap_or_else(default_libsql_path);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(display_err)?;
    }
    let db = Arc::new(
        libsql::Builder::new_local(&path)
            .build()
            .await
            .map_err(display_err)?,
    );
    let filesystem = Arc::new(LibSqlRootFilesystem::new(db));
    filesystem.run_migrations().await.map_err(display_err)?;
    Ok((filesystem, redact_libsql_path(&path)))
}

#[cfg(not(feature = "libsql"))]
async fn build_libsql_backend(_args: &Args, _run_id: &str) -> Result<BackendHandle, String> {
    Err("binary was built without the libsql feature".to_string())
}

#[cfg(feature = "postgres")]
async fn build_postgres_backend(args: &Args, run_id: &str) -> Result<BackendHandle, String> {
    let (filesystem, target) = build_postgres_root(args).await?;
    Ok(BackendHandle {
        governor: governor_from_root(filesystem, run_id)?,
        target,
    })
}

#[cfg(feature = "postgres")]
pub(crate) async fn build_postgres_root(
    args: &Args,
) -> Result<(Arc<ironclaw_filesystem::PostgresRootFilesystem>, String), String> {
    use ironclaw_filesystem::PostgresRootFilesystem;

    let url = resolve_postgres_url(args)?;
    let config = url
        .parse::<tokio_postgres::Config>()
        .map_err(|error| format!("parse Postgres URL: {error}"))?;
    let manager = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
    let pool = deadpool_postgres::Pool::builder(manager)
        .max_size(args.postgres_pool_size)
        .build()
        .map_err(display_err)?;
    let filesystem = Arc::new(PostgresRootFilesystem::new(pool));
    filesystem.run_migrations().await.map_err(display_err)?;
    Ok((filesystem, redact_postgres_url(&url)))
}

#[cfg(not(feature = "postgres"))]
async fn build_postgres_backend(_args: &Args, _run_id: &str) -> Result<BackendHandle, String> {
    Err("binary was built without the postgres feature".to_string())
}

pub(crate) fn governor_from_root<F>(
    root: Arc<F>,
    run_id: &str,
) -> Result<Arc<dyn ResourceGovernor>, String>
where
    F: RootFilesystem + 'static,
{
    let view = resource_mount_view(run_id)?;
    let scoped = Arc::new(ScopedFilesystem::with_fixed_view(root, view));
    let store = FilesystemResourceGovernorStore::new(scoped);
    Ok(Arc::new(PersistentResourceGovernor::new(store)))
}

fn resource_mount_view(run_id: &str) -> Result<MountView, String> {
    MountView::new(vec![MountGrant::new(
        MountAlias::new("/resources").map_err(display_err)?,
        VirtualPath::new(format!("/resources/stress/{run_id}")).map_err(display_err)?,
        MountPermissions::read_write_list_delete(),
    )])
    .map_err(display_err)
}

fn default_libsql_path() -> PathBuf {
    std::env::temp_dir().join(format!(
        "ironclaw-stress-{}.db",
        uuid::Uuid::new_v4().simple()
    ))
}

async fn cleanup_generated_libsql_path(path: &Path) {
    for candidate in [
        path.to_path_buf(),
        path.with_extension("db-wal"),
        path.with_extension("db-shm"),
    ] {
        match tokio::fs::remove_file(&candidate).await {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => eprintln!(
                "failed to remove generated libSQL file {}: {error}",
                candidate.display()
            ),
        }
    }
}

fn terminate_children(children: &mut Vec<(usize, Child, Option<JoinHandle<()>>)>) {
    for (_, child, _) in children.iter_mut() {
        let _ = child.kill();
    }
    for (child_index, mut child, stderr_reader) in children.drain(..) {
        let _ = child.wait();
        join_child_stderr_reader(child_index, stderr_reader);
    }
}

fn join_workers(handles: Vec<(usize, JoinHandle<Result<(), String>>)>) -> Result<(), String> {
    for (worker_index, handle) in handles {
        match handle.join() {
            Ok(result) => result.map_err(|error| format!("worker {worker_index}: {error}"))?,
            Err(payload) => {
                return Err(format!(
                    "worker {worker_index} panicked: {}",
                    panic_payload_to_string(&payload)
                ));
            }
        }
    }
    Ok(())
}

fn panic_payload_to_string(payload: &Box<dyn Any + Send + 'static>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        return (*message).to_string();
    }
    if let Some(message) = payload.downcast_ref::<String>() {
        return message.clone();
    }
    "non-string panic payload".to_string()
}

fn resolve_postgres_url(args: &Args) -> Result<String, String> {
    if let Some(url) = args.postgres_url.clone() {
        return Ok(url);
    }
    if let Some(url) = optional_env_var("IRONCLAW_FILESYSTEM_POSTGRES_URL")? {
        return Ok(url);
    }
    // silent-ok: IRONCLAW_FILESYSTEM_POSTGRES_URL is optional; DATABASE_URL is the documented fallback.
    if let Some(url) = optional_env_var("DATABASE_URL")? {
        return Ok(url);
    }
    Err(
        "Postgres requires --postgres-url, IRONCLAW_FILESYSTEM_POSTGRES_URL, or DATABASE_URL"
            .to_string(),
    )
}

fn optional_env_var(name: &str) -> Result<Option<String>, String> {
    match env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(VarError::NotPresent) => Ok(None),
        Err(VarError::NotUnicode(_)) => Err(format!("{name} is not valid Unicode")),
    }
}

fn display_err(error: impl std::fmt::Display) -> String {
    error.to_string()
}
