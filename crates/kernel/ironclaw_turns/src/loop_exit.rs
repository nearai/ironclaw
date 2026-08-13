use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_processes::{
    FailProcessRequest, JournaledProcessSnapshot, ProcessCheckpointRef, ProcessLeaseRequest,
    ProcessLeaseToken, ProcessStateTransitionRequest, ProcessSuspension, ProcessSuspensionKind,
    ProcessTransitionPort, ProcessWorkerId, SuspendProcessRequest,
};
use serde::{Deserialize, Serialize, de};

use ironclaw_loop_contracts::{
    LoopBlocked, LoopCancelled, LoopCheckpointKind, LoopCompleted, LoopCompletionKind, LoopExit,
    LoopFailed, LoopModelUsage, ResolvedRunProfile,
};

use crate::{
    BlockedReason, CapabilityActivityId, GateKind, LoopExitId, LoopMessageRef, LoopResultRef,
    SanitizedFailure, TurnCheckpointId, TurnError, TurnId, TurnRunId, TurnRunState, TurnScope,
    runner::{ClaimedTurnRun, TurnRunnerOutcome},
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
///
/// `failed.explanation_message_refs` are part of the driver-owned evidence
/// claim. Implementations must treat them as durable references only after
/// verifying they belong to the scoped run; unverified refs must not be
/// surfaced through a trusted failed outcome.
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

/// Trusted loop-exit applier used by `RebornTurnRunExecutor`.
///
/// This owns the only production construction path for `LoopExitValidationPolicy`:
/// drivers can submit `LoopExit` claims, but only host-owned evidence ports can
/// mint the validation policy that maps those claims to state transitions.
pub struct LoopExitApplier {
    transition_port: Arc<dyn ProcessTransitionPort<Error = TurnError>>,
    evidence_port: Arc<dyn LoopExitEvidencePort>,
}

impl LoopExitApplier {
    pub fn new(
        transition_port: Arc<dyn ProcessTransitionPort<Error = TurnError>>,
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
        // Capture the loop's reported usage before `validate` consumes the exit
        // and collapses it to a coarse outcome; carry it so the terminal
        // transition can persist it on the run record.
        let model_usage = reported_model_usage(&exit);
        let decision = validate_loop_exit(exit, policy);
        let snapshot = apply_validated_process_loop_exit(
            self.transition_port.as_ref(),
            claimed,
            decision.mapping,
            model_usage,
        )
        .await?;
        crate::turn_run_state_from_process_snapshot(snapshot)
    }

    pub async fn record_runner_failure(
        &self,
        claimed: &ClaimedTurnRun,
        failure: SanitizedFailure,
    ) -> Result<TurnRunState, TurnError> {
        let snapshot = self
            .transition_port
            .fail_process(FailProcessRequest {
                process_id: crate::process_projection::process_id_from_turn_run_id(
                    claimed.state.run_id,
                ),
                worker_id: process_worker_id_from_turn_runner_id(claimed.runner_id),
                lease_token: process_lease_token_from_turn(claimed.lease_token),
                failure,
                recovery: ironclaw_processes::ProcessFailureRecovery::Terminal,
                checkpoint_ref: claimed.state.checkpoint_id.map(|checkpoint_id| {
                    ProcessCheckpointRef::from_trusted(checkpoint_id.as_uuid().to_string())
                }),
                metadata: Some(crate::process_projection::agent_turn_metadata_from_claimed(
                    claimed,
                    claimed.state.model_usage,
                )),
            })
            .await?;
        crate::turn_run_state_from_process_snapshot(snapshot)
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
}

async fn apply_validated_process_loop_exit(
    transition_port: &dyn ProcessTransitionPort<Error = TurnError>,
    claimed: &ClaimedTurnRun,
    mapping: LoopExitMapping,
    model_usage: Option<LoopModelUsage>,
) -> Result<JournaledProcessSnapshot, TurnError> {
    match mapping {
        LoopExitMapping::RunnerOutcome(TurnRunnerOutcome::Completed) => {
            transition_port
                .complete_process(process_state_transition_request_from_claimed(
                    claimed,
                    model_usage,
                ))
                .await
        }
        LoopExitMapping::RunnerOutcome(TurnRunnerOutcome::Cancelled) => {
            transition_port
                .cancel_process(process_state_transition_request_from_claimed(
                    claimed,
                    model_usage,
                ))
                .await
        }
        LoopExitMapping::RunnerOutcome(TurnRunnerOutcome::Blocked {
            checkpoint_id,
            reason,
            blocked_activity_id,
            ..
        }) => {
            transition_port
                .suspend_process(SuspendProcessRequest {
                    process_id: crate::process_projection::process_id_from_turn_run_id(
                        claimed.state.run_id,
                    ),
                    worker_id: process_worker_id_from_turn_runner_id(claimed.runner_id),
                    lease_token: process_lease_token_from_turn(claimed.lease_token),
                    checkpoint_ref: ProcessCheckpointRef::from_trusted(
                        checkpoint_id.as_uuid().to_string(),
                    ),
                    suspension: process_suspension_from_blocked_reason(reason, blocked_activity_id),
                    metadata: Some(crate::process_projection::agent_turn_metadata_from_claimed(
                        claimed,
                        model_usage,
                    )),
                })
                .await
        }
        LoopExitMapping::RunnerOutcome(TurnRunnerOutcome::Failed { failure })
        | LoopExitMapping::RecoveryRequired { failure } => {
            transition_port
                .fail_process(FailProcessRequest {
                    process_id: crate::process_projection::process_id_from_turn_run_id(
                        claimed.state.run_id,
                    ),
                    worker_id: process_worker_id_from_turn_runner_id(claimed.runner_id),
                    lease_token: process_lease_token_from_turn(claimed.lease_token),
                    failure,
                    recovery: ironclaw_processes::ProcessFailureRecovery::Terminal,
                    // The failed exit's Final checkpoint is terminal evidence,
                    // not a resumable continuation point. It was verified
                    // before this transition; retain the checkpoint from the
                    // claimed state so retry resumes from the last safe
                    // BeforeModel/BeforeSideEffect/BeforeBlock checkpoint
                    // instead.
                    checkpoint_ref: claimed.state.checkpoint_id.map(|checkpoint_id| {
                        ProcessCheckpointRef::from_trusted(checkpoint_id.as_uuid().to_string())
                    }),
                    metadata: Some(crate::process_projection::agent_turn_metadata_from_claimed(
                        claimed,
                        model_usage,
                    )),
                })
                .await
        }
    }
}

fn process_lease_request_from_claimed(claimed: &ClaimedTurnRun) -> ProcessLeaseRequest {
    ProcessLeaseRequest {
        process_id: crate::process_projection::process_id_from_turn_run_id(claimed.state.run_id),
        worker_id: process_worker_id_from_turn_runner_id(claimed.runner_id),
        lease_token: process_lease_token_from_turn(claimed.lease_token),
    }
}

fn process_state_transition_request_from_claimed(
    claimed: &ClaimedTurnRun,
    model_usage: Option<LoopModelUsage>,
) -> ProcessStateTransitionRequest {
    ProcessStateTransitionRequest {
        lease: process_lease_request_from_claimed(claimed),
        metadata: Some(crate::process_projection::agent_turn_metadata_from_claimed(
            claimed,
            model_usage,
        )),
    }
}

fn process_worker_id_from_turn_runner_id(runner_id: crate::TurnRunnerId) -> ProcessWorkerId {
    ProcessWorkerId::from_trusted(runner_id.as_uuid().to_string())
}

fn process_lease_token_from_turn(lease_token: crate::TurnLeaseToken) -> ProcessLeaseToken {
    ProcessLeaseToken::from_trusted(lease_token.as_uuid().to_string())
}

fn process_suspension_from_blocked_reason(
    reason: BlockedReason,
    blocked_activity_id: Option<CapabilityActivityId>,
) -> ProcessSuspension {
    ProcessSuspension {
        kind: process_suspension_kind_from_gate_kind(reason.gate_kind()),
        gate_ref: Some(reason.gate_ref().clone()),
        activity_id: blocked_activity_id,
        credential_requirements: reason.credential_requirements().to_vec(),
        detail: None,
    }
}

fn process_suspension_kind_from_gate_kind(kind: GateKind) -> ProcessSuspensionKind {
    match kind {
        GateKind::Approval => ProcessSuspensionKind::Approval,
        GateKind::Auth => ProcessSuspensionKind::Authorization,
        GateKind::Resource => ProcessSuspensionKind::Resource,
        GateKind::AwaitDependentRun => ProcessSuspensionKind::AwaitingChildProcess,
        GateKind::ExternalTool => ProcessSuspensionKind::ExternalTool,
    }
}

/// The cumulative model usage a terminal exit reported, if any. Blocked and
/// cancelled exits carry none (usage rides completion/failure only).
fn reported_model_usage(exit: &LoopExit) -> Option<LoopModelUsage> {
    match exit {
        LoopExit::Completed(exit) => exit.model_usage,
        LoopExit::Failed(exit) => exit.model_usage,
        LoopExit::Blocked(_) | LoopExit::Cancelled(_) => None,
    }
}

/// Validate a driver-supplied claim against host-derived policy.
///
/// The claim vocabulary lives in `ironclaw_loop_contracts`; validating it into
/// a durable transition is this kernel's authority and stays here.
fn validate_loop_exit(
    exit: LoopExit,
    policy: LoopExitValidationPolicy,
) -> LoopExitValidationDecision {
    let exit_id = exit.exit_id().clone();
    match exit {
        LoopExit::Completed(exit) => validate_completed_exit(exit_id, exit, policy),
        LoopExit::Blocked(exit) if policy.blocked_evidence_verified => {
            match exit
                .kind
                .to_blocked_reason(exit.gate_ref, exit.credential_requirements)
            {
                Some(reason) => LoopExitValidationDecision::trusted(
                    exit_id,
                    TurnRunnerOutcome::Blocked {
                        checkpoint_id: exit.checkpoint_id,
                        state_ref: exit.state_ref,
                        reason,
                        blocked_activity_id: exit.blocked_activity_id,
                    },
                ),
                None => {
                    invalid_exit_decision(exit_id, LoopExitViolationKind::UnverifiedBlockedEvidence)
                }
            }
        }
        LoopExit::Blocked(_exit) => {
            invalid_exit_decision(exit_id, LoopExitViolationKind::UnverifiedBlockedEvidence)
        }
        LoopExit::Cancelled(exit) => validate_cancelled_exit(exit_id, exit, policy),
        LoopExit::Failed(exit) => validate_failed_exit(exit_id, exit, policy),
    }
}

/// Host-derived policy for validating a driver-supplied [`LoopExit`] claim.
///
/// Fields are private so callers cannot mint trusted evidence with struct
/// literal syntax outside this module. Use named constructors for fail-closed
/// test policies, and derive production policies from host-owned evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
pub(crate) struct LoopExitValidationPolicy {
    require_final_checkpoint: bool,
    allow_no_reply_completion: bool,
    final_checkpoint_verified: bool,
    host_cancellation_observed: bool,
    completion_refs_verified: bool,
    blocked_evidence_verified: bool,
    failure_evidence_verified: bool,
}

impl LoopExitValidationPolicy {
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
        Ok(Self::default().with_final_checkpoint_required(wire.require_final_checkpoint))
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

    /// Host-authored, secret-free failure detail naming the specific violation.
    ///
    /// The coarse [`Self::failure_category`] is the wire-stable user-facing
    /// signal; this detail rides `SanitizedFailure::detail` so the specific
    /// violation kind survives on the durable failure record (run state +
    /// `TurnLifecycleEvent.detail`) and reaches the failure explainer instead
    /// of being collapsed away.
    fn failure_detail(self) -> String {
        format!("loop exit violation: {}", self.category())
    }
}

fn validate_completed_exit(
    exit_id: LoopExitId,
    exit: LoopCompleted,
    policy: LoopExitValidationPolicy,
) -> LoopExitValidationDecision {
    if let Some(kind_violation) = completion_kind_ref_violation(&exit, policy) {
        return invalid_exit_decision(exit_id, kind_violation);
    }

    if exit.has_durable_completion_ref() && !policy.completion_refs_verified {
        return invalid_exit_decision(
            exit_id,
            LoopExitViolationKind::UnverifiedCompletionReference,
        );
    }

    if policy.require_final_checkpoint
        && (exit.final_checkpoint_id.is_none() || !policy.final_checkpoint_verified)
    {
        return invalid_exit_decision(exit_id, LoopExitViolationKind::MissingFinalCheckpoint);
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
        return invalid_exit_decision(exit_id, LoopExitViolationKind::CancellationNotObserved);
    }
    if policy.require_final_checkpoint
        && (exit.checkpoint_id.is_none() || !policy.final_checkpoint_verified)
    {
        return invalid_exit_decision(exit_id, LoopExitViolationKind::MissingFinalCheckpoint);
    }
    LoopExitValidationDecision::trusted(exit_id, TurnRunnerOutcome::Cancelled)
}

fn validate_failed_exit(
    exit_id: LoopExitId,
    exit: LoopFailed,
    policy: LoopExitValidationPolicy,
) -> LoopExitValidationDecision {
    if !policy.failure_evidence_verified {
        return invalid_exit_decision(exit_id, LoopExitViolationKind::UnverifiedFailureEvidence);
    }
    if policy.require_final_checkpoint
        && (exit.checkpoint_id.is_none() || !policy.final_checkpoint_verified)
    {
        return invalid_exit_decision(exit_id, LoopExitViolationKind::MissingFinalCheckpoint);
    }
    let failure = exit
        .safe_summary
        .unwrap_or_else(|| exit.reason_kind.to_sanitized_failure());
    LoopExitValidationDecision::trusted(exit_id, TurnRunnerOutcome::Failed { failure })
}

fn invalid_exit_decision(
    exit_id: LoopExitId,
    kind: LoopExitViolationKind,
) -> LoopExitValidationDecision {
    // Persist the specific violation kind on the sanitized failure detail so
    // the durable record keeps WHICH protocol rule was broken, not only the
    // coarse category (docs/internal/plans/2026-07-03-loop-failure-matrix.md).
    let failure = SanitizedFailure::from_trusted_static(kind.failure_category())
        .with_detail(kind.failure_detail());
    let mapping = TurnRunnerOutcome::Failed { failure }.into();

    LoopExitValidationDecision {
        exit_id,
        mapping,
        violation: Some(LoopExitViolation { kind }),
    }
}

#[cfg(test)]
/// Test-only receiver form of [`validate_loop_exit`], kept so the validation
/// suite reads exactly as it did before the claim vocabulary moved to
/// `ironclaw_loop_contracts`.
trait LoopExitValidateForTests {
    fn validate(self, policy: LoopExitValidationPolicy) -> LoopExitValidationDecision;
}

#[cfg(test)]
impl LoopExitValidateForTests for LoopExit {
    fn validate(self, policy: LoopExitValidationPolicy) -> LoopExitValidationDecision {
        validate_loop_exit(self, policy)
    }
}

#[cfg(test)]
mod tests;
