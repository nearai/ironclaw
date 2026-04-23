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
        active_skill_names: Vec::new(),
        source_channel: None,
        user_timezone: None,
        thread_goal: None,
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
