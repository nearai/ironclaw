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

pub mod acp_bridge;
pub mod api;
mod autonomous_recovery;
pub mod claude_bridge;
pub mod container;
pub mod job;
pub mod proxy_llm;

pub use acp_bridge::AcpBridgeRuntime;
pub use api::WorkerHttpClient;
pub use claude_bridge::ClaudeBridgeRuntime;
pub use container::WorkerRuntime;
pub use job::{Worker, WorkerDeps};
pub use proxy_llm::ProxyLlmProvider;

fn acp_bridge_timeout() -> std::time::Duration {
    std::time::Duration::from_secs(crate::config::AcpModeConfig::from_env().timeout_secs)
}

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

/// Run the ACP bridge subcommand (inside Docker containers).
pub async fn run_acp_bridge(job_id: uuid::Uuid, orchestrator_url: &str) -> anyhow::Result<()> {
    let agent_command = std::env::var("ACP_AGENT_COMMAND").map_err(|_| {
        anyhow::anyhow!("ACP_AGENT_COMMAND not set — cannot determine which agent to spawn")
    })?;

    let agent_args: Vec<String> = match std::env::var("ACP_AGENT_ARGS") {
        Ok(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
            tracing::warn!("Failed to parse ACP_AGENT_ARGS as JSON: {e}, using empty args");
            Vec::new()
        }),
        Err(_) => Vec::new(),
    };

    let agent_env: std::collections::HashMap<String, String> = match std::env::var("ACP_AGENT_ENV")
    {
        Ok(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
            tracing::warn!("Failed to parse ACP_AGENT_ENV as JSON: {e}, using empty env");
            std::collections::HashMap::new()
        }),
        Err(_) => std::collections::HashMap::new(),
    };

    tracing::info!(
        "Starting ACP bridge for job {} (orchestrator: {}, agent: {} {})",
        job_id,
        orchestrator_url,
        agent_command,
        agent_args.join(" ")
    );

    let config = acp_bridge::AcpBridgeConfig {
        job_id,
        orchestrator_url: orchestrator_url.to_string(),
        timeout: acp_bridge_timeout(),
        agent_command,
        agent_args,
        agent_env,
    };

    let rt = AcpBridgeRuntime::new(config)
        .map_err(|e| anyhow::anyhow!("ACP bridge init failed: {}", e))?;

    rt.run()
        .await
        .map_err(|e| anyhow::anyhow!("ACP bridge failed: {}", e))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acp_bridge_timeout_defaults_to_1800_seconds() {
        let _guard = crate::config::helpers::lock_env();
        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe { std::env::remove_var("ACP_TIMEOUT_SECS") };
        assert_eq!(acp_bridge_timeout(), std::time::Duration::from_secs(1800));
    }

    #[test]
    fn acp_bridge_timeout_respects_env_override() {
        let _guard = crate::config::helpers::lock_env();
        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe { std::env::set_var("ACP_TIMEOUT_SECS", "45") };
        assert_eq!(acp_bridge_timeout(), std::time::Duration::from_secs(45));
        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe { std::env::remove_var("ACP_TIMEOUT_SECS") };
    }
}
