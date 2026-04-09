//! A decorator that strips thinking tags from all LLM output.

use std::sync::Arc;

use async_trait::async_trait;
use rust_decimal::Decimal;

use crate::llm::{clean_response, truncate_at_tool_tags};
use crate::llm::error::LlmError;
use crate::llm::provider::{
    CompletionRequest, CompletionResponse, LlmProvider, ModelMetadata, ToolCompletionRequest,
    ToolCompletionResponse,
};

/// Wraps any `LlmProvider` and applies `clean_response()` to strip thinking
/// tags (`<think>`, `<thinking>`, etc.) from all completion output.
pub struct CleaningProvider {
    inner: Arc<dyn LlmProvider>,
}

impl CleaningProvider {
    pub fn new(inner: Arc<dyn LlmProvider>) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl LlmProvider for CleaningProvider {
    fn model_name(&self) -> &str {
        self.inner.model_name()
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        self.inner.cost_per_token()
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let mut resp = self.inner.complete(request).await?;
        resp.content = clean_response(&truncate_at_tool_tags(&resp.content));
        Ok(resp)
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        let mut resp = self.inner.complete_with_tools(request).await?;
        if let Some(ref content) = resp.content {
            resp.content = Some(clean_response(&truncate_at_tool_tags(content)));
        }
        for tc in &mut resp.tool_calls {
            if let Some(ref r) = tc.reasoning {
                tc.reasoning = Some(clean_response(&truncate_at_tool_tags(r)));
            }
        }
        Ok(resp)
    }

    async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        self.inner.list_models().await
    }

    async fn model_metadata(&self) -> Result<ModelMetadata, LlmError> {
        self.inner.model_metadata().await
    }

    fn effective_model_name(&self, requested_model: Option<&str>) -> String {
        self.inner.effective_model_name(requested_model)
    }

    fn active_model_name(&self) -> String {
        self.inner.active_model_name()
    }

    fn set_model(&self, model: &str) -> Result<(), LlmError> {
        self.inner.set_model(model)
    }

    fn calculate_cost(&self, input_tokens: u32, output_tokens: u32) -> Decimal {
        self.inner.calculate_cost(input_tokens, output_tokens)
    }

    fn cache_write_multiplier(&self) -> Decimal {
        self.inner.cache_write_multiplier()
    }

    fn cache_read_discount(&self) -> Decimal {
        self.inner.cache_read_discount()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::provider::{ChatMessage, FinishReason};

    struct FakeProvider;

    #[async_trait]
    impl LlmProvider for FakeProvider {
        fn model_name(&self) -> &str {
            "fake"
        }

        fn cost_per_token(&self) -> (Decimal, Decimal) {
            (Decimal::ZERO, Decimal::ZERO)
        }

        async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
            Ok(CompletionResponse {
                content: "<think>internal reasoning</think>The actual answer".to_string(),
                input_tokens: 10,
                output_tokens: 5,
                finish_reason: FinishReason::Stop,
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
            })
        }

        async fn complete_with_tools(
            &self,
            _req: ToolCompletionRequest,
        ) -> Result<ToolCompletionResponse, LlmError> {
            Ok(ToolCompletionResponse {
                content: Some("<think>thinking</think>tool response".to_string()),
                tool_calls: vec![],
                input_tokens: 10,
                output_tokens: 5,
                finish_reason: FinishReason::Stop,
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
            })
        }
    }

    #[tokio::test]
    async fn strips_think_tags_from_complete() {
        let provider = CleaningProvider::new(Arc::new(FakeProvider));
        let req = CompletionRequest::new(vec![ChatMessage::user("test")]);
        let resp = provider.complete(req).await.unwrap();
        assert_eq!(resp.content, "The actual answer");
    }

    #[tokio::test]
    async fn strips_think_tags_from_complete_with_tools() {
        let provider = CleaningProvider::new(Arc::new(FakeProvider));
        let req = ToolCompletionRequest::new(vec![ChatMessage::user("test")], vec![]);
        let resp = provider.complete_with_tools(req).await.unwrap();
        assert_eq!(resp.content.as_deref(), Some("tool response"));
    }

    #[tokio::test]
    async fn delegates_model_name() {
        let provider = CleaningProvider::new(Arc::new(FakeProvider));
        assert_eq!(provider.model_name(), "fake");
    }

    #[tokio::test]
    async fn propagates_error_from_inner_complete() {
        struct FailProvider;

        #[async_trait]
        impl LlmProvider for FailProvider {
            fn model_name(&self) -> &str {
                "fail"
            }
            fn cost_per_token(&self) -> (Decimal, Decimal) {
                (Decimal::ZERO, Decimal::ZERO)
            }
            async fn complete(
                &self,
                _req: CompletionRequest,
            ) -> Result<CompletionResponse, LlmError> {
                Err(LlmError::RequestFailed {
                    provider: "fail".to_string(),
                    reason: "simulated failure".to_string(),
                })
            }
            async fn complete_with_tools(
                &self,
                _req: ToolCompletionRequest,
            ) -> Result<ToolCompletionResponse, LlmError> {
                Err(LlmError::RequestFailed {
                    provider: "fail".to_string(),
                    reason: "simulated tool failure".to_string(),
                })
            }
        }

        let provider = CleaningProvider::new(Arc::new(FailProvider));

        let req = CompletionRequest::new(vec![ChatMessage::user("test")]);
        let err = provider.complete(req).await.unwrap_err();
        assert!(
            matches!(err, LlmError::RequestFailed { ref reason, .. } if reason == "simulated failure"),
            "expected RequestFailed, got: {err:?}"
        );

        let tool_req = ToolCompletionRequest::new(vec![ChatMessage::user("test")], vec![]);
        let tool_err = provider.complete_with_tools(tool_req).await.unwrap_err();
        assert!(
            matches!(tool_err, LlmError::RequestFailed { ref reason, .. } if reason == "simulated tool failure"),
            "expected RequestFailed, got: {tool_err:?}"
        );
    }

    #[tokio::test]
    async fn preserves_token_counts_after_cleaning() {
        let provider = CleaningProvider::new(Arc::new(FakeProvider));

        let req = CompletionRequest::new(vec![ChatMessage::user("test")]);
        let resp = provider.complete(req).await.unwrap();
        assert_eq!(resp.input_tokens, 10);
        assert_eq!(resp.output_tokens, 5);
        assert_eq!(resp.cache_read_input_tokens, 0);
        assert_eq!(resp.cache_creation_input_tokens, 0);
        assert_eq!(resp.finish_reason, FinishReason::Stop);
    }

    #[tokio::test]
    async fn passes_through_content_without_tags() {
        struct CleanProvider;

        #[async_trait]
        impl LlmProvider for CleanProvider {
            fn model_name(&self) -> &str {
                "clean"
            }
            fn cost_per_token(&self) -> (Decimal, Decimal) {
                (Decimal::ZERO, Decimal::ZERO)
            }
            async fn complete(
                &self,
                _req: CompletionRequest,
            ) -> Result<CompletionResponse, LlmError> {
                Ok(CompletionResponse {
                    content: "Just a normal response with no tags".to_string(),
                    input_tokens: 1,
                    output_tokens: 1,
                    finish_reason: FinishReason::Stop,
                    cache_read_input_tokens: 0,
                    cache_creation_input_tokens: 0,
                })
            }
            async fn complete_with_tools(
                &self,
                _req: ToolCompletionRequest,
            ) -> Result<ToolCompletionResponse, LlmError> {
                unreachable!()
            }
        }

        let provider = CleaningProvider::new(Arc::new(CleanProvider));
        let req = CompletionRequest::new(vec![ChatMessage::user("test")]);
        let resp = provider.complete(req).await.unwrap();
        assert_eq!(resp.content, "Just a normal response with no tags");
    }

    #[tokio::test]
    async fn preserves_tool_calls_alongside_cleaned_content() {
        struct ToolAndContentProvider;

        #[async_trait]
        impl LlmProvider for ToolAndContentProvider {
            fn model_name(&self) -> &str {
                "tool-content"
            }
            fn cost_per_token(&self) -> (Decimal, Decimal) {
                (Decimal::ZERO, Decimal::ZERO)
            }
            async fn complete(
                &self,
                _req: CompletionRequest,
            ) -> Result<CompletionResponse, LlmError> {
                unreachable!()
            }
            async fn complete_with_tools(
                &self,
                _req: ToolCompletionRequest,
            ) -> Result<ToolCompletionResponse, LlmError> {
                use crate::llm::provider::ToolCall;
                Ok(ToolCompletionResponse {
                    content: Some("<think>planning</think>I'll search for that".to_string()),
                    tool_calls: vec![ToolCall {
                        id: "call_123".to_string(),
                        name: "web_search".to_string(),
                        arguments: serde_json::json!({"query": "test"}),
                        reasoning: None,
                    }],
                    input_tokens: 20,
                    output_tokens: 15,
                    finish_reason: FinishReason::ToolUse,
                    cache_read_input_tokens: 0,
                    cache_creation_input_tokens: 0,
                })
            }
        }

        let provider = CleaningProvider::new(Arc::new(ToolAndContentProvider));
        let req = ToolCompletionRequest::new(vec![ChatMessage::user("test")], vec![]);
        let resp = provider.complete_with_tools(req).await.unwrap();
        assert_eq!(resp.content.as_deref(), Some("I'll search for that"));
        assert_eq!(resp.tool_calls.len(), 1);
        assert_eq!(resp.tool_calls[0].name, "web_search");
        assert_eq!(resp.finish_reason, FinishReason::ToolUse);
    }

    #[tokio::test]
    async fn handles_empty_string_content() {
        struct EmptyProvider;

        #[async_trait]
        impl LlmProvider for EmptyProvider {
            fn model_name(&self) -> &str {
                "empty"
            }
            fn cost_per_token(&self) -> (Decimal, Decimal) {
                (Decimal::ZERO, Decimal::ZERO)
            }
            async fn complete(
                &self,
                _req: CompletionRequest,
            ) -> Result<CompletionResponse, LlmError> {
                Ok(CompletionResponse {
                    content: String::new(),
                    input_tokens: 5,
                    output_tokens: 0,
                    finish_reason: FinishReason::Stop,
                    cache_read_input_tokens: 0,
                    cache_creation_input_tokens: 0,
                })
            }
            async fn complete_with_tools(
                &self,
                _req: ToolCompletionRequest,
            ) -> Result<ToolCompletionResponse, LlmError> {
                unreachable!()
            }
        }

        let provider = CleaningProvider::new(Arc::new(EmptyProvider));
        let req = CompletionRequest::new(vec![ChatMessage::user("test")]);
        let resp = provider.complete(req).await.unwrap();
        assert_eq!(resp.content, "");
    }

    #[tokio::test]
    async fn strips_to_empty_when_only_think_tags() {
        struct OnlyThinkProvider;

        #[async_trait]
        impl LlmProvider for OnlyThinkProvider {
            fn model_name(&self) -> &str {
                "only-think"
            }
            fn cost_per_token(&self) -> (Decimal, Decimal) {
                (Decimal::ZERO, Decimal::ZERO)
            }
            async fn complete(
                &self,
                _req: CompletionRequest,
            ) -> Result<CompletionResponse, LlmError> {
                Ok(CompletionResponse {
                    content: "<think>reasoning only, no visible output</think>".to_string(),
                    input_tokens: 8,
                    output_tokens: 6,
                    finish_reason: FinishReason::Stop,
                    cache_read_input_tokens: 0,
                    cache_creation_input_tokens: 0,
                })
            }
            async fn complete_with_tools(
                &self,
                _req: ToolCompletionRequest,
            ) -> Result<ToolCompletionResponse, LlmError> {
                unreachable!()
            }
        }

        let provider = CleaningProvider::new(Arc::new(OnlyThinkProvider));
        let req = CompletionRequest::new(vec![ChatMessage::user("test")]);
        let resp = provider.complete(req).await.unwrap();
        assert!(
            resp.content.trim().is_empty(),
            "expected empty content after stripping think-only response, got: {:?}",
            resp.content
        );
    }

    #[tokio::test]
    async fn strips_multiple_think_blocks() {
        struct MultiThinkProvider;

        #[async_trait]
        impl LlmProvider for MultiThinkProvider {
            fn model_name(&self) -> &str {
                "multi"
            }
            fn cost_per_token(&self) -> (Decimal, Decimal) {
                (Decimal::ZERO, Decimal::ZERO)
            }
            async fn complete(
                &self,
                _req: CompletionRequest,
            ) -> Result<CompletionResponse, LlmError> {
                Ok(CompletionResponse {
                    content: "<think>first thought</think>Hello <think>second thought</think>world"
                        .to_string(),
                    input_tokens: 10,
                    output_tokens: 10,
                    finish_reason: FinishReason::Stop,
                    cache_read_input_tokens: 0,
                    cache_creation_input_tokens: 0,
                })
            }
            async fn complete_with_tools(
                &self,
                _req: ToolCompletionRequest,
            ) -> Result<ToolCompletionResponse, LlmError> {
                unreachable!()
            }
        }

        let provider = CleaningProvider::new(Arc::new(MultiThinkProvider));
        let req = CompletionRequest::new(vec![ChatMessage::user("test")]);
        let resp = provider.complete(req).await.unwrap();
        assert_eq!(resp.content, "Hello world");
    }

    #[tokio::test]
    async fn delegates_cost_and_cache_methods() {
        struct CostProvider;

        #[async_trait]
        impl LlmProvider for CostProvider {
            fn model_name(&self) -> &str {
                "cost"
            }
            fn cost_per_token(&self) -> (Decimal, Decimal) {
                (Decimal::new(3, 6), Decimal::new(15, 6))
            }
            fn cache_write_multiplier(&self) -> Decimal {
                Decimal::new(125, 2)
            }
            fn cache_read_discount(&self) -> Decimal {
                Decimal::new(10, 0)
            }
            async fn complete(
                &self,
                _req: CompletionRequest,
            ) -> Result<CompletionResponse, LlmError> {
                unreachable!()
            }
            async fn complete_with_tools(
                &self,
                _req: ToolCompletionRequest,
            ) -> Result<ToolCompletionResponse, LlmError> {
                unreachable!()
            }
        }

        let provider = CleaningProvider::new(Arc::new(CostProvider));
        let (input, output) = provider.cost_per_token();
        assert_eq!(input, Decimal::new(3, 6));
        assert_eq!(output, Decimal::new(15, 6));
        assert_eq!(provider.cache_write_multiplier(), Decimal::new(125, 2));
        assert_eq!(provider.cache_read_discount(), Decimal::new(10, 0));
        let cost = provider.calculate_cost(1000, 500);
        assert_eq!(
            cost,
            Decimal::new(3, 6) * Decimal::from(1000) + Decimal::new(15, 6) * Decimal::from(500)
        );
    }

    #[tokio::test]
    async fn preserves_none_content_in_tool_response() {
        struct NoContentProvider;

        #[async_trait]
        impl LlmProvider for NoContentProvider {
            fn model_name(&self) -> &str {
                "no-content"
            }
            fn cost_per_token(&self) -> (Decimal, Decimal) {
                (Decimal::ZERO, Decimal::ZERO)
            }
            async fn complete(
                &self,
                _req: CompletionRequest,
            ) -> Result<CompletionResponse, LlmError> {
                unreachable!()
            }
            async fn complete_with_tools(
                &self,
                _req: ToolCompletionRequest,
            ) -> Result<ToolCompletionResponse, LlmError> {
                Ok(ToolCompletionResponse {
                    content: None,
                    tool_calls: vec![],
                    input_tokens: 0,
                    output_tokens: 0,
                    finish_reason: FinishReason::ToolUse,
                    cache_read_input_tokens: 0,
                    cache_creation_input_tokens: 0,
                })
            }
        }

        let provider = CleaningProvider::new(Arc::new(NoContentProvider));
        let req = ToolCompletionRequest::new(vec![ChatMessage::user("test")], vec![]);
        let resp = provider.complete_with_tools(req).await.unwrap();
        assert_eq!(resp.content, None);
    }

    #[tokio::test]
    async fn truncates_unclosed_tool_call_tags() {
        struct UnclosedToolTagProvider;

        #[async_trait]
        impl LlmProvider for UnclosedToolTagProvider {
            fn model_name(&self) -> &str {
                "unclosed"
            }
            fn cost_per_token(&self) -> (Decimal, Decimal) {
                (Decimal::ZERO, Decimal::ZERO)
            }
            async fn complete(
                &self,
                _req: CompletionRequest,
            ) -> Result<CompletionResponse, LlmError> {
                Ok(CompletionResponse {
                    content: "<think>hmm</think>The answer is 42\n<tool_call>{\"name\": \"search\"}"
                        .to_string(),
                    input_tokens: 10,
                    output_tokens: 8,
                    finish_reason: FinishReason::Stop,
                    cache_read_input_tokens: 0,
                    cache_creation_input_tokens: 0,
                })
            }
            async fn complete_with_tools(
                &self,
                _req: ToolCompletionRequest,
            ) -> Result<ToolCompletionResponse, LlmError> {
                Ok(ToolCompletionResponse {
                    content: Some(
                        "<think>plan</think>Here you go\n<tool_call>{\"name\": \"fetch\"}"
                            .to_string(),
                    ),
                    tool_calls: vec![],
                    input_tokens: 10,
                    output_tokens: 8,
                    finish_reason: FinishReason::Stop,
                    cache_read_input_tokens: 0,
                    cache_creation_input_tokens: 0,
                })
            }
        }

        let provider = CleaningProvider::new(Arc::new(UnclosedToolTagProvider));

        // complete() path
        let req = CompletionRequest::new(vec![ChatMessage::user("test")]);
        let resp = provider.complete(req).await.unwrap();
        assert_eq!(resp.content, "The answer is 42");

        // complete_with_tools() path
        let req = ToolCompletionRequest::new(vec![ChatMessage::user("test")], vec![]);
        let resp = provider.complete_with_tools(req).await.unwrap();
        assert_eq!(resp.content.as_deref(), Some("Here you go"));
    }

    #[tokio::test]
    async fn cleans_reasoning_field_on_tool_calls() {
        struct ReasoningToolProvider;

        #[async_trait]
        impl LlmProvider for ReasoningToolProvider {
            fn model_name(&self) -> &str {
                "reasoning-tool"
            }
            fn cost_per_token(&self) -> (Decimal, Decimal) {
                (Decimal::ZERO, Decimal::ZERO)
            }
            async fn complete(
                &self,
                _req: CompletionRequest,
            ) -> Result<CompletionResponse, LlmError> {
                unreachable!()
            }
            async fn complete_with_tools(
                &self,
                _req: ToolCompletionRequest,
            ) -> Result<ToolCompletionResponse, LlmError> {
                use crate::llm::provider::ToolCall;
                Ok(ToolCompletionResponse {
                    content: None,
                    tool_calls: vec![
                        ToolCall {
                            id: "call_1".to_string(),
                            name: "search".to_string(),
                            arguments: serde_json::json!({"q": "test"}),
                            reasoning: Some(
                                "<think>I need to search</think>Searching now\n<tool_call>{}"
                                    .to_string(),
                            ),
                        },
                        ToolCall {
                            id: "call_2".to_string(),
                            name: "fetch".to_string(),
                            arguments: serde_json::json!({}),
                            reasoning: None,
                        },
                    ],
                    input_tokens: 15,
                    output_tokens: 10,
                    finish_reason: FinishReason::ToolUse,
                    cache_read_input_tokens: 0,
                    cache_creation_input_tokens: 0,
                })
            }
        }

        let provider = CleaningProvider::new(Arc::new(ReasoningToolProvider));
        let req = ToolCompletionRequest::new(vec![ChatMessage::user("test")], vec![]);
        let resp = provider.complete_with_tools(req).await.unwrap();

        assert_eq!(resp.tool_calls.len(), 2);
        // First tool call: thinking tags stripped and unclosed tool_call truncated
        assert_eq!(
            resp.tool_calls[0].reasoning.as_deref(),
            Some("Searching now")
        );
        // Second tool call: None reasoning preserved
        assert_eq!(resp.tool_calls[1].reasoning, None);
    }
}
