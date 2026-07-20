//! Product-auth terminal resolution handling.
//!
//! Auth owns the durable terminal outcome. Product workflow maps that outcome
//! onto the exact parked turn gate, treating stale or duplicate delivery as a
//! successful no-op.

use std::sync::Arc;

use ironclaw_auth::{AuthContinuationRef, AuthFlowOutcome, AuthResolved};
use ironclaw_turns::{
    CancelRunPrecondition, CancelRunRequest, CancelRunResponse, GateRef, GateResumeDisposition,
    GetRunStateRequest, IdempotencyKey, ResumeTurnPrecondition, ResumeTurnRequest,
    ResumeTurnResponse, SanitizedCancelReason, TurnCoordinator, TurnError, TurnRunId, TurnScope,
    TurnStatus,
};
use uuid::Uuid;

use crate::binding_ref::{
    AUTH_CONTINUATION_BINDING_REF_RAW_MAX_BYTES, binding_ref_segment, bounded_idempotency_key,
};
use crate::{AuthContinuationRejectionKind, ProductWorkflowError};

/// User-visible effect produced by delivering one durable auth resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthResolutionDispatchOutcome {
    Resumed(ResumeTurnResponse),
    Canceled(CancelRunResponse),
    Ignored,
}

#[derive(Clone)]
pub struct ProductAuthTurnGateResumeDispatcher {
    turn_coordinator: Arc<dyn TurnCoordinator>,
}

impl ProductAuthTurnGateResumeDispatcher {
    pub fn new(turn_coordinator: Arc<dyn TurnCoordinator>) -> Self {
        Self { turn_coordinator }
    }

    /// Deliver one durable terminal auth result to its exact turn gate.
    ///
    /// Delivery is intentionally idempotent: once the run is no longer parked
    /// on this exact auth gate, the resolution has no remaining authority and
    /// returns [`AuthResolutionDispatchOutcome::Ignored`].
    pub async fn dispatch_auth_resolved(
        &self,
        resolution: AuthResolved,
    ) -> Result<AuthResolutionDispatchOutcome, ProductWorkflowError> {
        let AuthContinuationRef::TurnGateResume {
            turn_run_ref,
            gate_ref,
        } = &resolution.continuation
        else {
            return Ok(AuthResolutionDispatchOutcome::Ignored);
        };

        let run_id = parse_turn_run_id(turn_run_ref.as_str())?;
        let scope = turn_scope_from_auth_resolution(&resolution)?;
        let gate_resolution_ref = parse_gate_ref(gate_ref.as_str())?;
        let state = match self
            .turn_coordinator
            .get_run_state(GetRunStateRequest {
                scope: scope.clone(),
                run_id,
            })
            .await
        {
            Ok(state) => state,
            Err(TurnError::ScopeNotFound) => return Ok(AuthResolutionDispatchOutcome::Ignored),
            Err(error) => return Err(map_auth_resolution_error(error)),
        };
        if state.status != TurnStatus::BlockedAuth
            || state.gate_ref.as_ref() != Some(&gate_resolution_ref)
        {
            return Ok(AuthResolutionDispatchOutcome::Ignored);
        }

        let actor = state
            .actor
            .ok_or(ProductWorkflowError::AuthContinuationRejected {
                kind: AuthContinuationRejectionKind::UnauthorizedBlockedGate,
            })?;
        let binding_id = auth_resolution_binding_id(
            resolution.flow_id,
            &run_id,
            gate_ref.as_str(),
            resolution_outcome_key(resolution.outcome),
        );
        let idempotency_key = idempotency_key_for_binding(&binding_id)?;

        match resolution.outcome {
            AuthFlowOutcome::UserAborted => {
                let result = self
                    .turn_coordinator
                    .cancel_run(CancelRunRequest {
                        scope,
                        actor,
                        run_id,
                        precondition: Some(CancelRunPrecondition::BlockedAuthGate {
                            gate_ref: gate_resolution_ref,
                        }),
                        reason: SanitizedCancelReason::UserRequested,
                        idempotency_key,
                    })
                    .await;
                match result {
                    Ok(response) => Ok(AuthResolutionDispatchOutcome::Canceled(response)),
                    Err(error) if stale_gate_transition(&error) => {
                        Ok(AuthResolutionDispatchOutcome::Ignored)
                    }
                    Err(error) => Err(map_auth_resolution_error(error)),
                }
            }
            outcome => {
                let resume_disposition = match outcome {
                    AuthFlowOutcome::Authorized { .. } => None,
                    AuthFlowOutcome::ProviderDenied => Some(GateResumeDisposition::Denied),
                    AuthFlowOutcome::Expired | AuthFlowOutcome::Failed { .. } => {
                        Some(GateResumeDisposition::Error)
                    }
                    // The outer match handles this arm with exact cancellation.
                    AuthFlowOutcome::UserAborted => None,
                };
                let result = self
                    .turn_coordinator
                    .resume_turn(ResumeTurnRequest {
                        scope,
                        actor,
                        run_id,
                        gate_resolution_ref,
                        source_binding_ref: state.source_binding_ref,
                        reply_target_binding_ref: state.reply_target_binding_ref,
                        idempotency_key,
                        precondition: ResumeTurnPrecondition::BlockedAuthGate,
                        resume_disposition,
                    })
                    .await;
                match result {
                    Ok(response) => Ok(AuthResolutionDispatchOutcome::Resumed(response)),
                    Err(error) if stale_gate_transition(&error) => {
                        Ok(AuthResolutionDispatchOutcome::Ignored)
                    }
                    Err(error) => Err(map_auth_resolution_error(error)),
                }
            }
        }
    }
}

fn stale_gate_transition(error: &TurnError) -> bool {
    matches!(
        error,
        TurnError::ScopeNotFound
            | TurnError::InvalidTransition { .. }
            | TurnError::InvalidRequest { .. }
    )
}

fn map_auth_resolution_error(error: TurnError) -> ProductWorkflowError {
    match error {
        TurnError::Unauthorized | TurnError::LeaseMismatch => {
            ProductWorkflowError::TurnResumeDenied { error }
        }
        error => ProductWorkflowError::TurnSubmissionFailed { error },
    }
}

fn auth_resolution_binding_id(
    flow_id: ironclaw_auth::AuthFlowId,
    run_id: &TurnRunId,
    gate_ref: &str,
    outcome: &str,
) -> String {
    format!(
        "{}{}{}{}{}",
        binding_ref_segment("surface", "auth-resolution"),
        binding_ref_segment("flow", &flow_id.to_string()),
        binding_ref_segment("run", &run_id.to_string()),
        binding_ref_segment("gate", gate_ref),
        binding_ref_segment("outcome", outcome),
    )
}

const fn resolution_outcome_key(outcome: AuthFlowOutcome) -> &'static str {
    match outcome {
        AuthFlowOutcome::Authorized { .. } => "authorized",
        AuthFlowOutcome::ProviderDenied => "provider-denied",
        AuthFlowOutcome::UserAborted => "user-aborted",
        AuthFlowOutcome::Expired => "expired",
        AuthFlowOutcome::Failed { .. } => "failed",
    }
}

fn turn_scope_from_auth_resolution(
    resolution: &AuthResolved,
) -> Result<TurnScope, ProductWorkflowError> {
    let Some(thread_id) = resolution.scope.resource.thread_id.clone() else {
        return Err(ProductWorkflowError::AuthContinuationRejected {
            kind: AuthContinuationRejectionKind::MissingThreadScope,
        });
    };
    Ok(TurnScope::new_with_owner(
        resolution.scope.resource.tenant_id.clone(),
        resolution.scope.resource.agent_id.clone(),
        resolution.scope.resource.project_id.clone(),
        thread_id,
        Some(resolution.scope.resource.user_id.clone()),
    ))
}

fn parse_turn_run_id(value: &str) -> Result<TurnRunId, ProductWorkflowError> {
    Uuid::parse_str(value)
        .map(TurnRunId::from_uuid)
        .map_err(|_| ProductWorkflowError::AuthContinuationRejected {
            kind: AuthContinuationRejectionKind::InvalidTurnRunRef,
        })
}

fn parse_gate_ref(value: &str) -> Result<GateRef, ProductWorkflowError> {
    GateRef::new(value.to_string()).map_err(|_| ProductWorkflowError::AuthContinuationRejected {
        kind: AuthContinuationRejectionKind::InvalidGateRef,
    })
}

fn idempotency_key_for_binding(binding_id: &str) -> Result<IdempotencyKey, ProductWorkflowError> {
    bounded_idempotency_key(
        "auth-resolution",
        binding_id,
        AUTH_CONTINUATION_BINDING_REF_RAW_MAX_BYTES,
    )
    .map_err(|_| ProductWorkflowError::AuthContinuationRejected {
        kind: AuthContinuationRejectionKind::InvalidIdempotencyKey,
    })
}
