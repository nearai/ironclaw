use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::LoopDiagnosticRef;

use super::host::{
    AgentLoopHostError, AgentLoopHostErrorKind, LoopModelPort, LoopModelRequest, LoopModelResponse,
    LoopRunContext, LoopSafeSummary, sanitize_model_visible_text,
};
use super::milestones::{LoopHostMilestoneEmitter, LoopHostMilestoneSink};

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
    /// Called **before** dispatching the model request. Return `Err` with
    /// `AgentLoopHostErrorKind::BudgetExceeded` to reject the call.
    async fn pre_model_call(
        &self,
        context: &LoopRunContext,
        request: &LoopModelRequest,
    ) -> Result<(), LoopModelGatewayError>;

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
    ) -> Result<(), LoopModelGatewayError>;
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
            diagnostic_ref: None,
        })
    }

    pub fn with_diagnostic_ref(mut self, diagnostic_ref: LoopDiagnosticRef) -> Self {
        self.diagnostic_ref = Some(diagnostic_ref);
        self
    }

    fn into_host_error(self) -> AgentLoopHostError {
        let mut error = AgentLoopHostError::new(self.kind, self.safe_summary.as_str().to_string());
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
}

/// Provider/model policy guard consulted before dispatching a model call.
///
/// Implementations may enforce allow/deny lists for models, providers, or
/// any request-level policy. A denial short-circuits the call before any
/// provider or credential is touched.
#[async_trait]
pub trait LoopModelPolicyGuard: Send + Sync {
    /// Return `Ok(())` to allow the call, or `Err` with
    /// `AgentLoopHostErrorKind::PolicyDenied` and a sanitized summary.
    async fn check_model_policy(
        &self,
        context: &LoopRunContext,
        request: &LoopModelRequest,
    ) -> Result<(), LoopModelGatewayError>;
}

/// A no-op policy guard that allows every model call.
pub struct NoOpPolicyGuard;

#[async_trait]
impl LoopModelPolicyGuard for NoOpPolicyGuard {
    async fn check_model_policy(
        &self,
        _context: &LoopRunContext,
        _request: &LoopModelRequest,
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
    async fn pre_model_call(
        &self,
        _context: &LoopRunContext,
        _request: &LoopModelRequest,
    ) -> Result<(), LoopModelGatewayError> {
        Ok(())
    }

    async fn post_model_call(
        &self,
        _context: &LoopRunContext,
        _request: &LoopModelRequest,
        _outcome: ModelCallOutcome<'_>,
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
    S: LoopHostMilestoneSink + ?Sized,
{
    async fn stream_model(
        &self,
        request: LoopModelRequest,
    ) -> Result<LoopModelResponse, AgentLoopHostError> {
        // Policy check — rejects before any provider or credential is touched.
        if let Err(policy_error) = self
            .policy_guard
            .check_model_policy(&self.context, &request)
            .await
        {
            return Err(policy_error.into_host_error());
        }

        // Pre-call budget check — rejects before touching the provider.
        if let Err(budget_error) = self
            .accountant
            .pre_model_call(&self.context, &request)
            .await
        {
            return Err(budget_error.into_host_error());
        }

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

        let gateway_result = self
            .gateway
            .stream_model(LoopModelGatewayRequest {
                context: self.context.clone(),
                request: request.clone(),
            })
            .await
            .map(sanitize_model_response);

        // Post-call accounting fires on BOTH success and failure.
        let outcome = match &gateway_result {
            Ok(response) => ModelCallOutcome::Success(response),
            Err(error) => ModelCallOutcome::Failure(error),
        };
        if let Err(post_error) = self
            .accountant
            .post_model_call(&self.context, &request, outcome)
            .await
        {
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

fn sanitize_model_response(mut response: LoopModelResponse) -> LoopModelResponse {
    for chunk in &mut response.chunks {
        chunk.safe_text_delta =
            sanitize_model_visible_text(std::mem::take(&mut chunk.safe_text_delta));
    }
    response
}
