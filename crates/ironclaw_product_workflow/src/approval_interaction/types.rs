use ironclaw_host_api::{Action, ApprovalRequest, ApprovalRequestId, CapabilityId, ResourceScope};
use ironclaw_product_adapters::ProductWorkflowRejectionKind;
use ironclaw_run_state::ApprovalStatus;
use ironclaw_turns::run_profile::LoopSafeSummary;
use ironclaw_turns::{
    CancelRunResponse, GateRef, IdempotencyKey, ResumeTurnResponse, TurnActor, TurnRunId, TurnScope,
};
use serde::{Deserialize, Serialize};

use super::{approval_gate_ref, approval_rejected};
use crate::error::ProductWorkflowError;

const FALLBACK_APPROVAL_SUMMARY: &str = "Approval required";
const NO_EXPOSURE_SENTINELS: &[&str] = &["raw_prompt_sentinel", "raw_credential_sentinel"];

/// Stable reject reasons for product approval interactions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalInteractionRejectionKind {
    MissingGate,
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
            ProductWorkflowRejectionKind::Conflict => 409,
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
        Self {
            tenant_id: scope.tenant_id.clone(),
            user_id: actor.user_id.clone(),
            agent_id: scope.agent_id.clone(),
            project_id: scope.project_id.clone(),
            thread_id: scope.thread_id.clone(),
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
            summary: display_safe_summary(&self.request.reason),
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

fn display_safe_summary(reason: &str) -> String {
    let summary = reason.trim();
    if summary.is_empty() {
        return FALLBACK_APPROVAL_SUMMARY.to_string();
    }
    let lower = summary.to_ascii_lowercase();
    if NO_EXPOSURE_SENTINELS
        .iter()
        .any(|sentinel| lower.contains(sentinel))
    {
        return FALLBACK_APPROVAL_SUMMARY.to_string();
    }
    LoopSafeSummary::new(summary)
        .map(|safe_summary| safe_summary.to_string())
        .unwrap_or_else(|_| FALLBACK_APPROVAL_SUMMARY.to_string())
}
