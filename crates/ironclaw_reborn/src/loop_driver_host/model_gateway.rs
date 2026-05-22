use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_loop_support::{
    HostIdentityContextSource, HostManagedModelGateway, HostSkillContextSource,
    ThreadBackedLoopModelPort,
};
use ironclaw_threads::{SessionThreadService, ThreadScope};
use ironclaw_turns::run_profile::{
    AgentLoopHostError, InstructionMaterializationStore, LoopCapabilityPort, LoopModelGateway,
    LoopModelGatewayError, LoopModelGatewayRequest, LoopModelPort, LoopModelResponse,
    LoopPromptBundleAuthority, LoopSafeSummary,
};

pub(super) struct ThreadResolvingLoopModelGateway<S, G>
where
    S: SessionThreadService + ?Sized,
    G: HostManagedModelGateway + ?Sized,
{
    thread_service: Arc<S>,
    thread_scope: ThreadScope,
    host_gateway: Arc<G>,
    max_messages: usize,
    skill_context_source: Option<Arc<dyn HostSkillContextSource>>,
    identity_context_source: Option<Arc<dyn HostIdentityContextSource>>,
    instruction_materialization_store: Option<Arc<dyn InstructionMaterializationStore>>,
    capabilities: Option<Arc<dyn LoopCapabilityPort>>,
    prompt_authority: LoopPromptBundleAuthority,
}

pub(super) struct ThreadResolvingLoopModelGatewayConfig<S, G>
where
    S: SessionThreadService + ?Sized,
    G: HostManagedModelGateway + ?Sized,
{
    pub(super) thread_service: Arc<S>,
    pub(super) thread_scope: ThreadScope,
    pub(super) host_gateway: Arc<G>,
    pub(super) max_messages: usize,
    pub(super) skill_context_source: Option<Arc<dyn HostSkillContextSource>>,
    pub(super) identity_context_source: Option<Arc<dyn HostIdentityContextSource>>,
    pub(super) instruction_materialization_store: Option<Arc<dyn InstructionMaterializationStore>>,
    pub(super) capabilities: Option<Arc<dyn LoopCapabilityPort>>,
    pub(super) prompt_authority: LoopPromptBundleAuthority,
}

impl<S, G> ThreadResolvingLoopModelGateway<S, G>
where
    S: SessionThreadService + ?Sized,
    G: HostManagedModelGateway + ?Sized,
{
    pub(super) fn new(config: ThreadResolvingLoopModelGatewayConfig<S, G>) -> Self {
        Self {
            thread_service: config.thread_service,
            thread_scope: config.thread_scope,
            host_gateway: config.host_gateway,
            max_messages: config.max_messages,
            skill_context_source: config.skill_context_source,
            identity_context_source: config.identity_context_source,
            instruction_materialization_store: config.instruction_materialization_store,
            capabilities: config.capabilities,
            prompt_authority: config.prompt_authority,
        }
    }
}

#[async_trait]
impl<S, G> LoopModelGateway for ThreadResolvingLoopModelGateway<S, G>
where
    S: SessionThreadService + ?Sized + Send + Sync,
    G: HostManagedModelGateway + ?Sized + Send + Sync,
{
    async fn stream_model(
        &self,
        request: LoopModelGatewayRequest,
    ) -> Result<LoopModelResponse, LoopModelGatewayError> {
        let mut model_port = ThreadBackedLoopModelPort::new(
            Arc::clone(&self.thread_service),
            self.thread_scope.clone(),
            request.context,
            Arc::clone(&self.host_gateway),
            self.max_messages,
        )
        .with_prompt_bundle_authority(self.prompt_authority.clone());
        if let Some(source) = self.skill_context_source.as_ref() {
            model_port = model_port.with_skill_context_source(source.clone());
        }
        if let Some(source) = self.identity_context_source.as_ref() {
            model_port = model_port.with_identity_context_source(source.clone());
        }
        if let Some(store) = self.instruction_materialization_store.as_ref() {
            model_port = model_port.with_instruction_materialization_store(Arc::clone(store));
        }
        if let Some(capabilities) = self.capabilities.as_ref() {
            model_port = model_port.with_capability_port(Arc::clone(capabilities));
        }
        model_port
            .stream_model(request.request)
            .await
            .map_err(host_error_to_model_gateway_error)
    }
}

fn host_error_to_model_gateway_error(error: AgentLoopHostError) -> LoopModelGatewayError {
    let diagnostic_ref = error.diagnostic_ref;
    let mut converted = match LoopModelGatewayError::new(error.kind, error.safe_summary) {
        Ok(error) => error,
        Err(_) => LoopModelGatewayError {
            kind: error.kind,
            safe_summary: LoopSafeSummary::model_gateway_failed(),
            diagnostic_ref: None,
        },
    };
    if let Some(diagnostic_ref) = diagnostic_ref {
        converted = converted.with_diagnostic_ref(diagnostic_ref);
    }
    converted
}
