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
use crate::provider::{
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
    generation: u64,
    consecutive_failures: u32,
    opened_at: Option<Instant>,
    half_open_successes: u32,
}

impl BreakerState {
    fn new() -> Self {
        Self {
            state: CircuitState::Closed,
            generation: 0,
            consecutive_failures: 0,
            opened_at: None,
            half_open_successes: 0,
        }
    }

    fn transition_to(&mut self, state: CircuitState) {
        if self.state != state {
            self.state = state;
            self.generation = self.generation.wrapping_add(1);
        }
    }
}

/// Identifies the state-machine epoch in which a request was admitted.
#[derive(Debug, Clone, Copy)]
struct AdmissionToken {
    state: CircuitState,
    generation: u64,
}

impl AdmissionToken {
    fn new(state: &BreakerState) -> Self {
        Self {
            state: state.state,
            generation: state.generation,
        }
    }

    fn is_current(self, state: &BreakerState) -> bool {
        self.state == state.state && self.generation == state.generation
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
    async fn check_allowed(&self) -> Result<AdmissionToken, LlmError> {
        let mut state = self.state.lock().await;
        match state.state {
            CircuitState::Closed | CircuitState::HalfOpen => Ok(AdmissionToken::new(&state)),
            CircuitState::Open => {
                if let Some(opened_at) = state.opened_at {
                    if opened_at.elapsed() >= self.config.recovery_timeout {
                        state.transition_to(CircuitState::HalfOpen);
                        state.half_open_successes = 0;
                        tracing::info!(
                            provider = self.inner.model_name(),
                            "Circuit breaker: Open -> HalfOpen, allowing probe"
                        );
                        Ok(AdmissionToken::new(&state))
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
                    state.transition_to(CircuitState::Closed);
                    Ok(AdmissionToken::new(&state))
                }
            }
        }
    }

    /// Record a successful call.
    async fn record_success(&self, admission: AdmissionToken) {
        let mut state = self.state.lock().await;
        if !admission.is_current(&state) {
            tracing::debug!(
                provider = self.inner.model_name(),
                admitted_state = ?admission.state,
                admitted_generation = admission.generation,
                current_state = ?state.state,
                current_generation = state.generation,
                "Circuit breaker: ignoring success from a stale admission"
            );
            return;
        }

        match state.state {
            CircuitState::Closed => {
                state.consecutive_failures = 0;
            }
            CircuitState::HalfOpen => {
                state.half_open_successes += 1;
                if state.half_open_successes >= self.config.half_open_successes_needed {
                    state.transition_to(CircuitState::Closed);
                    state.consecutive_failures = 0;
                    state.opened_at = None;
                    tracing::info!(
                        provider = self.inner.model_name(),
                        "Circuit breaker: HalfOpen -> Closed (recovered)"
                    );
                }
            }
            CircuitState::Open => {}
        }
    }

    /// Record a failed call; only transient errors count toward the threshold.
    async fn record_failure(&self, admission: AdmissionToken, err: &LlmError) {
        if !is_transient(err) {
            return;
        }

        let mut state = self.state.lock().await;
        if !admission.is_current(&state) {
            tracing::debug!(
                provider = self.inner.model_name(),
                admitted_state = ?admission.state,
                admitted_generation = admission.generation,
                current_state = ?state.state,
                current_generation = state.generation,
                "Circuit breaker: ignoring failure from a stale admission"
            );
            return;
        }

        match state.state {
            CircuitState::Closed => {
                state.consecutive_failures += 1;
                if state.consecutive_failures >= self.config.failure_threshold {
                    state.transition_to(CircuitState::Open);
                    state.opened_at = Some(Instant::now());
                    tracing::warn!(
                        provider = self.inner.model_name(),
                        failures = state.consecutive_failures,
                        "Circuit breaker: Closed -> Open"
                    );
                }
            }
            CircuitState::HalfOpen => {
                state.transition_to(CircuitState::Open);
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
            | LlmError::BadGateway { .. }
            | LlmError::InvalidResponse { .. }
            | LlmError::EmptyResponse { .. }
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

    fn cache_write_multiplier(&self) -> Decimal {
        self.inner.cache_write_multiplier()
    }

    fn cache_read_discount(&self) -> Decimal {
        self.inner.cache_read_discount()
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let admission = self.check_allowed().await?;
        match self.inner.complete(request).await {
            Ok(resp) => {
                self.record_success(admission).await;
                Ok(resp)
            }
            Err(err) => {
                self.record_failure(admission, &err).await;
                Err(err)
            }
        }
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        let admission = self.check_allowed().await?;
        match self.inner.complete_with_tools(request).await {
            Ok(resp) => {
                self.record_success(admission).await;
                Ok(resp)
            }
            Err(err) => {
                self.record_failure(admission, &err).await;
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

    fn calculate_cost(&self, input_tokens: u32, output_tokens: u32) -> Decimal {
        self.inner.calculate_cost(input_tokens, output_tokens)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    use crate::testing::StubLlm;
    use tokio::sync::{Notify, mpsc};

    fn make_request() -> CompletionRequest {
        CompletionRequest::new(vec![crate::ChatMessage::user("hello")])
    }

    fn make_tool_request() -> ToolCompletionRequest {
        ToolCompletionRequest::new(vec![crate::ChatMessage::user("hello")], vec![])
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

    // === QA Plan P2 - 4.1: Provider chaos tests ===

    /// Provider that hangs forever (tests timeout handling at the caller).
    struct HangingProvider;

    #[async_trait]
    impl LlmProvider for HangingProvider {
        fn model_name(&self) -> &str {
            "hanging"
        }
        fn cost_per_token(&self) -> (Decimal, Decimal) {
            (Decimal::ZERO, Decimal::ZERO)
        }
        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            // Hang forever
            std::future::pending().await
        }
        async fn complete_with_tools(
            &self,
            _request: ToolCompletionRequest,
        ) -> Result<ToolCompletionResponse, LlmError> {
            std::future::pending().await
        }
    }

    #[tokio::test]
    async fn hanging_provider_behind_breaker_can_be_timed_out() {
        let hanging: Arc<dyn LlmProvider> = Arc::new(HangingProvider);
        let cb = CircuitBreakerProvider::new(hanging, fast_config(1));

        // The caller should be able to timeout the request.
        let result =
            tokio::time::timeout(Duration::from_millis(100), cb.complete(make_request())).await;

        // Should timeout, not hang forever.
        assert!(result.is_err(), "should timeout, not hang");
    }

    #[tokio::test]
    async fn rapid_open_close_cycles_do_not_corrupt_state() {
        let stub = Arc::new(StubLlm::failing("test"));
        let cb = CircuitBreakerProvider::new(
            stub.clone(),
            CircuitBreakerConfig {
                failure_threshold: 1,
                recovery_timeout: Duration::from_millis(10),
                half_open_successes_needed: 1,
            },
        );

        // Cycle through open/half-open/open several times.
        for _ in 0..5 {
            // Trip to open.
            let _ = cb.complete(make_request()).await;
            assert_eq!(cb.circuit_state().await, CircuitState::Open);

            // Wait for recovery.
            tokio::time::sleep(Duration::from_millis(15)).await;

            // Probe fails (stub still failing) → back to Open.
            let _ = cb.complete(make_request()).await;
            assert_eq!(cb.circuit_state().await, CircuitState::Open);
        }

        // Now flip to succeeding and verify recovery still works.
        tokio::time::sleep(Duration::from_millis(15)).await;
        stub.set_failing(false);
        let result = cb.complete(make_request()).await;
        assert!(result.is_ok());
        assert_eq!(cb.circuit_state().await, CircuitState::Closed);
    }

    struct ConcurrentOutcomeProvider {
        call_index: AtomicUsize,
        started: mpsc::UnboundedSender<usize>,
        release_failure: Notify,
        release_success: Notify,
        release_probe: Notify,
    }

    const CONCURRENT_TEST_TIMEOUT: Duration = Duration::from_secs(1);

    async fn next_started_call(started: &mut mpsc::UnboundedReceiver<usize>) -> Option<usize> {
        tokio::time::timeout(CONCURRENT_TEST_TIMEOUT, started.recv())
            .await
            .expect("provider call must start before the test timeout")
    }

    async fn finish_call(
        call: tokio::task::JoinHandle<Result<CompletionResponse, LlmError>>,
    ) -> Result<CompletionResponse, LlmError> {
        tokio::time::timeout(CONCURRENT_TEST_TIMEOUT, call)
            .await
            .expect("provider call must finish before the test timeout")
            .expect("provider call task must not panic")
    }

    #[async_trait]
    impl LlmProvider for ConcurrentOutcomeProvider {
        fn model_name(&self) -> &str {
            "concurrent-outcomes"
        }

        fn cost_per_token(&self) -> (Decimal, Decimal) {
            (Decimal::ZERO, Decimal::ZERO)
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            let call_index = self.call_index.fetch_add(1, Ordering::SeqCst);
            let _ = self.started.send(call_index);
            match call_index {
                0 => {
                    self.release_failure.notified().await;
                    Err(LlmError::RequestFailed {
                        provider: self.model_name().to_string(),
                        reason: "controlled failure".to_string(),
                    })
                }
                1 => {
                    self.release_success.notified().await;
                    Ok(CompletionResponse {
                        content: "controlled success".to_string(),
                        input_tokens: 0,
                        output_tokens: 0,
                        finish_reason: crate::FinishReason::Stop,
                        reasoning: None,
                        cache_read_input_tokens: 0,
                        cache_creation_input_tokens: 0,
                    })
                }
                2 => {
                    self.release_probe.notified().await;
                    Ok(CompletionResponse {
                        content: "recovery probe success".to_string(),
                        input_tokens: 0,
                        output_tokens: 0,
                        finish_reason: crate::FinishReason::Stop,
                        reasoning: None,
                        cache_read_input_tokens: 0,
                        cache_creation_input_tokens: 0,
                    })
                }
                other => panic!("unexpected concurrent test call {other}"),
            }
        }

        async fn complete_with_tools(
            &self,
            _request: ToolCompletionRequest,
        ) -> Result<ToolCompletionResponse, LlmError> {
            panic!("concurrent circuit-breaker regression uses complete()");
        }
    }

    #[tokio::test]
    async fn late_success_does_not_close_breaker_opened_by_concurrent_failure() {
        let (started_tx, mut started_rx) = mpsc::unbounded_channel();
        let inner = Arc::new(ConcurrentOutcomeProvider {
            call_index: AtomicUsize::new(0),
            started: started_tx,
            release_failure: Notify::new(),
            release_success: Notify::new(),
            release_probe: Notify::new(),
        });
        let cb = Arc::new(CircuitBreakerProvider::new(
            Arc::clone(&inner) as Arc<dyn LlmProvider>,
            CircuitBreakerConfig {
                failure_threshold: 1,
                recovery_timeout: Duration::from_secs(60),
                half_open_successes_needed: 1,
            },
        ));

        let first = {
            let cb = Arc::clone(&cb);
            tokio::spawn(async move { cb.complete(make_request()).await })
        };
        assert_eq!(next_started_call(&mut started_rx).await, Some(0));

        let second = {
            let cb = Arc::clone(&cb);
            tokio::spawn(async move { cb.complete(make_request()).await })
        };
        assert_eq!(next_started_call(&mut started_rx).await, Some(1));

        inner.release_failure.notify_one();
        assert!(finish_call(first).await.is_err());
        assert_eq!(cb.circuit_state().await, CircuitState::Open);

        inner.release_success.notify_one();
        assert!(finish_call(second).await.is_ok());
        assert_eq!(
            cb.circuit_state().await,
            CircuitState::Open,
            "a success admitted before the failure must not erase the open state"
        );

        let blocked = cb
            .complete(make_request())
            .await
            .expect_err("a caller must be rejected while the breaker is Open");
        assert!(
            matches!(
                blocked,
                LlmError::RequestFailed { ref reason, .. }
                    if reason.contains("Circuit breaker open")
            ),
            "caller should receive the typed circuit-breaker error, got {blocked:?}"
        );
        assert_eq!(
            inner.call_index.load(Ordering::SeqCst),
            2,
            "an Open breaker must reject the caller without invoking the provider"
        );
    }

    #[tokio::test]
    async fn stale_closed_success_does_not_count_as_half_open_recovery() {
        let (started_tx, mut started_rx) = mpsc::unbounded_channel();
        let inner = Arc::new(ConcurrentOutcomeProvider {
            call_index: AtomicUsize::new(0),
            started: started_tx,
            release_failure: Notify::new(),
            release_success: Notify::new(),
            release_probe: Notify::new(),
        });
        let cb = Arc::new(CircuitBreakerProvider::new(
            Arc::clone(&inner) as Arc<dyn LlmProvider>,
            CircuitBreakerConfig {
                failure_threshold: 1,
                recovery_timeout: Duration::ZERO,
                half_open_successes_needed: 1,
            },
        ));

        let failure = {
            let cb = Arc::clone(&cb);
            tokio::spawn(async move { cb.complete(make_request()).await })
        };
        assert_eq!(next_started_call(&mut started_rx).await, Some(0));

        let stale_success = {
            let cb = Arc::clone(&cb);
            tokio::spawn(async move { cb.complete(make_request()).await })
        };
        assert_eq!(next_started_call(&mut started_rx).await, Some(1));

        inner.release_failure.notify_one();
        assert!(finish_call(failure).await.is_err());
        assert_eq!(cb.circuit_state().await, CircuitState::Open);

        let recovery_probe = {
            let cb = Arc::clone(&cb);
            tokio::spawn(async move { cb.complete(make_request()).await })
        };
        assert_eq!(next_started_call(&mut started_rx).await, Some(2));
        assert_eq!(cb.circuit_state().await, CircuitState::HalfOpen);

        inner.release_success.notify_one();
        assert!(finish_call(stale_success).await.is_ok());
        assert_eq!(
            cb.circuit_state().await,
            CircuitState::HalfOpen,
            "only the admitted recovery probe may close the circuit"
        );

        inner.release_probe.notify_one();
        assert!(finish_call(recovery_probe).await.is_ok());
        assert_eq!(
            cb.circuit_state().await,
            CircuitState::Closed,
            "the admitted recovery probe should restore caller access"
        );
    }

    #[tokio::test]
    async fn mixed_error_types_only_transient_counts() {
        // Non-transient errors should never trip the breaker, even after many attempts.
        let non_transient = Arc::new(StubLlm::failing_non_transient("test"));
        let cb_nt = CircuitBreakerProvider::new(non_transient, fast_config(3));

        // 100 non-transient errors should not trip the breaker.
        for _ in 0..100 {
            let _ = cb_nt.complete(make_request()).await;
        }
        assert_eq!(cb_nt.circuit_state().await, CircuitState::Closed);
        assert_eq!(cb_nt.consecutive_failures().await, 0);
    }

    // === QA Plan 2.6: Edge case tests ===

    /// With a recovery_timeout of zero, the circuit should transition from
    /// Open to HalfOpen immediately on the next call (the elapsed time
    /// always >= Duration::ZERO). This verifies that zero-duration timeouts
    /// are not treated as a special "disabled" sentinel.
    #[tokio::test]
    async fn test_cooldown_at_zero_nanos() {
        let stub = Arc::new(StubLlm::failing("test"));
        let cb = CircuitBreakerProvider::new(
            stub.clone(),
            CircuitBreakerConfig {
                failure_threshold: 1,
                recovery_timeout: Duration::ZERO,
                half_open_successes_needed: 1,
            },
        );

        // Trip the breaker with one failure.
        let _ = cb.complete(make_request()).await;
        assert_eq!(cb.circuit_state().await, CircuitState::Open);

        // With recovery_timeout = 0, the very next call should transition
        // from Open -> HalfOpen immediately (no sleep needed).
        // Since the stub is still failing, the probe will fail, sending
        // it back to Open. But the key assertion is that the transition
        // to HalfOpen actually happened (not stuck in Open forever).
        stub.set_failing(false);
        let result = cb.complete(make_request()).await;
        assert!(
            result.is_ok(),
            "zero recovery_timeout should allow immediate probe"
        );
        assert_eq!(
            cb.circuit_state().await,
            CircuitState::Closed,
            "successful probe after zero-timeout should close the circuit"
        );

        // Verify it also works when the probe fails: should re-open, not
        // get stuck in some intermediate state.
        stub.set_failing(true);
        // Trip again.
        let _ = cb.complete(make_request()).await;
        assert_eq!(cb.circuit_state().await, CircuitState::Open);
        // Next call: Open -> HalfOpen (zero timeout), probe fails -> Open.
        let _ = cb.complete(make_request()).await;
        assert_eq!(
            cb.circuit_state().await,
            CircuitState::Open,
            "failed probe should re-open circuit even with zero timeout"
        );
    }

    /// When in half-open state, a single failure should immediately
    /// re-open the circuit (not close it or leave it in half-open).
    /// Also verifies that any accumulated half_open_successes are reset.
    #[tokio::test]
    async fn test_circuit_breaker_half_open_failure_reopens() {
        let stub = Arc::new(StubLlm::failing("test"));
        let cb = CircuitBreakerProvider::new(
            stub.clone(),
            CircuitBreakerConfig {
                failure_threshold: 1,
                recovery_timeout: Duration::from_millis(20),
                half_open_successes_needed: 3, // require multiple successes
            },
        );

        // Trip the breaker.
        let _ = cb.complete(make_request()).await;
        assert_eq!(cb.circuit_state().await, CircuitState::Open);

        // Wait for recovery, then succeed once to accumulate 1 half-open success.
        tokio::time::sleep(Duration::from_millis(30)).await;
        stub.set_failing(false);
        let _ = cb.complete(make_request()).await;
        // Still in half-open (need 3 successes, got 1).
        assert_eq!(cb.circuit_state().await, CircuitState::HalfOpen);

        // Now fail: should immediately re-open, discarding the 1 accumulated success.
        stub.set_failing(true);
        let _ = cb.complete(make_request()).await;
        assert_eq!(
            cb.circuit_state().await,
            CircuitState::Open,
            "failure in half-open should immediately re-open the circuit"
        );

        // After re-opening, wait for recovery and verify that the half-open
        // success counter was reset (need 3 fresh successes, not 2).
        tokio::time::sleep(Duration::from_millis(30)).await;
        stub.set_failing(false);

        // First success: half-open, count=1.
        let _ = cb.complete(make_request()).await;
        assert_eq!(cb.circuit_state().await, CircuitState::HalfOpen);

        // Second success: half-open, count=2.
        let _ = cb.complete(make_request()).await;
        assert_eq!(cb.circuit_state().await, CircuitState::HalfOpen);

        // Third success: closes the circuit.
        let _ = cb.complete(make_request()).await;
        assert_eq!(
            cb.circuit_state().await,
            CircuitState::Closed,
            "3 fresh successes needed after re-open, not 2"
        );
        assert_eq!(cb.consecutive_failures().await, 0);
    }
}
