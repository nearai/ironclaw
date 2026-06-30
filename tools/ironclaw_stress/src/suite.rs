use std::{
    fmt::Write as FmtWrite,
    fs::File,
    io::{BufWriter, Write as IoWrite},
    time::Instant,
};

use serde::Serialize;
use serde_json::{Value, json};

use crate::{
    Args, ModelLatencyProfile, Scenario, StressPreset, StressSuite, compare, run_once, sweep,
};

#[derive(Debug, Clone, Copy, Serialize)]
pub(crate) struct SuiteCase {
    pub(crate) label: &'static str,
    pub(crate) probe: &'static str,
    pub(crate) preset: Option<StressPreset>,
    pub(crate) scenario: Scenario,
}

#[derive(Debug)]
struct SuiteResult {
    label: &'static str,
    probe: &'static str,
    metrics: sweep::RunMetrics,
    top_failure_bucket: Option<TopFailureBucket>,
    top_operation_group: Option<TopOperationGroup>,
}

#[derive(Debug, Clone, Serialize)]
struct TopFailureBucket {
    bucket: String,
    count: u64,
}

#[derive(Debug, Clone, Serialize)]
struct TopOperationGroup {
    name: String,
    count: u64,
    p95_us: u128,
}

pub(crate) async fn run(args: &Args, suite_run_id: &str) -> Result<(), String> {
    let suite = args
        .suite
        .ok_or_else(|| "suite mode is not enabled".to_string())?;
    let cases = build_cases(suite);
    let mut jsonl = match &args.output_jsonl {
        Some(path) => {
            Some(BufWriter::new(File::create(path).map_err(|error| {
                format!("create {}: {error}", path.display())
            })?))
        }
        None => None,
    };
    let mut records = Vec::with_capacity(cases.len());
    let mut results = Vec::with_capacity(cases.len());

    eprintln!(
        "{} starting suite={} suite_run_id={} cases={}",
        crate::log_prefix(args),
        suite.as_str(),
        suite_run_id,
        cases.len()
    );

    for case in cases {
        let run_id = format!("{suite_run_id}-{}", case.label);
        let mut case_args = args.clone();
        apply_case(args, &case, &mut case_args, &run_id);
        crate::validate_args(&case_args)?;

        eprintln!(
            "{} suite case label=\"{}\" probe={} backend={} scenario={}",
            crate::log_prefix(args),
            case.label,
            case.probe,
            case_args.backend.as_str(),
            case_args.scenario.as_str()
        );
        crate::trace::prepare_trace_outputs(&case_args).await?;
        let started = Instant::now();
        let captured = run_once(&case_args, &run_id).await?;
        let metrics = captured.metrics();
        let summary = captured.summary_value();
        let duration_ms = started.elapsed().as_millis();
        let top_failure_bucket = top_failure_bucket(&summary);
        let top_operation_group = top_operation_group(&summary);
        let record = json!({
            "suite_run_id": suite_run_id,
            "suite": suite,
            "run_id": run_id,
            "label": case.label,
            "probe": case.probe,
            "backend": case_args.backend,
            "preset": case.preset,
            "scenario": case_args.scenario,
            "processes": case_args.processes,
            "concurrency": case_args.concurrency,
            "postgres_pool_size": case_args.postgres_pool_size,
            "users": case_args.users,
            "active_thread_count": case_args.active_thread_count,
            "tenants": case_args.tenants,
            "operations_per_thread": case_args.operations,
            "duration_seconds": case_args.duration_seconds,
            "warmup_seconds": case_args.warmup_seconds,
            "trace_jsonl_enabled": case_args.trace_jsonl.is_some(),
            "trace_jsonl": case_args.trace_jsonl.as_ref().map(|path| path.display().to_string()),
            "trace_interval_seconds": case_args.trace_interval_seconds,
            "model_latency_ms": case_args.model_latency_ms,
            "model_latency_profile": case_args.model_latency_profile,
            "model_latency_jitter_ms": case_args.model_latency_jitter_ms,
            "model_latency_spike_every": case_args.model_latency_spike_every,
            "model_latency_spike_ms": case_args.model_latency_spike_ms,
            "user_message_bytes": case_args.user_message_bytes,
            "assistant_message_bytes": case_args.assistant_message_bytes,
            "context_max_messages": case_args.context_max_messages,
            "context_growth_turns_per_operation": case_args.context_growth_turns_per_operation,
            "tool_calls_per_turn": case_args.tool_calls_per_turn,
            "tool_latency_ms": case_args.tool_latency_ms,
            "tool_output_bytes": case_args.tool_output_bytes,
            "tool_failure_every": case_args.tool_failure_every,
            "duration_ms": duration_ms,
            "top_failure_bucket": top_failure_bucket,
            "top_operation_group": top_operation_group,
            "metrics": metrics,
            "summary": summary,
        });
        if let Some(writer) = jsonl.as_mut() {
            serde_json::to_writer(&mut *writer, &record).map_err(|error| error.to_string())?;
            writer.write_all(b"\n").map_err(|error| error.to_string())?;
        }
        records.push(record);
        results.push(SuiteResult {
            label: case.label,
            probe: case.probe,
            metrics,
            top_failure_bucket,
            top_operation_group,
        });
    }

    if let Some(mut writer) = jsonl {
        writer.flush().map_err(|error| error.to_string())?;
    }

    let suite_value = json!({
        "suite_run_id": suite_run_id,
        "suite": suite,
        "backend": args.backend,
        "runs": records,
    });
    let encoded = serde_json::to_string_pretty(&suite_value).map_err(|error| error.to_string())?;
    println!("{encoded}");

    if args.human_read {
        eprint!("{}", render_suite_summary(suite, &results));
    }
    if args.bottleneck_report {
        eprint!("{}", render_suite_bottleneck_report(&results));
    }
    if let Some(path) = &args.compare_json {
        eprint!("{}", compare::render_comparison_report(path, &suite_value)?);
    }

    let threshold_inputs = results
        .iter()
        .map(|result| (result.label.to_string(), result.metrics))
        .collect::<Vec<_>>();
    sweep::enforce_thresholds(args, &threshold_inputs)
}

pub(crate) fn build_cases(suite: StressSuite) -> Vec<SuiteCase> {
    match suite {
        StressSuite::BottleneckFinder => vec![
            SuiteCase {
                label: "resource-contention",
                probe: "resource-governor",
                preset: Some(StressPreset::ResourceContention),
                scenario: Scenario::ReserveReconcile,
            },
            SuiteCase {
                label: "chat-baseline",
                probe: "thread-store",
                preset: Some(StressPreset::ChatBaseline),
                scenario: Scenario::ChatTurn,
            },
            SuiteCase {
                label: "large-context",
                probe: "context-read-amplification",
                preset: Some(StressPreset::LargeContext),
                scenario: Scenario::MixedUserSession,
            },
            SuiteCase {
                label: "tool-heavy",
                probe: "tool-transcript-writes",
                preset: Some(StressPreset::ToolHeavy),
                scenario: Scenario::ToolSession,
            },
            SuiteCase {
                label: "tool-wait",
                probe: "tool-latency-ceiling",
                preset: Some(StressPreset::ToolHeavy),
                scenario: Scenario::ToolSession,
            },
            SuiteCase {
                label: "tool-failure",
                probe: "tool-failure-path",
                preset: Some(StressPreset::ToolHeavy),
                scenario: Scenario::ToolSession,
            },
            SuiteCase {
                label: "model-tail",
                probe: "model-tail-latency",
                preset: Some(StressPreset::ModelTail),
                scenario: Scenario::MixedUserSession,
            },
            SuiteCase {
                label: "cpu-burn",
                probe: "cpu-pressure",
                preset: Some(StressPreset::CpuBurn),
                scenario: Scenario::CpuBurn,
            },
            SuiteCase {
                label: "memory-churn",
                probe: "memory-pressure",
                preset: Some(StressPreset::MemoryChurn),
                scenario: Scenario::MemoryChurn,
            },
        ],
        StressSuite::PostgresPoolPressure => vec![
            SuiteCase {
                label: "postgres-chat-pool",
                probe: "postgres-thread-store-pool",
                preset: Some(StressPreset::ChatBaseline),
                scenario: Scenario::ChatTurn,
            },
            SuiteCase {
                label: "postgres-context-pool",
                probe: "postgres-context-read-pool",
                preset: Some(StressPreset::LargeContext),
                scenario: Scenario::MixedUserSession,
            },
            SuiteCase {
                label: "postgres-tool-pool",
                probe: "postgres-tool-write-pool",
                preset: Some(StressPreset::ToolHeavy),
                scenario: Scenario::ToolSession,
            },
        ],
    }
}

fn apply_case(base_args: &Args, case: &SuiteCase, case_args: &mut Args, run_id: &str) {
    case_args.run_id = Some(run_id.to_string());
    case_args.suite = None;
    case_args.preset = case.preset;
    case_args.scenario = case.scenario;
    case_args.suite_case_label = Some(case.label.to_string());
    case_args.repetitions = 1;
    case_args.ramp_concurrency = None;
    case_args.ramp_users = None;
    case_args.sweep_concurrency.clear();
    case_args.sweep_users.clear();
    case_args.sweep_active_thread_count.clear();
    case_args.sweep_model_latency_ms.clear();
    case_args.sweep_user_message_bytes.clear();
    case_args.sweep_assistant_message_bytes.clear();
    case_args.sweep_context_max_messages.clear();
    case_args.sweep_context_growth_turns_per_operation.clear();
    case_args.sweep_tool_calls_per_turn.clear();
    case_args.sweep_tool_output_bytes.clear();
    case_args.output_jsonl = None;
    case_args.compare_json = None;
    case_args.human_read = false;
    case_args.bottleneck_report = false;
    case_args.prefill_threads = 0;
    case_args.prefill_turns_per_thread = 0;

    if let Some(trace_jsonl) = &base_args.trace_jsonl {
        case_args.trace_jsonl = Some(crate::trace::labeled_trace_path(trace_jsonl, case.label));
    }

    match case.label {
        "large-context" => {
            case_args.prefill_threads = base_args.users.clamp(1, 100);
            case_args.prefill_turns_per_thread = base_args
                .prefill_turns_per_thread
                .max(base_args.operations.clamp(1, 50));
            case_args.prefill_concurrency = base_args.prefill_concurrency.max(1);
            case_args.context_max_messages = base_args.context_max_messages.max(100);
            case_args.user_message_bytes = base_args.user_message_bytes.max(512);
            case_args.assistant_message_bytes = base_args.assistant_message_bytes.max(1024);
        }
        "tool-heavy" => {
            case_args.tool_calls_per_turn = base_args.tool_calls_per_turn.max(8);
            case_args.tool_output_bytes = base_args.tool_output_bytes.max(4096);
            case_args.assistant_message_bytes = base_args.assistant_message_bytes.max(1024);
        }
        "tool-wait" => {
            case_args.tool_calls_per_turn = base_args.tool_calls_per_turn.max(4);
            case_args.tool_latency_ms = base_args.tool_latency_ms.max(250);
            case_args.tool_output_bytes = base_args.tool_output_bytes.max(1024);
        }
        "tool-failure" => {
            case_args.tool_calls_per_turn = base_args.tool_calls_per_turn.max(4);
            case_args.tool_failure_every = base_args.tool_failure_every.max(3);
            case_args.span_log_failures = true;
        }
        "model-tail" => {
            case_args.model_latency_ms = base_args.model_latency_ms.max(100);
            case_args.model_latency_profile = ModelLatencyProfile::TailSpike;
            case_args.model_latency_spike_every = base_args.model_latency_spike_every.max(10);
            case_args.model_latency_spike_ms = base_args.model_latency_spike_ms.max(2000);
        }
        "postgres-chat-pool" => {
            apply_postgres_pool_pressure_defaults(base_args, case_args);
        }
        "postgres-context-pool" => {
            apply_postgres_pool_pressure_defaults(base_args, case_args);
            case_args.prefill_threads = base_args.users.clamp(1, 100);
            case_args.prefill_turns_per_thread = base_args
                .prefill_turns_per_thread
                .max(base_args.operations.clamp(1, 50));
            case_args.prefill_concurrency = base_args.prefill_concurrency.max(1);
            case_args.context_max_messages = base_args.context_max_messages.max(100);
        }
        "postgres-tool-pool" => {
            apply_postgres_pool_pressure_defaults(base_args, case_args);
            case_args.tool_calls_per_turn = base_args.tool_calls_per_turn.max(8);
            case_args.tool_output_bytes = base_args.tool_output_bytes.max(4096);
            case_args.assistant_message_bytes = base_args.assistant_message_bytes.max(1024);
        }
        _ => {}
    }
}

fn apply_postgres_pool_pressure_defaults(base_args: &Args, case_args: &mut Args) {
    case_args.concurrency = base_args.concurrency.max(8);
    case_args.users = base_args.users.max(100);
    case_args.postgres_pool_size = base_args.postgres_pool_size.clamp(1, 4);
}

fn top_failure_bucket(summary: &Value) -> Option<TopFailureBucket> {
    let errors = summary.get("errors")?.as_object()?;
    errors
        .iter()
        .filter_map(|(bucket, value)| value.as_u64().map(|count| (bucket, count)))
        .filter(|(_, count)| *count > 0)
        .max_by_key(|(_, count)| *count)
        .map(|(bucket, count)| TopFailureBucket {
            bucket: bucket.to_string(),
            count,
        })
}

fn top_operation_group(summary: &Value) -> Option<TopOperationGroup> {
    let groups = summary.get("operation_attribution")?.as_object()?;
    groups
        .iter()
        .filter_map(|(name, value)| {
            let count = value.get("count")?.as_u64()?;
            let p95 = value.pointer("/latency/p95_us")?.as_u64()?;
            (count > 0).then_some((name, count, p95))
        })
        .max_by_key(|(_, _, p95)| *p95)
        .map(|(name, count, p95)| TopOperationGroup {
            name: name.to_string(),
            count,
            p95_us: u128::from(p95),
        })
}

fn render_suite_summary(suite: StressSuite, results: &[SuiteResult]) -> String {
    let mut output = String::new();
    let _ = writeln!(output, "\nSuite summary ({})", suite.as_str());
    let _ = writeln!(
        output,
        "{:<24} {:<24} {:>9} {:>8} {:>10} {:>10} {:>10} {:>10} {:>10} {:<22} {:<22}",
        "case",
        "probe",
        "attempts",
        "fail%",
        "ops/sec",
        "p95",
        "p99",
        "cpu",
        "rss",
        "top_failure",
        "top_group"
    );
    let _ = writeln!(
        output,
        "{:-<24} {:-<24} {:->9} {:->8} {:->10} {:->10} {:->10} {:->10} {:->10} {:-<22} {:-<22}",
        "", "", "", "", "", "", "", "", "", "", ""
    );
    for result in results {
        let _ = writeln!(
            output,
            "{:<24} {:<24} {:>9} {:>7.2}% {:>10.2} {:>10} {:>10} {:>10} {:>10} {:<22} {:<22}",
            truncate(result.label, 24),
            truncate(result.probe, 24),
            result.metrics.attempted,
            failure_rate(result.metrics) * 100.0,
            result.metrics.throughput_ops_sec,
            format_latency_us(result.metrics.p95_us),
            format_latency_us(result.metrics.p99_us),
            format_optional_ms(result.metrics.cpu_ms),
            format_optional_kb(result.metrics.peak_rss_kb),
            format_top_failure(result.top_failure_bucket.as_ref()),
            format_top_group(result.top_operation_group.as_ref()),
        );
    }
    output
}

fn render_suite_bottleneck_report(results: &[SuiteResult]) -> String {
    let mut output = String::new();
    let _ = writeln!(output, "\nSuite bottleneck analysis");
    let _ = writeln!(output, "{:<22} {:<20} evidence", "signal", "case");
    let _ = writeln!(output, "{:-<22} {:-<20} {:-<56}", "", "", "");
    if results.is_empty() {
        let _ = writeln!(output, "{:<22} {:<20} no suite cases", "none", "-");
        return output;
    }

    if let Some(result) = results.iter().max_by(|left, right| {
        failure_rate(left.metrics)
            .partial_cmp(&failure_rate(right.metrics))
            .unwrap_or(std::cmp::Ordering::Equal)
    }) && result.metrics.failed > 0
    {
        let _ = writeln!(
            output,
            "{:<22} {:<20} failed={} attempted={} fail_rate={:.2}%",
            "failure_ceiling",
            truncate(result.label, 20),
            result.metrics.failed,
            result.metrics.attempted,
            failure_rate(result.metrics) * 100.0
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
            "{:<22} {:<20} throughput={:.2} ops/sec",
            "lowest_throughput",
            truncate(result.label, 20),
            result.metrics.throughput_ops_sec
        );
    }

    if let Some(result) = results.iter().max_by_key(|result| result.metrics.p95_us) {
        let _ = writeln!(
            output,
            "{:<22} {:<20} p95={} p99={} top_group={}",
            "highest_latency",
            truncate(result.label, 20),
            format_latency_us(result.metrics.p95_us),
            format_latency_us(result.metrics.p99_us),
            format_top_group(result.top_operation_group.as_ref())
        );
    }

    for result in results {
        if let Some(failure) = &result.top_failure_bucket {
            let _ = writeln!(
                output,
                "{:<22} {:<20} top_failure={} count={} failed={}",
                "case_failure",
                truncate(result.label, 20),
                truncate(&failure.bucket, 28),
                failure.count,
                result.metrics.failed
            );
        }
        if let Some(group) = &result.top_operation_group {
            let _ = writeln!(
                output,
                "{:<22} {:<20} top_group={} p95={} count={}",
                "case_operation_group",
                truncate(result.label, 20),
                truncate(&group.name, 28),
                format_latency_us(group.p95_us),
                group.count
            );
        }
    }

    if let Some(result) = results
        .iter()
        .filter(|result| result.metrics.cpu_ms.is_some())
        .max_by_key(|result| result.metrics.cpu_ms)
    {
        let _ = writeln!(
            output,
            "{:<22} {:<20} cpu={}",
            "highest_cpu",
            truncate(result.label, 20),
            format_optional_ms(result.metrics.cpu_ms)
        );
    }

    if let Some(result) = results
        .iter()
        .filter(|result| result.metrics.peak_rss_kb.is_some())
        .max_by_key(|result| result.metrics.peak_rss_kb)
    {
        let _ = writeln!(
            output,
            "{:<22} {:<20} peak_rss={}",
            "highest_rss",
            truncate(result.label, 20),
            format_optional_kb(result.metrics.peak_rss_kb)
        );
    }

    let _ = writeln!(output, "\nNext probes");
    let _ = writeln!(
        output,
        "- Rerun the worst case with --trace-jsonl, --human-read, and --bottleneck-report for stage-level attribution."
    );
    let _ = writeln!(
        output,
        "- Use --suite as a broad scan, then switch to --preset, --sweep-*, or --ramp-* on the failing case."
    );
    output
}

fn format_top_failure(failure: Option<&TopFailureBucket>) -> String {
    failure
        .map(|failure| format!("{}:{}", truncate(&failure.bucket, 16), failure.count))
        .unwrap_or_else(|| "-".to_string())
}

fn format_top_group(group: Option<&TopOperationGroup>) -> String {
    group
        .map(|group| {
            format!(
                "{}@{}",
                truncate(&group.name, 14),
                format_latency_us(group.p95_us)
            )
        })
        .unwrap_or_else(|| "-".to_string())
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

fn truncate(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut truncated = value
        .chars()
        .take(max_chars.saturating_sub(3))
        .collect::<String>();
    truncated.push_str("...");
    truncated
}
