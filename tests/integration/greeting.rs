//! IronClaw integration-test framework — smoke test.
//!
//! Proves the single LLM seam end-to-end: synthetic inbound → product workflow
//! → scheduler → planned agent loop → real `LlmProviderModelGateway` → real
//! `ironclaw_llm` decorator chain (hermetic passthrough) → scripted `TraceLlm`
//! → assistant reply finalized in thread history. InMemory storage, no
//! services, no keys, no Docker, no `integration` feature.
//!
//! Asserts both facets of the turn: the finalized reply (output seam) and the
//! model-visible system prompt (input seam, T0-SYSPROMPT) — consolidated here
//! rather than a redundant file, since both ride the same `build → submit_turn`
//! path.

// The support tree is large and shared; a single-test file only exercises a
// slice, so suppress dead-code warnings on the includes.
#[allow(dead_code)]
#[path = "support/mod.rs"]
mod ironclaw_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use ironclaw_support::builder::IronClawIntegrationHarness;
use ironclaw_support::reply::IronClawScriptedReply;

#[tokio::test]
async fn replies_to_greeting() {
    let harness = IronClawIntegrationHarness::test_default()
        .script([IronClawScriptedReply::text("Hello! How can I help?")])
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
    // Negative guard: the user's text appears only in a `User`-role message,
    // so the `System`-only filter must not match it — proves role discrimination.
    assert!(
        harness
            .assert_system_prompt_contains("hi there")
            .await
            .is_err(),
        "system-prompt assertion must not match user-role text"
    );
}
