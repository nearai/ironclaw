use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_auth::{
    AuthChallenge, AuthFlowId, AuthFlowManager, AuthFlowStatus, AuthProductError,
    CredentialAccountId, CredentialSelectionInput,
};
use ironclaw_turns::{
    CancelRunPrecondition, CancelRunRequest, GateRef, GateResumeDisposition, GetRunStateRequest,
    ResumeTurnPrecondition, ResumeTurnRequest, SanitizedCancelReason, TurnCoordinator, TurnError,
    TurnErrorCategory, TurnRunId, TurnStatus,
};

use super::types::is_pending_auth_status;
use super::{
    AuthGateRecord, AuthInteractionDecision, AuthInteractionRejectionKind, AuthInteractionScope,
    ListPendingAuthInteractionsRequest, ListPendingAuthInteractionsResponse,
    ResolveAuthInteractionRequest, ResolveAuthInteractionResponse, auth_rejected,
};
use crate::error::ProductWorkflowError;
use crate::gate_state::{BlockedGateState, BlockedGateStateError, blocked_gate_state};

#[async_trait]
pub trait AuthInteractionReadModel: Send + Sync {
    async fn auth_gates(
        &self,
        scope: &AuthInteractionScope,
    ) -> Result<Vec<AuthGateRecord>, ProductWorkflowError>;

    async fn auth_gate(
        &self,
        scope: &AuthInteractionScope,
        run_id_hint: Option<TurnRunId>,
        gate_ref: &GateRef,
    ) -> Result<Option<AuthGateRecord>, ProductWorkflowError>;
}

/// Auth-required service consumed by product/WebUI surfaces.
#[async_trait]
pub trait AuthInteractionService: Send + Sync {
    async fn list_pending(
        &self,
        request: ListPendingAuthInteractionsRequest,
    ) -> Result<ListPendingAuthInteractionsResponse, ProductWorkflowError>;

    async fn resolve(
        &self,
        request: ResolveAuthInteractionRequest,
    ) -> Result<ResolveAuthInteractionResponse, ProductWorkflowError>;
}

pub(crate) struct RejectingAuthInteractionService;

#[async_trait]
impl AuthInteractionService for RejectingAuthInteractionService {
    async fn list_pending(
        &self,
        _request: ListPendingAuthInteractionsRequest,
    ) -> Result<ListPendingAuthInteractionsResponse, ProductWorkflowError> {
        Err(auth_rejected(AuthInteractionRejectionKind::FlowUnavailable))
    }

    async fn resolve(
        &self,
        _request: ResolveAuthInteractionRequest,
    ) -> Result<ResolveAuthInteractionResponse, ProductWorkflowError> {
        Err(auth_rejected(AuthInteractionRejectionKind::FlowUnavailable))
    }
}

pub struct DefaultAuthInteractionService {
    read_model: Arc<dyn AuthInteractionReadModel>,
    flow_manager: Arc<dyn AuthFlowManager>,
    turn_coordinator: Arc<dyn TurnCoordinator>,
}

impl DefaultAuthInteractionService {
    pub fn new(
        read_model: Arc<dyn AuthInteractionReadModel>,
        flow_manager: Arc<dyn AuthFlowManager>,
        turn_coordinator: Arc<dyn TurnCoordinator>,
    ) -> Self {
        Self {
            read_model,
            flow_manager,
            turn_coordinator,
        }
    }

    async fn find_gate(
        &self,
        scope: &AuthInteractionScope,
        run_id_hint: Option<TurnRunId>,
        gate_ref: &GateRef,
    ) -> Result<AuthGateRecord, ProductWorkflowError> {
        let gate = self
            .read_model
            .auth_gate(scope, run_id_hint, gate_ref)
            .await?
            .ok_or_else(|| auth_rejected(AuthInteractionRejectionKind::MissingAuth))?;
        if gate.scope() != scope {
            return Err(auth_rejected(
                AuthInteractionRejectionKind::CrossScopeDenied,
            ));
        }
        Ok(gate)
    }

    async fn refresh_gate(
        &self,
        gate: &AuthGateRecord,
    ) -> Result<AuthGateRecord, ProductWorkflowError> {
        let Some(flow) = self
            .flow_manager
            .get_flow(&gate.flow().scope, gate.flow().id)
            .await
            .map_err(map_auth_product_error)?
        else {
            return Err(auth_rejected(AuthInteractionRejectionKind::MissingAuth));
        };
        AuthGateRecord::new(gate.run_id(), gate.gate_ref().clone(), flow)
    }

    async fn turn_gate_state(
        &self,
        request: &ResolveAuthInteractionRequest,
        run_id: TurnRunId,
    ) -> Result<BlockedGateState, ProductWorkflowError> {
        blocked_gate_state(
            self.turn_coordinator.as_ref(),
            &request.scope,
            &request.actor,
            run_id,
            &request.gate_ref,
            TurnStatus::BlockedAuth,
        )
        .await
        .map_err(map_blocked_gate_state_error)
    }

    async fn resume_completed_auth(
        &self,
        request: ResolveAuthInteractionRequest,
        gate: AuthGateRecord,
        run_id: TurnRunId,
    ) -> Result<ResolveAuthInteractionResponse, ProductWorkflowError> {
        let completion = match &request.decision {
            AuthInteractionDecision::CredentialProvided { credential_ref } => {
                AuthCompletionRef::CredentialProvided(credential_ref)
            }
            AuthInteractionDecision::CallbackCompleted { callback_ref } => {
                AuthCompletionRef::CallbackCompleted(callback_ref)
            }
            AuthInteractionDecision::Deny => {
                return Err(auth_rejected(
                    AuthInteractionRejectionKind::UnsupportedResult,
                ));
            }
        };
        validate_completion_ref(&gate, completion)?;
        self.resume_auth_gate(request, run_id, None).await
    }

    async fn resume_auth_gate(
        &self,
        request: ResolveAuthInteractionRequest,
        run_id: TurnRunId,
        resume_disposition: Option<GateResumeDisposition>,
    ) -> Result<ResolveAuthInteractionResponse, ProductWorkflowError> {
        let state = self
            .turn_coordinator
            .get_run_state(GetRunStateRequest {
                scope: request.scope.clone(),
                run_id,
            })
            .await
            .map_err(map_auth_resume_error)?;
        let response = self
            .turn_coordinator
            .resume_turn(ResumeTurnRequest {
                scope: request.scope,
                actor: request.actor,
                run_id,
                gate_resolution_ref: request.gate_ref,
                precondition: ResumeTurnPrecondition::BlockedAuthGate,
                source_binding_ref: state.source_binding_ref,
                reply_target_binding_ref: state.reply_target_binding_ref,
                idempotency_key: request.idempotency_key,
                resume_disposition,
            })
            .await
            .map_err(map_auth_resume_error)?;
        Ok(ResolveAuthInteractionResponse::Resumed(response))
    }

    async fn complete_selected_credential(
        &self,
        gate: AuthGateRecord,
        credential_ref: CredentialAccountId,
    ) -> Result<AuthGateRecord, ProductWorkflowError> {
        if gate.status() == AuthFlowStatus::Completed {
            return Ok(gate);
        }
        let Some(AuthChallenge::AccountSelectionRequired { .. }) = &gate.flow().challenge else {
            return Ok(gate);
        };
        let completed = self
            .flow_manager
            .complete_credential_selection(
                &gate.flow().scope,
                CredentialSelectionInput {
                    flow_id: gate.flow().id,
                    credential_account_id: credential_ref,
                },
            )
            .await
            .map_err(map_credential_selection_error)?;
        AuthGateRecord::new(gate.run_id(), gate.gate_ref().clone(), completed)
    }

    async fn rollback_auth_cancellation(
        &self,
        gate: &AuthGateRecord,
        expected_claimed_at: ironclaw_auth::Timestamp,
    ) -> Result<(), ProductWorkflowError> {
        self.flow_manager
            .rollback_cancellation(&gate.flow().scope, gate.flow().id, expected_claimed_at)
            .await
            .map(|_| ())
            .map_err(map_auth_product_error)
    }

    async fn cancel_denied_auth(
        &self,
        request: ResolveAuthInteractionRequest,
        gate: AuthGateRecord,
        run_id: TurnRunId,
    ) -> Result<ResolveAuthInteractionResponse, ProductWorkflowError> {
        let reservation = self
            .flow_manager
            .reserve_cancellation(&gate.flow().scope, gate.flow().id, Utc::now())
            .await
            .map_err(map_auth_product_error)?;
        match reservation.status {
            AuthFlowStatus::Canceling => {}
            AuthFlowStatus::Completed => {
                return self.resume_auth_gate(request, run_id, None).await;
            }
            AuthFlowStatus::CallbackReceived | AuthFlowStatus::Completing => {
                return Err(auth_rejected(AuthInteractionRejectionKind::StaleAuth));
            }
            AuthFlowStatus::Pending
            | AuthFlowStatus::AwaitingUser
            | AuthFlowStatus::Failed
            | AuthFlowStatus::Expired
            | AuthFlowStatus::Canceled => {
                return Err(auth_rejected(AuthInteractionRejectionKind::FlowUnavailable));
            }
        }
        let gate_state = match self.turn_gate_state(&request, run_id).await {
            Ok(state) => state,
            Err(error) => {
                if let Err(rollback_error) = self
                    .rollback_auth_cancellation(&gate, reservation.updated_at)
                    .await
                {
                    tracing::debug!(
                        flow_id = %gate.flow().id,
                        run_id = %run_id,
                        error = ?rollback_error,
                        "auth denial reservation rollback failed after run-state lookup"
                    );
                    return Err(rollback_error);
                }
                return Err(error);
            }
        };
        match gate_state {
            BlockedGateState::ParkedOnGate => {}
            BlockedGateState::NotParkedOnGate => {
                self.rollback_auth_cancellation(&gate, reservation.updated_at)
                    .await?;
                return Err(auth_rejected(AuthInteractionRejectionKind::StaleAuth));
            }
        }
        let precondition = CancelRunPrecondition::BlockedAuthGate {
            gate_ref: gate.gate_ref().clone(),
        };
        let response = match self
            .cancel_auth_run(request, run_id, Some(precondition))
            .await
        {
            Ok(response) => response,
            Err(error) => {
                if let Err(rollback_error) = self
                    .rollback_auth_cancellation(&gate, reservation.updated_at)
                    .await
                {
                    tracing::debug!(
                        flow_id = %gate.flow().id,
                        run_id = %run_id,
                        error = ?rollback_error,
                        "auth denial reservation rollback failed after run cancellation"
                    );
                    return Err(rollback_error);
                }
                return Err(error);
            }
        };
        if let Err(error) = self
            .flow_manager
            .finalize_cancellation(&gate.flow().scope, gate.flow().id, reservation.updated_at)
            .await
            .map_err(map_auth_product_error)
        {
            tracing::debug!(
                flow_id = %gate.flow().id,
                run_id = %run_id,
                ?error,
                "auth run was canceled but auth-flow cancellation finalization remains pending"
            );
            return Err(error);
        }
        Ok(response)
    }

    async fn cancel_auth_run(
        &self,
        request: ResolveAuthInteractionRequest,
        run_id: TurnRunId,
        precondition: Option<CancelRunPrecondition>,
    ) -> Result<ResolveAuthInteractionResponse, ProductWorkflowError> {
        let response = self
            .turn_coordinator
            .cancel_run(CancelRunRequest {
                scope: request.scope,
                actor: request.actor,
                run_id,
                precondition,
                reason: SanitizedCancelReason::UserRequested,
                idempotency_key: request.idempotency_key,
            })
            .await
            .map_err(map_auth_resume_error)?;
        Ok(ResolveAuthInteractionResponse::Canceled(response))
    }

    async fn replay_denied_auth(
        &self,
        request: ResolveAuthInteractionRequest,
        gate: &AuthGateRecord,
        run_id: TurnRunId,
    ) -> Result<ResolveAuthInteractionResponse, ProductWorkflowError> {
        let state = self
            .turn_coordinator
            .get_run_state(GetRunStateRequest {
                scope: request.scope.clone(),
                run_id,
            })
            .await
            .map_err(map_auth_resume_error)?;
        if state.status != TurnStatus::Cancelled {
            return Err(auth_rejected(AuthInteractionRejectionKind::StaleAuth));
        }
        match gate.status() {
            AuthFlowStatus::Canceled => {}
            AuthFlowStatus::Canceling => {
                self.flow_manager
                    .finalize_cancellation(
                        &gate.flow().scope,
                        gate.flow().id,
                        gate.flow().updated_at,
                    )
                    .await
                    .map_err(map_auth_product_error)?;
            }
            _ => return Err(auth_rejected(AuthInteractionRejectionKind::StaleAuth)),
        }
        // Route through cancel_run with the SAME idempotency key as the first
        // Deny. TurnCoordinator returns the cached cancellation for a repeated
        // key. A fresh key is still safe because the exact run is already
        // terminally canceled. Never call cancel_run here for a live run.
        self.cancel_auth_run(request, run_id, None).await
    }

    async fn cancel_denied_auth_without_flow(
        &self,
        request: ResolveAuthInteractionRequest,
        run_id: TurnRunId,
    ) -> Result<ResolveAuthInteractionResponse, ProductWorkflowError> {
        match self.turn_gate_state(&request, run_id).await? {
            BlockedGateState::ParkedOnGate => {}
            BlockedGateState::NotParkedOnGate => {
                return Err(auth_rejected(AuthInteractionRejectionKind::MissingAuth));
            }
        }
        let precondition = CancelRunPrecondition::BlockedAuthGate {
            gate_ref: request.gate_ref.clone(),
        };
        self.cancel_auth_run(request, run_id, Some(precondition))
            .await
    }
}

#[async_trait]
impl AuthInteractionService for DefaultAuthInteractionService {
    async fn list_pending(
        &self,
        request: ListPendingAuthInteractionsRequest,
    ) -> Result<ListPendingAuthInteractionsResponse, ProductWorkflowError> {
        let scope = AuthInteractionScope::from_turn(&request.scope, &request.actor);
        let mut auth = self
            .read_model
            .auth_gates(&scope)
            .await?
            .into_iter()
            .filter(|gate| gate.scope() == &scope && is_pending_auth_status(gate.status()))
            .filter_map(|gate| gate.to_view())
            .collect::<Vec<_>>();
        auth.sort_by(|left, right| {
            left.run_id
                .as_uuid()
                .cmp(&right.run_id.as_uuid())
                .then_with(|| {
                    left.auth_request_ref
                        .as_str()
                        .cmp(right.auth_request_ref.as_str())
                })
        });
        Ok(ListPendingAuthInteractionsResponse {
            auth_interactions: auth,
        })
    }

    async fn resolve(
        &self,
        request: ResolveAuthInteractionRequest,
    ) -> Result<ResolveAuthInteractionResponse, ProductWorkflowError> {
        let scope = AuthInteractionScope::from_turn(&request.scope, &request.actor);
        let gate = match self
            .find_gate(&scope, request.run_id_hint, &request.gate_ref)
            .await
        {
            Ok(gate) => gate,
            Err(ProductWorkflowError::AuthInteractionRejected {
                kind: AuthInteractionRejectionKind::MissingAuth,
            }) if matches!(request.decision, AuthInteractionDecision::Deny) => {
                let Some(run_id) = request.run_id_hint else {
                    return Err(auth_rejected(AuthInteractionRejectionKind::MissingAuth));
                };
                return self.cancel_denied_auth_without_flow(request, run_id).await;
            }
            Err(error) => return Err(error),
        };
        let run_id = request.run_id_hint.unwrap_or_else(|| gate.run_id());
        match (
            self.turn_gate_state(&request, run_id).await?,
            request.decision.clone(),
        ) {
            (BlockedGateState::ParkedOnGate, AuthInteractionDecision::Deny) => {
                let gate = self.refresh_gate(&gate).await?;
                self.cancel_denied_auth(request, gate, run_id).await
            }
            (
                BlockedGateState::ParkedOnGate,
                AuthInteractionDecision::CredentialProvided { credential_ref },
            ) => {
                let gate = self.refresh_gate(&gate).await?;
                let gate = self
                    .complete_selected_credential(gate, credential_ref)
                    .await?;
                self.resume_completed_auth(request, gate, run_id).await
            }
            (BlockedGateState::ParkedOnGate, AuthInteractionDecision::CallbackCompleted { .. }) => {
                let gate = self.refresh_gate(&gate).await?;
                self.resume_completed_auth(request, gate, run_id).await
            }
            (
                BlockedGateState::NotParkedOnGate,
                AuthInteractionDecision::CredentialProvided { .. }
                | AuthInteractionDecision::CallbackCompleted { .. },
            ) => {
                let gate = self.refresh_gate(&gate).await?;
                self.resume_completed_auth(request, gate, run_id).await
            }
            (BlockedGateState::NotParkedOnGate, AuthInteractionDecision::Deny) => {
                let gate = self.refresh_gate(&gate).await?;
                self.replay_denied_auth(request, &gate, run_id).await
            }
        }
    }
}

fn map_blocked_gate_state_error(error: BlockedGateStateError) -> ProductWorkflowError {
    match error {
        BlockedGateStateError::Turn(error) => map_gate_state_error(error),
        BlockedGateStateError::ActorMismatch => {
            auth_rejected(AuthInteractionRejectionKind::CrossScopeDenied)
        }
    }
}

enum AuthCompletionRef<'a> {
    CredentialProvided(&'a CredentialAccountId),
    CallbackCompleted(&'a AuthFlowId),
}

fn validate_completion_ref(
    gate: &AuthGateRecord,
    completion: AuthCompletionRef<'_>,
) -> Result<(), ProductWorkflowError> {
    if gate.status() != AuthFlowStatus::Completed {
        return Err(auth_rejected(AuthInteractionRejectionKind::StaleAuth));
    }
    match completion {
        AuthCompletionRef::CredentialProvided(credential_ref) => {
            let Some(account_id) = gate.flow().credential_account_id else {
                return Err(auth_rejected(AuthInteractionRejectionKind::StaleAuth));
            };
            if credential_ref != &account_id {
                return Err(auth_rejected(
                    AuthInteractionRejectionKind::InvalidCredentialRef,
                ));
            }
            Ok(())
        }
        AuthCompletionRef::CallbackCompleted(callback_ref) => {
            if callback_ref != &gate.flow().id {
                return Err(auth_rejected(
                    AuthInteractionRejectionKind::InvalidCallbackRef,
                ));
            }
            Ok(())
        }
    }
}

fn map_auth_product_error(error: AuthProductError) -> ProductWorkflowError {
    match error {
        AuthProductError::UnknownOrExpiredFlow => {
            auth_rejected(AuthInteractionRejectionKind::MissingAuth)
        }
        AuthProductError::CrossScopeDenied => {
            auth_rejected(AuthInteractionRejectionKind::CrossScopeDenied)
        }
        AuthProductError::BackendUnavailable
        | AuthProductError::BackendConflict
        | AuthProductError::MalformedConfig => {
            auth_rejected(AuthInteractionRejectionKind::FlowUnavailable)
        }
        AuthProductError::Canceled
        | AuthProductError::FlowAlreadyTerminal
        | AuthProductError::ProviderDenied
        | AuthProductError::RefreshFailed
        | AuthProductError::InvalidGrant => auth_rejected(AuthInteractionRejectionKind::StaleAuth),
        AuthProductError::MalformedCallback
        | AuthProductError::TokenExchangeFailed
        | AuthProductError::CredentialMissing
        | AuthProductError::AccountSelectionRequired
        | AuthProductError::ProviderIdentityAlreadyConnected
        | AuthProductError::InvalidRequest { .. } => {
            auth_rejected(AuthInteractionRejectionKind::UnsupportedResult)
        }
    }
}

fn map_credential_selection_error(error: AuthProductError) -> ProductWorkflowError {
    match error {
        AuthProductError::UnknownOrExpiredFlow => {
            auth_rejected(AuthInteractionRejectionKind::MissingAuth)
        }
        AuthProductError::CrossScopeDenied => {
            auth_rejected(AuthInteractionRejectionKind::CrossScopeDenied)
        }
        AuthProductError::BackendUnavailable
        | AuthProductError::BackendConflict
        | AuthProductError::MalformedConfig => {
            auth_rejected(AuthInteractionRejectionKind::FlowUnavailable)
        }
        AuthProductError::CredentialMissing
        | AuthProductError::AccountSelectionRequired
        | AuthProductError::InvalidRequest { .. } => {
            auth_rejected(AuthInteractionRejectionKind::InvalidCredentialRef)
        }
        AuthProductError::Canceled
        | AuthProductError::FlowAlreadyTerminal
        | AuthProductError::ProviderDenied
        | AuthProductError::RefreshFailed
        | AuthProductError::InvalidGrant => auth_rejected(AuthInteractionRejectionKind::StaleAuth),
        AuthProductError::MalformedCallback
        | AuthProductError::TokenExchangeFailed
        | AuthProductError::ProviderIdentityAlreadyConnected => {
            auth_rejected(AuthInteractionRejectionKind::UnsupportedResult)
        }
    }
}

fn map_gate_state_error(error: TurnError) -> ProductWorkflowError {
    match error.category() {
        TurnErrorCategory::ScopeNotFound => {
            auth_rejected(AuthInteractionRejectionKind::MissingAuth)
        }
        TurnErrorCategory::Unauthorized => {
            auth_rejected(AuthInteractionRejectionKind::CrossScopeDenied)
        }
        TurnErrorCategory::Unavailable => {
            auth_rejected(AuthInteractionRejectionKind::FlowUnavailable)
        }
        _ => ProductWorkflowError::TurnResumeDenied { error },
    }
}

fn map_auth_resume_error(error: TurnError) -> ProductWorkflowError {
    match error.category() {
        TurnErrorCategory::ScopeNotFound => {
            auth_rejected(AuthInteractionRejectionKind::MissingAuth)
        }
        TurnErrorCategory::Unauthorized => {
            auth_rejected(AuthInteractionRejectionKind::CrossScopeDenied)
        }
        TurnErrorCategory::InvalidRequest | TurnErrorCategory::Conflict => {
            auth_rejected(AuthInteractionRejectionKind::StaleAuth)
        }
        TurnErrorCategory::Unavailable => {
            auth_rejected(AuthInteractionRejectionKind::FlowUnavailable)
        }
        _ => ProductWorkflowError::TurnResumeDenied { error },
    }
}
