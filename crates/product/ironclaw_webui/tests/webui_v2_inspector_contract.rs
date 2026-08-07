//! Caller-level contract tests for the operator diagnostic surface.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use ironclaw_host_api::ids::{AgentId, ProjectId, TenantId, UserId};
use ironclaw_product_contracts::{
    inspector::{INSPECTOR_PROMPT_VIEW, INSPECTOR_SNAPSHOT_VIEW, INSPECTOR_UPDATES_VIEW},
    surface::{
        ProductSurface, ProductSurfaceCaller, ProductSurfaceError, ProductSurfaceInvokeRequest,
        ProductSurfaceInvokeResponse, ProductSurfaceQueryPage, ProductSurfaceQueryRequest,
        ProductSurfaceStreamRequest, ProductSurfaceStreamResponse,
    },
};
use ironclaw_webui::webui_v2::{WebUiV2Capabilities, WebUiV2State, webui_v2_router};
use tower::ServiceExt;

#[derive(Clone, Debug)]
struct QueryCall {
    caller: ProductSurfaceCaller,
    request: ProductSurfaceQueryRequest,
}

#[derive(Default)]
struct RecordingSurface {
    calls: Mutex<Vec<QueryCall>>,
}

impl RecordingSurface {
    fn calls(&self) -> Vec<QueryCall> {
        self.calls.lock().expect("lock calls").clone()
    }
}

#[async_trait]
impl ProductSurface for RecordingSurface {
    async fn invoke(
        &self,
        _caller: ProductSurfaceCaller,
        _request: ProductSurfaceInvokeRequest,
    ) -> Result<ProductSurfaceInvokeResponse, ProductSurfaceError> {
        Err(ProductSurfaceError::service_unavailable(false))
    }

    async fn query(
        &self,
        caller: ProductSurfaceCaller,
        request: ProductSurfaceQueryRequest,
    ) -> Result<ProductSurfaceQueryPage, ProductSurfaceError> {
        self.calls.lock().expect("lock calls").push(QueryCall {
            caller,
            request: request.clone(),
        });
        let payload = match request.view_id.as_str() {
            id if id == INSPECTOR_SNAPSHOT_VIEW.id => serde_json::json!({ "snapshot": null }),
            id if id == INSPECTOR_PROMPT_VIEW.id => serde_json::json!({
                "prompt": {
                    "components": [],
                    "reconstructed_prompt": { "content": "system", "original_bytes": 6, "truncated": false },
                }
            }),
            id if id == INSPECTOR_UPDATES_VIEW.id
                && request.cursor.as_deref() == Some("550e8400-e29b-41d4-a716-446655440000:7") =>
            {
                serde_json::json!({
                    "updates": [{
                        "stream_id": "550e8400-e29b-41d4-a716-446655440000",
                        "sequence": 8,
                        "update": { "type": "stats", "data": {} },
                    }],
                    "retention_floor": {
                        "stream_id": "550e8400-e29b-41d4-a716-446655440000",
                        "sequence": 8,
                    },
                    "latest_cursor": {
                        "stream_id": "550e8400-e29b-41d4-a716-446655440000",
                        "sequence": 8,
                    },
                    "rebase_required": true,
                })
            }
            id if id == INSPECTOR_UPDATES_VIEW.id
                && request.cursor.as_deref() == Some("550e8400-e29b-41d4-a716-446655440000:99") =>
            {
                serde_json::json!({
                    "updates": [],
                    "retention_floor": null,
                    "latest_cursor": null,
                    "rebase_required": true,
                })
            }
            id if id == INSPECTOR_UPDATES_VIEW.id => serde_json::json!({
                "updates": [],
                "retention_floor": null,
                "latest_cursor": null,
                "rebase_required": false,
            }),
            _ => return Err(ProductSurfaceError::service_unavailable(false)),
        };
        Ok(ProductSurfaceQueryPage {
            items: vec![payload],
            next_cursor: None,
        })
    }

    async fn stream_events(
        &self,
        _caller: ProductSurfaceCaller,
        _request: ProductSurfaceStreamRequest,
    ) -> Result<ProductSurfaceStreamResponse, ProductSurfaceError> {
        Err(ProductSurfaceError::service_unavailable(false))
    }
}

#[tokio::test]
async fn prompt_requires_operator_access_and_dispatches_authenticated_scope() {
    let surface = Arc::new(RecordingSurface::default());
    let denied = router(Arc::clone(&surface), caller(false), true, 1)
        .oneshot(
            Request::get("/api/webchat/v2/operator/inspector/threads/thread-a/runs/run-a/prompt")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    assert!(surface.calls().is_empty());

    let allowed = router(Arc::clone(&surface), caller(true), true, 1)
        .oneshot(
            Request::get("/api/webchat/v2/operator/inspector/threads/thread-a/runs/run-a/prompt")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(allowed.status(), StatusCode::OK);

    let calls = surface.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].request.view_id, INSPECTOR_PROMPT_VIEW.id);
    assert_eq!(
        calls[0].request.input,
        serde_json::json!({ "thread_id": "thread-a", "run_id": "run-a" })
    );
    assert_eq!(calls[0].caller.tenant_id.as_str(), "tenant-alpha");
    assert_eq!(calls[0].caller.user_id.as_str(), "user-alpha");
}

fn caller(operator: bool) -> ProductSurfaceCaller {
    ProductSurfaceCaller::new(
        TenantId::new("tenant-alpha").expect("tenant"),
        UserId::new("user-alpha").expect("user"),
        Some(AgentId::new("agent-alpha").expect("agent")),
        Some(ProjectId::new("project-alpha").expect("project")),
    )
    .with_operator_config(operator)
}

fn router(
    surface: Arc<RecordingSurface>,
    caller: ProductSurfaceCaller,
    capability: bool,
    stream_limit: usize,
) -> axum::Router {
    webui_v2_router(WebUiV2State::new(surface, stream_limit))
        .layer(axum::Extension(caller))
        .layer(axum::Extension(WebUiV2Capabilities {
            operator_webui_config: capability,
        }))
}

#[tokio::test]
async fn snapshot_requires_both_operator_gates_before_dispatch() {
    let surface = Arc::new(RecordingSurface::default());
    for (operator, capability) in [(false, true), (true, false)] {
        let response = router(Arc::clone(&surface), caller(operator), capability, 1)
            .oneshot(
                Request::get("/api/webchat/v2/operator/inspector/threads/thread-a/runs/run-a")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
    assert!(surface.calls().is_empty());
}

#[tokio::test]
async fn snapshot_dispatches_path_with_authenticated_caller_scope() {
    let surface = Arc::new(RecordingSurface::default());
    let response = router(Arc::clone(&surface), caller(true), true, 1)
        .oneshot(
            Request::get("/api/webchat/v2/operator/inspector/threads/thread-a/runs/run-a")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 4 * 1024)
        .await
        .expect("body");
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body).expect("json"),
        serde_json::json!({ "snapshot": null })
    );

    let calls = surface.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].request.view_id, INSPECTOR_SNAPSHOT_VIEW.id);
    assert_eq!(
        calls[0].request.input,
        serde_json::json!({ "thread_id": "thread-a", "run_id": "run-a" })
    );
    assert_eq!(calls[0].caller.tenant_id.as_str(), "tenant-alpha");
    assert_eq!(calls[0].caller.user_id.as_str(), "user-alpha");
}

#[tokio::test]
async fn updates_rejects_bad_cursor_and_bounds_concurrent_streams() {
    let surface = Arc::new(RecordingSurface::default());
    let app = router(Arc::clone(&surface), caller(true), true, 1);
    let bad_cursor = app
        .clone()
        .oneshot(
            Request::get(
                "/api/webchat/v2/operator/inspector/threads/thread-a/runs/run-a/events?after_cursor=bad",
            )
            .body(Body::empty())
            .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(bad_cursor.status(), StatusCode::BAD_REQUEST);
    assert!(
        surface.calls().is_empty(),
        "invalid cursor must be rejected before ProductSurface dispatch"
    );

    let first = app
        .clone()
        .oneshot(
            Request::get("/api/webchat/v2/operator/inspector/threads/thread-a/runs/run-a/events")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(first.status(), StatusCode::OK);

    let second = app
        .clone()
        .oneshot(
            Request::get("/api/webchat/v2/operator/inspector/threads/thread-a/runs/run-a/events")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
    drop(first);

    let cursor = "550e8400-e29b-41d4-a716-446655440000:7";
    let reconnect = app
        .oneshot(
            Request::get("/api/webchat/v2/operator/inspector/threads/thread-a/runs/run-a/events")
                .header("last-event-id", cursor)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(reconnect.status(), StatusCode::OK);
    let mut body = reconnect.into_body();
    let frame = tokio::time::timeout(std::time::Duration::from_secs(1), body.frame())
        .await
        .expect("SSE frame timeout")
        .expect("SSE frame")
        .expect("valid SSE frame");
    let data = frame.into_data().expect("data frame");
    let event = String::from_utf8_lossy(&data);
    assert!(
        event.contains("event: diagnostic_rebase"),
        "rebase signal must be visible to reconnecting clients"
    );
    assert!(
        event.contains("id: 550e8400-e29b-41d4-a716-446655440000:8"),
        "rebase signal must advance the EventSource resume cursor"
    );
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), body.frame())
            .await
            .is_err(),
        "retained updates must not follow a rebase event with an older or duplicate SSE id"
    );
    let calls = surface.calls();
    assert_eq!(
        calls
            .last()
            .expect("update query")
            .request
            .cursor
            .as_deref(),
        Some(cursor)
    );
}

#[tokio::test]
async fn missing_run_with_stale_cursor_emits_one_rebase_then_clears_resume_position() {
    let surface = Arc::new(RecordingSurface::default());
    let stale_cursor = "550e8400-e29b-41d4-a716-446655440000:99";
    let response = router(Arc::clone(&surface), caller(true), true, 1)
        .oneshot(
            Request::get("/api/webchat/v2/operator/inspector/threads/thread-a/runs/missing/events")
                .header("last-event-id", stale_cursor)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);

    let mut body = response.into_body();
    let frame = tokio::time::timeout(std::time::Duration::from_secs(1), body.frame())
        .await
        .expect("SSE frame timeout")
        .expect("SSE frame")
        .expect("valid SSE frame");
    let data = frame.into_data().expect("data frame");
    let event = String::from_utf8_lossy(&data);
    assert!(event.contains("event: diagnostic_rebase"));
    assert!(!event.contains("id:"), "missing runs have no resume cursor");
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(750), body.frame())
            .await
            .is_err(),
        "clearing the stale cursor must prevent repeated rebase events"
    );

    let calls = surface.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].request.cursor.as_deref(), Some(stale_cursor));
    assert_eq!(calls[1].request.cursor, None);
}
