//! Reborn integration-test framework — slice 1 smoke test.
//!
//! Proves the single LLM seam end-to-end: synthetic inbound → product workflow
//! → scheduler → planned agent loop → real `LlmProviderModelGateway` → real
//! `ironclaw_llm` decorator chain (hermetic passthrough) → scripted `TraceLlm`
//! → assistant reply finalized in thread history. InMemory storage, no services,
//! no keys, no Docker, no `integration` feature.

// The support tree is large and shared; a single-test file exercises only a
// slice of it, so suppress dead-code warnings on the includes (matches
// `reborn_qa_recorded_behavior.rs`).
#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
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
}
