use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_auth::{
    AuthChallenge, AuthFlowId, AuthFlowManager, AuthFlowStatus, AuthProductError,
    CredentialAccountId, CredentialSelectionInput,
};
use ironclaw_turns::{
    GateRef, GateResumeDisposition, GetRunStateRequest, ResumeTurnPrecondition, ResumeTurnRequest,
    TurnCoordinator, TurnError, TurnErrorCategory, TurnRunId, TurnStatus,
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

    /// Cancel the OAuth flow if it is in an active (non-terminal) status.
    /// Returns `Err(StaleAuth)` if the flow is already `Completed` (caller
    /// should use the resume path instead).  No-ops for already-terminal
    /// statuses (Failed / Expired / Canceled).
    async fn cancel_auth_flow_if_active(
        &self,
        gate: &AuthGateRecord,
    ) -> Result<(), ProductWorkflowError> {
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
                // DELIBERATE: a Deny arriving after the OAuth flow already
                // reached Completed is rejected as StaleAuth.  This is a race
                // (the user clicked Deny just as the OAuth callback landed).
                // The caller (`resume_denied_auth`) short-circuits here and the
                // run proceeds with the credential that was just obtained.
                //
                // Surfacing a friendlier "already connected" message, or
                // cancelling the run to honor the late Deny, was considered and
                // rejected as too complex for the initial implementation.  That
                // remains a possible follow-up; for now the late-Deny path is
                // intentionally a no-op from the run's perspective.
                //
                // The existing test `deny_on_completed_flow_rejects_with_stale_auth`
                // pins this behavior.
                return Err(auth_rejected(AuthInteractionRejectionKind::StaleAuth));
            }
        }
        Ok(())
    }

    async fn resume_denied_auth(
        &self,
        request: ResolveAuthInteractionRequest,
        gate: AuthGateRecord,
        run_id: TurnRunId,
    ) -> Result<ResolveAuthInteractionResponse, ProductWorkflowError> {
        self.cancel_auth_flow_if_active(&gate).await?;
        self.resume_auth_gate(request, run_id, Some(GateResumeDisposition::Denied))
            .await
    }

    async fn replay_denied_auth(
        &self,
        request: ResolveAuthInteractionRequest,
        run_id: TurnRunId,
    ) -> Result<ResolveAuthInteractionResponse, ProductWorkflowError> {
        // Route through resume_turn with the SAME idempotency key as the first
        // Deny.  TurnCoordinator::resume_turn returns the cached
        // ResumeTurnResponse for a repeated key before running the precondition
        // check, so this is idempotent regardless of current run state.  A
        // fresh key on a finished run still errors via the precondition
        // (correctly StaleAuth).
        self.resume_auth_gate(request, run_id, Some(GateResumeDisposition::Denied))
            .await
    }

    async fn resume_denied_auth_without_flow(
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
        self.resume_auth_gate(request, run_id, Some(GateResumeDisposition::Denied))
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
                return self.resume_denied_auth_without_flow(request, run_id).await;
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
                self.resume_denied_auth(request, gate, run_id).await
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
                self.replay_denied_auth(request, run_id).await
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
