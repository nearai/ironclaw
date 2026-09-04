//! Reborn integration-test framework — smoke test.
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

#[tokio::test]
async fn conversational_turns_reuse_one_pseudonymous_prompt_cache_key() {
    let harness = RebornIntegrationHarness::test_default()
        .script([
            RebornScriptedReply::text("first reply"),
            RebornScriptedReply::text("second reply"),
        ])
        .build()
        .await
        .expect("harness builds");

    harness
        .submit_turn("first message")
        .await
        .expect("first turn");
    harness
        .submit_turn("second message")
        .await
        .expect("second turn");

    let metadata = harness.scripted_llm.captured_request_metadata();
    assert_eq!(metadata.len(), 2, "one provider request per plain turn");
    let first = metadata[0]
        .get(ironclaw_llm::PROMPT_CACHE_KEY_METADATA)
        .expect("first turn carries a prompt cache key");
    let second = metadata[1]
        .get(ironclaw_llm::PROMPT_CACHE_KEY_METADATA)
        .expect("second turn carries a prompt cache key");
    assert_eq!(first, second, "one thread must reuse one cache key");
    assert_eq!(first.len(), 32, "cache key is the bounded digest");
    assert!(first.chars().all(|character| character.is_ascii_hexdigit()));
}
