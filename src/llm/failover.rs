//! Multi-provider LLM failover.
//!
//! Wraps multiple LlmProvider instances and tries each in sequence
//! until one succeeds. Transparent to callers --- same LlmProvider trait.

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use rust_decimal::Decimal;

use crate::error::LlmError;
use crate::llm::provider::{
    CompletionRequest, CompletionResponse, LlmProvider, ToolCompletionRequest,
    ToolCompletionResponse,
};

/// Returns `true` if the error is transient and the request should be retried
/// on the next provider in the failover chain.
///
/// Retryable: `RequestFailed`, `RateLimited`, `InvalidResponse`,
/// `SessionRenewalFailed`, `ModelNotAvailable`, `Http`, `Io`.
///
/// `ModelNotAvailable` is retryable because the next provider in the chain may
/// offer a different model, so it's worth trying.
///
/// Non-retryable errors (`AuthFailed`, `SessionExpired`, `ContextLengthExceeded`)
/// propagate immediately because a different provider won't fix them.
fn is_retryable(err: &LlmError) -> bool {
    matches!(
        err,
        LlmError::RequestFailed { .. }
            | LlmError::RateLimited { .. }
            | LlmError::InvalidResponse { .. }
            | LlmError::SessionRenewalFailed { .. }
            // ModelNotAvailable is retryable: the next provider may offer a different model.
            | LlmError::ModelNotAvailable { .. }
            | LlmError::Http(_)
            | LlmError::Io(_)
    )
}

/// An LLM provider that wraps multiple providers and tries each in sequence
/// on transient failures.
///
/// The first provider in the list is the primary. If it fails with a retryable
/// error, the next provider is tried, and so on. Non-retryable errors
/// (e.g. `AuthFailed`, `ContextLengthExceeded`) propagate immediately.
pub struct FailoverProvider {
    providers: Vec<Arc<dyn LlmProvider>>,
    /// Index of the provider that last handled a request successfully.
    /// Used by `model_name()` and `cost_per_token()` so downstream cost
    /// tracking reflects the provider that actually served the request.
    last_used: AtomicUsize,
}

impl FailoverProvider {
    /// Create a new failover provider.
    ///
    /// Returns an error if `providers` is empty.
    pub fn new(providers: Vec<Arc<dyn LlmProvider>>) -> Result<Self, LlmError> {
        if providers.is_empty() {
            return Err(LlmError::RequestFailed {
                provider: "failover".to_string(),
                reason: "FailoverProvider requires at least one provider".to_string(),
            });
        }
        Ok(Self {
            providers,
            last_used: AtomicUsize::new(0),
        })
    }

    /// Try each provider in sequence until one succeeds or all fail.
    async fn try_providers<T, F, Fut>(&self, mut call: F) -> Result<T, LlmError>
    where
        F: FnMut(Arc<dyn LlmProvider>) -> Fut,
        Fut: Future<Output = Result<T, LlmError>>,
    {
        let mut last_error: Option<LlmError> = None;

        for (i, provider) in self.providers.iter().enumerate() {
            let result = call(Arc::clone(provider)).await;
            match result {
                Ok(response) => {
                    self.last_used.store(i, Ordering::Relaxed);
                    return Ok(response);
                }
                Err(err) => {
                    if !is_retryable(&err) {
                        return Err(err);
                    }
                    if i + 1 < self.providers.len() {
                        tracing::warn!(
                            provider = %provider.model_name(),
                            error = %err,
                            next_provider = %self.providers[i + 1].model_name(),
                            "Provider failed with retryable error, trying next provider"
                        );
                    }
                    last_error = Some(err);
                }
            }
        }

        // SAFETY: providers is non-empty (checked in `new`), so at least one
        // iteration ran and `last_error` is `Some`.
        Err(last_error.expect("providers list is non-empty"))
    }
}

#[async_trait]
impl LlmProvider for FailoverProvider {
    fn model_name(&self) -> &str {
        self.providers[self.last_used.load(Ordering::Relaxed)].model_name()
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        self.providers[self.last_used.load(Ordering::Relaxed)].cost_per_token()
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        self.try_providers(|provider| {
            let req = request.clone();
            async move { provider.complete(req).await }
        })
        .await
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        self.try_providers(|provider| {
            let req = request.clone();
            async move { provider.complete_with_tools(req).await }
        })
        .await
    }

    async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        let mut all_models = Vec::new();

        for provider in &self.providers {
            match provider.list_models().await {
                Ok(models) => all_models.extend(models),
                Err(err) => {
                    tracing::warn!(
                        provider = %provider.model_name(),
                        error = %err,
                        "Failed to list models from provider, skipping"
                    );
                }
            }
        }

        all_models.sort();
        all_models.dedup();
        Ok(all_models)
    }

    fn effective_model_name(&self, requested_model: Option<&str>) -> String {
        self.providers[self.last_used.load(Ordering::Relaxed)].effective_model_name(requested_model)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Mutex;
    use std::time::Duration;

    use crate::llm::provider::{CompletionResponse, FinishReason, ToolCompletionResponse};

    /// A mock LLM provider that returns a predetermined result.
    struct MockProvider {
        name: String,
        input_cost: Decimal,
        output_cost: Decimal,
        complete_result: Mutex<Option<Result<CompletionResponse, LlmError>>>,
        tool_complete_result: Mutex<Option<Result<ToolCompletionResponse, LlmError>>>,
    }

    impl MockProvider {
        fn succeeding(name: &str, content: &str) -> Self {
            Self {
                name: name.to_string(),
                input_cost: Decimal::ZERO,
                output_cost: Decimal::ZERO,
                complete_result: Mutex::new(Some(Ok(CompletionResponse {
                    content: content.to_string(),
                    input_tokens: 10,
                    output_tokens: 5,
                    finish_reason: FinishReason::Stop,
                    response_id: None,
                }))),
                tool_complete_result: Mutex::new(Some(Ok(ToolCompletionResponse {
                    content: Some(content.to_string()),
                    tool_calls: vec![],
                    input_tokens: 10,
                    output_tokens: 5,
                    finish_reason: FinishReason::Stop,
                    response_id: None,
                }))),
            }
        }

        fn succeeding_with_cost(
            name: &str,
            content: &str,
            input_cost: Decimal,
            output_cost: Decimal,
        ) -> Self {
            Self {
                input_cost,
                output_cost,
                ..Self::succeeding(name, content)
            }
        }

        fn failing_retryable(name: &str) -> Self {
            Self {
                name: name.to_string(),
                input_cost: Decimal::ZERO,
                output_cost: Decimal::ZERO,
                complete_result: Mutex::new(Some(Err(LlmError::RequestFailed {
                    provider: name.to_string(),
                    reason: "server error".to_string(),
                }))),
                tool_complete_result: Mutex::new(Some(Err(LlmError::RequestFailed {
                    provider: name.to_string(),
                    reason: "server error".to_string(),
                }))),
            }
        }

        fn failing_non_retryable(name: &str) -> Self {
            Self {
                name: name.to_string(),
                input_cost: Decimal::ZERO,
                output_cost: Decimal::ZERO,
                complete_result: Mutex::new(Some(Err(LlmError::AuthFailed {
                    provider: name.to_string(),
                }))),
                tool_complete_result: Mutex::new(Some(Err(LlmError::AuthFailed {
                    provider: name.to_string(),
                }))),
            }
        }

        fn failing_rate_limited(name: &str) -> Self {
            Self {
                name: name.to_string(),
                input_cost: Decimal::ZERO,
                output_cost: Decimal::ZERO,
                complete_result: Mutex::new(Some(Err(LlmError::RateLimited {
                    provider: name.to_string(),
                    retry_after: Some(Duration::from_secs(30)),
                }))),
                tool_complete_result: Mutex::new(Some(Err(LlmError::RateLimited {
                    provider: name.to_string(),
                    retry_after: Some(Duration::from_secs(30)),
                }))),
            }
        }
    }

    #[async_trait]
    impl LlmProvider for MockProvider {
        fn model_name(&self) -> &str {
            &self.name
        }

        fn cost_per_token(&self) -> (Decimal, Decimal) {
            (self.input_cost, self.output_cost)
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            self.complete_result
                .lock()
                .unwrap()
                .take()
                .expect("MockProvider::complete called more than once")
        }

        async fn complete_with_tools(
            &self,
            _request: ToolCompletionRequest,
        ) -> Result<ToolCompletionResponse, LlmError> {
            self.tool_complete_result
                .lock()
                .unwrap()
                .take()
                .expect("MockProvider::complete_with_tools called more than once")
        }

        async fn list_models(&self) -> Result<Vec<String>, LlmError> {
            Ok(vec![self.name.clone()])
        }
    }

    fn make_request() -> CompletionRequest {
        CompletionRequest::new(vec![crate::llm::ChatMessage::user("hello")])
    }

    fn make_tool_request() -> ToolCompletionRequest {
        ToolCompletionRequest::new(vec![crate::llm::ChatMessage::user("hello")], vec![])
    }

    // Test 1: Primary succeeds, no failover occurs.
    #[tokio::test]
    async fn primary_succeeds_no_failover() {
        let primary = Arc::new(MockProvider::succeeding("primary", "primary response"));
        let fallback = Arc::new(MockProvider::succeeding("fallback", "fallback response"));

        let failover = FailoverProvider::new(vec![primary, fallback]).unwrap();

        let response = failover.complete(make_request()).await.unwrap();
        assert_eq!(response.content, "primary response");
    }

    // Test 2: Primary fails with retryable error, fallback succeeds.
    #[tokio::test]
    async fn primary_fails_retryable_fallback_succeeds() {
        let primary = Arc::new(MockProvider::failing_retryable("primary"));
        let fallback = Arc::new(MockProvider::succeeding("fallback", "fallback response"));

        let failover = FailoverProvider::new(vec![primary, fallback]).unwrap();

        let response = failover.complete(make_request()).await.unwrap();
        assert_eq!(response.content, "fallback response");
    }

    // Test 3: All providers fail, returns last error.
    #[tokio::test]
    async fn all_providers_fail_returns_last_error() {
        let primary = Arc::new(MockProvider::failing_retryable("primary"));
        let fallback = Arc::new(MockProvider::failing_retryable("fallback"));

        let failover = FailoverProvider::new(vec![primary, fallback]).unwrap();

        let err = failover.complete(make_request()).await.unwrap_err();
        match err {
            LlmError::RequestFailed { provider, .. } => {
                assert_eq!(provider, "fallback");
            }
            other => panic!("expected RequestFailed, got: {other:?}"),
        }
    }

    // Test 4: Non-retryable error fails immediately, no failover.
    #[tokio::test]
    async fn non_retryable_error_fails_immediately() {
        let primary = Arc::new(MockProvider::failing_non_retryable("primary"));
        let fallback = Arc::new(MockProvider::succeeding("fallback", "fallback response"));

        let failover = FailoverProvider::new(vec![primary, fallback]).unwrap();

        let err = failover.complete(make_request()).await.unwrap_err();
        match err {
            LlmError::AuthFailed { provider } => {
                assert_eq!(provider, "primary");
            }
            other => panic!("expected AuthFailed, got: {other:?}"),
        }
    }

    // Test 5: Three providers, first two fail (retryable), third succeeds.
    #[tokio::test]
    async fn three_providers_first_two_fail_third_succeeds() {
        let p1 = Arc::new(MockProvider::failing_retryable("provider-1"));
        let p2 = Arc::new(MockProvider::failing_rate_limited("provider-2"));
        let p3 = Arc::new(MockProvider::succeeding("provider-3", "third time lucky"));

        let failover = FailoverProvider::new(vec![p1, p2, p3]).unwrap();

        let response = failover.complete(make_request()).await.unwrap();
        assert_eq!(response.content, "third time lucky");
    }

    // Test: complete_with_tools follows same failover logic.
    #[tokio::test]
    async fn complete_with_tools_failover() {
        let primary = Arc::new(MockProvider::failing_retryable("primary"));
        let fallback = Arc::new(MockProvider::succeeding("fallback", "tools fallback"));

        let failover = FailoverProvider::new(vec![primary, fallback]).unwrap();

        let response = failover
            .complete_with_tools(make_tool_request())
            .await
            .unwrap();
        assert_eq!(response.content.as_deref(), Some("tools fallback"));
    }

    // Test: model_name and cost_per_token reflect the last-used provider.
    #[tokio::test]
    async fn model_name_and_cost_track_last_used_provider() {
        let fallback_cost = Decimal::new(15, 6); // 0.000015

        let primary = Arc::new(MockProvider::failing_retryable("primary-model"));
        let fallback = Arc::new(MockProvider::succeeding_with_cost(
            "fallback-model",
            "ok",
            fallback_cost,
            fallback_cost,
        ));

        let failover = FailoverProvider::new(vec![primary, fallback]).unwrap();

        // Before any call, defaults to primary (index 0).
        assert_eq!(failover.model_name(), "primary-model");
        assert_eq!(failover.cost_per_token(), (Decimal::ZERO, Decimal::ZERO));

        // After failover, should reflect the fallback provider.
        let _ = failover.complete(make_request()).await.unwrap();
        assert_eq!(failover.model_name(), "fallback-model");
        assert_eq!(failover.cost_per_token(), (fallback_cost, fallback_cost));
    }

    // Test: list_models aggregates from all providers.
    #[tokio::test]
    async fn list_models_aggregates_all() {
        let p1 = Arc::new(MockProvider::succeeding("model-a", "ok"));
        let p2 = Arc::new(MockProvider::succeeding("model-b", "ok"));

        let failover = FailoverProvider::new(vec![p1, p2]).unwrap();

        let models = failover.list_models().await.unwrap();
        assert!(models.contains(&"model-a".to_string()));
        assert!(models.contains(&"model-b".to_string()));
    }

    // Test: is_retryable correctly classifies errors.
    #[test]
    fn retryable_classification() {
        // Retryable
        assert!(is_retryable(&LlmError::RequestFailed {
            provider: "p".into(),
            reason: "err".into(),
        }));
        assert!(is_retryable(&LlmError::RateLimited {
            provider: "p".into(),
            retry_after: None,
        }));
        assert!(is_retryable(&LlmError::InvalidResponse {
            provider: "p".into(),
            reason: "bad json".into(),
        }));
        assert!(is_retryable(&LlmError::SessionRenewalFailed {
            provider: "p".into(),
            reason: "timeout".into(),
        }));
        assert!(is_retryable(&LlmError::Io(std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "reset"
        ))));
        assert!(is_retryable(&LlmError::ModelNotAvailable {
            provider: "p".into(),
            model: "m".into(),
        }));

        // Non-retryable
        assert!(!is_retryable(&LlmError::AuthFailed {
            provider: "p".into(),
        }));
        assert!(!is_retryable(&LlmError::SessionExpired {
            provider: "p".into(),
        }));
        assert!(!is_retryable(&LlmError::ContextLengthExceeded {
            used: 100_000,
            limit: 50_000,
        }));
    }

    // Test: empty providers list returns error (not panic).
    #[test]
    fn empty_providers_returns_error() {
        let result = FailoverProvider::new(vec![]);
        assert!(result.is_err());
    }
}
