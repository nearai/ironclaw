use std::{collections::BTreeMap, fmt::Write};

use crate::{
    Args, RunSummary,
    process_metrics::{ProcessMetrics, aggregate_process_metrics},
    summary::{FailureCauseSummary, LatencySummary},
    user_turn::UserTurnStageLatencySummary,
};

pub(crate) fn render_run_summary(summary: &RunSummary) -> String {
    let mut output = String::new();
    push_overview(
        &mut output,
        "Run summary",
        &[
            ("backend", summary.backend.as_str().to_string()),
            ("scenario", summary.scenario.as_str().to_string()),
            ("run_id", summary.run_id.clone()),
            ("target", summary.target.clone()),
            ("processes", summary.processes.to_string()),
            ("concurrency", summary.concurrency.to_string()),
            (
                "operations_per_thread",
                summary.operations_per_thread.to_string(),
            ),
            ("users", summary.users.to_string()),
            ("tenants", summary.tenants.to_string()),
            ("attempted", summary.attempted.to_string()),
            ("succeeded", summary.succeeded.to_string()),
            ("failed", summary.failed.to_string()),
            ("duration", format_duration_ms(summary.duration_ms)),
            (
                "throughput_ops_sec",
                format!("{:.2}", summary.throughput_ops_sec),
            ),
        ],
    );
    push_latency_table(
        &mut output,
        "Operation latency",
        &[("operation", summary.attempted, &summary.latency)],
    );
    push_process_table(&mut output, &summary.process);
    if let Some(stages) = &summary.stage_latency {
        push_stage_latency_table(&mut output, stages);
    }
    push_errors_table(&mut output, &summary.errors);
    push_failure_causes_table(&mut output, &summary.failure_causes);
    output
}

pub(crate) fn render_parent_summary(args: &Args, run_id: &str, summaries: &[RunSummary]) -> String {
    let attempted: u64 = summaries.iter().map(|summary| summary.attempted).sum();
    let succeeded: u64 = summaries.iter().map(|summary| summary.succeeded).sum();
    let failed: u64 = summaries.iter().map(|summary| summary.failed).sum();
    let max_duration_ms = summaries
        .iter()
        .map(|summary| summary.duration_ms)
        .max()
        .unwrap_or(0);
    let throughput_ops_sec = if max_duration_ms == 0 {
        0.0
    } else {
        attempted as f64 / (max_duration_ms as f64 / 1000.0)
    };
    let target = summaries
        .first()
        .map(|summary| summary.target.as_str())
        .unwrap_or("unknown");
    let errors = aggregate_errors(summaries);
    let failure_causes = aggregate_failure_causes(summaries);
    let process = aggregate_process_metrics(summaries.iter().map(|summary| &summary.process));

    let mut output = String::new();
    push_overview(
        &mut output,
        "Run summary",
        &[
            ("backend", args.backend.as_str().to_string()),
            ("scenario", args.scenario.as_str().to_string()),
            ("run_id", run_id.to_string()),
            ("target", target.to_string()),
            ("processes", args.processes.to_string()),
            ("concurrency_per_process", args.concurrency.to_string()),
            ("attempted", attempted.to_string()),
            ("succeeded", succeeded.to_string()),
            ("failed", failed.to_string()),
            ("max_duration", format_duration_ms(max_duration_ms)),
            ("throughput_ops_sec", format!("{throughput_ops_sec:.2}")),
        ],
    );
    push_process_table(&mut output, &process);
    push_child_table(&mut output, summaries);
    push_errors_table(&mut output, &errors);
    push_failure_causes_table(&mut output, &failure_causes);
    output
}

fn push_overview(output: &mut String, title: &str, rows: &[(&str, String)]) {
    let _ = writeln!(output, "\n{title}");
    let _ = writeln!(output, "{:<24} value", "field");
    let _ = writeln!(output, "{:-<24} {:-<32}", "", "");
    for (field, value) in rows {
        let _ = writeln!(output, "{field:<24} {value}");
    }
}

fn push_latency_table(output: &mut String, title: &str, rows: &[(&str, u64, &LatencySummary)]) {
    let _ = writeln!(output, "\n{title}");
    let _ = writeln!(
        output,
        "{:<24} {:>8} {:>10} {:>10} {:>10} {:>10}",
        "name", "count", "p50", "p95", "p99", "max"
    );
    let _ = writeln!(
        output,
        "{:-<24} {:->8} {:->10} {:->10} {:->10} {:->10}",
        "", "", "", "", "", ""
    );
    for (name, count, latency) in rows {
        let _ = writeln!(
            output,
            "{name:<24} {count:>8} {:>10} {:>10} {:>10} {:>10}",
            format_latency_us(latency.p50_us),
            format_latency_us(latency.p95_us),
            format_latency_us(latency.p99_us),
            format_latency_us(latency.max_us),
        );
    }
}

fn push_stage_latency_table(output: &mut String, stages: &UserTurnStageLatencySummary) {
    let rows = [
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
        ("resource_reconcile", &stages.resource_reconcile),
        ("resource_release", &stages.resource_release),
    ];
    let rows: Vec<(&str, u64, &LatencySummary)> = rows
        .into_iter()
        .filter(|(_, stage)| stage.count > 0)
        .map(|(name, stage)| (name, stage.count, &stage.latency))
        .collect();
    if !rows.is_empty() {
        push_latency_table(output, "Stage latency", &rows);
    }
}

fn push_process_table(output: &mut String, process: &ProcessMetrics) {
    let _ = writeln!(output, "\nProcess metrics");
    let _ = writeln!(output, "{:<24} {:>12}", "metric", "value");
    let _ = writeln!(output, "{:-<24} {:->12}", "", "");
    push_metric(
        output,
        "cpu_total",
        format_optional_ms(process.delta_cpu_ms),
    );
    push_metric(
        output,
        "cpu_user",
        format_optional_ms(process.delta_user_cpu_ms),
    );
    push_metric(
        output,
        "cpu_system",
        format_optional_ms(process.delta_system_cpu_ms),
    );
    push_metric(output, "peak_rss", format_optional_kb(process.peak_rss_kb));
    push_metric(
        output,
        "peak_threads",
        format_optional(process.peak_threads),
    );
    push_metric(
        output,
        "peak_open_fds",
        format_optional(process.peak_open_fds),
    );
}

fn push_metric(output: &mut String, name: &str, value: String) {
    let _ = writeln!(output, "{name:<24} {value:>12}");
}

fn push_child_table(output: &mut String, summaries: &[RunSummary]) {
    if summaries.len() < 2 {
        return;
    }
    let _ = writeln!(output, "\nChild summaries");
    let _ = writeln!(
        output,
        "{:<8} {:>9} {:>9} {:>9} {:>10} {:>10} {:>10} {:>10}",
        "child", "attempted", "succeeded", "failed", "duration", "p95", "p99", "max"
    );
    let _ = writeln!(
        output,
        "{:-<8} {:->9} {:->9} {:->9} {:->10} {:->10} {:->10} {:->10}",
        "", "", "", "", "", "", "", ""
    );
    for summary in summaries {
        let child = summary
            .child_index
            .map(|index| index.to_string())
            .unwrap_or_else(|| "-".to_string());
        let _ = writeln!(
            output,
            "{child:<8} {:>9} {:>9} {:>9} {:>10} {:>10} {:>10} {:>10}",
            summary.attempted,
            summary.succeeded,
            summary.failed,
            format_duration_ms(summary.duration_ms),
            format_latency_us(summary.latency.p95_us),
            format_latency_us(summary.latency.p99_us),
            format_latency_us(summary.latency.max_us),
        );
    }
}

fn push_errors_table(output: &mut String, errors: &BTreeMap<String, u64>) {
    if errors.is_empty() {
        return;
    }
    let _ = writeln!(output, "\nErrors");
    let _ = writeln!(output, "{:<36} {:>8}", "bucket", "count");
    let _ = writeln!(output, "{:-<36} {:->8}", "", "");
    for (bucket, count) in errors {
        let _ = writeln!(output, "{:<36} {:>8}", truncate(bucket, 36), count);
    }
}

fn push_failure_causes_table(
    output: &mut String,
    failure_causes: &BTreeMap<String, FailureCauseSummary>,
) {
    if failure_causes.is_empty() {
        return;
    }
    let _ = writeln!(output, "\nFailure causes");
    let _ = writeln!(
        output,
        "{:<32} {:>8} {:<36} sample_detail",
        "bucket", "count", "stages"
    );
    let _ = writeln!(output, "{:-<32} {:->8} {:-<36} {:-<32}", "", "", "", "");
    for (bucket, cause) in failure_causes {
        let stages = cause
            .stages
            .iter()
            .map(|(stage, count)| format!("{stage}:{count}"))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(
            output,
            "{:<32} {:>8} {:<36} {}",
            truncate(bucket, 32),
            cause.count,
            truncate(&stages, 36),
            truncate(&cause.sample_detail, 72),
        );
    }
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

fn format_duration_ms(ms: u128) -> String {
    if ms >= 1_000 {
        format!("{:.2}s", ms as f64 / 1_000.0)
    } else {
        format!("{ms}ms")
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

fn format_optional<T: std::fmt::Display>(value: Option<T>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_string())
}

fn format_optional_ms(value: Option<u128>) -> String {
    value
        .map(format_duration_ms)
        .unwrap_or_else(|| "-".to_string())
}

fn format_optional_kb(value: Option<u64>) -> String {
    match value {
        Some(value) if value >= 1024 * 1024 => format!("{:.2}GB", value as f64 / 1024.0 / 1024.0),
        Some(value) if value >= 1024 => format!("{:.1}MB", value as f64 / 1024.0),
        Some(value) => format!("{value}KB"),
        None => "-".to_string(),
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
