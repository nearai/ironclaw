use std::{collections::HashSet, hash::Hash};

use serde::{Deserialize, Serialize, de};

use crate::{
    BlockedReason, GateRef, LoopDiagnosticRef, LoopExitId, LoopGateRef, LoopMessageRef,
    LoopResultRef, LoopUsageSummaryRef, SanitizedFailure, TurnCheckpointId,
    runner::TurnRunnerOutcome,
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
            Self::Blocked(exit) if policy.blocked_evidence_verified => {
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
            Self::Blocked(_exit) => invalid_exit_decision(
                exit_id,
                LoopExitViolationKind::UnverifiedBlockedEvidence,
                policy.invalid_handling,
            ),
            Self::Cancelled(_exit) if policy.host_cancellation_observed => {
                LoopExitValidationDecision::trusted(exit_id, TurnRunnerOutcome::Cancelled)
            }
            Self::Cancelled(_exit) => invalid_exit_decision(
                exit_id,
                LoopExitViolationKind::CancellationNotObserved,
                policy.invalid_handling,
            ),
            Self::Failed(exit) if policy.failure_evidence_verified => {
                LoopExitValidationDecision::trusted(
                    exit_id,
                    TurnRunnerOutcome::Failed {
                        failure: exit.reason_kind.to_sanitized_failure(),
                    },
                )
            }
            Self::Failed(_exit) => invalid_exit_decision(
                exit_id,
                LoopExitViolationKind::UnverifiedFailureEvidence,
                policy.invalid_handling,
            ),
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
    pub gate_ref: LoopGateRef,
    pub checkpoint_id: TurnCheckpointId,
    pub exit_id: LoopExitId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopBlockedKind {
    Approval,
    Auth,
    Resource,
}

impl LoopBlockedKind {
    fn to_blocked_reason(self, gate_ref: LoopGateRef) -> Result<BlockedReason, ()> {
        let gate_ref = GateRef::new(gate_ref.as_str()).map_err(|_| ())?;
        Ok(match self {
            Self::Approval => BlockedReason::Approval { gate_ref },
            Self::Auth => BlockedReason::Auth { gate_ref },
            Self::Resource => BlockedReason::Resource { gate_ref },
        })
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

/// Host-derived policy for validating a driver-supplied [`LoopExit`] claim.
///
/// Fields are private so callers cannot mint trusted evidence with struct
/// literal syntax. Use the named constructors below so each trusted bit reads
/// as host-verified evidence at the call site. The default remains fail-closed:
/// invalid exits require recovery and no driver evidence is trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct LoopExitValidationPolicy {
    require_final_checkpoint: bool,
    host_cancellation_observed: bool,
    invalid_handling: LoopExitInvalidHandling,
    completion_refs_verified: bool,
    blocked_evidence_verified: bool,
    failure_evidence_verified: bool,
}

impl LoopExitValidationPolicy {
    pub fn recovery_required() -> Self {
        Self::default()
    }

    pub fn fail_terminal() -> Self {
        Self {
            invalid_handling: LoopExitInvalidHandling::FailTerminal,
            ..Self::default()
        }
    }

    pub fn require_final_checkpoint(mut self) -> Self {
        self.require_final_checkpoint = true;
        self
    }

    pub fn with_final_checkpoint_required(mut self, required: bool) -> Self {
        self.require_final_checkpoint = required;
        self
    }

    pub fn with_host_cancellation_observed(mut self) -> Self {
        self.host_cancellation_observed = true;
        self
    }

    pub fn with_host_verified_completion_refs(mut self) -> Self {
        self.completion_refs_verified = true;
        self
    }

    pub fn with_host_verified_blocked_evidence(mut self) -> Self {
        self.blocked_evidence_verified = true;
        self
    }

    pub fn with_host_verified_failure_evidence(mut self) -> Self {
        self.failure_evidence_verified = true;
        self
    }

    pub fn requires_final_checkpoint(&self) -> bool {
        self.require_final_checkpoint
    }

    pub fn host_cancellation_observed(&self) -> bool {
        self.host_cancellation_observed
    }

    pub fn invalid_handling(&self) -> LoopExitInvalidHandling {
        self.invalid_handling
    }

    pub fn completion_refs_verified(&self) -> bool {
        self.completion_refs_verified
    }

    pub fn blocked_evidence_verified(&self) -> bool {
        self.blocked_evidence_verified
    }

    pub fn failure_evidence_verified(&self) -> bool {
        self.failure_evidence_verified
    }
}

impl Default for LoopExitValidationPolicy {
    fn default() -> Self {
        Self {
            require_final_checkpoint: false,
            host_cancellation_observed: false,
            invalid_handling: LoopExitInvalidHandling::RecoveryRequired,
            completion_refs_verified: false,
            blocked_evidence_verified: false,
            failure_evidence_verified: false,
        }
    }
}

impl<'de> Deserialize<'de> for LoopExitValidationPolicy {
    /// Deserialize only the fail-closed policy subset.
    ///
    /// This is intentionally asymmetric with `Serialize`: host-minted policies
    /// may be serialized for diagnostics/snapshots, but untrusted wire payloads
    /// cannot deserialize back into host-verified evidence or terminal invalid
    /// handling.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct WirePolicy {
            #[serde(default)]
            require_final_checkpoint: bool,
            #[serde(default)]
            host_cancellation_observed: bool,
            #[serde(default)]
            invalid_handling: Option<LoopExitInvalidHandling>,
            #[serde(default)]
            completion_refs_verified: bool,
            #[serde(default)]
            blocked_evidence_verified: bool,
            #[serde(default)]
            failure_evidence_verified: bool,
        }

        let wire = WirePolicy::deserialize(deserializer)?;
        if wire.host_cancellation_observed
            || wire.completion_refs_verified
            || wire.blocked_evidence_verified
            || wire.failure_evidence_verified
        {
            return Err(de::Error::custom(
                "loop exit validation policy wire payload cannot mint host-verified evidence",
            ));
        }
        if matches!(
            wire.invalid_handling,
            Some(LoopExitInvalidHandling::FailTerminal)
        ) {
            return Err(de::Error::custom(
                "loop exit validation policy wire payload cannot select terminal invalid-exit handling",
            ));
        }
        Ok(Self::recovery_required().with_final_checkpoint_required(wire.require_final_checkpoint))
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

    if policy.require_final_checkpoint && exit.final_checkpoint_id.is_none() {
        return invalid_exit_decision(
            exit_id,
            LoopExitViolationKind::MissingFinalCheckpoint,
            policy.invalid_handling,
        );
    }

    LoopExitValidationDecision::trusted(exit_id, TurnRunnerOutcome::Completed)
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
