//! Caller-level contract tests for operator effective-config key routes.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};
use ironclaw_product_workflow::*;
use ironclaw_webui_v2::{
    DEFAULT_SSE_MAX_CONCURRENT_PER_CALLER, WebUiV2Capabilities, WebUiV2State, webui_v2_router,
};
use serde_json::{Value, json};
use tower::ServiceExt;

fn caller() -> WebUiAuthenticatedCaller {
    WebUiAuthenticatedCaller::new(
        TenantId::new("tenant-alpha").expect("tenant"),
        UserId::new("user-alpha").expect("user"),
        Some(AgentId::new("agent-alpha").expect("agent")),
        Some(ProjectId::new("project-alpha").expect("project")),
    )
}

fn service_unavailable_error() -> RebornServicesError {
    RebornServicesError {
        code: RebornServicesErrorCode::Unavailable,
        kind: RebornServicesErrorKind::ServiceUnavailable,
        status_code: 503,
        retryable: false,
        field: None,
        validation_code: None,
    }
}

#[derive(Debug, Clone, PartialEq)]
enum OperatorConfigCall {
    Get { key: String },
    Set { key: String, value: Value },
}

#[derive(Default)]
struct RecordingServices {
    calls: Mutex<Vec<OperatorConfigCall>>,
}

#[async_trait]
impl RebornServicesApi for RecordingServices {
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
        unreachable!("not exercised by this test")
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

    async fn get_operator_config_key(
        &self,
        _caller: WebUiAuthenticatedCaller,
        key: String,
    ) -> Result<RebornOperatorConfigGetResponse, RebornServicesError> {
        self.calls
            .lock()
            .expect("lock")
            .push(OperatorConfigCall::Get { key });
        Err(service_unavailable_error())
    }

    async fn set_operator_config_key(
        &self,
        _caller: WebUiAuthenticatedCaller,
        key: String,
        request: RebornOperatorConfigSetRequest,
    ) -> Result<RebornOperatorConfigGetResponse, RebornServicesError> {
        self.calls
            .lock()
            .expect("lock")
            .push(OperatorConfigCall::Set {
                key,
                value: request.value,
            });
        Err(service_unavailable_error())
    }
}

fn router_with(services: Arc<dyn RebornServicesApi>) -> Router {
    webui_v2_router(WebUiV2State::new(
        services,
        DEFAULT_SSE_MAX_CONCURRENT_PER_CALLER,
    ))
    .layer(axum::Extension(caller()))
    .layer(axum::Extension(WebUiV2Capabilities {
        operator_webui_config: true,
    }))
}

#[tokio::test]
async fn operator_config_key_routes_dispatch_path_and_body() {
    let services = Arc::new(RecordingServices::default());
    let router = router_with(services.clone());

    let get_response = router
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
    assert_eq!(get_response.status(), StatusCode::SERVICE_UNAVAILABLE);

    let set_value = json!({"model": "gpt-4", "temperature": 0.2});
    let set_response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/operator/config/provider.default")
                .header("content-type", "application/json")
                .body(Body::from(json!({"value": set_value.clone()}).to_string()))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(set_response.status(), StatusCode::SERVICE_UNAVAILABLE);

    assert_eq!(
        services.calls.lock().expect("lock").as_slice(),
        &[
            OperatorConfigCall::Get {
                key: "provider.default".to_string(),
            },
            OperatorConfigCall::Set {
                key: "provider.default".to_string(),
                value: set_value,
            },
        ]
    );
}

#[tokio::test]
async fn operator_config_key_rejects_invalid_key_and_missing_body_before_facade() {
    let services = Arc::new(RecordingServices::default());
    let router = router_with(services.clone());

    let invalid_key_response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/operator/config/Provider.Default")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(invalid_key_response.status(), StatusCode::BAD_REQUEST);

    let missing_body_response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/operator/config/provider.default")
                .header("content-type", "application/json")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert!(matches!(
        missing_body_response.status(),
        StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY
    ));

    assert!(services.calls.lock().expect("lock").is_empty());
}
