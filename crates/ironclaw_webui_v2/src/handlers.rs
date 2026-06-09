//! WebChat v2 HTTP handlers.
//!
//! Every handler:
//!
//! 1. Receives an authenticated caller as an `Extension<WebUiAuthenticatedCaller>`.
//!    Host composition is responsible for running the bearer-token middleware
//!    that builds that extension; the handler never sees a raw bearer token.
//! 2. Dispatches through [`RebornServicesApi`]. No direct access to the
//!    dispatcher, `HostRuntime`, run-state, DB stores, or any runtime lane.
//! 3. Maps every error through [`WebUiV2HttpError`] so the wire shape stays
//!    redacted and stable.
//!
//! [`RebornServicesApi`]: ironclaw_product_workflow::RebornServicesApi

use std::convert::Infallible;
use std::time::Duration;

use axum::Json;
use axum::extract::{Extension, Path, Query, State};
use axum::http::HeaderMap;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures::SinkExt;
use futures::stream::Stream;
use ironclaw_product_workflow::{
    CodexLoginStart, LifecyclePackageKind, LifecyclePackageRef, LlmConfigSnapshot, LlmModelsResult,
    LlmProbeRequest, LlmProbeResult, NearAiLoginRequest, NearAiLoginStart,
    NearAiWalletLoginRequest, NearAiWalletLoginResult, ProductWorkflowError, ProjectionCursor,
    RebornCancelRunResponse, RebornConnectableChannelListResponse, RebornCreateThreadResponse,
    RebornDeleteThreadRequest, RebornDeleteThreadResponse, RebornExtensionActionResponse,
    RebornExtensionListResponse, RebornExtensionRegistryResponse, RebornListAutomationsResponse,
    RebornListThreadsResponse, RebornOperatorCommandPlaneResponse,
    RebornOperatorConfigValidateRequest, RebornOperatorLogsQuery,
    RebornOperatorServiceLifecycleRequest, RebornOperatorSetupRequest, RebornResolveGateResponse,
    RebornServicesApi, RebornServicesError, RebornServicesErrorCode, RebornServicesErrorKind,
    RebornSetupExtensionResponse, RebornSkillActionResponse, RebornSkillContentResponse,
    RebornSkillListResponse, RebornSkillSearchResponse, RebornStreamEventsRequest,
    RebornSubmitTurnResponse, RebornTimelineRequest, RebornTimelineResponse, SetActiveLlmRequest,
    UpsertLlmProviderRequest, WebUiAuthenticatedCaller, WebUiCancelRunRequest,
    WebUiCreateThreadRequest, WebUiInboundValidationCode, WebUiInboundValidationError,
    WebUiListAutomationsRequest, WebUiListThreadsRequest, WebUiResolveGateRequest,
    WebUiSendMessageRequest, WebUiSetupExtensionRequest,
};
use serde::{Deserialize, Serialize};

use crate::error::WebUiV2HttpError;
use crate::router::{WebUiV2Capabilities, WebUiV2State};
use crate::schema::WebChatV2EventFrame;
use crate::sse_capacity::{SSE_MAX_LIFETIME, SseSlot};

#[derive(Debug, Clone, Serialize)]
pub struct WebUiV2SessionResponse {
    pub tenant_id: String,
    pub user_id: String,
    pub capabilities: WebUiV2Capabilities,
}

/// `GET /api/webchat/v2/session`
pub async fn get_session(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
) -> Json<WebUiV2SessionResponse> {
    Json(WebUiV2SessionResponse {
        tenant_id: caller.tenant_id.to_string(),
        user_id: caller.user_id.to_string(),
        capabilities: state.capabilities(),
    })
}

/// `POST /api/webchat/v2/threads`
///
/// Body shape: [`WebUiCreateThreadRequest`].
pub async fn create_thread(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(body): Json<WebUiCreateThreadRequest>,
) -> Result<Json<RebornCreateThreadResponse>, WebUiV2HttpError> {
    let response = state.services().create_thread(caller, body).await?;
    Ok(Json(response))
}

/// `DELETE /api/webchat/v2/threads/{thread_id}`
pub async fn delete_thread(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(thread_id): Path<String>,
) -> Result<Json<RebornDeleteThreadResponse>, WebUiV2HttpError> {
    let response = state
        .services()
        .delete_thread(caller, RebornDeleteThreadRequest { thread_id })
        .await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/threads/{thread_id}/messages`
///
/// Body shape: [`WebUiSendMessageRequest`] (the path `thread_id` overrides
/// any value in the body).
pub async fn send_message(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(thread_id): Path<String>,
    Json(mut body): Json<WebUiSendMessageRequest>,
) -> Result<Json<RebornSubmitTurnResponse>, WebUiV2HttpError> {
    body.thread_id = Some(thread_id);
    let response = state.services().submit_turn(caller, body).await?;
    Ok(Json(response))
}

/// `GET /api/webchat/v2/threads/{thread_id}/timeline`
///
/// Optional query parameters:
/// - `limit`: maximum number of messages per response. The facade
///   clamps to a hard ceiling so an unbounded value cannot widen the
///   response.
/// - `cursor`: opaque cursor echoed from the previous response's
///   `next_cursor` to load the page preceding it.
pub async fn get_timeline(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(thread_id): Path<String>,
    Query(query): Query<TimelineQuery>,
) -> Result<Json<RebornTimelineResponse>, WebUiV2HttpError> {
    let request = RebornTimelineRequest {
        thread_id,
        limit: query.limit,
        cursor: query.cursor,
    };
    let response = state.services().get_timeline(caller, request).await?;
    Ok(Json(response))
}

/// Query parameters for `get_timeline`. Both fields are optional — a
/// caller with neither gets the most recent page (default size).
#[derive(Debug, Default, Deserialize)]
pub struct TimelineQuery {
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub cursor: Option<String>,
}

/// SSE polling cadence for `stream_events`. The facade only exposes a
/// drain-style read; once the backlog is flushed the handler waits this
/// long before checking for newly arrived events.
const SSE_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// SSE keep-alive cadence. axum emits an SSE comment line every interval
/// to keep proxies from closing the idle connection.
const SSE_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(15);

/// HTTP header the browser's `EventSource` sends on auto-reconnect to
/// resume an SSE stream. The value is the `id:` of the last successfully
/// delivered event; for this surface the handler sets that to the JSON-
/// serialized projection cursor.
const LAST_EVENT_ID_HEADER: &str = "last-event-id";

/// `GET /api/webchat/v2/threads/{thread_id}/events`
///
/// Server-Sent Events stream. Each event carries one
/// [`WebChatV2EventFrame`] as JSON with the projection cursor as the
/// SSE `id` so the browser can resume from the last delivered event.
///
/// Resume cursor precedence: `Last-Event-ID` header (sent automatically
/// by the browser's `EventSource` on reconnect) wins over the
/// `?after_cursor=...` query parameter. Both are optional — first
/// connects pass neither and start from the projection origin.
///
/// The handler acquires a per-`(tenant, user)` concurrency slot before
/// returning the stream; callers at or above the configured cap receive
/// `429 Too Many Requests` with `retryable: true`. Each stream is also
/// closed after [`SSE_MAX_LIFETIME`] so the browser must reconnect with
/// `Last-Event-ID`, which bounds drift and recycles slots even under
/// long-running tab leaks.
///
/// Until the facade gains a true subscription API, the handler drains and
/// polls in a loop. Drain-only semantics are documented on
/// [`RebornServicesApi::stream_events`].
///
/// [`WebChatV2EventFrame`]: crate::schema::WebChatV2EventFrame
/// [`RebornServicesApi::stream_events`]: ironclaw_product_workflow::RebornServicesApi::stream_events
/// [`SSE_MAX_LIFETIME`]: crate::sse_capacity::SSE_MAX_LIFETIME
pub async fn stream_events(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(thread_id): Path<String>,
    headers: HeaderMap,
    Query(query): Query<StreamEventsQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, WebUiV2HttpError> {
    let slot = state
        .sse_capacity()
        .try_acquire(&caller.tenant_id, &caller.user_id)
        .ok_or_else(sse_concurrency_exhausted)?;
    let services = state.services().clone();
    let initial_cursor = headers
        .get(LAST_EVENT_ID_HEADER)
        // silent-ok: non-visible-ASCII Last-Event-ID is treated as absent so the
        // handler falls back to the query param / origin, matching the standard
        // EventSource contract (server SHOULD ignore a malformed Last-Event-ID).
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .or(query.after_cursor);
    let stream = build_sse_stream(services, caller, thread_id, initial_cursor, slot);
    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(SSE_KEEPALIVE_INTERVAL)))
}

/// Build the 429 response for SSE openings that exceed the per-caller
/// concurrency cap. `retryable: true` because the slot will free as soon
/// as one of the caller's existing streams closes.
fn sse_concurrency_exhausted() -> WebUiV2HttpError {
    WebUiV2HttpError::from(RebornServicesError {
        code: RebornServicesErrorCode::RateLimited,
        kind: RebornServicesErrorKind::Busy,
        status_code: 429,
        retryable: true,
        field: None,
        validation_code: None,
    })
}

/// Query parameters for `stream_events`. `after_cursor` is the opaque
/// projection cursor the browser saw last; on first connect it is omitted
/// so the handler drains from the origin.
#[derive(Debug, Default, Deserialize)]
pub struct StreamEventsQuery {
    #[serde(default)]
    pub after_cursor: Option<String>,
}

/// Redacted SSE error payload. Defined as a typed struct (not built with
/// `serde_json::json!`) so the `Serialize` derive is total — serialization
/// cannot fail on a tagged enum + bool, so there is no fallback branch.
#[derive(Debug, Clone, Serialize)]
struct SseErrorPayload {
    error: RebornServicesErrorCode,
    kind: RebornServicesErrorKind,
    retryable: bool,
}

fn build_sse_stream(
    services: std::sync::Arc<dyn RebornServicesApi>,
    caller: WebUiAuthenticatedCaller,
    thread_id: String,
    initial_cursor: Option<String>,
    slot: SseSlot,
) -> impl Stream<Item = Result<Event, Infallible>> {
    async_stream::stream! {
        // The slot guard moves into the generator and stays alive for
        // the lifetime of this stream. It drops automatically when the
        // generator is dropped (client disconnect, max-lifetime expiry,
        // or facade error), releasing the per-caller concurrency slot.
        let _slot_guard = slot;
        let started_at = tokio::time::Instant::now();
        let mut after_cursor = initial_cursor.and_then(parse_cursor_token);
        loop {
            // Force a clean close once the budget is exhausted so the
            // browser can reconnect with Last-Event-ID; this caps single-
            // stream lifetime regardless of client behavior and recycles
            // the slot. `remaining` also bounds the await below so a
            // stuck projection drain cannot pin the slot past the budget.
            let remaining = SSE_MAX_LIFETIME.saturating_sub(started_at.elapsed());
            if remaining.is_zero() {
                return;
            }
            let request = RebornStreamEventsRequest {
                thread_id: thread_id.clone(),
                after_cursor: after_cursor.clone(),
            };
            match tokio::time::timeout(
                remaining,
                services.stream_events(caller.clone(), request),
            )
            .await
            {
                Err(_elapsed) => {
                    // The facade drain was still pending when SSE_MAX_LIFETIME
                    // ran out. Returning here drops the generator (and the
                    // SseSlot it owns), so the per-caller concurrency budget
                    // recovers even under a stuck projection stream — without
                    // this bound, an unbounded `.await` on a non-resolving
                    // facade would pin the slot indefinitely.
                    tracing::debug!(
                        target = "ironclaw_webui_v2::sse",
                        "stream_events drain pending past SSE_MAX_LIFETIME; closing stream"
                    );
                    return;
                }
                Ok(Ok(response)) => {
                    if let Some(latest) = response.events.last() {
                        after_cursor = Some(latest.projection_cursor.clone());
                    }
                    for envelope in response.events {
                        let frame = WebChatV2EventFrame::from_outbound(envelope);
                        let id = cursor_token(frame.cursor());
                        match serde_json::to_string(&frame) {
                            Ok(payload) => {
                                let mut event = Event::default().event(frame.event_name()).data(payload);
                                if let Some(id) = id {
                                    event = event.id(id);
                                }
                                yield Ok(event);
                            }
                            Err(error) => {
                                // debug, not warn: this is an internal
                                // diagnostic, not user-facing status, and
                                // info!/warn! corrupts the REPL/TUI per
                                // CLAUDE.md.
                                tracing::debug!(
                                    target = "ironclaw_webui_v2::sse",
                                    error = %error,
                                    "failed to serialize WebChatV2EventFrame for SSE",
                                );
                            }
                        }
                    }
                    // Bound the poll sleep too so we never oversleep past the
                    // lifetime budget; the top-of-loop check then fires.
                    let sleep_for = SSE_POLL_INTERVAL
                        .min(SSE_MAX_LIFETIME.saturating_sub(started_at.elapsed()));
                    if sleep_for.is_zero() {
                        return;
                    }
                    tokio::time::sleep(sleep_for).await;
                }
                Ok(Err(error)) => {
                    // Surface a redacted error event and close the stream.
                    // Reconnect logic is the browser's responsibility.
                    tracing::debug!(
                        target = "ironclaw_webui_v2::sse",
                        error = ?error,
                        "facade rejected SSE drain; closing stream",
                    );
                    let payload = SseErrorPayload {
                        error: error.code,
                        kind: error.kind,
                        retryable: error.retryable,
                    };
                    yield Ok(Event::default()
                        .event("error")
                        .json_data(payload)
                        .expect("SseErrorPayload is a tagged enum + bool with derived Serialize; cannot fail")); // safety: typed struct with derived Serialize on serde-compatible fields only
                    return;
                }
            }
        }
    }
}

fn parse_cursor_token(token: String) -> Option<ProjectionCursor> {
    // The wire form is the JSON-serialized cursor; we accept it verbatim
    // so the browser can echo back the `id` of the last SSE event it saw
    // (which is exactly that JSON).
    serde_json::from_str(&token).ok()
}

fn cursor_token(cursor: &ProjectionCursor) -> Option<String> {
    serde_json::to_string(cursor).ok()
}

/// `POST /api/webchat/v2/threads/{thread_id}/runs/{run_id}/cancel`
///
/// Body shape: [`WebUiCancelRunRequest`] (path `thread_id` and `run_id`
/// override body values).
pub async fn cancel_run(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(CancelRunPath { thread_id, run_id }): Path<CancelRunPath>,
    Json(mut body): Json<WebUiCancelRunRequest>,
) -> Result<Json<RebornCancelRunResponse>, WebUiV2HttpError> {
    body.thread_id = Some(thread_id);
    body.run_id = Some(run_id);
    let response = state.services().cancel_run(caller, body).await?;
    Ok(Json(response))
}

#[derive(Debug, Deserialize)]
pub struct CancelRunPath {
    pub thread_id: String,
    pub run_id: String,
}

/// `POST /api/webchat/v2/threads/{thread_id}/runs/{run_id}/gates/{gate_ref}/resolve`
///
/// Body shape: [`WebUiResolveGateRequest`] (path overrides body for
/// `thread_id`, `run_id`, `gate_ref`).
pub async fn resolve_gate(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(ResolveGatePath {
        thread_id,
        run_id,
        gate_ref,
    }): Path<ResolveGatePath>,
    Json(mut body): Json<WebUiResolveGateRequest>,
) -> Result<Json<RebornResolveGateResponse>, WebUiV2HttpError> {
    body.thread_id = Some(thread_id);
    body.run_id = Some(run_id);
    body.gate_ref = Some(gate_ref);
    let response = state.services().resolve_gate(caller, body).await?;
    Ok(Json(response))
}

#[derive(Debug, Deserialize)]
pub struct ResolveGatePath {
    pub thread_id: String,
    pub run_id: String,
    pub gate_ref: String,
}

/// `GET /api/webchat/v2/threads`
///
/// Lists threads scoped to the authenticated caller. Pagination is
/// opaque: the response carries an optional `next_cursor` the browser
/// echoes back as `?cursor=...` on the next page request.
pub async fn list_threads(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Query(query): Query<ListThreadsQuery>,
) -> Result<Json<RebornListThreadsResponse>, WebUiV2HttpError> {
    let request = WebUiListThreadsRequest {
        limit: query.limit,
        cursor: query.cursor,
    };
    let response = state.services().list_threads(caller, request).await?;
    Ok(Json(response))
}

#[derive(Debug, Default, Deserialize)]
pub struct ListThreadsQuery {
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub cursor: Option<String>,
}

/// `GET /api/webchat/v2/automations`
///
/// Lists the caller-scoped schedule automations visible to the browser. The
/// optional `?limit=N` and `?run_limit=N` queries are capped by the product
/// workflow facade; the response is a single bounded page and does not include
/// a cursor.
pub async fn list_automations(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Query(query): Query<ListAutomationsQuery>,
) -> Result<Json<RebornListAutomationsResponse>, WebUiV2HttpError> {
    let request = WebUiListAutomationsRequest {
        limit: query.limit,
        run_limit: query.run_limit,
    };
    let response = state.services().list_automations(caller, request).await?;
    Ok(Json(response))
}

#[derive(Debug, Default, Deserialize)]
pub struct ListAutomationsQuery {
    /// Optional maximum number of schedule automations to return.
    #[serde(default)]
    pub limit: Option<u32>,
    /// Optional maximum number of recent runs to return per automation row.
    #[serde(default)]
    pub run_limit: Option<u32>,
}

/// `GET /api/webchat/v2/channels/connectable`
pub async fn list_connectable_channels(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
) -> Result<Json<RebornConnectableChannelListResponse>, WebUiV2HttpError> {
    let response = state.services().list_connectable_channels(caller).await?;
    Ok(Json(response))
}

/// `GET /api/webchat/v2/extensions`
pub async fn list_extensions(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
) -> Result<Json<RebornExtensionListResponse>, WebUiV2HttpError> {
    let response = state.services().list_extensions(caller).await?;
    Ok(Json(response))
}

/// `GET /api/webchat/v2/skills`
pub async fn list_skills(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
) -> Result<Json<RebornSkillListResponse>, WebUiV2HttpError> {
    let response = state.services().list_skills(caller).await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/skills/search`
pub async fn search_skills(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(body): Json<SearchSkillsBody>,
) -> Result<Json<RebornSkillSearchResponse>, WebUiV2HttpError> {
    let response = state.services().search_skills(caller, body.query).await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/skills/install`
pub async fn install_skill(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(body): Json<InstallSkillBody>,
) -> Result<Json<RebornSkillActionResponse>, WebUiV2HttpError> {
    let response = state
        .services()
        .install_skill(caller, body.name, body.content)
        .await?;
    Ok(Json(response))
}

/// `GET /api/webchat/v2/skills/{name}`
pub async fn get_skill_content(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(SkillPath { name }): Path<SkillPath>,
) -> Result<Json<RebornSkillContentResponse>, WebUiV2HttpError> {
    let response = state.services().read_skill_content(caller, name).await?;
    Ok(Json(response))
}

/// `PUT /api/webchat/v2/skills/{name}`
pub async fn update_skill(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(SkillPath { name }): Path<SkillPath>,
    Json(body): Json<UpdateSkillBody>,
) -> Result<Json<RebornSkillActionResponse>, WebUiV2HttpError> {
    let response = state
        .services()
        .update_skill(caller, name, body.content)
        .await?;
    Ok(Json(response))
}

/// `DELETE /api/webchat/v2/skills/{name}`
pub async fn remove_skill(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(SkillPath { name }): Path<SkillPath>,
) -> Result<Json<RebornSkillActionResponse>, WebUiV2HttpError> {
    let response = state.services().remove_skill(caller, name).await?;
    Ok(Json(response))
}

/// `GET /api/webchat/v2/extensions/registry`
pub async fn list_extension_registry(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
) -> Result<Json<RebornExtensionRegistryResponse>, WebUiV2HttpError> {
    let response = state.services().list_extension_registry(caller).await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/extensions/install`
pub async fn install_extension(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(body): Json<InstallExtensionBody>,
) -> Result<Json<RebornExtensionActionResponse>, WebUiV2HttpError> {
    let package_ref = extension_package_ref_for_request(Ok(body.package_ref), "package_ref")?;
    let response = state
        .services()
        .install_extension(caller, package_ref)
        .await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/extensions/{package_id}/activate`
pub async fn activate_extension(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(ExtensionPackagePath { package_id }): Path<ExtensionPackagePath>,
) -> Result<Json<RebornExtensionActionResponse>, WebUiV2HttpError> {
    let package_ref = extension_package_ref_for_request(
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, package_id),
        "package_id",
    )?;
    let response = state
        .services()
        .activate_extension(caller, package_ref)
        .await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/extensions/{package_id}/remove`
pub async fn remove_extension(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(ExtensionPackagePath { package_id }): Path<ExtensionPackagePath>,
) -> Result<Json<RebornExtensionActionResponse>, WebUiV2HttpError> {
    let package_ref = extension_package_ref_for_request(
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, package_id),
        "package_id",
    )?;
    let response = state
        .services()
        .remove_extension(caller, package_ref)
        .await?;
    Ok(Json(response))
}

/// `GET /api/webchat/v2/extensions/{package_id}/setup`
pub async fn get_extension_setup(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(ExtensionPackagePath { package_id }): Path<ExtensionPackagePath>,
) -> Result<Json<RebornSetupExtensionResponse>, WebUiV2HttpError> {
    let package_ref = extension_package_ref_for_request(
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, package_id),
        "package_id",
    )?;
    let response = state
        .services()
        .setup_extension(caller, package_ref, WebUiSetupExtensionRequest::default())
        .await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/extensions/{package_id}/setup`
///
/// V2-native route that returns a product-safe lifecycle projection. The route
/// exists so the v2 entrypoint inventory is complete and so future onboarding
/// port work has a stable surface to fill in without coupling this crate to v1
/// onboarding controllers.
///
/// The path segment is lifted into a lifecycle package ref at the
/// handler/facade boundary; a malformed identifier returns `400
/// invalid_argument` before the facade is called.
pub async fn setup_extension(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(ExtensionPackagePath { package_id }): Path<ExtensionPackagePath>,
    Json(body): Json<WebUiSetupExtensionRequest>,
) -> Result<Json<RebornSetupExtensionResponse>, WebUiV2HttpError> {
    let package_ref = extension_package_ref_for_request(
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, package_id),
        "package_id",
    )?;
    let response = state
        .services()
        .setup_extension(caller, package_ref, body)
        .await?;
    Ok(Json(response))
}

/// `GET /api/webchat/v2/operator/setup`
pub async fn get_operator_setup(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
) -> Result<Json<RebornOperatorCommandPlaneResponse>, WebUiV2HttpError> {
    let response = state.services().get_operator_setup(caller).await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/operator/setup`
pub async fn run_operator_setup(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(body): Json<RebornOperatorSetupRequest>,
) -> Result<Json<RebornOperatorCommandPlaneResponse>, WebUiV2HttpError> {
    let response = state.services().run_operator_setup(caller, body).await?;
    Ok(Json(response))
}

/// `GET /api/webchat/v2/operator/config`
pub async fn list_operator_config(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
) -> Result<Json<RebornOperatorCommandPlaneResponse>, WebUiV2HttpError> {
    let response = state.services().list_operator_config(caller).await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/operator/config/validate`
pub async fn validate_operator_config(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(body): Json<RebornOperatorConfigValidateRequest>,
) -> Result<Json<RebornOperatorCommandPlaneResponse>, WebUiV2HttpError> {
    let response = state
        .services()
        .validate_operator_config(caller, body)
        .await?;
    Ok(Json(response))
}

/// `GET /api/webchat/v2/operator/diagnostics`
pub async fn get_operator_diagnostics(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
) -> Result<Json<RebornOperatorCommandPlaneResponse>, WebUiV2HttpError> {
    let response = state.services().get_operator_diagnostics(caller).await?;
    Ok(Json(response))
}

/// `GET /api/webchat/v2/operator/status`
pub async fn get_operator_status(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
) -> Result<Json<RebornOperatorCommandPlaneResponse>, WebUiV2HttpError> {
    let response = state.services().get_operator_status(caller).await?;
    Ok(Json(response))
}

/// `GET /api/webchat/v2/operator/logs`
pub async fn query_operator_logs(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Query(query): Query<RebornOperatorLogsQuery>,
) -> Result<Json<RebornOperatorCommandPlaneResponse>, WebUiV2HttpError> {
    let response = state.services().query_operator_logs(caller, query).await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/operator/service`
pub async fn run_operator_service_lifecycle(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(body): Json<RebornOperatorServiceLifecycleRequest>,
) -> Result<Json<RebornOperatorCommandPlaneResponse>, WebUiV2HttpError> {
    let response = state
        .services()
        .run_operator_service_lifecycle(caller, body)
        .await?;
    Ok(Json(response))
}

/// Path param carrying the LLM provider id.
#[derive(Debug, Deserialize)]
pub struct LlmProviderPath {
    pub provider_id: String,
}

/// `GET /api/webchat/v2/llm/providers`
pub async fn get_llm_config(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
) -> Result<Json<LlmConfigSnapshot>, WebUiV2HttpError> {
    let response = state.services().get_llm_config(caller).await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/llm/providers`
pub async fn upsert_llm_provider(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(body): Json<UpsertLlmProviderRequest>,
) -> Result<Json<LlmConfigSnapshot>, WebUiV2HttpError> {
    let response = state.services().upsert_llm_provider(caller, body).await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/llm/providers/{provider_id}/delete`
pub async fn delete_llm_provider(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(LlmProviderPath { provider_id }): Path<LlmProviderPath>,
) -> Result<Json<LlmConfigSnapshot>, WebUiV2HttpError> {
    let response = state
        .services()
        .delete_llm_provider(caller, provider_id)
        .await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/llm/active`
pub async fn set_active_llm(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(body): Json<SetActiveLlmRequest>,
) -> Result<Json<LlmConfigSnapshot>, WebUiV2HttpError> {
    let response = state.services().set_active_llm(caller, body).await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/llm/test-connection`
pub async fn test_llm_connection(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(body): Json<LlmProbeRequest>,
) -> Result<Json<LlmProbeResult>, WebUiV2HttpError> {
    let response = state.services().test_llm_connection(caller, body).await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/llm/list-models`
pub async fn list_llm_models(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(body): Json<LlmProbeRequest>,
) -> Result<Json<LlmModelsResult>, WebUiV2HttpError> {
    let response = state.services().list_llm_models(caller, body).await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/llm/nearai/login`
pub async fn start_nearai_login(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(body): Json<NearAiLoginRequest>,
) -> Result<Json<NearAiLoginStart>, WebUiV2HttpError> {
    let response = state.services().start_nearai_login(caller, body).await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/llm/nearai/wallet`
///
/// Completes a NEAR AI wallet (NEP-413) login from a browser-signed message:
/// relays the signature to NEAR AI, stores the session token, and makes NEAR AI
/// active.
pub async fn complete_nearai_wallet_login(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(body): Json<NearAiWalletLoginRequest>,
) -> Result<Json<NearAiWalletLoginResult>, WebUiV2HttpError> {
    let response = state
        .services()
        .complete_nearai_wallet_login(caller, body)
        .await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/llm/codex/login`
///
/// Begins an OpenAI Codex device-code login. Takes no body — returns the user
/// code + verification URL to display; a background task completes the flow.
pub async fn start_codex_login(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
) -> Result<Json<CodexLoginStart>, WebUiV2HttpError> {
    let response = state.services().start_codex_login(caller).await?;
    Ok(Json(response))
}

#[derive(Debug, Deserialize)]
pub struct ExtensionPackagePath {
    pub package_id: String,
}

#[derive(Debug, Deserialize)]
pub struct InstallExtensionBody {
    pub package_ref: LifecyclePackageRef,
}

#[derive(Debug, Deserialize)]
pub struct SkillPath {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct SearchSkillsBody {
    pub query: String,
}

#[derive(Debug, Deserialize)]
pub struct InstallSkillBody {
    pub name: String,
    pub content: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSkillBody {
    pub content: String,
}

fn extension_package_ref_for_request(
    package_ref: Result<LifecyclePackageRef, ProductWorkflowError>,
    field: &'static str,
) -> Result<LifecyclePackageRef, RebornServicesError> {
    package_ref
        .and_then(LifecyclePackageRef::require_extension)
        .map_err(|_| {
            RebornServicesError::from(WebUiInboundValidationError::new(
                field,
                WebUiInboundValidationCode::InvalidId,
            ))
        })
}

/// `GET /api/webchat/v2/threads/{thread_id}/ws`
///
/// WebSocket transport variant of [`stream_events`]. The handler
/// accepts the WS upgrade, drains the same `RebornServicesApi::
/// stream_events` facade as the SSE handler, and writes each event as
/// a JSON text frame. Same lifetime + per-caller concurrency caps as
/// SSE.
///
/// Same-origin enforcement is the responsibility of host composition's
/// origin-check middleware — the descriptor declares
/// `WebSocketOriginPolicy::SameOriginRequired` so a future override
/// to a host-allowlist is one descriptor change away. This handler
/// trusts the composition layer to have already rejected
/// disallowed-origin upgrades.
pub async fn stream_events_ws(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(thread_id): Path<String>,
    headers: HeaderMap,
    Query(query): Query<StreamEventsQuery>,
    upgrade: axum::extract::ws::WebSocketUpgrade,
) -> Result<axum::response::Response, WebUiV2HttpError> {
    let slot = state
        .sse_capacity()
        .try_acquire(&caller.tenant_id, &caller.user_id)
        .ok_or_else(sse_concurrency_exhausted)?;
    let services = state.services().clone();
    let initial_cursor = headers
        .get(LAST_EVENT_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .or(query.after_cursor);
    Ok(upgrade.on_upgrade(move |socket| {
        ws_drain_loop(services, caller, thread_id, initial_cursor, slot, socket)
    }))
}

async fn ws_drain_loop(
    services: std::sync::Arc<dyn RebornServicesApi>,
    caller: WebUiAuthenticatedCaller,
    thread_id: String,
    initial_cursor: Option<String>,
    slot: SseSlot,
    mut socket: axum::extract::ws::WebSocket,
) {
    // Mirror the SSE generator: own the slot guard, bound stream
    // lifetime, drain stream_events on the same poll cadence, emit
    // each envelope as a JSON text frame.
    //
    // Two failure modes the loop must observe:
    //
    // 1. **Peer close / socket error.** The browser may close an
    //    idle tab without trading a close frame; the OS surfaces
    //    that as either a `Close` message or a socket-error on the
    //    next read. The loop watches `socket.recv()` on every
    //    `.await` so a dropped tab exits immediately instead of
    //    pinning the per-caller `SseSlot` for up to `SSE_MAX_LIFETIME`.
    // 2. **TCP backpressure on send.** A slow / hostile reader can
    //    leave bytes queued indefinitely. Each `socket.send().await`
    //    runs under `ws_send_with_timeout` so the per-caller slot
    //    is released within the lifetime budget regardless.
    let _slot_guard = slot;
    let started_at = tokio::time::Instant::now();
    let mut after_cursor = initial_cursor.and_then(parse_cursor_token);
    loop {
        let remaining = SSE_MAX_LIFETIME.saturating_sub(started_at.elapsed());
        if remaining.is_zero() {
            let _ =
                ws_send_with_timeout(&mut socket, None, std::time::Duration::from_millis(0)).await;
            return;
        }
        let request = RebornStreamEventsRequest {
            thread_id: thread_id.clone(),
            after_cursor: after_cursor.clone(),
        };
        let facade_call = services.stream_events(caller.clone(), request);
        let outcome = tokio::select! {
            biased;
            // Peer close / socket error wins over the facade poll —
            // if the browser already dropped the connection we want
            // to free the slot immediately, not wait for stream_events
            // to return.
            incoming = socket.recv() => {
                match incoming {
                    None | Some(Err(_)) => return,
                    Some(Ok(axum::extract::ws::Message::Close(_))) => return,
                    // Ignore other inbound frames (Ping/Pong are
                    // handled internally by axum; Text/Binary from
                    // the browser are not part of this contract).
                    Some(Ok(_)) => continue,
                }
            }
            facade = tokio::time::timeout(remaining, facade_call) => facade,
        };
        match outcome {
            Err(_elapsed) => {
                let _ = socket.close().await;
                return;
            }
            Ok(Ok(response)) => {
                if let Some(latest) = response.events.last() {
                    after_cursor = Some(latest.projection_cursor.clone());
                }
                for envelope in response.events {
                    match serde_json::to_string(&envelope) {
                        Ok(text) => {
                            let send_budget = SSE_MAX_LIFETIME.saturating_sub(started_at.elapsed());
                            if send_budget.is_zero() {
                                let _ = socket.close().await;
                                return;
                            }
                            if ws_send_with_timeout(
                                &mut socket,
                                Some(axum::extract::ws::Message::Text(text.into())),
                                send_budget,
                            )
                            .await
                            .is_err()
                            {
                                // Peer hung up, TCP backpressure
                                // exceeded budget, or socket otherwise
                                // unwritable. Drop the task and
                                // release the slot.
                                return;
                            }
                        }
                        Err(error) => {
                            tracing::debug!(
                                target = "ironclaw_webui_v2::ws",
                                error = %error,
                                "failed to serialize ProductOutboundEnvelope for WS",
                            );
                        }
                    }
                }
                let sleep_for =
                    SSE_POLL_INTERVAL.min(SSE_MAX_LIFETIME.saturating_sub(started_at.elapsed()));
                if sleep_for.is_zero() {
                    let _ = socket.close().await;
                    return;
                }
                // Race the poll-interval sleep against socket close
                // for the same reason as the facade call above: if
                // the peer drops during the idle window, free the
                // slot immediately.
                tokio::select! {
                    biased;
                    incoming = socket.recv() => match incoming {
                        None | Some(Err(_)) => return,
                        Some(Ok(axum::extract::ws::Message::Close(_))) => return,
                        Some(Ok(_)) => {}
                    },
                    _ = tokio::time::sleep(sleep_for) => {}
                }
            }
            Ok(Err(error)) => {
                tracing::debug!(
                    target = "ironclaw_webui_v2::ws",
                    error = ?error,
                    "facade rejected WS drain; closing stream",
                );
                let payload = SseErrorPayload {
                    error: error.code,
                    kind: error.kind,
                    retryable: error.retryable,
                };
                if let Ok(text) = serde_json::to_string(&payload) {
                    let send_budget = SSE_MAX_LIFETIME.saturating_sub(started_at.elapsed());
                    let _ = ws_send_with_timeout(
                        &mut socket,
                        Some(axum::extract::ws::Message::Text(text.into())),
                        send_budget,
                    )
                    .await;
                }
                let _ = socket.close().await;
                return;
            }
        }
    }
}

/// Send a WS frame (or close, when `frame` is `None`) bounded by
/// `budget`. Returns `Err(())` on timeout, peer hangup, or close
/// error so callers can release the per-caller `SseSlot` instead of
/// hanging indefinitely on a stalled socket.
async fn ws_send_with_timeout(
    socket: &mut axum::extract::ws::WebSocket,
    frame: Option<axum::extract::ws::Message>,
    budget: std::time::Duration,
) -> Result<(), ()> {
    if budget.is_zero() {
        let _ = socket.close().await;
        return Err(());
    }
    let send_future = async {
        match frame {
            Some(message) => socket.send(message).await.map_err(|_| ()),
            None => socket.close().await.map_err(|_| ()),
        }
    };
    match tokio::time::timeout(budget, send_future).await {
        Ok(result) => result,
        Err(_elapsed) => {
            tracing::debug!(
                target = "ironclaw_webui_v2::ws",
                budget_ms = budget.as_millis() as u64,
                "WS send exceeded lifetime budget; releasing slot",
            );
            Err(())
        }
    }
}
