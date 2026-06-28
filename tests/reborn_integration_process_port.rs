//! Reborn integration-test framework — slice 5 inert process port.
//!
//! Proves the `builtin.shell` dispatch path with the inert `RecordingProcessPort`
//! default: the command is recorded (real dispatch path ran) and no real OS
//! process was spawned (the safety invariant). No network, services, keys,
//! Docker, or `integration` feature.

#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
mod support;

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

// `.with_live_shell()` test omitted: a live `echo` is hermetic but offers no
// assertion the recording-port test above doesn't already cover — the scripted
// model reply is fixed regardless of actual shell output, so there is nothing
// to assert on the real execution result beyond "the tool was invoked", which
// the recording test already proves end-to-end through the real dispatch path.
