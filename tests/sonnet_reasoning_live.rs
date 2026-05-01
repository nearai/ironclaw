//! Live integration test that exercises end-to-end reasoning capture for Anthropic Claude:
//! Claude Sonnet 4.6 (claude-sonnet-4-6, Anthropic direct) → rig-core anthropic provider → llm_reasoning
//! extractor (`^claude` rule, matches `type: "thinking"` content blocks) →
//! CompletionResponse.reasoning.
//!
//! Run with:
//!   ANTHROPIC_API_KEY=sk-ant-... \
//!     cargo test --test sonnet_reasoning_live -- --ignored
//!
//! Anthropic only emits thinking content blocks when the request opts in via
//! `thinking: { type: "enabled", budget_tokens: N }`. RigAdapter injects that
//! parameter automatically for any model whose name starts with `claude-`,
//! using the budget from `ANTHROPIC_THINKING_BUDGET` (default 4096).
//!
//! Skipped by default to avoid hitting paid APIs in CI.

use ironclaw::llm::{ChatMessage, CompletionRequest, LlmProvider, RigAdapter};

#[tokio::test]
#[ignore]
async fn sonnet_reasoning_lands_in_completion_response() {
    let api_key =
        std::env::var("ANTHROPIC_API_KEY").expect("ANTHROPIC_API_KEY env var required");

    use rig::client::CompletionClient;
    use rig::providers::anthropic;

    let client: anthropic::Client = anthropic::Client::builder()
        .api_key(&api_key)
        .build()
        .expect("build anthropic client");
    let model_name = std::env::var("ANTHROPIC_MODEL")
        .unwrap_or_else(|_| "claude-sonnet-4-6".to_string());
    let model = client.completion_model(&model_name);
    let adapter = RigAdapter::new(model, &model_name);

    // max_tokens needs to be large enough for budget_tokens + actual answer.
    let req = CompletionRequest::new(vec![ChatMessage::user(
        "What is 7 times 8? Think step by step, then give the final answer.",
    )])
    .with_max_tokens(8192);

    let resp = adapter.complete(req).await.expect("completion");

    eprintln!("== model: {}", model_name);
    eprintln!("== content: {:?}", resp.content);
    eprintln!("== reasoning: {:?}", resp.reasoning);

    assert!(
        resp.reasoning
            .as_ref()
            .map(|s| !s.is_empty())
            .unwrap_or(false),
        "expected extended-thinking reasoning to be populated; got {:?}",
        resp.reasoning
    );
}
