//! Worker mode for running inside Docker containers.
//!
//! When `ironclaw worker` is invoked, the binary starts in worker mode:
//! - Connects to the orchestrator over HTTP
//! - Uses a `ProxyLlmProvider` that routes LLM calls through the orchestrator
//! - Runs container-safe tools (shell, file ops, patch)
//! - Reports status and completion back to the orchestrator
//!
//! ```text
//! ┌────────────────────────────────┐
//! │        Docker Container         │
//! │                                 │
//! │  ironclaw worker                │
//! │    ├─ ProxyLlmProvider ─────────┼──▶ Orchestrator /worker/{id}/llm/complete
//! │    ├─ SafetyLayer               │
//! │    ├─ ToolRegistry              │
//! │    │   ├─ shell                 │
//! │    │   ├─ read_file             │
//! │    │   ├─ write_file            │
//! │    │   ├─ list_dir              │
//! │    │   └─ apply_patch           │
//! │    └─ WorkerHttpClient ─────────┼──▶ Orchestrator /worker/{id}/status
//! │                                 │
//! └────────────────────────────────┘
//! ```

pub mod api;
pub mod claude_bridge;
pub mod container;
pub mod job;
pub mod proxy_llm;

pub use api::WorkerHttpClient;
pub use claude_bridge::ClaudeBridgeRuntime;
pub use container::WorkerRuntime;
pub use job::{Worker, WorkerDeps};
pub use proxy_llm::ProxyLlmProvider;

/// Run the Worker subcommand (inside Docker containers).
pub async fn run_worker(
    job_id: uuid::Uuid,
    orchestrator_url: &str,
    max_iterations: u32,
) -> anyhow::Result<()> {
    tracing::info!(
        "Starting worker for job {} (orchestrator: {})",
        job_id,
        orchestrator_url
    );

    let config = container::WorkerConfig {
        job_id,
        orchestrator_url: orchestrator_url.to_string(),
        max_iterations,
        timeout: std::time::Duration::from_secs(600),
    };

    let rt =
        WorkerRuntime::new(config).map_err(|e| anyhow::anyhow!("Worker init failed: {}", e))?;

    rt.run()
        .await
        .map_err(|e| anyhow::anyhow!("Worker failed: {}", e))
}

/// Run the Claude Code bridge subcommand (inside Docker containers).
pub async fn run_claude_bridge(
    job_id: uuid::Uuid,
    orchestrator_url: &str,
    max_turns: u32,
    model: &str,
) -> anyhow::Result<()> {
    tracing::info!(
        "Starting Claude Code bridge for job {} (orchestrator: {}, model: {})",
        job_id,
        orchestrator_url,
        model
    );

    let config = claude_bridge::ClaudeBridgeConfig {
        job_id,
        orchestrator_url: orchestrator_url.to_string(),
        max_turns,
        model: model.to_string(),
        timeout: std::time::Duration::from_secs(1800),
        allowed_tools: crate::config::ClaudeCodeConfig::from_env().allowed_tools,
    };

    let rt = ClaudeBridgeRuntime::new(config)
        .map_err(|e| anyhow::anyhow!("Claude bridge init failed: {}", e))?;

    rt.run()
        .await
        .map_err(|e| anyhow::anyhow!("Claude bridge failed: {}", e))
}
