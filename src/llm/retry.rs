//! Shared retry helpers and composable `RetryProvider` decorator for LLM providers.
//!
//! Provides:
//! - `is_retryable()` — `LlmError`-level retryability classification (shared with `failover.rs`)
//! - `retry_backoff_delay()` — exponential backoff with jitter
//! - `RetryProvider` — decorator that wraps any `LlmProvider` with automatic retries

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rand::Rng;
use rust_decimal::Decimal;

use crate::llm::error::LlmError;
use crate::llm::provider::{
    CompletionRequest, CompletionResponse, LlmProvider, ModelMetadata, ToolCompletionRequest,
    ToolCompletionResponse,
};

/// Upper bound for provider-suggested `Retry-After` delays.
///
/// This prevents malicious or malformed headers from turning a retryable
/// response into an effectively unbounded sleep.
pub(crate) const MAX_RETRY_AFTER_SECS: u64 = 3600;

/// Returns `true` if the `LlmError` is transient and the request should be retried.
///
/// Used by `RetryProvider` (retry the same provider) and `FailoverProvider`
/// (try the next provider). The question is: "could this exact same request
/// succeed if we try again?"
///
/// Retryable: `RequestFailed`, `RateLimited`, `BadGateway`, `InvalidResponse`,
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
            | LlmError::BadGateway { .. }
            | LlmError::InvalidResponse { .. }
            | LlmError::EmptyResponse { .. }
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

/// Clamp a provider-suggested retry delay to a safe maximum.
pub(crate) fn cap_retry_after(duration: Duration) -> Duration {
    duration.min(Duration::from_secs(MAX_RETRY_AFTER_SECS))
}

/// Parse a `Retry-After` header value into a capped `Duration`.
///
/// Supports both delay-seconds (RFC 7231 §7.1.3) and HTTP-date formats
/// (RFC 7231 §7.1.1 / IMF-fixdate). The implementation uses
/// `chrono::DateTime::parse_from_rfc2822`, which also accepts RFC 2822-style
/// dates.
///
/// Returns `DEFAULT_RETRY_AFTER` (60 s) if the header is missing or
/// unparseable. That "missing → 60 s" default is specifically for rate-limit
/// / auth-retry paths where we always want a non-zero delay even if the
/// upstream omits the header. Callers that need to distinguish
/// "header absent" from "header present but unparseable" — so that missing
/// headers can fall through to exponential backoff instead — must use
/// [`parse_retry_after_value`] on the `&HeaderValue` extracted from the
/// headers map directly.
pub(crate) fn parse_retry_after(header: Option<&reqwest::header::HeaderValue>) -> Duration {
    header
        .map(parse_retry_after_value)
        .unwrap_or(Duration::from_secs(DEFAULT_RETRY_AFTER_SECS))
}

/// Parse a *known-present* `Retry-After` header into a capped `Duration`.
///
/// Use this from call sites that want to preserve the `Option` shape around
/// header presence — e.g. 5xx retry paths where a missing header should
/// fall through to [`retry_backoff_delay`] instead of the 60-second default
/// that [`parse_retry_after`] returns for rate-limit semantics. Unparseable
/// values still fall back to `DEFAULT_RETRY_AFTER` (60 s).
pub(crate) fn parse_retry_after_value(header: &reqwest::header::HeaderValue) -> Duration {
    let parsed = header.to_str().ok().and_then(|v| {
        if let Ok(secs) = v.trim().parse::<u64>() {
            return Some(cap_retry_after(Duration::from_secs(secs)));
        }
        if let Ok(dt) = chrono::DateTime::parse_from_rfc2822(v.trim()) {
            let now = chrono::Utc::now();
            let delta = dt.signed_duration_since(now);
            return Some(cap_retry_after(Duration::from_secs(
                delta.num_seconds().max(0) as u64,
            )));
        }
        None
    });
    parsed.unwrap_or(Duration::from_secs(DEFAULT_RETRY_AFTER_SECS))
}

const DEFAULT_RETRY_AFTER_SECS: u64 = 60;

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

    async fn retry_loop<T, F, Fut>(&self, mut op: F, label: &str) -> Result<T, LlmError>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, LlmError>>,
    {
        let mut last_error: Option<LlmError> = None;

        for attempt in 0..=self.config.max_retries {
            match op().await {
                Ok(resp) => return Ok(resp),
                Err(err) => {
                    if !is_retryable(&err) || attempt == self.config.max_retries {
                        return Err(err);
                    }

                    let delay = match &err {
                        LlmError::RateLimited {
                            retry_after: Some(duration),
                            ..
                        }
                        | LlmError::BadGateway {
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
                        "Retrying after transient error{label}"
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
}

#[async_trait]
impl LlmProvider for RetryProvider {
    fn model_name(&self) -> &str {
        self.inner.model_name()
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        self.inner.cost_per_token()
    }

    fn cache_write_multiplier(&self) -> Decimal {
        self.inner.cache_write_multiplier()
    }

    fn cache_read_discount(&self) -> Decimal {
        self.inner.cache_read_discount()
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let inner = &self.inner;
        self.retry_loop(
            || {
                let req = request.clone();
                async move { inner.complete(req).await }
            },
            "",
        )
        .await
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        let inner = &self.inner;
        self.retry_loop(
            || {
                let req = request.clone();
                async move { inner.complete_with_tools(req).await }
            },
            " (tools)",
        )
        .await
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
        // BadGateway — regression test for #1994 (NEAR AI 502s were
        // surfacing to users unchanged because they were not retried).
        assert!(is_retryable(&LlmError::BadGateway {
            provider: "p".into(),
            status: 502,
            retry_after: None,
        }));
        // Any 5xx status code surfaces as BadGateway — including 500 which
        // is the exact case called out by the PR #2753 review (Gemini,
        // security-medium): upstream 500 with a Python traceback in the body
        // used to leak through `RequestFailed { reason }`, now mapped to
        // BadGateway which drops the body entirely.
        assert!(is_retryable(&LlmError::BadGateway {
            provider: "p".into(),
            status: 500,
            retry_after: None,
        }));
    }

    /// Regression for PR #2753 review: when `BadGateway` carries
    /// `retry_after: None` (because the upstream response didn't include a
    /// Retry-After header), the retry loop must fall through to
    /// `retry_backoff_delay` instead of matching the `Some(_)` arm. A
    /// previous version of `nearai_chat` wrapped every response in
    /// `Some(parse_retry_after(...))`, which defaulted missing headers to
    /// 60s and silently defeated exponential backoff.
    #[test]
    fn bad_gateway_without_retry_after_does_not_match_some_arm() {
        let err = LlmError::BadGateway {
            provider: "p".into(),
            status: 502,
            retry_after: None,
        };
        // Mirrors the match arms inside `RetryProvider::retry_loop` —
        // if this assertion ever fails, exponential backoff is silently
        // being replaced by whatever `retry_after` defaulted to.
        let explicitly_timed = matches!(
            err,
            LlmError::BadGateway {
                retry_after: Some(_),
                ..
            } | LlmError::RateLimited {
                retry_after: Some(_),
                ..
            }
        );
        assert!(
            !explicitly_timed,
            "BadGateway without Retry-After must fall through to exponential backoff"
        );

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

    // Regression test: Rate limiter fallback when Retry-After header is missing
    //
    // Verifies that RateLimited errors always have a duration (never None)
    // due to the 60-second fallback applied in all rate limit error creation sites
    // (nearai_chat.rs, anthropic_oauth.rs, embeddings.rs).
    #[test]
    fn rate_limited_error_always_has_duration() {
        let err = LlmError::RateLimited {
            provider: "test".to_string(),
            retry_after: Some(std::time::Duration::from_secs(60)),
        };

        if let LlmError::RateLimited { retry_after, .. } = err {
            assert!(
                retry_after.is_some(),
                "Rate limited error should always have retry_after duration"
            );
            assert_eq!(
                retry_after,
                Some(std::time::Duration::from_secs(60)),
                "Fallback should be 60 seconds"
            );
        } else {
            panic!("Expected RateLimited error");
        }
    }

    #[test]
    fn cap_retry_after_clamps_huge_delays() {
        assert_eq!(
            cap_retry_after(Duration::from_secs(u64::MAX)),
            Duration::from_secs(MAX_RETRY_AFTER_SECS)
        );
        assert_eq!(
            cap_retry_after(Duration::from_secs(0)),
            Duration::from_secs(0)
        );
    }

    #[test]
    fn parse_retry_after_delay_seconds() {
        let val = reqwest::header::HeaderValue::from_static("30");
        assert_eq!(parse_retry_after(Some(&val)), Duration::from_secs(30));
    }

    #[test]
    fn parse_retry_after_missing_header() {
        assert_eq!(
            parse_retry_after(None),
            Duration::from_secs(DEFAULT_RETRY_AFTER_SECS)
        );
    }

    /// `parse_retry_after_value` takes a known-present `&HeaderValue`, so
    /// callers that want to distinguish "absent" from "unparseable" (e.g.
    /// `nearai_chat`'s 5xx branch, which falls through to exponential
    /// backoff when the header is missing) can preserve the `Option` shape
    /// around presence. Unparseable values still default to the 60 s floor.
    #[test]
    fn parse_retry_after_value_parses_delay_seconds() {
        let val = reqwest::header::HeaderValue::from_static("30");
        assert_eq!(parse_retry_after_value(&val), Duration::from_secs(30));
    }

    #[test]
    fn parse_retry_after_value_unparseable_falls_back_to_default() {
        let val = reqwest::header::HeaderValue::from_static("not-a-number");
        assert_eq!(
            parse_retry_after_value(&val),
            Duration::from_secs(DEFAULT_RETRY_AFTER_SECS)
        );
    }

    #[test]
    fn parse_retry_after_unparseable() {
        let val = reqwest::header::HeaderValue::from_static("not-a-number");
        assert_eq!(
            parse_retry_after(Some(&val)),
            Duration::from_secs(DEFAULT_RETRY_AFTER_SECS)
        );
    }

    #[test]
    fn parse_retry_after_clamps_large_value() {
        let val = reqwest::header::HeaderValue::from_static("999999");
        assert_eq!(
            parse_retry_after(Some(&val)),
            Duration::from_secs(MAX_RETRY_AFTER_SECS)
        );
    }

    #[test]
    fn parse_retry_after_http_date() {
        let future = chrono::Utc::now() + chrono::Duration::seconds(30);
        let date_str = future.to_rfc2822();
        let val = reqwest::header::HeaderValue::from_str(&date_str).unwrap();
        let parsed = parse_retry_after(Some(&val));
        let diff = if parsed > Duration::from_secs(30) {
            parsed - Duration::from_secs(30)
        } else {
            Duration::from_secs(30) - parsed
        };
        assert!(
            diff <= Duration::from_secs(2),
            "expected ~30s, got {parsed:?} (diff {diff:?}) from header {date_str:?}"
        );
    }
}
