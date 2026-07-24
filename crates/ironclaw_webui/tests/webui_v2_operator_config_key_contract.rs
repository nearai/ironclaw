//! Caller-level contract tests for operator effective-config key routes.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use ironclaw_host_api::{
    AgentId, ProductSurface, ProductSurfaceCaller, ProductSurfaceError, ProductSurfaceErrorCode,
    ProductSurfaceErrorKind, ProjectId, TenantId, UserId,
};
use ironclaw_product::*;
use ironclaw_webui::webui_v2::{
    DEFAULT_SSE_MAX_CONCURRENT_PER_CALLER, WebUiV2Capabilities, WebUiV2State, webui_v2_router,
};
use serde_json::{Value, json};
use tower::ServiceExt;

fn caller() -> ProductSurfaceCaller {
    ProductSurfaceCaller::new(
        TenantId::new("tenant-alpha").expect("tenant"),
        UserId::new("user-alpha").expect("user"),
        Some(AgentId::new("agent-alpha").expect("agent")),
        Some(ProjectId::new("project-alpha").expect("project")),
    )
}

fn service_unavailable_error() -> ProductSurfaceError {
    ProductSurfaceError {
        code: ProductSurfaceErrorCode::Unavailable,
        kind: ProductSurfaceErrorKind::ServiceUnavailable,
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

impl RecordingServices {
    fn record_set(&self, key: String, value: Value) {
        self.calls
            .lock()
            .expect("lock")
            .push(OperatorConfigCall::Set { key, value });
    }
}

#[async_trait]
impl ProductSurface for RecordingServices {
    async fn query(
        &self,
        _caller: ProductSurfaceCaller,
        request: ironclaw_host_api::ProductSurfaceQueryRequest,
    ) -> Result<ironclaw_host_api::ProductSurfaceQueryPage, ProductSurfaceError> {
        if request.view_id != OPERATOR_CONFIG_KEY_VIEW.id {
            unreachable!("not exercised by this test");
        }
        let key = request
            .input
            .get("key")
            .and_then(serde_json::Value::as_str)
            .expect("operator config key param")
            .to_string();
        self.calls
            .lock()
            .expect("lock")
            .push(OperatorConfigCall::Get { key });
        Err(service_unavailable_error())
    }

    async fn invoke(
        &self,
        _caller: ProductSurfaceCaller,
        request: ironclaw_host_api::ProductSurfaceInvokeRequest,
    ) -> Result<ironclaw_host_api::ProductSurfaceInvokeResponse, ProductSurfaceError> {
        if request.operation_id.as_str() != "operator.config.set_key" {
            return Err(service_unavailable_error());
        }
        let input: RebornOperatorConfigSetProductRequest =
            serde_json::from_value(request.input).map_err(ProductSurfaceError::internal_from)?;
        self.record_set(input.key, input.value);
        Err(service_unavailable_error())
    }

    async fn stream_events(
        &self,
        _caller: ProductSurfaceCaller,
        _request: ironclaw_host_api::ProductSurfaceStreamRequest,
    ) -> Result<ironclaw_host_api::ProductSurfaceStreamResponse, ProductSurfaceError> {
        Err(service_unavailable_error())
    }
}

fn router_with(services: Arc<dyn ProductSurface>) -> Router {
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
