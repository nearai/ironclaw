use std::sync::Arc;

use crate::{
    HostIdentityContextSource, HostManagedModelGateway, HostManagedModelStreamSink,
    HostManagedPromptDiagnosticSink, HostSkillContextSource, LoopAttachmentReadPort,
    ThreadBackedLoopModelPort, ThreadContextWindowCache,
};
use async_trait::async_trait;
use ironclaw_loop_contracts::{
    InstructionMaterializationStore, LoopCapabilityPort, LoopModelGateway, LoopModelGatewayError,
    LoopModelGatewayRequest, LoopModelPort, LoopModelProgressSink, LoopModelResponse,
    LoopPromptBundleAuthority,
};
use ironclaw_threads::{SessionThreadService, ThreadScope};

use crate::model_gateway_error_mapping::host_error_to_model_gateway_error;

/// Everything a [`ThreadResolvingLoopModelGateway`] needs, as one value.
///
/// A params struct rather than an eleven-argument constructor, and public
/// rather than the struct's own fields: the driver host in `ironclaw_turn_runner`
/// assembles the run-scoped inputs, and this crate owns what the gateway does
/// with them. Adding a field is a compile error at the two call sites, which
/// is the property the previous `pub`-fields shape also had — without letting
/// a caller mutate the gateway after construction or build one field-by-field
/// from a partially-initialized state.
pub struct ThreadResolvingLoopModelGatewayParts<S, G>
where
    S: SessionThreadService + ?Sized,
    G: HostManagedModelGateway + ?Sized,
{
    pub thread_service: Arc<S>,
    pub thread_scope: ThreadScope,
    pub host_gateway: Arc<G>,
    pub max_messages: usize,
    pub skill_context_source: Option<Arc<dyn HostSkillContextSource>>,
    pub identity_context_source: Option<Arc<dyn HostIdentityContextSource>>,
    pub instruction_materialization_store: Option<Arc<dyn InstructionMaterializationStore>>,
    pub capabilities: Option<Arc<dyn LoopCapabilityPort>>,
    pub prompt_authority: LoopPromptBundleAuthority,
    pub context_window_cache: Option<Arc<ThreadContextWindowCache>>,
    pub attachment_read_port: Option<Arc<dyn LoopAttachmentReadPort>>,
    pub prompt_diagnostic_sink: Option<Arc<dyn HostManagedPromptDiagnosticSink>>,
}

/// Resolves a thread's transcript into a host-managed model request.
///
/// Fields are private and the only way in is [`Self::new`]: the WS3 shed moved
/// this port implementation out of `ironclaw_turn_runner`, and turning its
/// `pub(super)` fields into `pub` ones to keep the caller's struct literal
/// working would have let any downstream crate assemble a gateway with no
/// host-owned construction path (root `AGENTS.md`: "module-specific
/// initialization stays behind factories or builders in the owning crate").
pub struct ThreadResolvingLoopModelGateway<S, G>
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
    context_window_cache: Option<Arc<ThreadContextWindowCache>>,
    attachment_read_port: Option<Arc<dyn LoopAttachmentReadPort>>,
    prompt_diagnostic_sink: Option<Arc<dyn HostManagedPromptDiagnosticSink>>,
}

impl<S, G> ThreadResolvingLoopModelGateway<S, G>
where
    S: SessionThreadService + ?Sized,
    G: HostManagedModelGateway + ?Sized,
{
    pub fn new(parts: ThreadResolvingLoopModelGatewayParts<S, G>) -> Self {
        let ThreadResolvingLoopModelGatewayParts {
            thread_service,
            thread_scope,
            host_gateway,
            max_messages,
            skill_context_source,
            identity_context_source,
            instruction_materialization_store,
            capabilities,
            prompt_authority,
            context_window_cache,
            attachment_read_port,
            prompt_diagnostic_sink,
        } = parts;
        Self {
            thread_service,
            thread_scope,
            host_gateway,
            max_messages,
            skill_context_source,
            identity_context_source,
            instruction_materialization_store,
            capabilities,
            prompt_authority,
            context_window_cache,
            attachment_read_port,
            prompt_diagnostic_sink,
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
        self.stream_model_inner(request, None).await
    }

    async fn stream_model_with_progress(
        &self,
        request: LoopModelGatewayRequest,
        progress_sink: Arc<dyn LoopModelProgressSink>,
    ) -> Result<LoopModelResponse, LoopModelGatewayError> {
        self.stream_model_inner(request, Some(progress_sink)).await
    }
}

impl<S, G> ThreadResolvingLoopModelGateway<S, G>
where
    S: SessionThreadService + ?Sized + Send + Sync,
    G: HostManagedModelGateway + ?Sized + Send + Sync,
{
    async fn stream_model_inner(
        &self,
        request: LoopModelGatewayRequest,
        progress_sink: Option<Arc<dyn LoopModelProgressSink>>,
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
        if let Some(cache) = self.context_window_cache.as_ref() {
            model_port = model_port.with_context_window_cache(Arc::clone(cache));
        }
        if let Some(port) = self.attachment_read_port.as_ref() {
            model_port = model_port.with_attachment_read_port(Arc::clone(port));
        }
        if let Some(sink) = self.prompt_diagnostic_sink.as_ref() {
            model_port = model_port.with_prompt_diagnostic_sink(Arc::clone(sink));
        }
        if let Some(progress_sink) = progress_sink {
            model_port = model_port.with_stream_sink(Arc::new(LoopProgressHostStreamSink {
                inner: progress_sink,
            }));
        }
        model_port
            .stream_model(request.request)
            .await
            .map_err(host_error_to_model_gateway_error)
    }
}

struct LoopProgressHostStreamSink {
    inner: Arc<dyn LoopModelProgressSink>,
}

#[async_trait]
impl HostManagedModelStreamSink for LoopProgressHostStreamSink {
    async fn safe_text_update(&self, safe_text: String) {
        self.inner.model_text_update(safe_text).await;
    }
}
