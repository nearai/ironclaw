//! Circuit breaker for LLM providers.
//!
//! Wraps any `LlmProvider` with a state machine that trips open after
//! consecutive transient failures, preventing request storms against a
//! degraded backend. Automatically probes for recovery via half-open state.
//!
//! ```text
//!   Closed ──(failures >= threshold)──► Open
//!     ▲                                   │
//!     │                          (recovery timeout)
//!     │                                   ▼
//!     └──(probe succeeds)──── HalfOpen ──(probe fails)──► Open
//! ```

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use rust_decimal::Decimal;
use tokio::sync::Mutex;

use crate::error::LlmError;
use crate::llm::provider::{
    CompletionRequest, CompletionResponse, LlmProvider, ModelMetadata, ToolCompletionRequest,
    ToolCompletionResponse,
};

/// Configuration for the circuit breaker.
#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// Consecutive transient failures before the circuit opens.
    pub failure_threshold: u32,
    /// How long the circuit stays open before allowing a probe.
    pub recovery_timeout: Duration,
    /// Successful probes needed in half-open to close the circuit.
    pub half_open_successes_needed: u32,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            recovery_timeout: Duration::from_secs(30),
            half_open_successes_needed: 2,
        }
    }
}

/// Circuit breaker states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    /// Normal operation; tracking consecutive failures.
    Closed,
    /// Rejecting all calls; waiting for recovery timeout to elapse.
    Open,
    /// Allowing probe calls to test whether the backend recovered.
    HalfOpen,
}

/// Internal mutable state.
struct BreakerState {
    state: CircuitState,
    consecutive_failures: u32,
    opened_at: Option<Instant>,
    half_open_successes: u32,
}

impl BreakerState {
    fn new() -> Self {
        Self {
            state: CircuitState::Closed,
            consecutive_failures: 0,
            opened_at: None,
            half_open_successes: 0,
        }
    }
}

/// Wraps an `LlmProvider` with circuit breaker protection.
///
/// Tracks consecutive transient failures. After `failure_threshold` failures
/// the circuit opens and all requests are rejected for `recovery_timeout`.
/// After that timeout a probe call is allowed through (half-open); if it
/// succeeds the circuit closes, otherwise it reopens.
pub struct CircuitBreakerProvider {
    inner: Arc<dyn LlmProvider>,
    state: Mutex<BreakerState>,
    config: CircuitBreakerConfig,
}

impl CircuitBreakerProvider {
    pub fn new(inner: Arc<dyn LlmProvider>, config: CircuitBreakerConfig) -> Self {
        Self {
            inner,
            state: Mutex::new(BreakerState::new()),
            config,
        }
    }

    /// Current circuit state (for observability / health checks).
    pub async fn circuit_state(&self) -> CircuitState {
        self.state.lock().await.state
    }

    /// Number of consecutive failures recorded so far.
    pub async fn consecutive_failures(&self) -> u32 {
        self.state.lock().await.consecutive_failures
    }

    /// Pre-flight: is a call allowed right now?
    async fn check_allowed(&self) -> Result<(), LlmError> {
        let mut state = self.state.lock().await;
        match state.state {
            CircuitState::Closed | CircuitState::HalfOpen => Ok(()),
            CircuitState::Open => {
                if let Some(opened_at) = state.opened_at {
                    if opened_at.elapsed() >= self.config.recovery_timeout {
                        state.state = CircuitState::HalfOpen;
                        state.half_open_successes = 0;
                        tracing::info!(
                            provider = self.inner.model_name(),
                            "Circuit breaker: Open -> HalfOpen, allowing probe"
                        );
                        Ok(())
                    } else {
                        let remaining = self
                            .config
                            .recovery_timeout
                            .checked_sub(opened_at.elapsed())
                            .unwrap_or(Duration::ZERO);
                        Err(LlmError::RequestFailed {
                            provider: self.inner.model_name().to_string(),
                            reason: format!(
                                "Circuit breaker open ({} consecutive failures, \
                                 recovery in {:.0}s)",
                                state.consecutive_failures,
                                remaining.as_secs_f64()
                            ),
                        })
                    }
                } else {
                    // opened_at should always be Some when Open; recover gracefully
                    state.state = CircuitState::Closed;
                    Ok(())
                }
            }
        }
    }

    /// Record a successful call.
    async fn record_success(&self) {
        let mut state = self.state.lock().await;
        match state.state {
            CircuitState::Closed => {
                state.consecutive_failures = 0;
            }
            CircuitState::HalfOpen => {
                state.half_open_successes += 1;
                if state.half_open_successes >= self.config.half_open_successes_needed {
                    state.state = CircuitState::Closed;
                    state.consecutive_failures = 0;
                    state.opened_at = None;
                    tracing::info!(
                        provider = self.inner.model_name(),
                        "Circuit breaker: HalfOpen -> Closed (recovered)"
                    );
                }
            }
            CircuitState::Open => {
                // Shouldn't get here (check_allowed blocks Open), but recover
                state.state = CircuitState::Closed;
                state.consecutive_failures = 0;
                state.opened_at = None;
            }
        }
    }

    /// Record a failed call; only transient errors count toward the threshold.
    async fn record_failure(&self, err: &LlmError) {
        if !is_transient(err) {
            return;
        }

        let mut state = self.state.lock().await;
        match state.state {
            CircuitState::Closed => {
                state.consecutive_failures += 1;
                if state.consecutive_failures >= self.config.failure_threshold {
                    state.state = CircuitState::Open;
                    state.opened_at = Some(Instant::now());
                    tracing::warn!(
                        provider = self.inner.model_name(),
                        failures = state.consecutive_failures,
                        "Circuit breaker: Closed -> Open"
                    );
                }
            }
            CircuitState::HalfOpen => {
                state.state = CircuitState::Open;
                state.opened_at = Some(Instant::now());
                state.half_open_successes = 0;
                tracing::warn!(
                    provider = self.inner.model_name(),
                    "Circuit breaker: HalfOpen -> Open (probe failed)"
                );
            }
            CircuitState::Open => {}
        }
    }
}

/// Returns `true` for errors that indicate the provider is degraded
/// (server errors, rate limits, network failures, auth infrastructure down).
///
/// This answers: "should this error count toward tripping the circuit breaker?"
///
/// Includes `SessionExpired` because repeated session failures signal backend
/// auth infrastructure trouble.
///
/// Excludes client errors that are the caller's problem, not backend trouble:
/// `AuthFailed`, `ContextLengthExceeded`, `ModelNotAvailable`, `Json`.
///
/// See also `retry::is_retryable()` which answers a different question:
/// "could retrying this exact request succeed?"
fn is_transient(err: &LlmError) -> bool {
    matches!(
        err,
        LlmError::RequestFailed { .. }
            | LlmError::RateLimited { .. }
            | LlmError::InvalidResponse { .. }
            | LlmError::SessionExpired { .. }
            | LlmError::SessionRenewalFailed { .. }
            | LlmError::Http(_)
            | LlmError::Io(_)
    )
}

#[async_trait]
impl LlmProvider for CircuitBreakerProvider {
    fn model_name(&self) -> &str {
        self.inner.model_name()
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        self.inner.cost_per_token()
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        self.check_allowed().await?;
        match self.inner.complete(request).await {
            Ok(resp) => {
                self.record_success().await;
                Ok(resp)
            }
            Err(err) => {
                self.record_failure(&err).await;
                Err(err)
            }
        }
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        self.check_allowed().await?;
        match self.inner.complete_with_tools(request).await {
            Ok(resp) => {
                self.record_success().await;
                Ok(resp)
            }
            Err(err) => {
                self.record_failure(&err).await;
                Err(err)
            }
        }
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

    fn fast_config(threshold: u32) -> CircuitBreakerConfig {
        CircuitBreakerConfig {
            failure_threshold: threshold,
            recovery_timeout: Duration::from_millis(50),
            half_open_successes_needed: 1,
        }
    }

    // -- State machine tests --

    #[tokio::test]
    async fn closed_allows_calls_and_resets_on_success() {
        let stub = Arc::new(StubLlm::new("ok").with_model_name("test"));
        let cb = CircuitBreakerProvider::new(stub, fast_config(3));

        let resp = cb.complete(make_request()).await;
        assert!(resp.is_ok());
        assert_eq!(cb.circuit_state().await, CircuitState::Closed);
        assert_eq!(cb.consecutive_failures().await, 0);
    }

    #[tokio::test]
    async fn failures_accumulate_then_trip_to_open() {
        let stub = Arc::new(StubLlm::failing("test"));
        let cb = CircuitBreakerProvider::new(stub, fast_config(3));

        // First 2 failures: still closed
        for i in 0..2 {
            let _ = cb.complete(make_request()).await;
            assert_eq!(cb.circuit_state().await, CircuitState::Closed);
            assert_eq!(cb.consecutive_failures().await, i + 1);
        }

        // 3rd failure: trips to open
        let _ = cb.complete(make_request()).await;
        assert_eq!(cb.circuit_state().await, CircuitState::Open);
    }

    #[tokio::test]
    async fn open_rejects_immediately() {
        let stub = Arc::new(StubLlm::failing("test"));
        let cb = CircuitBreakerProvider::new(
            stub,
            CircuitBreakerConfig {
                failure_threshold: 1,
                recovery_timeout: Duration::from_secs(60),
                half_open_successes_needed: 1,
            },
        );

        // Trip the breaker
        let _ = cb.complete(make_request()).await;
        assert_eq!(cb.circuit_state().await, CircuitState::Open);

        // Next call should fail with circuit breaker message
        let err = cb.complete(make_request()).await.unwrap_err();
        match err {
            LlmError::RequestFailed { reason, .. } => {
                assert!(
                    reason.contains("Circuit breaker open"),
                    "Expected circuit breaker message, got: {}",
                    reason
                );
            }
            other => panic!("Expected RequestFailed, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn recovery_timeout_transitions_to_half_open() {
        let stub = Arc::new(StubLlm::failing("test"));
        let cb = CircuitBreakerProvider::new(stub, fast_config(1));

        // Trip to open
        let _ = cb.complete(make_request()).await;
        assert_eq!(cb.circuit_state().await, CircuitState::Open);

        // Wait for recovery timeout
        tokio::time::sleep(Duration::from_millis(60)).await;

        // Next call should transition to half-open (and fail, since stub fails)
        let _ = cb.complete(make_request()).await;
        // Failed probe sends it back to Open
        assert_eq!(cb.circuit_state().await, CircuitState::Open);
    }

    #[tokio::test]
    async fn half_open_success_closes_circuit() {
        let stub = Arc::new(StubLlm::failing("test"));
        let cb = CircuitBreakerProvider::new(stub.clone(), fast_config(1));

        // Trip to open
        let _ = cb.complete(make_request()).await;
        assert_eq!(cb.circuit_state().await, CircuitState::Open);

        // Wait for recovery, then make the stub succeed
        tokio::time::sleep(Duration::from_millis(60)).await;
        stub.set_failing(false);

        // Probe should succeed, closing the circuit
        let resp = cb.complete(make_request()).await;
        assert!(resp.is_ok());
        assert_eq!(cb.circuit_state().await, CircuitState::Closed);
        assert_eq!(cb.consecutive_failures().await, 0);
    }

    #[tokio::test]
    async fn half_open_failure_reopens_circuit() {
        let stub = Arc::new(StubLlm::failing("test"));
        let cb = CircuitBreakerProvider::new(stub, fast_config(1));

        // Trip to open
        let _ = cb.complete(make_request()).await;

        // Wait for recovery timeout
        tokio::time::sleep(Duration::from_millis(60)).await;

        // Probe fails (stub still failing)
        let _ = cb.complete(make_request()).await;
        assert_eq!(cb.circuit_state().await, CircuitState::Open);
    }

    #[tokio::test]
    async fn non_transient_errors_do_not_trip_breaker() {
        let stub = Arc::new(StubLlm::failing_non_transient("test"));
        let cb = CircuitBreakerProvider::new(stub, fast_config(1));

        // ContextLengthExceeded is not transient; breaker should stay closed
        for _ in 0..5 {
            let _ = cb.complete(make_request()).await;
        }
        assert_eq!(cb.circuit_state().await, CircuitState::Closed);
        assert_eq!(cb.consecutive_failures().await, 0);
    }

    #[tokio::test]
    async fn success_resets_failure_count() {
        let stub = Arc::new(StubLlm::failing("test"));
        let cb = CircuitBreakerProvider::new(stub.clone(), fast_config(3));

        // Accumulate 2 failures
        let _ = cb.complete(make_request()).await;
        let _ = cb.complete(make_request()).await;
        assert_eq!(cb.consecutive_failures().await, 2);

        // One success resets the counter
        stub.set_failing(false);
        let resp = cb.complete(make_request()).await;
        assert!(resp.is_ok());
        assert_eq!(cb.consecutive_failures().await, 0);
    }

    #[tokio::test]
    async fn complete_with_tools_uses_same_breaker_logic() {
        let stub = Arc::new(StubLlm::failing("test"));
        let cb = CircuitBreakerProvider::new(stub, fast_config(2));

        let _ = cb.complete_with_tools(make_tool_request()).await;
        let _ = cb.complete_with_tools(make_tool_request()).await;
        assert_eq!(cb.circuit_state().await, CircuitState::Open);
    }

    #[tokio::test]
    async fn multiple_half_open_successes_needed() {
        let stub = Arc::new(StubLlm::failing("test"));
        let cb = CircuitBreakerProvider::new(
            stub.clone(),
            CircuitBreakerConfig {
                failure_threshold: 1,
                recovery_timeout: Duration::from_millis(50),
                half_open_successes_needed: 3,
            },
        );

        // Trip to open
        let _ = cb.complete(make_request()).await;

        // Wait and flip to succeed
        tokio::time::sleep(Duration::from_millis(60)).await;
        stub.set_failing(false);

        // First probe: half-open, success but not enough yet
        let _ = cb.complete(make_request()).await;
        assert_eq!(cb.circuit_state().await, CircuitState::HalfOpen);

        // Second probe: still half-open
        let _ = cb.complete(make_request()).await;
        assert_eq!(cb.circuit_state().await, CircuitState::HalfOpen);

        // Third probe: closes
        let _ = cb.complete(make_request()).await;
        assert_eq!(cb.circuit_state().await, CircuitState::Closed);
    }

    // -- Error classification tests --

    #[test]
    fn transient_classification() {
        // Transient
        assert!(is_transient(&LlmError::RequestFailed {
            provider: "p".into(),
            reason: "err".into(),
        }));
        assert!(is_transient(&LlmError::RateLimited {
            provider: "p".into(),
            retry_after: None,
        }));
        assert!(is_transient(&LlmError::InvalidResponse {
            provider: "p".into(),
            reason: "bad".into(),
        }));
        assert!(is_transient(&LlmError::SessionExpired {
            provider: "p".into(),
        }));
        assert!(is_transient(&LlmError::SessionRenewalFailed {
            provider: "p".into(),
            reason: "timeout".into(),
        }));
        assert!(is_transient(&LlmError::Io(std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "reset"
        ))));

        // NOT transient
        assert!(!is_transient(&LlmError::AuthFailed {
            provider: "p".into(),
        }));
        assert!(!is_transient(&LlmError::ContextLengthExceeded {
            used: 100_000,
            limit: 50_000,
        }));
        assert!(!is_transient(&LlmError::ModelNotAvailable {
            provider: "p".into(),
            model: "m".into(),
        }));
        assert!(!is_transient(&LlmError::Json(
            serde_json::from_str::<String>("bad").unwrap_err()
        )));
    }

    // -- Passthrough delegation tests --

    #[tokio::test]
    async fn passthrough_methods_delegate_to_inner() {
        let stub = Arc::new(StubLlm::new("ok").with_model_name("my-model"));
        let cb = CircuitBreakerProvider::new(stub, fast_config(3));

        assert_eq!(cb.model_name(), "my-model");
        assert_eq!(cb.active_model_name(), "my-model");
        assert_eq!(cb.cost_per_token(), (Decimal::ZERO, Decimal::ZERO));
        assert_eq!(cb.calculate_cost(100, 50), Decimal::ZERO);
    }
}
