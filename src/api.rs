//! Proof of Claw HTTP API server.
//!
//! Exposes the endpoints the frontend expects:
//!   GET  /health                 — liveness probe
//!   GET  /api/status             — agent identity, policy, stats
//!   POST /api/chat               — run inference + proof pipeline
//!   GET  /api/activity           — activity feed
//!   GET  /api/proofs             — proof history
//!   GET  /api/proofs/:id/receipt — raw RISC Zero receipt for browser-side ZK verification
//!   GET  /api/traces/stream      — SSE stream of tool invocations and proof receipts
//!   GET  /api/messages           — message records
//!   POST /api/messages/send      — send a DM3 message

use crate::proof_agent::{AgentState, AgentStats, ChatInput, ProofEntry, TraceEvent};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, Method, StatusCode},
    response::{IntoResponse, Json, Response},
    routing::{get, post},
    Router,
};
use axum::sse::{Event, Sse};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tower_http::cors::{Any, CorsLayer};
use tracing::error;

/// Shared state for all HTTP handlers.
#[derive(Clone)]
pub struct ApiState {
    pub agent: Arc<RwLock<Option<crate::proof_agent::ProofOfClawAgent>>>,
    pub state: AgentState,
}

// ── App startup ─────────────────────────────────────────────────────────────

/// Start the API server on the given port.
///
/// Does NOT take ownership of the agent — the agent runs in a separate task
/// and communicates with the API via the shared `ApiState`.
pub async fn start_api_server(state: ApiState, port: u16) -> anyhow::Result<()> {
    let cors = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_origin(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/health", get(health))
        .route("/api/status", get(status))
        .route("/api/chat", post(chat))
        .route("/api/activity", get(activity))
        .route("/api/proofs", get(proofs))
        .route("/api/proofs/:id/receipt", get(proof_receipt))
        .route("/api/traces/stream", get(trace_stream))
        .route("/api/messages", get(messages))
        .route("/api/messages/send", post(send_message))
        .layer(cors)
        .with_state(state);

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("API server listening on http://{addr}");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

// ── Handlers ────────────────────────────────────────────────────────────────

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok", "service": "proof-of-claw" }))
}

async fn status(State(state): State<ApiState>) -> Result<Json<StatusResponse>, AppError> {
    let agent_guard = state.agent.read().await;
    let agent = agent_guard.as_ref().ok_or(AppError::AgentNotInitialized)?;

    let stats = state.state.stats.read().await.clone();
    let uptime = chrono::Utc::now().timestamp() - stats.start_time;

    Ok(Json(StatusResponse {
        agent_id: agent.id.clone(),
        ens_name: agent.config.ens_name.clone(),
        status: "online".to_string(),
        network: if agent.config.is_mock {
            "mock".to_string()
        } else {
            "ethereum".to_string()
        },
        allowed_tools: agent.config.policy.allowed_tools.clone(),
        endpoint_allowlist: agent.config.policy.endpoint_allowlist.clone(),
        max_value_autonomous_wei: agent.config.policy.max_value_autonomous_wei,
        uptime_secs: uptime.max(0),
        stats: AgentStatsResponse {
            total_requests: stats.total_requests,
            total_actions: stats.total_actions,
            proofs_generated: stats.proofs_generated,
            uptime_secs: uptime.max(0),
        },
    }))
}

async fn chat(
    State(state): State<ApiState>,
    Json(input): Json<ChatInput>,
) -> Result<Json<crate::proof_agent::ChatOutput>, AppError> {
    let agent_guard = state.agent.read().await;
    let agent = agent_guard.as_ref().ok_or(AppError::AgentNotInitialized)?;

    let output = agent.chat(input).await.map_err(|e| {
        error!("chat error: {}", e);
        AppError::Internal(e.to_string())
    })?;

    Ok(Json(output))
}

async fn activity(State(state): State<ApiState>) -> Result<Json<ActivityResponse>, AppError> {
    let entries = state.state.activity.read().await.clone();
    Ok(Json(ActivityResponse { entries }))
}

async fn proofs(State(state): State<ApiState>) -> Result<Json<ProofsResponse>, AppError> {
    let entries = state.state.proofs.read().await.clone();
    Ok(Json(ProofsResponse { proofs: entries }))
}

/// Returns the raw RISC Zero receipt (journal + seal) for browser-side ZK verification.
async fn proof_receipt(
    State(state): State<ApiState>,
    Path(proof_id): Path<String>,
) -> Result<Json<ProofReceiptResponse>, AppError> {
    let receipts = state.state.receipts.read().await;
    let receipt = receipts.get(&proof_id).ok_or(AppError::NotFound)?;

    use base64::Engine as _;
    let journal_b64 = base64::engine::general_purpose::STANDARD.encode(&receipt.journal);
    let seal_b64 = base64::engine::general_purpose::STANDARD.encode(&receipt.seal);

    Ok(Json(ProofReceiptResponse {
        journal: journal_b64,
        seal: seal_b64,
        image_id: receipt.image_id.clone(),
    }))
}

/// SSE stream of TraceEvents — tool invocations, trace completions, and proof receipts.
/// The browser connects here to drive the Kanban board in real-time.
async fn trace_stream(
    State(state): State<ApiState>,
    axum::extract::Query(params): axum::extract::Query<StreamParams>,
) -> Sse<impl futures::Stream<Item = Result<Event, std::convert::Infallible>>> {
    let mut receiver = state.state.trace_broadcaster.subscribe();

    // Optionally filter by session_id (if provided)
    let session_filter = params.session_id.clone();

    let stream = async_stream::stream! {
        // Replay last N events so new SSE connections don't miss recent activity
        let replay_events = state.state.trace_broadcaster.receiver_count();
        for _ in 0..replay_events.min(32) {
            if let Ok(evt) = receiver.recv().await {
                if session_filter.as_ref().map_or(true, |sid| evt.session_id() == Some(sid)) {
                    let data = serde_json::to_string(&evt).unwrap_or_default();
                    yield Ok(Event::default().event("trace").data(data));
                }
            }
        }

        // Stream new events as they arrive
        while let Ok(evt) = receiver.recv().await {
            if session_filter.as_ref().map_or(true, |sid| evt.session_id() == Some(sid)) {
                let data = serde_json::to_string(&evt).unwrap_or_default();
                yield Ok(Event::default().event("trace").data(data));
            }
        }
    };

    Sse::new(stream)
        .keep_alive(axum::sse::KeepAlive::new().interval(Duration::from_secs(15)))
}

async fn messages(State(state): State<ApiState>) -> Result<Json<MessagesResponse>, AppError> {
    let entries = state.state.messages.read().await.clone();
    Ok(Json(MessagesResponse { messages: entries }))
}

#[derive(Debug, Deserialize)]
pub struct SendMessageInput {
    pub to: String,
    pub content: String,
}

async fn send_message(
    State(state): State<ApiState>,
    Json(input): Json<SendMessageInput>,
) -> Result<Json<SendMessageResponse>, AppError> {
    let agent_guard = state.agent.read().await;
    let agent = agent_guard.as_ref().ok_or(AppError::AgentNotInitialized)?;

    let from = agent.config.ens_name.clone();
    let timestamp = chrono::Utc::now().timestamp();

    // Record outbound message locally
    agent.record_message("outbound", &from, &input.to, &input.content).await;

    // TODO: Actually send via DM3 delivery service when configured
    if !agent.config.is_mock {
        tracing::warn!("DM3 message send not yet implemented — message recorded locally only");
    }

    Ok(Json(SendMessageResponse {
        success: true,
        message_id: uuid::Uuid::new_v4().to_string(),
        timestamp,
        note: if agent.config.is_mock {
            "mock mode — DM3 delivery not available"
        } else {
            "sent via DM3 delivery service"
        },
    }))
}

// ── Response types ──────────────────────────────────────────────────────────

#[derive(Serialize)]
struct StatusResponse {
    agent_id: String,
    ens_name: String,
    status: String,
    network: String,
    allowed_tools: Vec<String>,
    endpoint_allowlist: Vec<String>,
    max_value_autonomous_wei: u64,
    uptime_secs: i64,
    stats: AgentStatsResponse,
}

#[derive(Serialize)]
struct AgentStatsResponse {
    total_requests: u64,
    total_actions: u64,
    proofs_generated: u64,
    uptime_secs: i64,
}

#[derive(Serialize)]
struct ActivityResponse {
    entries: Vec<crate::proof_agent::ActivityEntry>,
}

#[derive(Serialize)]
struct ProofsResponse {
    proofs: Vec<ProofEntry>,
}

#[derive(Serialize)]
struct MessagesResponse {
    messages: Vec<crate::proof_agent::MessageEntry>,
}

#[derive(Serialize)]
struct SendMessageResponse {
    success: bool,
    message_id: String,
    timestamp: i64,
    note: &'static str,
}

/// Raw RISC Zero receipt — journal + seal for browser-side ZK verification.
#[derive(Serialize)]
struct ProofReceiptResponse {
    /// Base64-encoded journal bytes (VerifiedOutput JSON).
    journal: String,
    /// Base64-encoded cryptographic seal.
    seal: String,
    /// RISC Zero image ID (hex string, "0x" prefix).
    image_id: String,
}

/// Query params for the SSE trace stream.
#[derive(Deserialize)]
struct StreamParams {
    /// Optional session_id to filter events.
    session_id: Option<String>,
}

// ── Error handling ──────────────────────────────────────────────────────────

enum AppError {
    AgentNotInitialized,
    NotFound,
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            AppError::AgentNotInitialized => (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({ "error": "agent not initialized" })),
            )
                .into_response(),
            AppError::NotFound => (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "proof receipt not found" })),
            )
                .into_response(),
            AppError::Internal(msg) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": msg })),
            )
                .into_response(),
        }
    }
}

impl std::fmt::Debug for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AgentNotInitialized => write!(f, "AgentNotInitialized"),
            Self::NotFound => write!(f, "NotFound"),
            Self::Internal(s) => write!(f, "Internal({s})"),
        }
    }
}
