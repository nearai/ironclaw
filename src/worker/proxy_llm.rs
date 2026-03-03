//! LLM provider that proxies all calls through the orchestrator HTTP API.
//!
//! The worker never has direct access to API keys or session tokens.
//! All LLM requests go through the orchestrator, which holds the real credentials.

use std::sync::Arc;

use async_trait::async_trait;
use rust_decimal::Decimal;

use crate::error::LlmError;
use crate::llm::{
    CompletionRequest, CompletionResponse, LlmProvider, ToolCompletionRequest,
    ToolCompletionResponse,
};
use crate::worker::api::WorkerHttpClient;

/// An LLM provider that routes all calls through the orchestrator's HTTP API.
///
/// No API keys or secrets are needed in the container. The orchestrator
/// handles authentication and billing.
pub struct ProxyLlmProvider {
    client: Arc<WorkerHttpClient>,
    model_name: String,
}

impl ProxyLlmProvider {
    pub fn new(client: Arc<WorkerHttpClient>, model_name: String) -> Self {
        Self { client, model_name }
    }
}

#[async_trait]
impl LlmProvider for ProxyLlmProvider {
    fn provider_name(&self) -> &str {
        "proxy"
    }

    fn model_name(&self) -> &str {
        &self.model_name
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        // Cost tracking happens on the orchestrator side
        (Decimal::ZERO, Decimal::ZERO)
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        self.client
            .llm_complete(&request)
            .await
            .map_err(|e| LlmError::RequestFailed {
                provider: "proxy".to_string(),
                reason: e.to_string(),
            })
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        self.client
            .llm_complete_with_tools(&request)
            .await
            .map_err(|e| LlmError::RequestFailed {
                provider: "proxy".to_string(),
                reason: e.to_string(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_proxy_model_name() {
        let client = Arc::new(WorkerHttpClient::new(
            "http://localhost:50051".to_string(),
            uuid::Uuid::nil(),
            "test".to_string(),
        ));
        let provider = ProxyLlmProvider::new(client, "test-model".to_string());
        assert_eq!(provider.model_name(), "test-model");
    }

    #[test]
    fn test_proxy_cost_is_zero() {
        let client = Arc::new(WorkerHttpClient::new(
            "http://localhost:50051".to_string(),
            uuid::Uuid::nil(),
            "test".to_string(),
        ));
        let provider = ProxyLlmProvider::new(client, "test-model".to_string());
        let (input, output) = provider.cost_per_token();
        assert_eq!(input, Decimal::ZERO);
        assert_eq!(output, Decimal::ZERO);
    }
}
