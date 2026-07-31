//! Agent-specific child-result projection over generic process dependencies.

pub mod boot_recovery;
pub mod resolver;
pub mod store;

use chrono::{DateTime, Utc};
use ironclaw_host_api::ids::{CapabilityId, ThreadId};
use ironclaw_host_api::turn::{
    LoopResultRef, ReplyTargetBindingRef, SourceBindingRef, TurnGateRef, TurnRunId, TurnScope,
};
use ironclaw_loop_host::{SpawnSubagentMode, SubagentKindId};
use serde::{Deserialize, Serialize};

/// CAS state machine (§2): `Open -> Settled -> Drained`, `Open -> Abandoned`.
/// `Drained`/`Abandoned`-final edges are deleted (§2) — these states are
/// therefore transient on disk, never the long-lived resting state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AwaitEdgeState {
    Open,
    Settled,
    Drained,
    Abandoned,
}

/// Descendant-reservation release tri-state (§5.5), living on the same edge
/// file as `AwaitEdgeState` — one more CAS'd field, not a second file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReservationReleaseState {
    Unclaimed,
    Released,
}

/// The child run's terminal outcome, set in the same CAS write that
/// transitions the edge `Open -> Settled` (§5.4's `terminal_byte_len` sits
/// alongside it in that same write).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeTerminalKind {
    Completed,
    Failed,
    Cancelled,
    RecoveryRequired,
}

impl EdgeTerminalKind {
    pub fn from_status(status: ironclaw_turns::TurnStatus) -> Option<Self> {
        use ironclaw_turns::TurnStatus;
        match status {
            TurnStatus::Completed => Some(Self::Completed),
            TurnStatus::Failed => Some(Self::Failed),
            TurnStatus::Cancelled => Some(Self::Cancelled),
            TurnStatus::RecoveryRequired => Some(Self::RecoveryRequired),
            _ => None,
        }
    }

    pub fn to_status(self) -> ironclaw_turns::TurnStatus {
        use ironclaw_turns::TurnStatus;
        match self {
            Self::Completed => TurnStatus::Completed,
            Self::Failed => TurnStatus::Failed,
            Self::Cancelled => TurnStatus::Cancelled,
            Self::RecoveryRequired => TurnStatus::RecoveryRequired,
        }
    }
}

/// One await-edge: parent-awaits-child bookkeeping, §5.6 assembled — plus
/// four additive fields beyond the design doc's exact list (`gate_ref`, the
/// `source_binding_ref`/`reply_target_binding_ref` pair, `parent_run_context`,
/// and `terminal_reason`), each named as a spec deviation in the PR:
///
/// - `gate_ref` (D3): the pre-existing shared-batch-gate mechanism (one
///   `TurnGateRef` covering N children spawned in one call, parent resumes once
///   after the *last* sibling settles — live behavior, pinned by the
///   un-ignored e2e test `parallel_blocking_spawn_resumes_once_after_last_child`)
///   has no analog in the design doc's per-`(parent,child)` edge model. Sibling
///   edges under the same `parent_run_id` sharing this field are one
///   settle-group (`resolver.rs`); listing is a cheap list+filter under the
///   ≤4-spawns/turn, ≤16-descendants caps this ever sees.
/// - `source_binding_ref`/`reply_target_binding_ref`: these are pure
///   deterministic functions of `(parent_run_id, child_run_id)` at spawn time
///   (`ironclaw_loop_host::subagent_spawn_port`'s private `source_binding_ref`/
///   `reply_target_binding_ref` helpers) — stored here rather than
///   recomputed, to avoid duplicating that private format-string logic
///   across the crate boundary and the drift risk of two copies going stale
///   independently.
///
/// Identity (`parent_run_id`, `child_run_id`) lives in the path (§4.2), not
/// here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AwaitEdge {
    pub child_scope: TurnScope,
    pub child_thread_id: ThreadId,
    pub parent_thread_id: ThreadId,
    /// The parent's `LoopRunContext`, captured once at open time (from
    /// `AwaitedChildSetRecord.parent_run_context`, spawn-time-fresh) — a
    /// third additive field beyond §5.6's list, and a deviation found only
    /// at implementation/test time (not caught in review): re-fetching the
    /// parent's `TurnRunRecord` from `agent_turn_runtime` at settle time, from
    /// *inside* the synchronous `TurnCommittedEventObserver` callback the
    /// child's own commit invokes, deadlocks — the store's commit path holds
    /// a lock across observer dispatch, and a second `get_run_record` call
    /// for a *different* run_id re-enters it. Storing the already-resolved
    /// context avoids the re-entrant call entirely. `resolver::reconstruct_edge`
    /// closes the same deadlock class for the recovery path: it sources this
    /// field from `SubagentThreadMetadata.parent_run_context` instead, with
    /// zero live `agent_turn_runtime` lookup for the parent.
    pub parent_run_context: ironclaw_turns::run_profile::LoopRunContext,
    pub tree_root_run_id: TurnRunId,
    pub gate_ref: TurnGateRef,
    pub source_binding_ref: SourceBindingRef,
    pub reply_target_binding_ref: ReplyTargetBindingRef,
    pub subagent_kind: SubagentKindId,
    pub spawn_capability_id: CapabilityId,
    pub result_ref: LoopResultRef,
    pub mode: SpawnSubagentMode,
    pub state: AwaitEdgeState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_kind: Option<EdgeTerminalKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_byte_len: Option<u64>,
    /// The settling child's own sanitized failure category (mirrors
    /// `TurnLifecycleEvent::sanitized_reason`), set in the same `settle()`
    /// CAS write as `terminal_kind`. Exists so a D3 batch-gate group's drain
    /// loop can read each member's own terminal state off its own edge
    /// instead of misattributing the triggering sibling's status/reason to
    /// every member (external review finding on this PR — see
    /// `resolver.rs`'s `drain_settled_group`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_reason: Option<String>,
    pub reservation_release: ReservationReleaseState,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settled_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AwaitEdgeStoreError {
    #[error("await-edge store backend failed: {reason}")]
    Backend { reason: String },
}

pub(crate) fn map_await_edge_error(
    error: AwaitEdgeStoreError,
) -> ironclaw_turns::run_profile::AgentLoopHostError {
    use ironclaw_turns::run_profile::{AgentLoopHostError, AgentLoopHostErrorKind};
    AgentLoopHostError::new(AgentLoopHostErrorKind::Unavailable, error.to_string())
}
