use std::{collections::BTreeMap, fmt::Write};

use crate::{
    Args, RunSummary,
    db_probe::{DbProbeSnapshot, DbProbeSummary},
    process_metrics::{ProcessMetrics, aggregate_process_metrics},
    summary::{FailureCauseSummary, LatencySummary},
    user_turn::{
        UserTurnOperationAttributionSummary, UserTurnStageLatencySummary,
        operation_attribution_rows,
    },
};

pub(crate) fn render_run_summary(summary: &RunSummary) -> String {
    let mut output = String::new();
    push_overview(
        &mut output,
        "Run summary",
        &[
            ("backend", summary.backend.as_str().to_string()),
            ("preset", format_preset(summary.preset)),
            ("scenario", summary.scenario.as_str().to_string()),
            ("run_id", summary.run_id.clone()),
            ("target", summary.target.clone()),
            ("processes", summary.processes.to_string()),
            ("concurrency", summary.concurrency.to_string()),
            (
                "operations_per_thread",
                summary.operations_per_thread.to_string(),
            ),
            ("duration_seconds", summary.duration_seconds.to_string()),
            ("warmup_seconds", summary.warmup_seconds.to_string()),
            (
                "trace_jsonl_enabled",
                summary.trace_jsonl_enabled.to_string(),
            ),
            (
                "trace_interval_seconds",
                summary.trace_interval_seconds.to_string(),
            ),
            ("users", summary.users.to_string()),
            (
                "turn_state_max_terminal_records",
                format_optional(summary.turn_state_max_terminal_records),
            ),
            (
                "turn_state_max_events",
                format_optional(summary.turn_state_max_events),
            ),
            (
                "turn_state_max_idempotency_records",
                format_optional(summary.turn_state_max_idempotency_records),
            ),
            (
                "active_thread_count",
                format_active_thread_count(summary.active_thread_count, summary.users),
            ),
            ("tenants", summary.tenants.to_string()),
            ("prefill_threads", summary.prefill_threads.to_string()),
            (
                "prefill_turns_per_thread",
                summary.prefill_turns_per_thread.to_string(),
            ),
            (
                "prefill_concurrency",
                summary.prefill_concurrency.to_string(),
            ),
            ("model_latency_ms", summary.model_latency_ms.to_string()),
            (
                "model_latency_source",
                summary.model_latency_source.as_str().to_string(),
            ),
            (
                "provider_model",
                summary.provider_model.as_deref().unwrap_or("-").to_string(),
            ),
            (
                "provider_max_tokens",
                summary.provider_max_tokens.to_string(),
            ),
            (
                "model_latency_profile",
                summary.model_latency_profile.as_str().to_string(),
            ),
            (
                "model_latency_jitter_ms",
                summary.model_latency_jitter_ms.to_string(),
            ),
            (
                "model_latency_spike_every",
                summary.model_latency_spike_every.to_string(),
            ),
            (
                "model_latency_spike_ms",
                summary.model_latency_spike_ms.to_string(),
            ),
            ("user_message_bytes", summary.user_message_bytes.to_string()),
            (
                "assistant_message_bytes",
                summary.assistant_message_bytes.to_string(),
            ),
            (
                "context_max_messages",
                summary.context_max_messages.to_string(),
            ),
            (
                "thread_list_threads",
                summary.thread_list_threads.to_string(),
            ),
            (
                "thread_list_page_size",
                summary.thread_list_page_size.to_string(),
            ),
            (
                "context_growth_turns_per_op",
                summary.context_growth_turns_per_operation.to_string(),
            ),
            (
                "tool_calls_per_turn",
                summary.tool_calls_per_turn.to_string(),
            ),
            ("tool_latency_ms", summary.tool_latency_ms.to_string()),
            ("tool_output_bytes", summary.tool_output_bytes.to_string()),
            ("tool_failure_every", summary.tool_failure_every.to_string()),
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
    if let Some(db_probe) = &summary.db_probe {
        push_db_probe_table(&mut output, db_probe);
    }
    if let Some(prefill) = &summary.prefill {
        push_prefill_table(&mut output, prefill);
    }
    if let Some(attribution) = &summary.operation_attribution {
        push_operation_attribution_table(&mut output, attribution);
    }
    if let Some(stages) = &summary.stage_latency {
        push_stage_latency_table(&mut output, stages);
    }
    if let Some(api_capacity) = &summary.api_capacity {
        push_api_capacity_table(&mut output, api_capacity);
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
            (
                "turn_state_backend",
                args.turn_state_backend.as_str().to_string(),
            ),
            (
                "turn_state_max_terminal_records",
                format_optional(args.turn_state_max_terminal_records),
            ),
            (
                "turn_state_max_events",
                format_optional(args.turn_state_max_events),
            ),
            (
                "turn_state_max_idempotency_records",
                format_optional(args.turn_state_max_idempotency_records),
            ),
            ("preset", format_preset(args.preset)),
            ("scenario", args.scenario.as_str().to_string()),
            ("run_id", run_id.to_string()),
            ("target", target.to_string()),
            ("processes", args.processes.to_string()),
            ("concurrency_per_process", args.concurrency.to_string()),
            ("duration_seconds", args.duration_seconds.to_string()),
            ("warmup_seconds", args.warmup_seconds.to_string()),
            (
                "trace_jsonl_enabled",
                args.trace_jsonl.is_some().to_string(),
            ),
            (
                "trace_interval_seconds",
                args.trace_interval_seconds.to_string(),
            ),
            (
                "active_thread_count",
                format_active_thread_count(args.active_thread_count, args.users),
            ),
            ("prefill_threads", args.prefill_threads.to_string()),
            (
                "prefill_turns_per_thread",
                args.prefill_turns_per_thread.to_string(),
            ),
            ("prefill_concurrency", args.prefill_concurrency.to_string()),
            ("model_latency_ms", args.model_latency_ms.to_string()),
            (
                "model_latency_source",
                args.model_latency_source.as_str().to_string(),
            ),
            (
                "provider_model",
                args.provider_model.as_deref().unwrap_or("-").to_string(),
            ),
            ("provider_max_tokens", args.provider_max_tokens.to_string()),
            (
                "model_latency_profile",
                args.model_latency_profile.as_str().to_string(),
            ),
            (
                "model_latency_jitter_ms",
                args.model_latency_jitter_ms.to_string(),
            ),
            (
                "model_latency_spike_every",
                args.model_latency_spike_every.to_string(),
            ),
            (
                "model_latency_spike_ms",
                args.model_latency_spike_ms.to_string(),
            ),
            ("user_message_bytes", args.user_message_bytes.to_string()),
            (
                "assistant_message_bytes",
                args.assistant_message_bytes.to_string(),
            ),
            (
                "context_max_messages",
                args.context_max_messages.to_string(),
            ),
            ("thread_list_threads", args.thread_list_threads.to_string()),
            (
                "thread_list_page_size",
                args.thread_list_page_size.to_string(),
            ),
            (
                "context_growth_turns_per_op",
                args.context_growth_turns_per_operation.to_string(),
            ),
            ("tool_calls_per_turn", args.tool_calls_per_turn.to_string()),
            ("tool_latency_ms", args.tool_latency_ms.to_string()),
            ("tool_output_bytes", args.tool_output_bytes.to_string()),
            ("tool_failure_every", args.tool_failure_every.to_string()),
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
        ("list_threads_cold", &stages.list_threads_cold),
        ("list_threads_warm", &stages.list_threads_warm),
        ("claim_run", &stages.claim_run),
        ("block_run", &stages.block_run),
        ("resume_turn", &stages.resume_turn),
        ("reclaim_run", &stages.reclaim_run),
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

fn push_operation_attribution_table(
    output: &mut String,
    attribution: &UserTurnOperationAttributionSummary,
) {
    let rows = operation_attribution_latency_rows(attribution);
    if !rows.is_empty() {
        push_latency_table(output, "Operation attribution", &rows);
    }
}

fn operation_attribution_latency_rows(
    attribution: &UserTurnOperationAttributionSummary,
) -> Vec<(&'static str, u64, &LatencySummary)> {
    operation_attribution_rows(attribution)
        .into_iter()
        .filter(|(_, group)| group.count > 0)
        .map(|(name, group)| (name, group.count, &group.latency))
        .collect()
}

fn push_api_capacity_table(
    output: &mut String,
    api_capacity: &crate::api_capacity::ApiCapacitySummary,
) {
    push_overview(
        output,
        "API capacity",
        &[
            ("base_url", api_capacity.base_url.clone()),
            ("virtual_users", api_capacity.virtual_users.to_string()),
            (
                "read_qps_per_user",
                format!("{:.2}", api_capacity.read_qps_per_user),
            ),
            ("read_workers", api_capacity.read_workers.to_string()),
            ("read_mix", api_capacity.read_mix.clone()),
            ("page_size", api_capacity.page_size.to_string()),
            (
                "wait_for_assistant",
                api_capacity.wait_for_assistant.to_string(),
            ),
            (
                "terminal_timeout_ms",
                api_capacity.terminal_timeout_ms.to_string(),
            ),
        ],
    );
    let mut rows: Vec<(&str, u64, &LatencySummary)> = api_capacity
        .endpoints
        .iter()
        .map(|(name, summary)| (name.as_str(), summary.attempted, &summary.latency))
        .collect();
    rows.sort_by_key(|(name, _, _)| *name);
    if !rows.is_empty() {
        push_latency_table(output, "API endpoint latency", &rows);
    }
}

fn push_prefill_table(output: &mut String, prefill: &crate::user_turn::PrefillSummary) {
    let _ = writeln!(output, "\nPrefill");
    let _ = writeln!(
        output,
        "{:<12} {:>8} {:>8} {:>8} {:>10} {:>10} {:>10} {:>10}",
        "threads", "turns", "ok", "failed", "duration", "ops/sec", "p95", "max"
    );
    let _ = writeln!(
        output,
        "{:-<12} {:->8} {:->8} {:->8} {:->10} {:->10} {:->10} {:->10}",
        "", "", "", "", "", "", "", ""
    );
    let _ = writeln!(
        output,
        "{:<12} {:>8} {:>8} {:>8} {:>10} {:>10.2} {:>10} {:>10}",
        prefill.threads,
        prefill.turns_per_thread,
        prefill.succeeded,
        prefill.failed,
        format_duration_ms(prefill.duration_ms),
        prefill.throughput_ops_sec,
        format_latency_us(prefill.latency.p95_us),
        format_latency_us(prefill.latency.max_us),
    );
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

fn push_db_probe_table(output: &mut String, db_probe: &DbProbeSummary) {
    let _ = writeln!(output, "\nDB probe");
    let _ = writeln!(
        output,
        "{:<36} {:>12} {:>12} {:>12}",
        "metric", "before", "after", "delta"
    );
    let _ = writeln!(output, "{:-<36} {:->12} {:->12} {:->12}", "", "", "", "");
    push_db_size_metric(
        output,
        "libsql_file",
        db_probe.before.libsql_file_bytes,
        db_probe.after.libsql_file_bytes,
        db_probe.delta.libsql_file_bytes,
    );
    push_db_size_metric(
        output,
        "libsql_wal",
        db_probe.before.libsql_wal_bytes,
        db_probe.after.libsql_wal_bytes,
        db_probe.delta.libsql_wal_bytes,
    );
    push_db_size_metric(
        output,
        "libsql_shm",
        db_probe.before.libsql_shm_bytes,
        db_probe.after.libsql_shm_bytes,
        db_probe.delta.libsql_shm_bytes,
    );
    push_db_size_metric(
        output,
        "postgres_database_size",
        db_probe.before.postgres_database_size_bytes,
        db_probe.after.postgres_database_size_bytes,
        db_probe.delta.postgres_database_size_bytes,
    );
    push_db_count_metric(
        output,
        "postgres_active_connections",
        db_probe.before.postgres_active_connections,
        db_probe.after.postgres_active_connections,
    );
    push_db_count_metric(
        output,
        "postgres_idle_connections",
        db_probe.before.postgres_idle_connections,
        db_probe.after.postgres_idle_connections,
    );
    push_db_count_metric(
        output,
        "postgres_waiting_connections",
        db_probe.before.postgres_waiting_connections,
        db_probe.after.postgres_waiting_connections,
    );
    push_db_probe_error(output, "before_error", &db_probe.before);
    push_db_probe_error(output, "after_error", &db_probe.after);
}

fn push_db_size_metric(
    output: &mut String,
    name: &str,
    before: Option<u64>,
    after: Option<u64>,
    delta: Option<i128>,
) {
    if before.is_none() && after.is_none() && delta.is_none() {
        return;
    }
    let _ = writeln!(
        output,
        "{name:<36} {:>12} {:>12} {:>12}",
        format_optional_bytes(before),
        format_optional_bytes(after),
        format_optional_byte_delta(delta),
    );
}

fn push_db_count_metric(output: &mut String, name: &str, before: Option<u64>, after: Option<u64>) {
    if before.is_none() && after.is_none() {
        return;
    }
    let _ = writeln!(
        output,
        "{name:<36} {:>12} {:>12} {:>12}",
        format_optional(before),
        format_optional(after),
        "-"
    );
}

fn push_db_probe_error(output: &mut String, name: &str, snapshot: &DbProbeSnapshot) {
    if let Some(error) = &snapshot.error {
        let (before, after) = match name {
            "before_error" => (truncate(error, 48), "-".to_string()),
            "after_error" => ("-".to_string(), truncate(error, 48)),
            _ => ("-".to_string(), truncate(error, 48)),
        };
        let _ = writeln!(
            output,
            "{name:<36} {:>12} {:>12} {:>12}",
            before, after, "-"
        );
    }
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

fn format_active_thread_count(active_thread_count: usize, users: usize) -> String {
    if active_thread_count == 0 {
        format!("per-user ({users})")
    } else {
        active_thread_count.to_string()
    }
}

fn format_preset(preset: Option<crate::StressPreset>) -> String {
    preset
        .map(crate::StressPreset::as_str)
        .unwrap_or("-")
        .to_string()
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

fn format_optional_bytes(value: Option<u64>) -> String {
    value.map(format_bytes).unwrap_or_else(|| "-".to_string())
}

fn format_optional_byte_delta(value: Option<i128>) -> String {
    value
        .map(|value| {
            if value >= 0 {
                format!("+{}", format_bytes_i128(value))
            } else {
                format!("-{}", format_bytes_i128(value.saturating_abs()))
            }
        })
        .unwrap_or_else(|| "-".to_string())
}

fn format_bytes(value: u64) -> String {
    format_bytes_i128(i128::from(value))
}

fn format_bytes_i128(value: i128) -> String {
    if value >= 1024 * 1024 * 1024 {
        format!("{:.2}GB", value as f64 / 1024.0 / 1024.0 / 1024.0)
    } else if value >= 1024 * 1024 {
        format!("{:.1}MB", value as f64 / 1024.0 / 1024.0)
    } else if value >= 1024 {
        format!("{:.1}KB", value as f64 / 1024.0)
    } else {
        format!("{value}B")
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
