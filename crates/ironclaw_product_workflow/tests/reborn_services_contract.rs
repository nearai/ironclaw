//! Contract tests for WebUI-facing RebornServices facade.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_auth::{CredentialAccountId, CredentialAccountProjection};
use ironclaw_host_api::{AgentId, ApprovalRequestId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_product_adapters::{
    ProductAdapterError, ProductOutboundEnvelope, ProductWorkflowRejectionKind, ProjectionCursor,
    ProjectionStream, ProjectionSubscriptionRequest, ProtocolAuthFailure, RedactedString,
};
use ironclaw_product_workflow::{
    AUTOMATION_LIST_DEFAULT_PAGE_SIZE, AUTOMATION_LIST_MAX_PAGE_SIZE,
    AUTOMATION_RUN_HISTORY_DEFAULT_PAGE_SIZE, AUTOMATION_RUN_HISTORY_MAX_PAGE_SIZE,
    AUTOMATION_TRIGGER_THREAD_SOURCE_TAG, ApprovalInteractionDecision, ApprovalInteractionService,
    AuthInteractionDecision, AuthInteractionService, AutomationListRequest,
    AutomationProductFacade, ExtensionCredentialSetupService, ExtensionCredentialStatusRequest,
    ExtensionCredentialSubmitRequest, LifecycleExtensionCredentialRequirement,
    LifecycleExtensionCredentialSetup, LifecycleExtensionOnboarding, LifecycleExtensionRuntimeKind,
    LifecycleExtensionSource, LifecycleExtensionSummary, LifecycleInstalledExtensionSummary,
    LifecyclePackageKind, LifecyclePackageRef, LifecyclePhase, LifecycleProductAction,
    LifecycleProductContext, LifecycleProductFacade, LifecycleProductPayload,
    LifecycleProductResponse, LifecycleReadinessBlocker, ListPendingApprovalsRequest,
    ListPendingApprovalsResponse, ListPendingAuthInteractionsRequest,
    ListPendingAuthInteractionsResponse, OutboundPreferencesProductFacade, ProductAgentBoundCaller,
    ProductWorkflowError, RebornAutomationInfo, RebornAutomationRecentRunInfo,
    RebornAutomationRecentRunStatus, RebornAutomationRunStatus, RebornAutomationSource,
    RebornAutomationState, RebornChannelConnectAction, RebornChannelConnectStrategy,
    RebornConnectableChannelInfo, RebornDeleteThreadRequest, RebornExtensionOnboardingState,
    RebornGetRunStateRequest, RebornOutboundDeliveryModality,
    RebornOutboundDeliveryTargetCapabilities, RebornOutboundDeliveryTargetDescription,
    RebornOutboundDeliveryTargetId, RebornOutboundDeliveryTargetListResponse,
    RebornOutboundDeliveryTargetOption, RebornOutboundDeliveryTargetStatus,
    RebornOutboundDeliveryTargetSummary, RebornOutboundPreferencesResponse,
    RebornResolveGateResponse, RebornServices, RebornServicesApi, RebornServicesError,
    RebornServicesErrorCode, RebornServicesErrorKind, RebornSetOutboundPreferencesRequest,
    RebornStreamEventsRequest, RebornSubmitTurnResponse, RebornTimelineRequest,
    ResolveApprovalInteractionRequest, ResolveApprovalInteractionResponse,
    ResolveAuthInteractionRequest, ResolveAuthInteractionResponse,
    StaticConnectableChannelsProductFacade, WebUiAuthenticatedCaller, WebUiCancelRunRequest,
    WebUiCreateThreadRequest, WebUiInboundValidationCode, WebUiListAutomationsRequest,
    WebUiListThreadsRequest, WebUiResolveGateRequest, WebUiSendMessageRequest,
    WebUiSetupExtensionRequest, approval_gate_ref, automation_trigger_thread_metadata_json,
};
use ironclaw_threads::{
    AcceptInboundMessageRequest, AcceptedInboundMessage, AcceptedInboundMessageReplay,
    AppendAssistantDraftRequest, AppendCapabilityDisplayPreviewRequest,
    AppendToolResultReferenceRequest, ContextMessages, ContextWindow, CreateSummaryArtifactRequest,
    EnsureThreadRequest, InMemorySessionThreadService, ListThreadsForScopeRequest,
    ListThreadsForScopeResponse, LoadContextMessagesRequest, LoadContextWindowRequest,
    MessageContent, MessageKind, MessageStatus, RedactMessageRequest,
    ReplayAcceptedInboundMessageRequest, SessionThreadError, SessionThreadRecord,
    SessionThreadService, SummaryArtifact, ThreadHistory, ThreadHistoryRequest, ThreadMessageId,
    ThreadMessageRecord, ThreadScope, UpdateAssistantDraftRequest,
    UpdateToolResultReferenceRequest,
};
use ironclaw_turns::{
    AcceptedMessageRef, AdmissionRejection, AdmissionRejectionReason, CancelRunRequest,
    CancelRunResponse, DefaultTurnCoordinator, EventCursor, GateRef, GetRunStateRequest,
    InMemoryTurnStateStore, ReplyTargetBindingRef, ResumeTurnPrecondition, ResumeTurnRequest,
    ResumeTurnResponse, RunProfileId, RunProfileVersion, SourceBindingRef, SubmitTurnRequest,
    SubmitTurnResponse, TurnActor, TurnCapacityResource, TurnCoordinator, TurnError, TurnId,
    TurnRunId, TurnRunState, TurnScope, TurnStatus,
};
use serde_json::json;
use tokio::sync::{Notify, oneshot};

fn caller() -> WebUiAuthenticatedCaller {
    caller_for_user("user-alpha")
}

fn caller_for_user(user_id: &str) -> WebUiAuthenticatedCaller {
    caller_for_user_with_project(user_id, Some("project-alpha"))
}

fn turn_actor_for_user(user_id: &str) -> TurnActor {
    TurnActor::new(UserId::new(user_id).expect("valid user"))
}

fn caller_with_project(project_id: Option<&str>) -> WebUiAuthenticatedCaller {
    caller_for_user_with_project("user-alpha", project_id)
}

fn caller_without_agent() -> WebUiAuthenticatedCaller {
    WebUiAuthenticatedCaller::new(
        TenantId::new("tenant-alpha").expect("valid tenant"),
        UserId::new("user-alpha").expect("valid user"),
        None,
        Some(ProjectId::new("project-alpha").expect("valid project")),
    )
}

fn caller_for_user_with_project(
    user_id: &str,
    project_id: Option<&str>,
) -> WebUiAuthenticatedCaller {
    WebUiAuthenticatedCaller::new(
        TenantId::new("tenant-alpha").expect("valid tenant"),
        UserId::new(user_id).expect("valid user"),
        Some(AgentId::new("agent-alpha").expect("valid agent")),
        project_id.map(|project_id| ProjectId::new(project_id).expect("valid project")),
    )
}

fn run_id_string() -> String {
    "3d54a1f0-0a7f-4b9c-a350-4258f2fa3e18".to_string()
}

fn automation_run_id() -> TurnRunId {
    TurnRunId::parse("11111111-1111-1111-1111-111111111111").expect("valid automation run id")
}

fn fake_thread_history(owner: &WebUiAuthenticatedCaller, thread_id: &str) -> ThreadHistory {
    let thread_id = ThreadId::new(thread_id).expect("valid thread id");
    let scope = ThreadScope {
        tenant_id: owner.tenant_id.clone(),
        agent_id: owner.agent_id.clone().expect("test caller has agent"),
        project_id: owner.project_id.clone(),
        owner_user_id: Some(owner.user_id.clone()),
        mission_id: None,
    };
    ThreadHistory {
        thread: SessionThreadRecord {
            scope: scope.clone(),
            thread_id: thread_id.clone(),
            created_by_actor_id: owner.user_id.as_str().to_string(),
            title: Some("M2 facade contract thread".to_string()),
            metadata_json: None,
            goal: None,
        },
        messages: vec![ThreadMessageRecord {
            message_id: ThreadMessageId::new(),
            thread_id,
            sequence: 1,
            kind: MessageKind::User,
            status: MessageStatus::Submitted,
            actor_id: Some(owner.user_id.as_str().to_string()),
            source_binding_id: Some("webui-src:test".to_string()),
            reply_target_binding_id: Some("webui-reply:test".to_string()),
            turn_id: Some("turn-test".to_string()),
            turn_run_id: Some(run_id_string()),
            tool_result_ref: None,
            tool_result_provider_call: None,
            content: Some("timeline from fake M2 port".to_string()),
            redaction_ref: None,
        }],
        summary_artifacts: vec![],
    }
}

fn thread_scope_for(caller: &WebUiAuthenticatedCaller) -> ThreadScope {
    ThreadScope {
        tenant_id: caller.tenant_id.clone(),
        agent_id: caller.agent_id.clone().expect("agent id"),
        project_id: caller.project_id.clone(),
        owner_user_id: Some(caller.user_id.clone()),
        mission_id: None,
    }
}

fn legacy_webui_source_binding_id_for(
    caller: &WebUiAuthenticatedCaller,
    thread_id: &str,
) -> String {
    format!(
        "{}{}{}{}{}",
        segment("surface", "webui"),
        segment("tenant", caller.tenant_id.as_str()),
        segment(
            "agent",
            caller.agent_id.as_ref().map(AgentId::as_str).unwrap_or("")
        ),
        segment("thread", thread_id),
        segment("actor", caller.user_id.as_str())
    )
}

fn segment(name: &str, value: &str) -> String {
    format!("{name}:{}:{value};", value.len())
}

/// Establish thread ownership for `caller` under `thread_id` so subsequent
/// thread-bound facade calls pass the ownership check.
async fn setup_owned_thread(
    services: &RebornServices,
    owner: WebUiAuthenticatedCaller,
    thread_id: &str,
) {
    create_thread_for(services, owner, thread_id).await;
}

struct FakeTurnCoordinator {
    submissions: Mutex<Vec<SubmitTurnRequest>>,
    cancellations: Mutex<Vec<CancelRunRequest>>,
    resumptions: Mutex<Vec<ResumeTurnRequest>>,
    run_state_requests: Mutex<Vec<GetRunStateRequest>>,
    submit_error: Mutex<Option<TurnError>>,
    run_state_error: Mutex<Option<TurnError>>,
    run_state_actor: Mutex<Option<TurnActor>>,
    explicit_run_status: Mutex<Option<TurnStatus>>,
    parked_gate_ref: Mutex<Option<GateRef>>,
    parked_auth_gate: Mutex<bool>,
    parked_approval_gate: Mutex<bool>,
}

impl Default for FakeTurnCoordinator {
    fn default() -> Self {
        Self {
            submissions: Mutex::default(),
            cancellations: Mutex::default(),
            resumptions: Mutex::default(),
            run_state_requests: Mutex::default(),
            submit_error: Mutex::default(),
            run_state_error: Mutex::default(),
            run_state_actor: Mutex::new(Some(turn_actor_for_user("user-alpha"))),
            explicit_run_status: Mutex::default(),
            parked_gate_ref: Mutex::default(),
            parked_auth_gate: Mutex::default(),
            parked_approval_gate: Mutex::default(),
        }
    }
}

impl FakeTurnCoordinator {
    fn with_submit_error(error: TurnError) -> Self {
        Self {
            submit_error: Mutex::new(Some(error)),
            ..Self::default()
        }
    }

    fn with_run_state_error(error: TurnError) -> Self {
        Self {
            run_state_error: Mutex::new(Some(error)),
            ..Self::default()
        }
    }

    /// Programs `get_run_state` to return this gate as the run's currently
    /// parked gate. Needed by tests that exercise `resolve_gate` denied/
    /// cancelled paths now that `RebornServices` verifies the run is parked
    /// on the supplied gate before issuing cancellation.
    fn set_parked_gate(&self, gate_ref: GateRef) {
        *self.parked_gate_ref.lock().expect("lock") = Some(gate_ref);
        *self.parked_auth_gate.lock().expect("lock") = false;
        *self.parked_approval_gate.lock().expect("lock") = false;
    }

    fn set_parked_auth_gate(&self, gate_ref: GateRef) {
        *self.parked_gate_ref.lock().expect("lock") = Some(gate_ref);
        *self.parked_auth_gate.lock().expect("lock") = true;
        *self.parked_approval_gate.lock().expect("lock") = false;
    }

    fn set_parked_approval_gate(&self, gate_ref: GateRef) {
        *self.parked_gate_ref.lock().expect("lock") = Some(gate_ref);
        *self.parked_auth_gate.lock().expect("lock") = false;
        *self.parked_approval_gate.lock().expect("lock") = true;
    }

    fn set_run_state_actor(&self, actor: Option<TurnActor>) {
        *self.run_state_actor.lock().expect("lock") = actor;
    }

    fn set_run_state_status(&self, status: TurnStatus) {
        *self.explicit_run_status.lock().expect("lock") = Some(status);
    }

    fn submission_count(&self) -> usize {
        self.submissions.lock().expect("lock").len()
    }

    fn cancellation_count(&self) -> usize {
        self.cancellations.lock().expect("lock").len()
    }

    fn resumption_count(&self) -> usize {
        self.resumptions.lock().expect("lock").len()
    }

    fn run_state_request_count(&self) -> usize {
        self.run_state_requests.lock().expect("lock").len()
    }

    fn last_resumption_source_binding_ref(&self) -> Option<String> {
        self.resumptions
            .lock()
            .expect("lock")
            .last()
            .map(|request| request.source_binding_ref.as_str().to_string())
    }

    fn last_resumption_precondition(&self) -> Option<ResumeTurnPrecondition> {
        self.resumptions
            .lock()
            .expect("lock")
            .last()
            .map(|request| request.precondition)
    }

    fn last_submission_scope(&self) -> Option<ironclaw_turns::TurnScope> {
        self.submissions
            .lock()
            .expect("lock")
            .last()
            .map(|request| request.scope.clone())
    }
}

#[async_trait]
impl TurnCoordinator for FakeTurnCoordinator {
    async fn prepare_turn(&self, _scope: TurnScope) -> Result<TurnRunId, TurnError> {
        Ok(TurnRunId::new())
    }

    async fn submit_turn(
        &self,
        request: SubmitTurnRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        if let Some(error) = self.submit_error.lock().expect("lock").take() {
            return Err(error);
        }
        self.submissions.lock().expect("lock").push(request.clone());
        Ok(SubmitTurnResponse::Accepted {
            turn_id: TurnId::new(),
            run_id: TurnRunId::new(),
            status: TurnStatus::Queued,
            resolved_run_profile_id: RunProfileId::default_profile(),
            resolved_run_profile_version: RunProfileVersion::new(1),
            event_cursor: EventCursor(7),
            accepted_message_ref: request.accepted_message_ref,
            reply_target_binding_ref: request.reply_target_binding_ref,
        })
    }

    async fn resume_turn(
        &self,
        request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        self.resumptions.lock().expect("lock").push(request);
        Ok(ResumeTurnResponse {
            run_id: TurnRunId::new(),
            status: TurnStatus::Queued,
            event_cursor: EventCursor(11),
        })
    }

    async fn cancel_run(&self, request: CancelRunRequest) -> Result<CancelRunResponse, TurnError> {
        let run_id = request.run_id;
        self.cancellations.lock().expect("lock").push(request);
        Ok(CancelRunResponse {
            run_id,
            status: TurnStatus::Cancelled,
            event_cursor: EventCursor(13),
            already_terminal: false,
            actor: None,
        })
    }

    async fn get_run_state(&self, request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        if let Some(error) = self.run_state_error.lock().expect("lock").take() {
            return Err(error);
        }
        let actor = self.run_state_actor.lock().expect("lock").clone();
        let gate_ref = self.parked_gate_ref.lock().expect("lock").clone();
        let status = self
            .explicit_run_status
            .lock()
            .expect("lock")
            .unwrap_or_else(|| {
                if *self.parked_auth_gate.lock().expect("lock") {
                    TurnStatus::BlockedAuth
                } else if *self.parked_approval_gate.lock().expect("lock") {
                    TurnStatus::BlockedApproval
                } else {
                    TurnStatus::Queued
                }
            });
        let scope = request.scope.clone();
        let run_id = request.run_id;
        self.run_state_requests.lock().expect("lock").push(request);
        Ok(TurnRunState {
            scope,
            actor,
            turn_id: TurnId::new(),
            run_id,
            status,
            accepted_message_ref: AcceptedMessageRef::new("msg:replayed").expect("valid ref"),
            source_binding_ref: SourceBindingRef::new("webui-src:replayed").expect("valid ref"),
            reply_target_binding_ref: ReplyTargetBindingRef::new("webui-reply:replayed")
                .expect("valid ref"),
            resolved_run_profile_id: RunProfileId::default_profile(),
            resolved_run_profile_version: RunProfileVersion::new(1),
            resolved_model_route: None,
            received_at: Utc::now(),
            checkpoint_id: None,
            gate_ref,
            credential_requirements: Vec::new(),
            failure: None,
            event_cursor: EventCursor(17),
        })
    }
}

struct BlockingSubmitCoordinator {
    submit_entered: AtomicBool,
    submit_released: AtomicBool,
    entered_submit: Notify,
    release_submit: Notify,
    run_id: TurnRunId,
}

impl BlockingSubmitCoordinator {
    fn new() -> Self {
        Self {
            submit_entered: AtomicBool::new(false),
            submit_released: AtomicBool::new(false),
            entered_submit: Notify::new(),
            release_submit: Notify::new(),
            run_id: TurnRunId::new(),
        }
    }

    async fn wait_for_submit(&self) {
        while !self.submit_entered.load(Ordering::Acquire) {
            self.entered_submit.notified().await;
        }
    }

    fn release_submit(&self) {
        self.submit_released.store(true, Ordering::Release);
        self.release_submit.notify_waiters();
    }
}

#[async_trait]
impl TurnCoordinator for BlockingSubmitCoordinator {
    async fn prepare_turn(&self, _scope: TurnScope) -> Result<TurnRunId, TurnError> {
        Ok(TurnRunId::new())
    }

    async fn submit_turn(
        &self,
        request: SubmitTurnRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        self.submit_entered.store(true, Ordering::Release);
        self.entered_submit.notify_waiters();
        while !self.submit_released.load(Ordering::Acquire) {
            self.release_submit.notified().await;
        }
        Ok(SubmitTurnResponse::Accepted {
            turn_id: TurnId::new(),
            run_id: self.run_id,
            status: TurnStatus::Queued,
            resolved_run_profile_id: RunProfileId::default_profile(),
            resolved_run_profile_version: RunProfileVersion::new(1),
            event_cursor: EventCursor(23),
            accepted_message_ref: request.accepted_message_ref,
            reply_target_binding_ref: request.reply_target_binding_ref,
        })
    }

    async fn resume_turn(
        &self,
        _request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        panic!("resume_turn is not used by delete submit serialization tests")
    }

    async fn cancel_run(&self, _request: CancelRunRequest) -> Result<CancelRunResponse, TurnError> {
        panic!("cancel_run is not used by delete submit serialization tests")
    }

    async fn get_run_state(&self, request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        Ok(TurnRunState {
            scope: request.scope,
            actor: Some(turn_actor_for_user("user-alpha")),
            turn_id: TurnId::new(),
            run_id: request.run_id,
            status: TurnStatus::Queued,
            accepted_message_ref: AcceptedMessageRef::new("msg:blocked-submit").expect("valid ref"),
            source_binding_ref: SourceBindingRef::new("webui-src:blocked-submit")
                .expect("valid ref"),
            reply_target_binding_ref: ReplyTargetBindingRef::new("webui-reply:blocked-submit")
                .expect("valid ref"),
            resolved_run_profile_id: RunProfileId::default_profile(),
            resolved_run_profile_version: RunProfileVersion::new(1),
            resolved_model_route: None,
            received_at: Utc::now(),
            checkpoint_id: None,
            gate_ref: None,
            credential_requirements: Vec::new(),
            failure: None,
            event_cursor: EventCursor(29),
        })
    }
}

#[derive(Default)]
struct RecordingApprovalInteractionService {
    resolutions: Mutex<Vec<ResolveApprovalInteractionRequest>>,
}

impl RecordingApprovalInteractionService {
    fn resolution_count(&self) -> usize {
        self.resolutions.lock().expect("lock").len()
    }

    fn last_resolution(&self) -> Option<ResolveApprovalInteractionRequest> {
        self.resolutions.lock().expect("lock").last().cloned()
    }
}

#[async_trait]
impl ApprovalInteractionService for RecordingApprovalInteractionService {
    async fn list_pending(
        &self,
        _request: ListPendingApprovalsRequest,
    ) -> Result<ListPendingApprovalsResponse, ironclaw_product_workflow::ProductWorkflowError> {
        Ok(ListPendingApprovalsResponse { approvals: vec![] })
    }

    async fn resolve(
        &self,
        request: ResolveApprovalInteractionRequest,
    ) -> Result<ResolveApprovalInteractionResponse, ironclaw_product_workflow::ProductWorkflowError>
    {
        let run_id = request.run_id_hint.expect("webui passes run_id");
        let decision = request.decision;
        self.resolutions.lock().expect("lock").push(request);
        Ok(match decision {
            ApprovalInteractionDecision::ApproveOnce => {
                ResolveApprovalInteractionResponse::Approved(ResumeTurnResponse {
                    run_id,
                    status: TurnStatus::Queued,
                    event_cursor: EventCursor(19),
                })
            }
            ApprovalInteractionDecision::Deny => {
                ResolveApprovalInteractionResponse::Denied(CancelRunResponse {
                    run_id,
                    status: TurnStatus::Cancelled,
                    event_cursor: EventCursor(23),
                    already_terminal: false,
                    actor: None,
                })
            }
        })
    }
}

#[derive(Default)]
struct RecordingAuthInteractionService {
    resolutions: Mutex<Vec<ResolveAuthInteractionRequest>>,
}

impl RecordingAuthInteractionService {
    fn resolution_count(&self) -> usize {
        self.resolutions.lock().expect("lock").len()
    }

    fn last_resolution(&self) -> Option<ResolveAuthInteractionRequest> {
        self.resolutions.lock().expect("lock").last().cloned()
    }
}

#[async_trait]
impl AuthInteractionService for RecordingAuthInteractionService {
    async fn list_pending(
        &self,
        _request: ListPendingAuthInteractionsRequest,
    ) -> Result<ListPendingAuthInteractionsResponse, ironclaw_product_workflow::ProductWorkflowError>
    {
        Ok(ListPendingAuthInteractionsResponse {
            auth_interactions: vec![],
        })
    }

    async fn resolve(
        &self,
        request: ResolveAuthInteractionRequest,
    ) -> Result<ResolveAuthInteractionResponse, ironclaw_product_workflow::ProductWorkflowError>
    {
        let run_id = request.run_id_hint.expect("webui passes run_id");
        let decision = request.decision.clone();
        self.resolutions.lock().expect("lock").push(request);
        Ok(match decision {
            AuthInteractionDecision::CredentialProvided { .. }
            | AuthInteractionDecision::CallbackCompleted { .. } => {
                ResolveAuthInteractionResponse::Resumed(ResumeTurnResponse {
                    run_id,
                    status: TurnStatus::Queued,
                    event_cursor: EventCursor(29),
                })
            }
            AuthInteractionDecision::Deny => {
                ResolveAuthInteractionResponse::Canceled(CancelRunResponse {
                    run_id,
                    status: TurnStatus::Cancelled,
                    event_cursor: EventCursor(31),
                    already_terminal: false,
                    actor: None,
                })
            }
        })
    }
}

struct RecordingLifecycleFacade {
    package_refs: Mutex<Vec<LifecyclePackageRef>>,
    credential_requirements: Vec<LifecycleExtensionCredentialRequirement>,
    onboarding: Option<LifecycleExtensionOnboarding>,
}

impl RecordingLifecycleFacade {
    fn new() -> Self {
        Self {
            package_refs: Mutex::new(Vec::new()),
            credential_requirements: Vec::new(),
            onboarding: None,
        }
    }

    fn with_credential_requirements(
        credential_requirements: Vec<LifecycleExtensionCredentialRequirement>,
    ) -> Self {
        Self {
            package_refs: Mutex::new(Vec::new()),
            credential_requirements,
            onboarding: None,
        }
    }

    fn with_credential_requirements_and_onboarding(
        credential_requirements: Vec<LifecycleExtensionCredentialRequirement>,
        onboarding: LifecycleExtensionOnboarding,
    ) -> Self {
        Self {
            package_refs: Mutex::new(Vec::new()),
            credential_requirements,
            onboarding: Some(onboarding),
        }
    }

    fn package_refs(&self) -> Vec<LifecyclePackageRef> {
        self.package_refs.lock().expect("lock").clone()
    }

    fn extension_list_payload(
        &self,
        package_ref: &LifecyclePackageRef,
    ) -> Option<LifecycleProductPayload> {
        if self.credential_requirements.is_empty() {
            return None;
        }
        let summary = LifecycleExtensionSummary {
            package_ref: package_ref.clone(),
            name: package_ref.id.as_str().to_string(),
            version: "1.0.0".to_string(),
            description: "test extension".to_string(),
            source: LifecycleExtensionSource::HostBundled,
            runtime_kind: LifecycleExtensionRuntimeKind::FirstParty,
            visible_capability_ids: Vec::new(),
            visible_read_only_capability_ids: Vec::new(),
            credential_requirements: self.credential_requirements.clone(),
            onboarding: self.onboarding.clone(),
        };
        Some(LifecycleProductPayload::ExtensionList {
            extensions: vec![LifecycleInstalledExtensionSummary {
                summary,
                phase: LifecyclePhase::Configured,
            }],
            count: 1,
        })
    }
}

#[async_trait]
impl LifecycleProductFacade for RecordingLifecycleFacade {
    async fn execute(
        &self,
        _context: LifecycleProductContext,
        _action: ironclaw_product_workflow::LifecycleProductAction,
    ) -> Result<LifecycleProductResponse, ironclaw_product_workflow::ProductWorkflowError> {
        panic!("setup_extension should project package state, not execute lifecycle actions")
    }

    async fn project_package(
        &self,
        _context: LifecycleProductContext,
        package_ref: LifecyclePackageRef,
    ) -> Result<LifecycleProductResponse, ironclaw_product_workflow::ProductWorkflowError> {
        self.package_refs
            .lock()
            .expect("lock")
            .push(package_ref.clone());
        let phase = if self.credential_requirements.is_empty() {
            LifecyclePhase::UnsupportedOrLegacy
        } else {
            LifecyclePhase::Configured
        };
        let mut response = LifecycleProductResponse::projection(
            Some(package_ref),
            phase,
            vec![LifecycleReadinessBlocker::runtime(Some(
                "extension_lifecycle_store_unwired".to_string(),
            ))?],
        );
        response.payload = self.extension_list_payload(response.package_ref.as_ref().expect("ref"));
        Ok(response)
    }
}

struct ListingLifecycleFacade {
    extension: LifecycleInstalledExtensionSummary,
}

#[async_trait]
impl LifecycleProductFacade for ListingLifecycleFacade {
    async fn execute(
        &self,
        _context: LifecycleProductContext,
        action: LifecycleProductAction,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
        assert!(matches!(action, LifecycleProductAction::ExtensionList));
        Ok(LifecycleProductResponse {
            package_ref: None,
            phase: self.extension.phase,
            blockers: Vec::new(),
            message: None,
            payload: Some(LifecycleProductPayload::ExtensionList {
                extensions: vec![self.extension.clone()],
                count: 1,
            }),
        })
    }

    async fn project_package(
        &self,
        _context: LifecycleProductContext,
        _package_ref: LifecyclePackageRef,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
        panic!("list_extensions should execute the list action, not project one package")
    }
}

#[derive(Debug, Clone)]
struct ListAutomationCall {
    caller: ProductAgentBoundCaller,
    limit: usize,
    run_limit: usize,
}

#[derive(Default)]
struct RecordingAutomationFacade {
    list_calls: Mutex<Vec<ListAutomationCall>>,
}

impl RecordingAutomationFacade {
    fn list_calls(&self) -> Vec<ListAutomationCall> {
        self.list_calls.lock().expect("lock").clone()
    }
}

#[async_trait]
impl AutomationProductFacade for RecordingAutomationFacade {
    async fn list_automations(
        &self,
        caller: ProductAgentBoundCaller,
        request: AutomationListRequest,
    ) -> Result<Vec<RebornAutomationInfo>, RebornServicesError> {
        self.list_calls
            .lock()
            .expect("lock")
            .push(ListAutomationCall {
                caller,
                limit: request.limit,
                run_limit: request.run_limit,
            });
        Ok(vec![automation_info(
            "trigger-listed",
            "Daily status",
            "0 9 * * *",
            Some(RebornAutomationRunStatus::Ok),
        )])
    }
}

#[derive(Clone)]
struct StaticAutomationFacade {
    output: Vec<RebornAutomationInfo>,
}

#[async_trait]
impl AutomationProductFacade for StaticAutomationFacade {
    async fn list_automations(
        &self,
        _caller: ProductAgentBoundCaller,
        _request: AutomationListRequest,
    ) -> Result<Vec<RebornAutomationInfo>, RebornServicesError> {
        Ok(self.output.clone())
    }
}

#[derive(Debug, Clone)]
struct OutboundPreferencesSetCall {
    caller: WebUiAuthenticatedCaller,
    request: RebornSetOutboundPreferencesRequest,
}

#[derive(Default)]
struct RecordingOutboundPreferencesFacade {
    get_calls: Mutex<Vec<WebUiAuthenticatedCaller>>,
    set_calls: Mutex<Vec<OutboundPreferencesSetCall>>,
    list_calls: Mutex<Vec<WebUiAuthenticatedCaller>>,
}

impl RecordingOutboundPreferencesFacade {
    fn get_calls(&self) -> Vec<WebUiAuthenticatedCaller> {
        self.get_calls.lock().expect("lock").clone()
    }

    fn set_calls(&self) -> Vec<OutboundPreferencesSetCall> {
        self.set_calls.lock().expect("lock").clone()
    }

    fn list_calls(&self) -> Vec<WebUiAuthenticatedCaller> {
        self.list_calls.lock().expect("lock").clone()
    }
}

#[async_trait]
impl OutboundPreferencesProductFacade for RecordingOutboundPreferencesFacade {
    async fn get_outbound_preferences(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornOutboundPreferencesResponse, RebornServicesError> {
        self.get_calls.lock().expect("lock").push(caller);
        Ok(RebornOutboundPreferencesResponse {
            final_reply_target: Some(outbound_target_summary("slack-dm-alpha")),
            final_reply_target_status: RebornOutboundDeliveryTargetStatus::Available,
            default_modality: RebornOutboundDeliveryModality::Text,
        })
    }

    async fn set_outbound_preferences(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornSetOutboundPreferencesRequest,
    ) -> Result<RebornOutboundPreferencesResponse, RebornServicesError> {
        self.set_calls
            .lock()
            .expect("lock")
            .push(OutboundPreferencesSetCall { caller, request });
        Ok(RebornOutboundPreferencesResponse {
            final_reply_target: Some(outbound_target_summary("slack-dm-beta")),
            final_reply_target_status: RebornOutboundDeliveryTargetStatus::Available,
            default_modality: RebornOutboundDeliveryModality::Text,
        })
    }

    async fn list_outbound_delivery_targets(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornOutboundDeliveryTargetListResponse, RebornServicesError> {
        self.list_calls.lock().expect("lock").push(caller);
        Ok(RebornOutboundDeliveryTargetListResponse {
            targets: vec![RebornOutboundDeliveryTargetOption {
                target: outbound_target_summary("slack-dm-alpha"),
                capabilities: RebornOutboundDeliveryTargetCapabilities {
                    final_replies: true,
                    gate_prompts: true,
                    auth_prompts: true,
                },
            }],
            next_cursor: None,
        })
    }
}

fn outbound_target_summary(target_id: &str) -> RebornOutboundDeliveryTargetSummary {
    RebornOutboundDeliveryTargetSummary::new(
        outbound_target_id(target_id),
        "slack",
        "Slack DM",
        Some("Slack direct message".to_string()),
    )
    .expect("valid target summary")
}

fn outbound_target_id(target_id: &str) -> RebornOutboundDeliveryTargetId {
    RebornOutboundDeliveryTargetId::new(target_id).expect("valid target id")
}

fn automation_info(
    trigger_id: &str,
    name: impl Into<String>,
    cron: impl Into<String>,
    last_status: Option<RebornAutomationRunStatus>,
) -> RebornAutomationInfo {
    RebornAutomationInfo {
        automation_id: trigger_id.to_string(),
        name: name.into(),
        source: RebornAutomationSource::Schedule { cron: cron.into() },
        state: RebornAutomationState::Active,
        next_run_at: Some("2026-06-03T09:00:00Z".parse().expect("next run")),
        last_run_at: None,
        last_status,
        recent_runs: vec![RebornAutomationRecentRunInfo {
            run_id: Some(automation_run_id()),
            thread_id: ThreadId::new("thread-listed").expect("valid thread id"),
            fire_slot: Some("2026-06-03T09:00:00Z".parse().expect("fire slot")),
            status: RebornAutomationRecentRunStatus::Ok,
            submitted_at: "2026-06-03T09:00:01Z".parse().expect("submitted at"),
            completed_at: Some("2026-06-03T09:00:42Z".parse().expect("completed at")),
        }],
        is_active: true,
        created_at: Some("2026-06-02T18:00:00Z".parse().expect("created at")),
    }
}

#[derive(Default)]
struct RecordingExtensionCredentialSetupService {
    status_requests: Mutex<Vec<ExtensionCredentialStatusRequest>>,
    submit_requests: Mutex<Vec<ExtensionCredentialSubmitRequest>>,
}

impl RecordingExtensionCredentialSetupService {
    fn status_count(&self) -> usize {
        self.status_requests.lock().expect("lock").len()
    }

    fn submit_count(&self) -> usize {
        self.submit_requests.lock().expect("lock").len()
    }
}

#[async_trait]
impl ExtensionCredentialSetupService for RecordingExtensionCredentialSetupService {
    async fn credential_status(
        &self,
        request: ExtensionCredentialStatusRequest,
    ) -> Result<Option<CredentialAccountProjection>, RebornServicesError> {
        self.status_requests.lock().expect("lock").push(request);
        Ok(None)
    }

    async fn submit_manual_token(
        &self,
        request: ExtensionCredentialSubmitRequest,
    ) -> Result<CredentialAccountId, RebornServicesError> {
        self.submit_requests.lock().expect("lock").push(request);
        Ok(CredentialAccountId::new())
    }
}

struct AuthFailureProjectionStream;

#[async_trait]
impl ProjectionStream for AuthFailureProjectionStream {
    async fn drain(
        &self,
        _request: ProjectionSubscriptionRequest,
    ) -> Result<Vec<ProductOutboundEnvelope>, ProductAdapterError> {
        Err(ProductAdapterError::Authentication(
            ProtocolAuthFailure::SignatureMismatch,
        ))
    }
}

struct StaticErrorProjectionStream {
    error: ProductAdapterError,
}

#[async_trait]
impl ProjectionStream for StaticErrorProjectionStream {
    async fn drain(
        &self,
        _request: ProjectionSubscriptionRequest,
    ) -> Result<Vec<ProductOutboundEnvelope>, ProductAdapterError> {
        Err(self.error.clone())
    }
}

/// Projection stream that records every `drain` invocation. Used by the
/// `stream_events` ownership regression to assert that the projection
/// drain is never reached when the ownership probe fails — if the probe
/// were skipped, `drain_count()` would observe the unauthorized read.
#[derive(Default)]
struct RecordingProjectionStream {
    drains: Mutex<Vec<ProjectionSubscriptionRequest>>,
}

impl RecordingProjectionStream {
    fn drain_count(&self) -> usize {
        self.drains.lock().expect("lock").len()
    }

    fn requests(&self) -> Vec<ProjectionSubscriptionRequest> {
        self.drains.lock().expect("lock").clone()
    }
}

#[async_trait]
impl ProjectionStream for RecordingProjectionStream {
    async fn drain(
        &self,
        request: ProjectionSubscriptionRequest,
    ) -> Result<Vec<ProductOutboundEnvelope>, ProductAdapterError> {
        self.drains.lock().expect("lock").push(request);
        Ok(Vec::new())
    }
}

/// Lighter-weight projection stream used by the timeline drain
/// regressions: counts calls without retaining the request shape. Kept
/// alongside `RecordingProjectionStream` because some sites only need
/// the count and the leaner stub keeps those tests focused.
#[derive(Default)]
struct SpyProjectionStream {
    drain_count: Mutex<usize>,
}

impl SpyProjectionStream {
    fn drain_count(&self) -> usize {
        *self.drain_count.lock().expect("lock")
    }
}

#[async_trait]
impl ProjectionStream for SpyProjectionStream {
    async fn drain(
        &self,
        _request: ProjectionSubscriptionRequest,
    ) -> Result<Vec<ProductOutboundEnvelope>, ProductAdapterError> {
        *self.drain_count.lock().expect("lock") += 1;
        Ok(Vec::new())
    }
}

/// Stub thread service whose `list_thread_history` always returns
/// `ThreadScopeMismatch`. Used to lock in the contract that ownership probes
/// remap that variant to NotFound, since the current backends happen to return
/// `UnknownThread` for the same condition. All other methods panic — none of
/// the cancel_run / resolve_gate paths under test should reach them.
struct ScopeMismatchThreadStub;

#[async_trait]
impl SessionThreadService for ScopeMismatchThreadStub {
    async fn list_thread_history(
        &self,
        request: ThreadHistoryRequest,
    ) -> Result<ThreadHistory, SessionThreadError> {
        Err(SessionThreadError::ThreadScopeMismatch {
            thread_id: request.thread_id,
        })
    }

    async fn ensure_thread(
        &self,
        _request: EnsureThreadRequest,
    ) -> Result<SessionThreadRecord, SessionThreadError> {
        panic!("ScopeMismatchThreadStub::ensure_thread should not be reached")
    }

    async fn accept_inbound_message(
        &self,
        _request: AcceptInboundMessageRequest,
    ) -> Result<AcceptedInboundMessage, SessionThreadError> {
        panic!("ScopeMismatchThreadStub::accept_inbound_message should not be reached")
    }

    async fn replay_accepted_inbound_message(
        &self,
        _request: ReplayAcceptedInboundMessageRequest,
    ) -> Result<Option<AcceptedInboundMessageReplay>, SessionThreadError> {
        panic!("ScopeMismatchThreadStub::replay_accepted_inbound_message should not be reached")
    }

    async fn mark_message_submitted(
        &self,
        _scope: &ThreadScope,
        _thread_id: &ThreadId,
        _message_id: ThreadMessageId,
        _turn_id: String,
        _turn_run_id: String,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        panic!("ScopeMismatchThreadStub::mark_message_submitted should not be reached")
    }

    async fn mark_message_deferred_busy(
        &self,
        _scope: &ThreadScope,
        _thread_id: &ThreadId,
        _message_id: ThreadMessageId,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        panic!("ScopeMismatchThreadStub::mark_message_deferred_busy should not be reached")
    }

    async fn append_assistant_draft(
        &self,
        _request: AppendAssistantDraftRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        panic!("ScopeMismatchThreadStub::append_assistant_draft should not be reached")
    }

    async fn append_tool_result_reference(
        &self,
        _request: AppendToolResultReferenceRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        panic!("ScopeMismatchThreadStub::append_tool_result_reference should not be reached")
    }

    async fn append_capability_display_preview(
        &self,
        _request: AppendCapabilityDisplayPreviewRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        panic!("ScopeMismatchThreadStub::append_capability_display_preview should not be reached")
    }

    async fn update_tool_result_reference(
        &self,
        _request: UpdateToolResultReferenceRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        panic!("ScopeMismatchThreadStub::update_tool_result_reference should not be reached")
    }

    async fn update_assistant_draft(
        &self,
        _request: UpdateAssistantDraftRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        panic!("ScopeMismatchThreadStub::update_assistant_draft should not be reached")
    }

    async fn finalize_assistant_message(
        &self,
        _scope: &ThreadScope,
        _thread_id: &ThreadId,
        _message_id: ThreadMessageId,
        _content: MessageContent,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        panic!("ScopeMismatchThreadStub::finalize_assistant_message should not be reached")
    }

    async fn redact_message(
        &self,
        _request: RedactMessageRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        panic!("ScopeMismatchThreadStub::redact_message should not be reached")
    }

    async fn load_context_window(
        &self,
        _request: LoadContextWindowRequest,
    ) -> Result<ContextWindow, SessionThreadError> {
        panic!("ScopeMismatchThreadStub::load_context_window should not be reached")
    }

    async fn load_context_messages(
        &self,
        _request: LoadContextMessagesRequest,
    ) -> Result<ContextMessages, SessionThreadError> {
        panic!("ScopeMismatchThreadStub::load_context_messages should not be reached")
    }

    async fn create_summary_artifact(
        &self,
        _request: CreateSummaryArtifactRequest,
    ) -> Result<SummaryArtifact, SessionThreadError> {
        panic!("ScopeMismatchThreadStub::create_summary_artifact should not be reached")
    }
}

enum ScriptedThreadBehavior {
    BackendHistory,
    History(Box<ThreadHistory>),
    ListPages,
    SubmittedReplay { turn_run_id: Option<String> },
}

struct ScriptedThreadService {
    behavior: ScriptedThreadBehavior,
    history_requests: Mutex<Vec<ThreadHistoryRequest>>,
    list_requests: Mutex<Vec<ListThreadsForScopeRequest>>,
    list_responses: Mutex<Vec<ListThreadsForScopeResponse>>,
}

impl ScriptedThreadService {
    fn backend_history() -> Self {
        Self {
            behavior: ScriptedThreadBehavior::BackendHistory,
            history_requests: Mutex::new(Vec::new()),
            list_requests: Mutex::new(Vec::new()),
            list_responses: Mutex::new(Vec::new()),
        }
    }

    fn history(history: ThreadHistory) -> Self {
        Self {
            behavior: ScriptedThreadBehavior::History(Box::new(history)),
            history_requests: Mutex::new(Vec::new()),
            list_requests: Mutex::new(Vec::new()),
            list_responses: Mutex::new(Vec::new()),
        }
    }

    fn list_pages(responses: Vec<ListThreadsForScopeResponse>) -> Self {
        Self {
            behavior: ScriptedThreadBehavior::ListPages,
            history_requests: Mutex::new(Vec::new()),
            list_requests: Mutex::new(Vec::new()),
            list_responses: Mutex::new(responses),
        }
    }

    fn submitted_replay(turn_run_id: Option<String>) -> Self {
        Self {
            behavior: ScriptedThreadBehavior::SubmittedReplay { turn_run_id },
            history_requests: Mutex::new(Vec::new()),
            list_requests: Mutex::new(Vec::new()),
            list_responses: Mutex::new(Vec::new()),
        }
    }

    fn history_requests(&self) -> Vec<ThreadHistoryRequest> {
        self.history_requests.lock().expect("lock").clone()
    }

    fn list_requests(&self) -> Vec<ListThreadsForScopeRequest> {
        self.list_requests.lock().expect("lock").clone()
    }
}

#[async_trait]
impl SessionThreadService for ScriptedThreadService {
    async fn list_thread_history(
        &self,
        request: ThreadHistoryRequest,
    ) -> Result<ThreadHistory, SessionThreadError> {
        self.history_requests
            .lock()
            .expect("lock")
            .push(request.clone());
        match &self.behavior {
            ScriptedThreadBehavior::BackendHistory => Err(SessionThreadError::Backend(
                "backend detail /host/path secret-token".to_string(),
            )),
            ScriptedThreadBehavior::History(history) => Ok(history.as_ref().clone()),
            ScriptedThreadBehavior::ListPages => scripted_stub_unreachable("list_thread_history"),
            ScriptedThreadBehavior::SubmittedReplay { .. } => Ok(ThreadHistory {
                thread: SessionThreadRecord {
                    scope: request.scope,
                    thread_id: request.thread_id,
                    created_by_actor_id: "user-alpha".to_string(),
                    title: None,
                    metadata_json: None,
                    goal: None,
                },
                messages: Vec::new(),
                summary_artifacts: Vec::new(),
            }),
        }
    }

    async fn ensure_thread(
        &self,
        _request: EnsureThreadRequest,
    ) -> Result<SessionThreadRecord, SessionThreadError> {
        scripted_stub_unreachable("ensure_thread")
    }

    async fn accept_inbound_message(
        &self,
        _request: AcceptInboundMessageRequest,
    ) -> Result<AcceptedInboundMessage, SessionThreadError> {
        scripted_stub_unreachable("accept_inbound_message")
    }

    async fn replay_accepted_inbound_message(
        &self,
        request: ReplayAcceptedInboundMessageRequest,
    ) -> Result<Option<AcceptedInboundMessageReplay>, SessionThreadError> {
        match &self.behavior {
            ScriptedThreadBehavior::SubmittedReplay { turn_run_id } => {
                Ok(Some(AcceptedInboundMessageReplay {
                    scope: request.scope,
                    thread_id: ThreadId::new("thread-alpha").expect("valid thread"),
                    message_id: ThreadMessageId::new(),
                    sequence: 1,
                    status: MessageStatus::Submitted,
                    actor_id: Some(request.actor_id),
                    source_binding_id: Some(request.source_binding_id),
                    reply_target_binding_id: Some("webui-reply:replayed".to_string()),
                    turn_run_id: turn_run_id.clone(),
                }))
            }
            ScriptedThreadBehavior::BackendHistory
            | ScriptedThreadBehavior::History(_)
            | ScriptedThreadBehavior::ListPages => {
                scripted_stub_unreachable("replay_accepted_inbound_message")
            }
        }
    }

    async fn mark_message_submitted(
        &self,
        _scope: &ThreadScope,
        _thread_id: &ThreadId,
        _message_id: ThreadMessageId,
        _turn_id: String,
        _turn_run_id: String,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        scripted_stub_unreachable("mark_message_submitted")
    }

    async fn mark_message_deferred_busy(
        &self,
        _scope: &ThreadScope,
        _thread_id: &ThreadId,
        _message_id: ThreadMessageId,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        scripted_stub_unreachable("mark_message_deferred_busy")
    }

    async fn append_assistant_draft(
        &self,
        _request: AppendAssistantDraftRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        scripted_stub_unreachable("append_assistant_draft")
    }

    async fn append_tool_result_reference(
        &self,
        _request: AppendToolResultReferenceRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        scripted_stub_unreachable("append_tool_result_reference")
    }

    async fn append_capability_display_preview(
        &self,
        _request: AppendCapabilityDisplayPreviewRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        scripted_stub_unreachable("append_capability_display_preview")
    }

    async fn update_tool_result_reference(
        &self,
        _request: UpdateToolResultReferenceRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        scripted_stub_unreachable("update_tool_result_reference")
    }

    async fn update_assistant_draft(
        &self,
        _request: UpdateAssistantDraftRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        scripted_stub_unreachable("update_assistant_draft")
    }

    async fn finalize_assistant_message(
        &self,
        _scope: &ThreadScope,
        _thread_id: &ThreadId,
        _message_id: ThreadMessageId,
        _content: MessageContent,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        scripted_stub_unreachable("finalize_assistant_message")
    }

    async fn redact_message(
        &self,
        _request: RedactMessageRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        scripted_stub_unreachable("redact_message")
    }

    async fn load_context_window(
        &self,
        _request: LoadContextWindowRequest,
    ) -> Result<ContextWindow, SessionThreadError> {
        scripted_stub_unreachable("load_context_window")
    }

    async fn load_context_messages(
        &self,
        _request: LoadContextMessagesRequest,
    ) -> Result<ContextMessages, SessionThreadError> {
        scripted_stub_unreachable("load_context_messages")
    }

    async fn create_summary_artifact(
        &self,
        _request: CreateSummaryArtifactRequest,
    ) -> Result<SummaryArtifact, SessionThreadError> {
        scripted_stub_unreachable("create_summary_artifact")
    }

    async fn list_threads_for_scope(
        &self,
        request: ListThreadsForScopeRequest,
    ) -> Result<ListThreadsForScopeResponse, SessionThreadError> {
        match &self.behavior {
            ScriptedThreadBehavior::ListPages => {
                self.list_requests.lock().expect("lock").push(request);
                let mut responses = self.list_responses.lock().expect("lock");
                if responses.is_empty() {
                    scripted_stub_unreachable("list_threads_for_scope");
                }
                Ok(responses.remove(0))
            }
            _ => scripted_stub_unreachable("list_threads_for_scope"),
        }
    }
}

fn scripted_stub_unreachable(method: &str) -> ! {
    panic!("ScriptedThreadService::{method} should not be reached")
}

async fn create_thread_for(
    services: &RebornServices,
    caller: WebUiAuthenticatedCaller,
    thread_id: &str,
) {
    services
        .create_thread(
            caller,
            serde_json::from_value::<WebUiCreateThreadRequest>(json!({
                "client_action_id": format!("create-{thread_id}"),
                "requested_thread_id": thread_id
            }))
            .expect("create request"),
        )
        .await
        .expect("create thread");
}

#[tokio::test]
async fn duplicate_create_thread_replays_generated_thread_for_same_client_action() {
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    );
    let request = || {
        serde_json::from_value::<WebUiCreateThreadRequest>(json!({
            "client_action_id": "create-duplicate"
        }))
        .expect("request")
    };

    let first = services
        .create_thread(caller(), request())
        .await
        .expect("first create succeeds");
    let replayed = services
        .create_thread(caller(), request())
        .await
        .expect("duplicate create replays");

    assert_eq!(first.thread.thread_id, replayed.thread.thread_id);
    assert_eq!(first.thread.metadata_json, replayed.thread.metadata_json);
}

#[tokio::test]
async fn create_thread_metadata_is_serialized_json() {
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    );
    let client_action_id = "create-quote-\"-slash-\\-line-\u{2028}".to_string();

    let response = services
        .create_thread(
            caller(),
            serde_json::from_value::<WebUiCreateThreadRequest>(json!({
                "client_action_id": client_action_id
            }))
            .expect("request"),
        )
        .await
        .expect("create succeeds");

    let metadata = response.thread.metadata_json.expect("metadata");
    let metadata: serde_json::Value = serde_json::from_str(&metadata).expect("metadata json");
    assert_eq!(
        metadata["client_action_id"].as_str(),
        Some(client_action_id.as_str())
    );
}

#[test]
fn facade_error_taxonomy_serializes_all_stable_wire_names() {
    let error = RebornServicesError {
        code: RebornServicesErrorCode::Conflict,
        kind: RebornServicesErrorKind::Busy,
        status_code: 409,
        retryable: false,
        field: None,
        validation_code: None,
    };

    let json = serde_json::to_value(&error).expect("error json");

    assert_eq!(json["code"], "conflict");
    assert_eq!(json["kind"], "busy");
    assert_eq!(json["status_code"], 409);
    assert_eq!(json["retryable"], false);

    let cases = [
        (RebornServicesErrorKind::Validation, "validation"),
        (RebornServicesErrorKind::Duplicate, "duplicate"),
        (RebornServicesErrorKind::Busy, "busy"),
        (
            RebornServicesErrorKind::ParticipantDenied,
            "participant_denied",
        ),
        (RebornServicesErrorKind::BlockedApproval, "blocked_approval"),
        (
            RebornServicesErrorKind::BlockedAuthentication,
            "blocked_authentication",
        ),
        (RebornServicesErrorKind::BlockedResource, "blocked_resource"),
        (
            RebornServicesErrorKind::ReplayUnavailable,
            "replay_unavailable",
        ),
        (
            RebornServicesErrorKind::TimelineUnavailable,
            "timeline_unavailable",
        ),
        (
            RebornServicesErrorKind::ServiceUnavailable,
            "service_unavailable",
        ),
        (RebornServicesErrorKind::NotFound, "not_found"),
        (RebornServicesErrorKind::Conflict, "conflict"),
        (RebornServicesErrorKind::Internal, "internal"),
    ];
    for (kind, expected) in cases {
        assert_eq!(
            serde_json::to_value(kind).expect("kind json"),
            serde_json::json!(expected),
            "{kind:?} must keep its stable WebUI wire name"
        );
    }
}

#[tokio::test]
async fn submit_turn_uses_facade_and_thread_history_without_route_store_access() {
    let threads: Arc<dyn SessionThreadService> = Arc::new(InMemorySessionThreadService::default());
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let services = RebornServices::new(threads, coordinator.clone());
    create_thread_for(&services, caller(), "thread-alpha").await;

    let response = services
        .submit_turn(
            caller(),
            serde_json::from_value::<WebUiSendMessageRequest>(json!({
                "client_action_id": "send-1",
                "thread_id": "thread-alpha",
                "content": "hello from webui"
            }))
            .expect("request"),
        )
        .await
        .expect("submit succeeds");

    let RebornSubmitTurnResponse::Submitted {
        thread_id,
        status,
        event_cursor,
        ..
    } = response
    else {
        panic!("expected submitted response");
    };
    assert_eq!(thread_id.as_str(), "thread-alpha");
    assert_eq!(status, TurnStatus::Queued);
    assert_eq!(event_cursor, EventCursor(7));
    assert_eq!(coordinator.submission_count(), 1);

    let timeline = services
        .get_timeline(
            caller(),
            RebornTimelineRequest {
                thread_id: "thread-alpha".to_string(),
                ..Default::default()
            },
        )
        .await
        .expect("timeline");
    assert_eq!(timeline.messages.len(), 1);
    assert_eq!(timeline.messages[0].status, MessageStatus::Submitted);
    assert_eq!(
        timeline.messages[0].content.as_deref(),
        Some("hello from webui")
    );
    let submission_scope = coordinator
        .last_submission_scope()
        .expect("submission scope");
    assert_eq!(submission_scope.thread_id.as_str(), "thread-alpha");
    assert_eq!(submission_scope.tenant_id.as_str(), "tenant-alpha");
    assert_eq!(
        submission_scope.agent_id.expect("agent").as_str(),
        "agent-alpha"
    );
    assert_eq!(
        submission_scope.project_id.expect("project").as_str(),
        "project-alpha"
    );
}

#[tokio::test]
async fn submit_turn_records_skill_activation_message_before_turn_wake() {
    let threads: Arc<dyn SessionThreadService> = Arc::new(InMemorySessionThreadService::default());
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let recorded_for_hook = Arc::clone(&recorded);
    let services = RebornServices::new(threads, coordinator.clone())
        .with_skill_activation_recorder(move |scope, accepted_message_ref, message| {
            recorded_for_hook.lock().expect("lock").push((
                scope.thread_id.as_str().to_string(),
                accepted_message_ref.as_str().to_string(),
                message.to_string(),
            ));
            Ok(())
        });
    create_thread_for(&services, caller(), "thread-alpha").await;

    let submitted = services
        .submit_turn(
            caller(),
            serde_json::from_value::<WebUiSendMessageRequest>(json!({
                "client_action_id": "send-skill-activation",
                "thread_id": "thread-alpha",
                "content": "/code-review inspect this"
            }))
            .expect("request"),
        )
        .await
        .expect("submit succeeds");
    let RebornSubmitTurnResponse::Submitted {
        accepted_message_ref,
        ..
    } = submitted
    else {
        panic!("first submit should be accepted")
    };

    assert_eq!(coordinator.submission_count(), 1);
    assert_eq!(
        recorded.lock().expect("lock").as_slice(),
        &[(
            "thread-alpha".to_string(),
            accepted_message_ref.as_str().to_string(),
            "/code-review inspect this".to_string()
        )]
    );
}

#[tokio::test]
async fn busy_submit_clears_skill_activation_message() {
    let threads: Arc<dyn SessionThreadService> = Arc::new(InMemorySessionThreadService::default());
    let active_run_id = TurnRunId::new();
    let coordinator = Arc::new(FakeTurnCoordinator::with_submit_error(
        TurnError::ThreadBusy(ironclaw_turns::ThreadBusy {
            active_run_id,
            status: TurnStatus::Running,
            event_cursor: EventCursor(17),
        }),
    ));
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let cleared = Arc::new(Mutex::new(Vec::new()));
    let recorded_for_hook = Arc::clone(&recorded);
    let cleared_for_hook = Arc::clone(&cleared);
    let services = RebornServices::new(threads, coordinator.clone()).with_skill_activation_hooks(
        move |scope, accepted_message_ref, message| {
            recorded_for_hook.lock().expect("lock").push((
                scope.thread_id.as_str().to_string(),
                accepted_message_ref.as_str().to_string(),
                message.to_string(),
            ));
            Ok(())
        },
        move |scope, accepted_message_ref| {
            cleared_for_hook.lock().expect("lock").push((
                scope.thread_id.as_str().to_string(),
                accepted_message_ref.as_str().to_string(),
            ));
            Ok(())
        },
    );
    create_thread_for(&services, caller(), "thread-alpha").await;

    let deferred = services
        .submit_turn(
            caller(),
            serde_json::from_value::<WebUiSendMessageRequest>(json!({
                "client_action_id": "send-skill-activation-busy",
                "thread_id": "thread-alpha",
                "content": "/code-review inspect this"
            }))
            .expect("request"),
        )
        .await
        .expect("busy submit is deferred");

    assert!(matches!(
        deferred,
        RebornSubmitTurnResponse::DeferredBusy {
            active_run_id: id,
            ..
        } if id == active_run_id
    ));
    assert_eq!(coordinator.submission_count(), 0);
    let recorded = recorded.lock().expect("lock");
    let cleared = cleared.lock().expect("lock");
    assert_eq!(recorded.len(), 1);
    assert_eq!(
        cleared.as_slice(),
        &[(recorded[0].0.clone(), recorded[0].1.clone())],
        "deferred submissions must clear their activation input before returning"
    );
}

#[tokio::test]
async fn submit_turn_returns_internal_when_skill_activation_recorder_fails() {
    let threads: Arc<dyn SessionThreadService> = Arc::new(InMemorySessionThreadService::default());
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let services = RebornServices::new(threads, coordinator.clone())
        .with_skill_activation_recorder(|_, _, _| {
            Err(ironclaw_product_workflow::RebornServicesError {
                code: RebornServicesErrorCode::Internal,
                kind: RebornServicesErrorKind::Internal,
                status_code: 500,
                retryable: false,
                field: None,
                validation_code: None,
            })
        });
    create_thread_for(&services, caller(), "thread-alpha").await;

    let err = services
        .submit_turn(
            caller(),
            serde_json::from_value::<WebUiSendMessageRequest>(json!({
                "client_action_id": "send-recorder-fails",
                "thread_id": "thread-alpha",
                "content": "/code-review inspect this"
            }))
            .expect("request"),
        )
        .await
        .expect_err("recorder failure is surfaced");

    assert_eq!(err.code, RebornServicesErrorCode::Internal);
    assert_eq!(coordinator.submission_count(), 0);
    let timeline = services
        .get_timeline(
            caller(),
            RebornTimelineRequest {
                thread_id: "thread-alpha".to_string(),
                ..Default::default()
            },
        )
        .await
        .expect("timeline");
    assert_eq!(timeline.messages.len(), 1);
    assert_eq!(timeline.messages[0].status, MessageStatus::Accepted);
}

#[tokio::test]
async fn m2_facade_timeline_contract_uses_fake_thread_port_with_authenticated_scope() {
    let web_caller = caller();
    let expected_tenant_id = web_caller.tenant_id.clone();
    let expected_agent_id = web_caller.agent_id.clone().expect("test caller has agent");
    let expected_project_id = web_caller.project_id.clone();
    let expected_user_id = web_caller.user_id.clone();
    let thread_service = Arc::new(ScriptedThreadService::history(fake_thread_history(
        &web_caller,
        "thread-alpha",
    )));
    let services = RebornServices::new(
        thread_service.clone(),
        Arc::new(FakeTurnCoordinator::default()),
    );

    let timeline = services
        .get_timeline(
            web_caller.clone(),
            RebornTimelineRequest {
                thread_id: "thread-alpha".to_string(),
                ..Default::default()
            },
        )
        .await
        .expect("timeline is served by fake M2 thread port");

    assert_eq!(timeline.thread.thread_id.as_str(), "thread-alpha");
    assert_eq!(timeline.messages.len(), 1);
    assert_eq!(
        timeline.messages[0].content.as_deref(),
        Some("timeline from fake M2 port")
    );

    let requests = thread_service.history_requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.thread_id.as_str(), "thread-alpha");
    assert_eq!(request.scope.tenant_id, expected_tenant_id);
    assert_eq!(request.scope.agent_id, expected_agent_id);
    assert_eq!(request.scope.project_id, expected_project_id);
    assert_eq!(request.scope.owner_user_id, Some(expected_user_id));
}

#[tokio::test]
async fn m2_facade_stream_contract_uses_fake_projection_port_with_authenticated_scope() {
    let web_caller = caller();
    let event_stream = Arc::new(RecordingProjectionStream::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_event_stream(event_stream.clone());
    create_thread_for(&services, web_caller.clone(), "thread-alpha").await;
    let after_cursor = ProjectionCursor::new("cursor-alpha").expect("cursor");

    let response = services
        .stream_events(
            web_caller.clone(),
            RebornStreamEventsRequest {
                thread_id: "thread-alpha".to_string(),
                after_cursor: Some(after_cursor.clone()),
            },
        )
        .await
        .expect("stream is served by fake M2 projection port");

    assert!(response.events.is_empty());
    let requests = event_stream.requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.actor.user_id, web_caller.user_id);
    assert_eq!(request.scope.tenant_id, web_caller.tenant_id);
    assert_eq!(request.scope.agent_id, web_caller.agent_id);
    assert_eq!(request.scope.project_id, web_caller.project_id);
    assert_eq!(request.scope.thread_id.as_str(), "thread-alpha");
    assert_eq!(
        request.after_cursor.as_ref().map(ProjectionCursor::as_str),
        Some(after_cursor.as_str())
    );
}

#[tokio::test]
async fn duplicate_submit_replays_prior_handoff_without_second_submission() {
    let threads: Arc<dyn SessionThreadService> = Arc::new(InMemorySessionThreadService::default());
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let services = RebornServices::new(threads, coordinator.clone());
    create_thread_for(&services, caller(), "thread-alpha").await;

    let request = || {
        serde_json::from_value::<WebUiSendMessageRequest>(json!({
            "client_action_id": "send-duplicate",
            "thread_id": "thread-alpha",
            "content": "hello once"
        }))
        .expect("request")
    };

    services
        .submit_turn(caller(), request())
        .await
        .expect("first submit succeeds");
    let replayed = services
        .submit_turn(caller(), request())
        .await
        .expect("duplicate submit replays");

    assert!(matches!(
        replayed,
        RebornSubmitTurnResponse::AlreadySubmitted { .. }
    ));
    assert_eq!(coordinator.submission_count(), 1);
}

#[tokio::test]
async fn submitted_replay_with_missing_or_invalid_run_id_maps_to_replay_unavailable() {
    for turn_run_id in [None, Some("not-a-uuid".to_string())] {
        let coordinator = Arc::new(FakeTurnCoordinator::default());
        let services = RebornServices::new(
            Arc::new(ScriptedThreadService::submitted_replay(turn_run_id)),
            coordinator.clone(),
        );

        let err = services
            .submit_turn(
                caller(),
                serde_json::from_value::<WebUiSendMessageRequest>(json!({
                    "client_action_id": "send-replay-corrupt",
                    "thread_id": "thread-alpha",
                    "content": "hello from webui"
                }))
                .expect("request"),
            )
            .await
            .expect_err("corrupt submitted replay cannot be reconstructed");

        assert_eq!(err.code, RebornServicesErrorCode::Conflict);
        assert_eq!(err.kind, RebornServicesErrorKind::ReplayUnavailable);
        assert_eq!(err.status_code, 409);
        assert!(!err.retryable);
        assert_eq!(coordinator.submission_count(), 0);
    }
}

#[tokio::test]
async fn submit_turn_rejects_missing_thread_before_turn_submission() {
    let threads: Arc<dyn SessionThreadService> = Arc::new(InMemorySessionThreadService::default());
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let services = RebornServices::new(threads, coordinator.clone());

    let err = services
        .submit_turn(
            caller(),
            serde_json::from_value::<WebUiSendMessageRequest>(json!({
                "client_action_id": "send-missing",
                "thread_id": "thread-missing",
                "content": "this thread was never created"
            }))
            .expect("request"),
        )
        .await
        .expect_err("missing thread must reject");

    assert_eq!(err.code, RebornServicesErrorCode::NotFound);
    assert_eq!(err.status_code, 404);
    assert_eq!(coordinator.submission_count(), 0);
}

#[tokio::test]
async fn submit_turn_maps_capacity_exceeded_to_non_retryable_rate_limit() {
    let threads: Arc<dyn SessionThreadService> = Arc::new(InMemorySessionThreadService::default());
    let coordinator = Arc::new(FakeTurnCoordinator::with_submit_error(
        TurnError::capacity_exceeded(TurnCapacityResource::SubmitTurn, 1),
    ));
    let services = RebornServices::new(threads, coordinator.clone());
    create_thread_for(&services, caller(), "thread-alpha").await;

    let err = services
        .submit_turn(
            caller(),
            serde_json::from_value::<WebUiSendMessageRequest>(json!({
                "client_action_id": "send-capacity",
                "thread_id": "thread-alpha",
                "content": "capacity denied"
            }))
            .expect("request"),
        )
        .await
        .expect_err("capacity error must map through facade");

    assert_eq!(err.code, RebornServicesErrorCode::RateLimited);
    assert_eq!(err.status_code, 429);
    assert!(!err.retryable);
    assert_eq!(coordinator.submission_count(), 0);
}

#[tokio::test]
async fn submit_turn_rejects_non_owner_before_turn_submission() {
    let threads: Arc<dyn SessionThreadService> = Arc::new(InMemorySessionThreadService::default());
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let services = RebornServices::new(threads, coordinator.clone());
    create_thread_for(&services, caller(), "thread-alpha").await;

    let err = services
        .submit_turn(
            caller_for_user("user-beta"),
            serde_json::from_value::<WebUiSendMessageRequest>(json!({
                "client_action_id": "send-denied",
                "thread_id": "thread-alpha",
                "content": "wrong participant"
            }))
            .expect("request"),
        )
        .await
        .expect_err("non-owner must reject");

    assert_eq!(err.code, RebornServicesErrorCode::NotFound);
    assert_eq!(err.status_code, 404);
    assert_eq!(coordinator.submission_count(), 0);
}

#[tokio::test]
async fn same_thread_retry_replays_legacy_submitted_message_after_binding_key_change() {
    let caller = caller();
    let thread_scope = thread_scope_for(&caller);
    let thread_id = ThreadId::new("thread-alpha").expect("valid thread");
    let legacy_binding_id = legacy_webui_source_binding_id_for(&caller, thread_id.as_str());
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    thread_service
        .ensure_thread(EnsureThreadRequest {
            scope: thread_scope.clone(),
            thread_id: Some(thread_id.clone()),
            created_by_actor_id: caller.user_id.as_str().to_string(),
            title: None,
            metadata_json: None,
        })
        .await
        .expect("thread");
    let accepted = thread_service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: thread_scope.clone(),
            thread_id: thread_id.clone(),
            actor_id: caller.user_id.as_str().to_string(),
            source_binding_id: Some(legacy_binding_id.clone()),
            reply_target_binding_id: Some(legacy_binding_id),
            external_event_id: Some("send-legacy-submitted".to_string()),
            content: MessageContent::text("hello once"),
        })
        .await
        .expect("accepted");
    let run_id = TurnRunId::new();
    thread_service
        .mark_message_submitted(
            &thread_scope,
            &thread_id,
            accepted.message_id,
            "turn-legacy".to_string(),
            run_id.to_string(),
        )
        .await
        .expect("submitted");

    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let services = RebornServices::new(thread_service.clone(), coordinator.clone());

    let replayed = services
        .submit_turn(
            caller,
            serde_json::from_value::<WebUiSendMessageRequest>(json!({
                "client_action_id": "send-legacy-submitted",
                "thread_id": "thread-alpha",
                "content": "hello once"
            }))
            .expect("request"),
        )
        .await
        .expect("legacy submit replays");

    let RebornSubmitTurnResponse::AlreadySubmitted {
        thread_id: replayed_thread_id,
        run_id: replayed_run_id,
        ..
    } = replayed
    else {
        panic!("expected already submitted replay");
    };
    assert_eq!(replayed_thread_id, thread_id);
    assert_eq!(replayed_run_id, run_id);
    assert_eq!(coordinator.submission_count(), 0);
}

#[tokio::test]
async fn same_thread_retry_reuses_legacy_accepted_message_without_creating_duplicate() {
    let caller = caller();
    let thread_scope = thread_scope_for(&caller);
    let thread_id = ThreadId::new("thread-alpha").expect("valid thread");
    let legacy_binding_id = legacy_webui_source_binding_id_for(&caller, thread_id.as_str());
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    thread_service
        .ensure_thread(EnsureThreadRequest {
            scope: thread_scope.clone(),
            thread_id: Some(thread_id.clone()),
            created_by_actor_id: caller.user_id.as_str().to_string(),
            title: None,
            metadata_json: None,
        })
        .await
        .expect("thread");
    let accepted = thread_service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: thread_scope.clone(),
            thread_id: thread_id.clone(),
            actor_id: caller.user_id.as_str().to_string(),
            source_binding_id: Some(legacy_binding_id.clone()),
            reply_target_binding_id: Some(legacy_binding_id),
            external_event_id: Some("send-legacy-accepted".to_string()),
            content: MessageContent::text("hello once"),
        })
        .await
        .expect("accepted");

    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let services = RebornServices::new(thread_service.clone(), coordinator.clone());

    let response = services
        .submit_turn(
            caller.clone(),
            serde_json::from_value::<WebUiSendMessageRequest>(json!({
                "client_action_id": "send-legacy-accepted",
                "thread_id": "thread-alpha",
                "content": "hello once"
            }))
            .expect("request"),
        )
        .await
        .expect("legacy accepted submit");

    assert!(matches!(
        response,
        RebornSubmitTurnResponse::Submitted { .. }
    ));
    assert_eq!(coordinator.submission_count(), 1);

    let timeline = services
        .get_timeline(
            caller,
            RebornTimelineRequest {
                thread_id: thread_id.as_str().to_string(),
                ..Default::default()
            },
        )
        .await
        .expect("timeline");
    assert_eq!(timeline.messages.len(), 1);
    assert_eq!(timeline.messages[0].message_id, accepted.message_id);
    assert_eq!(timeline.messages[0].status, MessageStatus::Submitted);
}

#[tokio::test]
async fn duplicate_submit_rejects_cross_thread_reuse_maps_to_duplicate_kind() {
    let threads: Arc<dyn SessionThreadService> = Arc::new(InMemorySessionThreadService::default());
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let services = RebornServices::new(threads, coordinator.clone());
    create_thread_for(&services, caller(), "thread-alpha").await;
    create_thread_for(&services, caller(), "thread-beta").await;

    services
        .submit_turn(
            caller(),
            serde_json::from_value::<WebUiSendMessageRequest>(json!({
                "client_action_id": "send-cross-thread",
                "thread_id": "thread-alpha",
                "content": "hello once"
            }))
            .expect("request"),
        )
        .await
        .expect("first submit succeeds");

    let err = services
        .submit_turn(
            caller(),
            serde_json::from_value::<WebUiSendMessageRequest>(json!({
                "client_action_id": "send-cross-thread",
                "thread_id": "thread-beta",
                "content": "hello twice"
            }))
            .expect("request"),
        )
        .await
        .expect_err("cross-thread duplicate is rejected");

    assert_eq!(err.code, RebornServicesErrorCode::Conflict);
    assert_eq!(err.kind, RebornServicesErrorKind::Duplicate);
    assert_eq!(err.status_code, 409);
    assert_eq!(coordinator.submission_count(), 1);

    let alpha_timeline = services
        .get_timeline(
            caller(),
            RebornTimelineRequest {
                thread_id: "thread-alpha".to_string(),
                ..Default::default()
            },
        )
        .await
        .expect("alpha timeline");
    assert_eq!(alpha_timeline.messages.len(), 1);

    let beta_timeline = services
        .get_timeline(
            caller(),
            RebornTimelineRequest {
                thread_id: "thread-beta".to_string(),
                ..Default::default()
            },
        )
        .await
        .expect("beta timeline");
    assert!(beta_timeline.messages.is_empty());
}

#[tokio::test]
async fn concurrent_duplicate_submit_creates_one_message_and_replays_outcome() {
    let threads: Arc<dyn SessionThreadService> = Arc::new(InMemorySessionThreadService::default());
    let coordinator = Arc::new(DefaultTurnCoordinator::new(Arc::new(
        InMemoryTurnStateStore::default(),
    )));
    let services = RebornServices::new(threads, coordinator);
    create_thread_for(&services, caller(), "thread-alpha").await;
    let services = Arc::new(services);

    let request = || {
        serde_json::from_value::<WebUiSendMessageRequest>(json!({
            "client_action_id": "send-concurrent",
            "thread_id": "thread-alpha",
            "content": "hello once"
        }))
        .expect("request")
    };

    let first = {
        let services = services.clone();
        tokio::spawn(async move { services.submit_turn(caller(), request()).await })
    };
    let second = {
        let services = services.clone();
        tokio::spawn(async move { services.submit_turn(caller(), request()).await })
    };

    let first = first.await.expect("first task join").expect("first submit");
    let second = second
        .await
        .expect("second task join")
        .expect("second submit");

    let first_run_id = match &first {
        RebornSubmitTurnResponse::Submitted { run_id, .. }
        | RebornSubmitTurnResponse::AlreadySubmitted { run_id, .. } => *run_id,
        RebornSubmitTurnResponse::DeferredBusy { .. } => {
            panic!("duplicate submit must not defer while deduping")
        }
    };
    let second_run_id = match &second {
        RebornSubmitTurnResponse::Submitted { run_id, .. }
        | RebornSubmitTurnResponse::AlreadySubmitted { run_id, .. } => *run_id,
        RebornSubmitTurnResponse::DeferredBusy { .. } => {
            panic!("duplicate submit must not defer while deduping")
        }
    };
    assert_eq!(first_run_id, second_run_id);

    let timeline = services
        .get_timeline(
            caller(),
            RebornTimelineRequest {
                thread_id: "thread-alpha".to_string(),
                ..Default::default()
            },
        )
        .await
        .expect("timeline");
    assert_eq!(timeline.messages.len(), 1);
    assert_eq!(timeline.messages[0].status, MessageStatus::Submitted);
    assert_eq!(timeline.messages[0].content.as_deref(), Some("hello once"));
}

#[tokio::test]
async fn refresh_reresolves_thread_to_same_canonical_scope() {
    let threads: Arc<dyn SessionThreadService> = Arc::new(InMemorySessionThreadService::default());
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let services = RebornServices::new(threads, coordinator);
    create_thread_for(&services, caller(), "thread-alpha").await;

    let first = services
        .get_timeline(
            caller(),
            RebornTimelineRequest {
                thread_id: "thread-alpha".to_string(),
                ..Default::default()
            },
        )
        .await
        .expect("first resolve");
    let refreshed = services
        .get_timeline(
            caller(),
            RebornTimelineRequest {
                thread_id: "thread-alpha".to_string(),
                ..Default::default()
            },
        )
        .await
        .expect("refresh resolve");

    assert_eq!(first.thread, refreshed.thread);
    assert_eq!(refreshed.thread.thread_id.as_str(), "thread-alpha");
    assert_eq!(refreshed.thread.scope.tenant_id.as_str(), "tenant-alpha");
    assert_eq!(refreshed.thread.scope.agent_id.as_str(), "agent-alpha");
    assert_eq!(
        refreshed
            .thread
            .scope
            .owner_user_id
            .expect("owner")
            .as_str(),
        "user-alpha"
    );
}

#[tokio::test]
async fn get_timeline_rejects_cross_user_access() {
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    );
    create_thread_for(&services, caller(), "thread-alpha").await;

    let err = services
        .get_timeline(
            caller_for_user("user-beta"),
            RebornTimelineRequest {
                thread_id: "thread-alpha".to_string(),
                ..Default::default()
            },
        )
        .await
        .expect_err("cross-user timeline read must be rejected");

    assert_eq!(err.code, RebornServicesErrorCode::NotFound);
    assert_eq!(err.status_code, 404);
}

#[tokio::test]
async fn delete_thread_removes_owned_thread() {
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    );
    create_thread_for(&services, caller(), "thread-alpha").await;

    let response = services
        .delete_thread(
            caller(),
            RebornDeleteThreadRequest {
                thread_id: "thread-alpha".to_string(),
            },
        )
        .await
        .expect("delete owned thread");

    assert_eq!(response.thread_id.as_str(), "thread-alpha");
    assert!(response.deleted);

    let err = services
        .get_timeline(
            caller(),
            RebornTimelineRequest {
                thread_id: "thread-alpha".to_string(),
                ..Default::default()
            },
        )
        .await
        .expect_err("deleted thread must no longer be readable");
    assert_eq!(err.code, RebornServicesErrorCode::NotFound);
    assert_eq!(err.status_code, 404);
}

#[tokio::test]
async fn delete_thread_rejects_cross_user_access_without_deleting_owner_thread() {
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    );
    let alice = caller();
    create_thread_for(&services, alice.clone(), "thread-alpha").await;

    let err = services
        .delete_thread(
            caller_for_user("user-beta"),
            RebornDeleteThreadRequest {
                thread_id: "thread-alpha".to_string(),
            },
        )
        .await
        .expect_err("cross-user delete must be rejected");

    assert_eq!(err.code, RebornServicesErrorCode::NotFound);
    assert_eq!(err.status_code, 404);

    services
        .get_timeline(
            alice,
            RebornTimelineRequest {
                thread_id: "thread-alpha".to_string(),
                ..Default::default()
            },
        )
        .await
        .expect("owner thread must remain after rejected cross-user delete");
}

#[tokio::test]
async fn delete_thread_rejects_thread_with_active_run() {
    let threads: Arc<dyn SessionThreadService> = Arc::new(InMemorySessionThreadService::default());
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let services = RebornServices::new(threads, coordinator.clone());
    create_thread_for(&services, caller(), "thread-alpha").await;
    services
        .submit_turn(
            caller(),
            serde_json::from_value::<WebUiSendMessageRequest>(json!({
                "client_action_id": "send-before-delete",
                "thread_id": "thread-alpha",
                "content": "keep this run alive"
            }))
            .expect("request"),
        )
        .await
        .expect("submit succeeds");

    let err = services
        .delete_thread(
            caller(),
            RebornDeleteThreadRequest {
                thread_id: "thread-alpha".to_string(),
            },
        )
        .await
        .expect_err("active thread delete must be rejected");

    assert_eq!(err.code, RebornServicesErrorCode::Conflict);
    assert_eq!(err.kind, RebornServicesErrorKind::Busy);
    assert_eq!(err.status_code, 409);
    assert_eq!(coordinator.run_state_request_count(), 1);
    services
        .get_timeline(
            caller(),
            RebornTimelineRequest {
                thread_id: "thread-alpha".to_string(),
                ..Default::default()
            },
        )
        .await
        .expect("rejected delete must leave thread readable");
}

#[tokio::test]
async fn delete_thread_waits_for_in_flight_submit_before_active_run_check() {
    let threads: Arc<dyn SessionThreadService> = Arc::new(InMemorySessionThreadService::default());
    let coordinator = Arc::new(BlockingSubmitCoordinator::new());
    let services = RebornServices::new(threads, coordinator.clone());
    create_thread_for(&services, caller(), "thread-alpha").await;

    let submit_services = services.clone();
    let submit_handle = tokio::spawn(async move {
        submit_services
            .submit_turn(
                caller(),
                serde_json::from_value::<WebUiSendMessageRequest>(json!({
                    "client_action_id": "send-racing-delete",
                    "thread_id": "thread-alpha",
                    "content": "submit while delete races"
                }))
                .expect("request"),
            )
            .await
    });
    coordinator.wait_for_submit().await;

    let delete_services = services.clone();
    let (delete_done_tx, mut delete_done_rx) = oneshot::channel();
    tokio::spawn(async move {
        let result = delete_services
            .delete_thread(
                caller(),
                RebornDeleteThreadRequest {
                    thread_id: "thread-alpha".to_string(),
                },
            )
            .await;
        let _ = delete_done_tx.send(result);
    });

    let early_delete = tokio::time::timeout(Duration::from_millis(25), &mut delete_done_rx).await;
    assert!(
        early_delete.is_err(),
        "delete must wait behind the in-flight submit operation"
    );

    coordinator.release_submit();
    submit_handle
        .await
        .expect("submit task joins")
        .expect("submit succeeds");

    let err = delete_done_rx
        .await
        .expect("delete result")
        .expect_err("delete sees submitted active run after waiting");
    assert_eq!(err.code, RebornServicesErrorCode::Conflict);
    assert_eq!(err.kind, RebornServicesErrorKind::Busy);
    services
        .get_timeline(
            caller(),
            RebornTimelineRequest {
                thread_id: "thread-alpha".to_string(),
                ..Default::default()
            },
        )
        .await
        .expect("rejected delete must leave thread readable");
}

#[tokio::test]
async fn stream_events_rejects_cross_user_access_before_draining_stream() {
    let stream = Arc::new(SpyProjectionStream::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_event_stream(stream.clone());
    create_thread_for(&services, caller(), "thread-alpha").await;

    let err = services
        .stream_events(
            caller_for_user("user-beta"),
            RebornStreamEventsRequest {
                thread_id: "thread-alpha".to_string(),
                after_cursor: None,
            },
        )
        .await
        .expect_err("cross-user stream read must be rejected");

    assert_eq!(err.code, RebornServicesErrorCode::NotFound);
    assert_eq!(err.status_code, 404);
    assert_eq!(stream.drain_count(), 0);
}

#[tokio::test]
async fn duplicate_submit_without_project_id_still_rejects_cross_thread_reuse() {
    let threads: Arc<dyn SessionThreadService> = Arc::new(InMemorySessionThreadService::default());
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let services = RebornServices::new(threads, coordinator.clone());
    let caller = caller_with_project(None);
    create_thread_for(&services, caller.clone(), "thread-alpha").await;
    create_thread_for(&services, caller.clone(), "thread-beta").await;

    services
        .submit_turn(
            caller.clone(),
            serde_json::from_value::<WebUiSendMessageRequest>(json!({
                "client_action_id": "send-no-project",
                "thread_id": "thread-alpha",
                "content": "hello once"
            }))
            .expect("request"),
        )
        .await
        .expect("first submit succeeds");

    let err = services
        .submit_turn(
            caller,
            serde_json::from_value::<WebUiSendMessageRequest>(json!({
                "client_action_id": "send-no-project",
                "thread_id": "thread-beta",
                "content": "hello twice"
            }))
            .expect("request"),
        )
        .await
        .expect_err("cross-thread duplicate is rejected without a project binding");

    assert_eq!(err.code, RebornServicesErrorCode::Conflict);
    assert_eq!(err.kind, RebornServicesErrorKind::Duplicate);
    assert_eq!(err.status_code, 409);
    assert_eq!(coordinator.submission_count(), 1);
}

#[tokio::test]
async fn duplicate_submit_is_isolated_by_project_scope() {
    let threads: Arc<dyn SessionThreadService> = Arc::new(InMemorySessionThreadService::default());
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let services = RebornServices::new(threads, coordinator.clone());
    create_thread_for(
        &services,
        caller_with_project(Some("project-alpha")),
        "thread-alpha",
    )
    .await;
    create_thread_for(
        &services,
        caller_with_project(Some("project-beta")),
        "thread-beta",
    )
    .await;

    let first = services
        .submit_turn(
            caller_with_project(Some("project-alpha")),
            serde_json::from_value::<WebUiSendMessageRequest>(json!({
                "client_action_id": "send-project-scoped",
                "thread_id": "thread-alpha",
                "content": "hello alpha"
            }))
            .expect("request"),
        )
        .await
        .expect("project alpha submit");
    let second = services
        .submit_turn(
            caller_with_project(Some("project-beta")),
            serde_json::from_value::<WebUiSendMessageRequest>(json!({
                "client_action_id": "send-project-scoped",
                "thread_id": "thread-beta",
                "content": "hello beta"
            }))
            .expect("request"),
        )
        .await
        .expect("project beta submit");

    assert!(matches!(first, RebornSubmitTurnResponse::Submitted { .. }));
    assert!(matches!(second, RebornSubmitTurnResponse::Submitted { .. }));
    assert_eq!(coordinator.submission_count(), 2);
}

#[tokio::test]
async fn validation_errors_are_stable_and_sanitized() {
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    );

    let err = services
        .submit_turn(
            caller(),
            serde_json::from_value::<WebUiSendMessageRequest>(json!({
                "client_action_id": "send-1",
                "thread_id": "thread-alpha"
            }))
            .expect("request"),
        )
        .await
        .expect_err("missing content rejected");

    assert_eq!(err.code, RebornServicesErrorCode::InvalidRequest);
    assert_eq!(err.kind, RebornServicesErrorKind::Validation);
    assert_eq!(err.status_code, 400);
    assert_eq!(err.field.as_deref(), Some("content"));
    assert_eq!(
        err.validation_code,
        Some(WebUiInboundValidationCode::MissingField)
    );
    let rendered = serde_json::to_string(&err).expect("json");
    assert!(!rendered.contains("backend"));
    assert!(!rendered.contains("TurnCoordinator"));
}

#[tokio::test]
async fn turn_admission_rejected_maps_to_busy_taxonomy() {
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::with_submit_error(
            TurnError::AdmissionRejected(AdmissionRejection::new(
                AdmissionRejectionReason::TenantLimit,
            )),
        )),
    );
    create_thread_for(&services, caller(), "thread-alpha").await;

    let err = services
        .submit_turn(
            caller(),
            serde_json::from_value::<WebUiSendMessageRequest>(json!({
                "client_action_id": "send-rate-limited",
                "thread_id": "thread-alpha",
                "content": "hello from webui"
            }))
            .expect("request"),
        )
        .await
        .expect_err("admission rejection is a stable busy/rate-limited error");

    assert_eq!(err.code, RebornServicesErrorCode::RateLimited);
    assert_eq!(err.kind, RebornServicesErrorKind::Busy);
    assert_eq!(err.status_code, 429);
    assert!(err.retryable);
}

#[tokio::test]
async fn turn_unauthorized_maps_to_forbidden() {
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::with_submit_error(
            TurnError::Unauthorized,
        )),
    );
    create_thread_for(&services, caller(), "thread-alpha").await;

    let err = services
        .submit_turn(
            caller(),
            serde_json::from_value::<WebUiSendMessageRequest>(json!({
                "client_action_id": "send-forbidden",
                "thread_id": "thread-alpha",
                "content": "hello from webui"
            }))
            .expect("request"),
        )
        .await
        .expect_err("turn unauthorized is forbidden");

    assert_eq!(err.code, RebornServicesErrorCode::Forbidden);
    assert_eq!(err.kind, RebornServicesErrorKind::ParticipantDenied);
    assert_eq!(err.status_code, 403);
}

#[tokio::test]
async fn turn_error_categories_map_to_facade_taxonomy() {
    let cases = [
        (
            "conflict",
            TurnError::Conflict {
                reason: "active run changed".to_string(),
            },
            RebornServicesErrorCode::Conflict,
            RebornServicesErrorKind::Conflict,
            409,
            false,
        ),
        (
            "scope-not-found",
            TurnError::ScopeNotFound,
            RebornServicesErrorCode::NotFound,
            RebornServicesErrorKind::NotFound,
            404,
            false,
        ),
        (
            "invalid-request",
            TurnError::InvalidRequest {
                reason: "invalid run profile".to_string(),
            },
            RebornServicesErrorCode::InvalidRequest,
            RebornServicesErrorKind::Validation,
            400,
            false,
        ),
        (
            "unavailable",
            TurnError::Unavailable {
                reason: "turn store unavailable".to_string(),
            },
            RebornServicesErrorCode::Unavailable,
            RebornServicesErrorKind::ServiceUnavailable,
            503,
            true,
        ),
    ];

    for (name, turn_error, expected_code, expected_kind, expected_status, expected_retryable) in
        cases
    {
        let services = RebornServices::new(
            Arc::new(InMemorySessionThreadService::default()),
            Arc::new(FakeTurnCoordinator::with_submit_error(turn_error)),
        );
        let thread_id = format!("thread-{name}");
        create_thread_for(&services, caller(), &thread_id).await;

        let err = services
            .submit_turn(
                caller(),
                serde_json::from_value::<WebUiSendMessageRequest>(json!({
                    "client_action_id": format!("send-{name}"),
                    "thread_id": thread_id,
                    "content": "hello from webui"
                }))
                .expect("request"),
            )
            .await
            .expect_err("turn error maps to stable facade taxonomy");

        assert_eq!(err.code, expected_code, "{name}");
        assert_eq!(err.kind, expected_kind, "{name}");
        assert_eq!(err.status_code, expected_status, "{name}");
        assert_eq!(err.retryable, expected_retryable, "{name}");
    }
}

#[tokio::test]
async fn stream_events_without_projection_stream_maps_to_replay_unavailable_taxonomy() {
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    );
    create_thread_for(&services, caller(), "thread-alpha").await;

    let err = services
        .stream_events(
            caller(),
            RebornStreamEventsRequest {
                thread_id: "thread-alpha".to_string(),
                after_cursor: None,
            },
        )
        .await
        .expect_err("missing projection stream is replay unavailable");

    assert_eq!(err.code, RebornServicesErrorCode::Unavailable);
    assert_eq!(err.kind, RebornServicesErrorKind::ReplayUnavailable);
    assert_eq!(err.status_code, 503);
    assert!(!err.retryable);
}

#[tokio::test]
async fn adapter_authentication_maps_to_unauthenticated() {
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_event_stream(Arc::new(AuthFailureProjectionStream));
    // stream_events now ownership-probes the caller before draining; seed the
    // thread under the caller so the probe passes and the adapter auth error
    // is what the test observes.
    setup_owned_thread(&services, caller(), "thread-alpha").await;

    let err = services
        .stream_events(
            caller(),
            RebornStreamEventsRequest {
                thread_id: "thread-alpha".to_string(),
                after_cursor: None,
            },
        )
        .await
        .expect_err("adapter auth failure is unauthenticated");

    assert_eq!(err.code, RebornServicesErrorCode::Unauthenticated);
    assert_eq!(err.kind, RebornServicesErrorKind::ParticipantDenied);
    assert_eq!(err.status_code, 401);
}

#[tokio::test]
async fn projection_transient_maps_to_replay_unavailable_taxonomy() {
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_event_stream(Arc::new(StaticErrorProjectionStream {
        error: ProductAdapterError::WorkflowTransient {
            reason: RedactedString::new("provider stack trace with /host/path and secret-token"),
        },
    }));
    create_thread_for(&services, caller(), "thread-alpha").await;

    let err = services
        .stream_events(
            caller(),
            RebornStreamEventsRequest {
                thread_id: "thread-alpha".to_string(),
                after_cursor: None,
            },
        )
        .await
        .expect_err("projection transient is replay unavailable");

    assert_eq!(err.code, RebornServicesErrorCode::Unavailable);
    assert_eq!(err.kind, RebornServicesErrorKind::ReplayUnavailable);
    assert_eq!(err.status_code, 503);
    assert!(err.retryable);
    let rendered = format!("{err:?} {}", serde_json::to_string(&err).expect("json"));
    assert!(!rendered.contains("secret-token"));
    assert!(!rendered.contains("/host/path"));
    assert!(!rendered.contains("provider stack trace"));
}

#[tokio::test]
async fn projection_egress_denied_maps_to_blocked_resource_taxonomy() {
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_event_stream(Arc::new(StaticErrorProjectionStream {
        error: ProductAdapterError::EgressDenied {
            reason: RedactedString::new("denied api key secret-token"),
        },
    }));
    create_thread_for(&services, caller(), "thread-alpha").await;

    let err = services
        .stream_events(
            caller(),
            RebornStreamEventsRequest {
                thread_id: "thread-alpha".to_string(),
                after_cursor: None,
            },
        )
        .await
        .expect_err("blocked resource is stable taxonomy");

    assert_eq!(err.code, RebornServicesErrorCode::Forbidden);
    assert_eq!(err.kind, RebornServicesErrorKind::BlockedResource);
    assert_eq!(err.status_code, 403);
    let rendered = format!("{err:?} {}", serde_json::to_string(&err).expect("json"));
    assert!(!rendered.contains("secret-token"));
}

#[tokio::test]
async fn workflow_rejection_kinds_map_to_facade_taxonomy() {
    let cases = [
        (
            ProductWorkflowRejectionKind::ThreadBusy,
            409,
            RebornServicesErrorCode::Conflict,
            RebornServicesErrorKind::Busy,
        ),
        (
            ProductWorkflowRejectionKind::AdmissionRejected,
            429,
            RebornServicesErrorCode::RateLimited,
            RebornServicesErrorKind::Busy,
        ),
        (
            ProductWorkflowRejectionKind::ScopeNotFound,
            404,
            RebornServicesErrorCode::NotFound,
            RebornServicesErrorKind::NotFound,
        ),
        (
            ProductWorkflowRejectionKind::Unauthorized,
            403,
            RebornServicesErrorCode::Forbidden,
            RebornServicesErrorKind::ParticipantDenied,
        ),
        (
            ProductWorkflowRejectionKind::InvalidRequest,
            400,
            RebornServicesErrorCode::InvalidRequest,
            RebornServicesErrorKind::Validation,
        ),
        (
            ProductWorkflowRejectionKind::Unavailable,
            503,
            RebornServicesErrorCode::Unavailable,
            RebornServicesErrorKind::ReplayUnavailable,
        ),
        (
            ProductWorkflowRejectionKind::Conflict,
            409,
            RebornServicesErrorCode::Conflict,
            RebornServicesErrorKind::Conflict,
        ),
    ];

    for (workflow_kind, status_code, expected_code, expected_kind) in cases {
        let services = RebornServices::new(
            Arc::new(InMemorySessionThreadService::default()),
            Arc::new(FakeTurnCoordinator::default()),
        )
        .with_event_stream(Arc::new(StaticErrorProjectionStream {
            error: ProductAdapterError::WorkflowRejected {
                kind: workflow_kind,
                status_code,
                retryable: false,
                reason: RedactedString::new("internal workflow detail secret-token"),
            },
        }));
        create_thread_for(&services, caller(), "thread-alpha").await;

        let err = services
            .stream_events(
                caller(),
                RebornStreamEventsRequest {
                    thread_id: "thread-alpha".to_string(),
                    after_cursor: None,
                },
            )
            .await
            .expect_err("workflow rejection maps to stable facade taxonomy");

        assert_eq!(err.code, expected_code);
        assert_eq!(err.kind, expected_kind);
        assert_eq!(err.status_code, status_code);
        assert!(
            !serde_json::to_string(&err)
                .expect("json")
                .contains("secret-token")
        );
    }
}

#[tokio::test]
async fn timeline_backend_failure_maps_to_timeline_unavailable_taxonomy() {
    let services = RebornServices::new(
        Arc::new(ScriptedThreadService::backend_history()),
        Arc::new(FakeTurnCoordinator::default()),
    );

    let err = services
        .get_timeline(
            caller(),
            RebornTimelineRequest {
                thread_id: "thread-alpha".to_string(),
                ..Default::default()
            },
        )
        .await
        .expect_err("timeline backend failure is stable unavailable taxonomy");

    assert_eq!(err.code, RebornServicesErrorCode::Unavailable);
    assert_eq!(err.kind, RebornServicesErrorKind::TimelineUnavailable);
    assert_eq!(err.status_code, 503);
    assert!(err.retryable);
    let rendered = format!("{err:?} {}", serde_json::to_string(&err).expect("json"));
    assert!(!rendered.contains("secret-token"));
    assert!(!rendered.contains("/host/path"));
    assert!(!rendered.contains("backend detail"));
}

#[tokio::test]
async fn cancel_run_uses_turn_facade_and_stable_response() {
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        coordinator.clone(),
    );
    create_thread_for(&services, caller(), "thread-alpha").await;

    let response = services
        .cancel_run(
            caller(),
            serde_json::from_value::<WebUiCancelRunRequest>(json!({
                "client_action_id": "cancel-1",
                "thread_id": "thread-alpha",
                "run_id": run_id_string(),
                "reason": "user_requested"
            }))
            .expect("request"),
        )
        .await
        .expect("cancel succeeds");

    assert_eq!(response.status, TurnStatus::Cancelled);
    assert_eq!(response.event_cursor, EventCursor(13));
    assert!(!response.already_terminal);
    assert_eq!(coordinator.cancellation_count(), 1);
}

#[tokio::test]
async fn approved_gate_resolution_resumes_turn() {
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        coordinator.clone(),
    );
    create_thread_for(&services, caller(), "thread-alpha").await;

    let response = services
        .resolve_gate(
            caller(),
            serde_json::from_value::<WebUiResolveGateRequest>(json!({
                "client_action_id": "gate-1",
                "thread_id": "thread-alpha",
                "run_id": run_id_string(),
                "gate_ref": "gate-alpha",
                "resolution": "approved"
            }))
            .expect("request"),
        )
        .await
        .expect("gate resolution succeeds");

    assert!(matches!(response, RebornResolveGateResponse::Resumed(_)));
    assert_eq!(coordinator.resumption_count(), 1);
    assert_eq!(
        coordinator.last_resumption_precondition(),
        Some(ResumeTurnPrecondition::AnyBlockedGate)
    );
    assert!(
        coordinator
            .last_resumption_source_binding_ref()
            .expect("resume source binding")
            .contains("gate-alpha")
    );
}

#[tokio::test]
async fn resolve_gate_rejects_missing_run_state_actor() {
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        coordinator.clone(),
    );
    create_thread_for(&services, caller(), "thread-alpha").await;
    coordinator.set_parked_gate(GateRef::new("gate-alpha").expect("gate"));
    coordinator.set_run_state_actor(None);

    let err = services
        .resolve_gate(
            caller(),
            serde_json::from_value::<WebUiResolveGateRequest>(json!({
                "client_action_id": "gate-missing-actor",
                "thread_id": "thread-alpha",
                "run_id": run_id_string(),
                "gate_ref": "gate-alpha",
                "resolution": "denied"
            }))
            .expect("request"),
        )
        .await
        .expect_err("missing run-state actor must fail closed");

    assert_eq!(err.code, RebornServicesErrorCode::Forbidden);
    assert_eq!(err.kind, RebornServicesErrorKind::ParticipantDenied);
    assert_eq!(err.status_code, 403);
    assert_eq!(coordinator.cancellation_count(), 0);
}

#[tokio::test]
async fn resolve_gate_rejects_mismatched_run_state_actor() {
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        coordinator.clone(),
    );
    create_thread_for(&services, caller(), "thread-alpha").await;
    coordinator.set_parked_gate(GateRef::new("gate-alpha").expect("gate"));
    coordinator.set_run_state_actor(Some(turn_actor_for_user("user-beta")));

    let err = services
        .resolve_gate(
            caller(),
            serde_json::from_value::<WebUiResolveGateRequest>(json!({
                "client_action_id": "gate-mismatched-actor",
                "thread_id": "thread-alpha",
                "run_id": run_id_string(),
                "gate_ref": "gate-alpha",
                "resolution": "denied"
            }))
            .expect("request"),
        )
        .await
        .expect_err("mismatched run-state actor must be rejected");

    assert_eq!(err.code, RebornServicesErrorCode::Forbidden);
    assert_eq!(err.kind, RebornServicesErrorKind::ParticipantDenied);
    assert_eq!(err.status_code, 403);
    assert_eq!(coordinator.cancellation_count(), 0);
}

#[tokio::test]
async fn generic_gate_resolution_rejects_blocked_auth_run() {
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        coordinator.clone(),
    );
    create_thread_for(&services, caller(), "thread-alpha").await;
    coordinator.set_parked_auth_gate(GateRef::new("custom-auth-gate").expect("gate"));

    let err = services
        .resolve_gate(
            caller(),
            serde_json::from_value::<WebUiResolveGateRequest>(json!({
                "client_action_id": "gate-auth-fallback",
                "thread_id": "thread-alpha",
                "run_id": run_id_string(),
                "gate_ref": "custom-auth-gate",
                "resolution": "approved"
            }))
            .expect("request"),
        )
        .await
        .expect_err("generic resolver must not resume auth gate");

    assert_eq!(err.code, RebornServicesErrorCode::Unavailable);
    assert_eq!(err.kind, RebornServicesErrorKind::BlockedAuthentication);
    assert_eq!(coordinator.resumption_count(), 0);
}

#[tokio::test]
async fn blocked_auth_run_routes_non_prefixed_gate_to_auth_interaction_service() {
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let auth_interactions = Arc::new(RecordingAuthInteractionService::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        coordinator.clone(),
    )
    .with_auth_interactions(auth_interactions.clone());
    create_thread_for(&services, caller(), "thread-alpha").await;
    coordinator.set_parked_auth_gate(GateRef::new("custom-auth-gate").expect("gate"));

    let response = services
        .resolve_gate(
            caller(),
            serde_json::from_value::<WebUiResolveGateRequest>(json!({
                "client_action_id": "gate-auth-state-routed",
                "thread_id": "thread-alpha",
                "run_id": run_id_string(),
                "gate_ref": "custom-auth-gate",
                "resolution": "denied"
            }))
            .expect("request"),
        )
        .await
        .expect("blocked auth status routes to auth interaction service");

    assert!(matches!(response, RebornResolveGateResponse::Cancelled(_)));
    assert_eq!(auth_interactions.resolution_count(), 1);
    let resolution = auth_interactions.last_resolution().expect("resolution");
    assert_eq!(resolution.gate_ref.as_str(), "custom-auth-gate");
    assert_eq!(resolution.decision, AuthInteractionDecision::Deny);
    assert_eq!(coordinator.cancellation_count(), 0);
}

#[tokio::test]
async fn blocked_auth_run_with_stale_gate_ref_returns_conflict() {
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let auth_interactions = Arc::new(RecordingAuthInteractionService::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        coordinator.clone(),
    )
    .with_auth_interactions(auth_interactions.clone());
    create_thread_for(&services, caller(), "thread-alpha").await;
    coordinator.set_parked_auth_gate(GateRef::new("gate-current").expect("gate"));

    let err = services
        .resolve_gate(
            caller(),
            serde_json::from_value::<WebUiResolveGateRequest>(json!({
                "client_action_id": "gate-auth-stale",
                "thread_id": "thread-alpha",
                "run_id": run_id_string(),
                "gate_ref": "gate-stale",
                "resolution": "approved"
            }))
            .expect("request"),
        )
        .await
        .expect_err("stale auth gate_ref must produce Conflict");

    assert_eq!(err.code, RebornServicesErrorCode::Conflict);
    assert_eq!(err.kind, RebornServicesErrorKind::BlockedAuthentication);
    assert_eq!(err.status_code, 409);
    assert_eq!(coordinator.resumption_count(), 0);
    assert_eq!(coordinator.cancellation_count(), 0);
    assert_eq!(auth_interactions.resolution_count(), 0);
}

#[tokio::test]
async fn blocked_approval_run_routes_non_prefixed_gate_to_approval_interaction_service() {
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let approval_interactions = Arc::new(RecordingApprovalInteractionService::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        coordinator.clone(),
    )
    .with_approval_interactions(approval_interactions.clone());
    create_thread_for(&services, caller(), "thread-alpha").await;
    coordinator.set_parked_approval_gate(GateRef::new("custom-approval-gate").expect("gate"));

    let response = services
        .resolve_gate(
            caller(),
            serde_json::from_value::<WebUiResolveGateRequest>(json!({
                "client_action_id": "gate-approval-state-routed",
                "thread_id": "thread-alpha",
                "run_id": run_id_string(),
                "gate_ref": "custom-approval-gate",
                "resolution": "approved"
            }))
            .expect("request"),
        )
        .await
        .expect("blocked approval status routes to approval interaction service");

    assert!(matches!(response, RebornResolveGateResponse::Resumed(_)));
    assert_eq!(approval_interactions.resolution_count(), 1);
    let resolution = approval_interactions.last_resolution().expect("resolution");
    assert_eq!(resolution.gate_ref.as_str(), "custom-approval-gate");
    assert_eq!(
        resolution.decision,
        ApprovalInteractionDecision::ApproveOnce
    );
    assert_eq!(coordinator.resumption_count(), 0);
}

#[tokio::test]
async fn blocked_approval_run_with_stale_gate_ref_returns_conflict() {
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let approval_interactions = Arc::new(RecordingApprovalInteractionService::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        coordinator.clone(),
    )
    .with_approval_interactions(approval_interactions.clone());
    create_thread_for(&services, caller(), "thread-alpha").await;
    coordinator.set_parked_approval_gate(GateRef::new("gate-current").expect("gate"));

    let err = services
        .resolve_gate(
            caller(),
            serde_json::from_value::<WebUiResolveGateRequest>(json!({
                "client_action_id": "gate-approval-stale",
                "thread_id": "thread-alpha",
                "run_id": run_id_string(),
                "gate_ref": "gate-stale",
                "resolution": "denied"
            }))
            .expect("request"),
        )
        .await
        .expect_err("stale approval gate_ref must produce Conflict");

    assert_eq!(err.code, RebornServicesErrorCode::Conflict);
    assert_eq!(err.kind, RebornServicesErrorKind::BlockedApproval);
    assert_eq!(err.status_code, 409);
    assert_eq!(coordinator.resumption_count(), 0);
    assert_eq!(coordinator.cancellation_count(), 0);
    assert_eq!(approval_interactions.resolution_count(), 0);
}

#[tokio::test]
async fn terminal_run_state_rejects_gate_resolution_before_shape_fallback() {
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let approval_interactions = Arc::new(RecordingApprovalInteractionService::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        coordinator.clone(),
    )
    .with_approval_interactions(approval_interactions.clone());
    create_thread_for(&services, caller(), "thread-alpha").await;
    coordinator.set_run_state_status(TurnStatus::Completed);
    let gate_ref = approval_gate_ref(ApprovalRequestId::new()).expect("approval gate ref");

    let err = services
        .resolve_gate(
            caller(),
            serde_json::from_value::<WebUiResolveGateRequest>(json!({
                "client_action_id": "gate-terminal",
                "thread_id": "thread-alpha",
                "run_id": run_id_string(),
                "gate_ref": gate_ref.as_str(),
                "resolution": "approved"
            }))
            .expect("request"),
        )
        .await
        .expect_err("terminal run must fail closed before shape fallback");

    assert_eq!(err.code, RebornServicesErrorCode::Conflict);
    assert_eq!(err.kind, RebornServicesErrorKind::Conflict);
    assert_eq!(err.status_code, 409);
    assert_eq!(coordinator.resumption_count(), 0);
    assert_eq!(coordinator.cancellation_count(), 0);
    assert_eq!(approval_interactions.resolution_count(), 0);
}

#[tokio::test]
async fn approval_gate_resolution_uses_approval_interaction_service() {
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let approval_interactions = Arc::new(RecordingApprovalInteractionService::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        coordinator.clone(),
    )
    .with_approval_interactions(approval_interactions.clone());
    create_thread_for(&services, caller(), "thread-alpha").await;
    let gate_ref = approval_gate_ref(ApprovalRequestId::new()).expect("approval gate ref");

    let response = services
        .resolve_gate(
            caller(),
            serde_json::from_value::<WebUiResolveGateRequest>(json!({
                "client_action_id": "approval-gate-1",
                "thread_id": "thread-alpha",
                "run_id": run_id_string(),
                "gate_ref": gate_ref.as_str(),
                "resolution": "approved"
            }))
            .expect("request"),
        )
        .await
        .expect("approval gate resolution succeeds");

    assert!(matches!(response, RebornResolveGateResponse::Resumed(_)));
    assert_eq!(approval_interactions.resolution_count(), 1);
    assert_eq!(coordinator.resumption_count(), 0);
    assert_eq!(
        approval_interactions
            .last_resolution()
            .expect("resolution")
            .gate_ref,
        gate_ref
    );
}

#[tokio::test]
async fn approval_gate_denial_uses_approval_interaction_service_and_returns_cancelled() {
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let approval_interactions = Arc::new(RecordingApprovalInteractionService::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        coordinator.clone(),
    )
    .with_approval_interactions(approval_interactions.clone());
    create_thread_for(&services, caller(), "thread-alpha").await;
    let gate_ref = approval_gate_ref(ApprovalRequestId::new()).expect("approval gate ref");

    let response = services
        .resolve_gate(
            caller(),
            serde_json::from_value::<WebUiResolveGateRequest>(json!({
                "client_action_id": "approval-gate-deny",
                "thread_id": "thread-alpha",
                "run_id": run_id_string(),
                "gate_ref": gate_ref.as_str(),
                "resolution": "denied"
            }))
            .expect("request"),
        )
        .await
        .expect("approval gate denial succeeds");

    assert!(matches!(response, RebornResolveGateResponse::Cancelled(_)));
    assert_eq!(approval_interactions.resolution_count(), 1);
    assert_eq!(coordinator.cancellation_count(), 0);
    assert_eq!(
        approval_interactions
            .last_resolution()
            .expect("resolution")
            .decision,
        ApprovalInteractionDecision::Deny
    );
}

#[tokio::test]
async fn credential_gate_resolution_returns_sanitized_stable_error_until_gate_port_exists() {
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        coordinator.clone(),
    );
    create_thread_for(&services, caller(), "thread-alpha").await;
    let credential_ref = CredentialAccountId::new();

    let err = services
        .resolve_gate(
            caller(),
            serde_json::from_value::<WebUiResolveGateRequest>(json!({
                "client_action_id": "gate-credential",
                "thread_id": "thread-alpha",
                "run_id": run_id_string(),
                "gate_ref": "gate-alpha",
                "resolution": "credential_provided",
                "credential_ref": credential_ref.to_string()
            }))
            .expect("request"),
        )
        .await
        .expect_err("credential resolution is not wired yet");

    assert_eq!(err.code, RebornServicesErrorCode::Unavailable);
    assert_eq!(err.kind, RebornServicesErrorKind::BlockedAuthentication);
    assert_eq!(err.status_code, 503);
    assert_eq!(coordinator.resumption_count(), 0);
    let rendered = format!("{err:?} {}", serde_json::to_string(&err).expect("json"));
    assert!(!rendered.contains(credential_ref.to_string().as_str()));
}

#[tokio::test]
async fn auth_gate_credential_resolution_uses_auth_interaction_service() {
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let auth_interactions = Arc::new(RecordingAuthInteractionService::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        coordinator.clone(),
    )
    .with_auth_interactions(auth_interactions.clone());
    create_thread_for(&services, caller(), "thread-alpha").await;
    let credential_ref = CredentialAccountId::new();

    let response = services
        .resolve_gate(
            caller(),
            serde_json::from_value::<WebUiResolveGateRequest>(json!({
                "client_action_id": "gate-credential",
                "thread_id": "thread-alpha",
                "run_id": run_id_string(),
                "gate_ref": "gate:auth-alpha",
                "resolution": "credential_provided",
                "credential_ref": credential_ref.to_string()
            }))
            .expect("request"),
        )
        .await
        .expect("credential resolution routes through auth interaction service");

    assert!(matches!(response, RebornResolveGateResponse::Resumed(_)));
    assert_eq!(auth_interactions.resolution_count(), 1);
    let resolution = auth_interactions.last_resolution().expect("resolution");
    assert_eq!(resolution.gate_ref.as_str(), "gate:auth-alpha");
    assert_eq!(
        resolution.decision,
        AuthInteractionDecision::CredentialProvided { credential_ref }
    );
    assert_eq!(coordinator.resumption_count(), 0);
}

#[tokio::test]
async fn hook_auth_gate_denial_uses_auth_interaction_service() {
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let auth_interactions = Arc::new(RecordingAuthInteractionService::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        coordinator.clone(),
    )
    .with_auth_interactions(auth_interactions.clone());
    create_thread_for(&services, caller(), "thread-alpha").await;

    let response = services
        .resolve_gate(
            caller(),
            serde_json::from_value::<WebUiResolveGateRequest>(json!({
                "client_action_id": "gate-auth-deny",
                "thread_id": "thread-alpha",
                "run_id": run_id_string(),
                "gate_ref": "gate:hook-auth-alpha",
                "resolution": "denied"
            }))
            .expect("request"),
        )
        .await
        .expect("auth denial routes through auth interaction service");

    assert!(matches!(response, RebornResolveGateResponse::Cancelled(_)));
    assert_eq!(auth_interactions.resolution_count(), 1);
    let resolution = auth_interactions.last_resolution().expect("resolution");
    assert_eq!(resolution.gate_ref.as_str(), "gate:hook-auth-alpha");
    assert_eq!(resolution.decision, AuthInteractionDecision::Deny);
    assert_eq!(coordinator.cancellation_count(), 0);
}

#[tokio::test]
async fn missing_run_state_for_auth_gate_still_routes_to_auth_interaction_service() {
    let coordinator = Arc::new(FakeTurnCoordinator::with_run_state_error(
        TurnError::ScopeNotFound,
    ));
    let auth_interactions = Arc::new(RecordingAuthInteractionService::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        coordinator.clone(),
    )
    .with_auth_interactions(auth_interactions.clone());
    create_thread_for(&services, caller(), "thread-alpha").await;

    let response = services
        .resolve_gate(
            caller(),
            serde_json::from_value::<WebUiResolveGateRequest>(json!({
                "client_action_id": "gate-auth-missing-run",
                "thread_id": "thread-alpha",
                "run_id": run_id_string(),
                "gate_ref": "gate:hook-auth-missing",
                "resolution": "denied"
            }))
            .expect("request"),
        )
        .await
        .expect("typed auth gate routes to auth interaction service when run-state is gone");

    assert!(matches!(response, RebornResolveGateResponse::Cancelled(_)));
    assert_eq!(auth_interactions.resolution_count(), 1);
    assert_eq!(
        auth_interactions
            .last_resolution()
            .expect("resolution")
            .gate_ref
            .as_str(),
        "gate:hook-auth-missing"
    );
    assert_eq!(coordinator.cancellation_count(), 0);
}

#[tokio::test]
async fn denied_gate_resolution_cancels_run() {
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        coordinator.clone(),
    );
    create_thread_for(&services, caller(), "thread-alpha").await;
    coordinator.set_parked_gate(GateRef::new("gate-alpha").expect("gate"));

    let response = services
        .resolve_gate(
            caller(),
            serde_json::from_value::<WebUiResolveGateRequest>(json!({
                "client_action_id": "gate-2",
                "thread_id": "thread-alpha",
                "run_id": run_id_string(),
                "gate_ref": "gate-alpha",
                "resolution": "denied"
            }))
            .expect("request"),
        )
        .await
        .expect("gate denial succeeds");

    assert!(matches!(response, RebornResolveGateResponse::Cancelled(_)));
    assert_eq!(coordinator.cancellation_count(), 1);
}

// Regression: cancel_run must reject when the authenticated user does not own
// the thread. TurnScope only carries (tenant, agent, project, thread_id), so
// without this check any caller sharing an agent scope could cancel another
// user's run by guessing the run_id.
#[tokio::test]
async fn cancel_run_rejects_cross_user_access() {
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        coordinator.clone(),
    );
    let alice = caller();
    create_thread_for(&services, alice.clone(), "thread-alice").await;

    // Bob shares Alice's (tenant, agent, project) scope and guesses her thread.
    let bob = WebUiAuthenticatedCaller::new(
        TenantId::new("tenant-alpha").expect("tenant"),
        UserId::new("user-bob").expect("user"),
        alice.agent_id.clone(),
        alice.project_id.clone(),
    );

    let err = services
        .cancel_run(
            bob,
            serde_json::from_value::<WebUiCancelRunRequest>(json!({
                "client_action_id": "cancel-cross",
                "thread_id": "thread-alice",
                "run_id": run_id_string(),
                "reason": "user_requested"
            }))
            .expect("request"),
        )
        .await
        .expect_err("cross-user cancel must be rejected");

    // 404 rather than 403 so the existence of Alice's thread is not leaked.
    assert_eq!(err.code, RebornServicesErrorCode::NotFound);
    assert_eq!(err.status_code, 404);
    assert_eq!(
        coordinator.cancellation_count(),
        0,
        "turn coordinator must NOT be called for cross-user cancel"
    );
}

// Regression: the ownership probe must collapse `ThreadScopeMismatch` and
// `UnknownThread` into the same NotFound response. Current backends return
// `UnknownThread` for `list_thread_history` scope mismatches, but the contract
// also permits `ThreadScopeMismatch`; if a future backend change starts
// emitting it, the default `map_thread_error` path would surface 409 Conflict
// instead, signalling to attackers that the thread exists under a different
// owner. Lock in the explicit remap by driving cancel_run through a stub that
// always returns `ThreadScopeMismatch`.
#[tokio::test]
async fn cancel_run_remaps_thread_scope_mismatch_to_not_found() {
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let services = RebornServices::new(Arc::new(ScopeMismatchThreadStub), coordinator.clone());

    let err = services
        .cancel_run(
            caller(),
            serde_json::from_value::<WebUiCancelRunRequest>(json!({
                "client_action_id": "cancel-scope-mismatch",
                "thread_id": "thread-alpha",
                "run_id": run_id_string(),
                "reason": "user_requested"
            }))
            .expect("request"),
        )
        .await
        .expect_err("scope mismatch must surface as NotFound");

    assert_eq!(err.code, RebornServicesErrorCode::NotFound);
    assert_eq!(err.status_code, 404);
    assert_eq!(
        coordinator.cancellation_count(),
        0,
        "turn coordinator must NOT be called when ownership probe fails"
    );
}

// Regression: resolve_gate must reject when the authenticated user does not
// own the thread, for both the approve→resume path and the deny/cancel path.
#[tokio::test]
async fn resolve_gate_rejects_cross_user_access() {
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        coordinator.clone(),
    );
    let alice = caller();
    create_thread_for(&services, alice.clone(), "thread-alice").await;
    coordinator.set_parked_gate(GateRef::new("gate-alpha").expect("gate"));

    let bob = WebUiAuthenticatedCaller::new(
        TenantId::new("tenant-alpha").expect("tenant"),
        UserId::new("user-bob").expect("user"),
        alice.agent_id.clone(),
        alice.project_id.clone(),
    );

    let err = services
        .resolve_gate(
            bob,
            serde_json::from_value::<WebUiResolveGateRequest>(json!({
                "client_action_id": "gate-cross",
                "thread_id": "thread-alice",
                "run_id": run_id_string(),
                "gate_ref": "gate-alpha",
                "resolution": "denied"
            }))
            .expect("request"),
        )
        .await
        .expect_err("cross-user gate resolution must be rejected");

    assert_eq!(err.code, RebornServicesErrorCode::NotFound);
    assert_eq!(err.status_code, 404);
    assert_eq!(
        coordinator.cancellation_count(),
        0,
        "turn coordinator must NOT be called for cross-user resolve"
    );
}

// Regression: stream_events shares the TurnScope shape with cancel_run /
// resolve_gate / get_run_state — none of which carry owner_user_id — so the
// projection drain must be gated by the same ownership probe. Without it, a
// caller who shares the (tenant, agent, project) scope could read another
// user's projection feed by guessing thread_id.
#[tokio::test]
async fn stream_events_rejects_cross_user_access() {
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let event_stream = Arc::new(RecordingProjectionStream::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        coordinator.clone(),
    )
    .with_event_stream(event_stream.clone());
    let alice = caller();
    setup_owned_thread(&services, alice.clone(), "thread-alice").await;

    let bob = WebUiAuthenticatedCaller::new(
        TenantId::new("tenant-alpha").expect("tenant"),
        UserId::new("user-bob").expect("user"),
        alice.agent_id.clone(),
        alice.project_id.clone(),
    );

    let err = services
        .stream_events(
            bob,
            RebornStreamEventsRequest {
                thread_id: "thread-alice".to_string(),
                after_cursor: None,
            },
        )
        .await
        .expect_err("cross-user stream_events must be rejected");

    assert_eq!(err.code, RebornServicesErrorCode::NotFound);
    assert_eq!(err.status_code, 404);
    assert_eq!(
        event_stream.drain_count(),
        0,
        "projection stream must NOT be drained when ownership probe fails"
    );
}

// Regression: when create_thread is given an explicit `requested_thread_id`,
// a thread that already exists under a different owner would surface as
// `ThreadScopeMismatch` → `409 Conflict` via `map_thread_error`. That gives
// any caller sharing the (tenant, agent, project) scope an existence oracle
// for thread ids they did not create. Explicit-id collisions must redact to
// the same `NotFound` outcome as the cancel_run / resolve_gate / stream_events
// ownership probe. The auto-generated path keeps `map_thread_error` since the
// caller cannot usefully probe deterministically-derived UUIDv5 ids.
#[tokio::test]
async fn create_thread_explicit_id_collision_remaps_to_not_found() {
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    );
    let alice = caller();
    setup_owned_thread(&services, alice.clone(), "thread-alice").await;

    let bob = WebUiAuthenticatedCaller::new(
        TenantId::new("tenant-alpha").expect("tenant"),
        UserId::new("user-bob").expect("user"),
        alice.agent_id.clone(),
        alice.project_id.clone(),
    );

    let err = services
        .create_thread(
            bob,
            serde_json::from_value::<WebUiCreateThreadRequest>(json!({
                "client_action_id": "create-cross",
                "requested_thread_id": "thread-alice",
            }))
            .expect("request"),
        )
        .await
        .expect_err("cross-user create_thread with explicit id must be rejected");

    assert_eq!(err.code, RebornServicesErrorCode::NotFound);
    assert_eq!(err.status_code, 404);
}

// Regression: cancel_run is not gate-aware, so without a parked-on-gate check
// a denied/cancelled resolution carrying a stale or attacker-supplied gate_ref
// would cancel any non-terminal run with the matching run_id. Mismatched gate
// must produce Conflict and cancel_run must never be invoked.
#[tokio::test]
async fn denied_gate_resolution_with_stale_gate_ref_returns_conflict() {
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        coordinator.clone(),
    );
    create_thread_for(&services, caller(), "thread-alpha").await;
    // The run is parked on `gate-current`, but the browser supplies `gate-stale`.
    coordinator.set_parked_gate(GateRef::new("gate-current").expect("gate"));

    let err = services
        .resolve_gate(
            caller(),
            serde_json::from_value::<WebUiResolveGateRequest>(json!({
                "client_action_id": "gate-stale",
                "thread_id": "thread-alpha",
                "run_id": run_id_string(),
                "gate_ref": "gate-stale",
                "resolution": "denied"
            }))
            .expect("request"),
        )
        .await
        .expect_err("stale gate_ref must produce Conflict, not silent cancel");

    assert_eq!(err.code, RebornServicesErrorCode::Conflict);
    assert_eq!(err.kind, RebornServicesErrorKind::BlockedApproval);
    assert_eq!(err.status_code, 409);
    assert_eq!(
        coordinator.cancellation_count(),
        0,
        "cancel_run must NOT be called for stale gate_ref"
    );
}

// Regression: `Approved { always: true }` requests a persistent approval which
// this facade cannot honor (no approval-policy port). Reject as Unavailable
// instead of silently downgrading to one-shot.
#[tokio::test]
async fn approved_gate_resolution_with_persistent_flag_is_rejected() {
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        coordinator.clone(),
    );
    create_thread_for(&services, caller(), "thread-alpha").await;

    let err = services
        .resolve_gate(
            caller(),
            serde_json::from_value::<WebUiResolveGateRequest>(json!({
                "client_action_id": "gate-always",
                "thread_id": "thread-alpha",
                "run_id": run_id_string(),
                "gate_ref": "gate-alpha",
                "resolution": "approved",
                "always": true,
            }))
            .expect("request"),
        )
        .await
        .expect_err("persistent approval must be rejected");

    assert_eq!(err.code, RebornServicesErrorCode::Unavailable);
    assert_eq!(err.kind, RebornServicesErrorKind::BlockedApproval);
    assert_eq!(err.status_code, 503);
    assert_eq!(
        coordinator.resumption_count(),
        0,
        "resume_turn must NOT be called for unsupported persistent approval"
    );
}

#[tokio::test]
async fn approval_gate_resolution_with_persistent_flag_is_rejected_without_approval_interaction() {
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let approval_interactions = Arc::new(RecordingApprovalInteractionService::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        coordinator.clone(),
    )
    .with_approval_interactions(approval_interactions.clone());
    create_thread_for(&services, caller(), "thread-alpha").await;
    let gate_ref = approval_gate_ref(ApprovalRequestId::new()).expect("approval gate ref");

    let err = services
        .resolve_gate(
            caller(),
            serde_json::from_value::<WebUiResolveGateRequest>(json!({
                "client_action_id": "approval-gate-always",
                "thread_id": "thread-alpha",
                "run_id": run_id_string(),
                "gate_ref": gate_ref.as_str(),
                "resolution": "approved",
                "always": true,
            }))
            .expect("request"),
        )
        .await
        .expect_err("persistent approval must be rejected");

    assert_eq!(err.code, RebornServicesErrorCode::Unavailable);
    assert_eq!(err.kind, RebornServicesErrorKind::BlockedApproval);
    assert_eq!(err.status_code, 503);
    assert_eq!(approval_interactions.resolution_count(), 0);
    assert_eq!(coordinator.resumption_count(), 0);
}

#[tokio::test]
async fn setup_extension_projects_through_configured_lifecycle_facade() {
    let lifecycle_facade = Arc::new(RecordingLifecycleFacade::new());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_lifecycle_product_facade(lifecycle_facade.clone());

    let response = services
        .setup_extension(
            caller(),
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "github")
                .expect("valid package ref"),
            WebUiSetupExtensionRequest::default(),
        )
        .await
        .expect("setup extension response");

    assert_eq!(
        response.package_ref,
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, "github")
            .expect("valid package ref")
    );
    assert_eq!(response.phase, LifecyclePhase::UnsupportedOrLegacy);
    assert!(response.blockers.iter().any(|blocker| matches!(
        blocker,
        LifecycleReadinessBlocker::Runtime { ref_id: Some(ref_id) }
            if ref_id.as_str() == "extension_lifecycle_store_unwired"
    )));
    assert_eq!(
        lifecycle_facade.package_refs(),
        vec![
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "github")
                .expect("valid package ref")
        ]
    );
}

#[tokio::test]
async fn list_extensions_projects_onboarding_payload_through_reborn_services() {
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_lifecycle_product_facade(Arc::new(ListingLifecycleFacade {
        extension: LifecycleInstalledExtensionSummary {
            summary: extension_summary(
                "github",
                vec![manual_credential_requirement("github_runtime_token", true)],
                Some(onboarding_fixture()),
            ),
            phase: LifecyclePhase::Installed,
        },
    }));

    let response = services
        .list_extensions(caller())
        .await
        .expect("extension list response");
    let extension = response.extensions.first().expect("one extension");

    assert_eq!(extension.tools, vec!["github.read", "github.write"]);
    assert_eq!(
        extension.onboarding_state,
        Some(RebornExtensionOnboardingState::SetupRequired)
    );
    let onboarding = extension.onboarding.as_ref().expect("onboarding payload");
    assert_eq!(
        onboarding.credential_instructions.as_deref(),
        Some("Paste the GitHub token IronClaw should use.")
    );
    assert_eq!(
        onboarding.credential_next_step.as_deref(),
        Some("After saving the token, activate GitHub to publish its tools.")
    );
}

#[tokio::test]
async fn list_automation_dispatches_through_product_facade() {
    let automation_facade = Arc::new(RecordingAutomationFacade::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_automation_product_facade(automation_facade.clone());

    let listed = services
        .list_automations(
            caller(),
            WebUiListAutomationsRequest {
                limit: Some(10),
                run_limit: None,
            },
        )
        .await
        .expect("list automations");
    assert_eq!(listed.automations.len(), 1);
    assert_eq!(listed.automations[0].automation_id, "trigger-listed");
    assert_eq!(
        listed.automations[0].source,
        RebornAutomationSource::Schedule {
            cron: "0 9 * * *".to_string()
        }
    );
    assert_eq!(listed.automations[0].state, RebornAutomationState::Active);
    assert_eq!(
        listed.automations[0].last_status,
        Some(RebornAutomationRunStatus::Ok)
    );
    assert_eq!(listed.automations[0].recent_runs.len(), 1);
    assert_eq!(
        listed.automations[0].recent_runs[0].status,
        RebornAutomationRecentRunStatus::Ok
    );
    assert_eq!(
        listed.automations[0].recent_runs[0].thread_id.as_str(),
        "thread-listed"
    );

    let list_calls = automation_facade.list_calls();
    assert_eq!(list_calls.len(), 1);
    assert_eq!(list_calls[0].caller.user_id.as_str(), "user-alpha");
    assert_eq!(list_calls[0].caller.agent_id.as_str(), "agent-alpha");
    assert_eq!(
        list_calls[0]
            .caller
            .project_id
            .as_ref()
            .map(ProjectId::as_str),
        Some("project-alpha")
    );
    assert_eq!(list_calls[0].limit, 10);
    assert_eq!(
        list_calls[0].run_limit, AUTOMATION_RUN_HISTORY_DEFAULT_PAGE_SIZE as usize,
        "omitted automation run history limit must use AUTOMATION_RUN_HISTORY_DEFAULT_PAGE_SIZE ({})",
        AUTOMATION_RUN_HISTORY_DEFAULT_PAGE_SIZE
    );
}

#[tokio::test]
async fn list_connectable_channels_unwired_returns_empty_list() {
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    );

    let response = services
        .list_connectable_channels(caller())
        .await
        .expect("connectable channels response");

    assert!(response.channels.is_empty());
}

#[tokio::test]
async fn list_connectable_channels_returns_configured_action_metadata() {
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_connectable_channels_facade(Arc::new(StaticConnectableChannelsProductFacade::new(vec![
        RebornConnectableChannelInfo {
            channel: "slack".to_string(),
            display_name: "Slack".to_string(),
            strategy: RebornChannelConnectStrategy::InboundProofCode,
            action: RebornChannelConnectAction {
                title: "Slack account connection".to_string(),
                instructions: "Message the Slack app, then enter the code here.".to_string(),
                input_placeholder: "Enter Slack pairing code...".to_string(),
                submit_label: "Connect".to_string(),
                success_message: "Slack account connected.".to_string(),
                error_message: "Invalid or expired Slack pairing code.".to_string(),
            },
            command_aliases: vec!["slack".to_string(), "slack account".to_string()],
        },
    ])));

    let response = services
        .list_connectable_channels(caller())
        .await
        .expect("connectable channels response");

    let channel = response.channels.first().expect("configured channel");
    assert_eq!(channel.channel, "slack");
    assert_eq!(channel.display_name, "Slack");
    assert_eq!(
        channel.strategy,
        RebornChannelConnectStrategy::InboundProofCode
    );
    assert_eq!(
        channel.action.instructions,
        "Message the Slack app, then enter the code here."
    );
    assert_eq!(
        channel.command_aliases,
        vec!["slack".to_string(), "slack account".to_string()]
    );
}

#[test]
fn channel_connect_action_serializes_neutral_input_placeholder_and_accepts_legacy_code_placeholder()
{
    let action = RebornChannelConnectAction {
        title: "Slack channel access".to_string(),
        instructions: "Choose allowed channels.".to_string(),
        input_placeholder: "C0123456789".to_string(),
        submit_label: "Save channels".to_string(),
        success_message: "Slack channels saved.".to_string(),
        error_message: "Slack channel update failed.".to_string(),
    };

    let serialized = serde_json::to_value(&action).expect("action serializes");
    assert_eq!(serialized["input_placeholder"], "C0123456789");
    assert!(serialized.get("code_placeholder").is_none());

    let legacy: RebornChannelConnectAction = serde_json::from_value(serde_json::json!({
        "title": "Slack account connection",
        "instructions": "Message the Slack app, then enter the code here.",
        "code_placeholder": "Enter Slack pairing code...",
        "submit_label": "Connect",
        "success_message": "Slack account connected.",
        "error_message": "Invalid or expired Slack pairing code."
    }))
    .expect("legacy action deserializes");
    assert_eq!(legacy.input_placeholder, "Enter Slack pairing code...");
}

#[tokio::test]
async fn get_outbound_preferences_unwired_returns_empty_projection() {
    // arch-exempt: large_file, outbound pref tests belong at API seam, plan docs/plans/2026-06-05-trigger-delivery-default-outbound-e2e-plan.md.
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    );

    let response = services
        .get_outbound_preferences(caller())
        .await
        .expect("default outbound preferences");

    assert!(response.final_reply_target.is_none());
    assert_eq!(
        response.default_modality,
        RebornOutboundDeliveryModality::Text
    );
}

#[test]
fn outbound_delivery_modality_text_round_trips_as_text() {
    let serialized = serde_json::to_value(RebornOutboundDeliveryModality::Text)
        .expect("serialize text modality");
    assert_eq!(serialized, json!("text"));

    let deserialized: RebornOutboundDeliveryModality =
        serde_json::from_value(serialized).expect("deserialize text modality");
    assert_eq!(deserialized, RebornOutboundDeliveryModality::Text);
}

#[test]
fn set_outbound_preferences_empty_json_defaults_final_target_to_none() {
    let request: RebornSetOutboundPreferencesRequest =
        serde_json::from_value(json!({})).expect("deserialize empty preferences request");

    assert!(request.final_reply_target_id.is_none());
}

#[test]
fn outbound_preferences_response_preserves_client_json_shape() {
    let response = RebornOutboundPreferencesResponse {
        final_reply_target: Some(outbound_target_summary("slack-dm-alpha")),
        final_reply_target_status: RebornOutboundDeliveryTargetStatus::Available,
        default_modality: RebornOutboundDeliveryModality::Text,
    };

    let serialized = serde_json::to_value(&response).expect("serialize preferences response");
    assert_eq!(
        serialized,
        json!({
            "final_reply_target": {
                "target_id": "slack-dm-alpha",
                "channel": "slack",
                "display_name": "Slack DM",
                "description": "Slack direct message",
            },
            "final_reply_target_status": "available",
            "default_modality": "text",
        })
    );

    let deserialized: RebornOutboundPreferencesResponse =
        serde_json::from_value(serialized).expect("deserialize preferences response");
    assert_eq!(deserialized, response);
}

#[test]
fn outbound_preferences_response_empty_json_defaults_to_text_without_target() {
    let response: RebornOutboundPreferencesResponse =
        serde_json::from_value(json!({})).expect("deserialize empty preferences response");

    assert!(response.final_reply_target.is_none());
    assert_eq!(
        response.final_reply_target_status,
        RebornOutboundDeliveryTargetStatus::NoneConfigured
    );
    assert_eq!(
        response.default_modality,
        RebornOutboundDeliveryModality::Text
    );
}

#[test]
fn outbound_preferences_response_missing_status_defaults_to_available_when_target_present() {
    let response: RebornOutboundPreferencesResponse = serde_json::from_value(json!({
        "final_reply_target": {
            "target_id": "slack-dm-alpha",
            "channel": "slack",
            "display_name": "Slack DM",
            "description": "Slack direct message",
        },
        "default_modality": "text",
    }))
    .expect("deserialize legacy preferences response");

    assert_eq!(
        response.final_reply_target_status,
        RebornOutboundDeliveryTargetStatus::Available
    );
    assert!(response.final_reply_target.is_some());
    assert_eq!(
        response.default_modality,
        RebornOutboundDeliveryModality::Text
    );
}

#[test]
fn outbound_preferences_response_serializes_unavailable_status_without_target() {
    let response = RebornOutboundPreferencesResponse {
        final_reply_target: None,
        final_reply_target_status: RebornOutboundDeliveryTargetStatus::Unavailable,
        default_modality: RebornOutboundDeliveryModality::Text,
    };

    let serialized =
        serde_json::to_value(&response).expect("serialize unavailable preferences response");
    assert_eq!(
        serialized,
        json!({
            "final_reply_target_status": "unavailable",
            "default_modality": "text",
        })
    );

    let deserialized: RebornOutboundPreferencesResponse =
        serde_json::from_value(serialized).expect("deserialize unavailable preferences response");
    assert_eq!(deserialized, response);
}

#[test]
fn outbound_preferences_response_serializes_none_configured_status_explicitly() {
    let response = RebornOutboundPreferencesResponse {
        final_reply_target: None,
        final_reply_target_status: RebornOutboundDeliveryTargetStatus::NoneConfigured,
        default_modality: RebornOutboundDeliveryModality::Text,
    };

    let serialized =
        serde_json::to_value(&response).expect("serialize none configured preferences response");
    assert_eq!(
        serialized,
        json!({
            "final_reply_target_status": "none_configured",
            "default_modality": "text",
        })
    );
}

#[test]
fn outbound_target_summary_preserves_client_json_shape() {
    let summary = outbound_target_summary("slack-dm-alpha");

    let serialized = serde_json::to_value(&summary).expect("serialize target summary");
    assert_eq!(
        serialized,
        json!({
            "target_id": "slack-dm-alpha",
            "channel": "slack",
            "display_name": "Slack DM",
            "description": "Slack direct message",
        })
    );

    let deserialized: RebornOutboundDeliveryTargetSummary =
        serde_json::from_value(serialized).expect("deserialize target summary");
    assert_eq!(deserialized.target_id.as_str(), "slack-dm-alpha");
    assert_eq!(deserialized.channel.as_str(), "slack");
    assert_eq!(deserialized.display_name.as_str(), "Slack DM");
    assert_eq!(
        deserialized
            .description
            .as_ref()
            .map(|description| description.as_str()),
        Some("Slack direct message")
    );
}

#[test]
fn outbound_target_list_response_preserves_empty_json_shape_without_cursor() {
    let response = RebornOutboundDeliveryTargetListResponse {
        targets: Vec::new(),
        next_cursor: None,
    };

    let serialized = serde_json::to_value(&response).expect("serialize empty target list");
    assert_eq!(serialized, json!({ "targets": [] }));
    assert!(
        serialized.get("next_cursor").is_none(),
        "None cursor must be omitted from the client payload"
    );

    let deserialized: RebornOutboundDeliveryTargetListResponse =
        serde_json::from_value(json!({ "targets": [] })).expect("deserialize empty target list");
    assert!(deserialized.targets.is_empty());
    assert!(deserialized.next_cursor.is_none());
}

#[test]
fn outbound_target_list_response_preserves_json_shape_with_cursor() {
    let response = RebornOutboundDeliveryTargetListResponse {
        targets: vec![RebornOutboundDeliveryTargetOption {
            target: outbound_target_summary("slack-dm-alpha"),
            capabilities: RebornOutboundDeliveryTargetCapabilities {
                final_replies: true,
                gate_prompts: true,
                auth_prompts: true,
            },
        }],
        next_cursor: Some("opaque-page-token".to_string()),
    };

    let serialized = serde_json::to_value(&response).expect("serialize target list with cursor");
    assert_eq!(
        serialized,
        json!({
            "targets": [{
                "target": {
                    "target_id": "slack-dm-alpha",
                    "channel": "slack",
                    "display_name": "Slack DM",
                    "description": "Slack direct message",
                },
                "capabilities": {
                    "final_replies": true,
                    "gate_prompts": true,
                    "auth_prompts": true,
                },
            }],
            "next_cursor": "opaque-page-token",
        })
    );

    let deserialized: RebornOutboundDeliveryTargetListResponse =
        serde_json::from_value(serialized).expect("deserialize target list with cursor");
    assert_eq!(deserialized, response);
}

#[test]
fn outbound_target_summary_rejects_malformed_display_fields() {
    for (field, invalid_value) in [
        ("channel", json!("")),
        ("channel", json!("slack\ninjected")),
        ("display_name", json!("")),
        ("display_name", json!("Slack DM\u{0000}")),
        ("description", json!("Slack direct\rmessage")),
    ] {
        let mut payload = json!({
            "target_id": "slack-dm-alpha",
            "channel": "slack",
            "display_name": "Slack DM",
            "description": "Slack direct message",
        });
        payload[field] = invalid_value;

        serde_json::from_value::<RebornOutboundDeliveryTargetSummary>(payload)
            .expect_err("malformed target summary display field");
    }

    for (field, invalid_value) in [
        ("channel", json!("a".repeat(129))),
        ("display_name", json!("a".repeat(257))),
        ("description", json!("a".repeat(1025))),
    ] {
        let mut payload = json!({
            "target_id": "slack-dm-alpha",
            "channel": "slack",
            "display_name": "Slack DM",
            "description": "Slack direct message",
        });
        payload[field] = invalid_value;

        serde_json::from_value::<RebornOutboundDeliveryTargetSummary>(payload)
            .expect_err("oversized target summary display field");
    }

    RebornOutboundDeliveryTargetSummary::new(
        outbound_target_id("slack-dm-alpha"),
        "slack",
        "Slack DM\ninjected",
        None,
    )
    .expect_err("constructor rejects malformed display field");
}

#[test]
fn outbound_target_display_fields_reject_whitespace_only_required_values_and_outer_whitespace() {
    for (field, invalid_value) in [
        ("channel", json!(" ")),
        ("channel", json!("\t")),
        ("display_name", json!(" ")),
        ("display_name", json!("\t")),
        ("channel", json!(" slack")),
        ("channel", json!("slack ")),
        ("display_name", json!(" Slack DM")),
        ("display_name", json!("Slack DM ")),
        ("description", json!(" Slack direct message")),
        ("description", json!("Slack direct message ")),
    ] {
        let mut payload = json!({
            "target_id": "slack-dm-alpha",
            "channel": "slack",
            "display_name": "Slack DM",
            "description": "Slack direct message",
        });
        payload[field] = invalid_value;

        serde_json::from_value::<RebornOutboundDeliveryTargetSummary>(payload)
            .expect_err("target summary display fields reject whitespace-only or padded values");
    }
}

#[test]
fn outbound_target_id_and_display_fields_reject_unicode_line_separators() {
    for target_id in [
        "slack-dm-alpha\u{2028}injected",
        "slack-dm-alpha\u{2029}injected",
    ] {
        RebornOutboundDeliveryTargetId::new(target_id)
            .expect_err("target id rejects unicode line separators");
        serde_json::from_value::<RebornSetOutboundPreferencesRequest>(json!({
            "final_reply_target_id": target_id,
        }))
        .expect_err("preference request rejects target id unicode line separators");
    }

    for (field, invalid_value) in [
        ("channel", json!("slack\u{2028}injected")),
        ("channel", json!("slack\u{2029}injected")),
        ("display_name", json!("Slack DM\u{2028}injected")),
        ("display_name", json!("Slack DM\u{2029}injected")),
        ("description", json!("Slack direct\u{2028}message")),
        ("description", json!("Slack direct\u{2029}message")),
    ] {
        let mut payload = json!({
            "target_id": "slack-dm-alpha",
            "channel": "slack",
            "display_name": "Slack DM",
            "description": "Slack direct message",
        });
        payload[field] = invalid_value;

        serde_json::from_value::<RebornOutboundDeliveryTargetSummary>(payload)
            .expect_err("target summary display fields reject unicode line separators");
    }
}

#[test]
fn outbound_target_id_and_display_fields_reject_unsafe_unicode_formatting() {
    for target_id in [
        "slack-dm-alpha\u{202e}injected",
        "slack-dm-alpha\u{2066}injected",
        "slack-dm-alpha\u{200b}injected",
        "slack-dm-alpha\u{feff}injected",
    ] {
        RebornOutboundDeliveryTargetId::new(target_id)
            .expect_err("target id rejects unsafe unicode formatting characters");
        serde_json::from_value::<RebornSetOutboundPreferencesRequest>(json!({
            "final_reply_target_id": target_id,
        }))
        .expect_err("preference request rejects unsafe unicode formatting characters");
    }

    for (field, invalid_value) in [
        ("channel", json!("slack\u{202e}injected")),
        ("channel", json!("slack\u{2066}injected")),
        ("channel", json!("slack\u{200b}injected")),
        ("channel", json!("slack\u{feff}injected")),
        ("display_name", json!("Slack DM\u{202e}injected")),
        ("display_name", json!("Slack DM\u{2066}injected")),
        ("display_name", json!("Slack DM\u{200b}injected")),
        ("display_name", json!("Slack DM\u{feff}injected")),
        ("description", json!("Slack direct\u{202e}message")),
        ("description", json!("Slack direct\u{2066}message")),
        ("description", json!("Slack direct\u{200b}message")),
        ("description", json!("Slack direct\u{feff}message")),
    ] {
        let mut payload = json!({
            "target_id": "slack-dm-alpha",
            "channel": "slack",
            "display_name": "Slack DM",
            "description": "Slack direct message",
        });
        payload[field] = invalid_value;

        serde_json::from_value::<RebornOutboundDeliveryTargetSummary>(payload)
            .expect_err("target summary display fields reject unsafe unicode formatting");
    }
}

#[test]
fn outbound_target_empty_description_is_accepted() {
    let description =
        RebornOutboundDeliveryTargetDescription::new("").expect("empty description is allowed");
    assert_eq!(description.as_str(), "");

    let summary = RebornOutboundDeliveryTargetSummary::new(
        outbound_target_id("slack-dm-alpha"),
        "slack",
        "Slack DM",
        Some("".to_string()),
    )
    .expect("summary accepts empty description");

    assert_eq!(
        summary
            .description
            .as_ref()
            .map(RebornOutboundDeliveryTargetDescription::as_str),
        Some("")
    );
}

#[tokio::test]
async fn outbound_preferences_unwired_mutations_and_target_listing_fail_closed() {
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    );

    let set_error = services
        .set_outbound_preferences(
            caller(),
            RebornSetOutboundPreferencesRequest {
                final_reply_target_id: Some(outbound_target_id("slack-dm-alpha")),
            },
        )
        .await
        .expect_err("unwired preference mutation");
    assert_eq!(set_error.code, RebornServicesErrorCode::Unavailable);
    assert_eq!(set_error.status_code, 503);
    assert!(!set_error.retryable);

    let list_error = services
        .list_outbound_delivery_targets(caller())
        .await
        .expect_err("unwired target listing");
    assert_eq!(list_error.code, RebornServicesErrorCode::Unavailable);
    assert_eq!(list_error.status_code, 503);
    assert!(!list_error.retryable);
}

#[tokio::test]
async fn outbound_preferences_facade_forwards_caller_and_request() {
    let outbound_facade = Arc::new(RecordingOutboundPreferencesFacade::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_outbound_preferences_facade(outbound_facade.clone());

    let get_response = services
        .get_outbound_preferences(caller())
        .await
        .expect("get outbound preferences");
    assert_eq!(
        get_response
            .final_reply_target
            .as_ref()
            .map(|target| target.target_id.as_str()),
        Some("slack-dm-alpha")
    );

    let set_response = services
        .set_outbound_preferences(
            caller_for_user_with_project("user-bravo", None),
            RebornSetOutboundPreferencesRequest {
                final_reply_target_id: Some(outbound_target_id("slack-dm-beta")),
            },
        )
        .await
        .expect("set outbound preferences");
    assert_eq!(
        set_response
            .final_reply_target
            .as_ref()
            .map(|target| target.target_id.as_str()),
        Some("slack-dm-beta")
    );

    let targets = services
        .list_outbound_delivery_targets(caller_for_user("user-charlie"))
        .await
        .expect("list outbound targets");
    assert_eq!(targets.targets.len(), 1);
    assert_eq!(
        targets.targets[0].target.target_id.as_str(),
        "slack-dm-alpha"
    );
    assert!(targets.targets[0].capabilities.final_replies);

    let get_calls = outbound_facade.get_calls();
    assert_eq!(get_calls.len(), 1);
    assert_eq!(get_calls[0].tenant_id.as_str(), "tenant-alpha");
    assert_eq!(get_calls[0].user_id.as_str(), "user-alpha");

    let set_calls = outbound_facade.set_calls();
    assert_eq!(set_calls.len(), 1);
    assert_eq!(set_calls[0].caller.user_id.as_str(), "user-bravo");
    assert!(set_calls[0].caller.agent_id.is_some());
    assert!(set_calls[0].caller.project_id.is_none());
    assert_eq!(
        set_calls[0]
            .request
            .final_reply_target_id
            .as_ref()
            .map(|target_id| target_id.as_str()),
        Some("slack-dm-beta")
    );

    let list_calls = outbound_facade.list_calls();
    assert_eq!(list_calls.len(), 1);
    assert_eq!(list_calls[0].user_id.as_str(), "user-charlie");
}

#[tokio::test]
async fn set_outbound_preferences_can_clear_final_target() {
    let outbound_facade = Arc::new(RecordingOutboundPreferencesFacade::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_outbound_preferences_facade(outbound_facade.clone());

    services
        .set_outbound_preferences(
            caller(),
            RebornSetOutboundPreferencesRequest {
                final_reply_target_id: None,
            },
        )
        .await
        .expect("clear outbound preferences");

    let set_calls = outbound_facade.set_calls();
    assert_eq!(set_calls.len(), 1);
    assert!(set_calls[0].request.final_reply_target_id.is_none());
}

#[tokio::test]
async fn set_outbound_preferences_rejects_malformed_target_id_before_facade() {
    for target_id in [
        "",
        " ",
        " slack-dm-alpha",
        "slack-dm-alpha ",
        "slack-dm-alpha\ninjected",
        "slack-dm-alpha\0injected",
    ] {
        serde_json::from_value::<RebornSetOutboundPreferencesRequest>(json!({
            "final_reply_target_id": target_id,
        }))
        .expect_err("malformed target id");
    }

    let oversized_target_id = "a".repeat(513);
    serde_json::from_value::<RebornSetOutboundPreferencesRequest>(json!({
        "final_reply_target_id": oversized_target_id,
    }))
    .expect_err("oversized target id");
}

#[tokio::test]
async fn set_outbound_preferences_accepts_max_length_target_id_before_facade() {
    let outbound_facade = Arc::new(RecordingOutboundPreferencesFacade::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_outbound_preferences_facade(outbound_facade.clone());

    let max_length_target_id = "a".repeat(512);
    services
        .set_outbound_preferences(
            caller(),
            RebornSetOutboundPreferencesRequest {
                final_reply_target_id: Some(outbound_target_id(&max_length_target_id)),
            },
        )
        .await
        .expect("max-length target id");

    let set_calls = outbound_facade.set_calls();
    assert_eq!(set_calls.len(), 1);
    assert_eq!(
        set_calls[0]
            .request
            .final_reply_target_id
            .as_ref()
            .map(|target_id| target_id.as_str()),
        Some(max_length_target_id.as_str())
    );
}

#[tokio::test]
async fn list_automations_rejects_missing_agent_id() {
    let automation_facade = Arc::new(RecordingAutomationFacade::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_automation_product_facade(automation_facade.clone());

    let err = services
        .list_automations(
            caller_without_agent(),
            WebUiListAutomationsRequest {
                limit: Some(10),
                run_limit: None,
            },
        )
        .await
        .expect_err("missing agent id should fail closed");

    assert_eq!(err.code, RebornServicesErrorCode::InvalidRequest);
    assert_eq!(err.status_code, 400);
    assert_eq!(automation_facade.list_calls().len(), 0);
}

#[tokio::test]
async fn list_automations_clamps_oversize_limit_before_product_facade() {
    let automation_facade = Arc::new(RecordingAutomationFacade::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_automation_product_facade(automation_facade.clone());

    services
        .list_automations(
            caller(),
            WebUiListAutomationsRequest {
                limit: Some(u32::MAX),
                run_limit: None,
            },
        )
        .await
        .expect("list automations");

    let list_calls = automation_facade.list_calls();
    assert_eq!(list_calls.len(), 1);
    assert_eq!(
        list_calls[0].limit, AUTOMATION_LIST_MAX_PAGE_SIZE as usize,
        "automation list limit must be clamped to AUTOMATION_LIST_MAX_PAGE_SIZE ({}) before the product facade",
        AUTOMATION_LIST_MAX_PAGE_SIZE
    );
}

#[tokio::test]
async fn list_automations_clamps_zero_limit_before_product_facade() {
    let automation_facade = Arc::new(RecordingAutomationFacade::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_automation_product_facade(automation_facade.clone());

    services
        .list_automations(
            caller(),
            WebUiListAutomationsRequest {
                limit: Some(0),
                run_limit: None,
            },
        )
        .await
        .expect("list automations");

    let list_calls = automation_facade.list_calls();
    assert_eq!(list_calls.len(), 1);
    assert_eq!(
        list_calls[0].limit, 1,
        "automation list limit must be clamped to at least one row"
    );
}

#[tokio::test]
async fn list_automations_uses_default_limit_when_omitted() {
    let automation_facade = Arc::new(RecordingAutomationFacade::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_automation_product_facade(automation_facade.clone());

    services
        .list_automations(
            caller(),
            WebUiListAutomationsRequest {
                limit: None,
                run_limit: None,
            },
        )
        .await
        .expect("list automations");

    let list_calls = automation_facade.list_calls();
    assert_eq!(list_calls.len(), 1);
    assert_eq!(
        list_calls[0].limit, AUTOMATION_LIST_DEFAULT_PAGE_SIZE as usize,
        "omitted automation list limit must use AUTOMATION_LIST_DEFAULT_PAGE_SIZE ({})",
        AUTOMATION_LIST_DEFAULT_PAGE_SIZE
    );
}

#[tokio::test]
async fn list_automations_clamps_oversize_run_limit_before_product_facade() {
    let automation_facade = Arc::new(RecordingAutomationFacade::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_automation_product_facade(automation_facade.clone());

    services
        .list_automations(
            caller(),
            WebUiListAutomationsRequest {
                limit: None,
                run_limit: Some(u32::MAX),
            },
        )
        .await
        .expect("list automations");

    let list_calls = automation_facade.list_calls();
    assert_eq!(list_calls.len(), 1);
    assert_eq!(
        list_calls[0].run_limit, AUTOMATION_RUN_HISTORY_MAX_PAGE_SIZE as usize,
        "automation run history limit must be clamped to AUTOMATION_RUN_HISTORY_MAX_PAGE_SIZE ({}) before the product facade",
        AUTOMATION_RUN_HISTORY_MAX_PAGE_SIZE
    );
}

#[tokio::test]
async fn list_automations_allows_zero_run_limit_before_product_facade() {
    let automation_facade = Arc::new(RecordingAutomationFacade::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_automation_product_facade(automation_facade.clone());

    services
        .list_automations(
            caller(),
            WebUiListAutomationsRequest {
                limit: None,
                run_limit: Some(0),
            },
        )
        .await
        .expect("list automations");

    let list_calls = automation_facade.list_calls();
    assert_eq!(list_calls.len(), 1);
    assert_eq!(
        list_calls[0].run_limit, 0,
        "explicit zero automation run history limit must disable embedded run history"
    );
}

#[test]
fn reborn_automation_state_round_trips_serde_for_every_variant() {
    let cases = [
        (RebornAutomationState::Active, "\"active\""),
        (RebornAutomationState::Scheduled, "\"scheduled\""),
        (RebornAutomationState::Paused, "\"paused\""),
        (RebornAutomationState::Disabled, "\"disabled\""),
        (RebornAutomationState::Inactive, "\"inactive\""),
        (RebornAutomationState::Completed, "\"completed\""),
        (RebornAutomationState::Unknown, "\"unknown\""),
    ];

    for (state, expected_wire) in cases {
        let serialized = serde_json::to_string(&state).expect("serialize state");
        assert_eq!(serialized, expected_wire);
        let deserialized: RebornAutomationState =
            serde_json::from_str(&serialized).expect("deserialize state");
        assert_eq!(deserialized, state);
    }
}

#[test]
fn reborn_automation_recent_run_info_round_trips_typed_ids_and_preserves_unknown_status() {
    let recent_run = RebornAutomationRecentRunInfo {
        run_id: Some(automation_run_id()),
        thread_id: ThreadId::new("thread-listed").expect("valid thread id"),
        fire_slot: Some("2026-06-03T09:00:00Z".parse().expect("fire slot")),
        status: RebornAutomationRecentRunStatus::Running,
        submitted_at: "2026-06-03T09:00:01Z".parse().expect("submitted at"),
        completed_at: None,
    };

    let serialized = serde_json::to_value(&recent_run).expect("serialize recent run");
    assert_eq!(
        serialized,
        json!({
            "run_id": "11111111-1111-1111-1111-111111111111",
            "thread_id": "thread-listed",
            "fire_slot": "2026-06-03T09:00:00Z",
            "status": "running",
            "submitted_at": "2026-06-03T09:00:01Z",
        })
    );

    let deserialized: RebornAutomationRecentRunInfo =
        serde_json::from_value(serialized).expect("deserialize recent run");
    assert_eq!(deserialized, recent_run);

    let future_status: RebornAutomationRecentRunInfo = serde_json::from_value(json!({
        "run_id": "11111111-1111-1111-1111-111111111111",
        "thread_id": "thread-listed",
        "status": "cancelled",
        "submitted_at": "2026-06-03T09:00:01Z",
    }))
    .expect("deserialize future recent run status");
    assert_eq!(
        future_status.status,
        RebornAutomationRecentRunStatus::Unknown
    );

    let defaulted_status: RebornAutomationRecentRunInfo = serde_json::from_value(json!({
        "run_id": "11111111-1111-1111-1111-111111111111",
        "thread_id": "thread-listed",
        "submitted_at": "2026-06-03T09:00:01Z",
    }))
    .expect("deserialize defaulted recent run status");
    assert_eq!(
        defaulted_status.status,
        RebornAutomationRecentRunStatus::Unknown
    );

    serde_json::from_value::<RebornAutomationRecentRunInfo>(json!({
        "run_id": "11111111-1111-1111-1111-111111111111",
        "thread_id": "thread-listed",
        "status": { "backend": "future" },
        "submitted_at": "2026-06-03T09:00:01Z",
    }))
    .expect_err("recent run rejects malformed status");

    serde_json::from_value::<RebornAutomationRecentRunInfo>(json!({
        "run_id": "not-a-uuid",
        "thread_id": "thread-listed",
        "status": "running",
        "submitted_at": "2026-06-03T09:00:01Z",
    }))
    .expect_err("recent run rejects malformed run_id");

    serde_json::from_value::<RebornAutomationRecentRunInfo>(json!({
        "run_id": "11111111-1111-1111-1111-111111111111",
        "thread_id": "thread/listed",
        "status": "running",
        "submitted_at": "2026-06-03T09:00:01Z",
    }))
    .expect_err("recent run rejects malformed thread_id");
}

#[tokio::test]
async fn list_automations_returns_empty_list() {
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_automation_product_facade(Arc::new(StaticAutomationFacade { output: Vec::new() }));

    let listed = services
        .list_automations(caller(), WebUiListAutomationsRequest::default())
        .await
        .expect("list automations");

    assert!(listed.automations.is_empty());
}

#[tokio::test]
async fn automation_facade_unwired_fails_closed() {
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    );

    let error = services
        .list_automations(caller(), WebUiListAutomationsRequest::default())
        .await
        .expect_err("unwired automation facade");

    assert_eq!(error.code, RebornServicesErrorCode::Unavailable);
    assert_eq!(error.status_code, 503);
    assert!(error.retryable);
}

#[tokio::test]
async fn setup_extension_returns_post_setup_onboarding_payload() {
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_lifecycle_product_facade(Arc::new(
        RecordingLifecycleFacade::with_credential_requirements_and_onboarding(
            vec![manual_credential_requirement("github_runtime_token", true)],
            onboarding_fixture(),
        ),
    ));

    let response = services
        .setup_extension(
            caller(),
            lifecycle_package_ref("github"),
            WebUiSetupExtensionRequest::default(),
        )
        .await
        .expect("setup extension response");

    let onboarding = response.onboarding.as_ref().expect("onboarding payload");
    assert_eq!(response.phase, LifecyclePhase::Configured);
    assert_eq!(
        onboarding.credential_instructions.as_deref(),
        Some("github is installed. Activate it to make its tools available.")
    );
    assert_eq!(
        onboarding.credential_next_step.as_deref(),
        Some("After saving the token, activate GitHub to publish its tools.")
    );
}

#[tokio::test]
async fn setup_extension_rejects_blank_required_manual_secret() {
    let credentials = Arc::new(RecordingExtensionCredentialSetupService::default());
    let services =
        setup_services_with_requirements(vec![manual_credential_requirement("api_token", true)])
            .with_extension_credentials(credentials.clone());

    let err = services
        .setup_extension(
            caller(),
            lifecycle_package_ref("github"),
            WebUiSetupExtensionRequest {
                action: Some("submit".to_string()),
                payload: Some(json!({
                    "secrets": {
                        "api_token": "   "
                    }
                })),
            },
        )
        .await
        .expect_err("blank required token is rejected");

    assert_setup_validation(err, "secrets", WebUiInboundValidationCode::Blank);
    assert_eq!(credentials.status_count(), 1);
    assert_eq!(credentials.submit_count(), 0);
}

#[tokio::test]
async fn setup_extension_rejects_unknown_secret_name() {
    let credentials = Arc::new(RecordingExtensionCredentialSetupService::default());
    let services =
        setup_services_with_requirements(vec![manual_credential_requirement("api_token", true)])
            .with_extension_credentials(credentials.clone());

    let err = services
        .setup_extension(
            caller(),
            lifecycle_package_ref("github"),
            WebUiSetupExtensionRequest {
                action: Some("submit".to_string()),
                payload: Some(json!({
                    "secrets": {
                        "unknown_name": "value"
                    }
                })),
            },
        )
        .await
        .expect_err("unknown secret name is rejected");

    assert_setup_validation(err, "secrets", WebUiInboundValidationCode::InvalidValue);
    assert_eq!(credentials.status_count(), 0);
    assert_eq!(credentials.submit_count(), 0);
}

#[tokio::test]
async fn setup_extension_rejects_oauth_secret_via_manual_submit() {
    let credentials = Arc::new(RecordingExtensionCredentialSetupService::default());
    let services =
        setup_services_with_requirements(vec![oauth_credential_requirement("google_oauth", true)])
            .with_extension_credentials(credentials.clone());

    let err = services
        .setup_extension(
            caller(),
            lifecycle_package_ref("google"),
            WebUiSetupExtensionRequest {
                action: Some("submit".to_string()),
                payload: Some(json!({
                    "secrets": {
                        "google_oauth": "value"
                    }
                })),
            },
        )
        .await
        .expect_err("oauth credential cannot be submitted as a manual token");

    assert_setup_validation(err, "secrets", WebUiInboundValidationCode::InvalidValue);
    assert_eq!(credentials.status_count(), 0);
    assert_eq!(credentials.submit_count(), 0);
}

fn setup_services_with_requirements(
    requirements: Vec<LifecycleExtensionCredentialRequirement>,
) -> RebornServices {
    RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_lifecycle_product_facade(Arc::new(
        RecordingLifecycleFacade::with_credential_requirements(requirements),
    ))
}

fn lifecycle_package_ref(package_id: &str) -> LifecyclePackageRef {
    LifecyclePackageRef::new(LifecyclePackageKind::Extension, package_id)
        .expect("valid package ref")
}

fn extension_summary(
    package_id: &str,
    credential_requirements: Vec<LifecycleExtensionCredentialRequirement>,
    onboarding: Option<LifecycleExtensionOnboarding>,
) -> LifecycleExtensionSummary {
    LifecycleExtensionSummary {
        package_ref: lifecycle_package_ref(package_id),
        name: package_id.to_string(),
        version: "1.0.0".to_string(),
        description: "test extension".to_string(),
        source: LifecycleExtensionSource::HostBundled,
        runtime_kind: LifecycleExtensionRuntimeKind::FirstParty,
        visible_capability_ids: vec![format!("{package_id}.read"), format!("{package_id}.write")],
        visible_read_only_capability_ids: Vec::new(),
        credential_requirements,
        onboarding,
    }
}

fn onboarding_fixture() -> LifecycleExtensionOnboarding {
    LifecycleExtensionOnboarding {
        instructions: "GitHub needs a token before its tools can run.".to_string(),
        credential_instructions: Some("Paste the GitHub token IronClaw should use.".to_string()),
        setup_url: Some("https://github.com/settings/personal-access-tokens/new".to_string()),
        credential_next_step: Some(
            "After saving the token, activate GitHub to publish its tools.".to_string(),
        ),
    }
}

fn manual_credential_requirement(
    name: &str,
    required: bool,
) -> LifecycleExtensionCredentialRequirement {
    LifecycleExtensionCredentialRequirement {
        name: name.to_string(),
        provider: "github".to_string(),
        required,
        setup: LifecycleExtensionCredentialSetup::ManualToken,
    }
}

fn oauth_credential_requirement(
    name: &str,
    required: bool,
) -> LifecycleExtensionCredentialRequirement {
    LifecycleExtensionCredentialRequirement {
        name: name.to_string(),
        provider: "google".to_string(),
        required,
        setup: LifecycleExtensionCredentialSetup::OAuth {
            scopes: vec!["https://www.googleapis.com/auth/gmail.readonly".to_string()],
        },
    }
}

fn assert_setup_validation(
    err: RebornServicesError,
    field: &str,
    code: WebUiInboundValidationCode,
) {
    assert_eq!(err.kind, RebornServicesErrorKind::Validation);
    assert_eq!(err.status_code, 400);
    assert_eq!(err.field.as_deref(), Some(field));
    assert_eq!(err.validation_code, Some(code));
}

#[tokio::test]
async fn get_run_state_returns_stable_dto_without_m3_internal_fields() {
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        coordinator.clone(),
    );
    setup_owned_thread(&services, caller(), "thread-alpha").await;

    let response = services
        .get_run_state(
            caller(),
            RebornGetRunStateRequest {
                thread_id: "thread-alpha".to_string(),
                run_id: run_id_string(),
            },
        )
        .await
        .expect("get_run_state succeeds");

    assert_eq!(response.run_id.as_uuid().to_string(), run_id_string());
    assert_eq!(response.status, TurnStatus::Queued);
    assert_eq!(response.event_cursor, EventCursor(17));
    assert_eq!(response.accepted_message_ref.as_str(), "msg:replayed");
    assert_eq!(response.resolved_run_profile_version, 1);
    assert_eq!(
        response.resolved_run_profile_id,
        RunProfileId::default_profile().as_str()
    );
    assert!(response.gate_ref.is_none());
    assert!(response.failure.is_none());
    assert!(response.checkpoint_id.is_none());
    assert_eq!(coordinator.run_state_request_count(), 1);

    // Stable DTO must not surface M3-internal binding refs, model route, or
    // raw turn scope to WebUI consumers.
    let rendered = serde_json::to_string(&response).expect("json");
    assert!(!rendered.contains("source_binding_ref"));
    assert!(!rendered.contains("reply_target_binding_ref"));
    assert!(!rendered.contains("resolved_model_route"));
    assert!(!rendered.contains("webui-src:replayed"));
    assert!(!rendered.contains("webui-reply:replayed"));
    assert!(!rendered.contains("\"scope\""));
}

#[tokio::test]
async fn get_run_state_rejects_invalid_thread_id() {
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        coordinator.clone(),
    );

    let err = services
        .get_run_state(
            caller(),
            RebornGetRunStateRequest {
                thread_id: String::new(),
                run_id: run_id_string(),
            },
        )
        .await
        .expect_err("blank thread_id must be rejected");

    assert_eq!(err.code, RebornServicesErrorCode::InvalidRequest);
    assert_eq!(err.status_code, 400);
    assert_eq!(err.field.as_deref(), Some("thread_id"));
    assert_eq!(
        err.validation_code,
        Some(WebUiInboundValidationCode::InvalidId)
    );
    // Errors must be sanitized — no internal type names leak through.
    let rendered = serde_json::to_string(&err).expect("json");
    assert!(!rendered.contains("TurnCoordinator"));
    assert!(!rendered.contains("HostRuntime"));
    assert_eq!(coordinator.run_state_request_count(), 0);
}

#[tokio::test]
async fn get_run_state_rejects_non_uuid_run_id() {
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        coordinator.clone(),
    );

    let err = services
        .get_run_state(
            caller(),
            RebornGetRunStateRequest {
                thread_id: "thread-alpha".to_string(),
                run_id: "not-a-uuid".to_string(),
            },
        )
        .await
        .expect_err("non-uuid run_id must be rejected");

    assert_eq!(err.code, RebornServicesErrorCode::InvalidRequest);
    assert_eq!(err.status_code, 400);
    assert_eq!(err.field.as_deref(), Some("run_id"));
    assert_eq!(
        err.validation_code,
        Some(WebUiInboundValidationCode::InvalidId)
    );
    assert_eq!(coordinator.run_state_request_count(), 0);
}

#[tokio::test]
async fn get_run_state_maps_scope_not_found_to_not_found() {
    let coordinator = Arc::new(FakeTurnCoordinator::with_run_state_error(
        TurnError::ScopeNotFound,
    ));
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        coordinator.clone(),
    );
    setup_owned_thread(&services, caller(), "thread-alpha").await;

    let err = services
        .get_run_state(
            caller(),
            RebornGetRunStateRequest {
                thread_id: "thread-alpha".to_string(),
                run_id: run_id_string(),
            },
        )
        .await
        .expect_err("missing run must surface NotFound");

    assert_eq!(err.code, RebornServicesErrorCode::NotFound);
    assert_eq!(err.status_code, 404);
    assert!(!err.retryable);
}

// Regression: get_run_state must reject when the authenticated user does not
// own the thread. TurnScope only carries (tenant, agent, project, thread_id),
// so without this check any caller sharing an agent scope could read another
// user's run state by guessing thread_id and run_id.
#[tokio::test]
async fn get_run_state_rejects_cross_user_access() {
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        coordinator.clone(),
    );
    let alice = caller();
    setup_owned_thread(&services, alice.clone(), "thread-alice").await;

    let bob = WebUiAuthenticatedCaller::new(
        TenantId::new("tenant-alpha").expect("tenant"),
        UserId::new("user-bob").expect("user"),
        alice.agent_id.clone(),
        alice.project_id.clone(),
    );

    let err = services
        .get_run_state(
            bob,
            RebornGetRunStateRequest {
                thread_id: "thread-alice".to_string(),
                run_id: run_id_string(),
            },
        )
        .await
        .expect_err("cross-user run-state read must be rejected");

    // 404 rather than 403 so the existence of Alice's thread is not leaked.
    assert_eq!(err.code, RebornServicesErrorCode::NotFound);
    assert_eq!(err.status_code, 404);
    assert_eq!(
        coordinator.run_state_request_count(),
        0,
        "turn coordinator must NOT be called for cross-user run-state read"
    );
}

/// Seed `count` user messages into the in-memory thread service so the
/// timeline pagination regressions below have real, ordered message
/// rows to slice against.
async fn seed_thread_messages(
    threads: &InMemorySessionThreadService,
    caller: &WebUiAuthenticatedCaller,
    thread_id: &str,
    count: usize,
) {
    let scope = thread_scope_for(caller);
    let parsed_thread_id = ironclaw_host_api::ThreadId::new(thread_id).expect("thread id");
    for index in 0..count {
        threads
            .accept_inbound_message(AcceptInboundMessageRequest {
                scope: scope.clone(),
                thread_id: parsed_thread_id.clone(),
                actor_id: caller.user_id.as_str().to_string(),
                source_binding_id: None,
                reply_target_binding_id: None,
                external_event_id: None,
                content: MessageContent::text(format!("msg-{index}")),
            })
            .await
            .expect("seed message");
    }
}

// Regression for the timeline-pagination review (Medium). Without
// per-response caps a thread with hundreds of messages would force a
// multi-megabyte JSON allocation + serialize per call, since the route
// rate limit only bounds open frequency. `get_timeline` must (a) clamp
// `limit` to a hard ceiling so callers cannot bypass the cap, (b)
// return at most `limit` messages per page, and (c) surface a
// `next_cursor` the browser can echo back to load the page preceding
// it. When the caller has reached the start of the thread, `next_cursor`
// must be `None` so the browser stops asking for more.
#[tokio::test]
async fn get_timeline_pages_messages_with_cursor() {
    let threads = Arc::new(InMemorySessionThreadService::default());
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let services = RebornServices::new(threads.clone(), coordinator);
    let alice = caller();
    setup_owned_thread(&services, alice.clone(), "thread-paginate").await;
    seed_thread_messages(&threads, &alice, "thread-paginate", 25).await;

    let first = services
        .get_timeline(
            alice.clone(),
            RebornTimelineRequest {
                thread_id: "thread-paginate".to_string(),
                limit: Some(10),
                ..Default::default()
            },
        )
        .await
        .expect("first page");
    assert_eq!(
        first.messages.len(),
        10,
        "first page must honor the requested limit"
    );
    // The page is the newest 10 messages — the page boundary sits at
    // the message just *before* the earliest one in this page.
    let earliest_in_first_page = first
        .messages
        .first()
        .expect("first message on page")
        .sequence;
    let next_cursor = first
        .next_cursor
        .as_deref()
        .expect("next_cursor must surface when more messages remain");

    let second = services
        .get_timeline(
            alice.clone(),
            RebornTimelineRequest {
                thread_id: "thread-paginate".to_string(),
                limit: Some(10),
                cursor: Some(next_cursor.to_string()),
            },
        )
        .await
        .expect("second page");
    assert_eq!(second.messages.len(), 10, "second page is fully populated");
    // Every message in page two must be older than every message in
    // page one.
    assert!(
        second
            .messages
            .last()
            .map(|message| message.sequence < earliest_in_first_page)
            .unwrap_or(false),
        "second page must contain messages strictly older than the first"
    );
    assert!(
        second.next_cursor.is_some(),
        "more pages remain after the second page"
    );

    let third = services
        .get_timeline(
            alice,
            RebornTimelineRequest {
                thread_id: "thread-paginate".to_string(),
                limit: Some(10),
                cursor: second.next_cursor.clone(),
            },
        )
        .await
        .expect("third page");
    // Five messages remain (25 - 10 - 10) and the caller has reached
    // the start of the thread, so next_cursor must be None.
    assert_eq!(third.messages.len(), 5);
    assert!(
        third.next_cursor.is_none(),
        "next_cursor must be None once all older messages are exhausted"
    );
}

// Regression: `limit` must be clamped to the facade's hard ceiling so a
// caller cannot widen the response by passing a huge value. Without the
// clamp, the per-route rate limit would be the only thing bounding
// per-request response size.
#[tokio::test]
async fn get_timeline_clamps_oversize_limit_to_hard_ceiling() {
    let threads = Arc::new(InMemorySessionThreadService::default());
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let services = RebornServices::new(threads.clone(), coordinator);
    let alice = caller();
    setup_owned_thread(&services, alice.clone(), "thread-cap").await;
    // Seed more than the hard ceiling so the clamp is observable.
    seed_thread_messages(&threads, &alice, "thread-cap", 250).await;

    let response = services
        .get_timeline(
            alice,
            RebornTimelineRequest {
                thread_id: "thread-cap".to_string(),
                limit: Some(u32::MAX),
                ..Default::default()
            },
        )
        .await
        .expect("clamped timeline");
    assert!(
        response.messages.len() <= 200,
        "limit must be clamped to TIMELINE_MAX_PAGE_SIZE (200); got {}",
        response.messages.len()
    );
    assert!(
        response.next_cursor.is_some(),
        "next_cursor must surface because the underlying thread has more messages than the cap"
    );
}

// Regression: a malformed cursor must be rejected at the wire boundary
// with an InvalidValue validation error rather than silently treated as
// "no cursor". Without this guard, a caller could send garbage in
// `cursor=...` and quietly load page 1 instead of the intended page.
#[tokio::test]
async fn get_timeline_rejects_malformed_cursor() {
    let threads = Arc::new(InMemorySessionThreadService::default());
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let services = RebornServices::new(threads, coordinator);
    let alice = caller();
    setup_owned_thread(&services, alice.clone(), "thread-bad-cursor").await;

    let err = services
        .get_timeline(
            alice,
            RebornTimelineRequest {
                thread_id: "thread-bad-cursor".to_string(),
                limit: None,
                cursor: Some("not-a-valid-cursor".to_string()),
            },
        )
        .await
        .expect_err("malformed cursor must be rejected");

    assert_eq!(err.code, RebornServicesErrorCode::InvalidRequest);
    assert_eq!(err.field.as_deref(), Some("cursor"));
    assert_eq!(
        err.validation_code,
        Some(WebUiInboundValidationCode::InvalidValue)
    );
}

#[test]
fn facade_source_avoids_forbidden_runtime_dependencies() {
    let source = std::fs::read_to_string("src/reborn_services.rs").expect("facade source");
    for forbidden in [
        "CapabilityHost",
        "ironclaw_capabilities",
        "ironclaw_dispatcher",
        "ironclaw_host_runtime",
        "ironclaw_run_state",
        "ironclaw_storage",
        "RuntimeLane",
        "pub fn thread_service",
        "pub fn turn_coordinator",
    ] {
        assert!(
            !source.contains(forbidden),
            "RebornServices facade must not expose route handlers to {forbidden}"
        );
    }

    let _ = Utc::now();
}

// Regression for the missing-error-path-test review (Medium): the
// new `list_threads` facade path must fail closed until a backend
// override for `list_threads_for_scope` is wired. The default
// `SessionThreadService` impl returns `Backend(...)`, and the
// facade is supposed to translate that into a retryable
// `service_unavailable` (HTTP 503) — never an empty thread list
// that pretends the caller owns nothing. This test pins the wire
// contract so a future regression that quietly returns Ok([]) on a
// missing backend would break the test, not silently mislead
// callers.
#[tokio::test]
async fn list_threads_unimplemented_backend_returns_service_unavailable() {
    // `ScopeMismatchThreadStub` is reused here because it
    // intentionally does NOT override the trait's default
    // `list_threads_for_scope` impl, so the facade sees the
    // unimplemented-enumeration error path. The in-memory backend
    // grew a real enumeration impl (local-dev needed working
    // sidebar listing), so it can no longer stand in for a backend
    // without enumeration support.
    let services = RebornServices::new(
        Arc::new(ScopeMismatchThreadStub),
        Arc::new(FakeTurnCoordinator::default()),
    );

    let error = services
        .list_threads(caller(), WebUiListThreadsRequest::default())
        .await
        .expect_err(
            "list_threads must fail closed when the SessionThreadService backend \
             does not implement list_threads_for_scope",
        );
    assert_eq!(error.code, RebornServicesErrorCode::Unavailable);
    assert_eq!(error.status_code, 503);
    assert!(
        error.retryable,
        "Backend errors are retryable so the browser can re-poll once a v2-aware \
         backend overrides list_threads_for_scope",
    );

    // Confirm the wire shape is the snake_case enum the WebUi handler maps
    // to its `error` field; matching on the variant alone would still pass
    // if someone changed `#[serde(rename_all = ...)]` to PascalCase.
    let json = serde_json::to_value(&error).expect("serialize");
    assert_eq!(
        json["code"], "unavailable",
        "wire code must be snake_case `unavailable`; got: {json}"
    );
    assert_eq!(json["retryable"], true);
}

#[tokio::test]
async fn list_threads_hides_automation_trigger_threads() {
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    let services = RebornServices::new(
        thread_service.clone(),
        Arc::new(FakeTurnCoordinator::default()),
    );
    let caller = caller();
    let visible_thread_id = ThreadId::new("thread-visible").expect("visible thread id");
    let automation_thread_id = ThreadId::new("thread-automation").expect("automation thread id");
    let malformed_metadata_thread_id =
        ThreadId::new("thread-malformed-metadata").expect("malformed metadata thread id");

    thread_service
        .ensure_thread(EnsureThreadRequest {
            scope: thread_scope_for(&caller),
            thread_id: Some(visible_thread_id.clone()),
            created_by_actor_id: caller.user_id.as_str().to_string(),
            title: Some("Visible chat".to_string()),
            metadata_json: Some(json!({ "source": "webui" }).to_string()),
        })
        .await
        .expect("visible thread");
    thread_service
        .ensure_thread(EnsureThreadRequest {
            scope: thread_scope_for(&caller),
            thread_id: Some(automation_thread_id.clone()),
            created_by_actor_id: caller.user_id.as_str().to_string(),
            title: Some("Automation run".to_string()),
            metadata_json: Some(automation_trigger_thread_metadata_json(
                "trigger-scheduled-summary",
            )),
        })
        .await
        .expect("automation thread");
    thread_service
        .ensure_thread(EnsureThreadRequest {
            scope: thread_scope_for(&caller),
            thread_id: Some(malformed_metadata_thread_id.clone()),
            created_by_actor_id: caller.user_id.as_str().to_string(),
            title: Some("Malformed metadata chat".to_string()),
            metadata_json: Some(format!(
                r#"{{"source":"{AUTOMATION_TRIGGER_THREAD_SOURCE_TAG}""#
            )),
        })
        .await
        .expect("malformed metadata thread");

    let response = services
        .list_threads(caller, WebUiListThreadsRequest::default())
        .await
        .expect("list threads");
    let thread_ids = response
        .threads
        .iter()
        .map(|thread| thread.thread_id.clone())
        .collect::<Vec<_>>();

    assert_eq!(thread_ids.len(), 2);
    assert!(thread_ids.contains(&visible_thread_id));
    assert!(thread_ids.contains(&malformed_metadata_thread_id));
    assert!(
        !thread_ids.contains(&automation_thread_id),
        "automation trigger threads should be accessible by direct id but hidden from the chat list",
    );
}

#[tokio::test]
async fn list_threads_breaks_out_when_cursor_does_not_advance_for_automation_threads() {
    let caller = caller();
    let scope = thread_scope_for(&caller);
    let automation_thread = |thread_id: &str| SessionThreadRecord {
        scope: scope.clone(),
        thread_id: ThreadId::new(thread_id).expect("automation thread id"),
        created_by_actor_id: caller.user_id.as_str().to_string(),
        title: Some(format!("Automation run {thread_id}")),
        metadata_json: Some(automation_trigger_thread_metadata_json(
            "trigger-scheduled-summary",
        )),
        goal: None,
    };
    let stalled_cursor = "cursor-stalled".to_string();
    let thread_service = Arc::new(ScriptedThreadService::list_pages(vec![
        ListThreadsForScopeResponse {
            threads: vec![automation_thread("thread-automation-stall-1")],
            next_cursor: Some(stalled_cursor.clone()),
        },
        ListThreadsForScopeResponse {
            threads: vec![automation_thread("thread-automation-stall-2")],
            next_cursor: Some(stalled_cursor.clone()),
        },
    ]));
    let services = RebornServices::new(
        thread_service.clone(),
        Arc::new(FakeTurnCoordinator::default()),
    );

    let response = tokio::time::timeout(
        Duration::from_secs(1),
        services.list_threads(
            caller,
            WebUiListThreadsRequest {
                limit: Some(2),
                cursor: None,
            },
        ),
    )
    .await
    .expect("list_threads should terminate when backend cursor stalls")
    .expect("list threads");

    assert!(
        response.threads.is_empty(),
        "automation trigger threads must stay hidden even when every fetched page is filtered",
    );
    assert_eq!(
        response.next_cursor, None,
        "stalled cursor must be cleared so callers do not keep replaying the same filtered page",
    );
    let list_requests = thread_service.list_requests();
    assert_eq!(
        list_requests.len(),
        2,
        "facade should fetch the stalled page once and then break on the repeated cursor",
    );
    assert_eq!(list_requests[0].cursor, None);
    assert_eq!(list_requests[1].cursor.as_deref(), Some("cursor-stalled"));
}

#[tokio::test]
async fn list_threads_caps_filtered_pages_when_automation_threads_dominate() {
    let caller = caller();
    let scope = thread_scope_for(&caller);
    let automation_thread = |index: usize| SessionThreadRecord {
        scope: scope.clone(),
        thread_id: ThreadId::new(format!("thread-automation-budget-{index:02}"))
            .expect("automation thread id"),
        created_by_actor_id: caller.user_id.as_str().to_string(),
        title: Some(format!("Automation run {index}")),
        metadata_json: Some(automation_trigger_thread_metadata_json(
            "trigger-scheduled-summary",
        )),
        goal: None,
    };
    let responses = (0..20)
        .map(|index| ListThreadsForScopeResponse {
            threads: vec![automation_thread(index)],
            next_cursor: Some(format!("cursor-{index:02}")),
        })
        .collect::<Vec<_>>();
    let thread_service = Arc::new(ScriptedThreadService::list_pages(responses));
    let services = RebornServices::new(
        thread_service.clone(),
        Arc::new(FakeTurnCoordinator::default()),
    );

    let response = services
        .list_threads(
            caller,
            WebUiListThreadsRequest {
                limit: Some(1),
                cursor: None,
            },
        )
        .await
        .expect("list threads");

    assert!(
        response.threads.is_empty(),
        "automation trigger threads must stay hidden when filter pages are exhausted",
    );
    assert_eq!(
        response.next_cursor, None,
        "filter page budget exhaustion must clear the cursor so callers do not keep scanning",
    );
    let list_requests = thread_service.list_requests();
    assert_eq!(
        list_requests.len(),
        20,
        "facade must enforce a hard cap on filtered backend pages",
    );
    assert!(
        list_requests
            .iter()
            .all(|request| request.limit == Some(50)),
        "facade should use a fixed candidate page size instead of shrinking toward one"
    );
}

#[tokio::test]
async fn list_threads_skips_hidden_automation_threads_when_filling_page() {
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    let services = RebornServices::new(
        thread_service.clone(),
        Arc::new(FakeTurnCoordinator::default()),
    );
    let caller = caller();
    let automation_thread_id = ThreadId::new("thread-a-automation").expect("automation thread id");
    let first_visible_thread_id =
        ThreadId::new("thread-b-visible").expect("first visible thread id");
    let second_visible_thread_id =
        ThreadId::new("thread-c-visible").expect("second visible thread id");

    thread_service
        .ensure_thread(EnsureThreadRequest {
            scope: thread_scope_for(&caller),
            thread_id: Some(automation_thread_id.clone()),
            created_by_actor_id: caller.user_id.as_str().to_string(),
            title: Some("Automation run".to_string()),
            metadata_json: Some(automation_trigger_thread_metadata_json(
                "trigger-scheduled-summary",
            )),
        })
        .await
        .expect("automation thread");
    thread_service
        .ensure_thread(EnsureThreadRequest {
            scope: thread_scope_for(&caller),
            thread_id: Some(first_visible_thread_id.clone()),
            created_by_actor_id: caller.user_id.as_str().to_string(),
            title: Some("First visible chat".to_string()),
            metadata_json: Some(json!({ "source": "webui" }).to_string()),
        })
        .await
        .expect("first visible thread");
    thread_service
        .ensure_thread(EnsureThreadRequest {
            scope: thread_scope_for(&caller),
            thread_id: Some(second_visible_thread_id.clone()),
            created_by_actor_id: caller.user_id.as_str().to_string(),
            title: Some("Second visible chat".to_string()),
            metadata_json: Some(json!({ "source": "webui" }).to_string()),
        })
        .await
        .expect("second visible thread");

    let first_page = services
        .list_threads(
            caller.clone(),
            WebUiListThreadsRequest {
                limit: Some(1),
                cursor: None,
            },
        )
        .await
        .expect("list first visible page");
    assert_eq!(
        first_page
            .threads
            .iter()
            .map(|thread| thread.thread_id.clone())
            .collect::<Vec<_>>(),
        vec![first_visible_thread_id],
    );
    assert_eq!(first_page.next_cursor.as_deref(), Some("thread-b-visible"));

    let second_page = services
        .list_threads(
            caller,
            WebUiListThreadsRequest {
                limit: Some(1),
                cursor: first_page.next_cursor,
            },
        )
        .await
        .expect("list second visible page");
    assert_eq!(
        second_page
            .threads
            .iter()
            .map(|thread| thread.thread_id.clone())
            .collect::<Vec<_>>(),
        vec![second_visible_thread_id],
    );
    assert_eq!(second_page.next_cursor, None);
}
