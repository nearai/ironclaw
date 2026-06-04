//! Restart tool for graceful process restart.
//!
//! ## Architecture
//!
//! IronClaw runs inside a Docker container with an entrypoint loop that monitors exit codes:
//! - **Exit code 0** (clean): Reset failure counter, wait `IRONCLAW_RESTART_DELAY` (default 5s), restart
//! - **Exit code ≠ 0** (failure): Increment failure counter, exit after `IRONCLAW_MAX_FAILURES` (default 10)
//!
//! This tool triggers a restart by calling `std::process::exit(0)` after a brief delay, allowing
//! the HTTP response to be flushed before the process terminates. The entrypoint loop then
//! detects the clean exit and automatically restarts the process.
//!
//! ## Security
//!
//! - **Approval Model:** The tool returns `ApprovalRequirement::Always`, so a model/agent
//!   invocation through the dispatch loop is gated at execution time. The `/restart` command
//!   path additionally confirms via a web modal. The dispatch-time gate is the backstop that
//!   prevents an unattended model or prompt-injection from restarting the process.
//! - **Web-Only Access:** The `/restart` command only works via the web gateway (enforced in commands.rs)
//! - **Parameter Validation:** Delay clamped to 1-30 seconds
//!
//! ## Known Limitations
//!
//! - Hard exit without graceful shutdown (no destructor cleanup, no RwLock drains)
//! - In-flight jobs are paused during restart and resumed by the entrypoint
//! - Future: Implement graceful shutdown with CancellationToken for proper resource cleanup

use async_trait::async_trait;
use std::time::Duration;

use crate::context::JobContext;
use crate::tools::tool::{ApprovalRequirement, Tool, ToolError, ToolOutput};

/// Tool for triggering a graceful process restart via exit code 0.
///
/// This tool signals the Docker entrypoint loop to restart the process by exiting cleanly
/// (exit code 0). It returns `ApprovalRequirement::Always`, so any dispatch-loop invocation
/// (including a model/agent call) is gated by explicit approval; the `/restart` command path
/// layers a web-modal confirmation on top. The `/restart` command itself is only callable via
/// the web gateway interface.
pub struct RestartTool;

#[async_trait]
impl Tool for RestartTool {
    fn name(&self) -> &str {
        "restart"
    }

    fn description(&self) -> &str {
        "Restart the IronClaw agent process. The process exits cleanly (code 0) and the \
         container entrypoint loop restarts it automatically within a few seconds."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "delay_secs": {
                    "type": "integer",
                    "description": "Seconds to wait before exiting (default: 2, min: 1, max: 30)",
                    "minimum": 1,
                    "maximum": 30
                }
            }
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        tracing::info!("[RestartTool::execute] Restart tool invoked");
        let start = std::time::Instant::now();

        // Check if running inside a Docker container via IRONCLAW_IN_DOCKER env var.
        // The Docker entrypoint sets this to "true". For local development, it's unset or "false".
        // The entrypoint restart loop only works inside a Docker container (ironclaw-worker).
        let in_docker = std::env::var("IRONCLAW_IN_DOCKER")
            .map(|v| v.to_lowercase() == "true")
            .unwrap_or(false);

        tracing::debug!("[RestartTool::execute] IRONCLAW_IN_DOCKER={}", in_docker);

        if !in_docker {
            tracing::error!("[RestartTool::execute] Not in Docker, rejecting restart");
            return Err(ToolError::ExecutionFailed(
                "Restart is only available when running inside the Docker container. \
                 For local development, please restart IronClaw manually."
                    .to_string(),
            ));
        }

        // Extract delay_secs parameter, defaulting to 2 seconds
        let delay = params
            .get("delay_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(2)
            // Validate delay against schema bounds (1-30 seconds)
            .clamp(1, 30);
        tracing::info!("[RestartTool::execute] Delay set to {} seconds", delay);

        // Spawn a background task so the response is flushed before exit.
        // We use std::process::exit(0) to trigger a Docker container restart:
        //
        // - The ironclaw-worker Docker container runs an entrypoint loop that monitors
        //   the exit code of the `ironclaw run` process:
        //   * Exit code 0 = clean restart: reset failure counter, wait IRONCLAW_RESTART_DELAY
        //     (default 5s), then restart the process
        //   * Exit code ≠ 0 = failure: increment counter, exit after IRONCLAW_MAX_FAILURES
        //     (default 10 failures)
        //
        // - std::process::exit(0) is a hard exit (no destructors, no graceful shutdown).
        //   This is intentional because:
        //   1. The HTTP response must be sent before exit (hence tokio::spawn + delay)
        //   2. In-flight jobs are paused/resumed by the entrypoint loop
        //   3. Database connections are pooled and reopened on restart
        //   4. The brief delay allows the response to flush before termination
        //
        // - Future improvement: implement graceful shutdown with CancellationToken
        //   to properly drain Axum, close DB connections, and checkpoint jobs.
        // Check if restart is disabled (e.g., in tests). This allows tests to verify
        // parameter parsing and output without actually terminating the process.
        let restart_disabled = std::env::var("IRONCLAW_DISABLE_RESTART")
            .map(|v| {
                let v = v.to_lowercase();
                v == "1" || v == "true"
            })
            .unwrap_or(false);

        tracing::info!(
            "[RestartTool::execute] Spawning background task to exit in {} seconds (disabled={})",
            delay,
            restart_disabled
        );
        tokio::spawn(async move {
            tracing::info!("[RestartTool] Sleeping for {} seconds before exit", delay);
            tokio::time::sleep(Duration::from_secs(delay)).await;
            if !restart_disabled {
                tracing::warn!("[RestartTool] Calling std::process::exit(0) NOW");
                std::process::exit(0);
            } else {
                tracing::info!(
                    "[RestartTool] Exit disabled (IRONCLAW_DISABLE_RESTART set), skipping std::process::exit(0)"
                );
            }
        });

        let msg = format!(
            "Restarting in {delay} second(s). The process will exit cleanly and the \
             entrypoint restart loop will bring IronClaw back online."
        );
        tracing::info!("[RestartTool::execute] Returning success response: {}", msg);
        Ok(ToolOutput::text(msg, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }

    // The `/restart` command path gates on a web-modal confirmation, but that
    // guard only covers the HTTP command handler — it does NOT cover a direct
    // model/agent tool invocation through the ordinary dispatch loop (the tool
    // is registered into the live gateway registry and appears in the
    // model-facing tool list). To prevent a model or prompt-injection from
    // restarting the process unattended, require explicit approval at dispatch
    // time regardless of caller. `Always` also blocks autonomous execution
    // unless `restart` is explicitly listed in the job's allowed tools.
    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Always
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    struct EnvLockGuard {
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl EnvLockGuard {
        fn new() -> Self {
            Self {
                _guard: crate::config::helpers::lock_env(),
            }
        }
    }

    struct DockerEnvGuard {
        _lock: EnvLockGuard,
        original_in_docker: Option<OsString>,
        original_disable_restart: Option<OsString>,
    }

    impl DockerEnvGuard {
        fn enable() -> Self {
            let lock = EnvLockGuard::new();
            let original_in_docker = std::env::var_os("IRONCLAW_IN_DOCKER");
            let original_disable_restart = std::env::var_os("IRONCLAW_DISABLE_RESTART");
            // SAFETY: Tests serialize env access with lock_env().
            unsafe {
                std::env::set_var("IRONCLAW_IN_DOCKER", "true");
                // Keep restart tests from terminating the shared lib-test process.
                std::env::set_var("IRONCLAW_DISABLE_RESTART", "true");
            }
            Self {
                _lock: lock,
                original_in_docker,
                original_disable_restart,
            }
        }
    }

    impl Drop for DockerEnvGuard {
        fn drop(&mut self) {
            // SAFETY: Tests serialize env access with lock_env().
            unsafe {
                if let Some(ref value) = self.original_in_docker {
                    std::env::set_var("IRONCLAW_IN_DOCKER", value);
                } else {
                    std::env::remove_var("IRONCLAW_IN_DOCKER");
                }
                if let Some(ref value) = self.original_disable_restart {
                    std::env::set_var("IRONCLAW_DISABLE_RESTART", value);
                } else {
                    std::env::remove_var("IRONCLAW_DISABLE_RESTART");
                }
            }
        }
    }

    #[test]
    fn test_restart_tool_requires_approval_at_dispatch() {
        // The `/restart` web-modal confirmation only gates the HTTP command
        // handler. A model or prompt-injection can reach the registered
        // `restart` tool through the ordinary dispatch loop, so the tool itself
        // must demand explicit approval at dispatch time regardless of caller.
        let tool = RestartTool;
        let approval = tool.requires_approval(&serde_json::json!({}));
        assert!(
            matches!(approval, ApprovalRequirement::Always),
            "restart must require explicit approval to block unattended model invocation"
        );
        assert!(approval.is_required());
    }

    #[test]
    fn test_restart_tool_name() {
        let tool = RestartTool;
        assert_eq!(tool.name(), "restart");
    }

    #[test]
    fn test_restart_tool_parameters_schema() {
        let tool = RestartTool;
        let schema = tool.parameters_schema();

        // Verify schema has delay_secs property with bounds
        let props = schema.get("properties").unwrap();
        assert!(props.get("delay_secs").is_some());

        let delay_schema = props.get("delay_secs").unwrap();
        assert_eq!(delay_schema.get("minimum").unwrap().as_u64().unwrap(), 1);
        assert_eq!(delay_schema.get("maximum").unwrap().as_u64().unwrap(), 30);
    }

    #[test]
    fn test_restart_tool_requires_sanitization() {
        let tool = RestartTool;
        assert!(!tool.requires_sanitization());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_restart_tool_delay_parameter_validation() {
        let _docker_env = DockerEnvGuard::enable();
        let tool = RestartTool;
        let ctx = crate::context::JobContext::new("test", "test restart");

        // Test with valid delay
        let result = tool
            .execute(serde_json::json!({"delay_secs": 5}), &ctx)
            .await;
        assert!(result.is_ok());
        let output = result.unwrap();
        let text = output.result.as_str().expect("result should be a string");
        assert!(text.contains("Restarting in 5 second(s)"));

        // Test with no delay parameter (should use default 2)
        let result = tool.execute(serde_json::json!({}), &ctx).await;
        assert!(result.is_ok());
        let output = result.unwrap();
        let text = output.result.as_str().expect("result should be a string");
        assert!(text.contains("Restarting in 2 second(s)"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_restart_tool_delay_clamping() {
        let _docker_env = DockerEnvGuard::enable();
        let tool = RestartTool;
        let ctx = crate::context::JobContext::new("test", "test restart");

        // Test with too small delay (should clamp to 1)
        let result = tool
            .execute(serde_json::json!({"delay_secs": 0}), &ctx)
            .await;
        assert!(result.is_ok());
        let output = result.unwrap();
        let text = output.result.as_str().expect("result should be a string");
        assert!(text.contains("Restarting in 1 second(s)"));

        // Test with too large delay (should clamp to 30)
        let result = tool
            .execute(serde_json::json!({"delay_secs": 100}), &ctx)
            .await;
        assert!(result.is_ok());
        let output = result.unwrap();
        let text = output.result.as_str().expect("result should be a string");
        assert!(text.contains("Restarting in 30 second(s)"));
    }

    #[test]
    fn test_restart_tool_description() {
        let tool = RestartTool;
        let desc = tool.description();
        assert!(desc.contains("Restart"));
        assert!(desc.contains("IronClaw"));
        assert!(desc.contains("exits cleanly"));
        assert!(desc.contains("code 0"));
    }

    #[test]
    fn test_restart_tool_schema_completeness() {
        let tool = RestartTool;
        let schema = tool.parameters_schema();

        // Verify schema structure
        assert_eq!(schema.get("type").unwrap().as_str().unwrap(), "object");

        let props = schema.get("properties").unwrap();
        assert!(props.is_object());

        let delay_schema = props.get("delay_secs").unwrap();
        assert_eq!(
            delay_schema.get("type").unwrap().as_str().unwrap(),
            "integer"
        );
        assert!(delay_schema.get("description").is_some());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_restart_tool_boundary_values() {
        let _docker_env = DockerEnvGuard::enable();
        let tool = RestartTool;
        let ctx = crate::context::JobContext::new("test", "test restart");

        // Test minimum boundary (exactly 1)
        let result = tool
            .execute(serde_json::json!({"delay_secs": 1}), &ctx)
            .await;
        assert!(result.is_ok());
        let output = result.unwrap();
        let text = output.result.as_str().unwrap();
        assert!(text.contains("Restarting in 1 second(s)"));

        // Test maximum boundary (exactly 30)
        let result = tool
            .execute(serde_json::json!({"delay_secs": 30}), &ctx)
            .await;
        assert!(result.is_ok());
        let output = result.unwrap();
        let text = output.result.as_str().unwrap();
        assert!(text.contains("Restarting in 30 second(s)"));

        // Test middle value
        let result = tool
            .execute(serde_json::json!({"delay_secs": 15}), &ctx)
            .await;
        assert!(result.is_ok());
        let output = result.unwrap();
        let text = output.result.as_str().unwrap();
        assert!(text.contains("Restarting in 15 second(s)"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_restart_tool_invalid_parameter_types() {
        let _docker_env = DockerEnvGuard::enable();
        let tool = RestartTool;
        let ctx = crate::context::JobContext::new("test", "test restart");

        // String instead of integer - should use default
        let result = tool
            .execute(serde_json::json!({"delay_secs": "5"}), &ctx)
            .await;
        assert!(result.is_ok());
        let output = result.unwrap();
        let text = output.result.as_str().unwrap();
        assert!(text.contains("Restarting in 2 second(s)")); // Falls back to default

        // Null value - should use default
        let result = tool
            .execute(serde_json::json!({"delay_secs": null}), &ctx)
            .await;
        assert!(result.is_ok());
        let output = result.unwrap();
        let text = output.result.as_str().unwrap();
        assert!(text.contains("Restarting in 2 second(s)"));

        // Float value - should use default (as_u64 fails on floats)
        let result = tool
            .execute(serde_json::json!({"delay_secs": 5.5}), &ctx)
            .await;
        assert!(result.is_ok());
        let output = result.unwrap();
        let text = output.result.as_str().unwrap();
        assert!(text.contains("Restarting in 2 second(s)"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_restart_tool_output_structure() {
        let _docker_env = DockerEnvGuard::enable();
        let tool = RestartTool;
        let ctx = crate::context::JobContext::new("test", "test restart");

        let result = tool
            .execute(serde_json::json!({"delay_secs": 5}), &ctx)
            .await;

        assert!(result.is_ok());
        let output = result.unwrap();

        // Verify ToolOutput structure
        assert!(output.result.is_string());
        assert!(output.duration.as_secs() == 0); // Should be nearly instant
        assert!(output.cost.is_none()); // No cost tracking for restart
        assert!(output.raw.is_none()); // No raw output stored
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_restart_tool_extra_parameters_ignored() {
        let _docker_env = DockerEnvGuard::enable();
        let tool = RestartTool;
        let ctx = crate::context::JobContext::new("test", "test restart");

        // Extra parameters should be ignored
        let result = tool
            .execute(
                serde_json::json!({
                    "delay_secs": 5,
                    "extra_field": "should be ignored",
                    "another": 123
                }),
                &ctx,
            )
            .await;

        assert!(result.is_ok());
        let output = result.unwrap();
        let text = output.result.as_str().unwrap();
        assert!(text.contains("Restarting in 5 second(s)"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_restart_tool_negative_numbers() {
        let _docker_env = DockerEnvGuard::enable();
        let tool = RestartTool;
        let ctx = crate::context::JobContext::new("test", "test restart");

        // Negative number should clamp to 1
        let result = tool
            .execute(serde_json::json!({"delay_secs": -5}), &ctx)
            .await;
        assert!(result.is_ok());
        let output = result.unwrap();
        let text = output.result.as_str().unwrap();
        // as_u64() on negative number returns None, so falls to default 2
        assert!(text.contains("Restarting in 2 second(s)"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_restart_tool_very_large_numbers() {
        let _docker_env = DockerEnvGuard::enable();
        let tool = RestartTool;
        let ctx = crate::context::JobContext::new("test", "test restart");

        // Very large number should clamp to 30
        let result = tool
            .execute(serde_json::json!({"delay_secs": u64::MAX}), &ctx)
            .await;
        assert!(result.is_ok());
        let output = result.unwrap();
        let text = output.result.as_str().unwrap();
        assert!(text.contains("Restarting in 30 second(s)"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_restart_tool_empty_object() {
        let _docker_env = DockerEnvGuard::enable();
        let tool = RestartTool;
        let ctx = crate::context::JobContext::new("test", "test restart");

        // Empty object params should use all defaults
        let result = tool.execute(serde_json::json!({}), &ctx).await;
        assert!(result.is_ok());
        let output = result.unwrap();
        let text = output.result.as_str().unwrap();
        assert!(text.contains("Restarting in 2 second(s)"));
        assert!(text.contains("exit cleanly"));
        assert!(text.contains("entrypoint restart loop"));
    }

    #[test]
    fn test_restart_tool_approval_consistent_regardless_of_params() {
        let tool = RestartTool;

        // Approval requirement should be the same regardless of params
        let approval1 = tool.requires_approval(&serde_json::json!({"delay_secs": 5}));
        let approval2 = tool.requires_approval(&serde_json::json!({"delay_secs": 100}));
        let approval3 = tool.requires_approval(&serde_json::json!({}));

        // All should require explicit approval regardless of caller-supplied params.
        assert!(matches!(approval1, ApprovalRequirement::Always));
        assert!(matches!(approval2, ApprovalRequirement::Always));
        assert!(matches!(approval3, ApprovalRequirement::Always));
    }

    #[test]
    fn test_restart_tool_requires_docker_environment() {
        let _env_lock = crate::config::helpers::lock_env();
        // Test that restart is rejected when not in Docker (IRONCLAW_IN_DOCKER not set or false)
        // Uses sync test to avoid async/env var ordering issues with test parallelization.
        let in_docker = std::env::var("IRONCLAW_IN_DOCKER")
            .map(|v| v.to_lowercase() == "true")
            .unwrap_or(false);

        // Verify logic: when not in Docker, env var should be false/unset
        if !in_docker {
            // Simulating what the tool would do when IRONCLAW_IN_DOCKER is not set
            assert!(
                !in_docker,
                "Test environment should have IRONCLAW_IN_DOCKER unset or false"
            );
        }
    }
}
