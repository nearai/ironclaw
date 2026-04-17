//! Event sourcing types.
//!
//! Every significant action within a thread is recorded as an event.
//! This enables replay, debugging, reflection, and trace-based testing.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::capability::LeaseId;

/// Generate a short human-readable summary of tool parameters for display.
///
/// For `http`: shows the URL. For `web_search`: shows the query.
/// For other tools: shows the first string argument, truncated.
/// Returns `None` for empty or unrecognizable params.
pub fn summarize_params(action_name: &str, params: &serde_json::Value) -> Option<String> {
    let summary = match action_name {
        "http" | "web_fetch" => params
            .get("url")
            .and_then(|v| v.as_str())
            .map(|u| truncate(u, 80)),
        "web_search" | "llm_context" => params
            .get("query")
            .and_then(|v| v.as_str())
            .map(|q| truncate(q, 60)),
        "memory_search" => params
            .get("query")
            .and_then(|v| v.as_str())
            .map(|q| truncate(q, 60)),
        "memory_write" => params
            .get("target")
            .and_then(|v| v.as_str())
            .map(|t| t.to_string()),
        "memory_read" => params
            .get("path")
            .and_then(|v| v.as_str())
            .map(|p| p.to_string()),
        "shell" => params
            .get("command")
            .and_then(|v| v.as_str())
            .map(|c| truncate(c, 60)),
        "message" => params
            .get("content")
            .and_then(|v| v.as_str())
            .map(|c| truncate(c, 40)),
        _ => {
            // Generic: show first string value
            if let Some(obj) = params.as_object() {
                obj.values()
                    .find_map(|v| v.as_str())
                    .map(|s| truncate(s, 50))
            } else {
                None
            }
        }
    };
    summary.filter(|s| !s.is_empty())
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        // Find a safe UTF-8 boundary
        let mut end = max.min(s.len());
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end]) // safety: end is validated by is_char_boundary loop above
    }
}
use crate::types::step::{StepId, TokenUsage};
use crate::types::thread::{ThreadId, ThreadState};

/// Strongly-typed event identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EventId(pub Uuid);

impl EventId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for EventId {
    fn default() -> Self {
        Self::new()
    }
}

/// A recorded event in a thread's execution history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadEvent {
    pub id: EventId,
    pub thread_id: ThreadId,
    pub timestamp: DateTime<Utc>,
    pub kind: EventKind,
}

impl ThreadEvent {
    pub fn new(thread_id: ThreadId, kind: EventKind) -> Self {
        Self {
            id: EventId::new(),
            thread_id,
            timestamp: Utc::now(),
            kind,
        }
    }
}

/// The specific kind of event that occurred.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EventKind {
    // ── Thread lifecycle ────────────────────────────────────
    StateChanged {
        from: ThreadState,
        to: ThreadState,
        reason: Option<String>,
    },

    // ── Step lifecycle ──────────────────────────────────────
    StepStarted {
        step_id: StepId,
    },
    StepCompleted {
        step_id: StepId,
        tokens: TokenUsage,
    },
    StepFailed {
        step_id: StepId,
        error: String,
    },

    // ── Action execution ────────────────────────────────────
    ActionExecuted {
        step_id: StepId,
        action_name: String,
        call_id: String,
        duration_ms: u64,
        /// Short human-readable summary of parameters (e.g., URL for http tool).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        params_summary: Option<String>,
        /// Truncated preview of the action output for live UI display.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output_preview: Option<String>,
    },
    ActionFailed {
        step_id: StepId,
        action_name: String,
        call_id: String,
        error: String,
        /// Short human-readable summary of parameters.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        params_summary: Option<String>,
    },

    // ── Capability leases ───────────────────────────────────
    LeaseGranted {
        lease_id: LeaseId,
        capability_name: String,
    },
    LeaseRevoked {
        lease_id: LeaseId,
        reason: String,
    },
    LeaseExpired {
        lease_id: LeaseId,
    },

    // ── Messages ────────────────────────────────────────────
    MessageAdded {
        role: String,
        content_preview: String,
    },

    // ── Thread tree ─────────────────────────────────────────
    ChildSpawned {
        child_id: ThreadId,
        goal: String,
    },
    ChildCompleted {
        child_id: ThreadId,
    },

    // ── Approval flow ───────────────────────────────────────
    ApprovalRequested {
        action_name: String,
        call_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parameters: Option<serde_json::Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        allow_always: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        gate_name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        params_summary: Option<String>,
    },
    ApprovalReceived {
        call_id: String,
        approved: bool,
    },

    // ── Self-improvement ──────────────────────────────────────
    SelfImprovementStarted,
    SelfImprovementComplete {
        prompt_updated: bool,
        patterns_added: usize,
    },
    SelfImprovementFailed {
        error: String,
    },

    // ── Skill activation ───────────────────────────────────────
    SkillActivated {
        skill_names: Vec<String>,
    },

    // ── Code execution instrumentation ────────────────────────
    /// Emitted when a code (REPL) execution attempt fails. Enables aggregate
    /// analysis of code execution failure modes to determine whether the
    /// runtime (Monty), the LLM, or tool dispatch is the primary source of
    /// failures.
    CodeExecutionFailed {
        step_id: StepId,
        /// Classified failure category.
        category: crate::types::step::CodeExecutionFailure,
        /// The error message text (truncated to 500 chars).
        error: String,
        /// Hash of the Python code that was executed, for dedup/correlation.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        code_hash: Option<String>,
        /// Duration of the code execution attempt in milliseconds.
        #[serde(default)]
        duration_ms: u64,
    },

    // ── Orchestrator versioning ───────────────────────────────
    OrchestratorRollback {
        from_version: u64,
        to_version: u64,
        reason: String,
    },

    /// Unknown event kind — catch-all for forward compatibility during
    /// rolling deploys. Older binaries deserializing events written by
    /// newer binaries will produce this variant instead of failing.
    #[serde(other)]
    Unknown,
}

/// Build a truncated preview string from a tool output JSON value.
///
/// Returns `None` for null/empty outputs. Truncates to `max_len` chars
/// at a char boundary (never splits a multi-byte character).
pub fn truncate_output_preview(value: &serde_json::Value, max_len: usize) -> Option<String> {
    if value.is_null() {
        return None;
    }
    let s = match value {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    if s.is_empty() {
        return None;
    }
    if s.len() <= max_len {
        Some(s)
    } else {
        // Truncate at a char boundary
        let mut end = max_len;
        while !s.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        let mut truncated = s[..end].to_string();
        truncated.push_str("...");
        Some(truncated)
    }
}
