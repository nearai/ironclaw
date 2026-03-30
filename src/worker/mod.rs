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
mod autonomous_recovery;
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
        drift_config: resolve_drift_config(),
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

/// Resolve drift monitor configuration from environment variables.
///
/// Uses the same `parse_bool_env` / `parse_optional_env` helpers as the
/// host-side `AgentConfig::resolve()` to ensure identical parsing semantics.
/// Logs a warning and falls back to default on parse error (container
/// startup should not fail over drift config).
fn resolve_drift_config() -> crate::agent::drift_monitor::DriftConfig {
    use crate::agent::drift_monitor::DriftConfig;

    match resolve_drift_config_inner() {
        Ok(config) => config,
        Err(e) => {
            tracing::warn!(
                "Failed to parse drift config from env, using defaults: {}",
                e
            );
            DriftConfig::default()
        }
    }
}

fn resolve_drift_config_inner()
-> Result<crate::agent::drift_monitor::DriftConfig, crate::error::ConfigError> {
    use crate::agent::drift_monitor::DriftConfig;
    use crate::config::helpers::{parse_bool_env, parse_optional_env};

    let defaults = DriftConfig::default();
    Ok(DriftConfig {
        enabled: parse_bool_env("IRONCLAW_DRIFT_ENABLED", defaults.enabled)?,
        repetition_threshold: parse_optional_env(
            "IRONCLAW_DRIFT_REPETITION_THRESHOLD",
            defaults.repetition_threshold,
        )?,
        repetition_window: parse_optional_env(
            "IRONCLAW_DRIFT_REPETITION_WINDOW",
            defaults.repetition_window,
        )?,
        failure_spiral_threshold: parse_optional_env(
            "IRONCLAW_DRIFT_FAILURE_THRESHOLD",
            defaults.failure_spiral_threshold,
        )?,
        cycling_window: parse_optional_env(
            "IRONCLAW_DRIFT_CYCLING_WINDOW",
            defaults.cycling_window,
        )?,
        silence_threshold: parse_optional_env(
            "IRONCLAW_DRIFT_SILENCE_THRESHOLD",
            defaults.silence_threshold,
        )?,
    }
    .clamped())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_container_drift_config_from_env() {
        let _guard = crate::config::helpers::lock_env();
        // safety: test-only, guarded by ENV_MUTEX
        unsafe {
            std::env::set_var("IRONCLAW_DRIFT_ENABLED", "false");
            std::env::set_var("IRONCLAW_DRIFT_FAILURE_THRESHOLD", "8");
            std::env::set_var("IRONCLAW_DRIFT_CYCLING_WINDOW", "12");
        }

        let config = resolve_drift_config();
        assert!(!config.enabled);
        assert_eq!(config.failure_spiral_threshold, 8);
        assert_eq!(config.cycling_window, 12);
        // Non-overridden fields keep defaults
        assert_eq!(config.repetition_threshold, 3);

        unsafe {
            std::env::remove_var("IRONCLAW_DRIFT_ENABLED");
            std::env::remove_var("IRONCLAW_DRIFT_FAILURE_THRESHOLD");
            std::env::remove_var("IRONCLAW_DRIFT_CYCLING_WINDOW");
        }
    }

    #[test]
    fn test_container_drift_config_invalid_env_logs_warning() {
        let _guard = crate::config::helpers::lock_env();
        // safety: test-only, guarded by ENV_MUTEX
        unsafe {
            std::env::set_var("IRONCLAW_DRIFT_FAILURE_THRESHOLD", "not_a_number");
        }

        // Should fall back to defaults and not panic
        let config = resolve_drift_config();
        assert_eq!(
            config.failure_spiral_threshold,
            crate::agent::drift_monitor::DriftConfig::default().failure_spiral_threshold
        );

        unsafe {
            std::env::remove_var("IRONCLAW_DRIFT_FAILURE_THRESHOLD");
        }
    }
}
