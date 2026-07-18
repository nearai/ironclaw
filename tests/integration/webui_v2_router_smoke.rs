//! `ironclaw_webui` on the int-tier coverage lane (enabler (a)).
//!
//! First scenario crossing an enumerated `--test` binary into the WebChat v2
//! route surface: the crate's own 5,801-line contract suite
//! (`crates/ironclaw_webui/tests/webui_v2_handlers_contract.rs`) never
//! runs under the coverage-lane invocation, which passes only the root-tree
//! suite names.
//!
//! Drives the BARE crate router (`webui_v2_router()` over a minimal fake
//! `RebornServicesApi`), not composition's `webui_v2_app` wrapper — the
//! wrapper needs the heavier `build_reborn_runtime` tier (named follow-on).
//! Composition deliberately does not re-export the bare router/state
//! (facade-only rule), so this suite carries the root DEV-dependency on
//! `ironclaw_webui` itself — production binaries are unaffected.
//!
//! `MinimalWebuiServices` duplicates the role of the contract suite's
//! `StubServices`; extraction of a shared in-crate `test_support` module was
//! reviewed and deferred (production-crate touch outside this batch's
//! budget). The 24 required methods the scenario never calls return the
//! shared rejecting error (`rejecting_reborn_services_error`, the public
//! fakes helper — `RebornServicesError::service_unavailable` is
//! `pub(super)`) so an unexpected dispatch fails loudly.
//!
//! Flat suite, no harness mounts: this models an HTTP wire contract, not an
//! agent turn.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode};
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_product_workflow::{
    LifecyclePackageRef, RebornCancelRunResponse, RebornCreateThreadResponse,
    RebornDeleteThreadRequest, RebornDeleteThreadResponse, RebornExtensionActionResponse,
    RebornExtensionListResponse, RebornExtensionRegistryResponse, RebornGetRunStateRequest,
    RebornGetRunStateResponse, RebornListAutomationsResponse, RebornListThreadsResponse,
    RebornOutboundDeliveryTargetListResponse, RebornOutboundPreferencesResponse,
    RebornResolveGateResponse, RebornRetryRunResponse, RebornServicesApi, RebornServicesError,
    RebornSetOutboundPreferencesRequest, RebornSetupExtensionResponse, RebornSkillActionResponse,
    RebornSkillContentResponse, RebornSkillListResponse, RebornSkillSearchResponse,
    RebornStreamEventsRequest, RebornStreamEventsResponse, RebornSubmitTurnResponse,
    RebornTimelineRequest, RebornTimelineResponse, WebUiAuthenticatedCaller, WebUiCancelRunRequest,
    WebUiCreateThreadRequest, WebUiListAutomationsRequest, WebUiListThreadsRequest,
    WebUiResolveGateRequest, WebUiRetryRunRequest, WebUiSendMessageRequest,
    WebUiSetupExtensionRequest, rejecting_reborn_services_error,
};
use ironclaw_threads::{SessionThreadRecord, ThreadScope};
use ironclaw_webui::webui_v2::{
    DEFAULT_SSE_MAX_CONCURRENT_PER_CALLER, WebUiV2Capabilities, WebUiV2State, webui_v2_router,
};
use serde_json::Value;
use tower::ServiceExt;

/// Minimal `RebornServicesApi` fake: only `create_thread` is real (records
/// the request, returns a canned thread); the other 24 required methods
/// reject with the shared error helper.
#[derive(Default)]
struct MinimalWebuiServices {
    create_thread_calls: Mutex<Vec<WebUiCreateThreadRequest>>,
}

#[async_trait]
impl RebornServicesApi for MinimalWebuiServices {
    async fn create_thread(
        &self,
        _caller: WebUiAuthenticatedCaller,
        request: WebUiCreateThreadRequest,
    ) -> Result<RebornCreateThreadResponse, RebornServicesError> {
        self.create_thread_calls.lock().expect("lock").push(request);
        Ok(RebornCreateThreadResponse {
            thread: SessionThreadRecord {
                thread_id: ThreadId::new("thread:webui-v2-smoke").expect("thread id"),
                scope: ThreadScope {
                    tenant_id: TenantId::new("tenant-smoke").expect("tenant"),
                    agent_id: AgentId::new("agent-smoke").expect("agent"),
                    project_id: None,
                    owner_user_id: Some(UserId::new("user-smoke").expect("user")),
                    mission_id: None,
                },
                created_by_actor_id: "user-smoke".to_string(),
                title: None,
                metadata_json: None,
                goal: None,
                created_at: None,
                updated_at: None,
            },
        })
    }

    async fn submit_turn(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: WebUiSendMessageRequest,
    ) -> Result<RebornSubmitTurnResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
    }

    async fn delete_thread(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: RebornDeleteThreadRequest,
    ) -> Result<RebornDeleteThreadResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
    }

    async fn get_timeline(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: RebornTimelineRequest,
    ) -> Result<RebornTimelineResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
    }

    async fn stream_events(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: RebornStreamEventsRequest,
    ) -> Result<RebornStreamEventsResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
    }

    async fn cancel_run(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: WebUiCancelRunRequest,
    ) -> Result<RebornCancelRunResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
    }

    async fn resolve_gate(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: WebUiResolveGateRequest,
    ) -> Result<RebornResolveGateResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
    }

    async fn retry_run(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: WebUiRetryRunRequest,
    ) -> Result<RebornRetryRunResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
    }

    async fn get_run_state(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: RebornGetRunStateRequest,
    ) -> Result<RebornGetRunStateResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
    }

    async fn list_threads(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: WebUiListThreadsRequest,
    ) -> Result<RebornListThreadsResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
    }

    async fn list_automations(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: WebUiListAutomationsRequest,
    ) -> Result<RebornListAutomationsResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
    }

    async fn get_outbound_preferences(
        &self,
        _caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornOutboundPreferencesResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
    }

    async fn set_outbound_preferences(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: RebornSetOutboundPreferencesRequest,
    ) -> Result<RebornOutboundPreferencesResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
    }

    async fn list_outbound_delivery_targets(
        &self,
        _caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornOutboundDeliveryTargetListResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
    }

    async fn list_extensions(
        &self,
        _caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornExtensionListResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
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

    async fn list_extension_registry(
        &self,
        _caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornExtensionRegistryResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
    }

    async fn install_extension(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _package_ref: LifecyclePackageRef,
    ) -> Result<RebornExtensionActionResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
    }

    async fn activate_extension(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _package_ref: LifecyclePackageRef,
    ) -> Result<RebornExtensionActionResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
    }

    async fn remove_extension(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _package_ref: LifecyclePackageRef,
    ) -> Result<RebornExtensionActionResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
    }

    async fn setup_extension(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _package_ref: LifecyclePackageRef,
        _request: WebUiSetupExtensionRequest,
    ) -> Result<RebornSetupExtensionResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
    }
}

/// Router exactly as the crate's contract suite builds it: real
/// `webui_v2_router`, auth bypassed by injecting the authenticated-caller
/// `Extension` directly (production composition's bearer middleware
/// constructs it).
fn smoke_router(services: Arc<MinimalWebuiServices>) -> Router {
    webui_v2_router(WebUiV2State::new(
        services,
        DEFAULT_SSE_MAX_CONCURRENT_PER_CALLER,
    ))
    .layer(axum::Extension(WebUiAuthenticatedCaller::new(
        TenantId::new("tenant-smoke").expect("tenant"),
        UserId::new("user-smoke").expect("user"),
        Some(AgentId::new("agent-smoke").expect("agent")),
        Some(ProjectId::new("project-smoke").expect("project")),
    )))
    .layer(axum::Extension(WebUiV2Capabilities::default()))
}

#[tokio::test]
async fn create_thread_round_trips_through_router() {
    let services = Arc::new(MinimalWebuiServices::default());
    let router = smoke_router(Arc::clone(&services));

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/threads")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"client_action_id":"smoke-act-1"}"#))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let body: Value = serde_json::from_slice(&bytes).expect("json body");
    assert!(
        body["thread"]["thread_id"].is_string(),
        "response carries the created thread id: {body}"
    );
    assert_eq!(
        services.create_thread_calls.lock().expect("lock").len(),
        1,
        "facade create_thread dispatched exactly once"
    );
}
