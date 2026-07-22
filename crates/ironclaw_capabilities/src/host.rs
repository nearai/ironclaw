// arch-exempt: large_file, Slice-C `authorize()` extraction is a behavior-preserving step in the capability-path collapse (doc §9); net additions are transitional and shrink as later slices route dispatch through the sealed `Authorized` witness and retire the mirror request DTOs, plan #6175
use chrono::Utc;
use ironclaw_authorization::{
    CapabilityLease, CapabilityLeaseStore, TrustAwareCapabilityDispatchAuthorizer,
};
use ironclaw_extensions::ExtensionRegistry;
use ironclaw_host_api::{
    ActivityId, Actor, ApprovalRequestId, AuthorizeResult, Authorized, Blocked,
    CapabilityAuthorizer, CapabilityDescriptor, CapabilityDispatchResult, CapabilityDispatcher,
    CapabilityGrantId, CapabilityId, Decision, DenyReason, DenyRef, DispatchError,
    EffectiveRuntimePolicy, ExecutionContext, GateRef, GateWaypoint, Invocation,
    InvocationFingerprint, InvocationId, Obligation, PermissionMode, ProcessAuthorizedContinuation,
    ProcessId, ResourceEstimate, ResourceScope, RuntimeKind, RuntimeLane, Timestamp,
};
use ironclaw_processes::{ProcessManager, ProcessStart};
use ironclaw_run_state::{
    ApprovalRequestStore, ApprovalStatus, RunStart, RunStateApprovalStore, RunStateError,
    RunStateStore, RunStatus,
};
use ironclaw_runtime_policy::{PlannerError, plan_capability};
use ironclaw_safety::shell_command_display_text;
use ironclaw_trust::{TrustDecision, TrustPolicy};
use tracing::{debug, warn};

use crate::trust::{TrustEvaluationError, evaluate_invocation_trust};

use crate::helpers::{
    CapabilityActionKind, CapabilityRunStateTransition, apply_run_state_transition_if_configured,
    approval_not_approved_error_kind, capability_lease_error_kind,
    claim_error_may_be_concurrent_resume, complete_run_after_side_effect, fail_run_if_configured,
    invocation_fingerprint_for_kind, matching_approval_lease,
    matching_claimed_approval_lease_for_auth_resume, resume_context_mismatch_kind,
    run_state_error_kind, validate_approval_request_matches_invocation,
};
use crate::obligations::post_dispatch_obligations;
use crate::ports::{CredentialPresence, HostPolicyFacts, PolicyAction};
use crate::{
    CapabilityAuthResumeRequest, CapabilityInvocationError, CapabilityInvocationRequest,
    CapabilityInvocationResult, CapabilityObligationAbortRequest,
    CapabilityObligationCompletionRequest, CapabilityObligationError,
    CapabilityObligationFailureKind, CapabilityObligationHandler, CapabilityObligationOutcome,
    CapabilityObligationPhase, CapabilityObligationRequest, CapabilityResumeRequest,
    CapabilitySpawnRequest, CapabilitySpawnResult,
};

pub struct CapabilityHost<'a, D>
where
    D: CapabilityDispatcher + ?Sized,
{
    registry: &'a ExtensionRegistry,
    dispatcher: &'a D,
    authorizer: &'a dyn TrustAwareCapabilityDispatchAuthorizer,
    /// Provider-trust classifier the kernel evaluates in-fold (§5.3.2/§9), so
    /// trust is computed here rather than received as a caller-stamped field.
    trust_policy: &'a dyn TrustPolicy,
    /// Resolved runtime policy the in-fold planner (`plan_capability`) enforces
    /// before dispatch — the relocation of host_runtime's `enforce_runtime_policy`.
    runtime_policy: &'a EffectiveRuntimePolicy,
    /// Host-mediated policy *facts* the `authorize()` fold reads (§5.3.2/§9).
    /// Supplies credential-presence facts so a missing credential surfaces as
    /// `AuthorizationRequiresAuth` *before* the approval decision — the
    /// relocation of host_runtime's `credential_preflight_check`. Facts only:
    /// the kernel maps them to the verdict; the port never decides.
    policy_facts: &'a dyn HostPolicyFacts,
    run_state: Option<&'a dyn RunStateStore>,
    approval_requests: Option<&'a dyn ApprovalRequestStore>,
    run_state_approval_store: Option<&'a dyn RunStateApprovalStore>,
    capability_leases: Option<&'a dyn CapabilityLeaseStore>,
    process_manager: Option<&'a dyn ProcessManager>,
    obligation_handler: Option<&'a dyn CapabilityObligationHandler>,
}

// `CapabilityHost` IS the kernel authorizer (Slice-C wiring, arch-simplification
// §3/§5.3.2). Implementing `CapabilityAuthorizer` here — and NOWHERE else, per
// the `reborn_authorized_seal_ratchet` — is the "test-seal" half of the
// `Authorized` witness: only this crate can mint an `AuthorizationGrant`, so only
// the code that runs the authorize fold can seal an `Authorized`. The
// `authorize()` method that consumes the grant lands in a following wiring slice;
// this activates the seal so that ratchet becomes load-bearing.
impl<'a, D> CapabilityAuthorizer for CapabilityHost<'a, D> where D: CapabilityDispatcher + ?Sized {}

/// Specification for a lease that must be claimed AFTER authorization succeeds.
///
/// Used by `resume_json` where the approval lease is claimed only after
/// `authorize_dispatch_with_trust` returns `Allow` — keeping the lease `Active`
/// if authorization is denied.
struct PendingClaimAfterAuth<'r> {
    leases: &'r dyn CapabilityLeaseStore,
    grant_id: CapabilityGrantId,
    fingerprint: InvocationFingerprint,
    /// The approval lease's frozen expiry, carried from the full grant so the
    /// sealed witness never outlives the approval that authorized it. `None`
    /// when the grant declares no `expires_at`. Threaded through even though the
    /// claim is deferred past authorization: the seal is minted before the
    /// claim, so the expiry must travel on the pending-claim spec rather than
    /// being read back from a not-yet-claimed lease.
    grant_expiry: Option<Timestamp>,
}

/// Which blocked run a resume-path preflight failure may fail (§5.3.2/§9, R-A).
/// Mirrors host_runtime's two deleted matchers: the approval-resume /
/// spawn-resume paths key on a `BlockedApproval` record and compare the
/// `approval_request_id`; the auth-resume path keys on a `BlockedAuth` record and
/// does NOT compare `approval_request_id` (its `block_auth` transition clears the
/// persisted id to `None`).
#[derive(Debug, Clone, Copy)]
enum BlockedResumeKind {
    Approval {
        approval_request_id: ApprovalRequestId,
    },
    Auth,
}

/// Encodes the three mutually-exclusive approval-lease states that
/// `dispatch_resumed_capability` must handle.
enum ResumedLeaseState<'r> {
    /// A one-shot `Active` lease to claim *after* `authorize_dispatch_with_trust`
    /// returns `Allow`.  Used by `resume_json` so that a `Deny` leaves the
    /// lease `Active` (the claim is deferred past the authorize call).
    PendingClaim(PendingClaimAfterAuth<'r>),
    /// A lease already transitioned to `Claimed` by a prior `resume_json` auth
    /// bounce.  Used by `auth_resume_json` when the invocation previously passed
    /// an approval gate; reuses the existing `Claimed` lease without a second
    /// approval prompt.
    AlreadyClaimed(&'r dyn CapabilityLeaseStore, Box<CapabilityLease>),
    /// No prior approval lease is in play.  Used by `auth_resume_json` when
    /// `approval_request_id` is `None` (the invocation never passed an approval
    /// gate before hitting the auth gate).
    NoPriorLease,
}

/// Parameters for the converging dispatch tail shared between `resume_json`
/// and `auth_resume_json`.  All fields are resolved by the respective
/// method preamble before the shared tail begins.
struct ResumedDispatchParams<'r> {
    run_state: &'r dyn RunStateStore,
    scope: ResourceScope,
    invocation_id: InvocationId,
    capability_id: CapabilityId,
    estimate: ResourceEstimate,
    input: serde_json::Value,
    authorized_context: ExecutionContext,
    descriptor: &'r CapabilityDescriptor,
    /// Approval-lease state for this resume.  See [`ResumedLeaseState`].
    lease_state: ResumedLeaseState<'r>,
}

/// Outcome of the extracted `authorize()` fold (arch-simplification §5.3.2,
/// §9 step 2): the sealed [`AuthorizeResult`] trichotomy (§3) *plus* the
/// behavior-preserving side-band `invoke_json` still needs to reproduce today's
/// exact dispatch and error mapping while the capability path is mid-migration.
///
/// Why this wraps `AuthorizeResult` rather than being one:
/// - `Denied` — `AuthorizeResult::Denied(DenyRef)` collapses the policy
///   [`DenyReason`] to an opaque correlation UUID; today's caller returns
///   `AuthorizationDenied { reason }`, so the reason rides here until denial
///   folds into `Resolution` (a later slice).
/// - `Authorized` — today's `invoke_json` still owns `dispatch_json` and the
///   post-dispatch obligation lifecycle, so it needs the raw `obligations` and
///   the prepared `obligation_outcome`. Those `Option`-shaped mounts/reservation
///   are the *exact* values dispatch receives; the sealed witness's provisional,
///   forward-looking `mounts`/`reservation` deliberately do NOT drive today's
///   dispatch (§5.3.2/§5.3.3 — the dispatcher still reserves against the
///   governor when `resource_reservation` is `None`).
enum AuthorizeFold {
    /// Authorization allowed dispatch. Boxed because its payload (obligations +
    /// prepared outcome + the boxed witness) dwarfs the ref-sized deny/block
    /// variants (`clippy::large_enum_variant`).
    Authorized(Box<AuthorizedFold>),
    /// Terminal policy denial (`AuthorizeResult::Denied`). `reason` is the
    /// model-visible policy verdict the caller resurfaces as
    /// `AuthorizationDenied { reason }`.
    Denied {
        result: AuthorizeResult,
        reason: DenyReason,
    },
    /// A re-entrant approval gate (`AuthorizeResult::Blocked(Blocked::Approval)`).
    /// The pending approval was persisted and the run transitioned to
    /// `BlockedApproval` inside `authorize`; the caller returns
    /// `AuthorizationRequiresApproval`.
    Blocked { result: AuthorizeResult },
}

/// Payload of [`AuthorizeFold::Authorized`] — the allowed-dispatch side-band.
///
/// `result` is `Some(AuthorizeResult::Authorized(..))` for every allowed,
/// dispatchable invocation: actor-less contexts seal as [`Actor::System`] and
/// origin is the real ingress fact. `result` is `None` only when the descriptor
/// resolves to no untrusted [`RuntimeLane`] (a host-internal `System` runtime) or
/// when a context carries no resolvable ingress origin. Inline dispatch requires
/// a witness; process spawn allows `System` runtime continuations to remain
/// witness-less because those execute through the process host path, not an
/// untrusted runtime lane.
struct AuthorizedFold {
    result: Option<AuthorizeResult>,
    frozen_deadline: Option<Timestamp>,
    obligations: Vec<Obligation>,
    obligation_outcome: CapabilityObligationOutcome,
}

fn authorized_dispatch_witness(
    result: Option<AuthorizeResult>,
    capability_id: &CapabilityId,
) -> Result<Box<Authorized>, CapabilityInvocationError> {
    match result {
        Some(AuthorizeResult::Authorized(authorized)) => Ok(authorized),
        _ => Err(CapabilityInvocationError::from(
            DispatchError::MissingAuthorization {
                capability: capability_id.clone(),
            },
        )),
    }
}

fn process_authorized_continuation(
    result: Option<AuthorizeResult>,
    capability_id: &CapabilityId,
    runtime: RuntimeKind,
    process_id: ProcessId,
) -> Result<Option<ProcessAuthorizedContinuation>, CapabilityInvocationError> {
    match result {
        Some(AuthorizeResult::Authorized(authorized)) => {
            ProcessAuthorizedContinuation::from_authorized(*authorized, Utc::now(), process_id)
                .map(Some)
                .map_err(|authorized| {
                    let reservation = authorized.abort();
                    if reservation.is_some() {
                        tracing::warn!(
                            process_id = %process_id,
                            capability_id = %capability_id,
                            "spawn authorization witness expired before process start; reservation returned to obligation abort path"
                        );
                    }
                    CapabilityInvocationError::from(DispatchError::AuthorizationExpired {
                        capability: capability_id.clone(),
                    })
                })
        }
        None if runtime == RuntimeKind::System => Ok(None),
        _ => Err(CapabilityInvocationError::from(
            DispatchError::MissingAuthorization {
                capability: capability_id.clone(),
            },
        )),
    }
}

impl<'a, D> CapabilityHost<'a, D>
where
    D: CapabilityDispatcher + ?Sized,
{
    pub fn new(
        registry: &'a ExtensionRegistry,
        dispatcher: &'a D,
        authorizer: &'a dyn TrustAwareCapabilityDispatchAuthorizer,
        trust_policy: &'a dyn TrustPolicy,
        runtime_policy: &'a EffectiveRuntimePolicy,
        policy_facts: &'a dyn HostPolicyFacts,
    ) -> Self {
        Self {
            registry,
            dispatcher,
            authorizer,
            trust_policy,
            runtime_policy,
            policy_facts,
            run_state: None,
            approval_requests: None,
            run_state_approval_store: None,
            capability_leases: None,
            process_manager: None,
            obligation_handler: None,
        }
    }

    /// Attaches the run-state store used to record invocation lifecycle.
    ///
    /// Required for `resume_json`. Strongly recommended for `invoke_json` and
    /// `spawn_json` so denials, obligation rejections, and dispatch failures
    /// transition the run record to `Failed` instead of being silently
    /// dropped. Without it, error paths still return the right user-facing
    /// error but no run record is persisted.
    pub fn with_run_state(mut self, run_state: &'a dyn RunStateStore) -> Self {
        self.run_state = Some(run_state);
        self.run_state_approval_store = None;
        self
    }

    /// Attaches the approval-request store used to persist approval prompts.
    ///
    /// Required for `invoke_json` paths whose authorizer returns
    /// `Decision::RequireApproval` and for `resume_json`. Without it, an
    /// approval-required dispatch fails with `ApprovalStoreMissing` rather
    /// than blocking for human review.
    pub fn with_approval_requests(
        mut self,
        approval_requests: &'a dyn ApprovalRequestStore,
    ) -> Self {
        self.approval_requests = Some(approval_requests);
        self.run_state_approval_store = None;
        self
    }

    /// Attaches a combined durable run-state/approval store that can persist a
    /// pending approval and transition the invocation to `BlockedApproval` in one
    /// transaction. Production composition should prefer this over separate
    /// stores when both records live in the same backend.
    pub fn with_run_state_approval_store(mut self, store: &'a dyn RunStateApprovalStore) -> Self {
        self.run_state = Some(store);
        self.approval_requests = Some(store);
        self.run_state_approval_store = Some(store);
        self
    }

    /// Attaches the capability-lease store used to consume approved leases.
    ///
    /// Required for `resume_json`; not consulted by `invoke_json` or
    /// `spawn_json`.
    pub fn with_capability_leases(
        mut self,
        capability_leases: &'a dyn CapabilityLeaseStore,
    ) -> Self {
        self.capability_leases = Some(capability_leases);
        self
    }

    /// Attaches the process manager used to spawn long-running invocations.
    ///
    /// Required for `spawn_json`; not consulted by `invoke_json` or
    /// `resume_json`. Without it, `spawn_json` fails with
    /// `ProcessManagerMissing`.
    pub fn with_process_manager(mut self, process_manager: &'a dyn ProcessManager) -> Self {
        self.process_manager = Some(process_manager);
        self
    }

    /// Attaches the obligation handler that satisfies allow-decision
    /// obligations before/after side effects. Without a handler, non-empty
    /// obligations fail closed.
    pub fn with_obligation_handler(mut self, handler: &'a dyn CapabilityObligationHandler) -> Self {
        self.obligation_handler = Some(handler);
        self
    }

    #[tracing::instrument(
        level = "debug",
        skip(self, request),
        fields(
            invocation_id = %request.context.invocation_id,
            capability_id = %request.capability_id,
            scope = ?request.context.resource_scope,
        )
    )]
    pub async fn invoke_json(
        &self,
        request: CapabilityInvocationRequest,
    ) -> Result<CapabilityInvocationResult, CapabilityInvocationError> {
        let invocation_id = request.context.invocation_id;
        let capability_id = request.capability_id.clone();
        let scope = request.context.resource_scope.clone();

        // The whole pre-dispatch authority fold — context validation,
        // fingerprint, run-state start, capability lookup, trust-aware
        // authorization, obligation preparation, and (Slice C) minting the
        // sealed `Authorized` witness — is one method. `invoke_json` maps its
        // `AuthorizeResult` back to today's exact dispatch and error behavior.
        let (obligations, obligation_outcome, authorized) = match self.authorize(&request).await? {
            AuthorizeFold::Authorized(fold) => {
                let AuthorizedFold {
                    result,
                    frozen_deadline: _,
                    obligations,
                    obligation_outcome,
                } = *fold;
                debug!(
                    authorize_result = ?result.as_ref().map(AuthorizeResult::kind),
                    obligation_count = obligations.len(),
                    "capability authorization allowed dispatch"
                );
                let authorized = match authorized_dispatch_witness(result, &capability_id) {
                    Ok(authorized) => authorized,
                    Err(error) => {
                        self.abort_obligations(
                            CapabilityObligationPhase::Invoke,
                            &request.context,
                            &request.capability_id,
                            &request.estimate,
                            obligations.as_slice(),
                            &obligation_outcome,
                        )
                        .await;
                        apply_run_state_transition_if_configured(
                            self.run_state,
                            &scope,
                            invocation_id,
                            &error,
                        )
                        .await;
                        return Err(error);
                    }
                };
                (obligations, obligation_outcome, authorized)
            }
            AuthorizeFold::Denied { result, reason } => {
                debug!(
                    authorize_result = %result.kind(),
                    reason = ?reason,
                    "capability authorization denied dispatch"
                );
                return Err(CapabilityInvocationError::AuthorizationDenied {
                    capability: request.capability_id,
                    reason,
                    detail: None,
                });
            }
            AuthorizeFold::Blocked { result } => {
                debug!(
                    authorize_result = %result.kind(),
                    "capability authorization requires approval"
                );
                return Err(CapabilityInvocationError::AuthorizationRequiresApproval {
                    capability: request.capability_id,
                });
            }
        };

        debug!("capability dispatch starting");
        let dispatch = match self.dispatcher.dispatch_json(*authorized).await {
            Ok(dispatch) => {
                debug!(
                    provider = %dispatch.provider,
                    runtime = ?dispatch.runtime,
                    "capability dispatch completed"
                );
                dispatch
            }
            Err(error) => {
                debug!(
                    dispatch_failure_kind = %error.failure_kind(),
                    "capability dispatch failed"
                );
                self.abort_obligations(
                    CapabilityObligationPhase::Invoke,
                    &request.context,
                    &request.capability_id,
                    &request.estimate,
                    obligations.as_slice(),
                    &obligation_outcome,
                )
                .await;
                let error =
                    enrich_dispatch_error_credential_requirements(error, obligations.as_slice());
                let invocation_error = CapabilityInvocationError::from(error);
                apply_run_state_transition_if_configured(
                    self.run_state,
                    &scope,
                    invocation_id,
                    &invocation_error,
                )
                .await;
                return Err(invocation_error);
            }
        };

        let dispatch = match self
            .complete_dispatch_obligations(
                CapabilityObligationPhase::Invoke,
                &request.context,
                &request.capability_id,
                &request.estimate,
                obligations.as_slice(),
                &dispatch,
            )
            .await
        {
            Ok(dispatch) => dispatch,
            Err(error) => {
                debug!(
                    error_kind = obligation_invocation_error_kind(&error),
                    "capability invoke obligation completion failed"
                );
                let cleanup_outcome = CapabilityObligationOutcome::default();
                self.abort_obligations(
                    CapabilityObligationPhase::Invoke,
                    &request.context,
                    &request.capability_id,
                    &request.estimate,
                    obligations.as_slice(),
                    &cleanup_outcome,
                )
                .await;
                fail_run_if_configured(
                    self.run_state,
                    &scope,
                    invocation_id,
                    obligation_invocation_error_kind(&error),
                )
                .await;
                return Err(error);
            }
        };

        if let Some(run_state) = self.run_state {
            complete_run_after_side_effect(
                run_state,
                &scope,
                invocation_id,
                &capability_id,
                "dispatch",
            )
            .await;
            debug!("capability run state completed");
        }

        debug!("capability invocation completed");
        Ok(CapabilityInvocationResult { dispatch })
    }

    /// The pre-dispatch authority fold for `invoke_json`, extracted per
    /// arch-simplification §9 step 2 / §5.3.2: validate the context, fingerprint
    /// the invocation, start the run record, resolve the descriptor, run
    /// trust-aware authorization, and on `Allow` prepare obligations and mint
    /// the sealed [`Authorized`] witness. Every side effect that today's inline
    /// fold performed — run-state `start`/`fail`/`block`, approval
    /// persist-and-rollback, obligation `prepare`, and each early error return —
    /// stays here, verbatim; `invoke_json` only maps the returned
    /// [`AuthorizeFold`] back to today's outcome.
    /// Compute provider trust for `capability_id` (§5.3.2/§9): the kernel now
    /// classifies trust itself instead of trusting a caller-stamped field.
    fn evaluate_trust(
        &self,
        capability_id: &CapabilityId,
    ) -> Result<TrustDecision, CapabilityInvocationError> {
        evaluate_invocation_trust(self.registry, self.trust_policy, capability_id)
            .map_err(|error| trust_error_to_invocation_error(capability_id, error))
    }

    /// Enforce runtime policy for `descriptor` (relocated from host_runtime's
    /// `enforce_runtime_policy`). A planner refusal is a model-visible
    /// `AuthorizationDenied` (-> `Authorization` failure kind), matching today's
    /// `runtime_policy_failure`.
    fn enforce_runtime_policy(
        &self,
        descriptor: &CapabilityDescriptor,
    ) -> Result<(), CapabilityInvocationError> {
        match plan_capability(descriptor, self.runtime_policy) {
            Ok(_plan) => Ok(()),
            Err(error) => Err(runtime_policy_error_to_invocation_error(
                &descriptor.id,
                error,
            )),
        }
    }

    /// Persistent-approval fold (§5.2.7/§5.3.2): a prior scoped approval may
    /// already authorize this invocation. Relocated from host_runtime's former
    /// `apply_persistent_approval_policy`: only for permission modes that allow
    /// it, re-authorize with each candidate grant injected; adopt the first grant
    /// that flips the decision to `Allow`, so no fresh approval gate is raised.
    ///
    /// The kernel owns the re-authorize decision because it holds the authorizer;
    /// [`HostPolicyFacts::persistent_grants`] only surfaces the candidate grants.
    /// Mutates `authorize_context` in place — pushing the adopted grant so the
    /// subsequent main authorization allows without approval. A no-op when the
    /// permission mode forbids persistent approval or no candidate grant flips the
    /// decision, leaving `authorize_context` untouched.
    ///
    /// Returns the adopted grant's `constraints.expires_at` (a frozen fact the
    /// seal's deadline is derived from), or `None` when no grant is adopted or the
    /// adopted grant has no expiry.
    ///
    /// This adds a second authorizer invocation per candidate grant (the re-auth
    /// probe), exactly as the host_runtime implementation did; the loop is bounded
    /// to the grants the port returns.
    async fn apply_persistent_approval(
        &self,
        authorize_context: &mut ExecutionContext,
        descriptor: &CapabilityDescriptor,
        capability_id: &CapabilityId,
        estimate: &ResourceEstimate,
        trust_decision: &TrustDecision,
        action: PolicyAction,
    ) -> Option<Timestamp> {
        if !permission_mode_allows_persistent_approval(descriptor.default_permission) {
            debug!(
                capability_id = %capability_id,
                permission = ?descriptor.default_permission,
                "persistent approval skipped for manifest policy"
            );
            return None;
        }
        let grants = self
            .policy_facts
            .persistent_grants(capability_id, authorize_context, action)
            .await;
        for grant in grants {
            // Mirror host_runtime's `apply_persistent_approval_policy`: clear the
            // candidate's grants and inject exactly this single grant, then
            // re-authorize with the SAME authorizer method the action uses.
            let mut candidate = authorize_context.clone();
            candidate.grants.grants.clear();
            candidate.grants.grants.push(grant.clone());
            let decision = match action {
                PolicyAction::Dispatch => {
                    self.authorizer
                        .authorize_dispatch_with_trust(
                            &candidate,
                            descriptor,
                            estimate,
                            trust_decision,
                        )
                        .await
                }
                PolicyAction::SpawnCapability => {
                    self.authorizer
                        .authorize_spawn_with_trust(
                            &candidate,
                            descriptor,
                            estimate,
                            trust_decision,
                        )
                        .await
                }
            };
            if let Decision::Allow { .. } = decision {
                debug!(
                    capability_id = %capability_id,
                    "persistent approval policy matched; injecting scoped grant"
                );
                let adopted_expiry = grant.constraints.expires_at;
                authorize_context.grants.grants.push(grant);
                return adopted_expiry;
            }
        }
        None
    }

    async fn authorize(
        &self,
        request: &CapabilityInvocationRequest,
    ) -> Result<AuthorizeFold, CapabilityInvocationError> {
        let invocation_id = request.context.invocation_id;
        let scope = request.context.resource_scope.clone();
        if request.context.validate().is_err() {
            debug!("capability invocation rejected invalid execution context");
            return Err(CapabilityInvocationError::AuthorizationDenied {
                capability: request.capability_id.clone(),
                reason: DenyReason::InternalInvariantViolation,
                detail: None,
            });
        }
        debug!("capability invocation started");

        let invocation_fingerprint = invocation_fingerprint_for_kind(
            CapabilityActionKind::Dispatch,
            &scope,
            &request.capability_id,
            &request.estimate,
            &request.input,
        )
        .map_err(|source| CapabilityInvocationError::InvocationFingerprint {
            capability: request.capability_id.clone(),
            source,
        })?;

        // Resolve the descriptor BEFORE starting a run record: an unknown
        // capability must short-circuit without creating a run record (restoring
        // the behavior host_runtime's deleted pre-check provided). Neither the
        // fingerprint above nor `run_state.start` below needs the descriptor, so
        // hoisting this lookup is safe; everything from `start` onward keeps its
        // original order (the credential pre-flight still runs after `start`).
        let Some(descriptor) = self.registry.get_capability(&request.capability_id) else {
            debug!("capability invocation failed before authorization: unknown capability");
            return Err(CapabilityInvocationError::UnknownCapability {
                capability: request.capability_id.clone(),
            });
        };

        if let Some(run_state) = self.run_state {
            run_state
                .start(RunStart {
                    invocation_id,
                    capability_id: request.capability_id.clone(),
                    scope: scope.clone(),
                    authenticated_actor_user_id: request
                        .context
                        .authenticated_actor_user_id
                        .clone(),
                })
                .await?;
            debug!("capability run state started");
        }

        // Kernel-computed trust + in-fold runtime-policy planning (§5.3.2/§9),
        // relocated from host_runtime's `open_pre_authorization`. The
        // `context.trust` stamp reproduces what `open_pre_authorization` did
        // before calling the authorizer.
        let trust_decision = match self.evaluate_trust(&request.capability_id) {
            Ok(d) => d,
            Err(error) => {
                apply_run_state_transition_if_configured(
                    self.run_state,
                    &scope,
                    invocation_id,
                    &error,
                )
                .await;
                return Err(error);
            }
        };
        if let Err(error) = self.enforce_runtime_policy(descriptor) {
            apply_run_state_transition_if_configured(self.run_state, &scope, invocation_id, &error)
                .await;
            return Err(error);
        }

        // Credential pre-flight (§5.3.2/§9), relocated from host_runtime's
        // `credential_preflight_check`. Ordered credential-before-approval on
        // purpose: a missing credential surfaces as `AuthorizationRequiresAuth`
        // *before* the authorizer's approval decision, so a human approval is
        // never consumed for an action that cannot yet execute. The port returns
        // facts only; the kernel maps them. `Indeterminate` (transient store
        // fault) skips the pre-flight — the dispatch-time obligation check is the
        // enforcing backstop and a fault must not burn a user auth interaction.
        match self
            .policy_facts
            .credential_presence(&request.capability_id, &scope)
            .await
        {
            CredentialPresence::Satisfied | CredentialPresence::Indeterminate => {}
            CredentialPresence::Missing {
                required_secrets,
                requirements,
            } => {
                let error = CapabilityInvocationError::AuthorizationRequiresAuth {
                    capability: request.capability_id.clone(),
                    required_secrets,
                    credential_requirements: requirements,
                };
                apply_run_state_transition_if_configured(
                    self.run_state,
                    &scope,
                    invocation_id,
                    &error,
                )
                .await;
                return Err(error);
            }
        }

        let mut authorize_context = request.context.clone();
        authorize_context.trust = trust_decision.effective_trust.class();

        let frozen_deadline = self
            .apply_persistent_approval(
                &mut authorize_context,
                descriptor,
                &request.capability_id,
                &request.estimate,
                &trust_decision,
                PolicyAction::Dispatch,
            )
            .await;

        match self
            .authorizer
            .authorize_dispatch_with_trust(
                &authorize_context,
                descriptor,
                &request.estimate,
                &trust_decision,
            )
            .await
        {
            Decision::Allow {
                obligations: allowed_obligations,
            } => {
                let allowed_obligations = allowed_obligations.into_vec();
                debug!(
                    obligation_count = allowed_obligations.len(),
                    "capability authorization allowed dispatch"
                );
                let obligation_outcome = match self
                    .prepare_obligations(
                        CapabilityObligationPhase::Invoke,
                        &authorize_context,
                        &request.capability_id,
                        &request.estimate,
                        allowed_obligations.clone(),
                    )
                    .await
                {
                    Ok(outcome) => {
                        debug!("capability invoke obligations prepared");
                        outcome
                    }
                    Err(error) => {
                        debug!(
                            error_kind = obligation_invocation_error_kind(&error),
                            "capability invoke obligation preparation failed"
                        );
                        apply_run_state_transition_if_configured(
                            self.run_state,
                            &scope,
                            invocation_id,
                            &error,
                        )
                        .await;
                        return Err(error);
                    }
                };
                let result = self.seal_authorization(
                    &authorize_context,
                    &request.capability_id,
                    &request.estimate,
                    &request.input,
                    descriptor,
                    &obligation_outcome,
                    frozen_deadline,
                );
                Ok(AuthorizeFold::Authorized(Box::new(AuthorizedFold {
                    result,
                    frozen_deadline: None,
                    obligations: allowed_obligations,
                    obligation_outcome,
                })))
            }
            Decision::Deny { reason } => {
                debug!(
                    reason = ?reason,
                    "capability authorization denied dispatch"
                );
                fail_run_if_configured(
                    self.run_state,
                    &scope,
                    invocation_id,
                    "AuthorizationDenied",
                )
                .await;
                Ok(AuthorizeFold::Denied {
                    result: AuthorizeResult::Denied(DenyRef::new()),
                    reason,
                })
            }
            Decision::RequireApproval {
                request: mut approval,
            } => {
                let approval_request_id = approval.id;
                add_capability_input_display_hint(
                    &mut approval.reason,
                    &request.capability_id,
                    &request.input,
                );
                debug!(
                    approval_request_id = %approval_request_id,
                    "capability authorization requires approval"
                );
                if let Err(error) = validate_approval_request_matches_invocation(
                    &approval,
                    &request.context,
                    &request.capability_id,
                    &request.estimate,
                    CapabilityActionKind::Dispatch,
                ) {
                    debug!(
                        approval_request_id = %approval_request_id,
                        "capability approval request did not match invocation"
                    );
                    fail_run_if_configured(
                        self.run_state,
                        &scope,
                        invocation_id,
                        "ApprovalRequestMismatch",
                    )
                    .await;
                    return Err(error);
                }

                if let Some(existing) = &approval.invocation_fingerprint {
                    if existing != &invocation_fingerprint {
                        debug!(
                            approval_request_id = %approval_request_id,
                            "capability approval fingerprint mismatch"
                        );
                        fail_run_if_configured(
                            self.run_state,
                            &scope,
                            invocation_id,
                            "InvocationFingerprintMismatch",
                        )
                        .await;
                        return Err(CapabilityInvocationError::ApprovalFingerprintMismatch {
                            capability: request.capability_id.clone(),
                        });
                    }
                } else {
                    approval.invocation_fingerprint = Some(invocation_fingerprint);
                }

                match (self.run_state, self.approval_requests) {
                    (Some(run_state), Some(approval_requests)) => {
                        if let Some(combined_store) = self.run_state_approval_store {
                            if let Err(error) = combined_store
                                .save_pending_and_block_approval(
                                    scope.clone(),
                                    invocation_id,
                                    approval,
                                )
                                .await
                            {
                                debug!(
                                    approval_request_id = %approval_request_id,
                                    "capability approval block failed in combined store"
                                );
                                fail_run_if_configured(
                                    Some(run_state),
                                    &scope,
                                    invocation_id,
                                    "ApprovalBlock",
                                )
                                .await;
                                return Err(CapabilityInvocationError::from(error));
                            }
                            debug!(
                                approval_request_id = %approval_request_id,
                                "capability approval persisted and run state blocked"
                            );
                        } else {
                            let approval_id = approval.id;
                            if let Err(error) = approval_requests
                                .save_pending(scope.clone(), approval.clone())
                                .await
                            {
                                debug!(
                                    approval_request_id = %approval_id,
                                    "capability approval request persistence failed"
                                );
                                fail_run_if_configured(
                                    Some(run_state),
                                    &scope,
                                    invocation_id,
                                    "ApprovalStore",
                                )
                                .await;
                                return Err(CapabilityInvocationError::from(error));
                            }
                            if let Err(error) = run_state
                                .block_approval(&scope, invocation_id, approval)
                                .await
                            {
                                debug!(
                                    approval_request_id = %approval_id,
                                    "capability run state approval block failed"
                                );
                                if let Err(discard_error) =
                                    approval_requests.discard_pending(&scope, approval_id).await
                                {
                                    warn!(
                                        approval_request_id = %approval_id,
                                        invocation_id = %invocation_id,
                                        transition_error_kind = run_state_error_kind(&discard_error),
                                        "approval rollback failed after run-state block transition failed",
                                    );
                                }
                                fail_run_if_configured(
                                    Some(run_state),
                                    &scope,
                                    invocation_id,
                                    "ApprovalBlock",
                                )
                                .await;
                                return Err(CapabilityInvocationError::from(error));
                            }
                            debug!(
                                approval_request_id = %approval_id,
                                "capability approval persisted and run state blocked"
                            );
                        }
                    }
                    (Some(run_state), None) => {
                        debug!(
                            approval_request_id = %approval_request_id,
                            store = "approval_requests",
                            "capability approval cannot block because store is missing"
                        );
                        fail_run_if_configured(
                            Some(run_state),
                            &scope,
                            invocation_id,
                            "ApprovalStoreMissing",
                        )
                        .await;
                        return Err(CapabilityInvocationError::ApprovalStoreMissing {
                            capability: request.capability_id.clone(),
                            store: "approval_requests",
                        });
                    }
                    (None, Some(_)) => {
                        debug!(
                            approval_request_id = %approval_request_id,
                            store = "run_state",
                            "capability approval cannot block because store is missing"
                        );
                        return Err(CapabilityInvocationError::ApprovalStoreMissing {
                            capability: request.capability_id.clone(),
                            store: "run_state",
                        });
                    }
                    (None, None) => {
                        debug!(
                            approval_request_id = %approval_request_id,
                            store = "run_state and approval_requests",
                            "capability approval cannot block because stores are missing"
                        );
                        return Err(CapabilityInvocationError::ApprovalStoreMissing {
                            capability: request.capability_id.clone(),
                            store: "run_state and approval_requests",
                        });
                    }
                }
                Ok(AuthorizeFold::Blocked {
                    result: AuthorizeResult::Blocked(Blocked::Approval(GateWaypoint::new(
                        GateRef::for_approval_request(approval_request_id),
                    ))),
                })
            }
        }
    }

    /// Mint the sealed [`Authorized`] witness for an allowed invoke, spawn, or
    /// resume (arch-simplification §5.3.2).
    ///
    /// Actor and origin are authoritative frozen facts: actor-less contexts seal
    /// [`Actor::System`] rather than falling back to `user_id`, and origin comes
    /// from the ingress-stamped context, with `run_id` reconstruction preserved
    /// for transitional loop callers. Returns `None` only for a host-internal
    /// `System` runtime with no untrusted [`RuntimeLane`], or for a defensive
    /// origin-less context shape no production ingress should produce.
    ///
    /// Shared by the invoke, spawn, and resume authorize folds so the same six
    /// frozen facts seal every path (§9 step 2). `scope` is derived from
    /// `context.resource_scope` — every caller's `scope` local is exactly that
    /// value (`request.context.resource_scope.clone()`), so passing it separately
    /// would only duplicate it.
    // arch-exempt: too_many_args, seals independent frozen facts from three call sites (invoke/spawn/resume) with differing sources, so no single request/context bundle unifies them; arg list shrinks as later slices route dispatch through the witness, plan #6175
    #[allow(clippy::too_many_arguments)]
    fn seal_authorization(
        &self,
        context: &ExecutionContext,
        capability_id: &CapabilityId,
        estimate: &ResourceEstimate,
        input: &serde_json::Value,
        descriptor: &CapabilityDescriptor,
        obligation_outcome: &CapabilityObligationOutcome,
        frozen_deadline: Option<Timestamp>,
    ) -> Option<AuthorizeResult> {
        // Actor is sealed at the membrane; NO fallback to `user_id`. An
        // actor-less (system service / one-shot) context seals `Actor::System`
        // as its own class.
        let actor = match context.authenticated_actor_user_id.clone() {
            Some(user_id) => Actor::Sealed(user_id),
            None => Actor::System,
        };
        // Lane resolved from the descriptor's runtime kind; `System` runtimes
        // have no untrusted execution lane (`None`) and are not sealed here.
        let lane = RuntimeLane::from_runtime_kind(descriptor.runtime)?;
        let scope = &context.resource_scope;
        // Origin is the ingress-stamped authority fact (§5.2.1). The loop path
        // also carries `run_id`, so a context that stamped only `run_id` still
        // reconstructs `LoopRun` for transitional compatibility.
        let origin = context.resolved_origin()?;
        let invocation = Invocation {
            activity_id: ActivityId::from_uuid(context.invocation_id.as_uuid()),
            capability: capability_id.clone(),
            // PROVISIONAL (Slice C): the loop expresses input by reference; the
            // membrane will resolve it. Cloned here so today's dispatch keeps
            // ownership of the request `input`.
            input: input.clone(),
            scope: scope.clone(),
            actor,
            origin,
            estimate: estimate.clone(),
            correlation_id: context.correlation_id,
            process_id: context.process_id,
            parent_process_id: context.parent_process_id,
        };
        // Keep the fold's mounts verbatim. `None` means the capability declared
        // no mount obligation; it is not equivalent to an empty mount view.
        let mounts = obligation_outcome.mounts.clone();
        // The real reservation the fold's `ReserveResources` obligation produced
        // (the estimate is already reserved in-fold), or `None` when the
        // capability declares no resource obligation. No synthesized placeholder.
        let reservation = obligation_outcome.resource_reservation.clone();
        // Deadline from the shortest-lived frozen fact (the caller pre-min's its
        // candidates into `frozen_deadline`), or a bounded default TTL. See
        // [`witness_deadline`].
        let deadline = witness_deadline([frozen_deadline]);
        Some(AuthorizeResult::Authorized(Box::new(Authorized::seal(
            self.authorization_grant(),
            invocation,
            lane,
            mounts,
            reservation,
            deadline,
        ))))
    }

    pub async fn resume_json(
        &self,
        request: CapabilityResumeRequest,
    ) -> Result<CapabilityInvocationResult, CapabilityInvocationError> {
        let run_state =
            self.run_state
                .ok_or_else(|| CapabilityInvocationError::ResumeStoreMissing {
                    capability: request.capability_id.clone(),
                    store: "run_state",
                })?;
        let approval_requests = self.approval_requests.ok_or_else(|| {
            CapabilityInvocationError::ResumeStoreMissing {
                capability: request.capability_id.clone(),
                store: "approval_requests",
            }
        })?;
        let capability_leases = self.capability_leases.ok_or_else(|| {
            CapabilityInvocationError::ResumeStoreMissing {
                capability: request.capability_id.clone(),
                store: "capability_leases",
            }
        })?;

        let invocation_id = request.context.invocation_id;
        let capability_id = request.capability_id.clone();
        let scope = request.context.resource_scope.clone();
        if request.context.validate().is_err() {
            return Err(CapabilityInvocationError::AuthorizationDenied {
                capability: request.capability_id,
                reason: DenyReason::InternalInvariantViolation,
                detail: None,
            });
        }

        // Resume-path pre-authorization (§5.3.2/§9, R-A): resolve the descriptor
        // and enforce runtime-policy planning BEFORE the run-state lookup so an
        // unknown capability short-circuits to `UnknownCapability`
        // (→ `MissingRuntime`) instead of the run-state-not-found `Backend` path,
        // and a policy tightened between invoke and resume fails closed. On
        // refusal only the matching `BlockedApproval` run is failed.
        self.resume_preflight(
            &request.context,
            &request.capability_id,
            BlockedResumeKind::Approval {
                approval_request_id: request.approval_request_id,
            },
        )
        .await?;

        let invocation_fingerprint = invocation_fingerprint_for_kind(
            CapabilityActionKind::Dispatch,
            &scope,
            &request.capability_id,
            &request.estimate,
            &request.input,
        )
        .map_err(|source| CapabilityInvocationError::InvocationFingerprint {
            capability: request.capability_id.clone(),
            source,
        })?;

        let run_record = run_state
            .get(&scope, invocation_id)
            .await?
            .ok_or(RunStateError::UnknownInvocation { invocation_id })?;
        if run_record.authenticated_actor_user_id != request.context.authenticated_actor_user_id {
            return Err(CapabilityInvocationError::AuthorizationDenied {
                capability: request.capability_id,
                reason: DenyReason::PolicyDenied,
                detail: None,
            });
        }
        if run_record.status != RunStatus::BlockedApproval {
            return Err(CapabilityInvocationError::ResumeNotBlocked {
                capability: request.capability_id,
                status: run_record.status,
            });
        }
        let capability_mismatch = run_record.capability_id != request.capability_id;
        let approval_request_mismatch =
            run_record.approval_request_id != Some(request.approval_request_id);
        if capability_mismatch || approval_request_mismatch {
            fail_run_if_configured(
                Some(run_state),
                &scope,
                invocation_id,
                "ResumeContextMismatch",
            )
            .await;
            return Err(CapabilityInvocationError::ResumeContextMismatch {
                capability: request.capability_id,
                kind: resume_context_mismatch_kind(capability_mismatch, approval_request_mismatch),
            });
        }

        let approval = approval_requests
            .get(&scope, request.approval_request_id)
            .await?
            .ok_or(RunStateError::UnknownApprovalRequest {
                request_id: request.approval_request_id,
            })?;
        if approval.status != ApprovalStatus::Approved {
            if approval.status != ApprovalStatus::Pending {
                fail_run_if_configured(
                    Some(run_state),
                    &scope,
                    invocation_id,
                    approval_not_approved_error_kind(approval.status),
                )
                .await;
            }
            return Err(CapabilityInvocationError::ApprovalNotApproved {
                capability: request.capability_id,
                status: approval.status,
            });
        }
        if let Err(error) = validate_approval_request_matches_invocation(
            &approval.request,
            &request.context,
            &request.capability_id,
            &request.estimate,
            CapabilityActionKind::Dispatch,
        ) {
            fail_run_if_configured(
                Some(run_state),
                &scope,
                invocation_id,
                "ApprovalRequestMismatch",
            )
            .await;
            return Err(error);
        }
        if approval.request.invocation_fingerprint.as_ref() != Some(&invocation_fingerprint) {
            fail_run_if_configured(
                Some(run_state),
                &scope,
                invocation_id,
                "InvocationFingerprintMismatch",
            )
            .await;
            return Err(CapabilityInvocationError::ApprovalFingerprintMismatch {
                capability: request.capability_id,
            });
        }

        let Some(descriptor) = self.registry.get_capability(&request.capability_id) else {
            fail_run_if_configured(Some(run_state), &scope, invocation_id, "UnknownCapability")
                .await;
            return Err(CapabilityInvocationError::UnknownCapability {
                capability: request.capability_id,
            });
        };

        let Some(lease) = matching_approval_lease(
            capability_leases,
            &request.context,
            &request.capability_id,
            &invocation_fingerprint,
        )
        .await
        else {
            fail_run_if_configured(
                Some(run_state),
                &scope,
                invocation_id,
                "ApprovalLeaseMissing",
            )
            .await;
            return Err(CapabilityInvocationError::ApprovalLeaseMissing {
                capability: request.capability_id,
            });
        };
        let mut authorized_context = request.context.clone();
        authorized_context.grants.grants.push(lease.grant.clone());
        // The lease is claimed INSIDE `dispatch_resumed_capability`, after
        // `authorize_dispatch_with_trust` returns Allow.  Deferring the claim
        // preserves the original contract: a Deny leaves the lease Active.
        let grant_id = lease.grant.id;
        // Carry the lease expiry onto the pending-claim spec so the sealed
        // witness minted in `authorize_resumed` is bounded by the approval that
        // authorized it (the claim, and thus a readable claimed lease, happens
        // only after the seal).
        let grant_expiry = lease.grant.constraints.expires_at;

        self.dispatch_resumed_capability(ResumedDispatchParams {
            run_state,
            scope,
            invocation_id,
            capability_id,
            estimate: request.estimate,
            input: request.input,
            authorized_context,
            descriptor,
            lease_state: ResumedLeaseState::PendingClaim(PendingClaimAfterAuth {
                leases: capability_leases,
                grant_id,
                fingerprint: invocation_fingerprint,
                grant_expiry,
            }),
        })
        .await
    }

    /// Resume an invocation that was previously blocked at an auth gate.
    ///
    /// Validates that the run record is in `BlockedAuth` status.  When the
    /// invocation also passed an earlier approval gate (`approval_request_id`
    /// is `Some`), validates and claims the fingerprinted approval lease before
    /// dispatch so the prior approval is honoured without a second approval
    /// prompt.  When `approval_request_id` is `None` no lease step is needed
    /// and the path falls through to normal authorization + dispatch.
    pub async fn auth_resume_json(
        &self,
        request: CapabilityAuthResumeRequest,
    ) -> Result<CapabilityInvocationResult, CapabilityInvocationError> {
        let run_state =
            self.run_state
                .ok_or_else(|| CapabilityInvocationError::ResumeStoreMissing {
                    capability: request.capability_id.clone(),
                    store: "run_state",
                })?;

        let invocation_id = request.context.invocation_id;
        let capability_id = request.capability_id.clone();
        let scope = request.context.resource_scope.clone();
        if request.context.validate().is_err() {
            return Err(CapabilityInvocationError::AuthorizationDenied {
                capability: request.capability_id,
                reason: DenyReason::InternalInvariantViolation,
                detail: None,
            });
        }

        // Resume-path pre-authorization (§5.3.2/§9, R-A): descriptor + runtime-policy
        // planning BEFORE the run-state lookup (see `resume_json`). On refusal only
        // the matching `BlockedAuth` run is failed — `approval_request_id` is NOT
        // compared, because `block_auth` clears it to `None` on the record.
        self.resume_preflight(
            &request.context,
            &request.capability_id,
            BlockedResumeKind::Auth,
        )
        .await?;

        let run_record = run_state
            .get(&scope, invocation_id)
            .await?
            .ok_or(RunStateError::UnknownInvocation { invocation_id })?;
        if run_record.authenticated_actor_user_id != request.context.authenticated_actor_user_id {
            return Err(CapabilityInvocationError::AuthorizationDenied {
                capability: request.capability_id,
                reason: DenyReason::PolicyDenied,
                detail: None,
            });
        }
        if run_record.status != RunStatus::BlockedAuth {
            return Err(CapabilityInvocationError::ResumeNotBlocked {
                capability: request.capability_id,
                status: run_record.status,
            });
        }
        // Verify the capability_id on the request matches the one recorded in
        // the run state when the run was originally started.  A mismatch means
        // the caller is trying to resume a different capability than the one
        // that was blocked — treat it as a context mismatch and fail the run.
        if run_record.capability_id != request.capability_id {
            fail_run_if_configured(
                Some(run_state),
                &scope,
                invocation_id,
                "ResumeContextMismatch",
            )
            .await;
            return Err(CapabilityInvocationError::ResumeContextMismatch {
                capability: request.capability_id,
                kind: resume_context_mismatch_kind(true, false),
            });
        }

        // Check that the capability still exists before acquiring or mutating any
        // approval lease.  Moving this check above the lease-acquisition block
        // ensures an unknown capability returns `UnknownCapability` without
        // touching the lease at all — preventing a one-shot lease from being
        // permanently stranded in `Claimed`/`Dispatching` when the capability
        // was unregistered between the original invocation and this resume.
        let Some(descriptor) = self.registry.get_capability(&request.capability_id) else {
            fail_run_if_configured(Some(run_state), &scope, invocation_id, "UnknownCapability")
                .await;
            return Err(CapabilityInvocationError::UnknownCapability {
                capability: request.capability_id,
            });
        };

        // When the invocation previously passed an approval gate, validate and
        // claim the fingerprinted approval lease so the existing approval
        // carries through without requiring a second human approval.
        //
        // `approval_lease_to_consume` tracks the lease that must be consumed
        // after a successful dispatch.  It is `Some` only when a lease was
        // found and used; the `None` branch (no prior approval) skips the
        // consume step entirely.
        let (authorized_context, approval_lease_to_consume) = if let Some(approval_request_id) =
            request.approval_request_id
        {
            let approval_requests = self.approval_requests.ok_or_else(|| {
                CapabilityInvocationError::ResumeStoreMissing {
                    capability: request.capability_id.clone(),
                    store: "approval_requests",
                }
            })?;
            let capability_leases = self.capability_leases.ok_or_else(|| {
                CapabilityInvocationError::ResumeStoreMissing {
                    capability: request.capability_id.clone(),
                    store: "capability_leases",
                }
            })?;

            let invocation_fingerprint = invocation_fingerprint_for_kind(
                CapabilityActionKind::Dispatch,
                &scope,
                &request.capability_id,
                &request.estimate,
                &request.input,
            )
            .map_err(|source| CapabilityInvocationError::InvocationFingerprint {
                capability: request.capability_id.clone(),
                source,
            })?;

            let approval = approval_requests
                .get(&scope, approval_request_id)
                .await?
                .ok_or(RunStateError::UnknownApprovalRequest {
                    request_id: approval_request_id,
                })?;
            if approval.status != ApprovalStatus::Approved {
                if approval.status != ApprovalStatus::Pending {
                    fail_run_if_configured(
                        Some(run_state),
                        &scope,
                        invocation_id,
                        approval_not_approved_error_kind(approval.status),
                    )
                    .await;
                }
                return Err(CapabilityInvocationError::ApprovalNotApproved {
                    capability: request.capability_id,
                    status: approval.status,
                });
            }
            if let Err(error) = validate_approval_request_matches_invocation(
                &approval.request,
                &request.context,
                &request.capability_id,
                &request.estimate,
                CapabilityActionKind::Dispatch,
            ) {
                fail_run_if_configured(
                    Some(run_state),
                    &scope,
                    invocation_id,
                    "ApprovalRequestMismatch",
                )
                .await;
                return Err(error);
            }
            if approval.request.invocation_fingerprint.as_ref() != Some(&invocation_fingerprint) {
                fail_run_if_configured(
                    Some(run_state),
                    &scope,
                    invocation_id,
                    "InvocationFingerprintMismatch",
                )
                .await;
                return Err(CapabilityInvocationError::ApprovalFingerprintMismatch {
                    capability: request.capability_id,
                });
            }

            // Try to find an Active lease (clean first-time path).
            let active_lease = matching_approval_lease(
                capability_leases,
                &request.context,
                &request.capability_id,
                &invocation_fingerprint,
            )
            .await;

            let claimed = if let Some(lease) = active_lease {
                // Fresh Active lease: claim it (Active→Claimed), then immediately
                // advance it to Dispatching via begin_dispatch_claimed.  This
                // ensures the in-flight single-winner fence covers the fresh path
                // just as it covers the reuse (already-Claimed) path below.
                // Without the second step a concurrent auth_resume_json that misses
                // the Active lease would find the Claimed lease in the reuse branch
                // and successfully call begin_dispatch_claimed itself — double-firing.
                let lease_id = lease.grant.id;
                let claimed = match capability_leases
                    .claim(&scope, lease_id, &invocation_fingerprint)
                    .await
                {
                    Ok(claimed) => claimed,
                    Err(error) => {
                        if claim_error_may_be_concurrent_resume(&error) {
                            warn!(
                                lease_id = %lease_id,
                                invocation_id = %invocation_id,
                                capability_id = %capability_id,
                                error_kind = capability_lease_error_kind(&error),
                                "approval lease claim lost to a concurrent auth-resume; leaving run state unchanged",
                            );
                        } else {
                            fail_run_if_configured(
                                Some(run_state),
                                &scope,
                                invocation_id,
                                "ApprovalLeaseClaim",
                            )
                            .await;
                        }
                        return Err(CapabilityInvocationError::Lease(Box::new(error)));
                    }
                };
                // Advance Claimed→Dispatching so the fence is set before dispatch.
                match capability_leases
                    .begin_dispatch_claimed(&scope, claimed.grant.id, &invocation_fingerprint)
                    .await
                {
                    Ok(dispatching_lease) => {
                        debug!(
                            lease_id = %dispatching_lease.grant.id,
                            invocation_id = %invocation_id,
                            capability_id = %capability_id,
                            "auth_resume fresh path advanced lease to Dispatching"
                        );
                        dispatching_lease
                    }
                    Err(error) => {
                        if claim_error_may_be_concurrent_resume(&error) {
                            warn!(
                                lease_id = %claimed.grant.id,
                                invocation_id = %invocation_id,
                                capability_id = %capability_id,
                                error_kind = capability_lease_error_kind(&error),
                                "approval lease reuse lost to a concurrent auth-resume; leaving run state unchanged",
                            );
                        } else {
                            fail_run_if_configured(
                                Some(run_state),
                                &scope,
                                invocation_id,
                                "ApprovalLeaseClaim",
                            )
                            .await;
                        }
                        return Err(CapabilityInvocationError::Lease(Box::new(error)));
                    }
                }
            } else if let Some(claimed_lease) = matching_claimed_approval_lease_for_auth_resume(
                capability_leases,
                &scope,
                &request.capability_id,
                &invocation_fingerprint,
            )
            .await
            {
                // Claimed lease from a prior resume_json auth bounce: atomically
                // transition it to Dispatching so exactly one concurrent auth-resume
                // wins the reuse race. The loser sees InactiveLease{Dispatching} and
                // bails — matching the Active-lease claim() loser path.
                match capability_leases
                    .begin_dispatch_claimed(&scope, claimed_lease.grant.id, &invocation_fingerprint)
                    .await
                {
                    Ok(dispatching_lease) => {
                        debug!(
                            lease_id = %dispatching_lease.grant.id,
                            invocation_id = %invocation_id,
                            capability_id = %capability_id,
                            approval_request_id = %approval_request_id,
                            "auth_resume won dispatch race for claimed approval lease"
                        );
                        dispatching_lease
                    }
                    Err(error) => {
                        if claim_error_may_be_concurrent_resume(&error) {
                            warn!(
                                lease_id = %claimed_lease.grant.id,
                                invocation_id = %invocation_id,
                                capability_id = %capability_id,
                                error_kind = capability_lease_error_kind(&error),
                                "approval lease reuse lost to a concurrent auth-resume; leaving run state unchanged",
                            );
                        } else {
                            fail_run_if_configured(
                                Some(run_state),
                                &scope,
                                invocation_id,
                                "ApprovalLeaseClaim",
                            )
                            .await;
                        }
                        return Err(CapabilityInvocationError::Lease(Box::new(error)));
                    }
                }
            } else {
                fail_run_if_configured(
                    Some(run_state),
                    &scope,
                    invocation_id,
                    "ApprovalLeaseMissing",
                )
                .await;
                return Err(CapabilityInvocationError::ApprovalLeaseMissing {
                    capability: request.capability_id,
                });
            };

            let mut ctx = request.context.clone();
            ctx.grants.grants.push(claimed.grant.clone());
            (ctx, Some((capability_leases, claimed)))
        } else {
            (request.context.clone(), None)
        };

        self.dispatch_resumed_capability(ResumedDispatchParams {
            run_state,
            scope,
            invocation_id,
            capability_id,
            estimate: request.estimate,
            input: request.input,
            authorized_context,
            descriptor,
            lease_state: match approval_lease_to_consume {
                Some((leases, lease)) => ResumedLeaseState::AlreadyClaimed(leases, Box::new(lease)),
                None => ResumedLeaseState::NoPriorLease,
            },
        })
        .await
    }

    pub async fn resume_spawn_json(
        &self,
        request: CapabilityResumeRequest,
    ) -> Result<CapabilitySpawnResult, CapabilityInvocationError> {
        let process_manager = self.process_manager.ok_or_else(|| {
            CapabilityInvocationError::ProcessManagerMissing {
                capability: request.capability_id.clone(),
            }
        })?;
        let run_state =
            self.run_state
                .ok_or_else(|| CapabilityInvocationError::ResumeStoreMissing {
                    capability: request.capability_id.clone(),
                    store: "run_state",
                })?;
        let approval_requests = self.approval_requests.ok_or_else(|| {
            CapabilityInvocationError::ResumeStoreMissing {
                capability: request.capability_id.clone(),
                store: "approval_requests",
            }
        })?;
        let capability_leases = self.capability_leases.ok_or_else(|| {
            CapabilityInvocationError::ResumeStoreMissing {
                capability: request.capability_id.clone(),
                store: "capability_leases",
            }
        })?;

        let invocation_id = request.context.invocation_id;
        let capability_id = request.capability_id.clone();
        let scope = request.context.resource_scope.clone();
        if request.context.validate().is_err() {
            return Err(CapabilityInvocationError::AuthorizationDenied {
                capability: request.capability_id,
                reason: DenyReason::InternalInvariantViolation,
                detail: None,
            });
        }

        // Resume-path pre-authorization (§5.3.2/§9, R-A): descriptor + runtime-policy
        // planning BEFORE the run-state lookup (see `resume_json`), so an unknown
        // capability short-circuits to `MissingRuntime` and a tightened policy fails
        // closed. On refusal only the matching `BlockedApproval` run is failed.
        self.resume_preflight(
            &request.context,
            &request.capability_id,
            BlockedResumeKind::Approval {
                approval_request_id: request.approval_request_id,
            },
        )
        .await?;

        let invocation_fingerprint = invocation_fingerprint_for_kind(
            CapabilityActionKind::Spawn,
            &scope,
            &request.capability_id,
            &request.estimate,
            &request.input,
        )
        .map_err(|source| CapabilityInvocationError::InvocationFingerprint {
            capability: request.capability_id.clone(),
            source,
        })?;

        let run_record = run_state
            .get(&scope, invocation_id)
            .await?
            .ok_or(RunStateError::UnknownInvocation { invocation_id })?;
        if run_record.authenticated_actor_user_id != request.context.authenticated_actor_user_id {
            return Err(CapabilityInvocationError::AuthorizationDenied {
                capability: request.capability_id,
                reason: DenyReason::PolicyDenied,
                detail: None,
            });
        }
        if run_record.status != RunStatus::BlockedApproval {
            return Err(CapabilityInvocationError::ResumeNotBlocked {
                capability: request.capability_id,
                status: run_record.status,
            });
        }
        let capability_mismatch = run_record.capability_id != request.capability_id;
        let approval_request_mismatch =
            run_record.approval_request_id != Some(request.approval_request_id);
        if capability_mismatch || approval_request_mismatch {
            fail_run_if_configured(
                Some(run_state),
                &scope,
                invocation_id,
                "ResumeContextMismatch",
            )
            .await;
            return Err(CapabilityInvocationError::ResumeContextMismatch {
                capability: request.capability_id,
                kind: resume_context_mismatch_kind(capability_mismatch, approval_request_mismatch),
            });
        }

        let approval = approval_requests
            .get(&scope, request.approval_request_id)
            .await?
            .ok_or(RunStateError::UnknownApprovalRequest {
                request_id: request.approval_request_id,
            })?;
        if approval.status != ApprovalStatus::Approved {
            if approval.status != ApprovalStatus::Pending {
                fail_run_if_configured(
                    Some(run_state),
                    &scope,
                    invocation_id,
                    approval_not_approved_error_kind(approval.status),
                )
                .await;
            }
            return Err(CapabilityInvocationError::ApprovalNotApproved {
                capability: request.capability_id,
                status: approval.status,
            });
        }
        if let Err(error) = validate_approval_request_matches_invocation(
            &approval.request,
            &request.context,
            &request.capability_id,
            &request.estimate,
            CapabilityActionKind::Spawn,
        ) {
            fail_run_if_configured(
                Some(run_state),
                &scope,
                invocation_id,
                "ApprovalRequestMismatch",
            )
            .await;
            return Err(error);
        }
        if approval.request.invocation_fingerprint.as_ref() != Some(&invocation_fingerprint) {
            fail_run_if_configured(
                Some(run_state),
                &scope,
                invocation_id,
                "InvocationFingerprintMismatch",
            )
            .await;
            return Err(CapabilityInvocationError::ApprovalFingerprintMismatch {
                capability: request.capability_id,
            });
        }

        let Some(descriptor) = self.registry.get_capability(&request.capability_id) else {
            fail_run_if_configured(Some(run_state), &scope, invocation_id, "UnknownCapability")
                .await;
            return Err(CapabilityInvocationError::UnknownCapability {
                capability: request.capability_id,
            });
        };

        let Some(lease) = matching_approval_lease(
            capability_leases,
            &request.context,
            &request.capability_id,
            &invocation_fingerprint,
        )
        .await
        else {
            fail_run_if_configured(
                Some(run_state),
                &scope,
                invocation_id,
                "ApprovalLeaseMissing",
            )
            .await;
            return Err(CapabilityInvocationError::ApprovalLeaseMissing {
                capability: request.capability_id,
            });
        };
        let mut authorized_context = request.context.clone();
        authorized_context.grants.grants.push(lease.grant.clone());

        // Kernel-computed trust on the spawn-resume path (§5.3.2/§9). Runtime-policy
        // planning already ran in `resume_preflight` above (fail-closed before the
        // lease was claimed), so it is not repeated here.
        let trust_decision = match self.evaluate_trust(&capability_id) {
            Ok(d) => d,
            Err(error) => {
                fail_run_if_configured(
                    Some(run_state),
                    &scope,
                    invocation_id,
                    "AuthorizationDenied",
                )
                .await;
                return Err(error);
            }
        };
        authorized_context.trust = trust_decision.effective_trust.class();

        let obligations = match self
            .authorizer
            .authorize_spawn_with_trust(
                &authorized_context,
                descriptor,
                &request.estimate,
                &trust_decision,
            )
            .await
        {
            Decision::Allow {
                obligations: allowed_obligations,
            } => allowed_obligations.into_vec(),
            Decision::Deny { reason } => {
                fail_run_if_configured(
                    Some(run_state),
                    &scope,
                    invocation_id,
                    "AuthorizationDenied",
                )
                .await;
                return Err(CapabilityInvocationError::AuthorizationDenied {
                    capability: request.capability_id,
                    reason,
                    detail: None,
                });
            }
            Decision::RequireApproval { .. } => {
                fail_run_if_configured(
                    Some(run_state),
                    &scope,
                    invocation_id,
                    "AuthorizationRequiresApproval",
                )
                .await;
                return Err(CapabilityInvocationError::AuthorizationRequiresApproval {
                    capability: request.capability_id,
                });
            }
        };

        let claimed_lease = match capability_leases
            .claim(&scope, lease.grant.id, &invocation_fingerprint)
            .await
        {
            Ok(lease) => lease,
            Err(error) => {
                if claim_error_may_be_concurrent_resume(&error) {
                    warn!(
                        lease_id = %lease.grant.id,
                        invocation_id = %invocation_id,
                        capability_id = %capability_id,
                        error_kind = capability_lease_error_kind(&error),
                        "spawn approval lease claim lost to a concurrent resume; leaving run state unchanged",
                    );
                } else {
                    fail_run_if_configured(
                        Some(run_state),
                        &scope,
                        invocation_id,
                        "ApprovalLeaseClaim",
                    )
                    .await;
                }
                return Err(CapabilityInvocationError::Lease(Box::new(error)));
            }
        };

        let obligation_outcome = match self
            .prepare_obligations(
                CapabilityObligationPhase::Spawn,
                &authorized_context,
                &request.capability_id,
                &request.estimate,
                obligations.clone(),
            )
            .await
        {
            Ok(outcome) => outcome,
            Err(error) => {
                apply_run_state_transition_if_configured(
                    Some(run_state),
                    &scope,
                    invocation_id,
                    &error,
                )
                .await;
                if let Err(revoke_error) = capability_leases
                    .revoke(&scope, claimed_lease.grant.id)
                    .await
                {
                    warn!(
                        lease_id = %claimed_lease.grant.id,
                        invocation_id = %invocation_id,
                        capability_id = %capability_id,
                        obligation_error = %error,
                        revoke_error_kind = capability_lease_error_kind(&revoke_error),
                        "capability lease revoke failed after spawn obligation failure; lease may remain claimed",
                    );
                }
                return Err(error);
            }
        };
        let effective_mounts = obligation_outcome
            .mounts
            .clone()
            .unwrap_or_else(|| authorized_context.mounts.clone());
        let resource_reservation_id = obligation_outcome
            .resource_reservation
            .as_ref()
            .map(|reservation| reservation.id);
        let process_id = ProcessId::new();
        let result = self.seal_authorization(
            &authorized_context,
            &request.capability_id,
            &request.estimate,
            &request.input,
            descriptor,
            &obligation_outcome,
            claimed_lease.grant.constraints.expires_at,
        );
        let authorized_continuation = match process_authorized_continuation(
            result,
            &request.capability_id,
            descriptor.runtime,
            process_id,
        ) {
            Ok(continuation) => continuation,
            Err(error) => {
                self.abort_obligations(
                    CapabilityObligationPhase::Spawn,
                    &authorized_context,
                    &request.capability_id,
                    &request.estimate,
                    obligations.as_slice(),
                    &obligation_outcome,
                )
                .await;
                fail_run_if_configured(Some(run_state), &scope, invocation_id, "ProcessSpawn")
                    .await;
                if let Err(revoke_error) = capability_leases
                    .revoke(&scope, claimed_lease.grant.id)
                    .await
                {
                    warn!(
                        lease_id = %claimed_lease.grant.id,
                        invocation_id = %invocation_id,
                        capability_id = %capability_id,
                        revoke_error_kind = capability_lease_error_kind(&revoke_error),
                        "capability lease revoke failed after spawn authorization failure; lease may remain claimed",
                    );
                }
                return Err(error);
            }
        };

        let process = match process_manager
            .spawn(ProcessStart {
                process_id,
                parent_process_id: authorized_context.process_id,
                invocation_id,
                scope: scope.clone(),
                authenticated_actor_user_id: authorized_context.authenticated_actor_user_id.clone(),
                extension_id: descriptor.provider.clone(),
                capability_id: request.capability_id.clone(),
                runtime: descriptor.runtime,
                grants: authorized_context.grants.clone(),
                mounts: effective_mounts,
                estimated_resources: request.estimate.clone(),
                resource_reservation_id,
                authorized_continuation,
                input: request.input,
            })
            .await
        {
            Ok(process) => process,
            Err(error) => {
                self.abort_obligations(
                    CapabilityObligationPhase::Spawn,
                    &authorized_context,
                    &request.capability_id,
                    &request.estimate,
                    obligations.as_slice(),
                    &obligation_outcome,
                )
                .await;
                fail_run_if_configured(Some(run_state), &scope, invocation_id, "ProcessSpawn")
                    .await;
                let invocation_error = CapabilityInvocationError::from(error);
                if let Err(revoke_error) = capability_leases
                    .revoke(&scope, claimed_lease.grant.id)
                    .await
                {
                    warn!(
                        lease_id = %claimed_lease.grant.id,
                        invocation_id = %invocation_id,
                        capability_id = %capability_id,
                        process_error = %invocation_error,
                        revoke_error_kind = capability_lease_error_kind(&revoke_error),
                        "capability lease revoke failed after process spawn failure; lease may remain claimed",
                    );
                }
                return Err(invocation_error);
            }
        };

        if let Err(error) = capability_leases
            .consume(&scope, claimed_lease.grant.id)
            .await
        {
            warn!(
                lease_id = %claimed_lease.grant.id,
                invocation_id = %invocation_id,
                capability_id = %capability_id,
                error_kind = capability_lease_error_kind(&error),
                "capability lease consume failed after successful process spawn; lease left in claimed state",
            );
        }

        complete_run_after_side_effect(run_state, &scope, invocation_id, &capability_id, "spawn")
            .await;
        Ok(CapabilitySpawnResult { process })
    }

    pub async fn spawn_json(
        &self,
        request: CapabilitySpawnRequest,
    ) -> Result<CapabilitySpawnResult, CapabilityInvocationError> {
        let process_manager = self.process_manager.ok_or_else(|| {
            CapabilityInvocationError::ProcessManagerMissing {
                capability: request.capability_id.clone(),
            }
        })?;
        let invocation_id = request.context.invocation_id;
        let capability_id = request.capability_id.clone();
        let scope = request.context.resource_scope.clone();
        // The pre-spawn authority fold — context validation, fingerprint,
        // run-state start, capability lookup, trust-aware spawn authorization,
        // obligation preparation, and (Slice C) minting the sealed `Authorized`
        // witness — is one method mirroring `authorize()`. `spawn_json` maps its
        // `AuthorizeFold` back to today's exact process-spawn and error behavior.
        let (obligations, obligation_outcome, authorized_result) =
            match self.authorize_spawn(&request).await? {
                AuthorizeFold::Authorized(fold) => {
                    let AuthorizedFold {
                        result,
                        frozen_deadline: _,
                        obligations,
                        obligation_outcome,
                    } = *fold;
                    (obligations, obligation_outcome, result)
                }
                AuthorizeFold::Denied { reason, .. } => {
                    return Err(CapabilityInvocationError::AuthorizationDenied {
                        capability: request.capability_id,
                        reason,
                        detail: None,
                    });
                }
                AuthorizeFold::Blocked { .. } => {
                    return Err(CapabilityInvocationError::AuthorizationRequiresApproval {
                        capability: request.capability_id,
                    });
                }
            };

        // Re-resolve the descriptor for the process start. `authorize_spawn`
        // already proved the capability exists (failing the run otherwise) and
        // the registry is immutable for the host's lifetime, so this lookup is
        // infallible in practice; it only re-borrows the descriptor that was
        // released when the fold returned. Fail closed on the unreachable `None`.
        let Some(descriptor) = self.registry.get_capability(&request.capability_id) else {
            // Obligations were already prepared by the fold — abort them so the
            // unreachable arm cannot leak a prepared reservation/mount grant.
            self.abort_obligations(
                CapabilityObligationPhase::Spawn,
                &request.context,
                &request.capability_id,
                &request.estimate,
                obligations.as_slice(),
                &obligation_outcome,
            )
            .await;
            fail_run_if_configured(self.run_state, &scope, invocation_id, "UnknownCapability")
                .await;
            return Err(CapabilityInvocationError::UnknownCapability {
                capability: request.capability_id,
            });
        };

        let effective_mounts = obligation_outcome
            .mounts
            .clone()
            .unwrap_or_else(|| request.context.mounts.clone());
        let resource_reservation_id = obligation_outcome
            .resource_reservation
            .as_ref()
            .map(|reservation| reservation.id);
        let process_id = ProcessId::new();
        let authorized_continuation = match process_authorized_continuation(
            authorized_result,
            &request.capability_id,
            descriptor.runtime,
            process_id,
        ) {
            Ok(continuation) => continuation,
            Err(error) => {
                self.abort_obligations(
                    CapabilityObligationPhase::Spawn,
                    &request.context,
                    &request.capability_id,
                    &request.estimate,
                    obligations.as_slice(),
                    &obligation_outcome,
                )
                .await;
                fail_run_if_configured(self.run_state, &scope, invocation_id, "ProcessSpawn").await;
                return Err(error);
            }
        };

        let process = match process_manager
            .spawn(ProcessStart {
                process_id,
                parent_process_id: request.context.process_id,
                invocation_id,
                scope: scope.clone(),
                authenticated_actor_user_id: request.context.authenticated_actor_user_id.clone(),
                extension_id: descriptor.provider.clone(),
                capability_id: request.capability_id.clone(),
                runtime: descriptor.runtime,
                grants: request.context.grants.clone(),
                mounts: effective_mounts,
                estimated_resources: request.estimate.clone(),
                resource_reservation_id,
                authorized_continuation,
                input: request.input,
            })
            .await
        {
            Ok(process) => process,
            Err(error) => {
                self.abort_obligations(
                    CapabilityObligationPhase::Spawn,
                    &request.context,
                    &request.capability_id,
                    &request.estimate,
                    obligations.as_slice(),
                    &obligation_outcome,
                )
                .await;
                fail_run_if_configured(self.run_state, &scope, invocation_id, "ProcessSpawn").await;
                return Err(CapabilityInvocationError::from(error));
            }
        };

        if let Some(run_state) = self.run_state {
            complete_run_after_side_effect(
                run_state,
                &scope,
                invocation_id,
                &capability_id,
                "spawn",
            )
            .await;
        }

        Ok(CapabilitySpawnResult { process })
    }

    /// The pre-spawn authority fold for `spawn_json`, extracted per
    /// arch-simplification §9 step 2 / §5.3.2 exactly as [`Self::authorize`] does
    /// for invoke: validate the context, fingerprint the spawn, start the run
    /// record, resolve the descriptor, run trust-aware spawn authorization, and
    /// on `Allow` prepare obligations and mint the sealed [`Authorized`] witness.
    /// Every side effect the inline fold performed — run-state
    /// `start`/`fail`/`block`, approval persist-and-rollback, obligation
    /// `prepare`, and each early error return — stays here verbatim; `spawn_json`
    /// only maps the returned [`AuthorizeFold`] back to today's outcome.
    async fn authorize_spawn(
        &self,
        request: &CapabilitySpawnRequest,
    ) -> Result<AuthorizeFold, CapabilityInvocationError> {
        let invocation_id = request.context.invocation_id;
        let scope = request.context.resource_scope.clone();
        if request.context.validate().is_err() {
            return Err(CapabilityInvocationError::AuthorizationDenied {
                capability: request.capability_id.clone(),
                reason: DenyReason::InternalInvariantViolation,
                detail: None,
            });
        }

        let invocation_fingerprint = invocation_fingerprint_for_kind(
            CapabilityActionKind::Spawn,
            &scope,
            &request.capability_id,
            &request.estimate,
            &request.input,
        )
        .map_err(|source| CapabilityInvocationError::InvocationFingerprint {
            capability: request.capability_id.clone(),
            source,
        })?;

        // Resolve the descriptor BEFORE starting a run record (see `authorize`):
        // an unknown capability short-circuits without creating a run record, so
        // no `fail_run_if_configured` is needed here.
        let Some(descriptor) = self.registry.get_capability(&request.capability_id) else {
            return Err(CapabilityInvocationError::UnknownCapability {
                capability: request.capability_id.clone(),
            });
        };

        if let Some(run_state) = self.run_state {
            run_state
                .start(RunStart {
                    invocation_id,
                    capability_id: request.capability_id.clone(),
                    scope: scope.clone(),
                    authenticated_actor_user_id: request
                        .context
                        .authenticated_actor_user_id
                        .clone(),
                })
                .await?;
        }

        // Kernel-computed trust + in-fold runtime-policy planning (§5.3.2/§9),
        // mirroring `authorize()` on the spawn path.
        let trust_decision = match self.evaluate_trust(&request.capability_id) {
            Ok(d) => d,
            Err(error) => {
                apply_run_state_transition_if_configured(
                    self.run_state,
                    &scope,
                    invocation_id,
                    &error,
                )
                .await;
                return Err(error);
            }
        };
        if let Err(error) = self.enforce_runtime_policy(descriptor) {
            apply_run_state_transition_if_configured(self.run_state, &scope, invocation_id, &error)
                .await;
            return Err(error);
        }

        // Credential pre-flight on the spawn path, mirroring `authorize()`
        // (§5.3.2/§9): a missing credential surfaces as `AuthorizationRequiresAuth`
        // before the spawn-approval decision. Facts only; `Indeterminate` skips.
        match self
            .policy_facts
            .credential_presence(&request.capability_id, &scope)
            .await
        {
            CredentialPresence::Satisfied | CredentialPresence::Indeterminate => {}
            CredentialPresence::Missing {
                required_secrets,
                requirements,
            } => {
                let error = CapabilityInvocationError::AuthorizationRequiresAuth {
                    capability: request.capability_id.clone(),
                    required_secrets,
                    credential_requirements: requirements,
                };
                apply_run_state_transition_if_configured(
                    self.run_state,
                    &scope,
                    invocation_id,
                    &error,
                )
                .await;
                return Err(error);
            }
        }

        let mut authorize_context = request.context.clone();
        authorize_context.trust = trust_decision.effective_trust.class();

        let frozen_deadline = self
            .apply_persistent_approval(
                &mut authorize_context,
                descriptor,
                &request.capability_id,
                &request.estimate,
                &trust_decision,
                PolicyAction::SpawnCapability,
            )
            .await;

        match self
            .authorizer
            .authorize_spawn_with_trust(
                &authorize_context,
                descriptor,
                &request.estimate,
                &trust_decision,
            )
            .await
        {
            Decision::Allow {
                obligations: allowed_obligations,
            } => {
                let allowed_obligations = allowed_obligations.into_vec();
                let obligation_outcome = match self
                    .prepare_obligations(
                        CapabilityObligationPhase::Spawn,
                        &authorize_context,
                        &request.capability_id,
                        &request.estimate,
                        allowed_obligations.clone(),
                    )
                    .await
                {
                    Ok(outcome) => outcome,
                    Err(error) => {
                        apply_run_state_transition_if_configured(
                            self.run_state,
                            &scope,
                            invocation_id,
                            &error,
                        )
                        .await;
                        return Err(error);
                    }
                };
                let result = self.seal_authorization(
                    &authorize_context,
                    &request.capability_id,
                    &request.estimate,
                    &request.input,
                    descriptor,
                    &obligation_outcome,
                    frozen_deadline,
                );
                Ok(AuthorizeFold::Authorized(Box::new(AuthorizedFold {
                    result,
                    frozen_deadline: None,
                    obligations: allowed_obligations,
                    obligation_outcome,
                })))
            }
            Decision::Deny { reason } => {
                fail_run_if_configured(
                    self.run_state,
                    &scope,
                    invocation_id,
                    "AuthorizationDenied",
                )
                .await;
                Ok(AuthorizeFold::Denied {
                    result: AuthorizeResult::Denied(DenyRef::new()),
                    reason,
                })
            }
            Decision::RequireApproval {
                request: mut approval,
            } => {
                let approval_request_id = approval.id;
                add_capability_input_display_hint(
                    &mut approval.reason,
                    &request.capability_id,
                    &request.input,
                );
                if let Err(error) = validate_approval_request_matches_invocation(
                    &approval,
                    &request.context,
                    &request.capability_id,
                    &request.estimate,
                    CapabilityActionKind::Spawn,
                ) {
                    fail_run_if_configured(
                        self.run_state,
                        &scope,
                        invocation_id,
                        "ApprovalRequestMismatch",
                    )
                    .await;
                    return Err(error);
                }

                if let Some(existing) = &approval.invocation_fingerprint {
                    if existing != &invocation_fingerprint {
                        fail_run_if_configured(
                            self.run_state,
                            &scope,
                            invocation_id,
                            "InvocationFingerprintMismatch",
                        )
                        .await;
                        return Err(CapabilityInvocationError::ApprovalFingerprintMismatch {
                            capability: request.capability_id.clone(),
                        });
                    }
                } else {
                    approval.invocation_fingerprint = Some(invocation_fingerprint);
                }

                match (self.run_state, self.approval_requests) {
                    (Some(run_state), Some(approval_requests)) => {
                        if let Some(combined_store) = self.run_state_approval_store {
                            if let Err(error) = combined_store
                                .save_pending_and_block_approval(
                                    scope.clone(),
                                    invocation_id,
                                    approval,
                                )
                                .await
                            {
                                fail_run_if_configured(
                                    Some(run_state),
                                    &scope,
                                    invocation_id,
                                    "ApprovalBlock",
                                )
                                .await;
                                return Err(CapabilityInvocationError::from(error));
                            }
                        } else {
                            let approval_id = approval.id;
                            if let Err(error) = approval_requests
                                .save_pending(scope.clone(), approval.clone())
                                .await
                            {
                                fail_run_if_configured(
                                    Some(run_state),
                                    &scope,
                                    invocation_id,
                                    "ApprovalStore",
                                )
                                .await;
                                return Err(CapabilityInvocationError::from(error));
                            }
                            if let Err(error) = run_state
                                .block_approval(&scope, invocation_id, approval)
                                .await
                            {
                                if let Err(discard_error) =
                                    approval_requests.discard_pending(&scope, approval_id).await
                                {
                                    warn!(
                                        approval_request_id = %approval_id,
                                        invocation_id = %invocation_id,
                                        transition_error_kind = run_state_error_kind(&discard_error),
                                        "approval rollback failed after spawn run-state block transition failed",
                                    );
                                }
                                fail_run_if_configured(
                                    Some(run_state),
                                    &scope,
                                    invocation_id,
                                    "ApprovalBlock",
                                )
                                .await;
                                return Err(CapabilityInvocationError::from(error));
                            }
                        }
                    }
                    (Some(run_state), None) => {
                        fail_run_if_configured(
                            Some(run_state),
                            &scope,
                            invocation_id,
                            "ApprovalStoreMissing",
                        )
                        .await;
                        return Err(CapabilityInvocationError::ApprovalStoreMissing {
                            capability: request.capability_id.clone(),
                            store: "approval_requests",
                        });
                    }
                    (None, Some(_)) => {
                        return Err(CapabilityInvocationError::ApprovalStoreMissing {
                            capability: request.capability_id.clone(),
                            store: "run_state",
                        });
                    }
                    (None, None) => {
                        return Err(CapabilityInvocationError::ApprovalStoreMissing {
                            capability: request.capability_id.clone(),
                            store: "run_state and approval_requests",
                        });
                    }
                }
                Ok(AuthorizeFold::Blocked {
                    result: AuthorizeResult::Blocked(Blocked::Approval(GateWaypoint::new(
                        GateRef::for_approval_request(approval_request_id),
                    ))),
                })
            }
        }
    }

    /// Resume-path pre-authorization, relocated from host_runtime's deleted
    /// `open_pre_authorization` + `fail_matching_blocked_{,auth_}resume_on_preflight_error`
    /// (§5.3.2/§9, R-A). Resolves the descriptor and enforces runtime-policy
    /// planning on the resumed capability BEFORE the fold's run-state lookup, so an
    /// unknown capability short-circuits to `UnknownCapability` (→ `MissingRuntime`)
    /// instead of the run-state-not-found `Backend` path, and a runtime policy
    /// tightened between invoke and resume fails closed (reversing #6386's
    /// "planning is NOT re-run on resume"). On refusal it fails ONLY the matching
    /// blocked run — via [`Self::fail_matching_blocked_resume_run`] — recording the
    /// planner-specific INTERNAL `error_kind`, then returns the sanitized error (the
    /// model-visible message stays sanitized through `DenyReason`; the planner
    /// detail rides only the run-state audit record). Trust is still classified
    /// downstream (in `authorize_resumed` / the spawn-resume fold), which stamps
    /// `context.trust` before the authorizer.
    async fn resume_preflight(
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
        let Some(run_state) = self.run_state else {
            return;
        };
        let scope = &context.resource_scope;
        let invocation_id = context.invocation_id;
        let record = match run_state.get(scope, invocation_id).await {
            Ok(Some(record)) => record,
            Ok(None) => return,
            Err(error) => {
                warn!(
                    invocation_id = %invocation_id,
                    capability_id = %capability_id,
                    preflight_error_kind = error_kind,
                    lookup_error_kind = run_state_error_kind(&error),
                    "resume preflight failed, but run-state lookup failed; leaving run state unchanged",
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
                    record.status == RunStatus::BlockedApproval
                        && record.approval_request_id == Some(approval_request_id)
                }
                BlockedResumeKind::Auth => record.status == RunStatus::BlockedAuth,
            };
        if matches {
            fail_run_if_configured(Some(run_state), scope, invocation_id, error_kind).await;
        }
    }

    /// Pre-dispatch authority fold shared by `resume_json` and
    /// `auth_resume_json`, extracted per arch-simplification §9 step 2 / §5.3.2
    /// exactly as [`Self::authorize`] does for invoke: run trust-aware
    /// authorization and map the `Decision`. On `Deny`/`RequireApproval` every
    /// side effect the inline fold performed stays here verbatim — the run-state
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
    async fn authorize_resumed(
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
                fail_run_if_configured(
                    Some(params.run_state),
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
        // persistent grant (e.g. `extension_activate` under admin-config trust)
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
                fail_run_if_configured(
                    Some(params.run_state),
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
                fail_run_if_configured(
                    Some(params.run_state),
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
    /// lease consume → `complete_run_after_side_effect` → Ok.
    ///
    /// On any failure: aborts applicable obligations, transitions run state,
    /// and revokes the claimed lease unless the error is a non-terminal
    /// `BlockAuth` transition (in which case the lease stays Claimed so a
    /// subsequent `auth_resume_json` can reuse it without a second approval).
    async fn dispatch_resumed_capability(
        &self,
        params: ResumedDispatchParams<'_>,
    ) -> Result<CapabilityInvocationResult, CapabilityInvocationError> {
        // Pre-dispatch authority fold (trust-aware authorization + Decision
        // mapping) extracted to `authorize_resumed`, mirroring `authorize()`.
        // The claim-before-dispatch ordering the resume paths depend on stays in
        // this tail: the approval lease claim and the authoritative obligation
        // preparation run BELOW, after the fold returns `Authorized`, so a `Deny`
        // still leaves a `PendingClaim` lease `Active` and never a second
        // authorization runs.
        let fold = self.authorize_resumed(&params).await?;

        let ResumedDispatchParams {
            run_state,
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
        let claimed_lease: Option<(&dyn CapabilityLeaseStore, CapabilityLease)> = match lease_state
        {
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
                                "approval lease claim lost to a concurrent resume; leaving run state unchanged",
                            );
                        } else {
                            fail_run_if_configured(
                                Some(run_state),
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
                apply_run_state_transition_if_configured(
                    Some(run_state),
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
        );
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
                apply_run_state_transition_if_configured(
                    Some(run_state),
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
                apply_run_state_transition_if_configured(
                    Some(run_state),
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
                fail_run_if_configured(
                    Some(run_state),
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

        complete_run_after_side_effect(
            run_state,
            &scope,
            invocation_id,
            &capability_id,
            "dispatch",
        )
        .await;
        Ok(CapabilityInvocationResult { dispatch })
    }

    async fn prepare_obligations(
        &self,
        phase: CapabilityObligationPhase,
        context: &ExecutionContext,
        capability_id: &ironclaw_host_api::CapabilityId,
        estimate: &ResourceEstimate,
        obligations: Vec<Obligation>,
    ) -> Result<CapabilityObligationOutcome, CapabilityInvocationError> {
        if obligations.is_empty() {
            return Ok(CapabilityObligationOutcome::default());
        }
        if matches!(phase, CapabilityObligationPhase::Spawn) {
            let unsupported = post_dispatch_obligations(&obligations);
            if !unsupported.is_empty() {
                return Err(CapabilityInvocationError::UnsupportedObligations {
                    capability: capability_id.clone(),
                    obligations: unsupported,
                });
            }
        }
        let Some(handler) = self.obligation_handler else {
            return Err(CapabilityInvocationError::UnsupportedObligations {
                capability: capability_id.clone(),
                obligations,
            });
        };
        handler
            .prepare(CapabilityObligationRequest {
                phase,
                context,
                capability_id,
                estimate,
                obligations: obligations.as_slice(),
            })
            .await
            .map_err(|error| prepare_obligation_error_to_invocation(capability_id, error))
    }

    async fn complete_dispatch_obligations(
        &self,
        phase: CapabilityObligationPhase,
        context: &ExecutionContext,
        capability_id: &ironclaw_host_api::CapabilityId,
        estimate: &ResourceEstimate,
        obligations: &[Obligation],
        dispatch: &CapabilityDispatchResult,
    ) -> Result<CapabilityDispatchResult, CapabilityInvocationError> {
        if obligations.is_empty() {
            return Ok(dispatch.clone());
        }
        let Some(handler) = self.obligation_handler else {
            let unsupported = post_dispatch_obligations(obligations);
            if unsupported.is_empty() {
                return Ok(dispatch.clone());
            }
            return Err(CapabilityInvocationError::UnsupportedObligations {
                capability: capability_id.clone(),
                obligations: unsupported,
            });
        };
        handler
            .complete_dispatch(CapabilityObligationCompletionRequest {
                phase,
                context,
                capability_id,
                estimate,
                obligations,
                dispatch,
            })
            .await
            .map_err(|error| completion_obligation_error_to_invocation(capability_id, error))
    }

    async fn abort_obligations(
        &self,
        phase: CapabilityObligationPhase,
        context: &ExecutionContext,
        capability_id: &ironclaw_host_api::CapabilityId,
        estimate: &ResourceEstimate,
        obligations: &[Obligation],
        outcome: &CapabilityObligationOutcome,
    ) {
        if obligations.is_empty() {
            return;
        }
        let Some(handler) = self.obligation_handler else {
            return;
        };
        if let Err(error) = handler
            .abort(CapabilityObligationAbortRequest {
                phase,
                context,
                capability_id,
                estimate,
                obligations,
                outcome,
            })
            .await
        {
            warn!(
                capability_id = %capability_id,
                error = %error,
                "obligation abort failed after downstream side-effect failure",
            );
        }
    }
}

/// Whether a capability's manifest permission mode may be upgraded by an
/// explicit persistent ("always allow") user decision — the gate on the kernel's
/// persistent-approval fold.
///
/// Pure over [`PermissionMode`] (a `host_api` type), relocated into the kernel
/// from host_runtime so the fold does not depend on host_runtime or
/// `ironclaw_approvals`. Semantics match `ironclaw_approvals`'
/// `permission_mode_allows_persistent_approval`: `Allow` and `Ask` are eligible;
/// `Deny` is not. Modes requiring mandatory per-invocation consent must use a
/// gate that does not offer persistent approval.
/// Bounded default validity window for the sealed witness when the authorization
/// froze no shorter-lived fact. Keeps no-frozen-fact capabilities on the prior
/// fixed window; a frozen fact, when present, always shortens this.
const WITNESS_DEFAULT_TTL: chrono::Duration = chrono::Duration::minutes(5);

/// Derive the sealed witness deadline from the shortest-lived frozen fact so a
/// held witness cannot outlive the facts that justified it (§5.3.2): take the
/// earliest of the candidate expiries, falling back to [`WITNESS_DEFAULT_TTL`]
/// from now when none is present. Candidate expiries today are the adopted
/// persistent-grant expiry (invoke/spawn) and the claimed approval lease's expiry
/// (resume). Credential-lease expiry integration is future — the credential
/// presence port returns presence, not lease expiry — so it is not a candidate
/// yet; do not block on it.
fn witness_deadline<I>(candidate_expiries: I) -> Timestamp
where
    I: IntoIterator<Item = Option<Timestamp>>,
{
    candidate_expiries
        .into_iter()
        .flatten()
        .min()
        .unwrap_or_else(|| chrono::Utc::now() + WITNESS_DEFAULT_TTL)
}

fn permission_mode_allows_persistent_approval(permission: PermissionMode) -> bool {
    matches!(permission, PermissionMode::Allow | PermissionMode::Ask)
}

/// Map a kernel trust-classification failure to the model-visible invocation
/// error, preserving today's outcome kinds: the "unknown capability" case →
/// `UnknownCapability` (host `MissingRuntime`); every other variant →
/// `AuthorizationDenied` (host `Authorization`).
fn trust_error_to_invocation_error(
    capability_id: &CapabilityId,
    error: TrustEvaluationError,
) -> CapabilityInvocationError {
    debug!(
        capability_id = %capability_id,
        trust_error = error.message(),
        "kernel trust classification refused to produce a decision"
    );
    if error.is_unknown_capability() {
        CapabilityInvocationError::UnknownCapability {
            capability: capability_id.clone(),
        }
    } else {
        CapabilityInvocationError::AuthorizationDenied {
            capability: capability_id.clone(),
            reason: DenyReason::InternalInvariantViolation,
            detail: None,
        }
    }
}

/// Map an in-fold runtime-policy planner refusal to the model-visible
/// `AuthorizationDenied` (host `Authorization`), matching today's
/// `runtime_policy_failure`.
fn runtime_policy_error_to_invocation_error(
    capability_id: &CapabilityId,
    error: PlannerError,
) -> CapabilityInvocationError {
    // The verdict collapses to `PolicyDenied`, but a bare `PolicyDenied` tells the
    // model nothing about *why*. So the model-visible `detail` carries a
    // plain-language explanation of the refusal — deliberately NOT the raw
    // `PlannerError` Display, which leaks internal `ProcessBackendKind::`/
    // `NetworkMode::`/`SecretMode::` enum tokens the model must never see (see
    // `planner_error_kind`). The full enum-token message stays server-side via
    // `debug!` (never `info!`/`warn!`) for operator diagnosis.
    debug!(
        capability_id = %capability_id,
        %error,
        "runtime-policy planner refused capability dispatch (fail-closed)"
    );
    CapabilityInvocationError::AuthorizationDenied {
        capability: capability_id.clone(),
        reason: DenyReason::PolicyDenied,
        detail: Some(planner_error_model_reason(&error).to_string()),
    }
}

/// Sanitized, model-visible explanation of a runtime-policy planner refusal:
/// a plain-language reason the model can surface or explain, deliberately free
/// of the internal `ProcessBackendKind::`/`NetworkMode::`/`SecretMode::` planner
/// enum tokens (see [`planner_error_kind`] and #6386). Rides the
/// `AuthorizationDenied { detail }` field.
fn planner_error_model_reason(error: &PlannerError) -> &'static str {
    match error {
        PlannerError::ProcessEffectsRequiredButProcessBackendIsNone { .. } => {
            "this capability needs to run a process, but process execution is disabled by policy for this runtime"
        }
        PlannerError::NetworkRequiredButNetworkModeIsDeny { .. } => {
            "this capability needs network access, but network egress is disabled by policy for this runtime"
        }
        PlannerError::SecretAccessRequiredButSecretModeIsDeny { .. } => {
            "this capability needs secret access, but secret access is disabled by policy for this runtime"
        }
    }
}

/// Internal (audit-only) `error_kind` for a runtime-policy planner refusal, kept
/// distinct from the sanitized model-visible `DenyReason::PolicyDenied` that
/// `runtime_policy_error_to_invocation_error` produces. Mirrors the strings
/// host_runtime's deleted `RuntimePolicyEvaluationError::kind` recorded on the
/// blocked-run failure so the run-state audit record is unchanged (e.g.
/// `"process_backend_none"`); the planner enum name never reaches the model.
fn planner_error_kind(error: &PlannerError) -> &'static str {
    match error {
        PlannerError::ProcessEffectsRequiredButProcessBackendIsNone { .. } => {
            "process_backend_none"
        }
        PlannerError::NetworkRequiredButNetworkModeIsDeny { .. } => "network_denied",
        PlannerError::SecretAccessRequiredButSecretModeIsDeny { .. } => "secret_denied",
    }
}

fn add_capability_input_display_hint(
    reason: &mut String,
    capability_id: &CapabilityId,
    input: &serde_json::Value,
) {
    let capability_id = capability_id.as_str();
    if capability_id != "shell"
        && capability_id != "builtin.shell"
        && !capability_id.ends_with(".shell")
    {
        return;
    }
    let Some(command) = input
        .get("command")
        .and_then(serde_json::Value::as_str)
        .map(shell_command_display_text)
    else {
        return;
    };
    if command.text.is_empty() {
        return;
    }
    reason.push_str("\n\nCommand:\n");
    reason.push_str(&command.text);
    if command.truncated {
        reason.push_str("\n[truncated]");
    }
}

/// Cleans up a claimed lease after a resume-path error using best-effort
/// abort-or-revoke semantics.
///
/// - If `error` is a `BlockAuth` (non-terminal auth gate), aborts the
///   `Dispatching` lease back to `Claimed` so the next `auth_resume_json`
///   call can reuse it without a new human approval.
/// - Otherwise revokes the lease terminally.
///
/// Both operations are best-effort: failures are logged as warnings and do
/// not propagate — the caller should already be returning an error.
///
/// `revoke_context` names the failure site ("obligation failure" or
/// "dispatch failure") and is included in the revoke warn message.
async fn cleanup_claimed_lease_after_resume_error(
    capability_leases: &dyn CapabilityLeaseStore,
    scope: &ResourceScope,
    claimed_grant_id: CapabilityGrantId,
    invocation_id: InvocationId,
    capability_id: &CapabilityId,
    error: &CapabilityInvocationError,
    revoke_context: &str,
) {
    if is_block_auth_transition(error) {
        if let Err(abort_error) = capability_leases
            .abort_dispatch_claimed(scope, claimed_grant_id)
            .await
        {
            warn!(
                lease_id = %claimed_grant_id,
                invocation_id = %invocation_id,
                capability_id = %capability_id,
                abort_error_kind = capability_lease_error_kind(&abort_error),
                "capability lease abort-dispatch failed after non-terminal auth bounce; lease may remain Dispatching",
            );
        }
    } else if let Err(revoke_error) = capability_leases.revoke(scope, claimed_grant_id).await {
        warn!(
            lease_id = %claimed_grant_id,
            invocation_id = %invocation_id,
            capability_id = %capability_id,
            revoke_error_kind = capability_lease_error_kind(&revoke_error),
            "capability lease revoke failed after {revoke_context}; lease may remain claimed",
        );
    }
}

/// Returns `true` when the error will transition the run to `BlockedAuth`
/// (a non-terminal, retriable auth gate).  Used to decide whether to skip
/// the post-claim lease revoke so `auth_resume_json` can reuse the same
/// Claimed lease without requiring a new human approval.
fn is_block_auth_transition(error: &CapabilityInvocationError) -> bool {
    matches!(
        error.run_state_transition(),
        Some(CapabilityRunStateTransition::BlockAuth { .. })
    )
}

fn prepare_obligation_error_to_invocation(
    capability_id: &ironclaw_host_api::CapabilityId,
    error: CapabilityObligationError,
) -> CapabilityInvocationError {
    match error {
        CapabilityObligationError::Unsupported { obligations } => {
            CapabilityInvocationError::UnsupportedObligations {
                capability: capability_id.clone(),
                obligations,
            }
        }
        CapabilityObligationError::AuthRequired {
            credential_requirements,
        } => CapabilityInvocationError::AuthorizationRequiresAuth {
            capability: capability_id.clone(),
            required_secrets: Vec::new(),
            credential_requirements,
        },
        CapabilityObligationError::Failed { kind } => CapabilityInvocationError::ObligationFailed {
            capability: capability_id.clone(),
            kind,
        },
    }
}

fn completion_obligation_error_to_invocation(
    capability_id: &ironclaw_host_api::CapabilityId,
    error: CapabilityObligationError,
) -> CapabilityInvocationError {
    match error {
        CapabilityObligationError::AuthRequired { .. } => {
            CapabilityInvocationError::ObligationFailed {
                capability: capability_id.clone(),
                kind: CapabilityObligationFailureKind::Secret,
            }
        }
        other => prepare_obligation_error_to_invocation(capability_id, other),
    }
}

fn obligation_invocation_error_kind(error: &CapabilityInvocationError) -> &'static str {
    // `run_state_transition` returns `None` for `CapabilityInvocationError::Dispatch`
    // because PR #4236 handles those failures via the disposition policy on the
    // outcome path. The obligation call sites only see this function for
    // diagnostic logging; fall back to a stable "Dispatch" label in that case.
    error
        .run_state_transition()
        .map(CapabilityRunStateTransition::error_kind)
        .unwrap_or("Dispatch")
}

/// Synthesize the auth-gate credential requirement for a runtime `AuthRequired`
/// that carries no auth detail of its own (the WASM-style 401 case), from the
/// capability's declared credential obligation.
///
/// Fires ONLY when the runtime gave no auth signal at all — both `required_secrets`
/// and `credential_requirements` empty — AND the capability declares EXACTLY ONE
/// credential obligation. A raw-secret-handle gate (`required_secrets` populated)
/// must not be turned into a product-auth provider prompt; and with multiple
/// credential obligations the failed credential cannot be attributed, so we leave
/// the gate unmodified rather than guess the wrong provider. The downstream WebUI
/// auth surface consumes exactly one provider (manual-token card for
/// `ManualToken` setup, OAuth launch for `OAuth` setup).
///
/// FOLLOW-UP (reactive OAuth refresh on runtime 401): for an `OAuth` credential
/// this gate is the *fallback* after refresh is exhausted — proactive refresh
/// may already have been attempted inline at injection (within the 5-min expiry
/// margin) or by the background keepalive worker. A runtime 401 still slips through when the token
/// looked fresh by `expires_at` but was revoked mid-life, where one reactive
/// "refresh + retry" before surfacing the gate would recover silently. That
/// retry does not exist today (pre-existing gap, not introduced here); the gate
/// remains correct for the genuinely-revoked case. Track as a resolver/egress
/// enhancement, not a change to this enrichment.
fn enrich_dispatch_error_credential_requirements(
    error: DispatchError,
    obligations: &[Obligation],
) -> DispatchError {
    let DispatchError::AuthRequired {
        ref required_secrets,
        ref credential_requirements,
        ..
    } = error
    else {
        return error;
    };
    if !required_secrets.is_empty() || !credential_requirements.is_empty() {
        return error;
    }
    let derived: Vec<_> = obligations
        .iter()
        .filter_map(Obligation::credential_auth_requirement)
        .collect();
    let [requirement] = derived.as_slice() else {
        return error; // zero or >1 credential obligations: do not guess
    };
    let DispatchError::AuthRequired {
        capability,
        required_secrets,
        ..
    } = error
    else {
        unreachable!("matched AuthRequired above")
    };
    DispatchError::AuthRequired {
        capability,
        required_secrets,
        credential_requirements: vec![requirement.clone()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{
        CapabilityId, ExtensionId, Obligation, RuntimeCredentialAccountSetup, SecretHandle,
        VendorId,
    };

    fn auth_required_empty(cap: &str) -> DispatchError {
        DispatchError::AuthRequired {
            capability: CapabilityId::new(cap).unwrap(),
            required_secrets: Vec::new(),
            credential_requirements: Vec::new(),
        }
    }

    fn auth_required_with_secrets(cap: &str) -> DispatchError {
        DispatchError::AuthRequired {
            capability: CapabilityId::new(cap).unwrap(),
            required_secrets: vec![SecretHandle::new("raw_secret").unwrap()],
            credential_requirements: Vec::new(),
        }
    }

    fn auth_required_with_provider(cap: &str, provider: &str) -> DispatchError {
        use ironclaw_host_api::RuntimeCredentialAuthRequirement;
        DispatchError::AuthRequired {
            capability: CapabilityId::new(cap).unwrap(),
            required_secrets: Vec::new(),
            credential_requirements: vec![RuntimeCredentialAuthRequirement {
                provider: VendorId::new(provider).unwrap(),
                setup: RuntimeCredentialAccountSetup::ManualToken,
                requester_extension: ExtensionId::new(provider).unwrap(),
                provider_scopes: Vec::new(),
            }],
        }
    }

    fn inject_credential_obligation(provider: &str) -> Obligation {
        Obligation::InjectCredentialAccountOnce {
            handle: SecretHandle::new(format!("{provider}_pat")).unwrap(),
            provider: VendorId::new(provider).unwrap(),
            setup: RuntimeCredentialAccountSetup::ManualToken,
            provider_scopes: Vec::new(),
            requester_extension: ExtensionId::new(provider).unwrap(),
        }
    }

    // WASM case: both empty + exactly one obligation → enriched with that provider.
    #[test]
    fn enrich_fills_empty_from_single_credential_obligation() {
        let error = auth_required_empty("echo.say");
        let obligations = [inject_credential_obligation("github")];

        let result = enrich_dispatch_error_credential_requirements(error, &obligations);

        let DispatchError::AuthRequired {
            credential_requirements,
            ..
        } = result
        else {
            panic!("expected AuthRequired");
        };
        assert_eq!(credential_requirements.len(), 1);
        assert_eq!(
            credential_requirements[0].provider,
            VendorId::new("github").unwrap()
        );
    }

    // required_secrets populated → returned unchanged (raw-secret gate must not become product-auth prompt).
    #[test]
    fn enrich_leaves_required_secrets_populated_unchanged() {
        let error = auth_required_with_secrets("echo.say");
        let obligations = [inject_credential_obligation("github")];

        let result = enrich_dispatch_error_credential_requirements(error, &obligations);

        let DispatchError::AuthRequired {
            required_secrets,
            credential_requirements,
            ..
        } = result
        else {
            panic!("expected AuthRequired");
        };
        assert_eq!(
            required_secrets.len(),
            1,
            "required_secrets must be preserved"
        );
        assert!(
            credential_requirements.is_empty(),
            "credential_requirements must remain empty when required_secrets are present"
        );
    }

    // credential_requirements already populated → returned unchanged (e.g. MCP runtime already supplied requirements).
    #[test]
    fn enrich_leaves_non_empty_credential_requirements_unchanged() {
        let error = auth_required_with_provider("echo.say", "mcp_provider");
        let obligations = [inject_credential_obligation("github")];

        let result = enrich_dispatch_error_credential_requirements(error, &obligations);

        let DispatchError::AuthRequired {
            credential_requirements,
            ..
        } = result
        else {
            panic!("expected AuthRequired");
        };
        assert_eq!(credential_requirements.len(), 1);
        assert_eq!(
            credential_requirements[0].provider,
            VendorId::new("mcp_provider").unwrap(),
            "original mcp_provider must be retained, not replaced by github"
        );
    }

    // ZERO credential obligations → unchanged (empty result, not a guess).
    #[test]
    fn enrich_leaves_unchanged_when_zero_credential_obligations() {
        let error = auth_required_empty("echo.say");
        let obligations: [Obligation; 0] = [];

        let result = enrich_dispatch_error_credential_requirements(error, &obligations);

        let DispatchError::AuthRequired {
            credential_requirements,
            ..
        } = result
        else {
            panic!("expected AuthRequired");
        };
        assert!(
            credential_requirements.is_empty(),
            "zero obligations must leave credential_requirements empty"
        );
    }

    // TWO credential obligations → NOT enriched (cannot attribute failure to one provider).
    #[test]
    fn enrich_leaves_unchanged_when_two_credential_obligations() {
        let error = auth_required_empty("echo.say");
        let obligations = [
            inject_credential_obligation("github"),
            inject_credential_obligation("gitlab"),
        ];

        let result = enrich_dispatch_error_credential_requirements(error, &obligations);

        let DispatchError::AuthRequired {
            credential_requirements,
            ..
        } = result
        else {
            panic!("expected AuthRequired");
        };
        assert!(
            credential_requirements.is_empty(),
            "two obligations must leave credential_requirements empty — cannot attribute which provider failed"
        );
    }

    // Non-AuthRequired variants returned unchanged.
    #[test]
    fn enrich_is_noop_for_non_auth_required_variants() {
        let error = DispatchError::UnknownCapability {
            capability: CapabilityId::new("echo.say").unwrap(),
        };
        let obligations = [inject_credential_obligation("github")];

        let result = enrich_dispatch_error_credential_requirements(error, &obligations);

        assert!(
            matches!(result, DispatchError::UnknownCapability { .. }),
            "non-AuthRequired variants must be returned unchanged"
        );
    }

    // --- Slice-C `authorize()` fold ---

    // Unconditionally allows with no obligations, so the fold reaches the seal.
    struct AllowAuthorizer;

    #[async_trait::async_trait]
    impl ironclaw_authorization::TrustAwareCapabilityDispatchAuthorizer for AllowAuthorizer {
        async fn authorize_dispatch_with_trust(
            &self,
            _context: &ExecutionContext,
            _descriptor: &CapabilityDescriptor,
            _estimate: &ResourceEstimate,
            _trust_decision: &TrustDecision,
        ) -> Decision {
            Decision::Allow {
                obligations: ironclaw_host_api::Obligations::empty(),
            }
        }
    }

    // Permissive policy-facts double: credential pre-flight always satisfied and
    // no persistent grants, so the in-fold credential check never fires.
    struct SatisfiedPolicyFacts;

    #[async_trait::async_trait]
    impl HostPolicyFacts for SatisfiedPolicyFacts {
        async fn credential_presence(
            &self,
            _capability_id: &CapabilityId,
            _scope: &ResourceScope,
        ) -> CredentialPresence {
            CredentialPresence::Satisfied
        }

        async fn persistent_grants(
            &self,
            _capability_id: &CapabilityId,
            _context: &ExecutionContext,
            _action: crate::ports::PolicyAction,
        ) -> Vec<ironclaw_host_api::CapabilityGrant> {
            Vec::new()
        }
    }

    // Returns a single persistent grant carrying `expiry`. With `AllowAuthorizer`
    // the persistent-approval probe adopts it, so its `expires_at` becomes the
    // witness's shortest-lived frozen fact.
    struct GrantWithExpiryPolicyFacts {
        expiry: Timestamp,
    }

    #[async_trait::async_trait]
    impl HostPolicyFacts for GrantWithExpiryPolicyFacts {
        async fn credential_presence(
            &self,
            _capability_id: &CapabilityId,
            _scope: &ResourceScope,
        ) -> CredentialPresence {
            CredentialPresence::Satisfied
        }

        async fn persistent_grants(
            &self,
            capability_id: &CapabilityId,
            context: &ExecutionContext,
            _action: crate::ports::PolicyAction,
        ) -> Vec<ironclaw_host_api::CapabilityGrant> {
            use ironclaw_host_api::{
                CapabilityGrant, CapabilityGrantId, GrantConstraints, MountView, NetworkPolicy,
                Principal,
            };
            vec![CapabilityGrant {
                id: CapabilityGrantId::new(),
                capability: capability_id.clone(),
                grantee: Principal::User(context.resource_scope.user_id.clone()),
                issued_by: Principal::HostRuntime,
                constraints: GrantConstraints {
                    allowed_effects: Vec::new(),
                    mounts: MountView::default(),
                    network: NetworkPolicy::default(),
                    secrets: Vec::new(),
                    resource_ceiling: None,
                    expires_at: Some(self.expiry),
                    max_invocations: None,
                },
            }]
        }
    }

    // `authorize()` never dispatches; this satisfies the `CapabilityHost` type
    // parameter without pulling in the integration-tier recording dispatcher.
    const ECHO_MANIFEST_FIXTURE: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "echo"
name = "Echo"
version = "0.1.0"
description = "Echo test extension"
trust = "third_party"

[runtime]
kind = "wasm"
module = "echo.wasm"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "echo.say"
description = "Echoes input"
effects = ["dispatch_capability"]
default_permission = "allow"
visibility = "host_internal"
input_schema_ref = "schemas/echo/say.input.v1.json"
output_schema_ref = "schemas/echo/say.output.v1.json"
"#;

    fn echo_registry() -> ExtensionRegistry {
        use ironclaw_extensions::{
            CapabilityProviderHostApiContract, ExtensionManifest, ExtensionPackage,
            HostApiContractRegistry, ManifestSource,
        };
        use ironclaw_host_api::{HostPortCatalog, VirtualPath};
        let mut contracts = HostApiContractRegistry::new();
        contracts
            .register(std::sync::Arc::new(
                CapabilityProviderHostApiContract::new().expect("capability provider contract"),
            ))
            .expect("register capability provider contract");
        let manifest = ExtensionManifest::parse(
            ECHO_MANIFEST_FIXTURE,
            ManifestSource::InstalledLocal,
            &HostPortCatalog::empty(),
            &contracts,
        )
        .unwrap();
        let package = ExtensionPackage::from_manifest(
            manifest,
            VirtualPath::new("/system/extensions/echo").unwrap(),
        )
        .unwrap();
        let mut registry = ExtensionRegistry::new();
        registry.insert(package).unwrap();
        registry
    }

    fn allow_request() -> CapabilityInvocationRequest {
        use ironclaw_host_api::{CapabilitySet, MountView, RuntimeKind, TrustClass, UserId};
        let mut context = ExecutionContext::local_default(
            UserId::new("user").unwrap(),
            ExtensionId::new("caller").unwrap(),
            RuntimeKind::Wasm,
            TrustClass::UserTrusted,
            CapabilitySet::default(),
            MountView::default(),
        )
        .unwrap();
        // A membrane-sealed actor and a real ingress origin are what make the
        // invocation seal-able. This models a direct product-surface action.
        context.authenticated_actor_user_id = Some(UserId::new("actor").unwrap());
        context.origin = Some(ironclaw_host_api::InvocationOrigin::Product(
            ironclaw_host_api::ProductKind::new("settings").unwrap(),
        ));
        CapabilityInvocationRequest {
            context,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            estimate: ResourceEstimate::default(),
            input: serde_json::json!({"message": "hi"}),
        }
    }

    /// Trust policy double for the in-fold `evaluate_trust` (§5.3.2/§9): always
    /// classifies the echo package as `user_trusted` so the kernel trust-eval
    /// succeeds and the `AllowAuthorizer` reaches the seal.
    struct StaticTrustPolicy;

    impl TrustPolicy for StaticTrustPolicy {
        fn evaluate(
            &self,
            _input: &ironclaw_trust::TrustPolicyInput,
        ) -> Result<TrustDecision, ironclaw_trust::TrustError> {
            use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustProvenance};
            Ok(TrustDecision {
                effective_trust: EffectiveTrustClass::user_trusted(),
                authority_ceiling: AuthorityCeiling {
                    allowed_effects: Vec::new(),
                    max_resource_ceiling: None,
                },
                provenance: TrustProvenance::Default,
                evaluated_at: chrono::Utc::now(),
            })
        }
    }

    /// Permissive runtime policy so the in-fold planner never denies the echo
    /// capability (echo declares only `dispatch_capability`, so no backend
    /// constraint is even exercised).
    fn permissive_runtime_policy() -> EffectiveRuntimePolicy {
        use ironclaw_host_api::runtime_policy::{
            ApprovalPolicy, AuditMode, DeploymentMode, FilesystemBackendKind, NetworkMode,
            ProcessBackendKind, RuntimeProfile, SecretMode,
        };
        EffectiveRuntimePolicy {
            deployment: DeploymentMode::LocalSingleUser,
            requested_profile: RuntimeProfile::LocalDev,
            resolved_profile: RuntimeProfile::LocalDev,
            filesystem_backend: FilesystemBackendKind::HostWorkspace,
            process_backend: ProcessBackendKind::LocalHost,
            network_mode: NetworkMode::DirectLogged,
            secret_mode: SecretMode::ScrubbedEnv,
            approval_policy: ApprovalPolicy::AskDestructive,
            audit_mode: AuditMode::LocalMinimal,
        }
    }

    // The Allow decision seals an `Authorized` whose lane is resolved from the
    // descriptor (echo is a WASM extension) and whose invocation carries the
    // exact capability/actor/input the request named. Echo declares no resource
    // obligation and no persistent grant is adopted, so the witness carries no
    // reservation (`None`, never a synthesized placeholder) and its deadline is
    // the bounded default TTL (§5.3.2).
    #[tokio::test]
    async fn authorize_allow_path_seals_authorized_with_lane_and_invocation() {
        use ironclaw_host_api::UserId;

        let registry = echo_registry();
        // Never dispatched on this authorize-only path; errors if it ever is.
        let dispatcher =
            ironclaw_host_api::dispatch_test_support::TestDispatcher::responding(|req, _| {
                Err(DispatchError::UnknownCapability {
                    capability: req.invocation.capability.clone(),
                })
            });
        let authorizer = AllowAuthorizer;
        let trust_policy = StaticTrustPolicy;
        let runtime_policy = permissive_runtime_policy();
        let policy_facts = SatisfiedPolicyFacts;
        let host = CapabilityHost::new(
            &registry,
            &dispatcher,
            &authorizer,
            &trust_policy,
            &runtime_policy,
            &policy_facts,
        );

        let request = allow_request();
        let before = chrono::Utc::now();
        let fold = host.authorize(&request).await.unwrap();
        let after = chrono::Utc::now();

        let AuthorizeFold::Authorized(fold) = fold else {
            panic!("expected an allowed authorization");
        };
        let Some(AuthorizeResult::Authorized(authorized)) = &fold.result else {
            panic!("allow path with a sealed actor must mint an Authorized witness");
        };
        assert_eq!(authorized.lane(), RuntimeLane::Wasm);
        let invocation = authorized.invocation();
        assert_eq!(
            invocation.capability,
            CapabilityId::new("echo.say").unwrap()
        );
        assert_eq!(
            invocation.actor,
            Actor::Sealed(UserId::new("actor").unwrap())
        );
        assert_eq!(
            invocation.origin,
            ironclaw_host_api::InvocationOrigin::Product(
                ironclaw_host_api::ProductKind::new("settings").unwrap()
            )
        );
        assert_eq!(invocation.input, serde_json::json!({"message": "hi"}));
        // No resource obligation → no reservation on the witness.
        assert!(
            authorized.reservation().is_none(),
            "echo declares no resource obligation; the witness must carry no reservation"
        );
        // No frozen fact → the bounded default TTL from authorize-time.
        assert!(authorized.deadline() >= before + WITNESS_DEFAULT_TTL);
        assert!(authorized.deadline() <= after + WITNESS_DEFAULT_TTL);
    }

    // When a persistent grant carrying an `expires_at` is adopted in the fold, the
    // witness deadline is that expiry (the shortest-lived frozen fact), not the
    // default TTL.
    #[tokio::test]
    async fn authorize_seals_witness_deadline_from_adopted_grant_expiry() {
        let expiry = chrono::DateTime::from_timestamp(2_000_000_000, 0).unwrap();
        let registry = echo_registry();
        // Never dispatched on this authorize-only path; errors if it ever is.
        let dispatcher =
            ironclaw_host_api::dispatch_test_support::TestDispatcher::responding(|req, _| {
                Err(DispatchError::UnknownCapability {
                    capability: req.invocation.capability.clone(),
                })
            });
        let authorizer = AllowAuthorizer;
        let trust_policy = StaticTrustPolicy;
        let runtime_policy = permissive_runtime_policy();
        let policy_facts = GrantWithExpiryPolicyFacts { expiry };
        let host = CapabilityHost::new(
            &registry,
            &dispatcher,
            &authorizer,
            &trust_policy,
            &runtime_policy,
            &policy_facts,
        );

        let request = allow_request();
        let fold = host.authorize(&request).await.unwrap();

        let AuthorizeFold::Authorized(fold) = fold else {
            panic!("expected an allowed authorization");
        };
        let Some(AuthorizeResult::Authorized(authorized)) = &fold.result else {
            panic!("allow path must mint an Authorized witness");
        };
        assert_eq!(
            authorized.deadline(),
            expiry,
            "adopted persistent-grant expiry is the shortest-lived frozen fact"
        );
    }

    #[tokio::test]
    async fn authorize_seals_system_actor_and_real_origin_across_ingresses() {
        use ironclaw_host_api::{InvocationOrigin, ProductKind, RoutineId, RunId, UserId};

        let registry = echo_registry();
        // Never dispatched on this authorize-only path; errors if it ever is.
        let dispatcher =
            ironclaw_host_api::dispatch_test_support::TestDispatcher::responding(|req, _| {
                Err(DispatchError::UnknownCapability {
                    capability: req.invocation.capability.clone(),
                })
            });
        let authorizer = AllowAuthorizer;
        let trust_policy = StaticTrustPolicy;
        let runtime_policy = permissive_runtime_policy();
        let policy_facts = SatisfiedPolicyFacts;
        let host = CapabilityHost::new(
            &registry,
            &dispatcher,
            &authorizer,
            &trust_policy,
            &runtime_policy,
            &policy_facts,
        );

        struct Case {
            actor_override: Option<UserId>,
            origin: Option<InvocationOrigin>,
            run_id: Option<RunId>,
            expected_actor: Actor,
            expected_origin: InvocationOrigin,
        }

        let loop_run = RunId::new();
        let cases = vec![
            Case {
                actor_override: None,
                origin: Some(InvocationOrigin::Product(
                    ProductKind::new("settings").unwrap(),
                )),
                run_id: None,
                expected_actor: Actor::System,
                expected_origin: InvocationOrigin::Product(ProductKind::new("settings").unwrap()),
            },
            Case {
                actor_override: Some(UserId::new("actor").unwrap()),
                origin: None,
                run_id: Some(loop_run),
                expected_actor: Actor::Sealed(UserId::new("actor").unwrap()),
                expected_origin: InvocationOrigin::LoopRun(loop_run),
            },
            Case {
                actor_override: None,
                origin: Some(InvocationOrigin::Automation(
                    RoutineId::new("heartbeat").unwrap(),
                )),
                run_id: None,
                expected_actor: Actor::System,
                expected_origin: InvocationOrigin::Automation(RoutineId::new("heartbeat").unwrap()),
            },
        ];

        for Case {
            actor_override,
            origin,
            run_id,
            expected_actor,
            expected_origin,
        } in cases
        {
            let mut request = allow_request();
            request.context.authenticated_actor_user_id = actor_override;
            request.context.origin = origin;
            request.context.run_id = run_id;

            let fold = host.authorize(&request).await.unwrap();
            let AuthorizeFold::Authorized(fold) = fold else {
                panic!("expected an allowed authorization for {expected_origin:?}");
            };
            let Some(AuthorizeResult::Authorized(authorized)) = &fold.result else {
                panic!("every allowed invocation must mint a witness ({expected_origin:?})");
            };
            let invocation = authorized.invocation();
            assert_eq!(
                invocation.actor, expected_actor,
                "actor mismatch for {expected_origin:?}"
            );
            assert_eq!(
                invocation.origin, expected_origin,
                "origin mismatch for {expected_origin:?}"
            );
        }
    }

    #[test]
    fn witness_deadline_takes_earliest_candidate_else_default_ttl() {
        let earlier = chrono::DateTime::from_timestamp(1_000, 0).unwrap();
        let later = chrono::DateTime::from_timestamp(2_000, 0).unwrap();
        // Shortest-lived candidate wins; `None` candidates are ignored.
        assert_eq!(
            witness_deadline([Some(later), None, Some(earlier)]),
            earlier
        );
        assert_eq!(witness_deadline([Some(earlier)]), earlier);
        // No frozen fact → bounded default TTL from now.
        let before = chrono::Utc::now();
        let fallback = witness_deadline([None, None]);
        let after = chrono::Utc::now();
        assert!(fallback >= before + WITNESS_DEFAULT_TTL);
        assert!(fallback <= after + WITNESS_DEFAULT_TTL);
    }

    // --- Resume-path witness deadline (`PendingClaim` lease expiry) ---

    // Lease store double for the resume dispatch tail. The pending approval
    // lease is claimed after authorization and consumed after successful
    // dispatch; all other lease operations are unreachable for this test.
    struct PendingClaimLeaseStore {
        lease: CapabilityLease,
    }

    #[async_trait::async_trait]
    impl CapabilityLeaseStore for PendingClaimLeaseStore {
        async fn issue(
            &self,
            _lease: CapabilityLease,
        ) -> Result<CapabilityLease, ironclaw_authorization::CapabilityLeaseError> {
            unimplemented!("authorize_resumed does not issue leases")
        }

        async fn revoke(
            &self,
            _scope: &ResourceScope,
            _lease_id: CapabilityGrantId,
        ) -> Result<CapabilityLease, ironclaw_authorization::CapabilityLeaseError> {
            unimplemented!("authorize_resumed does not revoke leases")
        }

        async fn get(
            &self,
            _scope: &ResourceScope,
            _lease_id: CapabilityGrantId,
        ) -> Option<CapabilityLease> {
            unimplemented!("authorize_resumed does not read leases")
        }

        async fn claim(
            &self,
            scope: &ResourceScope,
            lease_id: CapabilityGrantId,
            _invocation_fingerprint: &InvocationFingerprint,
        ) -> Result<CapabilityLease, ironclaw_authorization::CapabilityLeaseError> {
            assert_eq!(scope, &self.lease.scope); // safety: test-only lease-store double validates caller scope.
            assert_eq!(lease_id, self.lease.grant.id); // safety: test-only lease-store double validates caller lease id.
            let mut lease = self.lease.clone();
            lease.status = ironclaw_authorization::CapabilityLeaseStatus::Claimed;
            Ok(lease)
        }

        async fn consume(
            &self,
            scope: &ResourceScope,
            lease_id: CapabilityGrantId,
        ) -> Result<CapabilityLease, ironclaw_authorization::CapabilityLeaseError> {
            assert_eq!(scope, &self.lease.scope); // safety: test-only lease-store double validates caller scope.
            assert_eq!(lease_id, self.lease.grant.id); // safety: test-only lease-store double validates caller lease id.
            let mut lease = self.lease.clone();
            lease.status = ironclaw_authorization::CapabilityLeaseStatus::Consumed;
            Ok(lease)
        }

        async fn begin_dispatch_claimed(
            &self,
            _scope: &ResourceScope,
            _lease_id: CapabilityGrantId,
            _invocation_fingerprint: &InvocationFingerprint,
        ) -> Result<CapabilityLease, ironclaw_authorization::CapabilityLeaseError> {
            unimplemented!("authorize_resumed does not transition leases")
        }

        async fn abort_dispatch_claimed(
            &self,
            _scope: &ResourceScope,
            _lease_id: CapabilityGrantId,
        ) -> Result<CapabilityLease, ironclaw_authorization::CapabilityLeaseError> {
            unimplemented!("authorize_resumed does not transition leases")
        }

        async fn leases_for_scope(&self, _scope: &ResourceScope) -> Vec<CapabilityLease> {
            unimplemented!("authorize_resumed does not enumerate leases")
        }

        async fn active_leases_for_context(
            &self,
            _context: &ExecutionContext,
        ) -> Vec<CapabilityLease> {
            unimplemented!("authorize_resumed does not enumerate leases")
        }
    }

    // Run-state double for the successful resume tail: only the post-dispatch
    // completion transition is reachable.
    struct CompletionRunStateStore;

    #[async_trait::async_trait]
    impl RunStateStore for CompletionRunStateStore {
        async fn start(
            &self,
            _start: RunStart,
        ) -> Result<ironclaw_run_state::RunRecord, RunStateError> {
            unimplemented!("authorize_resumed Allow path does not mutate run state")
        }

        async fn block_approval(
            &self,
            _scope: &ResourceScope,
            _invocation_id: InvocationId,
            _approval: ironclaw_host_api::approval::ApprovalRequest,
        ) -> Result<ironclaw_run_state::RunRecord, RunStateError> {
            unimplemented!("authorize_resumed Allow path does not mutate run state")
        }

        async fn block_auth(
            &self,
            _scope: &ResourceScope,
            _invocation_id: InvocationId,
            _error_kind: String,
        ) -> Result<ironclaw_run_state::RunRecord, RunStateError> {
            unimplemented!("authorize_resumed Allow path does not mutate run state")
        }

        async fn complete(
            &self,
            scope: &ResourceScope,
            invocation_id: InvocationId,
        ) -> Result<ironclaw_run_state::RunRecord, RunStateError> {
            Ok(ironclaw_run_state::RunRecord {
                invocation_id,
                capability_id: CapabilityId::new("echo.say").unwrap(),
                scope: scope.clone(),
                authenticated_actor_user_id: None,
                status: RunStatus::Completed,
                approval_request_id: None,
                error_kind: None,
            })
        }

        async fn fail(
            &self,
            _scope: &ResourceScope,
            _invocation_id: InvocationId,
            _error_kind: String,
        ) -> Result<ironclaw_run_state::RunRecord, RunStateError> {
            unimplemented!("authorize_resumed Allow path does not mutate run state")
        }

        async fn get(
            &self,
            _scope: &ResourceScope,
            _invocation_id: InvocationId,
        ) -> Result<Option<ironclaw_run_state::RunRecord>, RunStateError> {
            unimplemented!("authorize_resumed Allow path does not read run state")
        }

        async fn records_for_scope(
            &self,
            _scope: &ResourceScope,
        ) -> Result<Vec<ironclaw_run_state::RunRecord>, RunStateError> {
            unimplemented!("authorize_resumed Allow path does not read run state")
        }
    }

    // A `resume_json` (`PendingClaim`) resume must seal the dispatch witness
    // deadline bounded by the approval lease's expiry — threaded onto the
    // pending-claim spec because the claim is deferred until after authorization
    // — NOT the 5-minute default TTL, so a held witness can never outlive the
    // approval that authorized it.
    #[tokio::test]
    async fn resumed_pending_claim_dispatch_seals_witness_deadline_from_lease_expiry() {
        // A lease expiry well inside the bounded 5-minute default window, so a
        // fallback to the default TTL would be observably wrong.
        let lease_expiry = chrono::Utc::now() + chrono::Duration::seconds(30);
        assert!(lease_expiry < chrono::Utc::now() + WITNESS_DEFAULT_TTL);

        let registry = echo_registry();
        let dispatcher =
            ironclaw_host_api::dispatch_test_support::TestDispatcher::responding(|request, _| {
                Ok(CapabilityDispatchResult {
                    capability_id: request.invocation.capability.clone(),
                    provider: ExtensionId::new("echo").unwrap(),
                    runtime: RuntimeKind::Wasm,
                    output: serde_json::json!({"ok": true}),
                    display_preview: None,
                    usage: ironclaw_host_api::ResourceUsage::default(),
                    receipt: ironclaw_host_api::ResourceReceipt {
                        id: ironclaw_host_api::ResourceReservationId::new(),
                        scope: request.invocation.scope.clone(),
                        status: ironclaw_host_api::ReservationStatus::Reconciled,
                        estimate: request.invocation.estimate.clone(),
                        actual: Some(ironclaw_host_api::ResourceUsage::default()),
                    },
                })
            });
        let authorizer = AllowAuthorizer;
        let trust_policy = StaticTrustPolicy;
        let runtime_policy = permissive_runtime_policy();
        let policy_facts = SatisfiedPolicyFacts;
        let host = CapabilityHost::new(
            &registry,
            &dispatcher,
            &authorizer,
            &trust_policy,
            &runtime_policy,
            &policy_facts,
        );

        let request = allow_request();
        let capability_id = request.capability_id.clone();
        let estimate = request.estimate.clone();
        let input = request.input.clone();
        let context = request.context.clone();
        let scope = context.resource_scope.clone();
        let invocation_id = context.invocation_id;
        let descriptor = registry
            .get_capability(&capability_id)
            .expect("echo.say is registered");

        let grant_id = CapabilityGrantId::new();
        let fingerprint =
            InvocationFingerprint::for_dispatch(&scope, &capability_id, &estimate, &input).unwrap();
        let leases = PendingClaimLeaseStore {
            lease: CapabilityLease {
                scope: scope.clone(),
                grant: ironclaw_host_api::CapabilityGrant {
                    id: grant_id,
                    capability: capability_id.clone(),
                    grantee: ironclaw_host_api::Principal::User(scope.user_id.clone()),
                    issued_by: ironclaw_host_api::Principal::HostRuntime,
                    constraints: ironclaw_host_api::GrantConstraints {
                        allowed_effects: Vec::new(),
                        mounts: ironclaw_host_api::MountView::default(),
                        network: ironclaw_host_api::NetworkPolicy::default(),
                        secrets: Vec::new(),
                        resource_ceiling: None,
                        expires_at: Some(lease_expiry),
                        max_invocations: None,
                    },
                },
                invocation_fingerprint: Some(fingerprint.clone()),
                status: ironclaw_authorization::CapabilityLeaseStatus::Active,
            },
        };
        let run_state = CompletionRunStateStore;

        let params = ResumedDispatchParams {
            run_state: &run_state,
            scope,
            invocation_id,
            capability_id,
            estimate,
            input,
            authorized_context: context,
            descriptor,
            lease_state: ResumedLeaseState::PendingClaim(PendingClaimAfterAuth {
                leases: &leases,
                grant_id,
                fingerprint,
                grant_expiry: Some(lease_expiry),
            }),
        };

        host.dispatch_resumed_capability(params).await.unwrap();
        let dispatched = dispatcher.last_request().unwrap();
        assert_eq!(
            dispatched.deadline, lease_expiry,
            "the sealed witness deadline must be bounded by the approval lease expiry, not the default TTL"
        );
    }
}
