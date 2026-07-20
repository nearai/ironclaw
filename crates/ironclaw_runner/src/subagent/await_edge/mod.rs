//! Subagent await-edge delivery — the CAS'd filesystem replacement for the
//! in-memory `BoundedSubagentGateResolutionStore` (deleted with this module's
//! introduction). See `docs/reborn/subagent-spawn/thread-harness-design.md`
//! for the full design; `§13`'s P1.x rows are this module's scope (blocking
//! mode only — background mode, the CLI `subagent edges` command, and
//! `subagent_inspect`/`subagent_extend` are later PRs).
//!
//! Module split (post plan-review, avoids reproducing `completion_observer.rs`'s
//! own giant-file problem): `mod.rs` (this file) owns the `AwaitEdge` payload,
//! its state enums, and path construction. `roster.rs` owns the scope roster
//! (§4.5 — 256-way shard, percent-encoded key). `store.rs` owns the CAS
//! primitives over both. `resolver.rs` owns the per-child/per-group settle
//! path (§2, §5.2, §5.5, §8.1) — the direct successor to
//! `SubagentCompletionObserver`. `boot_recovery.rs` owns the roster-driven
//! boot pass, the bounded-concurrency admission scheduler, and the lazy
//! backstop (§4.3, §5.3).

// The concrete CAS mechanism needs `ironclaw_filesystem`. `SubagentSpawnDeps`
// holds these as `Arc<dyn AwaitEdgeWriter>` / `Arc<dyn
// TurnCommittedEventObserver>` trait objects (loop_host/ironclaw_turns types).
pub mod boot_recovery;
pub mod resolver;
pub mod roster;
pub mod store;

use chrono::{DateTime, Utc};
use ironclaw_host_api::{CapabilityId, ThreadId};
use ironclaw_loop_host::{SpawnSubagentMode, SubagentKindId};
use ironclaw_turns::{
    GateRef, LoopResultRef, ReplyTargetBindingRef, SourceBindingRef, TurnRunId, TurnScope,
};
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
    Claimed,
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
///   `GateRef` covering N children spawned in one call, parent resumes once
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
    /// parent's `TurnRunRecord` from `turn_state_store` at settle time, from
    /// *inside* the synchronous `TurnCommittedEventObserver` callback the
    /// child's own commit invokes, deadlocks — the store's commit path holds
    /// a lock across observer dispatch, and a second `get_run_record` call
    /// for a *different* run_id re-enters it. Storing the already-resolved
    /// context avoids the re-entrant call entirely. `resolver::reconstruct_edge`
    /// closes the same deadlock class for the recovery path: it sources this
    /// field from `SubagentThreadMetadata.parent_run_context` instead, with
    /// zero live `turn_state_store` lookup for the parent.
    pub parent_run_context: ironclaw_turns::run_profile::LoopRunContext,
    pub tree_root_run_id: TurnRunId,
    pub gate_ref: GateRef,
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

/// Domain error for await-edge store operations. Follows the
/// `SubagentGoalStoreError`/`map_goal_error` convention in `goal_store.rs`
/// (the actual local precedent — `gate_resolution.rs`, which this module
/// replaces, had no domain error type of its own).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AwaitEdgeStoreError {
    #[error("await-edge for parent {parent_run_id} child {child_run_id} not found")]
    NotFound {
        parent_run_id: TurnRunId,
        child_run_id: TurnRunId,
    },
    #[error(
        "await-edge for parent {parent_run_id} child {child_run_id} version mismatch (concurrent CAS)"
    )]
    VersionMismatch {
        parent_run_id: TurnRunId,
        child_run_id: TurnRunId,
    },
    #[error(
        "await-edge invalid transition for parent {parent_run_id} child {child_run_id}: {reason}"
    )]
    InvalidTransition {
        parent_run_id: TurnRunId,
        child_run_id: TurnRunId,
        reason: String,
    },
    #[error("await-edge store backend failed: {reason}")]
    Backend { reason: String },
}

pub(crate) fn map_await_edge_error(
    error: AwaitEdgeStoreError,
) -> ironclaw_turns::run_profile::AgentLoopHostError {
    use ironclaw_turns::run_profile::{AgentLoopHostError, AgentLoopHostErrorKind};
    AgentLoopHostError::new(AgentLoopHostErrorKind::Unavailable, error.to_string())
}

/// `{some/<v>|none}` optional-axis path encoding (§4.2), matching the
/// `agents/<id>/projects/<id>` scope-path convention `goal_store.rs` uses.
/// A local 4-line pure helper rather than a shared dependency — the encoding
/// is trivial and each store owns its own path layout.
fn optional_axis_path(value: Option<&str>) -> String {
    match value {
        Some(value) => format!("some/{value}"),
        None => "none".to_string(),
    }
}

/// §4.2's canonical await-edge path, scope-relative (the mount rewrites
/// tenant/user; the agent/project axes are ordinary path segments beneath
/// it, per §4.5a).
pub(crate) fn edge_path(
    agent_id: Option<&str>,
    project_id: Option<&str>,
    parent_run_id: TurnRunId,
    child_run_id: TurnRunId,
) -> Result<ironclaw_host_api::ScopedPath, AwaitEdgeStoreError> {
    ironclaw_host_api::ScopedPath::new(format!(
        "{}/{}.json",
        edge_dir_for_parent(agent_id, project_id, parent_run_id)?.as_str(),
        child_run_id.as_uuid()
    ))
    .map_err(|error| AwaitEdgeStoreError::Backend {
        reason: format!("invalid await-edge path: {error}"),
    })
}

/// The directory holding every child edge for one parent run — the natural
/// listing unit `list_parents_with_unclosed_edges` (§4.3) and D3's
/// gate-group listing both use.
pub(crate) fn edge_dir_for_parent(
    agent_id: Option<&str>,
    project_id: Option<&str>,
    parent_run_id: TurnRunId,
) -> Result<ironclaw_host_api::ScopedPath, AwaitEdgeStoreError> {
    ironclaw_host_api::ScopedPath::new(format!(
        "{}/{}",
        edge_scope_root(agent_id, project_id)?.as_str(),
        parent_run_id.as_uuid(),
    ))
    .map_err(|error| AwaitEdgeStoreError::Backend {
        reason: format!("invalid await-edge parent directory: {error}"),
    })
}

/// The scope-isolated root under which every parent's edge directory for
/// this `(tenant, user, agent, project)` scope lives — the prefix
/// `list_parents_with_unclosed_edges` (§4.3) walks. Takes the raw agent/
/// project axis values (not a full `TurnScope`) so callers that only have
/// those two values in hand (e.g. a decoded [`roster::RosterKey`]) don't
/// need to fabricate an unrelated `ThreadId` just to build a `TurnScope`.
pub(crate) fn edge_scope_root(
    agent_id: Option<&str>,
    project_id: Option<&str>,
) -> Result<ironclaw_host_api::ScopedPath, AwaitEdgeStoreError> {
    ironclaw_host_api::ScopedPath::new(format!(
        "/turns/subagent-await-edges/agents/{}/projects/{}",
        optional_axis_path(agent_id),
        optional_axis_path(project_id),
    ))
    .map_err(|error| AwaitEdgeStoreError::Backend {
        reason: format!("invalid await-edge scope root: {error}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edge_path_encodes_optional_axes_as_some_none() {
        let parent = TurnRunId::new();
        let child = TurnRunId::new();

        let with_path = edge_path(Some("agent-1"), Some("project-1"), parent, child).unwrap();
        let without_path = edge_path(None, None, parent, child).unwrap();

        assert!(with_path.as_str().contains("agents/some/agent-1/"));
        assert!(with_path.as_str().contains("projects/some/project-1/"));
        assert!(without_path.as_str().contains("agents/none/"));
        assert!(without_path.as_str().contains("projects/none/"));
        assert_ne!(with_path.as_str(), without_path.as_str());
    }

    #[test]
    fn edge_dir_for_parent_is_the_edge_paths_prefix() {
        let parent = TurnRunId::new();
        let child = TurnRunId::new();
        let dir = edge_dir_for_parent(Some("agent-1"), None, parent).unwrap();
        let path = edge_path(Some("agent-1"), None, parent, child).unwrap();
        assert!(path.as_str().starts_with(dir.as_str()));
    }
}
