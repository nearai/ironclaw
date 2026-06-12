//! Integration test for `NormalizingProvider` — RC1/M1 closure.
//!
//! Verifies that when an inner provider returns a `ToolCompletionResponse`
//! with `finish_reason = FinishReason::Unknown` and a non-empty `tool_calls`
//! list, the `NormalizingProvider` decorator forces `finish_reason` to
//! `FinishReason::ToolUse`.
//!
//! This closes audit RC1/M1 (Bedrock returning `Unknown` with tool calls).

use std::sync::Arc;

use async_trait::async_trait;
use rust_decimal::Decimal;

use ironclaw_llm::NormalizingProvider;
use ironclaw_llm::error::LlmError;
use ironclaw_llm::{
    ChatMessage, CompletionRequest, CompletionResponse, FinishReason, LlmProvider, ToolCall,
    ToolCompletionRequest, ToolCompletionResponse,
};

// ── Stub ────────────────────────────────────────────────────────────────────
//
// `StubLlm` in `ironclaw_llm::testing` always returns `FinishReason::Stop`
// with an empty `tool_calls` vec and cannot be reconfigured for the specific
// (Unknown + non-empty tool_calls) scenario that RC1 exercises. A minimal
// one-shot inline stub is therefore used here.

/// Returns a fixed `ToolCompletionResponse` with `finish_reason = Unknown`
/// and one tool call — simulating Bedrock's misbehaving wire response.
struct BedrockUnknownStub;

#[async_trait]
impl LlmProvider for BedrockUnknownStub {
    fn model_name(&self) -> &str {
        "bedrock-unknown-stub"
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        (Decimal::ZERO, Decimal::ZERO)
    }

    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        unimplemented!("only complete_with_tools is exercised by this test")
    }

    async fn complete_with_tools(
        &self,
        _request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        Ok(ToolCompletionResponse {
            content: None,
            tool_calls: vec![ToolCall {
                id: "1".to_string(),
                name: "echo".to_string(),
                arguments: serde_json::json!({}),
                reasoning: None,
                signature: None,
                arguments_parse_error: None,
            }],
            input_tokens: 10,
            output_tokens: 5,
            // Bedrock (and similar providers) sometimes emit Unknown here even
            // when tool_calls is non-empty — that is the bug RC1/M1 fixes.
            finish_reason: FinishReason::Unknown,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
            reasoning: None,
        })
    }
}

// ── Test ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn bedrock_unknown_finish_with_calls_routes_through_normalizing() {
    let stub: Arc<dyn LlmProvider> = Arc::new(BedrockUnknownStub);
    let wrapped = NormalizingProvider::new(stub);

    let request = ToolCompletionRequest::new(vec![ChatMessage::user("ping")], vec![]);

    let resp = wrapped
        .complete_with_tools(request)
        .await
        .expect("complete_with_tools should succeed");

    // RC1/M1 invariant: non-empty tool_calls must always produce ToolUse.
    assert!(
        matches!(resp.finish_reason, FinishReason::ToolUse),
        "expected FinishReason::ToolUse, got {:?}",
        resp.finish_reason,
    );
    assert_eq!(resp.tool_calls.len(), 1, "tool call must be preserved");
    assert_eq!(resp.tool_calls[0].name, "echo");
    assert_eq!(resp.tool_calls[0].id, "1");
}
