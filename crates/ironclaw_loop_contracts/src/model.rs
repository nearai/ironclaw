//! The model-call contract: the gateway a host implements, the budget and
//! policy hooks that wrap it, and the request/error/outcome shapes they trade.
//!
//! Every implementation lives above this crate. The two no-op guards here are
//! the contract's neutral zero values, not adapters: they perform no I/O and
//! exist so a caller can wire the port without a budget or policy engine.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use ironclaw_host_api::turn::LoopGateRef;

use crate::host::{
    AgentLoopHostError, AgentLoopHostErrorKind, AgentLoopHostErrorReasonKind, LoopModelRequest,
    LoopModelResponse, LoopModelUsage, LoopRunContext, LoopSafeSummary,
};
use crate::model_work::{ModelWorkOutcome, ModelWorkRequest};

/// Outcome passed to [`LoopModelBudgetAccountant::post_model_call`] so the
/// accountant can record usage on success or note the failure kind.
#[derive(Debug, Clone)]
pub enum ModelCallOutcome<'a> {
    /// The model call succeeded; the response is available for inspection.
    Success(&'a LoopModelResponse),
    /// The model call failed with the given gateway error.
    Failure(&'a LoopModelGatewayError),
}

/// Budget/resource accounting boundary invoked around every model call flowing
/// through `ironclaw_turns`'s `HostManagedLoopModelPort`.
///
/// Implementations may enforce token budgets, call-count limits, cost caps, or
/// any other resource policy. A `pre_model_call` rejection short-circuits the
/// provider call entirely.
#[async_trait]
pub trait LoopModelBudgetAccountant: Send + Sync {
    /// Called **before** any model-backed work dispatches to a provider.
    async fn pre_model_work(
        &self,
        context: &LoopRunContext,
        request: &ModelWorkRequest,
    ) -> Result<(), LoopModelGatewayError>;

    /// Called after model-backed work succeeds or fails.
    async fn post_model_work(
        &self,
        context: &LoopRunContext,
        request: &ModelWorkRequest,
        outcome: ModelWorkOutcome,
    ) -> Result<(), LoopModelGatewayError>;

    /// Called **before** dispatching the model request. Return `Err` with
    /// `AgentLoopHostErrorKind::SpendBudgetExceeded` to reject the call when
    /// the configured model-spend budget is exhausted.
    async fn pre_model_call(
        &self,
        context: &LoopRunContext,
        request: &LoopModelRequest,
    ) -> Result<(), LoopModelGatewayError> {
        self.pre_model_work(context, &ModelWorkRequest::for_assistant(context, request))
            .await
    }

    /// Called **after** the model call completes (or fails). Implementations
    /// should record success usage and reconcile or release any pre-call
    /// reservation for provider failures. Any durable accounting/reconciliation
    /// failure must be returned so callers fail closed instead of hiding stuck
    /// reservations or missing failed-call accounting behind the provider error.
    async fn post_model_call(
        &self,
        context: &LoopRunContext,
        request: &LoopModelRequest,
        outcome: ModelCallOutcome<'_>,
    ) -> Result<(), LoopModelGatewayError> {
        self.post_model_work(
            context,
            &ModelWorkRequest::for_assistant(context, request),
            ModelWorkOutcome::from_model_call(outcome),
        )
        .await
    }

    /// Best-effort synchronous finalization of any in-flight reservation for
    /// this run. Invoked from cancellation and failed post-accounting paths
    /// where awaiting [`Self::post_model_call`] is impossible. Implementations
    /// may release a cancelled reservation or retry reconciliation when actual
    /// provider usage is already known. Default impl is a no-op for
    /// accountants that do not hold per-run state.
    ///
    /// This is *the* cancellation-safety hook: when the model future is
    /// dropped mid-await, the surrounding port's [`Drop`] runs synchronously
    /// and calls `release_in_flight` so the reservation does not orphan
    /// until period rollover.
    fn release_in_flight(&self, _context: &LoopRunContext) {}
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopModelGatewayRequest {
    pub context: LoopRunContext,
    pub request: LoopModelRequest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Error)]
#[error("loop model gateway {kind:?}: {safe_summary}")]
/// Sanitized model-gateway failure surfaced through the loop-host wire contract.
///
/// `AgentLoopHostErrorKind::CredentialUnavailable` means the host could not
/// provide a scoped, non-reusable credential for the selected provider/model;
/// callers must treat it as a host-owned credential acquisition failure, not as
/// provider output. `AgentLoopHostErrorKind::BudgetAccountingFailed` can
/// surface after a provider failure when post-call accounting/release fails
/// closed; it is distinct from a provider or configured-budget exhaustion.
pub struct LoopModelGatewayError {
    pub kind: AgentLoopHostErrorKind,
    pub safe_summary: LoopSafeSummary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_kind: Option<AgentLoopHostErrorReasonKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_ref: Option<LoopGateRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
    /// Deterministic evidence that recovery can advance to this ordered
    /// provider fallback index.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_fallback_index: Option<u32>,
    /// Provider-reported usage for a call that consumed tokens before failing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<LoopModelUsage>,
    /// Secret-value-scrubbed cause text for model recovery and failure
    /// explanation. Unlike `safe_summary`, path and payload delimiters are
    /// allowed.
    #[serde(skip)]
    pub detail: Option<String>,
}

impl LoopModelGatewayError {
    pub fn new(
        kind: AgentLoopHostErrorKind,
        safe_summary: impl Into<String>,
    ) -> Result<Self, String> {
        Ok(Self {
            kind,
            safe_summary: LoopSafeSummary::new(safe_summary)?,
            reason_kind: None,
            gate_ref: None,
            retry_after_ms: None,
            next_fallback_index: None,
            usage: None,
            detail: None,
        })
    }

    /// Build the gateway error for a primary model call that exceeded its
    /// timeout. Surfaces as `AgentLoopHostErrorKind::Unavailable`, which the
    /// recovery strategy treats as retryable — the correct disposition for a
    /// transient provider/gateway stall. Infallible: the safe summary is a
    /// known-good literal.
    pub fn timed_out() -> Self {
        Self {
            kind: AgentLoopHostErrorKind::Unavailable,
            safe_summary: LoopSafeSummary::model_gateway_timed_out(),
            reason_kind: None,
            gate_ref: None,
            retry_after_ms: None,
            next_fallback_index: None,
            usage: None,
            detail: None,
        }
    }

    pub fn with_reason_kind(mut self, reason_kind: AgentLoopHostErrorReasonKind) -> Self {
        self.reason_kind = Some(reason_kind);
        self
    }

    pub fn with_gate_ref(mut self, gate_ref: LoopGateRef) -> Self {
        self.gate_ref = Some(gate_ref);
        self
    }

    pub fn with_retry_after_ms(mut self, retry_after_ms: u64) -> Self {
        self.retry_after_ms = Some(retry_after_ms);
        self
    }

    pub fn with_next_fallback_index(mut self, fallback_index: u32) -> Self {
        self.next_fallback_index = Some(fallback_index);
        self
    }

    pub fn with_usage(mut self, usage: LoopModelUsage) -> Self {
        self.usage = Some(usage);
        self
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn into_host_error(self) -> AgentLoopHostError {
        let mut error = AgentLoopHostError::new(self.kind, self.safe_summary.as_str().to_string());
        if let Some(reason_kind) = self.reason_kind {
            error = error.with_reason_kind(reason_kind);
        }
        if let Some(gate_ref) = self.gate_ref {
            error = error.with_gate_ref(gate_ref);
        }
        if let Some(retry_after_ms) = self.retry_after_ms {
            error = error.with_retry_after_ms(retry_after_ms);
        }
        if let Some(next_fallback_index) = self.next_fallback_index {
            error = error.with_next_fallback_index(next_fallback_index);
        }
        if let Some(usage) = self.usage {
            error = error.with_usage(usage);
        }
        if let Some(detail) = self.detail {
            error = error.with_detail(detail);
        }
        error
    }
}

#[async_trait]
pub trait LoopModelGateway: Send + Sync {
    async fn stream_model(
        &self,
        request: LoopModelGatewayRequest,
    ) -> Result<LoopModelResponse, LoopModelGatewayError>;

    async fn stream_model_with_progress(
        &self,
        request: LoopModelGatewayRequest,
        _progress_sink: Arc<dyn LoopModelProgressSink>,
    ) -> Result<LoopModelResponse, LoopModelGatewayError> {
        self.stream_model(request).await
    }
}

#[async_trait]
pub trait LoopModelProgressSink: Send + Sync {
    async fn model_text_update(&self, safe_text: String);
}

/// Provider/model policy guard consulted before dispatching a model call.
///
/// Implementations may enforce allow/deny lists for models, providers, or
/// any request-level policy. A denial short-circuits the call before any
/// provider or credential is touched.
#[async_trait]
pub trait LoopModelPolicyGuard: Send + Sync {
    /// Return `Ok(())` to allow model-backed work, or `Err` with
    /// `AgentLoopHostErrorKind::PolicyDenied` and a sanitized summary.
    async fn check_model_work_policy(
        &self,
        context: &LoopRunContext,
        request: &ModelWorkRequest,
    ) -> Result<(), LoopModelGatewayError>;

    /// Return `Ok(())` to allow the call, or `Err` with
    /// `AgentLoopHostErrorKind::PolicyDenied` and a sanitized summary.
    async fn check_model_policy(
        &self,
        context: &LoopRunContext,
        request: &LoopModelRequest,
    ) -> Result<(), LoopModelGatewayError> {
        self.check_model_work_policy(context, &ModelWorkRequest::for_assistant(context, request))
            .await
    }
}

/// A no-op policy guard that allows every model call.
pub struct NoOpPolicyGuard;

#[async_trait]
impl LoopModelPolicyGuard for NoOpPolicyGuard {
    async fn check_model_work_policy(
        &self,
        _context: &LoopRunContext,
        _request: &ModelWorkRequest,
    ) -> Result<(), LoopModelGatewayError> {
        Ok(())
    }
}

/// A no-op budget accountant that approves every call and records nothing.
///
/// Used as the default when no budget policy is configured.
pub struct NoOpBudgetAccountant;

#[async_trait]
impl LoopModelBudgetAccountant for NoOpBudgetAccountant {
    async fn pre_model_work(
        &self,
        _context: &LoopRunContext,
        _request: &ModelWorkRequest,
    ) -> Result<(), LoopModelGatewayError> {
        Ok(())
    }

    async fn post_model_work(
        &self,
        _context: &LoopRunContext,
        _request: &ModelWorkRequest,
        _outcome: ModelWorkOutcome,
    ) -> Result<(), LoopModelGatewayError> {
        Ok(())
    }
}
