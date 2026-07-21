//! Loop progress events, cancellation signals, their ports, and the
//! [`AgentLoopDriverHost`] blanket trait composing every loop host port.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_host_api::CapabilityId;
use serde::{Deserialize, Serialize};

use crate::CapabilityActivityId;
use crate::run_profile::compaction::{CompactionInitiator, LoopCompactionPort};
use crate::run_profile::system_inference::SystemInferenceTaskId;

use super::capability::{CapabilityFailureKind, LoopCapabilityPort};
use super::checkpoint::{LoopCheckpointKind, LoopCheckpointPort};
use super::context::LoopContextPort;
use super::error::AgentLoopHostError;
use super::input::{LoopCancelReasonKind, LoopInputPort};
use super::model::{LoopModelPort, LoopPromptPort, PromptMode};
use super::refs::{CapabilitySurfaceVersion, LoopPromptBundleRef, LoopSafeSummary};
use super::run_context::LoopRunInfoPort;
use super::transcript::LoopTranscriptPort;

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopProgressEvent {
    DriverNote {
        kind: LoopDriverNoteKind,
        safe_summary: LoopSafeSummary,
    },
    IterationStarted {
        iteration: u32,
    },
    PromptBundleBuilt {
        iteration: u32,
        bundle_ref: LoopPromptBundleRef,
        mode: PromptMode,
        surface_version: Option<CapabilitySurfaceVersion>,
        message_count: u32,
        identity_message_count: u32,
        instruction_snippet_count: u32,
    },
    CapabilityBatchStarted {
        iteration: u32,
        call_count: u32,
        policy: BatchPolicyKind,
    },
    CapabilityBatchCompleted {
        iteration: u32,
        result_count: u32,
        denied_count: u32,
        gated_count: u32,
        failed_count: u32,
    },
    CapabilityActivityFailed {
        activity_id: CapabilityActivityId,
        capability_id: CapabilityId,
        reason_kind: CapabilityFailureKind,
        /// Bounded, host-authored sanitized failure summary (e.g. a builtin's
        /// `"invalid JSON: ..."` message) so the live per-tool UI card can show
        /// the real reason, not just the kind. Additive; `None` when no
        /// host-authored summary is available.
        safe_summary: Option<String>,
    },
    GateBlocked {
        iteration: u32,
        gate_kind: LoopGateKind,
    },
    CheckpointWritten {
        iteration: u32,
        kind: LoopCheckpointKind,
    },
    CompactionStarted {
        task_id: SystemInferenceTaskId,
        initiator: CompactionInitiator,
    },
    CompactionCompleted {
        task_id: SystemInferenceTaskId,
        compression_ratio_ppm: u32,
    },
    CompactionFailed {
        task_id: SystemInferenceTaskId,
        reason_kind: LoopSafeSummary,
    },
    CompactionLeakDetected {
        task_id: SystemInferenceTaskId,
        reason_kind: LoopSafeSummary,
    },
    GoalRefreshStarted {
        task_id: SystemInferenceTaskId,
    },
    GoalRefreshCompleted {
        task_id: SystemInferenceTaskId,
    },
    GoalRefreshFailed {
        task_id: SystemInferenceTaskId,
        reason_kind: LoopSafeSummary,
    },
    GoalRefreshLeakDetected {
        task_id: SystemInferenceTaskId,
        reason_kind: LoopSafeSummary,
    },
}

impl LoopProgressEvent {
    pub fn driver_note(
        kind: LoopDriverNoteKind,
        safe_summary: impl Into<String>,
    ) -> Result<Self, String> {
        Ok(Self::DriverNote {
            kind,
            safe_summary: LoopSafeSummary::new(safe_summary)?,
        })
    }

    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::DriverNote { .. } => "driver_note",
            Self::IterationStarted { .. } => "iteration_started",
            Self::PromptBundleBuilt { .. } => "prompt_bundle_built",
            Self::CapabilityBatchStarted { .. } => "capability_batch_started",
            Self::CapabilityBatchCompleted { .. } => "capability_batch_completed",
            Self::CapabilityActivityFailed { .. } => "capability_activity_failed",
            Self::GateBlocked { .. } => "gate_blocked",
            Self::CheckpointWritten { .. } => "checkpoint_written",
            Self::CompactionStarted { .. } => "compaction_started",
            Self::CompactionCompleted { .. } => "compaction_completed",
            Self::CompactionFailed { .. } => "compaction_failed",
            Self::CompactionLeakDetected { .. } => "compaction_leak_detected",
            Self::GoalRefreshStarted { .. } => "goal_refresh_started",
            Self::GoalRefreshCompleted { .. } => "goal_refresh_completed",
            Self::GoalRefreshFailed { .. } => "goal_refresh_failed",
            Self::GoalRefreshLeakDetected { .. } => "goal_refresh_leak_detected",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchPolicyKind {
    Sequential,
    Parallel,
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopGateKind {
    Approval,
    Auth,
    ResourceWait,
    AwaitDependentRun,
    ExternalTool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopDriverNoteKind {
    Context,
    Planning,
    Waiting,
    Retrying,
    /// An event-triggered hook subscription stopped before the run did —
    /// typically because the durable event log reported a replay gap that
    /// the subscription cannot bridge without losing events. Surfaced as
    /// an operator-visible note so the missing telemetry isn't silently
    /// invisible (NOTE(#3640)).
    EventSubscriptionTerminated,
}

#[async_trait]
pub trait LoopProgressPort: Send + Sync {
    /// Emit observational progress for UI/status consumers.
    ///
    /// Progress events are best-effort and must not be used as
    /// recoverability-critical durability markers. A failed progress emission
    /// must not invalidate already-completed durable work; callers should treat
    /// this like host model milestone projection, where sink failures are
    /// logged/observed without changing the provider or checkpoint outcome.
    async fn emit_loop_progress(&self, event: LoopProgressEvent) -> Result<(), AgentLoopHostError>;
}

/// Per-run cancellation observation point.
///
/// The canonical executor consults this between strategy calls. The method is
/// intentionally synchronous and non-blocking: implementations should expose a
/// cheap snapshot, usually backed by an atomic flag plus immutable signal data.
///
/// Cancellation is cooperative. Most executor stages observe it only at
/// explicit boundaries via [`LoopCancellationPort::observe_cancellation`].
/// Executor-owned waits that can safely race host work, such as prompt
/// compaction, may also wait on
/// [`LoopCancellationPort::cancellation_requested`] to avoid timer polling.
#[async_trait]
pub trait LoopCancellationPort: Send + Sync {
    /// Returns `Some(signal)` once cancellation has been requested for this run.
    ///
    /// Implementations must be idempotent across reads. After the request fires,
    /// repeated calls must keep returning the same signal.
    fn observe_cancellation(&self) -> Option<LoopCancellationSignal>;

    /// Waits until cancellation has been requested for this run and returns the
    /// same stable signal reported by [`Self::observe_cancellation`].
    async fn cancellation_requested(&self) -> LoopCancellationSignal;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopCancellationSignal {
    pub reason_kind: LoopCancelReasonKind,
    pub requested_at: DateTime<Utc>,
}

pub trait AgentLoopDriverHost:
    LoopRunInfoPort
    + LoopContextPort
    + LoopPromptPort
    + LoopInputPort
    + LoopModelPort
    + LoopCapabilityPort
    + LoopTranscriptPort
    + LoopCheckpointPort
    + LoopProgressPort
    + LoopCompactionPort
    + LoopCancellationPort
    + Send
    + Sync
{
}

impl<T> AgentLoopDriverHost for T where
    T: LoopRunInfoPort
        + LoopContextPort
        + LoopPromptPort
        + LoopInputPort
        + LoopModelPort
        + LoopCapabilityPort
        + LoopTranscriptPort
        + LoopCheckpointPort
        + LoopProgressPort
        + LoopCompactionPort
        + LoopCancellationPort
        + Send
        + Sync
{
}
