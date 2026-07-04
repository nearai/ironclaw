//! Reborn integration-test framework — slice 1 smoke test.
//!
//! Proves the single LLM seam end-to-end: synthetic inbound → product workflow
//! → scheduler → planned agent loop → real `LlmProviderModelGateway` → real
//! `ironclaw_llm` decorator chain (hermetic passthrough) → scripted `TraceLlm`
//! → assistant reply finalized in thread history. InMemory storage, no services,
//! no keys, no Docker, no `integration` feature.
//!
//! Asserts BOTH facets of the one default turn: the finalized reply (output
//! seam) and the model-visible system prompt (input seam, T0-SYSPROMPT). The
//! system-prompt assertion rides this smoke test rather than a redundant file —
//! it exercises the same `build → submit_turn` path, so consolidating avoids a
//! second full support-tree compile for zero new path coverage.

// The support tree is large and shared; a single-test file exercises only a
// slice of it, so suppress dead-code warnings on the includes (matches
// `reborn_qa_recorded_behavior.rs`).
#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::reply::RebornScriptedReply;

#[tokio::test]
async fn replies_to_greeting() {
    let harness = RebornIntegrationHarness::test_default()
        .script([RebornScriptedReply::text("Hello! How can I help?")])
        .build()
        .await
        .expect("harness builds");
    harness
        .submit_turn("hi there")
        .await
        .expect("turn completes");
    harness
        .assert_reply_contains("Hello! How can I help?")
        .await
        .expect("reply finalized in thread history");
    // Input seam (T0-SYSPROMPT): the composed capability policy is rendered
    // into a `System`-role message the model actually saw this turn.
    harness
        .assert_system_prompt_contains("Use only visible capabilities.")
        .await
        .expect("composed capability policy reached the model as a system prompt");
    // Negative guard: the user's own turn text appears in the captured request
    // but only in a `User`-role message, so the `System`-only filter must not
    // match it — proves the assertion discriminates on role, not mere presence.
    assert!(
        harness
            .assert_system_prompt_contains("hi there")
            .await
            .is_err(),
        "system-prompt assertion must not match user-role text"
    );
}
