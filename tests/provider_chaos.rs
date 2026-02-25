//! LLM provider chaos tests (QA Plan item 4.1).
//!
//! Tests the failover chain, circuit breaker, and retry logic under realistic
//! failure modes with specialized mock providers.
//!
//! Mock providers:
//! - `FlakeyProvider` -- Fails N times, then succeeds
//! - `HangingProvider` -- Hangs forever (tests caller-side timeout)
//! - `GarbageProvider` -- Returns valid response structure with garbage content

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rust_decimal::Decimal;

use ironclaw::error::LlmError;
use ironclaw::llm::{
    ChatMessage, CircuitBreakerConfig, CircuitBreakerProvider, CompletionRequest,
    CompletionResponse, CooldownConfig, FailoverProvider, FinishReason, LlmProvider, RetryConfig,
    RetryProvider, ToolCompletionRequest, ToolCompletionResponse,
};

// ---------------------------------------------------------------------------
// Mock providers
// ---------------------------------------------------------------------------

/// Provider that fails N times then succeeds.
///
/// Thread-safe: uses atomic counter so it works correctly across retries
/// and concurrent access.
struct FlakeyProvider {
    failures_remaining: AtomicU32,
    success_response: String,
    name: String,
    call_count: AtomicU32,
}

impl FlakeyProvider {
    fn new(failures: u32, response: impl Into<String>) -> Self {
        Self {
            failures_remaining: AtomicU32::new(failures),
            success_response: response.into(),
            name: "flakey".to_string(),
            call_count: AtomicU32::new(0),
        }
    }

    fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    fn calls(&self) -> u32 {
        self.call_count.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl LlmProvider for FlakeyProvider {
    fn model_name(&self) -> &str {
        &self.name
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        (Decimal::ZERO, Decimal::ZERO)
    }

    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        self.call_count.fetch_add(1, Ordering::Relaxed);
        let prev = self.failures_remaining.load(Ordering::Relaxed);
        if prev > 0 {
            // Attempt to decrement; if another thread decremented first, that's fine.
            let _ = self.failures_remaining.compare_exchange(
                prev,
                prev - 1,
                Ordering::Relaxed,
                Ordering::Relaxed,
            );
            return Err(LlmError::RequestFailed {
                provider: self.name.clone(),
                reason: format!("transient failure ({} remaining)", prev - 1),
            });
        }
        Ok(CompletionResponse {
            content: self.success_response.clone(),
            input_tokens: 10,
            output_tokens: 5,
            finish_reason: FinishReason::Stop,
        })
    }

    async fn complete_with_tools(
        &self,
        _request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        self.call_count.fetch_add(1, Ordering::Relaxed);
        let prev = self.failures_remaining.load(Ordering::Relaxed);
        if prev > 0 {
            let _ = self.failures_remaining.compare_exchange(
                prev,
                prev - 1,
                Ordering::Relaxed,
                Ordering::Relaxed,
            );
            return Err(LlmError::RequestFailed {
                provider: self.name.clone(),
                reason: format!("transient failure ({} remaining)", prev - 1),
            });
        }
        Ok(ToolCompletionResponse {
            content: Some(self.success_response.clone()),
            tool_calls: vec![],
            input_tokens: 10,
            output_tokens: 5,
            finish_reason: FinishReason::Stop,
        })
    }
}

/// Provider that hangs forever (tests timeout handling at the caller).
struct HangingProvider {
    name: String,
}

impl HangingProvider {
    fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

#[async_trait]
impl LlmProvider for HangingProvider {
    fn model_name(&self) -> &str {
        &self.name
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        (Decimal::ZERO, Decimal::ZERO)
    }

    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        // Hang forever -- callers must use tokio::time::timeout.
        std::future::pending().await
    }

    async fn complete_with_tools(
        &self,
        _request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        std::future::pending().await
    }
}

/// Provider that returns valid response structures but with garbage content.
///
/// This tests that the system handles "technically valid but semantically
/// nonsensical" responses gracefully.
struct GarbageProvider {
    name: String,
    call_count: AtomicU32,
}

impl GarbageProvider {
    fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            call_count: AtomicU32::new(0),
        }
    }

    fn calls(&self) -> u32 {
        self.call_count.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl LlmProvider for GarbageProvider {
    fn model_name(&self) -> &str {
        &self.name
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        (Decimal::ZERO, Decimal::ZERO)
    }

    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        self.call_count.fetch_add(1, Ordering::Relaxed);
        Ok(CompletionResponse {
            content: "\x00\x01\x02\x7f garbage \u{FFFD} response".to_string(),
            input_tokens: 0,
            output_tokens: 0,
            finish_reason: FinishReason::Unknown,
        })
    }

    async fn complete_with_tools(
        &self,
        _request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        self.call_count.fetch_add(1, Ordering::Relaxed);
        Ok(ToolCompletionResponse {
            content: Some(String::new()), // empty content
            tool_calls: vec![],
            input_tokens: 0,
            output_tokens: 0,
            finish_reason: FinishReason::Unknown,
        })
    }
}

/// Simple always-ok provider for use as a reliable fallback in tests.
struct ReliableProvider {
    name: String,
    response: String,
    call_count: AtomicU32,
}

impl ReliableProvider {
    fn new(name: impl Into<String>, response: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            response: response.into(),
            call_count: AtomicU32::new(0),
        }
    }

    fn calls(&self) -> u32 {
        self.call_count.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl LlmProvider for ReliableProvider {
    fn model_name(&self) -> &str {
        &self.name
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        (Decimal::ZERO, Decimal::ZERO)
    }

    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        self.call_count.fetch_add(1, Ordering::Relaxed);
        Ok(CompletionResponse {
            content: self.response.clone(),
            input_tokens: 10,
            output_tokens: 5,
            finish_reason: FinishReason::Stop,
        })
    }

    async fn complete_with_tools(
        &self,
        _request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        self.call_count.fetch_add(1, Ordering::Relaxed);
        Ok(ToolCompletionResponse {
            content: Some(self.response.clone()),
            tool_calls: vec![],
            input_tokens: 10,
            output_tokens: 5,
            finish_reason: FinishReason::Stop,
        })
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_request() -> CompletionRequest {
    CompletionRequest::new(vec![ChatMessage::user("hello")])
}

fn make_tool_request() -> ToolCompletionRequest {
    ToolCompletionRequest::new(vec![ChatMessage::user("hello")], vec![])
}

// ---------------------------------------------------------------------------
// Test: FlakeyProvider eventually succeeds through RetryProvider
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_flakey_provider_eventually_succeeds() {
    // FlakeyProvider fails 3 times then succeeds.
    // RetryProvider with max_retries=5 should be enough to get through.
    let flakey = Arc::new(FlakeyProvider::new(3, "success after retries"));
    let retry = RetryProvider::new(flakey.clone(), RetryConfig { max_retries: 5 });

    let result = tokio::time::timeout(Duration::from_secs(30), retry.complete(make_request()))
        .await
        .expect("should not timeout with 30s budget");

    let response = result.expect("should succeed after retries");
    assert_eq!(response.content, "success after retries");
    // Should have been called 4 times: 3 failures + 1 success
    assert_eq!(flakey.calls(), 4, "expected 3 failures + 1 success = 4 calls");
}

/// Verify that a FlakeyProvider with more failures than retries exhausts
/// retries and returns an error.
#[tokio::test]
async fn test_flakey_provider_exhausts_retries() {
    // Fails 10 times, but retry allows only 2 retries (3 attempts total).
    let flakey = Arc::new(FlakeyProvider::new(10, "never reached"));
    let retry = RetryProvider::new(flakey.clone(), RetryConfig { max_retries: 2 });

    let result = retry.complete(make_request()).await;
    assert!(result.is_err(), "should fail when retries are exhausted");
    // 3 total attempts: initial + 2 retries
    assert_eq!(flakey.calls(), 3);
}

// ---------------------------------------------------------------------------
// Test: HangingProvider times out with tokio::time::timeout
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_hanging_provider_times_out() {
    let hanging: Arc<dyn LlmProvider> = Arc::new(HangingProvider::new("hanging-provider"));

    let result =
        tokio::time::timeout(Duration::from_millis(200), hanging.complete(make_request())).await;

    // Should be a timeout error, not hang forever.
    assert!(
        result.is_err(),
        "HangingProvider should timeout, not hang forever"
    );
}

/// HangingProvider behind a CircuitBreakerProvider can still be timed out.
#[tokio::test]
async fn test_hanging_provider_behind_circuit_breaker_times_out() {
    let hanging: Arc<dyn LlmProvider> = Arc::new(HangingProvider::new("hanging-behind-cb"));
    let cb = CircuitBreakerProvider::new(
        hanging,
        CircuitBreakerConfig {
            failure_threshold: 3,
            recovery_timeout: Duration::from_secs(30),
            half_open_successes_needed: 1,
        },
    );

    let result =
        tokio::time::timeout(Duration::from_millis(200), cb.complete(make_request())).await;

    assert!(
        result.is_err(),
        "should timeout even when wrapped in circuit breaker"
    );
}

/// complete_with_tools also hangs and can be timed out.
#[tokio::test]
async fn test_hanging_provider_complete_with_tools_times_out() {
    let hanging: Arc<dyn LlmProvider> = Arc::new(HangingProvider::new("hanging-tools"));

    let result = tokio::time::timeout(
        Duration::from_millis(200),
        hanging.complete_with_tools(make_tool_request()),
    )
    .await;

    assert!(result.is_err(), "complete_with_tools should also timeout");
}

// ---------------------------------------------------------------------------
// Test: GarbageProvider returns valid response with garbage content
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_garbage_provider_returns_error_or_empty() {
    let garbage = Arc::new(GarbageProvider::new("garbage-provider"));

    // complete() returns a valid CompletionResponse with garbage content.
    let response = garbage
        .complete(make_request())
        .await
        .expect("garbage provider should not return an error");

    // The response is structurally valid but the content is nonsensical.
    assert!(
        !response.content.is_empty(),
        "garbage content should be non-empty"
    );
    assert_eq!(
        response.finish_reason,
        FinishReason::Unknown,
        "garbage response has Unknown finish reason"
    );
    assert_eq!(response.input_tokens, 0);
    assert_eq!(response.output_tokens, 0);

    // complete_with_tools() returns empty content.
    let tool_response = garbage
        .complete_with_tools(make_tool_request())
        .await
        .expect("garbage provider tool completion should not error");

    assert_eq!(
        tool_response.content,
        Some(String::new()),
        "tool response should have empty content"
    );
    assert!(tool_response.tool_calls.is_empty());
    assert_eq!(garbage.calls(), 2, "should have recorded 2 calls total");
}

/// GarbageProvider is not retried by RetryProvider since it returns Ok.
#[tokio::test]
async fn test_garbage_provider_not_retried() {
    let garbage = Arc::new(GarbageProvider::new("garbage-no-retry"));
    let retry = RetryProvider::new(garbage.clone(), RetryConfig { max_retries: 3 });

    let response = retry.complete(make_request()).await;
    assert!(response.is_ok(), "garbage Ok response should pass through");
    assert_eq!(garbage.calls(), 1, "should only call once -- no retry on Ok");
}

// ---------------------------------------------------------------------------
// Test: Circuit breaker trips and recovers
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_circuit_breaker_trips_and_recovers() {
    // Use a FlakeyProvider that fails 5 times then succeeds.
    let flakey = Arc::new(FlakeyProvider::new(5, "recovered"));
    let cb = CircuitBreakerProvider::new(
        flakey.clone(),
        CircuitBreakerConfig {
            failure_threshold: 3,
            recovery_timeout: Duration::from_millis(50),
            half_open_successes_needed: 1,
        },
    );

    // Send 3 failures to trip the breaker.
    for _ in 0..3 {
        let _ = cb.complete(make_request()).await;
    }

    // Circuit should now be open.
    let state = cb.circuit_state().await;
    assert_eq!(
        state,
        ironclaw::llm::circuit_breaker::CircuitState::Open,
        "circuit should be open after 3 failures"
    );

    // Requests while open should be rejected immediately with a circuit breaker message.
    let err = cb.complete(make_request()).await.unwrap_err();
    match &err {
        LlmError::RequestFailed { reason, .. } => {
            assert!(
                reason.contains("Circuit breaker open"),
                "expected circuit breaker message, got: {}",
                reason
            );
        }
        other => panic!("expected RequestFailed, got: {:?}", other),
    }

    // Wait for recovery timeout.
    tokio::time::sleep(Duration::from_millis(60)).await;

    // The FlakeyProvider still has 2 failures remaining (5 - 3 = 2).
    // The first probe (half-open) will fail, sending it back to open.
    let _ = cb.complete(make_request()).await;
    assert_eq!(
        cb.circuit_state().await,
        ironclaw::llm::circuit_breaker::CircuitState::Open,
        "probe failed, should reopen"
    );

    // Wait again for recovery.
    tokio::time::sleep(Duration::from_millis(60)).await;

    // Second probe: FlakeyProvider has 1 failure remaining.
    let _ = cb.complete(make_request()).await;
    assert_eq!(
        cb.circuit_state().await,
        ironclaw::llm::circuit_breaker::CircuitState::Open,
        "still one failure left, should reopen again"
    );

    // Wait once more.
    tokio::time::sleep(Duration::from_millis(60)).await;

    // Third probe: FlakeyProvider should now succeed (all 5 failures consumed).
    let result = cb.complete(make_request()).await;
    assert!(result.is_ok(), "should succeed after all failures consumed");
    assert_eq!(result.unwrap().content, "recovered");
    assert_eq!(
        cb.circuit_state().await,
        ironclaw::llm::circuit_breaker::CircuitState::Closed,
        "circuit should close after successful probe"
    );
}

// ---------------------------------------------------------------------------
// Test: Failover chain under chaos
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_failover_chain_under_chaos() {
    // First provider is flakey (fails 3 times), second is reliable.
    // FailoverProvider should fall back to the reliable one on failures
    // from the flakey provider, then route back to flakey once it recovers.
    //
    // Use a high cooldown threshold (100) so the flakey provider doesn't
    // enter cooldown during this test -- we want to test pure failover
    // behavior, not cooldown.
    let flakey: Arc<dyn LlmProvider> =
        Arc::new(FlakeyProvider::new(3, "flakey recovered").with_name("flakey-primary"));
    let reliable: Arc<dyn LlmProvider> =
        Arc::new(ReliableProvider::new("reliable-backup", "backup response"));

    let config = CooldownConfig {
        cooldown_duration: Duration::from_secs(300),
        failure_threshold: 100, // high threshold: no cooldown during this test
    };
    let failover =
        FailoverProvider::with_cooldown(vec![flakey.clone(), reliable.clone()], config)
            .expect("should create failover with 2 providers");

    // Request 1: flakey fails, reliable succeeds.
    let r = failover.complete(make_request()).await.unwrap();
    assert_eq!(r.content, "backup response");

    // Request 2: flakey fails again, reliable succeeds.
    let r = failover.complete(make_request()).await.unwrap();
    assert_eq!(r.content, "backup response");

    // Request 3: flakey fails (third failure), reliable succeeds.
    let r = failover.complete(make_request()).await.unwrap();
    assert_eq!(r.content, "backup response");

    // Request 4: flakey should now succeed (all 3 failures consumed).
    let r = failover.complete(make_request()).await.unwrap();
    assert_eq!(r.content, "flakey recovered");
}

/// Failover with cooldown: flakey provider enters cooldown, backup serves,
/// then flakey recovers after cooldown expires.
#[tokio::test]
async fn test_failover_cooldown_with_flakey_provider() {
    let flakey: Arc<dyn LlmProvider> =
        Arc::new(FlakeyProvider::new(3, "flakey back").with_name("flakey-cd"));
    let reliable: Arc<dyn LlmProvider> =
        Arc::new(ReliableProvider::new("reliable-cd", "reliable"));

    let config = CooldownConfig {
        cooldown_duration: Duration::from_millis(50),
        failure_threshold: 2,
    };
    let failover =
        FailoverProvider::with_cooldown(vec![flakey.clone(), reliable.clone()], config)
            .expect("should create failover with cooldown");

    // Requests 1-2: flakey fails twice, reaching cooldown threshold.
    let r = failover.complete(make_request()).await.unwrap();
    assert_eq!(r.content, "reliable");
    let r = failover.complete(make_request()).await.unwrap();
    assert_eq!(r.content, "reliable");

    // Request 3: flakey should be in cooldown, only reliable called.
    // (flakey's 3rd failure would be consumed if called, but it's skipped.)
    let r = failover.complete(make_request()).await.unwrap();
    assert_eq!(r.content, "reliable");

    // Wait for cooldown to expire, then flakey gets retried.
    tokio::time::sleep(Duration::from_millis(60)).await;

    // After cooldown: flakey is tried again. It still has 1 failure remaining.
    let r = failover.complete(make_request()).await.unwrap();
    // Flakey fails again (3rd failure consumed), reliable serves.
    assert_eq!(r.content, "reliable");

    // Wait again for cooldown.
    tokio::time::sleep(Duration::from_millis(60)).await;

    // Now flakey should succeed (all 3 failures consumed).
    let r = failover.complete(make_request()).await.unwrap();
    assert_eq!(r.content, "flakey back");
}

/// Three providers: first always fails, second is flakey, third is reliable.
/// Tests cascading failover through multiple providers.
#[tokio::test]
async fn test_failover_three_provider_cascade() {
    let always_fail: Arc<dyn LlmProvider> =
        Arc::new(FlakeyProvider::new(u32::MAX, "unreachable").with_name("always-fail"));
    let flakey: Arc<dyn LlmProvider> =
        Arc::new(FlakeyProvider::new(2, "flakey ok").with_name("flakey-middle"));
    let reliable: Arc<dyn LlmProvider> =
        Arc::new(ReliableProvider::new("reliable-last", "last resort"));

    let failover =
        FailoverProvider::new(vec![always_fail, flakey.clone(), reliable.clone()])
            .expect("three providers");

    // Request 1: always-fail fails, flakey fails (1st), reliable serves.
    let r = failover.complete(make_request()).await.unwrap();
    assert_eq!(r.content, "last resort");

    // Request 2: always-fail fails, flakey fails (2nd), reliable serves.
    let r = failover.complete(make_request()).await.unwrap();
    assert_eq!(r.content, "last resort");

    // Request 3: always-fail fails, flakey now succeeds.
    let r = failover.complete(make_request()).await.unwrap();
    assert_eq!(r.content, "flakey ok");
}

/// Failover with a mix of transient and non-transient errors.
/// Non-transient error from primary should propagate immediately.
#[tokio::test]
async fn test_failover_non_transient_stops_chain() {
    // Provider that returns a non-transient error.
    struct NonTransientProvider;

    #[async_trait]
    impl LlmProvider for NonTransientProvider {
        fn model_name(&self) -> &str {
            "non-transient"
        }
        fn cost_per_token(&self) -> (Decimal, Decimal) {
            (Decimal::ZERO, Decimal::ZERO)
        }
        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            Err(LlmError::ContextLengthExceeded {
                used: 200_000,
                limit: 100_000,
            })
        }
        async fn complete_with_tools(
            &self,
            _request: ToolCompletionRequest,
        ) -> Result<ToolCompletionResponse, LlmError> {
            Err(LlmError::ContextLengthExceeded {
                used: 200_000,
                limit: 100_000,
            })
        }
    }

    let primary: Arc<dyn LlmProvider> = Arc::new(NonTransientProvider);
    let backup = Arc::new(ReliableProvider::new("backup", "should not reach"));

    let failover =
        FailoverProvider::new(vec![primary, backup.clone() as Arc<dyn LlmProvider>])
            .expect("failover");

    let err = failover.complete(make_request()).await.unwrap_err();
    assert!(
        matches!(err, LlmError::ContextLengthExceeded { .. }),
        "non-transient error should propagate: {:?}",
        err
    );
    // Backup should never have been called.
    assert_eq!(backup.calls(), 0, "backup should not be called for non-transient errors");
}

/// Full stack: RetryProvider wrapping FlakeyProvider, behind a
/// CircuitBreakerProvider. Verifies the full chain works together.
#[tokio::test]
async fn test_retry_plus_circuit_breaker_integration() {
    // Flakey provider that fails 2 times then succeeds.
    let flakey = Arc::new(FlakeyProvider::new(2, "stack success"));
    let retry: Arc<dyn LlmProvider> = Arc::new(RetryProvider::new(
        flakey.clone(),
        RetryConfig { max_retries: 3 },
    ));
    let cb = CircuitBreakerProvider::new(
        retry,
        CircuitBreakerConfig {
            failure_threshold: 10, // high threshold so we don't trip
            recovery_timeout: Duration::from_secs(30),
            half_open_successes_needed: 1,
        },
    );

    let result = tokio::time::timeout(Duration::from_secs(30), cb.complete(make_request()))
        .await
        .expect("should not timeout");

    let response = result.expect("retry+CB stack should succeed");
    assert_eq!(response.content, "stack success");
    assert_eq!(
        cb.circuit_state().await,
        ironclaw::llm::circuit_breaker::CircuitState::Closed,
        "circuit should remain closed"
    );
}

/// Full chain: RetryProvider -> FailoverProvider -> CircuitBreakerProvider.
/// Primary is flakey with insufficient retries to recover; failover catches it.
#[tokio::test]
async fn test_full_chain_retry_failover_circuit_breaker() {
    // Primary: flakey, fails 5 times. Retry allows 2 retries (3 attempts).
    // After retry exhaustion, failover should kick in to the reliable backup.
    let flakey = Arc::new(FlakeyProvider::new(5, "not reachable").with_name("flakey-full"));
    let retry_primary: Arc<dyn LlmProvider> = Arc::new(RetryProvider::new(
        flakey.clone(),
        RetryConfig { max_retries: 2 },
    ));

    // Backup: always reliable.
    let reliable: Arc<dyn LlmProvider> =
        Arc::new(ReliableProvider::new("reliable-full", "backup ok"));

    // Failover wraps both.
    let failover: Arc<dyn LlmProvider> = Arc::new(
        FailoverProvider::new(vec![retry_primary, reliable.clone()])
            .expect("failover"),
    );

    // Circuit breaker on top.
    let cb = CircuitBreakerProvider::new(
        failover,
        CircuitBreakerConfig {
            failure_threshold: 10,
            recovery_timeout: Duration::from_secs(30),
            half_open_successes_needed: 1,
        },
    );

    let result = tokio::time::timeout(Duration::from_secs(30), cb.complete(make_request()))
        .await
        .expect("should not timeout");

    let response = result.expect("full chain should succeed via failover");
    assert_eq!(response.content, "backup ok");
}

/// Verify that GarbageProvider content flows through the full decorator chain
/// without causing panics or unexpected errors.
#[tokio::test]
async fn test_garbage_through_full_chain() {
    let garbage: Arc<dyn LlmProvider> = Arc::new(GarbageProvider::new("garbage-chain"));
    let retry: Arc<dyn LlmProvider> = Arc::new(RetryProvider::new(
        garbage.clone(),
        RetryConfig { max_retries: 1 },
    ));
    let cb = CircuitBreakerProvider::new(
        retry,
        CircuitBreakerConfig {
            failure_threshold: 5,
            recovery_timeout: Duration::from_secs(30),
            half_open_successes_needed: 1,
        },
    );

    let result = cb.complete(make_request()).await;
    assert!(result.is_ok(), "garbage should flow through without error");

    let response = result.unwrap();
    assert!(
        response.content.contains("garbage"),
        "garbage content should be preserved"
    );
    assert_eq!(
        cb.circuit_state().await,
        ironclaw::llm::circuit_breaker::CircuitState::Closed,
        "Ok responses should not trip the breaker"
    );
}
