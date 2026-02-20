//! Shared retry helpers and composable `RetryProvider` decorator for LLM providers.
//!
//! Provides:
//! - `is_retryable()` — `LlmError`-level retryability classification (shared with `failover.rs`)
//! - `retry_backoff_delay()` — exponential backoff with jitter
//! - `RetryProvider` — decorator that wraps any `LlmProvider` with automatic retries

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rand::Rng;
use rust_decimal::Decimal;

use crate::error::LlmError;
use crate::llm::provider::{
    CompletionRequest, CompletionResponse, LlmProvider, ModelMetadata, ToolCompletionRequest,
    ToolCompletionResponse,
};

/// Returns `true` if the `LlmError` is transient and the request should be retried.
///
/// Used by `RetryProvider` (retry the same provider) and `FailoverProvider`
/// (try the next provider). The question is: "could this exact same request
/// succeed if we try again?"
///
/// Retryable: `RequestFailed`, `RateLimited`, `InvalidResponse`,
/// `SessionRenewalFailed`, `Http`, `Io`.
///
/// Non-retryable: `AuthFailed`, `SessionExpired`, `ContextLengthExceeded`,
/// `ModelNotAvailable`, `Json`.
/// - `SessionExpired` — handled by session renewal layer, not by retry
/// - `ModelNotAvailable` — the model won't appear between attempts
/// - `Json` — a serde parse bug, not a transient failure
///
/// See also `circuit_breaker::is_transient()` which answers a different
/// question: "does this error indicate the backend is degraded?"
pub(crate) fn is_retryable(err: &LlmError) -> bool {
    matches!(
        err,
        LlmError::RequestFailed { .. }
            | LlmError::RateLimited { .. }
            | LlmError::InvalidResponse { .. }
            | LlmError::SessionRenewalFailed { .. }
            | LlmError::Http(_)
            | LlmError::Io(_)
    )
}

/// Calculate exponential backoff delay with random jitter.
///
/// Base delay is 1 second, doubled each attempt, with +/-25% jitter.
/// - attempt 0: ~1s (0.75s - 1.25s)
/// - attempt 1: ~2s (1.5s - 2.5s)
/// - attempt 2: ~4s (3.0s - 5.0s)
pub(crate) fn retry_backoff_delay(attempt: u32) -> Duration {
    let base_ms: u64 = 1000u64.saturating_mul(2u64.saturating_pow(attempt));
    let jitter_range = base_ms / 4; // 25%
    let jitter = if jitter_range > 0 {
        let offset = rand::thread_rng().gen_range(0..=jitter_range * 2);
        offset as i64 - jitter_range as i64
    } else {
        0
    };
    let delay_ms = (base_ms as i64 + jitter).max(100) as u64;
    Duration::from_millis(delay_ms)
}

/// Configuration for the retry decorator.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts (not counting the initial attempt).
    /// Default: 3.
    pub max_retries: u32,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self { max_retries: 3 }
    }
}

/// Composable decorator that wraps any `LlmProvider` with automatic retries.
///
/// On transient errors, sleeps using exponential backoff and retries.
/// On non-transient errors (`AuthFailed`, `ContextLengthExceeded`, `SessionExpired`),
/// returns immediately.
///
/// Special handling for `RateLimited { retry_after }`: uses the provider-suggested
/// duration if available, otherwise falls back to standard backoff.
pub struct RetryProvider {
    inner: Arc<dyn LlmProvider>,
    config: RetryConfig,
}

impl RetryProvider {
    pub fn new(inner: Arc<dyn LlmProvider>, config: RetryConfig) -> Self {
        Self { inner, config }
    }
}

#[async_trait]
impl LlmProvider for RetryProvider {
    fn model_name(&self) -> &str {
        self.inner.model_name()
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        self.inner.cost_per_token()
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let mut last_error: Option<LlmError> = None;

        for attempt in 0..=self.config.max_retries {
            let req = request.clone();
            match self.inner.complete(req).await {
                Ok(resp) => return Ok(resp),
                Err(err) => {
                    if !is_retryable(&err) || attempt == self.config.max_retries {
                        return Err(err);
                    }

                    let delay = match &err {
                        LlmError::RateLimited {
                            retry_after: Some(duration),
                            ..
                        } => *duration,
                        _ => retry_backoff_delay(attempt),
                    };

                    tracing::warn!(
                        provider = %self.inner.model_name(),
                        attempt = attempt + 1,
                        max_retries = self.config.max_retries,
                        delay_ms = delay.as_millis() as u64,
                        error = %err,
                        "Retrying after transient error"
                    );

                    last_error = Some(err);
                    tokio::time::sleep(delay).await;
                }
            }
        }

        Err(last_error.unwrap_or_else(|| LlmError::RequestFailed {
            provider: self.inner.model_name().to_string(),
            reason: "retry loop exited unexpectedly".to_string(),
        }))
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        let mut last_error: Option<LlmError> = None;

        for attempt in 0..=self.config.max_retries {
            let req = request.clone();
            match self.inner.complete_with_tools(req).await {
                Ok(resp) => return Ok(resp),
                Err(err) => {
                    if !is_retryable(&err) || attempt == self.config.max_retries {
                        return Err(err);
                    }

                    let delay = match &err {
                        LlmError::RateLimited {
                            retry_after: Some(duration),
                            ..
                        } => *duration,
                        _ => retry_backoff_delay(attempt),
                    };

                    tracing::warn!(
                        provider = %self.inner.model_name(),
                        attempt = attempt + 1,
                        max_retries = self.config.max_retries,
                        delay_ms = delay.as_millis() as u64,
                        error = %err,
                        "Retrying after transient error (tools)"
                    );

                    last_error = Some(err);
                    tokio::time::sleep(delay).await;
                }
            }
        }

        Err(last_error.unwrap_or_else(|| LlmError::RequestFailed {
            provider: self.inner.model_name().to_string(),
            reason: "retry loop exited unexpectedly".to_string(),
        }))
    }

    async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        self.inner.list_models().await
    }

    async fn model_metadata(&self) -> Result<ModelMetadata, LlmError> {
        self.inner.model_metadata().await
    }

    fn active_model_name(&self) -> String {
        self.inner.active_model_name()
    }

    fn set_model(&self, model: &str) -> Result<(), LlmError> {
        self.inner.set_model(model)
    }

    fn seed_response_chain(&self, thread_id: &str, response_id: String) {
        self.inner.seed_response_chain(thread_id, response_id)
    }

    fn get_response_chain_id(&self, thread_id: &str) -> Option<String> {
        self.inner.get_response_chain_id(thread_id)
    }

    fn calculate_cost(&self, input_tokens: u32, output_tokens: u32) -> Decimal {
        self.inner.calculate_cost(input_tokens, output_tokens)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::testing::StubLlm;

    fn make_request() -> CompletionRequest {
        CompletionRequest::new(vec![crate::llm::ChatMessage::user("hello")])
    }

    fn make_tool_request() -> ToolCompletionRequest {
        ToolCompletionRequest::new(vec![crate::llm::ChatMessage::user("hello")], vec![])
    }

    fn fast_config(max_retries: u32) -> RetryConfig {
        RetryConfig { max_retries }
    }

    // -- Backoff delay tests --

    #[test]
    fn test_retry_backoff_delay_exponential_growth() {
        // Run multiple samples to verify the range, accounting for jitter
        for _ in 0..20 {
            let d0 = retry_backoff_delay(0);
            let d1 = retry_backoff_delay(1);
            let d2 = retry_backoff_delay(2);

            // Attempt 0: base 1000ms, jitter +/-250ms -> [750, 1250]
            assert!(d0.as_millis() >= 750, "attempt 0 too low: {:?}", d0);
            assert!(d0.as_millis() <= 1250, "attempt 0 too high: {:?}", d0);

            // Attempt 1: base 2000ms, jitter +/-500ms -> [1500, 2500]
            assert!(d1.as_millis() >= 1500, "attempt 1 too low: {:?}", d1);
            assert!(d1.as_millis() <= 2500, "attempt 1 too high: {:?}", d1);

            // Attempt 2: base 4000ms, jitter +/-1000ms -> [3000, 5000]
            assert!(d2.as_millis() >= 3000, "attempt 2 too low: {:?}", d2);
            assert!(d2.as_millis() <= 5000, "attempt 2 too high: {:?}", d2);
        }
    }

    #[test]
    fn test_retry_backoff_delay_minimum() {
        // Even at attempt 0, delay should be at least 100ms (the minimum floor)
        for _ in 0..20 {
            let delay = retry_backoff_delay(0);
            assert!(delay.as_millis() >= 100);
        }
    }

    #[test]
    fn test_retry_backoff_delay_no_overflow() {
        // Very high attempt numbers should not panic from overflow
        let delay = retry_backoff_delay(30);
        assert!(delay.as_millis() >= 100);
    }

    // -- is_retryable() classification tests --

    #[test]
    fn test_is_retryable_classification() {
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
            reason: "bad".into(),
        }));
        assert!(is_retryable(&LlmError::SessionRenewalFailed {
            provider: "p".into(),
            reason: "timeout".into(),
        }));
        assert!(is_retryable(&LlmError::Io(std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "reset"
        ))));

        // NOT retryable
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
        assert!(!is_retryable(&LlmError::ModelNotAvailable {
            provider: "p".into(),
            model: "m".into(),
        }));
    }

    // -- RetryProvider tests --

    #[tokio::test]
    async fn success_on_first_attempt() {
        let stub = Arc::new(StubLlm::new("ok").with_model_name("test"));
        let retry = RetryProvider::new(stub.clone(), fast_config(3));

        let resp = retry.complete(make_request()).await;
        assert!(resp.is_ok());
        assert_eq!(resp.unwrap().content, "ok");
        assert_eq!(stub.calls(), 1);
    }

    #[tokio::test]
    async fn retries_transient_errors_then_succeeds() {
        // StubLlm starts failing, then we flip it to succeed.
        // With max_retries=2, it will try 3 times total.
        let stub = Arc::new(StubLlm::failing("test"));
        let retry = RetryProvider::new(stub.clone(), fast_config(2));

        // Spawn a task that flips the stub to succeed after a short delay
        let stub_clone = stub.clone();
        tokio::spawn(async move {
            // Wait for at least 1 retry attempt (backoff is ~1s, so 1.5s should be enough)
            tokio::time::sleep(Duration::from_millis(1500)).await;
            stub_clone.set_failing(false);
        });

        let resp = retry.complete(make_request()).await;
        assert!(resp.is_ok());
        // Should have called at least twice (first fail, then succeed after flip)
        assert!(stub.calls() >= 2);
    }

    #[tokio::test]
    async fn non_transient_error_fails_immediately() {
        let stub = Arc::new(StubLlm::failing_non_transient("test"));
        let retry = RetryProvider::new(stub.clone(), fast_config(3));

        let err = retry.complete(make_request()).await.unwrap_err();
        assert!(matches!(err, LlmError::ContextLengthExceeded { .. }));
        // Should only be called once — no retries for non-transient errors
        assert_eq!(stub.calls(), 1);
    }

    #[tokio::test]
    async fn exhausts_retries_then_returns_error() {
        let stub = Arc::new(StubLlm::failing("test"));
        // max_retries=0 means only the initial attempt, no retries
        let retry = RetryProvider::new(stub.clone(), fast_config(0));

        let err = retry.complete(make_request()).await.unwrap_err();
        assert!(matches!(err, LlmError::RequestFailed { .. }));
        assert_eq!(stub.calls(), 1);
    }

    #[tokio::test]
    async fn complete_with_tools_retries_same_as_complete() {
        let stub = Arc::new(StubLlm::failing_non_transient("test"));
        let retry = RetryProvider::new(stub.clone(), fast_config(3));

        let err = retry
            .complete_with_tools(make_tool_request())
            .await
            .unwrap_err();
        assert!(matches!(err, LlmError::ContextLengthExceeded { .. }));
        assert_eq!(stub.calls(), 1);
    }

    #[tokio::test]
    async fn passthrough_methods_delegate_to_inner() {
        let stub = Arc::new(StubLlm::new("ok").with_model_name("my-model"));
        let retry = RetryProvider::new(stub, fast_config(3));

        assert_eq!(retry.model_name(), "my-model");
        assert_eq!(retry.active_model_name(), "my-model");
        assert_eq!(retry.cost_per_token(), (Decimal::ZERO, Decimal::ZERO));
        assert_eq!(retry.calculate_cost(100, 50), Decimal::ZERO);
    }
}
