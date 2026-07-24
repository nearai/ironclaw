// arch-exempt: large_file, lifecycle handler regressions reuse the existing WebUI contract harness, plan #6175
//! Caller-level contract tests for the WebChat v2 axum handlers.
//!
//! Per `.claude/rules/testing.md` "Test Through the Caller", these tests
//! drive a real axum [`Router`] (built from [`webui_v2_router`]) against a
//! stub [`ProductSurface`] so the regression target is the wire
//! contract — body shape, path/query plumbing, error mapping — not just
//! the facade method bodies that are already covered in
//! `ironclaw_product`.

// arch-exempt: large_file, WebUI ProductSurface route contracts stay in the caller-level handler suite until the WebUI route split lands, plan #5985

#[path = "support/product_surface.rs"]
mod programmable_surface;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderName, Method, Request, StatusCode, header};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use chrono::Utc;
use http_body_util::BodyExt;
use ironclaw_host_api::{
    ActivityId, AgentId, Blocked, CapabilityId, ExtensionId, GateRef, GateWaypoint, InvocationId,
    Outcome, OutcomeRefs, ProductSurface, ProductSurfaceCaller, ProductSurfaceError,
    ProductSurfaceErrorCode, ProductSurfaceErrorKind, ProductSurfaceValidationCode, ProjectId,
    Resolution, ResultPreviewMeta, ResultProgress, ResultRef, RuntimeKind, SafeSummary, TenantId,
    TerminateHint, ThreadId, ToolVerdict, UserId,
};
use ironclaw_product::{
    ADMIN_USER_DELETE_CAPABILITY_ID, ADMIN_USER_PUT_SECRET_CAPABILITY_ID, ADMIN_USER_SECRETS_VIEW,
    ADMIN_USER_SET_ROLE_CAPABILITY_ID, ADMIN_USER_SET_STATUS_CAPABILITY_ID,
    ADMIN_USER_UPDATE_CAPABILITY_ID, ADMIN_USER_VIEW, ADMIN_USERS_VIEW, AUTOMATIONS_VIEW,
    AdminUserRecord, AdminUserRole, AdminUserSecretMeta, AdminUserStatus, CodexLoginStart,
    EXTENSION_IMPORT_CAPABILITY_ID, EXTENSION_INSTALL_CAPABILITY_ID, EXTENSION_REGISTRY_VIEW,
    EXTENSION_REMOVE_CAPABILITY_ID, EXTENSION_SETUP_SUBMIT_CAPABILITY_ID, EXTENSION_SETUP_VIEW,
    EXTENSIONS_VIEW, FS_LIST_VIEW, FS_MOUNTS_VIEW, FS_STAT_VIEW, FsMount, GLOBAL_AUTO_APPROVE_VIEW,
    LLM_ACTIVE_SET_CAPABILITY_ID, LLM_CONFIG_VIEW, LLM_PROVIDER_DELETE_CAPABILITY_ID,
    LLM_PROVIDER_UPSERT_CAPABILITY_ID, LOGS_VIEW, LifecyclePackageKind, LifecyclePackageRef,
    LifecyclePublicState, LlmActiveSelection, LlmConfigSnapshot, LlmModelsResult, LlmProbeRequest,
    LlmProbeResult, LlmProviderView, NearAiLoginRequest, NearAiLoginStart,
    NearAiWalletLoginRequest, NearAiWalletLoginResult, OPERATOR_CONFIG_KEY_VIEW,
    OPERATOR_CONFIG_LIST_VIEW, OPERATOR_CONFIG_SET_AUTO_APPROVE_CAPABILITY_ID,
    OPERATOR_CONFIG_VALIDATE_VIEW, OPERATOR_DIAGNOSTICS_VIEW, OPERATOR_LOGS_VIEW,
    OPERATOR_SETUP_RUN_CAPABILITY_ID, OPERATOR_SETUP_VIEW, OPERATOR_STATUS_VIEW,
    OUTBOUND_DELIVERY_TARGETS_VIEW, OUTBOUND_PREFERENCES_SET_CAPABILITY_ID,
    OUTBOUND_PREFERENCES_VIEW, PROJECT_DELETE_CAPABILITY_ID, PROJECT_FS_LIST_VIEW,
    PROJECT_FS_STAT_VIEW, PROJECT_MEMBER_ADD_CAPABILITY_ID, PROJECT_MEMBER_REMOVE_CAPABILITY_ID,
    PROJECT_MEMBER_UPDATE_CAPABILITY_ID, PROJECT_MEMBERS_VIEW, PROJECT_UPDATE_CAPABILITY_ID,
    PROJECT_VIEW, PROJECTS_VIEW, ProductCancelRunRequest, ProductCreateThreadRequest,
    ProductListAutomationsRequest, ProductListThreadsRequest, ProductResolveGateRequest,
    ProductRetryRunRequest, ProductSubmitTurnRequest, ProjectFsEntry, ProjectFsEntryKind,
    ProjectFsFile, ProjectFsStat, RUN_ARTIFACT_SCHEMA, RUN_ARTIFACT_VIEW,
    RebornAccountLoginLinkResponse, RebornAccountTracesResponse, RebornAdminCreateUserRequest,
    RebornAdminDeleteSecretProductRequest, RebornAdminSecretDeletedResponse,
    RebornAdminSetRoleProductRequest, RebornAdminSetStatusProductRequest,
    RebornAdminUpdateUserProductRequest, RebornAdminUserCreatedResponse, RebornAdminUserListQuery,
    RebornAdminUserListResponse, RebornAdminUserRequest, RebornAdminUserResponse,
    RebornAdminUserSecretsListResponse, RebornAttachmentBytes, RebornAttachmentRequest,
    RebornAutomationInfo, RebornAutomationMutationResponse, RebornAutomationRecentRunInfo,
    RebornAutomationRecentRunStatus, RebornAutomationRequest, RebornAutomationSource,
    RebornAutomationState, RebornCancelRunResponse, RebornCreateProjectRequest,
    RebornCreateThreadResponse, RebornExtensionInfo, RebornExtensionListResponse,
    RebornExtensionRegistryResponse, RebornFsListRequest, RebornFsListResponse, RebornFsMountInfo,
    RebornFsMountsResponse, RebornFsReadRequest, RebornFsStatRequest, RebornFsStatResponse,
    RebornGetProjectRequest, RebornGetRunStateResponse, RebornGlobalAutoApproveRequest,
    RebornGlobalAutoApproveResponse, RebornListAutomationsResponse, RebornListMembersResponse,
    RebornListProjectsResponse, RebornListThreadsResponse, RebornLogQueryRequest,
    RebornLogQueryResponse, RebornOperatorArea, RebornOperatorCommandPlaneResponse,
    RebornOperatorConfigDiagnostic, RebornOperatorConfigDiagnosticSeverity,
    RebornOperatorConfigEntry, RebornOperatorConfigGetResponse, RebornOperatorConfigListResponse,
    RebornOperatorConfigSetProductRequest, RebornOperatorConfigSetRequest,
    RebornOperatorConfigValidateRequest, RebornOperatorConfigValidateResponse,
    RebornOperatorLogsQuery, RebornOperatorServiceLifecycleAction,
    RebornOperatorServiceLifecycleRequest, RebornOperatorSetupResponse, RebornOperatorSetupStatus,
    RebornOperatorSurfaceStatus, RebornOutboundDeliveryTargetCapabilities,
    RebornOutboundDeliveryTargetId, RebornOutboundDeliveryTargetListResponse,
    RebornOutboundDeliveryTargetOption, RebornOutboundDeliveryTargetStatus,
    RebornOutboundDeliveryTargetSummary, RebornOutboundPreferencesResponse,
    RebornProjectFsListRequest, RebornProjectFsListResponse, RebornProjectFsReadRequest,
    RebornProjectFsStatRequest, RebornProjectFsStatResponse, RebornProjectInfo,
    RebornProjectMemberInfo, RebornProjectMemberStatus, RebornProjectResponse, RebornProjectRole,
    RebornProjectState, RebornRenameAutomationProductRequest, RebornResolveGateResponse,
    RebornResumeGateResponse, RebornRetryRunResponse, RebornRunArtifact, RebornRunArtifactRequest,
    RebornSetupExtensionResponse, RebornSkillContentResponse, RebornSkillListResponse,
    RebornSkillSearchResponse, RebornStreamEventsRequest, RebornStreamEventsResponse,
    RebornSubmitTurnResponse, RebornTimelineRequest, RebornTimelineResponse,
    RebornTraceCreditsResponse, RebornTraceHoldAuthorizeProductRequest,
    RebornTraceHoldAuthorizeResponse, RebornViewPage, RebornViewQuery, RunArtifactLogs,
    RunArtifactRedaction, SKILL_AUTO_ACTIVATE_LEARNED_SET_CAPABILITY_ID,
    SKILL_AUTO_ACTIVATE_SET_CAPABILITY_ID, SKILL_CONTENT_VIEW, SKILL_INSTALL_CAPABILITY_ID,
    SKILL_REMOVE_CAPABILITY_ID, SKILL_SEARCH_VIEW, SKILL_UPDATE_CAPABILITY_ID, SKILLS_VIEW,
    THREAD_DELETE_CAPABILITY_ID, THREADS_VIEW, TIMELINE_VIEW, TRACE_ACCOUNT_TRACES_VIEW,
    TRACE_CREDITS_VIEW, rejecting_product_surface_error,
};
use ironclaw_product::{
    AdapterInstallationId, CapabilityActivityStatusView, CapabilityActivityView,
    ExternalConversationRef, FinalReplyView, ProductAdapterId, ProductOutboundEnvelope,
    ProductOutboundPayload, ProductOutboundTarget, ProductProjectionItem, ProductProjectionState,
    ProgressKind, ProgressUpdateView, ProjectionCursor,
};
use ironclaw_threads::SessionThreadRecord;
use ironclaw_turns::{
    AcceptedMessageRef, EventCursor, ReplyTargetBindingRef, RunProfileId, RunProfileVersion,
    TurnRunId, TurnStatus,
};
use ironclaw_webui::webui_v2::{
    DEFAULT_SSE_MAX_CONCURRENT_PER_CALLER, WebUiV2Capabilities, WebUiV2RouteOptions, WebUiV2State,
    webui_v2_router, webui_v2_router_with_options,
};
use serde::Serialize;
use serde_json::Value;
use tokio::sync::Notify;
use tower::ServiceExt;

use programmable_surface::ProgrammableProductSurface;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ProductSurfaceCallId {
    CreateThread,
    SubmitTurn,
    CancelRun,
    ResolveGate,
    RetryRun,
    ProjectCreate,
    ProjectFsRead,
    FsRead,
    AttachmentRead,
    TraceAccountLoginLink,
    TraceHoldAuthorize,
    OperatorConfigSetKey,
    OperatorServiceLifecycle,
    LlmTestConnection,
    LlmListModels,
    LlmNearAiLogin,
    LlmNearAiWalletLogin,
    LlmCodexLogin,
    AdminUserCreate,
    AdminUserDeleteSecret,
    AutomationPause,
    AutomationResume,
    AutomationRename,
    AutomationDelete,
}

impl ProductSurfaceCallId {
    const fn as_str(self) -> &'static str {
        match self {
            Self::CreateThread => "thread.create",
            Self::SubmitTurn => "turn.submit",
            Self::CancelRun => "run.cancel",
            Self::ResolveGate => "gate.resolve",
            Self::RetryRun => "run.retry",
            Self::ProjectCreate => "project.create",
            Self::ProjectFsRead => "project.fs.read",
            Self::FsRead => "fs.read",
            Self::AttachmentRead => "attachment.read",
            Self::TraceAccountLoginLink => "trace.account_login_link",
            Self::TraceHoldAuthorize => "trace.hold_authorize",
            Self::OperatorConfigSetKey => "operator.config.set_key",
            Self::OperatorServiceLifecycle => "operator.service.lifecycle",
            Self::LlmTestConnection => "llm.test_connection",
            Self::LlmListModels => "llm.list_models",
            Self::LlmNearAiLogin => "llm.nearai.login",
            Self::LlmNearAiWalletLogin => "llm.nearai.wallet_login",
            Self::LlmCodexLogin => "llm.codex.login",
            Self::AdminUserCreate => "admin.user.create",
            Self::AdminUserDeleteSecret => "admin.user.delete_secret",
            Self::AutomationPause => "automation.pause",
            Self::AutomationResume => "automation.resume",
            Self::AutomationRename => "automation.rename",
            Self::AutomationDelete => "automation.delete",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "thread.create" => Some(Self::CreateThread),
            "turn.submit" => Some(Self::SubmitTurn),
            "run.cancel" => Some(Self::CancelRun),
            "gate.resolve" => Some(Self::ResolveGate),
            "run.retry" => Some(Self::RetryRun),
            "project.create" => Some(Self::ProjectCreate),
            "project.fs.read" => Some(Self::ProjectFsRead),
            "fs.read" => Some(Self::FsRead),
            "attachment.read" => Some(Self::AttachmentRead),
            "trace.account_login_link" => Some(Self::TraceAccountLoginLink),
            "trace.hold_authorize" => Some(Self::TraceHoldAuthorize),
            "operator.config.set_key" => Some(Self::OperatorConfigSetKey),
            "operator.service.lifecycle" => Some(Self::OperatorServiceLifecycle),
            "llm.test_connection" => Some(Self::LlmTestConnection),
            "llm.list_models" => Some(Self::LlmListModels),
            "llm.nearai.login" => Some(Self::LlmNearAiLogin),
            "llm.nearai.wallet_login" => Some(Self::LlmNearAiWalletLogin),
            "llm.codex.login" => Some(Self::LlmCodexLogin),
            "admin.user.create" => Some(Self::AdminUserCreate),
            "admin.user.delete_secret" => Some(Self::AdminUserDeleteSecret),
            "automation.pause" => Some(Self::AutomationPause),
            "automation.resume" => Some(Self::AutomationResume),
            "automation.rename" => Some(Self::AutomationRename),
            "automation.delete" => Some(Self::AutomationDelete),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct RecordedProductSurfaceCallRequest {
    call_id: String,
    input: Value,
}

impl RecordedProductSurfaceCallRequest {
    fn from_value(call_id: ProductSurfaceCallId, input: Value) -> Self {
        Self {
            call_id: call_id.as_str().to_string(),
            input,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum RecordedProductSurfaceCallResponse {
    Json(Value),
    ProjectFile(ProjectFsFile),
    Attachment(RebornAttachmentBytes),
}

impl RecordedProductSurfaceCallResponse {
    fn json<T: Serialize>(value: T) -> Result<Self, ProductSurfaceError> {
        Ok(Self::Json(
            serde_json::to_value(value).map_err(ProductSurfaceError::internal_from)?,
        ))
    }

    fn project_file(file: ProjectFsFile) -> Self {
        Self::ProjectFile(file)
    }

    fn attachment(bytes: RebornAttachmentBytes) -> Self {
        Self::Attachment(bytes)
    }

    fn into_value(self) -> Result<Value, ProductSurfaceError> {
        match self {
            Self::Json(value) => Ok(value),
            Self::ProjectFile(file) => {
                serde_json::to_value(file).map_err(ProductSurfaceError::internal_from)
            }
            Self::Attachment(bytes) => {
                serde_json::to_value(bytes).map_err(ProductSurfaceError::internal_from)
            }
        }
    }
}

fn caller() -> ProductSurfaceCaller {
    caller_for_user("user-alpha")
}

fn caller_for_user(user_id: &str) -> ProductSurfaceCaller {
    ProductSurfaceCaller::new(
        TenantId::new("tenant-alpha").expect("tenant"),
        UserId::new(user_id).expect("user"),
        Some(AgentId::new("agent-alpha").expect("agent")),
        Some(ProjectId::new("project-alpha").expect("project")),
    )
}

fn router_with(services: Arc<dyn ProductSurface>) -> Router {
    router_with_caller(services, WebUiV2Capabilities::default(), caller())
}

fn router_with_caller(
    services: Arc<dyn ProductSurface>,
    capabilities: WebUiV2Capabilities,
    caller: ProductSurfaceCaller,
) -> Router {
    webui_v2_router(WebUiV2State::new(
        services,
        DEFAULT_SSE_MAX_CONCURRENT_PER_CALLER,
    ))
    // Production composition runs the bearer-token middleware that
    // constructs this `Extension`; tests bypass auth and inject the
    // caller directly so the regression target is the handler itself.
    .layer(axum::Extension(caller))
    .layer(axum::Extension(capabilities))
}

fn router_with_capabilities(
    services: Arc<dyn ProductSurface>,
    capabilities: WebUiV2Capabilities,
) -> Router {
    router_with_caller(services, capabilities, caller())
}

fn router_with_caller_only(services: Arc<dyn ProductSurface>) -> Router {
    webui_v2_router(WebUiV2State::new(
        services,
        DEFAULT_SSE_MAX_CONCURRENT_PER_CALLER,
    ))
    .layer(axum::Extension(caller()))
}

fn service_unavailable_error(retryable: bool) -> ProductSurfaceError {
    ProductSurfaceError {
        code: ProductSurfaceErrorCode::Unavailable,
        kind: ProductSurfaceErrorKind::ServiceUnavailable,
        status_code: 503,
        retryable,
        field: None,
        validation_code: None,
    }
}

fn successful_resolution(activity_id: ActivityId) -> Resolution {
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
        summary: SafeSummary::new("ok").expect("static summary is redaction-safe"),
        progress: ResultProgress::MadeProgress,
        terminate_hint: TerminateHint::Continue,
    })
}

fn blocked_auth_resolution(activity_id: ActivityId) -> Resolution {
    Resolution::Blocked(Blocked::Auth(GateWaypoint::new(GateRef::from_uuid(
        activity_id.as_uuid(),
    ))))
}

type OperatorConfigSetCall = (String, Value);
type LlmUpsertCall = (String, bool);
type LlmActiveCall = (String, Option<String>);
type LogsCall = RebornLogQueryRequest;
type OperatorLogsCall = RebornOperatorLogsQuery;

fn operator_config_surface_not_wired_diagnostic() -> RebornOperatorConfigDiagnostic {
    RebornOperatorConfigDiagnostic {
        key: "*".to_string(),
        severity: RebornOperatorConfigDiagnosticSeverity::Error,
        reason_code: "operator_config_service_not_wired".to_string(),
        message: "Operator config diagnostics are available, but the effective config service is not wired yet.".to_string(),
        owning_area: RebornOperatorArea::Config,
        remediation: "Use bootstrap config, environment variables, or existing CLI setup until the operator config service is enabled.".to_string(),
    }
}

fn operator_config_validation_diagnostics(
    keys: Vec<String>,
) -> Vec<RebornOperatorConfigDiagnostic> {
    let keys = if keys.is_empty() {
        vec!["*".to_string()]
    } else {
        keys
    };

    keys.into_iter()
        .map(operator_config_key_diagnostic)
        .collect()
}

fn operator_config_key_diagnostic(key: String) -> RebornOperatorConfigDiagnostic {
    let normalized = key.to_ascii_lowercase();
    let is_secret = ["api_key", "credential", "password", "secret", "token"]
        .iter()
        .any(|marker| normalized.contains(marker));

    let (reason_code, message, remediation) = if key == "*" {
        (
            "operator_config_service_not_wired",
            "Operator config validation is available, but the effective config service is not wired yet.",
            "Use bootstrap config, environment variables, or existing CLI setup until the operator config service is enabled.",
        )
    } else if is_secret {
        (
            "operator_config_secret_not_wired",
            "Secret-backed operator config is not writable through the operator API yet.",
            "Store secrets through the configured secret provider or bootstrap environment until the operator secrets flow is enabled.",
        )
    } else if normalized.starts_with("deprecated.") || normalized.starts_with("legacy.") {
        (
            "operator_config_deprecated",
            "This operator config key is deprecated and is not applied by the Reborn runtime.",
            "Move the setting to the current config key before relying on operator-managed startup.",
        )
    } else if normalized.starts_with("bootstrap.") {
        (
            "operator_config_immutable",
            "Bootstrap config is immutable from the browser operator API.",
            "Change this setting in bootstrap config and restart the host process.",
        )
    } else if matches!(
        normalized.as_str(),
        "provider.default" | "model.default" | "profile.default"
    ) {
        (
            "operator_config_not_wired",
            "This parsed operator config key is not wired into runtime behavior yet.",
            "Keep using the existing setup path for this setting until effective config persistence is enabled.",
        )
    } else {
        (
            "operator_config_unknown_key",
            "This operator config key is not recognized by the current Reborn runtime.",
            "Remove the key or rename it to a documented operator config key.",
        )
    };

    RebornOperatorConfigDiagnostic {
        key,
        severity: RebornOperatorConfigDiagnosticSeverity::Error,
        reason_code: reason_code.to_string(),
        message: message.to_string(),
        owning_area: RebornOperatorArea::Config,
        remediation: remediation.to_string(),
    }
}

fn operator_config_diagnostic_command_plane_response(
    area: RebornOperatorArea,
) -> RebornOperatorCommandPlaneResponse {
    RebornOperatorCommandPlaneResponse {
        area,
        status: RebornOperatorSurfaceStatus::Unavailable,
        message: "Operator config has unsupported or not-yet-wired settings.".to_string(),
        operator_status: None,
        logs: None,
        service_lifecycle: None,
        diagnostics: vec![operator_config_surface_not_wired_diagnostic()],
    }
}

#[derive(Default)]
struct StubServices {
    create_thread_calls: Mutex<Vec<ProductCreateThreadRequest>>,
    submit_turn_calls: Mutex<Vec<ProductSubmitTurnRequest>>,
    get_timeline_calls: Mutex<Vec<RebornTimelineRequest>>,
    browse_fs_calls: Mutex<Vec<RebornFsListRequest>>,
    global_auto_approve_enabled: Mutex<bool>,
    global_auto_approve_calls: Mutex<usize>,
    stall_global_auto_approve: Mutex<bool>,
    next_global_auto_approve_error: Mutex<Option<ProductSurfaceError>>,
    view_queries: Mutex<Vec<RebornViewQuery>>,
    next_extensions_view: Mutex<Option<RebornExtensionListResponse>>,
    invoke_calls: Mutex<Vec<(CapabilityId, Value, ActivityId)>>,
    surface_calls: Mutex<Vec<RecordedProductSurfaceCallRequest>>,
    next_surface_response:
        Mutex<Option<Result<RecordedProductSurfaceCallResponse, ProductSurfaceError>>>,
    next_invoke_response: Mutex<Option<Result<Resolution, ProductSurfaceError>>>,
    read_attachment_calls: Mutex<Vec<RebornAttachmentRequest>>,
    read_attachment_response: Mutex<Option<RebornAttachmentBytes>>,
    stream_events_calls: Mutex<Vec<RebornStreamEventsRequest>>,
    cancel_run_calls: Mutex<Vec<ProductCancelRunRequest>>,
    resolve_gate_calls: Mutex<Vec<ProductResolveGateRequest>>,
    retry_run_calls: Mutex<Vec<ProductRetryRunRequest>>,
    list_threads_calls: Mutex<Vec<ProductListThreadsRequest>>,
    list_automations_calls: Mutex<Vec<ProductListAutomationsRequest>>,
    /// Captures the authenticated caller's user id for each
    /// `trace_account_traces` call, so the contract test can assert the handler
    /// forwards the caller (the route is caller-scoped).
    trace_account_traces_callers: Mutex<Vec<String>>,
    /// Forwarded caller user-ids for each `trace_account_login_link` call.
    trace_account_login_link_callers: Mutex<Vec<String>>,
    next_list_automations_error: Mutex<Option<ProductSurfaceError>>,
    get_outbound_preferences_calls: Mutex<usize>,
    list_outbound_delivery_targets_calls: Mutex<usize>,
    list_operator_config_calls: Mutex<usize>,
    operator_config_entries: Mutex<Vec<RebornOperatorConfigEntry>>,
    get_operator_config_key_calls: Mutex<Vec<String>>,
    set_operator_config_key_calls: Mutex<Vec<OperatorConfigSetCall>>,
    next_set_operator_config_key_error: Mutex<Option<ProductSurfaceError>>,
    validate_operator_config_calls: Mutex<Vec<Vec<String>>>,
    query_logs_calls: Mutex<Vec<LogsCall>>,
    query_operator_logs_calls: Mutex<Vec<OperatorLogsCall>>,
    run_operator_service_lifecycle_calls: Mutex<Vec<RebornOperatorServiceLifecycleAction>>,
    get_llm_config_calls: Mutex<usize>,
    upsert_llm_provider_calls: Mutex<Vec<LlmUpsertCall>>,
    delete_llm_provider_calls: Mutex<Vec<String>>,
    set_active_llm_calls: Mutex<Vec<LlmActiveCall>>,
    test_llm_connection_calls: Mutex<Vec<String>>,
    list_llm_models_calls: Mutex<Vec<String>>,
    next_create_thread_error: Mutex<Option<ProductSurfaceError>>,
    next_retry_run: Mutex<VecDeque<Result<RebornRetryRunResponse, ProductSurfaceError>>>,
    /// Per-call queued responses for `stream_events`. When non-empty, the
    /// front entry is popped and returned on each call so SSE tests can
    /// drive the handler through specific projection envelopes, error
    /// branches, or empty drains in a deterministic order.
    next_stream_events: Mutex<VecDeque<Result<RebornStreamEventsResponse, ProductSurfaceError>>>,
    stream_events_notify: Arc<Notify>,
    /// Queued response for the next `submit_turn` call. When `Some`, the value
    /// is taken and returned instead of the default `Submitted` response.
    next_submit_response: Mutex<Option<RebornSubmitTurnResponse>>,
}

impl StubServices {
    fn fail_create_thread(&self, error: ProductSurfaceError) {
        *self.next_create_thread_error.lock().expect("lock") = Some(error);
    }

    /// Stage the bytes `read_attachment` should return. When unset, the stub
    /// inherits the trait default (404 not found).
    fn set_attachment(&self, bytes: RebornAttachmentBytes) {
        *self.read_attachment_response.lock().expect("lock") = Some(bytes);
    }

    fn fail_list_automations(&self, error: ProductSurfaceError) {
        *self.next_list_automations_error.lock().expect("lock") = Some(error);
    }

    fn enqueue_invoke_response(&self, response: Result<Resolution, ProductSurfaceError>) {
        *self.next_invoke_response.lock().expect("lock") = Some(response);
    }

    fn enqueue_operation_response(
        &self,
        response: Result<RecordedProductSurfaceCallResponse, ProductSurfaceError>,
    ) {
        *self.next_surface_response.lock().expect("lock") = Some(response);
    }

    fn set_extensions_view(&self, response: RebornExtensionListResponse) {
        *self.next_extensions_view.lock().expect("lock") = Some(response);
    }

    fn fail_set_operator_config_key(&self, error: ProductSurfaceError) {
        *self
            .next_set_operator_config_key_error
            .lock()
            .expect("lock") = Some(error);
    }

    fn enqueue_retry_run(&self, response: Result<RebornRetryRunResponse, ProductSurfaceError>) {
        self.next_retry_run
            .lock()
            .expect("lock")
            .push_back(response);
    }

    /// Queue one response for the next `stream_events` call. Tests use this
    /// to drive the SSE handler through programmable projection envelopes
    /// or error branches. Falls back to an empty `Ok` drain when the queue
    /// is empty.
    fn enqueue_stream_events(
        &self,
        response: Result<RebornStreamEventsResponse, ProductSurfaceError>,
    ) {
        self.next_stream_events
            .lock()
            .expect("lock")
            .push_back(response);
    }

    /// Triggered the first time `stream_events` is invoked. Lets the SSE
    /// test wait on the actual facade call rather than guessing at a
    /// timeout — axum's SSE body is lazy, so the handler does not run
    /// until the client polls the body.
    fn stream_events_signal(&self) -> Arc<Notify> {
        self.stream_events_notify.clone()
    }

    fn set_next_submit_response(&self, response: RebornSubmitTurnResponse) {
        *self.next_submit_response.lock().expect("lock") = Some(response);
    }
}

impl StubServices {
    async fn global_auto_approve_enabled(
        &self,
        _caller: ProductSurfaceCaller,
    ) -> Result<bool, ProductSurfaceError> {
        *self.global_auto_approve_calls.lock().expect("lock") += 1;
        if *self.stall_global_auto_approve.lock().expect("lock") {
            std::future::pending::<()>().await;
        }
        if let Some(error) = self
            .next_global_auto_approve_error
            .lock()
            .expect("lock")
            .take()
        {
            return Err(error);
        }
        Ok(*self.global_auto_approve_enabled.lock().expect("lock"))
    }

    async fn create_thread(
        &self,
        _caller: ProductSurfaceCaller,
        request: ProductCreateThreadRequest,
    ) -> Result<RebornCreateThreadResponse, ProductSurfaceError> {
        self.create_thread_calls
            .lock()
            .expect("lock")
            .push(request.clone());
        if let Some(error) = self.next_create_thread_error.lock().expect("lock").take() {
            return Err(error);
        }
        Ok(RebornCreateThreadResponse {
            thread: SessionThreadRecord {
                thread_id: ironclaw_host_api::ThreadId::new("thread:fake").expect("thread id"),
                scope: ironclaw_threads::ThreadScope {
                    tenant_id: TenantId::new("tenant-alpha").expect("tenant"),
                    agent_id: AgentId::new("agent-alpha").expect("agent"),
                    project_id: Some(ProjectId::new("project-alpha").expect("project")),
                    owner_user_id: Some(UserId::new("user-alpha").expect("user")),
                    mission_id: None,
                },
                created_by_actor_id: "user-alpha".to_string(),
                title: None,
                metadata_json: request
                    .client_action_id
                    .as_ref()
                    .map(|id| format!("{{\"client_action_id\":\"{id}\"}}")),
                goal: None,
                created_at: None,
                updated_at: None,
            },
        })
    }

    async fn submit_turn(
        &self,
        _caller: ProductSurfaceCaller,
        request: ProductSubmitTurnRequest,
    ) -> Result<RebornSubmitTurnResponse, ProductSurfaceError> {
        self.submit_turn_calls
            .lock()
            .expect("lock")
            .push(request.clone());
        if let Some(next) = self.next_submit_response.lock().expect("lock").take() {
            return Ok(next);
        }
        Ok(RebornSubmitTurnResponse::Submitted {
            thread_id: ironclaw_host_api::ThreadId::new(
                request.thread_id.clone().unwrap_or_default(),
            )
            .expect("thread id"),
            accepted_message_ref: ironclaw_turns::AcceptedMessageRef::new("msg:fake").expect("ref"),
            turn_id: "turn:fake".to_string(),
            run_id: TurnRunId::new(),
            status: TurnStatus::Queued,
            resolved_run_profile_id: RunProfileId::default_profile().as_str().to_string(),
            resolved_run_profile_version: RunProfileVersion::new(1).as_u64(),
            event_cursor: EventCursor(1),
        })
    }

    async fn get_timeline(
        &self,
        _caller: ProductSurfaceCaller,
        request: RebornTimelineRequest,
    ) -> Result<RebornTimelineResponse, ProductSurfaceError> {
        self.get_timeline_calls
            .lock()
            .expect("lock")
            .push(request.clone());
        Ok(RebornTimelineResponse {
            thread: SessionThreadRecord {
                thread_id: ironclaw_host_api::ThreadId::new(request.thread_id.clone())
                    .expect("thread id"),
                scope: ironclaw_threads::ThreadScope {
                    tenant_id: TenantId::new("tenant-alpha").expect("tenant"),
                    agent_id: AgentId::new("agent-alpha").expect("agent"),
                    project_id: Some(ProjectId::new("project-alpha").expect("project")),
                    owner_user_id: Some(UserId::new("user-alpha").expect("user")),
                    mission_id: None,
                },
                created_by_actor_id: "user-alpha".to_string(),
                title: None,
                metadata_json: None,
                goal: None,
                created_at: None,
                updated_at: None,
            },
            messages: Vec::new(),
            summary_artifacts: Vec::new(),
            next_cursor: None,
        })
    }

    async fn invoke(
        &self,
        _caller: ProductSurfaceCaller,
        capability: CapabilityId,
        input: serde_json::Value,
        activity_id: ActivityId,
    ) -> Result<Resolution, ProductSurfaceError> {
        if capability.as_str() == LLM_PROVIDER_UPSERT_CAPABILITY_ID {
            let provider_id = input
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(ProductSurfaceError::internal)?
                .to_string();
            self.upsert_llm_provider_calls
                .lock()
                .expect("lock")
                .push((provider_id, input.get("api_key").is_some()));
            if let Some(response) = self.next_invoke_response.lock().expect("lock").take() {
                return response;
            }
            return Err(service_unavailable_error(false));
        }
        if capability.as_str() == LLM_PROVIDER_DELETE_CAPABILITY_ID
            && let Some(provider_id) = input.get("provider_id").and_then(Value::as_str)
        {
            self.delete_llm_provider_calls
                .lock()
                .expect("lock")
                .push(provider_id.to_string());
        }
        if capability.as_str() == LLM_ACTIVE_SET_CAPABILITY_ID
            && let Some(provider_id) = input.get("provider_id").and_then(Value::as_str)
        {
            self.set_active_llm_calls.lock().expect("lock").push((
                provider_id.to_string(),
                input
                    .get("model")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
            ));
        }
        self.invoke_calls
            .lock()
            .expect("lock")
            .push((capability, input, activity_id));
        if let Some(response) = self.next_invoke_response.lock().expect("lock").take() {
            return response;
        }
        Err(service_unavailable_error(false))
    }

    async fn query(
        &self,
        caller: ProductSurfaceCaller,
        query: RebornViewQuery,
    ) -> Result<RebornViewPage, ProductSurfaceError> {
        self.view_queries.lock().expect("lock").push(query.clone());
        match query.view_id.as_str() {
            id if id == RUN_ARTIFACT_VIEW.id => {
                let request: RebornRunArtifactRequest =
                    serde_json::from_value(query.params).expect("artifact params");
                let run_id = TurnRunId::parse(&request.run_id).expect("test run id");
                let artifact = RebornRunArtifact {
                    schema: RUN_ARTIFACT_SCHEMA.to_string(),
                    generated_at: Utc::now(),
                    thread_id: request.thread_id,
                    run: RebornGetRunStateResponse {
                        turn_id: "turn-artifact".to_string(),
                        run_id,
                        status: TurnStatus::Completed,
                        event_cursor: EventCursor(1),
                        accepted_message_ref: AcceptedMessageRef::new("msg:artifact")
                            .expect("message ref"),
                        resolved_run_profile_id: "default".to_string(),
                        resolved_run_profile_version: 1,
                        received_at: Utc::now(),
                        checkpoint_id: None,
                        gate_ref: None,
                        failure: None,
                        usage: None,
                        cost: None,
                    },
                    messages: Vec::new(),
                    logs: RunArtifactLogs {
                        source: "test".to_string(),
                        available: true,
                        complete: false,
                        truncated: false,
                        unavailable_reason: None,
                        entries: Vec::new(),
                    },
                    redaction: RunArtifactRedaction {
                        pipeline: "deterministic-trace-redactor-v1".to_string(),
                        applied: false,
                    },
                };
                Ok(RebornViewPage {
                    payload: serde_json::to_value(artifact).expect("artifact payload"),
                    next_cursor: None,
                })
            }
            id if id == LOGS_VIEW.id => {
                let mut request: RebornLogQueryRequest =
                    serde_json::from_value(query.params).expect("logs params");
                request.cursor = query.cursor.or(request.cursor);
                if request.tail && request.follow {
                    return Err(ProductSurfaceError {
                        code: ProductSurfaceErrorCode::InvalidRequest,
                        kind: ProductSurfaceErrorKind::Validation,
                        status_code: 400,
                        retryable: false,
                        field: Some("follow".to_string()),
                        validation_code: Some(ProductSurfaceValidationCode::InvalidValue),
                    });
                }
                self.query_logs_calls.lock().expect("lock").push(request);
                let response = RebornLogQueryResponse {
                    source: "test".to_string(),
                    entries: Vec::new(),
                    next_cursor: None,
                    tail_supported: true,
                    follow_supported: true,
                };
                Ok(RebornViewPage {
                    payload: serde_json::to_value(response).expect("logs payload"),
                    next_cursor: None,
                })
            }
            id if id == OPERATOR_LOGS_VIEW.id => {
                let mut request: RebornOperatorLogsQuery =
                    serde_json::from_value(query.params).expect("operator logs params");
                request.cursor = query.cursor.or(request.cursor);
                self.query_operator_logs_calls
                    .lock()
                    .expect("lock")
                    .push(request);
                Ok(RebornViewPage {
                    payload: serde_json::to_value(operator_command_response(
                        RebornOperatorArea::Logs,
                    ))
                    .expect("operator logs payload"),
                    next_cursor: None,
                })
            }
            id if id == LLM_CONFIG_VIEW.id => {
                *self.get_llm_config_calls.lock().expect("lock") += 1;
                Ok(RebornViewPage {
                    payload: serde_json::to_value(llm_snapshot("openai"))
                        .expect("llm config payload"),
                    next_cursor: None,
                })
            }
            id if id == THREADS_VIEW.id => {
                let mut request: ProductListThreadsRequest =
                    serde_json::from_value(query.params).expect("thread list params");
                request.cursor = query.cursor.or(request.cursor);
                self.list_threads_calls.lock().expect("lock").push(request);
                Ok(RebornViewPage {
                    payload: serde_json::to_value(RebornListThreadsResponse {
                        threads: Vec::new(),
                        next_cursor: None,
                    })
                    .expect("thread list payload"),
                    next_cursor: None,
                })
            }
            id if id == TIMELINE_VIEW.id => {
                let mut request: RebornTimelineRequest =
                    serde_json::from_value(query.params).expect("timeline params");
                request.cursor = query.cursor.or(request.cursor);
                let response = self.get_timeline(caller, request).await?;
                Ok(RebornViewPage {
                    payload: serde_json::to_value(response).expect("timeline payload"),
                    next_cursor: None,
                })
            }
            id if id == GLOBAL_AUTO_APPROVE_VIEW.id => {
                let _: RebornGlobalAutoApproveRequest =
                    serde_json::from_value(query.params).expect("global auto approve params");
                let enabled = self.global_auto_approve_enabled(caller).await?;
                Ok(RebornViewPage {
                    payload: serde_json::to_value(RebornGlobalAutoApproveResponse { enabled })
                        .expect("global auto approve payload"),
                    next_cursor: None,
                })
            }
            id if id == ADMIN_USERS_VIEW.id => {
                let mut request: RebornAdminUserListQuery =
                    serde_json::from_value(query.params).expect("admin users params");
                request.cursor = query.cursor.or(request.cursor);
                Ok(RebornViewPage {
                    payload: serde_json::to_value(RebornAdminUserListResponse {
                        users: vec![sample_admin_user("user-admin")],
                        next_cursor: None,
                    })
                    .expect("admin users payload"),
                    next_cursor: None,
                })
            }
            id if id == ADMIN_USER_VIEW.id => {
                let request: RebornAdminUserRequest =
                    serde_json::from_value(query.params).expect("admin user params");
                Ok(RebornViewPage {
                    payload: serde_json::to_value(RebornAdminUserResponse {
                        user: sample_admin_user(request.user_id.as_str()),
                    })
                    .expect("admin user payload"),
                    next_cursor: None,
                })
            }
            id if id == ADMIN_USER_SECRETS_VIEW.id => {
                let _: RebornAdminUserRequest =
                    serde_json::from_value(query.params).expect("admin user secrets params");
                Ok(RebornViewPage {
                    payload: serde_json::to_value(RebornAdminUserSecretsListResponse {
                        secrets: vec![AdminUserSecretMeta {
                            handle: "openai_api_key".to_string(),
                            created_at: Some("2026-06-17T00:00:00Z".to_string()),
                            updated_at: Some("2026-06-17T00:00:00Z".to_string()),
                        }],
                    })
                    .expect("admin user secrets payload"),
                    next_cursor: None,
                })
            }
            id if id == AUTOMATIONS_VIEW.id => {
                let request: ProductListAutomationsRequest =
                    serde_json::from_value(query.params).expect("automation list params");
                self.list_automations_calls
                    .lock()
                    .expect("lock")
                    .push(request);
                if let Some(error) = self
                    .next_list_automations_error
                    .lock()
                    .expect("lock")
                    .take()
                {
                    return Err(error);
                }
                Ok(RebornViewPage {
                    payload: serde_json::to_value(RebornListAutomationsResponse {
                        automations: vec![
                            automation_info("automation-listed", "Daily status", "0 9 * * *"),
                            automation_info("automation-alpha", "Renamed status", "0 9 * * *"),
                        ],
                        scheduler_enabled: true,
                    })
                    .expect("automation list payload"),
                    next_cursor: None,
                })
            }
            id if id == OUTBOUND_PREFERENCES_VIEW.id => {
                *self.get_outbound_preferences_calls.lock().expect("lock") += 1;
                Ok(RebornViewPage {
                    payload: serde_json::to_value(outbound_preferences_response("slack-dm-alpha"))
                        .expect("outbound preferences payload"),
                    next_cursor: None,
                })
            }
            id if id == OUTBOUND_DELIVERY_TARGETS_VIEW.id => {
                *self
                    .list_outbound_delivery_targets_calls
                    .lock()
                    .expect("lock") += 1;
                Ok(RebornViewPage {
                    payload: serde_json::to_value(outbound_delivery_targets_response())
                        .expect("outbound delivery targets payload"),
                    next_cursor: None,
                })
            }
            id if id == TRACE_CREDITS_VIEW.id => Ok(RebornViewPage {
                payload: serde_json::to_value(trace_credits_response())
                    .expect("trace credits payload"),
                next_cursor: None,
            }),
            id if id == TRACE_ACCOUNT_TRACES_VIEW.id => {
                let user_id = caller.actor().user_id.as_str().to_string();
                self.trace_account_traces_callers
                    .lock()
                    .expect("lock")
                    .push(user_id);
                Ok(RebornViewPage {
                    payload: serde_json::to_value(RebornAccountTracesResponse {
                        enrolled: false,
                        traces: vec![],
                    })
                    .expect("trace account payload"),
                    next_cursor: None,
                })
            }
            id if id == OPERATOR_CONFIG_LIST_VIEW.id => {
                *self.list_operator_config_calls.lock().expect("lock") += 1;
                Ok(RebornViewPage {
                    payload: serde_json::to_value(RebornOperatorConfigListResponse {
                        entries: self.operator_config_entries.lock().expect("lock").clone(),
                        precedence: vec!["default".to_string()],
                        diagnostics: Vec::new(),
                    })
                    .expect("operator config list payload"),
                    next_cursor: None,
                })
            }
            id if id == OPERATOR_CONFIG_KEY_VIEW.id => {
                let key = query.params["key"]
                    .as_str()
                    .expect("operator config key param")
                    .to_string();
                self.get_operator_config_key_calls
                    .lock()
                    .expect("lock")
                    .push(key.clone());
                let entry = self
                    .operator_config_entries
                    .lock()
                    .expect("lock")
                    .iter()
                    .find(|entry| entry.key == key)
                    .cloned()
                    .unwrap_or_else(|| operator_config_entry(key, serde_json::json!("configured")));
                Ok(RebornViewPage {
                    payload: serde_json::to_value(RebornOperatorConfigGetResponse { entry })
                        .expect("operator config key payload"),
                    next_cursor: None,
                })
            }
            id if id == OPERATOR_CONFIG_VALIDATE_VIEW.id => {
                let request: RebornOperatorConfigValidateRequest =
                    serde_json::from_value(query.params).expect("operator config validate params");
                self.validate_operator_config_calls
                    .lock()
                    .expect("lock")
                    .push(request.keys.clone());
                let diagnostics = operator_config_validation_diagnostics(request.keys);
                Ok(RebornViewPage {
                    payload: serde_json::to_value(RebornOperatorConfigValidateResponse {
                        valid: diagnostics.is_empty(),
                        diagnostics,
                    })
                    .expect("operator config validate payload"),
                    next_cursor: None,
                })
            }
            id if id == OPERATOR_SETUP_VIEW.id => {
                let setup_input = self
                    .invoke_calls
                    .lock()
                    .expect("lock")
                    .iter()
                    .rev()
                    .find(|(capability, _, _)| {
                        capability.as_str() == OPERATOR_SETUP_RUN_CAPABILITY_ID
                    })
                    .map(|(_, input, _)| input.clone());
                let active_provider_id = setup_input
                    .as_ref()
                    .and_then(|input| input.get("provider_id"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string);
                let active_model = setup_input
                    .as_ref()
                    .and_then(|input| input.get("model"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string);
                Ok(RebornViewPage {
                    payload: serde_json::to_value(RebornOperatorSetupResponse {
                        area: RebornOperatorArea::Setup,
                        status: RebornOperatorSetupStatus::Incomplete,
                        message: "setup incomplete".to_string(),
                        active_provider_id,
                        active_model,
                        steps: Vec::new(),
                        diagnostics: Vec::new(),
                    })
                    .expect("operator setup payload"),
                    next_cursor: None,
                })
            }
            id if id == OPERATOR_DIAGNOSTICS_VIEW.id => Ok(RebornViewPage {
                payload: serde_json::to_value(operator_config_diagnostic_command_plane_response(
                    RebornOperatorArea::Diagnostics,
                ))
                .expect("operator diagnostics payload"),
                next_cursor: None,
            }),
            id if id == OPERATOR_STATUS_VIEW.id => Ok(RebornViewPage {
                payload: serde_json::to_value(operator_config_diagnostic_command_plane_response(
                    RebornOperatorArea::Status,
                ))
                .expect("operator status payload"),
                next_cursor: None,
            }),
            id if id == EXTENSIONS_VIEW.id => Ok(RebornViewPage {
                payload: serde_json::to_value(
                    self.next_extensions_view
                        .lock()
                        .expect("lock")
                        .take()
                        .unwrap_or_else(|| RebornExtensionListResponse {
                            extensions: vec![extension_info("google-calendar", true)],
                        }),
                )
                .expect("extensions payload"),
                next_cursor: None,
            }),
            id if id == EXTENSION_REGISTRY_VIEW.id => Ok(RebornViewPage {
                payload: serde_json::to_value(RebornExtensionRegistryResponse {
                    entries: Vec::new(),
                })
                .expect("extension registry payload"),
                next_cursor: None,
            }),
            id if id == EXTENSION_SETUP_VIEW.id => {
                let package_id = query.params["package_id"]
                    .as_str()
                    .expect("extension setup package_id param");
                let package_ref =
                    LifecyclePackageRef::new(LifecyclePackageKind::Extension, package_id)
                        .expect("extension setup package ref");
                Ok(RebornViewPage {
                    payload: serde_json::to_value(extension_setup_response(package_ref))
                        .expect("extension setup payload"),
                    next_cursor: None,
                })
            }
            id if id == SKILLS_VIEW.id => Ok(RebornViewPage {
                payload: serde_json::to_value(RebornSkillListResponse {
                    skills: Vec::new(),
                    count: 0,
                    auto_activate_learned: true,
                })
                .expect("skills payload"),
                next_cursor: None,
            }),
            id if id == SKILL_SEARCH_VIEW.id => Ok(RebornViewPage {
                payload: serde_json::to_value(RebornSkillSearchResponse {
                    catalog: Vec::new(),
                    installed: Vec::new(),
                    registry_url: String::new(),
                    catalog_error: None,
                })
                .expect("skill search payload"),
                next_cursor: None,
            }),
            id if id == SKILL_CONTENT_VIEW.id => {
                let name = query.params["name"].as_str().expect("skill name param");
                Ok(RebornViewPage {
                    payload: serde_json::to_value(RebornSkillContentResponse {
                        name: name.to_string(),
                        content: format!("# {name}\n"),
                    })
                    .expect("skill content payload"),
                    next_cursor: None,
                })
            }
            id if id == PROJECT_FS_LIST_VIEW.id => {
                let request: RebornProjectFsListRequest =
                    serde_json::from_value(query.params).expect("project fs list params");
                Ok(RebornViewPage {
                    payload: serde_json::to_value(RebornProjectFsListResponse {
                        entries: vec![ProjectFsEntry {
                            name: "report.md".to_string(),
                            path: format!("{}/report.md", request.path.trim_end_matches('/')),
                            kind: ProjectFsEntryKind::File,
                        }],
                    })
                    .expect("project fs list payload"),
                    next_cursor: None,
                })
            }
            id if id == PROJECT_FS_STAT_VIEW.id => {
                let request: RebornProjectFsStatRequest =
                    serde_json::from_value(query.params).expect("project fs stat params");
                Ok(RebornViewPage {
                    payload: serde_json::to_value(RebornProjectFsStatResponse {
                        stat: ProjectFsStat {
                            path: request.path,
                            kind: ProjectFsEntryKind::File,
                            size_bytes: 7,
                            mime_type: "text/markdown".to_string(),
                        },
                    })
                    .expect("project fs stat payload"),
                    next_cursor: None,
                })
            }
            id if id == FS_MOUNTS_VIEW.id => Ok(RebornViewPage {
                payload: serde_json::to_value(RebornFsMountsResponse {
                    mounts: vec![
                        RebornFsMountInfo {
                            mount: FsMount::Memory,
                            label: "Memory".to_string(),
                        },
                        RebornFsMountInfo {
                            mount: FsMount::Workspace,
                            label: "Workspace files".to_string(),
                        },
                    ],
                })
                .expect("fs mounts payload"),
                next_cursor: None,
            }),
            id if id == FS_LIST_VIEW.id => {
                let request: RebornFsListRequest =
                    serde_json::from_value(query.params).expect("fs list params");
                self.browse_fs_calls
                    .lock()
                    .expect("lock")
                    .push(request.clone());
                Ok(RebornViewPage {
                    payload: serde_json::to_value(RebornFsListResponse {
                        mount: request.mount,
                        path: request.path,
                        entries: vec![ProjectFsEntry {
                            name: "today.md".to_string(),
                            path: "daily/today.md".to_string(),
                            kind: ProjectFsEntryKind::File,
                        }],
                    })
                    .expect("fs list payload"),
                    next_cursor: None,
                })
            }
            id if id == FS_STAT_VIEW.id => {
                let request: RebornFsStatRequest =
                    serde_json::from_value(query.params).expect("fs stat params");
                Ok(RebornViewPage {
                    payload: serde_json::to_value(RebornFsStatResponse {
                        stat: ProjectFsStat {
                            path: request.path,
                            kind: ProjectFsEntryKind::File,
                            size_bytes: 7,
                            mime_type: "text/markdown".to_string(),
                        },
                    })
                    .expect("fs stat payload"),
                    next_cursor: None,
                })
            }
            id if id == PROJECTS_VIEW.id => Ok(RebornViewPage {
                payload: serde_json::to_value(RebornListProjectsResponse {
                    projects: vec![sample_project_info("project-alpha")],
                })
                .expect("projects payload"),
                next_cursor: None,
            }),
            id if id == PROJECT_VIEW.id => {
                let request: RebornGetProjectRequest =
                    serde_json::from_value(query.params).expect("project params");
                Ok(RebornViewPage {
                    payload: serde_json::to_value(RebornProjectResponse {
                        project: sample_project_info(&request.project_id),
                    })
                    .expect("project payload"),
                    next_cursor: None,
                })
            }
            id if id == PROJECT_MEMBERS_VIEW.id => Ok(RebornViewPage {
                payload: serde_json::to_value(RebornListMembersResponse {
                    members: vec![
                        sample_member_info("user-beta"),
                        sample_member_info("body-user"),
                        sample_member_info("path-user"),
                    ],
                })
                .expect("project members payload"),
                next_cursor: None,
            }),
            _ => Err(rejecting_product_surface_error()),
        }
    }

    async fn read_fs_file(
        &self,
        _caller: ProductSurfaceCaller,
        request: RebornFsReadRequest,
    ) -> Result<ProjectFsFile, ProductSurfaceError> {
        Ok(ProjectFsFile {
            path: request.path,
            filename: Some("today.md".to_string()),
            mime_type: "text/markdown".to_string(),
            size_bytes: 7,
            bytes: b"# notes".to_vec(),
        })
    }

    async fn read_attachment(
        &self,
        _caller: ProductSurfaceCaller,
        request: RebornAttachmentRequest,
    ) -> Result<RebornAttachmentBytes, ProductSurfaceError> {
        self.read_attachment_calls
            .lock()
            .expect("lock")
            .push(request);
        match self.read_attachment_response.lock().expect("lock").clone() {
            Some(bytes) => Ok(bytes),
            None => Err(ProductSurfaceError {
                code: ProductSurfaceErrorCode::NotFound,
                kind: ProductSurfaceErrorKind::NotFound,
                status_code: 404,
                retryable: false,
                field: None,
                validation_code: None,
            }),
        }
    }

    async fn stream_events(
        &self,
        _caller: ProductSurfaceCaller,
        request: RebornStreamEventsRequest,
    ) -> Result<RebornStreamEventsResponse, ProductSurfaceError> {
        self.stream_events_calls
            .lock()
            .expect("lock")
            .push(request.clone());
        self.stream_events_notify.notify_waiters();
        if let Some(response) = self.next_stream_events.lock().expect("lock").pop_front() {
            return response;
        }
        // Empty drain; SSE handler will keep-alive until the test drops it.
        Ok(RebornStreamEventsResponse { events: Vec::new() })
    }

    async fn cancel_run(
        &self,
        _caller: ProductSurfaceCaller,
        request: ProductCancelRunRequest,
    ) -> Result<RebornCancelRunResponse, ProductSurfaceError> {
        self.cancel_run_calls
            .lock()
            .expect("lock")
            .push(request.clone());
        Ok(RebornCancelRunResponse {
            run_id: TurnRunId::new(),
            status: TurnStatus::Cancelled,
            event_cursor: EventCursor(2),
            already_terminal: false,
        })
    }

    async fn resolve_gate(
        &self,
        _caller: ProductSurfaceCaller,
        request: ProductResolveGateRequest,
    ) -> Result<RebornResolveGateResponse, ProductSurfaceError> {
        self.resolve_gate_calls
            .lock()
            .expect("lock")
            .push(request.clone());
        Ok(RebornResolveGateResponse::Resumed(
            RebornResumeGateResponse {
                run_id: TurnRunId::new(),
                status: TurnStatus::Queued,
                event_cursor: EventCursor(3),
            },
        ))
    }

    async fn retry_run(
        &self,
        _caller: ProductSurfaceCaller,
        request: ProductRetryRunRequest,
    ) -> Result<RebornRetryRunResponse, ProductSurfaceError> {
        self.retry_run_calls
            .lock()
            .expect("lock")
            .push(request.clone());
        self.next_retry_run
            .lock()
            .expect("lock")
            .pop_front()
            .expect("retry_run test stub called without queued response")
    }

    async fn set_operator_config_key(
        &self,
        _caller: ProductSurfaceCaller,
        key: String,
        request: RebornOperatorConfigSetRequest,
    ) -> Result<RebornOperatorConfigGetResponse, ProductSurfaceError> {
        self.set_operator_config_key_calls
            .lock()
            .expect("lock")
            .push((key.clone(), request.value.clone()));
        if let Some(error) = self
            .next_set_operator_config_key_error
            .lock()
            .expect("lock")
            .take()
        {
            return Err(error);
        }
        Ok(RebornOperatorConfigGetResponse {
            entry: operator_config_entry(key, request.value),
        })
    }

    async fn run_operator_service_lifecycle(
        &self,
        _caller: ProductSurfaceCaller,
        request: RebornOperatorServiceLifecycleRequest,
    ) -> Result<RebornOperatorCommandPlaneResponse, ProductSurfaceError> {
        self.run_operator_service_lifecycle_calls
            .lock()
            .expect("lock")
            .push(request.action);
        Ok(operator_command_response(
            RebornOperatorArea::ServiceLifecycle,
        ))
    }

    async fn test_llm_connection(
        &self,
        _caller: ProductSurfaceCaller,
        request: LlmProbeRequest,
    ) -> Result<LlmProbeResult, ProductSurfaceError> {
        self.test_llm_connection_calls
            .lock()
            .expect("lock")
            .push(request.provider_id);
        Ok(LlmProbeResult {
            ok: true,
            message: "ok".to_string(),
        })
    }

    async fn list_llm_models(
        &self,
        _caller: ProductSurfaceCaller,
        request: LlmProbeRequest,
    ) -> Result<LlmModelsResult, ProductSurfaceError> {
        self.list_llm_models_calls
            .lock()
            .expect("lock")
            .push(request.provider_id);
        Ok(LlmModelsResult {
            ok: true,
            models: vec!["model-a".to_string()],
            message: String::new(),
        })
    }

    async fn trace_account_login_link(
        &self,
        caller: ProductSurfaceCaller,
    ) -> Result<RebornAccountLoginLinkResponse, ProductSurfaceError> {
        // Capture the forwarded caller (tenant AND user — this is the trusted
        // identity boundary) so the contract test can verify the caller-scoped
        // route threads the authenticated identity, then return a hermetic
        // canned mint (no network / filesystem).
        self.trace_account_login_link_callers
            .lock()
            .expect("lock")
            .push(format!(
                "{}/{}",
                caller.tenant_id.as_str(),
                caller.actor().user_id.as_str()
            ));
        Ok(RebornAccountLoginLinkResponse {
            minted: true,
            enrolled: true,
            url: Some("https://commons.example/account/login?code=stub".to_string()),
        })
    }

    async fn record_product_surface_call(
        &self,
        caller: ProductSurfaceCaller,
        request: RecordedProductSurfaceCallRequest,
    ) -> Result<RecordedProductSurfaceCallResponse, ProductSurfaceError> {
        self.surface_calls
            .lock()
            .expect("lock")
            .push(request.clone());
        if let Some(response) = self.next_surface_response.lock().expect("lock").take() {
            return response;
        }
        let call_id = ProductSurfaceCallId::parse(request.call_id.as_str())
            .ok_or_else(|| service_unavailable_error(false))?;
        match call_id {
            ProductSurfaceCallId::CreateThread => RecordedProductSurfaceCallResponse::json(
                self.create_thread(
                    caller,
                    serde_json::from_value(request.input).expect("input"),
                )
                .await?,
            ),
            ProductSurfaceCallId::SubmitTurn => RecordedProductSurfaceCallResponse::json(
                self.submit_turn(
                    caller,
                    serde_json::from_value(request.input).expect("input"),
                )
                .await?,
            ),
            ProductSurfaceCallId::CancelRun => RecordedProductSurfaceCallResponse::json(
                self.cancel_run(
                    caller,
                    serde_json::from_value(request.input).expect("input"),
                )
                .await?,
            ),
            ProductSurfaceCallId::ResolveGate => RecordedProductSurfaceCallResponse::json(
                self.resolve_gate(
                    caller,
                    serde_json::from_value(request.input).expect("input"),
                )
                .await?,
            ),
            ProductSurfaceCallId::RetryRun => RecordedProductSurfaceCallResponse::json(
                self.retry_run(
                    caller,
                    serde_json::from_value(request.input).expect("input"),
                )
                .await?,
            ),
            ProductSurfaceCallId::ProjectCreate => {
                let _: RebornCreateProjectRequest =
                    serde_json::from_value(request.input).expect("input");
                RecordedProductSurfaceCallResponse::json(RebornProjectResponse {
                    project: sample_project_info("project-created"),
                })
            }
            ProductSurfaceCallId::ProjectFsRead => {
                let request: RebornProjectFsReadRequest =
                    serde_json::from_value(request.input).expect("input");
                Ok(RecordedProductSurfaceCallResponse::project_file(
                    ProjectFsFile {
                        path: request.path,
                        filename: Some("report.md".to_string()),
                        mime_type: "text/markdown".to_string(),
                        size_bytes: 7,
                        bytes: b"# notes".to_vec(),
                    },
                ))
            }
            ProductSurfaceCallId::FsRead => Ok(RecordedProductSurfaceCallResponse::project_file(
                self.read_fs_file(
                    caller,
                    serde_json::from_value(request.input).expect("input"),
                )
                .await?,
            )),
            ProductSurfaceCallId::AttachmentRead => {
                Ok(RecordedProductSurfaceCallResponse::attachment(
                    self.read_attachment(
                        caller,
                        serde_json::from_value(request.input).expect("input"),
                    )
                    .await?,
                ))
            }
            ProductSurfaceCallId::TraceAccountLoginLink => {
                RecordedProductSurfaceCallResponse::json(
                    self.trace_account_login_link(caller).await?,
                )
            }
            ProductSurfaceCallId::TraceHoldAuthorize => {
                let _: RebornTraceHoldAuthorizeProductRequest =
                    serde_json::from_value(request.input).expect("input");
                RecordedProductSurfaceCallResponse::json(RebornTraceHoldAuthorizeResponse {
                    authorized: true,
                })
            }
            ProductSurfaceCallId::OperatorConfigSetKey => {
                let request: RebornOperatorConfigSetProductRequest =
                    serde_json::from_value(request.input).expect("input");
                RecordedProductSurfaceCallResponse::json(
                    self.set_operator_config_key(
                        caller,
                        request.key,
                        RebornOperatorConfigSetRequest {
                            value: request.value,
                        },
                    )
                    .await?,
                )
            }
            ProductSurfaceCallId::OperatorServiceLifecycle => {
                RecordedProductSurfaceCallResponse::json(
                    self.run_operator_service_lifecycle(
                        caller,
                        serde_json::from_value(request.input).expect("input"),
                    )
                    .await?,
                )
            }
            ProductSurfaceCallId::LlmTestConnection => RecordedProductSurfaceCallResponse::json(
                self.test_llm_connection(
                    caller,
                    serde_json::from_value(request.input).expect("input"),
                )
                .await?,
            ),
            ProductSurfaceCallId::LlmListModels => RecordedProductSurfaceCallResponse::json(
                self.list_llm_models(
                    caller,
                    serde_json::from_value(request.input).expect("input"),
                )
                .await?,
            ),
            ProductSurfaceCallId::LlmNearAiLogin => {
                let _: NearAiLoginRequest = serde_json::from_value(request.input).expect("input");
                RecordedProductSurfaceCallResponse::json(NearAiLoginStart {
                    auth_url: "https://near.ai/login".to_string(),
                })
            }
            ProductSurfaceCallId::LlmNearAiWalletLogin => {
                let _: NearAiWalletLoginRequest =
                    serde_json::from_value(request.input).expect("input");
                RecordedProductSurfaceCallResponse::json(NearAiWalletLoginResult { active: true })
            }
            ProductSurfaceCallId::LlmCodexLogin => {
                RecordedProductSurfaceCallResponse::json(CodexLoginStart {
                    user_code: "TEST-CODE".to_string(),
                    verification_uri: "https://openai.com/device".to_string(),
                })
            }
            ProductSurfaceCallId::AdminUserCreate => {
                let request: RebornAdminCreateUserRequest =
                    serde_json::from_value(request.input).expect("input");
                RecordedProductSurfaceCallResponse::json(RebornAdminUserCreatedResponse {
                    user: sample_admin_user(request.email.as_deref().unwrap_or("user-admin")),
                    api_token: "token-test".to_string(),
                })
            }
            ProductSurfaceCallId::AdminUserDeleteSecret => {
                let request: RebornAdminDeleteSecretProductRequest =
                    serde_json::from_value(request.input).expect("input");
                RecordedProductSurfaceCallResponse::json(RebornAdminSecretDeletedResponse {
                    handle: request.handle,
                    deleted: true,
                })
            }
            ProductSurfaceCallId::AutomationPause => {
                let request: RebornAutomationRequest =
                    serde_json::from_value(request.input).expect("input");
                RecordedProductSurfaceCallResponse::json(RebornAutomationMutationResponse {
                    updated: true,
                    automation: Some(automation_info(
                        request.automation_id.as_str(),
                        "Paused status",
                        "*/5 * * * *",
                    )),
                })
            }
            ProductSurfaceCallId::AutomationResume => {
                let request: RebornAutomationRequest =
                    serde_json::from_value(request.input).expect("input");
                RecordedProductSurfaceCallResponse::json(RebornAutomationMutationResponse {
                    updated: true,
                    automation: Some(automation_info(
                        request.automation_id.as_str(),
                        "Resumed status",
                        "*/5 * * * *",
                    )),
                })
            }
            ProductSurfaceCallId::AutomationRename => {
                let request: RebornRenameAutomationProductRequest =
                    serde_json::from_value(request.input).expect("input");
                RecordedProductSurfaceCallResponse::json(RebornAutomationMutationResponse {
                    updated: true,
                    automation: Some(automation_info(
                        request.automation_id.as_str(),
                        request.name.as_deref().unwrap_or("Renamed status"),
                        "*/5 * * * *",
                    )),
                })
            }
            ProductSurfaceCallId::AutomationDelete => {
                let _: RebornAutomationRequest =
                    serde_json::from_value(request.input).expect("input");
                RecordedProductSurfaceCallResponse::json(RebornAutomationMutationResponse {
                    updated: true,
                    automation: None,
                })
            }
        }
    }
}

#[async_trait]
impl ProductSurface for StubServices {
    async fn invoke(
        &self,
        caller: ProductSurfaceCaller,
        request: ironclaw_host_api::ProductSurfaceInvokeRequest,
    ) -> Result<ironclaw_host_api::ProductSurfaceInvokeResponse, ProductSurfaceError> {
        if let Some(call_id) = ProductSurfaceCallId::parse(request.operation_id.as_str()) {
            let output = self
                .record_product_surface_call(
                    caller,
                    RecordedProductSurfaceCallRequest::from_value(call_id, request.input),
                )
                .await?
                .into_value()?;
            return Ok(ironclaw_host_api::ProductSurfaceInvokeResponse { output });
        }

        let output = StubServices::invoke(
            self,
            caller,
            request.operation_id,
            request.input,
            request.activity_id,
        )
        .await?;
        let output = serde_json::to_value(output).map_err(ProductSurfaceError::internal_from)?;
        Ok(ironclaw_host_api::ProductSurfaceInvokeResponse { output })
    }

    async fn query(
        &self,
        caller: ProductSurfaceCaller,
        request: ironclaw_host_api::ProductSurfaceQueryRequest,
    ) -> Result<ironclaw_host_api::ProductSurfaceQueryPage, ProductSurfaceError> {
        let page = StubServices::query(
            self,
            caller,
            RebornViewQuery {
                view_id: request.view_id,
                params: request.input,
                cursor: request.cursor,
            },
        )
        .await?;
        Ok(ironclaw_host_api::ProductSurfaceQueryPage {
            items: vec![page.payload],
            next_cursor: page.next_cursor,
        })
    }

    async fn stream_events(
        &self,
        caller: ProductSurfaceCaller,
        request: ironclaw_host_api::ProductSurfaceStreamRequest,
    ) -> Result<ironclaw_host_api::ProductSurfaceStreamResponse, ProductSurfaceError> {
        let thread_id = request.stream_id.ok_or_else(|| {
            ProductSurfaceError::validation("stream_id", ProductSurfaceValidationCode::MissingField)
        })?;
        let after_cursor = request
            .after_cursor
            .map(ProjectionCursor::new)
            .transpose()
            .map_err(ProductSurfaceError::internal_from)?;
        let response = StubServices::stream_events(
            self,
            caller,
            RebornStreamEventsRequest {
                thread_id,
                after_cursor,
            },
        )
        .await?;
        let events = response
            .events
            .into_iter()
            .map(serde_json::to_value)
            .collect::<Result<Vec<_>, _>>()
            .map_err(ProductSurfaceError::internal_from)?;
        Ok(ironclaw_host_api::ProductSurfaceStreamResponse {
            events,
            next_cursor: None,
        })
    }
}

fn operator_command_response(area: RebornOperatorArea) -> RebornOperatorCommandPlaneResponse {
    RebornOperatorCommandPlaneResponse {
        area,
        status: RebornOperatorSurfaceStatus::Available,
        message: "operator route dispatched".to_string(),
        operator_status: None,
        logs: None,
        service_lifecycle: None,
        diagnostics: Vec::new(),
    }
}

fn operator_config_entry(key: String, value: Value) -> RebornOperatorConfigEntry {
    RebornOperatorConfigEntry {
        key,
        value,
        source: "test".to_string(),
        redacted: false,
        mutable: true,
    }
}

fn extension_setup_response(package_ref: LifecyclePackageRef) -> RebornSetupExtensionResponse {
    RebornSetupExtensionResponse {
        package_ref,
        phase: LifecyclePublicState::SetupNeeded,
        blockers: Vec::new(),
        payload: None,
        secrets: Vec::new(),
        onboarding: None,
    }
}

fn extension_info(id: &str, active: bool) -> RebornExtensionInfo {
    RebornExtensionInfo {
        package_ref: LifecyclePackageRef::new(LifecyclePackageKind::Extension, id.to_string())
            .expect("extension package ref"),
        display_name: id.to_string(),
        runtime: "first_party".to_string(),
        description: format!("{id} extension"),
        tools: Vec::new(),
        installation_state: if active {
            LifecyclePublicState::Active
        } else {
            LifecyclePublicState::SetupNeeded
        },
        activation_error: None,
        version: Some("1.0.0".to_string()),
        onboarding: None,
        auth_accounts: Vec::new(),
        surfaces: Vec::new(),
        install_scope: None,
    }
}

fn outbound_target_id(target_id: &str) -> RebornOutboundDeliveryTargetId {
    RebornOutboundDeliveryTargetId::new(target_id).expect("valid target id")
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

fn outbound_preferences_response(target_id: &str) -> RebornOutboundPreferencesResponse {
    RebornOutboundPreferencesResponse {
        final_reply_target: Some(outbound_target_summary(target_id)),
        final_reply_target_status: RebornOutboundDeliveryTargetStatus::Available,
        default_modality: Default::default(),
    }
}

fn outbound_delivery_targets_response() -> RebornOutboundDeliveryTargetListResponse {
    RebornOutboundDeliveryTargetListResponse {
        targets: vec![
            RebornOutboundDeliveryTargetOption {
                target: outbound_target_summary("slack-dm-alpha"),
                capabilities: RebornOutboundDeliveryTargetCapabilities {
                    final_replies: true,
                    gate_prompts: true,
                    auth_prompts: true,
                },
            },
            RebornOutboundDeliveryTargetOption {
                target: RebornOutboundDeliveryTargetSummary::new(
                    outbound_target_id("slack-status-alpha"),
                    "slack",
                    "Slack status",
                    None,
                )
                .expect("valid target summary"),
                capabilities: RebornOutboundDeliveryTargetCapabilities {
                    final_replies: false,
                    gate_prompts: false,
                    auth_prompts: false,
                },
            },
        ],
        next_cursor: None,
    }
}

fn trace_credits_response() -> RebornTraceCreditsResponse {
    RebornTraceCreditsResponse {
        enrolled: false,
        pending_credit: 0.0,
        final_credit: 0.0,
        delayed_credit_delta: 0.0,
        submissions_total: 0,
        submissions_submitted: 0,
        submissions_accepted: 0,
        submissions_revoked: 0,
        submissions_expired: 0,
        credit_events_total: 0,
        last_submission_at: None,
        last_credit_sync_at: None,
        recent_explanations: Vec::new(),
        manual_review_hold_count: 0,
        holds: Vec::new(),
        note: "Local view as of last sync; authoritative ledger is server-side.".to_string(),
    }
}

fn automation_info(automation_id: &str, name: &str, cron: &str) -> RebornAutomationInfo {
    RebornAutomationInfo {
        automation_id: automation_id.to_string(),
        name: name.to_string(),
        source: RebornAutomationSource::Schedule {
            cron: cron.to_string(),
            timezone: "UTC".to_string(),
        },
        state: RebornAutomationState::Active,
        next_run_at: None,
        last_run_at: None,
        last_status: None,
        recent_runs: vec![RebornAutomationRecentRunInfo {
            run_id: Some(
                TurnRunId::parse("11111111-1111-1111-1111-111111111111").expect("valid run id"),
            ),
            thread_id: Some(ThreadId::new("thread-listed").expect("valid thread id")),
            fire_slot: None,
            status: RebornAutomationRecentRunStatus::Running,
            submitted_at: "2026-06-03T09:00:01Z".parse().expect("submitted at"),
            completed_at: None,
        }],
        is_active: true,
        created_at: None,
        active_hold: None,
    }
}

fn llm_snapshot(provider_id: &str) -> LlmConfigSnapshot {
    LlmConfigSnapshot {
        providers: vec![LlmProviderView {
            id: provider_id.to_string(),
            description: "provider".to_string(),
            adapter: "open_ai_completions".to_string(),
            default_model: "model-a".to_string(),
            base_url: Some("https://api.example.test/v1".to_string()),
            builtin: true,
            active: true,
            active_model: Some("model-a".to_string()),
            api_key_required: true,
            accepts_api_key: true,
            api_key_set: true,
            can_list_models: true,
        }],
        active: Some(LlmActiveSelection {
            provider_id: provider_id.to_string(),
            model: Some("model-a".to_string()),
        }),
    }
}

async fn read_json(response: axum::response::Response) -> Value {
    let bytes = to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("body bytes");
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(bytes.as_ref()).into_owned()))
}

#[tokio::test]
async fn create_thread_dispatches_through_facade() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/threads")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"client_action_id":"act-1"}"#))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let body = read_json(response).await;
    assert!(body["thread"]["thread_id"].is_string(), "thread_id present");
    assert_eq!(
        services.create_thread_calls.lock().expect("lock").len(),
        1,
        "facade called exactly once"
    );
}

#[tokio::test]
async fn delete_thread_path_dispatches_through_facade() {
    let services = Arc::new(StubServices::default());
    services.enqueue_invoke_response(Ok(successful_resolution(ActivityId::new())));
    let router = router_with(services.clone());

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::DELETE)
                .uri("/api/webchat/v2/threads/thread-delete")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let body = read_json(response).await;
    assert_eq!(body["thread_id"], "thread-delete");
    assert_eq!(body["deleted"], true);
    let calls = services.invoke_calls.lock().expect("lock").clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].0,
        CapabilityId::new(THREAD_DELETE_CAPABILITY_ID).expect("capability id")
    );
    assert_eq!(
        calls[0].1,
        serde_json::json!({ "thread_id": "thread-delete" })
    );
}

#[tokio::test]
async fn send_message_path_overrides_body_thread_id() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/threads/thread-from-path/messages")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"client_action_id":"act-1","thread_id":"thread-from-body","content":"hi"}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let calls = services.submit_turn_calls.lock().expect("lock").clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].thread_id.as_deref(),
        Some("thread-from-path"),
        "path segment must win over body field"
    );
}

// Regression: RejectedBusy must round-trip as {"outcome":"rejected_busy","notice":"..."} on the
// POST /messages wire. Per `.claude/rules/testing.md` "Test Through the Caller", the serde tag
// sits between the axum handler and the browser — only a caller-level test catches a missing
// variant or a broken tag.
//
// Fresh-path variant: run metadata is Some — wire must include active_run_id, status,
// event_cursor fields so the client can poll the blocking run.
#[tokio::test]
async fn send_message_rejected_busy_wire_shape() {
    let services = Arc::new(StubServices::default());
    services.set_next_submit_response(RebornSubmitTurnResponse::RejectedBusy {
        thread_id: ThreadId::new("thread-alpha").expect("thread id"),
        accepted_message_ref: ironclaw_turns::AcceptedMessageRef::new("msg:fake").expect("ref"),
        active_run_id: Some(TurnRunId::new()),
        status: Some(TurnStatus::BlockedApproval),
        event_cursor: Some(EventCursor(1)),
        notice: "An approval gate is open on this thread — resolve it (approve or deny) before continuing, then resend your message.".to_string(),
    });
    let router = router_with(services);

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/threads/thread-alpha/messages")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"content":"hello"}"#))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let body = read_json(response).await;
    assert_eq!(
        body["outcome"], "rejected_busy",
        "RejectedBusy must serialize with outcome tag 'rejected_busy'"
    );
    assert!(
        body["notice"]
            .as_str()
            .map(|s| !s.is_empty())
            .unwrap_or(false),
        "RejectedBusy must include a non-empty 'notice' field"
    );
    assert!(
        !body["active_run_id"].is_null(),
        "fresh RejectedBusy wire must include active_run_id when Some"
    );
    assert!(
        !body["status"].is_null(),
        "fresh RejectedBusy wire must include status when Some"
    );
    assert!(
        !body["event_cursor"].is_null(),
        "fresh RejectedBusy wire must include event_cursor when Some"
    );
}

// Test-through-the-caller: the handler must forward the request body's
// `enabled` flag through ProductSurface::invoke, not a hardcoded value.
// Posting `false` and asserting the capability input recorded `false` catches
// the arg-loss class (e.g. a handler that always passes `true`).
#[tokio::test]
async fn set_auto_activate_learned_invokes_capability_with_enabled_flag() {
    let services = Arc::new(StubServices::default());
    services.enqueue_invoke_response(Ok(successful_resolution(ActivityId::new())));
    let router = router_with(services.clone());

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/skills/auto-activate-learned")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"enabled":false}"#))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let body = read_json(response).await;
    assert_eq!(body["success"], true);
    let invoke_calls = services.invoke_calls.lock().expect("lock");
    assert_eq!(invoke_calls.len(), 1);
    assert_eq!(
        invoke_calls[0].0.as_str(),
        SKILL_AUTO_ACTIVATE_LEARNED_SET_CAPABILITY_ID
    );
    assert_eq!(invoke_calls[0].1, serde_json::json!({ "enabled": false }));
}

// Replay-path variant: run metadata is None — wire must omit active_run_id, status,
// event_cursor so the client receives no fabricated run reference it cannot query.
#[tokio::test]
async fn send_message_rejected_busy_replay_wire_shape_omits_run_fields() {
    let services = Arc::new(StubServices::default());
    services.set_next_submit_response(RebornSubmitTurnResponse::RejectedBusy {
        thread_id: ThreadId::new("thread-alpha").expect("thread id"),
        accepted_message_ref: ironclaw_turns::AcceptedMessageRef::new("msg:fake").expect("ref"),
        active_run_id: None,
        status: None,
        event_cursor: None,
        notice: "Ironclaw is still working on a previous message — resend yours once the current task finishes.".to_string(),
    });
    let router = router_with(services);

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/threads/thread-alpha/messages")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"content":"hello"}"#))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let body = read_json(response).await;
    assert_eq!(
        body["outcome"], "rejected_busy",
        "replay RejectedBusy must still serialize with outcome tag 'rejected_busy'"
    );
    assert!(
        body["notice"]
            .as_str()
            .map(|s| !s.is_empty())
            .unwrap_or(false),
        "replay RejectedBusy must include a non-empty 'notice' field"
    );
    assert!(
        body.get("active_run_id").is_none() || body["active_run_id"].is_null(),
        "replay RejectedBusy wire must omit active_run_id when None, got {:?}",
        body.get("active_run_id")
    );
    assert!(
        body.get("status").is_none() || body["status"].is_null(),
        "replay RejectedBusy wire must omit status when None"
    );
    assert!(
        body.get("event_cursor").is_none() || body["event_cursor"].is_null(),
        "replay RejectedBusy wire must omit event_cursor when None"
    );
}

#[tokio::test]
async fn get_timeline_threads_path_into_request() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/threads/thread-x/timeline")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let calls = services.get_timeline_calls.lock().expect("lock").clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].thread_id, "thread-x");
    let queries = services.view_queries.lock().expect("lock").clone();
    assert_eq!(queries.len(), 1);
    assert_eq!(queries[0].view_id.as_str(), TIMELINE_VIEW.id);
}

#[tokio::test]
async fn get_run_artifact_threads_path_and_run_path_into_request() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());
    let run_id = "3d54a1f0-0a7f-4b9c-a350-4258f2fa3e18";

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!(
                    "/api/webchat/v2/threads/thread-x/runs/{run_id}/artifact"
                ))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let payload: Value = serde_json::from_slice(&body).expect("artifact json");
    assert_eq!(payload["schema"], RUN_ARTIFACT_SCHEMA);
    let queries = services.view_queries.lock().expect("lock").clone();
    assert_eq!(queries.len(), 1);
    assert_eq!(queries[0].view_id, RUN_ARTIFACT_VIEW.id);
    let request: RebornRunArtifactRequest =
        serde_json::from_value(queries[0].params.clone()).expect("artifact params");
    assert_eq!(request.thread_id, "thread-x");
    assert_eq!(request.run_id, run_id);
}

// The attachment-bytes route carries three path segments and returns raw
// bytes rather than JSON. Per "Test Through the Caller", drive the real router
// so the Path<(_, _, _)> extractor, the byte response, and the headers are all
// exercised — a facade-only test would miss the path plumbing and Content-Type.
#[tokio::test]
async fn get_attachment_serves_bytes_with_authoritative_content_type() {
    let services = Arc::new(StubServices::default());
    services.set_attachment(RebornAttachmentBytes {
        mime_type: "image/png".to_string(),
        filename: Some("diagram.png".to_string()),
        bytes: vec![1, 2, 3, 4],
    });
    let router = router_with(services.clone());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/threads/thread-x/messages/msg-1/attachments/att-0")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("image/png")
    );
    assert_eq!(
        response
            .headers()
            .get("x-content-type-options")
            .and_then(|v| v.to_str().ok()),
        Some("nosniff")
    );
    let body = to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("body bytes");
    assert_eq!(body.as_ref(), &[1, 2, 3, 4]);

    // The whole (thread, message, attachment) triple reaches the facade — the
    // attachment id alone is not unique across a thread.
    let calls = services.read_attachment_calls.lock().expect("lock").clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].thread_id, "thread-x");
    assert_eq!(calls[0].message_id, "msg-1");
    assert_eq!(calls[0].attachment_id, "att-0");
}

#[tokio::test]
async fn get_attachment_missing_bytes_returns_404() {
    // The default stub leaves the attachment unset, mirroring an unwired
    // reader or an unknown attachment.
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/threads/thread-x/messages/msg-1/attachments/missing")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// Regression for the timeline pagination wire plumbing. Per
// `.claude/rules/testing.md` "Test Through the Caller", a facade-only
// test on `get_timeline` is not enough — the Query<TimelineQuery>
// extractor sits between the URL and the facade, and a future refactor
// that drops or renames a query field would only fail here.
#[tokio::test]
async fn get_timeline_forwards_query_params_to_facade() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(
                    "/api/webchat/v2/threads/thread-x/timeline?limit=42&cursor=opaque-from-browser",
                )
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let calls = services.get_timeline_calls.lock().expect("lock").clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].thread_id, "thread-x");
    assert_eq!(calls[0].limit, Some(42), "?limit= must reach the facade");
    assert_eq!(
        calls[0].cursor.as_deref(),
        Some("opaque-from-browser"),
        "?cursor= must reach the facade"
    );
    let queries = services.view_queries.lock().expect("lock").clone();
    assert_eq!(queries.len(), 1);
    assert_eq!(queries[0].view_id.as_str(), TIMELINE_VIEW.id);
    assert_eq!(queries[0].cursor.as_deref(), Some("opaque-from-browser"));
}

#[tokio::test]
async fn cancel_run_path_overrides_body_run_id() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/threads/thread-x/runs/run-from-path/cancel")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"client_action_id":"cancel-1","thread_id":"other","run_id":"run-from-body","reason":"user_requested"}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let calls = services.cancel_run_calls.lock().expect("lock").clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].thread_id.as_deref(), Some("thread-x"));
    assert_eq!(calls[0].run_id.as_deref(), Some("run-from-path"));
}

#[tokio::test]
async fn resolve_gate_path_overrides_body_gate_ref() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(
                    "/api/webchat/v2/threads/thread-x/runs/run-y/gates/gate-from-path/resolve",
                )
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"client_action_id":"gate-1","thread_id":"other","run_id":"other","gate_ref":"gate-from-body","resolution":"approved"}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let calls = services.resolve_gate_calls.lock().expect("lock").clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].thread_id.as_deref(), Some("thread-x"));
    assert_eq!(calls[0].run_id.as_deref(), Some("run-y"));
    assert_eq!(calls[0].gate_ref.as_deref(), Some("gate-from-path"));
}

#[tokio::test]
async fn retry_run_path_overrides_body_run_id() {
    let services = Arc::new(StubServices::default());
    services.enqueue_retry_run(Ok(RebornRetryRunResponse {
        run_id: TurnRunId::new(),
        status: TurnStatus::Queued,
        event_cursor: EventCursor(5),
    }));
    let router = router_with(services.clone());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/threads/thread-x/runs/run-from-path/retry")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"client_action_id":"retry-1","thread_id":"other","run_id":"run-from-body"}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let calls = services.retry_run_calls.lock().expect("lock").clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].thread_id.as_deref(), Some("thread-x"));
    assert_eq!(calls[0].run_id.as_deref(), Some("run-from-path"));
}

#[tokio::test]
async fn retry_run_non_retryable_error_maps_to_conflict_body() {
    let services = Arc::new(StubServices::default());
    services.enqueue_retry_run(Err(ProductSurfaceError {
        code: ProductSurfaceErrorCode::Conflict,
        kind: ProductSurfaceErrorKind::Conflict,
        status_code: 409,
        retryable: false,
        field: None,
        validation_code: None,
    }));
    let router = router_with(services);

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/threads/thread-x/runs/run-y/retry")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"client_action_id":"retry-not-retryable"}"#))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = read_json(response).await;
    assert_eq!(body["error"], "conflict");
    assert_eq!(body["kind"], "conflict");
    assert_eq!(body["retryable"], false);
}

#[tokio::test]
async fn retry_run_idempotent_replay_returns_same_response_shape() {
    let services = Arc::new(StubServices::default());
    let run_id = TurnRunId::new();
    let replay = RebornRetryRunResponse {
        run_id,
        status: TurnStatus::Queued,
        event_cursor: EventCursor(42),
    };
    services.enqueue_retry_run(Ok(replay.clone()));
    services.enqueue_retry_run(Ok(replay));
    let router = router_with(services.clone());

    async fn post_retry(router: Router) -> axum::response::Response {
        router
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/webchat/v2/threads/thread-x/runs/run-y/retry")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"client_action_id":"retry-replay"}"#))
                    .expect("request"),
            )
            .await
            .expect("oneshot")
    }

    let first = post_retry(router.clone()).await;
    let second = post_retry(router).await;

    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(second.status(), StatusCode::OK);
    assert_eq!(read_json(first).await, read_json(second).await);
    let calls = services.retry_run_calls.lock().expect("lock").clone();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].client_action_id, calls[1].client_action_id);
}

#[tokio::test]
async fn create_thread_error_maps_to_http_status() {
    let services = Arc::new(StubServices::default());
    services.fail_create_thread(ProductSurfaceError {
        code: ProductSurfaceErrorCode::Forbidden,
        kind: ProductSurfaceErrorKind::ParticipantDenied,
        status_code: 403,
        retryable: false,
        field: None,
        validation_code: None,
    });
    let router = router_with(services);

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/threads")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"client_action_id":"act-1"}"#))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body = read_json(response).await;
    assert_eq!(body["error"], "forbidden");
    assert_eq!(body["kind"], "participant_denied");
    assert_eq!(body["retryable"], false);
}

#[tokio::test]
async fn stream_events_emits_sse_content_type_and_drains_facade() {
    let services = Arc::new(StubServices::default());
    let signal = services.stream_events_signal();
    let router = router_with(services.clone());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/threads/thread-x/events")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let content_type = response
        .headers()
        .get(http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(
        content_type.starts_with("text/event-stream"),
        "SSE content type expected, got: {content_type}"
    );

    // The SSE body is lazy — drive it by polling the first frame, which
    // forces the handler's stream future to run. Notify resolves the
    // moment the stub's stream_events is hit, decoupling the assertion
    // from the SSE polling cadence.
    let mut body = response.into_body();
    let _poll = tokio::spawn(async move {
        let _ = body.frame().await;
    });
    tokio::time::timeout(std::time::Duration::from_secs(2), signal.notified())
        .await
        .expect("stream_events must be called within 2s after the body is polled");

    let calls = services.stream_events_calls.lock().expect("lock").len();
    assert!(
        calls >= 1,
        "facade.stream_events must be called at least once after the first SSE frame is read"
    );
}

#[tokio::test]
async fn stream_events_last_event_id_header_takes_precedence_over_query() {
    // Two distinct, parseable cursors so the precedence is observable in
    // the captured RebornStreamEventsRequest — if a future refactor flips
    // the `.or()` order, the facade will see cursor-B and this test fails.
    let header_cursor =
        ironclaw_product::ProjectionCursor::new("cursor-from-header").expect("cursor");
    let query_cursor =
        ironclaw_product::ProjectionCursor::new("cursor-from-query").expect("cursor");
    let header_json = serde_json::to_string(&header_cursor).expect("serialize header cursor");
    let query_json = serde_json::to_string(&query_cursor).expect("serialize query cursor");
    let query_encoded = url_encode(&query_json);

    let services = Arc::new(StubServices::default());
    let signal = services.stream_events_signal();
    let router = router_with(services.clone());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!(
                    "/api/webchat/v2/threads/thread-x/events?after_cursor={query_encoded}"
                ))
                .header("Last-Event-ID", header_json)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body();
    let _poll = tokio::spawn(async move {
        let _ = body.frame().await;
    });
    tokio::time::timeout(std::time::Duration::from_secs(2), signal.notified())
        .await
        .expect("stream_events must be called within 2s after the body is polled");

    let calls = services.stream_events_calls.lock().expect("lock").clone();
    assert_eq!(calls.len(), 1, "facade.stream_events called exactly once");
    assert_eq!(
        calls[0].after_cursor.as_ref(),
        Some(&header_cursor),
        "Last-Event-ID header must win over ?after_cursor= query param"
    );
}

#[tokio::test]
async fn list_automations_forwards_query_limits_to_facade() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/automations?limit=5&run_limit=7")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let body = read_json(response).await;
    assert_eq!(body["automations"][0]["automation_id"], "automation-listed");
    assert_eq!(
        body["automations"][0]["recent_runs"][0]["thread_id"],
        "thread-listed"
    );
    assert_eq!(
        body["automations"][0]["recent_runs"][0]["status"],
        "running"
    );
    // The scheduler status must survive handler serialization onto the wire so
    // the browser can warn when scheduling is off.
    assert_eq!(body["scheduler_enabled"], true);

    let calls = services
        .list_automations_calls
        .lock()
        .expect("lock")
        .clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].limit, Some(5));
    assert_eq!(calls[0].run_limit, Some(7));
}

#[tokio::test]
async fn list_threads_forwards_needs_approval_filter() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/threads?limit=12&needs_approval=true&candidate_thread_id=thread-active")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let calls = services.list_threads_calls.lock().expect("lock").clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].limit, Some(12));
    assert_eq!(
        calls[0].candidate_thread_id.as_deref(),
        Some("thread-active")
    );
    assert!(calls[0].needs_approval);
}

#[tokio::test]
async fn list_automations_omits_limits_and_forwards_none() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/automations")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let body = read_json(response).await;
    assert_eq!(body["automations"][0]["automation_id"], "automation-listed");

    let calls = services
        .list_automations_calls
        .lock()
        .expect("lock")
        .clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].limit, None);
    assert_eq!(calls[0].run_limit, None);
}

#[tokio::test]
async fn pause_and_resume_automation_dispatch_path_id_to_facade() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    let pause_response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/automations/automation-alpha/pause")
                .body(Body::empty())
                .expect("pause request"),
        )
        .await
        .expect("pause oneshot");
    assert_eq!(pause_response.status(), StatusCode::OK);
    let pause_body = read_json(pause_response).await;
    assert_eq!(pause_body["updated"], true);
    assert_eq!(
        pause_body["automation"]["automation_id"],
        "automation-alpha"
    );

    let resume_response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/automations/automation-alpha/resume")
                .body(Body::empty())
                .expect("resume request"),
        )
        .await
        .expect("resume oneshot");
    assert_eq!(resume_response.status(), StatusCode::OK);
    let resume_body = read_json(resume_response).await;
    assert_eq!(resume_body["updated"], true);
    assert_eq!(
        resume_body["automation"]["automation_id"],
        "automation-alpha"
    );

    let calls = services.surface_calls.lock().expect("lock").clone();
    assert_eq!(calls.len(), 2);
    assert_eq!(
        calls[0].call_id,
        ProductSurfaceCallId::AutomationPause.as_str()
    );
    assert_eq!(
        calls[0].input,
        serde_json::json!({ "automation_id": "automation-alpha" })
    );
    assert_eq!(
        calls[1].call_id,
        ProductSurfaceCallId::AutomationResume.as_str()
    );
    assert_eq!(
        calls[1].input,
        serde_json::json!({ "automation_id": "automation-alpha" })
    );
}

#[tokio::test]
async fn rename_automation_dispatches_path_id_and_body_to_facade() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/automations/automation-alpha")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"name":"Renamed status"}"#))
                .expect("rename request"),
        )
        .await
        .expect("rename oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let body = read_json(response).await;
    assert_eq!(body["updated"], true);
    assert_eq!(body["automation"]["automation_id"], "automation-alpha");
    assert_eq!(body["automation"]["name"], "Renamed status");

    let calls = services.surface_calls.lock().expect("lock").clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].call_id,
        ProductSurfaceCallId::AutomationRename.as_str()
    );
    assert_eq!(
        calls[0].input,
        serde_json::json!({
            "automation_id": "automation-alpha",
            "name": "Renamed status"
        })
    );
}

#[tokio::test]
async fn rename_automation_error_maps_to_http_status() {
    for (error, expected_status, expected_code, expected_kind, expected_retryable) in [
        (
            ProductSurfaceError {
                code: ProductSurfaceErrorCode::InvalidRequest,
                kind: ProductSurfaceErrorKind::Validation,
                status_code: 400,
                retryable: false,
                field: Some("name".to_string()),
                validation_code: Some(ProductSurfaceValidationCode::Blank),
            },
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "validation",
            false,
        ),
        (
            ProductSurfaceError {
                code: ProductSurfaceErrorCode::Forbidden,
                kind: ProductSurfaceErrorKind::ParticipantDenied,
                status_code: 403,
                retryable: false,
                field: None,
                validation_code: None,
            },
            StatusCode::FORBIDDEN,
            "forbidden",
            "participant_denied",
            false,
        ),
    ] {
        let services = Arc::new(StubServices::default());
        services.enqueue_operation_response(Err(error));
        let router = router_with(services.clone());

        let response = router
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/webchat/v2/automations/automation-alpha")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"name":"Renamed status"}"#))
                    .expect("rename request"),
            )
            .await
            .expect("rename oneshot");

        assert_eq!(response.status(), expected_status);
        let body = read_json(response).await;
        assert_eq!(body["error"], expected_code);
        assert_eq!(body["kind"], expected_kind);
        assert_eq!(body["retryable"], expected_retryable);
        let calls = services.surface_calls.lock().expect("lock").clone();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].call_id,
            ProductSurfaceCallId::AutomationRename.as_str()
        );
    }
}

#[tokio::test]
async fn trace_credits_returns_caller_scoped_unenrolled_zero_state() {
    // The handler reads through the descriptor-backed ProductSurface query.
    // A unique caller keeps the route shape pinned to authenticated identity.
    let user_id = format!(
        "webui-v2-trace-credits-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    );
    let unique_caller = ProductSurfaceCaller::new(
        TenantId::new("tenant-alpha").expect("tenant"),
        UserId::new(user_id.as_str()).expect("user"),
        None,
        None,
    );
    let services = Arc::new(StubServices::default());
    let router = webui_v2_router(WebUiV2State::new(
        services.clone(),
        DEFAULT_SSE_MAX_CONCURRENT_PER_CALLER,
    ))
    .layer(axum::Extension(unique_caller))
    .layer(axum::Extension(WebUiV2Capabilities::default()));

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/traces/credit")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let body = read_json(response).await;
    assert_eq!(body["enrolled"], false);
    assert_eq!(body["submissions_total"], 0);
    assert_eq!(body["submissions_submitted"], 0);
    assert_eq!(body["pending_credit"], 0.0);
    assert_eq!(body["final_credit"], 0.0);
    assert!(
        body["note"]
            .as_str()
            .expect("note")
            .contains("authoritative ledger is server-side")
    );
    let view_ids: Vec<String> = services
        .view_queries
        .lock()
        .expect("lock")
        .iter()
        .map(|query| query.view_id.clone())
        .collect();
    assert!(view_ids.contains(&TRACE_CREDITS_VIEW.id.to_string()));
}

#[tokio::test]
async fn trace_account_traces_returns_caller_scoped_unenrolled_zero_state() {
    // The handler reads through the descriptor-backed ProductSurface query.
    // A unique caller keeps the route shape pinned to authenticated identity.
    let user_id = format!(
        "webui-v2-account-traces-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    );
    let unique_caller = ProductSurfaceCaller::new(
        TenantId::new("tenant-alpha").expect("tenant"),
        UserId::new(user_id.as_str()).expect("user"),
        None,
        None,
    );
    let services = Arc::new(StubServices::default());
    let router = webui_v2_router(WebUiV2State::new(
        services.clone(),
        DEFAULT_SSE_MAX_CONCURRENT_PER_CALLER,
    ))
    .layer(axum::Extension(unique_caller))
    .layer(axum::Extension(WebUiV2Capabilities::default()));

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/traces/account")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let body = read_json(response).await;
    assert_eq!(body["enrolled"], false);
    assert_eq!(body["traces"].as_array().expect("traces array").len(), 0);

    // The route is caller-scoped: the handler must forward the authenticated
    // caller's user id to the facade (fails if it stops threading the caller).
    assert_eq!(
        services
            .trace_account_traces_callers
            .lock()
            .expect("lock")
            .clone(),
        vec![user_id],
    );
    let view_ids: Vec<String> = services
        .view_queries
        .lock()
        .expect("lock")
        .iter()
        .map(|query| query.view_id.clone())
        .collect();
    assert!(view_ids.contains(&TRACE_ACCOUNT_TRACES_VIEW.id.to_string()));
}

#[tokio::test]
async fn trace_account_login_link_returns_minted_url_to_caller_scope() {
    // POST /traces/account-login-link is caller-scoped and returns the minted
    // one-time URL in the authenticated response body — the only delivery
    // channel hosted users have (no host-file access).
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/traces/account-login-link")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let body = read_json(response).await;
    assert_eq!(body["minted"], true);
    assert_eq!(body["enrolled"], true);
    assert_eq!(
        body["url"],
        "https://commons.example/account/login?code=stub"
    );

    // The route must forward the authenticated caller's tenant AND user id to
    // the facade — the scope is trusted-caller-derived, never request input
    // (fails if the handler stops threading the caller).
    assert_eq!(
        services
            .trace_account_login_link_callers
            .lock()
            .expect("lock")
            .clone(),
        vec!["tenant-alpha/user-alpha".to_string()],
    );
}

#[tokio::test]
async fn delete_automation_dispatches_path_id_to_facade() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());
    services.enqueue_invoke_response(Ok(successful_resolution(ActivityId::new())));

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::DELETE)
                .uri("/api/webchat/v2/automations/automation-alpha")
                .body(Body::empty())
                .expect("delete request"),
        )
        .await
        .expect("delete oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let body = read_json(response).await;
    assert_eq!(body["updated"], true);
    assert!(body.get("automation").is_none() || body["automation"].is_null());
    let calls = services.surface_calls.lock().expect("lock").clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].call_id,
        ProductSurfaceCallId::AutomationDelete.as_str()
    );
    assert_eq!(
        calls[0].input,
        serde_json::json!({ "automation_id": "automation-alpha" })
    );
}

#[tokio::test]
async fn delete_automation_error_maps_to_http_status() {
    for (error, expected_status, expected_code, expected_kind, expected_retryable) in [
        (
            ProductSurfaceError {
                code: ProductSurfaceErrorCode::Forbidden,
                kind: ProductSurfaceErrorKind::ParticipantDenied,
                status_code: 403,
                retryable: false,
                field: None,
                validation_code: None,
            },
            StatusCode::FORBIDDEN,
            "forbidden",
            "participant_denied",
            false,
        ),
        (
            ProductSurfaceError {
                code: ProductSurfaceErrorCode::Unavailable,
                kind: ProductSurfaceErrorKind::ServiceUnavailable,
                status_code: 503,
                retryable: true,
                field: None,
                validation_code: None,
            },
            StatusCode::SERVICE_UNAVAILABLE,
            "unavailable",
            "service_unavailable",
            true,
        ),
    ] {
        let services = Arc::new(StubServices::default());
        services.enqueue_operation_response(Err(error));
        let router = router_with(services.clone());

        let response = router
            .oneshot(
                Request::builder()
                    .method(Method::DELETE)
                    .uri("/api/webchat/v2/automations/automation-alpha")
                    .body(Body::empty())
                    .expect("delete request"),
            )
            .await
            .expect("delete oneshot");

        assert_eq!(response.status(), expected_status);
        let body = read_json(response).await;
        assert_eq!(body["error"], expected_code);
        assert_eq!(body["kind"], expected_kind);
        assert_eq!(body["retryable"], expected_retryable);
        let calls = services.surface_calls.lock().expect("lock").clone();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].call_id,
            ProductSurfaceCallId::AutomationDelete.as_str()
        );
    }
}

#[tokio::test]
async fn list_automations_rejects_invalid_limit_query_with_400() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/automations?limit=not-a-number")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(
        services
            .list_automations_calls
            .lock()
            .expect("lock")
            .is_empty(),
        "invalid query input must be rejected before reaching the facade"
    );
}

#[tokio::test]
async fn list_automations_rejects_invalid_run_limit_query_with_400() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/automations?run_limit=not-a-number")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(
        services
            .list_automations_calls
            .lock()
            .expect("lock")
            .is_empty(),
        "invalid query input must be rejected before reaching the facade"
    );
}

#[tokio::test]
async fn list_automations_error_maps_to_http_status() {
    let services = Arc::new(StubServices::default());
    services.fail_list_automations(ProductSurfaceError {
        code: ProductSurfaceErrorCode::Forbidden,
        kind: ProductSurfaceErrorKind::ParticipantDenied,
        status_code: 403,
        retryable: false,
        field: None,
        validation_code: None,
    });
    let router = router_with(services);

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/automations")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body = read_json(response).await;
    assert_eq!(body["error"], "forbidden");
    assert_eq!(body["kind"], "participant_denied");
    assert_eq!(body["retryable"], false);
}

#[tokio::test]
async fn list_automations_include_completed_true_forwarded_to_facade() {
    // ?include_completed=true must be parsed and forwarded as `true` in the
    // ProductListAutomationsRequest so the facade can widen its exclusion slice.
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/automations?include_completed=true")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let calls = services
        .list_automations_calls
        .lock()
        .expect("lock")
        .clone();
    assert_eq!(calls.len(), 1);
    assert!(
        calls[0].include_completed,
        "include_completed=true must be forwarded to the facade"
    );
}

#[tokio::test]
async fn list_automations_include_completed_absent_defaults_to_false() {
    // No ?include_completed query param → `include_completed` must default to
    // false so existing callers that do not set the flag are unaffected.
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/automations")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let calls = services
        .list_automations_calls
        .lock()
        .expect("lock")
        .clone();
    assert_eq!(calls.len(), 1);
    assert!(
        !calls[0].include_completed,
        "absent include_completed must default to false (active-only)"
    );
}

// Regression: malformed `?include_completed=garbage` must be rejected at the
// Query extractor level (400 Bad Request) before the handler or facade run.
// The field is a plain `bool`; `serde_urlencoded` does not silently default
// unparseable values — it returns a deserialization error, which axum maps to
// 400. There is no silent fallback to `false`.
#[tokio::test]
async fn list_automations_malformed_include_completed_rejected_with_400() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/automations?include_completed=notabool")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "malformed include_completed must be rejected at query deserialization with 400, \
         not silently defaulted to false"
    );
    assert!(
        services
            .list_automations_calls
            .lock()
            .expect("lock")
            .is_empty(),
        "malformed include_completed must be rejected before reaching the facade"
    );
}

#[tokio::test]
async fn get_outbound_preferences_dispatches_through_facade() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/outbound/preferences")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let body = read_json(response).await;
    assert_eq!(body["final_reply_target"]["target_id"], "slack-dm-alpha");
    assert_eq!(body["final_reply_target_status"], "available");
    assert_eq!(
        *services
            .get_outbound_preferences_calls
            .lock()
            .expect("lock"),
        1
    );
    let view_ids: Vec<String> = services
        .view_queries
        .lock()
        .expect("lock")
        .iter()
        .map(|query| query.view_id.clone())
        .collect();
    assert!(view_ids.contains(&OUTBOUND_PREFERENCES_VIEW.id.to_string()));
}

#[tokio::test]
async fn set_outbound_preferences_dispatches_body_through_invoke() {
    let services = Arc::new(StubServices::default());
    services.enqueue_invoke_response(Ok(successful_resolution(ActivityId::new())));
    let router = router_with(services.clone());

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/outbound/preferences")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"final_reply_target_id":"slack-dm-beta","client_action_id":"outbound-save-1"}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let body = read_json(response).await;
    assert_eq!(body["final_reply_target"]["target_id"], "slack-dm-alpha");
    services.enqueue_invoke_response(Ok(successful_resolution(ActivityId::new())));
    let retry_response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/outbound/preferences")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"final_reply_target_id":"slack-dm-beta","client_action_id":"outbound-save-1"}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(retry_response.status(), StatusCode::OK);
    let invoke_calls = services.invoke_calls.lock().expect("lock");
    assert_eq!(invoke_calls.len(), 2);
    assert_eq!(
        invoke_calls[0].0.as_str(),
        OUTBOUND_PREFERENCES_SET_CAPABILITY_ID
    );
    assert_eq!(
        invoke_calls[0].1,
        serde_json::json!({ "final_reply_target_id": "slack-dm-beta" })
    );
    assert_eq!(
        invoke_calls[1].1,
        serde_json::json!({ "final_reply_target_id": "slack-dm-beta" })
    );
    assert_eq!(
        invoke_calls[0].2, invoke_calls[1].2,
        "identical outbound preference retries should reuse ProductSurface activity ids"
    );
    drop(invoke_calls);
    let view_ids: Vec<String> = services
        .view_queries
        .lock()
        .expect("lock")
        .iter()
        .map(|query| query.view_id.clone())
        .collect();
    assert_eq!(
        view_ids,
        vec![
            OUTBOUND_PREFERENCES_VIEW.id.to_string(),
            OUTBOUND_PREFERENCES_VIEW.id.to_string(),
        ]
    );
}

#[tokio::test]
async fn set_outbound_preferences_accepts_explicit_clear() {
    let services = Arc::new(StubServices::default());
    services.enqueue_invoke_response(Ok(successful_resolution(ActivityId::new())));
    let router = router_with(services.clone());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/outbound/preferences")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"final_reply_target_id":null,"client_action_id":"outbound-clear-1"}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let body = read_json(response).await;
    assert_eq!(body["final_reply_target"]["target_id"], "slack-dm-alpha");
    let invoke_calls = services.invoke_calls.lock().expect("lock");
    assert_eq!(invoke_calls.len(), 1);
    assert_eq!(
        invoke_calls[0].0.as_str(),
        OUTBOUND_PREFERENCES_SET_CAPABILITY_ID
    );
    assert_eq!(invoke_calls[0].1, serde_json::json!({}));
    drop(invoke_calls);
    assert_eq!(
        *services
            .get_outbound_preferences_calls
            .lock()
            .expect("lock"),
        1
    );
}

#[tokio::test]
async fn set_outbound_preferences_error_maps_to_http_status() {
    let services = Arc::new(StubServices::default());
    services.enqueue_invoke_response(Err(ProductSurfaceError {
        code: ProductSurfaceErrorCode::NotFound,
        kind: ProductSurfaceErrorKind::NotFound,
        status_code: 404,
        retryable: false,
        field: None,
        validation_code: None,
    }));
    let router = router_with(services);

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/outbound/preferences")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"final_reply_target_id":"target-does-not-exist","client_action_id":"outbound-error-1"}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = read_json(response).await;
    assert_eq!(body["error"], "not_found");
    assert_eq!(body["kind"], "not_found");
    assert_eq!(body["retryable"], false);
}
#[tokio::test]
async fn list_outbound_delivery_targets_uses_product_view() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/outbound/targets")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let body = read_json(response).await;
    assert_eq!(body["targets"][0]["target"]["target_id"], "slack-dm-alpha");
    assert_eq!(body["targets"][0]["capabilities"]["final_replies"], true);
    assert_eq!(
        body["targets"][1]["target"]["target_id"],
        "slack-status-alpha"
    );
    assert_eq!(body["targets"][1]["capabilities"]["final_replies"], false);
    assert_eq!(
        *services
            .list_outbound_delivery_targets_calls
            .lock()
            .expect("lock"),
        1
    );
    let view_ids: Vec<String> = services
        .view_queries
        .lock()
        .expect("lock")
        .iter()
        .map(|query| query.view_id.clone())
        .collect();
    assert!(view_ids.contains(&OUTBOUND_DELIVERY_TARGETS_VIEW.id.to_string()));
}

#[tokio::test]
async fn get_session_returns_caller_identity_and_capabilities() {
    let services = Arc::new(StubServices::default());
    let router = router_with_capabilities(
        services,
        WebUiV2Capabilities {
            operator_webui_config: true,
        },
    );

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/session")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let body = read_json(response).await;
    assert_eq!(body["tenant_id"], "tenant-alpha");
    assert_eq!(body["user_id"], "user-alpha");
    assert_eq!(body["capabilities"]["operator_webui_config"], true);

    // The session advertises the inline-attachment contract so the browser
    // file picker derives its `accept` set and size budgets from the server
    // rather than a static frontend list that can drift. The `accept` tokens
    // must be exactly the shared format registry's output (drift kill), and
    // the budgets must match what `decode_attachments` enforces.
    let expected = ironclaw_product::product_attachment_capabilities();
    let accept: Vec<String> = body["attachments"]["accept"]
        .as_array()
        .expect("attachments.accept is an array")
        .iter()
        .map(|token| {
            token
                .as_str()
                .expect("accept token is a string")
                .to_string()
        })
        .collect();
    assert_eq!(accept, expected.accept);
    // The registry emits exact MIME types *and* canonical extensions (only the
    // supported formats), never broad `image/*` wildcards that would admit
    // unsupported ones. The MIME types keep folder navigation working in the
    // native macOS picker — an extension-only `accept` makes a folder
    // double-click dismiss the dialog instead of opening it.
    assert!(
        accept.iter().any(|t| t == ".png"),
        "registry-derived accept must include an image extension: {accept:?}"
    );
    assert!(
        accept.iter().any(|t| t == "image/png"),
        "registry-derived accept must include the exact image MIME: {accept:?}"
    );
    assert!(
        accept.iter().any(|t| t == ".pdf"),
        "registry-derived accept must include .pdf: {accept:?}"
    );
    assert!(
        !accept.iter().any(|t| t.contains('*')),
        "accept must not advertise wildcards: {accept:?}"
    );
    assert_eq!(body["attachments"]["max_count"], expected.max_count);
    assert_eq!(
        body["attachments"]["max_file_bytes"],
        expected.max_file_bytes
    );
    assert_eq!(
        body["attachments"]["max_total_bytes"],
        expected.max_total_bytes
    );
}

#[tokio::test]
async fn get_session_returns_false_operator_capability_when_capabilities_default() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services);

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/session")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let body = read_json(response).await;
    assert_eq!(body["tenant_id"], "tenant-alpha");
    assert_eq!(body["user_id"], "user-alpha");
    assert_eq!(body["capabilities"]["operator_webui_config"], false);
}

// The browser hides the Projects surface (sidebar entry + `/projects` route)
// unless the deployment opts in. The gate is delivered through the session
// response's `features.reborn_projects` field, fed from
// `WebUiV2State::with_reborn_projects_enabled` at composition. Drive the real
// router (not just the state accessor) so a handler that forgot to surface the
// flag is caught — see `.claude/rules/testing.md` "Test Through the Caller".
#[tokio::test]
async fn get_session_reports_reborn_projects_feature_from_state_flag() {
    for enabled in [false, true] {
        let services = Arc::new(StubServices::default());
        let router = webui_v2_router(
            WebUiV2State::new(services, DEFAULT_SSE_MAX_CONCURRENT_PER_CALLER)
                .with_reborn_projects_enabled(enabled),
        )
        .layer(axum::Extension(caller()))
        .layer(axum::Extension(WebUiV2Capabilities::default()));

        let response = router
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/webchat/v2/session")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("oneshot");

        assert_eq!(response.status(), StatusCode::OK);
        let body = read_json(response).await;
        assert_eq!(
            body["features"]["reborn_projects"], enabled,
            "features.reborn_projects must mirror the state flag (enabled={enabled})"
        );
    }
}

// The approval card hint needs the effective global auto-approve setting. Keep
// that as a narrow facade read surfaced through the session bootstrap feature,
// not an operator config key lookup from the browser or route handler.
#[tokio::test]
async fn get_session_reports_global_auto_approve_feature_from_facade() {
    for enabled in [false, true] {
        let services = Arc::new(StubServices::default());
        *services.global_auto_approve_enabled.lock().expect("lock") = enabled;
        let router = webui_v2_router(WebUiV2State::new(
            services.clone(),
            DEFAULT_SSE_MAX_CONCURRENT_PER_CALLER,
        ))
        .layer(axum::Extension(caller()))
        .layer(axum::Extension(WebUiV2Capabilities::default()));

        let response = router
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/webchat/v2/session")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("oneshot");

        assert_eq!(response.status(), StatusCode::OK);
        let body = read_json(response).await;
        assert_eq!(
            body["features"]["global_auto_approve"], enabled,
            "features.global_auto_approve must mirror the facade flag (enabled={enabled})"
        );
        assert_eq!(
            *services.global_auto_approve_calls.lock().expect("lock"),
            1,
            "session handler should read the feature through the narrow facade"
        );
        assert!(
            services
                .get_operator_config_key_calls
                .lock()
                .expect("lock")
                .is_empty(),
            "session handler must not read arbitrary operator config keys"
        );
    }
}

#[tokio::test]
async fn get_session_refreshes_global_auto_approve_feature_between_requests() {
    let services = Arc::new(StubServices::default());
    let state = WebUiV2State::new(services.clone(), DEFAULT_SSE_MAX_CONCURRENT_PER_CALLER);
    let router = webui_v2_router(state)
        .layer(axum::Extension(caller_for_user("user-alpha")))
        .layer(axum::Extension(WebUiV2Capabilities::default()));

    *services.global_auto_approve_enabled.lock().expect("lock") = false;
    let initial = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/session")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(
        read_json(initial).await["features"]["global_auto_approve"],
        false
    );

    *services.global_auto_approve_enabled.lock().expect("lock") = true;
    let refreshed = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/session")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(
        read_json(refreshed).await["features"]["global_auto_approve"],
        true,
        "session bootstrap must reflect the mutable tenant/user settings flag"
    );
    assert_eq!(
        *services.global_auto_approve_calls.lock().expect("lock"),
        2,
        "session handler should re-read the mutable flag on each bootstrap request"
    );
}

#[tokio::test]
async fn get_session_defaults_global_auto_approve_false_when_facade_read_fails() {
    let services = Arc::new(StubServices::default());
    *services
        .next_global_auto_approve_error
        .lock()
        .expect("lock") = Some(service_unavailable_error(false));
    let router = webui_v2_router(WebUiV2State::new(
        services.clone(),
        DEFAULT_SSE_MAX_CONCURRENT_PER_CALLER,
    ))
    .layer(axum::Extension(caller()))
    .layer(axum::Extension(WebUiV2Capabilities::default()));

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/session")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let body = read_json(response).await;
    assert_eq!(body["features"]["global_auto_approve"], false);
    assert_eq!(*services.global_auto_approve_calls.lock().expect("lock"), 1);
}

#[tokio::test]
async fn get_session_defaults_global_auto_approve_false_when_facade_stalls() {
    let services = Arc::new(StubServices::default());
    *services.stall_global_auto_approve.lock().expect("lock") = true;
    let router = webui_v2_router(WebUiV2State::new(
        services.clone(),
        DEFAULT_SSE_MAX_CONCURRENT_PER_CALLER,
    ))
    .layer(axum::Extension(caller()))
    .layer(axum::Extension(WebUiV2Capabilities::default()));

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/session")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let body = read_json(response).await;
    assert_eq!(body["features"]["global_auto_approve"], false);
    assert_eq!(*services.global_auto_approve_calls.lock().expect("lock"), 1);
}

#[tokio::test]
async fn admin_user_reads_query_product_surface_views() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    let list = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/admin/users?limit=25&cursor=user-before")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("list users");
    assert_eq!(list.status(), StatusCode::OK);
    let body = read_json(list).await;
    assert_eq!(body["users"][0]["user_id"], "user-admin");

    let get = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/admin/users/user-admin")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("get user");
    assert_eq!(get.status(), StatusCode::OK);
    let body = read_json(get).await;
    assert_eq!(body["user"]["user_id"], "user-admin");

    let secrets = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/admin/users/user-admin/secrets")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("list secrets");
    assert_eq!(secrets.status(), StatusCode::OK);
    let body = read_json(secrets).await;
    assert_eq!(body["secrets"][0]["handle"], "openai_api_key");

    let queries = services.view_queries.lock().expect("lock").clone();
    assert_eq!(queries.len(), 3);
    assert_eq!(queries[0].view_id.as_str(), ADMIN_USERS_VIEW.id);
    assert_eq!(queries[0].cursor.as_deref(), Some("user-before"));
    assert_eq!(queries[1].view_id.as_str(), ADMIN_USER_VIEW.id);
    assert_eq!(queries[2].view_id.as_str(), ADMIN_USER_SECRETS_VIEW.id);
}

#[tokio::test]
async fn admin_user_mutations_invoke_product_capabilities_and_read_back_user() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    services.enqueue_invoke_response(Ok(successful_resolution(ActivityId::new())));
    let update = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PATCH)
                .uri("/api/webchat/v2/admin/users/user-admin")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"display_name":"Renamed"}"#))
                .expect("request"),
        )
        .await
        .expect("update user");
    assert_eq!(update.status(), StatusCode::OK);

    services.enqueue_invoke_response(Ok(successful_resolution(ActivityId::new())));
    let status = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/admin/users/user-admin/status")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"status":"suspended"}"#))
                .expect("request"),
        )
        .await
        .expect("set status");
    assert_eq!(status.status(), StatusCode::OK);

    services.enqueue_invoke_response(Ok(successful_resolution(ActivityId::new())));
    let role = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/admin/users/user-admin/role")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"role":"member"}"#))
                .expect("request"),
        )
        .await
        .expect("set role");
    assert_eq!(role.status(), StatusCode::OK);

    services.enqueue_invoke_response(Ok(successful_resolution(ActivityId::new())));
    let delete = router
        .oneshot(
            Request::builder()
                .method(Method::DELETE)
                .uri("/api/webchat/v2/admin/users/user-admin")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("delete user");
    assert_eq!(delete.status(), StatusCode::OK);
    let body = read_json(delete).await;
    assert_eq!(body["user_id"], "user-admin");
    assert_eq!(body["deleted"], true);

    let invoke_calls = services.invoke_calls.lock().expect("lock").clone();
    assert_eq!(invoke_calls.len(), 4);
    assert_eq!(
        invoke_calls[0].0,
        CapabilityId::new(ADMIN_USER_UPDATE_CAPABILITY_ID).expect("capability id")
    );
    let update_input: RebornAdminUpdateUserProductRequest =
        serde_json::from_value(invoke_calls[0].1.clone()).expect("update input");
    assert_eq!(update_input.user_id.as_str(), "user-admin");
    assert_eq!(update_input.display_name.as_deref(), Some("Renamed"));
    assert_eq!(
        invoke_calls[1].0,
        CapabilityId::new(ADMIN_USER_SET_STATUS_CAPABILITY_ID).expect("capability id")
    );
    let status_input: RebornAdminSetStatusProductRequest =
        serde_json::from_value(invoke_calls[1].1.clone()).expect("status input");
    assert_eq!(status_input.user_id.as_str(), "user-admin");
    assert_eq!(status_input.status, AdminUserStatus::Suspended);
    assert_eq!(
        invoke_calls[2].0,
        CapabilityId::new(ADMIN_USER_SET_ROLE_CAPABILITY_ID).expect("capability id")
    );
    let role_input: RebornAdminSetRoleProductRequest =
        serde_json::from_value(invoke_calls[2].1.clone()).expect("role input");
    assert_eq!(role_input.user_id.as_str(), "user-admin");
    assert_eq!(role_input.role, AdminUserRole::Member);
    assert_eq!(
        invoke_calls[3].0,
        CapabilityId::new(ADMIN_USER_DELETE_CAPABILITY_ID).expect("capability id")
    );
    let delete_input: RebornAdminUserRequest =
        serde_json::from_value(invoke_calls[3].1.clone()).expect("delete input");
    assert_eq!(delete_input.user_id.as_str(), "user-admin");

    let queries = services.view_queries.lock().expect("lock").clone();
    assert_eq!(queries.len(), 3);
    assert!(
        queries
            .iter()
            .all(|query| query.view_id == ADMIN_USER_VIEW.id)
    );
}

#[tokio::test]
async fn admin_user_secret_mutations_invoke_product_capabilities() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    services.enqueue_invoke_response(Ok(successful_resolution(ActivityId::new())));
    let put = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri("/api/webchat/v2/admin/users/user-admin/secrets/openai_api_key")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"value":"sk-test"}"#))
                .expect("request"),
        )
        .await
        .expect("put secret");
    assert_eq!(put.status(), StatusCode::OK);
    let body = read_json(put).await;
    assert_eq!(body["secret"]["handle"], "openai_api_key");

    services.enqueue_invoke_response(Ok(successful_resolution(ActivityId::new())));
    let delete = router
        .oneshot(
            Request::builder()
                .method(Method::DELETE)
                .uri("/api/webchat/v2/admin/users/user-admin/secrets/openai_api_key")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("delete secret");
    assert_eq!(delete.status(), StatusCode::OK);
    let body = read_json(delete).await;
    assert_eq!(body["handle"], "openai_api_key");
    assert_eq!(body["deleted"], true);

    let invoke_calls = services.invoke_calls.lock().expect("lock").clone();
    assert_eq!(invoke_calls.len(), 1);
    assert_eq!(
        invoke_calls[0].0,
        CapabilityId::new(ADMIN_USER_PUT_SECRET_CAPABILITY_ID).expect("capability id")
    );
    assert_eq!(
        invoke_calls[0].1,
        serde_json::json!({
            "user_id": "user-admin",
            "handle": "openai_api_key",
            "value": "sk-test"
        })
    );

    let surface_calls = services.surface_calls.lock().expect("lock").clone();
    assert_eq!(surface_calls.len(), 1);
    assert_eq!(
        surface_calls[0].call_id,
        ProductSurfaceCallId::AdminUserDeleteSecret.as_str()
    );
    assert_eq!(
        surface_calls[0].input,
        serde_json::json!({
            "user_id": "user-admin",
            "handle": "openai_api_key"
        })
    );

    let queries = services.view_queries.lock().expect("lock").clone();
    assert_eq!(queries.len(), 1);
    assert_eq!(queries[0].view_id.as_str(), ADMIN_USER_SECRETS_VIEW.id);
}

#[tokio::test]
async fn operator_routes_dispatch_to_facade_with_body_and_query_inputs() {
    let services = Arc::new(StubServices::default());
    let router = router_with_capabilities(
        services.clone(),
        WebUiV2Capabilities {
            operator_webui_config: true,
        },
    );

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/operator/setup")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);

    services.enqueue_invoke_response(Ok(successful_resolution(ActivityId::new())));
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/operator/setup")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"provider_id":"openai","model":"gpt-5-mini","webui_access_token":"webui-secret"}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/operator/config")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/operator/config/validate")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"keys":["provider.default","profile.default"]}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/operator/diagnostics")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/operator/status")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/operator/logs?limit=25&cursor=after-1&thread_id=thread-a&run_id=run-a&turn_id=turn-a&tool_call_id=tool-a&tool_name=shell&source=slack&follow=true")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/operator/service")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"action":"start"}"#))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);

    let invoke_calls = services.invoke_calls.lock().expect("lock").clone();
    assert_eq!(invoke_calls.len(), 1);
    assert_eq!(
        invoke_calls[0].0,
        CapabilityId::new(OPERATOR_SETUP_RUN_CAPABILITY_ID).expect("capability id")
    );
    assert_eq!(
        invoke_calls[0].1,
        serde_json::json!({
            "provider_id": "openai",
            "model": "gpt-5-mini",
            "webui_access_token": "webui-secret"
        })
    );
    assert_eq!(
        *services.list_operator_config_calls.lock().expect("lock"),
        1
    );
    assert_eq!(
        services
            .validate_operator_config_calls
            .lock()
            .expect("lock")
            .as_slice(),
        [vec![
            "provider.default".to_string(),
            "profile.default".to_string()
        ]]
    );
    let view_ids: Vec<_> = services
        .view_queries
        .lock()
        .expect("lock")
        .iter()
        .map(|query| query.view_id.clone())
        .collect();
    assert!(view_ids.contains(&OPERATOR_SETUP_VIEW.id.to_string()));
    assert!(view_ids.contains(&OPERATOR_CONFIG_LIST_VIEW.id.to_string()));
    assert!(view_ids.contains(&OPERATOR_CONFIG_VALIDATE_VIEW.id.to_string()));
    assert!(view_ids.contains(&OPERATOR_DIAGNOSTICS_VIEW.id.to_string()));
    assert!(view_ids.contains(&OPERATOR_STATUS_VIEW.id.to_string()));
    assert!(view_ids.contains(&OPERATOR_LOGS_VIEW.id.to_string()));
    let operator_log_calls = services.query_operator_logs_calls.lock().expect("lock");
    assert_eq!(operator_log_calls.len(), 1);
    assert_eq!(operator_log_calls[0].limit, Some(25));
    assert_eq!(operator_log_calls[0].cursor.as_deref(), Some("after-1"));
    assert_eq!(operator_log_calls[0].thread_id.as_deref(), Some("thread-a"));
    assert_eq!(operator_log_calls[0].run_id.as_deref(), Some("run-a"));
    assert_eq!(operator_log_calls[0].turn_id.as_deref(), Some("turn-a"));
    assert_eq!(
        operator_log_calls[0].tool_call_id.as_deref(),
        Some("tool-a")
    );
    assert_eq!(operator_log_calls[0].tool_name.as_deref(), Some("shell"));
    assert_eq!(operator_log_calls[0].source.as_deref(), Some("slack"));
    assert!(operator_log_calls[0].follow);
    assert!(!operator_log_calls[0].tail);
    drop(operator_log_calls);
    assert_eq!(
        services
            .run_operator_service_lifecycle_calls
            .lock()
            .expect("lock")
            .as_slice(),
        [RebornOperatorServiceLifecycleAction::Start]
    );
}

#[tokio::test]
async fn operator_config_routes_require_operator_capability() {
    let services = Arc::new(StubServices::default());
    let router = router_with_capabilities(services.clone(), WebUiV2Capabilities::default());

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/operator/setup")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/operator/setup")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"provider_id":"openai"}"#))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    // The extension admin-configuration routes are gated by the same
    // load-bearing `require_operator_webui_config` operator check: a
    // non-operator caller is rejected before the deployment-owned admin values
    // are read or any replacement capability is dispatched.
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/operator/extension-configuration")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri("/api/webchat/v2/operator/extension-configuration/extension.slack")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"values":[],"expected_revision":0,"idempotency_key":"non-operator"}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    assert!(services.invoke_calls.lock().expect("lock").is_empty());
}

#[tokio::test]
async fn settings_tool_routes_do_not_require_operator_capability() {
    let services = Arc::new(StubServices::default());
    let router = router_with_capabilities(services.clone(), WebUiV2Capabilities::default());

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/settings/tools")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);

    services.enqueue_invoke_response(Ok(successful_resolution(ActivityId::new())));
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/settings/tools")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"enabled":true}"#))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/settings/tools/ext.search")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"state":"always_allow"}"#))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);

    assert_eq!(
        *services.list_operator_config_calls.lock().expect("lock"),
        1
    );
    assert_eq!(
        services
            .set_operator_config_key_calls
            .lock()
            .expect("lock")
            .as_slice(),
        [(
            "tool.ext.search".to_string(),
            serde_json::json!({ "state": "always_allow" })
        )]
    );
    let invoke_calls = services.invoke_calls.lock().expect("lock").clone();
    assert_eq!(invoke_calls.len(), 1);
    assert_eq!(
        invoke_calls[0].0,
        CapabilityId::new(OPERATOR_CONFIG_SET_AUTO_APPROVE_CAPABILITY_ID).expect("capability id")
    );
    assert_eq!(invoke_calls[0].1, serde_json::json!({ "enabled": true }));
    assert_eq!(
        services
            .get_operator_config_key_calls
            .lock()
            .expect("lock")
            .as_slice(),
        ["agent.auto_approve_tools"]
    );
}

#[tokio::test]
async fn settings_tool_routes_fail_closed_without_capabilities_extension() {
    let services = Arc::new(StubServices::default());
    let router = router_with_caller_only(services.clone());

    for (method, uri, body) in [
        (Method::GET, "/api/webchat/v2/settings/tools", ""),
        (
            Method::POST,
            "/api/webchat/v2/settings/tools",
            r#"{"enabled":true}"#,
        ),
        (
            Method::POST,
            "/api/webchat/v2/settings/tools/ext.search",
            r#"{"state":"always_allow"}"#,
        ),
    ] {
        let mut request = Request::builder().method(method).uri(uri);
        if !body.is_empty() {
            request = request.header("content-type", "application/json");
        }
        let response = router
            .clone()
            .oneshot(request.body(Body::from(body)).expect("request"))
            .await
            .expect("oneshot");
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    assert_eq!(
        *services.list_operator_config_calls.lock().expect("lock"),
        0
    );
    assert!(
        services
            .set_operator_config_key_calls
            .lock()
            .expect("lock")
            .is_empty()
    );
    assert!(services.invoke_calls.lock().expect("lock").is_empty());
}

#[tokio::test]
async fn settings_tool_routes_expose_only_tool_approval_config() {
    let services = Arc::new(StubServices::default());
    services
        .operator_config_entries
        .lock()
        .expect("lock")
        .extend([
            operator_config_entry(
                "agent.auto_approve_tools".to_string(),
                serde_json::json!(true),
            ),
            operator_config_entry(
                "tool.ext.search".to_string(),
                serde_json::json!({ "state": "always_allow" }),
            ),
            operator_config_entry("provider.default".to_string(), serde_json::json!("openai")),
            operator_config_entry("secret.api_key".to_string(), serde_json::json!("redacted")),
        ]);
    let router = router_with_capabilities(services.clone(), WebUiV2Capabilities::default());

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/settings/tools")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);
    let body = read_json(response).await;
    let keys = body["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .map(|entry| entry["key"].as_str().expect("key"))
        .collect::<Vec<_>>();
    assert_eq!(keys, ["agent.auto_approve_tools", "tool.ext.search"]);

    services.enqueue_invoke_response(Ok(successful_resolution(ActivityId::new())));
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/settings/tools")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"enabled":false}"#))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/settings/tools/ext.search")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"state":"disabled"}"#))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);

    assert_eq!(
        services
            .set_operator_config_key_calls
            .lock()
            .expect("lock")
            .as_slice(),
        [(
            "tool.ext.search".to_string(),
            serde_json::json!({ "state": "disabled" })
        )],
    );
    let invoke_calls = services.invoke_calls.lock().expect("lock").clone();
    assert_eq!(invoke_calls.len(), 1);
    assert_eq!(
        invoke_calls[0].0,
        CapabilityId::new(OPERATOR_CONFIG_SET_AUTO_APPROVE_CAPABILITY_ID).expect("capability id")
    );
    assert_eq!(invoke_calls[0].1, serde_json::json!({ "enabled": false }));
}

#[tokio::test]
async fn settings_tool_writes_fail_closed_when_config_service_unwired() {
    let services = Arc::new(StubServices::default());
    services.fail_set_operator_config_key(service_unavailable_error(false));
    let router = router_with_capabilities(services, WebUiV2Capabilities::default());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/settings/tools/ext.search")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"state":"always_allow"}"#))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = read_json(response).await;
    assert_eq!(body["kind"], "service_unavailable");
    assert_eq!(body["retryable"], false);
}

#[tokio::test]
async fn settings_tool_permission_rejects_invalid_state_before_dispatch() {
    let services = Arc::new(StubServices::default());
    let router = router_with_capabilities(services.clone(), WebUiV2Capabilities::default());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/settings/tools/ext.search")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"state":"sometimes"}"#))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        services
            .set_operator_config_key_calls
            .lock()
            .expect("lock")
            .is_empty()
    );
}

#[tokio::test]
async fn settings_tool_permission_rejects_overlong_capability_id_before_dispatch() {
    let services = Arc::new(StubServices::default());
    let router = router_with_capabilities(services.clone(), WebUiV2Capabilities::default());
    let capability_id = "x".repeat(125);

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/api/webchat/v2/settings/tools/{capability_id}"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"state":"always_allow"}"#))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = read_json(response).await;
    assert_eq!(body["error"], "invalid_request");
    assert_eq!(body["field"], "capability_id");
    assert_eq!(body["validation_code"], "too_long");
    assert!(
        services
            .set_operator_config_key_calls
            .lock()
            .expect("lock")
            .is_empty()
    );
}

#[tokio::test]
async fn operator_logs_require_operator_capability() {
    let services = Arc::new(StubServices::default());
    let router = router_with_capabilities(services.clone(), WebUiV2Capabilities::default());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/operator/logs?limit=25&cursor=after-1&thread_id=thread-a&run_id=run-a&turn_id=turn-a&tool_call_id=tool-a&tool_name=shell&source=slack&follow=true")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(
        services
            .query_operator_logs_calls
            .lock()
            .expect("lock")
            .is_empty()
    );
}

#[tokio::test]
async fn logs_are_available_without_operator_capability() {
    let services = Arc::new(StubServices::default());
    let router = router_with_capabilities(services.clone(), WebUiV2Capabilities::default());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/logs?limit=25&cursor=after-1&thread_id=thread-a&run_id=run-a&turn_id=turn-a&tool_call_id=tool-a&tool_name=shell&source=slack&follow=true")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);

    let log_calls = services.query_logs_calls.lock().expect("lock");
    assert_eq!(log_calls.len(), 1);
    assert_eq!(log_calls[0].limit, Some(25));
    assert_eq!(log_calls[0].cursor.as_deref(), Some("after-1"));
    assert_eq!(log_calls[0].thread_id.as_deref(), Some("thread-a"));
    assert_eq!(log_calls[0].run_id.as_deref(), Some("run-a"));
    assert_eq!(log_calls[0].turn_id.as_deref(), Some("turn-a"));
    assert_eq!(log_calls[0].tool_call_id.as_deref(), Some("tool-a"));
    assert_eq!(log_calls[0].tool_name.as_deref(), Some("shell"));
    assert_eq!(log_calls[0].source.as_deref(), Some("slack"));
    assert!(log_calls[0].follow);
    assert!(!log_calls[0].tail);
}

#[tokio::test]
async fn logs_reject_ambiguous_tail_follow_modes() {
    let services = Arc::new(StubServices::default());
    let router = router_with_capabilities(services.clone(), WebUiV2Capabilities::default());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/logs?tail=true&follow=true")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let body = read_json(response).await;
    assert_eq!(body["error"], "invalid_request");
    assert_eq!(body["field"], "follow");
    assert_eq!(body["validation_code"], "invalid_value");
    assert!(services.query_logs_calls.lock().expect("lock").is_empty());
}

/// The operator configuration PUT is an ingress adapter over the canonical
/// product mutation conduit. It must not call an admin store or vendor handler
/// directly, and the untrusted wire idempotency key must be consumed at ingress
/// rather than forwarded as capability input authority.
#[tokio::test]
async fn admin_configuration_put_dispatches_through_generic_invoke() {
    let services = Arc::new(StubServices::default());
    let router = router_with_capabilities(
        services.clone(),
        WebUiV2Capabilities {
            operator_webui_config: true,
        },
    );

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri("/api/webchat/v2/operator/extension-configuration/extension.slack")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "values": [{"handle": "slack_team_id", "value": "T-ONE"}],
                        "expected_revision": 7,
                        "idempotency_key": "opaque-client-retry-key",
                    })
                    .to_string(),
                ))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    let status = response.status();
    let calls = services.invoke_calls.lock().expect("lock");

    assert_eq!(
        (status, calls.len()),
        (StatusCode::SERVICE_UNAVAILABLE, 1),
        "the route must reach generic invoke and preserve its failure status"
    );
    let (capability, input, _activity_id) = &calls[0];
    assert!(
        !capability.as_str().contains("slack"),
        "the mutation capability must be extension-generic: {capability}"
    );
    assert_eq!(input["group_id"], "extension.slack");
    assert_eq!(input["expected_revision"], 7);
    assert_eq!(input["values"][0]["handle"], "slack_team_id");
    assert!(
        input.get("idempotency_key").is_none(),
        "the authorized invocation scope, not untrusted input, owns idempotency: {input}"
    );
}

#[tokio::test]
async fn operator_config_key_routes_dispatch_path_and_body() {
    let services = Arc::new(StubServices::default());
    let router = router_with_capabilities(
        services.clone(),
        WebUiV2Capabilities {
            operator_webui_config: true,
        },
    );

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/operator/config/provider.default")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/operator/config/provider.default")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"value":{"provider":"openai"}}"#))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);

    assert_eq!(
        services
            .get_operator_config_key_calls
            .lock()
            .expect("lock")
            .as_slice(),
        ["provider.default".to_string()]
    );
    assert_eq!(
        services
            .set_operator_config_key_calls
            .lock()
            .expect("lock")
            .as_slice(),
        [(
            "provider.default".to_string(),
            serde_json::json!({ "provider": "openai" })
        )]
    );
}

#[tokio::test]
async fn operator_status_surfaces_unsupported_config_diagnostics() {
    let services = Arc::new(StubServices::default());
    let router = router_with_capabilities(
        services,
        WebUiV2Capabilities {
            operator_webui_config: true,
        },
    );

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/operator/status")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let body = read_json(response).await;
    assert_eq!(body["area"], "status");
    assert_eq!(body["status"], "unavailable");
    assert_eq!(
        body["diagnostics"][0]["reason_code"],
        "operator_config_service_not_wired"
    );
    assert_eq!(body["diagnostics"][0]["owning_area"], "config");
    assert_eq!(body["diagnostics"][0]["severity"], "error");
    assert!(body["diagnostics"][0]["remediation"].is_string());
}

#[tokio::test]
async fn operator_diagnostics_surface_reports_same_unsupported_config_reason() {
    let services = Arc::new(StubServices::default());
    let router = router_with_capabilities(
        services,
        WebUiV2Capabilities {
            operator_webui_config: true,
        },
    );

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/operator/diagnostics")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let body = read_json(response).await;
    assert_eq!(body["area"], "diagnostics");
    assert_eq!(body["status"], "unavailable");
    assert_eq!(
        body["diagnostics"][0]["reason_code"],
        "operator_config_service_not_wired"
    );
}

#[tokio::test]
async fn operator_config_validation_surfaces_redacted_reason_codes() {
    let services = Arc::new(StubServices::default());
    let router = router_with_capabilities(
        services,
        WebUiV2Capabilities {
            operator_webui_config: true,
        },
    );

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/operator/config/validate")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"keys":["provider.api_key","legacy.provider","bootstrap.database_url","provider.default","made.up"]}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let body = read_json(response).await;
    assert_eq!(body["valid"], false);
    let diagnostics = body["diagnostics"].as_array().expect("diagnostics");
    let reason_codes: Vec<_> = diagnostics
        .iter()
        .map(|diagnostic| diagnostic["reason_code"].as_str().expect("reason code"))
        .collect();
    assert_eq!(
        reason_codes,
        [
            "operator_config_secret_not_wired",
            "operator_config_deprecated",
            "operator_config_immutable",
            "operator_config_not_wired",
            "operator_config_unknown_key",
        ]
    );

    let rendered = serde_json::to_string(&body).expect("render body");
    assert!(!rendered.contains("sk-"));
    assert!(!rendered.contains("secret-value"));
}

#[tokio::test]
async fn operator_config_set_failure_does_not_echo_secret_value() {
    let services = Arc::new(StubServices::default());
    services.fail_set_operator_config_key(service_unavailable_error(false));
    let router = router_with_capabilities(
        services,
        WebUiV2Capabilities {
            operator_webui_config: true,
        },
    );

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/operator/config/provider.api_key")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"value":"sk-secret-value"}"#))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = read_json(response).await;
    let rendered = serde_json::to_string(&body).expect("render body");
    assert_eq!(body["kind"], "service_unavailable");
    assert!(!rendered.contains("sk-secret-value"));
}

#[tokio::test]
async fn extension_list_and_registry_use_product_views() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    for uri in [
        "/api/webchat/v2/extensions",
        "/api/webchat/v2/extensions/registry",
    ] {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(uri)
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("oneshot");

        assert_eq!(response.status(), StatusCode::OK);
    }

    let view_ids: Vec<String> = services
        .view_queries
        .lock()
        .expect("lock")
        .iter()
        .map(|query| query.view_id.clone())
        .collect();
    assert_eq!(
        view_ids,
        vec![
            EXTENSIONS_VIEW.id.to_string(),
            EXTENSION_REGISTRY_VIEW.id.to_string()
        ]
    );
}

#[tokio::test]
async fn skill_list_and_search_use_product_views() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    let list_response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/skills")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(list_response.status(), StatusCode::OK);

    let search_response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/skills/search")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"query":"registry"}"#))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(search_response.status(), StatusCode::OK);

    let queries = services.view_queries.lock().expect("lock").clone();
    assert_eq!(queries.len(), 2);
    assert_eq!(queries[0].view_id, SKILLS_VIEW.id);
    assert_eq!(queries[0].params, serde_json::json!({}));
    assert_eq!(queries[1].view_id, SKILL_SEARCH_VIEW.id);
    assert_eq!(
        queries[1].params,
        serde_json::json!({ "query": "registry" })
    );
}

#[tokio::test]
async fn skill_content_and_mutations_use_product_surface() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    let content_response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/skills/demo-skill")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(content_response.status(), StatusCode::OK);

    services.enqueue_invoke_response(Ok(successful_resolution(ActivityId::new())));
    let install_response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/skills/install")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"name":"demo-skill","content":"---\nname: demo-skill\n---\n"}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(install_response.status(), StatusCode::OK);

    services.enqueue_invoke_response(Ok(successful_resolution(ActivityId::new())));
    let install_retry_response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/skills/install")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"name":"demo-skill","content":"---\nname: demo-skill\n---\n"}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(install_retry_response.status(), StatusCode::OK);

    services.enqueue_invoke_response(Ok(successful_resolution(ActivityId::new())));
    let update_response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri("/api/webchat/v2/skills/demo-skill")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"content":"---\nname: demo-skill\n---\n# Demo\n"}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(update_response.status(), StatusCode::OK);

    services.enqueue_invoke_response(Ok(successful_resolution(ActivityId::new())));
    let toggle_response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/skills/demo-skill/auto-activate")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"enabled":false}"#))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(toggle_response.status(), StatusCode::OK);

    services.enqueue_invoke_response(Ok(successful_resolution(ActivityId::new())));
    let remove_response = router
        .oneshot(
            Request::builder()
                .method(Method::DELETE)
                .uri("/api/webchat/v2/skills/demo-skill")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(remove_response.status(), StatusCode::OK);

    let queries = services.view_queries.lock().expect("lock").clone();
    assert_eq!(queries.len(), 1);
    assert_eq!(queries[0].view_id, SKILL_CONTENT_VIEW.id);
    assert_eq!(
        queries[0].params,
        serde_json::json!({ "name": "demo-skill" })
    );

    let invoke_calls = services.invoke_calls.lock().expect("lock");
    let calls = invoke_calls
        .iter()
        .map(|(capability, input, _activity_id)| (capability.as_str(), input.clone()))
        .collect::<Vec<_>>();
    assert_eq!(
        calls,
        vec![
            (
                SKILL_INSTALL_CAPABILITY_ID,
                serde_json::json!({
                    "name": "demo-skill",
                    "content": "---\nname: demo-skill\n---\n",
                }),
            ),
            (
                SKILL_INSTALL_CAPABILITY_ID,
                serde_json::json!({
                    "name": "demo-skill",
                    "content": "---\nname: demo-skill\n---\n",
                }),
            ),
            (
                SKILL_UPDATE_CAPABILITY_ID,
                serde_json::json!({
                    "name": "demo-skill",
                    "content": "---\nname: demo-skill\n---\n# Demo\n",
                }),
            ),
            (
                SKILL_AUTO_ACTIVATE_SET_CAPABILITY_ID,
                serde_json::json!({
                    "name": "demo-skill",
                    "enabled": false,
                }),
            ),
            (
                SKILL_REMOVE_CAPABILITY_ID,
                serde_json::json!({ "name": "demo-skill" }),
            ),
        ]
    );
    assert_eq!(
        invoke_calls[0].2, invoke_calls[1].2,
        "identical ProductSurface requests should reuse the same durable replay key"
    );
    assert_ne!(
        invoke_calls[0].2, invoke_calls[2].2,
        "changed generic ProductSurface requests should also receive fresh activity ids"
    );
}

#[tokio::test]
async fn install_extension_invokes_lifecycle_capability_with_body_package_ref() {
    let services = Arc::new(StubServices::default());
    services.enqueue_invoke_response(Ok(successful_resolution(ActivityId::new())));
    let router = router_with(services.clone());

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/extensions/install")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"package_ref":{"kind":"extension","id":"google-calendar"},"client_action_id":"install-google-calendar"}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let body = read_json(response).await;
    assert_eq!(body["success"], true);
    services.enqueue_invoke_response(Ok(successful_resolution(ActivityId::new())));
    // The membership read-back must see the newly installed package, so prime
    // the stub view for the second install target.
    services.set_extensions_view(RebornExtensionListResponse {
        extensions: vec![extension_info("nearai-mcp", false)],
    });
    let retry_response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/extensions/install")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"package_ref":{"kind":"extension","id":"nearai-mcp"},"client_action_id":"install-nearai-mcp"}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(retry_response.status(), StatusCode::OK);
    let invoke_calls = services.invoke_calls.lock().expect("lock").clone();
    assert_eq!(invoke_calls.len(), 2);
    assert_eq!(
        invoke_calls[0].0,
        CapabilityId::new(EXTENSION_INSTALL_CAPABILITY_ID).expect("capability id")
    );
    assert_eq!(
        invoke_calls[0].1,
        serde_json::json!({ "extension_id": "google-calendar" })
    );
    let queries = services.view_queries.lock().expect("lock").clone();
    assert_eq!(queries.len(), 2);
    assert_eq!(queries[0].view_id, EXTENSIONS_VIEW.id);
    assert_eq!(queries[1].view_id, EXTENSIONS_VIEW.id);
}

#[tokio::test]
async fn install_extension_accepts_auth_gate_only_after_setup_needed_read_back() {
    let services = Arc::new(StubServices::default());
    services.enqueue_invoke_response(Ok(blocked_auth_resolution(ActivityId::new())));
    services.set_extensions_view(RebornExtensionListResponse {
        extensions: vec![extension_info("google-calendar", false)],
    });
    let router = router_with(services.clone());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/extensions/install")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"package_ref":{"kind":"extension","id":"google-calendar"},"client_action_id":"install-auth-gate"}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let body = read_json(response).await;
    assert_eq!(body["success"], true);

    let queries = services.view_queries.lock().expect("lock").clone();
    assert_eq!(queries.len(), 1);
    assert_eq!(queries[0].view_id, EXTENSIONS_VIEW.id);
}

#[tokio::test]
async fn install_extension_rejects_capability_success_without_exact_membership_readback() {
    let services = Arc::new(StubServices::default());
    services.enqueue_invoke_response(Ok(successful_resolution(ActivityId::new())));
    let router = router_with(services.clone());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/extensions/install")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"package_ref":{"kind":"extension","id":"github"},"client_action_id":"install-github"}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = read_json(response).await;
    assert_eq!(body["kind"], "service_unavailable");
    assert_eq!(body["retryable"], true);
    let queries = services.view_queries.lock().expect("lock").clone();
    assert_eq!(queries.len(), 1);
    assert_eq!(queries[0].view_id, EXTENSIONS_VIEW.id);
}

#[tokio::test]
async fn install_extension_uses_client_gesture_idempotency_not_permanent_input_deduplication() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    // Two distinct client gestures, then a response-lost retry of the second.
    for client_action_id in [
        "install-gesture-one",
        "install-gesture-two",
        "install-gesture-two",
    ] {
        services.enqueue_invoke_response(Ok(successful_resolution(ActivityId::new())));
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/webchat/v2/extensions/install")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        r#"{{"package_ref":{{"kind":"extension","id":"google-calendar"}},"client_action_id":"{client_action_id}"}}"#,
                    )))
                    .expect("request"),
            )
            .await
            .expect("oneshot");
        assert_eq!(response.status(), StatusCode::OK);
    }

    let calls = services.invoke_calls.lock().expect("lock").clone();
    assert_eq!(calls.len(), 3);
    assert_ne!(
        calls[0].2, calls[1].2,
        "separate install gestures must never replay one permanent cached lifecycle outcome"
    );
    assert_eq!(
        calls[1].2, calls[2].2,
        "the client action id must survive response-lost retries as the ProductSurface activity id"
    );
}

#[tokio::test]
async fn install_extension_rejects_non_extension_package_kind_with_400() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/extensions/install")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"package_ref":{"kind":"skill","id":"nearai-mcp"}}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = read_json(response).await;
    assert_eq!(body["error"], "invalid_request");
    assert_eq!(body["field"], "package_ref");
    assert_eq!(body["validation_code"], "invalid_id");
    assert!(
        services.invoke_calls.lock().expect("lock").is_empty(),
        "invalid package kind must not reach the capability path"
    );
}

/// #5499 review finding #4: the admin-only import route must reject a caller
/// whose bearer token lacks `operator_webui_config` BEFORE the facade is
/// reached. Lower-level lifecycle tests cannot catch this route dropping its
/// `require_operator_webui_config` gate.
#[tokio::test]
async fn import_extension_requires_operator_webui_config() {
    let services = Arc::new(StubServices::default());
    let router = router_with_capabilities(services.clone(), WebUiV2Capabilities::default());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/extensions/import")
                .header("content-type", "application/zip")
                .body(Body::from(b"PK\x03\x04not-really-a-zip".to_vec()))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body = read_json(response).await;
    assert_eq!(body["error"], "forbidden");
    assert!(
        services.invoke_calls.lock().expect("lock").is_empty(),
        "a non-operator caller must never reach the import facade"
    );
}

/// #5499 review finding #4: an operator upload must forward the raw zip bytes
/// through the single ProductSurface capability path. The HTTP boundary encodes
/// bytes as base64; product-workflow decodes before reaching lifecycle import.
#[tokio::test]
async fn import_extension_invokes_product_surface_with_zip_bytes() {
    let services = Arc::new(StubServices::default());
    services.enqueue_invoke_response(Ok(successful_resolution(ActivityId::new())));
    let router = router_with_capabilities(
        services.clone(),
        WebUiV2Capabilities {
            operator_webui_config: true,
        },
    );
    // Deliberately non-UTF-8 so byte fidelity (not just string round-trip) is
    // what the assertion proves.
    let bundle: Vec<u8> = b"PK\x03\x04\x00\xff\xfe binary zip bytes".to_vec();

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/extensions/import")
                .header("content-type", "application/zip")
                .body(Body::from(bundle.clone()))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let invoke_calls = services.invoke_calls.lock().expect("lock").clone();
    assert_eq!(invoke_calls.len(), 1);
    assert_eq!(
        invoke_calls[0].0,
        CapabilityId::new(EXTENSION_IMPORT_CAPABILITY_ID).expect("capability id")
    );
    assert_eq!(
        invoke_calls[0].1,
        serde_json::json!({ "bundle_base64": STANDARD.encode(&bundle) })
    );
}

/// #5499 review finding #1: the import route is operator-only, so a router
/// built `without_operator_routes()` (the shape composition serves to
/// deployments with no operator surface) must not mount it at all — exactly
/// like its operator siblings. Before the fix the route was mounted
/// unconditionally and answered 403 here instead of 404.
#[tokio::test]
async fn import_extension_is_stripped_alongside_operator_routes() {
    let services = Arc::new(StubServices::default());
    let router = webui_v2_router_with_options(
        WebUiV2State::new(services, DEFAULT_SSE_MAX_CONCURRENT_PER_CALLER),
        WebUiV2RouteOptions::without_operator_routes(),
    )
    .layer(axum::Extension(caller()))
    .layer(axum::Extension(WebUiV2Capabilities::default()));

    let import_response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/extensions/import")
                .header("content-type", "application/zip")
                .body(Body::from(b"PK\x03\x04".to_vec()))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    // Parity check: an undisputed operator sibling on the same stripped router.
    let operator_status_response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/operator/status")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(
        operator_status_response.status(),
        StatusCode::NOT_FOUND,
        "operator sibling is stripped from a without_operator_routes router"
    );
    assert_eq!(
        import_response.status(),
        StatusCode::NOT_FOUND,
        "the admin-only import route must be stripped exactly like its operator siblings"
    );
}

#[tokio::test]
async fn remove_extension_decodes_path_package_id_to_lifecycle_path() {
    let services = Arc::new(StubServices::default());
    services.enqueue_invoke_response(Ok(successful_resolution(ActivityId::new())));
    let router = router_with(services.clone());

    let activate_response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/extensions/google-calendar/activate")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"client_action_id":"activate-google-calendar"}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(activate_response.status(), StatusCode::NOT_FOUND);
    assert!(
        services.invoke_calls.lock().expect("lock").is_empty(),
        "the removed activate action must not reach the lifecycle capability path"
    );

    let remove_response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/extensions/google-calendar/remove")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"client_action_id":"remove-google-calendar"}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(remove_response.status(), StatusCode::OK);
    let invoke_calls = services.invoke_calls.lock().expect("lock").clone();
    assert_eq!(invoke_calls.len(), 1);
    assert_eq!(
        invoke_calls[0].0,
        CapabilityId::new(EXTENSION_REMOVE_CAPABILITY_ID).expect("capability id")
    );
    assert_eq!(
        invoke_calls[0].1,
        serde_json::json!({ "extension_id": "google-calendar" })
    );
}

#[tokio::test]
async fn get_extension_setup_queries_product_surface_view() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/extensions/telegram/setup")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let body = read_json(response).await;
    assert_eq!(body["package_ref"]["id"], "telegram");
    assert_eq!(body["package_ref"]["kind"], "extension");
    assert_eq!(body["phase"], "setup_needed");

    let queries = services.view_queries.lock().expect("lock");
    assert!(
        queries
            .iter()
            .any(|query| query.view_id == EXTENSION_SETUP_VIEW.id
                && query.params == serde_json::json!({ "package_id": "telegram" })),
        "GET setup must read through the ProductSurface extension_setup view: {queries:?}",
    );
}

// The path segment must become a lifecycle package ref at the
// handler/facade boundary. A well-formed package id reaches the facade
// and round-trips into the response.
#[tokio::test]
async fn setup_extension_invokes_product_surface_capability() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());
    services.enqueue_invoke_response(Ok(successful_resolution(ActivityId::new())));

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/extensions/telegram/setup")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"client_action_id":"setup-telegram","action":"begin"}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let body = read_json(response).await;
    assert_eq!(
        body["package_ref"]["id"], "telegram",
        "facade must echo the package id from the path",
    );
    assert_eq!(body["package_ref"]["kind"], "extension");
    assert_eq!(body["phase"], "setup_needed");
    assert!(
        body.get("status").is_none(),
        "setup_extension must not expose legacy status aliases: {body}"
    );

    services.enqueue_invoke_response(Ok(successful_resolution(ActivityId::new())));
    let retry_response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/extensions/telegram/setup")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"client_action_id":"setup-telegram","action":"begin"}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(retry_response.status(), StatusCode::OK);
    let retry_body = read_json(retry_response).await;
    assert_eq!(retry_body["package_ref"]["id"], "telegram");
    assert_eq!(retry_body["package_ref"]["kind"], "extension");
    // The stub setup view reports SetupNeeded on every read; the retry echoes
    // the same read-back phase (main's stub used InstallationState::Unsupported,
    // retired by the #6520 lifecycle model).
    assert_eq!(retry_body["phase"], "setup_needed");

    let invoke_calls = services.invoke_calls.lock().expect("lock").clone();
    assert_eq!(invoke_calls.len(), 2);
    let expected_capability =
        CapabilityId::new(EXTENSION_SETUP_SUBMIT_CAPABILITY_ID).expect("capability id");
    let expected_input = serde_json::json!({
            "extension_id": "telegram",
            "action": "begin"
    });
    assert_eq!(invoke_calls[0].0, expected_capability);
    assert_eq!(invoke_calls[1].0, expected_capability);
    assert_eq!(invoke_calls[0].1, expected_input);
    assert_eq!(invoke_calls[1].1, expected_input);
    assert_eq!(
        invoke_calls[0].2, invoke_calls[1].2,
        "the setup client action id must survive response-lost retries as the ProductSurface activity id"
    );
    let queries = services.view_queries.lock().expect("lock");
    assert!(
        queries
            .iter()
            .any(|query| query.view_id == EXTENSION_SETUP_VIEW.id
                && query.params == serde_json::json!({ "package_id": "telegram" })),
        "POST setup must read back through the ProductSurface extension_setup view: {queries:?}",
    );
}

// Companion to the typed-internals fix: a malformed identifier in
// the route path must be rejected at the handler/facade boundary
// before the facade is called, with the same `invalid_request` wire
// shape any other inbound validation failure produces. Without
// boundary validation, a path like `../etc` would silently flow
// into the facade as a raw `String` and the typed-internals rule in
// `.claude/rules/types.md` would be broken in practice.
#[tokio::test]
async fn setup_extension_rejects_malformed_package_id_with_400() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    // `%0A` decodes to a newline and triggers control-character validation in
    // LifecyclePackageRef::new.
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/extensions/bad%0Aid/setup")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = read_json(response).await;
    assert_eq!(body["error"], "invalid_request");
    assert_eq!(body["field"], "package_id");
    assert_eq!(body["validation_code"], "invalid_id");
    assert_eq!(body["retryable"], false);
}

#[tokio::test]
async fn get_extension_setup_rejects_malformed_package_id_with_400() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/extensions/bad%0Aid/setup")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = read_json(response).await;
    assert_eq!(body["error"], "invalid_request");
    assert_eq!(body["field"], "package_id");
    assert_eq!(body["validation_code"], "invalid_id");
}

#[tokio::test]
async fn llm_provider_routes_keep_key_bearing_mutations_on_typed_surface() {
    let services = Arc::new(StubServices::default());
    let router = router_with_capabilities(
        services.clone(),
        WebUiV2Capabilities {
            operator_webui_config: true,
        },
    );

    let get_response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/llm/providers")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(get_response.status(), StatusCode::OK);
    let get_body = read_json(get_response).await;
    assert_eq!(get_body["providers"][0]["accepts_api_key"], true);

    services.enqueue_invoke_response(Ok(successful_resolution(ActivityId::new())));
    let upsert_response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/llm/providers")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"id":"acme","name":"Acme","adapter":"open_ai_completions","base_url":"https://api.acme.test/v1","default_model":"acme-1","api_key":"sk-test","set_active":true,"model":"acme-1","client_action_id":"llm-upsert-acme-1"}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(upsert_response.status(), StatusCode::OK);

    services.enqueue_invoke_response(Ok(successful_resolution(ActivityId::new())));
    let delete_response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/llm/providers/acme/delete")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(delete_response.status(), StatusCode::OK);

    services.enqueue_invoke_response(Ok(successful_resolution(ActivityId::new())));
    let active_response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/llm/active")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"provider_id":"openai","model":"gpt-5"}"#))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(active_response.status(), StatusCode::OK);

    let probe_body = r#"{"provider_id":"openai","adapter":"open_ai_completions","base_url":"https://api.openai.com/v1","model":"gpt-5","api_key":"sk-test"}"#;
    let test_response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/llm/test-connection")
                .header("content-type", "application/json")
                .body(Body::from(probe_body))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(test_response.status(), StatusCode::OK);

    let models_response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/llm/list-models")
                .header("content-type", "application/json")
                .body(Body::from(probe_body))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(models_response.status(), StatusCode::OK);

    assert_eq!(*services.get_llm_config_calls.lock().expect("lock"), 4);
    let view_ids: Vec<String> = services
        .view_queries
        .lock()
        .expect("lock")
        .iter()
        .map(|query| query.view_id.clone())
        .collect();
    assert!(view_ids.contains(&LLM_CONFIG_VIEW.id.to_string()));
    assert_eq!(
        services
            .upsert_llm_provider_calls
            .lock()
            .expect("lock")
            .as_slice(),
        [("acme".to_string(), true)]
    );
    assert_eq!(
        services
            .delete_llm_provider_calls
            .lock()
            .expect("lock")
            .as_slice(),
        ["acme"]
    );
    assert_eq!(
        services
            .set_active_llm_calls
            .lock()
            .expect("lock")
            .as_slice(),
        [("openai".to_string(), Some("gpt-5".to_string()))]
    );
    let invoke_calls = services.invoke_calls.lock().expect("lock").clone();
    assert_eq!(invoke_calls.len(), 2);
    assert_eq!(
        invoke_calls[0].0.as_str(),
        LLM_PROVIDER_DELETE_CAPABILITY_ID
    );
    assert_eq!(
        invoke_calls[0].1,
        serde_json::json!({ "provider_id": "acme" })
    );
    assert_eq!(invoke_calls[1].0.as_str(), LLM_ACTIVE_SET_CAPABILITY_ID);
    assert_eq!(
        invoke_calls[1].1,
        serde_json::json!({ "provider_id": "openai", "model": "gpt-5" })
    );
    assert_eq!(
        services
            .test_llm_connection_calls
            .lock()
            .expect("lock")
            .as_slice(),
        ["openai"]
    );
    assert_eq!(
        services
            .list_llm_models_calls
            .lock()
            .expect("lock")
            .as_slice(),
        ["openai"]
    );
}

#[tokio::test]
async fn llm_provider_routes_require_operator_capability() {
    let services = Arc::new(StubServices::default());
    let router = router_with_capabilities(services.clone(), WebUiV2Capabilities::default());

    let upsert_body = r#"{"id":"acme","name":"Acme","adapter":"open_ai_completions","base_url":"https://api.acme.test/v1","default_model":"acme-1","api_key":"sk-test","set_active":true,"model":"acme-1"}"#;
    let active_body = r#"{"provider_id":"openai","model":"gpt-5"}"#;
    let probe_body = r#"{"provider_id":"openai","adapter":"open_ai_completions","base_url":"https://api.openai.com/v1","model":"gpt-5","api_key":"sk-test"}"#;
    let nearai_login_body = r#"{"provider":"github","origin":"https://app.example"}"#;
    let nearai_wallet_body = r#"{"account_id":"alice.near","public_key":"ed25519:test","signature":"AA==","message":"login","recipient":"near.ai","nonce":[]}"#;
    let cases = [
        ("GET", "/api/webchat/v2/llm/providers", None),
        ("POST", "/api/webchat/v2/llm/providers", Some(upsert_body)),
        ("POST", "/api/webchat/v2/llm/providers/acme/delete", None),
        ("POST", "/api/webchat/v2/llm/active", Some(active_body)),
        (
            "POST",
            "/api/webchat/v2/llm/test-connection",
            Some(probe_body),
        ),
        ("POST", "/api/webchat/v2/llm/list-models", Some(probe_body)),
        (
            "POST",
            "/api/webchat/v2/llm/nearai/login",
            Some(nearai_login_body),
        ),
        (
            "POST",
            "/api/webchat/v2/llm/nearai/wallet",
            Some(nearai_wallet_body),
        ),
        ("POST", "/api/webchat/v2/llm/codex/login", None),
    ];

    for (method, uri, body) in cases {
        let mut builder = Request::builder().method(method).uri(uri);
        if body.is_some() {
            builder = builder.header("content-type", "application/json");
        }
        let request = builder
            .body(body.map_or_else(Body::empty, Body::from))
            .expect("request");
        let response = router.clone().oneshot(request).await.expect("oneshot");
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{method} {uri}");
    }

    assert_eq!(*services.get_llm_config_calls.lock().expect("lock"), 0);
    assert!(
        services.invoke_calls.lock().expect("lock").is_empty(),
        "operator-gated LLM config routes must not reach ProductSurface invoke"
    );
    assert!(
        services
            .test_llm_connection_calls
            .lock()
            .expect("lock")
            .is_empty()
    );
    assert!(
        services
            .list_llm_models_calls
            .lock()
            .expect("lock")
            .is_empty()
    );
}

fn url_encode(value: &str) -> String {
    // Minimal application/x-www-form-urlencoded helper: percent-encode every
    // byte that is not an unreserved character per RFC 3986. Avoids pulling
    // in a urlencoding dep just for one test value.
    let mut out = String::with_capacity(value.len() * 3);
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

// A browser tab reuses one connection_id while navigating between threads.
// The replacement must cancel the prior response even when a proxy has not
// propagated the browser's close yet; otherwise stale streams consume the
// per-caller cap and the new thread remains disconnected until refresh.
#[tokio::test]
async fn stream_events_same_connection_id_supersedes_stale_stream() {
    let services: Arc<dyn ProductSurface> = Arc::new(StubServices::default());
    let router = webui_v2_router(WebUiV2State::new(services, 1)).layer(axum::Extension(caller()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    let serve_handle = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut first = tokio::net::TcpStream::connect(addr)
        .await
        .expect("first tcp");
    first
        .write_all(
            b"GET /api/webchat/v2/threads/thread-a/events?connection_id=browser-tab&connection_generation=1 HTTP/1.1\r\n\
              Host: localhost\r\n\
              Accept: text/event-stream\r\n\
              Connection: close\r\n\
              \r\n",
        )
        .await
        .expect("first request");
    let mut first_headers = [0_u8; 512];
    let first_read = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        first.read(&mut first_headers),
    )
    .await
    .expect("first headers within timeout")
    .expect("first headers");
    assert!(
        std::str::from_utf8(&first_headers[..first_read])
            .expect("first headers utf8")
            .starts_with("HTTP/1.1 200"),
        "first stream must be admitted"
    );

    let mut replacement = tokio::net::TcpStream::connect(addr)
        .await
        .expect("replacement tcp");
    replacement
        .write_all(
            b"GET /api/webchat/v2/threads/thread-b/events?connection_id=browser-tab&connection_generation=2 HTTP/1.1\r\n\
              Host: localhost\r\n\
              Accept: text/event-stream\r\n\
              Connection: close\r\n\
              \r\n",
        )
        .await
        .expect("replacement request");
    let mut replacement_headers = [0_u8; 512];
    let replacement_read = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        replacement.read(&mut replacement_headers),
    )
    .await
    .expect("replacement headers within timeout")
    .expect("replacement headers");
    assert!(
        std::str::from_utf8(&replacement_headers[..replacement_read])
            .expect("replacement headers utf8")
            .starts_with("HTTP/1.1 200"),
        "same-tab replacement must bypass its own stale slot"
    );

    let first_closed = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let mut buffer = [0_u8; 512];
        loop {
            if first.read(&mut buffer).await.expect("read first stream") == 0 {
                return;
            }
        }
    })
    .await;
    assert!(
        first_closed.is_ok(),
        "superseded stream must close promptly instead of retaining a slot"
    );

    let mut late_stale = tokio::net::TcpStream::connect(addr)
        .await
        .expect("late stale tcp");
    late_stale
        .write_all(
            b"GET /api/webchat/v2/threads/thread-a/events?connection_id=browser-tab&connection_generation=1 HTTP/1.1\r\n\
              Host: localhost\r\n\
              Accept: text/event-stream\r\n\
              Connection: close\r\n\
              \r\n",
        )
        .await
        .expect("late stale request");
    let mut stale_headers = [0_u8; 512];
    let stale_read = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        late_stale.read(&mut stale_headers),
    )
    .await
    .expect("stale response within timeout")
    .expect("stale response");
    assert!(
        std::str::from_utf8(&stale_headers[..stale_read])
            .expect("stale response utf8")
            .starts_with("HTTP/1.1 204"),
        "a delayed older route request must stop without replacing the current stream"
    );

    let mut different_tab = tokio::net::TcpStream::connect(addr)
        .await
        .expect("different-tab tcp");
    different_tab
        .write_all(
            b"GET /api/webchat/v2/threads/thread-c/events?connection_id=other-tab HTTP/1.1\r\n\
              Host: localhost\r\n\
              Accept: text/event-stream\r\n\
              Connection: close\r\n\
              \r\n",
        )
        .await
        .expect("different-tab request");
    let mut rejected_headers = [0_u8; 512];
    let rejected_read = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        different_tab.read(&mut rejected_headers),
    )
    .await
    .expect("rejection headers within timeout")
    .expect("rejection headers");
    assert!(
        std::str::from_utf8(&rejected_headers[..rejected_read])
            .expect("rejection headers utf8")
            .starts_with("HTTP/1.1 429"),
        "a distinct tab must still respect the per-caller cap"
    );

    drop(replacement);
    serve_handle.abort();
}

// Regression for the WS-shares-SSE-pool review (Medium): the WS
// transport must draw from the same `SseCapacity` pool as the SSE
// transport for the same `(tenant, user)`. If they kept independent
// counters, a caller could open `cap` SSE streams *and* `cap` WS
// streams concurrently — doubling the backend `stream_events` drain
// the cap is supposed to bound.
//
// The PR description claims this shared-pool semantic; this test
// locks it in by making the pool size 1, consuming the only slot
// with an held-open SSE response, then asserting a same-caller WS
// upgrade attempt returns 429 until the SSE body is dropped.
#[tokio::test]
async fn stream_events_ws_shares_capacity_with_sse_streams() {
    let services: Arc<dyn ProductSurface> = Arc::new(StubServices::default());
    // Pool size 1: any one open stream (SSE or WS) must exhaust the
    // budget for the caller.
    let router = webui_v2_router(WebUiV2State::new(services, 1)).layer(axum::Extension(caller()));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    let serve_handle = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });

    // Step 1: consume the only slot with a held-open SSE connection
    // via a low-level reqwest-style raw HTTP GET. We use plain TCP
    // so we can hold the response open without consuming the body
    // — the `SseSlot` guard lives inside the response body and is
    // released only when the stream drops.
    let mut sse_stream = tokio::net::TcpStream::connect(addr).await.expect("tcp");
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    sse_stream
        .write_all(
            b"GET /api/webchat/v2/threads/thread-x/events HTTP/1.1\r\n\
              Host: localhost\r\n\
              Accept: text/event-stream\r\n\
              Connection: keep-alive\r\n\
              \r\n",
        )
        .await
        .expect("write sse request");
    // Read just enough to confirm we got a 200 OK + the start of
    // headers; this guarantees the handler ran `try_acquire`.
    let mut header_buf = [0u8; 512];
    let n = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        sse_stream.read(&mut header_buf),
    )
    .await
    .expect("sse header read within 5s")
    .expect("sse header read");
    let header_prefix = std::str::from_utf8(&header_buf[..n]).expect("utf8 headers");
    assert!(
        header_prefix.starts_with("HTTP/1.1 200"),
        "SSE handshake must return 200; got: {header_prefix:?}",
    );

    // Step 2: same-caller WS upgrade must hit the shared cap. Use a
    // real WS handshake against the same listener; the upgrade
    // response carries the 429 from `try_acquire` before any frames
    // flow.
    let ws_url = format!("ws://{addr}/api/webchat/v2/threads/thread-x/ws");
    let ws_attempt = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio_tungstenite::connect_async(ws_url.clone()),
    )
    .await
    .expect("ws connect attempt within 5s");
    match ws_attempt {
        Ok((_ws, response)) => panic!(
            "WS upgrade must be rejected while SSE holds the only slot; \
             instead the server returned status {} and completed the upgrade",
            response.status().as_u16(),
        ),
        Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
            assert_eq!(
                response.status().as_u16(),
                429,
                "WS upgrade must hit the same per-caller cap as SSE",
            );
        }
        Err(other) => panic!("WS upgrade failed with unexpected error: {other:?}"),
    }

    // Step 3: drop the SSE stream → kernel closes the connection
    // → axum drops the response body → `SseSlot` decrements. After
    // a yield the slot is reusable and the WS upgrade succeeds.
    drop(sse_stream);
    tokio::task::yield_now().await;
    // Give the server task a moment to observe the EOF and drop
    // the body; we cannot await a specific signal, but a short
    // polling loop converges quickly without timing flakiness.
    let recovered = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            match tokio_tungstenite::connect_async(ws_url.clone()).await {
                Ok((ws, response)) => return Ok::<_, ()>((ws, response)),
                Err(_) => tokio::time::sleep(std::time::Duration::from_millis(25)).await,
            }
        }
    })
    .await
    .expect("WS must complete upgrade within 5s after the SSE slot is released");
    let (mut ws, response) = recovered.expect("recovered ws");
    assert_eq!(
        response.status().as_u16(),
        101,
        "WS must complete the upgrade once the SSE slot has been released",
    );
    let _ = ws.close(None).await;
    serve_handle.abort();
}

// Regression for the per-caller SSE concurrency review (Medium): once the
// router is mounted, an authenticated caller must not be able to keep
// opening long-lived `EventSource` connections beyond the configured cap
// — even though each new request stays under the descriptor's per-caller
// rate limit. Without the cap, sustained reconnects would multiply
// backend `stream_events` drains at `connections × poll-interval`.
#[tokio::test]
async fn stream_events_caps_concurrent_streams_per_caller() {
    let services: Arc<dyn ProductSurface> = Arc::new(StubServices::default());
    // Use a low custom cap so the test runs without burning resources.
    let router = webui_v2_router(WebUiV2State::new(services, 2)).layer(axum::Extension(caller()));

    let open_stream = || {
        router.clone().oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/threads/thread-x/events")
                .body(Body::empty())
                .expect("request"),
        )
    };

    let first = open_stream().await.expect("first oneshot");
    assert_eq!(first.status(), StatusCode::OK);
    let second = open_stream().await.expect("second oneshot");
    assert_eq!(second.status(), StatusCode::OK);

    // Third open must hit the cap. Keep the first two responses alive so
    // their slots stay reserved — the SSE generator (and the slot it
    // owns) lives inside the response body.
    let third = open_stream().await.expect("third oneshot");
    assert_eq!(
        third.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "third concurrent open from same caller must be rejected"
    );
    let body = read_json(third).await;
    assert_eq!(body["error"], "rate_limited");
    assert_eq!(body["kind"], "busy");
    assert_eq!(body["retryable"], true);

    // Release the first stream — slot returns to the pool.
    drop(first);
    // The SSE body's drop chain runs synchronously, but yield once so any
    // pending wakers settle before we measure recovery.
    tokio::task::yield_now().await;

    let recovered = open_stream().await.expect("oneshot after release");
    assert_eq!(
        recovered.status(),
        StatusCode::OK,
        "slot must be reusable after the earlier stream is dropped"
    );

    drop(second);
    drop(recovered);
}

// Regression for the "stalled facade drain" review point: SSE_MAX_LIFETIME
// must bound the await on `services.stream_events`, not just the top-of-loop
// check. If a projection drain stalls (e.g. an upstream wedge), an unbounded
// `.await` would keep the `SseSlot` held even after the configured lifetime
// elapses — defeating the per-caller concurrency recovery the cap promises.
//
// Drives a stub whose `stream_events` returns a future that never resolves,
// advances Tokio's virtual time past `SSE_MAX_LIFETIME`, and asserts the
// stream actually terminates and the slot is reusable for a new connection.
#[tokio::test(start_paused = true)]
async fn stream_events_releases_slot_when_facade_drain_stalls_past_max_lifetime() {
    // Cap of 1 so we can observe slot release directly: a second open
    // returns 429 while the first is held, and 200 once it's released.
    let services = Arc::new(ProgrammableProductSurface::default());
    services.stall_stream_events();
    let services: Arc<dyn ProductSurface> = services;
    let router = webui_v2_router(WebUiV2State::new(services, 1)).layer(axum::Extension(caller()));

    let open_stream = || {
        router.clone().oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/threads/thread-x/events")
                .body(Body::empty())
                .expect("request"),
        )
    };

    // First open: handler acquires the slot and constructs the SSE body.
    let first = open_stream().await.expect("first oneshot");
    assert_eq!(first.status(), StatusCode::OK);

    // Spawn a task that drains the body so the SSE generator actually runs
    // and reaches the `tokio::time::timeout(...)` against the stalled drain.
    let mut first_body = first.into_body();
    let body_task = tokio::spawn(async move { while (first_body.frame().await).is_some() {} });

    // Yield so the spawned body poll runs at least once and parks inside
    // the drain timeout future.
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;

    // While the only stream is stalled, opening another must hit the cap.
    let blocked = open_stream().await.expect("blocked oneshot");
    assert_eq!(
        blocked.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "with the only stream stalled inside the drain, the slot must be held"
    );
    drop(blocked);

    // Advance virtual time past SSE_MAX_LIFETIME. The drain timeout fires,
    // the generator returns, the `SseSlot` Drop releases the slot.
    tokio::time::advance(Duration::from_secs(6 * 60)).await;

    // Body task completes when the generator returns. Cap with a real
    // timeout in case the body hangs (would surface a regression cleanly).
    tokio::time::timeout(Duration::from_secs(2), body_task)
        .await
        .expect("body task must complete after SSE_MAX_LIFETIME elapses")
        .expect("body task joined cleanly");

    // Slot must now be free; a fresh open succeeds.
    let recovered = open_stream().await.expect("oneshot after slot release");
    assert_eq!(
        recovered.status(),
        StatusCode::OK,
        "slot must be released after the lifetime budget bounds the stalled drain"
    );
    drop(recovered);
}

/// Build a minimal `ProductOutboundEnvelope` with a caller-supplied
/// projection cursor and reply text. The exact payload shape is not the
/// contract under test (it lives in `ironclaw_product`); these
/// tests only care that whatever the facade hands back becomes a
/// well-formed SSE event.
fn make_projection_envelope(cursor: &str, text: &str) -> ProductOutboundEnvelope {
    make_outbound_envelope(
        cursor,
        ProductOutboundPayload::FinalReply(FinalReplyView {
            turn_run_id: TurnRunId::new(),
            text: text.into(),
            generated_at: chrono::Utc::now(),
        }),
    )
}

fn make_tool_progress_envelope(cursor: &str) -> ProductOutboundEnvelope {
    make_outbound_envelope(
        cursor,
        ProductOutboundPayload::Progress(ProgressUpdateView {
            turn_run_id: TurnRunId::new(),
            kind: ProgressKind::ToolRunning,
            generated_at: chrono::Utc::now(),
        }),
    )
}

fn make_projection_update_envelope(cursor: &str) -> ProductOutboundEnvelope {
    make_outbound_envelope(
        cursor,
        ProductOutboundPayload::ProjectionUpdate {
            state: ProductProjectionState::new(
                "thread-x",
                vec![ProductProjectionItem::Text {
                    id: "message-1".to_string(),
                    run_id: None,
                    body: "projection body".to_string(),
                }],
            )
            .expect("projection state"),
        },
    )
}

fn make_capability_activity_envelope(cursor: &str) -> ProductOutboundEnvelope {
    make_outbound_envelope(
        cursor,
        ProductOutboundPayload::CapabilityActivity(CapabilityActivityView {
            invocation_id: InvocationId::new(),
            turn_run_id: Some(TurnRunId::new()),
            thread_id: Some(ThreadId::new("thread-x").expect("thread id")),
            capability_id: CapabilityId::new("script.echo").expect("capability id"),
            status: CapabilityActivityStatusView::Running,
            provider: Some(ExtensionId::new("script").expect("provider id")),
            runtime: Some(RuntimeKind::Script),
            process_id: None,
            output_bytes: None,
            error_kind: None,
            error_detail: None,
            subtitle: None,
            input_summary: None,
            updated_at: chrono::Utc::now(),
            activity_order: None,
        }),
    )
}

fn make_outbound_envelope(
    cursor: &str,
    payload: ProductOutboundPayload,
) -> ProductOutboundEnvelope {
    ProductOutboundEnvelope::new(
        ProductAdapterId::new("webui_v2").expect("adapter id"), // safety: literal valid id
        AdapterInstallationId::new("install:alpha").expect("install id"), // safety: literal valid id
        ProductOutboundTarget::new(
            ReplyTargetBindingRef::new("reply:fake").expect("reply ref"), // safety: literal valid ref
            ExternalConversationRef::new(None, "conv-1", None, None).expect("conv ref"), // safety: literal valid ref
            None,
        ),
        ProjectionCursor::new(cursor).expect("cursor"), // safety: test-supplied
        payload,
    )
}

/// One parsed SSE event from the wire bytes. `event:`, `id:`, and `data:`
/// fields are extracted; everything else (comments, keep-alives) is
/// ignored.
#[derive(Default, Debug)]
struct ParsedSseEvent {
    event: Option<String>,
    id: Option<String>,
    data: Option<String>,
}

/// Minimal SSE chunk parser tailored to the handler's emit shape. The
/// handler writes each event as `event: <name>\n[id: <cursor>\n]data:
/// <json>\n\n`; this helper splits the buffer on the blank-line
/// separator and pulls out the three fields. It is deliberately not a
/// general SSE parser — the handler's emit shape is fixed and any drift
/// would be the regression the surrounding tests are pinning.
fn parse_sse_events(bytes: &[u8]) -> Vec<ParsedSseEvent> {
    let text = String::from_utf8_lossy(bytes);
    let mut events = Vec::new();
    for block in text.split("\n\n") {
        let block = block.trim_matches(|c| c == '\n' || c == '\r');
        if block.is_empty() {
            continue;
        }
        let mut parsed = ParsedSseEvent::default();
        for line in block.split('\n') {
            let line = line.trim_end_matches('\r');
            if let Some(rest) = line.strip_prefix("event:") {
                parsed.event = Some(rest.trim_start().to_string());
            } else if let Some(rest) = line.strip_prefix("id:") {
                parsed.id = Some(rest.trim_start().to_string());
            } else if let Some(rest) = line.strip_prefix("data:") {
                parsed.data = Some(rest.trim_start().to_string());
            }
        }
        if parsed.event.is_some() || parsed.data.is_some() {
            events.push(parsed);
        }
    }
    events
}

/// Pull body frames until the predicate fires or the timeout elapses,
/// returning whatever bytes were collected. SSE bodies in axum surface as
/// a stream of frames where each frame is a single `\n\n`-terminated
/// event; tests want to inspect the wire shape after N events arrive.
async fn collect_sse_until<F>(body: &mut Body, timeout: Duration, mut done: F) -> Vec<u8>
where
    F: FnMut(&[u8]) -> bool,
{
    let deadline = std::time::Instant::now() + timeout;
    let mut buf = Vec::<u8>::new();
    while std::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        match tokio::time::timeout(remaining, body.frame()).await {
            Ok(Some(Ok(frame))) => {
                if let Some(data) = frame.data_ref() {
                    buf.extend_from_slice(data.as_ref());
                    if done(&buf) {
                        return buf;
                    }
                }
            }
            // Stream closed or errored: return what we have so the caller
            // can still assert on the bytes we collected before close.
            Ok(_) => return buf,
            Err(_) => return buf,
        }
    }
    buf
}

#[tokio::test]
async fn stream_events_sets_unbuffered_sse_headers() {
    let services = Arc::new(StubServices::default());

    let router = router_with(services);
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/threads/thread-x/events")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let headers = response.headers();
    assert_eq!(
        headers
            .get(header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok()),
        Some("no-cache, no-transform"),
        "SSE must opt out of intermediary transforms that can buffer chunks"
    );
    assert_eq!(
        headers
            .get(HeaderName::from_static("x-accel-buffering"))
            .and_then(|value| value.to_str().ok()),
        Some("no"),
        "SSE must ask reverse proxies not to buffer stream chunks"
    );
    assert!(
        headers
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("text/event-stream")),
        "SSE content type must remain event-stream"
    );
}

#[tokio::test]
async fn stream_events_continues_immediately_after_non_empty_batch() {
    let services = Arc::new(StubServices::default());

    let envelope_a = make_projection_update_envelope("cursor:live-a");
    let envelope_b = make_projection_update_envelope("cursor:live-b");
    services.enqueue_stream_events(Ok(RebornStreamEventsResponse {
        events: vec![envelope_a],
    }));
    services.enqueue_stream_events(Ok(RebornStreamEventsResponse {
        events: vec![envelope_b],
    }));

    let router = router_with(services.clone());
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/threads/thread-x/events")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);

    let mut body = response.into_body();
    let bytes = collect_sse_until(&mut body, Duration::from_millis(750), |buf| {
        parse_sse_events(buf).len() >= 2
    })
    .await;
    drop(body);

    let events = parse_sse_events(&bytes);
    assert!(
        events.len() >= 2,
        "second SSE event must not wait for the idle poll interval; got {events:?}; raw: {}",
        String::from_utf8_lossy(&bytes)
    );

    let calls = services.stream_events_calls.lock().expect("lock").clone();
    assert!(
        calls.len() >= 2,
        "SSE handler must immediately re-enter stream_events after a non-empty batch"
    );
    let expected_cursor = ProjectionCursor::new("cursor:live-a").expect("cursor");
    assert_eq!(
        calls[1].after_cursor.as_ref(),
        Some(&expected_cursor),
        "follow-up call must still preserve cursor ordering"
    );
}

// Pins the *wire* contract the browser sees, not just the handler being
// called: each envelope must emit a typed WebChat v2 event with the
// JSON-serialized projection cursor as the SSE `id` and the redacted
// browser frame as `data`. Also asserts that the next poll carries the
// *latest* cursor in `after_cursor`, so a future refactor that loses
// cursor advancement breaks loudly.
#[tokio::test]
async fn stream_events_emits_typed_browser_events_with_cursor_ids() {
    let services = Arc::new(StubServices::default());

    let envelope_a = make_projection_envelope("cursor:a", "hello");
    let envelope_b = make_tool_progress_envelope("cursor:b");
    let envelope_c = make_projection_update_envelope("cursor:c");
    let envelope_d = make_capability_activity_envelope("cursor:d");

    services.enqueue_stream_events(Ok(RebornStreamEventsResponse {
        events: vec![
            envelope_a.clone(),
            envelope_b.clone(),
            envelope_c.clone(),
            envelope_d.clone(),
        ],
    }));
    // Second drain is empty: lets the test observe `after_cursor`
    // advancement on the follow-up call without producing more events.
    services.enqueue_stream_events(Ok(RebornStreamEventsResponse { events: Vec::new() }));

    let router = router_with(services.clone());
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/threads/thread-x/events")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);

    // Pump frames directly in this task — the body cannot be moved to a
    // background task and then dropped, since dropping kills the SSE
    // generator before the second `stream_events` call can run. Instead,
    // keep awaiting frames in-place, accumulating bytes, until we have
    // both (a) the two emitted SSE events and (b) the second drain call
    // observed via `services.stream_events_calls`.
    let mut body = response.into_body();
    let mut bytes = Vec::<u8>::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let have_events = bytes.windows(2).filter(|w| *w == b"\n\n").count() >= 4;
        let saw_second_call = services.stream_events_calls.lock().expect("lock").len() >= 2;
        if have_events && saw_second_call {
            break;
        }
        if std::time::Instant::now() >= deadline {
            break;
        }
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        match tokio::time::timeout(remaining, body.frame()).await {
            Ok(Some(Ok(frame))) => {
                if let Some(data) = frame.data_ref() {
                    bytes.extend_from_slice(data.as_ref());
                }
            }
            _ => break,
        }
    }
    drop(body);

    let events = parse_sse_events(&bytes);
    assert!(
        events.len() >= 4,
        "expected at least four SSE events, got: {events:?}; raw: {}",
        String::from_utf8_lossy(&bytes)
    );

    let cursor_a_json =
        serde_json::to_string(envelope_a.projection_cursor()).expect("cursor-a json");
    let cursor_b_json =
        serde_json::to_string(envelope_b.projection_cursor()).expect("cursor-b json");
    let cursor_c_json =
        serde_json::to_string(envelope_c.projection_cursor()).expect("cursor-c json");
    let cursor_d_json =
        serde_json::to_string(envelope_d.projection_cursor()).expect("cursor-d json");

    assert_eq!(events[0].event.as_deref(), Some("final_reply"));
    assert_eq!(events[0].id.as_deref(), Some(cursor_a_json.as_str()));
    let event_a_json: Value =
        serde_json::from_str(events[0].data.as_deref().expect("data")).expect("event a json");
    assert_eq!(event_a_json["cursor"], "cursor:a");
    assert_eq!(event_a_json["type"], "final_reply");
    assert_eq!(event_a_json["reply"]["text"], "hello");
    assert!(event_a_json["reply"]["turn_run_id"].is_string());
    assert!(event_a_json["reply"]["generated_at"].is_string());
    assert!(
        event_a_json.get("target").is_none(),
        "browser event frame must not expose adapter target metadata"
    );
    assert!(
        event_a_json.get("delivery_attempt_id").is_none(),
        "browser event frame must not expose delivery metadata"
    );

    assert_eq!(events[1].event.as_deref(), Some("capability_progress"));
    assert_eq!(events[1].id.as_deref(), Some(cursor_b_json.as_str()));
    let event_b_json: Value =
        serde_json::from_str(events[1].data.as_deref().expect("data")).expect("event b json");
    assert_eq!(event_b_json["cursor"], "cursor:b");
    assert_eq!(event_b_json["type"], "capability_progress");
    assert_eq!(event_b_json["progress"]["kind"], "tool_running");

    assert_eq!(events[2].event.as_deref(), Some("projection_update"));
    assert_eq!(events[2].id.as_deref(), Some(cursor_c_json.as_str()));
    let event_c_json: Value =
        serde_json::from_str(events[2].data.as_deref().expect("data")).expect("event c json");
    assert_eq!(event_c_json["cursor"], "cursor:c");
    assert_eq!(event_c_json["type"], "projection_update");
    assert_eq!(event_c_json["state"]["thread_id"], "thread-x");
    assert_eq!(
        event_c_json["state"]["items"][0]["text"]["body"],
        "projection body"
    );

    assert_eq!(events[3].event.as_deref(), Some("capability_activity"));
    assert_eq!(events[3].id.as_deref(), Some(cursor_d_json.as_str()));
    let event_d_json: Value =
        serde_json::from_str(events[3].data.as_deref().expect("data")).expect("event d json");
    assert_eq!(event_d_json["cursor"], "cursor:d");
    assert_eq!(event_d_json["type"], "capability_activity");
    assert_eq!(event_d_json["activity"]["status"], "running");
    assert_eq!(event_d_json["activity"]["capability_id"], "script.echo");
    assert!(event_d_json["activity"].get("arguments").is_none());
    assert!(event_d_json["activity"].get("result").is_none());
    assert_no_adapter_metadata(&event_b_json);
    assert_no_adapter_metadata(&event_c_json);
    assert_no_adapter_metadata(&event_d_json);

    let calls = services.stream_events_calls.lock().expect("lock").clone();
    assert!(
        calls.len() >= 2,
        "second poll must occur so cursor advancement is observable; saw {} call(s)",
        calls.len()
    );
    assert_eq!(
        calls[1].after_cursor.as_ref(),
        Some(envelope_d.projection_cursor()),
        "second poll must advance after_cursor to the last emitted cursor"
    );
}

fn assert_no_adapter_metadata(json: &Value) {
    assert!(
        json.get("target").is_none(),
        "browser event frame must not expose adapter target metadata"
    );
    assert!(
        json.get("delivery_attempt_id").is_none(),
        "browser event frame must not expose delivery metadata"
    );
}

// Regression for the "SSE facade error event path is untested" review
// (Medium). When `ProductSurface::stream_events` returns Err, the
// handler must emit one SSE `error` frame carrying only the redacted
// `error` code + `retryable` flag (no `field`, no internal `detail`),
// then close the stream — never propagate an HTTP error on a long-lived
// SSE connection because the browser would replay it as a hard
// reconnect failure.
#[tokio::test]
async fn stream_events_facade_error_emits_redacted_error_event_and_closes() {
    let services = Arc::new(StubServices::default());
    services.enqueue_stream_events(Err(ProductSurfaceError {
        code: ProductSurfaceErrorCode::Forbidden,
        kind: ProductSurfaceErrorKind::ParticipantDenied,
        status_code: 403,
        retryable: false,
        // The handler must NOT echo these into the SSE payload — the
        // redacted shape carries only `error`, `kind`, and `retryable`.
        field: Some("thread_id".into()),
        validation_code: None,
    }));

    let router = router_with(services.clone());
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/threads/thread-x/events")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    // The handler must surface the facade error as an SSE event, not as a
    // failed HTTP open. EventSource cannot recover from a non-OK status.
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "SSE open must succeed even when the facade drain errors; the error path is an in-stream event"
    );

    let mut body = response.into_body();
    // Read until we see a `stream_error` event chunk, or the stream closes.
    let bytes = collect_sse_until(&mut body, Duration::from_secs(2), |buf| {
        buf.windows(b"event: stream_error".len())
            .any(|w| w == b"event: stream_error")
            && buf.windows(2).any(|w| w == b"\n\n")
    })
    .await;

    let events = parse_sse_events(&bytes);
    let error_event = events
        .iter()
        .find(|event| event.event.as_deref() == Some("stream_error"))
        .unwrap_or_else(|| {
            panic!(
                "expected an SSE `stream_error` event, got: {events:?}; raw: {}",
                String::from_utf8_lossy(&bytes)
            )
        });
    let payload: Value = serde_json::from_str(error_event.data.as_deref().expect("error data"))
        .expect("error data is JSON");
    assert_eq!(
        payload["error"], "forbidden",
        "error event must carry the redacted error code"
    );
    assert_eq!(
        payload["kind"], "participant_denied",
        "error event must carry the redacted error kind"
    );
    assert_eq!(
        payload["retryable"], false,
        "error event must carry the retryable flag verbatim"
    );
    assert!(
        payload.get("field").is_none(),
        "redacted SSE error payload must not leak the failing field name"
    );
    assert!(
        payload.get("validation_code").is_none(),
        "redacted SSE error payload must not leak validation metadata"
    );

    // The stream closes after the error event. Polling once more must
    // return `None` (end-of-stream) within a small budget.
    let final_frame = tokio::time::timeout(Duration::from_millis(500), body.frame()).await;
    let closed = matches!(final_frame, Ok(None) | Err(_));
    assert!(
        closed,
        "facade error must close the SSE stream, but body.frame() yielded another chunk"
    );
}

#[tokio::test]
async fn missing_caller_extension_returns_500() {
    // No `Extension(caller)` layer — exercises the failure mode if host
    // composition forgets to run the bearer middleware.
    let services: Arc<dyn ProductSurface> = Arc::new(StubServices::default());
    let router = webui_v2_router(WebUiV2State::new(
        services,
        DEFAULT_SSE_MAX_CONCURRENT_PER_CALLER,
    ));

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/threads")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"client_action_id":"act-1"}"#))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    // axum's `Extension` extractor maps a missing extension to 500.
    assert_eq!(
        response.status(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "missing caller extension must fail closed, not bypass auth"
    );

    // Drain the body to make sure no facade method was hit before the
    // extractor failed.
    let _ = response.into_body().collect().await.expect("drain body");
}

// Regression for the "WS transport's projection payload + redacted
// error frame untested" review (Medium). The composition crate's WS
// caller-level test verifies the upgrade returns 101, but only a real
// WS connection that pumps frames can catch breakage in the
// per-envelope JSON serialization, cursor advancement on the
// `after_cursor` field, or the redacted error frame the handler emits
// on facade failure.
#[tokio::test]
async fn stream_events_ws_emits_projection_frames_and_redacted_error() {
    use futures::StreamExt;
    use tokio_tungstenite::tungstenite::Message as WsMessage;

    let services = Arc::new(StubServices::default());

    let envelope_a = make_projection_envelope("cursor:a", "hello");
    let envelope_b = make_projection_envelope("cursor:b", "world");
    services.enqueue_stream_events(Ok(RebornStreamEventsResponse {
        events: vec![envelope_a.clone(), envelope_b.clone()],
    }));
    // After draining the two real events, the next drain produces a
    // facade error so the handler exercises the redacted-error-frame +
    // close path before lifetime expiry.
    services.enqueue_stream_events(Err(ProductSurfaceError {
        code: ProductSurfaceErrorCode::Unavailable,
        kind: ProductSurfaceErrorKind::ServiceUnavailable,
        status_code: 503,
        retryable: true,
        field: None,
        validation_code: None,
    }));

    let router = router_with(services.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    let serve_handle = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });

    let url = format!("ws://{addr}/api/webchat/v2/threads/thread-x/ws");
    let (mut ws, response) = tokio::time::timeout(
        Duration::from_secs(5),
        tokio_tungstenite::connect_async(url),
    )
    .await
    .expect("ws connect within 5s")
    .expect("ws upgrade");
    assert_eq!(response.status().as_u16(), 101);

    // Read frames until we see both projection envelopes and the
    // redacted error frame, or the stream closes.
    let mut text_frames: Vec<String> = Vec::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline && text_frames.len() < 3 {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        match tokio::time::timeout(remaining, ws.next()).await {
            Ok(Some(Ok(WsMessage::Text(text)))) => text_frames.push(text.to_string()),
            Ok(Some(Ok(WsMessage::Close(_)))) | Ok(None) => break,
            Ok(Some(Ok(_))) => continue, // ignore ping/pong/binary
            Ok(Some(Err(_))) => break,
            Err(_) => break,
        }
    }
    let _ = ws.close(None).await;
    serve_handle.abort();

    assert!(
        text_frames.len() >= 3,
        "expected projection envelopes + error frame; got {} text frame(s): {:?}",
        text_frames.len(),
        text_frames,
    );

    // First two frames carry the projection envelopes, in order.
    let envelope_a_json: Value = serde_json::from_str(&text_frames[0]).expect("envelope a parses");
    let expected_a: Value = serde_json::to_value(&envelope_a).expect("envelope a value");
    assert_eq!(
        envelope_a_json, expected_a,
        "first WS frame must carry the first ProductOutboundEnvelope verbatim",
    );
    let envelope_b_json: Value = serde_json::from_str(&text_frames[1]).expect("envelope b parses");
    let expected_b: Value = serde_json::to_value(&envelope_b).expect("envelope b value");
    assert_eq!(envelope_b_json, expected_b);

    // Third frame is the redacted error payload — `error` code +
    // `retryable` flag only. No `detail`, `field`, `validation_code`,
    // or any internal diagnostic must leak through.
    let error_json: Value =
        serde_json::from_str(&text_frames[2]).expect("error frame parses as json");
    assert_eq!(error_json["error"], serde_json::json!("unavailable"));
    assert_eq!(error_json["retryable"], serde_json::json!(true));
    assert!(
        error_json.get("detail").is_none(),
        "redacted error frame must not carry server diagnostics",
    );
    assert!(error_json.get("field").is_none());
    assert!(error_json.get("validation_code").is_none());

    // The handler must have advanced `after_cursor` between the two
    // drains so the browser would resume from cursor:b on reconnect.
    let calls = services.stream_events_calls.lock().expect("lock").clone();
    assert!(
        calls.len() >= 2,
        "second poll must occur for the redacted-error path to fire",
    );
    assert_eq!(
        calls[1].after_cursor.as_ref(),
        Some(envelope_b.projection_cursor()),
        "second WS poll must advance after_cursor to the last emitted projection cursor",
    );
}

#[tokio::test]
async fn stream_events_ws_resumes_from_last_event_id_before_query_cursor() {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    let services = Arc::new(StubServices::default());
    services.enqueue_stream_events(Err(ProductSurfaceError {
        code: ProductSurfaceErrorCode::Unavailable,
        kind: ProductSurfaceErrorKind::ServiceUnavailable,
        status_code: 503,
        retryable: true,
        field: None,
        validation_code: None,
    }));

    let query_cursor = make_projection_envelope("cursor:query", "query");
    let header_cursor = make_projection_envelope("cursor:header", "header");
    let query_cursor_json =
        serde_json::to_string(query_cursor.projection_cursor()).expect("query cursor");
    let header_cursor_json =
        serde_json::to_string(header_cursor.projection_cursor()).expect("header cursor");

    let router = router_with(services.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    let serve_handle = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });

    let url = format!(
        "ws://{addr}/api/webchat/v2/threads/thread-x/ws?after_cursor={}",
        url_encode(&query_cursor_json)
    );
    let mut request = url.into_client_request().expect("ws request");
    request.headers_mut().insert(
        "Last-Event-ID",
        header_cursor_json.parse().expect("header cursor value"),
    );

    let (mut ws, response) = tokio::time::timeout(
        Duration::from_secs(5),
        tokio_tungstenite::connect_async(request),
    )
    .await
    .expect("ws connect within 5s")
    .expect("ws upgrade");
    assert_eq!(response.status().as_u16(), 101);

    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        if !services
            .stream_events_calls
            .lock()
            .expect("lock")
            .is_empty()
        {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "WS handler did not call stream_events"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let _ = ws.close(None).await;
    serve_handle.abort();

    let calls = services.stream_events_calls.lock().expect("lock").clone();
    assert_eq!(
        calls[0].after_cursor.as_ref(),
        Some(header_cursor.projection_cursor()),
        "Last-Event-ID must win over ?after_cursor= for WS reconnects, matching SSE"
    );
}

// Regression for the WS-idle-close review (Medium): the WS drain
// loop must observe socket close immediately. Without this, an
// idle peer (closed tab, dropped network) leaves the loop polling
// the facade at the 1Hz cadence — its per-caller `SseSlot` stays
// reserved until `SSE_MAX_LIFETIME` (5 min). With the recv-aware
// select, a peer close releases the slot within one poll cycle.
//
// The test pins the budget at 1 stream per caller, opens a WS,
// closes the browser side, and asserts a subsequent WS upgrade from
// the same caller succeeds within ~2s (well under the 5-minute
// lifetime). If the loop didn't observe the close, the second
// upgrade would 429 for minutes.
#[tokio::test]
async fn stream_events_ws_releases_slot_on_peer_close() {
    use futures::SinkExt;

    let services: Arc<dyn ProductSurface> = Arc::new(StubServices::default());
    let router = webui_v2_router(WebUiV2State::new(services, 1)).layer(axum::Extension(caller()));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    let serve_handle = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });

    let url = format!("ws://{addr}/api/webchat/v2/threads/thread-x/ws");

    // Open WS #1, send a Close frame, drop the client.
    let (mut ws_one, response) = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio_tungstenite::connect_async(url.clone()),
    )
    .await
    .expect("ws connect within 5s")
    .expect("ws upgrade");
    assert_eq!(response.status().as_u16(), 101);
    let _ = ws_one
        .send(tokio_tungstenite::tungstenite::Message::Close(None))
        .await;
    drop(ws_one);

    // Wait briefly for the server-side WS task to observe the close
    // and release the slot. With the recv-aware select the slot
    // returns within one poll cycle; without it, it would be pinned
    // for SSE_MAX_LIFETIME.
    let recovered = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            match tokio_tungstenite::connect_async(url.clone()).await {
                Ok(pair) => return pair,
                Err(_) => tokio::time::sleep(std::time::Duration::from_millis(50)).await,
            }
        }
    })
    .await
    .expect(
        "second WS upgrade must succeed within 3s after peer close \
         — the slot should have been released by the recv-aware select",
    );
    assert_eq!(
        recovered.1.status().as_u16(),
        101,
        "second WS upgrade must complete once the slot has been released",
    );
    let mut ws_two = recovered.0;
    let _ = ws_two.close(None).await;
    serve_handle.abort();
}

#[tokio::test]
async fn operator_setup_accepts_secret_request_without_echoing_values() {
    let services = Arc::new(StubServices::default());
    let router = router_with_capabilities(
        services.clone(),
        WebUiV2Capabilities {
            operator_webui_config: true,
        },
    );

    services.enqueue_invoke_response(Ok(successful_resolution(ActivityId::new())));
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/operator/setup")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"provider_id":"openai","adapter":"open_ai_completions","model":"gpt-5-mini","api_key":"sk-secret-value","webui_access_token":"webui-secret-value"}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let body = read_json(response).await;
    assert_eq!(body["active_provider_id"], "openai");
    assert_eq!(body["active_model"], "gpt-5-mini");
    let rendered = serde_json::to_string(&body).expect("render body");
    assert!(!rendered.contains("sk-secret-value"));
    assert!(!rendered.contains("webui-secret-value"));

    let invoke_calls = services.invoke_calls.lock().expect("lock").clone();
    assert_eq!(invoke_calls.len(), 1);
    assert_eq!(
        invoke_calls[0].0,
        CapabilityId::new(OPERATOR_SETUP_RUN_CAPABILITY_ID).expect("capability id")
    );
    assert_eq!(
        invoke_calls[0].1,
        serde_json::json!({
            "provider_id": "openai",
            "adapter": "open_ai_completions",
            "model": "gpt-5-mini",
            "api_key": "sk-secret-value",
            "webui_access_token": "webui-secret-value"
        })
    );
}

#[tokio::test]
async fn list_fs_mounts_returns_browsable_mounts() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/fs/mounts")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let body = read_json(response).await;
    // Assert set membership, not index order, so a semantically-equivalent
    // ordering change does not fail spuriously.
    let mounts: Vec<&str> = body["mounts"]
        .as_array()
        .expect("mounts array")
        .iter()
        .map(|m| m["mount"].as_str().expect("mount string"))
        .collect();
    assert!(
        mounts.contains(&"memory"),
        "memory mount present: {mounts:?}"
    );
    assert!(
        mounts.contains(&"workspace"),
        "workspace mount present: {mounts:?}"
    );
    let queries = services.view_queries.lock().expect("lock");
    assert_eq!(queries.len(), 1);
    assert_eq!(queries[0].view_id.as_str(), FS_MOUNTS_VIEW.id);
}

#[tokio::test]
async fn browse_fs_dir_lists_mount_relative_entries() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/fs/list?mount=memory&path=daily")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let body = read_json(response).await;
    assert_eq!(body["mount"], "memory");
    assert_eq!(body["entries"][0]["name"], "today.md");
    assert_eq!(body["entries"][0]["path"], "daily/today.md");
    let queries = services.view_queries.lock().expect("lock");
    assert_eq!(queries[0].view_id.as_str(), FS_LIST_VIEW.id);
}

#[tokio::test]
async fn browse_fs_dir_forwards_optional_project_selector() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/fs/list?mount=memory&path=daily&project_id=project-beta")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let calls = services.browse_fs_calls.lock().expect("lock");
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].project_id.as_ref().map(ProjectId::as_str),
        Some("project-beta")
    );
}

#[tokio::test]
async fn list_project_files_queries_product_surface_view() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/threads/thread-alpha/files?path=/workspace")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let body = read_json(response).await;
    assert_eq!(body["entries"][0]["path"], "/workspace/report.md");
    let queries = services.view_queries.lock().expect("lock");
    assert_eq!(queries.len(), 1);
    assert_eq!(queries[0].view_id.as_str(), PROJECT_FS_LIST_VIEW.id);
}

#[tokio::test]
async fn stat_project_file_queries_product_surface_view() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/threads/thread-alpha/files/stat?path=/workspace/report.md")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let body = read_json(response).await;
    assert_eq!(body["stat"]["path"], "/workspace/report.md");
    let queries = services.view_queries.lock().expect("lock");
    assert_eq!(queries.len(), 1);
    assert_eq!(queries[0].view_id.as_str(), PROJECT_FS_STAT_VIEW.id);
}

#[tokio::test]
async fn read_fs_file_serves_attachment_with_nosniff() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services);

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/fs/content?mount=memory&path=daily/today.md")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("x-content-type-options")
            .and_then(|v| v.to_str().ok()),
        Some("nosniff"),
    );
    assert!(
        response
            .headers()
            .get("content-disposition")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|value| value.contains("attachment")),
        "fs download must be served as an attachment",
    );
    let body = to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("body bytes");
    assert_eq!(&body[..], b"# notes");
}

#[tokio::test]
async fn read_fs_file_rejects_blank_path() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services);

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/fs/content?mount=memory")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn stat_fs_path_returns_metadata() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/fs/stat?mount=memory&path=daily/today.md")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let body = read_json(response).await;
    assert_eq!(body["stat"]["path"], "daily/today.md");
    assert_eq!(body["stat"]["kind"], "file");
    assert_eq!(body["stat"]["mime_type"], "text/markdown");
    let queries = services.view_queries.lock().expect("lock");
    assert_eq!(queries[0].view_id.as_str(), FS_STAT_VIEW.id);
}

#[tokio::test]
async fn stat_fs_path_rejects_blank_path() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services);

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/fs/stat?mount=memory")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

// --- Project route handler tests (path-param override + status codes) --------

fn sample_admin_user(user_id: &str) -> AdminUserRecord {
    AdminUserRecord {
        user_id: UserId::new(user_id).expect("user id"),
        email: Some(format!("{user_id}@example.test")),
        display_name: Some("Admin User".to_string()),
        status: AdminUserStatus::Active,
        role: AdminUserRole::Admin,
        created_at: "2026-06-17T00:00:00Z".to_string(),
        updated_at: "2026-06-17T00:00:00Z".to_string(),
        created_by: Some(UserId::new("user-alpha").expect("user id")),
        last_login_at: None,
        metadata: Default::default(),
    }
}

fn sample_project_info(project_id: &str) -> RebornProjectInfo {
    RebornProjectInfo {
        project_id: project_id.to_string(),
        name: "Sample".to_string(),
        description: String::new(),
        icon: None,
        color: None,
        metadata: serde_json::json!({}),
        state: RebornProjectState::Active,
        role: RebornProjectRole::Owner,
        created_at: "2026-06-17T00:00:00Z".parse().expect("created at"),
        updated_at: "2026-06-17T00:00:00Z".parse().expect("updated at"),
    }
}

fn sample_member_info(user_id: &str) -> RebornProjectMemberInfo {
    RebornProjectMemberInfo {
        user_id: user_id.to_string(),
        role: RebornProjectRole::Editor,
        status: RebornProjectMemberStatus::Active,
        granted_by: "user-alpha".to_string(),
        created_at: "2026-06-17T00:00:00Z".parse().expect("created at"),
        updated_at: "2026-06-17T00:00:00Z".parse().expect("updated at"),
    }
}

/// The path `project_id` must override any value carried in the body, so a
/// caller cannot target a different project than the URL names.
#[tokio::test]
async fn update_project_path_id_overrides_body() {
    let services = Arc::new(StubServices::default());
    let app = router_with(services.clone());
    services.enqueue_invoke_response(Ok(successful_resolution(ActivityId::new())));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/webchat/v2/projects/path-project")
                .header("content-type", "application/json")
                // A hostile body names a different project; the path must win.
                .body(Body::from(
                    serde_json::json!({ "project_id": "body-project", "name": "x" }).to_string(),
                ))
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = read_json(response).await;
    assert_eq!(body["project"]["project_id"], "path-project");
    let calls = services.invoke_calls.lock().expect("lock");
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].0,
        CapabilityId::new(PROJECT_UPDATE_CAPABILITY_ID).expect("capability id")
    );
    assert_eq!(
        calls[0].1["project_id"], "path-project",
        "path project_id must override the body value"
    );
    assert_eq!(calls[0].1["name"], "x");
    drop(calls);
    let queries = services.view_queries.lock().expect("lock");
    assert_eq!(queries.len(), 1);
    assert_eq!(queries[0].view_id.as_str(), PROJECT_VIEW.id);
}

#[tokio::test]
async fn get_project_queries_product_surface_view() {
    let services = Arc::new(StubServices::default());
    let app = router_with(services.clone());
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/webchat/v2/projects/project-alpha")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let body = read_json(response).await;
    assert_eq!(body["project"]["project_id"], "project-alpha");
    let queries = services.view_queries.lock().expect("lock");
    assert_eq!(queries.len(), 1);
    assert_eq!(queries[0].view_id.as_str(), PROJECT_VIEW.id);
}

/// Both path ids (project + user) must override the body on member role update.
#[tokio::test]
async fn update_member_path_ids_override_body() {
    let services = Arc::new(StubServices::default());
    let app = router_with(services.clone());
    services.enqueue_invoke_response(Ok(successful_resolution(ActivityId::new())));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/webchat/v2/projects/path-project/members/path-user")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "project_id": "body-project",
                        "user_id": "body-user",
                        "role": "editor"
                    })
                    .to_string(),
                ))
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let calls = services.invoke_calls.lock().expect("lock");
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].0,
        CapabilityId::new(PROJECT_MEMBER_UPDATE_CAPABILITY_ID).expect("capability id")
    );
    assert_eq!(calls[0].1["project_id"], "path-project");
    assert_eq!(calls[0].1["user_id"], "path-user");
    let queries = services.view_queries.lock().expect("lock");
    assert_eq!(queries.len(), 1);
    assert_eq!(queries[0].view_id.as_str(), PROJECT_MEMBERS_VIEW.id);
}

/// `add_project_member` takes user_id from the BODY (the path has no user
/// segment) but the project_id from the path.
#[tokio::test]
async fn add_member_takes_user_from_body_project_from_path() {
    let services = Arc::new(StubServices::default());
    let app = router_with(services.clone());
    services.enqueue_invoke_response(Ok(successful_resolution(ActivityId::new())));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/webchat/v2/projects/path-project/members")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "project_id": "body-project",
                        "user_id": "body-user",
                        "role": "viewer"
                    })
                    .to_string(),
                ))
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let calls = services.invoke_calls.lock().expect("lock");
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].0,
        CapabilityId::new(PROJECT_MEMBER_ADD_CAPABILITY_ID).expect("capability id")
    );
    assert_eq!(
        calls[0].1["project_id"], "path-project",
        "project from path"
    );
    assert_eq!(calls[0].1["user_id"], "body-user", "user from body");
    let queries = services.view_queries.lock().expect("lock");
    assert_eq!(queries.len(), 1);
    assert_eq!(queries[0].view_id.as_str(), PROJECT_MEMBERS_VIEW.id);
}

#[tokio::test]
async fn list_project_members_queries_product_surface_view() {
    let services = Arc::new(StubServices::default());
    let app = router_with(services.clone());
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/webchat/v2/projects/project-alpha/members")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let body = read_json(response).await;
    assert_eq!(body["members"][0]["user_id"], "user-beta");
    let queries = services.view_queries.lock().expect("lock");
    assert_eq!(queries.len(), 1);
    assert_eq!(queries[0].view_id.as_str(), PROJECT_MEMBERS_VIEW.id);
}

#[tokio::test]
async fn delete_project_returns_204() {
    let services = Arc::new(StubServices::default());
    let app = router_with(services.clone());
    services.enqueue_invoke_response(Ok(successful_resolution(ActivityId::new())));
    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/webchat/v2/projects/p1")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let calls = services.invoke_calls.lock().expect("lock");
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].0,
        CapabilityId::new(PROJECT_DELETE_CAPABILITY_ID).expect("capability id")
    );
    assert_eq!(calls[0].1, serde_json::json!({ "project_id": "p1" }));
}

#[tokio::test]
async fn remove_member_returns_204() {
    let services = Arc::new(StubServices::default());
    let app = router_with(services.clone());
    services.enqueue_invoke_response(Ok(successful_resolution(ActivityId::new())));
    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/webchat/v2/projects/p1/members/u1")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let calls = services.invoke_calls.lock().expect("lock");
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].0,
        CapabilityId::new(PROJECT_MEMBER_REMOVE_CAPABILITY_ID).expect("capability id")
    );
    assert_eq!(
        calls[0].1,
        serde_json::json!({ "project_id": "p1", "user_id": "u1" })
    );
}

/// Project listing is routed through the ProductSurface view boundary.
#[tokio::test]
async fn list_projects_queries_product_surface_view() {
    let services = Arc::new(StubServices::default());
    let app = router_with(services.clone());
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/webchat/v2/projects")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = read_json(response).await;
    assert_eq!(body["projects"][0]["project_id"], "project-alpha");
    let queries = services.view_queries.lock().expect("lock");
    assert_eq!(queries.len(), 1);
    assert_eq!(queries[0].view_id.as_str(), PROJECTS_VIEW.id);
}

#[tokio::test]
async fn remove_extension_uses_client_gesture_idempotency_not_permanent_input_deduplication() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    // Two distinct client gestures, then a response-lost retry of the second.
    // The live-repro defect: an input-derived activity id permanently
    // deduplicated every remove of one extension, replaying the first
    // remove's recorded success (reinstall → remove silently no-ops).
    for client_action_id in [
        "remove-gesture-one",
        "remove-gesture-two",
        "remove-gesture-two",
    ] {
        services.enqueue_invoke_response(Ok(successful_resolution(ActivityId::new())));
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/webchat/v2/extensions/google-calendar/remove")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        r#"{{"client_action_id":"{client_action_id}"}}"#,
                    )))
                    .expect("request"),
            )
            .await
            .expect("oneshot");
        assert_eq!(response.status(), StatusCode::OK);
    }

    let calls = services.invoke_calls.lock().expect("lock").clone();
    assert_eq!(calls.len(), 3);
    assert_ne!(
        calls[0].2, calls[1].2,
        "separate remove gestures must never replay one permanent cached lifecycle outcome"
    );
    assert_eq!(
        calls[1].2, calls[2].2,
        "the remove client action id must survive response-lost retries as the ProductSurface activity id"
    );
}
