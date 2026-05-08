//! Test helpers for `ironclaw_llm`.
//!
//! Gated behind the `testing` feature (or `cfg(test)`). Downstream test code
//! can opt in by depending on this crate with `features = ["testing"]`.

pub mod fault_injection;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use async_trait::async_trait;
use rust_decimal::Decimal;

use crate::error::LlmError;
use crate::provider::{
    CompletionRequest, CompletionResponse, FinishReason, LlmProvider, ToolCompletionRequest,
    ToolCompletionResponse,
};

// ── Session test constants ──────────────────────────────────────────────

/// Generic session token for persistence tests.
pub const TEST_SESSION_TOKEN: &str = "test_token_123";

/// NEAR AI session token variant A.
pub const TEST_SESSION_NEARAI_ABC: &str = "sess_abc123";

/// NEAR AI session token variant B.
pub const TEST_SESSION_NEARAI_XYZ: &str = "sess_xyz789";

// ── StubLlm ─────────────────────────────────────────────────────────────

/// What kind of error the stub should produce when failing.
#[derive(Clone, Copy, Debug)]
pub enum StubErrorKind {
    /// Transient/retryable error (`LlmError::RequestFailed`).
    Transient,
    /// Non-transient error (`LlmError::ContextLengthExceeded`).
    NonTransient,
}

/// A configurable LLM provider stub for tests.
///
/// Supports:
/// - Fixed response content
/// - Call counting via [`calls()`](Self::calls)
/// - Runtime failure toggling via [`set_failing()`](Self::set_failing)
/// - Configurable error kinds (transient vs non-transient)
///
/// Use this in tests instead of creating ad-hoc stub implementations.
pub struct StubLlm {
    model_name: String,
    response: String,
    call_count: AtomicU32,
    should_fail: AtomicBool,
    error_kind: StubErrorKind,
    /// Optional fault injector for fine-grained failure control.
    /// When set, takes precedence over the `should_fail` / `error_kind` fields.
    fault_injector: Option<Arc<fault_injection::FaultInjector>>,
}

impl StubLlm {
    /// Create a new stub that returns the given response.
    pub fn new(response: impl Into<String>) -> Self {
        Self {
            model_name: "stub-model".to_string(),
            response: response.into(),
            call_count: AtomicU32::new(0),
            should_fail: AtomicBool::new(false),
            error_kind: StubErrorKind::Transient,
            fault_injector: None,
        }
    }

    /// Create a stub that always fails with a transient error.
    pub fn failing(name: impl Into<String>) -> Self {
        Self {
            model_name: name.into(),
            response: String::new(),
            call_count: AtomicU32::new(0),
            should_fail: AtomicBool::new(true),
            error_kind: StubErrorKind::Transient,
            fault_injector: None,
        }
    }

    /// Create a stub that always fails with a non-transient error.
    pub fn failing_non_transient(name: impl Into<String>) -> Self {
        Self {
            model_name: name.into(),
            response: String::new(),
            call_count: AtomicU32::new(0),
            should_fail: AtomicBool::new(true),
            error_kind: StubErrorKind::NonTransient,
            fault_injector: None,
        }
    }

    /// Set the model name.
    pub fn with_model_name(mut self, name: impl Into<String>) -> Self {
        self.model_name = name.into();
        self
    }

    /// Get the number of times `complete` or `complete_with_tools` was called.
    pub fn calls(&self) -> u32 {
        self.call_count.load(Ordering::Relaxed)
    }

    /// Attach a fault injector for fine-grained failure control.
    ///
    /// When set, the injector's `next_action()` is consulted on every call,
    /// taking precedence over the `should_fail` / `error_kind` fields.
    pub fn with_fault_injector(mut self, injector: Arc<fault_injection::FaultInjector>) -> Self {
        self.fault_injector = Some(injector);
        self
    }

    /// Toggle whether calls should fail at runtime.
    pub fn set_failing(&self, fail: bool) {
        self.should_fail.store(fail, Ordering::Relaxed);
    }

    /// Check the fault injector or should_fail flag, returning an error if
    /// the call should fail, or None if it should succeed.
    async fn check_faults(&self) -> Option<LlmError> {
        if let Some(ref injector) = self.fault_injector {
            match injector.next_action() {
                fault_injection::FaultAction::Fail(fault) => {
                    return Some(fault.to_llm_error(&self.model_name));
                }
                fault_injection::FaultAction::Delay(duration) => {
                    tokio::time::sleep(duration).await;
                }
                fault_injection::FaultAction::Succeed => {}
            }
        } else if self.should_fail.load(Ordering::Relaxed) {
            return Some(self.make_error());
        }
        None
    }

    fn make_error(&self) -> LlmError {
        match self.error_kind {
            StubErrorKind::Transient => LlmError::RequestFailed {
                provider: self.model_name.clone(),
                reason: "server error".to_string(),
            },
            StubErrorKind::NonTransient => LlmError::ContextLengthExceeded {
                used: 100_000,
                limit: 50_000,
            },
        }
    }
}

impl Default for StubLlm {
    fn default() -> Self {
        Self::new("OK")
    }
}

#[async_trait]
impl LlmProvider for StubLlm {
    fn model_name(&self) -> &str {
        &self.model_name
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        (Decimal::ZERO, Decimal::ZERO)
    }

    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        self.call_count.fetch_add(1, Ordering::Relaxed);
        if let Some(err) = self.check_faults().await {
            return Err(err);
        }
        Ok(CompletionResponse {
            content: self.response.clone(),
            input_tokens: 10,
            output_tokens: 5,
            finish_reason: FinishReason::Stop,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
        })
    }

    async fn complete_with_tools(
        &self,
        _request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        self.call_count.fetch_add(1, Ordering::Relaxed);
        if let Some(err) = self.check_faults().await {
            return Err(err);
        }
        Ok(ToolCompletionResponse {
            content: Some(self.response.clone()),
            tool_calls: Vec::new(),
            input_tokens: 10,
            output_tokens: 5,
            finish_reason: FinishReason::Stop,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
            reasoning: None,
        })
    }
}
