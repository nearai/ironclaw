//! Restart tool for graceful process restart.

use async_trait::async_trait;
use std::time::Duration;

use crate::context::JobContext;
use crate::tools::tool::{Tool, ToolError, ToolOutput};

/// Tool for triggering a graceful process restart.
///
/// The tool spawns a background task that sleeps briefly then exits with code 0.
/// This allows the HTTP response to be sent before the process terminates.
/// The entrypoint restart loop detects the clean exit and brings the process back online.
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
        let start = std::time::Instant::now();

        // Extract delay_secs parameter, defaulting to 2 seconds
        let delay = params
            .get("delay_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(2);

        // Spawn a background task so the response is flushed before exit
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(delay)).await;
            std::process::exit(0);
        });

        Ok(ToolOutput::text(
            format!(
                "Restarting in {delay} second(s). The process will exit cleanly and the \
                 entrypoint restart loop will bring IronClaw back online."
            ),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}
