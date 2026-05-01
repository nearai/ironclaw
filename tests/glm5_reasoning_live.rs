//! Live integration test that exercises the patched rig-core fork end-to-end:
//! GLM-5 (Z.AI OpenAI-compatible) → rig-core (forked: Message::Assistant carries
//! reasoning_content) → llm_reasoning extractor → CompletionResponse.reasoning.
//!
//! Run with: BLUE_API_KEY=... cargo test --test glm5_reasoning_live -- --ignored
//! Skipped by default to avoid hitting paid APIs in CI.

use ironclaw::llm::{ChatMessage, CompletionRequest, LlmProvider, RigAdapter};

#[tokio::test]
#[ignore]
async fn glm5_reasoning_lands_in_completion_response() {
    let api_key = std::env::var("BLUE_API_KEY").expect("BLUE_API_KEY env var required");
    let base_url = std::env::var("BLUE_BASE_URL")
        .unwrap_or_else(|_| "https://api.z.ai/api/paas/v4".to_string());

    use rig::client::CompletionClient;
    use rig::providers::openai;

    let client: openai::Client = openai::Client::builder()
        .api_key(&api_key)
        .base_url(&base_url)
        .build()
        .expect("build openai client");
    let client = client.completions_api();
    let model = client.completion_model("glm-5");
    let adapter = RigAdapter::new(model, "glm-5");

    let req = CompletionRequest::new(vec![ChatMessage::user(
        "Reply with the single word OK and nothing else.",
    )])
    .with_max_tokens(100);

    let resp = adapter.complete(req).await.expect("completion");

    eprintln!("== content: {:?}", resp.content);
    eprintln!("== reasoning: {:?}", resp.reasoning);

    assert!(
        resp.reasoning
            .as_ref()
            .map(|s| !s.is_empty())
            .unwrap_or(false),
        "expected reasoning_content to be populated; got {:?}",
        resp.reasoning
    );
}
