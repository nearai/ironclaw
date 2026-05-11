use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::LoopDiagnosticRef;

use super::host::{
    AgentLoopHostError, AgentLoopHostErrorKind, LoopModelPort, LoopModelRequest, LoopModelResponse,
    LoopRunContext, LoopSafeSummary,
};
use super::model_route::ResolvedModelRoute;
use super::milestones::{LoopHostMilestoneEmitter, LoopHostMilestoneSink};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopModelGatewayRequest {
    pub context: LoopRunContext,
    pub request: LoopModelRequest,
    /// The resolved model route from the run profile, if any.
    ///
    /// Gateway implementations use this to select the correct provider instance
    /// instead of relying solely on `CompletionRequest.model`.
    pub resolved_route: Option<ResolvedModelRoute>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Error)]
#[error("loop model gateway {kind:?}: {safe_summary}")]
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

#[derive(Clone)]
pub struct HostManagedLoopModelPort<G, S>
where
    G: LoopModelGateway + ?Sized,
    S: LoopHostMilestoneSink + ?Sized,
{
    context: LoopRunContext,
    gateway: Arc<G>,
    milestones: LoopHostMilestoneEmitter<S>,
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
        self.milestones
            .model_started(request.model_preference.clone())
            .await?;
        let resolved_route = self
            .context
            .resolved_run_profile
            .resolved_model_route
            .clone();
        let response = self
            .gateway
            .stream_model(LoopModelGatewayRequest {
                context: self.context.clone(),
                request,
                resolved_route,
            })
            .await
            .map_err(LoopModelGatewayError::into_host_error)?;
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
