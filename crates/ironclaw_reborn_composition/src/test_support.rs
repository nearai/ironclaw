//! Test-only helpers for driving budget E2E tests against
//! [`build_reborn_runtime`].
//!
//! Gated behind the `test-support` feature so production builds never pay
//! the cost of the mock gateway / introspection accessors. The shapes here
//! are deliberately small: a mock [`HostManagedModelGateway`] with
//! per-turn scripted responses (including token usage), and the public
//! `with_model_gateway_override` test hook that mirrors the crate-private
//! one used inside `runtime.rs`.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_loop_support::{
    HostManagedModelError, HostManagedModelErrorKind, HostManagedModelGateway,
    HostManagedModelRequest, HostManagedModelResponse,
};
use ironclaw_turns::run_profile::{LoopCapabilityPort, LoopModelUsage};

use crate::runtime_input::RebornRuntimeInput;

/// One scripted reply from the mock LLM.
///
/// `usage` is forwarded into [`HostManagedModelResponse::usage`] so the
/// budget accountant reconciles against real provider numbers, not the
/// reservation estimate.
#[derive(Debug, Clone)]
pub struct ScriptedReply {
    pub text: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
}

impl ScriptedReply {
    pub fn new(text: impl Into<String>, input_tokens: u32, output_tokens: u32) -> Self {
        Self {
            text: text.into(),
            input_tokens,
            output_tokens,
        }
    }

    fn into_response(self) -> HostManagedModelResponse {
        HostManagedModelResponse::assistant_reply(self.text).with_usage(LoopModelUsage {
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
        })
    }
}

/// Mock [`HostManagedModelGateway`] that returns scripted assistant
/// replies with configurable token usage.
///
/// Replies are consumed in FIFO order. When the script runs out the
/// gateway falls back to a sentinel reply with zero tokens — tests that
/// drive multiple turns should pre-load the matching number of
/// [`ScriptedReply`] entries.
///
/// Every `stream_model` call is recorded so tests can assert the call
/// count after the run completes.
#[derive(Debug, Default)]
pub struct BudgetTestGateway {
    replies: Mutex<Vec<ScriptedReply>>,
    fallback: Option<ScriptedReply>,
    calls: Mutex<Vec<HostManagedModelRequest>>,
}

impl BudgetTestGateway {
    pub fn new() -> Self {
        Self::default()
    }

    /// Single-reply convenience: every model call returns the same
    /// assistant text with the given token counts.
    pub fn with_constant(text: impl Into<String>, input_tokens: u32, output_tokens: u32) -> Self {
        Self {
            replies: Mutex::new(Vec::new()),
            fallback: Some(ScriptedReply::new(text, input_tokens, output_tokens)),
            calls: Mutex::new(Vec::new()),
        }
    }

    /// Push one scripted reply. Replies are consumed in FIFO order.
    pub fn push(&self, reply: ScriptedReply) {
        self.replies
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(reply);
    }

    /// Number of `stream_model` calls observed so far.
    pub fn call_count(&self) -> usize {
        self.calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    fn next_reply(&self) -> ScriptedReply {
        let mut script = self
            .replies
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if script.is_empty() {
            return self
                .fallback
                .clone()
                .unwrap_or_else(|| ScriptedReply::new("budget test fallback reply", 0, 0));
        }
        script.remove(0)
    }
}

#[async_trait]
impl HostManagedModelGateway for BudgetTestGateway {
    async fn stream_model(
        &self,
        request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        self.calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(request);
        Ok(self.next_reply().into_response())
    }

    async fn stream_model_with_capabilities(
        &self,
        request: HostManagedModelRequest,
        _capabilities: Arc<dyn LoopCapabilityPort>,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        // The budget tests don't need capability dispatch — fall through
        // to the plain stream path. If a future test needs tool calls,
        // extend this with a separate scripted-tool-call queue.
        self.calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(request);
        Ok(self.next_reply().into_response())
    }
}

/// Mock gateway that always fails with the given error kind. Useful for
/// driving the cancellation / provider-error paths in budget tests
/// without depending on tokio cancel semantics.
#[derive(Debug)]
pub struct FailingTestGateway {
    pub kind: HostManagedModelErrorKind,
    pub summary: String,
}

impl FailingTestGateway {
    pub fn new(kind: HostManagedModelErrorKind, summary: impl Into<String>) -> Self {
        Self {
            kind,
            summary: summary.into(),
        }
    }
}

#[async_trait]
impl HostManagedModelGateway for FailingTestGateway {
    async fn stream_model(
        &self,
        _request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        Err(HostManagedModelError::safe(self.kind, self.summary.clone()))
    }
}

/// Extension trait that gives integration tests the same crate-private
/// `with_model_gateway_override` hook the in-crate tests use, without
/// promoting the field-level setter to `pub` outside `test-support`.
pub trait RebornRuntimeInputTestExt {
    /// Inject a stub `HostManagedModelGateway` (typically a
    /// [`BudgetTestGateway`]) so `send_user_message` flows through it
    /// instead of the LLM-backed gateway.
    fn with_test_model_gateway(self, gateway: Arc<dyn HostManagedModelGateway>) -> Self;

    /// Pair the test gateway with a deterministic cost table so the
    /// budget accountant reconciles real per-token prices on every
    /// `post_model_call`. Without this, gateway overrides produce no
    /// accountant and budget tests can't assert ledger state.
    fn with_test_model_cost_table(
        self,
        cost_table: Arc<dyn ironclaw_loop_support::ModelCostTable>,
    ) -> Self;
}

impl RebornRuntimeInputTestExt for RebornRuntimeInput {
    fn with_test_model_gateway(self, gateway: Arc<dyn HostManagedModelGateway>) -> Self {
        // The crate-private setter is only `pub(crate)`; we forward through
        // the public test-support surface so integration tests do not need
        // to reach into the runtime's internal fields directly.
        self.with_model_gateway_override_for_tests(gateway)
    }

    fn with_test_model_cost_table(
        self,
        cost_table: Arc<dyn ironclaw_loop_support::ModelCostTable>,
    ) -> Self {
        self.with_model_cost_table_override_for_tests(cost_table)
    }
}
