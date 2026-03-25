//! Event sourcing types.
//!
//! Every significant action within a thread is recorded as an event.
//! This enables replay, debugging, reflection, and trace-based testing.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::capability::LeaseId;
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
    },
    ActionFailed {
        step_id: StepId,
        action_name: String,
        call_id: String,
        error: String,
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
    },
    ApprovalReceived {
        call_id: String,
        approved: bool,
    },

    // ── Reflection ───────────────────────────────────────────
    ReflectionStarted,
    ReflectionComplete {
        docs_produced: usize,
        doc_types: Vec<String>,
        tokens_used: u64,
    },
    ReflectionFailed {
        error: String,
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
}
