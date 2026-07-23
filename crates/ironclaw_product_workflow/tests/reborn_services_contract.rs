//! Contract tests for WebUI-facing RebornServices facade.

// arch-exempt: large_file, contract suite tracks the ProductSurface facade one seam per test; splits with the domain-port decomposition, plan #5985

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use chrono::Utc;
use ironclaw_approvals::{
    AutoApproveSettingInput, AutoApproveSettingKey, AutoApproveSettingRecord,
    AutoApproveSettingStore, CapabilityPermissionStoreError, PersistentApprovalAction,
    PersistentApprovalPolicy, PersistentApprovalPolicyError, PersistentApprovalPolicyInput,
    PersistentApprovalPolicyKey, PersistentApprovalPolicyStore, ToolPermissionOverride,
    ToolPermissionOverrideInput, ToolPermissionOverrideKey, ToolPermissionOverrideStore,
};
use ironclaw_attachments::InboundAttachment;
use ironclaw_auth::{
    AuthAccountLastError, AuthAccountState, CredentialAccountId, CredentialAccountProjection,
    CredentialAccountStatus,
};
use ironclaw_host_api::{
    ActivityId, AgentId, ApprovalRequestId, CapabilityId, EffectKind, ExtensionId, InvocationId,
    Outcome, OutcomeRefs, PermissionMode, Principal, ProjectId, Resolution, ResourceScope,
    ResultPreviewMeta, ResultProgress, ResultRef, SafeSummary, SecretHandle, TenantId,
    TerminateHint, ThreadId, ToolVerdict, UserId,
};
use ironclaw_host_api::{CapabilitySurfaceKind, InstallationState};
use ironclaw_product_adapters::{
    ProductAdapterError, ProductOutboundEnvelope, ProductWorkflowRejectionKind, ProjectionCursor,
    ProjectionStream, ProjectionSubscriptionRequest, ProtocolAuthFailure, RedactedString,
};
use ironclaw_product_workflow::{
    ADMIN_USER_DELETE_CAPABILITY_ID, ADMIN_USER_DELETE_SECRET_CAPABILITY_ID,
    ADMIN_USER_PUT_SECRET_CAPABILITY_ID, ADMIN_USER_SECRETS_VIEW,
    ADMIN_USER_SET_ROLE_CAPABILITY_ID, ADMIN_USER_SET_STATUS_CAPABILITY_ID,
    ADMIN_USER_UPDATE_CAPABILITY_ID, ADMIN_USER_VIEW, ADMIN_USERS_VIEW,
    AUTOMATION_DELETE_CAPABILITY_ID, AUTOMATION_LIST_DEFAULT_PAGE_SIZE,
    AUTOMATION_LIST_MAX_PAGE_SIZE, AUTOMATION_PAUSE_CAPABILITY_ID, AUTOMATION_RENAME_CAPABILITY_ID,
    AUTOMATION_RESUME_CAPABILITY_ID, AUTOMATION_RUN_HISTORY_DEFAULT_PAGE_SIZE,
    AUTOMATION_RUN_HISTORY_MAX_PAGE_SIZE, AUTOMATION_TRIGGER_THREAD_SOURCE_TAG, AUTOMATIONS_VIEW,
    ActiveModelReader, ApprovalInteractionActionView, ApprovalInteractionDecision,
    ApprovalInteractionScope, ApprovalInteractionService, AuthInteractionDecision,
    AuthInteractionService, AutomationListRequest, AutomationName, AutomationProductFacade,
    CREATE_THREAD_OPERATION, ChannelAuthAccountState, ChannelConfigFacade, ChannelConnectionFacade,
    ChannelConnectionRequirement, CodexLoginStart, EXTENSION_IMPORT_CAPABILITY_ID,
    EXTENSION_SETUP_SUBMIT_CAPABILITY_ID, EXTENSION_SETUP_VIEW, EXTENSIONS_VIEW,
    ExtensionCredentialSetupService, ExtensionCredentialStatusRequest,
    ExtensionCredentialSubmitRequest, FS_LIST_VIEW, FS_MOUNTS_VIEW, FS_STAT_VIEW,
    FilesystemBrowseReader, FsMount, GLOBAL_AUTO_APPROVE_VIEW, InboundAttachmentLander,
    InboundAttachmentReader, LLM_ACTIVE_SET_CAPABILITY_ID, LLM_CONFIG_VIEW,
    LLM_PROVIDER_DELETE_CAPABILITY_ID, LLM_PROVIDER_UPSERT_CAPABILITY_ID, LOGS_VIEW,
    LifecycleChannelDirections, LifecycleExtensionCredentialRequirement,
    LifecycleExtensionCredentialSetup, LifecycleExtensionOnboarding, LifecycleExtensionRuntimeKind,
    LifecycleExtensionSource, LifecycleExtensionSummary, LifecycleInstalledExtensionSummary,
    LifecyclePackageKind, LifecyclePackageRef, LifecycleProductAction, LifecycleProductContext,
    LifecycleProductFacade, LifecycleProductPayload, LifecycleProductResponse,
    LifecycleReadinessBlocker, ListPendingApprovalsRequest, ListPendingApprovalsResponse,
    ListPendingAuthInteractionsRequest, ListPendingAuthInteractionsResponse, LlmActiveSelection,
    LlmConfigService, LlmConfigServiceError, LlmConfigSnapshot, LlmModelsResult, LlmProbeRequest,
    LlmProbeResult, LlmProviderView, NearAiLoginRequest, NearAiLoginStart,
    NearAiWalletLoginRequest, NearAiWalletLoginResult, OPERATOR_CONFIG_KEY_VIEW,
    OPERATOR_CONFIG_LIST_VIEW, OPERATOR_CONFIG_SET_AUTO_APPROVE_CAPABILITY_ID,
    OPERATOR_CONFIG_SET_TOOL_PERMISSION_CAPABILITY_ID, OPERATOR_CONFIG_VALIDATE_VIEW,
    OPERATOR_DIAGNOSTICS_VIEW, OPERATOR_LOGS_VIEW, OPERATOR_SETUP_RUN_CAPABILITY_ID,
    OPERATOR_SETUP_VIEW, OPERATOR_STATUS_VIEW, OUTBOUND_DELIVERY_TARGETS_VIEW,
    OUTBOUND_PREFERENCES_SET_CAPABILITY, OUTBOUND_PREFERENCES_SET_CAPABILITY_ID,
    OUTBOUND_PREFERENCES_VIEW, OperatorLogsService, OperatorServiceLifecycleService,
    OperatorStatusService, OutboundPreferencesProductFacade, PROJECT_DELETE_CAPABILITY_ID,
    PROJECT_FS_LIST_VIEW, PROJECT_FS_STAT_VIEW, PROJECT_MEMBER_ADD_CAPABILITY_ID,
    PROJECT_MEMBER_REMOVE_CAPABILITY_ID, PROJECT_MEMBER_UPDATE_CAPABILITY_ID, PROJECT_MEMBERS_VIEW,
    PROJECT_UPDATE_CAPABILITY_ID, PROJECT_VIEW, PROJECTS_VIEW, PendingApprovalInteractionView,
    ProductAgentBoundCaller, ProductCapabilityInput, ProductCapabilityInvoker, ProductOperationId,
    ProductSurface, ProductWorkflowError, ProjectCaller, ProjectFilesystemReader, ProjectFsEntry,
    ProjectFsEntryKind, ProjectFsError, ProjectFsFile, ProjectFsStat, ProjectService,
    ProjectServiceError, RUN_ARTIFACT_VIEW, RebornAccountTracesResponse, RebornAddMemberRequest,
    RebornAttachmentRequest, RebornAutomationInfo, RebornAutomationMutationResponse,
    RebornAutomationRecentRunInfo, RebornAutomationRecentRunStatus, RebornAutomationRequest,
    RebornAutomationRunStatus, RebornAutomationSource, RebornAutomationState,
    RebornChannelConfigField, RebornChannelConnectAction, RebornChannelConnectStrategy,
    RebornCreateProjectRequest, RebornDeleteProjectRequest, RebornDeleteThreadRequest,
    RebornExtensionListResponse, RebornExtensionOnboardingState, RebornExtensionSurface,
    RebornFsListRequest, RebornFsListResponse, RebornFsMountsRequest, RebornFsMountsResponse,
    RebornFsStatRequest, RebornFsStatResponse, RebornGetProjectRequest, RebornGetRunStateRequest,
    RebornGlobalAutoApproveRequest, RebornGlobalAutoApproveResponse, RebornListAutomationsResponse,
    RebornListMembersRequest, RebornListMembersResponse, RebornListProjectsRequest,
    RebornListProjectsResponse, RebornListThreadsResponse, RebornLogLevel, RebornLogQueryRequest,
    RebornLogQueryResponse, RebornOperatorCommandPlaneResponse,
    RebornOperatorConfigDiagnosticSeverity, RebornOperatorConfigGetResponse,
    RebornOperatorConfigListResponse, RebornOperatorConfigSetRequest,
    RebornOperatorConfigValidateResponse, RebornOperatorLogsQuery, RebornOperatorSetupRequest,
    RebornOperatorSetupStatus, RebornOperatorStatusCheck, RebornOperatorStatusResponse,
    RebornOperatorStatusSeverity, RebornOperatorStatusState, RebornOperatorSurfaceStatus,
    RebornOperatorToolCatalog, RebornOperatorToolInfo, RebornOutboundDeliveryModality,
    RebornOutboundDeliveryTargetCapabilities, RebornOutboundDeliveryTargetDescription,
    RebornOutboundDeliveryTargetId, RebornOutboundDeliveryTargetListResponse,
    RebornOutboundDeliveryTargetOption, RebornOutboundDeliveryTargetStatus,
    RebornOutboundDeliveryTargetSummary, RebornOutboundPreferencesResponse,
    RebornProjectFsListRequest, RebornProjectFsListResponse, RebornProjectFsStatRequest,
    RebornProjectFsStatResponse, RebornProjectInfo, RebornProjectMemberInfo,
    RebornProjectMemberStatus, RebornProjectResponse, RebornProjectRole, RebornProjectState,
    RebornRemoveMemberRequest, RebornRenameAutomationProductRequest, RebornResolveGateResponse,
    RebornRunArtifact, RebornRunArtifactRequest, RebornServiceLifecycleAction,
    RebornServiceLifecycleRequest, RebornServiceLifecycleResponse, RebornServiceLifecycleState,
    RebornServices, RebornServicesError, RebornServicesErrorCode, RebornServicesErrorKind,
    RebornSetOutboundPreferencesRequest, RebornSetupExtensionResponse, RebornSkillContentResponse,
    RebornSkillInfo, RebornSkillListResponse, RebornSkillSearchResponse, RebornSkillSourceKind,
    RebornSkillTrustLevel, RebornStreamEventsRequest, RebornSubmitTurnResponse,
    RebornTimelineRequest, RebornTimelineResponse, RebornTraceCreditsResponse,
    RebornUpdateMemberRoleRequest, RebornUpdateProjectRequest, RebornViewPage, RebornViewQuery,
    ResolveApprovalInteractionRequest, ResolveApprovalInteractionResponse,
    ResolveAuthInteractionRequest, ResolveAuthInteractionResponse, SKILL_CONTENT_VIEW,
    SKILL_SEARCH_VIEW, SKILLS_VIEW, SetActiveLlmRequest, SkillsProductFacade,
    StaticOperatorStatusService, THREAD_DELETE_CAPABILITY_ID, THREADS_VIEW, TIMELINE_VIEW,
    TRACE_ACCOUNT_TRACES_VIEW, TRACE_CREDITS_VIEW, TriggerRunThreadScope, UpsertLlmProviderRequest,
    WebUiAuthenticatedCaller, WebUiCancelRunRequest, WebUiCreateThreadRequest,
    WebUiInboundValidationCode, WebUiInboundValidationError, WebUiListAutomationsRequest,
    WebUiListThreadsRequest, WebUiRenameAutomationRequest, WebUiResolveGateRequest,
    WebUiRetryRunRequest, WebUiSendMessageRequest, WebUiSetupExtensionRequest, approval_gate_ref,
    automation_trigger_thread_metadata_json,
};
use ironclaw_product_workflow::{
    AdminCreateUserFields, AdminCreatedUser, AdminUserError, AdminUserRecord, AdminUserRole,
    AdminUserSecretMeta, AdminUserService, AdminUserStatus, RebornAdminCreateUserRequest,
    RebornAdminDeleteSecretProductRequest, RebornAdminPutSecretProductRequest,
    RebornAdminPutSecretRequest, RebornAdminSetRoleProductRequest, RebornAdminSetRoleRequest,
    RebornAdminSetStatusProductRequest, RebornAdminSetStatusRequest,
    RebornAdminUpdateUserProductRequest, RebornAdminUpdateUserRequest, RebornAdminUserListQuery,
    RebornAdminUserListResponse, RebornAdminUserRequest, RebornAdminUserResponse,
    RebornAdminUserSecretsListResponse,
};
use ironclaw_threads::{
    AcceptInboundMessageRequest, AcceptedInboundMessage, AcceptedInboundMessageReplay,
    AppendAssistantDraftRequest, AppendCapabilityDisplayPreviewRequest,
    AppendToolResultReferenceRequest, AttachmentKind, AttachmentRef, ContextMessages,
    ContextWindow, CreateSummaryArtifactRequest, EnsureThreadRequest, InMemorySessionThreadService,
    ListThreadsForScopeRequest, ListThreadsForScopeResponse, LoadContextMessagesRequest,
    LoadContextWindowRequest, MessageContent, MessageKind, MessageStatus, RedactMessageRequest,
    ReplayAcceptedInboundMessageRequest, SessionThreadError, SessionThreadRecord,
    SessionThreadService, SummaryArtifact, ThreadHistory, ThreadHistoryRequest, ThreadMessageId,
    ThreadMessageRecord, ThreadScope, UpdateAssistantDraftRequest,
    UpdateToolResultReferenceRequest,
};
use ironclaw_turns::run_profile::{LoopModelRouteSnapshot, LoopModelUsage};
use ironclaw_turns::test_support::in_memory_turn_state_store;
use ironclaw_turns::{
    AcceptedMessageRef, AdmissionRejection, AdmissionRejectionReason, CancelRunRequest,
    CancelRunResponse, DefaultTurnCoordinator, EventCursor, GateRef, GetRunStateRequest,
    ReplyTargetBindingRef, ResumeTurnPrecondition, ResumeTurnRequest, ResumeTurnResponse,
    RetryTurnRequest, RetryTurnResponse, RunProfileId, RunProfileVersion, SanitizedFailure,
    SourceBindingRef, SubmitTurnRequest, SubmitTurnResponse, TurnActor, TurnCapacityResource,
    TurnCoordinator, TurnError, TurnId, TurnOriginKind, TurnRunId, TurnRunState, TurnScope,
    TurnStatus,
};
use secrecy::SecretString;
use serde::Serialize;
use serde_json::json;
use tokio::sync::{Notify, oneshot};

fn caller() -> WebUiAuthenticatedCaller {
    caller_for_user("user-alpha")
}

/// Wait until the wall clock is strictly past `floor`, so the next thread
/// created/used gets a later activity timestamp — deterministic regardless
/// of clock resolution. Uses async sleep to avoid blocking the test runtime
/// (`std::thread::sleep` would block the tokio executor).
async fn wait_until_after(floor: chrono::DateTime<Utc>) {
    while Utc::now() <= floor {
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    }
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
            created_at: None,
            updated_at: None,
        },
        messages: vec![ThreadMessageRecord {
            message_id: ThreadMessageId::new(),
            thread_id,
            sequence: 1,
            kind: MessageKind::User,
            status: MessageStatus::Submitted,
            created_at: None,
            updated_at: None,
            actor_id: Some(owner.user_id.as_str().to_string()),
            source_binding_id: Some("webui-src:test".to_string()),
            reply_target_binding_id: Some("webui-reply:test".to_string()),
            turn_id: Some("turn-test".to_string()),
            turn_run_id: Some(run_id_string()),
            tool_result_ref: None,
            tool_result_provider_call: None,
            content: Some("timeline from fake M2 port".to_string()),
            attachments: Vec::new(),
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
    retries: Mutex<Vec<RetryTurnRequest>>,
    retry_attempts: Mutex<usize>,
    run_state_requests: Mutex<Vec<GetRunStateRequest>>,
    submit_error: Mutex<Option<TurnError>>,
    retry_error: Mutex<Option<TurnError>>,
    run_state_error: Mutex<Option<TurnError>>,
    run_state_actor: Mutex<Option<TurnActor>>,
    explicit_run_status: Mutex<Option<TurnStatus>>,
    parked_gate_ref: Mutex<Option<GateRef>>,
    parked_auth_gate: Mutex<bool>,
    parked_approval_gate: Mutex<bool>,
    run_state_failure: Mutex<Option<SanitizedFailure>>,
    run_state_usage: Mutex<Option<LoopModelUsage>>,
    run_state_model_route: Mutex<Option<LoopModelRouteSnapshot>>,
}

impl Default for FakeTurnCoordinator {
    fn default() -> Self {
        Self {
            submissions: Mutex::default(),
            cancellations: Mutex::default(),
            resumptions: Mutex::default(),
            retries: Mutex::default(),
            retry_attempts: Mutex::default(),
            run_state_requests: Mutex::default(),
            submit_error: Mutex::default(),
            retry_error: Mutex::default(),
            run_state_error: Mutex::default(),
            run_state_actor: Mutex::new(Some(turn_actor_for_user("user-alpha"))),
            explicit_run_status: Mutex::default(),
            parked_gate_ref: Mutex::default(),
            parked_auth_gate: Mutex::default(),
            parked_approval_gate: Mutex::default(),
            run_state_failure: Mutex::default(),
            run_state_usage: Mutex::default(),
            run_state_model_route: Mutex::default(),
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

    fn with_retry_error(error: TurnError) -> Self {
        Self {
            retry_error: Mutex::new(Some(error)),
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

    fn set_run_state_failure(&self, failure: SanitizedFailure) {
        *self.run_state_failure.lock().expect("lock") = Some(failure);
    }

    fn set_run_state_usage(&self, usage: LoopModelUsage, model_route: LoopModelRouteSnapshot) {
        *self.run_state_usage.lock().expect("lock") = Some(usage);
        *self.run_state_model_route.lock().expect("lock") = Some(model_route);
    }

    /// Report usage for a default-model run: token usage is captured but no
    /// `resolved_model_route` is set (the caller did not pick a model).
    fn set_run_state_usage_default_model(&self, usage: LoopModelUsage) {
        *self.run_state_usage.lock().expect("lock") = Some(usage);
        *self.run_state_model_route.lock().expect("lock") = None;
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

    fn retry_count(&self) -> usize {
        self.retries.lock().expect("lock").len()
    }

    fn retry_attempt_count(&self) -> usize {
        *self.retry_attempts.lock().expect("lock")
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

    fn last_retry(&self) -> Option<RetryTurnRequest> {
        self.retries.lock().expect("lock").last().cloned()
    }

    fn last_submission_scope(&self) -> Option<ironclaw_turns::TurnScope> {
        self.submissions
            .lock()
            .expect("lock")
            .last()
            .map(|request| request.scope.clone())
    }

    fn last_submission_origin_kind(&self) -> Option<TurnOriginKind> {
        self.submissions
            .lock()
            .expect("lock")
            .last()
            .and_then(|request| request.product_context.as_ref().map(|c| c.origin))
    }

    fn last_cancellation_scope(&self) -> Option<TurnScope> {
        self.cancellations
            .lock()
            .expect("lock")
            .last()
            .map(|request| request.scope.clone())
    }

    fn last_cancellation_actor(&self) -> Option<TurnActor> {
        self.cancellations
            .lock()
            .expect("lock")
            .last()
            .map(|request| request.actor.clone())
    }

    /// Returns the `TurnScope` from the most recent `get_run_state` call.
    ///
    /// Used by trigger-thread tests to assert that `resolve_gate`,
    /// `cancel_run`, and `get_run_state` receive the trigger-owned scope
    /// (with `owner_user_id = Some(creator_user_id)`) rather than the
    /// WebUI caller's session scope.  This distinction is what #4754 ("Part A")
    /// and the `check_automation_trigger_access` reconstruction guarantee.
    fn last_run_state_scope(&self) -> Option<TurnScope> {
        self.run_state_requests
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

    async fn retry_turn(&self, request: RetryTurnRequest) -> Result<RetryTurnResponse, TurnError> {
        *self.retry_attempts.lock().expect("lock") += 1;
        if let Some(error) = self.retry_error.lock().expect("lock").take() {
            return Err(error);
        }
        self.retries.lock().expect("lock").push(request);
        Ok(RetryTurnResponse {
            run_id: TurnRunId::new(),
            status: TurnStatus::Queued,
            event_cursor: EventCursor(19),
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
            resolved_model_route: self.run_state_model_route.lock().expect("lock").clone(),
            model_usage: *self.run_state_usage.lock().expect("lock"),
            received_at: Utc::now(),
            checkpoint_id: None,
            gate_ref,
            blocked_activity_id: None,
            credential_requirements: Vec::new(),
            failure: self.run_state_failure.lock().expect("lock").clone(),
            event_cursor: EventCursor(17),
            product_context: None,
            resume_disposition: None,
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

    async fn retry_turn(
        &self,
        _request: ironclaw_turns::RetryTurnRequest,
    ) -> Result<ironclaw_turns::RetryTurnResponse, TurnError> {
        panic!("retry_turn is not used by delete submit serialization tests")
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
            model_usage: None,
            received_at: Utc::now(),
            checkpoint_id: None,
            gate_ref: None,
            blocked_activity_id: None,
            credential_requirements: Vec::new(),
            failure: None,
            event_cursor: EventCursor(29),
            product_context: None,
            resume_disposition: None,
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
            ApprovalInteractionDecision::ApproveOnce | ApprovalInteractionDecision::AlwaysAllow => {
                ResolveApprovalInteractionResponse::Approved(ResumeTurnResponse {
                    run_id,
                    status: TurnStatus::Queued,
                    event_cursor: EventCursor(19),
                })
            }
            ApprovalInteractionDecision::Deny => {
                ResolveApprovalInteractionResponse::Resumed(ResumeTurnResponse {
                    run_id,
                    status: TurnStatus::Queued,
                    event_cursor: EventCursor(23),
                })
            }
        })
    }
}

struct ThreadScopedApprovalInteractionService {
    pending_thread_ids: HashSet<ThreadId>,
}

#[async_trait]
impl ApprovalInteractionService for ThreadScopedApprovalInteractionService {
    async fn list_pending(
        &self,
        request: ListPendingApprovalsRequest,
    ) -> Result<ListPendingApprovalsResponse, ProductWorkflowError> {
        if !self.pending_thread_ids.contains(&request.scope.thread_id) {
            return Ok(ListPendingApprovalsResponse { approvals: vec![] });
        }
        let approval_request_id = ApprovalRequestId::new();
        Ok(ListPendingApprovalsResponse {
            approvals: vec![PendingApprovalInteractionView {
                scope: ApprovalInteractionScope::from_turn(&request.scope, &request.actor),
                run_id: TurnRunId::new(),
                gate_ref: approval_gate_ref(approval_request_id).expect("approval gate ref"),
                approval_request_id,
                summary: "Approval required".to_string(),
                action: ApprovalInteractionActionView::Dispatch {
                    capability_id: CapabilityId::new("demo.echo").expect("capability id"),
                },
            }],
        })
    }

    async fn resolve(
        &self,
        _request: ResolveApprovalInteractionRequest,
    ) -> Result<ResolveApprovalInteractionResponse, ProductWorkflowError> {
        panic!("resolve is not used by thread approval filter tests")
    }
}

struct ActorFallbackApprovalInteractionService {
    pending_thread_id: ThreadId,
    tenant_id: TenantId,
    owner_user_id: UserId,
    agent_id: AgentId,
    project_id: Option<ProjectId>,
}

#[async_trait]
impl ApprovalInteractionService for ActorFallbackApprovalInteractionService {
    async fn list_pending(
        &self,
        request: ListPendingApprovalsRequest,
    ) -> Result<ListPendingApprovalsResponse, ProductWorkflowError> {
        let is_expected_scope = request.scope.thread_id == self.pending_thread_id
            && request.scope.tenant_id == self.tenant_id
            && request.scope.agent_id.as_ref() == Some(&self.agent_id)
            && request.scope.project_id == self.project_id
            && !request.scope.has_explicit_thread_owner()
            && request.actor.user_id == self.owner_user_id;
        if !is_expected_scope {
            return Ok(ListPendingApprovalsResponse { approvals: vec![] });
        }
        let approval_request_id = ApprovalRequestId::new();
        Ok(ListPendingApprovalsResponse {
            approvals: vec![PendingApprovalInteractionView {
                scope: ApprovalInteractionScope::from_turn(&request.scope, &request.actor),
                run_id: TurnRunId::new(),
                gate_ref: approval_gate_ref(approval_request_id).expect("approval gate ref"),
                approval_request_id,
                summary: "Approval required".to_string(),
                action: ApprovalInteractionActionView::Dispatch {
                    capability_id: CapabilityId::new("demo.echo").expect("capability id"),
                },
            }],
        })
    }

    async fn resolve(
        &self,
        _request: ResolveApprovalInteractionRequest,
    ) -> Result<ResolveApprovalInteractionResponse, ProductWorkflowError> {
        panic!("resolve is not used by actor-fallback approval list tests")
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
    imported_bundles: Mutex<Vec<Vec<u8>>>,
    credential_requirements: Vec<LifecycleExtensionCredentialRequirement>,
    onboarding: Option<LifecycleExtensionOnboarding>,
}

impl RecordingLifecycleFacade {
    fn new() -> Self {
        Self {
            package_refs: Mutex::new(Vec::new()),
            imported_bundles: Mutex::new(Vec::new()),
            credential_requirements: Vec::new(),
            onboarding: None,
        }
    }

    fn with_credential_requirements(
        credential_requirements: Vec<LifecycleExtensionCredentialRequirement>,
    ) -> Self {
        Self {
            package_refs: Mutex::new(Vec::new()),
            imported_bundles: Mutex::new(Vec::new()),
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
            imported_bundles: Mutex::new(Vec::new()),
            credential_requirements,
            onboarding: Some(onboarding),
        }
    }

    fn package_refs(&self) -> Vec<LifecyclePackageRef> {
        self.package_refs.lock().expect("lock").clone()
    }

    fn imported_bundles(&self) -> Vec<Vec<u8>> {
        self.imported_bundles.lock().expect("lock").clone()
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
            surface_kinds: Vec::new(),
            channel_directions: None,
            channel_connection: None,
            channel_presentation: None,
            visible_capability_ids: Vec::new(),
            visible_read_only_capability_ids: Vec::new(),
            credential_requirements: self.credential_requirements.clone(),
            onboarding: self.onboarding.clone(),
        };
        Some(LifecycleProductPayload::ExtensionList {
            extensions: vec![LifecycleInstalledExtensionSummary {
                summary,
                phase: InstallationState::Configured,
                install_scope: None,
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
            InstallationState::Unsupported
        } else {
            InstallationState::Configured
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

    async fn import_extension_bundle(
        &self,
        _context: LifecycleProductContext,
        bundle: Vec<u8>,
    ) -> Result<LifecycleProductResponse, ironclaw_product_workflow::ProductWorkflowError> {
        self.imported_bundles.lock().expect("lock").push(bundle);
        Ok(LifecycleProductResponse {
            package_ref: None,
            phase: InstallationState::Installed,
            blockers: Vec::new(),
            message: Some("imported".to_string()),
            payload: None,
        })
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
    include_completed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AutomationMutationAction {
    Pause,
    Resume,
    Rename { name: AutomationName },
    Delete,
}

#[derive(Debug, Clone)]
struct AutomationMutationCall {
    caller: ProductAgentBoundCaller,
    automation_id: String,
    action: AutomationMutationAction,
}

#[derive(Default)]
struct RecordingAutomationFacade {
    list_calls: Mutex<Vec<ListAutomationCall>>,
    mutation_calls: Mutex<Vec<AutomationMutationCall>>,
}

impl RecordingAutomationFacade {
    fn list_calls(&self) -> Vec<ListAutomationCall> {
        self.list_calls.lock().expect("lock").clone()
    }

    fn mutation_calls(&self) -> Vec<AutomationMutationCall> {
        self.mutation_calls.lock().expect("lock").clone()
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
                include_completed: request.include_completed,
            });
        Ok(vec![automation_info(
            "trigger-listed",
            "Daily status",
            "0 9 * * *",
            Some(RebornAutomationRunStatus::Ok),
        )])
    }

    async fn resolve_run_thread_scope(
        &self,
        _caller: ProductAgentBoundCaller,
        _thread_id: &ThreadId,
    ) -> Result<Option<TriggerRunThreadScope>, RebornServicesError> {
        // Trigger-thread access is not wired in the recording facade.
        Ok(None)
    }

    async fn pause_automation(
        &self,
        caller: ProductAgentBoundCaller,
        automation_id: String,
    ) -> Result<RebornAutomationMutationResponse, RebornServicesError> {
        self.mutation_calls
            .lock()
            .expect("lock")
            .push(AutomationMutationCall {
                caller,
                automation_id,
                action: AutomationMutationAction::Pause,
            });
        Ok(RebornAutomationMutationResponse {
            updated: true,
            automation: Some(automation_info(
                "trigger-paused",
                "Daily status",
                "0 9 * * *",
                None,
            )),
        })
    }

    async fn resume_automation(
        &self,
        caller: ProductAgentBoundCaller,
        automation_id: String,
    ) -> Result<RebornAutomationMutationResponse, RebornServicesError> {
        self.mutation_calls
            .lock()
            .expect("lock")
            .push(AutomationMutationCall {
                caller,
                automation_id,
                action: AutomationMutationAction::Resume,
            });
        Ok(RebornAutomationMutationResponse {
            updated: true,
            automation: Some(automation_info(
                "trigger-resumed",
                "Daily status",
                "0 9 * * *",
                None,
            )),
        })
    }

    async fn rename_automation(
        &self,
        caller: ProductAgentBoundCaller,
        automation_id: String,
        name: AutomationName,
    ) -> Result<RebornAutomationMutationResponse, RebornServicesError> {
        self.mutation_calls
            .lock()
            .expect("lock")
            .push(AutomationMutationCall {
                caller,
                automation_id,
                action: AutomationMutationAction::Rename { name },
            });
        Ok(RebornAutomationMutationResponse {
            updated: true,
            automation: Some(automation_info(
                "trigger-renamed",
                "Renamed status",
                "0 9 * * *",
                None,
            )),
        })
    }

    async fn delete_automation(
        &self,
        caller: ProductAgentBoundCaller,
        automation_id: String,
    ) -> Result<RebornAutomationMutationResponse, RebornServicesError> {
        self.mutation_calls
            .lock()
            .expect("lock")
            .push(AutomationMutationCall {
                caller,
                automation_id,
                action: AutomationMutationAction::Delete,
            });
        Ok(RebornAutomationMutationResponse {
            updated: true,
            automation: None,
        })
    }
}

#[derive(Clone)]
struct StaticAutomationFacade {
    output: Vec<RebornAutomationInfo>,
    scheduler_enabled: bool,
    list_calls: Arc<Mutex<Vec<ListAutomationCall>>>,
    /// Scopes returned by `resolve_run_thread_scope`, keyed by the queried
    /// thread id so tests prove the lookup contract rather than accepting a
    /// cached scope for any request.
    resolve_scopes: HashMap<ThreadId, TriggerRunThreadScope>,
    resolve_calls: Arc<Mutex<Vec<ThreadId>>>,
}

impl StaticAutomationFacade {
    fn new(output: Vec<RebornAutomationInfo>) -> Self {
        Self {
            output,
            scheduler_enabled: true,
            list_calls: Arc::new(Mutex::new(Vec::new())),
            resolve_scopes: HashMap::new(),
            resolve_calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn with_scheduler_enabled(mut self, scheduler_enabled: bool) -> Self {
        self.scheduler_enabled = scheduler_enabled;
        self
    }

    fn with_resolve_scope_for_thread(
        mut self,
        thread_id: ThreadId,
        scope: TriggerRunThreadScope,
    ) -> Self {
        self.resolve_scopes.insert(thread_id, scope);
        self
    }

    fn resolve_calls(&self) -> Vec<ThreadId> {
        self.resolve_calls.lock().expect("lock").clone()
    }

    fn list_calls(&self) -> Vec<ListAutomationCall> {
        self.list_calls.lock().expect("lock").clone()
    }
}

#[async_trait]
impl AutomationProductFacade for StaticAutomationFacade {
    fn scheduler_enabled(&self) -> bool {
        self.scheduler_enabled
    }

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
                include_completed: request.include_completed,
            });
        Ok(self.output.clone())
    }

    async fn resolve_run_thread_scope(
        &self,
        _caller: ProductAgentBoundCaller,
        thread_id: &ThreadId,
    ) -> Result<Option<TriggerRunThreadScope>, RebornServicesError> {
        self.resolve_calls
            .lock()
            .expect("lock")
            .push(thread_id.clone());
        Ok(self.resolve_scopes.get(thread_id).cloned())
    }
}

/// An automation facade that initially exposes one trigger thread scope but can
/// have that scope revoked via `revoke()`. Used to verify that the service
/// revalidates authorization on every call rather than caching the result.
struct RevocableAutomationFacade {
    thread_id: ThreadId,
    scope: TriggerRunThreadScope,
    revoked: Mutex<bool>,
}

impl RevocableAutomationFacade {
    fn new(thread_id: ThreadId, caller: &WebUiAuthenticatedCaller) -> Self {
        let scope = TriggerRunThreadScope {
            agent_id: caller.agent_id.clone(),
            project_id: caller.project_id.clone(),
            creator_user_id: caller.user_id.clone(),
        };
        Self {
            thread_id,
            scope,
            revoked: Mutex::new(false),
        }
    }

    fn revoke(&self) {
        *self.revoked.lock().expect("lock") = true;
    }
}

#[async_trait]
impl AutomationProductFacade for RevocableAutomationFacade {
    async fn list_automations(
        &self,
        _caller: ProductAgentBoundCaller,
        _request: AutomationListRequest,
    ) -> Result<Vec<RebornAutomationInfo>, RebornServicesError> {
        Ok(Vec::new())
    }

    async fn resolve_run_thread_scope(
        &self,
        _caller: ProductAgentBoundCaller,
        thread_id: &ThreadId,
    ) -> Result<Option<TriggerRunThreadScope>, RebornServicesError> {
        if *self.revoked.lock().expect("lock") {
            return Ok(None);
        }
        if thread_id == &self.thread_id {
            Ok(Some(self.scope.clone()))
        } else {
            Ok(None)
        }
    }
}

/// An automation facade whose `resolve_run_thread_scope` always returns a
/// backend error (503 Unavailable, retryable). Used to verify that the timeline
/// call surfaces the backend error rather than masking it as a 404.
struct ErroringAutomationFacade {
    error: RebornServicesError,
}

impl ErroringAutomationFacade {
    fn unavailable() -> Self {
        Self {
            error: RebornServicesError {
                code: RebornServicesErrorCode::Unavailable,
                kind: RebornServicesErrorKind::ServiceUnavailable,
                status_code: 503,
                retryable: true,
                field: None,
                validation_code: None,
            },
        }
    }
}

#[async_trait]
impl AutomationProductFacade for ErroringAutomationFacade {
    async fn list_automations(
        &self,
        _caller: ProductAgentBoundCaller,
        _request: AutomationListRequest,
    ) -> Result<Vec<RebornAutomationInfo>, RebornServicesError> {
        Ok(Vec::new())
    }

    async fn resolve_run_thread_scope(
        &self,
        _caller: ProductAgentBoundCaller,
        _thread_id: &ThreadId,
    ) -> Result<Option<TriggerRunThreadScope>, RebornServicesError> {
        Err(self.error.clone())
    }
}

#[derive(Default)]
struct RecordingOutboundPreferencesFacade {
    get_calls: Mutex<Vec<WebUiAuthenticatedCaller>>,
    set_calls: Mutex<usize>,
    list_calls: Mutex<Vec<WebUiAuthenticatedCaller>>,
}

impl RecordingOutboundPreferencesFacade {
    fn get_calls(&self) -> Vec<WebUiAuthenticatedCaller> {
        self.get_calls.lock().expect("lock").clone()
    }

    fn set_calls(&self) -> usize {
        *self.set_calls.lock().expect("lock")
    }

    fn list_calls(&self) -> Vec<WebUiAuthenticatedCaller> {
        self.list_calls.lock().expect("lock").clone()
    }
}

type OutboundPreferencesInvokeCall = (WebUiAuthenticatedCaller, CapabilityId, serde_json::Value);

#[derive(Default, Clone)]
struct RecordingOutboundPreferencesInvoker {
    calls: Arc<Mutex<Vec<OutboundPreferencesInvokeCall>>>,
}

impl RecordingOutboundPreferencesInvoker {
    fn calls(&self) -> Vec<OutboundPreferencesInvokeCall> {
        self.calls.lock().expect("lock").clone()
    }
}

#[async_trait]
impl ProductCapabilityInvoker for RecordingOutboundPreferencesInvoker {
    async fn invoke(
        &self,
        caller: WebUiAuthenticatedCaller,
        capability: CapabilityId,
        input: serde_json::Value,
        activity_id: ActivityId,
    ) -> Result<Resolution, RebornServicesError> {
        self.calls
            .lock()
            .expect("lock")
            .push((caller, capability, input));
        Ok(operator_config_success_resolution(activity_id))
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
        let _ = (caller, request);
        *self.set_calls.lock().expect("lock") += 1;
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
        source: RebornAutomationSource::Schedule {
            cron: cron.into(),
            timezone: "UTC".to_string(),
        },
        state: RebornAutomationState::Active,
        next_run_at: Some("2026-06-03T09:00:00Z".parse().expect("next run")),
        last_run_at: None,
        last_status,
        recent_runs: vec![RebornAutomationRecentRunInfo {
            run_id: Some(automation_run_id()),
            thread_id: Some(ThreadId::new("thread-listed").expect("valid thread id")),
            fire_slot: Some("2026-06-03T09:00:00Z".parse().expect("fire slot")),
            status: RebornAutomationRecentRunStatus::Ok,
            submitted_at: "2026-06-03T09:00:01Z".parse().expect("submitted at"),
            completed_at: Some("2026-06-03T09:00:42Z".parse().expect("completed at")),
        }],
        is_active: true,
        created_at: Some("2026-06-02T18:00:00Z".parse().expect("created at")),
        active_hold: None,
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

    async fn mark_message_rejected_busy(
        &self,
        _scope: &ThreadScope,
        _thread_id: &ThreadId,
        _message_id: ThreadMessageId,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        panic!("ScopeMismatchThreadStub::mark_message_rejected_busy should not be reached")
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
    SubmittedReplay {
        turn_run_id: Option<String>,
    },
    RejectedBusyReplay,
    /// `mark_message_rejected_busy` fails; reconcile path replays the accepted
    /// message as RejectedBusy so no error surfaces to the caller.
    RejectedBusyMarkFails {
        /// Message id assigned by `accept_inbound_message`, shared so that
        /// `reconcile_terminal_duplicate` can match it against the handoff.
        message_id: ThreadMessageId,
    },
    /// `mark_message_rejected_busy` fails; reconcile path replays the accepted
    /// message as legacy DeferredBusy.  Unlike `RejectedBusyMarkFails`,
    /// `DeferredBusy` is non-terminal: `reconcile_terminal_duplicate` accepts
    /// only `RejectedBusy` as settled, so this replay does NOT satisfy
    /// reconciliation.  The original mark failure surfaces as a retryable error
    /// (Unavailable / 503) rather than a false-terminal RejectedBusy.
    DeferredBusyMarkFails {
        /// Message id assigned by `accept_inbound_message`, shared so that
        /// `reconcile_terminal_duplicate` can match it against the handoff.
        message_id: ThreadMessageId,
    },
}

struct ScriptedThreadService {
    behavior: ScriptedThreadBehavior,
    history_requests: Mutex<Vec<ThreadHistoryRequest>>,
    list_requests: Mutex<Vec<ListThreadsForScopeRequest>>,
    list_responses: Mutex<Vec<ListThreadsForScopeResponse>>,
    /// Tracks `replay_accepted_inbound_message` call count; used by
    /// `RejectedBusyMarkFails` (and `DeferredBusyMarkFails`) to return `None`
    /// on the first two calls (idempotency probes) and `Some(…)` on the third
    /// call (reconcile probe) onward.
    replay_call_count: Mutex<usize>,
}

impl ScriptedThreadService {
    fn backend_history() -> Self {
        Self {
            behavior: ScriptedThreadBehavior::BackendHistory,
            history_requests: Mutex::new(Vec::new()),
            list_requests: Mutex::new(Vec::new()),
            list_responses: Mutex::new(Vec::new()),
            replay_call_count: Mutex::new(0),
        }
    }

    fn history(history: ThreadHistory) -> Self {
        Self {
            behavior: ScriptedThreadBehavior::History(Box::new(history)),
            history_requests: Mutex::new(Vec::new()),
            list_requests: Mutex::new(Vec::new()),
            list_responses: Mutex::new(Vec::new()),
            replay_call_count: Mutex::new(0),
        }
    }

    fn list_pages(responses: Vec<ListThreadsForScopeResponse>) -> Self {
        Self {
            behavior: ScriptedThreadBehavior::ListPages,
            history_requests: Mutex::new(Vec::new()),
            list_requests: Mutex::new(Vec::new()),
            list_responses: Mutex::new(responses),
            replay_call_count: Mutex::new(0),
        }
    }

    fn submitted_replay(turn_run_id: Option<String>) -> Self {
        Self {
            behavior: ScriptedThreadBehavior::SubmittedReplay { turn_run_id },
            history_requests: Mutex::new(Vec::new()),
            list_requests: Mutex::new(Vec::new()),
            list_responses: Mutex::new(Vec::new()),
            replay_call_count: Mutex::new(0),
        }
    }

    fn rejected_busy_replay() -> Self {
        Self {
            behavior: ScriptedThreadBehavior::RejectedBusyReplay,
            history_requests: Mutex::new(Vec::new()),
            list_requests: Mutex::new(Vec::new()),
            list_responses: Mutex::new(Vec::new()),
            replay_call_count: Mutex::new(0),
        }
    }

    /// Scripted service for the mark-failure reconcile path:
    /// - `accept_inbound_message` accepts the message
    /// - `mark_message_rejected_busy` returns a backend error
    /// - `replay_accepted_inbound_message` returns `None` on the first two
    ///   calls (idempotency probes) and `Some(RejectedBusy)` on the third
    ///   call (reconcile probe), so `reconcile_terminal_duplicate` settles
    ///   without error
    fn rejected_busy_mark_fails() -> Self {
        Self {
            behavior: ScriptedThreadBehavior::RejectedBusyMarkFails {
                message_id: ThreadMessageId::new(),
            },
            history_requests: Mutex::new(Vec::new()),
            list_requests: Mutex::new(Vec::new()),
            list_responses: Mutex::new(Vec::new()),
            replay_call_count: Mutex::new(0),
        }
    }

    /// Scripted service for the legacy DeferredBusy mark-failure path:
    /// - `accept_inbound_message` accepts the message
    /// - `mark_message_rejected_busy` returns a backend error
    /// - `replay_accepted_inbound_message` returns `None` on the first two
    ///   calls (idempotency probes) and `Some(DeferredBusy)` on the reconcile
    ///   probe.  `DeferredBusy` is non-terminal: `reconcile_terminal_duplicate`
    ///   no longer accepts it as settled (only `RejectedBusy` qualifies), so the
    ///   mark failure propagates as a retryable Unavailable error rather than
    ///   silently producing a false-terminal RejectedBusy.
    fn deferred_busy_mark_fails() -> Self {
        Self {
            behavior: ScriptedThreadBehavior::DeferredBusyMarkFails {
                message_id: ThreadMessageId::new(),
            },
            history_requests: Mutex::new(Vec::new()),
            list_requests: Mutex::new(Vec::new()),
            list_responses: Mutex::new(Vec::new()),
            replay_call_count: Mutex::new(0),
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
            ScriptedThreadBehavior::SubmittedReplay { .. }
            | ScriptedThreadBehavior::RejectedBusyReplay
            | ScriptedThreadBehavior::RejectedBusyMarkFails { .. }
            | ScriptedThreadBehavior::DeferredBusyMarkFails { .. } => Ok(ThreadHistory {
                thread: SessionThreadRecord {
                    scope: request.scope,
                    thread_id: request.thread_id,
                    created_by_actor_id: "user-alpha".to_string(),
                    title: None,
                    metadata_json: None,
                    goal: None,
                    created_at: None,
                    updated_at: None,
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
        request: AcceptInboundMessageRequest,
    ) -> Result<AcceptedInboundMessage, SessionThreadError> {
        match &self.behavior {
            ScriptedThreadBehavior::RejectedBusyMarkFails { message_id }
            | ScriptedThreadBehavior::DeferredBusyMarkFails { message_id } => {
                Ok(AcceptedInboundMessage {
                    thread_id: request.thread_id,
                    message_id: *message_id,
                    sequence: 1,
                    idempotent_replay: false,
                })
            }
            _ => scripted_stub_unreachable("accept_inbound_message"),
        }
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
            ScriptedThreadBehavior::RejectedBusyReplay => Ok(Some(AcceptedInboundMessageReplay {
                scope: request.scope,
                thread_id: ThreadId::new("thread-alpha").expect("valid thread"),
                message_id: ThreadMessageId::new(),
                sequence: 1,
                status: MessageStatus::RejectedBusy,
                actor_id: Some(request.actor_id),
                source_binding_id: Some(request.source_binding_id),
                reply_target_binding_id: Some("webui-reply:replayed".to_string()),
                turn_run_id: None,
            })),
            ScriptedThreadBehavior::RejectedBusyMarkFails { message_id } => {
                // replay_webui_send_message probes with two source-binding variants
                // (main + legacy) before accepting the message, so calls 1 and 2
                // are the initial idempotency probes — both must return None so
                // accept_inbound_message is reached.  Call 3+ comes from
                // reconcile_terminal_duplicate after mark_message_rejected_busy
                // fails; return the already-settled RejectedBusy so reconciliation
                // succeeds without propagating the mark error.
                let mut count = self.replay_call_count.lock().expect("lock");
                *count += 1;
                if *count <= 2 {
                    Ok(None)
                } else {
                    Ok(Some(AcceptedInboundMessageReplay {
                        scope: request.scope,
                        thread_id: ThreadId::new("thread-alpha").expect("valid thread"),
                        message_id: *message_id,
                        sequence: 1,
                        status: MessageStatus::RejectedBusy,
                        actor_id: Some(request.actor_id),
                        source_binding_id: Some(request.source_binding_id),
                        reply_target_binding_id: Some("webui-reply:replayed".to_string()),
                        turn_run_id: None,
                    }))
                }
            }
            ScriptedThreadBehavior::DeferredBusyMarkFails { message_id } => {
                // Same two-phase probe as RejectedBusyMarkFails: calls 1 and 2 are
                // the initial idempotency probes and must return None.  Call 3+
                // comes from reconcile_terminal_duplicate; return legacy DeferredBusy.
                // DeferredBusy is non-terminal — reconcile_terminal_duplicate accepts
                // only RejectedBusy as settled, so this replay does NOT satisfy
                // reconciliation.  The original mark failure surfaces as an error.
                let mut count = self.replay_call_count.lock().expect("lock");
                *count += 1;
                if *count <= 2 {
                    Ok(None)
                } else {
                    Ok(Some(AcceptedInboundMessageReplay {
                        scope: request.scope,
                        thread_id: ThreadId::new("thread-alpha").expect("valid thread"),
                        message_id: *message_id,
                        sequence: 1,
                        status: MessageStatus::DeferredBusy,
                        actor_id: Some(request.actor_id),
                        source_binding_id: Some(request.source_binding_id),
                        reply_target_binding_id: Some("webui-reply:replayed".to_string()),
                        turn_run_id: None,
                    }))
                }
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

    async fn mark_message_rejected_busy(
        &self,
        _scope: &ThreadScope,
        _thread_id: &ThreadId,
        _message_id: ThreadMessageId,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        match &self.behavior {
            ScriptedThreadBehavior::RejectedBusyMarkFails { .. }
            | ScriptedThreadBehavior::DeferredBusyMarkFails { .. } => {
                Err(SessionThreadError::Backend(
                    "simulated backend failure in mark_message_rejected_busy".to_string(),
                ))
            }
            _ => scripted_stub_unreachable("mark_message_rejected_busy"),
        }
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
async fn default_invoke_uses_canonical_host_types_and_fails_closed() {
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    );

    let result: Result<Resolution, RebornServicesError> = services
        .invoke(
            caller(),
            CapabilityId::new("product.test_invoke").expect("valid capability id"),
            ProductCapabilityInput::json(json!({"request": "test"})),
            ActivityId::new(),
        )
        .await;

    let error = result.expect_err("unwired invoke must fail closed");
    assert_eq!(error.code, RebornServicesErrorCode::Unavailable);
    assert_eq!(error.kind, RebornServicesErrorKind::ServiceUnavailable);
    assert_eq!(error.status_code, 503);
    assert!(!error.retryable);
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

/// Project service that authorizes exactly one project id through `get_project`
/// and fails everything else, so create-thread project authorization can be
/// driven from the caller without a real repository.
#[derive(Debug)]
struct AuthorizingProjectService {
    allowed_project_id: String,
}

#[async_trait]
impl ProjectService for AuthorizingProjectService {
    async fn list_projects(
        &self,
        _caller: ProjectCaller,
        _request: RebornListProjectsRequest,
    ) -> Result<RebornListProjectsResponse, ProjectServiceError> {
        Err(ProjectServiceError::Internal)
    }

    async fn create_project(
        &self,
        _caller: ProjectCaller,
        _request: RebornCreateProjectRequest,
    ) -> Result<RebornProjectResponse, ProjectServiceError> {
        Err(ProjectServiceError::Internal)
    }

    async fn get_project(
        &self,
        _caller: ProjectCaller,
        request: RebornGetProjectRequest,
    ) -> Result<RebornProjectResponse, ProjectServiceError> {
        if request.project_id == self.allowed_project_id {
            Ok(RebornProjectResponse {
                project: RebornProjectInfo {
                    project_id: self.allowed_project_id.clone(),
                    name: "Authorized".to_string(),
                    description: String::new(),
                    icon: None,
                    color: None,
                    metadata: serde_json::json!({}),
                    state: RebornProjectState::Active,
                    role: RebornProjectRole::Owner,
                    created_at: "1970-01-01T00:00:00Z".parse().expect("created at"),
                    updated_at: "1970-01-01T00:00:00Z".parse().expect("updated at"),
                },
            })
        } else {
            // Mirrors the real service: no access (or unknown) collapses to NotFound.
            Err(ProjectServiceError::NotFound)
        }
    }

    async fn update_project(
        &self,
        _caller: ProjectCaller,
        _request: RebornUpdateProjectRequest,
    ) -> Result<RebornProjectResponse, ProjectServiceError> {
        Err(ProjectServiceError::Internal)
    }

    async fn delete_project(
        &self,
        _caller: ProjectCaller,
        _request: RebornDeleteProjectRequest,
    ) -> Result<(), ProjectServiceError> {
        Err(ProjectServiceError::Internal)
    }

    async fn list_members(
        &self,
        _caller: ProjectCaller,
        _request: RebornListMembersRequest,
    ) -> Result<RebornListMembersResponse, ProjectServiceError> {
        Err(ProjectServiceError::Internal)
    }

    async fn add_member(
        &self,
        _caller: ProjectCaller,
        _request: RebornAddMemberRequest,
    ) -> Result<RebornProjectMemberInfo, ProjectServiceError> {
        Err(ProjectServiceError::Internal)
    }

    async fn update_member_role(
        &self,
        _caller: ProjectCaller,
        _request: RebornUpdateMemberRoleRequest,
    ) -> Result<RebornProjectMemberInfo, ProjectServiceError> {
        Err(ProjectServiceError::Internal)
    }

    async fn remove_member(
        &self,
        _caller: ProjectCaller,
        _request: RebornRemoveMemberRequest,
    ) -> Result<(), ProjectServiceError> {
        Err(ProjectServiceError::Internal)
    }
}

struct EmptyFilesystemBrowser;

#[async_trait]
impl FilesystemBrowseReader for EmptyFilesystemBrowser {
    fn available_mounts(&self) -> Vec<FsMount> {
        vec![FsMount::Memory]
    }

    async fn list_dir(
        &self,
        _scope: &ResourceScope,
        _mount: FsMount,
        _path: &str,
    ) -> Result<Vec<ProjectFsEntry>, ProjectFsError> {
        Ok(Vec::new())
    }

    async fn read_file(
        &self,
        _scope: &ResourceScope,
        _mount: FsMount,
        _path: &str,
    ) -> Result<ProjectFsFile, ProjectFsError> {
        Err(ProjectFsError::NotFound)
    }

    async fn stat(
        &self,
        _scope: &ResourceScope,
        _mount: FsMount,
        _path: &str,
    ) -> Result<ProjectFsStat, ProjectFsError> {
        Err(ProjectFsError::NotFound)
    }
}

struct StaticProjectFilesystemReader;

#[async_trait]
impl ProjectFilesystemReader for StaticProjectFilesystemReader {
    async fn list_dir(
        &self,
        _thread_scope: &ThreadScope,
        path: &str,
    ) -> Result<Vec<ProjectFsEntry>, ProjectFsError> {
        Ok(vec![ProjectFsEntry {
            name: "report.md".to_string(),
            path: format!("{}/report.md", path.trim_end_matches('/')),
            kind: ProjectFsEntryKind::File,
        }])
    }

    async fn read_file(
        &self,
        _thread_scope: &ThreadScope,
        _path: &str,
    ) -> Result<ProjectFsFile, ProjectFsError> {
        Err(ProjectFsError::NotFound)
    }

    async fn stat(
        &self,
        _thread_scope: &ThreadScope,
        path: &str,
    ) -> Result<ProjectFsStat, ProjectFsError> {
        Ok(ProjectFsStat {
            path: path.to_string(),
            kind: ProjectFsEntryKind::File,
            size_bytes: 7,
            mime_type: "text/markdown".to_string(),
        })
    }
}

struct StaticFilesystemBrowser;

#[async_trait]
impl FilesystemBrowseReader for StaticFilesystemBrowser {
    fn available_mounts(&self) -> Vec<FsMount> {
        vec![FsMount::Memory, FsMount::Workspace]
    }

    async fn list_dir(
        &self,
        _scope: &ResourceScope,
        _mount: FsMount,
        path: &str,
    ) -> Result<Vec<ProjectFsEntry>, ProjectFsError> {
        Ok(vec![ProjectFsEntry {
            name: "today.md".to_string(),
            path: format!("{}/today.md", path.trim_end_matches('/')),
            kind: ProjectFsEntryKind::File,
        }])
    }

    async fn read_file(
        &self,
        _scope: &ResourceScope,
        _mount: FsMount,
        _path: &str,
    ) -> Result<ProjectFsFile, ProjectFsError> {
        Err(ProjectFsError::NotFound)
    }

    async fn stat(
        &self,
        _scope: &ResourceScope,
        _mount: FsMount,
        path: &str,
    ) -> Result<ProjectFsStat, ProjectFsError> {
        Ok(ProjectFsStat {
            path: path.to_string(),
            kind: ProjectFsEntryKind::File,
            size_bytes: 7,
            mime_type: "text/markdown".to_string(),
        })
    }
}

#[derive(Default)]
struct RecordingProjectService {
    listed: Mutex<usize>,
    read: Mutex<Vec<String>>,
    updated: Mutex<Vec<RebornUpdateProjectRequest>>,
    deleted: Mutex<Vec<String>>,
    listed_members: Mutex<Vec<String>>,
    added_members: Mutex<Vec<RebornAddMemberRequest>>,
    updated_members: Mutex<Vec<RebornUpdateMemberRoleRequest>>,
    removed_members: Mutex<Vec<RebornRemoveMemberRequest>>,
}

#[async_trait]
impl ProjectService for RecordingProjectService {
    async fn list_projects(
        &self,
        _caller: ProjectCaller,
        _request: RebornListProjectsRequest,
    ) -> Result<RebornListProjectsResponse, ProjectServiceError> {
        *self.listed.lock().expect("lock") += 1;
        Ok(RebornListProjectsResponse {
            projects: vec![sample_reborn_project("project-alpha")],
        })
    }

    async fn create_project(
        &self,
        _caller: ProjectCaller,
        request: RebornCreateProjectRequest,
    ) -> Result<RebornProjectResponse, ProjectServiceError> {
        Ok(RebornProjectResponse {
            project: sample_reborn_project(&request.name),
        })
    }

    async fn get_project(
        &self,
        _caller: ProjectCaller,
        request: RebornGetProjectRequest,
    ) -> Result<RebornProjectResponse, ProjectServiceError> {
        self.read
            .lock()
            .expect("lock")
            .push(request.project_id.clone());
        Ok(RebornProjectResponse {
            project: sample_reborn_project(&request.project_id),
        })
    }

    async fn update_project(
        &self,
        _caller: ProjectCaller,
        request: RebornUpdateProjectRequest,
    ) -> Result<RebornProjectResponse, ProjectServiceError> {
        self.updated.lock().expect("lock").push(request.clone());
        Ok(RebornProjectResponse {
            project: sample_reborn_project(&request.project_id),
        })
    }

    async fn delete_project(
        &self,
        _caller: ProjectCaller,
        request: RebornDeleteProjectRequest,
    ) -> Result<(), ProjectServiceError> {
        self.deleted.lock().expect("lock").push(request.project_id);
        Ok(())
    }

    async fn list_members(
        &self,
        _caller: ProjectCaller,
        request: RebornListMembersRequest,
    ) -> Result<RebornListMembersResponse, ProjectServiceError> {
        self.listed_members
            .lock()
            .expect("lock")
            .push(request.project_id);
        Ok(RebornListMembersResponse {
            members: vec![sample_reborn_project_member("user-beta")],
        })
    }

    async fn add_member(
        &self,
        _caller: ProjectCaller,
        request: RebornAddMemberRequest,
    ) -> Result<RebornProjectMemberInfo, ProjectServiceError> {
        self.added_members
            .lock()
            .expect("lock")
            .push(request.clone());
        Ok(sample_reborn_project_member(&request.user_id))
    }

    async fn update_member_role(
        &self,
        _caller: ProjectCaller,
        request: RebornUpdateMemberRoleRequest,
    ) -> Result<RebornProjectMemberInfo, ProjectServiceError> {
        self.updated_members
            .lock()
            .expect("lock")
            .push(request.clone());
        Ok(sample_reborn_project_member(&request.user_id))
    }

    async fn remove_member(
        &self,
        _caller: ProjectCaller,
        request: RebornRemoveMemberRequest,
    ) -> Result<(), ProjectServiceError> {
        self.removed_members.lock().expect("lock").push(request);
        Ok(())
    }
}

fn sample_reborn_project(project_id: &str) -> RebornProjectInfo {
    RebornProjectInfo {
        project_id: project_id.to_string(),
        name: "Sample".to_string(),
        description: String::new(),
        icon: None,
        color: None,
        metadata: json!({}),
        state: RebornProjectState::Active,
        role: RebornProjectRole::Owner,
        created_at: "1970-01-01T00:00:00Z".parse().expect("created at"),
        updated_at: "1970-01-01T00:00:00Z".parse().expect("updated at"),
    }
}

fn sample_reborn_project_member(user_id: &str) -> RebornProjectMemberInfo {
    RebornProjectMemberInfo {
        user_id: user_id.to_string(),
        role: RebornProjectRole::Editor,
        status: RebornProjectMemberStatus::Active,
        granted_by: "user-alpha".to_string(),
        created_at: "1970-01-01T00:00:00Z".parse().expect("created at"),
        updated_at: "1970-01-01T00:00:00Z".parse().expect("updated at"),
    }
}

#[tokio::test]
async fn project_and_filesystem_reads_are_available_as_product_views() {
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    let project_service = Arc::new(RecordingProjectService::default());
    let services = RebornServices::new(thread_service, Arc::new(FakeTurnCoordinator::default()))
        .with_project_filesystem_reader(Arc::new(StaticProjectFilesystemReader))
        .with_filesystem_browser(Arc::new(StaticFilesystemBrowser))
        .with_project_service(project_service.clone());
    create_thread_for(&services, caller(), "thread-product-surface").await;

    // safety: these are ProductSurface facade query calls in a contract test;
    // no database transaction is involved.
    let project_fs_list = services
        .query(
            caller(),
            PROJECT_FS_LIST_VIEW
                .query(
                    RebornProjectFsListRequest {
                        thread_id: "thread-product-surface".to_string(),
                        path: "/workspace".to_string(),
                    },
                    None,
                )
                .expect("project fs list query"),
        )
        .await
        .expect("project fs list view");
    let project_fs_list: RebornProjectFsListResponse =
        serde_json::from_value(project_fs_list.payload).expect("project fs list payload");
    assert_eq!(project_fs_list.entries[0].path, "/workspace/report.md");

    let project_fs_stat = services
        .query(
            caller(),
            PROJECT_FS_STAT_VIEW
                .query(
                    RebornProjectFsStatRequest {
                        thread_id: "thread-product-surface".to_string(),
                        path: "/workspace/report.md".to_string(),
                    },
                    None,
                )
                .expect("project fs stat query"),
        )
        .await
        .expect("project fs stat view");
    let project_fs_stat: RebornProjectFsStatResponse =
        serde_json::from_value(project_fs_stat.payload).expect("project fs stat payload");
    assert_eq!(project_fs_stat.stat.mime_type, "text/markdown");

    let mounts = services
        .query(
            caller(),
            FS_MOUNTS_VIEW
                .query(RebornFsMountsRequest {}, None)
                .expect("fs mounts query"),
        )
        .await
        .expect("fs mounts view");
    let mounts: RebornFsMountsResponse =
        serde_json::from_value(mounts.payload).expect("fs mounts payload");
    assert_eq!(mounts.mounts.len(), 2);

    let fs_list = services
        .query(
            caller(),
            FS_LIST_VIEW
                .query(
                    RebornFsListRequest {
                        mount: FsMount::Memory,
                        path: "daily".to_string(),
                        project_id: None,
                    },
                    None,
                )
                .expect("fs list query"),
        )
        .await
        .expect("fs list view");
    let fs_list: RebornFsListResponse =
        serde_json::from_value(fs_list.payload).expect("fs list payload");
    assert_eq!(fs_list.entries[0].path, "daily/today.md");

    let fs_stat = services
        .query(
            caller(),
            FS_STAT_VIEW
                .query(
                    RebornFsStatRequest {
                        mount: FsMount::Memory,
                        path: "daily/today.md".to_string(),
                        project_id: None,
                    },
                    None,
                )
                .expect("fs stat query"),
        )
        .await
        .expect("fs stat view");
    let fs_stat: RebornFsStatResponse =
        serde_json::from_value(fs_stat.payload).expect("fs stat payload");
    assert_eq!(fs_stat.stat.path, "daily/today.md");

    let projects = services
        .query(
            caller(),
            PROJECTS_VIEW
                .query(RebornListProjectsRequest { limit: Some(10) }, None)
                .expect("projects query"),
        )
        .await
        .expect("projects view");
    let projects: RebornListProjectsResponse =
        serde_json::from_value(projects.payload).expect("projects payload");
    assert_eq!(projects.projects[0].project_id, "project-alpha");

    let project = services
        .query(
            caller(),
            PROJECT_VIEW
                .query(
                    RebornGetProjectRequest {
                        project_id: "project-alpha".to_string(),
                    },
                    None,
                )
                .expect("project query"),
        )
        .await
        .expect("project view");
    let project: RebornProjectResponse =
        serde_json::from_value(project.payload).expect("project payload");
    assert_eq!(project.project.project_id, "project-alpha");

    let members = services
        .query(
            caller(),
            PROJECT_MEMBERS_VIEW
                .query(
                    RebornListMembersRequest {
                        project_id: "project-alpha".to_string(),
                    },
                    None,
                )
                .expect("project members query"),
        )
        .await
        .expect("project members view");
    let members: RebornListMembersResponse =
        serde_json::from_value(members.payload).expect("project members payload");
    assert_eq!(members.members[0].user_id, "user-beta");
}

#[tokio::test]
async fn project_mutations_are_available_as_product_capabilities() {
    let project_service = Arc::new(RecordingProjectService::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_project_service(project_service.clone());

    let update_resolution = services
        .invoke(
            caller(),
            CapabilityId::new(PROJECT_UPDATE_CAPABILITY_ID).expect("capability id"),
            ProductCapabilityInput::json(
                json!({ "project_id": "project-alpha", "name": "Renamed" }),
            ),
            ActivityId::new(),
        )
        .await
        .expect("project update capability");
    assert!(matches!(
        update_resolution,
        Resolution::Done(outcome) if outcome.verdict.is_success()
    ));
    assert_eq!(
        project_service.updated.lock().expect("lock")[0].project_id,
        "project-alpha"
    );

    let delete_resolution = services
        .invoke(
            caller(),
            CapabilityId::new(PROJECT_DELETE_CAPABILITY_ID).expect("capability id"),
            ProductCapabilityInput::json(json!({ "project_id": "project-alpha" })),
            ActivityId::new(),
        )
        .await
        .expect("project delete capability");
    assert!(matches!(
        delete_resolution,
        Resolution::Done(outcome) if outcome.verdict.is_success()
    ));
    assert_eq!(
        project_service.deleted.lock().expect("lock").as_slice(),
        ["project-alpha"]
    );

    let add_member_resolution = services
        .invoke(
            caller(),
            CapabilityId::new(PROJECT_MEMBER_ADD_CAPABILITY_ID).expect("capability id"),
            ProductCapabilityInput::json(
                serde_json::to_value(RebornAddMemberRequest {
                    project_id: "project-alpha".to_string(),
                    user_id: "user-beta".to_string(),
                    role: RebornProjectRole::Viewer,
                })
                .expect("project member add input"),
            ),
            ActivityId::new(),
        )
        .await
        .expect("project member add capability");
    assert!(matches!(
        add_member_resolution,
        Resolution::Done(outcome) if outcome.verdict.is_success()
    ));
    assert_eq!(
        project_service.added_members.lock().expect("lock")[0].user_id,
        "user-beta"
    );

    let update_member_resolution = services
        .invoke(
            caller(),
            CapabilityId::new(PROJECT_MEMBER_UPDATE_CAPABILITY_ID).expect("capability id"),
            ProductCapabilityInput::json(
                serde_json::to_value(RebornUpdateMemberRoleRequest {
                    project_id: "project-alpha".to_string(),
                    user_id: "user-beta".to_string(),
                    role: RebornProjectRole::Editor,
                })
                .expect("project member update input"),
            ),
            ActivityId::new(),
        )
        .await
        .expect("project member update capability");
    assert!(matches!(
        update_member_resolution,
        Resolution::Done(outcome) if outcome.verdict.is_success()
    ));
    assert_eq!(
        project_service.updated_members.lock().expect("lock")[0].role,
        RebornProjectRole::Editor
    );

    let remove_member_resolution = services
        .invoke(
            caller(),
            CapabilityId::new(PROJECT_MEMBER_REMOVE_CAPABILITY_ID).expect("capability id"),
            ProductCapabilityInput::json(
                serde_json::to_value(RebornRemoveMemberRequest {
                    project_id: "project-alpha".to_string(),
                    user_id: "user-beta".to_string(),
                })
                .expect("project member remove input"),
            ),
            ActivityId::new(),
        )
        .await
        .expect("project member remove capability");
    assert!(matches!(
        remove_member_resolution,
        Resolution::Done(outcome) if outcome.verdict.is_success()
    ));
    assert_eq!(
        project_service.removed_members.lock().expect("lock")[0].user_id,
        "user-beta"
    );
}

#[tokio::test]
async fn session_timeline_and_thread_delete_are_available_as_product_surface() {
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    );
    create_thread_for(&services, caller(), "thread-session-product-surface").await;

    // safety: these are ProductSurface facade query calls in a contract test;
    // no database transaction is involved.
    let global_auto_approve = services
        .query(
            caller(),
            GLOBAL_AUTO_APPROVE_VIEW
                .query(RebornGlobalAutoApproveRequest {}, None)
                .expect("global auto approve query"),
        )
        .await
        .expect("global auto approve view");
    let global_auto_approve: RebornGlobalAutoApproveResponse =
        serde_json::from_value(global_auto_approve.payload).expect("global auto approve payload");
    assert!(
        !global_auto_approve.enabled,
        "the default facade reports global auto-approve disabled"
    );

    let timeline = services
        .query(
            caller(),
            TIMELINE_VIEW
                .query(
                    RebornTimelineRequest::new("thread-session-product-surface").set_limit(10),
                    None,
                )
                .expect("timeline query"),
        )
        .await
        .expect("timeline view");
    let timeline: RebornTimelineResponse =
        serde_json::from_value(timeline.payload).expect("timeline payload");
    assert_eq!(
        timeline.thread.thread_id.as_str(),
        "thread-session-product-surface"
    );
    assert!(timeline.messages.is_empty());

    let delete_resolution = services
        .invoke(
            caller(),
            CapabilityId::new(THREAD_DELETE_CAPABILITY_ID).expect("capability id"),
            ProductCapabilityInput::json(
                serde_json::to_value(RebornDeleteThreadRequest {
                    thread_id: "thread-session-product-surface".to_string(),
                })
                .expect("thread delete input"),
            ),
            ActivityId::new(),
        )
        .await
        .expect("thread delete capability");
    assert!(matches!(
        delete_resolution,
        Resolution::Done(outcome) if outcome.verdict.is_success()
    ));
}

#[tokio::test]
async fn browse_fs_authorizes_project_selector_and_fails_closed() {
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_filesystem_browser(Arc::new(EmptyFilesystemBrowser))
    .with_project_service(Arc::new(AuthorizingProjectService {
        allowed_project_id: "project-scoped".to_string(),
    }));

    let response = services
        .browse_fs_dir(
            caller_with_project(Some("project-alpha")),
            RebornFsListRequest {
                mount: FsMount::Memory,
                path: String::new(),
                project_id: Some(ProjectId::new("project-scoped").expect("project id")),
            },
        )
        .await;
    assert!(response.is_ok(), "authorized project selector must browse");

    let error = services
        .browse_fs_dir(
            caller_with_project(Some("project-alpha")),
            RebornFsListRequest {
                mount: FsMount::Memory,
                path: String::new(),
                project_id: Some(ProjectId::new("project-denied").expect("project id")),
            },
        )
        .await
        .expect_err("unauthorized project selector must fail closed");
    assert_eq!(error.code, RebornServicesErrorCode::NotFound);
}

#[tokio::test]
async fn create_thread_scopes_to_authorized_project() {
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    let services = RebornServices::new(
        thread_service.clone(),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_project_service(Arc::new(AuthorizingProjectService {
        allowed_project_id: "project-scoped".to_string(),
    }));

    // Caller's default scope is project-alpha; the request proposes a different,
    // authorized project, which must become the new thread's scope.
    services
        .create_thread(
            caller_with_project(Some("project-alpha")),
            serde_json::from_value::<WebUiCreateThreadRequest>(json!({
                "client_action_id": "create-scoped",
                "requested_thread_id": "thread-scoped",
                "project_id": "project-scoped"
            }))
            .expect("request"),
        )
        .await
        .expect("authorized project create succeeds");

    let record = thread_service
        .read_thread_by_id(ThreadId::new("thread-scoped").expect("thread id"))
        .await
        .expect("created thread exists");
    assert_eq!(
        record.scope.project_id.as_ref().map(|id| id.as_str()),
        Some("project-scoped"),
        "new thread must adopt the authorized project scope"
    );
}

#[tokio::test]
async fn create_thread_rejects_unauthorized_project() {
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_project_service(Arc::new(AuthorizingProjectService {
        allowed_project_id: "project-allowed".to_string(),
    }));

    let err = services
        .create_thread(
            caller_with_project(Some("project-alpha")),
            serde_json::from_value::<WebUiCreateThreadRequest>(json!({
                "client_action_id": "create-denied",
                "requested_thread_id": "thread-denied",
                "project_id": "project-forbidden"
            }))
            .expect("request"),
        )
        .await
        .expect_err("a project the caller cannot access must be rejected");

    // Fail closed on the deny→not-found contract: a project the caller can't
    // access collapses to NotFound/404 (no existence oracle), not some
    // unrelated internal error that `expect_err` alone would also accept.
    assert_eq!(err.code, RebornServicesErrorCode::NotFound);
    assert_eq!(err.status_code, 404);
}

#[tokio::test]
async fn create_thread_without_proposed_project_keeps_caller_scope() {
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    let services = RebornServices::new(
        thread_service.clone(),
        Arc::new(FakeTurnCoordinator::default()),
    );

    // No proposed project (and no project service wired): behavior is unchanged —
    // the thread keeps the caller's default project scope.
    services
        .create_thread(
            caller_with_project(Some("project-alpha")),
            serde_json::from_value::<WebUiCreateThreadRequest>(json!({
                "client_action_id": "create-default",
                "requested_thread_id": "thread-default"
            }))
            .expect("request"),
        )
        .await
        .expect("default create succeeds");

    let record = thread_service
        .read_thread_by_id(ThreadId::new("thread-default").expect("thread id"))
        .await
        .expect("created thread exists");
    assert_eq!(
        record.scope.project_id.as_ref().map(|id| id.as_str()),
        Some("project-alpha"),
        "without a proposed project the caller's scope is unchanged"
    );
}

#[test]
fn product_surface_descriptor_helpers_keep_view_and_capability_declarations_typed() {
    let query = THREADS_VIEW
        .query(
            WebUiListThreadsRequest::default()
                .set_limit(25)
                .set_needs_approval(true),
            Some("cursor-1".to_string()),
        )
        .expect("thread query");

    assert_eq!(query.view_id, THREADS_VIEW.id);
    assert_eq!(query.params["limit"], 25);
    assert_eq!(query.params["needs_approval"], true);
    assert_eq!(query.cursor.as_deref(), Some("cursor-1"));
    assert!(THREADS_VIEW.descriptor().paginated);

    let response = THREADS_VIEW
        .decode_page(RebornViewPage {
            payload: serde_json::to_value(RebornListThreadsResponse {
                threads: Vec::new(),
                next_cursor: Some("cursor-2".to_string()),
            })
            .expect("thread response payload"),
            next_cursor: Some("cursor-2".to_string()),
        })
        .expect("typed thread response");

    assert!(response.threads.is_empty());
    assert_eq!(response.next_cursor.as_deref(), Some("cursor-2"));

    let command = CREATE_THREAD_OPERATION
        .request(WebUiCreateThreadRequest {
            client_action_id: Some("action-1".to_string()),
            requested_thread_id: None,
            project_id: None,
        })
        .expect("create thread command");
    assert_eq!(
        command.operation_id,
        ProductOperationId::CreateThread.as_str()
    );
    assert_eq!(command.input["client_action_id"], "action-1");

    assert_eq!(
        OUTBOUND_PREFERENCES_SET_CAPABILITY
            .capability_id()
            .expect("capability id")
            .as_str(),
        OUTBOUND_PREFERENCES_SET_CAPABILITY_ID
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
        .get_timeline(caller(), RebornTimelineRequest::new("thread-alpha"))
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
    assert_eq!(
        coordinator.last_submission_origin_kind(),
        Some(TurnOriginKind::WebUi),
        "WebUI submit must produce WebUi origin"
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

    let rejected = services
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
        .expect("busy submit is rejected");

    assert!(matches!(
        rejected,
        RebornSubmitTurnResponse::RejectedBusy {
            active_run_id: Some(id),
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
        "rejected submissions must clear their activation input before returning"
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
        .get_timeline(caller(), RebornTimelineRequest::new("thread-alpha"))
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
            RebornTimelineRequest::new("thread-alpha"),
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
            RebornTimelineRequest::new(thread_id.as_str().to_string()),
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
            RebornTimelineRequest::new("thread-alpha".to_string()),
        )
        .await
        .expect("alpha timeline");
    assert_eq!(alpha_timeline.messages.len(), 1);

    let beta_timeline = services
        .get_timeline(
            caller(),
            RebornTimelineRequest::new("thread-beta".to_string()),
        )
        .await
        .expect("beta timeline");
    assert!(beta_timeline.messages.is_empty());
}

#[tokio::test]
async fn concurrent_duplicate_submit_creates_one_message_and_replays_outcome() {
    let threads: Arc<dyn SessionThreadService> = Arc::new(InMemorySessionThreadService::default());
    let coordinator = Arc::new(DefaultTurnCoordinator::new(Arc::new(
        in_memory_turn_state_store(),
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
        RebornSubmitTurnResponse::RejectedBusy { .. } => {
            panic!("duplicate submit must not defer while deduping")
        }
    };
    let second_run_id = match &second {
        RebornSubmitTurnResponse::Submitted { run_id, .. }
        | RebornSubmitTurnResponse::AlreadySubmitted { run_id, .. } => *run_id,
        RebornSubmitTurnResponse::RejectedBusy { .. } => {
            panic!("duplicate submit must not defer while deduping")
        }
    };
    assert_eq!(first_run_id, second_run_id);

    let timeline = services
        .get_timeline(
            caller(),
            RebornTimelineRequest::new("thread-alpha".to_string()),
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
            RebornTimelineRequest::new("thread-alpha".to_string()),
        )
        .await
        .expect("first resolve");
    let refreshed = services
        .get_timeline(
            caller(),
            RebornTimelineRequest::new("thread-alpha".to_string()),
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
            RebornTimelineRequest::new("thread-alpha".to_string()),
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
            RebornTimelineRequest::new("thread-alpha".to_string()),
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
            RebornTimelineRequest::new("thread-alpha".to_string()),
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
            RebornTimelineRequest::new("thread-alpha".to_string()),
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
            RebornTimelineRequest::new("thread-alpha".to_string()),
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
            RebornTimelineRequest::new("thread-alpha".to_string()),
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
async fn retry_run_uses_turn_facade_and_stable_response() {
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        coordinator.clone(),
    );
    create_thread_for(&services, caller(), "thread-alpha").await;

    let response = services
        .retry_run(
            caller(),
            serde_json::from_value::<WebUiRetryRunRequest>(json!({
                "client_action_id": "retry-1",
                "thread_id": "thread-alpha",
                "run_id": run_id_string()
            }))
            .expect("request"),
        )
        .await
        .expect("retry succeeds");

    assert_eq!(response.status, TurnStatus::Queued);
    assert_eq!(response.event_cursor, EventCursor(19));
    assert_eq!(coordinator.retry_count(), 1);
    let retry = coordinator.last_retry().expect("retry request");
    assert_eq!(
        retry.run_id,
        TurnRunId::parse(&run_id_string()).expect("run id")
    );
    assert_eq!(retry.actor, caller().actor());
    assert_eq!(
        retry.scope,
        caller().turn_scope(ThreadId::new("thread-alpha").expect("thread"))
    );
    assert!(
        retry
            .source_binding_ref
            .as_str()
            .contains("webui-retry-src")
    );
    assert!(
        retry
            .reply_target_binding_ref
            .as_str()
            .contains("webui-retry-reply")
    );
    assert_eq!(retry.idempotency_key.as_str(), "retry-1");
}

#[tokio::test]
async fn retry_run_rejects_invalid_run_id_without_turn_facade() {
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        coordinator.clone(),
    );

    let err = services
        .retry_run(
            caller(),
            serde_json::from_value::<WebUiRetryRunRequest>(json!({
                "client_action_id": "retry-invalid-run",
                "thread_id": "thread-alpha",
                "run_id": "not-a-run-uuid"
            }))
            .expect("request"),
        )
        .await
        .expect_err("invalid run id should fail validation");

    assert_eq!(err.code, RebornServicesErrorCode::InvalidRequest);
    assert_eq!(err.kind, RebornServicesErrorKind::Validation);
    assert_eq!(err.status_code, 400);
    assert_eq!(err.field.as_deref(), Some("run_id"));
    assert_eq!(
        err.validation_code,
        Some(WebUiInboundValidationCode::InvalidId)
    );
    assert_eq!(
        coordinator.retry_attempt_count(),
        0,
        "validation must fail before TurnCoordinator::retry_turn"
    );
    assert_eq!(coordinator.retry_count(), 0);
}

#[tokio::test]
async fn retry_run_maps_not_retryable_to_non_retryable_conflict() {
    let run_id = TurnRunId::parse(&run_id_string()).expect("run id");
    let coordinator = Arc::new(FakeTurnCoordinator::with_retry_error(
        TurnError::RunNotRetryable { run_id },
    ));
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        coordinator.clone(),
    );
    create_thread_for(&services, caller(), "thread-alpha").await;

    let err = services
        .retry_run(
            caller(),
            serde_json::from_value::<WebUiRetryRunRequest>(json!({
                "client_action_id": "retry-not-retryable",
                "thread_id": "thread-alpha",
                "run_id": run_id_string()
            }))
            .expect("request"),
        )
        .await
        .expect_err("not retryable maps to conflict");

    assert_eq!(err.code, RebornServicesErrorCode::Conflict);
    assert_eq!(err.kind, RebornServicesErrorKind::Conflict);
    assert_eq!(err.status_code, 409);
    assert!(!err.retryable);
    assert_eq!(coordinator.retry_attempt_count(), 1);
    assert_eq!(coordinator.retry_count(), 0);
}

#[tokio::test]
async fn retry_run_rejects_cross_user_access() {
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        coordinator.clone(),
    );
    let alice = caller();
    create_thread_for(&services, alice.clone(), "thread-alice").await;

    let bob = WebUiAuthenticatedCaller::new(
        TenantId::new("tenant-alpha").expect("tenant"),
        UserId::new("user-bob").expect("user"),
        alice.agent_id.clone(),
        alice.project_id.clone(),
    );

    let err = services
        .retry_run(
            bob,
            serde_json::from_value::<WebUiRetryRunRequest>(json!({
                "client_action_id": "retry-cross",
                "thread_id": "thread-alice",
                "run_id": run_id_string()
            }))
            .expect("request"),
        )
        .await
        .expect_err("cross-user retry must be rejected");

    assert_eq!(err.code, RebornServicesErrorCode::NotFound);
    assert_eq!(err.status_code, 404);
    assert_eq!(
        coordinator.retry_count(),
        0,
        "turn coordinator must NOT be called for cross-user retry"
    );
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
                "resolution": "declined"
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
                "resolution": "declined"
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
                "resolution": "declined"
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
                "resolution": "declined"
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
                "resolution": "declined"
            }))
            .expect("request"),
        )
        .await
        .expect("approval gate denial succeeds");

    assert!(matches!(response, RebornResolveGateResponse::Resumed(_)));
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
                "resolution": "declined"
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

/// A minimal auth-interaction stub that returns `Resumed` for every
/// Deny decision, mirroring the production path where the model is resumed
/// so it can surface the denial to the user.
struct DeniedResumedAuthInteractionService;

#[async_trait]
impl AuthInteractionService for DeniedResumedAuthInteractionService {
    async fn list_pending(
        &self,
        _request: ListPendingAuthInteractionsRequest,
    ) -> Result<ListPendingAuthInteractionsResponse, ProductWorkflowError> {
        Ok(ListPendingAuthInteractionsResponse {
            auth_interactions: vec![],
        })
    }

    async fn resolve(
        &self,
        request: ResolveAuthInteractionRequest,
    ) -> Result<ResolveAuthInteractionResponse, ProductWorkflowError> {
        let run_id = request.run_id_hint.expect("webui passes run_id");
        Ok(ResolveAuthInteractionResponse::Resumed(
            ResumeTurnResponse {
                run_id,
                status: TurnStatus::Queued,
                event_cursor: EventCursor(37),
            },
        ))
    }
}

#[tokio::test]
async fn hook_auth_gate_denial_maps_to_reborn_resumed() {
    // Verifies that a Deny decision (which produces `Resumed` from
    // `resume_denied_auth`) maps to `RebornResolveGateResponse::Resumed`
    // through the facade.
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let auth_interactions = Arc::new(DeniedResumedAuthInteractionService);
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        coordinator.clone(),
    )
    .with_auth_interactions(auth_interactions);
    create_thread_for(&services, caller(), "thread-alpha").await;

    let response = services
        .resolve_gate(
            caller(),
            serde_json::from_value::<WebUiResolveGateRequest>(json!({
                "client_action_id": "gate-auth-denial-resumed",
                "thread_id": "thread-alpha",
                "run_id": run_id_string(),
                "gate_ref": "gate:hook-auth-denial-resumed",
                "resolution": "declined"
            }))
            .expect("request"),
        )
        .await
        .expect(
            "Resumed from auth-interaction service must map to RebornResolveGateResponse::Resumed",
        );

    assert!(
        matches!(response, RebornResolveGateResponse::Resumed(_)),
        "expected Resumed, got: {response:?}"
    );
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
                "resolution": "declined"
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
                "resolution": "declined"
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
                "resolution": "declined"
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
                "resolution": "declined"
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

#[tokio::test]
async fn generic_gate_resolution_with_persistent_flag_is_rejected() {
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
        .expect_err("generic persistent gate resolution must be rejected");

    assert_eq!(err.code, RebornServicesErrorCode::Unavailable);
    assert_eq!(err.kind, RebornServicesErrorKind::BlockedApproval);
    assert_eq!(err.status_code, 503);
    assert_eq!(
        coordinator.resumption_count(),
        0,
        "resume_turn must NOT be called for unsupported generic persistent gate"
    );
}

#[tokio::test]
async fn approval_gate_resolution_with_persistent_flag_uses_approval_interaction_service() {
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
        .expect("persistent approval resolution succeeds");

    assert!(matches!(
        response,
        RebornResolveGateResponse::Resumed(response) if response.status == TurnStatus::Queued
    ));
    assert_eq!(approval_interactions.resolution_count(), 1);
    assert_eq!(
        approval_interactions
            .last_resolution()
            .expect("resolution")
            .decision,
        ApprovalInteractionDecision::AlwaysAllow
    );
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

    let response = query_extension_setup(&services, caller(), "github")
        .await
        .expect("setup extension response");

    assert_eq!(
        response.package_ref,
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, "github")
            .expect("valid package ref")
    );
    assert_eq!(response.phase, InstallationState::Unsupported);
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
async fn extension_setup_is_available_as_product_view() {
    let lifecycle_facade = Arc::new(RecordingLifecycleFacade::new());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_lifecycle_product_facade(lifecycle_facade.clone());

    let response = query_extension_setup(&services, caller(), "github")
        .await
        .expect("extension setup view response");

    assert_eq!(
        response.package_ref,
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, "github")
            .expect("valid package ref")
    );
    assert_eq!(response.phase, InstallationState::Unsupported);
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
            phase: InstallationState::Installed,
            install_scope: None,
        },
    }));

    let response = query_extensions(&services, caller())
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

    let listed = query_automations(
        &services,
        caller(),
        WebUiListAutomationsRequest::default().set_limit(10),
    )
    .await
    .expect("list automations");
    assert_eq!(listed.automations.len(), 1);
    assert_eq!(listed.automations[0].automation_id, "trigger-listed");
    assert_eq!(
        listed.automations[0].source,
        RebornAutomationSource::Schedule {
            cron: "0 9 * * *".to_string(),
            timezone: "UTC".to_string(),
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
        listed.automations[0].recent_runs[0]
            .thread_id
            .as_ref()
            .map(|t| t.as_str()),
        Some("thread-listed")
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
async fn list_extensions_projects_channel_surface_with_directions_and_connection() {
    // Channel discovery is extension-surface data: an installed extension
    // whose summary declares an inbound+outbound channel surface projects a
    // typed `channel` surface with its connect affordance — there is no
    // separate connectable-channel registry or route.
    let mut summary = extension_summary("slack", Vec::new(), None);
    summary.surface_kinds = vec![CapabilitySurfaceKind::Channel];
    summary.channel_directions = Some(LifecycleChannelDirections {
        inbound: true,
        outbound: true,
    });
    summary.channel_connection = Some(ChannelConnectionRequirement {
        channel: "slack".to_string(),
        display_name: "Slack".to_string(),
        strategy: RebornChannelConnectStrategy::OAuth,
        instructions: "Connect Slack with OAuth.".to_string(),
        input_placeholder: String::new(),
        submit_label: "Connect Slack".to_string(),
        error_message: "Slack OAuth connection failed.".to_string(),
    });
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_lifecycle_product_facade(Arc::new(ListingLifecycleFacade {
        extension: LifecycleInstalledExtensionSummary {
            summary,
            phase: InstallationState::Active,
            install_scope: None,
        },
    }));

    let response = query_extensions(&services, caller())
        .await
        .expect("extensions response");

    let info = response
        .extensions
        .iter()
        .find(|extension| extension.package_ref.id.as_str() == "slack")
        .expect("channel extension listed");
    let channel = info
        .surfaces
        .iter()
        .find_map(|surface| match surface {
            RebornExtensionSurface::Channel {
                inbound,
                outbound,
                connection,
                ..
            } => Some((inbound, outbound, connection.clone())),
            _ => None,
        })
        .expect("channel surface projected");
    assert!(channel.0, "inbound direction must project");
    assert!(channel.1, "outbound direction must project");
    let connection = channel.2.expect("connect affordance carried");
    assert_eq!(connection.strategy, RebornChannelConnectStrategy::OAuth);
    assert_eq!(connection.submit_label, "Connect Slack");
    // S5 wire gap: the connect affordance now carries the manifest display name
    // so the frontend never derives a label from the channel id.
    assert_eq!(connection.display_name, "Slack");
    // §6.1 installation-state enum replaces the activation_status string.
    assert_eq!(info.installation_state, InstallationState::Active);
}

/// A caller-scoped channel-connection facade that reports a fixed set of
/// connected channels (mirrors the production port shape the composition crate
/// wires; the default `StaticChannelConnectionFacade` reports none).
struct ConnectedChannelConnectionFacade {
    connections: std::collections::HashMap<String, bool>,
}

#[async_trait]
impl ChannelConnectionFacade for ConnectedChannelConnectionFacade {
    async fn caller_channel_connections(
        &self,
        _caller: WebUiAuthenticatedCaller,
    ) -> Result<std::collections::HashMap<String, bool>, RebornServicesError> {
        Ok(self.connections.clone())
    }
}

/// Golden wire fixture (UI-1 / UI-2 / AUTH-9): an arbitrary channel on a
/// multi-surface extension freezes the exact shape the frontend renders — the
/// §6.1 installation-state enum, the per-vendor accounts list carrying the §6.3
/// auth-account state enum (`account_id` / `label` / `state` / `is_default`),
/// each surface's `resolved_account_id` + binding source, and the connection
/// requirement's `display_name`. Vendor-neutral on purpose: `acme` proves no
/// concrete product is needed. The retired stopgap fields (the
/// `activation_status` string and the `connected` bool) are gone from the wire.
#[tokio::test]
async fn list_extensions_golden_wire_multi_surface_extension_freezes_accounts_list() {
    let mut summary = extension_summary(
        "acme",
        vec![LifecycleExtensionCredentialRequirement {
            name: "acme_oauth_token".to_string(),
            provider: "acme".to_string(),
            required: true,
            setup: LifecycleExtensionCredentialSetup::ManualToken,
        }],
        None,
    );
    summary.surface_kinds = vec![
        CapabilitySurfaceKind::Tool,
        CapabilitySurfaceKind::Channel,
        CapabilitySurfaceKind::Auth,
    ];
    summary.channel_directions = Some(LifecycleChannelDirections {
        inbound: true,
        outbound: true,
    });
    summary.channel_connection = Some(ChannelConnectionRequirement {
        channel: "acme".to_string(),
        display_name: "Acme Messenger".to_string(),
        strategy: RebornChannelConnectStrategy::OAuth,
        instructions: "Connect Acme Messenger with OAuth.".to_string(),
        input_placeholder: String::new(),
        submit_label: "Connect Acme Messenger".to_string(),
        error_message: "Acme Messenger OAuth connection failed.".to_string(),
    });

    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_lifecycle_product_facade(Arc::new(ListingLifecycleFacade {
        extension: LifecycleInstalledExtensionSummary {
            summary,
            phase: InstallationState::Active,
            install_scope: None,
        },
    }))
    .with_channel_connection_facade(Arc::new(ConnectedChannelConnectionFacade {
        connections: std::collections::HashMap::from([("acme".to_string(), true)]),
    }));

    let response = query_extensions(&services, caller())
        .await
        .expect("extensions response");
    let info = response
        .extensions
        .iter()
        .find(|extension| extension.package_ref.id.as_str() == "acme")
        .expect("multi-surface extension listed");

    // §6.1 installation-state enum on the wire (replaces the activation_status string).
    assert_eq!(info.installation_state, InstallationState::Active);

    // §6.4 / ADR 0001 accounts list — the frozen shape, named field for field.
    // A live grant backfills to `connected` (MIG-1); one account per vendor,
    // is_default (list length ≤ 1, shape only).
    let accounts = serde_json::to_value(&info.auth_accounts).expect("auth_accounts serialize");
    assert_eq!(
        accounts,
        json!([{
            "vendor": "acme",
            "accounts": [{
                "account_id": "acme",
                "label": "acme",
                "state": "connected",
                "is_default": true
            }]
        }]),
        "the per-vendor accounts list freezes the account_id/label/state/is_default shape",
    );

    // Surface keys: tool / channel-with-resolved-account / auth, in declared order.
    let surfaces = serde_json::to_value(&info.surfaces).expect("surfaces serialize");
    assert_eq!(
        surfaces,
        json!([
            { "kind": "tool" },
            {
                "kind": "channel",
                "inbound": true,
                "outbound": true,
                "resolved_account_id": "acme",
                "binding_source": "default",
                "connection": {
                    "channel": "acme",
                    "display_name": "Acme Messenger",
                    "strategy": "oauth",
                    "instructions": "Connect Acme Messenger with OAuth.",
                    "input_placeholder": "",
                    "submit_label": "Connect Acme Messenger",
                    "error_message": "Acme Messenger OAuth connection failed."
                }
            },
            { "kind": "auth" }
        ]),
        "surface keys + the channel surface's resolved account + binding source + display_name are frozen",
    );

    // The retired stopgap fields are gone from the wire.
    let info_json = serde_json::to_value(info).expect("info serialize");
    assert!(
        info_json.get("activation_status").is_none(),
        "the activation_status string stopgap must be gone",
    );
    let channel_json = &surfaces.as_array().expect("surfaces array")[1];
    assert!(
        channel_json.get("connected").is_none(),
        "the connected bool stopgap must be gone from the channel surface",
    );
}

/// A lifecycle facade that lists one installed extension in a caller-chosen
/// installation state and reports a redacted per-extension activation error —
/// drives the terminal `Failed` installation-state (§6.1) and `activation_error`
/// projection through the real descriptor-backed `EXTENSIONS_VIEW` seam.
struct FailedStateLifecycleFacade {
    extension: LifecycleInstalledExtensionSummary,
    activation_errors: std::collections::HashMap<String, String>,
}

#[async_trait]
impl LifecycleProductFacade for FailedStateLifecycleFacade {
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

    async fn installed_activation_errors(
        &self,
        _context: LifecycleProductContext,
    ) -> Result<std::collections::HashMap<String, String>, ProductWorkflowError> {
        Ok(self.activation_errors.clone())
    }
}

/// A channel-connection facade that also reports the caller's durable
/// auth-account status per vendor, so the extensions wire projects real §6.3
/// states (expired / refresh-failed) instead of the connected/disconnected
/// collapse the connection bool alone permits.
struct AccountStatusConnectionFacade {
    connections: std::collections::HashMap<String, bool>,
    account_states: std::collections::HashMap<String, ChannelAuthAccountState>,
}

#[async_trait]
impl ChannelConnectionFacade for AccountStatusConnectionFacade {
    async fn caller_channel_connections(
        &self,
        _caller: WebUiAuthenticatedCaller,
    ) -> Result<std::collections::HashMap<String, bool>, RebornServicesError> {
        Ok(self.connections.clone())
    }

    async fn caller_channel_account_states(
        &self,
        _caller: WebUiAuthenticatedCaller,
    ) -> Result<std::collections::HashMap<String, ChannelAuthAccountState>, RebornServicesError>
    {
        Ok(self.account_states.clone())
    }
}

/// Un-collapse regression (G1/G2/G3) driven through the real
/// descriptor-backed `EXTENSIONS_VIEW` seam. Before the projection fix
/// this shape was unrepresentable/collapsed: a `Failed` extension read as
/// `Installed`, a live-grant account read as `connected` with no error, and
/// `activation_error` was hard-coded `None`. The facade must now project all
/// three distinctly.
#[tokio::test]
async fn list_extensions_surfaces_failed_state_expired_account_and_activation_error() {
    let mut summary = extension_summary(
        "acme",
        vec![LifecycleExtensionCredentialRequirement {
            name: "acme_oauth_token".to_string(),
            provider: "acme".to_string(),
            required: true,
            setup: LifecycleExtensionCredentialSetup::ManualToken,
        }],
        None,
    );
    summary.surface_kinds = vec![CapabilitySurfaceKind::Channel];
    summary.channel_directions = Some(LifecycleChannelDirections {
        inbound: true,
        outbound: true,
    });
    summary.channel_connection = Some(ChannelConnectionRequirement {
        channel: "acme".to_string(),
        display_name: "Acme Messenger".to_string(),
        strategy: RebornChannelConnectStrategy::OAuth,
        instructions: "Connect Acme Messenger with OAuth.".to_string(),
        input_placeholder: String::new(),
        submit_label: "Connect Acme Messenger".to_string(),
        error_message: "Acme Messenger OAuth connection failed.".to_string(),
    });

    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_lifecycle_product_facade(Arc::new(FailedStateLifecycleFacade {
        extension: LifecycleInstalledExtensionSummary {
            summary,
            // An enabled extension whose activation failed for a non-auth
            // reason — the terminal `Failed` state, distinct from a pristine
            // `Installed`, carrying its reason.
            phase: InstallationState::Failed,
            install_scope: None,
        },
        activation_errors: std::collections::HashMap::from([(
            "acme".to_string(),
            "activation failed: runtime credential rejected".to_string(),
        )]),
    }))
    .with_channel_connection_facade(Arc::new(AccountStatusConnectionFacade {
        // The caller still holds a binding (connected), yet the durable
        // credential-account status says the grant's refresh failed. The real
        // status must win over the connected backfill.
        connections: std::collections::HashMap::from([("acme".to_string(), true)]),
        account_states: std::collections::HashMap::from([(
            "acme".to_string(),
            ChannelAuthAccountState {
                account_status: Some(CredentialAccountStatus::RefreshFailed),
                active_flow_status: None,
            },
        )]),
    }));

    let response = query_extensions(&services, caller())
        .await
        .expect("extensions response");
    let info = response
        .extensions
        .iter()
        .find(|extension| extension.package_ref.id.as_str() == "acme")
        .expect("extension listed");

    // (a) The terminal §6.1 `Failed` state projects distinctly — NOT collapsed
    // to Installed/Active.
    assert_eq!(
        info.installation_state,
        InstallationState::Failed,
        "a Failed extension must project its own installation_state",
    );
    assert_ne!(info.installation_state, InstallationState::Installed);

    // (c) The redacted activation error reaches the DTO with its reason (the
    // frontend card renders this slot; it was fed `None` before).
    assert_eq!(
        info.activation_error.as_deref(),
        Some("activation failed: runtime credential rejected"),
        "the installation record's last_error must reach the projected DTO",
    );

    // (b) The auth account projects its real §6.3 state + typed last error,
    // not the connected/disconnected collapse.
    let account = info
        .auth_accounts
        .first()
        .and_then(|vendor| vendor.accounts.first())
        .expect("vendor account projected");
    assert_eq!(
        account.state,
        AuthAccountState::Expired,
        "a refresh-failed credential account must project `expired`, not `connected`",
    );
    assert_eq!(
        account.last_error,
        Some(AuthAccountLastError::RefreshFailed),
        "the account's typed last_error must reach the projected DTO",
    );
}

#[test]
fn channel_connect_action_serializes_neutral_input_placeholder() {
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
}

#[tokio::test]
async fn get_outbound_preferences_unwired_returns_empty_projection() {
    // arch-exempt: large_file, outbound pref tests belong at API seam, plan docs/plans/2026-06-05-trigger-delivery-default-outbound-e2e-plan.md.
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    );

    let page = services
        .query(
            caller(),
            RebornViewQuery {
                view_id: OUTBOUND_PREFERENCES_VIEW.id.to_string(),
                params: json!({}),
                cursor: None,
            },
        )
        .await
        .expect("default outbound preferences");
    let response: RebornOutboundPreferencesResponse =
        serde_json::from_value(page.payload).expect("outbound preferences payload");

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
        .invoke(
            caller(),
            CapabilityId::new(OUTBOUND_PREFERENCES_SET_CAPABILITY_ID).expect("capability id"),
            ProductCapabilityInput::json(json!({ "final_reply_target_id": "slack-dm-alpha" })),
            ActivityId::new(),
        )
        .await
        .expect_err("unwired preference mutation");
    assert_eq!(set_error.code, RebornServicesErrorCode::Unavailable);
    assert_eq!(set_error.status_code, 503);
    assert!(!set_error.retryable);

    let list_error = services
        .query(
            caller(),
            RebornViewQuery {
                view_id: OUTBOUND_DELIVERY_TARGETS_VIEW.id.to_string(),
                params: json!({}),
                cursor: None,
            },
        )
        .await
        .expect_err("unwired target listing");
    assert_eq!(list_error.code, RebornServicesErrorCode::Unavailable);
    assert_eq!(list_error.status_code, 503);
    assert!(!list_error.retryable);
}

#[tokio::test]
async fn outbound_preferences_facade_forwards_caller_and_request() {
    let outbound_facade = Arc::new(RecordingOutboundPreferencesFacade::default());
    let invoker = RecordingOutboundPreferencesInvoker::default();
    let services = RebornServices::new_with_product_capability_invoker(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
        invoker.clone(),
    )
    .with_outbound_preferences_facade(outbound_facade.clone());

    let get_page = services
        .query(
            caller(),
            RebornViewQuery {
                view_id: OUTBOUND_PREFERENCES_VIEW.id.to_string(),
                params: json!({}),
                cursor: None,
            },
        )
        .await
        .expect("get outbound preferences");
    let get_response: RebornOutboundPreferencesResponse =
        serde_json::from_value(get_page.payload).expect("outbound preferences payload");
    assert_eq!(
        get_response
            .final_reply_target
            .as_ref()
            .map(|target| target.target_id.as_str()),
        Some("slack-dm-alpha")
    );

    services
        .invoke(
            caller_for_user_with_project("user-bravo", None),
            CapabilityId::new(OUTBOUND_PREFERENCES_SET_CAPABILITY_ID).expect("capability id"),
            ProductCapabilityInput::json(json!({ "final_reply_target_id": "slack-dm-beta" })),
            ActivityId::new(),
        )
        .await
        .expect("set outbound preferences");
    let set_page = services
        .query(
            caller_for_user_with_project("user-bravo", None),
            RebornViewQuery {
                view_id: OUTBOUND_PREFERENCES_VIEW.id.to_string(),
                params: json!({}),
                cursor: None,
            },
        )
        .await
        .expect("read outbound preferences after mutation");
    let set_response: RebornOutboundPreferencesResponse =
        serde_json::from_value(set_page.payload).expect("outbound preferences payload");
    assert_eq!(
        set_response
            .final_reply_target
            .as_ref()
            .map(|target| target.target_id.as_str()),
        Some("slack-dm-alpha")
    );

    let targets_page = services
        .query(
            caller_for_user("user-charlie"),
            RebornViewQuery {
                view_id: OUTBOUND_DELIVERY_TARGETS_VIEW.id.to_string(),
                params: json!({}),
                cursor: None,
            },
        )
        .await
        .expect("list outbound targets");
    let targets: RebornOutboundDeliveryTargetListResponse =
        serde_json::from_value(targets_page.payload).expect("outbound targets payload");
    assert_eq!(targets.targets.len(), 1);
    assert_eq!(
        targets.targets[0].target.target_id.as_str(),
        "slack-dm-alpha"
    );
    assert!(targets.targets[0].capabilities.final_replies);

    let get_calls = outbound_facade.get_calls();
    assert_eq!(get_calls.len(), 2);
    assert_eq!(get_calls[0].tenant_id.as_str(), "tenant-alpha");
    assert_eq!(get_calls[0].user_id.as_str(), "user-alpha");
    assert_eq!(get_calls[1].user_id.as_str(), "user-bravo");

    assert_eq!(outbound_facade.set_calls(), 0);
    let invoke_calls = invoker.calls();
    assert_eq!(invoke_calls.len(), 1);
    assert_eq!(invoke_calls[0].0.user_id.as_str(), "user-bravo");
    assert!(invoke_calls[0].0.agent_id.is_some());
    assert!(invoke_calls[0].0.project_id.is_none());
    assert_eq!(
        invoke_calls[0].1.as_str(),
        OUTBOUND_PREFERENCES_SET_CAPABILITY_ID
    );
    assert_eq!(
        invoke_calls[0].2,
        json!({ "final_reply_target_id": "slack-dm-beta" })
    );

    let list_calls = outbound_facade.list_calls();
    assert_eq!(list_calls.len(), 1);
    assert_eq!(list_calls[0].user_id.as_str(), "user-charlie");
}

#[tokio::test]
async fn outbound_preferences_reads_are_available_as_product_views() {
    let outbound_facade = Arc::new(RecordingOutboundPreferencesFacade::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_outbound_preferences_facade(outbound_facade.clone());
    let preferences_caller = caller_for_user("user-outbound-preferences");
    let targets_caller = caller_for_user("user-outbound-targets");

    let preferences_page = services
        .query(
            preferences_caller.clone(),
            RebornViewQuery {
                view_id: OUTBOUND_PREFERENCES_VIEW.id.to_string(),
                params: json!({}),
                cursor: None,
            },
        )
        .await
        .expect("outbound preferences view");
    let preferences: RebornOutboundPreferencesResponse =
        serde_json::from_value(preferences_page.payload).expect("outbound preferences payload");
    assert_eq!(
        preferences
            .final_reply_target
            .as_ref()
            .map(|target| target.target_id.as_str()),
        Some("slack-dm-alpha")
    );

    let targets_page = services
        .query(
            targets_caller.clone(),
            RebornViewQuery {
                view_id: OUTBOUND_DELIVERY_TARGETS_VIEW.id.to_string(),
                params: json!({}),
                cursor: None,
            },
        )
        .await
        .expect("outbound targets view");
    let targets: RebornOutboundDeliveryTargetListResponse =
        serde_json::from_value(targets_page.payload).expect("outbound targets payload");
    assert_eq!(targets.targets.len(), 1);
    assert_eq!(outbound_facade.get_calls(), vec![preferences_caller]);
    assert_eq!(outbound_facade.list_calls(), vec![targets_caller]);
}

#[tokio::test]
async fn trace_reads_are_available_as_product_views() {
    let user_id = format!(
        "trace-query-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    );
    let caller = WebUiAuthenticatedCaller::new(
        TenantId::new("tenant-alpha").expect("tenant"),
        UserId::new(user_id.as_str()).expect("user"),
        None,
        None,
    );
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    );

    let credits_page = services
        .query(
            caller.clone(),
            RebornViewQuery {
                view_id: TRACE_CREDITS_VIEW.id.to_string(),
                params: json!({}),
                cursor: None,
            },
        )
        .await
        .expect("trace credits view");
    let credits: RebornTraceCreditsResponse =
        serde_json::from_value(credits_page.payload).expect("trace credits payload");
    assert!(!credits.enrolled);
    assert_eq!(credits.submissions_total, 0);
    assert!(credits.note.contains("authoritative ledger is server-side"));

    let traces_page = services
        .query(
            caller,
            RebornViewQuery {
                view_id: TRACE_ACCOUNT_TRACES_VIEW.id.to_string(),
                params: json!({}),
                cursor: None,
            },
        )
        .await
        .expect("trace account traces view");
    let traces: RebornAccountTracesResponse =
        serde_json::from_value(traces_page.payload).expect("trace account traces payload");
    assert!(!traces.enrolled);
    assert!(traces.traces.is_empty());
}

#[tokio::test]
async fn set_outbound_preferences_can_clear_final_target() {
    let outbound_facade = Arc::new(RecordingOutboundPreferencesFacade::default());
    let invoker = RecordingOutboundPreferencesInvoker::default();
    let services = RebornServices::new_with_product_capability_invoker(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
        invoker.clone(),
    )
    .with_outbound_preferences_facade(outbound_facade.clone());

    services
        .invoke(
            caller(),
            CapabilityId::new(OUTBOUND_PREFERENCES_SET_CAPABILITY_ID).expect("capability id"),
            ProductCapabilityInput::json(json!({})),
            ActivityId::new(),
        )
        .await
        .expect("clear outbound preferences");

    assert_eq!(outbound_facade.set_calls(), 0);
    let invoke_calls = invoker.calls();
    assert_eq!(invoke_calls.len(), 1);
    assert_eq!(
        invoke_calls[0].1.as_str(),
        OUTBOUND_PREFERENCES_SET_CAPABILITY_ID
    );
    assert_eq!(invoke_calls[0].2, json!({}));
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
    let invoker = RecordingOutboundPreferencesInvoker::default();
    let services = RebornServices::new_with_product_capability_invoker(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
        invoker.clone(),
    )
    .with_outbound_preferences_facade(outbound_facade.clone());

    let max_length_target_id = "a".repeat(512);
    services
        .invoke(
            caller(),
            CapabilityId::new(OUTBOUND_PREFERENCES_SET_CAPABILITY_ID).expect("capability id"),
            ProductCapabilityInput::json(json!({ "final_reply_target_id": max_length_target_id })),
            ActivityId::new(),
        )
        .await
        .expect("max-length target id");

    assert_eq!(outbound_facade.set_calls(), 0);
    let invoke_calls = invoker.calls();
    assert_eq!(invoke_calls.len(), 1);
    assert_eq!(
        invoke_calls[0]
            .2
            .get("final_reply_target_id")
            .and_then(serde_json::Value::as_str),
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

    let err = query_automations(
        &services,
        caller_without_agent(),
        WebUiListAutomationsRequest::default().set_limit(10),
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

    query_automations(
        &services,
        caller(),
        WebUiListAutomationsRequest::default().set_limit(u32::MAX),
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

    query_automations(
        &services,
        caller(),
        WebUiListAutomationsRequest::default().set_limit(0),
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

    query_automations(&services, caller(), WebUiListAutomationsRequest::default())
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

    query_automations(
        &services,
        caller(),
        WebUiListAutomationsRequest::default().set_run_limit(u32::MAX),
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

    query_automations(
        &services,
        caller(),
        WebUiListAutomationsRequest::default().set_run_limit(0),
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

#[tokio::test]
async fn list_automations_forwards_include_completed_true_to_product_facade() {
    let automation_facade = Arc::new(RecordingAutomationFacade::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_automation_product_facade(automation_facade.clone());

    query_automations(
        &services,
        caller(),
        WebUiListAutomationsRequest::default().set_include_completed(true),
    )
    .await
    .expect("list automations");

    let list_calls = automation_facade.list_calls();
    assert_eq!(list_calls.len(), 1);
    assert!(
        list_calls[0].include_completed,
        "include_completed=true must be forwarded to the product facade unchanged"
    );
}

#[tokio::test]
async fn list_automations_forwards_include_completed_false_to_product_facade() {
    let automation_facade = Arc::new(RecordingAutomationFacade::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_automation_product_facade(automation_facade.clone());

    query_automations(&services, caller(), WebUiListAutomationsRequest::default())
        .await
        .expect("list automations");

    let list_calls = automation_facade.list_calls();
    assert_eq!(list_calls.len(), 1);
    assert!(
        !list_calls[0].include_completed,
        "include_completed=false must be forwarded to the product facade unchanged"
    );
}

#[tokio::test]
async fn pause_automation_rejects_missing_agent_id() {
    let automation_facade = Arc::new(RecordingAutomationFacade::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_automation_product_facade(automation_facade.clone());

    let err = invoke_json_product_capability(
        &services,
        caller_without_agent(),
        AUTOMATION_PAUSE_CAPABILITY_ID,
        RebornAutomationRequest {
            automation_id: "trigger-alpha".to_string(),
        },
    )
    .await
    .expect_err("missing agent id should fail closed");

    assert_eq!(err.code, RebornServicesErrorCode::InvalidRequest);
    assert_eq!(err.status_code, 400);
    assert_eq!(automation_facade.mutation_calls().len(), 0);
}

#[tokio::test]
async fn resume_automation_rejects_missing_agent_id() {
    let automation_facade = Arc::new(RecordingAutomationFacade::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_automation_product_facade(automation_facade.clone());

    let err = invoke_json_product_capability(
        &services,
        caller_without_agent(),
        AUTOMATION_RESUME_CAPABILITY_ID,
        RebornAutomationRequest {
            automation_id: "trigger-alpha".to_string(),
        },
    )
    .await
    .expect_err("missing agent id should fail closed");

    assert_eq!(err.code, RebornServicesErrorCode::InvalidRequest);
    assert_eq!(err.status_code, 400);
    assert_eq!(automation_facade.mutation_calls().len(), 0);
}

#[tokio::test]
async fn rename_automation_rejects_missing_agent_id() {
    let automation_facade = Arc::new(RecordingAutomationFacade::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_automation_product_facade(automation_facade.clone());

    let err = invoke_json_product_capability(
        &services,
        caller_without_agent(),
        AUTOMATION_RENAME_CAPABILITY_ID,
        RebornRenameAutomationProductRequest {
            automation_id: "trigger-alpha".to_string(),
            name: Some("Renamed".to_string()),
        },
    )
    .await
    .expect_err("missing agent id should fail closed");

    assert_eq!(err.code, RebornServicesErrorCode::InvalidRequest);
    assert_eq!(err.status_code, 400);
    assert_eq!(automation_facade.mutation_calls().len(), 0);
}

#[tokio::test]
async fn delete_automation_rejects_missing_agent_id() {
    let automation_facade = Arc::new(RecordingAutomationFacade::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_automation_product_facade(automation_facade.clone());

    let err = invoke_json_product_capability(
        &services,
        caller_without_agent(),
        AUTOMATION_DELETE_CAPABILITY_ID,
        RebornAutomationRequest {
            automation_id: "trigger-alpha".to_string(),
        },
    )
    .await
    .expect_err("missing agent id should fail closed");

    assert_eq!(err.code, RebornServicesErrorCode::InvalidRequest);
    assert_eq!(err.status_code, 400);
    assert_eq!(automation_facade.mutation_calls().len(), 0);
}

#[tokio::test]
async fn automation_mutations_forward_caller_scope_to_product_facade() {
    let automation_facade = Arc::new(RecordingAutomationFacade::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_automation_product_facade(automation_facade.clone());
    let caller = caller();
    let expected_agent_id = caller.agent_id.clone().expect("agent id");

    let pause = invoke_json_product_capability(
        &services,
        caller.clone(),
        AUTOMATION_PAUSE_CAPABILITY_ID,
        RebornAutomationRequest {
            automation_id: "trigger-alpha".to_string(),
        },
    )
    .await
    .expect("pause automation");
    assert!(matches!(
        pause,
        Resolution::Done(outcome) if outcome.verdict.is_success()
    ));

    let resume = invoke_json_product_capability(
        &services,
        caller.clone(),
        AUTOMATION_RESUME_CAPABILITY_ID,
        RebornAutomationRequest {
            automation_id: "trigger-alpha".to_string(),
        },
    )
    .await
    .expect("resume automation");
    assert!(matches!(
        resume,
        Resolution::Done(outcome) if outcome.verdict.is_success()
    ));

    let rename = invoke_json_product_capability(
        &services,
        caller.clone(),
        AUTOMATION_RENAME_CAPABILITY_ID,
        RebornRenameAutomationProductRequest {
            automation_id: "trigger-alpha".to_string(),
            name: Some("  Renamed status  ".to_string()),
        },
    )
    .await
    .expect("rename automation");
    assert!(matches!(
        rename,
        Resolution::Done(outcome) if outcome.verdict.is_success()
    ));

    let delete = invoke_json_product_capability(
        &services,
        caller.clone(),
        AUTOMATION_DELETE_CAPABILITY_ID,
        RebornAutomationRequest {
            automation_id: "trigger-alpha".to_string(),
        },
    )
    .await
    .expect("delete automation");
    assert!(matches!(
        delete,
        Resolution::Done(outcome) if outcome.verdict.is_success()
    ));

    let calls = automation_facade.mutation_calls();
    assert_eq!(calls.len(), 4);
    assert_eq!(calls[0].action, AutomationMutationAction::Pause);
    assert_eq!(calls[0].automation_id, "trigger-alpha");
    assert_eq!(calls[0].caller.tenant_id, caller.tenant_id);
    assert_eq!(calls[0].caller.user_id, caller.user_id);
    assert_eq!(calls[0].caller.agent_id, expected_agent_id);
    assert_eq!(calls[0].caller.project_id, caller.project_id);
    assert_eq!(calls[1].action, AutomationMutationAction::Resume);
    assert_eq!(calls[1].automation_id, "trigger-alpha");
    assert_eq!(calls[1].caller.tenant_id, caller.tenant_id);
    assert_eq!(calls[1].caller.user_id, caller.user_id);
    assert_eq!(calls[1].caller.agent_id, expected_agent_id);
    assert_eq!(calls[1].caller.project_id, caller.project_id);
    assert_eq!(
        calls[2].action,
        AutomationMutationAction::Rename {
            name: AutomationName::new("Renamed status").expect("valid automation name")
        }
    );
    assert_eq!(calls[2].automation_id, "trigger-alpha");
    assert_eq!(calls[2].caller.tenant_id, caller.tenant_id);
    assert_eq!(calls[2].caller.user_id, caller.user_id);
    assert_eq!(calls[2].caller.agent_id, expected_agent_id);
    assert_eq!(calls[2].caller.project_id, caller.project_id);
    assert_eq!(calls[3].action, AutomationMutationAction::Delete);
    assert_eq!(calls[3].automation_id, "trigger-alpha");
    assert_eq!(calls[3].caller.tenant_id, caller.tenant_id);
    assert_eq!(calls[3].caller.user_id, caller.user_id);
    assert_eq!(calls[3].caller.agent_id, expected_agent_id);
    assert_eq!(calls[3].caller.project_id, caller.project_id);
}

#[tokio::test]
async fn automation_mutations_are_available_as_product_capabilities() {
    let automation_facade = Arc::new(RecordingAutomationFacade::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_automation_product_facade(automation_facade.clone());

    for (capability_id, input) in [
        (
            AUTOMATION_PAUSE_CAPABILITY_ID,
            serde_json::to_value(RebornAutomationRequest {
                automation_id: "trigger-alpha".to_string(),
            })
            .expect("pause input"),
        ),
        (
            AUTOMATION_RESUME_CAPABILITY_ID,
            serde_json::to_value(RebornAutomationRequest {
                automation_id: "trigger-alpha".to_string(),
            })
            .expect("resume input"),
        ),
        (
            AUTOMATION_RENAME_CAPABILITY_ID,
            serde_json::to_value(RebornRenameAutomationProductRequest {
                automation_id: "trigger-alpha".to_string(),
                name: Some("Renamed status".to_string()),
            })
            .expect("rename input"),
        ),
        (
            AUTOMATION_DELETE_CAPABILITY_ID,
            serde_json::to_value(RebornAutomationRequest {
                automation_id: "trigger-alpha".to_string(),
            })
            .expect("delete input"),
        ),
    ] {
        let resolution = services
            .invoke(
                caller(),
                CapabilityId::new(capability_id).expect("capability id"),
                ProductCapabilityInput::json(input),
                ActivityId::new(),
            )
            .await
            .expect("automation capability");
        assert!(matches!(
            resolution,
            Resolution::Done(outcome) if outcome.verdict.is_success()
        ));
    }

    let calls = automation_facade.mutation_calls();
    assert_eq!(calls.len(), 4);
    assert_eq!(calls[0].action, AutomationMutationAction::Pause);
    assert_eq!(calls[1].action, AutomationMutationAction::Resume);
    assert_eq!(
        calls[2].action,
        AutomationMutationAction::Rename {
            name: AutomationName::new("Renamed status").expect("valid automation name")
        }
    );
    assert_eq!(calls[3].action, AutomationMutationAction::Delete);
}

#[tokio::test]
async fn rename_automation_validates_name_before_product_facade() {
    let automation_facade = Arc::new(RecordingAutomationFacade::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_automation_product_facade(automation_facade.clone());

    for (request, expected_code) in [
        (
            WebUiRenameAutomationRequest { name: None },
            WebUiInboundValidationCode::MissingField,
        ),
        (
            WebUiRenameAutomationRequest {
                name: Some("  ".to_string()),
            },
            WebUiInboundValidationCode::Blank,
        ),
        (
            WebUiRenameAutomationRequest {
                name: Some("x".repeat(257)),
            },
            WebUiInboundValidationCode::TooLong,
        ),
    ] {
        let err = invoke_json_product_capability(
            &services,
            caller(),
            AUTOMATION_RENAME_CAPABILITY_ID,
            RebornRenameAutomationProductRequest {
                automation_id: "trigger-alpha".to_string(),
                name: request.name,
            },
        )
        .await
        .expect_err("invalid name should fail before facade");

        assert_eq!(err.code, RebornServicesErrorCode::InvalidRequest);
        assert_eq!(err.status_code, 400);
        assert_eq!(err.field.as_deref(), Some("name"));
        assert_eq!(err.validation_code, Some(expected_code));
    }

    assert_eq!(automation_facade.mutation_calls().len(), 0);
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
        thread_id: Some(ThreadId::new("thread-listed").expect("valid thread id")),
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

#[derive(Default)]
struct RecordingOperatorLogsService {
    requests: Mutex<Vec<RebornLogQueryRequest>>,
}

impl RecordingOperatorLogsService {
    fn requests(&self) -> Vec<RebornLogQueryRequest> {
        self.requests.lock().expect("lock").clone()
    }
}

#[async_trait]
impl OperatorLogsService for RecordingOperatorLogsService {
    async fn query_logs(
        &self,
        _caller: WebUiAuthenticatedCaller,
        request: RebornLogQueryRequest,
    ) -> Result<RebornLogQueryResponse, RebornServicesError> {
        self.requests.lock().expect("lock").push(request);
        Ok(RebornLogQueryResponse {
            source: "test".to_string(),
            entries: Vec::new(),
            next_cursor: None,
            tail_supported: false,
            follow_supported: false,
        })
    }
}

struct CrateRootLifecycleBackend;

async fn query_logs_view(
    services: &RebornServices,
    caller: WebUiAuthenticatedCaller,
    mut request: RebornLogQueryRequest,
) -> Result<RebornLogQueryResponse, RebornServicesError> {
    let cursor = request.cursor.take();
    let page = services
        .query(
            caller,
            RebornViewQuery {
                view_id: LOGS_VIEW.id.to_string(),
                params: serde_json::to_value(request)
                    .map_err(RebornServicesError::internal_from)?,
                cursor,
            },
        )
        .await?;
    serde_json::from_value(page.payload).map_err(RebornServicesError::internal_from)
}

async fn query_operator_logs_view(
    services: &RebornServices,
    caller: WebUiAuthenticatedCaller,
    mut request: RebornOperatorLogsQuery,
) -> Result<RebornOperatorCommandPlaneResponse, RebornServicesError> {
    let cursor = request.cursor.take();
    let page = services
        .query(
            caller,
            RebornViewQuery {
                view_id: OPERATOR_LOGS_VIEW.id.to_string(),
                params: serde_json::to_value(request)
                    .map_err(RebornServicesError::internal_from)?,
                cursor,
            },
        )
        .await?;
    serde_json::from_value(page.payload).map_err(RebornServicesError::internal_from)
}

#[async_trait]
impl OperatorServiceLifecycleService for CrateRootLifecycleBackend {
    async fn control_service(
        &self,
        _caller: WebUiAuthenticatedCaller,
        request: RebornServiceLifecycleRequest,
    ) -> Result<RebornServiceLifecycleResponse, RebornServicesError> {
        Ok(RebornServiceLifecycleResponse {
            action: request.action,
            state: RebornServiceLifecycleState::Unsupported,
            message: "not wired".to_string(),
            remediation: None,
        })
    }
}

#[tokio::test]
async fn query_operator_logs_bounds_query_before_logs_service() {
    let operator_logs = Arc::new(RecordingOperatorLogsService::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_operator_logs_service(operator_logs.clone());

    let oversized_cursor = format!("  {}  ", "c".repeat(2048));
    let oversized_target = format!("{}é", "t".repeat(512));
    let oversized_thread_id = format!("{}é", "thread-".repeat(80));
    let oversized_run_id = format!("{}é", "run-".repeat(100));
    let boundary_source = format!("{}é", "s".repeat(254));
    let response = query_operator_logs_view(
        &services,
        caller().with_operator_webui_config(true),
        RebornOperatorLogsQuery {
            limit: Some(u32::MAX),
            cursor: Some(oversized_cursor),
            level: Some(RebornLogLevel::Warn),
            target: Some(oversized_target),
            thread_id: Some(oversized_thread_id),
            run_id: Some(oversized_run_id),
            turn_id: Some("turn-1".to_string()),
            tool_call_id: Some("tool-call-1".to_string()),
            tool_name: Some("shell".to_string()),
            source: Some(boundary_source),
            tail: true,
            follow: false,
        },
    )
    .await
    .expect("operator logs query");

    assert_eq!(response.status, RebornOperatorSurfaceStatus::Available);
    let requests = operator_logs.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].limit, Some(500));
    assert_eq!(requests[0].cursor.as_ref().map(String::len), Some(512));
    assert_eq!(requests[0].target.as_ref().map(String::len), Some(256));
    assert_eq!(requests[0].thread_id.as_ref().map(String::len), Some(256));
    assert_eq!(requests[0].run_id.as_ref().map(String::len), Some(256));
    assert_eq!(requests[0].turn_id.as_deref(), Some("turn-1"));
    assert_eq!(requests[0].tool_call_id.as_deref(), Some("tool-call-1"));
    assert_eq!(requests[0].tool_name.as_deref(), Some("shell"));
    let source = requests[0].source.as_deref().expect("bounded source");
    assert_eq!(source.len(), 256);
    assert!(source.ends_with('é'));
    assert!(source.is_char_boundary(source.len()));
    let run_id = requests[0].run_id.as_deref().expect("bounded run id");
    assert_eq!(run_id.len(), 256);
    assert!(run_id.ends_with(" ... [truncated]"));
    assert!(run_id.is_char_boundary(run_id.len()));
    assert_eq!(requests[0].level, Some(RebornLogLevel::Warn));
    assert!(requests[0].tail);
    assert!(!requests[0].follow);
}

#[tokio::test]
async fn query_operator_logs_forwards_follow_mode_to_logs_service() {
    let operator_logs = Arc::new(RecordingOperatorLogsService::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_operator_logs_service(operator_logs.clone());

    query_operator_logs_view(
        &services,
        caller().with_operator_webui_config(true),
        RebornOperatorLogsQuery {
            limit: Some(25),
            cursor: Some("after:7".to_string()),
            level: Some(RebornLogLevel::Info),
            target: Some("ironclaw".to_string()),
            thread_id: None,
            run_id: None,
            turn_id: None,
            tool_call_id: None,
            tool_name: None,
            source: None,
            tail: false,
            follow: true,
        },
    )
    .await
    .expect("operator logs follow query");

    let requests = operator_logs.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].limit, Some(25));
    assert_eq!(requests[0].cursor.as_deref(), Some("after:7"));
    assert_eq!(requests[0].level, Some(RebornLogLevel::Info));
    assert_eq!(requests[0].target.as_deref(), Some("ironclaw"));
    assert!(!requests[0].tail);
    assert!(requests[0].follow);
}

#[tokio::test]
async fn query_operator_logs_rejects_ambiguous_tail_follow_modes() {
    let operator_logs = Arc::new(RecordingOperatorLogsService::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_operator_logs_service(operator_logs.clone());

    let err = query_operator_logs_view(
        &services,
        caller().with_operator_webui_config(true),
        RebornOperatorLogsQuery {
            limit: None,
            cursor: None,
            level: None,
            target: None,
            thread_id: None,
            run_id: None,
            turn_id: None,
            tool_call_id: None,
            tool_name: None,
            source: None,
            tail: true,
            follow: true,
        },
    )
    .await
    .expect_err("tail and follow cannot be combined");

    assert_eq!(err.kind, RebornServicesErrorKind::Validation);
    assert_eq!(err.status_code, 400);
    assert_eq!(err.field.as_deref(), Some("follow"));
    assert_eq!(
        err.validation_code,
        Some(WebUiInboundValidationCode::InvalidValue)
    );
    assert!(operator_logs.requests().is_empty());
}

#[tokio::test]
async fn query_logs_requires_thread_scope() {
    let operator_logs = Arc::new(RecordingOperatorLogsService::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_operator_logs_service(operator_logs.clone());

    let err = query_logs_view(&services, caller(), RebornLogQueryRequest::default())
        .await
        .expect_err("public logs require a thread scope");

    assert_eq!(err.kind, RebornServicesErrorKind::Validation);
    assert_eq!(err.status_code, 400);
    assert_eq!(err.field.as_deref(), Some("thread_id"));
    assert_eq!(
        err.validation_code,
        Some(WebUiInboundValidationCode::MissingField)
    );
    assert!(operator_logs.requests().is_empty());
}

#[tokio::test]
async fn query_logs_rejects_ambiguous_tail_follow_modes() {
    let operator_logs = Arc::new(RecordingOperatorLogsService::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_operator_logs_service(operator_logs.clone());

    let err = query_logs_view(
        &services,
        caller(),
        RebornLogQueryRequest::default()
            .set_thread_id("thread-alpha")
            .set_tail(true)
            .set_follow(true),
    )
    .await
    .expect_err("tail and follow cannot be combined");

    assert_eq!(err.kind, RebornServicesErrorKind::Validation);
    assert_eq!(err.status_code, 400);
    assert_eq!(err.field.as_deref(), Some("follow"));
    assert_eq!(
        err.validation_code,
        Some(WebUiInboundValidationCode::InvalidValue)
    );
    assert!(operator_logs.requests().is_empty());
}

#[tokio::test]
async fn query_logs_forwards_owned_thread_scope_to_logs_service() {
    let operator_logs = Arc::new(RecordingOperatorLogsService::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_operator_logs_service(operator_logs.clone());

    setup_owned_thread(&services, caller(), "thread-alpha").await;

    query_logs_view(
        &services,
        caller(),
        RebornLogQueryRequest::default()
            .set_limit(25)
            .set_cursor("after:7")
            .set_level(RebornLogLevel::Info)
            .set_target("ironclaw")
            .set_thread_id("thread-alpha")
            .set_follow(true),
    )
    .await
    .expect("owned thread logs query");

    let requests = operator_logs.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].limit, Some(25));
    assert_eq!(requests[0].cursor.as_deref(), Some("after:7"));
    assert_eq!(requests[0].level, Some(RebornLogLevel::Info));
    assert_eq!(requests[0].target.as_deref(), Some("ironclaw"));
    assert_eq!(requests[0].thread_id.as_deref(), Some("thread-alpha"));
    assert!(!requests[0].tail);
    assert!(requests[0].follow);
}

#[tokio::test]
async fn query_logs_rejects_thread_owned_by_another_caller() {
    let operator_logs = Arc::new(RecordingOperatorLogsService::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_operator_logs_service(operator_logs.clone());

    setup_owned_thread(&services, caller_for_user("user-bob"), "thread-bob").await;

    let err = query_logs_view(
        &services,
        caller(),
        RebornLogQueryRequest::default()
            .set_limit(25)
            .set_thread_id("thread-bob"),
    )
    .await
    .expect_err("foreign thread logs are not caller-visible");

    assert_eq!(err.status_code, 404);
    assert_eq!(err.kind, RebornServicesErrorKind::NotFound);
    assert!(operator_logs.requests().is_empty());
}

#[tokio::test]
async fn run_artifact_selects_one_owned_run_and_queries_only_its_scoped_logs() {
    let owner = caller();
    let thread_scope = thread_scope_for(&owner);
    let thread_id = ThreadId::new("thread-artifact").expect("thread id");
    let run_id = TurnRunId::parse(&run_id_string()).expect("run id");
    let other_run_id = TurnRunId::new();
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    thread_service
        .ensure_thread(EnsureThreadRequest {
            scope: thread_scope.clone(),
            thread_id: Some(thread_id.clone()),
            created_by_actor_id: owner.user_id.as_str().to_string(),
            title: None,
            metadata_json: None,
        })
        .await
        .expect("thread");
    seed_submitted_message(
        &thread_service,
        &thread_scope,
        &thread_id,
        &run_id,
        "selected run",
    )
    .await;
    seed_submitted_message(
        &thread_service,
        &thread_scope,
        &thread_id,
        &other_run_id,
        "other run",
    )
    .await;
    let operator_logs = Arc::new(RecordingOperatorLogsService::default());
    let services = RebornServices::new(thread_service, Arc::new(FakeTurnCoordinator::default()))
        .with_operator_logs_service(operator_logs.clone());

    let page = services
        .query(
            owner,
            RebornViewQuery {
                view_id: RUN_ARTIFACT_VIEW.id.to_string(),
                params: serde_json::to_value(RebornRunArtifactRequest {
                    thread_id: thread_id.to_string(),
                    run_id: run_id.to_string(),
                })
                .expect("artifact params"),
                cursor: None,
            },
        )
        .await
        .expect("owned run artifact");
    let artifact: RebornRunArtifact =
        serde_json::from_value(page.payload).expect("artifact payload");

    assert_eq!(artifact.messages.len(), 1);
    assert_eq!(artifact.messages[0].content, "selected run");
    assert!(!artifact.logs.complete);
    let requests = operator_logs.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].thread_id.as_deref(), Some("thread-artifact"));
    assert_eq!(
        requests[0].run_id.as_deref(),
        Some(run_id.to_string().as_str())
    );
    assert_eq!(requests[0].limit, Some(500));
}

#[tokio::test]
async fn run_artifact_rejects_another_user_before_querying_logs() {
    let owner = caller_for_user("user-bob");
    let thread_scope = thread_scope_for(&owner);
    let thread_id = ThreadId::new("thread-bob-artifact").expect("thread id");
    let run_id = TurnRunId::parse(&run_id_string()).expect("run id");
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    thread_service
        .ensure_thread(EnsureThreadRequest {
            scope: thread_scope.clone(),
            thread_id: Some(thread_id.clone()),
            created_by_actor_id: owner.user_id.as_str().to_string(),
            title: None,
            metadata_json: None,
        })
        .await
        .expect("thread");
    seed_submitted_message(
        &thread_service,
        &thread_scope,
        &thread_id,
        &run_id,
        "private trajectory",
    )
    .await;
    let operator_logs = Arc::new(RecordingOperatorLogsService::default());
    let services = RebornServices::new(thread_service, Arc::new(FakeTurnCoordinator::default()))
        .with_operator_logs_service(operator_logs.clone());

    let error = services
        .query(
            caller(),
            RebornViewQuery {
                view_id: RUN_ARTIFACT_VIEW.id.to_string(),
                params: serde_json::to_value(RebornRunArtifactRequest {
                    thread_id: thread_id.to_string(),
                    run_id: run_id.to_string(),
                })
                .expect("artifact params"),
                cursor: None,
            },
        )
        .await
        .expect_err("foreign run must not be exported");

    assert_eq!(error.status_code, 404);
    assert_eq!(error.kind, RebornServicesErrorKind::NotFound);
    assert!(operator_logs.requests().is_empty());
}

async fn seed_submitted_message(
    thread_service: &Arc<InMemorySessionThreadService>,
    scope: &ThreadScope,
    thread_id: &ThreadId,
    run_id: &TurnRunId,
    content: &str,
) {
    let accepted = thread_service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope.clone(),
            thread_id: thread_id.clone(),
            actor_id: scope
                .owner_user_id
                .as_ref()
                .expect("test owner")
                .as_str()
                .to_string(),
            source_binding_id: None,
            reply_target_binding_id: None,
            external_event_id: Some(format!("artifact-{run_id}")),
            content: MessageContent::text(content),
        })
        .await
        .expect("accepted");
    thread_service
        .mark_message_submitted(
            scope,
            thread_id,
            accepted.message_id,
            format!("turn-{run_id}"),
            run_id.to_string(),
        )
        .await
        .expect("submitted");
}

#[tokio::test]
async fn operator_service_lifecycle_contract_is_implementable_from_crate_root() {
    let backend = CrateRootLifecycleBackend;
    let response = backend
        .control_service(
            caller(),
            RebornServiceLifecycleRequest {
                action: RebornServiceLifecycleAction::Status,
            },
        )
        .await
        .expect("crate-root lifecycle service implementation");

    assert_eq!(response.action, RebornServiceLifecycleAction::Status);
    assert_eq!(response.state, RebornServiceLifecycleState::Unsupported);
}

/// External creator user id used in trigger-thread scope tests.
///
/// Trigger threads are stored with the `creator_user_id` of the actor that
/// fired the trigger (e.g. a Slack user), which is intentionally different
/// from the WebUI caller (`"user-alpha"`/`"user-alice"`/`"user-bob"`).
/// Using a distinct value here proves the scope reconstruction uses the
/// stored creator — not the caller — to build the `ThreadScope`.
const TRIGGER_CREATOR_USER_ID: &str = "user-trigger-creator";

/// Build a `ThreadScope` matching how `record_trigger_prompt` actually stores
/// trigger-fired threads: same tenant/agent/project as the trigger record, but
/// `owner_user_id` = the **external creator** (not the WebUI caller).
fn trigger_thread_scope_for(caller: &WebUiAuthenticatedCaller) -> ThreadScope {
    ThreadScope {
        tenant_id: caller.tenant_id.clone(),
        agent_id: caller.agent_id.clone().expect("agent id"),
        project_id: caller.project_id.clone(),
        owner_user_id: Some(
            UserId::new(TRIGGER_CREATOR_USER_ID).expect("valid trigger creator user id"),
        ),
        mission_id: None,
    }
}

/// Build the `TriggerRunThreadScope` that `resolve_run_thread_scope` returns
/// for a trigger whose thread was stored via `trigger_thread_scope_for`.
fn trigger_run_thread_scope_for(caller: &WebUiAuthenticatedCaller) -> TriggerRunThreadScope {
    TriggerRunThreadScope {
        agent_id: caller.agent_id.clone(),
        project_id: caller.project_id.clone(),
        creator_user_id: UserId::new(TRIGGER_CREATOR_USER_ID)
            .expect("valid trigger creator user id"),
    }
}

// Regression tests for the automation-trigger timeline fallback.
// Bug: `get_timeline` scoped the thread lookup to the WebUI user's
// `owner_user_id`, but trigger-fired threads are stored with the external
// creator's `owner_user_id`.  The user-scoped probe returned `UnknownThread`,
// and the handler propagated `404` without checking whether the thread
// belongs to one of the caller's automations.

#[tokio::test]
async fn get_timeline_succeeds_for_own_automation_trigger_thread() {
    // Trigger thread stored with the EXTERNAL creator's owner_user_id — not the
    // WebUI caller's.  The old guessing code would produce a caller-scoped
    // ThreadScope and miss this thread; the new `resolve_run_thread_scope` path
    // must reconstruct the true scope and return the history.
    let trigger_thread_id = ThreadId::new("thread-trigger-alpha").expect("valid trigger thread id");
    let caller = caller();
    let thread_service = Arc::new(InMemorySessionThreadService::default());

    // Store the trigger thread under the external creator's scope (not caller).
    thread_service
        .ensure_thread(EnsureThreadRequest {
            scope: trigger_thread_scope_for(&caller),
            thread_id: Some(trigger_thread_id.clone()),
            created_by_actor_id: "system".to_string(),
            title: Some("Scheduled run".to_string()),
            metadata_json: Some(automation_trigger_thread_metadata_json(
                "trigger-scheduled-alpha",
            )),
        })
        .await
        .expect("trigger thread stored");

    // The automation facade recognises the thread and returns the trigger scope.
    let automation_facade = Arc::new(
        StaticAutomationFacade::new(vec![RebornAutomationInfo {
            automation_id: "trigger-scheduled-alpha".to_string(),
            name: "Morning briefing".to_string(),
            source: RebornAutomationSource::Schedule {
                cron: "0 9 * * *".to_string(),
                timezone: "UTC".to_string(),
            },
            state: RebornAutomationState::Active,
            next_run_at: None,
            last_run_at: None,
            last_status: Some(RebornAutomationRunStatus::Ok),
            recent_runs: vec![RebornAutomationRecentRunInfo {
                run_id: Some(automation_run_id()),
                thread_id: Some(trigger_thread_id.clone()),
                fire_slot: None,
                status: RebornAutomationRecentRunStatus::Ok,
                submitted_at: "2026-06-09T09:00:01Z".parse().expect("submitted_at"),
                completed_at: Some("2026-06-09T09:00:42Z".parse().expect("completed_at")),
            }],
            is_active: true,
            created_at: None,
            active_hold: None,
        }])
        .with_resolve_scope_for_thread(
            trigger_thread_id.clone(),
            trigger_run_thread_scope_for(&caller),
        ),
    );

    let services = RebornServices::new(thread_service, Arc::new(FakeTurnCoordinator::default()))
        .with_automation_product_facade(automation_facade);

    let response = services
        .get_timeline(
            caller,
            RebornTimelineRequest::new(trigger_thread_id.as_str().to_string()),
        )
        .await
        .expect("owner should be able to read their automation trigger thread timeline");

    assert_eq!(response.thread.thread_id, trigger_thread_id);
}

/// Records the scope and storage key each byte read is issued under so a test
/// can assert the reader addressed the right project mount AND resolved the
/// right attachment key.
struct RecordingAttachmentReader {
    bytes: Vec<u8>,
    reads: Mutex<Vec<(ThreadScope, String)>>,
}

#[async_trait]
impl InboundAttachmentReader for RecordingAttachmentReader {
    async fn read(
        &self,
        thread_scope: &ThreadScope,
        storage_key: &str,
    ) -> Result<Vec<u8>, RebornServicesError> {
        self.reads
            .lock()
            .expect("lock")
            .push((thread_scope.clone(), storage_key.to_string()));
        Ok(self.bytes.clone())
    }
}

// Regression for the trigger-thread byte-read scope. `read_attachment` shares
// the timeline's automation-trigger fallback, which resolves the thread under
// the trigger creator's scope (not the WebUI caller's session scope). The bytes
// must be read back under that same resolved scope — reading under the caller's
// session scope would address the wrong project mount and 404.
#[tokio::test]
async fn read_attachment_reads_trigger_thread_bytes_under_creator_scope() {
    let trigger_thread_id = ThreadId::new("thread-trigger-bytes").expect("valid trigger thread id");
    let caller = caller();
    let thread_service = Arc::new(InMemorySessionThreadService::default());

    thread_service
        .ensure_thread(EnsureThreadRequest {
            scope: trigger_thread_scope_for(&caller),
            thread_id: Some(trigger_thread_id.clone()),
            created_by_actor_id: "system".to_string(),
            title: Some("Scheduled run".to_string()),
            metadata_json: Some(automation_trigger_thread_metadata_json("trigger-bytes")),
        })
        .await
        .expect("trigger thread stored");

    // A landed image attachment on the trigger thread, stored under the
    // creator's scope.
    let accepted = thread_service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: trigger_thread_scope_for(&caller),
            thread_id: trigger_thread_id.clone(),
            actor_id: "system".to_string(),
            source_binding_id: None,
            reply_target_binding_id: None,
            external_event_id: Some("trigger-image".to_string()),
            content: MessageContent::with_attachments(
                "see image",
                vec![AttachmentRef {
                    id: "att-0".to_string(),
                    kind: AttachmentKind::Image,
                    mime_type: "image/png".to_string(),
                    filename: Some("p.png".to_string()),
                    size_bytes: Some(4),
                    storage_key: Some("/workspace/attachments/2026-06-14/m-0-p.png".to_string()),
                    extracted_text: None,
                }],
            ),
        })
        .await
        .expect("message with attachment accepted");

    let automation_facade = Arc::new(
        StaticAutomationFacade::new(vec![RebornAutomationInfo {
            automation_id: "trigger-bytes".to_string(),
            name: "Morning briefing".to_string(),
            source: RebornAutomationSource::Schedule {
                cron: "0 9 * * *".to_string(),
                timezone: "UTC".to_string(),
            },
            state: RebornAutomationState::Active,
            next_run_at: None,
            last_run_at: None,
            last_status: Some(RebornAutomationRunStatus::Ok),
            recent_runs: vec![RebornAutomationRecentRunInfo {
                run_id: Some(automation_run_id()),
                thread_id: Some(trigger_thread_id.clone()),
                fire_slot: None,
                status: RebornAutomationRecentRunStatus::Ok,
                submitted_at: "2026-06-09T09:00:01Z".parse().expect("submitted_at"),
                completed_at: Some("2026-06-09T09:00:42Z".parse().expect("completed_at")),
            }],
            is_active: true,
            created_at: None,
            active_hold: None,
        }])
        .with_resolve_scope_for_thread(
            trigger_thread_id.clone(),
            trigger_run_thread_scope_for(&caller),
        ),
    );

    let reader = Arc::new(RecordingAttachmentReader {
        bytes: vec![1, 2, 3, 4],
        reads: Mutex::new(Vec::new()),
    });
    let services = RebornServices::new(thread_service, Arc::new(FakeTurnCoordinator::default()))
        .with_automation_product_facade(automation_facade)
        .with_inbound_attachment_reader(reader.clone());

    let result = services
        .read_attachment(
            caller,
            RebornAttachmentRequest {
                thread_id: trigger_thread_id.as_str().to_string(),
                message_id: accepted.message_id.to_string(),
                attachment_id: "att-0".to_string(),
            },
        )
        .await
        .expect("owner should be able to read their trigger thread's attachment");

    assert_eq!(result.bytes, vec![1, 2, 3, 4]);
    assert_eq!(result.mime_type, "image/png");

    // The fix: the read was issued under the trigger creator's scope (not the
    // caller's session scope) and for the landed attachment's own storage key.
    let reads = reader.reads.lock().expect("lock");
    assert_eq!(reads.len(), 1);
    let (scope, storage_key) = &reads[0];
    assert_eq!(
        scope.owner_user_id,
        Some(UserId::new(TRIGGER_CREATOR_USER_ID).expect("trigger creator user id")),
    );
    assert_eq!(storage_key, "/workspace/attachments/2026-06-14/m-0-p.png");
}

#[tokio::test]
async fn get_timeline_rejects_other_users_automation_trigger_thread() {
    // A trigger thread owned by alice's automation. Bob tries to read it.
    let alice = caller_for_user("user-alice");
    let bob = caller_for_user("user-bob");
    let trigger_thread_id = ThreadId::new("thread-trigger-beta").expect("valid trigger thread id");

    let thread_service = Arc::new(InMemorySessionThreadService::default());
    // Store the thread in alice's trigger scope.
    thread_service
        .ensure_thread(EnsureThreadRequest {
            scope: trigger_thread_scope_for(&alice),
            thread_id: Some(trigger_thread_id.clone()),
            created_by_actor_id: "system".to_string(),
            title: Some("Alice's scheduled run".to_string()),
            metadata_json: Some(automation_trigger_thread_metadata_json(
                "trigger-alices-job",
            )),
        })
        .await
        .expect("alice trigger thread stored");

    // Bob's facade returns no automations and no resolve_scope — the fallback
    // must deny him because resolve_run_thread_scope returns None.
    let automation_facade = Arc::new(StaticAutomationFacade::new(Vec::new()));

    let services = RebornServices::new(thread_service, Arc::new(FakeTurnCoordinator::default()))
        .with_automation_product_facade(automation_facade);

    let err = services
        .get_timeline(
            bob,
            RebornTimelineRequest::new(trigger_thread_id.as_str().to_string()),
        )
        .await
        .expect_err("non-owner must not read another user's trigger thread");

    assert_eq!(err.code, RebornServicesErrorCode::NotFound);
    assert_eq!(err.status_code, 404);
}

// Contract: backend errors from `resolve_run_thread_scope` must surface as 503
// Unavailable, not be masked as 404 NotFound.  A backend outage should never
// look like an authorization miss to the caller.
#[tokio::test]
async fn get_timeline_surfaces_trigger_scope_lookup_backend_error() {
    // The primary user-scoped lookup will miss (thread stored under trigger
    // creator scope), then the automation fallback fires.  The facade returns
    // a 503 Unavailable error — the service must propagate that error rather
    // than converting it to 404.
    let caller = caller();
    let trigger_thread_id =
        ThreadId::new("thread-trigger-backend-err").expect("valid trigger thread id");

    let thread_service = Arc::new(InMemorySessionThreadService::default());
    // Store the thread under the external creator's scope so the user-scoped
    // lookup misses and the automation fallback is invoked.
    thread_service
        .ensure_thread(EnsureThreadRequest {
            scope: trigger_thread_scope_for(&caller),
            thread_id: Some(trigger_thread_id.clone()),
            created_by_actor_id: "system".to_string(),
            title: Some("Trigger backend error test thread".to_string()),
            metadata_json: Some(automation_trigger_thread_metadata_json(
                "trigger-backend-err-automation",
            )),
        })
        .await
        .expect("trigger thread stored");

    // The automation facade returns a 503 backend error from resolve_run_thread_scope.
    let automation_facade = Arc::new(ErroringAutomationFacade::unavailable());

    let services = RebornServices::new(thread_service, Arc::new(FakeTurnCoordinator::default()))
        .with_automation_product_facade(automation_facade);

    let err = services
        .get_timeline(
            caller,
            RebornTimelineRequest::new(trigger_thread_id.as_str().to_string()),
        )
        .await
        .expect_err("backend error from facade must propagate, not become 404");

    assert_eq!(
        err.code,
        RebornServicesErrorCode::Unavailable,
        "backend lookup error must surface as Unavailable, not NotFound"
    );
    assert_eq!(err.status_code, 503);
    assert!(err.retryable, "backend outage error must be retryable");
}

/// A `SessionThreadService` that returns `UnknownThread` on its first
/// `list_thread_history` call and `Backend(...)` on every subsequent call.
/// Used to test the error-taxonomy contract when the caller-scoped probe misses
/// (→ automation fallback fires) but the trigger-owned scope reload then errors.
struct FirstMissBackendErrorThreadService {
    call_count: Mutex<usize>,
}

impl FirstMissBackendErrorThreadService {
    fn new() -> Self {
        Self {
            call_count: Mutex::new(0),
        }
    }
}

#[async_trait]
impl SessionThreadService for FirstMissBackendErrorThreadService {
    async fn list_thread_history(
        &self,
        request: ThreadHistoryRequest,
    ) -> Result<ThreadHistory, SessionThreadError> {
        let mut count = self.call_count.lock().expect("lock");
        *count += 1;
        if *count == 1 {
            Err(SessionThreadError::UnknownThread {
                thread_id: request.thread_id,
            })
        } else {
            Err(SessionThreadError::Backend(
                "backend error on trigger-owned reload".to_string(),
            ))
        }
    }

    async fn ensure_thread(
        &self,
        _request: EnsureThreadRequest,
    ) -> Result<SessionThreadRecord, SessionThreadError> {
        panic!("FirstMissBackendErrorThreadService::ensure_thread should not be reached")
    }

    async fn accept_inbound_message(
        &self,
        _request: AcceptInboundMessageRequest,
    ) -> Result<AcceptedInboundMessage, SessionThreadError> {
        panic!("FirstMissBackendErrorThreadService::accept_inbound_message should not be reached")
    }

    async fn replay_accepted_inbound_message(
        &self,
        _request: ReplayAcceptedInboundMessageRequest,
    ) -> Result<Option<AcceptedInboundMessageReplay>, SessionThreadError> {
        panic!(
            "FirstMissBackendErrorThreadService::replay_accepted_inbound_message should not be reached"
        )
    }

    async fn mark_message_submitted(
        &self,
        _scope: &ThreadScope,
        _thread_id: &ThreadId,
        _message_id: ThreadMessageId,
        _turn_id: String,
        _turn_run_id: String,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        panic!("FirstMissBackendErrorThreadService::mark_message_submitted should not be reached")
    }

    async fn mark_message_rejected_busy(
        &self,
        _scope: &ThreadScope,
        _thread_id: &ThreadId,
        _message_id: ThreadMessageId,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        panic!(
            "FirstMissBackendErrorThreadService::mark_message_rejected_busy should not be reached"
        )
    }

    async fn append_assistant_draft(
        &self,
        _request: AppendAssistantDraftRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        panic!("FirstMissBackendErrorThreadService::append_assistant_draft should not be reached")
    }

    async fn append_tool_result_reference(
        &self,
        _request: AppendToolResultReferenceRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        panic!(
            "FirstMissBackendErrorThreadService::append_tool_result_reference should not be reached"
        )
    }

    async fn append_capability_display_preview(
        &self,
        _request: AppendCapabilityDisplayPreviewRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        panic!(
            "FirstMissBackendErrorThreadService::append_capability_display_preview should not be reached"
        )
    }

    async fn update_tool_result_reference(
        &self,
        _request: UpdateToolResultReferenceRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        panic!(
            "FirstMissBackendErrorThreadService::update_tool_result_reference should not be reached"
        )
    }

    async fn update_assistant_draft(
        &self,
        _request: UpdateAssistantDraftRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        panic!("FirstMissBackendErrorThreadService::update_assistant_draft should not be reached")
    }

    async fn finalize_assistant_message(
        &self,
        _scope: &ThreadScope,
        _thread_id: &ThreadId,
        _message_id: ThreadMessageId,
        _content: MessageContent,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        panic!(
            "FirstMissBackendErrorThreadService::finalize_assistant_message should not be reached"
        )
    }

    async fn redact_message(
        &self,
        _request: RedactMessageRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        panic!("FirstMissBackendErrorThreadService::redact_message should not be reached")
    }

    async fn load_context_window(
        &self,
        _request: LoadContextWindowRequest,
    ) -> Result<ContextWindow, SessionThreadError> {
        panic!("FirstMissBackendErrorThreadService::load_context_window should not be reached")
    }

    async fn load_context_messages(
        &self,
        _request: LoadContextMessagesRequest,
    ) -> Result<ContextMessages, SessionThreadError> {
        panic!("FirstMissBackendErrorThreadService::load_context_messages should not be reached")
    }

    async fn list_threads_for_scope(
        &self,
        _request: ListThreadsForScopeRequest,
    ) -> Result<ListThreadsForScopeResponse, SessionThreadError> {
        panic!("FirstMissBackendErrorThreadService::list_threads_for_scope should not be reached")
    }

    async fn create_summary_artifact(
        &self,
        _request: CreateSummaryArtifactRequest,
    ) -> Result<SummaryArtifact, SessionThreadError> {
        panic!("FirstMissBackendErrorThreadService::create_summary_artifact should not be reached")
    }
}

// Contract: when the caller-scoped probe misses (UnknownThread → automation
// fallback fires) and `resolve_run_thread_scope` authorizes access, but the
// second `list_thread_history` call for the trigger-owned scope returns a
// backend error, the result must be Unavailable (503) — NOT the 404 NotFound
// that would have been returned had the automation facade also denied access.
// A backend outage must never be surfaced as an authorization miss.
#[tokio::test]
async fn get_timeline_surfaces_backend_error_from_unscoped_trigger_history_reload() {
    let caller = caller();
    let trigger_thread_id =
        ThreadId::new("thread-trigger-reload-error").expect("valid trigger thread id");

    // Thread service: first call (caller-scoped probe) → UnknownThread,
    // second call (trigger-owned scope reload) → Backend error.
    let thread_service = Arc::new(FirstMissBackendErrorThreadService::new());

    // Automation facade authorizes: the facade resolves a scope for the
    // thread, so the service proceeds to the trigger-owned reload.
    let automation_facade = Arc::new(
        StaticAutomationFacade::new(Vec::new()).with_resolve_scope_for_thread(
            trigger_thread_id.clone(),
            trigger_run_thread_scope_for(&caller),
        ),
    );

    let services = RebornServices::new(thread_service, Arc::new(FakeTurnCoordinator::default()))
        .with_automation_product_facade(automation_facade);

    let err = services
        .get_timeline(
            caller,
            RebornTimelineRequest::new(trigger_thread_id.as_str().to_string()),
        )
        .await
        .expect_err("backend error on trigger-owned reload must surface as 503, not 404");

    // Must be Unavailable, not NotFound: the backend error on the reload
    // must not be mistaken for an authorization miss.
    assert_eq!(
        err.code,
        RebornServicesErrorCode::Unavailable,
        "trigger-owned reload backend error must map to Unavailable, not NotFound"
    );
    assert_eq!(err.status_code, 503);
    assert!(err.retryable, "backend outage must be retryable");
}

// Contract: when `TriggerRunThreadScope.agent_id` is `None` the fallback must
// substitute the bound caller's `agent_id` so the reconstructed `TurnScope`
// can locate the thread in storage.
#[tokio::test]
async fn get_timeline_uses_caller_agent_when_trigger_scope_omits_agent_id() {
    // `TriggerRunThreadScope.agent_id` is `Option<AgentId>`.  When it is
    // `None` (e.g. the trigger record was stored without an explicit agent),
    // `check_automation_trigger_access` falls back to `bound_caller.agent_id`.
    // This test seeds the thread under the scope that fallback should produce
    // (caller's agent, trigger's project, creator's owner) and verifies that
    // the timeline resolves — proving the fallback actually runs.
    let caller = caller();
    let trigger_thread_id =
        ThreadId::new("thread-trigger-no-agent").expect("valid trigger thread id");

    // The thread is stored under the scope the fallback reconstructs:
    //   agent_id    = bound_caller.agent_id  (falls back from None)
    //   project_id  = trigger_scope.project_id
    //   owner_user_id = Some(creator_user_id)
    let fallback_scope = ThreadScope {
        tenant_id: caller.tenant_id.clone(),
        agent_id: caller.agent_id.clone().expect("test caller has agent"),
        project_id: caller.project_id.clone(),
        owner_user_id: Some(
            UserId::new(TRIGGER_CREATOR_USER_ID).expect("valid trigger creator user id"),
        ),
        mission_id: None,
    };
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    thread_service
        .ensure_thread(EnsureThreadRequest {
            scope: fallback_scope,
            thread_id: Some(trigger_thread_id.clone()),
            created_by_actor_id: "system".to_string(),
            title: Some("Agent-omitted trigger run".to_string()),
            metadata_json: Some(automation_trigger_thread_metadata_json(
                "trigger-no-agent-automation",
            )),
        })
        .await
        .expect("trigger thread stored");

    // The trigger scope has agent_id = None, exercising the fallback branch.
    let scope_with_no_agent = TriggerRunThreadScope {
        agent_id: None,
        project_id: caller.project_id.clone(),
        creator_user_id: UserId::new(TRIGGER_CREATOR_USER_ID)
            .expect("valid trigger creator user id"),
    };
    let automation_facade = Arc::new(
        StaticAutomationFacade::new(vec![])
            .with_resolve_scope_for_thread(trigger_thread_id.clone(), scope_with_no_agent),
    );

    let services = RebornServices::new(thread_service, Arc::new(FakeTurnCoordinator::default()))
        .with_automation_product_facade(automation_facade);

    let response = services
        .get_timeline(
            caller,
            RebornTimelineRequest::new(trigger_thread_id.as_str().to_string()),
        )
        .await
        .expect("timeline must resolve when agent_id is None via caller fallback");

    assert_eq!(
        response.thread.thread_id, trigger_thread_id,
        "fallback to caller agent_id must locate the trigger-owned thread"
    );
}

// Regression tests for the automation-trigger gate/approval interaction
// fallback.  Bug: `resolve_gate`, `cancel_run`, `get_run_state`, and
// `stream_events` all called `resolve_webui_thread_metadata` (user-scoped
// probe only) rather than `resolve_thread_access_for_caller` (user-scoped
// probe + automation fallback). Any gate-approval or auth-submit action on a
// trigger-fired thread therefore returned 404, even when the caller owned the
// automation that produced the thread.

fn automation_facade_with_trigger_thread(
    trigger_thread_id: ThreadId,
    caller: &WebUiAuthenticatedCaller,
) -> Arc<StaticAutomationFacade> {
    Arc::new(
        StaticAutomationFacade::new(vec![RebornAutomationInfo {
            automation_id: "trigger-gate-automation".to_string(),
            name: "Gate test automation".to_string(),
            source: RebornAutomationSource::Schedule {
                cron: "0 9 * * *".to_string(),
                timezone: "UTC".to_string(),
            },
            state: RebornAutomationState::Active,
            next_run_at: None,
            last_run_at: None,
            last_status: Some(RebornAutomationRunStatus::Ok),
            recent_runs: vec![RebornAutomationRecentRunInfo {
                run_id: Some(automation_run_id()),
                thread_id: Some(trigger_thread_id.clone()),
                fire_slot: None,
                status: RebornAutomationRecentRunStatus::Ok,
                submitted_at: "2026-06-10T09:00:01Z".parse().expect("submitted_at"),
                completed_at: None,
            }],
            is_active: true,
            created_at: None,
            active_hold: None,
        }])
        .with_resolve_scope_for_thread(
            trigger_thread_id.clone(),
            trigger_run_thread_scope_for(caller),
        ),
    )
}

/// Set up a trigger thread stored under the external creator's scope and
/// return the thread_id.  Mirrors `record_trigger_prompt` which sets
/// `owner_user_id = Some(creator_user_id)`.
async fn setup_trigger_thread(
    thread_service: &Arc<InMemorySessionThreadService>,
    caller: &WebUiAuthenticatedCaller,
    thread_id: &str,
) -> ThreadId {
    let tid = ThreadId::new(thread_id).expect("valid trigger thread id");
    thread_service
        .ensure_thread(EnsureThreadRequest {
            scope: trigger_thread_scope_for(caller),
            thread_id: Some(tid.clone()),
            created_by_actor_id: "system".to_string(),
            title: Some("Gate test trigger thread".to_string()),
            metadata_json: Some(automation_trigger_thread_metadata_json(
                "trigger-gate-automation",
            )),
        })
        .await
        .expect("trigger thread stored");
    tid
}

async fn setup_ownerless_trigger_thread(
    thread_service: &Arc<InMemorySessionThreadService>,
    caller: &WebUiAuthenticatedCaller,
    thread_id: &str,
) -> ThreadId {
    let tid = ThreadId::new(thread_id).expect("valid trigger thread id");
    let mut scope = trigger_thread_scope_for(caller);
    scope.owner_user_id = None;
    thread_service
        .ensure_thread(EnsureThreadRequest {
            scope,
            thread_id: Some(tid.clone()),
            created_by_actor_id: "system".to_string(),
            title: Some("Legacy trigger thread".to_string()),
            metadata_json: Some(automation_trigger_thread_metadata_json(
                "trigger-gate-automation",
            )),
        })
        .await
        .expect("legacy trigger thread stored");
    tid
}

#[tokio::test]
async fn resolve_gate_approval_succeeds_for_own_automation_trigger_thread() {
    // The caller owns the automation that produced the trigger thread. Approval
    // of a gate on that thread must succeed via the automation fallback.
    //
    // Post-#4754 ("Part A") verification: `check_automation_trigger_access`
    // must forward the trigger-owned `TurnScope` (with
    // `owner_user_id = Some(TRIGGER_CREATOR_USER_ID)`) — not the WebUI
    // caller's user_id — to the turn coordinator's `get_run_state` call.
    // The fake coordinator is configured to return `BlockedApproval` only
    // for any scope it receives; this assertion proves the coordinator
    // actually gets the trigger-owned scope, not the caller's session scope.
    let caller = caller();
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    let trigger_thread_id =
        setup_trigger_thread(&thread_service, &caller, "thread-trigger-gate-alpha").await;

    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let approval_interactions = Arc::new(RecordingApprovalInteractionService::default());
    // Program coordinator to report BlockedApproval with an approval gate.
    let gate_ref = approval_gate_ref(ApprovalRequestId::new()).expect("approval gate ref");
    coordinator.set_parked_approval_gate(gate_ref.clone());
    coordinator.set_run_state_actor(Some(turn_actor_for_user(TRIGGER_CREATOR_USER_ID)));

    let services = RebornServices::new(thread_service, coordinator.clone())
        .with_automation_product_facade(automation_facade_with_trigger_thread(
            trigger_thread_id.clone(),
            &caller,
        ))
        .with_approval_interactions(approval_interactions.clone());

    let response = services
        .resolve_gate(
            caller.clone(),
            serde_json::from_value::<WebUiResolveGateRequest>(json!({
                "client_action_id": "approval-trigger-1",
                "thread_id": trigger_thread_id.as_str(),
                "run_id": run_id_string(),
                "gate_ref": gate_ref.as_str(),
                "resolution": "approved"
            }))
            .expect("request"),
        )
        .await
        .expect("automation owner should be able to approve gate on trigger thread");

    assert!(
        matches!(response, RebornResolveGateResponse::Resumed(_)),
        "expected Resumed, got {response:?}"
    );
    assert_eq!(
        approval_interactions.resolution_count(),
        1,
        "approval interaction should have been called"
    );

    // Part A scope assertion: the coordinator must receive the trigger-owned
    // scope (owner = TRIGGER_CREATOR_USER_ID), not the WebUI caller's scope
    // (owner = "user-alpha"). This confirms `check_automation_trigger_access`
    // reconstructs the scope from `TriggerRunThreadScope.creator_user_id` and
    // that the reconstructed scope flows through to the turn coordinator.
    let expected_trigger_scope = TurnScope::new_with_owner(
        caller.tenant_id.clone(),
        caller.agent_id.clone(),
        caller.project_id.clone(),
        trigger_thread_id.clone(),
        Some(UserId::new(TRIGGER_CREATOR_USER_ID).expect("valid creator user id")),
    );
    assert_eq!(
        coordinator.last_run_state_scope(),
        Some(expected_trigger_scope),
        "get_run_state must receive the trigger-owned scope (owner = TRIGGER_CREATOR_USER_ID), \
         not the WebUI caller's session scope (owner = user-alpha)"
    );
    assert_eq!(
        approval_interactions
            .last_resolution()
            .expect("approval resolution")
            .actor
            .user_id,
        UserId::new(TRIGGER_CREATOR_USER_ID).expect("valid creator user id"),
        "approval resolution must resume the run as the trigger creator, not the WebUI caller"
    );
}

#[tokio::test]
async fn cancel_run_succeeds_for_own_automation_trigger_thread() {
    // The caller owns the automation, but the run itself belongs to the trigger
    // creator. cancel_run must forward both the trigger-owned scope and the run
    // actor to the turn coordinator.
    let caller = caller();
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    let trigger_thread_id =
        setup_trigger_thread(&thread_service, &caller, "thread-trigger-cancel-alpha").await;
    let coordinator = Arc::new(FakeTurnCoordinator::default());

    let services =
        RebornServices::new(thread_service, coordinator.clone()).with_automation_product_facade(
            automation_facade_with_trigger_thread(trigger_thread_id.clone(), &caller),
        );

    let response = services
        .cancel_run(
            caller.clone(),
            serde_json::from_value::<WebUiCancelRunRequest>(json!({
                "client_action_id": "cancel-trigger-1",
                "thread_id": trigger_thread_id.as_str(),
                "run_id": run_id_string(),
                "reason": "user_requested"
            }))
            .expect("request"),
        )
        .await
        .expect("automation owner should be able to cancel trigger thread run");

    assert_eq!(response.status, TurnStatus::Cancelled);
    let expected_trigger_scope = TurnScope::new_with_owner(
        caller.tenant_id.clone(),
        caller.agent_id.clone(),
        caller.project_id.clone(),
        trigger_thread_id,
        Some(UserId::new(TRIGGER_CREATOR_USER_ID).expect("valid creator user id")),
    );
    assert_eq!(
        coordinator.last_cancellation_scope(),
        Some(expected_trigger_scope),
        "cancel_run must receive the trigger-owned scope"
    );
    assert_eq!(
        coordinator
            .last_cancellation_actor()
            .expect("cancel actor")
            .user_id,
        UserId::new(TRIGGER_CREATOR_USER_ID).expect("valid creator user id"),
        "cancel_run must use the trigger creator actor, not the WebUI caller"
    );
}

#[tokio::test]
async fn get_run_state_succeeds_for_own_automation_trigger_thread() {
    // get_run_state is read-only, but it still must resolve the browser thread
    // id to the trigger-owned scope before querying the coordinator.
    let caller = caller();
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    let trigger_thread_id =
        setup_trigger_thread(&thread_service, &caller, "thread-trigger-state-alpha").await;
    let coordinator = Arc::new(FakeTurnCoordinator::default());

    let services =
        RebornServices::new(thread_service, coordinator.clone()).with_automation_product_facade(
            automation_facade_with_trigger_thread(trigger_thread_id.clone(), &caller),
        );

    let response = services
        .get_run_state(
            caller.clone(),
            RebornGetRunStateRequest {
                thread_id: trigger_thread_id.as_str().to_string(),
                run_id: run_id_string(),
            },
        )
        .await
        .expect("automation owner should be able to read trigger run state");

    assert_eq!(response.status, TurnStatus::Queued);
    let expected_trigger_scope = TurnScope::new_with_owner(
        caller.tenant_id.clone(),
        caller.agent_id.clone(),
        caller.project_id.clone(),
        trigger_thread_id,
        Some(UserId::new(TRIGGER_CREATOR_USER_ID).expect("valid creator user id")),
    );
    assert_eq!(
        coordinator.last_run_state_scope(),
        Some(expected_trigger_scope),
        "get_run_state must query the trigger-owned scope"
    );
}

#[tokio::test]
async fn resolve_gate_rejects_other_users_automation_trigger_thread() {
    // Alice owns the trigger thread. Bob should get 404, not a gate resolution.
    let alice = caller_for_user("user-alice");
    let bob = caller_for_user("user-bob");
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    let trigger_thread_id =
        setup_trigger_thread(&thread_service, &alice, "thread-trigger-gate-beta").await;

    // Bob has no automations — resolve_run_thread_scope returns None, fallback denies him.
    let bob_automation_facade = Arc::new(StaticAutomationFacade::new(Vec::new()));
    let approval_interactions = Arc::new(RecordingApprovalInteractionService::default());
    let gate_ref = approval_gate_ref(ApprovalRequestId::new()).expect("approval gate ref");

    let services = RebornServices::new(thread_service, Arc::new(FakeTurnCoordinator::default()))
        .with_automation_product_facade(bob_automation_facade)
        .with_approval_interactions(approval_interactions.clone());

    let err = services
        .resolve_gate(
            bob,
            serde_json::from_value::<WebUiResolveGateRequest>(json!({
                "client_action_id": "approval-trigger-rejected",
                "thread_id": trigger_thread_id.as_str(),
                "run_id": run_id_string(),
                "gate_ref": gate_ref.as_str(),
                "resolution": "approved"
            }))
            .expect("request"),
        )
        .await
        .expect_err("non-owner must not resolve gate on another user's trigger thread");

    assert_eq!(err.code, RebornServicesErrorCode::NotFound);
    assert_eq!(err.status_code, 404);
    assert_eq!(
        approval_interactions.resolution_count(),
        0,
        "approval interaction must not be called for unauthorized caller"
    );
}

// Regression: stream_events used the WebUI caller's user_id as the projection
// identity even after resolve_thread_access_for_caller succeeded via the
// automation fallback. For a trigger-fired thread the run events are keyed
// under the trigger creator's user_id, not the WebUI caller's; passing the
// caller's id caused the turn-event replay filter (owner_user_id) and the
// runtime event stream key (EventStreamKey) to select the wrong bucket —
// approval-gate events were invisible to the chat page.
//
// The fix: after authorization succeeds, derive the projection identity from
// scope.explicit_owner_user_id() (the creator for trigger threads; falls back
// to caller for normal session threads where thread_owner = ActorFallback).
#[tokio::test]
async fn stream_events_uses_trigger_creator_as_projection_identity() {
    // The caller ("user-alpha") owns the automation. The trigger thread was
    // stored under the external creator's scope ("user-trigger-creator").
    // stream_events must pass the CREATOR's identity to the projection drain,
    // not the caller's, so the correct event stream bucket is selected.
    let caller = caller();
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    let trigger_thread_id =
        setup_trigger_thread(&thread_service, &caller, "thread-trigger-stream-alpha").await;

    let event_stream = Arc::new(RecordingProjectionStream::default());
    let services = RebornServices::new(thread_service, Arc::new(FakeTurnCoordinator::default()))
        .with_automation_product_facade(automation_facade_with_trigger_thread(
            trigger_thread_id.clone(),
            &caller,
        ))
        .with_event_stream(event_stream.clone());

    services
        .stream_events(
            caller.clone(),
            RebornStreamEventsRequest {
                thread_id: trigger_thread_id.as_str().to_string(),
                after_cursor: None,
            },
        )
        .await
        .expect("automation owner should be able to stream trigger thread events");

    // The projection drain must have been called with the trigger CREATOR's
    // user_id, not the WebUI caller's user_id. Events are owned by the
    // run's submitting identity (the creator); using the caller's id
    // filters to the wrong stream/event bucket.
    let requests = event_stream.requests();
    assert_eq!(
        requests.len(),
        1,
        "projection drain must be called exactly once"
    );
    assert_eq!(
        requests[0].actor.user_id,
        UserId::new(TRIGGER_CREATOR_USER_ID).expect("valid creator user id"),
        "projection actor must be the trigger creator (owner of the run events), \
         not the WebUI caller (who proved visibility via automation ownership)"
    );
    // The scope must still carry the thread_id correctly.
    assert_eq!(
        requests[0].scope.thread_id, trigger_thread_id,
        "projection scope thread_id must match the trigger thread"
    );
}

#[tokio::test]
async fn stream_events_revalidates_facade_on_every_poll() {
    // Every stream_events poll must call resolve_run_thread_scope — there is no
    // authorization cache. This ensures a caller that loses automation
    // visibility between polls cannot keep draining the trigger-owned stream.
    let caller = caller();
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    let trigger_thread_id = setup_trigger_thread(
        &thread_service,
        &caller,
        "thread-trigger-stream-revalidate-alpha",
    )
    .await;

    let automation_facade =
        automation_facade_with_trigger_thread(trigger_thread_id.clone(), &caller);
    let event_stream = Arc::new(RecordingProjectionStream::default());
    let services = RebornServices::new(thread_service, Arc::new(FakeTurnCoordinator::default()))
        .with_automation_product_facade(automation_facade.clone())
        .with_event_stream(event_stream.clone());

    for _ in 0..3 {
        services
            .stream_events(
                caller.clone(),
                RebornStreamEventsRequest {
                    thread_id: trigger_thread_id.as_str().to_string(),
                    after_cursor: None,
                },
            )
            .await
            .expect("automation owner should be able to repeatedly stream trigger events");
    }

    assert_eq!(
        automation_facade.resolve_calls(),
        vec![
            trigger_thread_id.clone(),
            trigger_thread_id.clone(),
            trigger_thread_id.clone()
        ],
        "every stream_events poll must call resolve_run_thread_scope (no authz caching)"
    );
    assert_eq!(
        event_stream.requests().len(),
        3,
        "event polling must not be suppressed"
    );
}

#[tokio::test]
async fn stream_events_fails_when_visibility_revoked_between_polls() {
    // If the caller's automation visibility is revoked between polls,
    // the next poll must fail with not_found — the authz result is not cached.
    let caller = caller();
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    let trigger_thread_id = setup_trigger_thread(
        &thread_service,
        &caller,
        "thread-trigger-stream-revoke-alpha",
    )
    .await;

    // A facade that starts with the scope available but can revoke it.
    let revocable_facade = Arc::new(RevocableAutomationFacade::new(
        trigger_thread_id.clone(),
        &caller,
    ));
    let event_stream = Arc::new(RecordingProjectionStream::default());
    let services = RebornServices::new(thread_service, Arc::new(FakeTurnCoordinator::default()))
        .with_automation_product_facade(revocable_facade.clone())
        .with_event_stream(event_stream.clone());

    // First poll succeeds — caller still has automation visibility.
    services
        .stream_events(
            caller.clone(),
            RebornStreamEventsRequest {
                thread_id: trigger_thread_id.as_str().to_string(),
                after_cursor: None,
            },
        )
        .await
        .expect("first poll must succeed while scope is visible");

    // Revoke visibility.
    revocable_facade.revoke();

    // Second poll must fail — visibility was revoked and there is no cached authz.
    let err = services
        .stream_events(
            caller.clone(),
            RebornStreamEventsRequest {
                thread_id: trigger_thread_id.as_str().to_string(),
                after_cursor: None,
            },
        )
        .await
        .expect_err("second poll must fail after visibility is revoked");

    assert_eq!(
        err.code,
        RebornServicesErrorCode::NotFound,
        "revoked visibility must surface as not_found, not a stale cached grant"
    );
    assert_eq!(err.status_code, 404);
}

#[tokio::test]
async fn get_timeline_rejects_thread_id_absent_from_callers_automations() {
    // The thread_id does not appear in the caller's automation run history at
    // all — `resolve_run_thread_scope` returns `None`.  The service must return
    // 404 and must NOT fall back to guessing the thread scope.
    let caller = caller();
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    // No threads stored anywhere.

    // Automation facade knows about a DIFFERENT thread, not the requested one.
    let unrelated_thread_id =
        ThreadId::new("thread-unrelated-xyz").expect("valid unrelated thread id");
    let automation_facade = Arc::new(
        StaticAutomationFacade::new(vec![RebornAutomationInfo {
            automation_id: "trigger-other".to_string(),
            name: "Other automation".to_string(),
            source: RebornAutomationSource::Schedule {
                cron: "0 12 * * *".to_string(),
                timezone: "UTC".to_string(),
            },
            state: RebornAutomationState::Active,
            next_run_at: None,
            last_run_at: None,
            last_status: None,
            recent_runs: vec![RebornAutomationRecentRunInfo {
                run_id: Some(automation_run_id()),
                thread_id: Some(unrelated_thread_id),
                fire_slot: None,
                status: RebornAutomationRecentRunStatus::Ok,
                submitted_at: "2026-06-10T12:00:00Z".parse().expect("submitted_at"),
                completed_at: Some("2026-06-10T12:01:00Z".parse().expect("completed_at")),
            }],
            is_active: true,
            created_at: None,
            active_hold: None,
        }]), // resolve_scope is None — the facade does not recognise the requested thread.
    );

    let services = RebornServices::new(thread_service, Arc::new(FakeTurnCoordinator::default()))
        .with_automation_product_facade(automation_facade);

    let err = services
        .get_timeline(
            caller,
            RebornTimelineRequest::new("thread-absent-from-automations".to_string()),
        )
        .await
        .expect_err("unknown thread_id must return 404");

    assert_eq!(err.code, RebornServicesErrorCode::NotFound);
    assert_eq!(err.status_code, 404);
}

#[tokio::test]
async fn list_automations_returns_empty_list() {
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_automation_product_facade(Arc::new(StaticAutomationFacade::new(Vec::new())));

    let listed = query_automations(&services, caller(), WebUiListAutomationsRequest::default())
        .await
        .expect("list automations");

    assert!(listed.automations.is_empty());
    // Default facade reports the scheduler as running.
    assert!(listed.scheduler_enabled);
}

#[tokio::test]
async fn list_automations_surfaces_disabled_scheduler() {
    // Regression: when the trigger poller is off, the response must report
    // scheduler_enabled=false so the browser can warn that listed automations
    // will not fire. Previously the wire response had no such signal.
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_automation_product_facade(Arc::new(
        StaticAutomationFacade::new(Vec::new()).with_scheduler_enabled(false),
    ));

    let listed = query_automations(&services, caller(), WebUiListAutomationsRequest::default())
        .await
        .expect("list automations");

    assert!(!listed.scheduler_enabled);
}

#[tokio::test]
async fn automation_facade_unwired_fails_closed() {
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    );

    let error = query_automations(&services, caller(), WebUiListAutomationsRequest::default())
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

    let response = query_extension_setup(&services, caller(), "github")
        .await
        .expect("setup extension response");

    let onboarding = response.onboarding.as_ref().expect("onboarding payload");
    assert_eq!(response.phase, InstallationState::Configured);
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

    let err = invoke_extension_setup_submit(
        &services,
        caller(),
        "github",
        WebUiSetupExtensionRequest {
            client_action_id: None,
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

    let err = invoke_extension_setup_submit(
        &services,
        caller(),
        "github",
        WebUiSetupExtensionRequest {
            client_action_id: None,
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

    let err = invoke_extension_setup_submit(
        &services,
        caller(),
        "google",
        WebUiSetupExtensionRequest {
            client_action_id: None,
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

/// One recorded configure-port save: the target extension id plus the
/// submitted `(handle, value)` pairs.
type RecordedChannelConfigSave = (String, Vec<(String, String)>);

/// Recording fake of the channel-config configure port: serves a fixed
/// field-status projection and records every save.
#[derive(Default)]
struct RecordingChannelConfigFacade {
    fields: Vec<RebornChannelConfigField>,
    saves: Mutex<Vec<RecordedChannelConfigSave>>,
}

impl RecordingChannelConfigFacade {
    fn with_fields(fields: Vec<RebornChannelConfigField>) -> Self {
        Self {
            fields,
            saves: Mutex::new(Vec::new()),
        }
    }

    fn saves(&self) -> Vec<RecordedChannelConfigSave> {
        self.saves.lock().expect("saves lock").clone()
    }
}

#[async_trait]
impl ChannelConfigFacade for RecordingChannelConfigFacade {
    async fn field_status(
        &self,
        _extension_id: &ExtensionId,
    ) -> Result<Vec<RebornChannelConfigField>, RebornServicesError> {
        Ok(self.fields.clone())
    }

    async fn save_values(
        &self,
        extension_id: &ExtensionId,
        values: Vec<(String, String)>,
    ) -> Result<(), RebornServicesError> {
        self.saves
            .lock()
            .expect("saves lock")
            .push((extension_id.as_str().to_string(), values));
        Ok(())
    }
}

fn channel_config_field(name: &str, label: &str, secret: bool) -> RebornChannelConfigField {
    RebornChannelConfigField {
        name: name.to_string(),
        label: label.to_string(),
        secret,
        provided: false,
    }
}

/// The setup facade renders manifest-declared channel-config fields (the
/// non-secret descriptors in `fields`, the secret ones in the existing
/// `secrets` shape, presence only) and routes submitted values to the
/// configure port while credential secrets keep the credential path.
#[tokio::test]
async fn setup_extension_projects_and_routes_channel_config_values() {
    let credentials = Arc::new(RecordingExtensionCredentialSetupService::default());
    let channel_config = Arc::new(RecordingChannelConfigFacade::with_fields(vec![
        channel_config_field("bot_token", "Bot token", true),
        channel_config_field("public_url", "Public webhook URL", false),
    ]));
    let services =
        setup_services_with_requirements(vec![manual_credential_requirement("api_token", false)])
            .with_extension_credentials(credentials.clone())
            .with_channel_config_facade(channel_config.clone());

    // View: fields from the non-secret descriptors, secret channel fields in
    // the secrets list (presence only, manual-token shape).
    let view = query_extension_setup(&services, caller(), "github")
        .await
        .expect("setup view");
    assert_eq!(view.fields.len(), 1);
    assert_eq!(view.fields[0].name, "public_url");
    assert_eq!(view.fields[0].prompt, "Public webhook URL");
    assert!(view.fields[0].placeholder.is_none());
    let bot_token = view
        .secrets
        .iter()
        .find(|secret| secret.name == "bot_token")
        .expect("secret channel field surfaces in the secrets shape");
    assert!(!bot_token.provided);
    assert!(
        view.secrets.iter().any(|secret| secret.name == "api_token"),
        "credential requirements keep their own entry"
    );

    // Submit: channel values route to the configure port; the credential
    // secret stays on the credential path.
    let response = submit_extension_setup_and_query(
        &services,
        caller(),
        "github",
        WebUiSetupExtensionRequest {
            client_action_id: None,
            action: Some("submit".to_string()),
            payload: Some(json!({
                "secrets": {
                    "bot_token": "xbt-123",
                    "api_token": "cred-456"
                },
                "fields": {
                    "public_url": "https://hooks.example.test/updates"
                }
            })),
        },
    )
    .await
    .expect("setup submit");
    assert_eq!(response.fields.len(), 1);
    let saves = channel_config.saves();
    assert_eq!(saves.len(), 1);
    assert_eq!(saves[0].0, "github");
    assert!(
        saves[0]
            .1
            .contains(&("bot_token".to_string(), "xbt-123".to_string()))
    );
    assert!(saves[0].1.contains(&(
        "public_url".to_string(),
        "https://hooks.example.test/updates".to_string()
    )));
    assert!(
        !saves[0].1.iter().any(|(name, _)| name == "api_token"),
        "credential secrets must not leak into the channel-config port"
    );
    assert_eq!(
        credentials.submit_count(),
        1,
        "the credential secret still reaches the credential path"
    );
}

/// A submitted `fields` value that matches no declared non-secret handle is
/// rejected before anything is stored.
#[tokio::test]
async fn setup_extension_rejects_unknown_channel_config_field() {
    let credentials = Arc::new(RecordingExtensionCredentialSetupService::default());
    let channel_config = Arc::new(RecordingChannelConfigFacade::with_fields(vec![
        channel_config_field("public_url", "Public webhook URL", false),
    ]));
    let services = setup_services_with_requirements(Vec::new())
        .with_extension_credentials(credentials.clone())
        .with_channel_config_facade(channel_config.clone());

    let err = invoke_extension_setup_submit(
        &services,
        caller(),
        "github",
        WebUiSetupExtensionRequest {
            client_action_id: None,
            action: Some("submit".to_string()),
            payload: Some(json!({
                "fields": {
                    "unknown_field": "value"
                }
            })),
        },
    )
    .await
    .expect_err("unknown field handle is rejected");

    assert_setup_validation(err, "fields", WebUiInboundValidationCode::InvalidValue);
    assert!(channel_config.saves().is_empty());
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

#[derive(Debug, PartialEq, Eq)]
struct SetupUpsertCall {
    id: String,
    adapter: String,
    base_url: Option<String>,
    default_model: Option<String>,
    api_key_set: bool,
    set_active: bool,
    model: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
struct SetupSetActiveCall {
    provider_id: String,
    model: Option<String>,
}

struct SetupRecordingLlmConfigService {
    snapshot_calls: Mutex<usize>,
    snapshot_callers: Mutex<Vec<WebUiAuthenticatedCaller>>,
    upsert_provider_calls: Mutex<Vec<SetupUpsertCall>>,
    delete_provider_calls: Mutex<Vec<String>>,
    set_active_calls: Mutex<Vec<SetupSetActiveCall>>,
    test_connection_calls: Mutex<usize>,
    list_models_calls: Mutex<usize>,
    snapshot: Mutex<LlmConfigSnapshot>,
    next_snapshot_error: Mutex<Option<LlmConfigServiceError>>,
    next_upsert_error: Mutex<Option<LlmConfigServiceError>>,
    next_set_active_error: Mutex<Option<LlmConfigServiceError>>,
}

impl Default for SetupRecordingLlmConfigService {
    fn default() -> Self {
        Self {
            snapshot_calls: Mutex::new(0),
            snapshot_callers: Mutex::new(Vec::new()),
            upsert_provider_calls: Mutex::new(Vec::new()),
            delete_provider_calls: Mutex::new(Vec::new()),
            set_active_calls: Mutex::new(Vec::new()),
            test_connection_calls: Mutex::new(0),
            list_models_calls: Mutex::new(0),
            snapshot: Mutex::new(Self::empty_snapshot()),
            next_snapshot_error: Mutex::new(None),
            next_upsert_error: Mutex::new(None),
            next_set_active_error: Mutex::new(None),
        }
    }
}

impl SetupRecordingLlmConfigService {
    fn snapshot_count(&self) -> usize {
        *self.snapshot_calls.lock().expect("lock")
    }

    fn snapshot_callers(&self) -> Vec<WebUiAuthenticatedCaller> {
        self.snapshot_callers.lock().expect("lock").clone()
    }

    fn upsert_provider_count(&self) -> usize {
        self.upsert_provider_calls.lock().expect("lock").len()
    }

    fn set_active_count(&self) -> usize {
        self.set_active_calls.lock().expect("lock").len()
    }

    fn test_connection_count(&self) -> usize {
        *self.test_connection_calls.lock().expect("lock")
    }

    fn list_models_count(&self) -> usize {
        *self.list_models_calls.lock().expect("lock")
    }

    fn use_active_snapshot(&self, provider_id: &str, model: &str) {
        *self.snapshot.lock().expect("lock") = Self::active_snapshot(provider_id, model);
    }

    fn fail_next_snapshot(&self, error: LlmConfigServiceError) {
        *self.next_snapshot_error.lock().expect("lock") = Some(error);
    }

    fn fail_next_upsert(&self, error: LlmConfigServiceError) {
        *self.next_upsert_error.lock().expect("lock") = Some(error);
    }

    fn fail_next_set_active(&self, error: LlmConfigServiceError) {
        *self.next_set_active_error.lock().expect("lock") = Some(error);
    }

    fn empty_snapshot() -> LlmConfigSnapshot {
        LlmConfigSnapshot {
            providers: Vec::new(),
            active: None,
        }
    }

    fn active_snapshot(provider_id: &str, model: &str) -> LlmConfigSnapshot {
        LlmConfigSnapshot {
            providers: vec![LlmProviderView {
                id: provider_id.to_string(),
                description: "configured provider".to_string(),
                adapter: "open_ai_completions".to_string(),
                default_model: model.to_string(),
                base_url: Some("https://api.example.test/v1".to_string()),
                builtin: false,
                active: true,
                active_model: Some(model.to_string()),
                api_key_required: true,
                accepts_api_key: true,
                api_key_set: true,
                can_list_models: true,
            }],
            active: Some(LlmActiveSelection {
                provider_id: provider_id.to_string(),
                model: Some(model.to_string()),
            }),
        }
    }
}

#[async_trait]
impl LlmConfigService for SetupRecordingLlmConfigService {
    async fn snapshot(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<LlmConfigSnapshot, LlmConfigServiceError> {
        *self.snapshot_calls.lock().expect("lock") += 1;
        self.snapshot_callers.lock().expect("lock").push(caller);
        if let Some(error) = self.next_snapshot_error.lock().expect("lock").take() {
            return Err(error);
        }
        Ok(self.snapshot.lock().expect("lock").clone())
    }

    async fn upsert_provider(
        &self,
        _caller: WebUiAuthenticatedCaller,
        request: UpsertLlmProviderRequest,
    ) -> Result<LlmConfigSnapshot, LlmConfigServiceError> {
        if let Some(error) = self.next_upsert_error.lock().expect("lock").take() {
            return Err(error);
        }
        self.upsert_provider_calls
            .lock()
            .expect("lock")
            .push(SetupUpsertCall {
                id: request.id,
                adapter: request.adapter,
                base_url: request.base_url,
                default_model: request.default_model,
                api_key_set: request.api_key.is_some(),
                set_active: request.set_active,
                model: request.model,
            });
        Ok(self.snapshot.lock().expect("lock").clone())
    }

    async fn delete_provider(
        &self,
        _caller: WebUiAuthenticatedCaller,
        provider_id: String,
    ) -> Result<LlmConfigSnapshot, LlmConfigServiceError> {
        self.delete_provider_calls
            .lock()
            .expect("lock")
            .push(provider_id);
        Ok(self.snapshot.lock().expect("lock").clone())
    }

    async fn set_active(
        &self,
        _caller: WebUiAuthenticatedCaller,
        request: SetActiveLlmRequest,
    ) -> Result<LlmConfigSnapshot, LlmConfigServiceError> {
        if let Some(error) = self.next_set_active_error.lock().expect("lock").take() {
            return Err(error);
        }
        self.set_active_calls
            .lock()
            .expect("lock")
            .push(SetupSetActiveCall {
                provider_id: request.provider_id,
                model: request.model,
            });
        Ok(self.snapshot.lock().expect("lock").clone())
    }

    async fn test_connection(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: LlmProbeRequest,
    ) -> Result<LlmProbeResult, LlmConfigServiceError> {
        *self.test_connection_calls.lock().expect("lock") += 1;
        Ok(LlmProbeResult {
            ok: true,
            message: "ok".to_string(),
        })
    }

    async fn list_models(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: LlmProbeRequest,
    ) -> Result<LlmModelsResult, LlmConfigServiceError> {
        *self.list_models_calls.lock().expect("lock") += 1;
        Ok(LlmModelsResult {
            ok: true,
            models: vec!["model-a".to_string()],
            message: String::new(),
        })
    }

    async fn start_nearai_login(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: NearAiLoginRequest,
    ) -> Result<NearAiLoginStart, LlmConfigServiceError> {
        panic!("start_nearai_login is not used by operator setup tests")
    }

    async fn complete_nearai_wallet_login(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: NearAiWalletLoginRequest,
    ) -> Result<NearAiWalletLoginResult, LlmConfigServiceError> {
        panic!("complete_nearai_wallet_login is not used by operator setup tests")
    }

    async fn start_codex_login(
        &self,
        _caller: WebUiAuthenticatedCaller,
    ) -> Result<CodexLoginStart, LlmConfigServiceError> {
        panic!("start_codex_login is not used by operator setup tests")
    }
}

struct RecordingOperatorStatusService {
    response: RebornOperatorStatusResponse,
    callers: Mutex<Vec<WebUiAuthenticatedCaller>>,
}

impl RecordingOperatorStatusService {
    fn new(response: RebornOperatorStatusResponse) -> Self {
        Self {
            response,
            callers: Mutex::new(Vec::new()),
        }
    }

    fn callers(&self) -> Vec<WebUiAuthenticatedCaller> {
        self.callers.lock().expect("lock").clone()
    }
}

#[async_trait]
impl OperatorStatusService for RecordingOperatorStatusService {
    async fn status(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornOperatorStatusResponse, RebornServicesError> {
        self.callers.lock().expect("lock").push(caller);
        Ok(self.response.clone())
    }
}

fn services_with_setup_llm_config(
    llm_config: Arc<SetupRecordingLlmConfigService>,
) -> RebornServices {
    RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_llm_config_service(llm_config)
}

#[derive(Clone)]
struct StaticOperatorToolCatalogForTest {
    tools: Vec<RebornOperatorToolInfo>,
}

#[async_trait]
impl RebornOperatorToolCatalog for StaticOperatorToolCatalogForTest {
    async fn list_operator_tools(
        &self,
        _caller: &ironclaw_host_api::UserId,
    ) -> Vec<RebornOperatorToolInfo> {
        // Ownership filtering is exercised by the composition-tier catalog
        // test; this static catalog is caller-agnostic on purpose.
        self.tools.clone()
    }
}

struct FailingAllowPersistentApprovalPolicyStore;

#[async_trait]
impl PersistentApprovalPolicyStore for FailingAllowPersistentApprovalPolicyStore {
    async fn allow(
        &self,
        _input: PersistentApprovalPolicyInput,
    ) -> Result<PersistentApprovalPolicy, PersistentApprovalPolicyError> {
        Err(PersistentApprovalPolicyError::Filesystem(
            "persistent policy store unavailable".to_string(),
        ))
    }

    async fn lookup(
        &self,
        _key: &PersistentApprovalPolicyKey,
    ) -> Result<Option<PersistentApprovalPolicy>, PersistentApprovalPolicyError> {
        Ok(None)
    }

    async fn revoke(
        &self,
        _key: &PersistentApprovalPolicyKey,
    ) -> Result<PersistentApprovalPolicy, PersistentApprovalPolicyError> {
        Err(PersistentApprovalPolicyError::UnknownPolicy)
    }

    async fn revoke_if_source_approval_request(
        &self,
        _key: &PersistentApprovalPolicyKey,
        _source_approval_request_id: ApprovalRequestId,
    ) -> Result<Option<PersistentApprovalPolicy>, PersistentApprovalPolicyError> {
        Ok(None)
    }
}

#[derive(Debug, Default)]
struct RecordingAutoApproveSettingStore {
    get_keys: Mutex<Vec<AutoApproveSettingKey>>,
}

impl RecordingAutoApproveSettingStore {
    fn get_keys(&self) -> Vec<AutoApproveSettingKey> {
        self.get_keys.lock().expect("lock").clone()
    }
}

#[async_trait]
impl AutoApproveSettingStore for RecordingAutoApproveSettingStore {
    async fn set(
        &self,
        input: AutoApproveSettingInput,
    ) -> Result<AutoApproveSettingRecord, CapabilityPermissionStoreError> {
        let key = AutoApproveSettingKey::from_resource_scope(&input.scope);
        let now = Utc::now();
        Ok(AutoApproveSettingRecord {
            key,
            enabled: input.enabled,
            updated_by: input.updated_by,
            created_at: now,
            updated_at: now,
        })
    }

    async fn get(
        &self,
        key: &AutoApproveSettingKey,
    ) -> Result<Option<AutoApproveSettingRecord>, CapabilityPermissionStoreError> {
        self.get_keys.lock().expect("lock").push(key.clone());
        Ok(None)
    }
}

type OperatorConfigServices = RebornServices<OperatorConfigAutoApproveInvoker>;

#[derive(Clone)]
struct OperatorConfigAutoApproveInvoker {
    auto_approve: Arc<dyn AutoApproveSettingStore>,
    overrides: Arc<dyn ToolPermissionOverrideStore>,
    persistent_policies: Arc<dyn PersistentApprovalPolicyStore>,
    tools: Arc<Vec<RebornOperatorToolInfo>>,
}

#[async_trait]
impl ProductCapabilityInvoker for OperatorConfigAutoApproveInvoker {
    async fn invoke(
        &self,
        caller: WebUiAuthenticatedCaller,
        capability: CapabilityId,
        input: serde_json::Value,
        activity_id: ActivityId,
    ) -> Result<Resolution, RebornServicesError> {
        match capability.as_str() {
            OPERATOR_CONFIG_SET_AUTO_APPROVE_CAPABILITY_ID => {
                self.invoke_auto_approve(caller, input, activity_id).await
            }
            OPERATOR_CONFIG_SET_TOOL_PERMISSION_CAPABILITY_ID => {
                self.invoke_tool_permission(caller, input, activity_id)
                    .await
            }
            capability => panic!("unexpected operator config capability {capability}"),
        }
    }
}

impl OperatorConfigAutoApproveInvoker {
    async fn invoke_auto_approve(
        &self,
        caller: WebUiAuthenticatedCaller,
        input: serde_json::Value,
        activity_id: ActivityId,
    ) -> Result<Resolution, RebornServicesError> {
        let enabled = input
            .get("enabled")
            .and_then(serde_json::Value::as_bool)
            .expect("auto-approve capability input must carry enabled bool");
        let scope = ResourceScope {
            tenant_id: caller.tenant_id.clone(),
            user_id: caller.user_id.clone(),
            agent_id: caller.agent_id.clone(),
            project_id: caller.project_id.clone(),
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::from_uuid(activity_id.as_uuid()),
        }
        .tenant_user_settings_scope();
        self.auto_approve
            .set(AutoApproveSettingInput {
                scope,
                enabled,
                updated_by: Principal::User(caller.user_id.clone()),
            })
            .await
            .map_err(RebornServicesError::internal_from)?;
        Ok(operator_config_success_resolution(activity_id))
    }

    async fn invoke_tool_permission(
        &self,
        caller: WebUiAuthenticatedCaller,
        input: serde_json::Value,
        activity_id: ActivityId,
    ) -> Result<Resolution, RebornServicesError> {
        let capability_id = input
            .get("capability_id")
            .and_then(serde_json::Value::as_str)
            .expect("tool-permission capability input must carry capability_id");
        let state = input
            .get("state")
            .and_then(serde_json::Value::as_str)
            .expect("tool-permission capability input must carry state");
        let tool_capability_id = CapabilityId::new(capability_id).expect("capability id");
        let tool = self
            .tools
            .iter()
            .find(|tool| tool.capability_id == tool_capability_id)
            .unwrap_or_else(|| panic!("operator tool {capability_id}"));
        let scope = ResourceScope {
            tenant_id: caller.tenant_id.clone(),
            user_id: caller.user_id.clone(),
            agent_id: caller.agent_id.clone(),
            project_id: caller.project_id.clone(),
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::from_uuid(activity_id.as_uuid()),
        }
        .tenant_user_settings_scope();
        let override_key = ToolPermissionOverrideKey::new(&scope, tool.capability_id.clone());
        let policy_key = PersistentApprovalPolicyKey::new(
            &scope,
            PersistentApprovalAction::Dispatch,
            tool.capability_id.clone(),
            Principal::Extension(tool.provider.clone()),
        );

        match state {
            "default" => {
                self.revoke_tool_policy(&policy_key).await?;
                self.overrides
                    .clear(&override_key)
                    .await
                    .map_err(RebornServicesError::internal_from)?;
            }
            "always_allow" => {
                if tool.default_permission == PermissionMode::Deny
                    || tool
                        .effects
                        .iter()
                        .any(|effect| matches!(effect, EffectKind::Financial))
                {
                    return Err(RebornServicesError::from(WebUiInboundValidationError::new(
                        "state",
                        WebUiInboundValidationCode::InvalidValue,
                    )));
                }
                self.persistent_policies
                    .allow(PersistentApprovalPolicyInput {
                        scope: scope.clone(),
                        action: PersistentApprovalAction::Dispatch,
                        capability_id: tool.capability_id.clone(),
                        grantee: Principal::Extension(tool.provider.clone()),
                        approved_by: Principal::User(caller.user_id.clone()),
                        constraints: ironclaw_host_api::GrantConstraints {
                            allowed_effects: tool.effects.as_ref().to_vec(),
                            mounts: Default::default(),
                            network: Default::default(),
                            secrets: Vec::new(),
                            resource_ceiling: None,
                            expires_at: None,
                            max_invocations: None,
                        },
                        source_approval_request_id: None,
                    })
                    .await
                    .map_err(RebornServicesError::internal_from)?;
                self.overrides
                    .clear(&override_key)
                    .await
                    .map_err(RebornServicesError::internal_from)?;
            }
            "ask_each_time" | "disabled" => {
                self.revoke_tool_policy(&policy_key).await?;
                let state = match state {
                    "ask_each_time" => ToolPermissionOverride::AskEachTime,
                    "disabled" => ToolPermissionOverride::Disabled,
                    _ => unreachable!("state matched above"),
                };
                self.overrides
                    .set(ToolPermissionOverrideInput {
                        scope,
                        capability_id: tool.capability_id.clone(),
                        state,
                        updated_by: Principal::User(caller.user_id.clone()),
                    })
                    .await
                    .map_err(RebornServicesError::internal_from)?;
            }
            state => panic!("unexpected tool-permission state {state}"),
        }

        Ok(operator_config_success_resolution(activity_id))
    }

    async fn revoke_tool_policy(
        &self,
        policy_key: &PersistentApprovalPolicyKey,
    ) -> Result<(), RebornServicesError> {
        match self.persistent_policies.revoke(policy_key).await {
            Ok(_) | Err(PersistentApprovalPolicyError::UnknownPolicy) => Ok(()),
            Err(error) => Err(RebornServicesError::internal_from(error)),
        }
    }
}

fn operator_config_success_resolution(activity_id: ActivityId) -> Resolution {
    Resolution::Done(Outcome {
        refs: OutcomeRefs {
            result: ResultRef::from_uuid(activity_id.as_uuid()),
            byte_len: 0,
            preview: None,
            preview_meta: ResultPreviewMeta::default(),
            origin: None,
            output_digest: None,
        },
        verdict: ToolVerdict::Success,
        summary: SafeSummary::new("operator config updated")
            .expect("static summary is redaction-safe"),
        progress: ResultProgress::MadeProgress,
        terminate_hint: TerminateHint::Continue,
    })
}

fn services_with_operator_approval_config() -> OperatorConfigServices {
    services_with_operator_approval_config_parts().0
}

fn services_with_operator_approval_config_parts() -> (
    OperatorConfigServices,
    Arc<
        ironclaw_approvals::FilesystemPersistentApprovalPolicyStore<
            ironclaw_filesystem::InMemoryBackend,
        >,
    >,
) {
    let persistent_policies = Arc::new(
        ironclaw_approvals::test_support::in_memory_backed_persistent_approval_policy_store(),
    );
    let policy_store: Arc<dyn PersistentApprovalPolicyStore> = persistent_policies.clone();
    let services = services_with_operator_approval_config_policy_store(policy_store);
    (services, persistent_policies)
}

fn services_with_operator_approval_config_policy_store(
    persistent_policies: Arc<dyn PersistentApprovalPolicyStore>,
) -> OperatorConfigServices {
    services_with_operator_approval_config_stores(
        Arc::new(ironclaw_approvals::test_support::in_memory_backed_auto_approve_setting_store()),
        persistent_policies,
    )
}

fn services_with_operator_approval_config_stores(
    auto_approve: Arc<dyn AutoApproveSettingStore>,
    persistent_policies: Arc<dyn PersistentApprovalPolicyStore>,
) -> OperatorConfigServices {
    let overrides: Arc<dyn ToolPermissionOverrideStore> = Arc::new(
        ironclaw_approvals::test_support::in_memory_backed_capability_permission_override_store(),
    );
    let tools = Arc::new(operator_config_test_tools());
    RebornServices::new_with_product_capability_invoker(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
        OperatorConfigAutoApproveInvoker {
            auto_approve: Arc::clone(&auto_approve),
            overrides: Arc::clone(&overrides),
            persistent_policies: Arc::clone(&persistent_policies),
            tools: Arc::clone(&tools),
        },
    )
    .with_operator_approval_config(
        overrides,
        auto_approve,
        persistent_policies.clone(),
        Arc::new(StaticOperatorToolCatalogForTest {
            tools: tools.as_ref().clone(),
        }),
    )
}

fn operator_config_test_tools() -> Vec<RebornOperatorToolInfo> {
    vec![
        RebornOperatorToolInfo {
            capability_id: CapabilityId::new("tool.alpha").expect("capability id"),
            provider: ExtensionId::new("extension.alpha").expect("extension id"),
            description: Arc::from("Alpha tool"),
            default_permission: PermissionMode::Ask,
            effects: Arc::from([EffectKind::ExecuteCode]),
        },
        RebornOperatorToolInfo {
            capability_id: CapabilityId::new("tool.default_allow").expect("capability id"),
            provider: ExtensionId::new("extension.default_allow").expect("extension id"),
            description: Arc::from("Default-allow tool"),
            default_permission: PermissionMode::Allow,
            effects: Arc::from([EffectKind::DispatchCapability]),
        },
        RebornOperatorToolInfo {
            capability_id: CapabilityId::new("tool.locked").expect("capability id"),
            provider: ExtensionId::new("extension.locked").expect("extension id"),
            description: Arc::from("Locked tool"),
            default_permission: PermissionMode::Deny,
            effects: Arc::from([]),
        },
        RebornOperatorToolInfo {
            capability_id: CapabilityId::new("tool.financial").expect("capability id"),
            provider: ExtensionId::new("extension.financial").expect("extension id"),
            description: Arc::from("Financial tool"),
            default_permission: PermissionMode::Ask,
            effects: Arc::from([EffectKind::Financial]),
        },
        RebornOperatorToolInfo {
            capability_id: CapabilityId::new("nearai.web_search").expect("capability id"),
            provider: ExtensionId::new("nearai").expect("extension id"),
            description: Arc::from("Search through the NEAR AI MCP server."),
            default_permission: PermissionMode::Ask,
            effects: Arc::from([EffectKind::DispatchCapability]),
        },
        RebornOperatorToolInfo {
            capability_id: CapabilityId::new("github.get_repo").expect("capability id"),
            provider: ExtensionId::new("github").expect("extension id"),
            description: Arc::from("Read GitHub repository metadata."),
            default_permission: PermissionMode::Ask,
            effects: Arc::from([EffectKind::DispatchCapability]),
        },
        RebornOperatorToolInfo {
            capability_id: CapabilityId::new("google-calendar.list_events").expect("capability id"),
            provider: ExtensionId::new("google-calendar").expect("extension id"),
            description: Arc::from("List Google Calendar events."),
            default_permission: PermissionMode::Ask,
            effects: Arc::from([EffectKind::DispatchCapability]),
        },
    ]
}

fn operator_config_entry_value<'a>(
    response: &'a ironclaw_product_workflow::RebornOperatorConfigListResponse,
    key: &str,
) -> &'a serde_json::Value {
    &response
        .entries
        .iter()
        .find(|entry| entry.key == key)
        .unwrap_or_else(|| panic!("{key} entry"))
        .value
}

async fn query_operator_config_list<S: ProductSurface + ?Sized>(
    services: &S,
    caller: WebUiAuthenticatedCaller,
) -> RebornOperatorConfigListResponse {
    let page = services
        .query(
            caller,
            RebornViewQuery {
                view_id: OPERATOR_CONFIG_LIST_VIEW.id.to_string(),
                params: json!({}),
                cursor: None,
            },
        )
        .await
        .expect("operator config list view");
    serde_json::from_value(page.payload).expect("operator config list payload")
}

async fn query_operator_config_key<S: ProductSurface + ?Sized>(
    services: &S,
    caller: WebUiAuthenticatedCaller,
    key: &str,
) -> Result<RebornOperatorConfigGetResponse, RebornServicesError> {
    let page = services
        .query(
            caller,
            RebornViewQuery {
                view_id: OPERATOR_CONFIG_KEY_VIEW.id.to_string(),
                params: json!({ "key": key }),
                cursor: None,
            },
        )
        .await?;
    serde_json::from_value(page.payload).map_err(RebornServicesError::internal_from)
}

async fn query_operator_setup<S: ProductSurface + ?Sized>(
    services: &S,
    caller: WebUiAuthenticatedCaller,
) -> Result<ironclaw_product_workflow::RebornOperatorSetupResponse, RebornServicesError> {
    let page = services
        .query(
            caller,
            RebornViewQuery {
                view_id: OPERATOR_SETUP_VIEW.id.to_string(),
                params: json!({}),
                cursor: None,
            },
        )
        .await?;
    serde_json::from_value(page.payload).map_err(RebornServicesError::internal_from)
}

async fn query_automations<S: ProductSurface + ?Sized>(
    services: &S,
    caller: WebUiAuthenticatedCaller,
    request: WebUiListAutomationsRequest,
) -> Result<RebornListAutomationsResponse, RebornServicesError> {
    let page = services
        .query(caller, AUTOMATIONS_VIEW.query(request, None)?)
        .await?;
    AUTOMATIONS_VIEW.decode_page(page)
}

async fn invoke_json_product_capability<S, T>(
    services: &S,
    caller: WebUiAuthenticatedCaller,
    capability_id: &str,
    input: T,
) -> Result<Resolution, RebornServicesError>
where
    S: ProductSurface + ?Sized,
    T: Serialize,
{
    services
        .invoke(
            caller,
            CapabilityId::new(capability_id).expect("capability id"),
            ProductCapabilityInput::json(serde_json::to_value(input).expect("capability input")),
            ActivityId::new(),
        )
        .await
}

async fn query_threads<S: ProductSurface + ?Sized>(
    services: &S,
    caller: WebUiAuthenticatedCaller,
    mut request: WebUiListThreadsRequest,
) -> Result<RebornListThreadsResponse, RebornServicesError> {
    let cursor = request.cursor.take();
    let page = services
        .query(caller, THREADS_VIEW.query(request, cursor)?)
        .await?;
    THREADS_VIEW.decode_page(page)
}

async fn query_extensions<S: ProductSurface + ?Sized>(
    services: &S,
    caller: WebUiAuthenticatedCaller,
) -> Result<RebornExtensionListResponse, RebornServicesError> {
    let page = services
        .query(
            caller,
            RebornViewQuery {
                view_id: EXTENSIONS_VIEW.id.to_string(),
                params: json!({}),
                cursor: None,
            },
        )
        .await?;
    serde_json::from_value(page.payload).map_err(RebornServicesError::internal_from)
}

async fn query_extension_setup<S: ProductSurface + ?Sized>(
    services: &S,
    caller: WebUiAuthenticatedCaller,
    package_id: &str,
) -> Result<RebornSetupExtensionResponse, RebornServicesError> {
    let page = services
        .query(
            caller,
            RebornViewQuery {
                view_id: EXTENSION_SETUP_VIEW.id.to_string(),
                params: json!({ "package_id": package_id }),
                cursor: None,
            },
        )
        .await?;
    serde_json::from_value(page.payload).map_err(RebornServicesError::internal_from)
}

async fn invoke_extension_setup_submit<S: ProductSurface + ?Sized>(
    services: &S,
    caller: WebUiAuthenticatedCaller,
    package_id: &str,
    request: WebUiSetupExtensionRequest,
) -> Result<Resolution, RebornServicesError> {
    let mut input = serde_json::to_value(request).map_err(RebornServicesError::internal_from)?;
    input
        .as_object_mut()
        .expect("setup request serializes as object")
        .insert(
            "extension_id".to_string(),
            serde_json::Value::String(package_id.to_string()),
        );
    services
        .invoke(
            caller,
            CapabilityId::new(EXTENSION_SETUP_SUBMIT_CAPABILITY_ID).expect("capability id"),
            ProductCapabilityInput::json(input),
            ActivityId::new(),
        )
        .await
}

async fn submit_extension_setup_and_query<S: ProductSurface + ?Sized>(
    services: &S,
    caller: WebUiAuthenticatedCaller,
    package_id: &str,
    request: WebUiSetupExtensionRequest,
) -> Result<RebornSetupExtensionResponse, RebornServicesError> {
    let caller_for_query = caller.clone();
    invoke_extension_setup_submit(services, caller, package_id, request).await?;
    query_extension_setup(services, caller_for_query, package_id).await
}

#[tokio::test]
async fn extension_import_is_available_as_product_capability() {
    let lifecycle_facade = Arc::new(RecordingLifecycleFacade::new());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_lifecycle_product_facade(lifecycle_facade.clone());
    let bundle: Vec<u8> = b"PK\x03\x04\x00\xff\xfe binary zip bytes".to_vec();

    let resolution = services
        .invoke(
            caller(),
            CapabilityId::new(EXTENSION_IMPORT_CAPABILITY_ID).expect("capability id"),
            ProductCapabilityInput::json(json!({ "bundle_base64": STANDARD.encode(&bundle) })),
            ActivityId::new(),
        )
        .await
        .expect("extension import");

    assert!(matches!(
        resolution,
        Resolution::Done(outcome) if outcome.verdict.is_success()
    ));
    assert_eq!(lifecycle_facade.imported_bundles(), vec![bundle]);
}

#[tokio::test]
async fn skill_reads_are_available_as_product_views() {
    let skills = Arc::new(RecordingSkillsFacade::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_skills_product_facade(skills.clone());

    let list_page = services
        .query(
            caller(),
            RebornViewQuery {
                view_id: SKILLS_VIEW.id.to_string(),
                params: json!({}),
                cursor: None,
            },
        )
        .await
        .expect("skills view");
    let listed: RebornSkillListResponse =
        serde_json::from_value(list_page.payload).expect("skills payload");
    assert_eq!(listed.count, 1);
    assert_eq!(listed.skills[0].name, "local-skill");

    let search_page = services
        .query(
            caller(),
            RebornViewQuery {
                view_id: SKILL_SEARCH_VIEW.id.to_string(),
                params: json!({ "query": "registry" }),
                cursor: None,
            },
        )
        .await
        .expect("skill search view");
    let searched: RebornSkillSearchResponse =
        serde_json::from_value(search_page.payload).expect("skill search payload");
    assert_eq!(searched.registry_url, "https://skills.example.test");
    assert_eq!(
        skills.search_queries.lock().expect("lock").as_slice(),
        ["registry"]
    );

    let content_page = services
        .query(
            caller(),
            RebornViewQuery {
                view_id: SKILL_CONTENT_VIEW.id.to_string(),
                params: json!({ "name": "local-skill" }),
                cursor: None,
            },
        )
        .await
        .expect("skill content view");
    let content: RebornSkillContentResponse =
        serde_json::from_value(content_page.payload).expect("skill content payload");
    assert_eq!(content.name, "local-skill");
    assert_eq!(content.content, "# local-skill\n");
}

fn skill_info(name: &str) -> RebornSkillInfo {
    RebornSkillInfo {
        name: name.to_string(),
        description: format!("{name} skill"),
        version: "1.0.0".to_string(),
        trust: RebornSkillTrustLevel::Installed,
        source: RebornSkillSourceKind::User,
        source_kind: RebornSkillSourceKind::User,
        keywords: Vec::new(),
        usage_hint: None,
        setup_hint: None,
        bundle_path: None,
        install_source_url: None,
        has_requirements: false,
        has_scripts: false,
        can_edit: true,
        can_delete: true,
        auto_activate: true,
    }
}

#[derive(Default)]
struct RecordingSkillsFacade {
    search_queries: Mutex<Vec<String>>,
}

#[async_trait]
impl SkillsProductFacade for RecordingSkillsFacade {
    async fn list_skills(
        &self,
        _caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornSkillListResponse, RebornServicesError> {
        Ok(RebornSkillListResponse {
            skills: vec![skill_info("local-skill")],
            count: 1,
            auto_activate_learned: true,
        })
    }

    async fn search_skills(
        &self,
        _caller: WebUiAuthenticatedCaller,
        query: String,
    ) -> Result<RebornSkillSearchResponse, RebornServicesError> {
        self.search_queries.lock().expect("lock").push(query);
        Ok(RebornSkillSearchResponse {
            catalog: vec![json!({ "name": "registry-skill" })],
            installed: vec![skill_info("local-skill")],
            registry_url: "https://skills.example.test".to_string(),
            catalog_error: None,
        })
    }

    async fn read_skill_content(
        &self,
        _caller: WebUiAuthenticatedCaller,
        name: String,
    ) -> Result<RebornSkillContentResponse, RebornServicesError> {
        Ok(RebornSkillContentResponse {
            content: format!("# {name}\n"),
            name,
        })
    }
}

fn operator_policy_scope_for_test(tenant_id: &str, user_id: &str) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new(tenant_id).expect("tenant id"),
        user_id: UserId::new(user_id).expect("user id"),
        agent_id: None,
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

#[tokio::test]
async fn operator_config_reads_provider_grantee_policies_as_always_allow() {
    let (services, persistent_policies) = services_with_operator_approval_config_parts();
    let operator_scope = operator_policy_scope_for_test("tenant-alpha", "user-alpha");

    for (capability_id, provider) in [
        ("nearai.web_search", "nearai"),
        ("github.get_repo", "github"),
        ("google-calendar.list_events", "google-calendar"),
    ] {
        persistent_policies
            .allow(PersistentApprovalPolicyInput {
                scope: operator_scope.clone(),
                action: PersistentApprovalAction::Dispatch,
                capability_id: CapabilityId::new(capability_id).expect("capability id"),
                grantee: Principal::Extension(ExtensionId::new(provider).expect("extension id")),
                approved_by: Principal::User(UserId::new("user-alpha").expect("user id")),
                constraints: ironclaw_host_api::GrantConstraints {
                    allowed_effects: vec![EffectKind::DispatchCapability],
                    mounts: Default::default(),
                    network: Default::default(),
                    secrets: Vec::new(),
                    resource_ceiling: None,
                    expires_at: None,
                    max_invocations: None,
                },
                source_approval_request_id: Some(ApprovalRequestId::new()),
            })
            .await
            .expect("seed provider-grantee always-allow policy");
    }

    let config = query_operator_config_list(&services, caller()).await;
    for capability_id in [
        "nearai.web_search",
        "github.get_repo",
        "google-calendar.list_events",
    ] {
        let value = operator_config_entry_value(&config, &format!("tool.{capability_id}"));
        assert_eq!(value["state"], "always_allow");
        assert_eq!(value["effective_source"], "override");
    }
}

#[tokio::test]
async fn global_auto_approve_enabled_scopes_read_by_caller_tenant_and_user() {
    let auto_approve = Arc::new(RecordingAutoApproveSettingStore::default());
    let services = services_with_operator_approval_config_stores(
        auto_approve.clone(),
        Arc::new(
            ironclaw_approvals::test_support::in_memory_backed_persistent_approval_policy_store(),
        ),
    );
    let caller = WebUiAuthenticatedCaller::new(
        TenantId::new("tenant-scope").expect("tenant"),
        UserId::new("user-scope").expect("user"),
        Some(AgentId::new("agent-scope").expect("agent")),
        Some(ProjectId::new("project-scope").expect("project")),
    );

    let enabled = services
        .global_auto_approve_enabled(caller)
        .await
        .expect("global auto approve read");

    assert!(
        enabled,
        "unset auto-approve should resolve through the default"
    );
    let keys = auto_approve.get_keys();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].tenant_id.as_str(), "tenant-scope");
    assert_eq!(keys[0].user_id.as_str(), "user-scope");
}

#[tokio::test]
async fn operator_config_reads_and_writes_auto_approve_and_tool_permissions() {
    let (services, persistent_policies) = services_with_operator_approval_config_parts();

    let initial = query_operator_config_list(&services, caller()).await;
    assert_eq!(
        operator_config_entry_value(&initial, "agent.auto_approve_tools"),
        &json!(true)
    );
    assert_eq!(
        operator_config_entry_value(&initial, "tool.tool.alpha")["state"],
        "always_allow"
    );
    assert_eq!(
        operator_config_entry_value(&initial, "tool.tool.alpha")["effective_source"],
        "global"
    );
    assert_eq!(
        operator_config_entry_value(&initial, "tool.tool.default_allow")["state"],
        "always_allow",
        "follow-global tools auto-approve while global auto-approve defaults on"
    );
    assert_eq!(
        operator_config_entry_value(&initial, "tool.tool.default_allow")["effective_source"],
        "global"
    );
    assert_eq!(
        operator_config_entry_value(&initial, "tool.tool.locked")["state"],
        "disabled"
    );
    assert_eq!(
        operator_config_entry_value(&initial, "tool.tool.financial")["state"],
        "ask_each_time"
    );
    assert_eq!(
        operator_config_entry_value(&initial, "tool.tool.financial")["locked"],
        true
    );
    assert_eq!(
        operator_config_entry_value(&initial, "tool.tool.financial")["effective_source"],
        "locked"
    );

    let financial_error = services
        .set_operator_config_key(
            caller(),
            "tool.tool.financial".to_string(),
            RebornOperatorConfigSetRequest {
                value: json!({ "state": "always_allow" }),
            },
        )
        .await
        .expect_err("hard-floor tool cannot be made always-allow");
    assert_eq!(
        financial_error.code,
        RebornServicesErrorCode::InvalidRequest
    );
    assert_eq!(financial_error.kind, RebornServicesErrorKind::Validation);
    assert_eq!(
        financial_error.validation_code,
        Some(WebUiInboundValidationCode::InvalidValue)
    );

    services
        .set_operator_config_key(
            caller(),
            "agent.auto_approve_tools".to_string(),
            RebornOperatorConfigSetRequest { value: json!(true) },
        )
        .await
        .expect("enable global auto approve");

    let globally_allowed = query_operator_config_key(&services, caller(), "tool.tool.alpha")
        .await
        .expect("tool config");
    assert_eq!(globally_allowed.entry.value["state"], "always_allow");
    assert_eq!(globally_allowed.entry.value["effective_source"], "global");

    let default_allow_global =
        query_operator_config_key(&services, caller(), "tool.tool.default_allow")
            .await
            .expect("default-allow tool config");
    assert_eq!(default_allow_global.entry.value["state"], "always_allow");
    assert_eq!(
        default_allow_global.entry.value["effective_source"],
        "global"
    );

    let ask_override = services
        .set_operator_config_key(
            caller(),
            "tool.tool.alpha".to_string(),
            RebornOperatorConfigSetRequest {
                value: json!({ "state": "ask_each_time" }),
            },
        )
        .await
        .expect("ask each time override");
    assert_eq!(ask_override.entry.value["state"], "ask_each_time");
    assert_eq!(ask_override.entry.value["effective_source"], "override");

    let follows_global = services
        .set_operator_config_key(
            caller(),
            "tool.tool.alpha".to_string(),
            RebornOperatorConfigSetRequest {
                value: json!({ "state": "default" }),
            },
        )
        .await
        .expect("clear tool override");
    assert_eq!(follows_global.entry.value["state"], "always_allow");
    assert_eq!(follows_global.entry.value["effective_source"], "global");

    let disabled = services
        .set_operator_config_key(
            caller(),
            "tool.tool.alpha".to_string(),
            RebornOperatorConfigSetRequest {
                value: json!({ "state": "disabled" }),
            },
        )
        .await
        .expect("disable tool");
    assert_eq!(disabled.entry.value["state"], "disabled");
    assert_eq!(disabled.entry.value["effective_source"], "override");

    let allowed = services
        .set_operator_config_key(
            caller(),
            "tool.tool.alpha".to_string(),
            RebornOperatorConfigSetRequest {
                value: json!({ "state": "always_allow" }),
            },
        )
        .await
        .expect("always allow tool");
    assert_eq!(allowed.entry.value["state"], "always_allow");
    assert_eq!(allowed.entry.value["effective_source"], "override");

    let operator_scope = operator_policy_scope_for_test("tenant-alpha", "user-alpha");
    let extension_key = PersistentApprovalPolicyKey::new(
        &operator_scope,
        PersistentApprovalAction::Dispatch,
        CapabilityId::new("tool.alpha").expect("capability id"),
        Principal::Extension(ExtensionId::new("extension.alpha").expect("extension id")),
    );
    assert!(
        persistent_policies
            .lookup(&extension_key)
            .await
            .expect("lookup extension-grantee policy")
            .and_then(|policy| policy.active_grant())
            .is_some(),
        "settings-page always_allow must write the provider extension grantee"
    );

    let user_key = PersistentApprovalPolicyKey::new(
        &operator_scope,
        PersistentApprovalAction::Dispatch,
        CapabilityId::new("tool.alpha").expect("capability id"),
        Principal::User(UserId::new("user-alpha").expect("user id")),
    );
    assert!(
        persistent_policies
            .lookup(&user_key)
            .await
            .expect("lookup user-grantee policy")
            .is_none(),
        "settings-page always_allow must not write a user-grantee policy"
    );

    let reset = services
        .set_operator_config_key(
            caller(),
            "tool.tool.alpha".to_string(),
            RebornOperatorConfigSetRequest {
                value: json!({ "state": "default" }),
            },
        )
        .await
        .expect("clear persistent always-allow");
    assert_eq!(reset.entry.value["state"], "always_allow");
    assert_eq!(reset.entry.value["effective_source"], "global");
    assert!(
        persistent_policies
            .lookup(&extension_key)
            .await
            .expect("lookup extension-grantee policy after reset")
            .and_then(|policy| policy.active_grant())
            .is_none(),
        "default must clear the settings-page persistent always-allow policy"
    );
}

#[tokio::test]
async fn operator_config_reads_are_available_as_product_views() {
    let services = services_with_operator_approval_config();

    let list_page = services
        .query(
            caller(),
            RebornViewQuery {
                view_id: OPERATOR_CONFIG_LIST_VIEW.id.to_string(),
                params: json!({}),
                cursor: None,
            },
        )
        .await
        .expect("operator config list view");
    let list: RebornOperatorConfigListResponse =
        serde_json::from_value(list_page.payload).expect("operator config list payload");
    assert_eq!(
        operator_config_entry_value(&list, "agent.auto_approve_tools"),
        &json!(true)
    );

    let key_page = services
        .query(
            caller(),
            RebornViewQuery {
                view_id: OPERATOR_CONFIG_KEY_VIEW.id.to_string(),
                params: json!({ "key": "tool.tool.alpha" }),
                cursor: None,
            },
        )
        .await
        .expect("operator config key view");
    let key: RebornOperatorConfigGetResponse =
        serde_json::from_value(key_page.payload).expect("operator config key payload");
    assert_eq!(key.entry.key, "tool.tool.alpha");
    assert_eq!(key.entry.value["state"], "always_allow");
    assert_eq!(key.entry.value["effective_source"], "global");

    let validate_page = services
        .query(
            caller(),
            RebornViewQuery {
                view_id: OPERATOR_CONFIG_VALIDATE_VIEW.id.to_string(),
                params: json!({ "keys": ["provider.default", "legacy.old"] }),
                cursor: None,
            },
        )
        .await
        .expect("operator config validate view");
    let validate: RebornOperatorConfigValidateResponse =
        serde_json::from_value(validate_page.payload).expect("operator config validate payload");
    assert!(!validate.valid);
    assert!(validate.diagnostics.iter().any(|diagnostic| {
        diagnostic.reason_code == "operator_config_deprecated" && diagnostic.key == "legacy.old"
    }));
}

#[tokio::test]
async fn operator_config_is_scoped_by_tenant_and_user() {
    let services = services_with_operator_approval_config();
    let alice_tenant_a = WebUiAuthenticatedCaller::new(
        TenantId::new("tenant-alpha").expect("tenant"),
        UserId::new("alice").expect("user"),
        Some(AgentId::new("agent-alpha").expect("agent")),
        Some(ProjectId::new("project-alpha").expect("project")),
    );
    let bob_tenant_a = WebUiAuthenticatedCaller::new(
        TenantId::new("tenant-alpha").expect("tenant"),
        UserId::new("bob").expect("user"),
        Some(AgentId::new("agent-alpha").expect("agent")),
        Some(ProjectId::new("project-alpha").expect("project")),
    );
    let alice_tenant_a_other_project = WebUiAuthenticatedCaller::new(
        TenantId::new("tenant-alpha").expect("tenant"),
        UserId::new("alice").expect("user"),
        Some(AgentId::new("agent-beta").expect("agent")),
        Some(ProjectId::new("project-beta").expect("project")),
    );
    let alice_tenant_b = WebUiAuthenticatedCaller::new(
        TenantId::new("tenant-beta").expect("tenant"),
        UserId::new("alice").expect("user"),
        Some(AgentId::new("agent-alpha").expect("agent")),
        Some(ProjectId::new("project-alpha").expect("project")),
    );

    services
        .set_operator_config_key(
            alice_tenant_a.clone(),
            "agent.auto_approve_tools".to_string(),
            // Set the non-default (off) so isolation is provable: other
            // users/tenants must still read the default (on), not this value.
            RebornOperatorConfigSetRequest {
                value: json!(false),
            },
        )
        .await
        .expect("alice disables auto approve in tenant alpha");
    services
        .set_operator_config_key(
            alice_tenant_a.clone(),
            "tool.tool.alpha".to_string(),
            RebornOperatorConfigSetRequest {
                value: json!({ "state": "disabled" }),
            },
        )
        .await
        .expect("alice disables tool in tenant alpha");

    let alice_alpha = query_operator_config_key(&services, alice_tenant_a, "tool.tool.alpha")
        .await
        .expect("alice alpha tool config");
    assert_eq!(alice_alpha.entry.value["state"], "disabled");
    assert_eq!(alice_alpha.entry.value["effective_source"], "override");

    let alice_alpha_other_project =
        query_operator_config_list(&services, alice_tenant_a_other_project).await;
    assert_eq!(
        operator_config_entry_value(&alice_alpha_other_project, "agent.auto_approve_tools"),
        &json!(false),
        "auto-approve settings are scoped by tenant/user, not agent/project"
    );
    assert_eq!(
        operator_config_entry_value(&alice_alpha_other_project, "tool.tool.alpha")["state"],
        "disabled",
        "tool overrides are scoped by tenant/user, not agent/project"
    );
    assert_eq!(
        operator_config_entry_value(&alice_alpha_other_project, "tool.tool.alpha")["effective_source"],
        "override"
    );

    for caller in [bob_tenant_a, alice_tenant_b] {
        let config = query_operator_config_list(&services, caller).await;
        assert_eq!(
            operator_config_entry_value(&config, "agent.auto_approve_tools"),
            &json!(true),
            "auto-approve must not leak across user or tenant"
        );
        assert_eq!(
            operator_config_entry_value(&config, "tool.tool.alpha")["state"],
            "always_allow",
            "tool override must not leak across user or tenant (follows global default-on)"
        );
        assert_eq!(
            operator_config_entry_value(&config, "tool.tool.alpha")["effective_source"],
            "global"
        );
    }
}

#[tokio::test]
async fn operator_config_preserves_override_when_always_allow_policy_write_fails() {
    let services = services_with_operator_approval_config_policy_store(Arc::new(
        FailingAllowPersistentApprovalPolicyStore,
    ));

    let ask_override = services
        .set_operator_config_key(
            caller(),
            "tool.tool.alpha".to_string(),
            RebornOperatorConfigSetRequest {
                value: json!({ "state": "ask_each_time" }),
            },
        )
        .await
        .expect("ask each time override");
    assert_eq!(ask_override.entry.value["state"], "ask_each_time");
    assert_eq!(ask_override.entry.value["effective_source"], "override");

    let error = services
        .set_operator_config_key(
            caller(),
            "tool.tool.alpha".to_string(),
            RebornOperatorConfigSetRequest {
                value: json!({ "state": "always_allow" }),
            },
        )
        .await
        .expect_err("persistent policy write failure");
    assert_eq!(error.code, RebornServicesErrorCode::Internal);

    let preserved = query_operator_config_key(&services, caller(), "tool.tool.alpha")
        .await
        .expect("tool config after failed always_allow");
    assert_eq!(preserved.entry.value["state"], "ask_each_time");
    assert_eq!(
        preserved.entry.value["effective_source"], "override",
        "failed policy writes must not clear the existing per-tool override"
    );
}

#[tokio::test]
async fn operator_config_reports_unknown_keys_distinct_from_invalid_values() {
    let services = services_with_operator_approval_config();

    let unknown_key = query_operator_config_key(&services, caller(), "tool.tool.missing")
        .await
        .expect_err("unknown tool key");
    assert_eq!(
        unknown_key.validation_code,
        Some(WebUiInboundValidationCode::UnknownKey)
    );

    let invalid_value = services
        .set_operator_config_key(
            caller(),
            "tool.tool.alpha".to_string(),
            RebornOperatorConfigSetRequest {
                value: json!({ "state": "sometimes" }),
            },
        )
        .await
        .expect_err("invalid tool state");
    assert_eq!(
        invalid_value.validation_code,
        Some(WebUiInboundValidationCode::InvalidValue)
    );
}

#[tokio::test]
async fn operator_diagnostics_aggregates_status_setup_and_config_reasons() {
    let llm_config = Arc::new(SetupRecordingLlmConfigService::default());
    llm_config.use_active_snapshot("openai", "gpt-5-mini");
    let status_service = Arc::new(RecordingOperatorStatusService::new(
        RebornOperatorStatusResponse {
            generated_at: Utc::now(),
            overall: RebornOperatorStatusState::Blocked,
            checks: vec![
                RebornOperatorStatusCheck {
                    id: "storage".to_string(),
                    status: RebornOperatorStatusState::Blocked,
                    severity: RebornOperatorStatusSeverity::Critical,
                    summary: "storage backend is unavailable".to_string(),
                    remediation: Some("repair storage configuration".to_string()),
                },
                RebornOperatorStatusCheck {
                    id: "provider_model".to_string(),
                    status: RebornOperatorStatusState::Ready,
                    severity: RebornOperatorStatusSeverity::Info,
                    summary: "provider and model are configured".to_string(),
                    remediation: None,
                },
                RebornOperatorStatusCheck {
                    id: "sk-secret-token".to_string(),
                    status: RebornOperatorStatusState::Blocked,
                    severity: RebornOperatorStatusSeverity::Critical,
                    summary: "failed with sk-test1234567890 at /home/alice/.env".to_string(),
                    remediation: Some(
                        "inspect /home/alice/.ssh/id_ed25519 and credential token".to_string(),
                    ),
                },
                RebornOperatorStatusCheck {
                    id: "workspace_path".to_string(),
                    status: RebornOperatorStatusState::Blocked,
                    severity: RebornOperatorStatusSeverity::Warning,
                    summary: "artifact staged at /workspace/ironclaw/.env".to_string(),
                    remediation: Some("remove /workspace/ironclaw/secrets.env".to_string()),
                },
            ],
        },
    ));
    let services = services_with_setup_llm_config(llm_config.clone())
        .with_operator_status_service(status_service.clone());
    let diagnostics_caller =
        caller_for_user_with_project("user-diagnostics", Some("project-diagnostics"))
            .with_operator_webui_config(true);

    let page = services
        .query(
            diagnostics_caller.clone(),
            RebornViewQuery {
                view_id: OPERATOR_DIAGNOSTICS_VIEW.id.to_string(),
                params: json!({}),
                cursor: None,
            },
        )
        .await
        .expect("operator diagnostics");
    let response: RebornOperatorCommandPlaneResponse =
        serde_json::from_value(page.payload).expect("operator diagnostics payload");

    assert_eq!(response.area.as_str(), "diagnostics");
    assert_eq!(response.status, RebornOperatorSurfaceStatus::Unavailable);
    assert!(response.operator_status.is_some());
    let reason_codes = response
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.reason_code.as_str())
        .collect::<Vec<_>>();
    assert!(reason_codes.contains(&"operator_doctor_storage_blocked"));
    assert!(reason_codes.contains(&"operator_doctor_status_blocked"));
    assert!(reason_codes.contains(&"operator_doctor_workspace_path_blocked"));
    assert!(reason_codes.contains(&"operator_config_service_not_wired"));
    assert!(!reason_codes.contains(&"operator_doctor_provider_model_ready"));
    let rendered = serde_json::to_string(&response).expect("serialize diagnostics");
    assert!(!rendered.contains("sk-"));
    assert!(!rendered.contains("/home/"));
    assert!(!rendered.contains("/workspace/"));
    assert!(!rendered.contains(".ssh"));
    assert!(!rendered.contains("credential token"));
    assert!(
        response.diagnostics.iter().any(|diagnostic| {
            diagnostic.reason_code == "operator_doctor_status_blocked"
                && diagnostic.key == "[redacted operator status detail]"
                && diagnostic.message == "[redacted operator status detail]"
                && diagnostic.remediation == "[redacted operator status detail]"
        }),
        "sensitive status-derived diagnostic details should be redacted"
    );
    assert!(
        response.diagnostics.iter().any(|diagnostic| {
            diagnostic.reason_code == "operator_doctor_workspace_path_blocked"
                && diagnostic.key == "workspace_path"
                && diagnostic.message == "[redacted operator status detail]"
                && diagnostic.remediation == "[redacted operator status detail]"
        }),
        "/workspace/ status-derived diagnostic details should be redacted"
    );
    assert_eq!(status_service.callers(), vec![diagnostics_caller.clone()]);
    assert_eq!(llm_config.snapshot_callers(), vec![diagnostics_caller]);
}

#[tokio::test]
async fn operator_command_plane_reads_are_available_as_product_views() {
    let llm_config = Arc::new(SetupRecordingLlmConfigService::default());
    llm_config.use_active_snapshot("openai", "gpt-5-mini");
    let services =
        services_with_setup_llm_config(llm_config).with_operator_status_service(Arc::new(
            StaticOperatorStatusService::new(RebornOperatorStatusResponse {
                generated_at: Utc::now(),
                overall: RebornOperatorStatusState::Ready,
                checks: Vec::new(),
            }),
        ));

    let operator = caller().with_operator_webui_config(true);
    let diagnostics_page = services
        .query(
            operator.clone(),
            RebornViewQuery {
                view_id: OPERATOR_DIAGNOSTICS_VIEW.id.to_string(),
                params: json!({}),
                cursor: None,
            },
        )
        .await
        .expect("operator diagnostics view");
    let diagnostics: RebornOperatorCommandPlaneResponse =
        serde_json::from_value(diagnostics_page.payload).expect("diagnostics payload");
    assert_eq!(diagnostics.area.as_str(), "diagnostics");
    assert_eq!(diagnostics.status, RebornOperatorSurfaceStatus::Unavailable);
    assert!(
        diagnostics
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.reason_code == "operator_config_service_not_wired")
    );

    let status_page = services
        .query(
            operator,
            RebornViewQuery {
                view_id: OPERATOR_STATUS_VIEW.id.to_string(),
                params: json!({}),
                cursor: None,
            },
        )
        .await
        .expect("operator status view");
    let status: RebornOperatorCommandPlaneResponse =
        serde_json::from_value(status_page.payload).expect("status payload");
    assert_eq!(status.area.as_str(), "status");
    assert_eq!(status.status, RebornOperatorSurfaceStatus::Available);
    assert!(status.operator_status.is_some());
}

#[tokio::test]
async fn llm_config_snapshot_is_available_as_product_view() {
    let llm_config = Arc::new(SetupRecordingLlmConfigService::default());
    llm_config.use_active_snapshot("openai", "gpt-5-mini");
    let services = services_with_setup_llm_config(llm_config.clone());
    let snapshot_caller = caller_for_user_with_project("user-llm", Some("project-llm"))
        .with_operator_webui_config(true);

    let page = services
        .query(
            snapshot_caller.clone(),
            RebornViewQuery {
                view_id: LLM_CONFIG_VIEW.id.to_string(),
                params: json!({}),
                cursor: None,
            },
        )
        .await
        .expect("llm config view");
    let snapshot: LlmConfigSnapshot =
        serde_json::from_value(page.payload).expect("llm config payload");

    assert_eq!(
        snapshot
            .active
            .as_ref()
            .map(|active| active.provider_id.as_str()),
        Some("openai")
    );
    assert_eq!(
        snapshot
            .active
            .as_ref()
            .and_then(|active| active.model.as_deref()),
        Some("gpt-5-mini")
    );
    assert_eq!(llm_config.snapshot_callers(), vec![snapshot_caller]);
}

#[tokio::test]
async fn operator_only_product_views_require_operator_webui_config() {
    let llm_config = Arc::new(SetupRecordingLlmConfigService::default());
    let services = services_with_setup_llm_config(llm_config.clone());

    let err = services
        .query(
            caller_for_user_with_project("user-llm", Some("project-llm")),
            RebornViewQuery {
                view_id: LLM_CONFIG_VIEW.id.to_string(),
                params: json!({}),
                cursor: None,
            },
        )
        .await
        .expect_err("non-operator caller cannot query operator-only product views");

    assert_eq!(err.code, RebornServicesErrorCode::Forbidden);
    assert_eq!(err.kind, RebornServicesErrorKind::ParticipantDenied);
    assert_eq!(
        llm_config.snapshot_callers(),
        Vec::<WebUiAuthenticatedCaller>::new()
    );
}

#[tokio::test]
async fn operator_diagnostics_reports_setup_service_absence_without_failing_route() {
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_operator_status_service(Arc::new(StaticOperatorStatusService::new(
        RebornOperatorStatusResponse {
            generated_at: Utc::now(),
            overall: RebornOperatorStatusState::Ready,
            checks: Vec::new(),
        },
    )));

    let page = services
        .query(
            caller().with_operator_webui_config(true),
            RebornViewQuery {
                view_id: OPERATOR_DIAGNOSTICS_VIEW.id.to_string(),
                params: json!({}),
                cursor: None,
            },
        )
        .await
        .expect("operator diagnostics");
    let response: RebornOperatorCommandPlaneResponse =
        serde_json::from_value(page.payload).expect("operator diagnostics payload");

    assert_eq!(response.area.as_str(), "diagnostics");
    assert!(response.diagnostics.iter().any(|diagnostic| {
        diagnostic.reason_code == "operator_setup_service_not_wired"
            && diagnostic.severity == RebornOperatorConfigDiagnosticSeverity::Error
    }));
}

#[tokio::test]
async fn get_operator_setup_returns_snapshot_from_llm_config() {
    let llm_config = Arc::new(SetupRecordingLlmConfigService::default());
    llm_config.use_active_snapshot("openai", "gpt-5-mini");
    let services = services_with_setup_llm_config(llm_config.clone());

    let response = query_operator_setup(&services, caller().with_operator_webui_config(true))
        .await
        .expect("setup response");

    assert_eq!(llm_config.snapshot_count(), 1);
    assert_eq!(response.active_provider_id.as_deref(), Some("openai"));
    assert_eq!(response.active_model.as_deref(), Some("gpt-5-mini"));
    assert_eq!(response.status, RebornOperatorSetupStatus::Complete);
    assert!(response.diagnostics.is_empty());
    assert!(response.steps.iter().any(|step| {
        step.name == "profile"
            && step.status == ironclaw_product_workflow::RebornOperatorSetupStepStatus::Complete
    }));
    assert!(response.steps.iter().any(|step| {
        step.name == "webui_access"
            && step.status == ironclaw_product_workflow::RebornOperatorSetupStepStatus::Complete
    }));
}

#[tokio::test]
async fn get_operator_setup_without_llm_config_returns_service_unavailable() {
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    );

    let err = query_operator_setup(&services, caller().with_operator_webui_config(true))
        .await
        .expect_err("setup service is unavailable");

    assert_eq!(err.code, RebornServicesErrorCode::Unavailable);
    assert_eq!(err.kind, RebornServicesErrorKind::ServiceUnavailable);
    assert_eq!(err.status_code, 503);
}

#[tokio::test]
async fn setup_response_reflects_active_provider_and_model() {
    let llm_config = Arc::new(SetupRecordingLlmConfigService::default());
    llm_config.use_active_snapshot("openai", "gpt-5-mini");
    let services = services_with_setup_llm_config(llm_config.clone());

    let response = services
        .run_operator_setup(caller(), RebornOperatorSetupRequest::default())
        .await
        .expect("setup response");

    assert_eq!(response.active_provider_id.as_deref(), Some("openai"));
    assert_eq!(response.active_model.as_deref(), Some("gpt-5-mini"));
    assert_eq!(response.status, RebornOperatorSetupStatus::Complete);
    assert!(response.steps.iter().any(|step| {
        step.name == "provider"
            && step.status == ironclaw_product_workflow::RebornOperatorSetupStepStatus::Complete
    }));
    assert!(response.steps.iter().any(|step| {
        step.name == "model"
            && step.status == ironclaw_product_workflow::RebornOperatorSetupStepStatus::Complete
    }));
    assert!(response.steps.iter().any(|step| {
        step.name == "profile"
            && step.status == ironclaw_product_workflow::RebornOperatorSetupStepStatus::Complete
    }));
    assert!(response.steps.iter().any(|step| {
        step.name == "webui_access"
            && step.status == ironclaw_product_workflow::RebornOperatorSetupStepStatus::Complete
    }));
}

#[tokio::test]
async fn run_operator_setup_without_llm_config_returns_service_unavailable() {
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    );

    let err = services
        .run_operator_setup(caller(), RebornOperatorSetupRequest::default())
        .await
        .expect_err("setup service is unavailable");

    assert_eq!(err.code, RebornServicesErrorCode::Unavailable);
    assert_eq!(err.kind, RebornServicesErrorKind::ServiceUnavailable);
    assert_eq!(err.status_code, 503);
}

#[tokio::test]
async fn run_operator_setup_requires_provider_id_for_provider_changes() {
    let llm_config = Arc::new(SetupRecordingLlmConfigService::default());
    llm_config.use_active_snapshot("openai", "gpt-5-mini");
    let services = services_with_setup_llm_config(llm_config.clone());

    let err = services
        .run_operator_setup(
            caller(),
            RebornOperatorSetupRequest::default()
                .set_adapter("open_ai_completions")
                .set_api_key(SecretString::from("sk-secret".to_string())),
        )
        .await
        .expect_err("provider changes require provider_id");

    assert_setup_validation(err, "provider_id", WebUiInboundValidationCode::InvalidValue);
    assert_eq!(llm_config.snapshot_count(), 0);
    assert_eq!(llm_config.upsert_provider_count(), 0);
    assert_eq!(llm_config.set_active_count(), 0);
}

#[tokio::test]
async fn run_operator_setup_rejects_model_without_provider_id() {
    let llm_config = Arc::new(SetupRecordingLlmConfigService::default());
    llm_config.use_active_snapshot("openai", "gpt-5-mini");
    let services = services_with_setup_llm_config(llm_config.clone());

    let err = services
        .run_operator_setup(
            caller(),
            RebornOperatorSetupRequest::default().set_model("gpt-5-mini"),
        )
        .await
        .expect_err("model requires provider_id");

    assert_setup_validation(err, "model", WebUiInboundValidationCode::InvalidValue);
    assert_eq!(llm_config.snapshot_count(), 0);
}

#[tokio::test]
async fn run_operator_setup_rejects_base_url_without_adapter() {
    let llm_config = Arc::new(SetupRecordingLlmConfigService::default());
    let services = services_with_setup_llm_config(llm_config.clone());

    let err = services
        .run_operator_setup(
            caller(),
            RebornOperatorSetupRequest::default()
                .set_provider_id("openai")
                .set_base_url("https://api.example.test/v1"),
        )
        .await
        .expect_err("base_url requires adapter");

    assert_setup_validation(err, "base_url", WebUiInboundValidationCode::InvalidValue);
    assert_eq!(llm_config.snapshot_count(), 0);
    assert_eq!(llm_config.upsert_provider_count(), 0);
    assert_eq!(llm_config.set_active_count(), 0);
}

#[tokio::test]
async fn run_operator_setup_rejects_api_key_without_adapter() {
    let llm_config = Arc::new(SetupRecordingLlmConfigService::default());
    let services = services_with_setup_llm_config(llm_config.clone());

    let err = services
        .run_operator_setup(
            caller(),
            RebornOperatorSetupRequest::default()
                .set_provider_id("openai")
                .set_api_key(SecretString::from("sk-secret".to_string())),
        )
        .await
        .expect_err("api_key requires adapter");

    assert_setup_validation(err, "api_key", WebUiInboundValidationCode::InvalidValue);
    assert_eq!(llm_config.snapshot_count(), 0);
    assert_eq!(llm_config.upsert_provider_count(), 0);
    assert_eq!(llm_config.set_active_count(), 0);
}

#[tokio::test]
async fn run_operator_setup_rejects_internal_base_url_before_upsert() {
    let llm_config = Arc::new(SetupRecordingLlmConfigService::default());
    let services = services_with_setup_llm_config(llm_config.clone());

    let err = services
        .run_operator_setup(
            caller(),
            RebornOperatorSetupRequest::default()
                .set_provider_id("openai")
                .set_adapter("open_ai_completions")
                .set_base_url("http://169.254.169.254/latest/meta-data/"),
        )
        .await
        .expect_err("metadata endpoint is rejected");

    assert_setup_validation(err, "base_url", WebUiInboundValidationCode::InvalidValue);
    assert_eq!(llm_config.upsert_provider_count(), 0);
}

#[tokio::test]
async fn run_operator_setup_rejects_blank_profile_before_provider_write() {
    let llm_config = Arc::new(SetupRecordingLlmConfigService::default());
    let services = services_with_setup_llm_config(llm_config.clone());

    let err = services
        .run_operator_setup(
            caller(),
            RebornOperatorSetupRequest::default()
                .set_provider_id("openai")
                .set_adapter("open_ai_completions")
                .set_profile_id("   "),
        )
        .await
        .expect_err("blank profile id is rejected");

    assert_setup_validation(err, "profile_id", WebUiInboundValidationCode::InvalidValue);
    assert_eq!(llm_config.upsert_provider_count(), 0);
    assert_eq!(llm_config.set_active_count(), 0);
}

#[tokio::test]
async fn run_operator_setup_rejects_oversized_profile_before_provider_write() {
    let llm_config = Arc::new(SetupRecordingLlmConfigService::default());
    let services = services_with_setup_llm_config(llm_config.clone());

    let err = services
        .run_operator_setup(
            caller(),
            RebornOperatorSetupRequest::default()
                .set_provider_id("openai")
                .set_adapter("open_ai_completions")
                .set_profile_id("x".repeat(129)),
        )
        .await
        .expect_err("oversized profile id is rejected");

    assert_setup_validation(err, "profile_id", WebUiInboundValidationCode::InvalidValue);
    assert_eq!(llm_config.upsert_provider_count(), 0);
    assert_eq!(llm_config.set_active_count(), 0);
}

#[tokio::test]
async fn run_operator_setup_rejects_short_webui_access_token_before_provider_write() {
    let llm_config = Arc::new(SetupRecordingLlmConfigService::default());
    let services = services_with_setup_llm_config(llm_config.clone());

    let err = services
        .run_operator_setup(
            caller(),
            RebornOperatorSetupRequest::default()
                .set_provider_id("openai")
                .set_adapter("open_ai_completions")
                .set_webui_access_token(SecretString::from("too-short".to_string())),
        )
        .await
        .expect_err("short WebUI token is rejected");

    assert_setup_validation(
        err,
        "webui_access_token",
        WebUiInboundValidationCode::InvalidValue,
    );
    assert_eq!(llm_config.upsert_provider_count(), 0);
    assert_eq!(llm_config.set_active_count(), 0);
}

#[tokio::test]
async fn run_operator_setup_rejects_serve_weak_webui_access_token_before_provider_write() {
    let llm_config = Arc::new(SetupRecordingLlmConfigService::default());
    let services = services_with_setup_llm_config(llm_config.clone());

    let err = services
        .run_operator_setup(
            caller(),
            RebornOperatorSetupRequest::default()
                .set_provider_id("openai")
                .set_adapter("open_ai_completions")
                .set_webui_access_token(SecretString::from("x".repeat(16))),
        )
        .await
        .expect_err("16-byte WebUI token is rejected");

    assert_setup_validation(
        err,
        "webui_access_token",
        WebUiInboundValidationCode::InvalidValue,
    );
    assert_eq!(llm_config.upsert_provider_count(), 0);
    assert_eq!(llm_config.set_active_count(), 0);
}

#[tokio::test]
async fn run_operator_setup_rejects_oversized_webui_access_token_before_provider_write() {
    let llm_config = Arc::new(SetupRecordingLlmConfigService::default());
    let services = services_with_setup_llm_config(llm_config.clone());

    let err = services
        .run_operator_setup(
            caller(),
            RebornOperatorSetupRequest::default()
                .set_provider_id("openai")
                .set_adapter("open_ai_completions")
                .set_webui_access_token(SecretString::from("x".repeat(4097))),
        )
        .await
        .expect_err("oversized WebUI token is rejected");

    assert_setup_validation(
        err,
        "webui_access_token",
        WebUiInboundValidationCode::InvalidValue,
    );
    assert_eq!(llm_config.upsert_provider_count(), 0);
    assert_eq!(llm_config.set_active_count(), 0);
}

#[tokio::test]
async fn upsert_llm_provider_allows_loopback_base_url_for_self_hosted() {
    // Loopback/private endpoints are the primary self-hosted use case (Ollama,
    // vLLM): the guard must let them through to the service, not reject them as
    // "internal". Only the always-blocked classes (metadata/link-local,
    // multicast, unspecified) are rejected — see the metadata cases above.
    let llm_config = Arc::new(SetupRecordingLlmConfigService::default());
    let services = services_with_setup_llm_config(llm_config.clone());

    services
        .invoke(
            caller(),
            CapabilityId::new(LLM_PROVIDER_UPSERT_CAPABILITY_ID).expect("capability id"),
            ProductCapabilityInput::llm_provider_upsert(UpsertLlmProviderRequest {
                id: "ollama".to_string(),
                client_action_id: None,
                name: None,
                adapter: "ollama".to_string(),
                base_url: Some("http://127.0.0.1:11434/v1".to_string()),
                default_model: None,
                api_key: None,
                set_active: false,
                model: None,
            }),
            ActivityId::new(),
        )
        .await
        .expect("loopback endpoint reaches the service");

    assert_eq!(llm_config.upsert_provider_count(), 1);
}

#[tokio::test]
async fn llm_config_mutations_are_available_as_product_capabilities() {
    let llm_config = Arc::new(SetupRecordingLlmConfigService::default());
    llm_config.use_active_snapshot("openai", "gpt-5-mini");
    let services = services_with_setup_llm_config(llm_config.clone());

    services
        .invoke(
            caller(),
            CapabilityId::new(LLM_PROVIDER_DELETE_CAPABILITY_ID).expect("capability id"),
            ProductCapabilityInput::json(json!({ "provider_id": "acme" })),
            ActivityId::new(),
        )
        .await
        .expect("delete provider");
    services
        .invoke(
            caller(),
            CapabilityId::new(LLM_ACTIVE_SET_CAPABILITY_ID).expect("capability id"),
            ProductCapabilityInput::json(json!({ "provider_id": "openai", "model": "gpt-5-mini" })),
            ActivityId::new(),
        )
        .await
        .expect("set active provider");

    assert_eq!(
        llm_config
            .delete_provider_calls
            .lock()
            .expect("lock")
            .as_slice(),
        ["acme"]
    );
    assert_eq!(
        llm_config.set_active_calls.lock().expect("lock").as_slice(),
        [SetupSetActiveCall {
            provider_id: "openai".to_string(),
            model: Some("gpt-5-mini".to_string()),
        }]
    );
}

#[tokio::test]
async fn operator_setup_run_is_available_as_product_capability() {
    let llm_config = Arc::new(SetupRecordingLlmConfigService::default());
    llm_config.use_active_snapshot("openai", "gpt-5-mini");
    let services = services_with_setup_llm_config(llm_config.clone());

    let resolution = services
        .invoke(
            caller(),
            CapabilityId::new(OPERATOR_SETUP_RUN_CAPABILITY_ID).expect("capability id"),
            ProductCapabilityInput::json(json!({
                "provider_id": "openai",
                "adapter": "open_ai_completions",
                "base_url": "https://api.example.test/v1",
                "model": "gpt-5-mini",
                "api_key": "sk-secret"
            })),
            ActivityId::new(),
        )
        .await
        .expect("operator setup capability");

    assert!(matches!(
        resolution,
        Resolution::Done(outcome) if outcome.verdict.is_success()
    ));
    assert_eq!(llm_config.snapshot_count(), 1);
    assert_eq!(
        llm_config
            .upsert_provider_calls
            .lock()
            .expect("lock")
            .as_slice(),
        [SetupUpsertCall {
            id: "openai".to_string(),
            adapter: "open_ai_completions".to_string(),
            base_url: Some("https://api.example.test/v1".to_string()),
            default_model: Some("gpt-5-mini".to_string()),
            api_key_set: true,
            set_active: true,
            model: Some("gpt-5-mini".to_string()),
        }]
    );
}

#[tokio::test]
async fn test_llm_connection_allows_loopback_base_url_for_self_hosted() {
    let llm_config = Arc::new(SetupRecordingLlmConfigService::default());
    let services = services_with_setup_llm_config(llm_config.clone());

    services
        .test_llm_connection(
            caller(),
            LlmProbeRequest {
                adapter: "ollama".to_string(),
                base_url: Some("http://127.0.0.1:11434/v1".to_string()),
                provider_id: "ollama".to_string(),
                model: Some("qwen3:latest".to_string()),
                api_key: None,
            },
        )
        .await
        .expect("loopback probe reaches the service");

    assert_eq!(llm_config.test_connection_count(), 1);
}

#[tokio::test]
async fn list_llm_models_allows_localhost_base_url_for_self_hosted() {
    // Regression: `validate_llm_base_url` used to reject `localhost`, breaking
    // the "Fetch models" button for self-hosted Ollama (the dialog showed
    // "Invalid value (base_url)").
    let llm_config = Arc::new(SetupRecordingLlmConfigService::default());
    let services = services_with_setup_llm_config(llm_config.clone());

    services
        .list_llm_models(
            caller(),
            LlmProbeRequest {
                adapter: "ollama".to_string(),
                base_url: Some("http://localhost:11434".to_string()),
                provider_id: "ollama".to_string(),
                model: None,
                api_key: None,
            },
        )
        .await
        .expect("localhost probe reaches the service");

    assert_eq!(llm_config.list_models_count(), 1);
}

#[tokio::test]
async fn list_llm_models_rejects_internal_base_url_before_service() {
    let llm_config = Arc::new(SetupRecordingLlmConfigService::default());
    let services = services_with_setup_llm_config(llm_config.clone());

    let err = services
        .list_llm_models(
            caller(),
            LlmProbeRequest {
                adapter: "open_ai_completions".to_string(),
                base_url: Some("http://169.254.169.254/latest/meta-data/".to_string()),
                provider_id: "openai".to_string(),
                model: Some("gpt-5-mini".to_string()),
                api_key: Some(SecretString::from("sk-secret".to_string())),
            },
        )
        .await
        .expect_err("metadata probe endpoint is rejected");

    assert_setup_validation(err, "base_url", WebUiInboundValidationCode::InvalidValue);
    assert_eq!(llm_config.list_models_count(), 0);
}

#[tokio::test]
async fn run_operator_setup_upserts_and_activates_provider_config() {
    let llm_config = Arc::new(SetupRecordingLlmConfigService::default());
    llm_config.use_active_snapshot("openai", "gpt-5-mini");
    let services = services_with_setup_llm_config(llm_config.clone());

    let response = services
        .run_operator_setup(
            caller(),
            RebornOperatorSetupRequest::default()
                .set_provider_id("openai")
                .set_adapter("open_ai_completions")
                .set_base_url("https://api.example.test/v1")
                .set_model("gpt-5-mini")
                .set_api_key(SecretString::from("sk-secret".to_string())),
        )
        .await
        .expect("setup response");

    assert_eq!(response.status, RebornOperatorSetupStatus::Complete);
    assert_eq!(llm_config.snapshot_count(), 1);
    assert_eq!(
        llm_config
            .upsert_provider_calls
            .lock()
            .expect("lock")
            .as_slice(),
        [SetupUpsertCall {
            id: "openai".to_string(),
            adapter: "open_ai_completions".to_string(),
            base_url: Some("https://api.example.test/v1".to_string()),
            default_model: Some("gpt-5-mini".to_string()),
            api_key_set: true,
            set_active: true,
            model: Some("gpt-5-mini".to_string()),
        }]
    );
    assert_eq!(llm_config.set_active_count(), 0);
}

#[tokio::test]
async fn run_operator_setup_ignores_redacted_webui_access_token_sentinel() {
    let llm_config = Arc::new(SetupRecordingLlmConfigService::default());
    llm_config.use_active_snapshot("openai", "gpt-5-mini");
    let services = services_with_setup_llm_config(llm_config.clone());

    let response = services
        .run_operator_setup(
            caller(),
            RebornOperatorSetupRequest::default()
                .set_provider_id("openai")
                .set_model("gpt-5-mini")
                .set_webui_access_token(SecretString::from("••••••••".to_string())),
        )
        .await
        .expect("setup response");

    assert_eq!(response.status, RebornOperatorSetupStatus::Complete);
    let webui_step = response
        .steps
        .iter()
        .find(|step| step.name == "webui_access")
        .expect("webui access step");
    assert_eq!(
        webui_step.status,
        ironclaw_product_workflow::RebornOperatorSetupStepStatus::Complete
    );
    assert_eq!(
        llm_config.set_active_calls.lock().expect("lock").as_slice(),
        [SetupSetActiveCall {
            provider_id: "openai".to_string(),
            model: Some("gpt-5-mini".to_string()),
        }]
    );
    let serialized = serde_json::to_string(&response).expect("serialize setup response");
    assert!(!serialized.contains("••••••••"));
}

#[tokio::test]
async fn run_operator_setup_rejects_unwired_host_mutations_before_provider_write() {
    let llm_config = Arc::new(SetupRecordingLlmConfigService::default());
    let services = services_with_setup_llm_config(llm_config.clone());

    let err = services
        .run_operator_setup(
            caller(),
            RebornOperatorSetupRequest::default()
                .set_provider_id("openai")
                .set_adapter("open_ai_completions")
                .set_profile_id("production")
                .set_webui_access_token(SecretString::from(
                    "webui-secret-token-value-32-bytes".to_string(),
                )),
        )
        .await
        .expect_err("unwired host mutations fail closed");

    assert_eq!(err.code, RebornServicesErrorCode::Unavailable);
    assert_eq!(err.kind, RebornServicesErrorKind::ServiceUnavailable);
    assert_eq!(err.status_code, 503);
    assert_eq!(llm_config.upsert_provider_count(), 0);
    assert_eq!(llm_config.set_active_count(), 0);
}

#[tokio::test]
async fn run_operator_setup_rejects_profile_only_host_mutation_before_provider_write() {
    let llm_config = Arc::new(SetupRecordingLlmConfigService::default());
    let services = services_with_setup_llm_config(llm_config.clone());

    let err = services
        .run_operator_setup(
            caller(),
            RebornOperatorSetupRequest::default()
                .set_provider_id("openai")
                .set_adapter("open_ai_completions")
                .set_profile_id("production"),
        )
        .await
        .expect_err("unwired profile mutation fails closed");

    assert_eq!(err.code, RebornServicesErrorCode::Unavailable);
    assert_eq!(err.kind, RebornServicesErrorKind::ServiceUnavailable);
    assert_eq!(err.status_code, 503);
    assert_eq!(llm_config.upsert_provider_count(), 0);
    assert_eq!(llm_config.set_active_count(), 0);
}

#[tokio::test]
async fn run_operator_setup_rejects_token_only_host_mutation_before_provider_write() {
    let llm_config = Arc::new(SetupRecordingLlmConfigService::default());
    let services = services_with_setup_llm_config(llm_config.clone());

    let err = services
        .run_operator_setup(
            caller(),
            RebornOperatorSetupRequest::default()
                .set_provider_id("openai")
                .set_adapter("open_ai_completions")
                .set_webui_access_token(SecretString::from(
                    "webui-secret-token-value-32-bytes".to_string(),
                )),
        )
        .await
        .expect_err("unwired WebUI token mutation fails closed");

    assert_eq!(err.code, RebornServicesErrorCode::Unavailable);
    assert_eq!(err.kind, RebornServicesErrorKind::ServiceUnavailable);
    assert_eq!(err.status_code, 503);
    assert_eq!(llm_config.upsert_provider_count(), 0);
    assert_eq!(llm_config.set_active_count(), 0);
}

#[tokio::test]
async fn run_operator_setup_selects_existing_provider_without_adapter() {
    let llm_config = Arc::new(SetupRecordingLlmConfigService::default());
    let services = services_with_setup_llm_config(llm_config.clone());

    services
        .run_operator_setup(
            caller(),
            RebornOperatorSetupRequest::default()
                .set_provider_id("openai")
                .set_model("gpt-5-mini"),
        )
        .await
        .expect("setup response");

    assert_eq!(llm_config.snapshot_count(), 1);
    assert_eq!(llm_config.upsert_provider_count(), 0);
    assert_eq!(
        llm_config.set_active_calls.lock().expect("lock").as_slice(),
        [SetupSetActiveCall {
            provider_id: "openai".to_string(),
            model: Some("gpt-5-mini".to_string()),
        }]
    );
}

#[tokio::test]
async fn run_operator_setup_without_provider_change_returns_snapshot() {
    let llm_config = Arc::new(SetupRecordingLlmConfigService::default());
    llm_config.use_active_snapshot("openai", "gpt-5-mini");
    let services = services_with_setup_llm_config(llm_config.clone());

    let response = services
        .run_operator_setup(caller(), RebornOperatorSetupRequest::default())
        .await
        .expect("setup response");

    assert_eq!(response.status, RebornOperatorSetupStatus::Complete);
    assert_eq!(llm_config.snapshot_count(), 1);
    assert_eq!(llm_config.upsert_provider_count(), 0);
    assert_eq!(llm_config.set_active_count(), 0);
}

#[tokio::test]
async fn run_operator_setup_propagates_llm_config_service_error() {
    let upsert_config = Arc::new(SetupRecordingLlmConfigService::default());
    upsert_config.fail_next_upsert(LlmConfigServiceError::Unavailable);
    let upsert_services = services_with_setup_llm_config(upsert_config);
    let upsert_err = upsert_services
        .run_operator_setup(
            caller(),
            RebornOperatorSetupRequest::default()
                .set_provider_id("openai")
                .set_adapter("open_ai_completions"),
        )
        .await
        .expect_err("upsert error propagates");
    assert_eq!(upsert_err.code, RebornServicesErrorCode::Unavailable);
    assert_eq!(upsert_err.status_code, 503);

    let set_active_config = Arc::new(SetupRecordingLlmConfigService::default());
    set_active_config.fail_next_set_active(LlmConfigServiceError::Unavailable);
    let set_active_services = services_with_setup_llm_config(set_active_config);
    let set_active_err = set_active_services
        .run_operator_setup(
            caller(),
            RebornOperatorSetupRequest::default().set_provider_id("openai"),
        )
        .await
        .expect_err("set_active error propagates");
    assert_eq!(set_active_err.code, RebornServicesErrorCode::Unavailable);
    assert_eq!(set_active_err.status_code, 503);

    let snapshot_config = Arc::new(SetupRecordingLlmConfigService::default());
    snapshot_config.fail_next_snapshot(LlmConfigServiceError::Unavailable);
    let snapshot_services = services_with_setup_llm_config(snapshot_config);
    let snapshot_err = snapshot_services
        .run_operator_setup(caller(), RebornOperatorSetupRequest::default())
        .await
        .expect_err("snapshot error propagates");
    assert_eq!(snapshot_err.code, RebornServicesErrorCode::Unavailable);
    assert_eq!(snapshot_err.status_code, 503);
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
        surface_kinds: Vec::new(),
        channel_directions: None,
        channel_connection: None,
        channel_presentation: None,
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
    // A failed run carries a model-visible `detail` (free-form backend cause
    // text, scrubbed only for secret VALUES). The public run-state DTO must
    // keep the user-facing `category` but strip `detail` so internal
    // diagnostics never reach the browser (see
    // `SanitizedFailure::public_projection`).
    coordinator.set_run_state_failure(
        SanitizedFailure::new("model_unavailable")
            .expect("valid category")
            .with_detail("HTTP 500 from provider at /internal/models/route-xyz"),
    );
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
    // The user-facing category survives; the model-visible detail is stripped.
    let failure = response.failure.as_ref().expect("failure present");
    assert_eq!(failure.category(), "model_unavailable");
    assert_eq!(
        failure.detail(),
        None,
        "public run-state DTO must not expose the model-visible failure detail"
    );
    assert!(response.checkpoint_id.is_none());
    assert_eq!(coordinator.run_state_request_count(), 1);

    // Stable DTO must not surface M3-internal binding refs, model route, raw
    // turn scope, or the internal failure detail to WebUI consumers.
    let rendered = serde_json::to_string(&response).expect("json");
    assert!(!rendered.contains("source_binding_ref"));
    assert!(!rendered.contains("reply_target_binding_ref"));
    assert!(!rendered.contains("resolved_model_route"));
    assert!(!rendered.contains("webui-src:replayed"));
    assert!(!rendered.contains("webui-reply:replayed"));
    assert!(!rendered.contains("\"scope\""));
    assert!(!rendered.contains("\"detail\""));
    assert!(!rendered.contains("/internal/models/route-xyz"));
    // With no reported usage, the token/cost fields are omitted entirely.
    assert!(!rendered.contains("\"usage\""));
    assert!(!rendered.contains("\"cost\""));
}

#[tokio::test]
async fn get_run_state_surfaces_token_usage_and_priced_cost() {
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    // A run that reported usage against a concrete (gpt-4o) model surfaces both
    // the token counts and a USD cost priced from the shared cost table.
    coordinator.set_run_state_usage(
        LoopModelUsage {
            input_tokens: 1_000,
            output_tokens: 500,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
        },
        LoopModelRouteSnapshot::new("openai", "gpt-4o", "config:v1", "auth:v1"),
    );
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

    let usage = response.usage.expect("token usage surfaced");
    assert_eq!(usage.input_tokens, 1_000);
    assert_eq!(usage.output_tokens, 500);

    let cost = response
        .cost
        .as_ref()
        .expect("cost priced from resolved model");
    // gpt-4o: input 0.0000025/tok → 1000 * 0.0000025 = 0.0025; output 0.00001/tok
    // → 500 * 0.00001 = 0.005; total 0.0075.
    assert_eq!(cost.input_cost_usd, "0.0025");
    assert_eq!(cost.output_cost_usd, "0.005");
    assert_eq!(cost.total_cost_usd, "0.0075");
    assert_eq!(cost.currency, "USD");

    // The resolved route stays internal even though its model id fed pricing.
    let rendered = serde_json::to_string(&response).expect("json");
    assert!(!rendered.contains("resolved_model_route"));
    assert!(rendered.contains("\"usage\""));
    assert!(rendered.contains("\"cost\""));
}

/// Stub [`ActiveModelReader`] returning a fixed active/default model id.
struct FixedActiveModelReader(Option<String>);

impl ActiveModelReader for FixedActiveModelReader {
    fn active_model_id(&self) -> Option<String> {
        self.0.clone()
    }
}

#[tokio::test]
async fn get_run_state_prices_default_model_run_against_active_model() {
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    // A default-model run: usage is reported but no route was resolved (the
    // caller sent no `model`). Pricing falls back to the runtime's live active
    // model so the run is still priced instead of reporting `usage` with no
    // `cost`.
    coordinator.set_run_state_usage_default_model(LoopModelUsage {
        input_tokens: 1_000,
        output_tokens: 500,
        cache_read_input_tokens: 0,
        cache_creation_input_tokens: 0,
    });
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        coordinator.clone(),
    )
    .with_active_model_reader(Arc::new(FixedActiveModelReader(Some("gpt-4o".to_string()))));
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

    let usage = response.usage.expect("token usage surfaced");
    assert_eq!(usage.input_tokens, 1_000);
    assert_eq!(usage.output_tokens, 500);

    // gpt-4o pricing, identical to the explicit-route case: input 1000 *
    // 0.0000025 = 0.0025; output 500 * 0.00001 = 0.005; total 0.0075.
    let cost = response
        .cost
        .as_ref()
        .expect("default-model run priced against the active model");
    assert_eq!(cost.input_cost_usd, "0.0025");
    assert_eq!(cost.output_cost_usd, "0.005");
    assert_eq!(cost.total_cost_usd, "0.0075");

    // The active model was only used to price; it is not leaked as a route.
    let rendered = serde_json::to_string(&response).expect("json");
    assert!(!rendered.contains("resolved_model_route"));
    assert!(!rendered.contains("gpt-4o"));
}

#[tokio::test]
async fn get_run_state_default_model_run_omits_cost_without_active_model() {
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    coordinator.set_run_state_usage_default_model(LoopModelUsage {
        input_tokens: 1_000,
        output_tokens: 500,
        cache_read_input_tokens: 0,
        cache_creation_input_tokens: 0,
    });
    // No active-model reader wired (and a reader that reports no concrete model
    // behaves the same): the run reports token usage but omits cost rather than
    // mispricing against a sentinel.
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        coordinator.clone(),
    )
    .with_active_model_reader(Arc::new(FixedActiveModelReader(None)));
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

    assert!(response.usage.is_some(), "token usage still surfaced");
    assert!(
        response.cost.is_none(),
        "cost omitted when no concrete model is available"
    );
    let rendered = serde_json::to_string(&response).expect("json");
    assert!(rendered.contains("\"usage\""));
    assert!(!rendered.contains("\"cost\""));
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
            RebornTimelineRequest::new("thread-paginate".to_string()).set_limit(10),
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
            RebornTimelineRequest::new("thread-cap".to_string()).set_limit(u32::MAX),
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

    let error = query_threads(&services, caller(), WebUiListThreadsRequest::default())
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

    let response = query_threads(&services, caller, WebUiListThreadsRequest::default())
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
async fn list_threads_needs_approval_returns_only_automation_threads_with_pending_approval() {
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    let caller = caller();
    let pending_thread_id = ThreadId::new("thread-pending").expect("pending thread id");
    let automation_pending_thread_id =
        setup_trigger_thread(&thread_service, &caller, "thread-automation-pending").await;
    let approval_service = Arc::new(ThreadScopedApprovalInteractionService {
        pending_thread_ids: [
            pending_thread_id.clone(),
            automation_pending_thread_id.clone(),
        ]
        .into_iter()
        .collect(),
    });
    let services = RebornServices::new(
        thread_service.clone(),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_automation_product_facade(automation_facade_with_trigger_thread(
        automation_pending_thread_id.clone(),
        &caller,
    ))
    .with_approval_interactions(approval_service);

    thread_service
        .ensure_thread(EnsureThreadRequest {
            scope: thread_scope_for(&caller),
            thread_id: Some(pending_thread_id.clone()),
            created_by_actor_id: caller.user_id.as_str().to_string(),
            title: Some("Normal chat pending approval".to_string()),
            metadata_json: Some(json!({ "source": "webui" }).to_string()),
        })
        .await
        .expect("normal pending thread");

    let response = query_threads(
        &services,
        caller,
        WebUiListThreadsRequest::default().set_needs_approval(true),
    )
    .await
    .expect("list approval threads");
    let thread_ids = response
        .threads
        .iter()
        .map(|thread| thread.thread_id.clone())
        .collect::<Vec<_>>();

    assert_eq!(
        thread_ids,
        vec![automation_pending_thread_id],
        "notification thread filter must exclude non-automation approval threads",
    );
}

#[tokio::test]
async fn list_threads_needs_approval_queries_pending_with_run_scope_shape() {
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    let caller = caller();
    let automation_pending_thread_id =
        setup_trigger_thread(&thread_service, &caller, "thread-automation-run-scope").await;
    let trigger_scope = trigger_run_thread_scope_for(&caller);
    let approval_service = Arc::new(ActorFallbackApprovalInteractionService {
        pending_thread_id: automation_pending_thread_id.clone(),
        tenant_id: caller.tenant_id.clone(),
        owner_user_id: trigger_scope.creator_user_id.clone(),
        agent_id: caller.agent_id.clone().expect("agent id"),
        project_id: caller.project_id.clone(),
    });
    let services = RebornServices::new(
        thread_service.clone(),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_automation_product_facade(automation_facade_with_trigger_thread(
        automation_pending_thread_id.clone(),
        &caller,
    ))
    .with_approval_interactions(approval_service);

    let response = query_threads(
        &services,
        caller,
        WebUiListThreadsRequest::default().set_needs_approval(true),
    )
    .await
    .expect("list approval threads");
    let thread_ids = response
        .threads
        .iter()
        .map(|thread| thread.thread_id.clone())
        .collect::<Vec<_>>();

    assert_eq!(
        thread_ids,
        vec![automation_pending_thread_id],
        "notification approval lookup must use the same actor-fallback turn scope shape as blocked run state",
    );
}

#[tokio::test]
async fn list_threads_needs_approval_uses_bounded_run_candidates() {
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    let caller = caller();
    let automation_pending_thread_id =
        setup_trigger_thread(&thread_service, &caller, "thread-automation-bounded").await;
    let trigger_scope = trigger_run_thread_scope_for(&caller);
    let approval_service = Arc::new(ActorFallbackApprovalInteractionService {
        pending_thread_id: automation_pending_thread_id.clone(),
        tenant_id: caller.tenant_id.clone(),
        owner_user_id: trigger_scope.creator_user_id.clone(),
        agent_id: caller.agent_id.clone().expect("agent id"),
        project_id: caller.project_id.clone(),
    });
    let automation_facade =
        automation_facade_with_trigger_thread(automation_pending_thread_id.clone(), &caller);
    let services = RebornServices::new(
        thread_service.clone(),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_automation_product_facade(automation_facade.clone())
    .with_approval_interactions(approval_service);

    let response = query_threads(
        &services,
        caller,
        WebUiListThreadsRequest::default().set_needs_approval(true),
    )
    .await
    .expect("list approval threads");

    assert_eq!(response.threads.len(), 1);
    let list_calls = automation_facade.list_calls();
    assert_eq!(list_calls.len(), 1);
    assert_eq!(list_calls[0].limit, 20);
    assert_eq!(list_calls[0].run_limit, 20);
    assert!(list_calls[0].include_completed);
    assert_eq!(
        automation_facade.resolve_calls(),
        vec![automation_pending_thread_id],
        "notification lookup should still resolve the thread id once to recover the true trigger creator scope",
    );
}

#[tokio::test]
async fn list_threads_needs_approval_finds_legacy_ownerless_automation_thread() {
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    let caller = caller();
    let automation_pending_thread_id =
        setup_ownerless_trigger_thread(&thread_service, &caller, "thread-automation-ownerless")
            .await;
    let trigger_scope = trigger_run_thread_scope_for(&caller);
    let approval_service = Arc::new(ActorFallbackApprovalInteractionService {
        pending_thread_id: automation_pending_thread_id.clone(),
        tenant_id: caller.tenant_id.clone(),
        owner_user_id: trigger_scope.creator_user_id.clone(),
        agent_id: caller.agent_id.clone().expect("agent id"),
        project_id: caller.project_id.clone(),
    });
    let services = RebornServices::new(
        thread_service.clone(),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_automation_product_facade(automation_facade_with_trigger_thread(
        automation_pending_thread_id.clone(),
        &caller,
    ))
    .with_approval_interactions(approval_service);

    let response = query_threads(
        &services,
        caller,
        WebUiListThreadsRequest::default().set_needs_approval(true),
    )
    .await
    .expect("list approval threads");
    let thread_ids = response
        .threads
        .iter()
        .map(|thread| thread.thread_id.clone())
        .collect::<Vec<_>>();

    assert_eq!(
        thread_ids,
        vec![automation_pending_thread_id],
        "notification approval lookup must include legacy ownerless automation run threads",
    );
}

#[tokio::test]
async fn list_threads_needs_approval_uses_automation_name_when_thread_title_missing() {
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    let caller = caller();
    let automation_pending_thread_id =
        ThreadId::new("thread-automation-titleless").expect("valid trigger thread id");
    thread_service
        .ensure_thread(EnsureThreadRequest {
            scope: trigger_thread_scope_for(&caller),
            thread_id: Some(automation_pending_thread_id.clone()),
            created_by_actor_id: "system".to_string(),
            title: None,
            metadata_json: Some(automation_trigger_thread_metadata_json(
                "trigger-gate-automation",
            )),
        })
        .await
        .expect("titleless trigger thread stored");
    let trigger_scope = trigger_run_thread_scope_for(&caller);
    let approval_service = Arc::new(ActorFallbackApprovalInteractionService {
        pending_thread_id: automation_pending_thread_id.clone(),
        tenant_id: caller.tenant_id.clone(),
        owner_user_id: trigger_scope.creator_user_id.clone(),
        agent_id: caller.agent_id.clone().expect("agent id"),
        project_id: caller.project_id.clone(),
    });
    let services = RebornServices::new(
        thread_service.clone(),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_automation_product_facade(automation_facade_with_trigger_thread(
        automation_pending_thread_id.clone(),
        &caller,
    ))
    .with_approval_interactions(approval_service);

    let response = query_threads(
        &services,
        caller,
        WebUiListThreadsRequest::default().set_needs_approval(true),
    )
    .await
    .expect("list approval threads");

    assert_eq!(response.threads.len(), 1);
    assert_eq!(
        response.threads[0].title.as_deref(),
        Some("Gate test automation"),
        "notification approval thread fallback title should use the automation name",
    );
}

#[tokio::test]
async fn list_threads_needs_approval_checks_candidate_automation_thread() {
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    let caller = caller();
    let automation_pending_thread_id =
        setup_trigger_thread(&thread_service, &caller, "thread-automation-candidate").await;
    let approval_service = Arc::new(ThreadScopedApprovalInteractionService {
        pending_thread_ids: [automation_pending_thread_id.clone()].into_iter().collect(),
    });
    let services = RebornServices::new(
        thread_service.clone(),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_automation_product_facade(Arc::new(
        StaticAutomationFacade::new(Vec::new()).with_resolve_scope_for_thread(
            automation_pending_thread_id.clone(),
            trigger_run_thread_scope_for(&caller),
        ),
    ))
    .with_approval_interactions(approval_service);

    let response = query_threads(
        &services,
        caller,
        WebUiListThreadsRequest::default()
            .set_needs_approval(true)
            .set_candidate_thread_id(automation_pending_thread_id.as_str()),
    )
    .await
    .expect("list approval threads");
    let thread_ids = response
        .threads
        .iter()
        .map(|thread| thread.thread_id.clone())
        .collect::<Vec<_>>();

    assert_eq!(
        thread_ids,
        vec![automation_pending_thread_id],
        "candidate thread must still be automation-authorized before entering notifications",
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
        created_at: None,
        updated_at: None,
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
        query_threads(
            &services,
            caller,
            WebUiListThreadsRequest::default().set_limit(2),
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
        created_at: None,
        updated_at: None,
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

    let response = query_threads(
        &services,
        caller,
        WebUiListThreadsRequest::default().set_limit(1),
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

    // Threads list newest-activity first, so create them oldest → newest:
    // second visible, then first visible, then the automation thread last.
    // That yields a candidate order of [automation, first, second], so the
    // facade has to skip the leading hidden automation thread while filling
    // the first page — the behavior under test. Waiting past each stamp
    // keeps the `created_at` order strict regardless of clock resolution.
    let second = thread_service
        .ensure_thread(EnsureThreadRequest {
            scope: thread_scope_for(&caller),
            thread_id: Some(second_visible_thread_id.clone()),
            created_by_actor_id: caller.user_id.as_str().to_string(),
            title: Some("Second visible chat".to_string()),
            metadata_json: Some(json!({ "source": "webui" }).to_string()),
        })
        .await
        .expect("second visible thread");
    wait_until_after(second.updated_at.expect("activity stamp")).await;
    let first = thread_service
        .ensure_thread(EnsureThreadRequest {
            scope: thread_scope_for(&caller),
            thread_id: Some(first_visible_thread_id.clone()),
            created_by_actor_id: caller.user_id.as_str().to_string(),
            title: Some("First visible chat".to_string()),
            metadata_json: Some(json!({ "source": "webui" }).to_string()),
        })
        .await
        .expect("first visible thread");
    wait_until_after(first.updated_at.expect("activity stamp")).await;
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

    let first_page = query_threads(
        &services,
        caller.clone(),
        WebUiListThreadsRequest::default().set_limit(1),
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

    let second_page = query_threads(
        &services,
        caller,
        WebUiListThreadsRequest::default()
            .set_limit(1)
            .set_cursor(first_page.next_cursor.expect("first visible page cursor")),
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

// ---------------------------------------------------------------------------
// Notice-text mapping: rejected_busy_notice maps TurnStatus to the right copy
// ---------------------------------------------------------------------------

#[tokio::test]
async fn rejected_busy_notice_blocked_approval_contains_approval_copy() {
    let threads: Arc<dyn SessionThreadService> = Arc::new(InMemorySessionThreadService::default());
    let coordinator = Arc::new(FakeTurnCoordinator::with_submit_error(
        TurnError::ThreadBusy(ironclaw_turns::ThreadBusy {
            active_run_id: TurnRunId::new(),
            status: TurnStatus::BlockedApproval,
            event_cursor: EventCursor(5),
        }),
    ));
    let services = RebornServices::new(threads, coordinator);
    create_thread_for(&services, caller(), "thread-notice").await;

    let response = services
        .submit_turn(
            caller(),
            serde_json::from_value::<WebUiSendMessageRequest>(json!({
                "client_action_id": "send-notice-approval",
                "thread_id": "thread-notice",
                "content": "hello"
            }))
            .expect("request"),
        )
        .await
        .expect("busy submit succeeds with RejectedBusy");

    match response {
        RebornSubmitTurnResponse::RejectedBusy {
            status: Some(status),
            notice,
            ..
        } => {
            assert_eq!(status, TurnStatus::BlockedApproval);
            assert_eq!(
                notice,
                "An approval gate is open on this thread — resolve it (approve or deny) before continuing, then resend your message."
            );
        }
        other => panic!("expected RejectedBusy, got {other:?}"),
    }
}

#[tokio::test]
async fn rejected_busy_notice_blocked_auth_contains_auth_copy() {
    let threads: Arc<dyn SessionThreadService> = Arc::new(InMemorySessionThreadService::default());
    let coordinator = Arc::new(FakeTurnCoordinator::with_submit_error(
        TurnError::ThreadBusy(ironclaw_turns::ThreadBusy {
            active_run_id: TurnRunId::new(),
            status: TurnStatus::BlockedAuth,
            event_cursor: EventCursor(5),
        }),
    ));
    let services = RebornServices::new(threads, coordinator);
    create_thread_for(&services, caller(), "thread-notice").await;

    let response = services
        .submit_turn(
            caller(),
            serde_json::from_value::<WebUiSendMessageRequest>(json!({
                "client_action_id": "send-notice-auth",
                "thread_id": "thread-notice",
                "content": "hello"
            }))
            .expect("request"),
        )
        .await
        .expect("busy submit succeeds with RejectedBusy");

    match response {
        RebornSubmitTurnResponse::RejectedBusy {
            status: Some(status),
            notice,
            ..
        } => {
            assert_eq!(status, TurnStatus::BlockedAuth);
            assert_eq!(
                notice,
                "An authentication gate is open on this thread — complete authentication before continuing, then resend your message."
            );
        }
        other => panic!("expected RejectedBusy, got {other:?}"),
    }
}

#[tokio::test]
async fn rejected_busy_notice_generic_status_contains_generic_copy() {
    let threads: Arc<dyn SessionThreadService> = Arc::new(InMemorySessionThreadService::default());
    let coordinator = Arc::new(FakeTurnCoordinator::with_submit_error(
        TurnError::ThreadBusy(ironclaw_turns::ThreadBusy {
            active_run_id: TurnRunId::new(),
            status: TurnStatus::Running,
            event_cursor: EventCursor(5),
        }),
    ));
    let services = RebornServices::new(threads, coordinator);
    create_thread_for(&services, caller(), "thread-notice").await;

    let response = services
        .submit_turn(
            caller(),
            serde_json::from_value::<WebUiSendMessageRequest>(json!({
                "client_action_id": "send-notice-generic",
                "thread_id": "thread-notice",
                "content": "hello"
            }))
            .expect("request"),
        )
        .await
        .expect("busy submit succeeds with RejectedBusy");

    match response {
        RebornSubmitTurnResponse::RejectedBusy {
            status: Some(status),
            notice,
            ..
        } => {
            assert_eq!(status, TurnStatus::Running);
            assert_eq!(
                notice,
                "Ironclaw is still working on a previous message — resend yours once the current task finishes."
            );
        }
        other => panic!("expected RejectedBusy, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Replay regression: a replayed RejectedBusy must return RejectedBusy again,
// never submit a new run (contract from PR #4838)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn replayed_rejected_busy_returns_rejected_busy_without_new_submission() {
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    // ScriptedThreadService pre-seeds the message as RejectedBusy — simulates
    // the client retrying after the original rejection response was lost.
    let services = RebornServices::new(
        Arc::new(ScriptedThreadService::rejected_busy_replay()),
        coordinator.clone(),
    );

    let response = services
        .submit_turn(
            caller(),
            serde_json::from_value::<WebUiSendMessageRequest>(json!({
                "client_action_id": "send-replay-rejected-busy",
                "thread_id": "thread-alpha",
                "content": "hello from webui"
            }))
            .expect("request"),
        )
        .await
        .expect("replayed RejectedBusy must succeed (not error)");

    assert!(
        matches!(response, RebornSubmitTurnResponse::RejectedBusy { .. }),
        "replay of RejectedBusy must return RejectedBusy, got {response:?}"
    );
    assert_eq!(
        coordinator.submission_count(),
        0,
        "a replayed RejectedBusy must not produce a new turn submission"
    );
}

// ---------------------------------------------------------------------------
// Option<> run-metadata contract: replay path yields None; fresh path yields Some
// ---------------------------------------------------------------------------

#[tokio::test]
async fn replayed_rejected_busy_returns_none_run_metadata() {
    // Replay: the original blocking run is gone — run metadata must be None,
    // not a fabricated run-id or status that the client cannot query.
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let services = RebornServices::new(
        Arc::new(ScriptedThreadService::rejected_busy_replay()),
        coordinator.clone(),
    );

    let response = services
        .submit_turn(
            caller(),
            serde_json::from_value::<WebUiSendMessageRequest>(json!({
                "client_action_id": "send-replay-none-metadata",
                "thread_id": "thread-alpha",
                "content": "replay with none metadata"
            }))
            .expect("request"),
        )
        .await
        .expect("replayed RejectedBusy must succeed");

    match response {
        RebornSubmitTurnResponse::RejectedBusy {
            active_run_id,
            status,
            event_cursor,
            notice,
            ..
        } => {
            assert!(
                active_run_id.is_none(),
                "replayed RejectedBusy must not fabricate active_run_id, got {active_run_id:?}"
            );
            assert!(
                status.is_none(),
                "replayed RejectedBusy must not fabricate status, got {status:?}"
            );
            assert!(
                event_cursor.is_none(),
                "replayed RejectedBusy must not fabricate event_cursor, got {event_cursor:?}"
            );
            assert!(
                !notice.is_empty(),
                "replayed RejectedBusy must carry a notice"
            );
        }
        other => panic!("expected RejectedBusy, got {other:?}"),
    }
    assert_eq!(
        coordinator.submission_count(),
        0,
        "replay must not produce a new turn submission"
    );
}

#[tokio::test]
async fn fresh_rejected_busy_returns_some_run_metadata() {
    // Fresh ThreadBusy: the blocking run is live — run metadata must be Some
    // with the real values so the client can poll the existing run.
    let active_run_id = TurnRunId::new();
    let coordinator = Arc::new(FakeTurnCoordinator::with_submit_error(
        TurnError::ThreadBusy(ironclaw_turns::ThreadBusy {
            active_run_id,
            status: TurnStatus::Running,
            event_cursor: EventCursor(7),
        }),
    ));
    let threads: Arc<dyn SessionThreadService> = Arc::new(InMemorySessionThreadService::default());
    let services = RebornServices::new(threads, coordinator.clone());
    create_thread_for(&services, caller(), "thread-busy-fresh").await;

    let response = services
        .submit_turn(
            caller(),
            serde_json::from_value::<WebUiSendMessageRequest>(json!({
                "client_action_id": "send-fresh-busy-metadata",
                "thread_id": "thread-busy-fresh",
                "content": "hello busy"
            }))
            .expect("request"),
        )
        .await
        .expect("fresh RejectedBusy must succeed");

    match response {
        RebornSubmitTurnResponse::RejectedBusy {
            active_run_id: returned_run_id,
            status: returned_status,
            event_cursor: returned_cursor,
            notice,
            ..
        } => {
            assert_eq!(
                returned_run_id,
                Some(active_run_id),
                "fresh RejectedBusy must carry the real blocking run id"
            );
            assert_eq!(
                returned_status,
                Some(TurnStatus::Running),
                "fresh RejectedBusy must carry the real blocking run status"
            );
            assert_eq!(
                returned_cursor,
                Some(EventCursor(7)),
                "fresh RejectedBusy must carry the real event cursor"
            );
            assert!(!notice.is_empty(), "fresh RejectedBusy must carry a notice");
        }
        other => panic!("expected RejectedBusy, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Mark-failure reconcile path: mark_message_rejected_busy errors → replay
// confirms RejectedBusy → no error surfaces, RejectedBusy returned
// ---------------------------------------------------------------------------

#[tokio::test]
async fn rejected_busy_mark_failure_reconciles_via_replay_and_returns_rejected_busy() {
    // Arrange: coordinator returns ThreadBusy so the busy path fires; the
    // scripted thread service makes mark_message_rejected_busy fail and then
    // supplies a RejectedBusy replay on the reconcile probe so
    // reconcile_terminal_duplicate settles the race without propagating the error.
    let active_run_id = TurnRunId::new();
    let coordinator = Arc::new(FakeTurnCoordinator::with_submit_error(
        TurnError::ThreadBusy(ironclaw_turns::ThreadBusy {
            active_run_id,
            status: TurnStatus::Running,
            event_cursor: EventCursor(3),
        }),
    ));
    let services = RebornServices::new(
        Arc::new(ScriptedThreadService::rejected_busy_mark_fails()),
        coordinator,
    );

    // Act: submit a fresh turn against thread-alpha (which the scripted service
    // owns); coordinator fires ThreadBusy, mark fails, reconcile replays.
    let response = services
        .submit_turn(
            caller(),
            serde_json::from_value::<WebUiSendMessageRequest>(json!({
                "client_action_id": "send-mark-fail-reconcile",
                "thread_id": "thread-alpha",
                "content": "hello mark-fail"
            }))
            .expect("request"),
        )
        .await
        .expect("mark-failure reconcile must succeed (not error)");

    // Assert: the mark error must NOT propagate to the caller — reconcile_terminal_duplicate
    // replays the accepted message, sees RejectedBusy, and returns Ok(()).
    // The response is built from the original ThreadBusy metadata (active_run_id,
    // status, event_cursor), proving the full path ran without dropping state.
    match response {
        RebornSubmitTurnResponse::RejectedBusy {
            active_run_id: returned_run_id,
            status: returned_status,
            event_cursor: returned_cursor,
            notice,
            ..
        } => {
            assert_eq!(
                returned_run_id,
                Some(active_run_id),
                "mark-failure reconcile must carry the real blocking run id from ThreadBusy"
            );
            assert_eq!(
                returned_status,
                Some(TurnStatus::Running),
                "mark-failure reconcile must carry the real blocking run status"
            );
            assert_eq!(
                returned_cursor,
                Some(EventCursor(3)),
                "mark-failure reconcile must carry the real event cursor"
            );
            assert!(!notice.is_empty(), "RejectedBusy must carry a notice");
        }
        other => {
            panic!("mark-failure reconcile must return RejectedBusy (not error), got {other:?}")
        }
    }
}

// ---------------------------------------------------------------------------
// Legacy DeferredBusy mark-failure reconcile path: mark_message_rejected_busy errors
// → replay returns legacy DeferredBusy (non-terminal) → predicate does NOT match
// → original mark error surfaces as Unavailable, not a false-terminal RejectedBusy
// ---------------------------------------------------------------------------

#[tokio::test]
async fn legacy_deferred_busy_mark_failure_surfaces_error_not_false_terminal() {
    // Arrange: coordinator returns ThreadBusy so the busy path fires; the
    // scripted thread service makes mark_message_rejected_busy fail and then
    // supplies a legacy DeferredBusy replay on the reconcile probe.
    // DeferredBusy is non-terminal — reconcile_terminal_duplicate must NOT
    // accept it as settled.  The predicate now matches only RejectedBusy, so
    // the `_ =>` arm propagates the original mark failure as an error.
    let active_run_id = TurnRunId::new();
    let coordinator = Arc::new(FakeTurnCoordinator::with_submit_error(
        TurnError::ThreadBusy(ironclaw_turns::ThreadBusy {
            active_run_id,
            status: TurnStatus::Running,
            event_cursor: EventCursor(3),
        }),
    ));
    let services = RebornServices::new(
        Arc::new(ScriptedThreadService::deferred_busy_mark_fails()),
        coordinator,
    );

    // Act: submit a fresh turn against thread-alpha; coordinator fires ThreadBusy,
    // mark_message_rejected_busy fails, reconcile sees legacy DeferredBusy which
    // no longer matches → the original mark error must propagate.
    let error = services
        .submit_turn(
            caller(),
            serde_json::from_value::<WebUiSendMessageRequest>(json!({
                "client_action_id": "send-deferred-busy-mark-fail-reconcile",
                "thread_id": "thread-alpha",
                "content": "hello deferred-busy mark-fail"
            }))
            .expect("request"),
        )
        .await
        .expect_err(
            "legacy DeferredBusy reconcile must surface the mark failure as an error, \
             not silently return a false-terminal RejectedBusy",
        );

    // Assert: SessionThreadError::Backend maps to service_unavailable(true) —
    // code=Unavailable, status_code=503, retryable=true.
    assert_eq!(
        error.code,
        RebornServicesErrorCode::Unavailable,
        "DeferredBusy reconcile miss must surface the backend mark failure (Unavailable), got {error:?}",
    );
    assert_eq!(
        error.status_code, 503,
        "DeferredBusy reconcile miss must return 503, got {error:?}",
    );
    assert!(
        error.retryable,
        "backend mark failure is retryable, got {error:?}",
    );
}

/// Test lander that records what it was asked to land and returns a ref per
/// attachment with a deterministic `storage_key`, so the facade test can assert
/// both that decode→land ran and that the returned refs reach the transcript.
#[derive(Default)]
struct RecordingLander {
    landed: Mutex<Vec<(ThreadScope, String, Vec<InboundAttachment>)>>,
}

#[async_trait]
impl InboundAttachmentLander for RecordingLander {
    async fn land(
        &self,
        thread_scope: &ThreadScope,
        message_id: &str,
        attachments: Vec<InboundAttachment>,
    ) -> Result<Vec<AttachmentRef>, RebornServicesError> {
        let refs = attachments
            .iter()
            .enumerate()
            .map(|(index, attachment)| AttachmentRef {
                id: attachment.id.clone(),
                // The real bridge derives kind from the MIME type; mirror that.
                kind: ironclaw_common::kind_for_mime(&attachment.mime_type),
                mime_type: attachment.mime_type.clone(),
                filename: attachment.filename.clone(),
                size_bytes: Some(attachment.bytes.len() as u64),
                storage_key: Some(format!(
                    "/workspace/attachments/test/{message_id}-{index}-landed"
                )),
                extracted_text: None,
            })
            .collect();
        self.landed.lock().expect("lander mutex").push((
            thread_scope.clone(),
            message_id.to_string(),
            attachments,
        ));
        Ok(refs)
    }
}

#[tokio::test]
async fn submit_turn_lands_attachments_and_persists_refs_on_the_user_message() {
    use base64::Engine;

    let threads: Arc<dyn SessionThreadService> = Arc::new(InMemorySessionThreadService::default());
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let lander = Arc::new(RecordingLander::default());
    let services = RebornServices::new(Arc::clone(&threads), coordinator.clone())
        .with_inbound_attachments(lander.clone());
    create_thread_for(&services, caller(), "thread-alpha").await;

    let pdf_b64 = base64::engine::general_purpose::STANDARD.encode(b"%PDF-1.7 body");
    services
        .submit_turn(
            caller(),
            serde_json::from_value::<WebUiSendMessageRequest>(json!({
                "client_action_id": "send-att",
                "thread_id": "thread-alpha",
                "content": "see attached",
                "attachments": [{
                    "mime_type": "application/pdf",
                    "filename": "report.pdf",
                    "data_base64": pdf_b64,
                }],
            }))
            .expect("request"),
        )
        .await
        .expect("submit succeeds");

    // The lander was invoked with the caller-derived thread scope plus the
    // decoded attachment bytes + metadata.
    {
        let landed = lander.landed.lock().expect("lander mutex");
        assert_eq!(landed.len(), 1);
        assert_eq!(landed[0].0, thread_scope_for(&caller()));
        assert_eq!(landed[0].2.len(), 1);
        assert_eq!(landed[0].2[0].mime_type, "application/pdf");
        assert_eq!(landed[0].2[0].filename.as_deref(), Some("report.pdf"));
        assert_eq!(landed[0].2[0].bytes, b"%PDF-1.7 body");
    }

    // The returned refs are persisted on the accepted user message.
    let history = threads
        .list_thread_history(ThreadHistoryRequest {
            scope: thread_scope_for(&caller()),
            thread_id: ThreadId::new("thread-alpha").unwrap(),
        })
        .await
        .expect("history");
    let user_message = history
        .messages
        .iter()
        .find(|message| message.kind == MessageKind::User)
        .expect("user message present");
    assert_eq!(user_message.content.as_deref(), Some("see attached"));
    assert_eq!(user_message.attachments.len(), 1);
    let attachment_ref = &user_message.attachments[0];
    assert_eq!(attachment_ref.kind, AttachmentKind::Document);
    assert_eq!(attachment_ref.mime_type, "application/pdf");
    assert_eq!(attachment_ref.filename.as_deref(), Some("report.pdf"));
    assert!(
        attachment_ref
            .storage_key
            .as_deref()
            .is_some_and(|key| key.ends_with("-landed")),
        "expected landed storage_key, got {:?}",
        attachment_ref.storage_key
    );
}

#[tokio::test]
async fn get_timeline_returns_attachment_refs_on_the_user_message() {
    use base64::Engine;

    // The browser renders attachment cards from the timeline, and they must
    // survive a page refresh. The browser's surface is `get_timeline`, not
    // `list_thread_history`, so drive that path (test through the caller) and
    // assert the projected `ThreadMessageRecord` still carries the refs.
    let threads: Arc<dyn SessionThreadService> = Arc::new(InMemorySessionThreadService::default());
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let lander = Arc::new(RecordingLander::default());
    let services = RebornServices::new(Arc::clone(&threads), coordinator.clone())
        .with_inbound_attachments(lander.clone());
    create_thread_for(&services, caller(), "thread-alpha").await;

    let csv_b64 = base64::engine::general_purpose::STANDARD.encode(b"a,b\n1,2\n");
    services
        .submit_turn(
            caller(),
            serde_json::from_value::<WebUiSendMessageRequest>(json!({
                "client_action_id": "send-att",
                "thread_id": "thread-alpha",
                "content": "spreadsheet attached",
                "attachments": [{
                    "mime_type": "text/csv",
                    "filename": "data.csv",
                    "data_base64": csv_b64,
                }],
            }))
            .expect("request"),
        )
        .await
        .expect("submit succeeds");

    let timeline = services
        .get_timeline(
            caller(),
            RebornTimelineRequest::new("thread-alpha".to_string()),
        )
        .await
        .expect("timeline");

    let user_message = timeline
        .messages
        .iter()
        .find(|message| message.kind == MessageKind::User)
        .expect("user message present in timeline");
    assert_eq!(user_message.attachments.len(), 1);
    let attachment_ref = &user_message.attachments[0];
    assert_eq!(attachment_ref.kind, AttachmentKind::Document);
    assert_eq!(attachment_ref.mime_type, "text/csv");
    assert_eq!(attachment_ref.filename.as_deref(), Some("data.csv"));
    assert!(
        attachment_ref
            .storage_key
            .as_deref()
            .is_some_and(|key| !key.is_empty()),
        "timeline ref must carry a non-empty storage_key so the agent can re-read it later"
    );
}

#[tokio::test]
async fn submit_turn_rejects_attachments_when_no_lander_is_wired() {
    use base64::Engine;

    let threads: Arc<dyn SessionThreadService> = Arc::new(InMemorySessionThreadService::default());
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    // No `.with_inbound_attachments(...)`: a deployment without attachment
    // support must reject rather than silently drop the files.
    let services = RebornServices::new(threads, coordinator);
    create_thread_for(&services, caller(), "thread-alpha").await;

    let pdf_b64 = base64::engine::general_purpose::STANDARD.encode(b"%PDF-1.7");
    let err = services
        .submit_turn(
            caller(),
            serde_json::from_value::<WebUiSendMessageRequest>(json!({
                "client_action_id": "send-att",
                "thread_id": "thread-alpha",
                "content": "see attached",
                "attachments": [{
                    "mime_type": "application/pdf",
                    "data_base64": pdf_b64,
                }],
            }))
            .expect("request"),
        )
        .await
        .expect_err("attachments without a lander must be rejected");
    assert_eq!(err.kind, RebornServicesErrorKind::ServiceUnavailable);
}

// ---------------------------------------------------------------------------
// Admin user management: facade authorization + last-admin protection.
//
// Drives the facade methods through a fake `AdminUserService` port so the
// load-bearing NEW logic — role-based authorization (read every request),
// operator bypass, and last-admin protection — is tested through the caller.
// The composition adapter over the real identity store is thin mapping;
// crate-tier is the reachable tier here because the integration harness does
// not wire the admin service (no token minter in-harness).
// ---------------------------------------------------------------------------

fn admin_record(user_id: &str, role: AdminUserRole, status: AdminUserStatus) -> AdminUserRecord {
    AdminUserRecord {
        user_id: UserId::new(user_id).expect("user id"),
        email: None,
        display_name: None,
        status,
        role,
        created_at: "2026-07-07T00:00:00Z".to_string(),
        updated_at: "2026-07-07T00:00:00Z".to_string(),
        created_by: None,
        last_login_at: None,
        metadata: std::collections::BTreeMap::new(),
    }
}

#[derive(Default)]
struct FakeAdminUsers {
    users: Mutex<HashMap<String, AdminUserRecord>>,
}

impl FakeAdminUsers {
    fn with(records: impl IntoIterator<Item = AdminUserRecord>) -> Self {
        let map = records
            .into_iter()
            .map(|record| (record.user_id.as_str().to_string(), record))
            .collect();
        Self {
            users: Mutex::new(map),
        }
    }
}

#[async_trait]
impl AdminUserService for FakeAdminUsers {
    async fn list_users(
        &self,
        _tenant: &TenantId,
        status: Option<AdminUserStatus>,
        after: Option<&UserId>,
        limit: usize,
    ) -> Result<Vec<AdminUserRecord>, AdminUserError> {
        // Mirror the real port contract: status filter, then user_id-ascending
        // order, then the `after` cursor, then bound to `limit`.
        let mut records: Vec<AdminUserRecord> = self
            .users
            .lock()
            .unwrap()
            .values()
            .filter(|record| status.is_none_or(|want| record.status == want))
            .cloned()
            .collect();
        records.sort_by(|a, b| a.user_id.as_str().cmp(b.user_id.as_str()));
        let after = after.map(UserId::as_str);
        Ok(records
            .into_iter()
            .filter(|record| after.is_none_or(|cursor| record.user_id.as_str() > cursor))
            .take(limit)
            .collect())
    }

    async fn get_user(
        &self,
        _tenant: &TenantId,
        user_id: &UserId,
    ) -> Result<Option<AdminUserRecord>, AdminUserError> {
        Ok(self.users.lock().unwrap().get(user_id.as_str()).cloned())
    }

    async fn create_user(
        &self,
        _tenant: &TenantId,
        _actor: &UserId,
        fields: AdminCreateUserFields,
    ) -> Result<AdminCreatedUser, AdminUserError> {
        let record = admin_record("created-user", fields.role, AdminUserStatus::Active);
        self.users
            .lock()
            .unwrap()
            .insert("created-user".to_string(), record.clone());
        Ok(AdminCreatedUser {
            record,
            api_token: SecretString::from("minted-token"),
        })
    }

    async fn update_profile(
        &self,
        _tenant: &TenantId,
        user_id: &UserId,
        display_name: Option<String>,
        _metadata: Option<std::collections::BTreeMap<String, String>>,
    ) -> Result<AdminUserRecord, AdminUserError> {
        let mut users = self.users.lock().unwrap();
        let record = users
            .get_mut(user_id.as_str())
            .ok_or(AdminUserError::NotFound)?;
        if display_name.is_some() {
            record.display_name = display_name;
        }
        Ok(record.clone())
    }

    async fn set_status(
        &self,
        _tenant: &TenantId,
        user_id: &UserId,
        status: AdminUserStatus,
    ) -> Result<AdminUserRecord, AdminUserError> {
        let mut users = self.users.lock().unwrap();
        let record = users
            .get_mut(user_id.as_str())
            .ok_or(AdminUserError::NotFound)?;
        record.status = status;
        Ok(record.clone())
    }

    async fn set_role(
        &self,
        _tenant: &TenantId,
        user_id: &UserId,
        role: AdminUserRole,
    ) -> Result<AdminUserRecord, AdminUserError> {
        let mut users = self.users.lock().unwrap();
        let record = users
            .get_mut(user_id.as_str())
            .ok_or(AdminUserError::NotFound)?;
        record.role = role;
        Ok(record.clone())
    }

    async fn delete_user(
        &self,
        _tenant: &TenantId,
        user_id: &UserId,
    ) -> Result<(), AdminUserError> {
        self.users.lock().unwrap().remove(user_id.as_str());
        Ok(())
    }

    async fn count_active_admins(&self, _tenant: &TenantId) -> Result<usize, AdminUserError> {
        Ok(self
            .users
            .lock()
            .unwrap()
            .values()
            .filter(|record| record.status == AdminUserStatus::Active && record.role.is_admin())
            .count())
    }

    async fn list_secrets(
        &self,
        _tenant: &TenantId,
        _user_id: &UserId,
    ) -> Result<Vec<AdminUserSecretMeta>, AdminUserError> {
        Ok(Vec::new())
    }

    async fn put_secret(
        &self,
        _tenant: &TenantId,
        _user_id: &UserId,
        handle: SecretHandle,
        _material: SecretString,
    ) -> Result<AdminUserSecretMeta, AdminUserError> {
        Ok(AdminUserSecretMeta {
            handle: handle.as_str().to_string(),
            created_at: None,
            updated_at: None,
        })
    }

    async fn delete_secret(
        &self,
        _tenant: &TenantId,
        _user_id: &UserId,
        _handle: SecretHandle,
    ) -> Result<bool, AdminUserError> {
        Ok(true)
    }
}

fn admin_services(fake: FakeAdminUsers) -> RebornServices {
    RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_admin_user_service(Arc::new(fake))
}

fn assert_forbidden(err: RebornServicesError) {
    assert_eq!(err.status_code, 403, "expected a 403 authorization failure");
    assert_eq!(err.code, RebornServicesErrorCode::Forbidden);
}

#[tokio::test]
async fn admin_users_are_available_as_product_views_and_capabilities() {
    let services = admin_services(FakeAdminUsers::with([
        admin_record("user-alpha", AdminUserRole::Admin, AdminUserStatus::Active),
        admin_record("user-beta", AdminUserRole::Member, AdminUserStatus::Active),
    ]));
    let target = UserId::new("user-beta").expect("user");

    // safety: these are ProductSurface facade query calls in a contract test;
    // no database transaction is involved.
    let users = services
        .query(
            caller(),
            ADMIN_USERS_VIEW
                .query(
                    RebornAdminUserListQuery {
                        limit: Some(2),
                        ..Default::default()
                    },
                    None,
                )
                .expect("admin users query"),
        )
        .await
        .expect("admin users view");
    let users: RebornAdminUserListResponse =
        serde_json::from_value(users.payload).expect("admin users payload");
    assert_eq!(users.users.len(), 2);
    assert_eq!(users.users[1].user_id.as_str(), "user-beta");
    assert_eq!(users.next_cursor.as_deref(), Some("user-beta"));

    let user = services
        .query(
            caller(),
            ADMIN_USER_VIEW
                .query(
                    RebornAdminUserRequest {
                        user_id: target.clone(),
                    },
                    None,
                )
                .expect("admin user query"),
        )
        .await
        .expect("admin user view");
    let user: RebornAdminUserResponse =
        serde_json::from_value(user.payload).expect("admin user payload");
    assert_eq!(user.user.user_id.as_str(), "user-beta");

    let secrets = services
        .query(
            caller(),
            ADMIN_USER_SECRETS_VIEW
                .query(
                    RebornAdminUserRequest {
                        user_id: target.clone(),
                    },
                    None,
                )
                .expect("admin user secrets query"),
        )
        .await
        .expect("admin user secrets view");
    let secrets: RebornAdminUserSecretsListResponse =
        serde_json::from_value(secrets.payload).expect("admin user secrets payload");
    assert!(secrets.secrets.is_empty());

    let update_resolution = services
        .invoke(
            caller(),
            CapabilityId::new(ADMIN_USER_UPDATE_CAPABILITY_ID).expect("capability id"),
            ProductCapabilityInput::json(
                serde_json::to_value(RebornAdminUpdateUserProductRequest {
                    user_id: target.clone(),
                    display_name: Some("Beta Renamed".to_string()),
                    metadata: None,
                })
                .expect("admin update input"),
            ),
            ActivityId::new(),
        )
        .await
        .expect("admin update capability");
    assert!(matches!(
        update_resolution,
        Resolution::Done(outcome) if outcome.verdict.is_success()
    ));

    let updated = services
        .query(
            caller(),
            ADMIN_USER_VIEW
                .query(
                    RebornAdminUserRequest {
                        user_id: target.clone(),
                    },
                    None,
                )
                .expect("admin user query"),
        )
        .await
        .expect("admin user view after update");
    let updated: RebornAdminUserResponse =
        serde_json::from_value(updated.payload).expect("admin user payload");
    assert_eq!(updated.user.display_name.as_deref(), Some("Beta Renamed"));

    let put_secret_resolution = services
        .invoke(
            caller(),
            CapabilityId::new(ADMIN_USER_PUT_SECRET_CAPABILITY_ID).expect("capability id"),
            ProductCapabilityInput::json(
                serde_json::to_value(RebornAdminPutSecretProductRequest {
                    user_id: target.clone(),
                    handle: "openai_api_key".to_string(),
                    value: "sk-test".to_string(),
                })
                .expect("admin secret put input"),
            ),
            ActivityId::new(),
        )
        .await
        .expect("admin secret put capability");
    assert!(matches!(
        put_secret_resolution,
        Resolution::Done(outcome) if outcome.verdict.is_success()
    ));

    let delete_secret_resolution = services
        .invoke(
            caller(),
            CapabilityId::new(ADMIN_USER_DELETE_SECRET_CAPABILITY_ID).expect("capability id"),
            ProductCapabilityInput::json(
                serde_json::to_value(RebornAdminDeleteSecretProductRequest {
                    user_id: target.clone(),
                    handle: "openai_api_key".to_string(),
                })
                .expect("admin secret delete input"),
            ),
            ActivityId::new(),
        )
        .await
        .expect("admin secret delete capability");
    assert!(matches!(
        delete_secret_resolution,
        Resolution::Done(outcome) if outcome.verdict.is_success()
    ));

    let status_resolution = services
        .invoke(
            caller(),
            CapabilityId::new(ADMIN_USER_SET_STATUS_CAPABILITY_ID).expect("capability id"),
            ProductCapabilityInput::json(
                serde_json::to_value(RebornAdminSetStatusProductRequest {
                    user_id: target.clone(),
                    status: AdminUserStatus::Suspended,
                })
                .expect("admin status input"),
            ),
            ActivityId::new(),
        )
        .await
        .expect("admin status capability");
    assert!(matches!(
        status_resolution,
        Resolution::Done(outcome) if outcome.verdict.is_success()
    ));

    let suspended = services
        .query(
            caller(),
            ADMIN_USER_VIEW
                .query(
                    RebornAdminUserRequest {
                        user_id: target.clone(),
                    },
                    None,
                )
                .expect("admin user query"),
        )
        .await
        .expect("admin user view after status");
    let suspended: RebornAdminUserResponse =
        serde_json::from_value(suspended.payload).expect("admin user payload");
    assert_eq!(suspended.user.status, AdminUserStatus::Suspended);

    let role_resolution = services
        .invoke(
            caller(),
            CapabilityId::new(ADMIN_USER_SET_ROLE_CAPABILITY_ID).expect("capability id"),
            ProductCapabilityInput::json(
                serde_json::to_value(RebornAdminSetRoleProductRequest {
                    user_id: target.clone(),
                    role: AdminUserRole::Admin,
                })
                .expect("admin role input"),
            ),
            ActivityId::new(),
        )
        .await
        .expect("admin role capability");
    assert!(matches!(
        role_resolution,
        Resolution::Done(outcome) if outcome.verdict.is_success()
    ));

    let promoted = services
        .query(
            caller(),
            ADMIN_USER_VIEW
                .query(
                    RebornAdminUserRequest {
                        user_id: target.clone(),
                    },
                    None,
                )
                .expect("admin user query"),
        )
        .await
        .expect("admin user view after role");
    let promoted: RebornAdminUserResponse =
        serde_json::from_value(promoted.payload).expect("admin user payload");
    assert_eq!(promoted.user.role, AdminUserRole::Admin);

    let delete_resolution = services
        .invoke(
            caller(),
            CapabilityId::new(ADMIN_USER_DELETE_CAPABILITY_ID).expect("capability id"),
            ProductCapabilityInput::json(
                serde_json::to_value(RebornAdminUserRequest {
                    user_id: target.clone(),
                })
                .expect("admin delete input"),
            ),
            ActivityId::new(),
        )
        .await
        .expect("admin delete capability");
    assert!(matches!(
        delete_resolution,
        Resolution::Done(outcome) if outcome.verdict.is_success()
    ));

    let deleted = services
        .query(
            caller(),
            ADMIN_USER_VIEW
                .query(RebornAdminUserRequest { user_id: target }, None)
                .expect("admin user query"),
        )
        .await
        .expect_err("deleted user is no longer readable");
    assert_eq!(deleted.code, RebornServicesErrorCode::NotFound);
}

/// Drive EVERY admin verb through the facade and assert each is a 403.
/// `authorize_admin` is a predicate that gates side effects, so it must be
/// tested at every call site — not just `list` (.claude/rules/testing.md,
/// "test through the caller"): a verb that forgot to call it would be an
/// unauthorized read/mutation hole invisible to a single-endpoint test.
async fn assert_every_admin_verb_forbidden(services: &RebornServices) {
    let target = UserId::new("some-target").expect("user");
    assert_forbidden(
        services
            .list_admin_users(caller(), RebornAdminUserListQuery::default())
            .await
            .expect_err("list"),
    );
    assert_forbidden(
        services
            .get_admin_user(caller(), target.clone())
            .await
            .expect_err("get"),
    );
    assert_forbidden(
        services
            .create_admin_user(
                caller(),
                RebornAdminCreateUserRequest {
                    email: None,
                    display_name: None,
                    role: AdminUserRole::Member,
                },
            )
            .await
            .expect_err("create"),
    );
    assert_forbidden(
        services
            .update_admin_user(
                caller(),
                target.clone(),
                RebornAdminUpdateUserRequest::default(),
            )
            .await
            .expect_err("update"),
    );
    assert_forbidden(
        services
            .set_admin_user_status(
                caller(),
                target.clone(),
                RebornAdminSetStatusRequest {
                    status: AdminUserStatus::Suspended,
                },
            )
            .await
            .expect_err("status"),
    );
    assert_forbidden(
        services
            .set_admin_user_role(
                caller(),
                target.clone(),
                RebornAdminSetRoleRequest {
                    role: AdminUserRole::Admin,
                },
            )
            .await
            .expect_err("role"),
    );
    assert_forbidden(
        services
            .delete_admin_user(caller(), target.clone())
            .await
            .expect_err("delete"),
    );
    assert_forbidden(
        services
            .list_admin_user_secrets(caller(), target.clone())
            .await
            .expect_err("list_secrets"),
    );
    assert_forbidden(
        services
            .put_admin_user_secret(
                caller(),
                target.clone(),
                SecretHandle::new("handle").unwrap(),
                RebornAdminPutSecretRequest {
                    value: "v".to_string(),
                },
            )
            .await
            .expect_err("put_secret"),
    );
    assert_forbidden(
        services
            .delete_admin_user_secret(caller(), target, SecretHandle::new("handle").unwrap())
            .await
            .expect_err("delete_secret"),
    );
}

#[tokio::test]
async fn admin_member_caller_is_forbidden_on_every_verb() {
    // caller() resolves to user-alpha; seeded as a plain member → 403 on EVERY
    // admin verb, not just list.
    let services = admin_services(FakeAdminUsers::with([admin_record(
        "user-alpha",
        AdminUserRole::Member,
        AdminUserStatus::Active,
    )]));
    assert_every_admin_verb_forbidden(&services).await;
    // Self-privilege-escalation: a member cannot promote their own record.
    assert_forbidden(
        services
            .set_admin_user_role(
                caller(),
                UserId::new("user-alpha").expect("user"),
                RebornAdminSetRoleRequest {
                    role: AdminUserRole::Admin,
                },
            )
            .await
            .expect_err("a member must not promote themselves"),
    );
}

#[tokio::test]
async fn admin_unknown_caller_is_forbidden_on_every_verb() {
    // The caller has no user record at all. Same 403 as a member — the facade
    // must never leak (via a different status/code) whether the caller record
    // exists but is under-privileged vs. does not exist.
    let services = admin_services(FakeAdminUsers::default());
    assert_every_admin_verb_forbidden(&services).await;
}

#[tokio::test]
async fn admin_suspended_admin_is_forbidden_on_every_verb() {
    // Regression: `authorize_admin` used to check `role` only, so a SUSPENDED
    // admin kept full control (the role field still reads Admin). Status now
    // gates authorization, so suspending an admin revokes their admin API
    // access immediately — on every verb.
    let services = admin_services(FakeAdminUsers::with([admin_record(
        "user-alpha",
        AdminUserRole::Admin,
        AdminUserStatus::Suspended,
    )]));
    assert_every_admin_verb_forbidden(&services).await;

    // A suspended OWNER is likewise locked out (owner also clears the role
    // boundary, so status must gate it too).
    let services_owner = admin_services(FakeAdminUsers::with([admin_record(
        "user-alpha",
        AdminUserRole::Owner,
        AdminUserStatus::Suspended,
    )]));
    assert_forbidden(
        services_owner
            .list_admin_users(caller(), RebornAdminUserListQuery::default())
            .await
            .expect_err("a suspended owner must not reach the admin surface"),
    );
}

#[tokio::test]
async fn admin_caller_lists_and_creates_with_one_time_token() {
    let services = admin_services(FakeAdminUsers::with([admin_record(
        "user-alpha",
        AdminUserRole::Admin,
        AdminUserStatus::Active,
    )]));
    services
        .list_admin_users(caller(), RebornAdminUserListQuery::default())
        .await
        .expect("an admin may list users");
    let created = services
        .create_admin_user(
            caller(),
            RebornAdminCreateUserRequest {
                email: Some("new@acme.com".to_string()),
                display_name: Some("New".to_string()),
                role: AdminUserRole::Member,
            },
        )
        .await
        .expect("an admin may create a user");
    assert_eq!(created.api_token, "minted-token");
}

#[tokio::test]
async fn admin_list_forwards_status_filter_to_the_port() {
    // A dropped `query.status` extractor or a broken active/suspended mapping
    // would silently return every user. Seed one active + one suspended admin
    // and assert `?status=` narrows the caller-visible result.
    let services = admin_services(FakeAdminUsers::with([
        admin_record("user-active", AdminUserRole::Admin, AdminUserStatus::Active),
        admin_record(
            "user-suspended",
            AdminUserRole::Admin,
            AdminUserStatus::Suspended,
        ),
    ]));

    let suspended = services
        .list_admin_users(
            caller().with_operator_webui_config(true),
            RebornAdminUserListQuery {
                status: Some(AdminUserStatus::Suspended),
                ..Default::default()
            },
        )
        .await
        .expect("list suspended");
    assert_eq!(suspended.users.len(), 1, "only the suspended user matches");
    assert_eq!(suspended.users[0].user_id.as_str(), "user-suspended");

    let active = services
        .list_admin_users(
            caller().with_operator_webui_config(true),
            RebornAdminUserListQuery {
                status: Some(AdminUserStatus::Active),
                ..Default::default()
            },
        )
        .await
        .expect("list active");
    assert_eq!(active.users.len(), 1, "only the active user matches");
    assert_eq!(active.users[0].user_id.as_str(), "user-active");

    let all = services
        .list_admin_users(
            caller().with_operator_webui_config(true),
            RebornAdminUserListQuery::default(),
        )
        .await
        .expect("list all");
    assert_eq!(all.users.len(), 2, "no filter returns both");
}

#[tokio::test]
async fn admin_list_bounds_pages_and_threads_the_cursor() {
    // The facade must clamp the page and derive a `next_cursor` from a full
    // page, then honor that cursor on the next call — so a large tenant is
    // paged, not returned (and scanned) in one unbounded response.
    let services = admin_services(FakeAdminUsers::with([
        admin_record("user-a", AdminUserRole::Admin, AdminUserStatus::Active),
        admin_record("user-b", AdminUserRole::Member, AdminUserStatus::Active),
        admin_record("user-c", AdminUserRole::Member, AdminUserStatus::Active),
    ]));

    let page1 = services
        .list_admin_users(
            caller().with_operator_webui_config(true),
            RebornAdminUserListQuery {
                limit: Some(2),
                ..Default::default()
            },
        )
        .await
        .expect("page 1");
    assert_eq!(page1.users.len(), 2, "the page honors the limit");
    assert_eq!(page1.users[0].user_id.as_str(), "user-a");
    assert_eq!(page1.users[1].user_id.as_str(), "user-b");
    let cursor = page1.next_cursor.expect("a full page yields a next cursor");
    assert_eq!(cursor, "user-b", "the cursor is the last id on the page");

    let page2 = services
        .list_admin_users(
            caller().with_operator_webui_config(true),
            RebornAdminUserListQuery {
                limit: Some(2),
                cursor: Some(cursor),
                ..Default::default()
            },
        )
        .await
        .expect("page 2");
    assert_eq!(page2.users.len(), 1, "the final page holds the remainder");
    assert_eq!(page2.users[0].user_id.as_str(), "user-c");
    assert!(
        page2.next_cursor.is_none(),
        "a short page means no more users"
    );
}

#[tokio::test]
async fn admin_list_rejects_a_malformed_cursor() {
    let services = admin_services(FakeAdminUsers::default());
    let err = services
        .list_admin_users(
            caller().with_operator_webui_config(true),
            RebornAdminUserListQuery {
                cursor: Some("not a valid user id \u{7f}".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect_err("a malformed cursor is caller input at fault");
    assert_eq!(err.status_code, 400);
}

#[tokio::test]
async fn admin_operator_bypasses_role_check() {
    // An env-bearer operator has no user record but is an implicit admin.
    let services = admin_services(FakeAdminUsers::default());
    let operator = caller().with_operator_webui_config(true);
    services
        .list_admin_users(operator, RebornAdminUserListQuery::default())
        .await
        .expect("an operator token clears the admin boundary without a user record");
}

#[tokio::test]
async fn admin_last_admin_protection_blocks_demote_suspend_and_delete() {
    // caller() (user-alpha) is the SOLE active admin. Demote, suspend, AND
    // delete must all be blocked — any of the three would otherwise strand the
    // tenant with zero active admins. `delete` has its own guard distinct from
    // the demote/suspend path, so it is covered here explicitly.
    let services = admin_services(FakeAdminUsers::with([admin_record(
        "user-alpha",
        AdminUserRole::Admin,
        AdminUserStatus::Active,
    )]));
    let target = UserId::new("user-alpha").expect("user");

    let demote = services
        .set_admin_user_role(
            caller(),
            target.clone(),
            RebornAdminSetRoleRequest {
                role: AdminUserRole::Member,
            },
        )
        .await
        .expect_err("demoting the sole admin must be blocked");
    assert_eq!(demote.status_code, 409);
    assert_eq!(demote.field.as_deref(), Some("last_admin"));

    let suspend = services
        .set_admin_user_status(
            caller(),
            target.clone(),
            RebornAdminSetStatusRequest {
                status: AdminUserStatus::Suspended,
            },
        )
        .await
        .expect_err("suspending the sole admin must be blocked");
    assert_eq!(suspend.status_code, 409);
    assert_eq!(suspend.field.as_deref(), Some("last_admin"));

    let delete = services
        .delete_admin_user(caller(), target)
        .await
        .expect_err("deleting the sole admin must be blocked");
    assert_eq!(delete.status_code, 409);
    assert_eq!(delete.field.as_deref(), Some("last_admin"));
}

#[tokio::test]
async fn admin_last_admin_protection_allows_demote_with_a_second_admin() {
    // With two active admins, demoting one is allowed.
    let services = admin_services(FakeAdminUsers::with([
        admin_record("user-alpha", AdminUserRole::Admin, AdminUserStatus::Active),
        admin_record("user-beta", AdminUserRole::Admin, AdminUserStatus::Active),
    ]));
    services
        .set_admin_user_role(
            caller(),
            UserId::new("user-beta").expect("user"),
            RebornAdminSetRoleRequest {
                role: AdminUserRole::Member,
            },
        )
        .await
        .expect("demoting one of two admins is allowed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn admin_last_admin_protection_survives_concurrent_demotion() {
    // Two active admins. Fire both demotions concurrently: without serialization
    // each `ensure_not_last_admin` reads "2 admins", both pass, and both land →
    // 0 admins (a TOCTOU race). The per-tenant admin-mutation lock serializes
    // the check+mutation, so exactly one demotion succeeds and the other is a
    // 409 — the tenant always keeps an admin. Runs on a multi-thread runtime so
    // the two calls are genuinely parallel.
    let services = std::sync::Arc::new(admin_services(FakeAdminUsers::with([
        admin_record("user-alpha", AdminUserRole::Admin, AdminUserStatus::Active),
        admin_record("user-beta", AdminUserRole::Admin, AdminUserStatus::Active),
    ])));

    // Both demotions run as an OPERATOR caller (bypasses the role check and is
    // never itself demoted), so the second failure is deterministically the
    // 409 last-admin block — not a 403 from the caller losing its own role.
    let demote = |uid: &'static str| {
        let services = std::sync::Arc::clone(&services);
        async move {
            services
                .set_admin_user_role(
                    caller().with_operator_webui_config(true),
                    UserId::new(uid).expect("user"),
                    RebornAdminSetRoleRequest {
                        role: AdminUserRole::Member,
                    },
                )
                .await
        }
    };

    let (alpha, beta) = tokio::join!(demote("user-alpha"), demote("user-beta"));

    let successes = [alpha.is_ok(), beta.is_ok()]
        .into_iter()
        .filter(|ok| *ok)
        .count();
    let blocked = [&alpha, &beta]
        .into_iter()
        .filter(|result| matches!(result, Err(err) if err.status_code == 409))
        .count();
    assert_eq!(successes, 1, "exactly one concurrent demotion may land");
    assert_eq!(
        blocked, 1,
        "the other must be blocked by last-admin protection (never stranded at 0 admins)"
    );

    let remaining = services
        .list_admin_users(
            caller().with_operator_webui_config(true),
            RebornAdminUserListQuery::default(),
        )
        .await
        .expect("list")
        .users
        .into_iter()
        .filter(|u| u.role.is_admin() && u.status == AdminUserStatus::Active)
        .count();
    assert_eq!(
        remaining, 1,
        "the tenant must never be stranded without an admin"
    );
}
