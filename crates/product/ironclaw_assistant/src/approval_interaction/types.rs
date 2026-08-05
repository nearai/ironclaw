use crate::ProductSurfaceRejectionKind;
use ironclaw_approvals::ApprovalStatus;
use ironclaw_host_api::turn::{IdempotencyKey, TurnActor, TurnGateRef, TurnRunId, TurnScope};
use ironclaw_host_api::{
    action::Action,
    approval::ApprovalRequest,
    ids::{ApprovalRequestId, CapabilityId, InvocationId},
    resource::ResourceScope,
};
use ironclaw_turns::ResumeTurnResponse;
use serde::{Deserialize, Serialize};

use super::{approval_gate_ref, approval_rejected};
use crate::error::ProductSurfaceFailure;

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

    pub fn surface_rejection_kind(self) -> ProductSurfaceRejectionKind {
        match self {
            Self::MissingGate => ProductSurfaceRejectionKind::ScopeNotFound,
            Self::AmbiguousGate => ProductSurfaceRejectionKind::Ambiguous,
            Self::StaleGate => ProductSurfaceRejectionKind::Conflict,
            Self::CrossScopeDenied => ProductSurfaceRejectionKind::Unauthorized,
            Self::InvalidGateRef
            | Self::AlwaysAllowUnsupported
            | Self::UnsupportedAction
            | Self::InvalidBindingRef => ProductSurfaceRejectionKind::InvalidRequest,
            Self::LeaseTermsUnavailable | Self::ResolverUnavailable => {
                ProductSurfaceRejectionKind::Unavailable
            }
        }
    }

    pub fn status_code(self) -> u16 {
        match self.surface_rejection_kind() {
            ProductSurfaceRejectionKind::ScopeNotFound => 404,
            ProductSurfaceRejectionKind::Unauthorized => 403,
            ProductSurfaceRejectionKind::Conflict | ProductSurfaceRejectionKind::Ambiguous => 409,
            ProductSurfaceRejectionKind::Unavailable => 503,
            ProductSurfaceRejectionKind::InvalidRequest => 400,
            ProductSurfaceRejectionKind::ThreadBusy
            | ProductSurfaceRejectionKind::AdmissionRejected => 429,
        }
    }

    pub fn retryable(self) -> bool {
        matches!(
            self.surface_rejection_kind(),
            ProductSurfaceRejectionKind::Unavailable
                | ProductSurfaceRejectionKind::AdmissionRejected
                | ProductSurfaceRejectionKind::ThreadBusy
        )
    }
}

/// Caller-visible scope for approval interactions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalInteractionScope {
    pub tenant_id: ironclaw_host_api::ids::TenantId,
    pub user_id: ironclaw_host_api::ids::UserId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<ironclaw_host_api::ids::AgentId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<ironclaw_host_api::ids::ProjectId>,
    pub thread_id: ironclaw_host_api::ids::ThreadId,
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
    pub gate_ref: TurnGateRef,
    pub approval_request_id: ApprovalRequestId,
    pub summary: String,
    pub action: ApprovalInteractionActionView,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApprovalGateRecord {
    scope: ApprovalInteractionScope,
    resource_scope: ResourceScope,
    run_id: TurnRunId,
    gate_ref: TurnGateRef,
    request: ApprovalRequest,
    status: ApprovalStatus,
}

impl ApprovalGateRecord {
    pub fn new(
        resource_scope: ResourceScope,
        run_id: TurnRunId,
        gate_ref: TurnGateRef,
        request: ApprovalRequest,
    ) -> Result<Self, ProductSurfaceFailure> {
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
        gate_ref: TurnGateRef,
        request: ApprovalRequest,
        status: ApprovalStatus,
    ) -> Result<Self, ProductSurfaceFailure> {
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

    pub fn gate_ref(&self) -> &TurnGateRef {
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
    pub gate_ref: TurnGateRef,
    pub decision: ApprovalInteractionDecision,
    pub idempotency_key: IdempotencyKey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveApprovalInteractionResponse {
    Approved(ResumeTurnResponse),
    Resumed(ResumeTurnResponse),
}

fn display_safe_summary() -> String {
    FALLBACK_APPROVAL_SUMMARY.to_string()
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::{
        action::Action,
        approval::ApprovalRequest,
        ids::{
            AgentId, CapabilityId, CorrelationId, InvocationId, ProjectId, TenantId, ThreadId,
            UserId,
        },
        resource::ResourceEstimate,
        scope::Principal,
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

    /// The contracts-side prompt-lookup scope and this crate's interaction
    /// scope must derive the *same* `ResourceScope` from the same turn.
    ///
    /// WS2.5 moved the approval-prompt lookup scope into
    /// `ironclaw_product_contracts::approval_prompt` so the extension host can
    /// read the approval store without reaching up into product. That leaves
    /// two derivations of the same six fields in two crates, and nothing in the
    /// compiler couples them — this pins the equivalence in both directions, so
    /// a field added or re-mapped on either side fails here rather than
    /// silently scoping one reader's store read differently from the other's.
    /// Every field except `invocation_id` (freshly minted on each call, by
    /// design) must agree, and the owner-over-actor precedence must agree too.
    #[test]
    fn approval_prompt_lookup_scope_matches_the_interaction_scope_projection() {
        let actor = TurnActor::new(UserId::new("user:actor").unwrap());
        let owner_user_id = UserId::new("user:subject").unwrap();
        for scope in [
            // Shared/team subject: the explicit owner wins over the actor.
            TurnScope::new_with_owner(
                TenantId::new("tenant:shared").unwrap(),
                Some(AgentId::new("agent:shared").unwrap()),
                Some(ProjectId::new("project:shared").unwrap()),
                ThreadId::new("thread:shared").unwrap(),
                Some(owner_user_id.clone()),
            ),
            // Personal scope with no explicit owner: the actor is the owner,
            // and the optional ids are absent.
            TurnScope::new_with_owner(
                TenantId::new("tenant:personal").unwrap(),
                None,
                None,
                ThreadId::new("thread:personal").unwrap(),
                None,
            ),
        ] {
            let from_product =
                ApprovalInteractionScope::from_turn(&scope, &actor).to_resource_scope();
            let from_contracts =
                ironclaw_product_contracts::approval_prompt::approval_prompt_lookup_scope(
                    &scope,
                    &actor.user_id,
                );

            assert_eq!(from_contracts.tenant_id, from_product.tenant_id);
            assert_eq!(from_contracts.user_id, from_product.user_id);
            assert_eq!(from_contracts.agent_id, from_product.agent_id);
            assert_eq!(from_contracts.project_id, from_product.project_id);
            assert_eq!(from_contracts.mission_id, from_product.mission_id);
            assert_eq!(from_contracts.thread_id, from_product.thread_id);
        }
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
            ProductSurfaceFailure::ApprovalInteractionRejected {
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
        let wrong_gate_ref = TurnGateRef::new("gate:approval-wrong").unwrap();

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
            ProductSurfaceFailure::ApprovalInteractionRejected {
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
