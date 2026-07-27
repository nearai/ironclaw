//! `ironclaw_webui` on the int-tier coverage lane (enabler (a)).
//!
//! First scenario crossing an enumerated `--test` binary into the WebChat v2
//! route surface: the crate's own 5,801-line contract suite
//! (`crates/ironclaw_webui/tests/webui_v2_handlers_contract.rs`) never
//! runs under the coverage-lane invocation, which passes only the root-tree
//! suite names.
//!
//! Drives the BARE crate router (`webui_v2_router()` over a minimal fake
//! `ProductSurface`), not composition's `webui_v2_app` wrapper — the
//! wrapper needs the heavier `build_reborn_runtime` tier (named follow-on).
//! Composition deliberately does not re-export the bare router/state
//! (service-only rule), so this suite carries the root DEV-dependency on
//! `ironclaw_webui` itself — production binaries are unaffected.
//!
//! `MinimalWebuiServices` duplicates the role of the contract suite's
//! `StubServices`; extraction of a shared in-crate `test_support` module was
//! reviewed and deferred (production-crate touch outside this batch's
//! budget). Methods the scenario never calls return the shared rejecting
//! error (`rejecting_product_surface_error`, the public fakes helper —
//! `ProductSurfaceError::service_unavailable` is `pub(super)`) so an
//! unexpected dispatch fails loudly.
//!
//! Flat suite, no harness mounts: this models an HTTP wire contract, not an
//! agent turn.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode};
use ironclaw_host_api::{
    AgentId, ProductSurface, ProductSurfaceCaller, ProductSurfaceError,
    ProductSurfaceInvokeRequest, ProductSurfaceInvokeResponse, ProductSurfaceQueryPage,
    ProductSurfaceQueryRequest, ProductSurfaceStreamRequest, ProductSurfaceStreamResponse,
    ProjectId, TenantId, ThreadId, UserId,
};
use ironclaw_product::{
    CREATE_THREAD_COMMAND, ProductCreateThreadRequest, RebornCreateThreadResponse,
    rejecting_product_surface_error,
};
use ironclaw_threads::{SessionThreadRecord, ThreadScope};
use ironclaw_webui::webui_v2::{
    DEFAULT_SSE_MAX_CONCURRENT_PER_CALLER, WebUiV2Capabilities, WebUiV2State, webui_v2_router,
};
use serde_json::Value;
use tower::ServiceExt;

/// Minimal `ProductSurface` fake: only `create_thread` is real (records
/// the request, returns a canned thread); the other required methods reject
/// with the shared error helper.
#[derive(Default)]
struct MinimalWebuiServices {
    create_thread_calls: Mutex<Vec<ProductCreateThreadRequest>>,
}

impl MinimalWebuiServices {
    async fn create_thread(
        &self,
        _caller: ProductSurfaceCaller,
        request: ProductCreateThreadRequest,
    ) -> Result<RebornCreateThreadResponse, ProductSurfaceError> {
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
}

#[async_trait]
impl ProductSurface for MinimalWebuiServices {
    async fn invoke(
        &self,
        caller: ProductSurfaceCaller,
        request: ProductSurfaceInvokeRequest,
    ) -> Result<ProductSurfaceInvokeResponse, ProductSurfaceError> {
        if request.operation_id.as_str() == CREATE_THREAD_COMMAND.id {
            let request = serde_json::from_value(request.input)
                .map_err(ProductSurfaceError::internal_from)?;
            let output = serde_json::to_value(self.create_thread(caller, request).await?)
                .map_err(ProductSurfaceError::internal_from)?;
            return Ok(ProductSurfaceInvokeResponse { output });
        }
        Err(rejecting_product_surface_error())
    }

    async fn query(
        &self,
        _caller: ProductSurfaceCaller,
        _request: ProductSurfaceQueryRequest,
    ) -> Result<ProductSurfaceQueryPage, ProductSurfaceError> {
        Err(rejecting_product_surface_error())
    }

    async fn stream_events(
        &self,
        _caller: ProductSurfaceCaller,
        _request: ProductSurfaceStreamRequest,
    ) -> Result<ProductSurfaceStreamResponse, ProductSurfaceError> {
        Err(rejecting_product_surface_error())
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
    .layer(axum::Extension(ProductSurfaceCaller::new(
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
        "service create_thread dispatched exactly once"
    );
}
