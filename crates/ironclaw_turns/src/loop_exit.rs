use std::{collections::HashSet, hash::Hash};

use serde::{Deserialize, Serialize, de};

use crate::{
    BlockedReason, GateRef, LoopDiagnosticRef, LoopExitId, LoopGateRef, LoopMessageRef,
    LoopResultRef, LoopUsageSummaryRef, SanitizedFailure, TurnCheckpointId,
    run_profile::LoopProcessRef, runner::TurnRunnerOutcome,
};

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

    pub fn validate(self, policy: LoopExitValidationPolicy) -> LoopExitValidationDecision {
        let exit_id = self.exit_id().clone();
        match self {
            Self::Completed(exit) => validate_completed_exit(exit_id, exit, policy),
            Self::Blocked(exit) => validate_blocked_exit(exit_id, exit, policy),
            Self::Cancelled(exit) => validate_cancelled_exit(exit_id, exit, policy),
            Self::Failed(exit) => validate_failed_exit(exit_id, exit, policy),
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
            usage_summary_ref: None,
            diagnostic_ref: None,
            exit_id,
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
    pub usage_summary_ref: Option<LoopUsageSummaryRef>,
    pub exit_id: LoopExitId,
}

impl LoopCompleted {
    fn has_durable_completion_ref(&self) -> bool {
        !self.reply_message_refs.is_empty() || !self.result_refs.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopCompletionKind {
    FinalReply,
    AskUserReply,
    NoReply,
    DelegatedResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoopBlocked {
    pub kind: LoopBlockedKind,
    /// Durable suspension reference. Approval/auth/resource blocks must use a
    /// `gate:` loop ref; process blocks must use a `process:` loop ref.
    pub gate_ref: GateRef,
    pub checkpoint_id: TurnCheckpointId,
    pub exit_id: LoopExitId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopBlockedKind {
    Approval,
    Auth,
    Resource,
    /// Spawned process suspension — maps to `BlockedReason::Process`.
    Process,
}

impl LoopBlockedKind {
    fn to_blocked_reason(self, gate_ref: GateRef) -> Result<BlockedReason, ()> {
        match self {
            Self::Approval => {
                require_loop_gate_ref(&gate_ref)?;
                Ok(BlockedReason::Approval { gate_ref })
            }
            Self::Auth => {
                require_loop_gate_ref(&gate_ref)?;
                Ok(BlockedReason::Auth { gate_ref })
            }
            Self::Resource => {
                require_loop_gate_ref(&gate_ref)?;
                Ok(BlockedReason::Resource { gate_ref })
            }
            Self::Process => {
                require_loop_process_ref(&gate_ref)?;
                Ok(BlockedReason::Process { gate_ref })
            }
        }
    }
}

fn require_loop_gate_ref(gate_ref: &GateRef) -> Result<(), ()> {
    LoopGateRef::new(gate_ref.as_str())
        .map(|_| ())
        .map_err(|_| ())
}

fn require_loop_process_ref(gate_ref: &GateRef) -> Result<(), ()> {
    LoopProcessRef::new(gate_ref.as_str())
        .map(|_| ())
        .map_err(|_| ())
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoopFailed {
    pub reason_kind: LoopFailureKind,
    pub checkpoint_id: Option<TurnCheckpointId>,
    pub usage_summary_ref: Option<LoopUsageSummaryRef>,
    pub diagnostic_ref: Option<LoopDiagnosticRef>,
    pub exit_id: LoopExitId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopFailureKind {
    ModelError,
    ContextBuildFailed,
    CapabilityProtocolError,
    IterationLimit,
    InvalidModelOutput,
    CheckpointRejected,
    TranscriptWriteFailed,
    DriverBug,
    InterruptedUnexpectedly,
}

impl LoopFailureKind {
    fn to_sanitized_failure(self) -> SanitizedFailure {
        SanitizedFailure::from_trusted_static(match self {
            Self::ModelError => "model_error",
            Self::ContextBuildFailed => "context_build_failed",
            Self::CapabilityProtocolError => "capability_protocol_error",
            Self::IterationLimit => "iteration_limit",
            Self::InvalidModelOutput => "invalid_model_output",
            Self::CheckpointRejected => "checkpoint_rejected",
            Self::TranscriptWriteFailed => "transcript_write_failed",
            Self::DriverBug => "driver_bug",
            Self::InterruptedUnexpectedly => "interrupted_unexpectedly",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopExitInvalidHandling {
    FailTerminal,
    RecoveryRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoopExitValidationPolicy {
    pub require_final_checkpoint: bool,
    #[serde(default)]
    pub final_checkpoint_verified: bool,
    pub host_cancellation_observed: bool,
    pub invalid_handling: LoopExitInvalidHandling,
    pub completion_refs_verified: bool,
    pub blocked_evidence_verified: bool,
    pub failure_evidence_verified: bool,
}

impl Default for LoopExitValidationPolicy {
    fn default() -> Self {
        Self {
            require_final_checkpoint: false,
            final_checkpoint_verified: false,
            host_cancellation_observed: false,
            invalid_handling: LoopExitInvalidHandling::RecoveryRequired,
            completion_refs_verified: false,
            blocked_evidence_verified: false,
            failure_evidence_verified: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoopExitValidationDecision {
    pub exit_id: LoopExitId,
    pub mapping: LoopExitMapping,
    pub violation: Option<LoopExitViolation>,
}

impl LoopExitValidationDecision {
    fn trusted(exit_id: LoopExitId, outcome: TurnRunnerOutcome) -> Self {
        Self {
            exit_id,
            mapping: outcome.into(),
            violation: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopExitMapping {
    RunnerOutcome(TurnRunnerOutcome),
    RecoveryRequired { failure: SanitizedFailure },
}

impl From<TurnRunnerOutcome> for LoopExitMapping {
    fn from(outcome: TurnRunnerOutcome) -> Self {
        Self::RunnerOutcome(outcome)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoopExitViolation {
    kind: LoopExitViolationKind,
}

impl LoopExitViolation {
    pub fn kind(&self) -> LoopExitViolationKind {
        self.kind
    }

    pub fn category(&self) -> &'static str {
        self.kind.category()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopExitViolationKind {
    MissingCompletionReference,
    UnverifiedCompletionReference,
    MissingFinalCheckpoint,
    UnverifiedFinalCheckpoint,
    UnverifiedBlockedEvidence,
    UnverifiedFailureEvidence,
    CancellationNotObserved,
}

impl LoopExitViolationKind {
    fn category(self) -> &'static str {
        match self {
            Self::MissingCompletionReference => "missing_completion_reference",
            Self::UnverifiedCompletionReference => "unverified_completion_reference",
            Self::MissingFinalCheckpoint => "missing_final_checkpoint",
            Self::UnverifiedFinalCheckpoint => "unverified_final_checkpoint",
            Self::UnverifiedBlockedEvidence => "unverified_blocked_evidence",
            Self::UnverifiedFailureEvidence => "unverified_failure_evidence",
            Self::CancellationNotObserved => "cancellation_not_observed",
        }
    }

    fn failure_category(self) -> &'static str {
        match self {
            Self::CancellationNotObserved => "interrupted_unexpectedly",
            Self::MissingCompletionReference
            | Self::UnverifiedCompletionReference
            | Self::MissingFinalCheckpoint
            | Self::UnverifiedFinalCheckpoint
            | Self::UnverifiedBlockedEvidence
            | Self::UnverifiedFailureEvidence => "driver_protocol_violation",
        }
    }
}

const MAX_LOOP_EXIT_REF_COUNT: usize = 64;

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

fn validate_completed_exit(
    exit_id: LoopExitId,
    exit: LoopCompleted,
    policy: LoopExitValidationPolicy,
) -> LoopExitValidationDecision {
    if !exit.has_durable_completion_ref() {
        return invalid_exit_decision(
            exit_id,
            LoopExitViolationKind::MissingCompletionReference,
            policy.invalid_handling,
        );
    }

    if !policy.completion_refs_verified {
        return invalid_exit_decision(
            exit_id,
            LoopExitViolationKind::UnverifiedCompletionReference,
            policy.invalid_handling,
        );
    }

    if let Some(decision) =
        final_checkpoint_violation(&exit_id, exit.final_checkpoint_id.is_some(), policy)
    {
        return decision;
    }

    LoopExitValidationDecision::trusted(exit_id, TurnRunnerOutcome::Completed)
}

fn validate_blocked_exit(
    exit_id: LoopExitId,
    exit: LoopBlocked,
    policy: LoopExitValidationPolicy,
) -> LoopExitValidationDecision {
    if !policy.blocked_evidence_verified {
        return invalid_exit_decision(
            exit_id,
            LoopExitViolationKind::UnverifiedBlockedEvidence,
            policy.invalid_handling,
        );
    }

    match exit.kind.to_blocked_reason(exit.gate_ref) {
        Ok(reason) => LoopExitValidationDecision::trusted(
            exit_id,
            TurnRunnerOutcome::Blocked {
                checkpoint_id: exit.checkpoint_id,
                reason,
            },
        ),
        Err(()) => invalid_exit_decision(
            exit_id,
            LoopExitViolationKind::UnverifiedBlockedEvidence,
            policy.invalid_handling,
        ),
    }
}

fn validate_cancelled_exit(
    exit_id: LoopExitId,
    exit: LoopCancelled,
    policy: LoopExitValidationPolicy,
) -> LoopExitValidationDecision {
    if !policy.host_cancellation_observed {
        return invalid_exit_decision(
            exit_id,
            LoopExitViolationKind::CancellationNotObserved,
            policy.invalid_handling,
        );
    }

    if let Some(decision) =
        final_checkpoint_violation(&exit_id, exit.checkpoint_id.is_some(), policy)
    {
        return decision;
    }

    LoopExitValidationDecision::trusted(exit_id, TurnRunnerOutcome::Cancelled)
}

fn validate_failed_exit(
    exit_id: LoopExitId,
    exit: LoopFailed,
    policy: LoopExitValidationPolicy,
) -> LoopExitValidationDecision {
    if !policy.failure_evidence_verified {
        return invalid_exit_decision(
            exit_id,
            LoopExitViolationKind::UnverifiedFailureEvidence,
            policy.invalid_handling,
        );
    }

    if let Some(decision) =
        final_checkpoint_violation(&exit_id, exit.checkpoint_id.is_some(), policy)
    {
        return decision;
    }

    LoopExitValidationDecision::trusted(
        exit_id,
        TurnRunnerOutcome::Failed {
            failure: exit.reason_kind.to_sanitized_failure(),
        },
    )
}

fn final_checkpoint_violation(
    exit_id: &LoopExitId,
    checkpoint_present: bool,
    policy: LoopExitValidationPolicy,
) -> Option<LoopExitValidationDecision> {
    if !policy.require_final_checkpoint {
        return None;
    }
    if !checkpoint_present {
        return Some(invalid_exit_decision(
            exit_id.clone(),
            LoopExitViolationKind::MissingFinalCheckpoint,
            policy.invalid_handling,
        ));
    }
    if !policy.final_checkpoint_verified {
        return Some(invalid_exit_decision(
            exit_id.clone(),
            LoopExitViolationKind::UnverifiedFinalCheckpoint,
            policy.invalid_handling,
        ));
    }
    None
}

fn invalid_exit_decision(
    exit_id: LoopExitId,
    kind: LoopExitViolationKind,
    handling: LoopExitInvalidHandling,
) -> LoopExitValidationDecision {
    let failure = SanitizedFailure::from_trusted_static(kind.failure_category());
    let mapping = match handling {
        LoopExitInvalidHandling::FailTerminal => TurnRunnerOutcome::Failed { failure }.into(),
        LoopExitInvalidHandling::RecoveryRequired => LoopExitMapping::RecoveryRequired { failure },
    };

    LoopExitValidationDecision {
        exit_id,
        mapping,
        violation: Some(LoopExitViolation { kind }),
    }
}
