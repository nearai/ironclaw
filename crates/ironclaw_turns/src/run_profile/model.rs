use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{LoopDiagnosticRef, LoopGateRef};

use super::host::{
    AgentLoopHostError, AgentLoopHostErrorKind, AgentLoopHostErrorReasonKind, LoopModelPort,
    LoopModelRequest, LoopModelResponse, LoopRunContext, LoopSafeSummary, ParentLoopOutput,
    sanitize_model_visible_text,
};
use super::milestones::{LoopHostMilestoneEmitter, LoopHostMilestoneSink};
use super::model_work::{ModelWorkOutcome, ModelWorkRequest};

/// Hard ceiling on a single primary assistant model call.
///
/// This is a defense-in-depth bound that wraps the entire gateway call (every
/// provider, not just NEAR AI). It MUST stay below the runner lease
/// ([`crate::memory::DEFAULT_RUNNER_LEASE_TTL_SECONDS`] = 90s) so a hung
/// provider is surfaced as a retryable `Unavailable` error before the lease
/// reclaims the runner mid-flight — the failure mode that wedged the Reborn
/// runtime on 2026-06-24. The invariant is enforced by
/// `primary_model_call_timeout_is_below_runner_lease` below.
///
/// Layered ordering: provider HTTP timeout (`ironclaw_llm`
/// `DEFAULT_REQUEST_TIMEOUT_SECS` = 60s) < this wrapper (75s) < runner lease
/// (90s). The provider bound fires first on the common path so the precise
/// provider error surfaces; this wrapper catches gateway-layer stalls the inner
/// bound misses.
const PRIMARY_MODEL_CALL_TIMEOUT: Duration = Duration::from_secs(75);
const FALLBACK_TEXT_DELTA_MILESTONE_STEP: usize = 15;

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
/// through [`HostManagedLoopModelPort`].
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
    /// `AgentLoopHostErrorKind::BudgetExceeded` to reject the call.
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

    /// Best-effort synchronous release of any in-flight reservation for this
    /// run. Invoked from cancellation paths (parent task drop, timeout)
    /// where awaiting [`Self::post_model_call`] is impossible. Default impl
    /// is a no-op for accountants that do not hold per-run state.
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
/// provider output. `AgentLoopHostErrorKind::BudgetExceeded` can also surface
/// after a provider failure when post-call accounting/release fails closed.
pub struct LoopModelGatewayError {
    pub kind: AgentLoopHostErrorKind,
    pub safe_summary: LoopSafeSummary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_kind: Option<AgentLoopHostErrorReasonKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_ref: Option<LoopGateRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic_ref: Option<LoopDiagnosticRef>,
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
            diagnostic_ref: None,
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
            diagnostic_ref: None,
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

    pub fn with_diagnostic_ref(mut self, diagnostic_ref: LoopDiagnosticRef) -> Self {
        self.diagnostic_ref = Some(diagnostic_ref);
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
        if let Some(diagnostic_ref) = self.diagnostic_ref {
            error = error.with_diagnostic_ref(diagnostic_ref);
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

#[derive(Clone)]
pub struct HostManagedLoopModelPort<G, S>
where
    G: LoopModelGateway + ?Sized,
    S: LoopHostMilestoneSink + ?Sized,
{
    context: LoopRunContext,
    gateway: Arc<G>,
    milestones: LoopHostMilestoneEmitter<S>,
    accountant: Arc<dyn LoopModelBudgetAccountant>,
    policy_guard: Arc<dyn LoopModelPolicyGuard>,
}

impl<G, S> HostManagedLoopModelPort<G, S>
where
    G: LoopModelGateway + ?Sized,
    S: LoopHostMilestoneSink + ?Sized,
{
    pub fn new(context: LoopRunContext, gateway: Arc<G>, milestone_sink: Arc<S>) -> Self {
        let milestones = LoopHostMilestoneEmitter::new(context.clone(), milestone_sink);
        Self {
            context,
            gateway,
            milestones,
            accountant: Arc::new(NoOpBudgetAccountant),
            policy_guard: Arc::new(NoOpPolicyGuard),
        }
    }

    /// Create a port with a custom budget accountant injected.
    pub fn with_accountant(
        context: LoopRunContext,
        gateway: Arc<G>,
        milestone_sink: Arc<S>,
        accountant: Arc<dyn LoopModelBudgetAccountant>,
    ) -> Self {
        let milestones = LoopHostMilestoneEmitter::new(context.clone(), milestone_sink);
        Self {
            context,
            gateway,
            milestones,
            accountant,
            policy_guard: Arc::new(NoOpPolicyGuard),
        }
    }

    /// Create a fully-configured port with policy guard and budget accountant.
    pub fn with_guards(
        context: LoopRunContext,
        gateway: Arc<G>,
        milestone_sink: Arc<S>,
        accountant: Arc<dyn LoopModelBudgetAccountant>,
        policy_guard: Arc<dyn LoopModelPolicyGuard>,
    ) -> Self {
        let milestones = LoopHostMilestoneEmitter::new(context.clone(), milestone_sink);
        Self {
            context,
            gateway,
            milestones,
            accountant,
            policy_guard,
        }
    }
}

#[async_trait]
impl<G, S> LoopModelPort for HostManagedLoopModelPort<G, S>
where
    G: LoopModelGateway + ?Sized,
    S: LoopHostMilestoneSink + ?Sized + 'static,
{
    async fn stream_model(
        &self,
        request: LoopModelRequest,
    ) -> Result<LoopModelResponse, AgentLoopHostError> {
        let work_request = ModelWorkRequest::for_assistant(&self.context, &request);

        // Policy check — rejects before any provider or credential is touched.
        if let Err(policy_error) = self
            .policy_guard
            .check_model_work_policy(&self.context, &work_request)
            .await
        {
            return Err(policy_error.into_host_error());
        }

        // Pre-call budget check — rejects before touching the provider.
        if let Err(budget_error) = self
            .accountant
            .pre_model_work(&self.context, &work_request)
            .await
        {
            return Err(budget_error.into_host_error());
        }

        // From here forward, a reservation has been taken. The guard below
        // ensures it is released if `stream_model` is cancelled mid-await
        // (tokio drop, parent timeout) without ever reaching the explicit
        // `post_model_call` below.
        let mut release_guard =
            ReservationReleaseGuard::new(self.accountant.as_ref(), &self.context);

        if let Err(error) = self
            .milestones
            .model_started(request.model_preference.clone())
            .await
        {
            tracing::debug!(
                kind = ?error.kind,
                diagnostic_ref = ?error.diagnostic_ref,
                "loop model_started milestone failed before model request"
            );
        }

        // Bound the primary model call so a hung provider/gateway surfaces as a
        // retryable error before the runner lease reclaims this run mid-flight.
        // See `PRIMARY_MODEL_CALL_TIMEOUT`.
        let progress_sink = Arc::new(MilestoneModelProgressSink {
            milestones: self.milestones.clone(),
            emitted_text: AtomicBool::new(false),
        });
        let gateway_call = self.gateway.stream_model_with_progress(
            LoopModelGatewayRequest {
                context: self.context.clone(),
                request: request.clone(),
            },
            progress_sink.clone(),
        );
        let gateway_result =
            match tokio::time::timeout(PRIMARY_MODEL_CALL_TIMEOUT, gateway_call).await {
                Ok(result) => result.map(sanitize_model_response),
                Err(_elapsed) => Err(LoopModelGatewayError::timed_out()),
            };

        // Post-call accounting fires on BOTH success and failure. The
        // RAII guard stays armed across this await — if the future is
        // cancelled mid-`post_model_call`, the Drop path calls
        // `release_in_flight` to clean up. `release_in_flight` is
        // idempotent against a successful post-call (the in-flight
        // entry is already gone), so disarming after success isn't
        // strictly required — but we still disarm on the happy path so
        // the Drop log doesn't fire on every successful run.
        let outcome = match &gateway_result {
            Ok(response) => ModelCallOutcome::Success(response),
            Err(error) => ModelCallOutcome::Failure(error),
        };
        let post_result = self
            .accountant
            .post_model_call(&self.context, &request, outcome)
            .await;
        // Disarm only AFTER post_model_call returns. If we're past this
        // line the in-flight entry is either reconciled, released, or
        // retained on a storage error — in any of those cases the
        // guard's Drop call would be a no-op against the same entry.
        release_guard.disarm();
        if let Err(post_error) = post_result {
            let host_error = post_error.into_host_error();
            if let Err(milestone_error) = self.milestones.model_failed(host_error.kind).await {
                tracing::debug!(
                    kind = ?milestone_error.kind,
                    diagnostic_ref = ?milestone_error.diagnostic_ref,
                    "loop model_failed milestone failed after post-model accounting error"
                );
            }
            return Err(host_error);
        }

        let response = match gateway_result {
            Ok(response) => response,
            Err(error) => {
                let host_error = error.into_host_error();
                if let Err(milestone_error) = self.milestones.model_failed(host_error.kind).await {
                    tracing::debug!(
                        kind = ?milestone_error.kind,
                        diagnostic_ref = ?milestone_error.diagnostic_ref,
                        "loop model_failed milestone failed after model error"
                    );
                }
                return Err(host_error);
            }
        };

        for safe_delta in &response.safe_reasoning_deltas {
            if let Err(error) = self
                .milestones
                .model_reasoning_delta(safe_delta.clone())
                .await
            {
                tracing::debug!(
                    kind = ?error.kind,
                    diagnostic_ref = ?error.diagnostic_ref,
                    "loop model reasoning milestone failed after successful model response"
                );
            }
        }
        if matches!(response.output, ParentLoopOutput::AssistantReply(_))
            && !progress_sink.emitted_text()
        {
            let text_chunk_count = response
                .chunks
                .iter()
                .filter(|chunk| !chunk.safe_text_delta.is_empty())
                .count();
            let mut text_chunk_index = 0;
            let mut accumulated_text = String::new();
            for chunk in &response.chunks {
                if chunk.safe_text_delta.is_empty() {
                    continue;
                }
                accumulated_text.push_str(&chunk.safe_text_delta);
                text_chunk_index += 1;
                if !should_emit_fallback_text_delta(text_chunk_index, text_chunk_count) {
                    continue;
                }
                if let Err(error) = self
                    .milestones
                    .model_text_delta(accumulated_text.clone())
                    .await
                {
                    tracing::debug!(
                        kind = ?error.kind,
                        diagnostic_ref = ?error.diagnostic_ref,
                        "loop model text milestone failed after successful model response"
                    );
                }
            }
        }
        if let Err(error) = self
            .milestones
            .model_completed(response.effective_model_profile_id.clone())
            .await
        {
            tracing::debug!(
                kind = ?error.kind,
                diagnostic_ref = ?error.diagnostic_ref,
                "loop model_completed milestone failed after successful model response"
            );
        }
        Ok(response)
    }
}

struct MilestoneModelProgressSink<S>
where
    S: LoopHostMilestoneSink + ?Sized,
{
    milestones: LoopHostMilestoneEmitter<S>,
    emitted_text: AtomicBool,
}

impl<S> MilestoneModelProgressSink<S>
where
    S: LoopHostMilestoneSink + ?Sized,
{
    fn emitted_text(&self) -> bool {
        self.emitted_text.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl<S> LoopModelProgressSink for MilestoneModelProgressSink<S>
where
    S: LoopHostMilestoneSink + ?Sized,
{
    async fn model_text_update(&self, safe_text: String) {
        self.emitted_text.store(true, Ordering::SeqCst);
        if let Err(error) = self.milestones.model_text_delta(safe_text).await {
            tracing::debug!(
                kind = ?error.kind,
                diagnostic_ref = ?error.diagnostic_ref,
                "loop model text progress milestone failed during model stream"
            );
        }
    }
}

/// RAII guard that releases the in-flight reservation if the surrounding
/// future is cancelled before `post_model_call` runs.
///
/// On Drop, when still armed, the guard calls
/// [`LoopModelBudgetAccountant::release_in_flight`] — a synchronous
/// best-effort path that the accountant uses to drop the per-run reservation
/// id and call `governor.release` without awaiting. Callers MUST `disarm()`
/// the guard before delegating cleanup to the async `post_model_call` path,
/// otherwise the release would fire twice.
struct ReservationReleaseGuard<'a> {
    accountant: &'a dyn LoopModelBudgetAccountant,
    context: &'a LoopRunContext,
    armed: bool,
}

impl<'a> ReservationReleaseGuard<'a> {
    fn new(accountant: &'a dyn LoopModelBudgetAccountant, context: &'a LoopRunContext) -> Self {
        Self {
            accountant,
            context,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ReservationReleaseGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.accountant.release_in_flight(self.context);
        }
    }
}

fn sanitize_model_response(mut response: LoopModelResponse) -> LoopModelResponse {
    for chunk in &mut response.chunks {
        chunk.safe_text_delta =
            sanitize_model_visible_text(std::mem::take(&mut chunk.safe_text_delta));
    }
    for safe_delta in &mut response.safe_reasoning_deltas {
        *safe_delta = sanitize_model_visible_text(std::mem::take(safe_delta));
    }
    response
        .safe_reasoning_deltas
        .retain(|safe_delta| !safe_delta.is_empty());
    if let ParentLoopOutput::AssistantReply(reply) = &mut response.output {
        reply.content = sanitize_model_visible_text(std::mem::take(&mut reply.content));
    }
    response
}

fn should_emit_fallback_text_delta(chunk_index: usize, chunk_count: usize) -> bool {
    chunk_index == chunk_count || chunk_index.is_multiple_of(FALLBACK_TEXT_DELTA_MILESTONE_STEP)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The primary model-call timeout must fire before the runner lease can
    /// reclaim the run mid-flight. This guards against a silent regression of
    /// the 2026-06-24 wedge, where the provider timeout (120s) exceeded the
    /// lease (90s) and the lease killed runners before any timeout fired.
    #[test]
    fn primary_model_call_timeout_is_below_runner_lease() {
        let lease_secs = u64::try_from(crate::memory::DEFAULT_RUNNER_LEASE_TTL_SECONDS)
            .expect("runner lease TTL is non-negative");
        assert!(
            PRIMARY_MODEL_CALL_TIMEOUT.as_secs() < lease_secs,
            "primary model-call timeout ({}s) must be below the runner lease ({}s)",
            PRIMARY_MODEL_CALL_TIMEOUT.as_secs(),
            lease_secs,
        );
    }

    #[test]
    fn fallback_model_text_delta_emission_is_throttled_and_final() {
        let emitted = (1..=32)
            .filter(|chunk_index| should_emit_fallback_text_delta(*chunk_index, 32))
            .collect::<Vec<_>>();

        assert_eq!(emitted, vec![15, 30, 32]);
        assert!(should_emit_fallback_text_delta(14, 14));
        assert!(!should_emit_fallback_text_delta(1, 14));
    }
}
