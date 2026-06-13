use ironclaw_host_api::{
    Action, ApprovalRequest, ApprovalRequestId, CapabilityId, InvocationId, ResourceScope,
};
use ironclaw_product_adapters::ProductWorkflowRejectionKind;
use ironclaw_run_state::ApprovalStatus;
use ironclaw_turns::{
    CancelRunResponse, GateRef, IdempotencyKey, ResumeTurnResponse, TurnActor, TurnRunId, TurnScope,
};
use serde::{Deserialize, Serialize};

use super::{approval_gate_ref, approval_rejected};
use crate::error::ProductWorkflowError;

const FALLBACK_APPROVAL_SUMMARY: &str = "Approval required";

/// Stable reject reasons for product approval interactions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalInteractionRejectionKind {
    MissingGate,
    AmbiguousGate,
    StaleGate,
    CrossScopeDenied,
    InvalidGateRef,
    AlwaysAllowUnsupported,
    UnsupportedAction,
    LeaseTermsUnavailable,
    ResolverUnavailable,
    InvalidBindingRef,
}

impl ApprovalInteractionRejectionKind {
    pub fn sanitized_reason(self) -> &'static str {
        match self {
            Self::MissingGate => "approval gate was not found",
            Self::AmbiguousGate => "multiple approval gates are pending",
            Self::StaleGate => "approval gate is stale",
            Self::CrossScopeDenied => "approval gate is not visible to this caller",
            Self::InvalidGateRef => "approval gate reference is invalid",
            Self::AlwaysAllowUnsupported => "persistent approval is not supported",
            Self::UnsupportedAction => "approval action is not supported",
            Self::LeaseTermsUnavailable => "approval lease terms are unavailable",
            Self::ResolverUnavailable => "approval resolver is unavailable",
            Self::InvalidBindingRef => "approval resume binding is invalid",
        }
    }

    pub fn workflow_rejection_kind(self) -> ProductWorkflowRejectionKind {
        match self {
            Self::MissingGate => ProductWorkflowRejectionKind::ScopeNotFound,
            Self::AmbiguousGate => ProductWorkflowRejectionKind::Ambiguous,
            Self::StaleGate => ProductWorkflowRejectionKind::Conflict,
            Self::CrossScopeDenied => ProductWorkflowRejectionKind::Unauthorized,
            Self::InvalidGateRef
            | Self::AlwaysAllowUnsupported
            | Self::UnsupportedAction
            | Self::InvalidBindingRef => ProductWorkflowRejectionKind::InvalidRequest,
            Self::LeaseTermsUnavailable | Self::ResolverUnavailable => {
                ProductWorkflowRejectionKind::Unavailable
            }
        }
    }

    pub fn status_code(self) -> u16 {
        match self.workflow_rejection_kind() {
            ProductWorkflowRejectionKind::ScopeNotFound => 404,
            ProductWorkflowRejectionKind::Unauthorized => 403,
            ProductWorkflowRejectionKind::Conflict | ProductWorkflowRejectionKind::Ambiguous => 409,
            ProductWorkflowRejectionKind::Unavailable => 503,
            ProductWorkflowRejectionKind::InvalidRequest => 400,
            ProductWorkflowRejectionKind::ThreadBusy
            | ProductWorkflowRejectionKind::AdmissionRejected => 429,
        }
    }

    pub fn retryable(self) -> bool {
        matches!(
            self.workflow_rejection_kind(),
            ProductWorkflowRejectionKind::Unavailable
                | ProductWorkflowRejectionKind::AdmissionRejected
                | ProductWorkflowRejectionKind::ThreadBusy
        )
    }
}

/// Caller-visible scope for approval interactions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalInteractionScope {
    pub tenant_id: ironclaw_host_api::TenantId,
    pub user_id: ironclaw_host_api::UserId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<ironclaw_host_api::AgentId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<ironclaw_host_api::ProjectId>,
    pub thread_id: ironclaw_host_api::ThreadId,
}

impl ApprovalInteractionScope {
    pub fn from_turn(scope: &TurnScope, actor: &TurnActor) -> Self {
        let user_id = scope
            .explicit_owner_user_id()
            .cloned()
            .unwrap_or_else(|| actor.user_id.clone());
        Self {
            tenant_id: scope.tenant_id.clone(),
            user_id,
            agent_id: scope.agent_id.clone(),
            project_id: scope.project_id.clone(),
            thread_id: scope.thread_id.clone(),
        }
    }

    pub fn to_resource_scope(&self) -> ResourceScope {
        ResourceScope {
            tenant_id: self.tenant_id.clone(),
            user_id: self.user_id.clone(),
            agent_id: self.agent_id.clone(),
            project_id: self.project_id.clone(),
            mission_id: None,
            thread_id: Some(self.thread_id.clone()),
            invocation_id: InvocationId::new(),
        }
    }
}

/// Redacted action shape safe for product/UI display.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ApprovalInteractionActionView {
    Dispatch { capability_id: CapabilityId },
    SpawnCapability { capability_id: CapabilityId },
    Other,
}

impl ApprovalInteractionActionView {
    fn from_action(action: &Action) -> Self {
        match action {
            Action::Dispatch { capability, .. } => Self::Dispatch {
                capability_id: capability.clone(),
            },
            Action::SpawnCapability { capability, .. } => Self::SpawnCapability {
                capability_id: capability.clone(),
            },
            _ => Self::Other,
        }
    }
}

/// Product/UI-safe pending approval DTO.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingApprovalInteractionView {
    pub scope: ApprovalInteractionScope,
    pub run_id: TurnRunId,
    pub gate_ref: GateRef,
    pub approval_request_id: ApprovalRequestId,
    pub summary: String,
    pub action: ApprovalInteractionActionView,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApprovalGateRecord {
    scope: ApprovalInteractionScope,
    resource_scope: ResourceScope,
    run_id: TurnRunId,
    gate_ref: GateRef,
    request: ApprovalRequest,
    status: ApprovalStatus,
}

impl ApprovalGateRecord {
    pub fn new(
        resource_scope: ResourceScope,
        run_id: TurnRunId,
        gate_ref: GateRef,
        request: ApprovalRequest,
    ) -> Result<Self, ProductWorkflowError> {
        Self::with_status(
            resource_scope,
            run_id,
            gate_ref,
            request,
            ApprovalStatus::Pending,
        )
    }

    pub fn with_status(
        resource_scope: ResourceScope,
        run_id: TurnRunId,
        gate_ref: GateRef,
        request: ApprovalRequest,
        status: ApprovalStatus,
    ) -> Result<Self, ProductWorkflowError> {
        let scope = ApprovalInteractionScope {
            tenant_id: resource_scope.tenant_id.clone(),
            user_id: resource_scope.user_id.clone(),
            agent_id: resource_scope.agent_id.clone(),
            project_id: resource_scope.project_id.clone(),
            thread_id: resource_scope.thread_id.clone().ok_or_else(|| {
                approval_rejected(ApprovalInteractionRejectionKind::CrossScopeDenied)
            })?,
        };
        let expected_gate = approval_gate_ref(request.id)?;
        if gate_ref != expected_gate {
            return Err(approval_rejected(
                ApprovalInteractionRejectionKind::InvalidGateRef,
            ));
        }
        Ok(Self {
            scope,
            resource_scope,
            run_id,
            gate_ref,
            request,
            status,
        })
    }

    pub fn scope(&self) -> &ApprovalInteractionScope {
        &self.scope
    }

    pub fn resource_scope(&self) -> &ResourceScope {
        &self.resource_scope
    }

    pub fn run_id(&self) -> TurnRunId {
        self.run_id
    }

    pub fn gate_ref(&self) -> &GateRef {
        &self.gate_ref
    }

    pub fn request(&self) -> &ApprovalRequest {
        &self.request
    }

    pub fn status(&self) -> ApprovalStatus {
        self.status
    }

    pub(super) fn to_view(&self) -> PendingApprovalInteractionView {
        PendingApprovalInteractionView {
            scope: self.scope.clone(),
            run_id: self.run_id,
            gate_ref: self.gate_ref.clone(),
            approval_request_id: self.request.id,
            summary: display_safe_summary(),
            action: ApprovalInteractionActionView::from_action(self.request.action.as_ref()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListPendingApprovalsRequest {
    pub scope: TurnScope,
    pub actor: TurnActor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListPendingApprovalsResponse {
    pub approvals: Vec<PendingApprovalInteractionView>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalInteractionDecision {
    ApproveOnce,
    AlwaysAllow,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveApprovalInteractionRequest {
    pub scope: TurnScope,
    pub actor: TurnActor,
    pub run_id_hint: Option<TurnRunId>,
    pub gate_ref: GateRef,
    pub decision: ApprovalInteractionDecision,
    pub idempotency_key: IdempotencyKey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveApprovalInteractionResponse {
    Approved(ResumeTurnResponse),
    Denied(CancelRunResponse),
}

fn display_safe_summary() -> String {
    FALLBACK_APPROVAL_SUMMARY.to_string()
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::{
        Action, AgentId, ApprovalRequest, CapabilityId, CorrelationId, InvocationId, Principal,
        ProjectId, ResourceEstimate, TenantId, ThreadId, UserId,
    };

    use super::*;

    #[test]
    fn approval_interaction_scope_from_turn_uses_scope_owner_over_actor() {
        let actor = TurnActor::new(UserId::new("user:actor").unwrap());
        let owner_user_id = UserId::new("user:subject").unwrap();
        let thread_id = ThreadId::new("thread:shared").unwrap();
        let scope = TurnScope::new_with_owner(
            TenantId::new("tenant:shared").unwrap(),
            Some(AgentId::new("agent:shared").unwrap()),
            Some(ProjectId::new("project:shared").unwrap()),
            thread_id.clone(),
            Some(owner_user_id.clone()),
        );

        let interaction_scope = ApprovalInteractionScope::from_turn(&scope, &actor);

        assert_eq!(interaction_scope.user_id, owner_user_id);
        assert_eq!(interaction_scope.thread_id, thread_id);
        assert_eq!(
            interaction_scope.agent_id.as_ref().map(AgentId::as_str),
            Some("agent:shared")
        );
        assert_eq!(
            interaction_scope.project_id.as_ref().map(ProjectId::as_str),
            Some("project:shared")
        );
    }

    #[test]
    fn approval_gate_record_with_status_rejects_scope_without_thread_id() {
        let request = approval_request();
        let resource_scope =
            ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new())
                .unwrap();
        let gate_ref = approval_gate_ref(request.id).unwrap();

        let error = ApprovalGateRecord::with_status(
            resource_scope,
            TurnRunId::new(),
            gate_ref,
            request,
            ApprovalStatus::Pending,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            ProductWorkflowError::ApprovalInteractionRejected {
                kind: ApprovalInteractionRejectionKind::CrossScopeDenied
            }
        ));
    }

    #[test]
    fn approval_gate_record_with_status_rejects_mismatched_gate_ref() {
        let request = approval_request();
        let mut resource_scope =
            ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new())
                .unwrap();
        resource_scope.thread_id = Some(ThreadId::new("thread-a").unwrap());
        let wrong_gate_ref = GateRef::new("gate:approval-wrong").unwrap();

        let error = ApprovalGateRecord::with_status(
            resource_scope,
            TurnRunId::new(),
            wrong_gate_ref,
            request,
            ApprovalStatus::Pending,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            ProductWorkflowError::ApprovalInteractionRejected {
                kind: ApprovalInteractionRejectionKind::InvalidGateRef
            }
        ));
    }

    fn approval_request() -> ApprovalRequest {
        ApprovalRequest {
            id: ApprovalRequestId::new(),
            correlation_id: CorrelationId::new(),
            requested_by: Principal::User(UserId::new("alice").unwrap()),
            action: Box::new(Action::Dispatch {
                capability: CapabilityId::new("fixture.search").unwrap(),
                estimated_resources: ResourceEstimate::default(),
            }),
            invocation_fingerprint: None,
            reason: "needs approval".to_string(),
            reusable_scope: None,
        }
    }
}
