//! Integration tests for the engine v2 sandbox interception path.
//!
//! These tests drive `EffectBridgeAdapter::execute_action()` end-to-end —
//! the public surface that the engine v2 ExecutionLoop calls. They verify
//! that when a `WorkspaceMounts` table is installed and a sandbox-eligible
//! tool call carries a `/project/...` path, the call is dispatched through
//! the mount backend rather than the host tool registry.
//!
//! Why this is in `tests/` and not in a `mod tests` block: per
//! `.claude/rules/testing.md` ("Test Through the Caller, Not Just the
//! Helper"), `maybe_intercept` is a predicate that gates a side effect
//! (filesystem write/read), called from a wrapper (`execute_action_internal`)
//! whose call site is `execute_action`. A unit test on the helper alone
//! is **not sufficient** regression coverage — these tests close that gap
//! by driving the call site that production code actually invokes.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;

use ironclaw_engine::types::capability::LeaseId;
use ironclaw_engine::workspace::FilesystemBackend;
use ironclaw_engine::{
    CapabilityLease, EffectExecutor, GrantedActions, MountError, ProjectId, ProjectMountFactory,
    ProjectMounts, StepId, ThreadExecutionContext, ThreadId, ThreadType, WorkspaceMounts,
};

use ironclaw::bridge::EffectBridgeAdapter;
use ironclaw::hooks::HookRegistry;
use ironclaw::tools::ToolRegistry;
use ironclaw_safety::{SafetyConfig, SafetyLayer};

/// Simple factory: every project gets a `FilesystemBackend` rooted at the
/// supplied tempdir. Used by every test in this file.
#[derive(Debug)]
struct StaticFsFactory {
    root: PathBuf,
}

#[async_trait]
impl ProjectMountFactory for StaticFsFactory {
    async fn build(&self, _: ProjectId) -> Result<ProjectMounts, MountError> {
        let mut mounts = ProjectMounts::new();
        mounts.add(
            "/project/",
            Arc::new(FilesystemBackend::new(self.root.clone())),
        );
        Ok(mounts)
    }
}

/// Build an adapter with no host tools registered. Sandbox interception runs
/// before the host tool lookup, so unregistered tools never reach the
/// registry — that proves the test's outcome is from the mount backend, not
/// from a coincidentally-registered host tool.
fn make_adapter() -> Arc<EffectBridgeAdapter> {
    Arc::new(EffectBridgeAdapter::new(
        Arc::new(ToolRegistry::new()),
        Arc::new(SafetyLayer::new(&SafetyConfig {
            max_output_length: 100_000,
            injection_check_enabled: false,
        })),
        Arc::new(HookRegistry::default()),
        None,
    ))
}

fn make_lease(thread_id: ThreadId) -> CapabilityLease {
    CapabilityLease {
        id: LeaseId::new(),
        thread_id,
        capability_name: "fs.test".into(),
        granted_actions: GrantedActions::All,
        granted_at: Utc::now(),
        expires_at: None,
        max_uses: None,
        uses_remaining: None,
        revoked: false,
        revoked_reason: None,
    }
}

fn make_context(project_id: ProjectId) -> ThreadExecutionContext {
    ThreadExecutionContext {
        thread_id: ThreadId::new(),
        thread_type: ThreadType::Foreground,
        project_id,
        user_id: "test-user".into(),
        step_id: StepId::new(),
        current_call_id: Some("call_test_1".into()),
        source_channel: None,
        user_timezone: None,
        thread_goal: None,
        available_actions_snapshot: None,
        available_action_inventory_snapshot: None,
        gate_controller: ironclaw_engine::CancellingGateController::arc(),
        call_approval_granted: false,
        conversation_id: None,
    }
}

#[tokio::test]
async fn execute_action_writes_through_sandbox_mount() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let factory = StaticFsFactory {
        root: tempdir.path().to_path_buf(),
    };
    let mounts = Arc::new(WorkspaceMounts::new(Arc::new(factory)));

    let adapter = make_adapter();
    adapter
        .set_workspace_mounts(Some(Arc::clone(&mounts)))
        .await;

    let project_id = ProjectId::new();
    let ctx = make_context(project_id);
    let lease = make_lease(ctx.thread_id);

    let result = adapter
        .execute_action(
            "file_write",
            serde_json::json!({"path": "/project/foo.txt", "content": "hello sandbox"}),
            &lease,
            &ctx,
        )
        .await
        .expect("execute_action should succeed");

    assert!(
        !result.is_error,
        "expected success, got: {:?}",
        result.output
    );
    assert_eq!(result.action_name, "file_write");

    // The interception path produces a JSON-serialized response with these
    // fields. The ActionResult.output may be either a string (when the
    // sanitization wrapper kicks in) or already-parsed JSON, depending on
    // the safety layer's choices. Verify the bytes_written field appears
    // wherever the value lives.
    let serialized = serde_json::to_string(&result.output).unwrap();
    assert!(
        serialized.contains("bytes_written") && serialized.contains("13"),
        "expected serialized output to mention bytes_written and length 13: {serialized}"
    );

    // Most importantly: verify the file actually landed on disk in the
    // tempdir. This is the load-bearing assertion — without it, a buggy
    // interceptor could return a fake-looking JSON without ever calling
    // the backend.
    let written = std::fs::read_to_string(tempdir.path().join("foo.txt"))
        .expect("file should exist on disk after intercept");
    assert_eq!(written, "hello sandbox");
}

#[tokio::test]
async fn execute_action_reads_through_sandbox_mount() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    std::fs::write(tempdir.path().join("greeting.txt"), b"hi from disk").unwrap();

    let factory = StaticFsFactory {
        root: tempdir.path().to_path_buf(),
    };
    let mounts = Arc::new(WorkspaceMounts::new(Arc::new(factory)));
    let adapter = make_adapter();
    adapter
        .set_workspace_mounts(Some(Arc::clone(&mounts)))
        .await;

    let project_id = ProjectId::new();
    let ctx = make_context(project_id);
    let lease = make_lease(ctx.thread_id);

    let result = adapter
        .execute_action(
            "file_read",
            serde_json::json!({"path": "/project/greeting.txt"}),
            &lease,
            &ctx,
        )
        .await
        .expect("execute_action should succeed");

    assert!(!result.is_error);
    let serialized = serde_json::to_string(&result.output).unwrap();
    assert!(
        serialized.contains("hi from disk"),
        "expected output to contain file contents: {serialized}"
    );
}

#[tokio::test]
async fn no_workspace_mounts_falls_through_to_host_registry() {
    // With no mounts installed, the bridge MUST fall through to host tool
    // execution. Since we didn't register file_read in the registry, the
    // host execution path returns an error — that's the signal that the
    // sandbox path was correctly skipped (rather than silently swallowing
    // the call).
    let adapter = make_adapter();
    // Intentionally NOT calling set_workspace_mounts.

    let project_id = ProjectId::new();
    let ctx = make_context(project_id);
    let lease = make_lease(ctx.thread_id);

    let outcome = adapter
        .execute_action(
            "file_read",
            serde_json::json!({"path": "/project/whatever.txt"}),
            &lease,
            &ctx,
        )
        .await;

    // No mount table → falls through to host. Host has no `file_read` tool
    // registered → returns an error. The error is wrapped as `is_error: true`
    // in an ActionResult; either way, the call did NOT silently succeed.
    match outcome {
        Ok(result) if !result.is_error => panic!(
            "expected fall-through to host to fail (no tool registered), got success: {:?}",
            result.output
        ),
        _ => {} // either Err(...) or Ok(ActionResult { is_error: true, .. })
    }
}

#[tokio::test]
async fn host_path_falls_through_even_when_mounts_installed() {
    // When mounts are installed, paths that aren't under any mount prefix
    // (e.g. /Users/coder/notes.md, /etc/passwd) must still fall through
    // to the host registry rather than being silently routed into the
    // mount table. This is the case where the agent is intentionally
    // operating on host files outside the sandbox.
    let tempdir = tempfile::tempdir().expect("tempdir");
    let factory = StaticFsFactory {
        root: tempdir.path().to_path_buf(),
    };
    let mounts = Arc::new(WorkspaceMounts::new(Arc::new(factory)));
    let adapter = make_adapter();
    adapter
        .set_workspace_mounts(Some(Arc::clone(&mounts)))
        .await;

    let project_id = ProjectId::new();
    let ctx = make_context(project_id);
    let lease = make_lease(ctx.thread_id);

    let outcome = adapter
        .execute_action(
            "file_read",
            serde_json::json!({"path": "/Users/coder/notes.md"}),
            &lease,
            &ctx,
        )
        .await;

    // The path doesn't match `/project/` so the interceptor falls through.
    // Host registry has no file_read → error. Critically, the file in our
    // tempdir was NOT touched (would have been wrong if the interceptor
    // had silently mapped /Users/... to the project root).
    match outcome {
        Ok(result) if !result.is_error => panic!(
            "expected host fall-through error, got success: {:?}",
            result.output
        ),
        _ => {}
    }
    // tempdir should still be empty
    let entries: Vec<_> = std::fs::read_dir(tempdir.path()).unwrap().collect();
    assert_eq!(
        entries.len(),
        0,
        "sandbox tempdir should not have been written"
    );
}

#[tokio::test]
async fn invalid_project_path_surfaces_error_not_silent_fall_through() {
    // A path that DOES start with /project/ but contains a `..` escape
    // (e.g. /project/../etc/passwd) must be rejected by the mount backend.
    // The critical assertion is that:
    //   1. The call returns an error result (`is_error: true`), and
    //   2. No file outside the sandbox root was actually read.
    //
    // We deliberately do NOT assert on the specific error message text,
    // because the bridge runs `SafetyLayer::sanitize_tool_output` over
    // error messages and may redact sensitive-looking paths (like
    // `/etc/passwd`) to a generic block string. That's defense in depth
    // working as intended; the test just verifies that the call did NOT
    // succeed and did NOT exfiltrate `/etc/passwd` content into the result.
    let tempdir = tempfile::tempdir().expect("tempdir");
    let factory = StaticFsFactory {
        root: tempdir.path().to_path_buf(),
    };
    let mounts = Arc::new(WorkspaceMounts::new(Arc::new(factory)));
    let adapter = make_adapter();
    adapter
        .set_workspace_mounts(Some(Arc::clone(&mounts)))
        .await;

    let project_id = ProjectId::new();
    let ctx = make_context(project_id);
    let lease = make_lease(ctx.thread_id);

    let outcome = adapter
        .execute_action(
            "file_read",
            serde_json::json!({"path": "/project/../etc/passwd"}),
            &lease,
            &ctx,
        )
        .await;

    match outcome {
        Ok(result) => {
            assert!(
                result.is_error,
                "sandbox must reject `..` escape, got success: {:?}",
                result.output
            );
            // Whatever the (possibly redacted) error message says, the
            // result must NOT contain content from /etc/passwd. On most
            // systems /etc/passwd contains "root:" — confirm that string
            // does not appear in the response.
            let serialized = serde_json::to_string(&result.output).unwrap();
            assert!(
                !serialized.contains("root:"),
                "result must not leak /etc/passwd content: {serialized}"
            );
        }
        Err(e) => {
            // Errors at this layer are also acceptable — what matters is
            // that the call did not succeed.
            let s = e.to_string();
            assert!(
                !s.contains("root:"),
                "error must not leak /etc/passwd content: {s}"
            );
        }
    }
}

// ── Audited-funnel coverage (#4019 follow-up) ──
//
// Both `execute_action` branches must persist an `ActionRecord` with redacted
// sensitive params when the adapter holds an `Arc<dyn Database>`:
//   * host (non-intercepted) branch → routes through `execute_tool_audited`;
//   * sandbox/mount-intercepted branch → persists the same audit shape via the
//     shared `persist_tool_audit` helper from the already-produced intercepted
//     result.
// These tests drive the public `execute_action` caller (per the testing rule:
// test through the caller, not just the helper) and read the persisted rows
// back out of the database to confirm the audit landed and the secret was
// redacted.
#[cfg(feature = "libsql")]
mod audited_funnel {
    use super::*;

    use ironclaw::context::{ActionRecord, JobContext};
    use ironclaw::db::Database;
    use ironclaw::db::libsql::LibSqlBackend;
    use ironclaw::tools::{ApprovalRequirement, Tool, ToolError, ToolOutput};
    use uuid::Uuid;

    /// A host tool with a sensitive `secret` param, used to prove the audited
    /// funnel redacts before persisting. Returns the (non-secret) echo and
    /// requires no approval so `execute_action` runs the host branch end to
    /// end without a gate.
    struct SecretEchoTool;

    #[async_trait]
    impl Tool for SecretEchoTool {
        fn name(&self) -> &str {
            "audit_echo"
        }
        fn description(&self) -> &str {
            "Echoes its non-secret input; has one sensitive param."
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "note": { "type": "string" },
                    "secret": { "type": "string" }
                }
            })
        }
        fn sensitive_params(&self) -> &[&str] {
            &["secret"]
        }
        async fn execute(
            &self,
            params: serde_json::Value,
            _ctx: &JobContext,
        ) -> Result<ToolOutput, ToolError> {
            let note = params
                .get("note")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            Ok(ToolOutput::success(
                serde_json::json!({ "note": note }),
                std::time::Duration::from_millis(1),
            ))
        }
        fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
            ApprovalRequirement::Never
        }
    }

    /// Connect a fresh libsql test DB (concrete backend + trait object) and
    /// pre-create the user so `create_system_job`'s FK to `users` is satisfied.
    async fn test_db() -> (Arc<dyn Database>, Arc<LibSqlBackend>, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let backend = Arc::new(
            LibSqlBackend::new_local(&dir.path().join("audit.db"))
                .await
                .expect("LibSqlBackend"),
        );
        let db: Arc<dyn Database> = Arc::clone(&backend) as Arc<dyn Database>;
        db.run_migrations().await.expect("migrations");
        let now = chrono::Utc::now();
        db.create_user(&ironclaw::db::UserRecord {
            id: "test-user".to_string(),
            email: None,
            display_name: "test-user".to_string(),
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
        (db, backend, dir)
    }

    fn adapter_with_db(
        db: Arc<dyn Database>,
        tools: Arc<ToolRegistry>,
    ) -> Arc<EffectBridgeAdapter> {
        Arc::new(EffectBridgeAdapter::new(
            tools,
            Arc::new(SafetyLayer::new(&SafetyConfig {
                max_output_length: 100_000,
                injection_check_enabled: false,
            })),
            Arc::new(HookRegistry::default()),
            Some(db),
        ))
    }

    /// Collect every ActionRecord persisted under the user's system jobs (the
    /// audited funnel mints `category = 'system'` jobs when the caller has no
    /// existing persisted job — the bridge's case).
    async fn system_actions(backend: &LibSqlBackend, db: &Arc<dyn Database>) -> Vec<ActionRecord> {
        use libsql::params;
        let conn = backend.connect().await.expect("connect");
        let mut rows = conn
            .query(
                "SELECT id FROM agent_jobs WHERE category = 'system' AND user_id = ?1",
                params!["test-user"],
            )
            .await
            .expect("query system jobs");
        let mut actions = Vec::new();
        while let Some(row) = rows.next().await.expect("next row") {
            let id_str: String = row.get(0).expect("job id");
            if let Ok(job_id) = id_str.parse::<Uuid>() {
                actions.extend(db.get_job_actions(job_id).await.expect("get job actions"));
            }
        }
        actions
    }

    /// Collect the `title` of every system job (the audited funnel encodes the
    /// `DispatchSource` label into the title as `System: <source>` —
    /// `create_system_job` stores the source there, the `source` column is the
    /// fixed `"system"` category marker). Used to assert the audit attributes
    /// the originating channel rather than the synthetic `engine_v2` fallback.
    async fn system_job_titles(backend: &LibSqlBackend) -> Vec<String> {
        use libsql::params;
        let conn = backend.connect().await.expect("connect");
        let mut rows = conn
            .query(
                "SELECT title FROM agent_jobs WHERE category = 'system' AND user_id = ?1",
                params!["test-user"],
            )
            .await
            .expect("query system jobs");
        let mut titles = Vec::new();
        while let Some(row) = rows.next().await.expect("next row") {
            titles.push(row.get::<String>(0).expect("job title"));
        }
        titles
    }

    fn context_with_channel(project_id: ProjectId, channel: &str) -> ThreadExecutionContext {
        let mut ctx = make_context(project_id);
        ctx.source_channel = Some(channel.to_string());
        ctx
    }

    #[tokio::test]
    async fn host_branch_persists_redacted_action_record() {
        let (db, backend, _dir) = test_db().await;
        let tools = Arc::new(ToolRegistry::new());
        tools.register(Arc::new(SecretEchoTool)).await;

        // No workspace mounts installed → host (non-intercepted) branch.
        // Global auto-approve so the call clears the adapter's approval gate
        // and actually reaches execution (the audit path under test).
        let adapter = Arc::new(
            EffectBridgeAdapter::new(
                tools,
                Arc::new(SafetyLayer::new(&SafetyConfig {
                    max_output_length: 100_000,
                    injection_check_enabled: false,
                })),
                Arc::new(HookRegistry::default()),
                Some(Arc::clone(&db)),
            )
            .with_global_auto_approve(true),
        );

        let ctx = make_context(ProjectId::new());
        let lease = make_lease(ctx.thread_id);

        let result = adapter
            .execute_action(
                "audit_echo",
                serde_json::json!({ "note": "hello", "secret": "s3cr3t-value" }),
                &lease,
                &ctx,
            )
            .await
            .expect("host execute_action should succeed");
        assert!(!result.is_error, "host branch should succeed: {result:?}");

        let actions = system_actions(&backend, &db).await;
        assert_eq!(
            actions.len(),
            1,
            "host branch must persist exactly one ActionRecord, found {}",
            actions.len()
        );
        let action = &actions[0];
        assert_eq!(action.tool_name, "audit_echo");
        let serialized = serde_json::to_string(&action.input).expect("serialize input");
        assert!(
            !serialized.contains("s3cr3t-value"),
            "audit input must redact the sensitive `secret` param, got: {serialized}"
        );
        assert!(
            serialized.contains("hello"),
            "audit input should retain the non-sensitive `note` param, got: {serialized}"
        );
    }

    #[tokio::test]
    async fn sandbox_branch_persists_action_record() {
        let (db, backend, _dir) = test_db().await;
        // No host tools registered — sandbox interception fires before the
        // host tool lookup, so the audit row can only come from the sandbox
        // branch, not host execution.
        let tools = Arc::new(ToolRegistry::new());
        let adapter = adapter_with_db(Arc::clone(&db), tools);

        let mount_dir = tempfile::tempdir().expect("tempdir");
        let factory = StaticFsFactory {
            root: mount_dir.path().to_path_buf(),
        };
        let mounts = Arc::new(WorkspaceMounts::new(Arc::new(factory)));
        adapter
            .set_workspace_mounts(Some(Arc::clone(&mounts)))
            .await;

        let ctx = make_context(ProjectId::new());
        let lease = make_lease(ctx.thread_id);

        let result = adapter
            .execute_action(
                "file_write",
                serde_json::json!({ "path": "/project/audited.txt", "content": "sandbox audit" }),
                &lease,
                &ctx,
            )
            .await
            .expect("sandbox execute_action should succeed");
        assert!(
            !result.is_error,
            "sandbox branch should succeed: {result:?}"
        );

        let actions = system_actions(&backend, &db).await;
        assert_eq!(
            actions.len(),
            1,
            "sandbox branch must persist exactly one ActionRecord, found {}",
            actions.len()
        );
        assert_eq!(
            actions[0].tool_name, "file_write",
            "sandbox-dispatched call must produce the same audit shape (tool name) as host"
        );
    }

    /// The audit's `DispatchSource` must be derived from the thread's real
    /// `source_channel` (e.g. `gateway`/`slack`), not flattened to the
    /// synthetic `engine_v2` label. Drives the host branch with a distinct
    /// originating channel and asserts the persisted system job carries it.
    #[tokio::test]
    async fn audit_source_reflects_real_origin_channel() {
        let (db, backend, _dir) = test_db().await;
        let tools = Arc::new(ToolRegistry::new());
        tools.register(Arc::new(SecretEchoTool)).await;

        let adapter = Arc::new(
            EffectBridgeAdapter::new(
                tools,
                Arc::new(SafetyLayer::new(&SafetyConfig {
                    max_output_length: 100_000,
                    injection_check_enabled: false,
                })),
                Arc::new(HookRegistry::default()),
                Some(Arc::clone(&db)),
            )
            .with_global_auto_approve(true),
        );

        let ctx = context_with_channel(ProjectId::new(), "slack");
        let lease = make_lease(ctx.thread_id);

        let result = adapter
            .execute_action(
                "audit_echo",
                serde_json::json!({ "note": "hello", "secret": "s3cr3t-value" }),
                &lease,
                &ctx,
            )
            .await
            .expect("host execute_action should succeed");
        assert!(!result.is_error, "host branch should succeed: {result:?}");

        let titles = system_job_titles(&backend).await;
        assert_eq!(
            titles.len(),
            1,
            "expected exactly one system job, found {}: {titles:?}",
            titles.len()
        );
        assert!(
            titles[0].contains("channel:slack"),
            "audit must record the real origin channel `slack`, got: {}",
            titles[0]
        );
        assert!(
            !titles[0].contains("engine_v2"),
            "audit must not flatten a real channel to the `engine_v2` fallback, got: {}",
            titles[0]
        );
    }

    /// When the thread has no originating channel (a channel-less system
    /// thread), the audit falls back to the synthetic `engine_v2` label.
    #[tokio::test]
    async fn audit_source_falls_back_to_engine_v2_when_channelless() {
        let (db, backend, _dir) = test_db().await;
        let tools = Arc::new(ToolRegistry::new());
        tools.register(Arc::new(SecretEchoTool)).await;

        let adapter = Arc::new(
            EffectBridgeAdapter::new(
                tools,
                Arc::new(SafetyLayer::new(&SafetyConfig {
                    max_output_length: 100_000,
                    injection_check_enabled: false,
                })),
                Arc::new(HookRegistry::default()),
                Some(Arc::clone(&db)),
            )
            .with_global_auto_approve(true),
        );

        // `make_context` leaves `source_channel = None`.
        let ctx = make_context(ProjectId::new());
        let lease = make_lease(ctx.thread_id);

        adapter
            .execute_action(
                "audit_echo",
                serde_json::json!({ "note": "hello" }),
                &lease,
                &ctx,
            )
            .await
            .expect("host execute_action should succeed");

        let titles = system_job_titles(&backend).await;
        assert_eq!(
            titles.len(),
            1,
            "expected exactly one system job: {titles:?}"
        );
        assert!(
            titles[0].contains("channel:engine_v2"),
            "channel-less thread must fall back to `engine_v2`, got: {}",
            titles[0]
        );
    }
}
