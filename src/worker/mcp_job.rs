//! Background-job descriptor for running a single MCP tool call as a durable
//! IronClaw job (Phase 2 of the MCP background-jobs design).
//!
//! The job "mode" lives in `JobContext.metadata` (a `serde_json::Value`) rather
//! than the orchestrator `JobMode` enum, which is not on this dispatch path.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_common::{AppEvent, JobResultStatus};
use ironclaw_safety::SafetyLayer;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::channels::IncomingMessage;
use crate::channels::web::sse::SseManager;
use crate::context::{ContextManager, JobState};
use crate::tenant::SystemScope;
use crate::tools::mcp::McpClientStore;

/// Everything needed to run one MCP tool call as a background job and inject
/// its result back into the originating agent thread on completion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpJobSpec {
    /// MCP server name (e.g. "msbsandbox").
    pub server: String,
    /// Unprefixed MCP tool name (e.g. "run_python").
    pub tool: String,
    /// Tool call parameters.
    pub params: serde_json::Value,
    /// Owning user.
    pub user_id: String,
    /// Originating channel, used to inject the result on completion.
    pub channel: String,
    /// Originating thread, if any.
    pub thread_id: Option<String>,
}

impl McpJobSpec {
    /// The metadata blob persisted on the job's `JobContext`. Identity fields
    /// (`user_id` / `channel` / `thread_id`) live on the context itself, not
    /// here, so they are intentionally omitted.
    pub fn to_metadata(&self) -> serde_json::Value {
        serde_json::json!({
            "mode": "mcp_tool",
            "server": self.server,
            "tool": self.tool,
            "params": self.params,
        })
    }

    /// Rebuild a spec from a persisted metadata blob plus the context-supplied
    /// identity fields. Returns `None` unless `mode == "mcp_tool"` and the
    /// required `server` / `tool` fields are present.
    pub fn from_metadata(
        meta: &serde_json::Value,
        user_id: &str,
        channel: &str,
        thread_id: Option<String>,
    ) -> Option<Self> {
        if meta.get("mode").and_then(|m| m.as_str()) != Some("mcp_tool") {
            return None;
        }
        let server = meta.get("server").and_then(|v| v.as_str())?.to_string();
        let tool = meta.get("tool").and_then(|v| v.as_str())?.to_string();
        let params = meta
            .get("params")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        Some(Self {
            server,
            tool,
            params,
            user_id: user_id.to_string(),
            channel: channel.to_string(),
            thread_id,
        })
    }
}

/// Minimal seam for invoking an MCP tool, so `run_mcp_job` is testable
/// without a live `McpClient`. The real implementation resolves the per-user
/// client through the `McpClientStore`; tests supply a fake.
#[async_trait]
pub trait McpCaller: Send + Sync {
    /// Call `tool` on `server` for `user_id`, returning the joined text output.
    /// `Err(msg)` on an inactive server, a transport error, or a tool that
    /// reports `is_error` (msg carries the error text).
    async fn call(
        &self,
        user_id: &str,
        server: &str,
        tool: &str,
        params: serde_json::Value,
    ) -> Result<String, String>;
}

/// Production `McpCaller`: looks up the per-user client and joins the text
/// content parts exactly as `McpToolWrapper::execute` does.
pub struct StoreMcpCaller {
    pub client_store: Arc<McpClientStore>,
}

#[async_trait]
impl McpCaller for StoreMcpCaller {
    async fn call(
        &self,
        user_id: &str,
        server: &str,
        tool: &str,
        params: serde_json::Value,
    ) -> Result<String, String> {
        let client = self
            .client_store
            .get(user_id, server)
            .await
            .ok_or_else(|| format!("MCP server '{server}' is not active for this user"))?;
        let result = client
            .call_tool(tool, params)
            .await
            .map_err(|e| e.to_string())?;
        let content: String = result
            .content
            .iter()
            .filter_map(|b| b.as_text())
            .collect::<Vec<_>>()
            .join("\n");
        if result.is_error {
            Err(content)
        } else {
            Ok(content)
        }
    }
}

/// Dependencies for `run_mcp_job`. `store` / `sse_tx` / `inject_tx` are all
/// optional so the runner degrades gracefully (and stays unit-testable) when a
/// layer is absent — the job state in the `ContextManager` is always updated.
pub struct McpJobDeps {
    pub context_manager: Arc<ContextManager>,
    pub store: Option<SystemScope>,
    pub sse_tx: Option<Arc<SseManager>>,
    pub inject_tx: Option<tokio::sync::mpsc::Sender<IncomingMessage>>,
    pub caller: Arc<dyn McpCaller>,
    pub safety: Arc<SafetyLayer>,
}

/// First 8 chars of a job id, for human-facing job references.
fn short_id(id: Uuid) -> String {
    id.simple().to_string().chars().take(8).collect()
}

/// Move a job to `state` in the `ContextManager`, treating an invalid/idempotent
/// transition as benign (debug log) and a missing context as a real warning.
async fn transition(cm: &ContextManager, job_id: Uuid, state: JobState, reason: Option<String>) {
    match cm
        .update_context(job_id, move |c| c.transition_to(state, reason))
        .await
    {
        Ok(Ok(())) => {}
        Ok(Err(e)) => tracing::debug!("mcp job {job_id}: benign transition skip: {e}"),
        Err(e) => tracing::warn!("mcp job {job_id}: context missing on transition: {e}"),
    }
}

/// Best-effort persistence of job status (silent-ok: a missed write is
/// reconciled by the next poll / startup reconcile).
async fn persist_status(
    store: &Option<SystemScope>,
    job_id: Uuid,
    status: JobState,
    reason: Option<&str>,
) {
    if let Some(store) = store
        && let Err(e) = store.update_job_status(job_id, status, reason).await
    {
        tracing::warn!("mcp job {job_id}: failed to persist status: {e}");
    }
}

/// Best-effort persistence of a job event to the store + live SSE broadcast.
async fn emit_event(
    deps: &McpJobDeps,
    job_id: Uuid,
    event_type: &str,
    data: serde_json::Value,
    sse: AppEvent,
) {
    if let Some(store) = &deps.store
        && let Err(e) = store.save_job_event(job_id, event_type, &data).await
    {
        tracing::warn!("mcp job {job_id}: failed to persist {event_type} event: {e}");
    }
    if let Some(sse_tx) = &deps.sse_tx {
        sse_tx.broadcast(sse);
    }
}

/// Run a single MCP tool call as a durable background job: mark it in-progress,
/// invoke the tool at the server's configured (long) timeout, safety-scan the
/// untrusted output, record the terminal state, and inject the wrapped result
/// back into the originating thread so the agent resumes.
pub async fn run_mcp_job(job_id: Uuid, spec: McpJobSpec, deps: McpJobDeps) {
    // 1. In-progress.
    transition(
        &deps.context_manager,
        job_id,
        JobState::InProgress,
        Some("mcp job started".into()),
    )
    .await;
    persist_status(&deps.store, job_id, JobState::InProgress, None).await;

    // 2. Status event.
    let running = format!("Running {} in background", spec.tool);
    emit_event(
        &deps,
        job_id,
        "status",
        serde_json::json!({ "message": running }),
        AppEvent::JobStatus {
            job_id: job_id.to_string(),
            message: running.clone(),
        },
    )
    .await;

    // 3-5. Invoke the tool (at the server's Phase-1 long timeout) and reduce to
    // a Result<String, String>.
    let call_res = deps
        .caller
        .call(&spec.user_id, &spec.server, &spec.tool, spec.params.clone())
        .await;
    let ok = call_res.is_ok();
    let raw = match call_res {
        Ok(out) => out,
        Err(e) => e,
    };

    // 6. Safety scan — MCP output is untrusted external data and MUST be
    // sanitized + wrapped before it reaches the LLM.
    let sanitized = deps.safety.sanitize_tool_output(&spec.tool, &raw);
    let wrapped = deps.safety.wrap_for_llm(&spec.tool, &sanitized.content);

    // 7. Terminal state.
    let final_state = if ok {
        JobState::Completed
    } else {
        JobState::Failed
    };
    let reason = if ok {
        None
    } else {
        Some("mcp tool call failed".to_string())
    };
    transition(&deps.context_manager, job_id, final_state, reason.clone()).await;
    persist_status(&deps.store, job_id, final_state, reason.as_deref()).await;

    // 8. Result event.
    let result_status = if ok {
        JobResultStatus::Completed
    } else {
        JobResultStatus::Failed
    };
    emit_event(
        &deps,
        job_id,
        "result",
        serde_json::json!({ "status": if ok { "completed" } else { "failed" } }),
        AppEvent::JobResult {
            job_id: job_id.to_string(),
            status: result_status,
            session_id: None,
            fallback_deliverable: None,
        },
    )
    .await;

    // 9. Inject the wrapped result back into the originating thread so the agent
    // resumes. Silent-ok: the agent may be idle; the job state is already
    // persisted and durable.
    if let Some(tx) = &deps.inject_tx {
        let body = format!(
            "[Background job {}] {} {}: {}",
            short_id(job_id),
            spec.tool,
            if ok { "completed" } else { "failed" },
            wrapped
        );
        let mut msg =
            IncomingMessage::new(spec.channel.clone(), spec.user_id.clone(), body).into_internal();
        if let Some(t) = &spec.thread_id {
            msg = msg.with_thread(t.clone());
        }
        let _ = tx.send(msg).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_roundtrip() {
        let spec = McpJobSpec {
            server: "msbsandbox".into(),
            tool: "run_python".into(),
            params: serde_json::json!({"code":"print(1)"}),
            user_id: "u1".into(),
            channel: "gateway".into(),
            thread_id: Some("t1".into()),
        };
        let meta = spec.to_metadata();
        assert_eq!(meta["mode"], "mcp_tool");
        let back = McpJobSpec::from_metadata(&meta, "u1", "gateway", Some("t1".into())).unwrap();
        assert_eq!(back.server, "msbsandbox");
        assert_eq!(back.tool, "run_python");
        assert_eq!(back.params["code"], "print(1)");
        assert!(
            McpJobSpec::from_metadata(&serde_json::json!({"mode":"other"}), "u", "c", None)
                .is_none()
        );
    }

    #[test]
    fn from_metadata_requires_server_and_tool() {
        // mode is right but required fields missing → None (no panic).
        let meta = serde_json::json!({"mode":"mcp_tool","server":"s"});
        assert!(McpJobSpec::from_metadata(&meta, "u", "c", None).is_none());
    }

    /// Fake `McpCaller` returning a fixed result, so the runner is exercised
    /// end-to-end without a live MCP client or the DB store.
    struct FakeCaller {
        out: Result<String, String>,
    }

    #[async_trait]
    impl McpCaller for FakeCaller {
        async fn call(
            &self,
            _user_id: &str,
            _server: &str,
            _tool: &str,
            _params: serde_json::Value,
        ) -> Result<String, String> {
            self.out.clone()
        }
    }

    fn test_safety() -> Arc<SafetyLayer> {
        Arc::new(SafetyLayer::new(&ironclaw_safety::SafetyConfig {
            max_output_length: 100_000,
            injection_check_enabled: false,
        }))
    }

    fn test_spec() -> McpJobSpec {
        McpJobSpec {
            server: "msbsandbox".into(),
            tool: "run_python".into(),
            params: serde_json::json!({"code":"print(1)"}),
            user_id: "u1".into(),
            channel: "gateway".into(),
            thread_id: Some("t1".into()),
        }
    }

    #[tokio::test]
    async fn mcp_job_runs_completes_and_injects() {
        let cm = Arc::new(ContextManager::new(5));
        let job_id = cm.create_job("bg", "run tool").await.unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);

        let deps = McpJobDeps {
            context_manager: cm.clone(),
            store: None,
            sse_tx: None,
            inject_tx: Some(tx),
            caller: Arc::new(FakeCaller {
                out: Ok("RESULT_OK".into()),
            }),
            safety: test_safety(),
        };
        run_mcp_job(job_id, test_spec(), deps).await;

        // Job reached Completed in the context manager.
        let ctx = cm.get_context(job_id).await.unwrap();
        assert_eq!(ctx.state, JobState::Completed);

        // The originating thread received the wrapped result as an internal msg.
        let msg = rx.try_recv().expect("an inject message");
        assert!(
            msg.content.contains("RESULT_OK"),
            "injected body was: {}",
            msg.content
        );
        assert!(
            msg.is_internal,
            "injected message must bypass user pipeline"
        );
        assert_eq!(msg.thread_id.as_ref().map(|t| t.as_str()), Some("t1"));
    }

    #[tokio::test]
    async fn mcp_job_failure_marks_failed_and_still_injects() {
        let cm = Arc::new(ContextManager::new(5));
        let job_id = cm.create_job("bg", "run tool").await.unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);

        let deps = McpJobDeps {
            context_manager: cm.clone(),
            store: None,
            sse_tx: None,
            inject_tx: Some(tx),
            caller: Arc::new(FakeCaller {
                out: Err("BOOM".into()),
            }),
            safety: test_safety(),
        };
        run_mcp_job(job_id, test_spec(), deps).await;

        let ctx = cm.get_context(job_id).await.unwrap();
        assert_eq!(ctx.state, JobState::Failed);
        let msg = rx.try_recv().expect("an inject message even on failure");
        assert!(msg.content.contains("failed"), "body: {}", msg.content);
        assert!(msg.content.contains("BOOM"), "body: {}", msg.content);
    }
}
