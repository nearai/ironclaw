use std::{
    cmp::Ordering,
    collections::BTreeMap,
    fmt::Write,
    fs::File,
    io::{BufRead, BufReader},
    path::PathBuf,
};

use serde_json::Value;

use crate::{
    Args, RunSummary,
    capture::CapturedRun,
    db_probe::DbProbeSummary,
    process_metrics::{ProcessMetrics, aggregate_process_metrics},
    summary::FailureCauseSummary,
    trace,
    user_turn::{
        StageLatencySummary, UserTurnOperationAttributionSummary, UserTurnStageLatencySummary,
        operation_attribution_rows,
    },
};

pub(crate) fn render_bottleneck_report(
    args: &Args,
    run_id: &str,
    captured: &CapturedRun,
) -> String {
    let mut findings = Vec::new();
    match captured {
        CapturedRun::Single(summary) => analyze_summary(summary, &mut findings),
        CapturedRun::Parent { summaries, .. } => analyze_parent(args, summaries, &mut findings),
    }
    analyze_trace(args, &mut findings);
    findings.sort_by(compare_findings);

    let mut output = String::new();
    let _ = writeln!(output, "\nBottleneck analysis");
    let _ = writeln!(output, "{:<22} {:<7} evidence", "signal", "level");
    let _ = writeln!(output, "{:-<22} {:-<7} {:-<56}", "", "", "");
    if findings.is_empty() {
        let _ = writeln!(
            output,
            "{:<22} {:<7} no threshold-level signal found for run_id={run_id}",
            "none", "info"
        );
        let _ = writeln!(
            output,
            "\nNext probes\n- Increase concurrency/users with --bottleneck-report and enable --trace-jsonl for interval-level collapse detection."
        );
        return output;
    }

    for finding in &findings {
        let _ = writeln!(
            output,
            "{:<22} {:<7} {}",
            finding.signal, finding.level, finding.evidence
        );
    }

    let _ = writeln!(output, "\nNext probes");
    for finding in findings.iter().take(5) {
        let _ = writeln!(output, "- {}", finding.next_probe);
    }
    output
}

#[derive(Debug)]
struct Finding {
    level: &'static str,
    signal: &'static str,
    evidence: String,
    next_probe: String,
}

fn analyze_parent(args: &Args, summaries: &[RunSummary], findings: &mut Vec<Finding>) {
    let attempted = summaries.iter().map(|summary| summary.attempted).sum();
    let failed = summaries.iter().map(|summary| summary.failed).sum();
    let max_duration_ms = summaries
        .iter()
        .map(|summary| summary.duration_ms)
        .max()
        .unwrap_or(0);
    let process = aggregate_process_metrics(summaries.iter().map(|summary| &summary.process));
    analyze_failures(
        attempted,
        failed,
        &aggregate_errors(summaries),
        &aggregate_failure_causes(summaries),
        findings,
    );
    analyze_process(max_duration_ms, &process, findings);
    analyze_child_skew(args, summaries, findings);
    for summary in summaries {
        if let Some(attribution) = &summary.operation_attribution {
            analyze_operation_attribution(attribution, findings);
            break;
        }
    }
    for summary in summaries {
        if let Some(stages) = &summary.stage_latency {
            analyze_stage_latency(stages, findings);
            break;
        }
    }
    analyze_parent_db(summaries, findings);
}

fn analyze_summary(summary: &RunSummary, findings: &mut Vec<Finding>) {
    analyze_failures(
        summary.attempted,
        summary.failed,
        &summary.errors,
        &summary.failure_causes,
        findings,
    );
    analyze_process(summary.duration_ms, &summary.process, findings);
    if let Some(attribution) = &summary.operation_attribution {
        analyze_operation_attribution(attribution, findings);
    }
    if let Some(stages) = &summary.stage_latency {
        analyze_stage_latency(stages, findings);
    }
    if let Some(db_probe) = &summary.db_probe {
        analyze_db_probe(db_probe, findings);
    }
}

fn analyze_failures(
    attempted: u64,
    failed: u64,
    errors: &BTreeMap<String, u64>,
    failure_causes: &BTreeMap<String, FailureCauseSummary>,
    findings: &mut Vec<Finding>,
) {
    if attempted == 0 || failed == 0 {
        return;
    }
    let failure_rate = failed as f64 / attempted as f64;
    let level = if failure_rate >= 0.05 {
        "high"
    } else {
        "medium"
    };
    let top_error = errors
        .iter()
        .max_by_key(|(_, count)| *count)
        .map(|(error, count)| format!("{error}={count}"))
        .unwrap_or_else(|| "unknown".to_string());
    findings.push(Finding {
        level,
        signal: "failure_rate",
        evidence: format!(
            "{} failed of {} ({:.2}%), top_error={top_error}",
            failed,
            attempted,
            failure_rate * 100.0
        ),
        next_probe: failure_probe(top_error.as_str(), failure_causes),
    });
}

fn failure_probe(
    top_error: &str,
    failure_causes: &BTreeMap<String, FailureCauseSummary>,
) -> String {
    if top_error.starts_with("turn_thread_busy") {
        return "Raise --users or spread operations across more threads to separate storage limits from intentional same-thread serialization.".to_string();
    }
    if top_error.starts_with("storage_cross_process_cas_contention") {
        return "Compare --processes 1 against multi-process runs; this points at cross-process CAS/update contention.".to_string();
    }
    if let Some((bucket, cause)) = failure_causes.iter().next() {
        return format!(
            "Inspect failure bucket {bucket}; most common stages={:?}, sample={}",
            cause.stages, cause.sample_detail
        );
    }
    "Inspect stderr spans with --span-log-failures or lower --slow-span-threshold-ms to capture failing operation stages.".to_string()
}

fn analyze_stage_latency(stages: &UserTurnStageLatencySummary, findings: &mut Vec<Finding>) {
    let Some((name, stage)) = stage_rows(stages)
        .into_iter()
        .filter(|(_, stage)| stage.count > 0)
        .max_by_key(|(_, stage)| stage.latency.p95_us)
    else {
        return;
    };
    findings.push(Finding {
        level: if stage.latency.p95_us >= 500_000 {
            "high"
        } else {
            "info"
        },
        signal: "top_stage_p95",
        evidence: format!(
            "{} p95={} p99={} count={}",
            name,
            format_duration_us(stage.latency.p95_us),
            format_duration_us(stage.latency.p99_us),
            stage.count
        ),
        next_probe: stage_probe(name),
    });
}

fn analyze_operation_attribution(
    attribution: &UserTurnOperationAttributionSummary,
    findings: &mut Vec<Finding>,
) {
    let Some((name, group)) = operation_attribution_rows(attribution)
        .into_iter()
        .filter(|(_, group)| group.count > 0)
        .max_by_key(|(_, group)| group.latency.p95_us)
    else {
        return;
    };
    findings.push(Finding {
        level: if group.latency.p95_us >= 500_000 {
            "high"
        } else {
            "info"
        },
        signal: "top_operation_group",
        evidence: format!(
            "{} p95={} p99={} count={}",
            name,
            format_duration_us(group.latency.p95_us),
            format_duration_us(group.latency.p99_us),
            group.count
        ),
        next_probe: operation_attribution_probe(name),
    });
}

fn operation_attribution_probe(name: &str) -> String {
    match name {
        "thread_store_writes" => {
            "Sweep --assistant-message-bytes, --tool-output-bytes, and --active-thread-count to separate payload write cost from hot-thread write contention.".to_string()
        }
        "context_reads" => {
            "Sweep --context-max-messages and --prefill-turns-per-thread to measure context read amplification.".to_string()
        }
        "turn_store" => {
            "Compare --scenario chat-turn against reserve-reconcile; high turn_store points at run claim/complete state transitions.".to_string()
        }
        "resource_governor" => {
            "Run reserve-release and resource-contention presets to isolate governor reservation/reconcile/release writes.".to_string()
        }
        "model_tool_wait" => {
            "Sweep --model-latency-ms and --tool-latency-ms, or use --model-latency-source provider; if this bucket dominates, storage is not the current p95 ceiling.".to_string()
        }
        _ => format!("Inspect stage latency under {name} and rerun with --trace-jsonl for interval-level timing."),
    }
}

fn stage_probe(name: &str) -> String {
    match name {
        "accept_inbound" | "append_assistant" | "complete_run" | "load_context" => {
            format!("Run the same scenario with higher --users and compare {name}; this is likely storage read/write latency.")
        }
        "model_wait" => "Sweep --model-latency-ms and --model-latency-profile to isolate model wait from storage overhead.".to_string(),
        "tool_wait" => "Sweep --tool-latency-ms and --tool-calls-per-turn to isolate tool wait from transcript write overhead.".to_string(),
        "resource_reserve" | "resource_reconcile" | "resource_release" => {
            "Compare reserve-release and reserve-reconcile with the same concurrency to isolate resource-governor write contention.".to_string()
        }
        _ => format!("Enable --trace-jsonl and inspect intervals where {name} dominates operation latency."),
    }
}

fn analyze_process(duration_ms: u128, process: &ProcessMetrics, findings: &mut Vec<Finding>) {
    if let Some(cpu_ms) = process.delta_cpu_ms
        && duration_ms > 0
    {
        let cpu_cores = cpu_ms as f64 / duration_ms as f64;
        if cpu_cores >= 0.8 {
            findings.push(Finding {
                level: if cpu_cores >= 1.5 { "high" } else { "medium" },
                signal: "cpu_pressure",
                evidence: format!(
                    "cpu={} wall={} effective_cores={cpu_cores:.2}",
                    format_duration_ms(cpu_ms),
                    format_duration_ms(duration_ms)
                ),
                next_probe: "Run cpu-burn and the target scenario at the same concurrency; if both flatten together, host CPU is the ceiling.".to_string(),
            });
        }
    }

    if let (Some(start_rss), Some(peak_rss)) = (process.start.rss_kb, process.peak_rss_kb) {
        let delta_kb = peak_rss.saturating_sub(start_rss);
        if delta_kb >= 64 * 1024 || (start_rss > 0 && peak_rss >= start_rss.saturating_mul(2)) {
            findings.push(Finding {
                level: "medium",
                signal: "memory_growth",
                evidence: format!(
                    "rss_start={} peak={} delta={}",
                    format_kb(start_rss),
                    format_kb(peak_rss),
                    format_kb(delta_kb)
                ),
                next_probe: "Run memory-churn as a control, then rerun the target with --duration-seconds to see if RSS keeps climbing.".to_string(),
            });
        }
    }

    if let Some(open_fds) = process.peak_open_fds
        && open_fds >= 512
    {
        findings.push(Finding {
            level: "medium",
            signal: "fd_pressure",
            evidence: format!("peak_open_fds={open_fds}"),
            next_probe: "Lower pool/concurrency and compare peak_open_fds; a rising value can indicate connection/file descriptor pressure.".to_string(),
        });
    }
}

fn analyze_db_probe(db_probe: &DbProbeSummary, findings: &mut Vec<Finding>) {
    if let Some(error) = db_probe
        .after
        .error
        .as_ref()
        .or(db_probe.before.error.as_ref())
    {
        findings.push(Finding {
            level: "medium",
            signal: "db_probe_error",
            evidence: error.clone(),
            next_probe: "Fix probe access first; missing DB probe data weakens storage bottleneck attribution.".to_string(),
        });
    }

    if let Some(bytes) = db_probe.delta.libsql_file_bytes
        && bytes > 0
    {
        findings.push(Finding {
            level: "info",
            signal: "libsql_growth",
            evidence: format!("database_file_delta={}", format_bytes_i128(bytes)),
            next_probe: "Compare growth across chat-turn, context-growth, and tool-session to find write amplification.".to_string(),
        });
    }

    if let Some(bytes) = db_probe.delta.postgres_database_size_bytes
        && bytes > 0
    {
        findings.push(Finding {
            level: "info",
            signal: "postgres_growth",
            evidence: format!("database_size_delta={}", format_bytes_i128(bytes)),
            next_probe: "Track size delta per successful operation across user-message and tool-output payload sweeps.".to_string(),
        });
    }

    if let Some(waiting) = db_probe.after.postgres_waiting_connections
        && waiting > 0
    {
        findings.push(Finding {
            level: "high",
            signal: "postgres_waiting",
            evidence: format!("waiting_connections={waiting}"),
            next_probe: "Lower --postgres-pool-size or --concurrency and compare waiting connections against stage latency.".to_string(),
        });
    }
}

fn analyze_parent_db(summaries: &[RunSummary], findings: &mut Vec<Finding>) {
    let mut largest_libsql_delta = None;
    let mut largest_postgres_delta = None;
    let mut max_waiting = 0;
    for summary in summaries {
        let Some(db_probe) = &summary.db_probe else {
            continue;
        };
        largest_libsql_delta = max_i128(largest_libsql_delta, db_probe.delta.libsql_file_bytes);
        largest_postgres_delta = max_i128(
            largest_postgres_delta,
            db_probe.delta.postgres_database_size_bytes,
        );
        max_waiting = max_waiting.max(db_probe.after.postgres_waiting_connections.unwrap_or(0));
    }
    if let Some(bytes) = largest_libsql_delta
        && bytes > 0
    {
        findings.push(Finding {
            level: "info",
            signal: "libsql_growth",
            evidence: format!("largest_child_file_delta={}", format_bytes_i128(bytes)),
            next_probe: "Compare child trace files to see whether one process is driving most storage growth.".to_string(),
        });
    }
    if let Some(bytes) = largest_postgres_delta
        && bytes > 0
    {
        findings.push(Finding {
            level: "info",
            signal: "postgres_growth",
            evidence: format!("largest_child_size_delta={}", format_bytes_i128(bytes)),
            next_probe:
                "Compare per-child deltas and active connections before changing pool sizes."
                    .to_string(),
        });
    }
    if max_waiting > 0 {
        findings.push(Finding {
            level: "high",
            signal: "postgres_waiting",
            evidence: format!("max_child_waiting_connections={max_waiting}"),
            next_probe: "Tune --postgres-pool-size against --concurrency until waiting connections stop rising.".to_string(),
        });
    }
}

fn analyze_child_skew(args: &Args, summaries: &[RunSummary], findings: &mut Vec<Finding>) {
    if summaries.len() < 2 {
        return;
    }
    let min_duration = summaries
        .iter()
        .map(|summary| summary.duration_ms)
        .min()
        .unwrap_or(0);
    let max_duration = summaries
        .iter()
        .map(|summary| summary.duration_ms)
        .max()
        .unwrap_or(0);
    if min_duration > 0 && max_duration > min_duration + min_duration / 4 {
        findings.push(Finding {
            level: "medium",
            signal: "child_skew",
            evidence: format!(
                "processes={} min_duration={} max_duration={}",
                args.processes,
                format_duration_ms(min_duration),
                format_duration_ms(max_duration)
            ),
            next_probe: "Inspect child trace files; skew usually means lock contention, uneven DB waits, or OS scheduling pressure.".to_string(),
        });
    }
}

fn analyze_trace(args: &Args, findings: &mut Vec<Finding>) {
    let (samples, errors) = load_trace_samples(args);
    for error in errors {
        findings.push(Finding {
            level: "medium",
            signal: "trace_read_error",
            evidence: error,
            next_probe: "Verify --trace-jsonl points at the file written by this run; multi-process runs use child-specific trace files.".to_string(),
        });
    }
    if samples.is_empty() {
        return;
    }
    let max_recent_ops = samples
        .iter()
        .filter_map(|sample| sample.recent_ops_sec)
        .fold(0.0_f64, f64::max);
    let min_recent_ops = samples
        .iter()
        .filter_map(|sample| sample.recent_ops_sec)
        .filter(|value| *value > 0.0)
        .min_by(|left, right| left.partial_cmp(right).unwrap_or(Ordering::Equal));
    if let Some(min_recent_ops) = min_recent_ops
        && max_recent_ops > 0.0
        && min_recent_ops < max_recent_ops * 0.5
    {
        findings.push(Finding {
            level: "medium",
            signal: "throughput_drop",
            evidence: format!("trace_recent_ops_sec min={min_recent_ops:.2} max={max_recent_ops:.2}"),
            next_probe: "Inspect the trace interval with lowest recent_ops_sec and compare process RSS/CPU and DB delta at that timestamp.".to_string(),
        });
    }

    let Some(worst_latency) = samples
        .iter()
        .filter(|sample| sample.interval_count.unwrap_or(0) > 0)
        .max_by_key(|sample| sample.interval_p95_us.unwrap_or(0))
    else {
        return;
    };
    let p95 = worst_latency.interval_p95_us.unwrap_or(0);
    if p95 > 0 {
        findings.push(Finding {
            level: if p95 >= 500_000 { "high" } else { "info" },
            signal: "trace_worst_p95",
            evidence: format!(
                "{} seq={} phase={} p95={} recent_ops_sec={:.2}",
                worst_latency.path,
                worst_latency.sequence.unwrap_or(0),
                worst_latency.phase.as_deref().unwrap_or("unknown"),
                format_duration_us(p95),
                worst_latency.recent_ops_sec.unwrap_or(0.0)
            ),
            next_probe: "Use the trace sequence and elapsed_ms to correlate latency spikes with DB growth, RSS growth, and stderr spans.".to_string(),
        });
    }
}

#[derive(Debug)]
struct TraceSampleView {
    path: String,
    phase: Option<String>,
    sequence: Option<u64>,
    recent_ops_sec: Option<f64>,
    interval_count: Option<u64>,
    interval_p95_us: Option<u128>,
}

fn load_trace_samples(args: &Args) -> (Vec<TraceSampleView>, Vec<String>) {
    let mut samples = Vec::new();
    let mut errors = Vec::new();
    for path in trace_paths(args) {
        match load_trace_path(path) {
            Ok(path_samples) => samples.extend(path_samples),
            Err(error) => errors.push(error),
        }
    }
    (samples, errors)
}

fn trace_paths(args: &Args) -> Vec<PathBuf> {
    let Some(path) = &args.trace_jsonl else {
        return Vec::new();
    };
    if args.child_index.is_none() && args.processes > 1 {
        (0..args.processes)
            .map(|child_index| trace::child_trace_path(path, child_index))
            .collect()
    } else {
        vec![path.clone()]
    }
}

fn load_trace_path(path: PathBuf) -> Result<Vec<TraceSampleView>, String> {
    let file =
        File::open(&path).map_err(|error| format!("open trace {}: {error}", path.display()))?;
    let reader = BufReader::new(file);
    let mut samples = Vec::new();
    for line in reader.lines() {
        let line = line.map_err(|error| format!("read trace {}: {error}", path.display()))?;
        let value: Value = serde_json::from_str(&line)
            .map_err(|error| format!("parse trace {}: {error}", path.display()))?;
        samples.push(TraceSampleView {
            path: path.display().to_string(),
            phase: value
                .get("phase")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            sequence: value.get("sequence").and_then(Value::as_u64),
            recent_ops_sec: value.get("recent_ops_sec").and_then(Value::as_f64),
            interval_count: value
                .pointer("/interval_latency/count")
                .and_then(Value::as_u64),
            interval_p95_us: value
                .pointer("/interval_latency/latency/p95_us")
                .and_then(Value::as_u64)
                .map(u128::from),
        });
    }
    Ok(samples)
}

fn aggregate_errors(summaries: &[RunSummary]) -> BTreeMap<String, u64> {
    let mut errors = BTreeMap::new();
    for summary in summaries {
        for (error, count) in &summary.errors {
            *errors.entry(error.clone()).or_insert(0) += count;
        }
    }
    errors
}

fn aggregate_failure_causes(summaries: &[RunSummary]) -> BTreeMap<String, FailureCauseSummary> {
    let mut failure_causes = BTreeMap::new();
    for summary in summaries {
        for (bucket, cause) in &summary.failure_causes {
            let aggregate =
                failure_causes
                    .entry(bucket.clone())
                    .or_insert_with(|| FailureCauseSummary {
                        count: 0,
                        stages: BTreeMap::new(),
                        sample_detail: cause.sample_detail.clone(),
                    });
            aggregate.count += cause.count;
            for (stage, count) in &cause.stages {
                *aggregate.stages.entry(stage.clone()).or_insert(0) += count;
            }
        }
    }
    failure_causes
}

fn stage_rows(stages: &UserTurnStageLatencySummary) -> [(&'static str, &StageLatencySummary); 18] {
    [
        ("ensure_thread", &stages.ensure_thread),
        ("accept_inbound", &stages.accept_inbound),
        ("submit_turn", &stages.submit_turn),
        ("mark_submitted", &stages.mark_submitted),
        ("mark_rejected_busy", &stages.mark_rejected_busy),
        ("claim_run", &stages.claim_run),
        ("append_assistant", &stages.append_assistant),
        ("finalize_assistant", &stages.finalize_assistant),
        ("complete_run", &stages.complete_run),
        ("load_context", &stages.load_context),
        ("resource_reserve", &stages.resource_reserve),
        ("model_wait", &stages.model_wait),
        ("tool_wait", &stages.tool_wait),
        ("append_tool_result", &stages.append_tool_result),
        ("append_tool_preview", &stages.append_tool_preview),
        ("update_assistant_draft", &stages.update_assistant_draft),
        ("resource_reconcile", &stages.resource_reconcile),
        ("resource_release", &stages.resource_release),
    ]
}

fn compare_findings(left: &Finding, right: &Finding) -> Ordering {
    severity_rank(right.level)
        .cmp(&severity_rank(left.level))
        .then_with(|| left.signal.cmp(right.signal))
}

fn severity_rank(level: &str) -> u8 {
    match level {
        "high" => 3,
        "medium" => 2,
        _ => 1,
    }
}

fn max_i128(left: Option<i128>, right: Option<i128>) -> Option<i128> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}

fn format_duration_us(value: u128) -> String {
    if value >= 1_000_000 {
        format!("{:.2}s", value as f64 / 1_000_000.0)
    } else if value >= 1_000 {
        format!("{:.1}ms", value as f64 / 1_000.0)
    } else {
        format!("{value}us")
    }
}

fn format_duration_ms(value: u128) -> String {
    if value >= 1_000 {
        format!("{:.2}s", value as f64 / 1_000.0)
    } else {
        format!("{value}ms")
    }
}

fn format_kb(value: u64) -> String {
    if value >= 1024 * 1024 {
        format!("{:.1}GiB", value as f64 / (1024.0 * 1024.0))
    } else if value >= 1024 {
        format!("{:.1}MiB", value as f64 / 1024.0)
    } else {
        format!("{value}KiB")
    }
}

fn format_bytes_i128(value: i128) -> String {
    if value < 0 {
        format!("-{}", format_bytes(value.unsigned_abs()))
    } else {
        format_bytes(value as u128)
    }
}

fn format_bytes(value: u128) -> String {
    if value >= 1024 * 1024 * 1024 {
        format!("{:.1}GiB", value as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if value >= 1024 * 1024 {
        format!("{:.1}MiB", value as f64 / (1024.0 * 1024.0))
    } else if value >= 1024 {
        format!("{:.1}KiB", value as f64 / 1024.0)
    } else {
        format!("{value}B")
    }
}
