use std::collections::BTreeMap;

use crate::{
    Args, RunSummary, analysis, capture::CapturedRun, compare, human,
    process_metrics::aggregate_process_metrics, summary::FailureCauseSummary,
};

pub(crate) fn print_captured_run(
    args: &Args,
    run_id: &str,
    captured: &CapturedRun,
) -> Result<(), String> {
    match captured {
        CapturedRun::Single(summary) => print_run_summary(args, summary),
        CapturedRun::Parent { summaries, .. } => print_parent_summary(args, run_id, summaries),
    }?;

    if args.bottleneck_report && args.child_index.is_none() {
        eprint!(
            "{}",
            analysis::render_bottleneck_report(args, run_id, captured)
        );
    }
    if let Some(path) = &args.compare_json
        && args.child_index.is_none()
    {
        eprint!(
            "{}",
            compare::render_comparison_report(path, &captured.summary_value())?
        );
    }
    Ok(())
}

pub(crate) fn parent_summary_value(
    args: &Args,
    run_id: &str,
    summaries: &[RunSummary],
) -> serde_json::Value {
    let attempted: u64 = summaries.iter().map(|summary| summary.attempted).sum();
    let succeeded: u64 = summaries.iter().map(|summary| summary.succeeded).sum();
    let failed: u64 = summaries.iter().map(|summary| summary.failed).sum();
    let mut errors = BTreeMap::new();
    for summary in summaries {
        for (error, count) in &summary.errors {
            *errors.entry(error.clone()).or_insert(0) += count;
        }
    }
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
    let p99_us = summaries
        .iter()
        .map(|summary| summary.latency.p99_us)
        .max()
        .unwrap_or(0);
    let max_us = summaries
        .iter()
        .map(|summary| summary.latency.max_us)
        .max()
        .unwrap_or(0);
    let target = summaries
        .first()
        .map(|summary| summary.target.as_str())
        .unwrap_or("unknown");
    let process = aggregate_process_metrics(summaries.iter().map(|summary| &summary.process));

    serde_json::json!({
        "backend": args.backend,
        "preset": args.preset,
        "scenario": args.scenario,
        "run_id": run_id,
        "target": target,
        "processes": args.processes,
        "concurrency_per_process": args.concurrency,
        "duration_seconds": args.duration_seconds,
        "warmup_seconds": args.warmup_seconds,
        "trace_jsonl_enabled": args.trace_jsonl.is_some(),
        "trace_interval_seconds": args.trace_interval_seconds,
        "active_thread_count": args.active_thread_count,
        "turn_state_backend": args.turn_state_backend,
        "turn_state_max_terminal_records": args.turn_state_max_terminal_records,
        "turn_state_max_events": args.turn_state_max_events,
        "turn_state_max_idempotency_records": args.turn_state_max_idempotency_records,
        "thread_list_threads": args.thread_list_threads,
        "thread_list_page_size": args.thread_list_page_size,
        "prefill_threads": args.prefill_threads,
        "prefill_turns_per_thread": args.prefill_turns_per_thread,
        "prefill_concurrency": args.prefill_concurrency,
        "tool_calls_per_turn": args.tool_calls_per_turn,
        "tool_latency_ms": args.tool_latency_ms,
        "tool_output_bytes": args.tool_output_bytes,
        "tool_failure_every": args.tool_failure_every,
        "attempted": attempted,
        "succeeded": succeeded,
        "failed": failed,
        "max_duration_ms": max_duration_ms,
        "throughput_ops_sec": throughput_ops_sec,
        "worst_child_p99_us": p99_us,
        "worst_child_max_us": max_us,
        "process": process,
        "errors": errors,
        "failure_causes": failure_causes,
        "children": summaries,
    })
}

fn print_run_summary(args: &Args, summary: &RunSummary) -> Result<(), String> {
    let encoded = serde_json::to_string_pretty(summary).map_err(|error| error.to_string())?;
    println!("{encoded}");
    if args.human_read && args.child_index.is_none() {
        eprint!("{}", human::render_run_summary(summary));
    }
    Ok(())
}

fn print_parent_summary(args: &Args, run_id: &str, summaries: &[RunSummary]) -> Result<(), String> {
    let aggregate = parent_summary_value(args, run_id, summaries);
    let encoded = serde_json::to_string_pretty(&aggregate).map_err(|error| error.to_string())?;
    println!("{encoded}");
    if args.human_read {
        eprint!("{}", human::render_parent_summary(args, run_id, summaries));
    }
    Ok(())
}
