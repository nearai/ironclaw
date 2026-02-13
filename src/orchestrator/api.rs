//! Internal HTTP API for worker-to-orchestrator communication.
//!
//! This runs on a separate port (default 50051) from the web gateway.
//! All endpoints are authenticated via per-job bearer tokens.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, broadcast};
use uuid::Uuid;

use crate::channels::web::types::SseEvent;
use crate::history::Store;
use crate::llm::{CompletionRequest, LlmProvider, ToolCompletionRequest};
use crate::orchestrator::auth::{TokenStore, worker_auth_middleware};
use crate::orchestrator::job_manager::ContainerJobManager;
use crate::secrets::SecretsStore;
use crate::worker::api::JobEventPayload;
use crate::worker::api::{
    CompletionReport, CredentialResponse, JobDescription, ProxyCompletionRequest,
    ProxyCompletionResponse, ProxyToolCompletionRequest, ProxyToolCompletionResponse, StatusUpdate,
};

/// A follow-up prompt queued for a Claude Code bridge.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingPrompt {
    pub content: String,
    pub done: bool,
}

/// Shared state for the orchestrator API.
#[derive(Clone)]
pub struct OrchestratorState {
    pub llm: Arc<dyn LlmProvider>,
    pub job_manager: Arc<ContainerJobManager>,
    pub token_store: TokenStore,
    /// Broadcast channel for job events (consumed by the web gateway SSE).
    pub job_event_tx: Option<broadcast::Sender<(Uuid, SseEvent)>>,
    /// Buffered follow-up prompts for sandbox jobs, keyed by job_id.
    pub prompt_queue: Arc<Mutex<HashMap<Uuid, VecDeque<PendingPrompt>>>>,
    /// Database handle for persisting job events.
    pub store: Option<Arc<Store>>,
    /// Encrypted secrets store for credential injection into containers.
    pub secrets_store: Option<Arc<dyn SecretsStore + Send + Sync>>,
    /// User ID for secret lookups (single-tenant, typically "default").
    pub user_id: String,
}

/// The orchestrator's internal API server.
pub struct OrchestratorApi;

impl OrchestratorApi {
    /// Build the axum router for the internal API.
    pub fn router(state: OrchestratorState) -> Router {
        Router::new()
            // Worker routes: authenticated via route_layer middleware.
            .route("/worker/{job_id}/job", get(get_job))
            .route("/worker/{job_id}/llm/complete", post(llm_complete))
            .route(
                "/worker/{job_id}/llm/complete_with_tools",
                post(llm_complete_with_tools),
            )
            .route("/worker/{job_id}/status", post(report_status))
            .route("/worker/{job_id}/complete", post(report_complete))
            .route("/worker/{job_id}/event", post(job_event_handler))
            .route("/worker/{job_id}/prompt", get(get_prompt_handler))
            .route("/worker/{job_id}/credentials", get(get_credentials_handler))
            .route_layer(axum::middleware::from_fn_with_state(
                state.token_store.clone(),
                worker_auth_middleware,
            ))
            // Unauthenticated routes (added after the layer).
            .route("/health", get(health_check))
            .with_state(state)
    }

    /// Start the internal API server on the given port.
    ///
    /// On macOS/Windows (Docker Desktop), binds to loopback only because
    /// Docker Desktop routes `host.docker.internal` through its VM to the
    /// host's `127.0.0.1`.
    ///
    /// On Linux, containers reach the host via the docker bridge gateway
    /// (`172.17.0.1`), which is NOT loopback. Binding to `127.0.0.1`
    /// would reject container traffic. We bind to all interfaces instead
    /// and rely on `worker_auth_middleware` (applied as a route_layer on
    /// every `/worker/` endpoint) to reject unauthenticated requests.
    pub async fn start(
        state: OrchestratorState,
        port: u16,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let router = Self::router(state);
        let addr = if cfg!(target_os = "linux") {
            std::net::SocketAddr::from(([0, 0, 0, 0], port))
        } else {
            std::net::SocketAddr::from(([127, 0, 0, 1], port))
        };

        tracing::info!("Orchestrator internal API listening on {}", addr);

        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(listener, router).await?;

        Ok(())
    }
}

// -- Handlers --
//
// All /worker/ handlers below are behind the worker_auth_middleware route_layer,
// so they don't need to validate tokens themselves.

async fn health_check() -> &'static str {
    "ok"
}

async fn get_job(
    State(state): State<OrchestratorState>,
    Path(job_id): Path<Uuid>,
) -> Result<Json<JobDescription>, StatusCode> {
    let handle = state
        .job_manager
        .get_handle(job_id)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(JobDescription {
        title: format!("Job {}", job_id),
        description: handle.task_description,
        project_dir: handle.project_dir.map(|p| p.display().to_string()),
    }))
}

async fn llm_complete(
    State(state): State<OrchestratorState>,
    Path(job_id): Path<Uuid>,
    Json(req): Json<ProxyCompletionRequest>,
) -> Result<Json<ProxyCompletionResponse>, StatusCode> {
    let completion_req = CompletionRequest {
        messages: req.messages,
        max_tokens: req.max_tokens,
        temperature: req.temperature,
        stop_sequences: req.stop_sequences,
        metadata: std::collections::HashMap::new(),
    };

    let resp = state.llm.complete(completion_req).await.map_err(|e| {
        tracing::error!("LLM completion failed for job {}: {}", job_id, e);
        StatusCode::BAD_GATEWAY
    })?;

    Ok(Json(ProxyCompletionResponse {
        content: resp.content,
        input_tokens: resp.input_tokens,
        output_tokens: resp.output_tokens,
        finish_reason: format_finish_reason(resp.finish_reason),
    }))
}

async fn llm_complete_with_tools(
    State(state): State<OrchestratorState>,
    Path(job_id): Path<Uuid>,
    Json(req): Json<ProxyToolCompletionRequest>,
) -> Result<Json<ProxyToolCompletionResponse>, StatusCode> {
    let tool_req = ToolCompletionRequest {
        messages: req.messages,
        tools: req.tools,
        max_tokens: req.max_tokens,
        temperature: req.temperature,
        tool_choice: req.tool_choice,
        metadata: std::collections::HashMap::new(),
    };

    let resp = state.llm.complete_with_tools(tool_req).await.map_err(|e| {
        tracing::error!("LLM tool completion failed for job {}: {}", job_id, e);
        StatusCode::BAD_GATEWAY
    })?;

    Ok(Json(ProxyToolCompletionResponse {
        content: resp.content,
        tool_calls: resp.tool_calls,
        input_tokens: resp.input_tokens,
        output_tokens: resp.output_tokens,
        finish_reason: format_finish_reason(resp.finish_reason),
    }))
}

async fn report_status(
    State(state): State<OrchestratorState>,
    Path(job_id): Path<Uuid>,
    Json(update): Json<StatusUpdate>,
) -> Result<StatusCode, StatusCode> {
    tracing::debug!(
        job_id = %job_id,
        state = %update.state,
        iteration = update.iteration,
        "Worker status update"
    );

    state
        .job_manager
        .update_worker_status(job_id, update.message, update.iteration)
        .await;

    Ok(StatusCode::OK)
}

async fn report_complete(
    State(state): State<OrchestratorState>,
    Path(job_id): Path<Uuid>,
    Json(report): Json<CompletionReport>,
) -> Result<StatusCode, StatusCode> {
    if report.success {
        tracing::info!(
            job_id = %job_id,
            "Worker reported job complete"
        );
    } else {
        tracing::warn!(
            job_id = %job_id,
            message = ?report.message,
            "Worker reported job failure"
        );
    }

    // Store the result and clean up the container
    let result = crate::orchestrator::job_manager::CompletionResult {
        success: report.success,
        message: report.message.clone(),
    };
    if let Err(e) = state.job_manager.complete_job(job_id, result).await {
        tracing::error!(job_id = %job_id, "Failed to complete job cleanup: {}", e);
    }

    Ok(StatusCode::OK)
}

// -- Sandbox job event handlers --

/// Receive a job event from a worker or Claude Code bridge and broadcast + persist it.
async fn job_event_handler(
    State(state): State<OrchestratorState>,
    Path(job_id): Path<Uuid>,
    Json(payload): Json<JobEventPayload>,
) -> Result<StatusCode, StatusCode> {
    tracing::debug!(
        job_id = %job_id,
        event_type = %payload.event_type,
        "Job event received"
    );

    // Persist to DB (fire-and-forget)
    if let Some(ref store) = state.store {
        let store = Arc::clone(store);
        let event_type = payload.event_type.clone();
        let data = payload.data.clone();
        tokio::spawn(async move {
            if let Err(e) = store.save_job_event(job_id, &event_type, &data).await {
                tracing::warn!(job_id = %job_id, "Failed to persist job event: {}", e);
            }
        });
    }

    // Convert to SSE event and broadcast
    let job_id_str = job_id.to_string();
    let sse_event = match payload.event_type.as_str() {
        "message" => SseEvent::JobMessage {
            job_id: job_id_str,
            role: payload
                .data
                .get("role")
                .and_then(|v| v.as_str())
                .unwrap_or("assistant")
                .to_string(),
            content: payload
                .data
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        },
        "tool_use" => SseEvent::JobToolUse {
            job_id: job_id_str,
            tool_name: payload
                .data
                .get("tool_name")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string(),
            input: payload
                .data
                .get("input")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
        },
        "tool_result" => SseEvent::JobToolResult {
            job_id: job_id_str,
            tool_name: payload
                .data
                .get("tool_name")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string(),
            output: payload
                .data
                .get("output")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        },
        "result" => SseEvent::JobResult {
            job_id: job_id_str,
            status: payload
                .data
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string(),
            session_id: payload
                .data
                .get("session_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
        },
        _ => SseEvent::JobStatus {
            job_id: job_id_str,
            message: payload
                .data
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        },
    };

    // Broadcast via the channel (if configured)
    if let Some(ref tx) = state.job_event_tx {
        let _ = tx.send((job_id, sse_event));
    }

    Ok(StatusCode::OK)
}

/// Return the next queued follow-up prompt for a Claude Code bridge.
/// Returns 204 No Content if no prompt is available.
async fn get_prompt_handler(
    State(state): State<OrchestratorState>,
    Path(job_id): Path<Uuid>,
) -> Result<(StatusCode, Json<serde_json::Value>), StatusCode> {
    let mut queue = state.prompt_queue.lock().await;
    if let Some(prompts) = queue.get_mut(&job_id) {
        if let Some(prompt) = prompts.pop_front() {
            return Ok((
                StatusCode::OK,
                Json(serde_json::json!({
                    "content": prompt.content,
                    "done": prompt.done,
                })),
            ));
        }
    }

    // Return 204 with an empty body. The Json wrapper requires some value
    // but the status code signals "nothing here".
    Ok((StatusCode::NO_CONTENT, Json(serde_json::Value::Null)))
}

/// Serve decrypted credentials for a job's granted secrets.
///
/// Returns 204 if no grants exist, 503 if no secrets store is configured,
/// or a JSON array of `{ env_var, value }` pairs.
async fn get_credentials_handler(
    State(state): State<OrchestratorState>,
    Path(job_id): Path<Uuid>,
) -> Result<(StatusCode, Json<serde_json::Value>), StatusCode> {
    let grants = match state.token_store.get_grants(job_id).await {
        Some(g) if !g.is_empty() => g,
        _ => return Ok((StatusCode::NO_CONTENT, Json(serde_json::Value::Null))),
    };

    let secrets = state.secrets_store.as_ref().ok_or_else(|| {
        tracing::error!("Credentials requested but no secrets store configured");
        StatusCode::SERVICE_UNAVAILABLE
    })?;

    let mut credentials: Vec<CredentialResponse> = Vec::with_capacity(grants.len());

    for grant in &grants {
        let decrypted = secrets
            .get_decrypted(&state.user_id, &grant.secret_name)
            .await
            .map_err(|e| {
                tracing::error!(
                    job_id = %job_id,
                    secret = %grant.secret_name,
                    "Failed to decrypt secret for credential grant: {}", e
                );
                StatusCode::INTERNAL_SERVER_ERROR
            })?;

        // Record usage for audit trail
        if let Ok(secret) = secrets.get(&state.user_id, &grant.secret_name).await {
            if let Err(e) = secrets.record_usage(secret.id).await {
                tracing::warn!(
                    secret = %grant.secret_name,
                    "Failed to record credential usage: {}", e
                );
            }
        }

        tracing::debug!(
            job_id = %job_id,
            env_var = %grant.env_var,
            "Serving credential to container"
        );

        credentials.push(CredentialResponse {
            env_var: grant.env_var.clone(),
            value: decrypted.expose().to_string(),
        });
    }

    Ok((
        StatusCode::OK,
        Json(serde_json::to_value(&credentials).unwrap_or(serde_json::Value::Null)),
    ))
}

fn format_finish_reason(reason: crate::llm::FinishReason) -> String {
    match reason {
        crate::llm::FinishReason::Stop => "stop".to_string(),
        crate::llm::FinishReason::Length => "length".to_string(),
        crate::llm::FinishReason::ToolUse => "tool_use".to_string(),
        crate::llm::FinishReason::ContentFilter => "content_filter".to_string(),
        crate::llm::FinishReason::Unknown => "unknown".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;
    use uuid::Uuid;

    use crate::error::LlmError;
    use crate::llm::{
        CompletionRequest, CompletionResponse, ToolCompletionRequest, ToolCompletionResponse,
    };
    use crate::orchestrator::auth::TokenStore;
    use crate::orchestrator::job_manager::{ContainerJobConfig, ContainerJobManager};

    use super::*;

    /// Stub LLM provider that panics if called (tests only exercise routing/auth).
    struct StubLlm;

    #[async_trait::async_trait]
    impl crate::llm::LlmProvider for StubLlm {
        fn model_name(&self) -> &str {
            "stub"
        }
        fn cost_per_token(&self) -> (rust_decimal::Decimal, rust_decimal::Decimal) {
            (rust_decimal::Decimal::ZERO, rust_decimal::Decimal::ZERO)
        }
        async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
            Err(LlmError::RequestFailed {
                provider: "stub".into(),
                reason: "not implemented".into(),
            })
        }
        async fn complete_with_tools(
            &self,
            _req: ToolCompletionRequest,
        ) -> Result<ToolCompletionResponse, LlmError> {
            Err(LlmError::RequestFailed {
                provider: "stub".into(),
                reason: "not implemented".into(),
            })
        }
    }

    fn test_state() -> OrchestratorState {
        let token_store = TokenStore::new();
        let jm = ContainerJobManager::new(ContainerJobConfig::default(), token_store.clone());
        OrchestratorState {
            llm: Arc::new(StubLlm),
            job_manager: Arc::new(jm),
            token_store,
            job_event_tx: None,
            prompt_queue: Arc::new(Mutex::new(HashMap::new())),
            store: None,
            secrets_store: None,
            user_id: "default".to_string(),
        }
    }

    #[tokio::test]
    async fn health_requires_no_auth() {
        let state = test_state();
        let router = OrchestratorApi::router(state);

        let req = Request::builder()
            .uri("/health")
            .body(Body::empty())
            .unwrap();

        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn worker_route_rejects_missing_token() {
        let state = test_state();
        let router = OrchestratorApi::router(state);

        let job_id = Uuid::new_v4();
        let req = Request::builder()
            .uri(format!("/worker/{}/job", job_id))
            .body(Body::empty())
            .unwrap();

        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn worker_route_rejects_wrong_token() {
        let state = test_state();
        let router = OrchestratorApi::router(state);

        let job_id = Uuid::new_v4();
        let req = Request::builder()
            .uri(format!("/worker/{}/job", job_id))
            .header("Authorization", "Bearer totally-bogus")
            .body(Body::empty())
            .unwrap();

        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn worker_route_accepts_valid_token() {
        let state = test_state();
        let job_id = Uuid::new_v4();
        let token = state.token_store.create_token(job_id).await;

        let router = OrchestratorApi::router(state);

        let req = Request::builder()
            .uri(format!("/worker/{}/job", job_id))
            .header("Authorization", format!("Bearer {}", token))
            .body(Body::empty())
            .unwrap();

        let resp = router.oneshot(req).await.unwrap();
        // 404 because no container exists for this job_id, but NOT 401.
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn token_for_job_a_rejected_on_job_b() {
        let state = test_state();
        let job_a = Uuid::new_v4();
        let job_b = Uuid::new_v4();
        let token_a = state.token_store.create_token(job_a).await;

        let router = OrchestratorApi::router(state);

        // Use job_a's token to hit job_b's endpoint
        let req = Request::builder()
            .uri(format!("/worker/{}/job", job_b))
            .header("Authorization", format!("Bearer {}", token_a))
            .body(Body::empty())
            .unwrap();

        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // -- Prompt queue tests --

    #[tokio::test]
    async fn prompt_returns_204_when_queue_empty() {
        let state = test_state();
        let job_id = Uuid::new_v4();
        let token = state.token_store.create_token(job_id).await;
        let router = OrchestratorApi::router(state);

        let req = Request::builder()
            .uri(format!("/worker/{}/prompt", job_id))
            .header("Authorization", format!("Bearer {}", token))
            .body(Body::empty())
            .unwrap();

        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn prompt_returns_queued_prompt() {
        let state = test_state();
        let job_id = Uuid::new_v4();
        let token = state.token_store.create_token(job_id).await;

        // Queue a prompt
        {
            let mut q = state.prompt_queue.lock().await;
            q.entry(job_id).or_default().push_back(PendingPrompt {
                content: "What is the status?".to_string(),
                done: false,
            });
        }

        let router = OrchestratorApi::router(state);
        let req = Request::builder()
            .uri(format!("/worker/{}/prompt", job_id))
            .header("Authorization", format!("Bearer {}", token))
            .body(Body::empty())
            .unwrap();

        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["content"], "What is the status?");
        assert_eq!(json["done"], false);
    }

    // -- Credentials handler tests --

    #[tokio::test]
    async fn credentials_returns_204_when_no_grants() {
        let state = test_state();
        let job_id = Uuid::new_v4();
        let token = state.token_store.create_token(job_id).await;
        let router = OrchestratorApi::router(state);

        let req = Request::builder()
            .uri(format!("/worker/{}/credentials", job_id))
            .header("Authorization", format!("Bearer {}", token))
            .body(Body::empty())
            .unwrap();

        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn credentials_returns_503_when_no_secrets_store() {
        let state = test_state();
        let job_id = Uuid::new_v4();
        let token = state.token_store.create_token(job_id).await;

        // Store grants so we get past the 204 check
        state
            .token_store
            .store_grants(
                job_id,
                vec![crate::orchestrator::auth::CredentialGrant {
                    secret_name: "test_secret".to_string(),
                    env_var: "TEST_SECRET".to_string(),
                }],
            )
            .await;

        let router = OrchestratorApi::router(state);
        let req = Request::builder()
            .uri(format!("/worker/{}/credentials", job_id))
            .header("Authorization", format!("Bearer {}", token))
            .body(Body::empty())
            .unwrap();

        let resp = router.oneshot(req).await.unwrap();
        // No secrets_store configured → 503
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn credentials_returns_secrets_when_store_configured() {
        use secrecy::SecretString;
        let key = "0123456789abcdef0123456789abcdef";
        let crypto = Arc::new(
            crate::secrets::SecretsCrypto::new(SecretString::from(key.to_string())).unwrap(),
        );
        let secrets_store = Arc::new(crate::secrets::InMemorySecretsStore::new(crypto));

        // Create a secret
        secrets_store
            .create(
                "default",
                crate::secrets::CreateSecretParams {
                    name: "test_secret".to_string(),
                    value: SecretString::from("supersecretvalue".to_string()),
                    provider: None,
                    expires_at: None,
                },
            )
            .await
            .unwrap();

        let token_store = TokenStore::new();
        let jm = ContainerJobManager::new(ContainerJobConfig::default(), token_store.clone());
        let job_id = Uuid::new_v4();
        let token = token_store.create_token(job_id).await;
        token_store
            .store_grants(
                job_id,
                vec![crate::orchestrator::auth::CredentialGrant {
                    secret_name: "test_secret".to_string(),
                    env_var: "MY_SECRET".to_string(),
                }],
            )
            .await;

        let state = OrchestratorState {
            llm: Arc::new(StubLlm),
            job_manager: Arc::new(jm),
            token_store,
            job_event_tx: None,
            prompt_queue: Arc::new(Mutex::new(HashMap::new())),
            store: None,
            secrets_store: Some(secrets_store),
            user_id: "default".to_string(),
        };

        let router = OrchestratorApi::router(state);
        let req = Request::builder()
            .uri(format!("/worker/{}/credentials", job_id))
            .header("Authorization", format!("Bearer {}", token))
            .body(Body::empty())
            .unwrap();

        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let json: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
        assert_eq!(json.len(), 1);
        assert_eq!(json[0]["env_var"], "MY_SECRET");
        assert_eq!(json[0]["value"], "supersecretvalue");
    }

    // -- Job event handler tests --

    #[tokio::test]
    async fn job_event_broadcasts_message() {
        let (tx, mut rx) = broadcast::channel(16);
        let token_store = TokenStore::new();
        let jm = ContainerJobManager::new(ContainerJobConfig::default(), token_store.clone());
        let state = OrchestratorState {
            llm: Arc::new(StubLlm),
            job_manager: Arc::new(jm),
            token_store: token_store.clone(),
            job_event_tx: Some(tx),
            prompt_queue: Arc::new(Mutex::new(HashMap::new())),
            store: None,
            secrets_store: None,
            user_id: "default".to_string(),
        };

        let job_id = Uuid::new_v4();
        let token = token_store.create_token(job_id).await;
        let router = OrchestratorApi::router(state);

        let payload = serde_json::json!({
            "event_type": "message",
            "data": {
                "role": "assistant",
                "content": "Hello from worker"
            }
        });

        let req = Request::builder()
            .method("POST")
            .uri(format!("/worker/{}/event", job_id))
            .header("Authorization", format!("Bearer {}", token))
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_vec(&payload).unwrap()))
            .unwrap();

        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let (recv_id, event) = rx.recv().await.unwrap();
        assert_eq!(recv_id, job_id);
        match event {
            SseEvent::JobMessage {
                job_id: jid,
                role,
                content,
            } => {
                assert_eq!(jid, job_id.to_string());
                assert_eq!(role, "assistant");
                assert_eq!(content, "Hello from worker");
            }
            other => panic!("Expected JobMessage, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn job_event_handles_tool_use() {
        let (tx, mut rx) = broadcast::channel(16);
        let token_store = TokenStore::new();
        let jm = ContainerJobManager::new(ContainerJobConfig::default(), token_store.clone());
        let state = OrchestratorState {
            llm: Arc::new(StubLlm),
            job_manager: Arc::new(jm),
            token_store: token_store.clone(),
            job_event_tx: Some(tx),
            prompt_queue: Arc::new(Mutex::new(HashMap::new())),
            store: None,
            secrets_store: None,
            user_id: "default".to_string(),
        };

        let job_id = Uuid::new_v4();
        let token = token_store.create_token(job_id).await;
        let router = OrchestratorApi::router(state);

        let payload = serde_json::json!({
            "event_type": "tool_use",
            "data": {
                "tool_name": "shell",
                "input": {"command": "ls"}
            }
        });

        let req = Request::builder()
            .method("POST")
            .uri(format!("/worker/{}/event", job_id))
            .header("Authorization", format!("Bearer {}", token))
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_vec(&payload).unwrap()))
            .unwrap();

        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let (_recv_id, event) = rx.recv().await.unwrap();
        match event {
            SseEvent::JobToolUse { tool_name, .. } => {
                assert_eq!(tool_name, "shell");
            }
            other => panic!("Expected JobToolUse, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn job_event_handles_unknown_type() {
        let (tx, mut rx) = broadcast::channel(16);
        let token_store = TokenStore::new();
        let jm = ContainerJobManager::new(ContainerJobConfig::default(), token_store.clone());
        let state = OrchestratorState {
            llm: Arc::new(StubLlm),
            job_manager: Arc::new(jm),
            token_store: token_store.clone(),
            job_event_tx: Some(tx),
            prompt_queue: Arc::new(Mutex::new(HashMap::new())),
            store: None,
            secrets_store: None,
            user_id: "default".to_string(),
        };

        let job_id = Uuid::new_v4();
        let token = token_store.create_token(job_id).await;
        let router = OrchestratorApi::router(state);

        let payload = serde_json::json!({
            "event_type": "custom_thing",
            "data": { "message": "something custom" }
        });

        let req = Request::builder()
            .method("POST")
            .uri(format!("/worker/{}/event", job_id))
            .header("Authorization", format!("Bearer {}", token))
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_vec(&payload).unwrap()))
            .unwrap();

        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let (_recv_id, event) = rx.recv().await.unwrap();
        // Unknown event types fall through to JobStatus
        assert!(matches!(event, SseEvent::JobStatus { .. }));
    }

    // -- Status update test --

    #[tokio::test]
    async fn report_status_updates_handle() {
        let state = test_state();
        let job_id = Uuid::new_v4();
        let token = state.token_store.create_token(job_id).await;

        // Insert a handle so update_worker_status has something to update
        {
            let mut containers = state.job_manager.containers.write().await;
            containers.insert(
                job_id,
                crate::orchestrator::job_manager::ContainerHandle {
                    job_id,
                    container_id: "test-container".to_string(),
                    state: crate::orchestrator::job_manager::ContainerState::Running,
                    mode: crate::orchestrator::job_manager::JobMode::Worker,
                    created_at: chrono::Utc::now(),
                    project_dir: None,
                    task_description: "test".to_string(),
                    last_worker_status: None,
                    worker_iteration: 0,
                    completion_result: None,
                },
            );
        }

        let jm = Arc::clone(&state.job_manager);
        let router = OrchestratorApi::router(state);

        let update = serde_json::json!({
            "state": "in_progress",
            "message": "Iteration 5",
            "iteration": 5
        });

        let req = Request::builder()
            .method("POST")
            .uri(format!("/worker/{}/status", job_id))
            .header("Authorization", format!("Bearer {}", token))
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_vec(&update).unwrap()))
            .unwrap();

        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let handle = jm.get_handle(job_id).await.unwrap();
        assert_eq!(handle.worker_iteration, 5);
        assert_eq!(handle.last_worker_status.as_deref(), Some("Iteration 5"));
    }
}
