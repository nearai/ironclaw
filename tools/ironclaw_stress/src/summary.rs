use std::{collections::BTreeMap, time::Duration};

use serde::{Deserialize, Serialize};

use crate::{
    Sample,
    user_turn::{StageLatencySummary, UserTurnStageDurations, UserTurnStageLatencySummary},
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
        claim_run: summarize_stage(&stages, |stage| stage.claim_run),
        append_assistant: summarize_stage(&stages, |stage| stage.append_assistant),
        finalize_assistant: summarize_stage(&stages, |stage| stage.finalize_assistant),
        complete_run: summarize_stage(&stages, |stage| stage.complete_run),
        load_context: summarize_stage(&stages, |stage| stage.load_context),
        resource_reserve: summarize_stage(&stages, |stage| stage.resource_reserve),
        model_wait: summarize_stage(&stages, |stage| stage.model_wait),
        resource_reconcile: summarize_stage(&stages, |stage| stage.resource_reconcile),
        resource_release: summarize_stage(&stages, |stage| stage.resource_release),
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
