//! Channel-agnostic tool dispatch with audit trail.
//!
//! `ToolDispatcher` is the universal entry point for executing tools from
//! any non-agent caller — gateway handlers, CLI commands, routine engines,
//! or other channels. It creates a fresh system job for FK integrity,
//! executes the tool, records an `ActionRecord`, and returns the result.
//!
//! This is a third entry point alongside:
//! - v1: `Worker::execute_tool()` (agent agentic loop — has its own sequence tracking)
//! - v2: `EffectBridgeAdapter::execute_action()` (engine Python orchestrator)
//!
//! All three converge on the same `ToolRegistry`. Agent-initiated tool calls
//! must go through the agent's worker (which manages action sequence numbers
//! atomically); the dispatcher is only for callers that don't have an
//! existing agent job context.

use std::sync::Arc;
use std::time::Instant;

use tracing::debug;
use uuid::Uuid;

use crate::context::{ActionRecord, JobContext};
use crate::db::Database;
use crate::tools::registry::ToolRegistry;
use crate::tools::tool::{ToolError, ToolOutput};
use crate::tools::{prepare_tool_params, redact_params};
use ironclaw_safety::SafetyLayer;

/// Identifies where a tool dispatch originated.
///
/// `Channel` is intentionally a `String`, not an enum — channels are
/// extensions that can appear at runtime (gateway, CLI, telegram, slack,
/// WASM channels, future custom channels). Each dispatch creates a fresh
/// system job for audit trail purposes; agent-initiated tool calls must
/// use `Worker::execute_tool()` instead, which manages sequence numbers
/// against the agent's existing job.
#[derive(Debug, Clone)]
pub enum DispatchSource {
    /// A channel-initiated operation (gateway, CLI, telegram, etc.).
    Channel(String),
    /// A routine engine operation.
    Routine { routine_id: Uuid },
    /// An internal system operation.
    System,
}

impl std::fmt::Display for DispatchSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Channel(name) => write!(f, "channel:{name}"),
            Self::Routine { routine_id } => write!(f, "routine:{routine_id}"),
            Self::System => write!(f, "system"),
        }
    }
}

/// Channel-agnostic tool dispatcher with audit trail.
///
/// Wraps `ToolRegistry` + `SafetyLayer` + `Database` to provide a single
/// dispatch function that any caller can use to execute tools with the
/// same safety pipeline as the agent worker (param normalization, schema
/// validation, sensitive-param redaction, per-tool timeout, output
/// sanitization) plus `ActionRecord` persistence.
pub struct ToolDispatcher {
    registry: Arc<ToolRegistry>,
    safety: Arc<SafetyLayer>,
    store: Arc<dyn Database>,
}

impl ToolDispatcher {
    /// Create a new dispatcher.
    pub fn new(
        registry: Arc<ToolRegistry>,
        safety: Arc<SafetyLayer>,
        store: Arc<dyn Database>,
    ) -> Self {
        Self {
            registry,
            safety,
            store,
        }
    }

    /// Execute a tool by name with the given parameters.
    ///
    /// Pipeline (mirrors `Worker::execute_tool`):
    /// 1. Resolve the tool from the registry
    /// 2. Normalize parameters via `prepare_tool_params`
    /// 3. Validate parameters via `SafetyLayer::validator()`
    /// 4. Redact sensitive parameters for logging and audit
    /// 5. Create a fresh system job for FK integrity
    /// 6. Execute with the tool's per-tool timeout
    /// 7. Sanitize the result via `SafetyLayer::sanitize_tool_output`
    /// 8. Persist an `ActionRecord` with redacted params and sanitized output
    /// 9. Return the original `ToolOutput`
    ///
    /// Approval checks are skipped — channel-initiated operations are
    /// user-confirmed by definition. Audit-trail persistence failures are
    /// logged via `tracing::debug!` but do not mask the tool result —
    /// `debug!` is used (not `warn!`/`info!`) because dispatch calls may
    /// originate from interactive CLI/REPL sessions where `info!`/`warn!`
    /// output corrupts the terminal UI. See CLAUDE.md (Code Style → logging).
    pub async fn dispatch(
        &self,
        tool_name: &str,
        params: serde_json::Value,
        user_id: &str,
        source: DispatchSource,
    ) -> Result<ToolOutput, ToolError> {
        let (resolved_name, tool) =
            self.registry.get_resolved(tool_name).await.ok_or_else(|| {
                ToolError::ExecutionFailed(format!("tool not found: {tool_name}"))
            })?;

        // 1. Normalize parameters (coerce types, fill defaults).
        let normalized_params = prepare_tool_params(tool.as_ref(), &params);

        // 2. Schema validation.
        let validation = self
            .safety
            .validator()
            .validate_tool_params(&normalized_params);
        if !validation.is_valid {
            let details = validation
                .errors
                .iter()
                .map(|e| format!("{}: {}", e.field, e.message))
                .collect::<Vec<_>>()
                .join("; ");
            return Err(ToolError::InvalidParameters(format!(
                "Invalid tool parameters: {details}"
            )));
        }

        // 3. Redact sensitive params for log + audit. Sensitive values are
        //    still passed to the tool itself (via normalized_params), but
        //    never appear in the audit row or the dispatch log.
        let safe_params = redact_params(&normalized_params, tool.sensitive_params());

        // 4. Create a fresh system job for audit trail. Each dispatch
        //    becomes its own group of actions — sequence_num starts at 0
        //    with no risk of UNIQUE(job_id, sequence_num) collision.
        let source_label = source.to_string();
        let job_id = self
            .store
            .create_system_job(user_id, &source_label)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("failed to create system job: {e}")))?;

        let ctx = JobContext::system(user_id, job_id);
        let start = Instant::now();

        debug!(
            tool = %resolved_name,
            source = %source,
            user_id = %user_id,
            params = %safe_params,
            "dispatching tool"
        );

        // 5. Execute with per-tool timeout.
        let timeout = tool.execution_timeout();
        let result = tokio::time::timeout(timeout, tool.execute(normalized_params, &ctx)).await;
        let elapsed = start.elapsed();

        let final_result: Result<ToolOutput, ToolError> = match result {
            Ok(Ok(output)) => Ok(output),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(ToolError::Timeout(timeout)),
        };

        // 6. Build the ActionRecord with sanitized output (mirrors worker pattern).
        let action = ActionRecord::new(0, &resolved_name, safe_params);
        let action = match &final_result {
            Ok(output) => {
                let sanitized = serde_json::to_string_pretty(&output.result)
                    .ok()
                    .map(|s| self.safety.sanitize_tool_output(&resolved_name, &s).content);
                action.succeed(sanitized, output.result.clone(), elapsed)
            }
            Err(e) => action.fail(e.to_string(), elapsed),
        };

        // 7. Persist the audit record. Awaited (not spawned) so short-lived
        //    callers (CLI commands) cannot terminate before the row is written.
        if let Err(e) = self.store.save_action(job_id, &action).await {
            // `debug!` not `warn!`: dispatch is reachable from interactive
            // REPL/CLI channels where `warn!`/`info!` output corrupts the
            // terminal UI (CLAUDE.md → Code Style → logging).
            debug!(
                error = %e,
                tool = %resolved_name,
                job_id = %job_id,
                "failed to persist dispatch ActionRecord"
            );
        }

        final_result
    }

    /// Access the underlying tool registry.
    pub fn registry(&self) -> &Arc<ToolRegistry> {
        &self.registry
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_source_display() {
        assert_eq!(
            DispatchSource::Channel("gateway".into()).to_string(),
            "channel:gateway"
        );
        let id = Uuid::nil();
        assert_eq!(
            DispatchSource::Routine { routine_id: id }.to_string(),
            format!("routine:{id}")
        );
        assert_eq!(DispatchSource::System.to_string(), "system");
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Integration tests for the full `ToolDispatcher::dispatch` pipeline.
//
// These tests exercise the load-bearing new code path (`dispatch` called
// against a real tool, with a real libSQL-backed store), not just the
// `DispatchSource::Display` formatting above. They assert the four
// invariants the dispatcher promises callers:
//
// 1. **Audit trail persistence** — a row lands in `agent_jobs` + a
//    matching `ActionRecord` lands in `job_actions`.
// 2. **Sensitive parameter redaction** — `sensitive_params()` values
//    appear as `"[REDACTED]"` in the persisted audit row (but the tool
//    itself still sees the raw value).
// 3. **Per-tool timeout honored** — a slow tool is aborted at the
//    boundary declared by `execution_timeout()`.
// 4. **Output sanitization runs** — `SafetyLayer::sanitize_tool_output`
//    is applied before the pretty-printed output is stored.
// ────────────────────────────────────────────────────────────────────────────
#[cfg(all(test, feature = "libsql"))]
mod integration_tests {
    use super::*;
    use crate::config::SafetyConfig;
    use crate::context::JobContext;
    use crate::db::Database;
    use crate::db::libsql::LibSqlBackend;
    use crate::tools::tool::{Tool, ToolError, ToolOutput};
    use async_trait::async_trait;
    use ironclaw_safety::SafetyLayer;
    use std::time::Duration;

    // ── Stub tools ──────────────────────────────────────────

    /// Echoes its input back; declares `api_key` as sensitive so the
    /// dispatcher must redact it in the persisted audit row.
    struct RecordingTool {
        captured: Arc<std::sync::Mutex<Option<serde_json::Value>>>,
    }

    #[async_trait]
    impl Tool for RecordingTool {
        fn name(&self) -> &str {
            "recording_stub"
        }
        fn description(&self) -> &str {
            "Test stub that captures params and echoes them back."
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "message": { "type": "string" },
                    "api_key": { "type": "string" }
                },
                "required": ["message"]
            })
        }
        fn sensitive_params(&self) -> &[&str] {
            &["api_key"]
        }
        async fn execute(
            &self,
            params: serde_json::Value,
            _ctx: &JobContext,
        ) -> Result<ToolOutput, ToolError> {
            *self.captured.lock().expect("captured lock") = Some(params.clone());
            Ok(ToolOutput::success(
                serde_json::json!({
                    "echo": params.get("message").cloned().unwrap_or(serde_json::Value::Null),
                    "saw_api_key": params.get("api_key").is_some(),
                }),
                Duration::from_millis(1),
            ))
        }
    }

    /// Sleeps forever (well, 60s) but declares a 100ms timeout so the
    /// dispatcher aborts it quickly and records a failure.
    struct SlowTool;

    #[async_trait]
    impl Tool for SlowTool {
        fn name(&self) -> &str {
            "slow_stub"
        }
        fn description(&self) -> &str {
            "Test stub that sleeps past its declared timeout."
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({ "type": "object" })
        }
        fn execution_timeout(&self) -> Duration {
            Duration::from_millis(100)
        }
        async fn execute(
            &self,
            _params: serde_json::Value,
            _ctx: &JobContext,
        ) -> Result<ToolOutput, ToolError> {
            tokio::time::sleep(Duration::from_secs(60)).await;
            unreachable!("slow_stub should have been killed by its per-tool timeout")
        }
    }

    // ── Fixtures ────────────────────────────────────────────

    async fn test_dispatcher() -> (
        Arc<ToolDispatcher>,
        Arc<LibSqlBackend>,
        Arc<dyn Database>,
        Arc<ToolRegistry>,
        tempfile::TempDir,
    ) {
        let dir = tempfile::tempdir().expect("tempdir");
        let concrete = Arc::new(
            LibSqlBackend::new_local(&dir.path().join("test.db"))
                .await
                .expect("libsql backend"),
        );
        concrete.run_migrations().await.expect("migrations");
        let db: Arc<dyn Database> = Arc::clone(&concrete) as Arc<dyn Database>;

        // Bootstrap the single-user owner row so FK constraints on
        // agent_jobs.user_id are satisfied.
        use crate::db::UserRecord;
        let now = chrono::Utc::now();
        db.create_user(&UserRecord {
            id: "tester".to_string(),
            email: None,
            display_name: "tester".to_string(),
            status: "active".to_string(),
            role: "admin".to_string(),
            created_at: now,
            updated_at: now,
            last_login_at: None,
            created_by: None,
            metadata: serde_json::json!({}),
        })
        .await
        .expect("create user");

        let registry = Arc::new(ToolRegistry::new());
        let safety = Arc::new(SafetyLayer::new(&SafetyConfig {
            max_output_length: 65_536,
            injection_check_enabled: false,
        }));
        let dispatcher = Arc::new(ToolDispatcher::new(
            Arc::clone(&registry),
            safety,
            Arc::clone(&db),
        ));
        (dispatcher, concrete, db, registry, dir)
    }

    /// Fetch every system-category job for the test user. `list_agent_jobs_for_user`
    /// intentionally filters to `source = 'direct'` so system dispatches never
    /// pollute agent-job listings — the test needs a direct query to bypass
    /// that filter.
    async fn fetch_system_jobs_for_user(
        backend: &LibSqlBackend,
        user_id: &str,
    ) -> Vec<(Uuid, String)> {
        use libsql::params;
        let conn = backend.connect().await.expect("connect");
        let mut rows = conn
            .query(
                r#"
                SELECT id, title FROM agent_jobs
                WHERE category = 'system' AND user_id = ?1
                ORDER BY created_at DESC
                "#,
                params![user_id],
            )
            .await
            .expect("query");
        let mut out = Vec::new();
        while let Some(row) = rows.next().await.expect("next") {
            let id_str: String = row.get(0).expect("id text");
            let title: String = row.get(1).expect("title text");
            if let Ok(id) = id_str.parse::<Uuid>() {
                out.push((id, title));
            }
        }
        out
    }

    // ── Tests ───────────────────────────────────────────────

    #[tokio::test]
    async fn dispatch_persists_action_record_with_redacted_sensitive_params() {
        let (dispatcher, backend, db, registry, _dir) = test_dispatcher().await;
        let captured = Arc::new(std::sync::Mutex::new(None));
        registry
            .register(Arc::new(RecordingTool {
                captured: Arc::clone(&captured),
            }))
            .await;

        let output = dispatcher
            .dispatch(
                "recording_stub",
                serde_json::json!({
                    "message": "hello world",
                    "api_key": "super-secret-value"
                }),
                "tester",
                DispatchSource::Channel("gateway".into()),
            )
            .await
            .expect("dispatch should succeed");

        // Invariant 1: tool sees the RAW (un-redacted) params — the
        // `api_key` value must reach the tool itself.
        let seen = captured.lock().unwrap().clone().expect("tool was called");
        assert_eq!(
            seen.get("api_key").and_then(|v| v.as_str()),
            Some("super-secret-value"),
            "tool must see the real sensitive value"
        );
        // Sanity: output contains what the tool returned.
        assert_eq!(
            output.result.get("echo").and_then(|v| v.as_str()),
            Some("hello world")
        );

        // Invariant 2: a system job was created + the ActionRecord persisted.
        // `create_system_job` sets `title = format!("System: {source}")`, so
        // locate our job by the display-form of our `DispatchSource`. Use the
        // raw-SQL helper because `list_agent_jobs_for_user` filters out
        // `category = 'system'` rows on purpose.
        let system_jobs = fetch_system_jobs_for_user(&backend, "tester").await;
        let (system_job_id, _) = system_jobs
            .iter()
            .find(|(_, title)| title == "System: channel:gateway")
            .cloned()
            .expect("system job for the channel:gateway dispatch");
        let actions = db
            .get_job_actions(system_job_id)
            .await
            .expect("get job actions");
        assert_eq!(actions.len(), 1, "exactly one action per dispatched call");
        let action = &actions[0];
        assert_eq!(action.tool_name, "recording_stub");
        assert!(
            action.success,
            "action should be marked success for a Ok(ToolOutput) return"
        );

        // Invariant 3: sensitive params are redacted in the persisted
        // `input` on the audit row. Non-sensitive values survive; the
        // sensitive one becomes the `[REDACTED]` sentinel.
        let persisted_input = &action.input;
        assert_eq!(
            persisted_input.get("message").and_then(|v| v.as_str()),
            Some("hello world"),
            "non-sensitive params must survive redaction: {persisted_input}"
        );
        assert_eq!(
            persisted_input.get("api_key").and_then(|v| v.as_str()),
            Some("[REDACTED]"),
            "sensitive value must be redacted in the audit row: {persisted_input}"
        );
        let persisted_json = persisted_input.to_string();
        assert!(
            !persisted_json.contains("super-secret-value"),
            "raw sensitive value must not appear anywhere in the audit row: {persisted_json}"
        );

        // Invariant 4: sanitized output is populated (sanitization ran).
        assert!(
            action.output_sanitized.is_some(),
            "output_sanitized should be populated on the audit row"
        );
    }

    #[tokio::test]
    async fn dispatch_honors_per_tool_timeout_and_records_failure() {
        let (dispatcher, backend, db, registry, _dir) = test_dispatcher().await;
        registry.register(Arc::new(SlowTool)).await;

        let start = Instant::now();
        let result = dispatcher
            .dispatch(
                "slow_stub",
                serde_json::json!({}),
                "tester",
                DispatchSource::System,
            )
            .await;
        let elapsed = start.elapsed();

        // Must return a `Timeout` error after roughly the 100ms bound — not
        // the 60s the tool would sleep for.
        assert!(
            matches!(result, Err(ToolError::Timeout(_))),
            "expected Timeout, got {result:?}"
        );
        assert!(
            elapsed < Duration::from_secs(2),
            "dispatch must have enforced the per-tool 100ms timeout; actually slept {elapsed:?}"
        );

        // The audit row should still land, marked as a failure.
        let system_jobs = fetch_system_jobs_for_user(&backend, "tester").await;
        let (system_job_id, _) = system_jobs
            .iter()
            .find(|(_, title)| title == "System: system")
            .cloned()
            .expect("system job for the System-source dispatch");
        let actions = db
            .get_job_actions(system_job_id)
            .await
            .expect("get job actions");
        assert_eq!(
            actions.len(),
            1,
            "timeout should still record exactly one action"
        );
        assert!(
            !actions[0].success,
            "timed-out action must be marked success=false"
        );
    }
}
