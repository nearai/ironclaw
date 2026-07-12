use std::{collections::BTreeMap, time::Duration};

use serde::{Deserialize, Serialize};

use crate::{
    Sample,
    user_turn::{
        OperationAttributionSummary, StageLatencySummary, UserTurnOperationAttributionSummary,
        UserTurnStageDurations, UserTurnStageLatencySummary,
    },
};

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct LatencySummary {
    pub(crate) min_us: u128,
    pub(crate) p50_us: u128,
    pub(crate) p95_us: u128,
    pub(crate) p99_us: u128,
    pub(crate) max_us: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct FailureCause {
    pub(crate) bucket: String,
    pub(crate) stage: String,
    pub(crate) detail: String,
}

impl FailureCause {
    pub(crate) fn new(
        bucket: impl Into<String>,
        stage: impl Into<String>,
        detail: impl std::fmt::Display,
    ) -> Self {
        Self {
            bucket: bucket.into(),
            stage: stage.into(),
            detail: sanitize_failure_detail(detail),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct FailureCauseSummary {
    pub(crate) count: u64,
    pub(crate) stages: BTreeMap<String, u64>,
    pub(crate) sample_detail: String,
}

pub(crate) fn summarize_failure_causes(
    samples: &[Sample],
) -> BTreeMap<String, FailureCauseSummary> {
    let mut causes = BTreeMap::new();
    for failure in samples.iter().filter_map(|sample| sample.failure.as_ref()) {
        let summary = causes
            .entry(failure.bucket.clone())
            .or_insert_with(|| FailureCauseSummary {
                count: 0,
                stages: BTreeMap::new(),
                sample_detail: failure.detail.clone(),
            });
        summary.count += 1;
        *summary.stages.entry(failure.stage.clone()).or_insert(0) += 1;
    }
    causes
}

pub(crate) fn summarize_user_turn_stages(
    samples: &[Sample],
) -> Option<UserTurnStageLatencySummary> {
    let stages: Vec<UserTurnStageDurations> =
        samples.iter().filter_map(|sample| sample.stages).collect();
    if stages.is_empty() {
        return None;
    }

    Some(UserTurnStageLatencySummary {
        ensure_thread: summarize_stage(&stages, |stage| stage.ensure_thread),
        accept_inbound: summarize_stage(&stages, |stage| stage.accept_inbound),
        submit_turn: summarize_stage(&stages, |stage| stage.submit_turn),
        mark_submitted: summarize_stage(&stages, |stage| stage.mark_submitted),
        mark_rejected_busy: summarize_stage(&stages, |stage| stage.mark_rejected_busy),
        list_threads_cold: summarize_stage(&stages, |stage| stage.list_threads_cold),
        list_threads_warm: summarize_stage(&stages, |stage| stage.list_threads_warm),
        claim_run: summarize_stage(&stages, |stage| stage.claim_run),
        block_run: summarize_stage(&stages, |stage| stage.block_run),
        resume_turn: summarize_stage(&stages, |stage| stage.resume_turn),
        reclaim_run: summarize_stage(&stages, |stage| stage.reclaim_run),
        append_assistant: summarize_stage(&stages, |stage| stage.append_assistant),
        finalize_assistant: summarize_stage(&stages, |stage| stage.finalize_assistant),
        complete_run: summarize_stage(&stages, |stage| stage.complete_run),
        load_context: summarize_stage(&stages, |stage| stage.load_context),
        resource_reserve: summarize_stage(&stages, |stage| stage.resource_reserve),
        model_wait: summarize_stage(&stages, |stage| stage.model_wait),
        tool_wait: summarize_stage(&stages, |stage| stage.tool_wait),
        append_tool_result: summarize_stage(&stages, |stage| stage.append_tool_result),
        append_tool_preview: summarize_stage(&stages, |stage| stage.append_tool_preview),
        update_assistant_draft: summarize_stage(&stages, |stage| stage.update_assistant_draft),
        resource_reconcile: summarize_stage(&stages, |stage| stage.resource_reconcile),
        resource_release: summarize_stage(&stages, |stage| stage.resource_release),
    })
}

pub(crate) fn summarize_user_turn_operation_attribution(
    samples: &[Sample],
) -> Option<UserTurnOperationAttributionSummary> {
    let stages: Vec<UserTurnStageDurations> =
        samples.iter().filter_map(|sample| sample.stages).collect();
    if stages.is_empty() {
        return None;
    }

    Some(UserTurnOperationAttributionSummary {
        thread_store_writes: summarize_attribution_group(&stages, thread_store_write_duration),
        context_reads: summarize_attribution_group(&stages, context_read_duration),
        turn_store: summarize_attribution_group(&stages, turn_store_duration),
        resource_governor: summarize_attribution_group(&stages, resource_governor_duration),
        synthetic_wait: summarize_attribution_group(&stages, synthetic_wait_duration),
    })
}

fn summarize_stage(
    stages: &[UserTurnStageDurations],
    stage: impl Fn(&UserTurnStageDurations) -> Option<Duration>,
) -> StageLatencySummary {
    let mut latencies: Vec<u128> = stages
        .iter()
        .filter_map(stage)
        .map(|duration| duration.as_micros())
        .collect();
    latencies.sort_unstable();
    StageLatencySummary {
        count: latencies.len() as u64,
        latency: latency_summary(&latencies),
    }
}

pub(crate) fn latency_summary(latencies: &[u128]) -> LatencySummary {
    if latencies.is_empty() {
        return LatencySummary {
            min_us: 0,
            p50_us: 0,
            p95_us: 0,
            p99_us: 0,
            max_us: 0,
        };
    }
    LatencySummary {
        min_us: latencies[0],
        p50_us: percentile(latencies, 50),
        p95_us: percentile(latencies, 95),
        p99_us: percentile(latencies, 99),
        max_us: latencies[latencies.len() - 1],
    }
}

fn summarize_attribution_group(
    stages: &[UserTurnStageDurations],
    group: impl Fn(&UserTurnStageDurations) -> Duration,
) -> OperationAttributionSummary {
    let mut latencies: Vec<u128> = stages
        .iter()
        .map(group)
        .filter(|duration| *duration > Duration::ZERO)
        .map(|duration| duration.as_micros())
        .collect();
    latencies.sort_unstable();
    OperationAttributionSummary {
        count: latencies.len() as u64,
        latency: latency_summary(&latencies),
    }
}

fn thread_store_write_duration(stage: &UserTurnStageDurations) -> Duration {
    sum_durations([
        stage.ensure_thread,
        stage.accept_inbound,
        stage.mark_submitted,
        stage.mark_rejected_busy,
        stage.append_assistant,
        stage.finalize_assistant,
        stage.append_tool_result,
        stage.append_tool_preview,
        stage.update_assistant_draft,
    ])
}

fn context_read_duration(stage: &UserTurnStageDurations) -> Duration {
    sum_durations([
        stage.load_context,
        stage.list_threads_cold,
        stage.list_threads_warm,
    ])
}

fn turn_store_duration(stage: &UserTurnStageDurations) -> Duration {
    sum_durations([
        stage.submit_turn,
        stage.claim_run,
        stage.block_run,
        stage.resume_turn,
        stage.reclaim_run,
        stage.complete_run,
    ])
}

fn resource_governor_duration(stage: &UserTurnStageDurations) -> Duration {
    sum_durations([
        stage.resource_reserve,
        stage.resource_reconcile,
        stage.resource_release,
    ])
}

fn synthetic_wait_duration(stage: &UserTurnStageDurations) -> Duration {
    sum_durations([stage.model_wait, stage.tool_wait])
}

fn sum_durations<const N: usize>(durations: [Option<Duration>; N]) -> Duration {
    durations.into_iter().flatten().sum()
}

fn percentile(sorted: &[u128], percentile: usize) -> u128 {
    let last = sorted.len().saturating_sub(1);
    let index = (last * percentile).div_ceil(100);
    sorted[index.min(last)]
}

fn sanitize_failure_detail(detail: impl std::fmt::Display) -> String {
    const MAX_DETAIL_CHARS: usize = 512;

    let detail = detail.to_string().replace(['\n', '\r'], " ");
    let mut truncated = detail.chars().take(MAX_DETAIL_CHARS).collect::<String>();
    if detail.chars().count() > MAX_DETAIL_CHARS {
        truncated.push_str("...");
    }
    truncated
}
