use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_auth::{
    AuthChallenge, AuthFlowId, AuthFlowManager, AuthFlowStatus, AuthProductError,
    CredentialAccountId, CredentialSelectionInput,
};
use ironclaw_turns::{
    CancelRunRequest, GateRef, ResumeTurnPrecondition, ResumeTurnRequest, SanitizedCancelReason,
    TurnCoordinator, TurnError, TurnErrorCategory, TurnRunId, TurnStatus,
};

use super::types::is_pending_auth_status;
use super::{
    AuthGateRecord, AuthInteractionDecision, AuthInteractionRejectionKind, AuthInteractionScope,
    ListPendingAuthInteractionsRequest, ListPendingAuthInteractionsResponse,
    ResolveAuthInteractionRequest, ResolveAuthInteractionResponse, auth_rejected,
    auth_reply_binding_ref, auth_source_binding_ref,
};
use crate::binding_ref::binding_ref_segment;
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
        let binding_id = auth_interaction_binding_id(gate.flow().id, &run_id, gate.gate_ref());
        let response = self
            .turn_coordinator
            .resume_turn(ResumeTurnRequest {
                scope: request.scope,
                actor: request.actor,
                run_id,
                gate_resolution_ref: request.gate_ref,
                precondition: ResumeTurnPrecondition::BlockedAuthGate,
                source_binding_ref: auth_source_binding_ref(&binding_id)?,
                reply_target_binding_ref: auth_reply_binding_ref(&binding_id)?,
                idempotency_key: request.idempotency_key,
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

    async fn cancel_auth(
        &self,
        request: ResolveAuthInteractionRequest,
        gate: AuthGateRecord,
        run_id: TurnRunId,
    ) -> Result<ResolveAuthInteractionResponse, ProductWorkflowError> {
        match gate.status() {
            AuthFlowStatus::Pending
            | AuthFlowStatus::AwaitingUser
            | AuthFlowStatus::CallbackReceived
            | AuthFlowStatus::Completing => {
                self.flow_manager
                    .cancel_flow(&gate.flow().scope, gate.flow().id)
                    .await
                    .map_err(map_auth_product_error)?;
            }
            AuthFlowStatus::Failed | AuthFlowStatus::Expired | AuthFlowStatus::Canceled => {}
            AuthFlowStatus::Completed => {
                return Err(auth_rejected(AuthInteractionRejectionKind::StaleAuth));
            }
        }
        let response = self
            .turn_coordinator
            .cancel_run(CancelRunRequest {
                scope: request.scope,
                actor: request.actor,
                run_id,
                reason: SanitizedCancelReason::UserRequested,
                idempotency_key: request.idempotency_key,
            })
            .await
            .map_err(map_auth_resume_error)?;
        Ok(ResolveAuthInteractionResponse::Canceled(response))
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
        let gate = self
            .find_gate(&scope, request.run_id_hint, &request.gate_ref)
            .await?;
        let run_id = request.run_id_hint.unwrap_or_else(|| gate.run_id());
        match (
            self.turn_gate_state(&request, run_id).await?,
            request.decision.clone(),
        ) {
            (BlockedGateState::ParkedOnGate, AuthInteractionDecision::Deny) => {
                let gate = self.refresh_gate(&gate).await?;
                self.cancel_auth(request, gate, run_id).await
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
                if gate.status() != AuthFlowStatus::Canceled {
                    return Err(auth_rejected(AuthInteractionRejectionKind::StaleAuth));
                }
                self.cancel_auth(request, gate, run_id).await
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

fn auth_interaction_binding_id(
    flow_id: ironclaw_auth::AuthFlowId,
    run_id: &TurnRunId,
    gate_ref: &GateRef,
) -> String {
    format!(
        "{}{}{}{}",
        binding_ref_segment("surface", "auth-interaction"),
        binding_ref_segment("flow", &flow_id.to_string()),
        binding_ref_segment("run", &run_id.to_string()),
        binding_ref_segment("gate", gate_ref.as_str())
    )
}

fn map_auth_product_error(error: AuthProductError) -> ProductWorkflowError {
    match error {
        AuthProductError::UnknownOrExpiredFlow => {
            auth_rejected(AuthInteractionRejectionKind::MissingAuth)
        }
        AuthProductError::CrossScopeDenied => {
            auth_rejected(AuthInteractionRejectionKind::CrossScopeDenied)
        }
        AuthProductError::BackendUnavailable | AuthProductError::BackendConflict => {
            auth_rejected(AuthInteractionRejectionKind::FlowUnavailable)
        }
        AuthProductError::Canceled
        | AuthProductError::FlowAlreadyTerminal
        | AuthProductError::ProviderDenied
        | AuthProductError::RefreshFailed => auth_rejected(AuthInteractionRejectionKind::StaleAuth),
        AuthProductError::MalformedCallback
        | AuthProductError::TokenExchangeFailed
        | AuthProductError::CredentialMissing
        | AuthProductError::AccountSelectionRequired
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
        AuthProductError::BackendUnavailable | AuthProductError::BackendConflict => {
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
        | AuthProductError::RefreshFailed => auth_rejected(AuthInteractionRejectionKind::StaleAuth),
        AuthProductError::MalformedCallback | AuthProductError::TokenExchangeFailed => {
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
