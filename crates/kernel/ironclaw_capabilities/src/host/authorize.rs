//! The authorization fold — the one decision every workflow funnels through.
//!
//! `authorize` is the kernel's in-fold verdict (trust, runtime policy,
//! credential presence, approvals, obligations) and `seal_authorization` mints
//! the [`Authorized`] witness. Both are shared by invoke, spawn and the resume
//! tail, so they live here rather than with any one workflow.

use ironclaw_host_api::{
    Timestamp,
    authorized::{AuthorizeResult, Authorized, CapabilityAuthorizer},
    capability::{CapabilityDescriptor, EffectKind, PermissionMode, RuntimeCredentialRequirement},
    decision::{Decision, DenyReason},
    dispatch::{CapabilityDispatcher, DispatchAuthRequirement},
    ids::{ActivityId, CapabilityId, DenyRef, GateRef},
    invocation::{Actor, Invocation},
    lane::RuntimeLane,
    resolution::{Blocked, GateWaypoint},
    resource::ResourceEstimate,
    runtime_policy::ProcessBackendKind,
    scope::ExecutionContext,
};
use ironclaw_processes::ProcessInvocationStart;
use ironclaw_runtime_policy::plan_capability;
use ironclaw_trust::TrustDecision;
use tracing::{debug, warn};

use super::error_mapping::{
    add_capability_input_display_hint, obligation_invocation_error_kind,
    runtime_policy_error_to_invocation_error, trust_error_to_invocation_error,
};
use super::{AuthorizeFold, AuthorizedFold, CapabilityHost, InvocationInput};
use crate::helpers::{
    CapabilityActionKind, apply_invocation_state_transition_if_configured,
    fail_invocation_if_configured, invocation_fingerprint_for_kind,
    validate_approval_request_matches_invocation,
};
use crate::ports::{CredentialPresence, PolicyAction};
use crate::trust::{evaluate_invocation_trust, evaluate_package_trust};
use crate::{CapabilityInvocationError, CapabilityObligationOutcome, CapabilityObligationPhase};

impl<'a, D> CapabilityHost<'a, D>
where
    D: CapabilityDispatcher + ?Sized,
{
    /// Compute provider trust for `capability_id` (§5.3.2/§9): the kernel now
    /// classifies trust itself instead of trusting a caller-stamped field.
    pub(super) fn evaluate_trust(
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
    pub(super) fn enforce_runtime_policy(
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
    /// Add explicitly selected, manifest-declared credentials to a sandboxed
    /// shell invocation before authorization.
    ///
    /// `credential_contexts` contains active extension IDs. Each selected
    /// extension contributes only requirements that declare `placeholder_env`;
    /// ordinary command text never acquires credentials implicitly. The frozen
    /// descriptor then drives approval, credential staging, and proxy policy.
    pub(super) async fn enrich_invocation_descriptor(
        &self,
        descriptor: &CapabilityDescriptor,
        capability_id: &CapabilityId,
        input: &serde_json::Value,
    ) -> Result<CapabilityDescriptor, CapabilityInvocationError> {
        if capability_id.as_str() != "builtin.shell" {
            return Ok(descriptor.clone());
        }
        let contexts =
            ironclaw_host_api::process::shell_credential_contexts(input).map_err(|error| {
                CapabilityInvocationError::AuthorizationDenied {
                    capability: capability_id.clone(),
                    reason: DenyReason::PolicyDenied,
                    detail: Some(error.to_string()),
                }
            })?;
        if contexts.is_empty() {
            return Ok(descriptor.clone());
        }
        if self.runtime_policy.process_backend != ProcessBackendKind::UserSandbox {
            return Err(CapabilityInvocationError::AuthorizationDenied {
                capability: capability_id.clone(),
                reason: DenyReason::PolicyDenied,
                detail: Some(
                    "shell credential contexts require the managed user sandbox".to_string(),
                ),
            });
        }

        let mut selected: Vec<RuntimeCredentialRequirement> = Vec::new();
        for context in contexts {
            let Some(package) = self.registry.get_extension(&context) else {
                return Err(CapabilityInvocationError::AuthorizationDenied {
                    capability: capability_id.clone(),
                    reason: DenyReason::PolicyDenied,
                    detail: Some(format!(
                        "shell credential context `{context}` is not an active extension"
                    )),
                });
            };
            let package_trust = evaluate_package_trust(self.registry, self.trust_policy, &context)
                .map_err(|error| trust_error_to_invocation_error(capability_id, error))?;
            if !package_trust.effective_trust.is_privileged() {
                return Err(CapabilityInvocationError::AuthorizationDenied {
                    capability: capability_id.clone(),
                    reason: DenyReason::PolicyDenied,
                    detail: Some(format!(
                        "shell credential context `{context}` is not trusted for host execution"
                    )),
                });
            }
            let mut context_selected = 0usize;
            for declared in package
                .capabilities
                .iter()
                .flat_map(|candidate| candidate.runtime_credentials.iter())
                .filter(|requirement| requirement.placeholder_env.is_some())
            {
                context_selected += 1;
                if let Some(existing) = selected
                    .iter()
                    .find(|existing| existing.handle == declared.handle)
                {
                    if existing != declared {
                        return Err(CapabilityInvocationError::AuthorizationDenied {
                            capability: capability_id.clone(),
                            reason: DenyReason::PolicyDenied,
                            detail: Some(format!(
                                "shell credential context `{context}` has conflicting declarations \
                                 for handle `{}`",
                                declared.handle
                            )),
                        });
                    }
                } else {
                    selected.push(declared.clone());
                }
            }
            if context_selected == 0 {
                return Err(CapabilityInvocationError::AuthorizationDenied {
                    capability: capability_id.clone(),
                    reason: DenyReason::PolicyDenied,
                    detail: Some(format!(
                        "shell credential context `{context}` declares no shell credentials"
                    )),
                });
            }
        }

        let mut descriptor = descriptor.clone();
        if !descriptor.effects.contains(&EffectKind::UseSecret) {
            descriptor.effects.push(EffectKind::UseSecret);
        }
        descriptor.runtime_credentials = selected;
        Ok(descriptor)
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
    pub(super) async fn apply_persistent_approval(
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

    /// The pre-dispatch authority fold for `invoke_json`, extracted per
    /// arch-simplification §9 step 2 / §5.3.2: validate the context, fingerprint
    /// the invocation, start the invocation record, resolve the descriptor, run
    /// trust-aware authorization, and on `Allow` prepare obligations and mint
    /// the sealed [`Authorized`] witness. Every side effect that today's inline
    /// fold performed — process-invocation `start`/`fail`/`block`, approval
    /// persist-and-rollback, obligation `prepare`, and each early error return —
    /// stays here, verbatim; `invoke_json` only maps the returned
    /// [`AuthorizeFold`] back to today's outcome.
    pub(super) async fn authorize(
        &self,
        request: &InvocationInput,
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

        // Resolve the descriptor BEFORE starting a invocation record: an unknown
        // capability must short-circuit without creating a invocation record (restoring
        // the behavior host_runtime's deleted pre-check provided). Neither the
        // fingerprint above nor `invocation_state.start` below needs the descriptor, so
        // hoisting this lookup is safe; everything from `start` onward keeps its
        // original order (the credential pre-flight still runs after `start`).
        let Some(base_descriptor) = self.registry.get_capability(&request.capability_id) else {
            debug!("capability invocation failed before authorization: unknown capability");
            return Err(CapabilityInvocationError::UnknownCapability {
                capability: request.capability_id.clone(),
            });
        };
        let descriptor = self
            .enrich_invocation_descriptor(base_descriptor, &request.capability_id, &request.input)
            .await?;

        if let Some(invocation_state) = self.invocation_state {
            invocation_state
                .start(ProcessInvocationStart {
                    invocation_id,
                    capability_id: request.capability_id.clone(),
                    scope: scope.clone(),
                    authenticated_actor_user_id: request
                        .context
                        .authenticated_actor_user_id
                        .clone(),
                })
                .await?;
            debug!("capability invocation state started");
        }

        // Kernel-computed trust + in-fold runtime-policy planning (§5.3.2/§9),
        // relocated from host_runtime's `open_pre_authorization`. The
        // `context.trust` stamp reproduces what `open_pre_authorization` did
        // before calling the authorizer.
        let trust_decision = match self.evaluate_trust(&request.capability_id) {
            Ok(d) => d,
            Err(error) => {
                apply_invocation_state_transition_if_configured(
                    self.invocation_state,
                    &scope,
                    invocation_id,
                    &error,
                )
                .await;
                return Err(error);
            }
        };
        if let Err(error) = self.enforce_runtime_policy(&descriptor) {
            apply_invocation_state_transition_if_configured(
                self.invocation_state,
                &scope,
                invocation_id,
                &error,
            )
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
            .credential_presence(&descriptor, &scope)
            .await
        {
            CredentialPresence::Satisfied | CredentialPresence::Indeterminate => {}
            CredentialPresence::Missing {
                required_secrets,
                requirements,
            } => {
                let error = CapabilityInvocationError::AuthorizationRequiresAuth {
                    capability: request.capability_id.clone(),
                    requirement: Box::new(DispatchAuthRequirement {
                        required_secrets,
                        credential_requirements: requirements,
                        model_visible_cause: None,
                    }),
                };
                apply_invocation_state_transition_if_configured(
                    self.invocation_state,
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
                &descriptor,
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
                &descriptor,
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
                        apply_invocation_state_transition_if_configured(
                            self.invocation_state,
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
                    &descriptor,
                    &obligation_outcome,
                    frozen_deadline,
                )?;
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
                fail_invocation_if_configured(
                    self.invocation_state,
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
                    fail_invocation_if_configured(
                        self.invocation_state,
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
                        fail_invocation_if_configured(
                            self.invocation_state,
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

                match (self.invocation_state, self.approval_requests) {
                    (Some(invocation_state), Some(approval_requests)) => {
                        let approval_id = approval.id;
                        if let Err(error) = approval_requests
                            .save_pending(scope.clone(), approval.clone())
                            .await
                        {
                            debug!(
                                approval_request_id = %approval_id,
                                "capability approval request persistence failed"
                            );
                            fail_invocation_if_configured(
                                Some(invocation_state),
                                &scope,
                                invocation_id,
                                "ApprovalStore",
                            )
                            .await;
                            return Err(CapabilityInvocationError::from(error));
                        }
                        if let Err(error) = invocation_state
                            .block_approval(&scope, invocation_id, approval)
                            .await
                        {
                            debug!(
                                approval_request_id = %approval_id,
                                "capability invocation approval block failed"
                            );
                            if let Err(discard_error) =
                                approval_requests.discard_pending(&scope, approval_id).await
                            {
                                warn!(
                                    approval_request_id = %approval_id,
                                    invocation_id = %invocation_id,
                                    transition_error_kind = "ApprovalStore",
                                    error = %discard_error,
                                    "approval rollback failed after invocation block transition failed",
                                );
                            }
                            fail_invocation_if_configured(
                                Some(invocation_state),
                                &scope,
                                invocation_id,
                                "ApprovalBlock",
                            )
                            .await;
                            return Err(CapabilityInvocationError::from(error));
                        }
                        debug!(
                            approval_request_id = %approval_id,
                            "capability approval persisted and invocation blocked"
                        );
                    }
                    (Some(invocation_state), None) => {
                        debug!(
                            approval_request_id = %approval_request_id,
                            store = "approval_requests",
                            "capability approval cannot block because store is missing"
                        );
                        fail_invocation_if_configured(
                            Some(invocation_state),
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
                            store = "invocation_state",
                            "capability approval cannot block because store is missing"
                        );
                        return Err(CapabilityInvocationError::ApprovalStoreMissing {
                            capability: request.capability_id.clone(),
                            store: "invocation_state",
                        });
                    }
                    (None, None) => {
                        debug!(
                            approval_request_id = %approval_request_id,
                            store = "invocation_state and approval_requests",
                            "capability approval cannot block because stores are missing"
                        );
                        return Err(CapabilityInvocationError::ApprovalStoreMissing {
                            capability: request.capability_id.clone(),
                            store: "invocation_state and approval_requests",
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
    pub(super) fn seal_authorization(
        &self,
        context: &ExecutionContext,
        capability_id: &CapabilityId,
        estimate: &ResourceEstimate,
        input: &serde_json::Value,
        descriptor: &CapabilityDescriptor,
        obligation_outcome: &CapabilityObligationOutcome,
        frozen_deadline: Option<Timestamp>,
    ) -> Result<Option<AuthorizeResult>, CapabilityInvocationError> {
        // Actor is sealed at the membrane; NO fallback to `user_id`. An
        // actor-less (system service / one-shot) context seals `Actor::System`
        // as its own class.
        let actor = match context.authenticated_actor_user_id.clone() {
            Some(user_id) => Actor::Sealed(user_id),
            None => Actor::System,
        };
        // Lane resolved from the descriptor's runtime kind; `System` runtimes
        // have no untrusted execution lane (`None`) and are not sealed here.
        let Some(lane) = RuntimeLane::from_runtime_kind(descriptor.runtime) else {
            return Ok(None);
        };
        let scope = &context.resource_scope;
        // Origin is the ingress-stamped authority fact (§5.2.1). The loop path
        // also carries `run_id`, so a context that stamped only `run_id` still
        // reconstructs `LoopRun` for transitional compatibility.
        let Some(origin) = context.resolved_origin() else {
            return Ok(None);
        };
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
        Authorized::seal(
            self.authorization_grant(),
            invocation,
            descriptor.clone(),
            lane,
            mounts,
            reservation,
            deadline,
        )
        .map(|authorized| Some(AuthorizeResult::Authorized(Box::new(authorized))))
        .map_err(|error| CapabilityInvocationError::AuthorizationDenied {
            capability: capability_id.clone(),
            reason: DenyReason::PolicyDenied,
            detail: Some(error.to_string()),
        })
    }
}

/// Bounded default validity window for the sealed witness when the authorization
/// froze no shorter-lived fact. Keeps no-frozen-fact capabilities on the prior
/// fixed window; a frozen fact, when present, always shortens this.
pub(super) const WITNESS_DEFAULT_TTL: chrono::Duration = chrono::Duration::minutes(5);

/// Derive the sealed witness deadline from the shortest-lived frozen fact so a
/// held witness cannot outlive the facts that justified it (§5.3.2): take the
/// earliest of the candidate expiries, falling back to [`WITNESS_DEFAULT_TTL`]
/// from now when none is present. Candidate expiries today are the adopted
/// persistent-grant expiry (invoke/spawn) and the claimed approval lease's expiry
/// (resume). Credential-lease expiry integration is future — the credential
/// presence port returns presence, not lease expiry — so it is not a candidate
/// yet; do not block on it.
pub(super) fn witness_deadline<I>(candidate_expiries: I) -> Timestamp
where
    I: IntoIterator<Item = Option<Timestamp>>,
{
    candidate_expiries
        .into_iter()
        .flatten()
        .min()
        .unwrap_or_else(|| chrono::Utc::now() + WITNESS_DEFAULT_TTL)
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
pub(super) fn permission_mode_allows_persistent_approval(permission: PermissionMode) -> bool {
    matches!(permission, PermissionMode::Allow | PermissionMode::Ask)
}
