//! Caller-level contract tests for the WebChat v2 axum handlers.
//!
//! Per `.claude/rules/testing.md` "Test Through the Caller", these tests
//! drive a real axum [`Router`] (built from [`webui_v2_router`]) against a
//! stub [`RebornServicesApi`] so the regression target is the wire
//! contract — body shape, path/query plumbing, error mapping — not just
//! the facade method bodies that are already covered in
//! `ironclaw_product_workflow`.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode};
use http_body_util::BodyExt;
use ironclaw_host_api::{
    AgentId, CapabilityId, ExtensionId, InvocationId, ProjectId, RuntimeKind, TenantId, ThreadId,
    UserId,
};
use ironclaw_product_adapters::{
    AdapterInstallationId, CapabilityActivityStatusView, CapabilityActivityView,
    ExternalConversationRef, FinalReplyView, ProductAdapterId, ProductOutboundEnvelope,
    ProductOutboundPayload, ProductOutboundTarget, ProductProjectionItem, ProductProjectionState,
    ProgressKind, ProgressUpdateView, ProjectionCursor,
};
use ironclaw_product_workflow::{
    FsMount, LifecyclePackageRef, LifecyclePhase, LlmActiveSelection, LlmConfigSnapshot,
    LlmModelsResult, LlmProbeRequest, LlmProbeResult, LlmProviderView, ProjectFsEntry,
    ProjectFsEntryKind, ProjectFsFile, ProjectFsStat, RebornAttachmentBytes,
    RebornAttachmentRequest, RebornAutomationInfo, RebornAutomationRecentRunInfo,
    RebornAutomationRecentRunStatus, RebornAutomationSource, RebornAutomationState,
    RebornCancelRunResponse, RebornChannelConnectAction, RebornChannelConnectStrategy,
    RebornConnectableChannelInfo, RebornConnectableChannelListResponse, RebornCreateThreadResponse,
    RebornDeleteThreadRequest, RebornDeleteThreadResponse, RebornExtensionActionResponse,
    RebornExtensionListResponse, RebornExtensionRegistryResponse, RebornFsListRequest,
    RebornFsListResponse, RebornFsMountInfo, RebornFsMountsResponse, RebornFsReadRequest,
    RebornFsStatRequest, RebornFsStatResponse, RebornGetRunStateRequest, RebornGetRunStateResponse,
    RebornListAutomationsResponse, RebornListThreadsResponse, RebornOperatorArea,
    RebornOperatorCommandPlaneResponse, RebornOperatorConfigDiagnostic,
    RebornOperatorConfigDiagnosticSeverity, RebornOperatorConfigEntry,
    RebornOperatorConfigGetResponse, RebornOperatorConfigListResponse,
    RebornOperatorConfigSetRequest, RebornOperatorConfigValidateRequest,
    RebornOperatorConfigValidateResponse, RebornOperatorLogsQuery,
    RebornOperatorServiceLifecycleAction, RebornOperatorServiceLifecycleRequest,
    RebornOperatorSetupRequest, RebornOperatorSetupResponse, RebornOperatorSetupStatus,
    RebornOperatorSetupStep, RebornOperatorSetupStepStatus, RebornOperatorSurfaceStatus,
    RebornOutboundDeliveryTargetCapabilities, RebornOutboundDeliveryTargetId,
    RebornOutboundDeliveryTargetListResponse, RebornOutboundDeliveryTargetOption,
    RebornOutboundDeliveryTargetStatus, RebornOutboundDeliveryTargetSummary,
    RebornOutboundPreferencesResponse, RebornResolveGateResponse, RebornResumeGateResponse,
    RebornServicesApi, RebornServicesError, RebornServicesErrorCode, RebornServicesErrorKind,
    RebornSetOutboundPreferencesRequest, RebornSetupExtensionResponse, RebornSkillActionResponse,
    RebornSkillContentResponse, RebornSkillListResponse, RebornSkillSearchResponse,
    RebornStreamEventsRequest, RebornStreamEventsResponse, RebornSubmitTurnResponse,
    RebornTimelineRequest, RebornTimelineResponse, SetActiveLlmRequest, UpsertLlmProviderRequest,
    WebUiAuthenticatedCaller, WebUiCancelRunRequest, WebUiCreateThreadRequest,
    WebUiListAutomationsRequest, WebUiListThreadsRequest, WebUiResolveGateRequest,
    WebUiSendMessageRequest, WebUiSetupExtensionRequest, rejecting_reborn_services_error,
};
use ironclaw_threads::SessionThreadRecord;
use ironclaw_turns::{
    EventCursor, ReplyTargetBindingRef, RunProfileId, RunProfileVersion, TurnRunId, TurnStatus,
};
use ironclaw_webui_v2::{
    DEFAULT_SSE_MAX_CONCURRENT_PER_CALLER, WebUiV2Capabilities, WebUiV2State, webui_v2_router,
};
use serde_json::Value;
use tokio::sync::Notify;
use tower::ServiceExt;

fn caller() -> WebUiAuthenticatedCaller {
    WebUiAuthenticatedCaller::new(
        TenantId::new("tenant-alpha").expect("tenant"),
        UserId::new("user-alpha").expect("user"),
        Some(AgentId::new("agent-alpha").expect("agent")),
        Some(ProjectId::new("project-alpha").expect("project")),
    )
}

fn router_with(services: Arc<dyn RebornServicesApi>) -> Router {
    router_with_capabilities(services, WebUiV2Capabilities::default())
}

fn router_with_capabilities(
    services: Arc<dyn RebornServicesApi>,
    capabilities: WebUiV2Capabilities,
) -> Router {
    webui_v2_router(WebUiV2State::new(
        services,
        DEFAULT_SSE_MAX_CONCURRENT_PER_CALLER,
    ))
    // Production composition runs the bearer-token middleware that
    // constructs this `Extension`; tests bypass auth and inject the
    // caller directly so the regression target is the handler itself.
    .layer(axum::Extension(caller()))
    .layer(axum::Extension(capabilities))
}

fn service_unavailable_error(retryable: bool) -> RebornServicesError {
    RebornServicesError {
        code: RebornServicesErrorCode::Unavailable,
        kind: RebornServicesErrorKind::ServiceUnavailable,
        status_code: 503,
        retryable,
        field: None,
        validation_code: None,
    }
}

type OperatorSetupCall = (Option<String>, Option<String>, bool, bool);
type OperatorConfigSetCall = (String, Value);
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
    create_thread_calls: Mutex<Vec<WebUiCreateThreadRequest>>,
    delete_thread_calls: Mutex<Vec<RebornDeleteThreadRequest>>,
    submit_turn_calls: Mutex<Vec<WebUiSendMessageRequest>>,
    get_timeline_calls: Mutex<Vec<RebornTimelineRequest>>,
    read_attachment_calls: Mutex<Vec<RebornAttachmentRequest>>,
    read_attachment_response: Mutex<Option<RebornAttachmentBytes>>,
    stream_events_calls: Mutex<Vec<RebornStreamEventsRequest>>,
    cancel_run_calls: Mutex<Vec<WebUiCancelRunRequest>>,
    resolve_gate_calls: Mutex<Vec<WebUiResolveGateRequest>>,
    list_automations_calls: Mutex<Vec<WebUiListAutomationsRequest>>,
    next_list_automations_error: Mutex<Option<RebornServicesError>>,
    get_outbound_preferences_calls: Mutex<usize>,
    set_outbound_preferences_calls: Mutex<Vec<RebornSetOutboundPreferencesRequest>>,
    next_set_outbound_preferences_error: Mutex<Option<RebornServicesError>>,
    list_outbound_delivery_targets_calls: Mutex<usize>,
    list_connectable_channels_calls: Mutex<usize>,
    next_list_connectable_channels_error: Mutex<Option<RebornServicesError>>,
    get_operator_setup_calls: Mutex<usize>,
    run_operator_setup_calls: Mutex<Vec<OperatorSetupCall>>,
    list_operator_config_calls: Mutex<usize>,
    get_operator_config_key_calls: Mutex<Vec<String>>,
    set_operator_config_key_calls: Mutex<Vec<OperatorConfigSetCall>>,
    next_set_operator_config_key_error: Mutex<Option<RebornServicesError>>,
    validate_operator_config_calls: Mutex<Vec<Vec<String>>>,
    get_operator_diagnostics_calls: Mutex<usize>,
    get_operator_status_calls: Mutex<usize>,
    query_operator_logs_calls: Mutex<Vec<OperatorLogsCall>>,
    run_operator_service_lifecycle_calls: Mutex<Vec<RebornOperatorServiceLifecycleAction>>,
    list_extensions_calls: Mutex<usize>,
    list_extension_registry_calls: Mutex<usize>,
    install_extension_calls: Mutex<Vec<String>>,
    activate_extension_calls: Mutex<Vec<String>>,
    remove_extension_calls: Mutex<Vec<String>>,
    get_llm_config_calls: Mutex<usize>,
    upsert_llm_provider_calls: Mutex<Vec<String>>,
    delete_llm_provider_calls: Mutex<Vec<String>>,
    set_active_llm_calls: Mutex<Vec<(String, Option<String>)>>,
    test_llm_connection_calls: Mutex<Vec<String>>,
    list_llm_models_calls: Mutex<Vec<String>>,
    next_create_thread_error: Mutex<Option<RebornServicesError>>,
    /// Per-call queued responses for `stream_events`. When non-empty, the
    /// front entry is popped and returned on each call so SSE tests can
    /// drive the handler through specific projection envelopes, error
    /// branches, or empty drains in a deterministic order.
    next_stream_events: Mutex<VecDeque<Result<RebornStreamEventsResponse, RebornServicesError>>>,
    stream_events_notify: Arc<Notify>,
    /// Queued response for the next `submit_turn` call. When `Some`, the value
    /// is taken and returned instead of the default `Submitted` response.
    next_submit_response: Mutex<Option<RebornSubmitTurnResponse>>,
}

impl StubServices {
    fn fail_create_thread(&self, error: RebornServicesError) {
        *self.next_create_thread_error.lock().expect("lock") = Some(error);
    }

    /// Stage the bytes `read_attachment` should return. When unset, the stub
    /// inherits the trait default (404 not found).
    fn set_attachment(&self, bytes: RebornAttachmentBytes) {
        *self.read_attachment_response.lock().expect("lock") = Some(bytes);
    }

    fn fail_list_automations(&self, error: RebornServicesError) {
        *self.next_list_automations_error.lock().expect("lock") = Some(error);
    }

    fn fail_set_outbound_preferences(&self, error: RebornServicesError) {
        *self
            .next_set_outbound_preferences_error
            .lock()
            .expect("lock") = Some(error);
    }

    fn fail_list_connectable_channels(&self, error: RebornServicesError) {
        *self
            .next_list_connectable_channels_error
            .lock()
            .expect("lock") = Some(error);
    }

    fn fail_set_operator_config_key(&self, error: RebornServicesError) {
        *self
            .next_set_operator_config_key_error
            .lock()
            .expect("lock") = Some(error);
    }

    /// Queue one response for the next `stream_events` call. Tests use this
    /// to drive the SSE handler through programmable projection envelopes
    /// or error branches. Falls back to an empty `Ok` drain when the queue
    /// is empty.
    fn enqueue_stream_events(
        &self,
        response: Result<RebornStreamEventsResponse, RebornServicesError>,
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

#[async_trait]
impl RebornServicesApi for StubServices {
    async fn create_thread(
        &self,
        _caller: WebUiAuthenticatedCaller,
        request: WebUiCreateThreadRequest,
    ) -> Result<RebornCreateThreadResponse, RebornServicesError> {
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
        _caller: WebUiAuthenticatedCaller,
        request: WebUiSendMessageRequest,
    ) -> Result<RebornSubmitTurnResponse, RebornServicesError> {
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

    async fn delete_thread(
        &self,
        _caller: WebUiAuthenticatedCaller,
        request: RebornDeleteThreadRequest,
    ) -> Result<RebornDeleteThreadResponse, RebornServicesError> {
        self.delete_thread_calls
            .lock()
            .expect("lock")
            .push(request.clone());
        Ok(RebornDeleteThreadResponse {
            thread_id: ThreadId::new(request.thread_id).expect("thread id"),
            deleted: true,
        })
    }

    async fn get_timeline(
        &self,
        _caller: WebUiAuthenticatedCaller,
        request: RebornTimelineRequest,
    ) -> Result<RebornTimelineResponse, RebornServicesError> {
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

    async fn list_fs_mounts(
        &self,
        _caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornFsMountsResponse, RebornServicesError> {
        Ok(RebornFsMountsResponse {
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
    }

    async fn browse_fs_dir(
        &self,
        _caller: WebUiAuthenticatedCaller,
        request: RebornFsListRequest,
    ) -> Result<RebornFsListResponse, RebornServicesError> {
        Ok(RebornFsListResponse {
            mount: request.mount,
            path: request.path,
            entries: vec![ProjectFsEntry {
                name: "today.md".to_string(),
                path: "daily/today.md".to_string(),
                kind: ProjectFsEntryKind::File,
            }],
        })
    }

    async fn stat_fs_path(
        &self,
        _caller: WebUiAuthenticatedCaller,
        request: RebornFsStatRequest,
    ) -> Result<RebornFsStatResponse, RebornServicesError> {
        Ok(RebornFsStatResponse {
            stat: ProjectFsStat {
                path: request.path,
                kind: ProjectFsEntryKind::File,
                size_bytes: 7,
                mime_type: "text/markdown".to_string(),
            },
        })
    }

    async fn read_fs_file(
        &self,
        _caller: WebUiAuthenticatedCaller,
        request: RebornFsReadRequest,
    ) -> Result<ProjectFsFile, RebornServicesError> {
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
        _caller: WebUiAuthenticatedCaller,
        request: RebornAttachmentRequest,
    ) -> Result<RebornAttachmentBytes, RebornServicesError> {
        self.read_attachment_calls
            .lock()
            .expect("lock")
            .push(request);
        match self.read_attachment_response.lock().expect("lock").clone() {
            Some(bytes) => Ok(bytes),
            None => Err(RebornServicesError {
                code: RebornServicesErrorCode::NotFound,
                kind: RebornServicesErrorKind::NotFound,
                status_code: 404,
                retryable: false,
                field: None,
                validation_code: None,
            }),
        }
    }

    async fn stream_events(
        &self,
        _caller: WebUiAuthenticatedCaller,
        request: RebornStreamEventsRequest,
    ) -> Result<RebornStreamEventsResponse, RebornServicesError> {
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

    async fn get_run_state(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: RebornGetRunStateRequest,
    ) -> Result<RebornGetRunStateResponse, RebornServicesError> {
        // Not exercised by any current handler test — `get_run_state` is on
        // the facade trait but not wired to a WebChat v2 HTTP route. Fail
        // loud rather than fabricate a response so a future caller-level
        // test that forgets to program this path can't quietly pass.
        Err(RebornServicesError {
            code: RebornServicesErrorCode::Internal,
            kind: RebornServicesErrorKind::Internal,
            status_code: 500,
            retryable: false,
            field: None,
            validation_code: None,
        })
    }

    async fn cancel_run(
        &self,
        _caller: WebUiAuthenticatedCaller,
        request: WebUiCancelRunRequest,
    ) -> Result<RebornCancelRunResponse, RebornServicesError> {
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
        _caller: WebUiAuthenticatedCaller,
        request: WebUiResolveGateRequest,
    ) -> Result<RebornResolveGateResponse, RebornServicesError> {
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

    async fn list_threads(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: WebUiListThreadsRequest,
    ) -> Result<RebornListThreadsResponse, RebornServicesError> {
        Ok(RebornListThreadsResponse {
            threads: Vec::new(),
            next_cursor: None,
        })
    }

    async fn list_automations(
        &self,
        _caller: WebUiAuthenticatedCaller,
        request: WebUiListAutomationsRequest,
    ) -> Result<RebornListAutomationsResponse, RebornServicesError> {
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
        Ok(RebornListAutomationsResponse {
            automations: vec![automation_info(
                "automation-listed",
                "Daily status",
                "0 9 * * *",
            )],
            scheduler_enabled: true,
        })
    }

    async fn list_extensions(
        &self,
        _caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornExtensionListResponse, RebornServicesError> {
        *self.list_extensions_calls.lock().expect("lock") += 1;
        Ok(RebornExtensionListResponse {
            extensions: Vec::new(),
        })
    }

    async fn list_skills(
        &self,
        _caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornSkillListResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
    }

    async fn search_skills(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _query: String,
    ) -> Result<RebornSkillSearchResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
    }

    async fn install_skill(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _name: String,
        _content: Option<String>,
    ) -> Result<RebornSkillActionResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
    }

    async fn read_skill_content(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _name: String,
    ) -> Result<RebornSkillContentResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
    }

    async fn update_skill(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _name: String,
        _content: String,
    ) -> Result<RebornSkillActionResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
    }

    async fn remove_skill(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _name: String,
    ) -> Result<RebornSkillActionResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
    }

    async fn list_connectable_channels(
        &self,
        _caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornConnectableChannelListResponse, RebornServicesError> {
        *self.list_connectable_channels_calls.lock().expect("lock") += 1;
        if let Some(error) = self
            .next_list_connectable_channels_error
            .lock()
            .expect("lock")
            .take()
        {
            return Err(error);
        }
        Ok(RebornConnectableChannelListResponse {
            channels: vec![RebornConnectableChannelInfo {
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
                command_aliases: vec!["slack".to_string()],
            }],
        })
    }

    async fn get_operator_setup(
        &self,
        _caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornOperatorSetupResponse, RebornServicesError> {
        *self.get_operator_setup_calls.lock().expect("lock") += 1;
        Ok(RebornOperatorSetupResponse {
            area: RebornOperatorArea::Setup,
            status: RebornOperatorSetupStatus::Incomplete,
            message: "setup incomplete".to_string(),
            active_provider_id: None,
            active_model: None,
            steps: Vec::new(),
            diagnostics: Vec::new(),
        })
    }

    async fn run_operator_setup(
        &self,
        _caller: WebUiAuthenticatedCaller,
        request: RebornOperatorSetupRequest,
    ) -> Result<RebornOperatorSetupResponse, RebornServicesError> {
        self.run_operator_setup_calls.lock().expect("lock").push((
            request.provider_id.clone(),
            request.model.clone(),
            request.api_key.is_some(),
            request.webui_access_token.is_some(),
        ));
        Ok(RebornOperatorSetupResponse {
            area: RebornOperatorArea::Setup,
            status: RebornOperatorSetupStatus::Incomplete,
            message: "provider setup accepted".to_string(),
            active_provider_id: request.provider_id,
            active_model: request.model,
            steps: vec![
                RebornOperatorSetupStep {
                    name: "provider".to_string(),
                    status: RebornOperatorSetupStepStatus::Complete,
                    message: "provider accepted".to_string(),
                },
                RebornOperatorSetupStep {
                    name: "profile".to_string(),
                    status: RebornOperatorSetupStepStatus::Unsupported,
                    message: "profile setup is not wired".to_string(),
                },
            ],
            diagnostics: Vec::new(),
        })
    }

    async fn list_operator_config(
        &self,
        _caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornOperatorConfigListResponse, RebornServicesError> {
        *self.list_operator_config_calls.lock().expect("lock") += 1;
        Ok(RebornOperatorConfigListResponse {
            entries: Vec::new(),
            precedence: Vec::new(),
            diagnostics: Vec::new(),
        })
    }

    async fn get_operator_config_key(
        &self,
        _caller: WebUiAuthenticatedCaller,
        key: String,
    ) -> Result<RebornOperatorConfigGetResponse, RebornServicesError> {
        self.get_operator_config_key_calls
            .lock()
            .expect("lock")
            .push(key.clone());
        Ok(RebornOperatorConfigGetResponse {
            entry: operator_config_entry(key, serde_json::json!("configured")),
        })
    }

    async fn set_operator_config_key(
        &self,
        _caller: WebUiAuthenticatedCaller,
        key: String,
        request: RebornOperatorConfigSetRequest,
    ) -> Result<RebornOperatorConfigGetResponse, RebornServicesError> {
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

    async fn validate_operator_config(
        &self,
        _caller: WebUiAuthenticatedCaller,
        request: RebornOperatorConfigValidateRequest,
    ) -> Result<RebornOperatorConfigValidateResponse, RebornServicesError> {
        self.validate_operator_config_calls
            .lock()
            .expect("lock")
            .push(request.keys.clone());
        let diagnostics = operator_config_validation_diagnostics(request.keys);
        Ok(RebornOperatorConfigValidateResponse {
            valid: diagnostics.is_empty(),
            diagnostics,
        })
    }

    async fn get_operator_diagnostics(
        &self,
        _caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornOperatorCommandPlaneResponse, RebornServicesError> {
        *self.get_operator_diagnostics_calls.lock().expect("lock") += 1;
        Ok(operator_config_diagnostic_command_plane_response(
            RebornOperatorArea::Diagnostics,
        ))
    }

    async fn get_operator_status(
        &self,
        _caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornOperatorCommandPlaneResponse, RebornServicesError> {
        *self.get_operator_status_calls.lock().expect("lock") += 1;
        Ok(operator_config_diagnostic_command_plane_response(
            RebornOperatorArea::Status,
        ))
    }

    async fn query_operator_logs(
        &self,
        _caller: WebUiAuthenticatedCaller,
        query: RebornOperatorLogsQuery,
    ) -> Result<RebornOperatorCommandPlaneResponse, RebornServicesError> {
        self.query_operator_logs_calls
            .lock()
            .expect("lock")
            .push(query);
        Ok(operator_command_response(RebornOperatorArea::Logs))
    }

    async fn run_operator_service_lifecycle(
        &self,
        _caller: WebUiAuthenticatedCaller,
        request: RebornOperatorServiceLifecycleRequest,
    ) -> Result<RebornOperatorCommandPlaneResponse, RebornServicesError> {
        self.run_operator_service_lifecycle_calls
            .lock()
            .expect("lock")
            .push(request.action);
        Ok(operator_command_response(
            RebornOperatorArea::ServiceLifecycle,
        ))
    }

    async fn get_outbound_preferences(
        &self,
        _caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornOutboundPreferencesResponse, RebornServicesError> {
        *self.get_outbound_preferences_calls.lock().expect("lock") += 1;
        Ok(outbound_preferences_response("slack-dm-alpha"))
    }

    async fn set_outbound_preferences(
        &self,
        _caller: WebUiAuthenticatedCaller,
        request: RebornSetOutboundPreferencesRequest,
    ) -> Result<RebornOutboundPreferencesResponse, RebornServicesError> {
        if let Some(err) = self
            .next_set_outbound_preferences_error
            .lock()
            .expect("lock")
            .take()
        {
            return Err(err);
        }
        let target_id = request
            .final_reply_target_id
            .as_ref()
            .map(|id| id.as_str().to_string());
        self.set_outbound_preferences_calls
            .lock()
            .expect("lock")
            .push(request);
        Ok(match target_id {
            Some(id) => outbound_preferences_response(&id),
            None => RebornOutboundPreferencesResponse {
                final_reply_target: None,
                final_reply_target_status: RebornOutboundDeliveryTargetStatus::NoneConfigured,
                default_modality: Default::default(),
            },
        })
    }

    async fn list_outbound_delivery_targets(
        &self,
        _caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornOutboundDeliveryTargetListResponse, RebornServicesError> {
        *self
            .list_outbound_delivery_targets_calls
            .lock()
            .expect("lock") += 1;
        Ok(RebornOutboundDeliveryTargetListResponse {
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
        })
    }

    async fn list_extension_registry(
        &self,
        _caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornExtensionRegistryResponse, RebornServicesError> {
        *self.list_extension_registry_calls.lock().expect("lock") += 1;
        Ok(RebornExtensionRegistryResponse {
            entries: Vec::new(),
        })
    }

    async fn install_extension(
        &self,
        _caller: WebUiAuthenticatedCaller,
        package_ref: LifecyclePackageRef,
    ) -> Result<RebornExtensionActionResponse, RebornServicesError> {
        self.install_extension_calls
            .lock()
            .expect("lock")
            .push(package_ref.id.as_str().to_string());
        Ok(extension_action_response("installed"))
    }

    async fn activate_extension(
        &self,
        _caller: WebUiAuthenticatedCaller,
        package_ref: LifecyclePackageRef,
    ) -> Result<RebornExtensionActionResponse, RebornServicesError> {
        self.activate_extension_calls
            .lock()
            .expect("lock")
            .push(package_ref.id.as_str().to_string());
        Ok(extension_action_response("activated"))
    }

    async fn remove_extension(
        &self,
        _caller: WebUiAuthenticatedCaller,
        package_ref: LifecyclePackageRef,
    ) -> Result<RebornExtensionActionResponse, RebornServicesError> {
        self.remove_extension_calls
            .lock()
            .expect("lock")
            .push(package_ref.id.as_str().to_string());
        Ok(extension_action_response("removed"))
    }

    async fn setup_extension(
        &self,
        _caller: WebUiAuthenticatedCaller,
        package_ref: LifecyclePackageRef,
        _request: WebUiSetupExtensionRequest,
    ) -> Result<RebornSetupExtensionResponse, RebornServicesError> {
        Ok(RebornSetupExtensionResponse {
            package_ref,
            phase: LifecyclePhase::UnsupportedOrLegacy,
            blockers: Vec::new(),
            payload: None,
            secrets: Vec::new(),
            fields: Vec::new(),
            onboarding: None,
        })
    }

    async fn get_llm_config(
        &self,
        _caller: WebUiAuthenticatedCaller,
    ) -> Result<LlmConfigSnapshot, RebornServicesError> {
        *self.get_llm_config_calls.lock().expect("lock") += 1;
        Ok(llm_snapshot("openai"))
    }

    async fn upsert_llm_provider(
        &self,
        _caller: WebUiAuthenticatedCaller,
        request: UpsertLlmProviderRequest,
    ) -> Result<LlmConfigSnapshot, RebornServicesError> {
        self.upsert_llm_provider_calls
            .lock()
            .expect("lock")
            .push(request.id.clone());
        Ok(llm_snapshot(&request.id))
    }

    async fn delete_llm_provider(
        &self,
        _caller: WebUiAuthenticatedCaller,
        provider_id: String,
    ) -> Result<LlmConfigSnapshot, RebornServicesError> {
        self.delete_llm_provider_calls
            .lock()
            .expect("lock")
            .push(provider_id);
        Ok(llm_snapshot("openai"))
    }

    async fn set_active_llm(
        &self,
        _caller: WebUiAuthenticatedCaller,
        request: SetActiveLlmRequest,
    ) -> Result<LlmConfigSnapshot, RebornServicesError> {
        self.set_active_llm_calls
            .lock()
            .expect("lock")
            .push((request.provider_id.clone(), request.model.clone()));
        Ok(llm_snapshot(&request.provider_id))
    }

    async fn test_llm_connection(
        &self,
        _caller: WebUiAuthenticatedCaller,
        request: LlmProbeRequest,
    ) -> Result<LlmProbeResult, RebornServicesError> {
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
        _caller: WebUiAuthenticatedCaller,
        request: LlmProbeRequest,
    ) -> Result<LlmModelsResult, RebornServicesError> {
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

fn extension_action_response(message: &str) -> RebornExtensionActionResponse {
    RebornExtensionActionResponse {
        success: true,
        message: message.to_string(),
        activated: None,
        auth_url: None,
        awaiting_token: None,
        instructions: None,
        onboarding_state: None,
        onboarding: None,
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
    let router = router_with(services.clone());

    let response = router
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
    let calls = services.delete_thread_calls.lock().expect("lock").clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].thread_id, "thread-delete");
}

#[tokio::test]
async fn send_message_path_overrides_body_thread_id() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    let response = router
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
async fn create_thread_error_maps_to_http_status() {
    let services = Arc::new(StubServices::default());
    services.fail_create_thread(RebornServicesError {
        code: RebornServicesErrorCode::Forbidden,
        kind: RebornServicesErrorKind::ParticipantDenied,
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
        ironclaw_product_workflow::ProjectionCursor::new("cursor-from-header").expect("cursor");
    let query_cursor =
        ironclaw_product_workflow::ProjectionCursor::new("cursor-from-query").expect("cursor");
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
async fn trace_credits_returns_caller_scoped_unenrolled_zero_state() {
    // The facade's default `trace_credits` body reads contributor-local
    // Trace Commons state scoped by the authenticated caller's user id.
    // A unique per-test user id guarantees a fresh scope so the
    // unenrolled zero-state is deterministic on any machine.
    let user_id = format!(
        "webui-v2-trace-credits-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    );
    let unique_caller = WebUiAuthenticatedCaller::new(
        TenantId::new("tenant-alpha").expect("tenant"),
        UserId::new(user_id.as_str()).expect("user"),
        None,
        None,
    );
    let router = webui_v2_router(WebUiV2State::new(
        Arc::new(StubServices::default()),
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
    services.fail_list_automations(RebornServicesError {
        code: RebornServicesErrorCode::Forbidden,
        kind: RebornServicesErrorKind::ParticipantDenied,
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
}

#[tokio::test]
async fn set_outbound_preferences_dispatches_body_through_facade() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/outbound/preferences")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"final_reply_target_id":"slack-dm-beta"}"#))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let body = read_json(response).await;
    assert_eq!(body["final_reply_target"]["target_id"], "slack-dm-beta");
    let calls = services
        .set_outbound_preferences_calls
        .lock()
        .expect("lock");
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0]
            .final_reply_target_id
            .as_ref()
            .map(|target_id| target_id.as_str()),
        Some("slack-dm-beta")
    );
}

#[tokio::test]
async fn set_outbound_preferences_accepts_explicit_clear() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/outbound/preferences")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"final_reply_target_id":null}"#))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let body = read_json(response).await;
    assert!(body.get("final_reply_target").is_none());
    assert_eq!(body["final_reply_target_status"], "none_configured");
    let calls = services
        .set_outbound_preferences_calls
        .lock()
        .expect("lock");
    assert_eq!(calls.len(), 1);
    assert!(calls[0].final_reply_target_id.is_none());
}

#[tokio::test]
async fn set_outbound_preferences_error_maps_to_http_status() {
    let services = Arc::new(StubServices::default());
    services.fail_set_outbound_preferences(RebornServicesError {
        code: RebornServicesErrorCode::NotFound,
        kind: RebornServicesErrorKind::NotFound,
        status_code: 404,
        retryable: false,
        field: None,
        validation_code: None,
    });
    let router = router_with(services);

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/outbound/preferences")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"final_reply_target_id":"target-does-not-exist"}"#,
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
async fn list_outbound_delivery_targets_dispatches_through_facade() {
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
}

#[tokio::test]
async fn list_connectable_channels_dispatches_through_facade() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/channels/connectable")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let body = read_json(response).await;
    assert_eq!(body["channels"][0]["channel"], "slack");
    assert_eq!(body["channels"][0]["strategy"], "inbound_proof_code");
    assert_eq!(
        body["channels"][0]["action"]["instructions"],
        "Message the Slack app, then enter the code here."
    );
    assert_eq!(
        *services
            .list_connectable_channels_calls
            .lock()
            .expect("lock"),
        1
    );
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
    let expected = ironclaw_product_workflow::webui_attachment_capabilities();
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
                .uri("/api/webchat/v2/operator/logs?limit=25&cursor=after-1&thread_id=thread-a&run_id=run-a&turn_id=turn-a&tool_call_id=tool-a&tool_name=shell&source=slack")
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

    assert_eq!(*services.get_operator_setup_calls.lock().expect("lock"), 1);
    assert_eq!(
        services
            .run_operator_setup_calls
            .lock()
            .expect("lock")
            .as_slice(),
        [(
            Some("openai".to_string()),
            Some("gpt-5-mini".to_string()),
            false,
            true
        )]
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
    assert_eq!(
        *services
            .get_operator_diagnostics_calls
            .lock()
            .expect("lock"),
        1
    );
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
async fn operator_routes_require_operator_capability() {
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

    assert_eq!(*services.get_operator_setup_calls.lock().expect("lock"), 0);
    assert!(
        services
            .run_operator_setup_calls
            .lock()
            .expect("lock")
            .is_empty()
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
async fn list_connectable_channels_error_maps_to_http_status() {
    let services = Arc::new(StubServices::default());
    services.fail_list_connectable_channels(RebornServicesError {
        code: RebornServicesErrorCode::Unavailable,
        kind: RebornServicesErrorKind::ServiceUnavailable,
        status_code: 503,
        retryable: true,
        field: None,
        validation_code: None,
    });
    let router = router_with(services);

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/channels/connectable")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = read_json(response).await;
    assert_eq!(body["error"], "unavailable");
    assert_eq!(body["kind"], "service_unavailable");
    assert_eq!(body["retryable"], true);
}

#[tokio::test]
async fn extension_list_and_registry_dispatch_through_facade() {
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

    assert_eq!(*services.list_extensions_calls.lock().expect("lock"), 1);
    assert_eq!(
        *services.list_extension_registry_calls.lock().expect("lock"),
        1
    );
}

#[tokio::test]
async fn install_extension_decodes_body_package_ref_to_facade_call() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/extensions/install")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"package_ref":{"kind":"extension","id":"nearai-mcp"}}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        services
            .install_extension_calls
            .lock()
            .expect("lock")
            .as_slice(),
        ["nearai-mcp"]
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
        services
            .install_extension_calls
            .lock()
            .expect("lock")
            .is_empty(),
        "invalid package kind must not reach the facade"
    );
}

#[tokio::test]
async fn activate_and_remove_extension_decode_path_package_id_to_facade_call() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    for uri in [
        "/api/webchat/v2/extensions/google-calendar/activate",
        "/api/webchat/v2/extensions/google-calendar/remove",
    ] {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri(uri)
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("oneshot");

        assert_eq!(response.status(), StatusCode::OK);
    }

    assert_eq!(
        services
            .activate_extension_calls
            .lock()
            .expect("lock")
            .as_slice(),
        ["google-calendar"]
    );
    assert_eq!(
        services
            .remove_extension_calls
            .lock()
            .expect("lock")
            .as_slice(),
        ["google-calendar"]
    );
}

#[tokio::test]
async fn get_extension_setup_dispatches_package_ref_to_facade() {
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
    assert_eq!(body["phase"], "unsupported_or_legacy");
}

// The path segment must become a lifecycle package ref at the
// handler/facade boundary. A well-formed package id reaches the facade
// and round-trips into the response.
#[tokio::test]
async fn setup_extension_dispatches_package_ref_to_facade() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/extensions/telegram/setup")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"action":"begin"}"#))
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
    assert_eq!(body["phase"], "unsupported_or_legacy");
    assert!(
        body.get("status").is_none(),
        "setup_extension must not expose legacy status aliases: {body}"
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
async fn llm_provider_routes_dispatch_to_facade_methods() {
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

    let upsert_response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/llm/providers")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"id":"acme","name":"Acme","adapter":"open_ai_completions","base_url":"https://api.acme.test/v1","default_model":"acme-1","api_key":"sk-test","set_active":true,"model":"acme-1"}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(upsert_response.status(), StatusCode::OK);

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

    assert_eq!(*services.get_llm_config_calls.lock().expect("lock"), 1);
    assert_eq!(
        services
            .upsert_llm_provider_calls
            .lock()
            .expect("lock")
            .as_slice(),
        ["acme"]
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
        services
            .upsert_llm_provider_calls
            .lock()
            .expect("lock")
            .is_empty()
    );
    assert!(
        services
            .delete_llm_provider_calls
            .lock()
            .expect("lock")
            .is_empty()
    );
    assert!(
        services
            .set_active_llm_calls
            .lock()
            .expect("lock")
            .is_empty()
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
    let services: Arc<dyn RebornServicesApi> = Arc::new(StubServices::default());
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
    let services: Arc<dyn RebornServicesApi> = Arc::new(StubServices::default());
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
    /// Facade whose `stream_events` never returns; all other methods are
    /// unreachable for this regression.
    #[derive(Default)]
    struct StallingServices;

    #[async_trait]
    impl RebornServicesApi for StallingServices {
        async fn create_thread(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _request: WebUiCreateThreadRequest,
        ) -> Result<RebornCreateThreadResponse, RebornServicesError> {
            unreachable!("not exercised by this test")
        }
        async fn submit_turn(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _request: WebUiSendMessageRequest,
        ) -> Result<RebornSubmitTurnResponse, RebornServicesError> {
            unreachable!("not exercised by this test")
        }
        async fn delete_thread(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _request: RebornDeleteThreadRequest,
        ) -> Result<RebornDeleteThreadResponse, RebornServicesError> {
            unreachable!("not exercised by this test")
        }
        async fn get_timeline(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _request: RebornTimelineRequest,
        ) -> Result<RebornTimelineResponse, RebornServicesError> {
            unreachable!("not exercised by this test")
        }
        async fn stream_events(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _request: RebornStreamEventsRequest,
        ) -> Result<RebornStreamEventsResponse, RebornServicesError> {
            // Never resolves — simulates a wedged projection stream.
            std::future::pending().await
        }
        async fn cancel_run(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _request: WebUiCancelRunRequest,
        ) -> Result<RebornCancelRunResponse, RebornServicesError> {
            unreachable!("not exercised by this test")
        }
        async fn resolve_gate(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _request: WebUiResolveGateRequest,
        ) -> Result<RebornResolveGateResponse, RebornServicesError> {
            unreachable!("not exercised by this test")
        }
        async fn get_run_state(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _request: RebornGetRunStateRequest,
        ) -> Result<RebornGetRunStateResponse, RebornServicesError> {
            unreachable!("not exercised by this test")
        }
        async fn list_threads(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _request: WebUiListThreadsRequest,
        ) -> Result<RebornListThreadsResponse, RebornServicesError> {
            unreachable!("not exercised by this test")
        }
        async fn list_automations(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _request: WebUiListAutomationsRequest,
        ) -> Result<RebornListAutomationsResponse, RebornServicesError> {
            unreachable!("not exercised by this test")
        }
        async fn get_outbound_preferences(
            &self,
            _caller: WebUiAuthenticatedCaller,
        ) -> Result<RebornOutboundPreferencesResponse, RebornServicesError> {
            unreachable!("not exercised by this test")
        }
        async fn set_outbound_preferences(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _request: RebornSetOutboundPreferencesRequest,
        ) -> Result<RebornOutboundPreferencesResponse, RebornServicesError> {
            unreachable!("not exercised by this test")
        }
        async fn list_outbound_delivery_targets(
            &self,
            _caller: WebUiAuthenticatedCaller,
        ) -> Result<RebornOutboundDeliveryTargetListResponse, RebornServicesError> {
            unreachable!("not exercised by this test")
        }
        async fn list_extensions(
            &self,
            _caller: WebUiAuthenticatedCaller,
        ) -> Result<RebornExtensionListResponse, RebornServicesError> {
            unreachable!("not exercised by this test")
        }
        async fn list_skills(
            &self,
            _caller: WebUiAuthenticatedCaller,
        ) -> Result<RebornSkillListResponse, RebornServicesError> {
            unreachable!("not exercised by this test")
        }
        async fn search_skills(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _query: String,
        ) -> Result<RebornSkillSearchResponse, RebornServicesError> {
            unreachable!("not exercised by this test")
        }
        async fn install_skill(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _name: String,
            _content: Option<String>,
        ) -> Result<RebornSkillActionResponse, RebornServicesError> {
            unreachable!("not exercised by this test")
        }
        async fn read_skill_content(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _name: String,
        ) -> Result<RebornSkillContentResponse, RebornServicesError> {
            unreachable!("not exercised by this test")
        }
        async fn update_skill(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _name: String,
            _content: String,
        ) -> Result<RebornSkillActionResponse, RebornServicesError> {
            unreachable!("not exercised by this test")
        }
        async fn remove_skill(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _name: String,
        ) -> Result<RebornSkillActionResponse, RebornServicesError> {
            unreachable!("not exercised by this test")
        }
        async fn list_extension_registry(
            &self,
            _caller: WebUiAuthenticatedCaller,
        ) -> Result<RebornExtensionRegistryResponse, RebornServicesError> {
            unreachable!("not exercised by this test")
        }
        async fn install_extension(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _package_ref: LifecyclePackageRef,
        ) -> Result<RebornExtensionActionResponse, RebornServicesError> {
            unreachable!("not exercised by this test")
        }
        async fn activate_extension(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _package_ref: LifecyclePackageRef,
        ) -> Result<RebornExtensionActionResponse, RebornServicesError> {
            unreachable!("not exercised by this test")
        }
        async fn remove_extension(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _package_ref: LifecyclePackageRef,
        ) -> Result<RebornExtensionActionResponse, RebornServicesError> {
            unreachable!("not exercised by this test")
        }
        async fn setup_extension(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _package_ref: LifecyclePackageRef,
            _request: WebUiSetupExtensionRequest,
        ) -> Result<RebornSetupExtensionResponse, RebornServicesError> {
            unreachable!("not exercised by this test")
        }
    }

    // Cap of 1 so we can observe slot release directly: a second open
    // returns 429 while the first is held, and 200 once it's released.
    let services: Arc<dyn RebornServicesApi> = Arc::new(StallingServices);
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
/// contract under test (it lives in `ironclaw_product_adapters`); these
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
// (Medium). When `RebornServicesApi::stream_events` returns Err, the
// handler must emit one SSE `error` frame carrying only the redacted
// `error` code + `retryable` flag (no `field`, no internal `detail`),
// then close the stream — never propagate an HTTP error on a long-lived
// SSE connection because the browser would replay it as a hard
// reconnect failure.
#[tokio::test]
async fn stream_events_facade_error_emits_redacted_error_event_and_closes() {
    let services = Arc::new(StubServices::default());
    services.enqueue_stream_events(Err(RebornServicesError {
        code: RebornServicesErrorCode::Forbidden,
        kind: RebornServicesErrorKind::ParticipantDenied,
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
    // Read until we see an `error` event chunk, or the stream closes.
    let bytes = collect_sse_until(&mut body, Duration::from_secs(2), |buf| {
        buf.windows(b"event: error".len())
            .any(|w| w == b"event: error")
            && buf.windows(2).any(|w| w == b"\n\n")
    })
    .await;

    let events = parse_sse_events(&bytes);
    let error_event = events
        .iter()
        .find(|event| event.event.as_deref() == Some("error"))
        .unwrap_or_else(|| {
            panic!(
                "expected an SSE `error` event, got: {events:?}; raw: {}",
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
    let services: Arc<dyn RebornServicesApi> = Arc::new(StubServices::default());
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
    services.enqueue_stream_events(Err(RebornServicesError {
        code: RebornServicesErrorCode::Unavailable,
        kind: RebornServicesErrorKind::ServiceUnavailable,
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
    services.enqueue_stream_events(Err(RebornServicesError {
        code: RebornServicesErrorCode::Unavailable,
        kind: RebornServicesErrorKind::ServiceUnavailable,
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

    let services: Arc<dyn RebornServicesApi> = Arc::new(StubServices::default());
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

    assert_eq!(
        services
            .run_operator_setup_calls
            .lock()
            .expect("lock")
            .as_slice(),
        [(
            Some("openai".to_string()),
            Some("gpt-5-mini".to_string()),
            true,
            true,
        )]
    );
}

#[tokio::test]
async fn list_fs_mounts_returns_browsable_mounts() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services);

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
}

#[tokio::test]
async fn browse_fs_dir_lists_mount_relative_entries() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services);

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
    let router = router_with(services);

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
