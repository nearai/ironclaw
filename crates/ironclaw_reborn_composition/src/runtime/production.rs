use std::sync::{Arc, OnceLock};

use ironclaw_loop_support::{
    CapabilityAllowSet, CapabilityResolveError, CapabilityResultWrite,
    CapabilitySurfaceProfileResolver, CapabilityWriteResult, HostIdentityContextBuildError,
    HostIdentityContextCandidate, HostIdentityContextSource, LoopCapabilityInputResolver,
    LoopCapabilityPortFactory, LoopCapabilityResultWriter,
};
use ironclaw_product_workflow::{
    ApprovalInteractionService, ListPendingApprovalsRequest, ListPendingApprovalsResponse,
    ProductWorkflowError, ResolveApprovalInteractionRequest, ResolveApprovalInteractionResponse,
};
use ironclaw_turns::run_profile::{
    AgentLoopHostError, AgentLoopHostErrorKind, CapabilityInputRef, LoopCapabilityPort,
    LoopRunContext, PromptMode,
};

#[derive(Default)]
pub(super) struct EmptyIdentityContextSource;

#[async_trait::async_trait]
impl HostIdentityContextSource for EmptyIdentityContextSource {
    async fn load_identity_candidates(
        &self,
        _run_context: &LoopRunContext,
        _mode: PromptMode,
    ) -> Result<Vec<HostIdentityContextCandidate>, HostIdentityContextBuildError> {
        Ok(Vec::new())
    }
}

#[derive(Default)]
pub(super) struct UnavailableCapabilityIo;

#[async_trait::async_trait]
impl LoopCapabilityInputResolver for UnavailableCapabilityIo {
    async fn resolve_capability_input(
        &self,
        _run_context: &LoopRunContext,
        _input_ref: &CapabilityInputRef,
    ) -> Result<serde_json::Value, AgentLoopHostError> {
        Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::Unavailable,
            "capability input resolver is unavailable for production runtime launch",
        ))
    }
}

#[async_trait::async_trait]
impl LoopCapabilityResultWriter for UnavailableCapabilityIo {
    async fn write_capability_result(
        &self,
        _write: CapabilityResultWrite<'_>,
    ) -> Result<CapabilityWriteResult, AgentLoopHostError> {
        Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::Unavailable,
            "capability result writer is unavailable for production runtime launch",
        ))
    }
}

#[derive(Clone, Default)]
pub(super) struct UnavailableCapabilityPortFactory;

#[async_trait::async_trait]
impl LoopCapabilityPortFactory for UnavailableCapabilityPortFactory {
    async fn create_capability_port(
        &self,
        _run_context: &LoopRunContext,
    ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
        Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::Unavailable,
            "capability port is unavailable for production runtime launch",
        ))
    }
}

/// Production launch is fail-closed until a real capability surface resolver is wired.
/// Every resolution returns an empty allowlist and emits an audit-friendly warning.
pub(super) struct EmptyCapabilitySurfaceResolver;

#[async_trait::async_trait]
impl CapabilitySurfaceProfileResolver for EmptyCapabilitySurfaceResolver {
    async fn resolve(
        &self,
        _run_context: &LoopRunContext,
    ) -> Result<CapabilityAllowSet, CapabilityResolveError> {
        static WARNED_EMPTY_PRODUCTION_CAPABILITY_SURFACE: OnceLock<()> = OnceLock::new();
        WARNED_EMPTY_PRODUCTION_CAPABILITY_SURFACE.get_or_init(|| {
            tracing::warn!(
                "production capability surface resolver is fail-closed; returning empty allowlist"
            );
        });
        tracing::debug!(
            "production capability surface resolver returned fail-closed empty allowlist"
        );
        Ok(CapabilityAllowSet::allowlist(Vec::new()))
    }
}

pub(super) struct UnavailableApprovalInteractionService;

#[async_trait::async_trait]
impl ApprovalInteractionService for UnavailableApprovalInteractionService {
    async fn list_pending(
        &self,
        _request: ListPendingApprovalsRequest,
    ) -> Result<ListPendingApprovalsResponse, ProductWorkflowError> {
        Err(ProductWorkflowError::BeforeInboundPolicyFailed {
            reason: "approval interaction service is not wired for production runtime launch"
                .to_string(),
            permanent: true,
        })
    }

    async fn resolve(
        &self,
        _request: ResolveApprovalInteractionRequest,
    ) -> Result<ResolveApprovalInteractionResponse, ProductWorkflowError> {
        Err(ProductWorkflowError::BeforeInboundPolicyFailed {
            reason: "approval interaction service is not wired for production runtime launch"
                .to_string(),
            permanent: true,
        })
    }
}
