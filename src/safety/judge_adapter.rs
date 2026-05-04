//! Adapter bridging `LlmProvider` → `JudgeLlm` for the LLM-as-Judge layer.
//!
//! `LlmProviderJudge` wraps any `Arc<dyn LlmProvider>` and implements
//! `JudgeLlm`. It applies the configured timeout here (in the main crate,
//! which has tokio) so `ironclaw_safety` stays free of tokio as a dependency.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use ironclaw_safety::JudgeLlm;

use crate::llm::{ChatMessage, CompletionRequest, LlmProvider};

/// Adapter that implements [`JudgeLlm`] using the existing [`LlmProvider`].
///
/// Enforces `timeout_ms` on every judge call, failing open on expiry so that
/// a slow judge never stalls the agent. This keeps tokio out of the
/// `ironclaw_safety` crate while still honouring the timeout contract.
pub struct LlmProviderJudge {
    provider: Arc<dyn LlmProvider>,
    timeout: Duration,
}

impl LlmProviderJudge {
    pub fn new(provider: Arc<dyn LlmProvider>, timeout_ms: u64) -> Self {
        Self {
            provider,
            timeout: Duration::from_millis(timeout_ms),
        }
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
        let provider = Arc::clone(&self.provider);
        tokio::time::timeout(self.timeout, provider.complete(req))
            .await
            .map_err(|_| format!("judge LLM timed out after {}ms", self.timeout.as_millis()))?
            .map(|resp| resp.content)
            .map_err(|e| e.to_string())
    }
}
