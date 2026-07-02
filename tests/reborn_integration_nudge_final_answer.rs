//! Product-level proof: the final-answer nudge fires through the real
//! `submit_turn` entry point (product workflow → turn coordinator →
//! scheduler → agent loop → real `LlmProviderModelGateway` decorator chain
//! → scripted model), one layer up from the executor/driver-tier proof in
//! `crates/ironclaw_reborn/tests/planned_driver_e2e.rs`.
//!
//! `RebornIntegrationHarness::test_default()` resolves `requested_run_profile:
//! None` to `planned_default` — the profile Task 2 enabled driver-specific
//! nudges for — with no special wiring. Four identical `builtin.http` calls
//! (same URL) drive the real no-progress detector: `RecordingRuntimeHttpEgress`
//! (installed by `.with_builtin_http_tools()`) always returns the same fixed
//! scripted body, so the first call's output digest is first-seen
//! (`MadeProgress`) and the next three repeat the same digest (`NoChange`) —
//! `trailing_no_progress_results` reaches the default
//! `typed_progress_run_threshold` (3) right after the 4th capability batch —
//! `DefaultStopConditionStrategy::should_stop_after_observed_turn` in
//! `crates/ironclaw_agent_loop/src/strategies/stop.rs`. The executor then
//! resolves that `NoProgressDetected` stop via `try_final_answer_nudge`
//! (`crates/ironclaw_agent_loop/src/executor/loop_exit.rs`), issuing one
//! extra tool-free model call that the 5th scripted reply satisfies.
//!
//! Deviation from the plan's starting shape: the brief scripted
//! `builtin.echo`, reasoning that a first-party capability with a stable
//! digest would drive the detector. `builtin.echo` IS registered as a
//! first-party handler with `CapabilityVisibility::Model`
//! (`crates/ironclaw_host_runtime/src/first_party_tools/mod.rs`), but the
//! `reborn-planned-default` run profile's resolved capability surface (as
//! observed via `RUST_LOG=debug` — `visible_capability_sample`) does not
//! include it; the model gateway rejects the scripted call as "outside the
//! visible capability surface" (`ironclaw_reborn::model_gateway`), which
//! surfaces as a terminal `model_error`, not a no-progress signal. Swapping to
//! `builtin.http` (already proven visible + deterministic by
//! `tests/reborn_integration_tool_call.rs`) exercises the same digest-based
//! `NoChange` mechanism without depending on a capability outside this
//! profile's advertised surface.

#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
mod support;

use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::reply::RebornScriptedReply;

/// Fixed URL under the harness's `http_test_policy` allowlist
/// (`api.example.test`); every call gets the same scripted body from
/// `RecordingRuntimeHttpEgress::with_body`, so repeated calls produce the
/// same output digest.
const REPEATED_URL: &str = "https://api.example.test/v1/items";

#[tokio::test]
async fn no_progress_repeated_http_call_completes_via_final_answer_nudge() {
    let h = RebornIntegrationHarness::test_default()
        .with_builtin_http_tools()
        .script([
            RebornScriptedReply::tool_call(
                "builtin.http",
                serde_json::json!({"url": REPEATED_URL}),
            ),
            RebornScriptedReply::tool_call(
                "builtin.http",
                serde_json::json!({"url": REPEATED_URL}),
            ),
            RebornScriptedReply::tool_call(
                "builtin.http",
                serde_json::json!({"url": REPEATED_URL}),
            ),
            RebornScriptedReply::tool_call(
                "builtin.http",
                serde_json::json!({"url": REPEATED_URL}),
            ),
            RebornScriptedReply::text("final answer synthesized via nudge"),
        ])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("fetch the same item four times")
        .await
        .expect("turn completes");
    h.assert_reply_contains("final answer synthesized via nudge")
        .await
        .expect("reply finalized via the final-answer nudge");
}
