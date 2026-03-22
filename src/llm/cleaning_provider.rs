//! A decorator that strips thinking tags from all LLM output.

use std::sync::Arc;

use async_trait::async_trait;
use rust_decimal::Decimal;

use crate::llm::clean_response;
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
        resp.content = clean_response(&resp.content);
        Ok(resp)
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        let mut resp = self.inner.complete_with_tools(request).await?;
        if let Some(ref content) = resp.content {
            resp.content = Some(clean_response(content));
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

        async fn complete(
            &self,
            _req: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
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
}
