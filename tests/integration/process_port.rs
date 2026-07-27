//! Reborn integration-test framework — slice 5 inert process port.
//!
//! Proves the `builtin.shell` dispatch path with the inert `RecordingProcessPort`
//! default: the command is recorded (real dispatch path ran) and no real OS
//! process was spawned (the safety invariant). No network, services, keys,
//! Docker, or `integration` feature.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use reborn_support::assertions::ToolErrorClass;
use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::reply::RebornScriptedReply;
use serde_json::json;

/// Default build: a scripted `builtin.shell` call is recorded by the inert
/// `RecordingProcessPort` and no real OS process is spawned (slice 5 safety
/// invariant).
#[tokio::test]
async fn shell_call_recorded_not_executed() {
    let h = RebornIntegrationHarness::test_default()
        .with_builtin_http_tools()
        .script([
            RebornScriptedReply::tool_call("builtin.shell", json!({"command": "echo s5-probe"})),
            RebornScriptedReply::text("done"),
        ])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("run shell").await.expect("turn completes");
    h.assert_shell_command_recorded("s5-probe")
        .await
        .expect("command recorded by inert port");
    h.assert_shell_ran_through_inert_port()
        .await
        .expect("inert port ran, no real process spawned");
    h.assert_reply_contains("done")
        .await
        .expect("final reply finalized");
}

/// Guards the assertion helpers: on a plain text turn (no shell call) both shell
/// assertions must return `Err` — proving they don't pass vacuously on an empty
/// command list.
#[tokio::test]
async fn shell_assertions_fail_when_no_shell_call_ran() {
    let h = RebornIntegrationHarness::test_default()
        .script([RebornScriptedReply::text("no shell")])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("just talk").await.expect("turn completes");
    assert!(h.assert_shell_command_recorded("echo").await.is_err());
    assert!(h.assert_shell_ran_through_inert_port().await.is_err());
}

/// Error path — non-zero exit. A scripted `exit_code = 1` is NOT a tool error:
/// `builtin.shell` surfaces it as a *Completed* result carrying `"exit_code":1`
/// / `"success":false`, so the run completes and the model can react.
#[tokio::test]
async fn shell_non_zero_exit_surfaces_as_completed_result() {
    let h = RebornIntegrationHarness::test_default()
        .with_shell_exit_code(1)
        .script([
            RebornScriptedReply::tool_call("builtin.shell", json!({"command": "echo boom"})),
            RebornScriptedReply::text("done"),
        ])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("run shell").await.expect("turn completes");
    h.assert_tool_result_contains("\"exit_code\":1")
        .await
        .expect("non-zero exit surfaced in the model-visible tool result");
    h.assert_tool_result_contains("\"success\":false")
        .await
        .expect("success flag reflects the non-zero exit");
    h.assert_shell_command_recorded("echo boom")
        .await
        .expect("command dispatched through the inert port");
    h.assert_reply_contains("done")
        .await
        .expect("run recovered and finalized");
}

/// Error path — command timeout. A scripted `RuntimeProcessError::Timeout` maps
/// to a recoverable, model-visible `Failed{Resource}` capability error, so the
/// run continues to completion rather than dying with `driver_unavailable`.
#[tokio::test]
async fn shell_timeout_surfaces_recoverable_failed() {
    let h = RebornIntegrationHarness::test_default()
        .with_shell_timeout()
        .script([
            RebornScriptedReply::tool_call("builtin.shell", json!({"command": "sleep 999"})),
            RebornScriptedReply::text("done"),
        ])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("run shell").await.expect("turn completes");
    h.assert_tool_error(ToolErrorClass::Failed, "resource")
        .await
        .expect("timeout surfaced as a model-visible Failed{Resource} tool error");
    h.assert_reply_contains("done")
        .await
        .expect("run recovered and finalized (not terminal driver_unavailable)");
}

/// Live-shell path (`.with_live_shell()`) — proves `builtin.shell` dispatches
/// through the real `HostProcessPort`, not the inert `RecordingProcessPort`
/// the other tests in this file cover. Asserts both directions: (a) the real
/// command's echoed output is visible in the model-facing tool result — only
/// possible if an actual OS process ran — and (b) `assert_shell_ran_through_inert_port`
/// (which passes only when the inert port recorded a command) reports `Err`,
/// proving the recording port's command buffer stayed empty. Guards against a
/// regression that routes live-shell requests back through
/// `core_builtin_tools_default()` (the inert path) while every other test in
/// this file — which never exercises `.with_live_shell()` — would stay green.
///
/// Runs on a larger-stack thread (mirrors
/// `tests/reborn_qa_smoke_scenarios_e2e.rs::run_async_test_with_stack`): the
/// real `HostProcessPort` subprocess path (spawn + piped-stream capture)
/// adds enough async-state-machine depth on top of the full
/// `product_surface → composition → webui_v2 → runtime` chain to overflow
/// the default `#[tokio::test]` thread stack in a debug build.
#[test]
fn live_shell_uses_local_process_port() {
    run_with_larger_stack(async {
        let h = RebornIntegrationHarness::test_default()
            .with_live_shell()
            .script([
                RebornScriptedReply::tool_call(
                    "builtin.shell",
                    json!({"command": "echo live-shell-probe"}),
                ),
                RebornScriptedReply::text("done"),
            ])
            .build()
            .await
            .expect("harness builds");
        h.submit_turn("run shell").await.expect("turn completes");
        h.assert_tool_result_contains("live-shell-probe")
            .await
            .expect("real process output surfaced in the model-visible tool result");
        assert!(
            h.assert_shell_ran_through_inert_port().await.is_err(),
            "live shell must not route through the inert RecordingProcessPort"
        );
        h.assert_reply_contains("done")
            .await
            .expect("final reply finalized");
    });
}

/// Spawns `test` on a dedicated 16MB-stack thread with a current-thread tokio
/// runtime. See the doc comment on `live_shell_uses_local_process_port` for
/// why this one test needs it (matches the existing fix in
/// `tests/reborn_qa_smoke_scenarios_e2e.rs::run_async_test_with_stack`).
fn run_with_larger_stack<F>(test: F)
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    let handle = std::thread::Builder::new()
        .name("live_shell_uses_local_process_port".to_string())
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio test runtime")
                .block_on(test);
        })
        .expect("spawn stack-sized test thread");
    if let Err(panic) = handle.join() {
        std::panic::resume_unwind(panic);
    }
}
