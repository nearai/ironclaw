//! Caller-level contract tests for operator effective-config key routes.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};
use ironclaw_product_workflow::*;
use ironclaw_webui::webui_v2::{
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

fn service_unavailable_error() -> IronClawServicesError {
    IronClawServicesError {
        code: IronClawServicesErrorCode::Unavailable,
        kind: IronClawServicesErrorKind::ServiceUnavailable,
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
    async fn set_operator_config_key(
        &self,
        _caller: WebUiAuthenticatedCaller,
        key: String,
        request: IronClawOperatorConfigSetRequest,
    ) -> Result<IronClawOperatorConfigGetResponse, IronClawServicesError> {
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

#[async_trait]
impl ProductSurface for RecordingServices {
    async fn query(
        &self,
        _caller: WebUiAuthenticatedCaller,
        query: IronClawViewQuery,
    ) -> Result<IronClawViewPage, IronClawServicesError> {
        if query.view_id != OPERATOR_CONFIG_KEY_VIEW.id {
            unreachable!("not exercised by this test");
        }
        let key = query
            .params
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

    async fn execute_command(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: ProductOperationRequest,
    ) -> Result<ProductOperationResponse, IronClawServicesError> {
        let operation_id = ProductOperationId::parse(request.operation_id.as_str())
            .ok_or_else(service_unavailable_error)?;
        match operation_id {
            ProductOperationId::OperatorConfigSetKey => {
                let request: IronClawOperatorConfigSetProductRequest =
                    serde_json::from_value(request.input)
                        .map_err(IronClawServicesError::internal_from)?;
                ProductOperationResponse::json(
                    self.set_operator_config_key(
                        caller,
                        request.key,
                        IronClawOperatorConfigSetRequest {
                            value: request.value,
                        },
                    )
                    .await?,
                )
            }
            _ => Err(service_unavailable_error()),
        }
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
