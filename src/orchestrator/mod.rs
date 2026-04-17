//! Orchestrator for managing sandboxed worker containers.
//!
//! The orchestrator runs in the main agent process and provides:
//! - An internal HTTP API for worker communication (LLM proxy, status, secrets)
//! - Per-job bearer token authentication
//! - Container lifecycle management (create, monitor, stop)
//!
//! ```text
//! ┌───────────────────────────────────────────────┐
//! │              Orchestrator                       │
//! │                                                 │
//! │  Internal API (default :50051, configurable)    │
//! │    POST /worker/{id}/llm/complete               │
//! │    POST /worker/{id}/llm/complete_with_tools    │
//! │    GET  /worker/{id}/job                        │
//! │    GET  /worker/{id}/credentials                │
//! │    POST /worker/{id}/status                     │
//! │    POST /worker/{id}/complete                   │
//! │                                                 │
//! │  ContainerJobManager                            │
//! │    create_job() -> container + token             │
//! │    stop_job()                                    │
//! │    list_jobs()                                   │
//! │                                                 │
//! │  TokenStore                                     │
//! │    per-job bearer tokens (in-memory only)       │
//! │    per-job credential grants (in-memory only)   │
//! └───────────────────────────────────────────────┘
//! ```

pub mod api;
pub mod auth;
pub mod job_manager;
pub mod reaper;

pub use api::OrchestratorApi;
pub use auth::{CredentialGrant, TokenStore};
pub use job_manager::{
    CompletionResult, ContainerHandle, ContainerJobConfig, ContainerJobManager, JobMode,
};
pub use reaper::{ReaperConfig, SandboxReaper};

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use tokio::sync::{Mutex, broadcast};
use uuid::Uuid;

use crate::db::Database;
use crate::llm::LlmProvider;
use crate::secrets::SecretsStore;
use ironclaw_common::AppEvent;

/// Resolve the orchestrator port from the `ORCHESTRATOR_PORT` environment
/// variable, falling back to 50051.
fn resolve_orchestrator_port() -> u16 {
    std::env::var("ORCHESTRATOR_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(50051)
}

/// Result of orchestrator setup, containing all handles needed by the agent.
pub struct OrchestratorSetup {
    pub container_job_manager: Option<Arc<ContainerJobManager>>,
    pub job_event_tx: Option<broadcast::Sender<(Uuid, String, AppEvent)>>,
    pub prompt_queue: Arc<Mutex<HashMap<Uuid, VecDeque<api::PendingPrompt>>>>,
    pub docker_status: crate::sandbox::DockerStatus,
}

/// Detect Docker availability, create the container job manager, and start
/// the orchestrator internal API in the background.
pub async fn setup_orchestrator(
    config: &crate::config::Config,
    llm: &Arc<dyn LlmProvider>,
    db: Option<&Arc<dyn Database>>,
    secrets_store: Option<&Arc<dyn SecretsStore + Send + Sync>>,
) -> OrchestratorSetup {
    let prompt_queue = Arc::new(Mutex::new(
        HashMap::<Uuid, VecDeque<api::PendingPrompt>>::new(),
    ));

    let docker_status = if config.sandbox.enabled {
        let detection = crate::sandbox::check_docker().await;
        match detection.status {
            crate::sandbox::DockerStatus::Available => {
                tracing::info!("Docker is available");
            }
            crate::sandbox::DockerStatus::NotInstalled => {
                tracing::warn!(
                    "Docker is not installed -- sandbox disabled for this session. {}",
                    detection.platform.install_hint()
                );
            }
            crate::sandbox::DockerStatus::NotRunning => {
                tracing::warn!(
                    "Docker is installed but not running -- sandbox disabled for this session. {}",
                    detection.platform.start_hint()
                );
            }
            crate::sandbox::DockerStatus::Disabled => {}
        }
        detection.status
    } else {
        crate::sandbox::DockerStatus::Disabled
    };

    let (job_event_tx, container_job_manager) = if config.sandbox.enabled && docker_status.is_ok() {
        let (tx, _) = broadcast::channel(256);
        let job_event_tx = Some(tx);

        let token_store = TokenStore::new();
        let orchestrator_port = resolve_orchestrator_port();
        let job_config = ContainerJobConfig {
            image: config.sandbox.image.clone(),
            memory_limit_mb: config.sandbox.memory_limit_mb,
            cpu_shares: config.sandbox.cpu_shares,
            orchestrator_port,
            claude_code_api_key: std::env::var("ANTHROPIC_API_KEY").ok(),
            claude_code_oauth_token: crate::config::ClaudeCodeConfig::extract_oauth_token(),
            claude_code_model: config.claude_code.model.clone(),
            claude_code_max_turns: config.claude_code.max_turns,
            claude_code_memory_limit_mb: config.claude_code.memory_limit_mb,
            claude_code_allowed_tools: config.claude_code.allowed_tools.clone(),
            drift: config.agent.drift.clone(),
            acp_memory_limit_mb: config.acp.memory_limit_mb,
            acp_timeout_secs: config.acp.timeout_secs,
            mcp_per_job_enabled: std::env::var("MCP_PER_JOB_ENABLED")
                .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
                .unwrap_or(false),
            claude_code_enabled: config.claude_code.enabled,
            acp_enabled: config.acp.enabled,
        };
        let jm = Arc::new(ContainerJobManager::new(job_config, token_store.clone()));

        let orchestrator_state = api::OrchestratorState {
            llm: Arc::clone(llm),
            job_manager: Arc::clone(&jm),
            token_store,
            job_event_tx: job_event_tx.clone(),
            prompt_queue: Arc::clone(&prompt_queue),
            store: db.cloned(),
            secrets_store: secrets_store.cloned(),
            user_id: config.owner_id.clone(),
            job_owner_cache: Arc::new(std::sync::RwLock::new(std::collections::HashMap::new())),
        };

        tokio::spawn(async move {
            if let Err(e) = OrchestratorApi::start(orchestrator_state, orchestrator_port).await {
                tracing::error!("Orchestrator API failed: {}", e);
            }
        });

        if config.claude_code.enabled {
            tracing::info!(
                "Claude Code sandbox mode available (model: {}, max_turns: {})",
                config.claude_code.model,
                config.claude_code.max_turns
            );
        }
        if config.acp.enabled {
            tracing::info!("ACP agent sandbox mode available");
        }
        (job_event_tx, Some(jm))
    } else {
        (None, None)
    };

    OrchestratorSetup {
        container_job_manager,
        job_event_tx,
        prompt_queue,
        docker_status,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::helpers::lock_env;

    #[test]
    fn resolve_orchestrator_port_from_env() {
        let _guard = lock_env();

        // Safety: env-var mutation requires unsafe in edition 2024;
        // lock_env() serializes concurrent access from other test threads.

        // Absent env var → default 50051
        unsafe { std::env::remove_var("ORCHESTRATOR_PORT") };
        assert_eq!(resolve_orchestrator_port(), 50051);

        // Valid custom port
        unsafe { std::env::set_var("ORCHESTRATOR_PORT", "50052") };
        assert_eq!(resolve_orchestrator_port(), 50052);

        // Non-numeric value → fallback to default
        unsafe { std::env::set_var("ORCHESTRATOR_PORT", "not_a_port") };
        assert_eq!(resolve_orchestrator_port(), 50051);

        // Out of u16 range → fallback to default
        unsafe { std::env::set_var("ORCHESTRATOR_PORT", "99999") };
        assert_eq!(resolve_orchestrator_port(), 50051);

        // Cleanup
        unsafe { std::env::remove_var("ORCHESTRATOR_PORT") };
    }
}
