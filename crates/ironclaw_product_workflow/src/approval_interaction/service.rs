use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_approvals::DenyApproval;
use ironclaw_host_api::{Action, Principal};
use ironclaw_run_state::ApprovalStatus;
use ironclaw_turns::{
    CancelRunRequest, GateRef, GetRunStateRequest, ResumeTurnPrecondition, ResumeTurnRequest,
    SanitizedCancelReason, TurnCoordinator, TurnError, TurnErrorCategory, TurnRunId, TurnStatus,
};

use super::gate_ref::{approval_reply_binding_ref, approval_source_binding_ref};
use super::{
    ApprovalGateRecord, ApprovalInteractionDecision, ApprovalInteractionReadModel,
    ApprovalInteractionRejectionKind, ApprovalInteractionScope, ApprovalLeaseTermsProvider,
    ApprovalResolutionPort, ListPendingApprovalsRequest, ListPendingApprovalsResponse,
    ResolveApprovalInteractionRequest, ResolveApprovalInteractionResponse, approval_rejected,
};
use crate::error::ProductWorkflowError;

/// Approval-only service consumed by product/WebUI surfaces.
#[async_trait]
pub trait ApprovalInteractionService: Send + Sync {
    async fn list_pending(
        &self,
        request: ListPendingApprovalsRequest,
    ) -> Result<ListPendingApprovalsResponse, ProductWorkflowError>;

    async fn resolve(
        &self,
        request: ResolveApprovalInteractionRequest,
    ) -> Result<ResolveApprovalInteractionResponse, ProductWorkflowError>;
}

pub(crate) struct RejectingApprovalInteractionService;

#[async_trait]
impl ApprovalInteractionService for RejectingApprovalInteractionService {
    async fn list_pending(
        &self,
        _request: ListPendingApprovalsRequest,
    ) -> Result<ListPendingApprovalsResponse, ProductWorkflowError> {
        Err(approval_rejected(
            ApprovalInteractionRejectionKind::ResolverUnavailable,
        ))
    }

    async fn resolve(
        &self,
        _request: ResolveApprovalInteractionRequest,
    ) -> Result<ResolveApprovalInteractionResponse, ProductWorkflowError> {
        Err(approval_rejected(
            ApprovalInteractionRejectionKind::ResolverUnavailable,
        ))
    }
}

pub struct DefaultApprovalInteractionService {
    read_model: Arc<dyn ApprovalInteractionReadModel>,
    lease_terms_provider: Arc<dyn ApprovalLeaseTermsProvider>,
    resolver: Arc<dyn ApprovalResolutionPort>,
    turn_coordinator: Arc<dyn TurnCoordinator>,
}

#[derive(Clone, Copy)]
enum ApprovalCapabilityAction {
    Dispatch,
    Spawn,
}

#[derive(Clone, Copy)]
enum ApprovalTurnGateState {
    ParkedOnGate,
    NotParkedOnGate,
}

impl ApprovalCapabilityAction {
    fn from_action(action: &Action) -> Result<Self, ProductWorkflowError> {
        match action {
            Action::Dispatch { .. } => Ok(Self::Dispatch),
            Action::SpawnCapability { .. } => Ok(Self::Spawn),
            _ => Err(approval_rejected(
                ApprovalInteractionRejectionKind::UnsupportedAction,
            )),
        }
    }
}

impl DefaultApprovalInteractionService {
    pub fn new(
        read_model: Arc<dyn ApprovalInteractionReadModel>,
        lease_terms_provider: Arc<dyn ApprovalLeaseTermsProvider>,
        resolver: Arc<dyn ApprovalResolutionPort>,
        turn_coordinator: Arc<dyn TurnCoordinator>,
    ) -> Self {
        Self {
            read_model,
            lease_terms_provider,
            resolver,
            turn_coordinator,
        }
    }

    async fn find_gate(
        &self,
        scope: &ApprovalInteractionScope,
        run_id_hint: Option<TurnRunId>,
        gate_ref: &GateRef,
    ) -> Result<ApprovalGateRecord, ProductWorkflowError> {
        self.read_model
            .approval_gate(scope, run_id_hint, gate_ref)
            .await?
            .ok_or_else(|| approval_rejected(ApprovalInteractionRejectionKind::MissingGate))
    }

    async fn turn_gate_state(
        &self,
        request: &ResolveApprovalInteractionRequest,
        run_id: TurnRunId,
    ) -> Result<ApprovalTurnGateState, ProductWorkflowError> {
        let state = self
            .turn_coordinator
            .get_run_state(GetRunStateRequest {
                scope: request.scope.clone(),
                run_id,
            })
            .await
            .map_err(map_gate_state_error)?;
        if state.actor.as_ref() != Some(&request.actor) {
            return Err(approval_rejected(
                ApprovalInteractionRejectionKind::CrossScopeDenied,
            ));
        }
        if state.status != TurnStatus::BlockedApproval
            || state.gate_ref.as_ref() != Some(&request.gate_ref)
        {
            return Ok(ApprovalTurnGateState::NotParkedOnGate);
        }
        Ok(ApprovalTurnGateState::ParkedOnGate)
    }

    async fn approve_gate(
        &self,
        request: ResolveApprovalInteractionRequest,
        gate: ApprovalGateRecord,
        run_id: TurnRunId,
    ) -> Result<ResolveApprovalInteractionResponse, ProductWorkflowError> {
        let action = ApprovalCapabilityAction::from_action(gate.request().action.as_ref())?;
        let status = gate.status();
        if matches!(status, ApprovalStatus::Denied | ApprovalStatus::Expired) {
            return Err(approval_rejected(
                ApprovalInteractionRejectionKind::StaleGate,
            ));
        }
        let mut terms = self.lease_terms_provider.lease_terms_for(&gate).await?;
        terms.issued_by = Principal::User(request.actor.user_id.clone());
        match (status, action) {
            (ApprovalStatus::Pending, ApprovalCapabilityAction::Dispatch) => {
                self.resolver
                    .approve_dispatch(gate.resource_scope(), gate.request().id, terms)
                    .await?;
            }
            (ApprovalStatus::Pending, ApprovalCapabilityAction::Spawn) => {
                self.resolver
                    .approve_spawn(gate.resource_scope(), gate.request().id, terms)
                    .await?;
            }
            (ApprovalStatus::Approved, ApprovalCapabilityAction::Dispatch) => {
                self.resolver
                    .ensure_dispatch_lease(gate.resource_scope(), gate.request().id, terms)
                    .await?;
            }
            (ApprovalStatus::Approved, ApprovalCapabilityAction::Spawn) => {
                self.resolver
                    .ensure_spawn_lease(gate.resource_scope(), gate.request().id, terms)
                    .await?;
            }
            (ApprovalStatus::Denied | ApprovalStatus::Expired, _) => {
                return Err(approval_rejected(
                    ApprovalInteractionRejectionKind::StaleGate,
                ));
            }
        }

        let response = self
            .turn_coordinator
            .resume_turn(ResumeTurnRequest {
                scope: request.scope,
                actor: request.actor,
                run_id,
                gate_resolution_ref: request.gate_ref.clone(),
                precondition: ResumeTurnPrecondition::BlockedApprovalGate,
                source_binding_ref: approval_source_binding_ref(&request.gate_ref)?,
                reply_target_binding_ref: approval_reply_binding_ref(&request.gate_ref)?,
                idempotency_key: request.idempotency_key,
            })
            .await
            .map_err(map_approval_resume_error)?;
        Ok(ResolveApprovalInteractionResponse::Approved(response))
    }

    async fn deny_gate(
        &self,
        request: ResolveApprovalInteractionRequest,
        gate: ApprovalGateRecord,
        run_id: TurnRunId,
    ) -> Result<ResolveApprovalInteractionResponse, ProductWorkflowError> {
        match gate.status() {
            ApprovalStatus::Pending => {
                self.resolver
                    .deny(
                        gate.resource_scope(),
                        gate.request().id,
                        DenyApproval {
                            denied_by: Principal::User(request.actor.user_id.clone()),
                        },
                    )
                    .await?;
            }
            ApprovalStatus::Denied => {}
            ApprovalStatus::Approved | ApprovalStatus::Expired => {
                return Err(approval_rejected(
                    ApprovalInteractionRejectionKind::StaleGate,
                ));
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
            .map_err(map_approval_resume_error)?;
        Ok(ResolveApprovalInteractionResponse::Denied(response))
    }

    async fn replay_approved_gate(
        &self,
        request: ResolveApprovalInteractionRequest,
        run_id: TurnRunId,
    ) -> Result<ResolveApprovalInteractionResponse, ProductWorkflowError> {
        let response = self
            .turn_coordinator
            .resume_turn(ResumeTurnRequest {
                scope: request.scope,
                actor: request.actor,
                run_id,
                gate_resolution_ref: request.gate_ref.clone(),
                precondition: ResumeTurnPrecondition::BlockedApprovalGate,
                source_binding_ref: approval_source_binding_ref(&request.gate_ref)?,
                reply_target_binding_ref: approval_reply_binding_ref(&request.gate_ref)?,
                idempotency_key: request.idempotency_key,
            })
            .await
            .map_err(map_approval_resume_error)?;
        Ok(ResolveApprovalInteractionResponse::Approved(response))
    }

    async fn replay_denied_gate(
        &self,
        request: ResolveApprovalInteractionRequest,
        run_id: TurnRunId,
    ) -> Result<ResolveApprovalInteractionResponse, ProductWorkflowError> {
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
            .map_err(map_approval_resume_error)?;
        Ok(ResolveApprovalInteractionResponse::Denied(response))
    }
}

#[async_trait]
impl ApprovalInteractionService for DefaultApprovalInteractionService {
    async fn list_pending(
        &self,
        request: ListPendingApprovalsRequest,
    ) -> Result<ListPendingApprovalsResponse, ProductWorkflowError> {
        let scope = ApprovalInteractionScope::from_turn(&request.scope, &request.actor);
        let mut approvals = self
            .read_model
            .approval_gates(&scope)
            .await?
            .into_iter()
            .filter(|gate| gate.scope() == &scope && gate.status() == ApprovalStatus::Pending)
            .map(|gate| gate.to_view())
            .collect::<Vec<_>>();
        approvals.sort_by(|left, right| {
            left.run_id
                .as_uuid()
                .cmp(&right.run_id.as_uuid())
                .then_with(|| left.gate_ref.as_str().cmp(right.gate_ref.as_str()))
        });
        Ok(ListPendingApprovalsResponse { approvals })
    }

    async fn resolve(
        &self,
        request: ResolveApprovalInteractionRequest,
    ) -> Result<ResolveApprovalInteractionResponse, ProductWorkflowError> {
        let scope = ApprovalInteractionScope::from_turn(&request.scope, &request.actor);
        let gate = self
            .find_gate(&scope, request.run_id_hint, &request.gate_ref)
            .await?;
        let run_id = request.run_id_hint.unwrap_or_else(|| gate.run_id());
        match (
            self.turn_gate_state(&request, run_id).await?,
            gate.status(),
            request.decision,
        ) {
            (ApprovalTurnGateState::ParkedOnGate, _, ApprovalInteractionDecision::ApproveOnce) => {
                self.approve_gate(request, gate, run_id).await
            }
            (ApprovalTurnGateState::ParkedOnGate, _, ApprovalInteractionDecision::Deny) => {
                self.deny_gate(request, gate, run_id).await
            }
            (
                ApprovalTurnGateState::NotParkedOnGate,
                ApprovalStatus::Approved,
                ApprovalInteractionDecision::ApproveOnce,
            ) => self.replay_approved_gate(request, run_id).await,
            (
                ApprovalTurnGateState::NotParkedOnGate,
                ApprovalStatus::Denied,
                ApprovalInteractionDecision::Deny,
            ) => self.replay_denied_gate(request, run_id).await,
            _ => Err(approval_rejected(
                ApprovalInteractionRejectionKind::StaleGate,
            )),
        }
    }
}

fn map_gate_state_error(error: TurnError) -> ProductWorkflowError {
    match error.category() {
        TurnErrorCategory::ScopeNotFound => {
            approval_rejected(ApprovalInteractionRejectionKind::MissingGate)
        }
        TurnErrorCategory::Unauthorized => {
            approval_rejected(ApprovalInteractionRejectionKind::CrossScopeDenied)
        }
        TurnErrorCategory::Unavailable => ProductWorkflowError::Transient {
            reason: "approval gate state unavailable".to_string(),
        },
        _ => ProductWorkflowError::TurnResumeDenied { error },
    }
}

fn map_approval_resume_error(error: TurnError) -> ProductWorkflowError {
    match error.category() {
        TurnErrorCategory::ScopeNotFound => {
            approval_rejected(ApprovalInteractionRejectionKind::MissingGate)
        }
        TurnErrorCategory::Unauthorized => {
            approval_rejected(ApprovalInteractionRejectionKind::CrossScopeDenied)
        }
        TurnErrorCategory::InvalidRequest | TurnErrorCategory::Conflict => {
            approval_rejected(ApprovalInteractionRejectionKind::StaleGate)
        }
        TurnErrorCategory::Unavailable => ProductWorkflowError::Transient {
            reason: "approval gate resume unavailable".to_string(),
        },
        _ => ProductWorkflowError::TurnResumeDenied { error },
    }
}
