use std::{collections::HashSet, hash::Hash, sync::Arc};

use async_trait::async_trait;
use serde::{Deserialize, Serialize, de};

use crate::{
    BlockedReason, GateRef, LoopDiagnosticRef, LoopExitId, LoopGateRef, LoopMessageRef,
    LoopResultRef, LoopUsageSummaryRef, ResolvedRunProfile, SanitizedFailure, TurnCheckpointId,
    TurnError, TurnId, TurnRunId, TurnRunState, TurnScope,
    run_profile::{LoopCheckpointKind, LoopCheckpointStateRef},
    runner::{
        ApplyValidatedLoopExitRequest, ClaimedTurnRun, TurnRunTransitionPort, TurnRunnerOutcome,
    },
};

/// Evidence request for completion refs returned by a driver.
#[derive(Debug, Clone)]
pub struct CompletionEvidenceRequest<'a> {
    pub scope: &'a TurnScope,
    pub turn_id: TurnId,
    pub run_id: TurnRunId,
    pub reply_message_refs: &'a [LoopMessageRef],
    pub result_refs: &'a [LoopResultRef],
}

/// Evidence request for a terminal final checkpoint.
#[derive(Debug, Clone)]
pub struct FinalCheckpointEvidenceRequest<'a> {
    pub scope: &'a TurnScope,
    pub turn_id: TurnId,
    pub run_id: TurnRunId,
    pub checkpoint_id: &'a TurnCheckpointId,
}

/// Evidence request for a blocked loop exit.
#[derive(Debug, Clone)]
pub struct BlockedEvidenceRequest<'a> {
    pub scope: &'a TurnScope,
    pub turn_id: TurnId,
    pub run_id: TurnRunId,
    pub blocked: &'a LoopBlocked,
}

/// Evidence request for a failed loop exit.
#[derive(Debug, Clone)]
pub struct FailureEvidenceRequest<'a> {
    pub scope: &'a TurnScope,
    pub turn_id: TurnId,
    pub run_id: TurnRunId,
    pub failed: &'a LoopFailed,
}

/// Read-only durable evidence port used to validate driver-owned claims.
#[async_trait]
pub trait LoopExitEvidencePort: Send + Sync {
    async fn verify_completion_refs(
        &self,
        request: CompletionEvidenceRequest<'_>,
    ) -> Result<bool, TurnError>;

    async fn verify_final_checkpoint(
        &self,
        request: FinalCheckpointEvidenceRequest<'_>,
    ) -> Result<bool, TurnError>;

    async fn verify_blocked_evidence(
        &self,
        request: BlockedEvidenceRequest<'_>,
    ) -> Result<bool, TurnError>;

    async fn verify_failure_evidence(
        &self,
        request: FailureEvidenceRequest<'_>,
    ) -> Result<bool, TurnError>;

    async fn is_cancellation_observed(
        &self,
        scope: &TurnScope,
        turn_id: TurnId,
        run_id: TurnRunId,
    ) -> Result<bool, TurnError>;

    async fn latest_checkpoint_kind(
        &self,
        scope: &TurnScope,
        turn_id: TurnId,
        run_id: TurnRunId,
    ) -> Result<Option<LoopCheckpointKind>, TurnError>;
}

/// Trusted loop-exit applier used by `TurnRunnerWorker`.
///
/// This owns the only production construction path for `LoopExitValidationPolicy`:
/// drivers can submit `LoopExit` claims, but only host-owned evidence ports can
/// mint the validation policy that maps those claims to state transitions.
pub struct LoopExitApplier {
    transition_port: Arc<dyn TurnRunTransitionPort>,
    evidence_port: Arc<dyn LoopExitEvidencePort>,
}

impl LoopExitApplier {
    pub fn new(
        transition_port: Arc<dyn TurnRunTransitionPort>,
        evidence_port: Arc<dyn LoopExitEvidencePort>,
    ) -> Self {
        Self {
            transition_port,
            evidence_port,
        }
    }

    /// Derive policy from durable evidence, validate the exit, and apply the
    /// validated transition under the claimed run's lease.
    pub async fn apply(
        &self,
        claimed: &ClaimedTurnRun,
        exit: LoopExit,
    ) -> Result<TurnRunState, TurnError> {
        let policy = self.derive_policy(claimed, &exit).await?;
        let decision = exit.validate(policy);
        self.transition_port
            .apply_validated_loop_exit(ApplyValidatedLoopExitRequest {
                run_id: claimed.state.run_id,
                runner_id: claimed.runner_id,
                lease_token: claimed.lease_token,
                mapping: decision.mapping,
            })
            .await
    }

    async fn derive_policy(
        &self,
        claimed: &ClaimedTurnRun,
        exit: &LoopExit,
    ) -> Result<LoopExitValidationPolicy, TurnError> {
        let scope = &claimed.state.scope;
        let turn_id = claimed.state.turn_id;
        let run_id = claimed.state.run_id;
        let profile = &claimed.resolved_run_profile;
        let mut policy = LoopExitValidationPolicy {
            require_final_checkpoint: profile.checkpoint_policy.require_final_checkpoint,
            allow_no_reply_completion: profile.checkpoint_policy.allow_no_reply_completion,
            final_checkpoint_verified: false,
            host_cancellation_observed: false,
            invalid_handling: self.invalid_handling(scope, turn_id, run_id).await?,
            completion_refs_verified: false,
            blocked_evidence_verified: false,
            failure_evidence_verified: false,
        };

        match exit {
            LoopExit::Completed(completed) => {
                policy.completion_refs_verified = self
                    .evidence_port
                    .verify_completion_refs(CompletionEvidenceRequest {
                        scope,
                        turn_id,
                        run_id,
                        reply_message_refs: &completed.reply_message_refs,
                        result_refs: &completed.result_refs,
                    })
                    .await?;
                policy.final_checkpoint_verified = self
                    .verify_terminal_final_checkpoint(
                        scope,
                        turn_id,
                        run_id,
                        profile,
                        completed.final_checkpoint_id.as_ref(),
                    )
                    .await?;
            }
            LoopExit::Blocked(blocked) => {
                policy.blocked_evidence_verified = self
                    .evidence_port
                    .verify_blocked_evidence(BlockedEvidenceRequest {
                        scope,
                        turn_id,
                        run_id,
                        blocked,
                    })
                    .await?;
            }
            LoopExit::Cancelled(cancelled) => {
                policy.host_cancellation_observed = self
                    .evidence_port
                    .is_cancellation_observed(scope, turn_id, run_id)
                    .await?;
                policy.final_checkpoint_verified = self
                    .verify_terminal_final_checkpoint(
                        scope,
                        turn_id,
                        run_id,
                        profile,
                        cancelled.checkpoint_id.as_ref(),
                    )
                    .await?;
            }
            LoopExit::Failed(failed) => {
                policy.failure_evidence_verified = self
                    .evidence_port
                    .verify_failure_evidence(FailureEvidenceRequest {
                        scope,
                        turn_id,
                        run_id,
                        failed,
                    })
                    .await?;
                policy.final_checkpoint_verified = self
                    .verify_terminal_final_checkpoint(
                        scope,
                        turn_id,
                        run_id,
                        profile,
                        failed.checkpoint_id.as_ref(),
                    )
                    .await?;
            }
        }

        Ok(policy)
    }

    async fn verify_terminal_final_checkpoint(
        &self,
        scope: &TurnScope,
        turn_id: TurnId,
        run_id: TurnRunId,
        profile: &ResolvedRunProfile,
        checkpoint_id: Option<&TurnCheckpointId>,
    ) -> Result<bool, TurnError> {
        if !profile.checkpoint_policy.require_final_checkpoint {
            return Ok(true);
        }
        let Some(checkpoint_id) = checkpoint_id else {
            return Ok(false);
        };
        self.evidence_port
            .verify_final_checkpoint(FinalCheckpointEvidenceRequest {
                scope,
                turn_id,
                run_id,
                checkpoint_id,
            })
            .await
    }

    async fn invalid_handling(
        &self,
        scope: &TurnScope,
        turn_id: TurnId,
        run_id: TurnRunId,
    ) -> Result<LoopExitInvalidHandling, TurnError> {
        match self
            .evidence_port
            .latest_checkpoint_kind(scope, turn_id, run_id)
            .await?
        {
            Some(
                LoopCheckpointKind::BeforeSideEffect
                | LoopCheckpointKind::BeforeBlock
                | LoopCheckpointKind::Final,
            ) => Ok(LoopExitInvalidHandling::RecoveryRequired),
            Some(LoopCheckpointKind::BeforeModel) | None => {
                Ok(LoopExitInvalidHandling::FailTerminal)
            }
        }
    }
}

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

    fn validate(self, policy: LoopExitValidationPolicy) -> LoopExitValidationDecision {
        let exit_id = self.exit_id().clone();
        match self {
            Self::Completed(exit) => validate_completed_exit(exit_id, exit, policy),
            Self::Blocked(exit) if policy.blocked_evidence_verified => {
                match exit.kind.to_blocked_reason(exit.gate_ref) {
                    Ok(reason) => LoopExitValidationDecision::trusted(
                        exit_id,
                        TurnRunnerOutcome::Blocked {
                            checkpoint_id: exit.checkpoint_id,
                            state_ref: exit.state_ref,
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoopBlocked {
    pub kind: LoopBlockedKind,
    pub gate_ref: LoopGateRef,
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
}

impl LoopBlockedKind {
    fn to_blocked_reason(self, gate_ref: LoopGateRef) -> Result<BlockedReason, ()> {
        let gate_ref = GateRef::new(gate_ref.as_str()).map_err(|_| ())?;
        Ok(match self {
            Self::Approval => BlockedReason::Approval { gate_ref },
            Self::Auth => BlockedReason::Auth { gate_ref },
            Self::Resource => BlockedReason::Resource { gate_ref },
            Self::AwaitDependentRun => BlockedReason::AwaitDependentRun { gate_ref },
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
        }
    }

    fn to_sanitized_failure(self) -> SanitizedFailure {
        SanitizedFailure::from_trusted_static(self.as_str())
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
/// literal syntax outside this module. Use named constructors for fail-closed
/// test policies, and derive production policies from host-owned evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(crate) struct LoopExitValidationPolicy {
    require_final_checkpoint: bool,
    allow_no_reply_completion: bool,
    final_checkpoint_verified: bool,
    host_cancellation_observed: bool,
    invalid_handling: LoopExitInvalidHandling,
    completion_refs_verified: bool,
    blocked_evidence_verified: bool,
    failure_evidence_verified: bool,
}

impl LoopExitValidationPolicy {
    pub(crate) fn recovery_required() -> Self {
        Self::default()
    }

    #[cfg(test)]
    pub(crate) fn fail_terminal() -> Self {
        Self {
            invalid_handling: LoopExitInvalidHandling::FailTerminal,
            ..Self::default()
        }
    }

    #[cfg(test)]
    pub(crate) fn require_final_checkpoint(mut self) -> Self {
        self.require_final_checkpoint = true;
        self
    }

    pub(crate) fn with_final_checkpoint_required(mut self, required: bool) -> Self {
        self.require_final_checkpoint = required;
        self
    }

    #[cfg(test)]
    pub(crate) fn with_allow_no_reply_completion(mut self) -> Self {
        self.allow_no_reply_completion = true;
        self
    }

    #[cfg(test)]
    pub(crate) fn with_host_verified_final_checkpoint(mut self) -> Self {
        self.final_checkpoint_verified = true;
        self
    }

    #[cfg(test)]
    pub(crate) fn with_host_cancellation_observed(mut self) -> Self {
        self.host_cancellation_observed = true;
        self
    }

    #[cfg(test)]
    pub(crate) fn with_host_verified_completion_refs(mut self) -> Self {
        self.completion_refs_verified = true;
        self
    }

    #[cfg(test)]
    pub(crate) fn with_host_verified_blocked_evidence(mut self) -> Self {
        self.blocked_evidence_verified = true;
        self
    }

    #[cfg(test)]
    pub(crate) fn with_host_verified_failure_evidence(mut self) -> Self {
        self.failure_evidence_verified = true;
        self
    }

    #[cfg(test)]
    pub(crate) fn requires_final_checkpoint(&self) -> bool {
        self.require_final_checkpoint
    }

    #[cfg(test)]
    pub(crate) fn allows_no_reply_completion(&self) -> bool {
        self.allow_no_reply_completion
    }

    #[cfg(test)]
    pub(crate) fn final_checkpoint_verified(&self) -> bool {
        self.final_checkpoint_verified
    }

    #[cfg(test)]
    pub(crate) fn host_cancellation_observed(&self) -> bool {
        self.host_cancellation_observed
    }

    #[cfg(test)]
    pub(crate) fn invalid_handling(&self) -> LoopExitInvalidHandling {
        self.invalid_handling
    }

    #[cfg(test)]
    pub(crate) fn completion_refs_verified(&self) -> bool {
        self.completion_refs_verified
    }

    #[cfg(test)]
    pub(crate) fn blocked_evidence_verified(&self) -> bool {
        self.blocked_evidence_verified
    }

    #[cfg(test)]
    pub(crate) fn failure_evidence_verified(&self) -> bool {
        self.failure_evidence_verified
    }
}

impl Default for LoopExitValidationPolicy {
    fn default() -> Self {
        Self {
            require_final_checkpoint: false,
            allow_no_reply_completion: false,
            final_checkpoint_verified: false,
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
    /// cannot deserialize back into host-verified evidence or relaxed terminal
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
            allow_no_reply_completion: bool,
            #[serde(default)]
            final_checkpoint_verified: bool,
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
        if wire.allow_no_reply_completion
            || wire.final_checkpoint_verified
            || wire.host_cancellation_observed
            || wire.completion_refs_verified
            || wire.blocked_evidence_verified
            || wire.failure_evidence_verified
        {
            return Err(de::Error::custom(
                "loop exit validation policy wire payload cannot mint host-verified evidence or relaxed completion policy",
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
    MismatchedCompletionReferenceKind,
    UnverifiedCompletionReference,
    MissingFinalCheckpoint,
    UnverifiedBlockedEvidence,
    UnverifiedFailureEvidence,
    CancellationNotObserved,
    NoReplyNotAllowed,
}

impl LoopExitViolationKind {
    fn category(self) -> &'static str {
        match self {
            Self::MissingCompletionReference => "missing_completion_reference",
            Self::MismatchedCompletionReferenceKind => "mismatched_completion_reference_kind",
            Self::UnverifiedCompletionReference => "unverified_completion_reference",
            Self::MissingFinalCheckpoint => "missing_final_checkpoint",
            Self::UnverifiedBlockedEvidence => "unverified_blocked_evidence",
            Self::UnverifiedFailureEvidence => "unverified_failure_evidence",
            Self::CancellationNotObserved => "cancellation_not_observed",
            Self::NoReplyNotAllowed => "no_reply_not_allowed",
        }
    }

    fn failure_category(self) -> &'static str {
        match self {
            Self::CancellationNotObserved => "interrupted_unexpectedly",
            Self::MissingCompletionReference
            | Self::MismatchedCompletionReferenceKind
            | Self::UnverifiedCompletionReference
            | Self::MissingFinalCheckpoint
            | Self::UnverifiedBlockedEvidence
            | Self::UnverifiedFailureEvidence
            | Self::NoReplyNotAllowed => "driver_protocol_violation",
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
    if let Some(kind_violation) = completion_kind_ref_violation(&exit, policy) {
        return invalid_exit_decision(exit_id, kind_violation, policy.invalid_handling);
    }

    if exit.has_durable_completion_ref() && !policy.completion_refs_verified {
        return invalid_exit_decision(
            exit_id,
            LoopExitViolationKind::UnverifiedCompletionReference,
            policy.invalid_handling,
        );
    }

    if policy.require_final_checkpoint
        && (exit.final_checkpoint_id.is_none() || !policy.final_checkpoint_verified)
    {
        return invalid_exit_decision(
            exit_id,
            LoopExitViolationKind::MissingFinalCheckpoint,
            policy.invalid_handling,
        );
    }

    LoopExitValidationDecision::trusted(exit_id, TurnRunnerOutcome::Completed)
}

fn completion_kind_ref_violation(
    exit: &LoopCompleted,
    policy: LoopExitValidationPolicy,
) -> Option<LoopExitViolationKind> {
    match exit.completion_kind {
        LoopCompletionKind::FinalReply | LoopCompletionKind::AskUserReply => {
            if exit.reply_message_refs.is_empty() {
                Some(LoopExitViolationKind::MissingCompletionReference)
            } else {
                None
            }
        }
        LoopCompletionKind::DelegatedResult | LoopCompletionKind::ResultOnly => {
            if exit.result_refs.is_empty() {
                Some(LoopExitViolationKind::MissingCompletionReference)
            } else if !exit.reply_message_refs.is_empty() {
                Some(LoopExitViolationKind::MismatchedCompletionReferenceKind)
            } else {
                None
            }
        }
        LoopCompletionKind::NoReply => {
            if exit.has_durable_completion_ref() {
                Some(LoopExitViolationKind::MismatchedCompletionReferenceKind)
            } else if !policy.allow_no_reply_completion {
                Some(LoopExitViolationKind::NoReplyNotAllowed)
            } else {
                None
            }
        }
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
    if policy.require_final_checkpoint
        && (exit.checkpoint_id.is_none() || !policy.final_checkpoint_verified)
    {
        return invalid_exit_decision(
            exit_id,
            LoopExitViolationKind::MissingFinalCheckpoint,
            policy.invalid_handling,
        );
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
    if policy.require_final_checkpoint
        && (exit.checkpoint_id.is_none() || !policy.final_checkpoint_verified)
    {
        return invalid_exit_decision(
            exit_id,
            LoopExitViolationKind::MissingFinalCheckpoint,
            policy.invalid_handling,
        );
    }
    LoopExitValidationDecision::trusted(
        exit_id,
        TurnRunnerOutcome::Failed {
            failure: exit.reason_kind.to_sanitized_failure(),
        },
    )
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

#[cfg(test)]
mod tests;
