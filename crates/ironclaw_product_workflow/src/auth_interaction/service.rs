use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_auth::{
    AuthChallenge, AuthFlowManager, AuthFlowOutcome, AuthFlowRecord, AuthFlowState,
    AuthProductError, AuthResolved, CredentialAccountId, CredentialSelectionInput,
};
use ironclaw_turns::{GateRef, TurnCoordinator, TurnRunId};

use super::types::is_pending_auth_state;
use super::{
    AuthGateRecord, AuthInteractionDecision, AuthInteractionRejectionKind, AuthInteractionScope,
    ListPendingAuthInteractionsRequest, ListPendingAuthInteractionsResponse,
    ResolveAuthInteractionRequest, ResolveAuthInteractionResponse, auth_rejected,
};
use crate::auth_continuation::{
    AuthResolutionDispatchOutcome, ProductAuthTurnGateResumeDispatcher,
};
use crate::error::ProductWorkflowError;

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
    resolution_dispatcher: ProductAuthTurnGateResumeDispatcher,
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
            resolution_dispatcher: ProductAuthTurnGateResumeDispatcher::new(turn_coordinator),
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

    async fn complete_selected_credential(
        &self,
        gate: AuthGateRecord,
        credential_ref: CredentialAccountId,
    ) -> Result<AuthGateRecord, ProductWorkflowError> {
        if matches!(
            gate.state(),
            AuthFlowState::Resolved(AuthFlowOutcome::Authorized { .. })
        ) {
            return Ok(gate);
        }
        let Some(AuthChallenge::AccountSelectionRequired { .. }) = &gate.flow().challenge else {
            return self.refresh_gate(&gate).await;
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

    async fn resolve_deny_winner(
        &self,
        gate: &AuthGateRecord,
    ) -> Result<AuthFlowRecord, ProductWorkflowError> {
        match self
            .flow_manager
            .cancel_flow(&gate.flow().scope, gate.flow().id)
            .await
        {
            Ok(flow) => Ok(flow),
            Err(AuthProductError::Canceled | AuthProductError::FlowAlreadyTerminal) => self
                .flow_manager
                .get_flow(&gate.flow().scope, gate.flow().id)
                .await
                .map_err(map_auth_product_error)?
                .ok_or_else(|| auth_rejected(AuthInteractionRejectionKind::MissingAuth)),
            Err(error) => Err(map_auth_product_error(error)),
        }
    }

    async fn dispatch_and_mark(
        &self,
        flow: AuthFlowRecord,
    ) -> Result<ResolveAuthInteractionResponse, ProductWorkflowError> {
        let resolution = auth_resolution_from_flow(&flow)?;
        let outcome = self
            .resolution_dispatcher
            .dispatch_auth_resolved(resolution)
            .await?;
        self.flow_manager
            .mark_resolution_delivered(&flow.scope, flow.id, Utc::now())
            .await
            .map_err(map_auth_product_error)?;
        match outcome {
            AuthResolutionDispatchOutcome::Resumed(response) => {
                Ok(ResolveAuthInteractionResponse::Resumed(response))
            }
            AuthResolutionDispatchOutcome::Canceled(response) => {
                Ok(ResolveAuthInteractionResponse::Canceled(response))
            }
            AuthResolutionDispatchOutcome::Ignored => {
                Err(auth_rejected(AuthInteractionRejectionKind::StaleAuth))
            }
        }
    }
}

#[async_trait]
impl AuthInteractionService for DefaultAuthInteractionService {
    async fn list_pending(
        &self,
        request: ListPendingAuthInteractionsRequest,
    ) -> Result<ListPendingAuthInteractionsResponse, ProductWorkflowError> {
        let scope = AuthInteractionScope::from_turn(&request.scope, &request.actor);
        let gates = self.read_model.auth_gates(&scope).await?;
        // Capture `now` after the read: a slow read with a pre-read timestamp
        // would render gates as unexpired that already expired mid-await.
        let now = chrono::Utc::now();
        let mut auth = gates
            .into_iter()
            .filter(|gate| gate.scope() == &scope && is_pending_auth_state(gate.state()))
            .filter_map(|gate| gate.to_view(now))
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
        let flow = match request.decision {
            AuthInteractionDecision::CredentialProvided { credential_ref } => {
                let gate = self
                    .complete_selected_credential(gate, credential_ref)
                    .await?;
                validate_authorized_account(&gate, credential_ref)?;
                gate.flow().clone()
            }
            AuthInteractionDecision::CallbackCompleted { callback_ref } => {
                if callback_ref != gate.flow().id {
                    return Err(auth_rejected(
                        AuthInteractionRejectionKind::InvalidCallbackRef,
                    ));
                }
                let gate = self.refresh_gate(&gate).await?;
                require_terminal_flow(gate.flow())?;
                gate.flow().clone()
            }
            AuthInteractionDecision::Deny => self.resolve_deny_winner(&gate).await?,
        };
        self.dispatch_and_mark(flow).await
    }
}

fn auth_resolution_from_flow(flow: &AuthFlowRecord) -> Result<AuthResolved, ProductWorkflowError> {
    let AuthFlowState::Resolved(outcome) = flow.state else {
        return Err(auth_rejected(AuthInteractionRejectionKind::StaleAuth));
    };
    Ok(AuthResolved {
        flow_id: flow.id,
        scope: flow.scope.clone(),
        continuation: flow.continuation.clone(),
        provider: flow.provider.clone(),
        outcome,
        resolved_at: flow.updated_at,
    })
}

fn require_terminal_flow(flow: &AuthFlowRecord) -> Result<(), ProductWorkflowError> {
    if matches!(flow.state, AuthFlowState::Resolved(_)) {
        Ok(())
    } else {
        Err(auth_rejected(AuthInteractionRejectionKind::StaleAuth))
    }
}

fn validate_authorized_account(
    gate: &AuthGateRecord,
    credential_ref: CredentialAccountId,
) -> Result<(), ProductWorkflowError> {
    match gate.state() {
        AuthFlowState::Resolved(AuthFlowOutcome::Authorized { account_id })
            if account_id == credential_ref =>
        {
            Ok(())
        }
        AuthFlowState::Resolved(AuthFlowOutcome::Authorized { .. }) => Err(auth_rejected(
            AuthInteractionRejectionKind::InvalidCredentialRef,
        )),
        _ => Err(auth_rejected(AuthInteractionRejectionKind::StaleAuth)),
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
        AuthProductError::CredentialMissing
        | AuthProductError::AccountSelectionRequired
        | AuthProductError::InvalidRequest { .. } => {
            auth_rejected(AuthInteractionRejectionKind::InvalidCredentialRef)
        }
        error => map_auth_product_error(error),
    }
}
