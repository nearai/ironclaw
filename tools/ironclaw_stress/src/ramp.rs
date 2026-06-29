use std::{
    fmt::Write as FmtWrite,
    fs::File,
    io::{BufWriter, Write as IoWrite},
    time::Instant,
};

use serde::Serialize;
use serde_json::json;

use crate::{Args, compare, run_once, sweep};

#[derive(Debug, Clone, Copy, Serialize)]
pub(crate) enum RampAxis {
    Concurrency,
    Users,
}

impl RampAxis {
    fn as_str(self) -> &'static str {
        match self {
            Self::Concurrency => "concurrency",
            Self::Users => "users",
        }
    }
}

#[derive(Debug, Serialize)]
struct RampResult {
    step_index: usize,
    value: usize,
    run_id: String,
    metrics: sweep::RunMetrics,
    threshold_error: Option<String>,
}

pub(crate) fn is_enabled(args: &Args) -> bool {
    args.ramp_concurrency.is_some() || args.ramp_users.is_some()
}

pub(crate) async fn run(args: &Args, suite_run_id: &str) -> Result<(), String> {
    let (axis, max_value) = axis(args).ok_or_else(|| "ramp mode is not enabled".to_string())?;
    let start_value = match axis {
        RampAxis::Concurrency => args.concurrency,
        RampAxis::Users => args.users,
    };
    let values = build_values(start_value, max_value, args.ramp_factor);
    let mut jsonl = match &args.output_jsonl {
        Some(path) => {
            Some(BufWriter::new(File::create(path).map_err(|error| {
                format!("create {}: {error}", path.display())
            })?))
        }
        None => None,
    };
    let mut results = Vec::with_capacity(values.len());
    let mut stopped_on_threshold = false;

    eprintln!(
        "{} starting ramp suite_run_id={} axis={} start={} max={} factor={}",
        crate::log_prefix(args),
        suite_run_id,
        axis.as_str(),
        start_value,
        max_value,
        args.ramp_factor
    );

    for (step_index, value) in values.into_iter().enumerate() {
        let run_id = format!(
            "{suite_run_id}-ramp-{}-{value}",
            axis.as_str().replace('-', "_")
        );
        let label = format!("{}={value}", axis.as_str());
        let mut step_args = args.clone();
        step_args.run_id = Some(run_id.clone());
        step_args.ramp_concurrency = None;
        step_args.ramp_users = None;
        step_args.output_jsonl = None;
        if let Some(trace_jsonl) = &args.trace_jsonl {
            let trace_label = format!("ramp-{}-{value}", axis.as_str());
            step_args.trace_jsonl =
                Some(crate::trace::labeled_trace_path(trace_jsonl, &trace_label));
        }
        match axis {
            RampAxis::Concurrency => step_args.concurrency = value,
            RampAxis::Users => step_args.users = value,
        }

        eprintln!(
            "{} ramp point label=\"{}\" backend={} scenario={}",
            crate::log_prefix(args),
            label,
            step_args.backend.as_str(),
            step_args.scenario.as_str()
        );
        crate::trace::prepare_trace_outputs(&step_args).await?;
        let started = Instant::now();
        let captured = run_once(&step_args, &run_id).await?;
        let metrics = captured.metrics();
        let summary = captured.summary_value();
        let duration_ms = started.elapsed().as_millis();
        let threshold_error =
            sweep::enforce_thresholds(&step_args, &[(label.clone(), metrics)]).err();
        let record = json!({
            "suite_run_id": suite_run_id,
            "run_id": run_id,
            "axis": axis.as_str(),
            "step_index": step_index,
            "value": value,
            "backend": step_args.backend,
            "scenario": step_args.scenario,
            "processes": step_args.processes,
            "concurrency": step_args.concurrency,
            "users": step_args.users,
            "tenants": step_args.tenants,
            "operations_per_thread": step_args.operations,
            "duration_seconds": step_args.duration_seconds,
            "warmup_seconds": step_args.warmup_seconds,
            "trace_jsonl": step_args.trace_jsonl.as_ref().map(|path| path.display().to_string()),
            "model_latency_ms": step_args.model_latency_ms,
            "threshold_error": threshold_error,
            "duration_ms": duration_ms,
            "metrics": metrics,
            "summary": summary,
        });
        if let Some(writer) = jsonl.as_mut() {
            serde_json::to_writer(&mut *writer, &record).map_err(|error| error.to_string())?;
            writer.write_all(b"\n").map_err(|error| error.to_string())?;
        }

        let result = RampResult {
            step_index,
            value,
            run_id,
            metrics,
            threshold_error,
        };
        let should_stop = result.threshold_error.is_some();
        results.push(result);
        if should_stop {
            stopped_on_threshold = true;
            break;
        }
    }

    if let Some(mut writer) = jsonl {
        writer.flush().map_err(|error| error.to_string())?;
    }

    let last_passing = results
        .iter()
        .rev()
        .find(|result| result.threshold_error.is_none());
    let first_failing = results
        .iter()
        .find(|result| result.threshold_error.is_some());
    let suite = json!({
        "suite_run_id": suite_run_id,
        "axis": axis.as_str(),
        "start": start_value,
        "max": max_value,
        "factor": args.ramp_factor,
        "stopped_on_threshold": stopped_on_threshold,
        "last_passing": last_passing,
        "first_failing": first_failing,
        "runs": results,
    });
    let encoded = serde_json::to_string_pretty(&suite).map_err(|error| error.to_string())?;
    println!("{encoded}");

    if args.human_read {
        eprint!("{}", render_ramp_summary(axis, &results));
    }
    if args.bottleneck_report {
        eprint!("{}", render_ramp_bottleneck_report(axis, &results));
    }
    if let Some(path) = &args.compare_json {
        eprint!("{}", compare::render_comparison_report(path, &suite)?);
    }
    Ok(())
}

fn axis(args: &Args) -> Option<(RampAxis, usize)> {
    if let Some(max_concurrency) = args.ramp_concurrency {
        return Some((RampAxis::Concurrency, max_concurrency));
    }
    args.ramp_users
        .map(|max_users| (RampAxis::Users, max_users))
}

pub(crate) fn build_values(start: usize, max: usize, factor: usize) -> Vec<usize> {
    let mut values = Vec::new();
    let mut value = start;
    loop {
        values.push(value);
        if value >= max {
            break;
        }
        value = value.saturating_mul(factor).min(max);
        if values.last().copied() == Some(value) {
            break;
        }
    }
    values
}

fn render_ramp_summary(axis: RampAxis, results: &[RampResult]) -> String {
    let mut output = String::new();
    let _ = writeln!(output, "\nRamp summary");
    let _ = writeln!(
        output,
        "{:<14} {:>8} {:>9} {:>8} {:>10} {:>10} {:>10} {:>10} {:<12}",
        axis.as_str(),
        "attempts",
        "fail%",
        "ops/sec",
        "p95",
        "p99",
        "cpu",
        "rss",
        "status"
    );
    let _ = writeln!(
        output,
        "{:-<14} {:->8} {:->9} {:->8} {:->10} {:->10} {:->10} {:->10} {:-<12}",
        "", "", "", "", "", "", "", "", ""
    );
    for result in results {
        let status = if result.threshold_error.is_some() {
            "threshold"
        } else {
            "ok"
        };
        let _ = writeln!(
            output,
            "{:<14} {:>8} {:>8.2}% {:>8.2} {:>10} {:>10} {:>10} {:>10} {:<12}",
            result.value,
            result.metrics.attempted,
            failure_rate(result.metrics) * 100.0,
            result.metrics.throughput_ops_sec,
            format_latency_us(result.metrics.p95_us),
            format_latency_us(result.metrics.p99_us),
            format_optional_ms(result.metrics.cpu_ms),
            format_optional_kb(result.metrics.peak_rss_kb),
            status
        );
    }
    output
}

fn render_ramp_bottleneck_report(axis: RampAxis, results: &[RampResult]) -> String {
    let mut output = String::new();
    let _ = writeln!(output, "\nRamp bottleneck analysis");
    let _ = writeln!(output, "{:<22} evidence", "signal");
    let _ = writeln!(output, "{:-<22} {:-<56}", "", "");
    if results.is_empty() {
        let _ = writeln!(output, "{:<22} no ramp points", "none");
        return output;
    }

    let last_passing = results
        .iter()
        .rev()
        .find(|result| result.threshold_error.is_none());
    let first_failing = results
        .iter()
        .find(|result| result.threshold_error.is_some());
    match (last_passing, first_failing) {
        (Some(last_passing), Some(first_failing)) => {
            let _ = writeln!(
                output,
                "{:<22} last_passing_{}={} first_failing_{}={}",
                "limit_found",
                axis.as_str(),
                last_passing.value,
                axis.as_str(),
                first_failing.value
            );
            if let Some(error) = &first_failing.threshold_error {
                let _ = writeln!(output, "{:<22} {}", "threshold", error);
            }
        }
        (None, Some(first_failing)) => {
            let _ = writeln!(
                output,
                "{:<22} first point {}={} failed threshold",
                "limit_below_start",
                axis.as_str(),
                first_failing.value
            );
        }
        (Some(last_passing), None) => {
            let _ = writeln!(
                output,
                "{:<22} no threshold breach through {}={}",
                "limit_not_found",
                axis.as_str(),
                last_passing.value
            );
        }
        (None, None) => {}
    }

    if let Some(result) = results.iter().max_by_key(|result| result.metrics.p95_us) {
        let _ = writeln!(
            output,
            "{:<22} {}={} p95={} p99={}",
            "worst_latency",
            axis.as_str(),
            result.value,
            format_latency_us(result.metrics.p95_us),
            format_latency_us(result.metrics.p99_us)
        );
    }
    if let Some(result) = results.iter().min_by(|left, right| {
        left.metrics
            .throughput_ops_sec
            .partial_cmp(&right.metrics.throughput_ops_sec)
            .unwrap_or(std::cmp::Ordering::Equal)
    }) {
        let _ = writeln!(
            output,
            "{:<22} {}={} throughput={:.2} ops/sec",
            "lowest_throughput",
            axis.as_str(),
            result.value,
            result.metrics.throughput_ops_sec
        );
    }

    let _ = writeln!(output, "\nNext probes");
    if first_failing.is_some() {
        let _ = writeln!(
            output,
            "- Rerun the first failing point with --trace-jsonl and --bottleneck-report to inspect interval-level collapse."
        );
        let _ = writeln!(
            output,
            "- Rerun between the last passing and first failing values with a smaller starting point or lower --ramp-factor."
        );
    } else {
        let _ = writeln!(
            output,
            "- Increase the ramp maximum or tighten thresholds; this run did not find the limit."
        );
    }
    output
}

fn failure_rate(metrics: sweep::RunMetrics) -> f64 {
    if metrics.attempted == 0 {
        0.0
    } else {
        metrics.failed as f64 / metrics.attempted as f64
    }
}

fn format_latency_us(us: u128) -> String {
    if us >= 1_000_000 {
        format!("{:.2}s", us as f64 / 1_000_000.0)
    } else if us >= 1_000 {
        format!("{:.1}ms", us as f64 / 1_000.0)
    } else {
        format!("{us}us")
    }
}

fn format_duration_ms(ms: u128) -> String {
    if ms >= 1_000 {
        format!("{:.2}s", ms as f64 / 1_000.0)
    } else {
        format!("{ms}ms")
    }
}

fn format_kb(kb: u64) -> String {
    if kb >= 1024 * 1024 {
        format!("{:.2}GB", kb as f64 / 1024.0 / 1024.0)
    } else if kb >= 1024 {
        format!("{:.1}MB", kb as f64 / 1024.0)
    } else {
        format!("{kb}KB")
    }
}

fn format_optional_ms(value: Option<u128>) -> String {
    value
        .map(format_duration_ms)
        .unwrap_or_else(|| "-".to_string())
}

fn format_optional_kb(value: Option<u64>) -> String {
    value.map(format_kb).unwrap_or_else(|| "-".to_string())
}
