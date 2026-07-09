//! Generic "run an MCP tool as a background job" builtins.
//!
//! `tool_job_start` launches a long-running MCP tool call as a durable
//! background job (via [`crate::agent::Scheduler::dispatch_mcp_job`]) and
//! returns immediately with a job id; the wrapped result is later injected back
//! into the originating thread. `tool_job_status` reports a job's state.
//!
//! The MCP tool is named by **separate `server` + `tool`** params rather than a
//! single prefixed name: `mcp_tool_id` (the prefix scheme) is a lossy
//! single-underscore join and is not reliably invertible.

use std::sync::Arc;

use async_trait::async_trait;
use uuid::Uuid;

use crate::context::{ContextManager, JobContext};
use crate::tools::mcp::McpClientStore;
use crate::tools::tool::{Tool, ToolError, ToolOutput, require_str};
use crate::worker::mcp_job::McpJobSpec;

use super::job::SchedulerSlot;

/// Resolves whether a server's tools may run as background jobs, for a user.
///
/// A seam over the client store so the tools are testable without live MCP
/// clients (mirrors the `McpCaller` seam on the runner).
#[async_trait]
pub trait BackgroundPolicy: Send + Sync {
    /// `Some(true)` = server active and background-enabled; `Some(false)` =
    /// active but not enabled (opt-in missing); `None` = not active for the user.
    async fn allows_background(&self, user_id: &str, server: &str) -> Option<bool>;
}

/// Production policy: reads the per-user active client's `allow_background`.
pub struct StoreBackgroundPolicy {
    pub client_store: Arc<McpClientStore>,
}

#[async_trait]
impl BackgroundPolicy for StoreBackgroundPolicy {
    async fn allows_background(&self, user_id: &str, server: &str) -> Option<bool> {
        self.client_store
            .get(user_id, server)
            .await
            .map(|c| c.allows_background())
    }
}

/// `tool_job_start` — launch an MCP tool call as a background job.
pub struct CreateMcpJobTool {
    scheduler_slot: SchedulerSlot,
    policy: Arc<dyn BackgroundPolicy>,
}

impl CreateMcpJobTool {
    pub fn new(scheduler_slot: SchedulerSlot, policy: Arc<dyn BackgroundPolicy>) -> Self {
        Self {
            scheduler_slot,
            policy,
        }
    }
}

#[async_trait]
impl Tool for CreateMcpJobTool {
    fn name(&self) -> &str {
        "tool_job_start"
    }

    fn description(&self) -> &str {
        "Run a long-running MCP tool call as a background job. Returns a job_id \
         immediately; the result is delivered back into this conversation when it \
         finishes. Only works for MCP servers enabled for background jobs. Use \
         tool_job_status to poll."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "server": { "type": "string", "description": "MCP server name, e.g. 'msbsandbox'" },
                "tool": { "type": "string", "description": "MCP tool name on that server, e.g. 'run_python'" },
                "arguments": { "type": "object", "description": "Arguments object passed to the tool" }
            },
            "required": ["server", "tool"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let server = require_str(&params, "server")?.to_string();
        let tool = require_str(&params, "tool")?.to_string();
        let arguments = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));

        match self.policy.allows_background(&ctx.user_id, &server).await {
            None => {
                return Err(ToolError::InvalidParameters(format!(
                    "MCP server '{server}' is not active for you; add and enable it first"
                )));
            }
            Some(false) => {
                return Err(ToolError::InvalidParameters(format!(
                    "MCP server '{server}' is not enabled for background jobs \
                     (re-add it with --allow-background)"
                )));
            }
            Some(true) => {}
        }

        let scheduler = {
            let guard = self.scheduler_slot.read().await;
            guard.clone()
        }
        .ok_or_else(|| {
            ToolError::ExecutionFailed("scheduler is not available for background jobs".to_string())
        })?;

        // Originating channel/thread for the auto-resume injection come from the
        // dispatching turn's context metadata; fall back to the gateway channel.
        let channel = ctx
            .metadata
            .get("channel")
            .and_then(|v| v.as_str())
            .unwrap_or("gateway")
            .to_string();
        let thread_id = ctx
            .metadata
            .get("thread_id")
            .and_then(|v| v.as_str())
            .map(str::to_string);

        let spec = McpJobSpec {
            server: server.clone(),
            tool: tool.clone(),
            params: arguments,
            user_id: ctx.user_id.clone(),
            channel,
            thread_id,
        };

        let job_id = scheduler.dispatch_mcp_job(spec).await.map_err(|e| {
            ToolError::ExecutionFailed(format!("failed to start background job: {e}"))
        })?;

        Ok(ToolOutput::success(
            serde_json::json!({
                "job_id": job_id.to_string(),
                "state": "in_progress",
                "message": format!("Started {tool} on {server} as a background job"),
            }),
            start.elapsed(),
        ))
    }
}

/// `tool_job_status` — report the state of a background (or any) job.
pub struct McpJobStatusTool {
    context_manager: Arc<ContextManager>,
}

impl McpJobStatusTool {
    pub fn new(context_manager: Arc<ContextManager>) -> Self {
        Self { context_manager }
    }
}

#[async_trait]
impl Tool for McpJobStatusTool {
    fn name(&self) -> &str {
        "tool_job_status"
    }

    fn description(&self) -> &str {
        "Check the state of a background job started with tool_job_start, by job_id."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "job_id": { "type": "string", "description": "The job id returned by tool_job_start" }
            },
            "required": ["job_id"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let job_id_str = require_str(&params, "job_id")?;
        let job_id = Uuid::parse_str(job_id_str).map_err(|_| {
            ToolError::InvalidParameters(format!("'{job_id_str}' is not a valid job id"))
        })?;

        let jc = self
            .context_manager
            .get_context(job_id)
            .await
            .map_err(|_| {
                ToolError::InvalidParameters(format!("no job found for '{job_id_str}'"))
            })?;

        // Don't leak other users' jobs.
        if jc.user_id != ctx.user_id {
            return Err(ToolError::NotAuthorized(
                "that job belongs to another user".to_string(),
            ));
        }

        Ok(ToolOutput::success(
            serde_json::json!({
                "job_id": job_id.to_string(),
                "state": format!("{:?}", jc.state),
                "title": jc.title,
            }),
            start.elapsed(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::RwLock;

    struct FakePolicy(Option<bool>);

    #[async_trait]
    impl BackgroundPolicy for FakePolicy {
        async fn allows_background(&self, _user_id: &str, _server: &str) -> Option<bool> {
            self.0
        }
    }

    fn caller_ctx(user: &str) -> JobContext {
        let mut jc = JobContext::new("caller", "caller turn");
        jc.user_id = user.to_string();
        jc
    }

    fn tool_with_policy(policy: Option<bool>) -> CreateMcpJobTool {
        let slot: SchedulerSlot = Arc::new(RwLock::new(None)); // empty scheduler
        CreateMcpJobTool::new(slot, Arc::new(FakePolicy(policy)))
    }

    #[tokio::test]
    async fn start_rejects_inactive_server() {
        let tool = tool_with_policy(None);
        let params = serde_json::json!({"server":"srv","tool":"do"});
        let err = tool.execute(params, &caller_ctx("u1")).await.unwrap_err();
        assert!(matches!(err, ToolError::InvalidParameters(_)));
    }

    #[tokio::test]
    async fn start_rejects_non_background_server() {
        let tool = tool_with_policy(Some(false));
        let params = serde_json::json!({"server":"srv","tool":"do"});
        let err = tool.execute(params, &caller_ctx("u1")).await.unwrap_err();
        match err {
            ToolError::InvalidParameters(m) => assert!(m.contains("background")),
            other => panic!("expected InvalidParameters, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn start_requires_server_and_tool() {
        let tool = tool_with_policy(Some(true));
        let err = tool
            .execute(serde_json::json!({"server":"srv"}), &caller_ctx("u1"))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidParameters(_)));
    }

    #[tokio::test]
    async fn start_passes_validation_then_needs_scheduler() {
        // Eligible server + valid params, but the scheduler slot is empty →
        // proves validation passed and the tool reached the dispatch step.
        // (The eligible→job_id dispatch itself is covered by the scheduler's
        // dispatch_mcp_job test.)
        let tool = tool_with_policy(Some(true));
        let params = serde_json::json!({"server":"srv","tool":"do","arguments":{"x":1}});
        let err = tool.execute(params, &caller_ctx("u1")).await.unwrap_err();
        assert!(matches!(err, ToolError::ExecutionFailed(_)));
    }

    #[tokio::test]
    async fn status_reports_state_and_guards_ownership() {
        let cm = Arc::new(ContextManager::new(5));
        let job_id = cm.create_job_for_user("u1", "bg", "desc").await.unwrap();
        let tool = McpJobStatusTool::new(cm);

        // Owner sees the state.
        let out = tool
            .execute(
                serde_json::json!({"job_id": job_id.to_string()}),
                &caller_ctx("u1"),
            )
            .await
            .unwrap();
        assert_eq!(out.result.get("job_id").unwrap(), &job_id.to_string());

        // A different user is denied.
        let err = tool
            .execute(
                serde_json::json!({"job_id": job_id.to_string()}),
                &caller_ctx("intruder"),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::NotAuthorized(_)));

        // Unknown id → InvalidParameters.
        let err = tool
            .execute(
                serde_json::json!({"job_id": Uuid::new_v4().to_string()}),
                &caller_ctx("u1"),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidParameters(_)));
    }
}
