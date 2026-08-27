//! The tail every resume workflow converges on.
//!
//! Owns what is identical across approval-resume, auth-resume and
//! spawn-resume: the preflight that re-validates the blocked run, the
//! resume-shaped authorization fold, and the dispatch tail with its
//! lease-state handling. A resume workflow module owns its *preamble*; the
//! moment two of them agree, the code belongs here.

use ironclaw_authorization::{CapabilityLease, CapabilityLeaseStorePort};
use ironclaw_host_api::{
    authorized::AuthorizeResult,
    decision::Decision,
    dispatch::{CapabilityDispatchResult, CapabilityDispatcher},
    ids::{CapabilityId, DenyRef, GateRef},
    resolution::{Blocked, GateWaypoint},
    scope::ExecutionContext,
};
use ironclaw_processes::ProcessInvocationStatus;
use ironclaw_runtime_policy::plan_capability;
use tracing::warn;

use super::error_mapping::{
    cleanup_claimed_lease_after_resume_error, enrich_dispatch_error_credential_requirements,
    obligation_invocation_error_kind, planner_error_kind, runtime_policy_error_to_invocation_error,
};
use super::{
    AuthorizeFold, AuthorizedFold, BlockedResumeKind, CapabilityHost, ResumedDispatchParams,
    ResumedLeaseState, authorized_dispatch_witness,
};
use crate::helpers::{
    apply_invocation_state_transition_if_configured, capability_lease_error_kind,
    claim_error_may_be_concurrent_resume, complete_invocation_after_side_effect,
    fail_invocation_if_configured, invocation_state_error_kind,
};
use crate::ports::PolicyAction;
use crate::{CapabilityInvocationError, CapabilityObligationOutcome, CapabilityObligationPhase};

impl<'a, D> CapabilityHost<'a, D>
where
    D: CapabilityDispatcher + ?Sized,
{
    /// Resume-path pre-authorization, relocated from host_runtime's deleted
    /// `open_pre_authorization` + `fail_matching_blocked_{,auth_}resume_on_preflight_error`
    /// (§5.3.2/§9, R-A). Resolves the descriptor and enforces runtime-policy
    /// planning on the resumed capability BEFORE the fold's process-invocation lookup, so an
    /// unknown capability short-circuits to `UnknownCapability` (→ `MissingRuntime`)
    /// instead of the process-invocation-not-found `Backend` path, and a runtime policy
    /// tightened between invoke and resume fails closed (reversing #6386's
    /// "planning is NOT re-run on resume"). On refusal it fails ONLY the matching
    /// blocked run — via [`Self::fail_matching_blocked_resume_run`] — recording the
    /// planner-specific INTERNAL `error_kind`, then returns the sanitized error (the
    /// model-visible message stays sanitized through `DenyReason`; the planner
    /// detail rides only the process-invocation audit record). Trust is still classified
    /// downstream (in `authorize_resumed` / the spawn-resume fold), which stamps
    /// `context.trust` before the authorizer.
    pub(super) async fn resume_preflight(
        &self,
        context: &ExecutionContext,
        capability_id: &CapabilityId,
        blocked: BlockedResumeKind,
    ) -> Result<(), CapabilityInvocationError> {
        let Some(descriptor) = self.registry.get_capability(capability_id) else {
            self.fail_matching_blocked_resume_run(
                context,
                capability_id,
                blocked,
                "unknown_capability",
            )
            .await;
            return Err(CapabilityInvocationError::UnknownCapability {
                capability: capability_id.clone(),
            });
        };
        if let Err(planner_error) = plan_capability(descriptor, self.runtime_policy) {
            let error_kind = planner_error_kind(&planner_error);
            self.fail_matching_blocked_resume_run(context, capability_id, blocked, error_kind)
                .await;
            return Err(runtime_policy_error_to_invocation_error(
                capability_id,
                planner_error,
            ));
        }
        Ok(())
    }

    /// Fail ONLY the blocked run that matches this resume request, relocated from
    /// host_runtime's deleted `fail_matching_blocked_{,auth_}resume_on_preflight_error`
    /// (§5.3.2/§9, R-A). Keyed by the request scope + invocation; a wrong-scope or
    /// otherwise non-matching request leaves other blocked runs untouched (scope
    /// isolation). The matching run is transitioned to `Failed` with `error_kind`.
    async fn fail_matching_blocked_resume_run(
        &self,
        context: &ExecutionContext,
        capability_id: &CapabilityId,
        blocked: BlockedResumeKind,
        error_kind: &'static str,
    ) {
        let Some(invocation_state) = self.invocation_state else {
            return;
        };
        let scope = &context.resource_scope;
        let invocation_id = context.invocation_id;
        let record = match invocation_state.get(scope, invocation_id).await {
            Ok(Some(record)) => record,
            Ok(None) => return,
            Err(error) => {
                warn!(
                    invocation_id = %invocation_id,
                    capability_id = %capability_id,
                    preflight_error_kind = error_kind,
                    lookup_error_kind = invocation_state_error_kind(&error),
                    "resume preflight failed, but process-invocation lookup failed; leaving invocation state unchanged",
                );
                return;
            }
        };
        let matches = record.capability_id == *capability_id
            && record.authenticated_actor_user_id == context.authenticated_actor_user_id
            && match blocked {
                BlockedResumeKind::Approval {
                    approval_request_id,
                } => {
                    record.status == ProcessInvocationStatus::BlockedApproval
                        && record.approval_request_id == Some(approval_request_id)
                }
                BlockedResumeKind::Auth => record.status == ProcessInvocationStatus::BlockedAuth,
            };
        if matches {
            fail_invocation_if_configured(Some(invocation_state), scope, invocation_id, error_kind)
                .await;
        }
    }

    /// Pre-dispatch authority fold shared by `resume_json` and
    /// `auth_resume_json`, extracted per arch-simplification §9 step 2 / §5.3.2
    /// exactly as [`Self::authorize`] does for invoke: run trust-aware
    /// authorization and map the `Decision`. On `Deny`/`RequireApproval` every
    /// side effect the inline fold performed stays here verbatim — the process-invocation
    /// `fail` transition and the revoke of an `AlreadyClaimed` lease (transitioned
    /// to `Dispatching` in the `auth_resume_json` preamble) so a terminal refusal
    /// does not strand it.
    ///
    /// Unlike invoke/spawn, the `Authorized` fold carries only the raw
    /// `obligations`: [`Self::dispatch_resumed_capability`] runs the authoritative
    /// obligation preparation and the approval lease claim AFTER this returns, so
    /// the resume paths keep their hard claim-before-dispatch ordering (a
    /// `PendingClaim` lease stays `Active` on a `Deny`, and no second
    /// authorization runs). The witness's `obligation_outcome` is therefore a
    /// placeholder (`default()`) — the seal is a forward-looking artifact
    /// (§5.3.2) that does not gate dispatch and is minted only when the
    /// invocation is seal-able, so today's actor-less/`System` paths are
    /// unaffected.
    pub(super) async fn authorize_resumed(
        &self,
        params: &ResumedDispatchParams<'_>,
    ) -> Result<AuthorizeFold, CapabilityInvocationError> {
        // Kernel-computed trust (§5.3.2/§9): trust is classified here from the
        // resumed capability id rather than carried on the request. Runtime-policy
        // planning already ran in the caller's `resume_preflight` (§5.3.2/§9, R-A,
        // reversing #6386's "planning is NOT re-run on resume"); the `context.trust`
        // stamp below reproduces host_runtime's deleted `open_pre_authorization`.
        let trust_decision = match self.evaluate_trust(&params.capability_id) {
            Ok(d) => d,
            Err(error) => {
                fail_invocation_if_configured(
                    Some(params.invocation_state),
                    &params.scope,
                    params.invocation_id,
                    "AuthorizationDenied",
                )
                .await;
                return Err(error);
            }
        };
        let mut authorize_context = params.authorized_context.clone();
        authorize_context.trust = trust_decision.effective_trust.class();

        // Persistent-approval fold on the auth-resume re-dispatch (§5.2.7/§5.3.2),
        // relocated from host_runtime's former `auth_resume_capability` call to
        // `apply_persistent_approval_policy`. The loop rebuilds a grant-less
        // context after the credential gate; a capability authorized only by a
        // persistent grant (e.g. `extension_install` under admin-config trust)
        // would otherwise be re-authorized grant-less and denied. Excluded for
        // `resume_json` (`PendingClaim`), which always carries a fresh approval
        // lease and never had persistent-approval applied — preserving behavior.
        let mut adopted_grant_expiry = None;
        if !matches!(params.lease_state, ResumedLeaseState::PendingClaim(_)) {
            adopted_grant_expiry = self
                .apply_persistent_approval(
                    &mut authorize_context,
                    params.descriptor,
                    &params.capability_id,
                    &params.estimate,
                    &trust_decision,
                    PolicyAction::Dispatch,
                )
                .await;
        }
        // The claimed approval lease's expiry is a reachable frozen fact for an
        // `AlreadyClaimed` lease (which carries the full grant) and for a
        // `PendingClaim` (whose spec carries the grant expiry threaded from the
        // full lease at construction, since the claim is deferred past this
        // seal); `NoPriorLease` has none. Combined with any adopted
        // persistent-grant expiry, the seal takes the shortest-lived so the
        // witness never outlives the approval that authorized it.
        let claimed_lease_expiry = match &params.lease_state {
            ResumedLeaseState::AlreadyClaimed(_, lease) => lease.grant.constraints.expires_at,
            ResumedLeaseState::PendingClaim(pending) => pending.grant_expiry,
            ResumedLeaseState::NoPriorLease => None,
        };
        let frozen_deadline = [adopted_grant_expiry, claimed_lease_expiry]
            .into_iter()
            .flatten()
            .min();

        match self
            .authorizer
            .authorize_dispatch_with_trust(
                &authorize_context,
                params.descriptor,
                &params.estimate,
                &trust_decision,
            )
            .await
        {
            Decision::Allow {
                obligations: allowed_obligations,
            } => {
                let allowed_obligations = allowed_obligations.into_vec();
                let provisional_outcome = CapabilityObligationOutcome::default();
                Ok(AuthorizeFold::Authorized(Box::new(AuthorizedFold {
                    result: None,
                    frozen_deadline,
                    obligations: allowed_obligations,
                    obligation_outcome: provisional_outcome,
                })))
            }
            Decision::Deny { reason } => {
                fail_invocation_if_configured(
                    Some(params.invocation_state),
                    &params.scope,
                    params.invocation_id,
                    "AuthorizationDenied",
                )
                .await;
                // The AlreadyClaimed lease was transitioned to Dispatching in the
                // auth_resume_json preamble, before this authorization check ran.
                // A Deny is terminal — revoke the lease so it does not stay stuck
                // in Dispatching.  PendingClaim and NoPriorLease have no pre-authz
                // state mutation here.
                if let ResumedLeaseState::AlreadyClaimed(store, lease) = &params.lease_state
                    && let Err(error) = store.revoke(&params.scope, lease.grant.id).await
                {
                    warn!(
                        lease_id = %lease.grant.id,
                        revoke_error_kind = capability_lease_error_kind(&error),
                        "failed to revoke reused approval lease after authorization refused auth-resume; lease may remain Dispatching",
                    );
                }
                Ok(AuthorizeFold::Denied {
                    result: AuthorizeResult::Denied(DenyRef::new()),
                    reason,
                })
            }
            Decision::RequireApproval { .. } => {
                fail_invocation_if_configured(
                    Some(params.invocation_state),
                    &params.scope,
                    params.invocation_id,
                    "AuthorizationRequiresApproval",
                )
                .await;
                // Same as the Deny arm: the AlreadyClaimed lease was transitioned to
                // Dispatching before authorization ran; a RequireApproval refusal is
                // also terminal — revoke so it does not remain stuck in Dispatching.
                if let ResumedLeaseState::AlreadyClaimed(store, lease) = &params.lease_state
                    && let Err(error) = store.revoke(&params.scope, lease.grant.id).await
                {
                    warn!(
                        lease_id = %lease.grant.id,
                        revoke_error_kind = capability_lease_error_kind(&error),
                        "failed to revoke reused approval lease after authorization refused auth-resume; lease may remain Dispatching",
                    );
                }
                // The resume paths never persist a NEW approval here (they resume
                // an already-approved invocation); today's caller returns
                // `AuthorizationRequiresApproval` with no persisted gate, so the
                // forward-looking Blocked witness carries a fresh correlation id.
                Ok(AuthorizeFold::Blocked {
                    result: AuthorizeResult::Blocked(Blocked::Approval(GateWaypoint::new(
                        GateRef::new(),
                    ))),
                })
            }
        }
    }

    /// Converging tail shared by `resume_json` and `auth_resume_json`.
    ///
    /// Runs: trust-aware authorization → prepare obligations (Resume phase) →
    /// `dispatcher.dispatch_json` → complete dispatch obligations → optional
    /// lease consume → `complete_invocation_after_side_effect` → Ok.
    ///
    /// On any failure: aborts applicable obligations, transitions invocation state,
    /// and revokes the claimed lease unless the error is a non-terminal
    /// `BlockAuth` transition (in which case the lease stays Claimed so a
    /// subsequent `auth_resume_json` can reuse it without a second approval).
    pub(super) async fn dispatch_resumed_capability(
        &self,
        params: ResumedDispatchParams<'_>,
    ) -> Result<CapabilityDispatchResult, CapabilityInvocationError> {
        // Pre-dispatch authority fold (trust-aware authorization + Decision
        // mapping) extracted to `authorize_resumed`, mirroring `authorize()`.
        // The claim-before-dispatch ordering the resume paths depend on stays in
        // this tail: the approval lease claim and the authoritative obligation
        // preparation run BELOW, after the fold returns `Authorized`, so a `Deny`
        // still leaves a `PendingClaim` lease `Active` and never a second
        // authorization runs.
        let fold = self.authorize_resumed(&params).await?;

        let ResumedDispatchParams {
            invocation_state,
            scope,
            invocation_id,
            capability_id,
            estimate,
            input,
            authorized_context,
            descriptor,
            lease_state,
        } = params;

        let (obligations, frozen_deadline) = match fold {
            AuthorizeFold::Authorized(fold) => {
                let AuthorizedFold {
                    obligations,
                    frozen_deadline,
                    ..
                } = *fold;
                (obligations, frozen_deadline)
            }
            AuthorizeFold::Denied { reason, .. } => {
                return Err(CapabilityInvocationError::AuthorizationDenied {
                    capability: capability_id,
                    reason,
                    detail: None,
                });
            }
            AuthorizeFold::Blocked { .. } => {
                return Err(CapabilityInvocationError::AuthorizationRequiresApproval {
                    capability: capability_id,
                });
            }
        };

        // For `resume_json` (`PendingClaim`), the approval lease is claimed AFTER
        // authorization so that a `Deny` leaves the lease `Active` (the preamble
        // only injects the grant for the authorize call; the actual `Claimed`
        // transition is deferred to this point).
        //
        // For `auth_resume_json` with a prior approval (`AlreadyClaimed`), the
        // lease was already transitioned to `Claimed` in the preamble; reuse it
        // directly.
        //
        // For `auth_resume_json` with no prior approval (`NoPriorLease`), there
        // is no lease to claim or consume.
        let claimed_lease: Option<(&dyn CapabilityLeaseStorePort, CapabilityLease)> =
            match lease_state {
                ResumedLeaseState::PendingClaim(pc) => {
                    let grant_id = pc.grant_id;
                    match pc.leases.claim(&scope, grant_id, &pc.fingerprint).await {
                        Ok(claimed) => Some((pc.leases, claimed)),
                        Err(error) => {
                            if claim_error_may_be_concurrent_resume(&error) {
                                warn!(
                                    lease_id = %grant_id,
                                    invocation_id = %invocation_id,
                                    capability_id = %capability_id,
                                    error_kind = capability_lease_error_kind(&error),
                                    "approval lease claim lost to a concurrent resume; leaving invocation state unchanged",
                                );
                            } else {
                                fail_invocation_if_configured(
                                    Some(invocation_state),
                                    &scope,
                                    invocation_id,
                                    "ApprovalLeaseClaim",
                                )
                                .await;
                            }
                            return Err(CapabilityInvocationError::Lease(Box::new(error)));
                        }
                    }
                }
                ResumedLeaseState::AlreadyClaimed(leases, lease) => Some((leases, *lease)),
                ResumedLeaseState::NoPriorLease => None,
            };

        let obligation_outcome = match self
            .prepare_obligations(
                CapabilityObligationPhase::Resume,
                &authorized_context,
                &capability_id,
                &estimate,
                obligations.clone(),
            )
            .await
        {
            Ok(outcome) => outcome,
            Err(error) => {
                apply_invocation_state_transition_if_configured(
                    Some(invocation_state),
                    &scope,
                    invocation_id,
                    &error,
                )
                .await;
                // Non-terminal auth bounce: revert Dispatching → Claimed so the next
                // auth_resume_json call can find and reuse the lease.
                if let Some((capability_leases, ref claimed)) = claimed_lease {
                    cleanup_claimed_lease_after_resume_error(
                        capability_leases,
                        &scope,
                        claimed.grant.id,
                        invocation_id,
                        &capability_id,
                        &error,
                        "obligation failure",
                    )
                    .await;
                }
                return Err(error);
            }
        };

        let result = self.seal_authorization(
            &authorized_context,
            &capability_id,
            &estimate,
            &input,
            descriptor,
            &obligation_outcome,
            frozen_deadline,
        )?;
        let authorized = match authorized_dispatch_witness(result, &capability_id) {
            Ok(authorized) => authorized,
            Err(error) => {
                self.abort_obligations(
                    CapabilityObligationPhase::Resume,
                    &authorized_context,
                    &capability_id,
                    &estimate,
                    obligations.as_slice(),
                    &obligation_outcome,
                )
                .await;
                apply_invocation_state_transition_if_configured(
                    Some(invocation_state),
                    &scope,
                    invocation_id,
                    &error,
                )
                .await;
                if let Some((capability_leases, ref claimed)) = claimed_lease {
                    cleanup_claimed_lease_after_resume_error(
                        capability_leases,
                        &scope,
                        claimed.grant.id,
                        invocation_id,
                        &capability_id,
                        &error,
                        "dispatch authorization failure",
                    )
                    .await;
                }
                return Err(error);
            }
        };

        let dispatch = match self.dispatcher.dispatch_json(*authorized).await {
            Ok(dispatch) => dispatch,
            Err(error) => {
                self.abort_obligations(
                    CapabilityObligationPhase::Resume,
                    &authorized_context,
                    &capability_id,
                    &estimate,
                    obligations.as_slice(),
                    &obligation_outcome,
                )
                .await;
                let error =
                    enrich_dispatch_error_credential_requirements(error, obligations.as_slice());
                let invocation_error = CapabilityInvocationError::from(error);
                apply_invocation_state_transition_if_configured(
                    Some(invocation_state),
                    &scope,
                    invocation_id,
                    &invocation_error,
                )
                .await;
                // Non-terminal auth bounce: revert Dispatching → Claimed so the next
                // auth_resume_json call can find and reuse the lease.
                if let Some((capability_leases, ref claimed)) = claimed_lease {
                    cleanup_claimed_lease_after_resume_error(
                        capability_leases,
                        &scope,
                        claimed.grant.id,
                        invocation_id,
                        &capability_id,
                        &invocation_error,
                        "dispatch failure",
                    )
                    .await;
                }
                return Err(invocation_error);
            }
        };

        let dispatch = match self
            .complete_dispatch_obligations(
                CapabilityObligationPhase::Resume,
                &authorized_context,
                &capability_id,
                &estimate,
                obligations.as_slice(),
                &dispatch,
            )
            .await
        {
            Ok(dispatch) => dispatch,
            Err(error) => {
                let cleanup_outcome = CapabilityObligationOutcome::default();
                self.abort_obligations(
                    CapabilityObligationPhase::Resume,
                    &authorized_context,
                    &capability_id,
                    &estimate,
                    obligations.as_slice(),
                    &cleanup_outcome,
                )
                .await;
                fail_invocation_if_configured(
                    Some(invocation_state),
                    &scope,
                    invocation_id,
                    obligation_invocation_error_kind(&error),
                )
                .await;
                if let Some((capability_leases, ref claimed)) = claimed_lease
                    && let Err(revoke_error) =
                        capability_leases.revoke(&scope, claimed.grant.id).await
                {
                    warn!(
                        lease_id = %claimed.grant.id,
                        invocation_id = %invocation_id,
                        capability_id = %capability_id,
                        obligation_error = %error,
                        revoke_error_kind = capability_lease_error_kind(&revoke_error),
                        "capability lease revoke failed after completion obligation failure; lease may remain claimed",
                    );
                }
                return Err(error);
            }
        };

        if let Some((capability_leases, claimed)) = claimed_lease
            && let Err(error) = capability_leases.consume(&scope, claimed.grant.id).await
        {
            warn!(
                lease_id = %claimed.grant.id,
                invocation_id = %invocation_id,
                capability_id = %capability_id,
                error_kind = capability_lease_error_kind(&error),
                "capability lease consume failed after successful dispatch; lease left in claimed state",
            );
        }

        complete_invocation_after_side_effect(
            invocation_state,
            &scope,
            invocation_id,
            &capability_id,
            "dispatch",
        )
        .await;
        Ok(dispatch)
    }
}
