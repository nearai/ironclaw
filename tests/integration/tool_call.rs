//! Tool-calling turn: proves the §3.7 two-tier egress design end-to-end —
//! scripted `builtin.http` call → real `RuntimeHttpEgress` → recording egress
//! (Tier-2) → finalized reply. Same scripted `TraceLlm` seam as other harness tests.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::reply::RebornScriptedReply;
use serde_json::json;

#[tokio::test]
async fn runs_http_tool_call_through_recorded_egress() {
    let h = RebornIntegrationHarness::test_default()
        .with_builtin_http_tools()
        .script([
            RebornScriptedReply::tool_call("builtin.http", json!({"url": HTTP_TOOL_URL})),
            RebornScriptedReply::text("fetched"),
        ])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("fetch items").await.expect("turn completes");
    h.assert_tool_invoked("builtin.http")
        .await
        .expect("http tool ran");
    h.assert_egress_request_matching("api.example.test")
        .await
        .expect("Tier-2 egress captured");
    h.assert_reply_contains("fetched")
        .await
        .expect("final reply finalized");
}

const HTTP_TOOL_URL: &str = "https://api.example.test/v1/items";

/// Guards against vacuous pass: with no scripted tool call, both
/// `assert_tool_invoked` and `assert_egress_request_matching` must return `Err`.
#[tokio::test]
async fn assertions_fail_when_tool_did_not_run() {
    let h = RebornIntegrationHarness::test_default()
        .script([RebornScriptedReply::text("no tool")])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("just talk").await.expect("turn completes");
    assert!(h.assert_tool_invoked("builtin.http").await.is_err());
    assert!(
        h.assert_egress_request_matching("api.example.test")
            .await
            .is_err()
    );
}

/// Proves the assertions discriminate when the invocation + egress lists are
/// NON-empty: a real `builtin.http` call runs, but assertions for a *different*
/// capability/host must still return `Err` (the "present but no match" branch).
#[tokio::test]
async fn assertions_fail_when_tool_present_but_requested_tool_or_url_does_not_match() {
    let h = RebornIntegrationHarness::test_default()
        .with_builtin_http_tools()
        .script([
            RebornScriptedReply::tool_call("builtin.http", json!({"url": HTTP_TOOL_URL})),
            RebornScriptedReply::text("done"),
        ])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("fetch items").await.expect("turn completes");
    // Prove capture lists are NON-empty first, so the checks below exercise the
    // mismatch branch, not the empty-list branch.
    h.assert_tool_invoked("builtin.http")
        .await
        .expect("http tool ran before mismatch assertions");
    h.assert_egress_request_matching("api.example.test")
        .await
        .expect("http egress captured before mismatch assertions");
    // Non-empty invocation list — wrong capability id must fail.
    assert!(
        h.assert_tool_invoked("some.other.capability")
            .await
            .is_err()
    );
    // Non-empty egress list — non-matching host substring must fail.
    assert!(
        h.assert_egress_request_matching("nonmatching.host.test")
            .await
            .is_err()
    );
}

/// Proves the multi-segment `builtin.http.save` capability id (`.`→`__`
/// encoding to `builtin__http__save` at the provider seam) resolves end-to-end,
/// writing to the `/workspace` mount `core_builtin_tools` provides read-write.
#[tokio::test]
async fn runs_http_save_tool_call_through_recorded_egress() {
    let h = RebornIntegrationHarness::test_default()
        .with_builtin_http_tools()
        .script([
            RebornScriptedReply::tool_call(
                "builtin.http.save",
                json!({"url": HTTP_TOOL_URL, "save_to": "/workspace/response.json"}),
            ),
            RebornScriptedReply::text("saved"),
        ])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("fetch and save")
        .await
        .expect("turn completes");
    h.assert_tool_invoked("builtin.http.save")
        .await
        .expect("http.save tool ran");
    // The save path must reach the real `RuntimeHttpEgress`.
    h.assert_egress_request_matching("api.example.test")
        .await
        .expect("http.save egress captured");
    h.assert_reply_contains("saved")
        .await
        .expect("final reply finalized");
}

/// The globally-disabled `builtin.spawn_subagent` capability (configured
/// through `DefaultPlannedRuntimeConfig::disabled_capability_ids`, applied as
/// the OUTERMOST `PerSurfaceCapabilityDenyDecorator` in
/// `build_default_planned_runtime_inner` — see that function's doc comments)
/// must never reach the model-facing tool list, whichever port would
/// otherwise have surfaced it: the flavor-aware `SubagentSpawnCapabilityDecorator`
/// (always wired, independent of any harness extension registry) or the
/// host-runtime first-party manifest stub (`builtin_first_party_package()` in
/// `crates/ironclaw_host_runtime/src/first_party_tools/mod.rs`, included in
/// `core_builtin_tools()`'s registry unconditionally).
///
/// Non-vacuity: confirmed by direct inspection that `core_builtin_tools()`'s
/// capability port surfaces `builtin__spawn_subagent` when the deny decorator
/// is bypassed (i.e. `spawn_decorator` runs before the outermost deny filter
/// in composition order) — so this assertion is pinning a real strip, not
/// asserting absence from an already-empty surface. `builtin__http` is
/// asserted present as the non-vacuity control for THIS test's own capture.
#[tokio::test]
async fn disabled_spawn_subagent_capability_is_stripped_from_model_surface() {
    let h = RebornIntegrationHarness::test_default()
        .with_builtin_http_tools()
        .script([RebornScriptedReply::text("done")])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("hello").await.expect("turn completes");

    let captured = h.scripted_llm.captured_tool_definitions();
    let names: Vec<&str> = captured
        .iter()
        .flatten()
        .map(|def| def.name.as_str())
        .collect();

    // Neither encoding of the disabled capability id may appear in what the
    // model was shown (provider-seam `__` encoding, or the raw dotted id).
    assert!(
        !names.contains(&"builtin__spawn_subagent"),
        "disabled capability's provider seam name must not be advertised: {names:?}"
    );
    assert!(
        !names.contains(&"builtin.spawn_subagent"),
        "disabled capability's raw dotted id must not be advertised: {names:?}"
    );
    // Control: a real capability IS present, so the absence asserts above are
    // not vacuously true against an empty surface.
    assert!(
        names.contains(&"builtin__http"),
        "control tool builtin__http must be present: {names:?}"
    );
}

/// A model that calls the disabled `builtin.spawn_subagent` anyway is rejected
/// at the gateway (`CapabilitySurfaceDenyFilter`, before
/// `register_provider_tool_call` ever stages an invocation) — the whole
/// provider response fails with `InvalidOutput` → `Unavailable`, reaching a
/// terminal `TurnStatus::Failed`/`"model_error"` after exactly one scripted
/// turn. No `ToolResultReference` is persisted; `assert_tool_invoked`
/// returning `Err` proves the capability was never dispatched.
#[tokio::test]
async fn disabled_spawn_subagent_capability_call_anyway_fails_the_run() {
    let h = RebornIntegrationHarness::test_default()
        .with_builtin_http_tools()
        .script([RebornScriptedReply::tool_call(
            "builtin.spawn_subagent",
            json!({"goal": "test"}),
        )])
        .build()
        .await
        .expect("harness builds");

    let run_id = h
        .submit_turn_async("spawn a subagent")
        .await
        .expect("turn submitted");
    let state = h
        .wait_for_status(run_id, ironclaw_turns::TurnStatus::Failed)
        .await
        .expect("run reaches Failed after the disabled capability is rejected at the gateway");
    let failure = state
        .failure
        .as_ref()
        .expect("a Failed run must carry a failure detail");
    assert_eq!(
        failure.category(),
        "model_error",
        "expected LoopFailureKind::ModelError, got {failure:?}"
    );

    // No side effect: the capability was rejected before dispatch, so it was
    // never invoked.
    assert!(
        h.assert_tool_invoked("builtin.spawn_subagent")
            .await
            .is_err(),
        "disabled capability must never be dispatched, even when the model calls it anyway"
    );
}
