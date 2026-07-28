//! C-HOOKS (+ E-HOOK-INFRA): a wired `hook_dispatcher_builder_factory` should
//! fire hooks at the expected lifecycle points on a real coordinator-path turn,
//! and a hook deny should block the capability without wedging the run.
//!
//! These drive a full coordinator-path turn with an active hook dispatcher —
//! the first tests to do so — so they also pin that `HookedLoopCheckpointPort`
//! stays transparent for `stage_checkpoint_payload`/`load_checkpoint_payload`,
//! not just `checkpoint`. A planned run stages a checkpoint payload before
//! every model call, so any gap there fails every hooks-enabled turn.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use std::sync::{Arc, Mutex};

use ironclaw_events::{SecurityBoundary, SecurityDecision};
use ironclaw_hooks::dispatch::HOOK_DENY_PREDICATE_CODE;
use reborn_support::assertions::ToolErrorClass;
use reborn_support::builder::{RebornIntegrationHarness, StorageMode};
use reborn_support::hooks::{
    HOOK_TEST_DENY_REASON, RecordingHookLog, denying_hook_factory, recording_hook_factory,
};
use reborn_support::reply::RebornScriptedReply;
use serde_json::json;

const HTTP_TOOL_URL: &str = "https://api.example.test/v1/items";

#[derive(Debug, Default)]
struct RecordingTrajectoryObserver {
    inputs: Mutex<Vec<(String, String, serde_json::Value)>>,
    results: Mutex<Vec<(String, String, serde_json::Value)>>,
}

impl ironclaw_reborn_composition::RebornTrajectoryObserver for RecordingTrajectoryObserver {
    fn on_capability_input(
        &self,
        call_id: &str,
        capability_id: &str,
        arguments: &serde_json::Value,
    ) {
        self.inputs.lock().expect("inputs lock").push((
            call_id.to_string(),
            capability_id.to_string(),
            arguments.clone(),
        ));
    }

    fn on_capability_result(&self, call_id: &str, capability_id: &str, output: &serde_json::Value) {
        self.results.lock().expect("results lock").push((
            call_id.to_string(),
            capability_id.to_string(),
            output.clone(),
        ));
    }
}

/// The BeforeCapability gate hook fires before the dispatched capability, and
/// the AfterModel observer fires once for the turn — both recorded through
/// the real turn wire. The passing gate hook does not block the capability,
/// so the http tool still runs.
#[tokio::test]
async fn hooks_fire_at_lifecycle_points_on_coordinator_turn() {
    let log = RecordingHookLog::new();
    let h = RebornIntegrationHarness::test_default()
        .with_builtin_http_tools()
        .with_hook_factory(recording_hook_factory(log.clone()))
        .script([
            RebornScriptedReply::tool_call("builtin.http", json!({"url": HTTP_TOOL_URL})),
            RebornScriptedReply::text("done"),
        ])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("fetch items").await.expect("turn completes");

    h.assert_tool_invoked("builtin.http")
        .await
        .expect("http tool ran through the real capability path");
    // AfterModel fires only once per turn, at `finalize_assistant_message`
    // for the terminal text reply — the tool-call reply that precedes it
    // finalizes through the capability path, not the transcript port, so it
    // does not fire AfterModel on its own.
    assert_eq!(
        log.fires(),
        vec!["before_capability:builtin.http", "observer:AfterModel",],
        "hook fires must occur in lifecycle order: BeforeCapability (builtin.http dispatch) \
         -> AfterModel (final text reply)"
    );
}

#[tokio::test]
async fn production_observer_and_hooks_wire_through_libsql_harness() {
    let hooks = RecordingHookLog::new();
    let trajectory = Arc::new(RecordingTrajectoryObserver::default());
    let h = RebornIntegrationHarness::test_default()
        .storage(StorageMode::LibSql)
        .with_durable_capability_io_builtin_http_tools()
        .with_hook_factory(recording_hook_factory(hooks.clone()))
        .with_raw_trajectory_observer(trajectory.clone())
        .script([
            RebornScriptedReply::tool_call("builtin.http", json!({"url": HTTP_TOOL_URL})),
            RebornScriptedReply::text("done"),
        ])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("fetch items").await.expect("turn completes");
    h.assert_reply_contains("done")
        .await
        .expect("reply finalized");
    h.assert_reply_persists_after_reopen("done")
        .await
        .expect("libsql-backed reply survives an independent reopen");

    assert_eq!(
        hooks.fires(),
        vec!["before_capability:builtin.http", "observer:AfterModel"],
        "hook factory must fire through the real coordinator-path turn"
    );

    let inputs = trajectory.inputs.lock().expect("trajectory inputs");
    assert_eq!(inputs.len(), 1, "one capability input should be observed");
    let (input_call_id, input_capability, input_args) = &inputs[0];
    assert!(!input_call_id.is_empty(), "input call_id should be present");
    assert_eq!(input_capability, "builtin.http");
    assert_eq!(input_args["url"], HTTP_TOOL_URL);

    let results = trajectory.results.lock().expect("trajectory results");
    assert_eq!(results.len(), 1, "one capability result should be observed");
    let (result_call_id, result_capability, result_output) = &results[0];
    assert_eq!(
        result_call_id, input_call_id,
        "input/result trajectory events must correlate by call_id"
    );
    assert_eq!(result_capability, "builtin.http");
    assert!(
        result_output.to_string().contains("accepted"),
        "trajectory result should contain the scripted HTTP response, got {result_output}"
    );
}

/// A BeforeCapability hook deny should block the capability (it never reaches the
/// wire) yet the run should still complete — the hook error path must NOT wedge
/// the run.
#[tokio::test]
async fn hook_deny_blocks_capability_without_wedging_run() {
    let log = RecordingHookLog::new();
    let h = RebornIntegrationHarness::test_default()
        .with_builtin_http_tools()
        .with_hook_factory(denying_hook_factory(log.clone(), "builtin.http"))
        .script([
            RebornScriptedReply::tool_call("builtin.http", json!({"url": HTTP_TOOL_URL})),
            RebornScriptedReply::text("done"),
        ])
        .build()
        .await
        .expect("harness builds");
    // `submit_turn` waits for `Completed`: reaching it proves the deny did not
    // wedge the run (a wedged/failed run would fail this wait).
    h.submit_turn("fetch items")
        .await
        .expect("turn completes despite the hook deny");

    assert!(
        log.fired("before_capability_deny:builtin.http"),
        "deny hook must fire for builtin.http; saw {:?}",
        log.fires()
    );
    // The denied capability never reached the HTTP wire (blocked before the
    // inner runtime port), so no egress was captured.
    h.assert_egress_count(0)
        .await
        .expect("a hook-denied capability must not reach egress");
    // The model-visible tool-result envelope reports the hook's deny reason,
    // not a generic/blank denial — pins that the deny reason token actually
    // propagates to the persisted `ToolResultReference` the model sees.
    h.assert_tool_error(ToolErrorClass::Denied, HOOK_TEST_DENY_REASON)
        .await
        .expect("hook deny reason must be reported in the persisted tool-error summary");
    h.assert_security_audit_event_recorded(
        SecurityBoundary::HookDeny,
        SecurityDecision::Blocked,
        HOOK_DENY_PREDICATE_CODE,
    )
    .await
    .expect("hook deny must record a security-audit event through the harness recorder");
}
