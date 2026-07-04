//! C-HOOKS (+ E-HOOK-INFRA): a wired `hook_dispatcher_builder_factory` should
//! fire hooks at the expected lifecycle points on a real coordinator-path turn,
//! and a hook deny should block the capability without wedging the run.
//!
//! # BLOCKED by a production bug (both scenarios are `#[ignore]`d RED regressions)
//!
//! Wiring ANY hook dispatcher into a full coordinator-path turn fails EVERY
//! turn with `driver_unavailable`:
//! `HostUnavailableWithDiagnostics { stage: Checkpoint, kind: Unavailable,
//! safe_summary: "stage_checkpoint_payload not implemented" }`.
//!
//! Root cause: `ironclaw_hooks::middleware::checkpoint_port::
//! HookedLoopCheckpointPort` overrides only `LoopCheckpointPort::checkpoint`;
//! it does NOT forward `stage_checkpoint_payload`/`load_checkpoint_payload` to
//! its inner port, so those fall through to the trait's fail-closed defaults.
//! A planned run stages a checkpoint payload before the first model call, so
//! with a hook dispatcher active the turn dies there before any hook fires.
//! Hooks are off by default in production, so this latent bug has never been
//! exercised — this is the first full-turn-with-hooks path.
//!
//! TODO(reborn-hooks-checkpoint-forward): forward both methods through
//! `HookedLoopCheckpointPort` to `self.inner` (mirroring `checkpoint()`), then
//! remove the `#[ignore]`s below. Cross-crate fix in `ironclaw_hooks`, out of
//! scope for this tests-only lane.
//!
//! The E-HOOK-INFRA enabler (recording hook doubles, the
//! `hook_dispatcher_builder_factory` group-builder seam, `with_hook_factory`)
//! DOES land and is correct — these scenarios go green once the wrapper bug is fixed.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use reborn_support::assertions::ToolErrorClass;
use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::hooks::{
    HOOK_TEST_DENY_REASON, RecordingHookLog, denying_hook_factory, recording_hook_factory,
};
use reborn_support::reply::RebornScriptedReply;
use serde_json::json;

const HTTP_TOOL_URL: &str = "https://api.example.test/v1/items";

/// The AfterModel observer fires on the model call and the BeforeCapability gate
/// hook fires before the dispatched capability — both recorded through the real
/// turn wire. The passing gate hook does not block the capability, so the http
/// tool still runs.
///
/// `#[ignore]`d RED regression — currently fails at the checkpoint stage (see the
/// module-level bug note). Un-ignore once `HookedLoopCheckpointPort` forwards
/// `stage_checkpoint_payload`.
#[ignore = "blocked: HookedLoopCheckpointPort omits stage_checkpoint_payload forwarding; \
            hooked coordinator turn dies driver_unavailable at checkpoint_before_model \
            (TODO reborn-hooks-checkpoint-forward)"]
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
    // Both lifecycle points fired in order: AfterModel dispatches once per
    // `finalize_assistant_message` call, and the script's two assistant turns
    // sandwich the BeforeCapability gate hook right before dispatch.
    assert_eq!(
        log.fires(),
        vec![
            "observer:AfterModel",
            "before_capability:builtin.http",
            "observer:AfterModel",
        ],
        "hook fires must occur in lifecycle order: AfterModel (tool-call reply) -> \
         BeforeCapability (builtin.http dispatch) -> AfterModel (final text reply)"
    );
}

/// A BeforeCapability hook deny should block the capability (it never reaches the
/// wire) yet the run should still complete — the hook error path must NOT wedge
/// the run.
///
/// `#[ignore]`d RED regression — same checkpoint bug as above blocks it.
#[ignore = "blocked: HookedLoopCheckpointPort omits stage_checkpoint_payload forwarding; \
            hooked coordinator turn dies driver_unavailable at checkpoint_before_model \
            (TODO reborn-hooks-checkpoint-forward)"]
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
}
