//! Timing-only projection of one run's process-local diagnostic snapshot.
//!
//! This module deliberately carries NO `BoundedDiagnosticText` payloads
//! across into the artifact: no prompt text, no tool arguments, no tool
//! results. Capability names, statuses, counts, and durations only. The
//! artifact is a user-downloadable file, and the redaction pipeline that
//! guards `messages` does not run over this block.

use std::collections::HashMap;

use crate::inspector_store::DiagnosticTimingSnapshot;
use chrono::{DateTime, Utc};
use ironclaw_product_contracts::inspector::{
    DiagnosticMetricTotal, DiagnosticModelCallId, InspectorModelCallStatus, ToolExecutionStatus,
};
use serde::{Deserialize, Serialize};

/// Names the capture source in the exported file so a reader knows which
/// buffer produced (or failed to produce) these numbers.
pub const TIMINGS_SOURCE: &str = "diagnostic_store";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunArtifactTimings {
    pub source: String,
    pub available: bool,
    /// ponytail: always false. Timings come from the process-local, bounded
    /// `InMemoryDiagnosticStore` (`crates/product/ironclaw_assistant/src/inspector_store.rs:3`
    /// — "deliberately has no persistence backend"), capped by
    /// `DiagnosticStoreLimits` (same file, :49). A restart or an eviction
    /// removes a run's timings with no durable marker, exactly like the
    /// sibling `RunArtifactLogs.complete`. Ceiling: timings are unavailable
    /// for any run that left the buffer. Upgrade path: a durable diagnostics
    /// store, deliberately deferred — see this plan's Global Constraints.
    pub complete: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<String>,
    pub iterations: Vec<RunArtifactIterationTiming>,
    /// Tool executions the store never correlated to a model call, or whose
    /// model call left the buffer first. Counted, never dropped.
    #[serde(default)]
    pub unattributed_tools: Vec<RunArtifactToolTiming>,
    pub totals: RunArtifactTimingTotals,
}

impl Default for RunArtifactTimings {
    fn default() -> Self {
        unavailable("timings_absent")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunArtifactIterationTiming {
    /// Agent-loop iteration number this model call served.
    pub iteration: u32,
    /// Effective model when the provider resolved one, else the requested model.
    pub model: String,
    pub started_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    /// Wall-clock time inside the provider call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inference_ms: Option<u64>,
    pub status: InspectorModelCallStatus,
    /// Tool calls this iteration requested — the "how many tools ran before
    /// the assistant replied" number, per iteration.
    pub tool_calls: u32,
    /// Sum of the tool durations below. `None` when no tool reported one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_ms_total: Option<u64>,
    pub tools: Vec<RunArtifactToolTiming>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunArtifactToolTiming {
    pub capability_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    pub status: ToolExecutionStatus,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunArtifactTimingTotals {
    pub iterations: u64,
    pub tool_calls: u64,
    pub failed_tool_calls: u64,
    /// Summed provider latency. `unavailable_samples` counts calls that
    /// reported no duration, so a reader can tell "fast" from "unmeasured".
    pub inference_ms: DiagnosticMetricTotal,
    /// Sum of durations for tool executions still retained by the bounded
    /// diagnostic store. This is never presented as a complete run total.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retained_tool_ms: Option<u64>,
    /// False when the store's cumulative tool count exceeds its retained
    /// execution records, so `retained_tool_ms` is known to be partial.
    #[serde(default, skip_serializing_if = "is_false")]
    pub retained_tool_ms_complete: bool,
    /// run.received_at → the newest message `updated_at`. Approximate: it
    /// includes queue and persistence latency around the run, and it is the
    /// only end-to-end number available without widening into turn state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wall_clock_ms: Option<u64>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// The "no numbers, and here is why" value. Never an error — a missing
/// timing block must not fail an artifact export.
pub fn unavailable(reason: &str) -> RunArtifactTimings {
    RunArtifactTimings {
        source: TIMINGS_SOURCE.to_string(),
        available: false,
        complete: false,
        unavailable_reason: Some(reason.to_string()),
        iterations: Vec::new(),
        unattributed_tools: Vec::new(),
        totals: RunArtifactTimingTotals::default(),
    }
}

pub fn project_timings(
    snapshot: DiagnosticTimingSnapshot,
    wall_clock_ms: Option<u64>,
) -> RunArtifactTimings {
    let mut tools_by_call: HashMap<DiagnosticModelCallId, Vec<RunArtifactToolTiming>> =
        HashMap::new();
    let mut unattributed_tools = Vec::new();
    let known_calls: std::collections::HashSet<DiagnosticModelCallId> = snapshot
        .model_calls
        .iter()
        .map(|call| call.call_id)
        .collect();

    let mut retained_tool_ms = 0_u64;
    let mut retained_tool_ms_seen = false;
    for execution in &snapshot.tool_executions {
        let timing = RunArtifactToolTiming {
            capability_name: execution.capability_name.content().to_string(),
            duration_ms: execution.duration_ms,
            status: execution.status,
        };
        if let Some(duration) = execution.duration_ms {
            retained_tool_ms = retained_tool_ms.saturating_add(duration);
            retained_tool_ms_seen = true;
        }
        match execution
            .model_call_id
            .filter(|call_id| known_calls.contains(call_id))
        {
            Some(call_id) => tools_by_call.entry(call_id).or_default().push(timing),
            None => unattributed_tools.push(timing),
        }
    }

    let mut iterations: Vec<RunArtifactIterationTiming> = snapshot
        .model_calls
        .iter()
        .map(|call| {
            let tools = tools_by_call.remove(&call.call_id).unwrap_or_default();
            let tool_ms_total = sum_durations(&tools);
            RunArtifactIterationTiming {
                iteration: call.iteration,
                model: call
                    .effective_model
                    .as_ref()
                    .unwrap_or(&call.requested_model)
                    .content()
                    .to_string(),
                started_at: call.started_at,
                completed_at: call.completed_at,
                inference_ms: call.duration_ms,
                status: call.status,
                tool_calls: u32::try_from(tools.len()).unwrap_or(u32::MAX),
                tool_ms_total,
                tools,
            }
        })
        .collect();
    // Capture order follows completion, not iteration order; sort so the file
    // reads as the loop ran.
    iterations.sort_by_key(|iteration| iteration.iteration);

    RunArtifactTimings {
        source: TIMINGS_SOURCE.to_string(),
        available: true,
        complete: false,
        unavailable_reason: None,
        iterations,
        unattributed_tools,
        totals: RunArtifactTimingTotals {
            iterations: snapshot.stats.total_model_calls,
            tool_calls: snapshot.stats.total_tool_calls,
            failed_tool_calls: snapshot.stats.failed_tool_calls,
            inference_ms: snapshot.stats.total_latency_ms,
            retained_tool_ms: retained_tool_ms_seen.then_some(retained_tool_ms),
            retained_tool_ms_complete: u64::try_from(snapshot.tool_executions.len())
                .unwrap_or(u64::MAX)
                >= snapshot.stats.total_tool_calls,
            wall_clock_ms,
        },
    }
}

fn sum_durations(tools: &[RunArtifactToolTiming]) -> Option<u64> {
    let mut total = 0_u64;
    let mut seen = false;
    for tool in tools {
        if let Some(duration) = tool.duration_ms {
            total = total.saturating_add(duration);
            seen = true;
        }
    }
    seen.then_some(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::turn::CapabilityActivityId;
    use ironclaw_product_contracts::inspector::{
        DiagnosticMetricTotal, InspectorModelCallStatus, ModelCallDiagnostic,
        SessionDiagnosticStats, ToolExecutionDiagnostic, ToolExecutionStatus,
    };

    fn model_call(
        call_id: DiagnosticModelCallId,
        iteration: u32,
        duration_ms: u64,
    ) -> ModelCallDiagnostic {
        let started = Utc::now();
        ModelCallDiagnostic::new(
            call_id,
            iteration,
            "claude-opus-5",
            Some("claude-opus-5-20260101".to_string()),
            started,
            Some(started + chrono::Duration::milliseconds(duration_ms as i64)),
            Some(duration_ms),
            InspectorModelCallStatus::Succeeded,
            None,
            None,
        )
    }

    fn tool(
        model_call_id: Option<DiagnosticModelCallId>,
        name: &str,
        duration_ms: u64,
        status: ToolExecutionStatus,
    ) -> ToolExecutionDiagnostic {
        ToolExecutionDiagnostic::new(
            CapabilityActivityId::new(),
            model_call_id,
            name,
            None,
            None,
            status,
            Some(duration_ms),
            None,
            None,
            None,
        )
    }

    fn snapshot(
        model_calls: Vec<ModelCallDiagnostic>,
        tool_executions: Vec<ToolExecutionDiagnostic>,
        stats: SessionDiagnosticStats,
    ) -> DiagnosticTimingSnapshot {
        DiagnosticTimingSnapshot {
            model_calls: model_calls
                .iter()
                .map(crate::inspector_store::DiagnosticTimingModelCall::from)
                .collect(),
            tool_executions: tool_executions
                .iter()
                .map(crate::inspector_store::DiagnosticTimingToolExecution::from)
                .collect(),
            stats,
        }
    }

    #[test]
    fn tools_are_grouped_under_the_iteration_that_requested_them() {
        let first = DiagnosticModelCallId::new();
        let second = DiagnosticModelCallId::new();
        let projected = project_timings(
            snapshot(
                vec![model_call(first, 1, 4_100), model_call(second, 2, 900)],
                vec![
                    tool(Some(first), "shell", 22_000, ToolExecutionStatus::Succeeded),
                    tool(Some(first), "read_file", 30, ToolExecutionStatus::Succeeded),
                    tool(Some(second), "shell", 11, ToolExecutionStatus::Failed),
                ],
                SessionDiagnosticStats::default(),
            ),
            Some(91_000),
        );

        assert!(projected.available);
        assert!(
            !projected.complete,
            "process-local capture is never complete"
        );
        assert_eq!(projected.iterations.len(), 2);
        assert_eq!(projected.iterations[0].iteration, 1);
        assert_eq!(projected.iterations[0].inference_ms, Some(4_100));
        assert_eq!(projected.iterations[0].tool_calls, 2);
        assert_eq!(projected.iterations[0].tool_ms_total, Some(22_030));
        assert_eq!(projected.iterations[1].tool_calls, 1);
        assert!(projected.unattributed_tools.is_empty());
        assert_eq!(projected.totals.wall_clock_ms, Some(91_000));
    }

    #[test]
    fn iterations_are_ordered_by_iteration_number_not_capture_order() {
        let first = DiagnosticModelCallId::new();
        let second = DiagnosticModelCallId::new();
        let projected = project_timings(
            snapshot(
                vec![model_call(second, 7, 10), model_call(first, 2, 10)],
                Vec::new(),
                SessionDiagnosticStats::default(),
            ),
            None,
        );

        let seen: Vec<u32> = projected
            .iterations
            .iter()
            .map(|iteration| iteration.iteration)
            .collect();
        assert_eq!(seen, vec![2, 7]);
    }

    #[test]
    fn uncorrelated_tools_land_in_the_unattributed_bucket_and_still_count() {
        let call = DiagnosticModelCallId::new();
        let projected = project_timings(
            snapshot(
                vec![model_call(call, 1, 50)],
                vec![
                    tool(None, "shell", 700, ToolExecutionStatus::Succeeded),
                    tool(Some(call), "read_file", 5, ToolExecutionStatus::Succeeded),
                ],
                SessionDiagnosticStats::default(),
            ),
            None,
        );

        assert_eq!(projected.unattributed_tools.len(), 1);
        assert_eq!(projected.unattributed_tools[0].capability_name, "shell");
        assert_eq!(projected.iterations[0].tool_calls, 1);
        assert_eq!(projected.totals.retained_tool_ms, Some(705));
    }

    #[test]
    fn aggregate_totals_preserve_counts_and_unavailable_samples() {
        let call = DiagnosticModelCallId::new();
        let projected = project_timings(
            snapshot(
                vec![model_call(call, 1, 50)],
                vec![tool(Some(call), "shell", 7, ToolExecutionStatus::Failed)],
                SessionDiagnosticStats {
                    total_model_calls: 3,
                    total_tool_calls: 4,
                    failed_tool_calls: 2,
                    total_latency_ms: DiagnosticMetricTotal {
                        known_total: 50,
                        unavailable_samples: 1,
                    },
                    ..SessionDiagnosticStats::default()
                },
            ),
            None,
        );

        assert_eq!(projected.totals.iterations, 3);
        assert_eq!(projected.totals.tool_calls, 4);
        assert_eq!(projected.totals.failed_tool_calls, 2);
        assert_eq!(projected.totals.inference_ms.known_total, 50);
        assert_eq!(projected.totals.inference_ms.unavailable_samples, 1);
        assert_eq!(projected.totals.retained_tool_ms, Some(7));
        assert!(!projected.totals.retained_tool_ms_complete);
    }

    #[test]
    fn timing_totals_decode_legacy_payload_without_completion_flag() {
        let legacy = r#"{
            "iterations": 1,
            "tool_calls": 0,
            "failed_tool_calls": 0,
            "inference_ms": {
                "known_total": 5,
                "unavailable_samples": 0
            }
        }"#;

        let totals: RunArtifactTimingTotals =
            serde_json::from_str(legacy).expect("legacy timing totals should decode");
        assert!(!totals.retained_tool_ms_complete);

        let serialized = serde_json::to_string(&totals).expect("timing totals should serialize");
        assert!(!serialized.contains("retained_tool_ms_complete"));
    }

    #[test]
    fn timing_sums_saturate_at_u64_max() {
        let call = DiagnosticModelCallId::new();
        let projected = project_timings(
            snapshot(
                vec![model_call(call, 1, 1)],
                vec![
                    tool(
                        Some(call),
                        "first",
                        u64::MAX,
                        ToolExecutionStatus::Succeeded,
                    ),
                    tool(Some(call), "second", 1, ToolExecutionStatus::Succeeded),
                ],
                SessionDiagnosticStats::default(),
            ),
            None,
        );

        assert_eq!(projected.iterations[0].tool_ms_total, Some(u64::MAX));
        assert_eq!(projected.totals.retained_tool_ms, Some(u64::MAX));
    }

    #[test]
    fn a_tool_pointing_at_an_unknown_call_is_unattributed_not_dropped() {
        let known = DiagnosticModelCallId::new();
        let dangling = DiagnosticModelCallId::new();
        let projected = project_timings(
            snapshot(
                vec![model_call(known, 1, 50)],
                vec![tool(
                    Some(dangling),
                    "shell",
                    3,
                    ToolExecutionStatus::Succeeded,
                )],
                SessionDiagnosticStats::default(),
            ),
            None,
        );

        assert_eq!(projected.unattributed_tools.len(), 1);
        assert_eq!(projected.iterations[0].tool_calls, 0);
    }

    #[test]
    fn no_bounded_payload_text_reaches_the_projection() {
        let call = DiagnosticModelCallId::new();
        // Built through `new` rather than by mutating fields: `new` derives
        // `output_bytes` from `result`, and the `TryFrom` wire guard rejects a
        // record whose byte metadata disagrees with its result.
        let leaky = ToolExecutionDiagnostic::new(
            CapabilityActivityId::new(),
            Some(call),
            "shell",
            Some("{\"path\":\"/home/alice/secret.txt\"}".to_string()),
            Some("sk-ant-super-secret".to_string()),
            ToolExecutionStatus::Succeeded,
            Some(1),
            None,
            None,
            None,
        );

        let projected = project_timings(
            snapshot(
                vec![model_call(call, 1, 50)],
                vec![leaky],
                SessionDiagnosticStats::default(),
            ),
            None,
        );

        let serialized = serde_json::to_string(&projected).expect("serialize timings");
        assert!(!serialized.contains("secret.txt"));
        assert!(!serialized.contains("sk-ant-super-secret"));
        assert!(!serialized.contains("/home/alice"));
    }

    #[test]
    fn the_default_value_reports_itself_as_unavailable() {
        let absent = RunArtifactTimings::default();
        assert!(!absent.available);
        assert!(!absent.complete);
        assert!(absent.iterations.is_empty());
    }
}
