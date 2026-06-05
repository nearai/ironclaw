//! Runtime log streaming and log-level control.
//!
//! Owns `GET /api/logs/events` (SSE stream of live log entries prefixed with
//! DB history) and `GET/PUT /api/logs/level` (runtime log-level knob). The
//! slice is deliberately tiny — logs have no cross-cutting state, no shared
//! helpers, and only ever read from [`GatewayState::log_broadcaster`] /
//! [`GatewayState::log_level_handle`] / [`GatewayState::store`].

use std::convert::Infallible;
use std::sync::Arc;

use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{
        IntoResponse, Sse,
        sse::{Event, KeepAlive},
    },
};
use tokio_stream::StreamExt;

use crate::channels::web::auth::AuthenticatedUser;
use crate::channels::web::log_layer::LogEntry;
use crate::channels::web::platform::state::GatewayState;

/// How many log entries to load from DB when the page opens.
const LOG_HISTORY_LIMIT: i64 = 300;

/// `GET /api/logs/events` — SSE stream of log entries.
///
/// Loads the most recent `LOG_HISTORY_LIMIT` entries from DB as the history
/// prefix (info+ only), then forwards every new entry from the live broadcaster
/// (all levels). The subscription is opened before the DB query so no live
/// entries fall through the gap.
pub(crate) async fn logs_events_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(_user): AuthenticatedUser,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let broadcaster = state.log_broadcaster.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Log broadcaster not available".to_string(),
    ))?;

    // Subscribe before loading history so no live entries are missed.
    let rx = broadcaster.subscribe();

    // Load persisted history from DB (best-effort; empty slice if DB unavailable).
    let history: Vec<LogEntry> = if let Some(ref db) = state.store {
        db.list_log_entries(LOG_HISTORY_LIMIT)
            .await
            .unwrap_or_default() // silent-ok: dashboard refresh, live stream still works
            .into_iter()
            .map(|r| LogEntry {
                level: r.level,
                target: r.target,
                message: r.message,
                timestamp: r
                    .recorded_at
                    .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            })
            .collect()
    } else {
        Vec::new()
    };

    let history_stream = futures::stream::iter(history).map(|entry| {
        let data = serde_json::to_string(&entry).unwrap_or_default();
        Ok::<_, Infallible>(Event::default().event("log").data(data))
    });

    let live_stream = tokio_stream::wrappers::BroadcastStream::new(rx)
        .filter_map(|result| result.ok())
        .map(|entry| {
            let data = serde_json::to_string(&entry).unwrap_or_default();
            Ok::<_, Infallible>(Event::default().event("log").data(data))
        });

    let stream = history_stream.chain(live_stream);

    Ok((
        [("X-Accel-Buffering", "no"), ("Cache-Control", "no-cache")],
        Sse::new(stream).keep_alive(
            KeepAlive::new()
                .interval(std::time::Duration::from_secs(30))
                .text(""),
        ),
    ))
}

/// `GET /api/logs/level` — return the current log level.
pub(crate) async fn logs_level_get_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(_user): AuthenticatedUser,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let handle = state.log_level_handle.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Log level control not available".to_string(),
    ))?;
    Ok(Json(serde_json::json!({ "level": handle.current_level() })))
}

/// `PUT /api/logs/level` — set the log level at runtime.
pub(crate) async fn logs_level_set_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let handle = state.log_level_handle.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Log level control not available".to_string(),
    ))?;

    let level = body
        .get("level")
        .and_then(|v| v.as_str())
        .ok_or((StatusCode::BAD_REQUEST, "missing 'level' field".to_string()))?;

    handle
        .set_level(level)
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;

    tracing::info!(user_id = %user.user_id, "Log level changed to '{}'", handle.current_level());
    Ok(Json(serde_json::json!({ "level": handle.current_level() })))
}
