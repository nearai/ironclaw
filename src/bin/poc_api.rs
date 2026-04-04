//! Proof of Claw — standalone API server binary.
//!
//! Run with:  cargo run --bin poc_api
//! Or in mock mode (no external deps): MOCK_MODE=1 cargo run --bin poc_api

use anyhow::Result;
use axum::{
    extract::State,
    http::{Method, StatusCode},
    response::{IntoResponse, Json, Response},
    routing::{get, post},
    Router,
};
use proof_of_claw::{
    AgentConfig, AgentMessage, ExecutionTrace, InferenceRequest,
    MessagePayload, MessageType, PolicyEngine, ProofGenerator, ToolInvocation,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_http::cors::{Any, CorsLayer};
use tracing::{error, info};
use uuid::Uuid;

// ── App state ────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct AppState {
    agent: Arc<RwLock<Option<ProofOfClawAgent>>>,
    activity: Arc<RwLock<Vec<ActivityEntry>>>,
    proofs: Arc<RwLock<Vec<ProofEntry>>>,
    messages: Arc<RwLock<Vec<MessageEntry>>>,
    stats: Arc<RwLock<AgentStats>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ActivityEntry {
    id: String,
    timestamp: i64,
    action: String,
    details: String,
    within_policy: bool,
    tool: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProofEntry {
    id: String,
    timestamp: i64,
    session_id: String,
    agent_id: String,
    policy_hash: String,
    all_checks_passed: bool,
    requires_ledger: bool,
    action_value_wei: u64,
    output_commitment: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MessageEntry {
    id: String,
    timestamp: i64,
    direction: String,
    from: String,
    to: String,
    content: String,
    message_type: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct AgentStats {
    total_requests: u64,
    total_actions: u64,
    proofs_generated: u64,
    uptime_secs: i64,
    start_time: i64,
}

impl AppState {
    fn new() -> Self {
        Self {
            agent: Arc::new(RwLock::new(None)),
            activity: Arc::new(RwLock::new(Vec::new())),
            proofs: Arc::new(RwLock::new(Vec::new())),
            messages: Arc::new(RwLock::new(Vec::new())),
            stats: Arc::new(RwLock::new(AgentStats {
                start_time: chrono::Utc::now().timestamp(),
                ..Default::default()
            })),
        }
    }
}

// ── Agent ───────────────────────────────────────────────────────────────────

struct ProofOfClawAgent {
    id: String,
    config: AgentConfig,
    policy_engine: PolicyEngine,
    proof_generator: ProofGenerator,
}

impl ProofOfClawAgent {
    async fn new(config: AgentConfig) -> Result<Self> {
        let policy_engine = PolicyEngine::new(config.policy.clone());
        let proof_generator =
            ProofGenerator::new(false, config.risc_zero_image_id.clone().unwrap_or_default());

        Ok(Self {
            id: config.agent_id.clone(),
            config,
            policy_engine,
            proof_generator,
        })
    }

    async fn chat(&self, state: &AppState, input: ChatInput) -> Result<ChatOutput> {
        use proof_of_claw::{InferenceResponse, PolicySeverity};
        use proof_of_claw::ZeroGCompute;

        let timestamp = chrono::Utc::now().timestamp();
        let session_id = Uuid::new_v4().to_string();

        // ── Policy check ─────────────────────────────────────────────────
        let agent_msg = AgentMessage {
            message_type: MessageType::Execute,
            payload: MessagePayload {
                action: input.action.clone().unwrap_or_else(|| "query".to_string()),
                params: HashMap::from_iter([("message".to_string(), serde_json::json!(input.message))]),
                trace_root_hash: None,
                proof_receipt: None,
                required_approval: None,
            },
            nonce: 0,
            timestamp,
        };

        let mock_inference = InferenceResponse {
            content: String::new(),
            attestation_signature: String::new(),
            provider: String::new(),
        };
        let policy_result = self.policy_engine.check(&agent_msg, &mock_inference);

        // ── 0G Compute inference ─────────────────────────────────────────
        let zero_g = ZeroGCompute::new(&self.config).await?;
        let inference_resp = zero_g
            .inference(&InferenceRequest {
                system_prompt: input.system_prompt.clone().unwrap_or_else(|| {
                    "You are a provable AI agent. Every action is cryptographically verifiable."
                        .to_string()
                }),
                user_prompt: input.message.clone(),
                model: input.model.clone(),
            })
            .await?;

        // ── Build execution trace ────────────────────────────────────────
        let trace = ExecutionTrace {
            agent_id: self.id.clone(),
            session_id: session_id.clone(),
            timestamp,
            inference_commitment: inference_resp.attestation_signature.clone(),
            tool_invocations: vec![ToolInvocation {
                tool_name: agent_msg.payload.action.clone(),
                input_hash: format!("0x{}", hex::encode(Sha256::digest(&input.message))),
                output_hash: format!(
                    "0x{}",
                    hex::encode(Sha256::digest(&inference_resp.content))
                ),
                capability_hash: format!(
                    "0x{}",
                    hex::encode(Sha256::digest(agent_msg.payload.action.as_bytes()))
                ),
                timestamp,
                within_policy: policy_result.severity != PolicySeverity::Block,
            }],
            policy_check_results: vec![policy_result.clone()],
            output_commitment: format!(
                "0x{}",
                hex::encode(Sha256::digest(&inference_resp.content))
            ),
        };

        // ── Upload trace to 0G Storage ─────────────────────────────────
        let trace_root = {
            use proof_of_claw::ZeroGStorage;
            let storage = ZeroGStorage::new(&self.config).await?;
            storage.store_trace(&trace).await?
        };

        // ── Generate proof ──────────────────────────────────────────────
        let proof_receipt = self.proof_generator.generate_proof(&trace).await?;
        let verified = self.proof_generator.verify_receipt(&proof_receipt)?;

        // ── Record in state ─────────────────────────────────────────────
        {
            let mut stats = state.stats.write().await;
            stats.total_requests += 1;
            stats.total_actions += 1;
            stats.proofs_generated += 1;
        }

        let proof_id = Uuid::new_v4().to_string();

        let activity_entry = ActivityEntry {
            id: Uuid::new_v4().to_string(),
            timestamp,
            action: agent_msg.payload.action.clone(),
            details: format!(
                "Policy: {:?} | Ledger required: {}",
                policy_result.severity, verified.requires_ledger_approval
            ),
            within_policy: policy_result.severity != PolicySeverity::Block,
            tool: Some(agent_msg.payload.action),
        };
        state.activity.write().await.push(activity_entry);

        let proof_entry = ProofEntry {
            id: proof_id.clone(),
            timestamp,
            session_id: session_id.clone(),
            agent_id: self.id.clone(),
            policy_hash: verified.policy_hash,
            all_checks_passed: verified.all_checks_passed,
            requires_ledger: verified.requires_ledger_approval,
            action_value_wei: verified.action_value,
            output_commitment: trace.output_commitment,
        };
        state.proofs.write().await.push(proof_entry);

        Ok(ChatOutput {
            session_id,
            response: inference_resp.content,
            attestation: inference_resp.attestation_signature,
            proof_id,
            trace_root,
            policy_result,
            requires_ledger: verified.requires_ledger_approval,
        })
    }
}

// ── HTTP server ─────────────────────────────────────────────────────────────

async fn start_api_server(state: AppState, port: u16) -> Result<()> {
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
        .route("/api/messages", get(messages))
        .route("/api/messages/send", post(send_message))
        .layer(cors)
        .with_state(state);

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    info!("Proof of Claw API server listening on http://{addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

// ── Handlers ────────────────────────────────────────────────────────────────

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok", "service": "proof-of-claw" }))
}

async fn status(State(state): State<AppState>) -> Result<Json<StatusResponse>, AppError> {
    let agent = state.agent.read().await;
    let agent = agent.as_ref().ok_or(AppError::AgentNotInitialized)?;
    let stats = state.stats.read().await.clone();
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

#[derive(Debug, Deserialize)]
struct ChatInput {
    pub message: String,
    pub action: Option<String>,
    pub system_prompt: Option<String>,
    pub model: Option<String>,
}

async fn chat(
    State(state): State<AppState>,
    Json(input): Json<ChatInput>,
) -> Result<Json<ChatOutput>, AppError> {
    let agent = state.agent.read().await;
    let agent = agent.as_ref().ok_or(AppError::AgentNotInitialized)?;

    let output = agent.chat(&state, input).await.map_err(|e| {
        error!("chat error: {}", e);
        AppError::Internal(e.to_string())
    })?;

    Ok(Json(output))
}

async fn activity(State(state): State<AppState>) -> Result<Json<ActivityResponse>, AppError> {
    let entries = state.activity.read().await.clone();
    Ok(Json(ActivityResponse { entries }))
}

async fn proofs(State(state): State<AppState>) -> Result<Json<ProofsResponse>, AppError> {
    let entries = state.proofs.read().await.clone();
    Ok(Json(ProofsResponse { proofs: entries }))
}

async fn messages(State(state): State<AppState>) -> Result<Json<MessagesResponse>, AppError> {
    let entries = state.messages.read().await.clone();
    Ok(Json(MessagesResponse { messages: entries }))
}

#[derive(Debug, Deserialize)]
struct SendMessageInput {
    pub to: String,
    pub content: String,
}

async fn send_message(
    State(state): State<AppState>,
    Json(input): Json<SendMessageInput>,
) -> Result<Json<SendMessageResponse>, AppError> {
    let agent = state.agent.read().await;
    let agent = agent.as_ref().ok_or(AppError::AgentNotInitialized)?;

    let from = agent.config.ens_name.clone();
    let timestamp = chrono::Utc::now().timestamp();

    state.messages.write().await.push(MessageEntry {
        id: Uuid::new_v4().to_string(),
        timestamp,
        direction: "outbound".to_string(),
        from: from.clone(),
        to: input.to.clone(),
        content: input.content.clone(),
        message_type: "chat".to_string(),
    });

    // TODO: Send via DM3 delivery service when configured and not in mock mode
    if !agent.config.is_mock {
        info!(
            "DM3 send to {} from {} — delivery service integration pending",
            input.to, from
        );
    }

    Ok(Json(SendMessageResponse {
        success: true,
        message_id: Uuid::new_v4().to_string(),
        timestamp,
        note: if agent.config.is_mock {
            "mock mode — DM3 delivery not available"
        } else {
            "queued for DM3 delivery"
        },
    }))
}

// ── Response types ─────────────────────────────────────────────────────────

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
    entries: Vec<ActivityEntry>,
}

#[derive(Serialize)]
struct ProofsResponse {
    proofs: Vec<ProofEntry>,
}

#[derive(Serialize)]
struct MessagesResponse {
    messages: Vec<MessageEntry>,
}

#[derive(Serialize)]
struct SendMessageResponse {
    success: bool,
    message_id: String,
    timestamp: i64,
    note: &'static str,
}

#[derive(Serialize)]
struct ChatOutput {
    session_id: String,
    response: String,
    attestation: String,
    proof_id: String,
    trace_root: String,
    policy_result: proof_of_claw::PolicyResult,
    requires_ledger: bool,
}

// ── Error handling ──────────────────────────────────────────────────────────

enum AppError {
    AgentNotInitialized,
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            Self::AgentNotInitialized => (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({ "error": "agent not initialized" })),
            )
                .into_response(),
            Self::Internal(msg) => (
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
            Self::Internal(s) => write!(f, "Internal({s})"),
        }
    }
}

// ── Entry point ─────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    // Load .env before any config reads
    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt::init();

    info!("Starting Proof of Claw API Server");

    let mock_mode = std::env::var("MOCK_MODE")
        .unwrap_or_default()
        .eq_ignore_ascii_case("1")
        || std::env::var("MOCK_MODE")
            .unwrap_or_default()
            .eq_ignore_ascii_case("true");

    let config = if mock_mode {
        info!("MOCK mode — using default config, external services optional");
        AgentConfig::mock()?
    } else {
        info!("LIVE mode — loading config from environment");
        AgentConfig::from_env()?
    };

    let agent = ProofOfClawAgent::new(config).await?;

    info!(
        "Agent initialized: id={} ens={} mock={}",
        agent.id,
        agent.config.ens_name,
        agent.config.is_mock
    );

    let state = AppState::new();
    *state.agent.write().await = Some(agent);

    let port: u16 = std::env::var("API_PORT")
        .unwrap_or_else(|_| "8420".to_string())
        .parse()
        .unwrap_or(8420);

    start_api_server(state, port).await
}
