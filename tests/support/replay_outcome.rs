//! Snapshot-oriented view of what a replay produced.
//!
//! A trace fixture plays two roles. The `.json` file is the **replay driver**:
//! recorded LLM responses and HTTP exchanges the harness uses to stub real
//! calls deterministically. The `.snap` file generated from this struct is the
//! **regression snapshot**: the observable output of replaying that fixture —
//! what tools fired, in what order, the final state, and any issues the
//! retrospective analyzer flagged.
//!
//! Reviewers diff the snapshot, not the raw JSON. Keep the struct narrow so
//! small prompt-wording changes don't force snapshot churn.

#![allow(dead_code)] // Consumed by snapshot tests gated by features.

use std::collections::BTreeMap;

use serde::Serialize;

use ironclaw::channels::{OutgoingResponse, StatusUpdate};

use crate::support::test_rig::TestRig;

/// Short summary of a single status event, ordered and typed for snapshot
/// review. Excludes wall-clock fields, request IDs, and full tool output —
/// those live in the replay driver JSON, not here.
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventSummary {
    Thinking {
        message: String,
    },
    ToolStarted {
        name: String,
    },
    ToolCompleted {
        name: String,
        success: bool,
        error: Option<String>,
    },
    ToolResultPreview {
        name: String,
        /// Character length of the preview the UI received. The full preview
        /// text lives in the replay driver and is sensitive to prompt drift —
        /// using a bucketed length keeps snapshots stable.
        preview_len_bucket: usize,
    },
    Status {
        message: String,
    },
    ApprovalNeeded {
        tool_name: String,
    },
    AuthRequired {
        extension_name: String,
    },
    AuthCompleted {
        extension_name: String,
        success: bool,
    },
    Suggestions {
        count: usize,
    },
    /// Anything else we haven't explicitly modelled. Kept as a bucketed
    /// variant so unrelated new status kinds don't spam snapshot diffs.
    Other {
        variant: &'static str,
    },
}

/// Summary of a single tool invocation that the rig observed.
#[derive(Debug, Serialize)]
pub struct ToolCallSummary {
    pub name: String,
    pub success: bool,
}

/// Summary of a retrospective issue the engine flagged.
#[derive(Debug, Serialize)]
pub struct TraceIssueSummary {
    pub severity: String,
    pub category: String,
}

/// Summary of one engine thread's post-run state.
#[derive(Debug, Serialize)]
pub struct ThreadSummary {
    pub final_state: String,
    pub step_count: usize,
    pub message_roles: Vec<String>,
    pub event_kinds: Vec<String>,
    pub issues: Vec<TraceIssueSummary>,
}

/// Regression snapshot of a replay run.
///
/// Serialized as YAML by [`assert_replay_snapshot`]. Stable under LLM prompt
/// drift: it captures shape (tool sequence, final state, issue categories)
/// rather than model text.
#[derive(Debug, Serialize)]
pub struct ReplayOutcome {
    /// Number of outbound text responses the channel received.
    pub response_count: usize,
    /// Whether any final response was produced. Cheap for reviewers to
    /// interpret — `false` means the scenario hit a dead-end.
    pub has_final_response: bool,
    /// Tool invocations the channel observed, in order.
    pub tool_calls: Vec<ToolCallSummary>,
    /// Ordered status events, bucketed and trimmed for stability.
    pub events: Vec<EventSummary>,
    /// Histogram of status event kinds — makes coverage assertions cheap.
    pub event_kind_counts: BTreeMap<String, usize>,
    /// Number of LLM calls (bucketed to nearest 1).
    pub llm_call_count: u32,
    /// Number of safety-warning status events observed.
    pub safety_warning_count: usize,
    /// Per-thread retrospective analyzer output. Empty for engine v1 replays.
    pub engine_threads: Vec<ThreadSummary>,
}

impl ReplayOutcome {
    /// Capture the outcome of a just-completed replay from the rig.
    ///
    /// Call this after `wait_for_responses` / `run_trace`, before `shutdown`.
    pub async fn capture(rig: &TestRig, responses: &[OutgoingResponse]) -> Self {
        let status_events = rig.captured_status_events();
        let mut events: Vec<EventSummary> = Vec::with_capacity(status_events.len());
        let mut kind_counts: BTreeMap<String, usize> = BTreeMap::new();
        let mut safety_warning_count = 0usize;

        for event in status_events {
            let summary = match event {
                StatusUpdate::Thinking(msg) => {
                    *kind_counts.entry("Thinking".into()).or_default() += 1;
                    EventSummary::Thinking {
                        message: bucket_text(&msg),
                    }
                }
                StatusUpdate::ToolStarted { name, .. } => {
                    *kind_counts.entry("ToolStarted".into()).or_default() += 1;
                    EventSummary::ToolStarted {
                        name: strip_tool_params(&name),
                    }
                }
                StatusUpdate::ToolCompleted {
                    name,
                    success,
                    error,
                    ..
                } => {
                    *kind_counts.entry("ToolCompleted".into()).or_default() += 1;
                    EventSummary::ToolCompleted {
                        name: strip_tool_params(&name),
                        success,
                        error: error.map(|e| bucket_text(&e)),
                    }
                }
                StatusUpdate::ToolResult { name, preview, .. } => {
                    *kind_counts.entry("ToolResult".into()).or_default() += 1;
                    EventSummary::ToolResultPreview {
                        name: strip_tool_params(&name),
                        preview_len_bucket: bucket_usize(preview.chars().count(), 100),
                    }
                }
                StatusUpdate::StreamChunk(_) => {
                    *kind_counts.entry("StreamChunk".into()).or_default() += 1;
                    continue;
                }
                StatusUpdate::Status(msg) => {
                    *kind_counts.entry("Status".into()).or_default() += 1;
                    if is_safety_warning(&msg) {
                        safety_warning_count += 1;
                    }
                    EventSummary::Status {
                        message: bucket_text(&msg),
                    }
                }
                StatusUpdate::JobStarted { .. } => {
                    *kind_counts.entry("JobStarted".into()).or_default() += 1;
                    EventSummary::Other {
                        variant: "JobStarted",
                    }
                }
                StatusUpdate::ApprovalNeeded { tool_name, .. } => {
                    *kind_counts.entry("ApprovalNeeded".into()).or_default() += 1;
                    EventSummary::ApprovalNeeded { tool_name }
                }
                StatusUpdate::AuthRequired { extension_name, .. } => {
                    *kind_counts.entry("AuthRequired".into()).or_default() += 1;
                    EventSummary::AuthRequired { extension_name }
                }
                StatusUpdate::AuthCompleted {
                    extension_name,
                    success,
                    ..
                } => {
                    *kind_counts.entry("AuthCompleted".into()).or_default() += 1;
                    EventSummary::AuthCompleted {
                        extension_name,
                        success,
                    }
                }
                StatusUpdate::ImageGenerated { .. } => {
                    *kind_counts.entry("ImageGenerated".into()).or_default() += 1;
                    EventSummary::Other {
                        variant: "ImageGenerated",
                    }
                }
                StatusUpdate::Suggestions { suggestions } => {
                    *kind_counts.entry("Suggestions".into()).or_default() += 1;
                    EventSummary::Suggestions {
                        count: suggestions.len(),
                    }
                }
                StatusUpdate::ReasoningUpdate { .. } => {
                    *kind_counts.entry("ReasoningUpdate".into()).or_default() += 1;
                    EventSummary::Other {
                        variant: "ReasoningUpdate",
                    }
                }
                _ => {
                    *kind_counts.entry("Other".into()).or_default() += 1;
                    EventSummary::Other { variant: "Other" }
                }
            };
            events.push(summary);
        }

        let tool_calls: Vec<ToolCallSummary> = rig
            .tool_calls_completed()
            .into_iter()
            .map(|(name, success)| ToolCallSummary {
                name: strip_tool_params(&name),
                success,
            })
            .collect();

        let has_final_response = !responses.is_empty();

        let engine_threads = capture_engine_threads().await;

        Self {
            response_count: responses.len(),
            has_final_response,
            tool_calls,
            events,
            event_kind_counts: kind_counts,
            llm_call_count: rig.llm_call_count(),
            safety_warning_count,
            engine_threads,
        }
    }
}

/// Engine v2 formats tool names as `"echo(hello...)"`. The parameter summary
/// is useful for logs but depends on model wording and is a churn source —
/// strip it before snapshotting.
fn strip_tool_params(name: &str) -> String {
    match name.find('(') {
        Some(i) => name[..i].to_string(),
        None => name.to_string(),
    }
}

/// Bucket free-form message text to something stable under small rewording.
/// Keeps leading ~40 chars, lowercased, to help reviewers recognize which
/// event fired without comparing full model output.
fn bucket_text(s: &str) -> String {
    let trimmed: String = s.chars().take(40).collect();
    trimmed.to_lowercase()
}

fn bucket_usize(value: usize, bucket: usize) -> usize {
    if bucket == 0 {
        return value;
    }
    (value / bucket) * bucket
}

fn is_safety_warning(msg: &str) -> bool {
    let lower = msg.to_lowercase();
    lower.contains("sanitiz") || lower.contains("inject") || lower.contains("warning")
}

#[cfg(feature = "libsql")]
async fn capture_engine_threads() -> Vec<ThreadSummary> {
    let traces = ironclaw::bridge::engine_retrospectives_for_test().await;
    traces.into_iter().map(thread_summary_from).collect()
}

#[cfg(not(feature = "libsql"))]
async fn capture_engine_threads() -> Vec<ThreadSummary> {
    Vec::new()
}

#[cfg(feature = "libsql")]
fn thread_summary_from(trace: ironclaw_engine::executor::trace::ExecutionTrace) -> ThreadSummary {
    use ironclaw_engine::executor::trace::IssueSeverity;

    let issues = trace
        .issues
        .into_iter()
        .map(|issue| {
            let severity = match issue.severity {
                IssueSeverity::Error => "error",
                IssueSeverity::Warning => "warning",
                IssueSeverity::Info => "info",
            }
            .to_string();
            TraceIssueSummary {
                severity,
                category: issue.category,
            }
        })
        .collect();

    let message_roles = trace.messages.iter().map(|m| m.role.clone()).collect();

    let event_kinds = trace
        .events
        .iter()
        .map(|e| event_kind_name(&e.kind).to_string())
        .collect();

    ThreadSummary {
        final_state: format!("{:?}", trace.final_state),
        step_count: trace.step_count,
        message_roles,
        event_kinds,
        issues,
    }
}

#[cfg(feature = "libsql")]
fn event_kind_name(kind: &ironclaw_engine::EventKind) -> &'static str {
    use ironclaw_engine::EventKind;
    match kind {
        EventKind::StateChanged { .. } => "StateChanged",
        EventKind::StepStarted { .. } => "StepStarted",
        EventKind::StepCompleted { .. } => "StepCompleted",
        EventKind::StepFailed { .. } => "StepFailed",
        EventKind::ActionExecuted { .. } => "ActionExecuted",
        EventKind::ActionFailed { .. } => "ActionFailed",
        EventKind::LeaseGranted { .. } => "LeaseGranted",
        EventKind::LeaseRevoked { .. } => "LeaseRevoked",
        EventKind::LeaseExpired { .. } => "LeaseExpired",
        EventKind::MessageAdded { .. } => "MessageAdded",
        EventKind::ChildSpawned { .. } => "ChildSpawned",
        EventKind::ChildCompleted { .. } => "ChildCompleted",
        EventKind::ApprovalRequested { .. } => "ApprovalRequested",
        EventKind::ApprovalReceived { .. } => "ApprovalReceived",
        EventKind::SelfImprovementStarted => "SelfImprovementStarted",
        EventKind::SelfImprovementComplete { .. } => "SelfImprovementComplete",
        EventKind::SelfImprovementFailed { .. } => "SelfImprovementFailed",
        EventKind::SkillActivated { .. } => "SkillActivated",
        EventKind::CodeExecutionFailed { .. } => "CodeExecutionFailed",
        EventKind::OrchestratorRollback { .. } => "OrchestratorRollback",
        EventKind::Unknown => "Unknown",
    }
}

/// Assert that `outcome` matches the saved YAML snapshot for `name`.
///
/// Snapshots live at `tests/snapshots/replay__{name}.snap`. Use
/// `cargo insta review` to accept snapshot diffs interactively.
#[macro_export]
macro_rules! assert_replay_snapshot {
    ($name:expr, $outcome:expr) => {{
        let mut settings = ::insta::Settings::clone_current();
        settings.set_snapshot_path(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/snapshots"));
        settings.set_prepend_module_to_snapshot(false);
        settings.set_sort_maps(true);
        settings.set_omit_expression(true);
        settings.bind(|| {
            ::insta::assert_yaml_snapshot!(format!("replay__{}", $name), $outcome);
        });
    }};
}
