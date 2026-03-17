//! Adapter bridging `LlmProvider` → `JudgeLlm` for the LLM-as-Judge layer.
//!
//! `LlmProviderJudge` wraps any `Arc<dyn LlmProvider>` and implements
//! `JudgeLlm`, so `LlmJudge` can use the project's existing LLM
//! infrastructure — connection pooling, retry, rate limiting, and API key
//! management are all inherited automatically.

use std::sync::Arc;

use async_trait::async_trait;

use ironclaw_safety::JudgeLlm;

use crate::llm::{ChatMessage, CompletionRequest, LlmProvider};

/// Adapter that implements [`JudgeLlm`] using the existing [`LlmProvider`].
pub struct LlmProviderJudge {
    provider: Arc<dyn LlmProvider>,
}

impl LlmProviderJudge {
    pub fn new(provider: Arc<dyn LlmProvider>) -> Self {
        Self { provider }
    }
}

#[async_trait]
impl JudgeLlm for LlmProviderJudge {
    async fn complete_text(
        &self,
        system: &str,
        user: &str,
        model_override: Option<&str>,
        max_tokens: u32,
    ) -> Result<String, String> {
        let messages = vec![ChatMessage::system(system), ChatMessage::user(user)];
        let mut req = CompletionRequest::new(messages)
            .with_temperature(0.0)
            .with_max_tokens(max_tokens);
        if let Some(model) = model_override {
            req = req.with_model(model);
        }
        self.provider
            .complete(req)
            .await
            .map(|resp| resp.content)
            .map_err(|e| e.to_string())
    }
}
