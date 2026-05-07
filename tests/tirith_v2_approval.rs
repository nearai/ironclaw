//! Integration tests for the v2 effect-bridge tirith preflight
//! (`EffectBridgeAdapter::enforce_tool_permission`).
//!
//! Per the project's testing conventions (`/.claude/rules/testing.md` —
//! "Test Through the Caller, Not Just the Helper"), exercises drive
//! `EffectBridgeAdapter::execute_action()`, the public surface that the
//! engine v2 ExecutionLoop calls. This pins the invariants of the v2
//! tirith integration:
//!
//! 1. Block / Warn / WarnAck → `EngineError::GatePaused` with `reason: Some(_)`
//!    and `allow_always = false`.
//! 2. Fail-closed operational failures → `EngineError::LeaseDenied` (NEVER
//!    a GatePaused — clicking through fail-closed defeats fail-closed).
//! 3. `tirith.enabled = false` → tirith is not invoked, the existing v2
//!    permission logic decides.
//! 4. Non-shell tools never spawn the subprocess.
//!
//! Subprocess-spawning tests are Unix-only via the same fake-bin helper
//! used by `tests/tirith_preflight.rs`.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use tempfile::TempDir;

use ironclaw_engine::types::capability::{
    ActionDef, ActionDiscoveryMetadata, EffectType, LeaseId, ModelToolSurface,
};
use ironclaw_engine::{
    CapabilityLease, EffectExecutor, GrantedActions, ProjectId, StepId, ThreadExecutionContext,
    ThreadId, ThreadType,
};

use ironclaw::bridge::EffectBridgeAdapter;
use ironclaw::context::JobContext;
use ironclaw::hooks::HookRegistry;
use ironclaw::tools::builtin::TirithConfig;
use ironclaw::tools::{ApprovalRequirement, Tool, ToolError, ToolOutput, ToolRegistry};
use ironclaw_safety::{SafetyConfig, SafetyLayer};

// ── Fake tirith subprocess helper ──────────────────────────────

fn make_fake_tirith(exit: i32, stdout: &str) -> (TempDir, PathBuf) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("fake-tirith");
    let script = format!("#!/bin/sh\ncat <<'EOF'\n{stdout}\nEOF\nexit {exit}\n");
    std::fs::write(&path, script).expect("write");
    let mut perms = std::fs::metadata(&path).expect("meta").permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).expect("chmod");
    (tmp, path)
}

fn cfg_with(path: PathBuf, fail_open: bool) -> TirithConfig {
    TirithConfig {
        enabled: true,
        bin: path.to_str().unwrap().to_string(),
        timeout: Duration::from_secs(5),
        fail_open,
    }
}

// ── Mock tools ─────────────────────────────────────────────────

/// Minimal "shell" stand-in. `requires_approval = Never` so we can
/// distinguish a tirith-driven pause (we want) from the standard
/// always-approval path (we don't).
struct MockShell;

#[async_trait]
impl Tool for MockShell {
    fn name(&self) -> &str {
        "shell"
    }
    fn description(&self) -> &str {
        "Mock shell for tirith v2 wiring tests"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {"command": {"type": "string"}},
            "required": ["command"]
        })
    }
    async fn execute(
        &self,
        _params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput::text("ran", Duration::from_millis(1)))
    }
    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Never
    }
}

/// Non-shell tool. Tirith should never spawn for this name.
struct MockHttp;

#[async_trait]
impl Tool for MockHttp {
    fn name(&self) -> &str {
        "http"
    }
    fn description(&self) -> &str {
        "Mock http for non-shell short-circuit"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {"url": {"type": "string"}}
        })
    }
    async fn execute(
        &self,
        _params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput::text("ran", Duration::from_millis(1)))
    }
    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Never
    }
}

// ── Adapter scaffolding ────────────────────────────────────────

async fn make_adapter(tirith_cfg: Option<TirithConfig>) -> Arc<EffectBridgeAdapter> {
    let mut registry = ToolRegistry::new();
    if let Some(cfg) = tirith_cfg {
        registry = registry.with_tirith_config(cfg);
    }
    registry.register(Arc::new(MockShell)).await;
    registry.register(Arc::new(MockHttp)).await;
    Arc::new(EffectBridgeAdapter::new(
        Arc::new(registry),
        Arc::new(SafetyLayer::new(&SafetyConfig {
            max_output_length: 100_000,
            injection_check_enabled: false,
        })),
        Arc::new(HookRegistry::default()),
    ))
}

fn make_lease(thread_id: ThreadId, action: &str) -> CapabilityLease {
    CapabilityLease {
        id: LeaseId::new(),
        thread_id,
        capability_name: "tirith.test".into(),
        granted_actions: GrantedActions::Specific(vec![action.into()]),
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
        user_id: "tirith-test-user".into(),
        step_id: StepId::new(),
        current_call_id: Some("call_tirith_test_1".into()),
        source_channel: None,
        user_timezone: None,
        thread_goal: None,
        available_actions_snapshot: None,
        available_action_inventory_snapshot: None,
    }
}

fn shell_params(cmd: &str) -> serde_json::Value {
    serde_json::json!({"command": cmd})
}

// ── Tests ──────────────────────────────────────────────────────

#[tokio::test]
async fn block_pauses_v2_with_reason_and_allow_always_false() {
    let (_tmp, bin) = make_fake_tirith(
        1,
        r#"{"findings":[{"rule_id":"r1","severity":"HIGH","title":"homograph"}]}"#,
    );
    let adapter = make_adapter(Some(cfg_with(bin, true))).await;
    let ctx = make_context(ProjectId::new());
    let lease = make_lease(ctx.thread_id, "shell");

    let err = adapter
        .execute_action("shell", shell_params("ls"), &lease, &ctx)
        .await
        .expect_err("tirith block must produce a GatePaused");

    match err {
        ironclaw_engine::EngineError::GatePaused {
            reason,
            resume_kind,
            ..
        } => {
            let r = reason.expect("tirith pause must carry a reason");
            assert!(r.contains("HIGH") || r.contains("homograph"), "reason: {r}");
            assert!(matches!(
                *resume_kind,
                ironclaw_engine::ResumeKind::Approval {
                    allow_always: false
                }
            ));
        }
        other => panic!("expected GatePaused, got {other:?}"),
    }
}

#[tokio::test]
async fn fail_closed_missing_binary_denies_in_v2() {
    let cfg = TirithConfig {
        enabled: true,
        bin: "/nonexistent/tirith-xyz-abc".into(),
        timeout: Duration::from_secs(5),
        fail_open: false,
    };
    let adapter = make_adapter(Some(cfg)).await;
    let ctx = make_context(ProjectId::new());
    let lease = make_lease(ctx.thread_id, "shell");

    let err = adapter
        .execute_action("shell", shell_params("ls"), &lease, &ctx)
        .await
        .expect_err("fail-closed missing binary must error");

    assert!(
        matches!(err, ironclaw_engine::EngineError::LeaseDenied { .. }),
        "fail-closed must deny via LeaseDenied — clicking through GatePaused defeats fail-closed; got {err:?}"
    );
}

#[tokio::test]
async fn tirith_config_disabled_does_not_invoke_subprocess() {
    // Bin would Block if invoked. With cfg.enabled = false the subprocess
    // is never spawned and the existing v2 permission logic decides — for
    // a `requires_approval = Never` tool with default `AskEachTime`
    // permission this surfaces as a standard approval gate (not a tirith
    // pause), which is what we want to assert.
    let (_tmp, bin) = make_fake_tirith(1, r#"{"findings":[{"severity":"HIGH","title":"x"}]}"#);
    let mut cfg = cfg_with(bin, true);
    cfg.enabled = false;
    let adapter = make_adapter(Some(cfg)).await;
    let ctx = make_context(ProjectId::new());
    let lease = make_lease(ctx.thread_id, "shell");

    // For a Never-approval tool, default permission resolution lands on
    // AskEachTime and (without any auto-approve) gate-pauses with the
    // GENERIC reason — not tirith's. The contract: `reason.is_none()`.
    let result = adapter
        .execute_action("shell", shell_params("ls"), &lease, &ctx)
        .await;

    if let Err(ironclaw_engine::EngineError::GatePaused { reason, .. }) = &result {
        assert!(
            reason.is_none(),
            "tirith disabled must not produce a tirith reason; got {reason:?}"
        );
    }
    // Either Ok or generic GatePaused is acceptable — both prove tirith
    // never produced its rich reason.
}

#[tokio::test]
async fn non_shell_tool_does_not_spawn_tirith() {
    // Bin path points at a directory that would error if anyone tried to
    // exec it. The helper short-circuits on `tool_name != "shell"` BEFORE
    // resolving the binary, so this should still succeed (or land in
    // standard permission logic, not tirith).
    let cfg = TirithConfig {
        enabled: true,
        bin: "/this/path/would/error/if/spawned".into(),
        timeout: Duration::from_secs(5),
        fail_open: false, // makes a tirith spawn fail loudly (LeaseDenied)
    };
    let adapter = make_adapter(Some(cfg)).await;
    let ctx = make_context(ProjectId::new());
    let lease = make_lease(ctx.thread_id, "http");

    let result = adapter
        .execute_action("http", serde_json::json!({"url": "x"}), &lease, &ctx)
        .await;

    // Specifically must NOT be `LeaseDenied { reason: contains "Tirith unavailable" }`.
    if let Err(ironclaw_engine::EngineError::LeaseDenied { reason }) = &result {
        assert!(
            !reason.to_lowercase().contains("tirith"),
            "non-shell tool must not invoke tirith; got reason: {reason}"
        );
    }
}

#[tokio::test]
async fn no_tirith_config_runs_normal_permission_logic() {
    // Registry built without `with_tirith_config(...)`. The preflight
    // helper short-circuits on `self.tools.tirith_config()` returning
    // None and never spawns.
    let adapter = make_adapter(None).await;
    let ctx = make_context(ProjectId::new());
    let lease = make_lease(ctx.thread_id, "shell");

    let result = adapter
        .execute_action("shell", shell_params("ls"), &lease, &ctx)
        .await;

    if let Err(ironclaw_engine::EngineError::GatePaused { reason, .. }) = &result {
        assert!(
            reason.is_none(),
            "no tirith config must not produce a tirith reason; got {reason:?}"
        );
    }
}

#[tokio::test]
async fn allow_lets_call_proceed_through_v2() {
    // Tirith exit 0 => Allow. Combined with `requires_approval = Never`
    // and `AskEachTime` permission default, the only way the call still
    // gate-pauses is the standard ask-each-time gate. To isolate the
    // tirith-Allow contract we use `with_global_auto_approve(true)` so
    // the post-tirith standard gate clears too — leaving an actual `Ok`
    // ActionResult that proves both tirith ran AND let the call proceed.
    let (_tmp, bin) = make_fake_tirith(0, "{}");
    let registry = ToolRegistry::new().with_tirith_config(cfg_with(bin, true));
    registry.register(Arc::new(MockShell)).await;
    let adapter = EffectBridgeAdapter::new(
        Arc::new(registry),
        Arc::new(SafetyLayer::new(&SafetyConfig {
            max_output_length: 100_000,
            injection_check_enabled: false,
        })),
        Arc::new(HookRegistry::default()),
    )
    .with_global_auto_approve(true);
    let ctx = make_context(ProjectId::new());
    let lease = make_lease(ctx.thread_id, "shell");

    let result = adapter
        .execute_action("shell", shell_params("ls"), &lease, &ctx)
        .await
        .expect("tirith-allow + global auto-approve should let the call proceed");
    assert!(!result.is_error, "expected Ok result, got: {:?}", result);
}

#[tokio::test]
async fn pause_persists_canonical_name_not_alias() {
    // Regression test: when the LLM calls a discovery alias that resolves
    // to the canonical `shell` tool, the resulting `GatePaused.action_name`
    // must be the canonical (`shell`), NOT the alias (`execute_shell_cmd`).
    // The router resumes by passing `pending.action_name` back into the
    // adapter; if we persisted the alias, resume would re-resolve and could
    // land on a different tool than the one tirith just scanned.
    let (_tmp, bin) = make_fake_tirith(
        1,
        r#"{"findings":[{"rule_id":"r1","severity":"HIGH","title":"homograph"}]}"#,
    );
    let adapter = make_adapter(Some(cfg_with(bin, true))).await;

    // ActionDef whose canonical `name` is "shell" but whose discovery
    // alias is "execute_shell_cmd". `ActionDiscovery::resolve` matches
    // either, then returns the canonical via `.name`.
    let alias_action = ActionDef {
        name: "shell".into(),
        description: "shell with discovery alias".into(),
        parameters_schema: serde_json::json!({"type": "object"}),
        effects: vec![EffectType::WriteExternal],
        requires_approval: false,
        model_tool_surface: ModelToolSurface::FullSchema,
        discovery: Some(ActionDiscoveryMetadata {
            name: "execute_shell_cmd".into(),
            summary: None,
            schema_override: None,
        }),
    };
    let snapshot: Arc<[ActionDef]> = Arc::from(vec![alias_action]);
    let mut ctx = make_context(ProjectId::new());
    ctx.available_actions_snapshot = Some(snapshot);
    let lease = make_lease(ctx.thread_id, "shell");

    let err = adapter
        .execute_action(
            "execute_shell_cmd", // alias name the LLM used
            shell_params("ls"),
            &lease,
            &ctx,
        )
        .await
        .expect_err("tirith block must produce a GatePaused even via alias");

    match err {
        ironclaw_engine::EngineError::GatePaused {
            action_name,
            reason,
            ..
        } => {
            assert_eq!(
                action_name, "shell",
                "pause must persist the canonical resolved tool name, not the alias the LLM used"
            );
            assert!(
                reason.is_some(),
                "tirith pause must still carry the rich finding reason"
            );
        }
        other => panic!("expected GatePaused, got {other:?}"),
    }
}

#[tokio::test]
async fn approval_already_granted_skips_tirith_rescan() {
    // Tirith exit 1 (Block). Without `approval_already_granted=true` this
    // call would gate-pause with a tirith reason. With the flag set —
    // i.e. the user already saw + approved this exact pending call,
    // including any tirith reason in the prior approval prompt — tirith
    // must NOT re-scan, and the call must proceed (pinned via the
    // resume-path API `execute_resolved_pending_action`).
    let (_tmp, bin) = make_fake_tirith(
        1,
        r#"{"findings":[{"rule_id":"r1","severity":"HIGH","title":"would-block"}]}"#,
    );
    let adapter = make_adapter(Some(cfg_with(bin, true))).await;
    let ctx = make_context(ProjectId::new());
    let lease = make_lease(ctx.thread_id, "shell");

    let result = adapter
        .execute_resolved_pending_action(
            "shell",
            shell_params("ls"),
            &lease,
            &ctx,
            /*approval_already_granted=*/ true,
        )
        .await
        .expect("approval_already_granted must skip tirith re-scan and proceed");
    assert!(!result.is_error, "expected Ok result, got: {:?}", result);
}
