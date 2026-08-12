//! C-HOOKS (+ E-HOOK-INFRA): a wired `hook_dispatcher_builder_factory` should
//! fire hooks at the expected lifecycle points on a real coordinator-path turn,
//! a hook deny should block the capability without wedging the run, and
//! dispatcher-owned state must not leak from one run into the next (#6945).
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

use ironclaw_event_log::{SecurityBoundary, SecurityDecision};
use ironclaw_hooks::dispatch::HOOK_DENY_PREDICATE_CODE;
use reborn_support::assertions::ToolErrorClass;
use reborn_support::builder::{RebornIntegrationHarness, StorageMode};
use reborn_support::hooks::{
    HOOK_TEST_DENY_REASON, RecordingHookLog, denying_hook_factory, poisoning_hook_factory,
    recording_hook_factory,
};
use reborn_support::reply::RebornScriptedReply;
use serde_json::json;

const HTTP_TOOL_URL: &str = "https://api.example.test/v1/items";

#[derive(Debug, Default)]
struct RecordingTrajectoryObserver {
    inputs: Mutex<Vec<(String, String, serde_json::Value)>>,
    results: Mutex<Vec<(String, String, serde_json::Value)>>,
}

impl ironclaw_composition::RebornTrajectoryObserver for RecordingTrajectoryObserver {
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
    // #6284 item 4: the denial must also tell the model what to DO. A hook deny
    // is a genuine policy refusal, so the honest advice is to change the plan
    // rather than the arguments. Denials carried `model_observation: None`
    // before #6792, so nothing actionable was persisted at all.
    h.assert_denial_recovery_hint("revise_approach")
        .await
        .expect("a persisted denial must carry a recovery hint the model can act on");
    h.assert_security_audit_event_recorded(
        SecurityBoundary::HookDeny,
        SecurityDecision::Blocked,
        HOOK_DENY_PREDICATE_CODE,
    )
    .await
    .expect("hook deny must record a security-audit event through the harness recorder");
}

/// #6945: dispatcher-owned state must not survive from one run into the next.
///
/// `RebornLoopDriverHostFactory` offers three hook seams with two deliberately
/// different lifetimes. Production wires the isolating one
/// (`with_hook_dispatcher_builder_factory`, minted by
/// `ironclaw_composition::hooks` and installed at
/// `ironclaw_turn_runner::runtime`), so the closure runs once per
/// `build_text_only_host*` — i.e. once per run. The legacy
/// `with_hook_dispatcher(Arc<HookDispatcher>)` adapter deliberately does the
/// opposite and clones one dispatcher into every build. Nothing failed if a
/// caller swapped one for the other, and `crates/loop/ironclaw_hooks/CLAUDE.md` once
/// claimed a regression test — naming a file and two tests that never existed
/// — which is the gap #6945 tracks.
///
/// The observable is slot poisoning. The installed hook commits a gate-sink
/// protocol violation, so run 1 fails closed (the capability is denied and
/// never reaches the wire) **and** the hook's slot is poisoned. A poisoned slot
/// is skipped for the rest of that dispatcher's life. So:
///
/// - per-run dispatcher (production): run 2 gets a clean slot, the hook fires a
///   second time, and the fail-closed deny is re-applied — 2 fires, 0 egress.
/// - shared dispatcher (legacy adapter): run 2 skips the poisoned hook entirely,
///   so the gate goes quiet and the capability reaches the wire — 1 fire, 1
///   egress. Both assertions below flip, which is what makes this red-able.
///
/// Deliberately NOT asserted: predicate counter state. It is keyed by
/// `(hook_id, tenant_id, capability)` and shared across runs *by design* (the
/// evaluator is built once per tenant by composition, outside the per-run
/// closure), so asserting isolation for it would pin a rate-cap bypass.
#[tokio::test]
async fn poisoned_hook_slot_does_not_leak_into_the_next_run() {
    let log = RecordingHookLog::new();
    let h = RebornIntegrationHarness::test_default()
        .with_builtin_http_tools()
        .with_hook_factory(poisoning_hook_factory(log.clone(), "builtin.http"))
        // One entry per model call: each turn makes a tool call and then a
        // terminal text reply, so two turns need four.
        .script([
            RebornScriptedReply::tool_call("builtin.http", json!({"url": HTTP_TOOL_URL})),
            RebornScriptedReply::text("first done"),
            RebornScriptedReply::tool_call("builtin.http", json!({"url": HTTP_TOOL_URL})),
            RebornScriptedReply::text("second done"),
        ])
        .build()
        .await
        .expect("harness builds");

    h.submit_turn("fetch items")
        .await
        .expect("turn 1 completes");
    assert_eq!(
        log.fires(),
        vec!["before_capability_poison:builtin.http"],
        "run 1 must dispatch the hook exactly once before it poisons its slot"
    );
    h.assert_egress_count(0)
        .await
        .expect("run 1's fail-closed deny must keep the capability off the wire");

    h.submit_turn("fetch items again")
        .await
        .expect("turn 2 completes");

    // The load-bearing assertion. Under the legacy shared-dispatcher adapter the
    // run-1 poison survives into run 2, the hook is skipped, and this stays at
    // one fire.
    assert_eq!(
        log.fires(),
        vec![
            "before_capability_poison:builtin.http",
            "before_capability_poison:builtin.http",
        ],
        "run 2 must get a fresh dispatcher with an un-poisoned slot, so the hook \
         fires again; a shared dispatcher would skip it and record only one fire"
    );
    // …and the consequence that makes the leak a security problem rather than a
    // telemetry one: a skipped gate hook is an un-applied deny, so the
    // capability would reach real egress in run 2.
    h.assert_egress_count(0)
        .await
        .expect("run 2 must re-apply the fail-closed deny from a clean slot");
    h.assert_tool_error(
        ToolErrorClass::Denied,
        "hook completed without minting a decision",
    )
    .await
    .expect("run 2's denial must carry the fail-closed reason, not a stale/blank one");
}
