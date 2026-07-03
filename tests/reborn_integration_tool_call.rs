//! Reborn integration-test framework — tool-calling turn.
//!
//! Proves the tool path + the §3.7 two-tier egress design end-to-end: the
//! scripted model emits a `builtin.http` tool call → the real first-party tool
//! runtime executes it through `RuntimeHttpEgress` → the call is captured by the
//! recording egress (Tier-2) → the model finalizes a text reply. Uses the same
//! scripted `TraceLlm` seam beneath the real decorator chain as other harness
//! tests; no network, no services, no keys, no Docker, no `integration` feature.

#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
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

/// Guards the assertion helpers against silently passing: when the scripted tool
/// call is *absent* (a plain text turn on the default echo backend, which runs no
/// tool and captures no egress), both `assert_tool_invoked` and
/// `assert_egress_request_matching` must return `Err`.
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

/// Proves the assertion helpers discriminate when the invocation + egress lists
/// are NON-empty: a real `builtin.http` call runs, but assertions for a
/// *different* capability / host must return `Err` — exercising the
/// "present but no match" branch, not the empty-list branch (builder.rs:331).
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
    // Prove the capture lists are NON-empty first, so the negative checks below
    // exercise the mismatch branch rather than passing vacuously on empty lists.
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

/// Proves the multi-segment `builtin.http.save` capability id — whose
/// `.`→`__` encoding produces `builtin__http__save` at the provider seam
/// (reply.rs:33) — resolves end-to-end through the real runtime.
///
/// Args: `url` (same constant the existing test uses) + `save_to` under the
/// `/workspace` mount that `core_builtin_tools` provides with read-write
/// permissions. The recording egress returns a fixed `{"accepted":true}` body;
/// no network is touched.
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
    // The save path must reach the real `RuntimeHttpEgress`; assert the recorded
    // egress so a regression that bypasses it cannot pass this test.
    h.assert_egress_request_matching("api.example.test")
        .await
        .expect("http.save egress captured");
    h.assert_reply_contains("saved")
        .await
        .expect("final reply finalized");
}

/// The globally-disabled `builtin.spawn_subagent` capability
/// (`ironclaw_reborn::runtime::DISABLED_CAPABILITY_IDS`, applied as the
/// OUTERMOST `PerSurfaceCapabilityDenyDecorator` in
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
    // model was shown (the `.`→`__` provider-seam encoding, or the raw
    // dotted capability id — structurally impossible as a provider tool name
    // since `ProviderToolName` rejects dots, but checked defensively).
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

/// A model that hallucinates a call to the disabled `builtin.spawn_subagent`
/// capability anyway — the deny filter's `CapabilitySurfaceDenyFilter` strips
/// the id from `tool_definitions()`, so `builtin__spawn_subagent` is not in
/// `advertised_tool_names` at the model gateway
/// (`crates/ironclaw_reborn/src/model_gateway.rs::tool_response_to_host`).
/// The gateway falls back to `provider_calls_are_advertised_or_resolvable`,
/// which resolves the id via `provider_tool_call_capability_ids` — the deny
/// filter's inner port still resolves it structurally, but the deny filter's
/// own scope check then rejects it (`"provider tool call targets a disabled
/// capability"`, `AgentLoopHostErrorKind::InvalidInvocation`) — so the whole
/// provider response (not just the one call) is rejected with
/// `HostManagedModelErrorKind::InvalidOutput` before any
/// `CapabilityCallCandidate` is ever registered.
///
/// Observed contract (pinned as-is, no production changes): `InvalidOutput`
/// maps to `AgentLoopHostErrorKind::Unavailable` (`model_gateway_error` in
/// `crates/ironclaw_loop_support/src/lib.rs`), which the executor's
/// `ModelStage` classifies as `ModelErrorClass::Unavailable` and hands to the
/// recovery strategy — which aborts without a further model call, so the
/// run reaches a terminal `TurnStatus::Failed` with
/// `SanitizedFailure::category() == "model_error"` (`LoopFailureKind::ModelError`)
/// after consuming exactly the ONE scripted model turn (mirrors the sibling
/// gateway-error contract pinned by
/// `reborn_integration_cancel.rs::mid_turn_provider_error_reaches_failed_with_model_error_category`,
/// reached here via a distinct root cause: deny-filter rejection at
/// registration, not a raw provider error). This is a run-level failure, not
/// a model-visible `Failed`/`Denied` tool result — no `ToolResultReference`
/// is ever persisted for the call, because the deny filter rejects it before
/// `register_provider_tool_call` ever stages an invocation. The no-side-effect
/// proof is `assert_tool_invoked` returning `Err` — the capability was never
/// dispatched.
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
