use std::{
    fmt::Write as FmtWrite,
    fs::File,
    io::{BufWriter, Write as IoWrite},
    time::Instant,
};

use serde::Serialize;
use serde_json::json;

use crate::{Args, compare, run_once};

#[derive(Debug, Clone, Copy, Serialize)]
pub(crate) struct RunMetrics {
    pub(crate) attempted: u64,
    pub(crate) failed: u64,
    pub(crate) throughput_ops_sec: f64,
    pub(crate) cpu_ms: Option<u128>,
    pub(crate) peak_rss_kb: Option<u64>,
    pub(crate) p95_us: u128,
    pub(crate) p99_us: u128,
    pub(crate) max_us: u128,
}

impl RunMetrics {
    fn failure_rate(self) -> f64 {
        if self.attempted == 0 {
            0.0
        } else {
            self.failed as f64 / self.attempted as f64
        }
    }
}

#[derive(Debug)]
struct SweepCase {
    repetition: usize,
    concurrency: usize,
    users: usize,
    active_thread_count: usize,
    model_latency_ms: u64,
    user_message_bytes: usize,
    assistant_message_bytes: usize,
    context_max_messages: usize,
    context_growth_turns_per_operation: usize,
    tool_calls_per_turn: usize,
    tool_output_bytes: usize,
}

#[derive(Debug)]
struct SweepResult {
    label: String,
    run_id: String,
    case: SweepCase,
    metrics: RunMetrics,
}

pub(crate) fn is_enabled(args: &Args) -> bool {
    !args.sweep_concurrency.is_empty()
        || !args.sweep_users.is_empty()
        || !args.sweep_active_thread_count.is_empty()
        || !args.sweep_model_latency_ms.is_empty()
        || !args.sweep_user_message_bytes.is_empty()
        || !args.sweep_assistant_message_bytes.is_empty()
        || !args.sweep_context_max_messages.is_empty()
        || !args.sweep_context_growth_turns_per_operation.is_empty()
        || !args.sweep_tool_calls_per_turn.is_empty()
        || !args.sweep_tool_output_bytes.is_empty()
        || args.repetitions > 1
        || args.output_jsonl.is_some()
}

pub(crate) async fn run(args: &Args, suite_run_id: &str) -> Result<(), String> {
    let cases = build_cases(args);
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
        "{} starting sweep suite_run_id={} points={} repetitions={}",
        crate::log_prefix(args),
        suite_run_id,
        cases.len(),
        args.repetitions
    );

    for case in cases {
        let run_id = format!(
            "{suite_run_id}-r{}-c{}-u{}-at{}-m{}-ub{}-ab{}-ctx{}-cg{}-tc{}-tb{}",
            case.repetition,
            case.concurrency,
            case.users,
            case.active_thread_count,
            case.model_latency_ms,
            case.user_message_bytes,
            case.assistant_message_bytes,
            case.context_max_messages,
            case.context_growth_turns_per_operation,
            case.tool_calls_per_turn,
            case.tool_output_bytes
        );
        let label = format!(
            "r{} c{} u{} at{} m{} ub{} ab{} ctx{} cg{} tc{} tb{}",
            case.repetition,
            case.concurrency,
            case.users,
            case.active_thread_count,
            case.model_latency_ms,
            case.user_message_bytes,
            case.assistant_message_bytes,
            case.context_max_messages,
            case.context_growth_turns_per_operation,
            case.tool_calls_per_turn,
            case.tool_output_bytes
        );
        let mut case_args = args.clone();
        case_args.concurrency = case.concurrency;
        case_args.users = case.users;
        case_args.active_thread_count = case.active_thread_count;
        case_args.model_latency_ms = case.model_latency_ms;
        case_args.user_message_bytes = case.user_message_bytes;
        case_args.assistant_message_bytes = case.assistant_message_bytes;
        case_args.context_max_messages = case.context_max_messages;
        case_args.context_growth_turns_per_operation = case.context_growth_turns_per_operation;
        case_args.tool_calls_per_turn = case.tool_calls_per_turn;
        case_args.tool_output_bytes = case.tool_output_bytes;
        case_args.run_id = Some(run_id.clone());
        case_args.repetitions = 1;
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
        if let Some(trace_jsonl) = &args.trace_jsonl {
            let trace_label = format!(
                "sweep-r{}-c{}-u{}-at{}-m{}-ub{}-ab{}-ctx{}-cg{}-tc{}-tb{}",
                case.repetition,
                case.concurrency,
                case.users,
                case.active_thread_count,
                case.model_latency_ms,
                case.user_message_bytes,
                case.assistant_message_bytes,
                case.context_max_messages,
                case.context_growth_turns_per_operation,
                case.tool_calls_per_turn,
                case.tool_output_bytes
            );
            case_args.trace_jsonl =
                Some(crate::trace::labeled_trace_path(trace_jsonl, &trace_label));
        }

        eprintln!(
            "{} sweep point label=\"{}\" backend={} scenario={}",
            crate::log_prefix(args),
            label,
            case_args.backend.as_str(),
            case_args.scenario.as_str()
        );
        crate::trace::prepare_trace_outputs(&case_args).await?;
        let started = Instant::now();
        let captured = run_once(&case_args, &run_id).await?;
        let metrics = captured.metrics();
        let summary = captured.summary_value();
        let duration_ms = started.elapsed().as_millis();
        let record = json!({
            "suite_run_id": suite_run_id,
            "run_id": run_id,
            "label": label,
            "backend": case_args.backend,
            "preset": case_args.preset,
            "scenario": case_args.scenario,
            "processes": case_args.processes,
            "concurrency": case.concurrency,
            "users": case.users,
            "active_thread_count": case.active_thread_count,
            "threads_per_owner": case_args.threads_per_owner,
            "turn_state_backend": case_args.turn_state_backend,
            "gate_blocked_every": case_args.gate_blocked_every,
            "tenants": case_args.tenants,
            "operations_per_thread": case_args.operations,
            "duration_seconds": case_args.duration_seconds,
            "warmup_seconds": case_args.warmup_seconds,
            "trace_jsonl_enabled": case_args.trace_jsonl.is_some(),
            "trace_jsonl": case_args.trace_jsonl.as_ref().map(|path| path.display().to_string()),
            "trace_interval_seconds": case_args.trace_interval_seconds,
            "model_latency_ms": case.model_latency_ms,
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
            "repetition": case.repetition,
            "duration_ms": duration_ms,
            "metrics": metrics,
            "summary": summary,
        });
        if let Some(writer) = jsonl.as_mut() {
            serde_json::to_writer(&mut *writer, &record).map_err(|error| error.to_string())?;
            writer.write_all(b"\n").map_err(|error| error.to_string())?;
        }
        records.push(record);
        results.push(SweepResult {
            label,
            run_id,
            case,
            metrics,
        });
    }

    if let Some(mut writer) = jsonl {
        writer.flush().map_err(|error| error.to_string())?;
    }

    let suite = json!({
        "suite_run_id": suite_run_id,
        "runs": records,
    });
    let encoded = serde_json::to_string_pretty(&suite).map_err(|error| error.to_string())?;
    println!("{encoded}");

    if args.human_read {
        eprint!("{}", render_sweep_summary(&results));
    }
    if args.bottleneck_report {
        eprint!("{}", render_sweep_bottleneck_report(&results));
    }
    if let Some(path) = &args.compare_json {
        eprint!("{}", compare::render_comparison_report(path, &suite)?);
    }

    let threshold_inputs = results
        .iter()
        .map(|result| (result.label.clone(), result.metrics))
        .collect::<Vec<_>>();
    enforce_thresholds(args, &threshold_inputs)
}

pub(crate) fn enforce_thresholds(args: &Args, runs: &[(String, RunMetrics)]) -> Result<(), String> {
    let mut violations = Vec::new();
    for (label, metrics) in runs {
        if let Some(max_failure_rate) = args.max_failure_rate {
            let failure_rate = metrics.failure_rate();
            if failure_rate > max_failure_rate {
                violations.push(format!(
                    "{label}: failure_rate {:.4} > {:.4}",
                    failure_rate, max_failure_rate
                ));
            }
        }
        if let Some(max_p95_ms) = args.max_p95_ms
            && metrics.p95_us > u128::from(max_p95_ms) * 1_000
        {
            violations.push(format!(
                "{label}: p95 {} > {max_p95_ms}ms",
                format_latency_us(metrics.p95_us),
            ));
        }
        if let Some(min_throughput) = args.min_throughput
            && metrics.throughput_ops_sec < min_throughput
        {
            violations.push(format!(
                "{label}: throughput {:.2} < {:.2}",
                metrics.throughput_ops_sec, min_throughput
            ));
        }
        if let Some(max_rss_mb) = args.max_rss_mb
            && let Some(peak_rss_kb) = metrics.peak_rss_kb
            && peak_rss_kb > max_rss_mb.saturating_mul(1024)
        {
            violations.push(format!(
                "{label}: peak_rss {} > {max_rss_mb}MB",
                format_kb(peak_rss_kb)
            ));
        }
        if let Some(max_cpu_ms) = args.max_cpu_ms
            && let Some(cpu_ms) = metrics.cpu_ms
            && cpu_ms > max_cpu_ms
        {
            violations.push(format!(
                "{label}: cpu {} > {}",
                format_duration_ms(cpu_ms),
                format_duration_ms(max_cpu_ms)
            ));
        }
    }
    if violations.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "stress threshold violation(s): {}",
            violations.join("; ")
        ))
    }
}

fn build_cases(args: &Args) -> Vec<SweepCase> {
    let concurrency_values = if args.sweep_concurrency.is_empty() {
        vec![args.concurrency]
    } else {
        args.sweep_concurrency.clone()
    };
    let user_values = if args.sweep_users.is_empty() {
        vec![args.users]
    } else {
        args.sweep_users.clone()
    };
    let active_thread_count_values = if args.sweep_active_thread_count.is_empty() {
        vec![args.active_thread_count]
    } else {
        args.sweep_active_thread_count.clone()
    };
    let model_latency_values = if args.sweep_model_latency_ms.is_empty() {
        vec![args.model_latency_ms]
    } else {
        args.sweep_model_latency_ms.clone()
    };
    let user_message_byte_values = if args.sweep_user_message_bytes.is_empty() {
        vec![args.user_message_bytes]
    } else {
        args.sweep_user_message_bytes.clone()
    };
    let assistant_message_byte_values = if args.sweep_assistant_message_bytes.is_empty() {
        vec![args.assistant_message_bytes]
    } else {
        args.sweep_assistant_message_bytes.clone()
    };
    let context_max_message_values = if args.sweep_context_max_messages.is_empty() {
        vec![args.context_max_messages]
    } else {
        args.sweep_context_max_messages.clone()
    };
    let context_growth_turn_values = if args.sweep_context_growth_turns_per_operation.is_empty() {
        vec![args.context_growth_turns_per_operation]
    } else {
        args.sweep_context_growth_turns_per_operation.clone()
    };
    let tool_calls_per_turn_values = if args.sweep_tool_calls_per_turn.is_empty() {
        vec![args.tool_calls_per_turn]
    } else {
        args.sweep_tool_calls_per_turn.clone()
    };
    let tool_output_byte_values = if args.sweep_tool_output_bytes.is_empty() {
        vec![args.tool_output_bytes]
    } else {
        args.sweep_tool_output_bytes.clone()
    };

    let mut cases = Vec::new();
    for repetition in 1..=args.repetitions {
        for concurrency in &concurrency_values {
            for users in &user_values {
                for active_thread_count in &active_thread_count_values {
                    for model_latency_ms in &model_latency_values {
                        for user_message_bytes in &user_message_byte_values {
                            for assistant_message_bytes in &assistant_message_byte_values {
                                for context_max_messages in &context_max_message_values {
                                    for context_growth_turns_per_operation in
                                        &context_growth_turn_values
                                    {
                                        for tool_calls_per_turn in &tool_calls_per_turn_values {
                                            for tool_output_bytes in &tool_output_byte_values {
                                                cases.push(SweepCase {
                                                    repetition,
                                                    concurrency: *concurrency,
                                                    users: *users,
                                                    active_thread_count: *active_thread_count,
                                                    model_latency_ms: *model_latency_ms,
                                                    user_message_bytes: *user_message_bytes,
                                                    assistant_message_bytes:
                                                        *assistant_message_bytes,
                                                    context_max_messages: *context_max_messages,
                                                    context_growth_turns_per_operation:
                                                        *context_growth_turns_per_operation,
                                                    tool_calls_per_turn: *tool_calls_per_turn,
                                                    tool_output_bytes: *tool_output_bytes,
                                                });
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    cases
}

fn render_sweep_summary(results: &[SweepResult]) -> String {
    let mut output = String::new();
    let _ = writeln!(output, "\nSweep summary");
    let _ = writeln!(
        output,
        "{:<18} {:<8} {:>5} {:>6} {:>5} {:>8} {:>6} {:>6} {:>5} {:>5} {:>5} {:>6} {:>9} {:>8} {:>10} {:>10} {:>10} {:>10} {:>10} {:>10}",
        "label",
        "run",
        "conc",
        "users",
        "thr",
        "model",
        "uB",
        "aB",
        "ctx",
        "cg",
        "tc",
        "outB",
        "attempted",
        "fail%",
        "ops/sec",
        "p95",
        "p99",
        "max",
        "cpu",
        "rss"
    );
    let _ = writeln!(
        output,
        "{:-<18} {:-<8} {:->5} {:->6} {:->5} {:->8} {:->6} {:->6} {:->5} {:->5} {:->5} {:->6} {:->9} {:->8} {:->10} {:->10} {:->10} {:->10} {:->10} {:->10}",
        "", "", "", "", "", "", "", "", "", "", "", "", "", "", "", "", "", "", "", ""
    );
    for result in results {
        let short_run_id = result.run_id.chars().take(8).collect::<String>();
        let _ = writeln!(
            output,
            "{:<18} {:<8} {:>5} {:>6} {:>5} {:>8} {:>6} {:>6} {:>5} {:>5} {:>5} {:>6} {:>9} {:>7.2}% {:>10.2} {:>10} {:>10} {:>10} {:>10} {:>10}",
            truncate(&result.label, 18),
            short_run_id,
            result.case.concurrency,
            result.case.users,
            format_active_thread_count(result.case.active_thread_count, result.case.users),
            result.case.model_latency_ms,
            result.case.user_message_bytes,
            result.case.assistant_message_bytes,
            result.case.context_max_messages,
            result.case.context_growth_turns_per_operation,
            result.case.tool_calls_per_turn,
            result.case.tool_output_bytes,
            result.metrics.attempted,
            result.metrics.failure_rate() * 100.0,
            result.metrics.throughput_ops_sec,
            format_latency_us(result.metrics.p95_us),
            format_latency_us(result.metrics.p99_us),
            format_latency_us(result.metrics.max_us),
            format_optional_ms(result.metrics.cpu_ms),
            format_optional_kb(result.metrics.peak_rss_kb),
        );
    }
    output
}

fn render_sweep_bottleneck_report(results: &[SweepResult]) -> String {
    let mut output = String::new();
    let _ = writeln!(output, "\nSweep bottleneck analysis");
    let _ = writeln!(output, "{:<22} {:<18} evidence", "signal", "point");
    let _ = writeln!(output, "{:-<22} {:-<18} {:-<56}", "", "", "");
    if results.is_empty() {
        let _ = writeln!(output, "{:<22} {:<18} no sweep points", "none", "-");
        return output;
    }

    if let Some(result) = results.iter().max_by(|left, right| {
        left.metrics
            .failure_rate()
            .partial_cmp(&right.metrics.failure_rate())
            .unwrap_or(std::cmp::Ordering::Equal)
    }) && result.metrics.failed > 0
    {
        let _ = writeln!(
            output,
            "{:<22} {:<18} failed={} attempted={} fail_rate={:.2}%",
            "failure_ceiling",
            truncate(&result.label, 18),
            result.metrics.failed,
            result.metrics.attempted,
            result.metrics.failure_rate() * 100.0
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
            "{:<22} {:<18} throughput={:.2} ops/sec",
            "lowest_throughput",
            truncate(&result.label, 18),
            result.metrics.throughput_ops_sec
        );
    }

    if let Some(result) = results.iter().max_by_key(|result| result.metrics.p95_us) {
        let _ = writeln!(
            output,
            "{:<22} {:<18} p95={} p99={}",
            "highest_latency",
            truncate(&result.label, 18),
            format_latency_us(result.metrics.p95_us),
            format_latency_us(result.metrics.p99_us)
        );
    }

    if let Some(result) = results
        .iter()
        .filter(|result| result.metrics.cpu_ms.is_some())
        .max_by_key(|result| result.metrics.cpu_ms)
    {
        let _ = writeln!(
            output,
            "{:<22} {:<18} cpu={}",
            "highest_cpu",
            truncate(&result.label, 18),
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
            "{:<22} {:<18} peak_rss={}",
            "highest_rss",
            truncate(&result.label, 18),
            format_optional_kb(result.metrics.peak_rss_kb)
        );
    }

    let _ = writeln!(output, "\nNext probes");
    let _ = writeln!(
        output,
        "- Rerun the worst latency/throughput point with --trace-jsonl and --bottleneck-report to capture interval collapse."
    );
    let _ = writeln!(
        output,
        "- Add one dimension at a time to distinguish user fanout, concurrency, model latency, and storage growth."
    );
    output
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

fn format_active_thread_count(active_thread_count: usize, _users: usize) -> String {
    if active_thread_count == 0 {
        "all".to_string()
    } else {
        active_thread_count.to_string()
    }
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
