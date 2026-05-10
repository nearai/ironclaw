//! Trusted `LoopExit` applier for the Reborn turn-runner composition.
//!
//! `LoopExit` is a driver claim — the driver declares *what happened* but does
//! not set evidence booleans. `LoopExitApplier` sits between the driver's
//! `LoopExit` and the `TurnRunTransitionPort` state transition, deriving
//! evidence from durable stores (checkpoint state, transcript refs,
//! cancellation signals), computing the `LoopExitValidationPolicy`, and
//! delegating to the existing `LoopExit::validate()` +
//! `apply_validated_loop_exit()` path. Drivers never set evidence booleans.

use std::sync::Arc;

use async_trait::async_trait;

use ironclaw_turns::{
    GateRef, LoopBlockedKind, LoopExit, LoopExitInvalidHandling, LoopExitValidationPolicy,
    LoopMessageRef, LoopResultRef, ResolvedRunProfile, TurnCheckpointId, TurnError, TurnLeaseToken,
    TurnRunId, TurnRunState, TurnRunnerId, TurnScope,
    runner::{ApplyValidatedLoopExitRequest, TurnRunTransitionPort},
};

/// Port for verifying durable evidence backing a driver's `LoopExit` claim.
///
/// Each method checks whether the corresponding evidence exists durably —
/// i.e. has been persisted to a store that survives process restarts. The
/// applier calls these methods to derive `LoopExitValidationPolicy` booleans
/// before delegating to `LoopExit::validate()`.
///
/// Implementations should be side-effect-free: they only read stores, never
/// mutate state.
#[async_trait]
pub trait LoopExitEvidencePort: Send + Sync {
    /// Verify that completion references (reply messages and/or results) exist
    /// durably, belong to the given scope/run, and are finalized (not draft).
    async fn verify_completion_refs(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
        reply_refs: &[LoopMessageRef],
        result_refs: &[LoopResultRef],
    ) -> Result<bool, TurnError>;

    /// Verify that blocked evidence exists durably: the checkpoint belongs to
    /// the run, has the correct kind (e.g. `BeforeBlock`), and the gate/process
    /// ref exists and is pending.
    async fn verify_blocked_evidence(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
        kind: LoopBlockedKind,
        checkpoint_id: &TurnCheckpointId,
        gate_ref: &GateRef,
    ) -> Result<bool, TurnError>;

    /// Verify that the final checkpoint exists durably, belongs to the run, and
    /// has the correct terminal checkpoint kind for the claimed exit.
    async fn verify_final_checkpoint(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
        checkpoint_id: &TurnCheckpointId,
    ) -> Result<bool, TurnError>;

    /// Verify that failure diagnostic evidence exists durably if required
    /// by the profile.
    async fn verify_failure_evidence(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<bool, TurnError>;

    /// Check whether a host cancellation signal was received for this run.
    async fn is_cancellation_observed(&self, run_id: TurnRunId) -> Result<bool, TurnError>;
}

/// Trusted loop-exit applier that derives evidence from durable stores.
///
/// Constructed once per worker and reused across all runs. The applier is
/// parameterised on injectable ports for both evidence verification and
/// state transitions, making it fully testable without I/O.
pub struct LoopExitApplier {
    transition_port: Arc<dyn TurnRunTransitionPort>,
    evidence_port: Arc<dyn LoopExitEvidencePort>,
}

impl LoopExitApplier {
    /// Create a new applier with the given transition and evidence ports.
    pub fn new(
        transition_port: Arc<dyn TurnRunTransitionPort>,
        evidence_port: Arc<dyn LoopExitEvidencePort>,
    ) -> Self {
        Self {
            transition_port,
            evidence_port,
        }
    }

    /// Derive evidence-backed `LoopExitValidationPolicy`, validate the
    /// driver's `LoopExit` claim, and apply the resulting mapping through
    /// the transition port.
    ///
    /// This is the primary entry point called by `TurnRunnerWorker::apply_exit`.
    pub async fn apply(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
        runner_id: TurnRunnerId,
        lease_token: TurnLeaseToken,
        exit: LoopExit,
        profile: &ResolvedRunProfile,
    ) -> Result<TurnRunState, TurnError> {
        let policy = self.derive_policy(scope, run_id, &exit, profile).await?;
        let decision = exit.validate(policy);

        self.transition_port
            .apply_validated_loop_exit(ApplyValidatedLoopExitRequest {
                run_id,
                runner_id,
                lease_token,
                mapping: decision.mapping,
            })
            .await
    }

    /// Derive a `LoopExitValidationPolicy` by querying evidence ports.
    async fn derive_policy(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
        exit: &LoopExit,
        profile: &ResolvedRunProfile,
    ) -> Result<LoopExitValidationPolicy, TurnError> {
        let mut policy = LoopExitValidationPolicy {
            require_final_checkpoint: false,
            final_checkpoint_verified: false,
            host_cancellation_observed: false,
            invalid_handling: LoopExitInvalidHandling::RecoveryRequired,
            completion_refs_verified: false,
            blocked_evidence_verified: false,
            failure_evidence_verified: false,
        };

        match exit {
            LoopExit::Completed(completed) => {
                policy.require_final_checkpoint =
                    profile.checkpoint_policy.require_final_checkpoint;
                policy.completion_refs_verified = self
                    .evidence_port
                    .verify_completion_refs(
                        scope,
                        run_id,
                        &completed.reply_message_refs,
                        &completed.result_refs,
                    )
                    .await?;
                policy.final_checkpoint_verified = self
                    .verify_required_final_checkpoint(
                        scope,
                        run_id,
                        profile,
                        completed.final_checkpoint_id.as_ref(),
                    )
                    .await?;
            }
            LoopExit::Blocked(blocked) => {
                policy.blocked_evidence_verified = self
                    .evidence_port
                    .verify_blocked_evidence(
                        scope,
                        run_id,
                        blocked.kind,
                        &blocked.checkpoint_id,
                        &blocked.gate_ref,
                    )
                    .await?;
            }
            LoopExit::Cancelled(cancelled) => {
                policy.require_final_checkpoint =
                    profile.checkpoint_policy.require_final_checkpoint;
                policy.host_cancellation_observed =
                    self.evidence_port.is_cancellation_observed(run_id).await?;
                policy.final_checkpoint_verified = self
                    .verify_required_final_checkpoint(
                        scope,
                        run_id,
                        profile,
                        cancelled.checkpoint_id.as_ref(),
                    )
                    .await?;
            }
            LoopExit::Failed(failed) => {
                policy.require_final_checkpoint =
                    profile.checkpoint_policy.require_final_checkpoint;
                policy.failure_evidence_verified = self
                    .evidence_port
                    .verify_failure_evidence(scope, run_id)
                    .await?;
                policy.final_checkpoint_verified = self
                    .verify_required_final_checkpoint(
                        scope,
                        run_id,
                        profile,
                        failed.checkpoint_id.as_ref(),
                    )
                    .await?;
            }
        }

        Ok(policy)
    }

    async fn verify_required_final_checkpoint(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
        profile: &ResolvedRunProfile,
        checkpoint_id: Option<&TurnCheckpointId>,
    ) -> Result<bool, TurnError> {
        if !profile.checkpoint_policy.require_final_checkpoint {
            return Ok(false);
        }
        let Some(checkpoint_id) = checkpoint_id else {
            return Ok(false);
        };
        self.evidence_port
            .verify_final_checkpoint(scope, run_id, checkpoint_id)
            .await
    }
}

/// In-memory evidence port for tests.
///
/// All evidence verification returns `Ok(false)` by default (most restrictive /
/// untrusted). Use builder methods to override individual responses.
#[cfg(test)]
pub struct InMemoryLoopExitEvidencePort {
    completion_refs_verified: bool,
    blocked_evidence_verified: bool,
    final_checkpoint_verified: bool,
    failure_evidence_verified: bool,
    cancellation_observed: bool,
}

#[cfg(test)]
impl InMemoryLoopExitEvidencePort {
    /// Create a new in-memory evidence port with all evidence unverified.
    pub fn new() -> Self {
        Self {
            completion_refs_verified: false,
            blocked_evidence_verified: false,
            final_checkpoint_verified: false,
            failure_evidence_verified: false,
            cancellation_observed: false,
        }
    }

    /// Set whether completion refs verification succeeds.
    pub fn with_completion_refs_verified(mut self, verified: bool) -> Self {
        self.completion_refs_verified = verified;
        self
    }

    /// Set whether blocked evidence verification succeeds.
    pub fn with_blocked_evidence_verified(mut self, verified: bool) -> Self {
        self.blocked_evidence_verified = verified;
        self
    }

    /// Set whether final checkpoint verification succeeds.
    pub fn with_final_checkpoint_verified(mut self, verified: bool) -> Self {
        self.final_checkpoint_verified = verified;
        self
    }

    /// Set whether failure evidence verification succeeds.
    pub fn with_failure_evidence_verified(mut self, verified: bool) -> Self {
        self.failure_evidence_verified = verified;
        self
    }

    /// Set whether host cancellation was observed.
    pub fn with_cancellation_observed(mut self, observed: bool) -> Self {
        self.cancellation_observed = observed;
        self
    }

    /// Create a fully-trusted evidence port (all evidence verified).
    pub fn all_verified() -> Self {
        Self {
            completion_refs_verified: true,
            blocked_evidence_verified: true,
            final_checkpoint_verified: true,
            failure_evidence_verified: true,
            cancellation_observed: true,
        }
    }
}

#[cfg(test)]
impl Default for InMemoryLoopExitEvidencePort {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[async_trait]
impl LoopExitEvidencePort for InMemoryLoopExitEvidencePort {
    async fn verify_completion_refs(
        &self,
        _scope: &TurnScope,
        _run_id: TurnRunId,
        _reply_refs: &[LoopMessageRef],
        _result_refs: &[LoopResultRef],
    ) -> Result<bool, TurnError> {
        Ok(self.completion_refs_verified)
    }

    async fn verify_blocked_evidence(
        &self,
        _scope: &TurnScope,
        _run_id: TurnRunId,
        _kind: LoopBlockedKind,
        _checkpoint_id: &TurnCheckpointId,
        _gate_ref: &GateRef,
    ) -> Result<bool, TurnError> {
        Ok(self.blocked_evidence_verified)
    }

    async fn verify_final_checkpoint(
        &self,
        _scope: &TurnScope,
        _run_id: TurnRunId,
        _checkpoint_id: &TurnCheckpointId,
    ) -> Result<bool, TurnError> {
        Ok(self.final_checkpoint_verified)
    }

    async fn verify_failure_evidence(
        &self,
        _scope: &TurnScope,
        _run_id: TurnRunId,
    ) -> Result<bool, TurnError> {
        Ok(self.failure_evidence_verified)
    }

    async fn is_cancellation_observed(&self, _run_id: TurnRunId) -> Result<bool, TurnError> {
        Ok(self.cancellation_observed)
    }
}

#[cfg(test)]
mod tests;
