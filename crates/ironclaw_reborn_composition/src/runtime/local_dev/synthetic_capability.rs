use std::{
    collections::HashMap,
    sync::{Arc, Mutex as StdMutex},
};

use async_trait::async_trait;
use ironclaw_host_api::{CapabilityId, RuntimeKind};
use ironclaw_loop_support::{LoopCapabilityInputResolver, LoopCapabilityResultWriter};
use ironclaw_turns::run_profile::{
    AgentLoopHostError, AgentLoopHostErrorKind, CapabilityBatchInvocation, CapabilityBatchOutcome,
    CapabilityCallCandidate, CapabilityDescriptorView, CapabilityInvocation, CapabilityOutcome,
    CapabilitySurfaceVersion, ConcurrencyHint, LoopCapabilityPort, LoopRunContext,
    ProviderToolCall, ProviderToolCallCapabilityIds, ProviderToolCallReplay,
    ProviderToolDefinition, VisibleCapabilityRequest, VisibleCapabilitySurface,
};

pub(super) fn wrap_local_dev_synthetic_capabilities(
    inner: Arc<dyn LoopCapabilityPort>,
    capabilities: Vec<LocalDevSyntheticCapability>,
    run_context: LoopRunContext,
    input_resolver: Arc<dyn LoopCapabilityInputResolver>,
    result_writer: Arc<dyn LoopCapabilityResultWriter>,
) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
    if capabilities.is_empty() {
        return Ok(inner);
    }
    Ok(Arc::new(LocalDevSyntheticCapabilityPort::new(
        inner,
        capabilities,
        run_context,
        input_resolver,
        result_writer,
    )?))
}

pub(super) struct LocalDevSyntheticCapability {
    descriptor: LocalDevSyntheticCapabilityDescriptor,
    handler: Arc<dyn LocalDevSyntheticCapabilityHandler>,
}

impl LocalDevSyntheticCapability {
    pub(super) fn new(
        descriptor: LocalDevSyntheticCapabilityDescriptor,
        handler: Arc<dyn LocalDevSyntheticCapabilityHandler>,
    ) -> Self {
        Self {
            descriptor,
            handler,
        }
    }
}

pub(super) struct LocalDevSyntheticCapabilityDescriptor {
    capability_id: CapabilityId,
    provider_tool_name: String,
    description: String,
    concurrency_hint: ConcurrencyHint,
    parameters_schema: serde_json::Value,
}

impl LocalDevSyntheticCapabilityDescriptor {
    pub(super) fn new(
        capability_id: &str,
        provider_tool_name: &str,
        description: &str,
        concurrency_hint: ConcurrencyHint,
        parameters_schema: serde_json::Value,
    ) -> Result<Self, AgentLoopHostError> {
        Ok(Self {
            capability_id: CapabilityId::new(capability_id).map_err(|_| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Internal,
                    "synthetic capability id is invalid",
                )
            })?,
            provider_tool_name: provider_tool_name.to_string(),
            description: description.to_string(),
            concurrency_hint,
            parameters_schema,
        })
    }

    fn descriptor_view(&self) -> CapabilityDescriptorView {
        CapabilityDescriptorView {
            capability_id: self.capability_id.clone(),
            provider: None,
            runtime: RuntimeKind::System,
            safe_name: self.provider_tool_name.clone(),
            safe_description: self.description.clone(),
            concurrency_hint: self.concurrency_hint,
            parameters_schema: self.parameters_schema.clone(),
        }
    }

    fn tool_definition(&self) -> ProviderToolDefinition {
        ProviderToolDefinition {
            capability_id: self.capability_id.clone(),
            name: self.provider_tool_name.clone(),
            description: self.description.clone(),
            parameters: self.parameters_schema.clone(),
        }
    }
}

pub(super) struct LocalDevSyntheticCapabilityInvocation {
    pub(super) run_context: LoopRunContext,
    pub(super) request: CapabilityInvocation,
    pub(super) input: serde_json::Value,
    pub(super) result_writer: Arc<dyn LoopCapabilityResultWriter>,
}

#[async_trait]
pub(super) trait LocalDevSyntheticCapabilityHandler: Send + Sync {
    fn validate_provider_arguments(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<(), AgentLoopHostError>;

    async fn invoke(
        &self,
        invocation: LocalDevSyntheticCapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError>;
}

struct LocalDevSyntheticCapabilityPort {
    inner: Arc<dyn LoopCapabilityPort>,
    run_context: LoopRunContext,
    input_resolver: Arc<dyn LoopCapabilityInputResolver>,
    result_writer: Arc<dyn LoopCapabilityResultWriter>,
    capabilities_by_id: HashMap<CapabilityId, LocalDevSyntheticCapability>,
    capability_ids_by_provider_tool_name: HashMap<String, CapabilityId>,
    current_surface_version: StdMutex<Option<CapabilitySurfaceVersion>>,
}

impl LocalDevSyntheticCapabilityPort {
    fn new(
        inner: Arc<dyn LoopCapabilityPort>,
        capabilities: Vec<LocalDevSyntheticCapability>,
        run_context: LoopRunContext,
        input_resolver: Arc<dyn LoopCapabilityInputResolver>,
        result_writer: Arc<dyn LoopCapabilityResultWriter>,
    ) -> Result<Self, AgentLoopHostError> {
        let mut capabilities_by_id = HashMap::new();
        let mut capability_ids_by_provider_tool_name = HashMap::new();
        for capability in capabilities {
            let capability_id = capability.descriptor.capability_id.clone();
            let provider_tool_name = capability.descriptor.provider_tool_name.clone();
            if capabilities_by_id
                .insert(capability_id.clone(), capability)
                .is_some()
                || capability_ids_by_provider_tool_name
                    .insert(provider_tool_name, capability_id)
                    .is_some()
            {
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::InvalidInvocation,
                    "duplicate synthetic capability registration",
                ));
            }
        }
        Ok(Self {
            inner,
            run_context,
            input_resolver,
            result_writer,
            capabilities_by_id,
            capability_ids_by_provider_tool_name,
            current_surface_version: StdMutex::new(None),
        })
    }

    fn current_surface_version(&self) -> Result<CapabilitySurfaceVersion, AgentLoopHostError> {
        self.current_surface_version
            .lock()
            .map_err(|_| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Internal,
                    "synthetic capability surface lock failed",
                )
            })?
            .clone()
            .ok_or_else(|| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::StaleSurface,
                    "capability surface is unavailable",
                )
            })
    }

    fn synthetic_provider_call(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Option<(&CapabilityId, &LocalDevSyntheticCapability)> {
        self.capability_ids_by_provider_tool_name
            .get(&tool_call.name)
            .and_then(|capability_id| self.capabilities_by_id.get_key_value(capability_id))
    }
}

#[async_trait]
impl LoopCapabilityPort for LocalDevSyntheticCapabilityPort {
    fn tool_definitions(&self) -> Result<Vec<ProviderToolDefinition>, AgentLoopHostError> {
        let mut definitions = self.inner.tool_definitions()?;
        if self
            .current_surface_version
            .lock()
            .map_err(|_| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Internal,
                    "synthetic capability surface lock failed",
                )
            })?
            .is_none()
        {
            return Ok(definitions);
        }
        for capability in self.capabilities_by_id.values() {
            if !definitions
                .iter()
                .any(|definition| definition.capability_id == capability.descriptor.capability_id)
            {
                definitions.push(capability.descriptor.tool_definition());
            }
        }
        definitions.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(definitions)
    }

    fn provider_tool_call_capability_ids(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<ProviderToolCallCapabilityIds, AgentLoopHostError> {
        if let Some((capability_id, _)) = self.synthetic_provider_call(tool_call) {
            return Ok(ProviderToolCallCapabilityIds::single(capability_id.clone()));
        }
        self.inner.provider_tool_call_capability_ids(tool_call)
    }

    fn validate_provider_tool_call(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<(), AgentLoopHostError> {
        if let Some((_, capability)) = self.synthetic_provider_call(tool_call) {
            capability
                .handler
                .validate_provider_arguments(&tool_call.arguments)?;
            if tool_call.turn_id.is_none() {
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::InvalidInvocation,
                    "provider tool call is missing a provider turn id",
                ));
            }
            return Ok(());
        }
        self.inner.validate_provider_tool_call(tool_call)
    }

    async fn register_provider_tool_call(
        &self,
        tool_call: ProviderToolCall,
    ) -> Result<CapabilityCallCandidate, AgentLoopHostError> {
        let Some((capability_id, _)) = self.synthetic_provider_call(&tool_call) else {
            return self.inner.register_provider_tool_call(tool_call).await;
        };
        let capability_id = capability_id.clone();
        self.validate_provider_tool_call(&tool_call)?;
        let provider_turn_id = tool_call.turn_id.clone().ok_or_else(|| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "provider tool call is missing a provider turn id",
            )
        })?;
        let input_ref = self
            .input_resolver
            .register_provider_tool_call_input(&self.run_context, &tool_call)
            .await?;
        Ok(CapabilityCallCandidate {
            surface_version: self.current_surface_version()?,
            capability_id: capability_id.clone(),
            input_ref,
            effective_capability_ids: vec![capability_id.clone()],
            provider_replay: Some(ProviderToolCallReplay {
                provider_id: tool_call.provider_id,
                provider_model_id: tool_call.provider_model_id,
                provider_turn_id,
                provider_call_id: tool_call.id,
                provider_tool_name: tool_call.name,
                arguments: tool_call.arguments,
                response_reasoning: tool_call.response_reasoning,
                reasoning: tool_call.reasoning,
                signature: tool_call.signature,
            }),
        })
    }

    async fn visible_capabilities(
        &self,
        request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        let mut surface = self.inner.visible_capabilities(request).await?;
        for capability_id in self.capabilities_by_id.keys() {
            if surface
                .descriptors
                .iter()
                .any(|descriptor| &descriptor.capability_id == capability_id)
            {
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::InvalidInvocation,
                    "synthetic capability conflicts with runtime capability surface",
                ));
            }
        }
        *self.current_surface_version.lock().map_err(|_| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                "synthetic capability surface lock failed",
            )
        })? = Some(surface.version.clone());
        let mut synthetic_descriptors = self
            .capabilities_by_id
            .values()
            .map(|capability| capability.descriptor.descriptor_view())
            .collect::<Vec<_>>();
        synthetic_descriptors.sort_by(|left, right| left.safe_name.cmp(&right.safe_name));
        surface.descriptors.extend(synthetic_descriptors);
        Ok(surface)
    }

    async fn invoke_capability(
        &self,
        request: CapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        let Some(capability) = self.capabilities_by_id.get(&request.capability_id) else {
            return self.inner.invoke_capability(request).await;
        };
        let handler = Arc::clone(&capability.handler);
        if request.surface_version != self.current_surface_version()? {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::StaleSurface,
                "synthetic capability call cites a stale capability surface",
            ));
        }
        let input = self
            .input_resolver
            .resolve_capability_input(&self.run_context, &request.input_ref)
            .await?;
        handler
            .invoke(LocalDevSyntheticCapabilityInvocation {
                run_context: self.run_context.clone(),
                request,
                input,
                result_writer: Arc::clone(&self.result_writer),
            })
            .await
    }

    async fn invoke_capability_batch(
        &self,
        request: CapabilityBatchInvocation,
    ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
        let mut outcomes = Vec::new();
        let mut stopped_on_suspension = false;
        for invocation in request.invocations {
            let outcome = self.invoke_capability(invocation).await?;
            let is_suspension = outcome.is_suspension();
            outcomes.push(outcome);
            if request.stop_on_first_suspension && is_suspension {
                stopped_on_suspension = true;
                break;
            }
        }
        Ok(CapabilityBatchOutcome {
            outcomes,
            stopped_on_suspension,
        })
    }
}
