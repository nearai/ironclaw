//! Loop progress events, cancellation signals, their ports, and the
//! [`AgentLoopDriverHost`] blanket trait composing every loop host port.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_host_api::ids::CapabilityId;
use serde::{Deserialize, Serialize};

use crate::CapabilityActivityId;
use crate::run_profile::compaction::{CompactionInitiator, LoopCompactionPort};
use crate::run_profile::system_inference::SystemInferenceTaskId;

use ironclaw_host_api::result_meta::FailureKind;

use super::capability::LoopCapabilityPort;
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
        reason_kind: FailureKind,
        /// Bounded, host-authored sanitized failure summary (e.g. a builtin's
        /// `"invalid JSON: ..."` message) so the live per-tool UI card can show
        /// the real reason, not just the kind. Additive; `None` when no
        /// host-authored summary is available. Validated at construction, in
        /// lockstep with the sibling `LoopSafeSummary`-typed summary fields.
        safe_summary: Option<LoopSafeSummary>,
    },
    /// A recovery decision was applied to a failed model or capability
    /// operation. This is the durable numerator paired with failure
    /// milestones. `sequence` is stable across checkpoint replay so consumers
    /// can deduplicate an at-least-once physical append into one logical event.
    FailureRecovered {
        #[serde(default)]
        sequence: u64,
        stage: LoopRecoveryStage,
        class: LoopRecoveryClass,
        disposition: LoopRecoveryDisposition,
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
        #[serde(default, skip_serializing_if = "is_zero")]
        redacted_leak_count: u32,
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
            Self::FailureRecovered { .. } => "failure_recovered",
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

/// Loop stage that applied a recovery decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopRecoveryStage {
    Model,
    Capability,
}

impl LoopRecoveryStage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Model => "model",
            Self::Capability => "capability",
        }
    }
}

/// Stable recovery class used for durable recovery-rate dimensions.
///
/// Capability failures retain the canonical [`FailureKind`] rather than
/// copying its vocabulary into this crate. Model variants name the executor's
/// closed recovery buckets, which have no capability-failure equivalent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopRecoveryClass {
    Capability(FailureKind),
    ModelTransient,
    ModelContextOverflow,
    ModelContentFiltered,
    ModelInvalidOutput,
    ModelOutputTruncated,
    ModelUnavailable,
    ModelInternal,
    ModelStaleRequest,
    ModelUnauthorized,
    ModelCheckpointRejected,
    ModelTranscriptWriteFailed,
}

impl LoopRecoveryClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Capability(kind) => kind.as_str(),
            Self::ModelTransient => "model_transient",
            Self::ModelContextOverflow => "model_context_overflow",
            Self::ModelContentFiltered => "model_content_filtered",
            Self::ModelInvalidOutput => "model_invalid_output",
            Self::ModelOutputTruncated => "model_output_truncated",
            Self::ModelUnavailable => "model_unavailable",
            Self::ModelInternal => "model_internal",
            Self::ModelStaleRequest => "model_stale_request",
            Self::ModelUnauthorized => "model_unauthorized",
            Self::ModelCheckpointRejected => "model_checkpoint_rejected",
            Self::ModelTranscriptWriteFailed => "model_transcript_write_failed",
        }
    }
}

/// How the executor recovered from the failed attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopRecoveryDisposition {
    /// The failed operation will be re-issued, possibly after rebuilding the
    /// iteration or altering its prompt.
    Retried,
    /// The failure was surfaced as a typed model-visible observation so the
    /// model can choose a different action.
    ModelVisible,
}

impl LoopRecoveryDisposition {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Retried => "retried",
            Self::ModelVisible => "model_visible",
        }
    }
}

fn is_zero(value: &u32) -> bool {
    *value == 0
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
    /// Progress events are best-effort except
    /// [`LoopProgressEvent::FailureRecovered`], whose producer must propagate a
    /// failed append before continuing the recovery transition. Other progress
    /// failures must not invalidate already-completed durable work.
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
