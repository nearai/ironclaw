use std::time::Duration;

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
