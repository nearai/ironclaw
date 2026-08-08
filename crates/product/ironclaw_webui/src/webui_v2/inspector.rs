//! Operator-only HTTP and SSE transport for bounded run diagnostics.

use std::{convert::Infallible, sync::Arc, time::Duration};

use axum::{
    Json,
    extract::{Extension, Path, Query, State},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response, sse::Event, sse::KeepAlive, sse::Sse},
};
use futures::Stream;
use ironclaw_product_contracts::{
    inspector::{
        DEFAULT_MAX_RETAINED_UPDATES_PER_RUN, DiagnosticCursor, DiagnosticRunRequest,
        DiagnosticToolRequest, INSPECTOR_PROMPT_VIEW, INSPECTOR_SNAPSHOT_VIEW, INSPECTOR_TOOL_VIEW,
        INSPECTOR_UPDATES_VIEW,
    },
    surface::{
        BoundProductSurface, ProductSurface, ProductSurfaceCaller, ProductSurfaceError,
        ProductSurfaceErrorCode, ProductSurfaceErrorKind, ProductSurfaceQueryRequest,
        ProductSurfaceValidationCode,
    },
};
use serde::Deserialize;

use crate::webui_v2::{
    error::WebUiV2HttpError,
    router::{WebUiV2Capabilities, WebUiV2State},
    sse_capacity::{SSE_MAX_LIFETIME, SseAcquireResult, SseSlot},
};

const LAST_EVENT_ID: &str = "last-event-id";
const POLL_INTERVAL: Duration = Duration::from_millis(500);
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(15);

#[derive(Deserialize)]
pub struct InspectorRunPath {
    thread_id: String,
    run_id: String,
}

#[derive(Deserialize)]
pub struct InspectorToolPath {
    thread_id: String,
    run_id: String,
    activity_id: String,
}

#[derive(Default, Deserialize)]
pub struct InspectorUpdatesQuery {
    after_cursor: Option<String>,
    connection_id: Option<String>,
    connection_generation: Option<u64>,
}

fn stream_connection_id(connection_id: Option<&str>) -> Result<Option<&str>, WebUiV2HttpError> {
    let Some(connection_id) = connection_id else {
        return Ok(None);
    };
    if !connection_id.is_empty()
        && connection_id.len() <= 64
        && connection_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Ok(Some(connection_id));
    }
    Err(
        ProductSurfaceError::validation("connection_id", ProductSurfaceValidationCode::InvalidId)
            .into(),
    )
}

fn require_operator(
    caller: &ProductSurfaceCaller,
    capabilities: WebUiV2Capabilities,
) -> Result<(), WebUiV2HttpError> {
    if capabilities.operator_webui_config && caller.operator_config {
        return Ok(());
    }
    Err(ProductSurfaceError::from_status(ProductSurfaceErrorCode::Forbidden, 403, false).into())
}

fn invalid_cursor() -> WebUiV2HttpError {
    ProductSurfaceError::validation("after_cursor", ProductSurfaceValidationCode::InvalidValue)
        .into()
}

async fn query_one(
    services: Arc<dyn ProductSurface>,
    caller: ProductSurfaceCaller,
    view_id: &str,
    input: serde_json::Value,
    cursor: Option<String>,
) -> Result<serde_json::Value, ProductSurfaceError> {
    let page = BoundProductSurface::new(services, caller)
        .query(ProductSurfaceQueryRequest {
            view_id: view_id.to_string(),
            input,
            cursor,
            limit: None,
        })
        .await?;
    let mut items = page.items.into_iter();
    let item = items.next().ok_or_else(ProductSurfaceError::internal)?;
    if items.next().is_some() {
        return Err(ProductSurfaceError::internal());
    }
    Ok(item)
}

fn run_input(path: InspectorRunPath) -> Result<serde_json::Value, ProductSurfaceError> {
    serde_json::to_value(DiagnosticRunRequest {
        thread_id: path.thread_id,
        run_id: path.run_id,
    })
    .map_err(ProductSurfaceError::internal_from)
}

pub async fn get_inspector_snapshot(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<ProductSurfaceCaller>,
    Extension(capabilities): Extension<WebUiV2Capabilities>,
    Path(path): Path<InspectorRunPath>,
) -> Result<Json<serde_json::Value>, WebUiV2HttpError> {
    require_operator(&caller, capabilities)?;
    let input = run_input(path)?;
    Ok(Json(
        query_one(
            state.services().clone(),
            caller,
            INSPECTOR_SNAPSHOT_VIEW.id,
            input,
            None,
        )
        .await?,
    ))
}

pub async fn get_inspector_prompt(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<ProductSurfaceCaller>,
    Extension(capabilities): Extension<WebUiV2Capabilities>,
    Path(path): Path<InspectorRunPath>,
) -> Result<Json<serde_json::Value>, WebUiV2HttpError> {
    require_operator(&caller, capabilities)?;
    let input = run_input(path)?;
    Ok(Json(
        query_one(
            state.services().clone(),
            caller,
            INSPECTOR_PROMPT_VIEW.id,
            input,
            None,
        )
        .await?,
    ))
}

pub async fn get_inspector_tool(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<ProductSurfaceCaller>,
    Extension(capabilities): Extension<WebUiV2Capabilities>,
    Path(path): Path<InspectorToolPath>,
) -> Result<Json<serde_json::Value>, WebUiV2HttpError> {
    require_operator(&caller, capabilities)?;
    let input = serde_json::to_value(DiagnosticToolRequest {
        thread_id: path.thread_id,
        run_id: path.run_id,
        activity_id: path.activity_id,
    })
    .map_err(ProductSurfaceError::internal_from)?;
    Ok(Json(
        query_one(
            state.services().clone(),
            caller,
            INSPECTOR_TOOL_VIEW.id,
            input,
            None,
        )
        .await?,
    ))
}

pub async fn stream_inspector_updates(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<ProductSurfaceCaller>,
    Extension(capabilities): Extension<WebUiV2Capabilities>,
    Path(path): Path<InspectorRunPath>,
    headers: HeaderMap,
    Query(query): Query<InspectorUpdatesQuery>,
) -> Result<Response, WebUiV2HttpError> {
    require_operator(&caller, capabilities)?;
    let header_cursor = headers
        .get(LAST_EVENT_ID)
        .map(|value| value.to_str().map(str::to_string))
        .transpose()
        .map_err(|_| invalid_cursor())?;
    let cursor = header_cursor.or(query.after_cursor);
    if let Some(value) = cursor.as_deref() {
        DiagnosticCursor::parse(value).map_err(|_| invalid_cursor())?;
    }
    let connection_id = stream_connection_id(query.connection_id.as_deref())?;
    if connection_id.is_none() && query.connection_generation.is_some() {
        return Err(ProductSurfaceError::validation(
            "connection_generation",
            ProductSurfaceValidationCode::InvalidValue,
        )
        .into());
    }
    let slot = match state.sse_capacity().try_acquire_ordered(
        &caller.tenant_id,
        &caller.user_id,
        connection_id,
        connection_id.and(query.connection_generation),
    ) {
        SseAcquireResult::Acquired(slot) => slot,
        SseAcquireResult::AtCapacity { .. } => return Ok(capacity_rejected()),
        SseAcquireResult::StaleGeneration => return Ok(StatusCode::NO_CONTENT.into_response()),
    };
    let input = run_input(path)?;
    let stream = build_update_stream(state.services().clone(), caller, input, cursor, slot);
    let mut response = Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(KEEPALIVE_INTERVAL))
        .into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache, no-transform"),
    );
    response.headers_mut().insert(
        HeaderName::from_static("x-accel-buffering"),
        HeaderValue::from_static("no"),
    );
    Ok(response)
}

fn capacity_rejected() -> Response {
    WebUiV2HttpError::from(ProductSurfaceError {
        code: ProductSurfaceErrorCode::RateLimited,
        kind: ProductSurfaceErrorKind::Busy,
        status_code: 429,
        retryable: true,
        field: None,
        validation_code: None,
    })
    .into_response()
}

fn cursor_from_value(value: &serde_json::Value) -> Option<String> {
    let stream_id = value.get("stream_id")?.as_str()?;
    let sequence = value.get("sequence")?.as_u64()?;
    Some(format!("{stream_id}:{sequence}"))
}

fn error_event(error: ProductSurfaceError) -> Event {
    Event::default().event("stream_error").data(
        serde_json::json!({
            "error": error.code,
            "kind": error.kind,
            "retryable": error.retryable,
        })
        .to_string(),
    )
}

fn build_update_stream(
    services: Arc<dyn ProductSurface>,
    caller: ProductSurfaceCaller,
    input: serde_json::Value,
    initial_cursor: Option<String>,
    slot: SseSlot,
) -> impl Stream<Item = Result<Event, Infallible>> {
    async_stream::stream! {
        let mut slot_guard = slot;
        let started_at = tokio::time::Instant::now();
        let mut cursor = initial_cursor;
        // Flush response headers immediately even when this retained run has
        // no updates after its resume cursor. This event deliberately has no
        // SSE id, so it cannot advance or disturb diagnostic cursor ordering.
        yield Ok(Event::default().event("diagnostic_connected").data("{}"));
        loop {
            let remaining = SSE_MAX_LIFETIME.saturating_sub(started_at.elapsed());
            if remaining.is_zero() {
                return;
            }
            let query = query_one(
                Arc::clone(&services),
                caller.clone(),
                INSPECTOR_UPDATES_VIEW.id,
                input.clone(),
                cursor.clone(),
            );
            let result = tokio::select! {
                biased;
                _ = slot_guard.cancelled() => return,
                result = tokio::time::timeout(remaining, query) => result,
            };
            let page = match result {
                Ok(Ok(page)) => page,
                Ok(Err(error)) => {
                    yield Ok(error_event(error));
                    return;
                }
                Err(_) => return,
            };
            let rebase_required =
                page.get("rebase_required").and_then(serde_json::Value::as_bool) == Some(true);
            if rebase_required {
                let next_cursor = page.get("latest_cursor").and_then(cursor_from_value);
                let payload = serde_json::json!({
                    "retention_floor": page.get("retention_floor"),
                    "latest_cursor": page.get("latest_cursor"),
                });
                let mut event = Event::default().event("diagnostic_rebase").data(payload.to_string());
                if let Some(next_cursor) = next_cursor {
                    cursor = Some(next_cursor.clone());
                    event = event.id(next_cursor);
                } else {
                    cursor = None;
                }
                yield Ok(event);
            } else {
                let updates = match page.get("updates").and_then(serde_json::Value::as_array) {
                    Some(updates) => updates,
                    None => {
                        yield Ok(error_event(ProductSurfaceError::internal()));
                        return;
                    }
                };
                if updates.len() > DEFAULT_MAX_RETAINED_UPDATES_PER_RUN {
                    yield Ok(error_event(ProductSurfaceError::internal()));
                    return;
                }
                for update in updates {
                    let Some(next_cursor) = cursor_from_value(update) else {
                        yield Ok(error_event(ProductSurfaceError::internal()));
                        return;
                    };
                    cursor = Some(next_cursor.clone());
                    yield Ok(Event::default()
                        .event("diagnostic_update")
                        .id(next_cursor)
                        .data(update.to_string()));
                }
            }
            let sleep_for = POLL_INTERVAL.min(SSE_MAX_LIFETIME.saturating_sub(started_at.elapsed()));
            if sleep_for.is_zero() {
                return;
            }
            tokio::select! {
                biased;
                _ = slot_guard.cancelled() => return,
                _ = tokio::time::sleep(sleep_for) => {}
            }
        }
    }
}
