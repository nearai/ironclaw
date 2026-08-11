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
    inspector::{
        DEFAULT_MAX_RETAINED_UPDATES_PER_RUN, INSPECTOR_PROMPT_VIEW, INSPECTOR_SNAPSHOT_VIEW,
        INSPECTOR_TOOL_VIEW, INSPECTOR_UPDATES_VIEW,
    },
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
    oversized_updates: bool,
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
            id if id == INSPECTOR_TOOL_VIEW.id => serde_json::json!({
                "tool": {
                    "activity_id": "550e8400-e29b-41d4-a716-446655440001",
                    "capability_name": { "content": "builtin.echo", "original_bytes": 12, "truncated": false },
                    "status": "succeeded",
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
            id if id == INSPECTOR_UPDATES_VIEW.id && self.oversized_updates => {
                serde_json::json!({
                    "updates": vec![
                        serde_json::json!({
                            "stream_id": "550e8400-e29b-41d4-a716-446655440000",
                            "sequence": 1,
                            "update": { "type": "stats", "data": {} },
                        });
                        DEFAULT_MAX_RETAINED_UPDATES_PER_RUN + 1
                    ],
                    "retention_floor": null,
                    "latest_cursor": null,
                    "rebase_required": false,
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
async fn updates_fail_fast_when_product_surface_exceeds_the_retention_contract() {
    let surface = Arc::new(RecordingSurface {
        oversized_updates: true,
        ..RecordingSurface::default()
    });
    let response = router(surface, caller(true), true, 1)
        .oneshot(
            Request::get("/api/webchat/v2/operator/inspector/threads/thread-a/runs/run-a/events")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);

    let mut body = response.into_body();
    let connected = tokio::time::timeout(std::time::Duration::from_secs(1), body.frame())
        .await
        .expect("connected frame timeout")
        .expect("connected frame")
        .expect("valid connected frame")
        .into_data()
        .expect("connected data frame");
    assert!(String::from_utf8_lossy(&connected).contains("event: diagnostic_connected"));

    let failure = tokio::time::timeout(std::time::Duration::from_secs(1), body.frame())
        .await
        .expect("failure frame timeout")
        .expect("failure frame")
        .expect("valid failure frame")
        .into_data()
        .expect("failure data frame");
    let failure = String::from_utf8_lossy(&failure);
    assert!(failure.contains("event: stream_error"));
    assert!(failure.contains(r#""error":"internal""#));
    assert!(failure.contains(r#""retryable":false"#));
    assert!(!failure.contains("event: diagnostic_update"));
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
async fn every_inspector_route_requires_both_operator_gates_before_dispatch() {
    let surface = Arc::new(RecordingSurface::default());
    let paths = [
        "/api/webchat/v2/operator/inspector/threads/thread-a/runs/run-a",
        "/api/webchat/v2/operator/inspector/threads/thread-a/runs/run-a/prompt",
        "/api/webchat/v2/operator/inspector/threads/thread-a/runs/run-a/tools/550e8400-e29b-41d4-a716-446655440001",
        "/api/webchat/v2/operator/inspector/threads/thread-a/runs/run-a/events",
    ];
    for path in paths {
        for (operator, capability) in [(false, true), (true, false), (false, false)] {
            let response = router(Arc::clone(&surface), caller(operator), capability, 1)
                .oneshot(Request::get(path).body(Body::empty()).expect("request"))
                .await
                .expect("response");
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "path: {path}");
        }
    }
    assert!(surface.calls().is_empty());
}

#[tokio::test]
async fn missing_authenticated_context_fails_closed_before_dispatch() {
    let surface = Arc::new(RecordingSurface::default());
    let app = webui_v2_router(WebUiV2State::new(surface.clone(), 1)).layer(axum::Extension(
        WebUiV2Capabilities {
            operator_webui_config: true,
        },
    ));
    for path in [
        "/api/webchat/v2/operator/inspector/threads/thread-a/runs/run-a",
        "/api/webchat/v2/operator/inspector/threads/thread-a/runs/run-a/prompt",
        "/api/webchat/v2/operator/inspector/threads/thread-a/runs/run-a/tools/550e8400-e29b-41d4-a716-446655440001",
        "/api/webchat/v2/operator/inspector/threads/thread-a/runs/run-a/events",
    ] {
        let response = app
            .clone()
            .oneshot(Request::get(path).body(Body::empty()).expect("request"))
            .await
            .expect("response");
        assert_eq!(
            response.status(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "path: {path}",
        );
    }
    assert!(surface.calls().is_empty());
}

#[tokio::test]
async fn tool_detail_dispatches_exact_path_scope_and_authenticated_caller() {
    let surface = Arc::new(RecordingSurface::default());
    let activity_id = "550e8400-e29b-41d4-a716-446655440001";
    let response = router(Arc::clone(&surface), caller(true), true, 1)
        .oneshot(
            Request::get(format!(
                "/api/webchat/v2/operator/inspector/threads/thread-a/runs/run-a/tools/{activity_id}",
            ))
            .body(Body::empty())
            .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);

    let calls = surface.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].request.view_id, INSPECTOR_TOOL_VIEW.id);
    assert_eq!(
        calls[0].request.input,
        serde_json::json!({
            "thread_id": "thread-a",
            "run_id": "run-a",
            "activity_id": activity_id,
        })
    );
    assert_eq!(calls[0].caller.tenant_id.as_str(), "tenant-alpha");
    assert_eq!(calls[0].caller.user_id.as_str(), "user-alpha");
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

    for (invalid_query, expected_field, expected_validation_code) in [
        ("connection_id=invalid%20id", "connection_id", "invalid_id"),
        (
            "connection_generation=1",
            "connection_generation",
            "invalid_value",
        ),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::get(format!(
                    "/api/webchat/v2/operator/inspector/threads/thread-a/runs/run-a/events?{invalid_query}",
                ))
                .body(Body::empty())
                .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), 4 * 1024)
            .await
            .expect("error body");
        let error = serde_json::from_slice::<serde_json::Value>(&body).expect("error json");
        assert_eq!(error["field"], expected_field);
        assert_eq!(error["validation_code"], expected_validation_code);
    }
    assert!(
        surface.calls().is_empty(),
        "invalid connection metadata must be rejected before ProductSurface dispatch"
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
    let connected = tokio::time::timeout(std::time::Duration::from_secs(1), body.frame())
        .await
        .expect("SSE frame timeout")
        .expect("SSE frame")
        .expect("valid SSE frame");
    let connected_data = connected.into_data().expect("connected data frame");
    assert!(
        String::from_utf8_lossy(&connected_data).contains("event: diagnostic_connected"),
        "a reconnect must establish transport health without advancing its cursor",
    );
    let rebase = tokio::time::timeout(std::time::Duration::from_secs(1), body.frame())
        .await
        .expect("rebase frame timeout")
        .expect("rebase frame")
        .expect("valid rebase frame");
    let data = rebase.into_data().expect("rebase data frame");
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
async fn updates_replace_the_same_browser_stream_without_consuming_another_slot() {
    let surface = Arc::new(RecordingSurface::default());
    let app = router(surface, caller(true), true, 1);
    let path = "/api/webchat/v2/operator/inspector/threads/thread-a/runs/run-a/events";
    let first = app
        .clone()
        .oneshot(
            Request::get(format!(
                "{path}?connection_id=inspector-tab&connection_generation=1",
            ))
            .body(Body::empty())
            .expect("request"),
        )
        .await
        .expect("first response");
    assert_eq!(first.status(), StatusCode::OK);
    let mut first_body = first.into_body();
    let first_frame = tokio::time::timeout(std::time::Duration::from_secs(1), first_body.frame())
        .await
        .expect("connected frame timeout")
        .expect("connected frame")
        .expect("valid connected frame")
        .into_data()
        .expect("connected data frame");
    assert!(
        String::from_utf8_lossy(&first_frame).contains("event: diagnostic_connected"),
        "a fresh idle stream must flush an immediate connected event",
    );

    let replacement = app
        .clone()
        .oneshot(
            Request::get(format!(
                "{path}?connection_id=inspector-tab&connection_generation=2",
            ))
            .body(Body::empty())
            .expect("request"),
        )
        .await
        .expect("replacement response");
    assert_eq!(replacement.status(), StatusCode::OK);

    let stale = app
        .clone()
        .oneshot(
            Request::get(format!(
                "{path}?connection_id=inspector-tab&connection_generation=1",
            ))
            .body(Body::empty())
            .expect("request"),
        )
        .await
        .expect("stale response");
    assert_eq!(stale.status(), StatusCode::NO_CONTENT);

    let unrelated = app
        .oneshot(
            Request::get(format!(
                "{path}?connection_id=other-tab&connection_generation=1",
            ))
            .body(Body::empty())
            .expect("request"),
        )
        .await
        .expect("unrelated response");
    assert_eq!(unrelated.status(), StatusCode::TOO_MANY_REQUESTS);

    drop(first_body);
    drop(replacement);
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
    let connected = tokio::time::timeout(std::time::Duration::from_secs(1), body.frame())
        .await
        .expect("connected frame timeout")
        .expect("connected frame")
        .expect("valid connected frame");
    let connected_data = connected.into_data().expect("connected data frame");
    let connected_event = String::from_utf8_lossy(&connected_data);
    assert!(connected_event.contains("event: diagnostic_connected"));
    assert!(
        !connected_event.contains("id:"),
        "transport health must not advance the diagnostic cursor"
    );

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
