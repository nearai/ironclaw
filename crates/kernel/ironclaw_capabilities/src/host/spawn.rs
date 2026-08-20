//! Workflow 6 — `spawn_json`: starting a capability as a background process.
//!
//! `spawn_json` is the caller-facing entry point; `authorize_spawn` is its
//! private fold twin — the spawn-shaped counterpart of [`super::authorize`],
//! which additionally owns the process `start`/`fail`/`block` transitions and
//! the persist-and-rollback of a pending approval.

use ironclaw_host_api::{
    authorized::AuthorizeResult,
    decision::{Decision, DenyReason},
    dispatch::CapabilityDispatcher,
    ids::{DenyRef, GateRef, ProcessId},
    resolution::{Blocked, GateWaypoint},
};
use ironclaw_processes::{ProcessInvocationStart, ProcessStart};
use tracing::warn;

use super::error_mapping::add_capability_input_display_hint;
use super::{AuthorizeFold, AuthorizedFold, CapabilityHost, process_authorized_continuation};
use crate::helpers::{
    CapabilityActionKind, apply_invocation_state_transition_if_configured,
    complete_invocation_after_side_effect, fail_invocation_if_configured,
    invocation_fingerprint_for_kind, validate_approval_request_matches_invocation,
};
use crate::ports::{CredentialPresence, PolicyAction};
use crate::{
    CapabilityInvocationError, CapabilityObligationPhase, CapabilitySpawnRequest,
    CapabilitySpawnResult,
};

impl<'a, D> CapabilityHost<'a, D>
where
    D: CapabilityDispatcher + ?Sized,
{
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
        // process-invocation start, capability lookup, trust-aware spawn authorization,
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
            fail_invocation_if_configured(
                self.invocation_state,
                &scope,
                invocation_id,
                "UnknownCapability",
            )
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
                fail_invocation_if_configured(
                    self.invocation_state,
                    &scope,
                    invocation_id,
                    "ProcessSpawn",
                )
                .await;
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
                fail_invocation_if_configured(
                    self.invocation_state,
                    &scope,
                    invocation_id,
                    "ProcessSpawn",
                )
                .await;
                return Err(CapabilityInvocationError::from(error));
            }
        };

        if let Some(invocation_state) = self.invocation_state {
            complete_invocation_after_side_effect(
                invocation_state,
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
    /// Every side effect the inline fold performed — process-invocation
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

        // Resolve the descriptor BEFORE starting a invocation record (see `authorize`):
        // an unknown capability short-circuits without creating a invocation record, so
        // no `fail_invocation_if_configured` is needed here.
        let Some(descriptor) = self.registry.get_capability(&request.capability_id) else {
            return Err(CapabilityInvocationError::UnknownCapability {
                capability: request.capability_id.clone(),
            });
        };

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
        }

        // Kernel-computed trust + in-fold runtime-policy planning (§5.3.2/§9),
        // mirroring `authorize()` on the spawn path.
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
        if let Err(error) = self.enforce_runtime_policy(descriptor) {
            apply_invocation_state_transition_if_configured(
                self.invocation_state,
                &scope,
                invocation_id,
                &error,
            )
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
                    model_visible_cause: None,
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
                if let Err(error) = validate_approval_request_matches_invocation(
                    &approval,
                    &request.context,
                    &request.capability_id,
                    &request.estimate,
                    CapabilityActionKind::Spawn,
                ) {
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
                            if let Err(discard_error) =
                                approval_requests.discard_pending(&scope, approval_id).await
                            {
                                warn!(
                                    approval_request_id = %approval_id,
                                    invocation_id = %invocation_id,
                                    transition_error_kind = "ApprovalStore",
                                    error = %discard_error,
                                    "approval rollback failed after spawn invocation block transition failed",
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
                    }
                    (Some(invocation_state), None) => {
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
                        return Err(CapabilityInvocationError::ApprovalStoreMissing {
                            capability: request.capability_id.clone(),
                            store: "invocation_state",
                        });
                    }
                    (None, None) => {
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
}
