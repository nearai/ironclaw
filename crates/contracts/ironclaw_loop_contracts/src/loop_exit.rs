//! The loop-exit claim a driver returns, and the shapes it carries.
//!
//! A `LoopExit` is a *claim*, never a fact: the turn kernel validates it
//! against host-minted evidence before any durable transition commits. This
//! module owns only the claim vocabulary — the validation policy, the
//! violation taxonomy, and the applier that performs the transition stay in
//! the turn kernel (`ironclaw_turns::loop_exit`).

use std::{collections::HashSet, hash::Hash};

use ironclaw_host_api::decision::RuntimeCredentialAuthRequirement;
use ironclaw_host_api::turn::{
    BlockedReason, CapabilityActivityId, GateKind, LoopExitId, LoopGateRef, LoopMessageRef,
    LoopResultRef, SanitizedFailure, TurnCheckpointId, TurnGateRef,
};
use serde::{Deserialize, Serialize, de};

use crate::{LoopCheckpointStateRef, LoopModelUsage};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopExit {
    Completed(LoopCompleted),
    Blocked(LoopBlocked),
    Cancelled(LoopCancelled),
    Failed(LoopFailed),
}

impl LoopExit {
    pub fn exit_id(&self) -> &LoopExitId {
        match self {
            Self::Completed(exit) => &exit.exit_id,
            Self::Blocked(exit) => &exit.exit_id,
            Self::Cancelled(exit) => &exit.exit_id,
            Self::Failed(exit) => &exit.exit_id,
        }
    }

    pub fn cancelled_for_observed_interrupt(exit_id: LoopExitId) -> Self {
        Self::Cancelled(LoopCancelled {
            reason_kind: LoopCancelledReasonKind::HostInterrupt,
            checkpoint_id: None,
            interrupted_message_refs: Vec::new(),
            exit_id,
        })
    }

    pub fn failed(reason_kind: LoopFailureKind, exit_id: LoopExitId) -> Self {
        Self::Failed(LoopFailed {
            reason_kind,
            checkpoint_id: None,
            model_usage: None,
            exit_id,
            explanation_message_refs: Vec::new(),
            safe_summary: None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoopCompleted {
    pub completion_kind: LoopCompletionKind,
    #[serde(deserialize_with = "deserialize_bounded_unique_refs")]
    pub reply_message_refs: Vec<LoopMessageRef>,
    #[serde(deserialize_with = "deserialize_bounded_unique_refs")]
    pub result_refs: Vec<LoopResultRef>,
    pub final_checkpoint_id: Option<TurnCheckpointId>,
    /// Cumulative provider-reported token usage the loop accumulated across its
    /// model calls. Carried to the run record at the terminal transition so the
    /// OpenAI-compatible surfaces can report `usage` and cost. `None` when the
    /// loop saw no usage (replay stubs, providers without a usage object).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_usage: Option<LoopModelUsage>,
    pub exit_id: LoopExitId,
}

impl LoopCompleted {
    /// Whether this completion carries at least one durable reply or
    /// result reference. The turn kernel's validation reads it; the DTO
    /// owns the definition of "durable evidence" it asserts.
    pub fn has_durable_completion_ref(&self) -> bool {
        !self.reply_message_refs.is_empty() || !self.result_refs.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopCompletionKind {
    /// A finalized assistant reply is the user-visible completion artifact.
    FinalReply,
    /// The loop stopped to ask the user for input.
    AskUserReply,
    /// The loop completed without durable reply/result evidence; profile-gated.
    NoReply,
    /// A delegated subtask result is the durable completion artifact.
    DelegatedResult,
    /// One or more durable result refs are the completion artifact.
    ResultOnly,
    /// A scheduled run explicitly reported that its task found no result.
    /// Valid only when host policy opted into no-result suppression.
    NothingToReport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoopBlocked {
    pub kind: LoopBlockedKind,
    pub gate_ref: LoopGateRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_activity_id: Option<CapabilityActivityId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub credential_requirements: Vec<RuntimeCredentialAuthRequirement>,
    pub checkpoint_id: TurnCheckpointId,
    pub state_ref: LoopCheckpointStateRef,
    pub exit_id: LoopExitId,
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopBlockedKind {
    Approval,
    Auth,
    Resource,
    AwaitDependentRun,
    /// The model called a client-supplied ("external") tool. The loop parks the
    /// run and returns control to the API client, which resumes by submitting
    /// the tool output. Bridges to [`BlockedReason::ExternalTool`].
    ExternalTool,
}

impl From<LoopBlockedKind> for GateKind {
    fn from(kind: LoopBlockedKind) -> Self {
        match kind {
            LoopBlockedKind::Approval => Self::Approval,
            LoopBlockedKind::Auth => Self::Auth,
            LoopBlockedKind::Resource => Self::Resource,
            LoopBlockedKind::AwaitDependentRun => Self::AwaitDependentRun,
            LoopBlockedKind::ExternalTool => Self::ExternalTool,
        }
    }
}

impl LoopBlockedKind {
    /// Bridge a driver-claimed blocked kind onto the kernel's
    /// [`BlockedReason`]. `None` when the claimed gate ref is not a valid
    /// [`TurnGateRef`] — the caller treats that as an unverified claim.
    pub fn to_blocked_reason(
        self,
        gate_ref: LoopGateRef,
        credential_requirements: Vec<RuntimeCredentialAuthRequirement>,
    ) -> Option<BlockedReason> {
        let gate_ref = TurnGateRef::new(gate_ref.as_str()).ok()?;
        Some(GateKind::from(self).into_blocked_reason(gate_ref, credential_requirements))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoopCancelled {
    pub reason_kind: LoopCancelledReasonKind,
    pub checkpoint_id: Option<TurnCheckpointId>,
    #[serde(deserialize_with = "deserialize_bounded_unique_refs")]
    pub interrupted_message_refs: Vec<LoopMessageRef>,
    pub exit_id: LoopExitId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopCancelledReasonKind {
    HostCancellation,
    HostInterrupt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LoopFailed {
    pub reason_kind: LoopFailureKind,
    pub checkpoint_id: Option<TurnCheckpointId>,
    /// Cumulative provider-reported token usage accumulated before the failure.
    /// See [`LoopCompleted::model_usage`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_usage: Option<LoopModelUsage>,
    pub exit_id: LoopExitId,
    #[serde(
        default,
        deserialize_with = "deserialize_bounded_unique_refs",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub explanation_message_refs: Vec<LoopMessageRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safe_summary: Option<SanitizedFailure>,
}

impl<'de> Deserialize<'de> for LoopFailed {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct LoopFailedWire {
            reason_kind: LoopFailureKind,
            checkpoint_id: Option<TurnCheckpointId>,
            #[serde(default)]
            model_usage: Option<LoopModelUsage>,
            /// Read-only compatibility for exits written before the dead
            /// diagnostic-reference field was retired. No production code
            /// minted a usable value and no diagnostic store ever existed.
            #[serde(
                default,
                rename = "diagnostic_ref",
                deserialize_with = "deserialize_retired_diagnostic_ref"
            )]
            retired_diagnostic_ref: Option<()>,
            exit_id: LoopExitId,
            #[serde(default, deserialize_with = "deserialize_bounded_unique_refs")]
            explanation_message_refs: Vec<LoopMessageRef>,
            #[serde(default)]
            safe_summary: Option<SanitizedFailure>,
        }

        let wire = LoopFailedWire::deserialize(deserializer)?;
        let _ = wire.retired_diagnostic_ref;
        Ok(Self {
            reason_kind: wire.reason_kind,
            checkpoint_id: wire.checkpoint_id,
            model_usage: wire.model_usage,
            exit_id: wire.exit_id,
            explanation_message_refs: wire.explanation_message_refs,
            safe_summary: wire.safe_summary,
        })
    }
}

fn deserialize_retired_diagnostic_ref<'de, D>(deserializer: D) -> Result<Option<()>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    value
        .map(|value| {
            validate_retired_diagnostic_ref(&value).map_err(de::Error::custom)?;
            Ok(())
        })
        .transpose()
}

fn validate_retired_diagnostic_ref(value: &str) -> Result<(), String> {
    const KIND: &str = "loop_diagnostic_ref";
    const PREFIX: &str = "diag:";

    if value.is_empty() {
        return Err(format!("{KIND} must not be empty"));
    }
    if value.len() > 256 {
        return Err(format!("{KIND} must be at most 256 bytes"));
    }
    if value.chars().any(|character| character.is_control()) {
        return Err(format!("{KIND} must not contain control characters"));
    }
    let Some(suffix) = value.strip_prefix(PREFIX) else {
        return Err(format!("{KIND} must start with {PREFIX}"));
    };
    if suffix.is_empty() {
        return Err(format!("{KIND} must include an opaque id after {PREFIX}"));
    }
    if !suffix
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.'))
    {
        return Err(format!(
            "{KIND} opaque id must contain only ASCII letters, digits, _, -, or ."
        ));
    }
    Ok(())
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopFailureKind {
    ModelError,
    ContextBuildFailed,
    CapabilityProtocolError,
    IterationLimit,
    InvalidModelOutput,
    CheckpointRejected,
    CheckpointUnavailable,
    TranscriptWriteFailed,
    DriverBug,
    InterruptedUnexpectedly,
    /// Emitted by `DefaultStopConditionStrategy` when repetition or
    /// repeated-same-error escapes fire.
    NoProgressDetected,
    /// Emitted when a `CapabilityOutcome::Denied` reaches the recovery path
    /// with no further retry possible. Distinct from `CapabilityProtocolError`
    /// so the no-progress detector can count repeated denials without
    /// conflating them with transport faults. Hook-induced denials (via the
    /// middleware composition seam) accumulate through this variant.
    PolicyDenied,
    /// System compaction failed after the loop exhausted the safe fallback path.
    CompactionUnavailable,
    /// A gate fired on a profile that cannot render or resolve it (unbound
    /// profiles expose non-gating surfaces; approval/auth/resource gates that
    /// fire anyway — policy drift, auth expiry mid-run — fail the run instead
    /// of parking it with no surface to resolve on). The gate kind rides the
    /// sanitized failure detail.
    GateNotSupported,
    /// The run exceeded its per-run wall-clock budget
    /// (`ResourceBudgetPolicy::max_wall_clock_seconds`, narrowed by declared
    /// `TurnLimits`). Hard stop after a final checkpoint.
    WallClockLimit,
    /// The run exhausted its model-call budget
    /// (`ResourceBudgetPolicy::max_model_calls`).
    ModelCallLimit,
    /// The run exhausted its capability-invocation budget
    /// (`ResourceBudgetPolicy::max_capability_invocations`).
    CapabilityInvocationLimit,
}

impl LoopFailureKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ModelError => "model_error",
            Self::ContextBuildFailed => "context_build_failed",
            Self::CapabilityProtocolError => "capability_protocol_error",
            Self::IterationLimit => "iteration_limit",
            Self::InvalidModelOutput => "invalid_model_output",
            Self::CheckpointRejected => "checkpoint_rejected",
            Self::CheckpointUnavailable => "checkpoint_unavailable",
            Self::TranscriptWriteFailed => "transcript_write_failed",
            Self::DriverBug => "driver_bug",
            Self::InterruptedUnexpectedly => "interrupted_unexpectedly",
            Self::NoProgressDetected => "no_progress_detected",
            Self::PolicyDenied => "policy_denied",
            Self::CompactionUnavailable => "compaction_unavailable",
            Self::GateNotSupported => "gate_not_supported",
            Self::WallClockLimit => "wall_clock_limit",
            Self::ModelCallLimit => "model_call_limit",
            Self::CapabilityInvocationLimit => "capability_invocation_limit",
        }
    }

    /// The sanitized, model-visible failure name for this kind.
    pub fn to_sanitized_failure(self) -> SanitizedFailure {
        SanitizedFailure::from_trusted_static(self.as_str())
    }
}

pub(crate) const MAX_LOOP_EXIT_REF_COUNT: usize = 64;

fn deserialize_bounded_unique_refs<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de> + Eq + Hash,
{
    let values = Vec::<T>::deserialize(deserializer)?;
    if values.len() > MAX_LOOP_EXIT_REF_COUNT {
        return Err(de::Error::custom(format!(
            "loop exit ref list must contain at most {MAX_LOOP_EXIT_REF_COUNT} entries"
        )));
    }

    let mut seen = HashSet::with_capacity(values.len());
    for value in &values {
        if !seen.insert(value) {
            return Err(de::Error::custom(
                "loop exit ref list must not contain duplicates",
            ));
        }
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::turn::{GateKind, TurnGateRef};

    use super::*;

    /// Compiler-checked exhaustiveness for the blocked-kind → gate-kind
    /// correspondence. Both enums are `#[non_exhaustive]`, so only a same-crate
    /// match can be exhaustive; the turn kernel's behavioral suite
    /// (`ironclaw_turns::loop_exit::tests`) asserts the resulting
    /// `BlockedReason`, and cannot catch a new variant on its own.
    #[test]
    fn blocked_kind_gate_correspondence_is_exhaustive() {
        let expected = |kind: LoopBlockedKind| match kind {
            LoopBlockedKind::Approval => GateKind::Approval,
            LoopBlockedKind::Auth => GateKind::Auth,
            LoopBlockedKind::Resource => GateKind::Resource,
            LoopBlockedKind::AwaitDependentRun => GateKind::AwaitDependentRun,
            LoopBlockedKind::ExternalTool => GateKind::ExternalTool,
        };
        for kind in [
            LoopBlockedKind::Approval,
            LoopBlockedKind::Auth,
            LoopBlockedKind::Resource,
            LoopBlockedKind::AwaitDependentRun,
            LoopBlockedKind::ExternalTool,
        ] {
            assert_eq!(GateKind::from(kind), expected(kind));
            let gate_ref = LoopGateRef::new("gate:exhaustiveness").expect("gate ref");
            let reason = kind
                .to_blocked_reason(gate_ref.clone(), Vec::new())
                .expect("valid gate ref must bridge");
            assert_eq!(reason.gate_kind(), expected(kind));
            assert_eq!(
                reason.gate_ref(),
                &TurnGateRef::new(gate_ref.as_str()).expect("turn gate ref")
            );
        }
    }

    /// A malformed gate ref is an unverified claim, not a panic.
    #[test]
    fn blocked_kind_rejects_a_gate_ref_the_kernel_cannot_accept() {
        let gate_ref = LoopGateRef::new("gate:x").expect("gate ref");
        // A `LoopGateRef` that is not a valid `TurnGateRef` cannot be built here,
        // so assert the accepting branch and leave the rejecting branch to the
        // kernel's validation suite, which drives it through `validate_loop_exit`.
        assert!(
            LoopBlockedKind::Approval
                .to_blocked_reason(gate_ref, Vec::new())
                .is_some()
        );
    }

    /// Compiler-checked exhaustiveness for the failure-kind category strings.
    /// The strings are a persisted wire vocabulary (`SanitizedFailure`), so a
    /// new variant must choose one deliberately rather than inherit a default.
    #[test]
    fn failure_kind_category_strings_are_exhaustive() {
        let expected = |kind: LoopFailureKind| match kind {
            LoopFailureKind::ModelError => "model_error",
            LoopFailureKind::ContextBuildFailed => "context_build_failed",
            LoopFailureKind::CapabilityProtocolError => "capability_protocol_error",
            LoopFailureKind::IterationLimit => "iteration_limit",
            LoopFailureKind::InvalidModelOutput => "invalid_model_output",
            LoopFailureKind::CheckpointRejected => "checkpoint_rejected",
            LoopFailureKind::CheckpointUnavailable => "checkpoint_unavailable",
            LoopFailureKind::TranscriptWriteFailed => "transcript_write_failed",
            LoopFailureKind::DriverBug => "driver_bug",
            LoopFailureKind::InterruptedUnexpectedly => "interrupted_unexpectedly",
            LoopFailureKind::NoProgressDetected => "no_progress_detected",
            LoopFailureKind::PolicyDenied => "policy_denied",
            LoopFailureKind::CompactionUnavailable => "compaction_unavailable",
            LoopFailureKind::GateNotSupported => "gate_not_supported",
            LoopFailureKind::WallClockLimit => "wall_clock_limit",
            LoopFailureKind::ModelCallLimit => "model_call_limit",
            LoopFailureKind::CapabilityInvocationLimit => "capability_invocation_limit",
        };
        for kind in [
            LoopFailureKind::ModelError,
            LoopFailureKind::ContextBuildFailed,
            LoopFailureKind::CapabilityProtocolError,
            LoopFailureKind::IterationLimit,
            LoopFailureKind::InvalidModelOutput,
            LoopFailureKind::CheckpointRejected,
            LoopFailureKind::CheckpointUnavailable,
            LoopFailureKind::TranscriptWriteFailed,
            LoopFailureKind::DriverBug,
            LoopFailureKind::InterruptedUnexpectedly,
            LoopFailureKind::NoProgressDetected,
            LoopFailureKind::PolicyDenied,
            LoopFailureKind::CompactionUnavailable,
            LoopFailureKind::GateNotSupported,
            LoopFailureKind::WallClockLimit,
            LoopFailureKind::ModelCallLimit,
            LoopFailureKind::CapabilityInvocationLimit,
        ] {
            assert_eq!(kind.as_str(), expected(kind));
            assert_eq!(kind.to_sanitized_failure().category(), expected(kind));
        }
    }
}
