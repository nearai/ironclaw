//! Gateway status endpoint.
//!
//! Owns `GET /api/gateway/status` — a read-only snapshot of runtime state
//! (uptime, connection counts, cost / usage aggregates, active config) for
//! the admin dashboard. The response DTO ([`GatewayStatusResponse`]) lives
//! alongside the handler because nothing outside this slice consumes it
//! directly; the wire shape is the slice's public contract with the
//! browser.

use std::sync::Arc;

use axum::{Json, extract::State};
use serde::Serialize;

use crate::channels::web::auth::AuthenticatedUser;
use crate::channels::web::platform::state::GatewayState;

#[derive(Serialize)]
struct ModelUsageEntry {
    model: String,
    input_tokens: u64,
    output_tokens: u64,
    cost: String,
}

#[derive(Serialize)]
pub(crate) struct GatewayStatusResponse {
    version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    commit_hash: Option<String>,
    sse_connections: u64,
    ws_connections: u64,
    total_connections: u64,
    uptime_secs: u64,
    engine_v2: bool,
    restart_enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    daily_cost: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    actions_this_hour: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model_usage: Option<Vec<ModelUsageEntry>>,
    llm_backend: String,
    llm_model: String,
    enabled_channels: Vec<String>,
    engine_v2_enabled: bool,
}

/// `GET /api/gateway/status` — runtime snapshot for the admin dashboard.
pub(crate) async fn gateway_status_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(_user): AuthenticatedUser,
) -> Json<GatewayStatusResponse> {
    let sse_connections = state.sse.connection_count();
    let ws_connections = state
        .ws_tracker
        .as_ref()
        .map(|t| t.connection_count())
        .unwrap_or(0);

    let uptime_secs = state.startup_time.elapsed().as_secs();

    let (daily_cost, actions_this_hour, model_usage) = if let Some(ref cg) = state.cost_guard {
        let cost = cg.daily_spend().await;
        let actions = cg.actions_this_hour().await;
        let usage = cg.model_usage().await;
        let models: Vec<ModelUsageEntry> = usage
            .into_iter()
            .map(|(model, tokens)| ModelUsageEntry {
                model,
                input_tokens: tokens.input_tokens,
                output_tokens: tokens.output_tokens,
                cost: format!("{:.6}", tokens.cost),
            })
            .collect();
        (Some(format!("{:.4}", cost)), Some(actions), Some(models))
    } else {
        (None, None, None)
    };

    let restart_enabled = std::env::var("IRONCLAW_IN_DOCKER")
        .map(|v| v.to_lowercase() == "true")
        .unwrap_or(false);

    let commit_hash = {
        let h = env!("GIT_COMMIT_HASH");
        if h.is_empty() {
            None
        } else {
            let dirty = env!("GIT_DIRTY") == "true";
            Some(if dirty {
                format!("{h}-dirty")
            } else {
                h.to_string()
            })
        }
    };

    Json(GatewayStatusResponse {
        version: env!("CARGO_PKG_VERSION").to_string(),
        commit_hash,
        sse_connections,
        ws_connections,
        total_connections: sse_connections + ws_connections,
        uptime_secs,
        engine_v2: crate::bridge::is_engine_v2_enabled(),
        restart_enabled,
        daily_cost,
        actions_this_hour,
        model_usage,
        llm_backend: state.active_config.llm_backend.clone(),
        llm_model: state.active_config.llm_model.clone(),
        enabled_channels: state.active_config.enabled_channels.clone(),
        engine_v2_enabled: crate::bridge::is_engine_v2_enabled(),
    })
}
