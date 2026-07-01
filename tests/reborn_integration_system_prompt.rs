//! Reborn integration test — system-prompt capture seam (T0-SYSPROMPT).
//!
//! Proves the harness retains the scripted `TraceLlm` before the
//! `dyn LlmProvider` upcast so tests can assert on the model-visible system
//! prompt. A scripted turn runs the real prompt-assembly path; the composed
//! capability policy is rendered into a `System`-role message, and
//! `assert_system_prompt_contains` reads it back from `captured_requests()`.
//! Unblocks every prompt-injection / prompt-content assertion (C-SAFETY,
//! C-SKILL, C-PROFILE prompt-line).

// The support tree is large and shared; a single-test file exercises only a
// slice of it, so suppress dead-code warnings on the includes (matches
// `reborn_integration_greeting.rs`).
#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
mod support;

use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::reply::RebornScriptedReply;

#[tokio::test]
async fn captures_model_visible_system_prompt() {
    let harness = RebornIntegrationHarness::test_default()
        .script([RebornScriptedReply::text("done")])
        .build()
        .await
        .expect("harness builds");
    harness
        .submit_turn("hi there")
        .await
        .expect("turn completes");
    harness
        .assert_system_prompt_contains("Use only visible capabilities.")
        .await
        .expect("composed capability policy reached the model as a system prompt");
}
