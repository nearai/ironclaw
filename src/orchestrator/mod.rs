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

use crate::channels::web::types::SseEvent;
use crate::db::Database;
use crate::llm::LlmProvider;
use crate::safety::SafetyLayer;
use crate::secrets::SecretsStore;
use crate::tools::{ToolExecutor, ToolRegistry};

/// Result of orchestrator setup, containing all handles needed by the agent.
pub struct OrchestratorSetup {
    pub container_job_manager: Option<Arc<ContainerJobManager>>,
    pub job_event_tx: Option<broadcast::Sender<(Uuid, SseEvent)>>,
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
    tools: &Arc<ToolRegistry>,
    safety: &Arc<SafetyLayer>,
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
        let job_config = ContainerJobConfig {
            image: config.sandbox.image.clone(),
            memory_limit_mb: config.sandbox.memory_limit_mb,
            cpu_shares: config.sandbox.cpu_shares,
            orchestrator_port: 50051,
            claude_code_api_key: std::env::var("ANTHROPIC_API_KEY").ok(),
            claude_code_oauth_token: crate::config::ClaudeCodeConfig::extract_oauth_token(),
            claude_code_model: config.claude_code.model.clone(),
            claude_code_max_turns: config.claude_code.max_turns,
            claude_code_memory_limit_mb: config.claude_code.memory_limit_mb,
            claude_code_allowed_tools: config.claude_code.allowed_tools.clone(),
        };
        let jm = Arc::new(ContainerJobManager::new(job_config, token_store.clone()));

        // Build ToolExecutor for programmatic tool calling (PTC)
        let tool_executor = Arc::new(ToolExecutor::new(
            Arc::clone(tools),
            Arc::clone(safety),
            std::time::Duration::from_secs(60),
        ));

        // Wire the executor into the shared slot so WASM tools registered
        // during build_all() can resolve it lazily at execution time.
        tools.set_tool_executor(Arc::clone(&tool_executor));

        let orchestrator_state = api::OrchestratorState {
            llm: Arc::clone(llm),
            job_manager: Arc::clone(&jm),
            token_store,
            job_event_tx: job_event_tx.clone(),
            prompt_queue: Arc::clone(&prompt_queue),
            store: db.cloned(),
            secrets_store: secrets_store.cloned(),
            user_id: "default".to_string(),
            tool_executor: Some(tool_executor),
        };

        tokio::spawn(async move {
            if let Err(e) = OrchestratorApi::start(orchestrator_state, 50051).await {
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
