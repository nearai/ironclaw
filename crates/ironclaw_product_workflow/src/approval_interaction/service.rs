use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_approvals::{
    DenyApproval, LeaseApproval, PersistentApprovalAction, PersistentApprovalPolicyInput,
    PersistentApprovalPolicyKey, PersistentApprovalPolicyStore, ToolPermissionOverrideKey,
    ToolPermissionOverrideStore,
};
use ironclaw_host_api::{Action, CapabilityId, Principal, ResourceScope};
use ironclaw_run_state::ApprovalStatus;
use ironclaw_turns::{
    GateRef, GateResumeDisposition, ResumeTurnPrecondition, ResumeTurnRequest, TurnCoordinator,
    TurnError, TurnErrorCategory, TurnRunId, TurnStatus,
};

use super::gate_ref::{approval_reply_binding_ref, approval_source_binding_ref};
use super::{
    ApprovalGateRecord, ApprovalInteractionDecision, ApprovalInteractionReadModel,
    ApprovalInteractionRejectionKind, ApprovalInteractionScope, ApprovalLeaseTermsProvider,
    ApprovalResolutionPort, ListPendingApprovalsRequest, ListPendingApprovalsResponse,
    ResolveApprovalInteractionRequest, ResolveApprovalInteractionResponse, approval_rejected,
};
use crate::error::ProductWorkflowError;
use crate::gate_state::{BlockedGateState, BlockedGateStateError, blocked_gate_state};

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
    // arch-exempt: optional_arc, absence is the explicit fail-closed
    // AlwaysAllowUnsupported path for minimal/test compositions until user-facing
    // revoke controls land, plan #4539
    persistent_policies: Option<Arc<dyn PersistentApprovalPolicyStore>>,
    persistent_grantee_resolver: Option<Arc<dyn PersistentApprovalGranteeResolver>>,
    tool_permission_overrides: Option<Arc<dyn ToolPermissionOverrideStore>>,
    turn_coordinator: Arc<dyn TurnCoordinator>,
}

pub trait PersistentApprovalGranteeResolver: Send + Sync {
    fn persistent_approval_grantee(&self, capability_id: &CapabilityId) -> Option<Principal>;
}

#[derive(Clone, Copy)]
enum ApprovalCapabilityAction {
    Dispatch,
    Spawn,
}

struct PreparedAllowPolicy {
    input: PersistentApprovalPolicyInput,
    key: PersistentApprovalPolicyKey,
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
            persistent_policies: None,
            persistent_grantee_resolver: None,
            tool_permission_overrides: None,
            turn_coordinator,
        }
    }

    pub fn with_persistent_policy_store(
        mut self,
        persistent_policies: Arc<dyn PersistentApprovalPolicyStore>,
    ) -> Self {
        self.persistent_policies = Some(persistent_policies);
        self
    }

    pub fn with_persistent_grantee_resolver(
        mut self,
        persistent_grantee_resolver: Arc<dyn PersistentApprovalGranteeResolver>,
    ) -> Self {
        self.persistent_grantee_resolver = Some(persistent_grantee_resolver);
        self
    }

    pub fn with_tool_permission_override_store(
        mut self,
        tool_permission_overrides: Arc<dyn ToolPermissionOverrideStore>,
    ) -> Self {
        self.tool_permission_overrides = Some(tool_permission_overrides);
        self
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
    ) -> Result<BlockedGateState, ProductWorkflowError> {
        blocked_gate_state(
            self.turn_coordinator.as_ref(),
            &request.scope,
            &request.actor,
            run_id,
            &request.gate_ref,
            TurnStatus::BlockedApproval,
        )
        .await
        .map_err(map_blocked_gate_state_error)
    }

    async fn approve_gate(
        &self,
        request: ResolveApprovalInteractionRequest,
        gate: ApprovalGateRecord,
        run_id: TurnRunId,
        persistent: bool,
    ) -> Result<ResolveApprovalInteractionResponse, ProductWorkflowError> {
        let action = ApprovalCapabilityAction::from_action(gate.request().action.as_ref())?;
        let status = gate.status();
        if matches!(
            status,
            ApprovalStatus::Denied | ApprovalStatus::Expired | ApprovalStatus::Discarded
        ) {
            return Err(approval_rejected(
                ApprovalInteractionRejectionKind::StaleGate,
            ));
        }
        if persistent && status != ApprovalStatus::Pending {
            return Err(approval_rejected(
                ApprovalInteractionRejectionKind::AlwaysAllowUnsupported,
            ));
        }
        let mut terms = self.lease_terms_provider.lease_terms_for(&gate).await?;
        terms.issued_by = Principal::User(request.actor.user_id.clone());
        let persistent_policy = if persistent && status == ApprovalStatus::Pending {
            self.lease_terms_provider
                .persistent_approval_allowed(&gate)
                .await?;
            Some(self.prepare_allow_policy(&request, &gate, terms.clone())?)
        } else {
            None
        };
        let resolution = match (status, action) {
            (ApprovalStatus::Pending, ApprovalCapabilityAction::Dispatch) => {
                self.resolver
                    .approve_dispatch(gate.resource_scope(), gate.request().id, terms)
                    .await
            }
            (ApprovalStatus::Pending, ApprovalCapabilityAction::Spawn) => {
                self.resolver
                    .approve_spawn(gate.resource_scope(), gate.request().id, terms)
                    .await
            }
            (ApprovalStatus::Approved, ApprovalCapabilityAction::Dispatch) => {
                self.resolver
                    .ensure_dispatch_lease(gate.resource_scope(), gate.request().id, terms)
                    .await
            }
            (ApprovalStatus::Approved, ApprovalCapabilityAction::Spawn) => {
                self.resolver
                    .ensure_spawn_lease(gate.resource_scope(), gate.request().id, terms)
                    .await
            }
            (ApprovalStatus::Denied | ApprovalStatus::Expired | ApprovalStatus::Discarded, _) => {
                return Err(approval_rejected(
                    ApprovalInteractionRejectionKind::StaleGate,
                ));
            }
        };
        resolution?;

        let response = match self
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
                resume_disposition: None,
            })
            .await
            .map_err(map_approval_resume_error)
        {
            Ok(response) => response,
            Err(error) => return Err(error),
        };
        if let Some(policy) = persistent_policy {
            self.persist_allow_policy(policy).await;
        }
        Ok(ResolveApprovalInteractionResponse::Approved(response))
    }

    fn prepare_allow_policy(
        &self,
        request: &ResolveApprovalInteractionRequest,
        gate: &ApprovalGateRecord,
        terms: LeaseApproval,
    ) -> Result<PreparedAllowPolicy, ProductWorkflowError> {
        if self.persistent_policies.is_none() {
            return Err(approval_rejected(
                ApprovalInteractionRejectionKind::AlwaysAllowUnsupported,
            ));
        }
        let Some((action, capability_id)) =
            PersistentApprovalAction::from_action(gate.request().action.as_ref())
        else {
            return Err(approval_rejected(
                ApprovalInteractionRejectionKind::UnsupportedAction,
            ));
        };
        let grantee = self
            .persistent_grantee_resolver
            .as_ref()
            .and_then(|resolver| resolver.persistent_approval_grantee(&capability_id))
            .unwrap_or_else(|| gate.request().requested_by.clone());
        let input = PersistentApprovalPolicyInput {
            scope: persistent_approval_settings_scope(gate.resource_scope()),
            action,
            capability_id,
            grantee,
            approved_by: Principal::User(request.actor.user_id.clone()),
            constraints: ironclaw_host_api::GrantConstraints {
                max_invocations: None,
                ..terms.constraints
            },
            source_approval_request_id: Some(gate.request().id),
        };
        let key = PersistentApprovalPolicyKey::new(
            &input.scope,
            input.action,
            input.capability_id.clone(),
            input.grantee.clone(),
        );
        Ok(PreparedAllowPolicy { input, key })
    }

    async fn persist_allow_policy(&self, policy: PreparedAllowPolicy) {
        let Some(persistent_policies) = self.persistent_policies.as_ref() else {
            return;
        };
        let override_key =
            ToolPermissionOverrideKey::new(&policy.input.scope, policy.input.capability_id.clone());
        if let Err(error) = persistent_policies.allow(policy.input).await {
            // silent-ok: turn already resumed; next invocation re-prompts if lookup misses.
            tracing::warn!(
                error = %error,
                capability_id = %policy.key.capability_id,
                action = ?policy.key.action,
                "persistent approval policy write failed after approval resolution"
            );
            return;
        }
        let Some(tool_permission_overrides) = self.tool_permission_overrides.as_ref() else {
            return;
        };
        if let Err(error) = tool_permission_overrides.clear(&override_key).await {
            // silent-ok: the durable allow policy was written; if the old ask/deny
            // override remains, the next invocation safely prompts again.
            tracing::warn!(
                error = %error,
                capability_id = %override_key.capability_id,
                "tool permission override clear failed after persistent approval"
            );
        }
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
            ApprovalStatus::Approved | ApprovalStatus::Expired | ApprovalStatus::Discarded => {
                return Err(approval_rejected(
                    ApprovalInteractionRejectionKind::StaleGate,
                ));
            }
        }
        // The denial side effect is local to the first-time path; the resume +
        // response mapping is shared with `replay_denied_gate`.
        self.resume_denied(request, run_id).await
    }

    /// Resume a run whose approval gate was denied, carrying
    /// `GateResumeDisposition::Denied`. Shared by the first-time deny
    /// (`deny_gate`, after the durable `resolver.deny`) and the idempotent
    /// replay (`replay_denied_gate`). `TurnCoordinator::resume_turn` replays the
    /// cached response for a repeated idempotency key, so both callers are safe
    /// regardless of current run state.
    async fn resume_denied(
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
                resume_disposition: Some(GateResumeDisposition::Denied),
            })
            .await
            .map_err(map_approval_resume_error)?;
        Ok(ResolveApprovalInteractionResponse::Resumed(response))
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
                resume_disposition: None,
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
        // Idempotent replay: the SAME idempotency key as the first Deny.
        // TurnCoordinator::resume_turn returns the cached ResumeTurnResponse for
        // a repeated key before running the precondition check, so this is
        // idempotent regardless of current run state. A fresh key on a finished
        // run still errors via the precondition (correctly StaleGate).
        self.resume_denied(request, run_id).await
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
            (BlockedGateState::ParkedOnGate, _, ApprovalInteractionDecision::ApproveOnce) => {
                self.approve_gate(request, gate, run_id, false).await
            }
            (BlockedGateState::ParkedOnGate, _, ApprovalInteractionDecision::AlwaysAllow) => {
                self.approve_gate(request, gate, run_id, true).await
            }
            (BlockedGateState::ParkedOnGate, _, ApprovalInteractionDecision::Deny) => {
                self.deny_gate(request, gate, run_id).await
            }
            (
                BlockedGateState::NotParkedOnGate,
                ApprovalStatus::Approved,
                ApprovalInteractionDecision::ApproveOnce,
            ) => self.replay_approved_gate(request, run_id).await,
            (
                BlockedGateState::NotParkedOnGate,
                ApprovalStatus::Approved,
                ApprovalInteractionDecision::AlwaysAllow,
            ) => Err(approval_rejected(
                ApprovalInteractionRejectionKind::AlwaysAllowUnsupported,
            )),
            (
                BlockedGateState::NotParkedOnGate,
                ApprovalStatus::Denied,
                ApprovalInteractionDecision::Deny,
            ) => self.replay_denied_gate(request, run_id).await,
            _ => Err(approval_rejected(
                ApprovalInteractionRejectionKind::StaleGate,
            )),
        }
    }
}

fn map_blocked_gate_state_error(error: BlockedGateStateError) -> ProductWorkflowError {
    match error {
        BlockedGateStateError::Turn(error) => map_gate_state_error(error),
        BlockedGateStateError::ActorMismatch => {
            approval_rejected(ApprovalInteractionRejectionKind::CrossScopeDenied)
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

fn persistent_approval_settings_scope(scope: &ResourceScope) -> ResourceScope {
    scope.tenant_user_settings_scope()
}

#[cfg(test)]
mod tests {
    use ironclaw_turns::TurnCapacityResource;

    use super::*;

    #[test]
    fn map_gate_state_error_covers_turn_error_categories() {
        assert!(matches!(
            map_gate_state_error(TurnError::ScopeNotFound),
            ProductWorkflowError::ApprovalInteractionRejected {
                kind: ApprovalInteractionRejectionKind::MissingGate
            }
        ));
        assert!(matches!(
            map_gate_state_error(TurnError::Unauthorized),
            ProductWorkflowError::ApprovalInteractionRejected {
                kind: ApprovalInteractionRejectionKind::CrossScopeDenied
            }
        ));
        assert!(matches!(
            map_gate_state_error(TurnError::Unavailable {
                reason: "store down".to_string()
            }),
            ProductWorkflowError::Transient { .. }
        ));
        assert!(matches!(
            map_gate_state_error(TurnError::InvalidRequest {
                reason: "bad state".to_string()
            }),
            ProductWorkflowError::TurnResumeDenied { .. }
        ));
        assert!(matches!(
            map_gate_state_error(TurnError::capacity_exceeded(
                TurnCapacityResource::SubmitTurn,
                1
            )),
            ProductWorkflowError::TurnResumeDenied { .. }
        ));
    }

    #[test]
    fn map_approval_resume_error_covers_turn_error_categories() {
        assert!(matches!(
            map_approval_resume_error(TurnError::ScopeNotFound),
            ProductWorkflowError::ApprovalInteractionRejected {
                kind: ApprovalInteractionRejectionKind::MissingGate
            }
        ));
        assert!(matches!(
            map_approval_resume_error(TurnError::Unauthorized),
            ProductWorkflowError::ApprovalInteractionRejected {
                kind: ApprovalInteractionRejectionKind::CrossScopeDenied
            }
        ));
        assert!(matches!(
            map_approval_resume_error(TurnError::InvalidRequest {
                reason: "stale".to_string()
            }),
            ProductWorkflowError::ApprovalInteractionRejected {
                kind: ApprovalInteractionRejectionKind::StaleGate
            }
        ));
        assert!(matches!(
            map_approval_resume_error(TurnError::Conflict {
                reason: "stale".to_string()
            }),
            ProductWorkflowError::ApprovalInteractionRejected {
                kind: ApprovalInteractionRejectionKind::StaleGate
            }
        ));
        assert!(matches!(
            map_approval_resume_error(TurnError::Unavailable {
                reason: "store down".to_string()
            }),
            ProductWorkflowError::Transient { .. }
        ));
        assert!(matches!(
            map_approval_resume_error(TurnError::capacity_exceeded(
                TurnCapacityResource::SubmitTurn,
                1
            )),
            ProductWorkflowError::TurnResumeDenied { .. }
        ));
    }
}
