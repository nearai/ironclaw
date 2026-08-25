//! Agent-specific child-result projection over generic process dependencies.
//!
//! `§`-references in this module's doc comments cite
//! `docs/internal/reborn/subagent-spawn/README.md`, the canonical subagent
//! design/roadmap document.

pub mod boot_recovery;
pub mod resolver;
pub mod store;

use chrono::{DateTime, Utc};
use ironclaw_host_api::ids::{CapabilityId, ThreadId};
use ironclaw_host_api::turn::{LoopMessageRef, LoopResultRef, TurnGateRef, TurnRunId, TurnScope};
use ironclaw_loop_host::{SpawnSubagentMode, SubagentKindId};
use serde::{Deserialize, Serialize};

/// CAS state machine (§2.2): `Open -> Settled -> Drained`; abandon reaches any
/// non-terminal state, not just `Open` — the kernel's close-dependency guard
/// only gates the `consume` door.
/// A background child's result also walks the delivery chain in between:
/// `Settled -> ResultAppended -> AttentionScheduled`, with
/// `ResultAppended -> AttentionDeferredStreakCap -> AttentionScheduled` when a
/// streak cap parks it. Only `AttentionScheduled` rejoins the closing path, so
/// the parked state is a detour, never a terminus.
/// `Drained`/`Abandoned`-final edges are deleted (§2.2) — these states are
/// therefore transient on disk, never the long-lived resting state.
///
/// This enum is a *projection* of the kernel's `ProcessDependencyState`, which
/// stays domain-neutral: the kernel's `AttentionDeferred` is spelled
/// `AttentionDeferredStreakCap` here because the loop tier is the layer that
/// knows what a streak cap is. The names differ on purpose; neither side
/// renames to match the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AwaitEdgeState {
    Open,
    Settled,
    /// The settled child's result is durably appended to the parent thread.
    ResultAppended,
    /// The parent has been made attentive to that appended result.
    AttentionScheduled,
    /// Attention was withheld on purpose because the parent hit its
    /// consecutive-interruption cap. In flight, not closed: the edge stays
    /// claimable, and the next permitted or human-initiated run start drains it
    /// forward into `AttentionScheduled` (§4.1/§4.2). It is deliberately not
    /// consumable from here — consuming would strand the undelivered result;
    /// abandoning it is still allowed.
    AttentionDeferredStreakCap,
    Drained,
    Abandoned,
}

/// How the parent was made attentive to an appended background result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttentionOutcome {
    /// The parent was mid-run; the result waits in its steering queue.
    Queued,
    /// The parent was idle; attention started a run.
    Activated,
}

/// Descendant-reservation release tri-state (§2.2), living on the same edge
/// file as `AwaitEdgeState` — one more CAS'd field, not a second file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReservationReleaseState {
    Unclaimed,
    Released,
}

/// The child run's terminal outcome, set in the same CAS write that
/// transitions the edge `Open -> Settled` (§2.2's `terminal_byte_len` sits
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

/// One await-edge: parent-awaits-child bookkeeping, §2.6 assembled — plus
/// additive fields beyond the design doc's exact list (`gate_ref`,
/// `parent_run_context`, `spawn_provider_call_id`, and `terminal_reason`),
/// each named as a spec deviation in the PR:
///
/// - `gate_ref` (D3): the pre-existing shared-batch-gate mechanism (one
///   `TurnGateRef` covering N children spawned in one call, parent resumes once
///   after the *last* sibling settles — live behavior, pinned by the
///   un-ignored e2e test `parallel_blocking_spawn_resumes_once_after_last_child`)
///   has no analog in the design doc's per-`(parent,child)` edge model. Sibling
///   edges under the same `parent_run_id` sharing this field are one
///   settle-group (`resolver.rs`); listing is a cheap list+filter under the
///   ≤4-spawns/turn, ≤16-descendants caps this ever sees.
/// - `spawn_provider_call_id`: pins settlement updates to the original spawn
///   transcript row when later `result_read` calls share its result reference.
///
/// Identity (`parent_run_id`, `child_run_id`) lives in the path (§2.2), not
/// here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AwaitEdge {
    pub child_scope: TurnScope,
    pub child_thread_id: ThreadId,
    pub parent_thread_id: ThreadId,
    /// The parent's `LoopRunContext`, captured once at open time (from
    /// `AwaitedChildSetRecord.parent_run_context`, spawn-time-fresh) — a
    /// third additive field beyond §2.6's list, and a deviation found only
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
    pub parent_run_context: ironclaw_loop_contracts::LoopRunContext,
    pub tree_root_run_id: TurnRunId,
    pub gate_ref: TurnGateRef,
    pub subagent_kind: SubagentKindId,
    pub spawn_capability_id: CapabilityId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawn_provider_call_id: Option<String>,
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
    /// The parent-thread message the child's result was appended as, recorded
    /// in the same CAS write that moves the edge to `ResultAppended`. It is the
    /// evidence that the append is durable, so a replayed append returns the
    /// ref already recorded instead of writing a second message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub appended_message_ref: Option<LoopMessageRef>,
    /// How attention was delivered, recorded in the same CAS write that moves
    /// the edge to `AttentionScheduled`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attention_outcome: Option<AttentionOutcome>,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settled_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AwaitEdgeStoreError {
    #[error("await-edge store backend failed: {reason}")]
    Backend { reason: String },
    #[error("await edge close refused: undelivered result still on the edge (state: {state:?})")]
    UndeliveredResult { state: AwaitEdgeState },
}

pub(crate) fn map_await_edge_error(
    error: AwaitEdgeStoreError,
) -> ironclaw_loop_contracts::AgentLoopHostError {
    use ironclaw_loop_contracts::{AgentLoopHostError, AgentLoopHostErrorKind};
    AgentLoopHostError::new(AgentLoopHostErrorKind::Unavailable, error.to_string())
}
