//! Product-facing approval interaction boundary.
//!
//! This module owns the click-approval service shape used by WebUI/product
//! surfaces. It deliberately returns redacted DTOs and routes decisions through
//! injected canonical approval + turn coordination ports; it does not keep an
//! ad hoc approval queue or execute capabilities directly.

use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use ironclaw_approvals::{ApprovalResolutionError, ApprovalResolver, DenyApproval, LeaseApproval};
use ironclaw_authorization::CapabilityLeaseStore;
use ironclaw_events::AuditSink;
use ironclaw_host_api::{
    Action, ApprovalRequest, ApprovalRequestId, CapabilityId, InvocationId, Principal,
    ResourceScope,
};
use ironclaw_product_adapters::ProductWorkflowRejectionKind;
use ironclaw_run_state::{
    ApprovalRequestStore, ApprovalStatus, RunStateError, RunStateStore, RunStatus,
};
use ironclaw_turns::run_profile::LoopSafeSummary;
use ironclaw_turns::{
    CancelRunRequest, CancelRunResponse, GateRef, GetRunStateRequest, IdempotencyKey,
    ReplyTargetBindingRef, ResumeTurnPrecondition, ResumeTurnRequest, ResumeTurnResponse,
    SanitizedCancelReason, SourceBindingRef, TurnActor, TurnCoordinator, TurnError,
    TurnErrorCategory, TurnRunId, TurnScope, TurnStatus,
};
use serde::{Deserialize, Serialize};

use crate::binding_ref::{
    DEFAULT_BINDING_REF_RAW_MAX_BYTES, bounded_reply_target_binding_ref, bounded_source_binding_ref,
};
use crate::error::ProductWorkflowError;

const APPROVAL_GATE_PREFIX: &str = "gate:approval-";
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

fn approval_rejected(kind: ApprovalInteractionRejectionKind) -> ProductWorkflowError {
    ProductWorkflowError::ApprovalInteractionRejected { kind }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invocation_fingerprint: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PendingApprovalGateRecord {
    scope: ApprovalInteractionScope,
    resource_scope: ResourceScope,
    run_id: TurnRunId,
    gate_ref: GateRef,
    request: ApprovalRequest,
}

impl PendingApprovalGateRecord {
    pub fn new(
        resource_scope: ResourceScope,
        run_id: TurnRunId,
        gate_ref: GateRef,
        request: ApprovalRequest,
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

    fn to_view(&self) -> PendingApprovalInteractionView {
        PendingApprovalInteractionView {
            scope: self.scope.clone(),
            run_id: self.run_id,
            gate_ref: self.gate_ref.clone(),
            approval_request_id: self.request.id,
            summary: display_safe_summary(&self.request.reason),
            action: ApprovalInteractionActionView::from_action(self.request.action.as_ref()),
            invocation_fingerprint: self
                .request
                .invocation_fingerprint
                .as_ref()
                .map(|fingerprint| fingerprint.as_str().to_string()),
        }
    }
}

#[async_trait]
pub trait ApprovalInteractionReadModel: Send + Sync {
    async fn pending_approvals(
        &self,
        scope: &ApprovalInteractionScope,
    ) -> Result<Vec<PendingApprovalGateRecord>, ProductWorkflowError>;
}

/// Read-model backed directly by canonical run-state and approval records.
pub struct RunStateApprovalInteractionReadModel {
    run_state: Arc<dyn RunStateStore>,
    approval_requests: Arc<dyn ApprovalRequestStore>,
}

impl RunStateApprovalInteractionReadModel {
    pub fn new(
        run_state: Arc<dyn RunStateStore>,
        approval_requests: Arc<dyn ApprovalRequestStore>,
    ) -> Self {
        Self {
            run_state,
            approval_requests,
        }
    }
}

#[async_trait]
impl ApprovalInteractionReadModel for RunStateApprovalInteractionReadModel {
    async fn pending_approvals(
        &self,
        scope: &ApprovalInteractionScope,
    ) -> Result<Vec<PendingApprovalGateRecord>, ProductWorkflowError> {
        let owner_scope = resource_scope_for_interaction(scope);
        let approvals = self
            .approval_requests
            .records_for_scope(&owner_scope)
            .await
            .map_err(map_approval_read_error)?
            .into_iter()
            .filter(|record| record.status == ApprovalStatus::Pending)
            .map(|record| (record.request.id, record))
            .collect::<HashMap<_, _>>();
        if approvals.is_empty() {
            return Ok(Vec::new());
        }

        let mut gates = Vec::new();
        for run in self
            .run_state
            .records_for_scope(&owner_scope)
            .await
            .map_err(map_approval_read_error)?
        {
            if run.status != RunStatus::BlockedApproval {
                continue;
            }
            let Some(request_id) = run.approval_request_id else {
                continue;
            };
            let Some(approval) = approvals.get(&request_id) else {
                continue;
            };
            if approval.scope != run.scope {
                continue;
            }
            gates.push(PendingApprovalGateRecord::new(
                approval.scope.clone(),
                TurnRunId::from_uuid(run.invocation_id.as_uuid()),
                approval_gate_ref(request_id)?,
                approval.request.clone(),
            )?);
        }
        Ok(gates)
    }
}

#[async_trait]
pub trait ApprovalLeaseTermsProvider: Send + Sync {
    async fn lease_terms_for(
        &self,
        gate: &PendingApprovalGateRecord,
    ) -> Result<LeaseApproval, ProductWorkflowError>;
}

#[async_trait]
pub trait ApprovalResolutionPort: Send + Sync {
    async fn approve_dispatch(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
        approval: LeaseApproval,
    ) -> Result<(), ProductWorkflowError>;

    async fn approve_spawn(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
        approval: LeaseApproval,
    ) -> Result<(), ProductWorkflowError>;

    async fn deny(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
        denial: DenyApproval,
    ) -> Result<(), ProductWorkflowError>;
}

pub struct ApprovalResolverPort {
    approvals: Arc<dyn ApprovalRequestStore>,
    leases: Arc<dyn CapabilityLeaseStore>,
    audit_sink: Option<Arc<dyn AuditSink>>,
}

impl ApprovalResolverPort {
    pub fn new(
        approvals: Arc<dyn ApprovalRequestStore>,
        leases: Arc<dyn CapabilityLeaseStore>,
    ) -> Self {
        Self {
            approvals,
            leases,
            audit_sink: None,
        }
    }

    pub fn with_audit_sink(mut self, audit_sink: Arc<dyn AuditSink>) -> Self {
        self.audit_sink = Some(audit_sink);
        self
    }

    fn resolver(&self) -> ApprovalResolver<'_, dyn ApprovalRequestStore, dyn CapabilityLeaseStore> {
        let mut resolver = ApprovalResolver::new(self.approvals.as_ref(), self.leases.as_ref());
        if let Some(audit_sink) = &self.audit_sink {
            resolver = resolver.with_audit_sink(audit_sink.as_ref());
        }
        resolver
    }
}

#[async_trait]
impl ApprovalResolutionPort for ApprovalResolverPort {
    async fn approve_dispatch(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
        approval: LeaseApproval,
    ) -> Result<(), ProductWorkflowError> {
        self.resolver()
            .approve_dispatch(scope, request_id, approval)
            .await
            .map(|_| ())
            .map_err(map_approval_resolution_error)
    }

    async fn approve_spawn(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
        approval: LeaseApproval,
    ) -> Result<(), ProductWorkflowError> {
        self.resolver()
            .approve_spawn(scope, request_id, approval)
            .await
            .map(|_| ())
            .map_err(map_approval_resolution_error)
    }

    async fn deny(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
        denial: DenyApproval,
    ) -> Result<(), ProductWorkflowError> {
        self.resolver()
            .deny(scope, request_id, denial)
            .await
            .map(|_| ())
            .map_err(map_approval_resolution_error)
    }
}

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
    pub run_id: TurnRunId,
    pub gate_ref: GateRef,
    pub decision: ApprovalInteractionDecision,
    pub idempotency_key: IdempotencyKey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveApprovalInteractionResponse {
    Approved(ResumeTurnResponse),
    Denied(CancelRunResponse),
}

pub struct DefaultApprovalInteractionService {
    read_model: Arc<dyn ApprovalInteractionReadModel>,
    lease_terms_provider: Arc<dyn ApprovalLeaseTermsProvider>,
    resolver: Arc<dyn ApprovalResolutionPort>,
    turn_coordinator: Arc<dyn TurnCoordinator>,
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
        run_id: TurnRunId,
        gate_ref: &GateRef,
    ) -> Result<PendingApprovalGateRecord, ProductWorkflowError> {
        let approval_request_id = approval_request_id_from_gate_ref(gate_ref)?;
        self.read_model
            .pending_approvals(scope)
            .await?
            .into_iter()
            .find(|gate| {
                gate.run_id == run_id
                    && gate.gate_ref == *gate_ref
                    && gate.request.id == approval_request_id
                    && gate.scope == *scope
            })
            .ok_or_else(|| approval_rejected(ApprovalInteractionRejectionKind::MissingGate))
    }

    async fn assert_turn_is_parked_on_gate(
        &self,
        request: &ResolveApprovalInteractionRequest,
    ) -> Result<(), ProductWorkflowError> {
        let state = self
            .turn_coordinator
            .get_run_state(GetRunStateRequest {
                scope: request.scope.clone(),
                run_id: request.run_id,
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
            return Err(approval_rejected(
                ApprovalInteractionRejectionKind::StaleGate,
            ));
        }
        Ok(())
    }

    async fn approve_gate(
        &self,
        request: ResolveApprovalInteractionRequest,
        gate: PendingApprovalGateRecord,
    ) -> Result<ResolveApprovalInteractionResponse, ProductWorkflowError> {
        let approve_dispatch = match gate.request.action.as_ref() {
            Action::Dispatch { .. } => true,
            Action::SpawnCapability { .. } => false,
            _ => {
                return Err(approval_rejected(
                    ApprovalInteractionRejectionKind::UnsupportedAction,
                ));
            }
        };
        let mut terms = self.lease_terms_provider.lease_terms_for(&gate).await?;
        terms.issued_by = Principal::User(request.actor.user_id.clone());
        if approve_dispatch {
            self.resolver
                .approve_dispatch(gate.resource_scope(), gate.request.id, terms)
                .await?;
        } else {
            self.resolver
                .approve_spawn(gate.resource_scope(), gate.request.id, terms)
                .await?;
        }

        let response = self
            .turn_coordinator
            .resume_turn(ResumeTurnRequest {
                scope: request.scope,
                actor: request.actor,
                run_id: request.run_id,
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
        gate: PendingApprovalGateRecord,
    ) -> Result<ResolveApprovalInteractionResponse, ProductWorkflowError> {
        self.resolver
            .deny(
                gate.resource_scope(),
                gate.request.id,
                DenyApproval {
                    denied_by: Principal::User(request.actor.user_id.clone()),
                },
            )
            .await?;
        let response = self
            .turn_coordinator
            .cancel_run(CancelRunRequest {
                scope: request.scope,
                actor: request.actor,
                run_id: request.run_id,
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
            .pending_approvals(&scope)
            .await?
            .into_iter()
            .filter(|gate| gate.scope() == &scope)
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
        self.assert_turn_is_parked_on_gate(&request).await?;
        let scope = ApprovalInteractionScope::from_turn(&request.scope, &request.actor);
        let gate = self
            .find_gate(&scope, request.run_id, &request.gate_ref)
            .await?;
        match request.decision {
            ApprovalInteractionDecision::ApproveOnce => self.approve_gate(request, gate).await,
            ApprovalInteractionDecision::Deny => self.deny_gate(request, gate).await,
        }
    }
}

pub fn is_approval_gate_ref(gate_ref: &GateRef) -> bool {
    gate_ref.as_str().starts_with(APPROVAL_GATE_PREFIX)
}

pub fn approval_gate_ref(request_id: ApprovalRequestId) -> Result<GateRef, ProductWorkflowError> {
    GateRef::new(format!("{APPROVAL_GATE_PREFIX}{request_id}"))
        .map_err(|_| approval_rejected(ApprovalInteractionRejectionKind::InvalidGateRef))
}

fn approval_request_id_from_gate_ref(
    gate_ref: &GateRef,
) -> Result<ApprovalRequestId, ProductWorkflowError> {
    let Some(value) = gate_ref.as_str().strip_prefix(APPROVAL_GATE_PREFIX) else {
        return Err(approval_rejected(
            ApprovalInteractionRejectionKind::InvalidGateRef,
        ));
    };
    ApprovalRequestId::parse(value)
        .map_err(|_| approval_rejected(ApprovalInteractionRejectionKind::InvalidGateRef))
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

fn resource_scope_for_interaction(scope: &ApprovalInteractionScope) -> ResourceScope {
    ResourceScope {
        tenant_id: scope.tenant_id.clone(),
        user_id: scope.user_id.clone(),
        agent_id: scope.agent_id.clone(),
        project_id: scope.project_id.clone(),
        mission_id: None,
        thread_id: Some(scope.thread_id.clone()),
        invocation_id: InvocationId::new(),
    }
}

fn approval_source_binding_ref(
    gate_ref: &GateRef,
) -> Result<SourceBindingRef, ProductWorkflowError> {
    bounded_source_binding_ref(
        "approval-src",
        gate_ref.as_str(),
        DEFAULT_BINDING_REF_RAW_MAX_BYTES,
    )
    .map_err(|_| approval_rejected(ApprovalInteractionRejectionKind::InvalidBindingRef))
}

fn approval_reply_binding_ref(
    gate_ref: &GateRef,
) -> Result<ReplyTargetBindingRef, ProductWorkflowError> {
    bounded_reply_target_binding_ref(
        "approval-reply",
        gate_ref.as_str(),
        DEFAULT_BINDING_REF_RAW_MAX_BYTES,
    )
    .map_err(|_| approval_rejected(ApprovalInteractionRejectionKind::InvalidBindingRef))
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

fn map_approval_read_error(_error: RunStateError) -> ProductWorkflowError {
    ProductWorkflowError::Transient {
        reason: "approval read model unavailable".to_string(),
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

fn map_approval_resolution_error(error: ApprovalResolutionError) -> ProductWorkflowError {
    match error {
        ApprovalResolutionError::RunState(RunStateError::UnknownApprovalRequest { .. }) => {
            approval_rejected(ApprovalInteractionRejectionKind::MissingGate)
        }
        ApprovalResolutionError::RunState(RunStateError::ApprovalNotPending { .. })
        | ApprovalResolutionError::NotPending { .. }
        | ApprovalResolutionError::NotApproved { .. } => {
            approval_rejected(ApprovalInteractionRejectionKind::StaleGate)
        }
        ApprovalResolutionError::UnsupportedAction => {
            approval_rejected(ApprovalInteractionRejectionKind::UnsupportedAction)
        }
        ApprovalResolutionError::MissingInvocationFingerprint => {
            approval_rejected(ApprovalInteractionRejectionKind::StaleGate)
        }
        ApprovalResolutionError::RunState(_) | ApprovalResolutionError::Lease(_) => {
            ProductWorkflowError::Transient {
                reason: "approval resolver unavailable".to_string(),
            }
        }
    }
}
