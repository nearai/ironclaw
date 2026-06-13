use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    sync::{Arc, Mutex as StdMutex},
};

use chrono::Utc;
use uuid::Uuid;

use ironclaw_host_api::{
    CapabilityId, ExecutionContext, ExtensionId, InvocationId, MountView, ResourceScope,
    RuntimeKind, TrustClass, UserId,
};
use ironclaw_host_runtime::{
    CapabilitySurfacePolicy, HostRuntime, SurfaceKind,
    VisibleCapabilityRequest as HostVisibleCapabilityRequest,
};
use ironclaw_loop_support::{
    CapabilityResultWrite, HostManagedModelError, HostManagedModelErrorKind,
    HostManagedModelGateway, HostManagedModelMessageRole, HostManagedModelRequest,
    HostManagedModelResponse, HostManagedToolResultContent, LoopCapabilityInputResolver,
    LoopCapabilityPortFactory, LoopCapabilityResultWriter, loop_driver_execution_extension_id,
};
use ironclaw_threads::{
    AppendCapabilityDisplayPreviewRequest, CapabilityDisplayPreviewEnvelope,
    CapabilityDisplayPreviewEnvelopeInput, CapabilityDisplayPreviewStatus, SessionThreadService,
    ThreadMessageId, ThreadScope,
};
use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};
use ironclaw_turns::{
    LoopResultRef,
    run_profile::{
        AgentLoopHostError, AgentLoopHostErrorKind, CapabilityInputRef, LoopCapabilityPort,
        LoopHostMilestoneSink, LoopRunContext, ProviderToolCall, sanitize_model_visible_text,
    },
};

use crate::local_dev_capability_policy::LocalDevCapabilityPolicy;
use crate::local_dev_mounts::scoped_skill_management_mount_view;
use crate::{
    RebornServices,
    projection::{CapabilityDisplayPreviewResult, CapabilityDisplayPreviewStore},
    runtime::LocalDevSelectableSkillContextSource,
};

pub(super) mod extension_surface;
mod refreshing_capability_port;
#[cfg(test)]
mod shell_tests;
mod skill_activation;
mod surface_disclosure;
mod synthetic_capability;

use extension_surface::{LocalDevExtensionSurface, LocalDevExtensionSurfaceSource};
use refreshing_capability_port::{
    RefreshingLocalDevCapabilityPortConfig, create_refreshing_local_dev_capability_port,
};
#[cfg(test)]
pub(crate) use skill_activation::SKILL_ACTIVATE_CAPABILITY_ID;

pub(super) struct LocalDevCapabilityWiring {
    pub(super) capability_factory: Arc<dyn LoopCapabilityPortFactory>,
    pub(super) capability_input_resolver: Arc<dyn LoopCapabilityInputResolver>,
    pub(super) capability_result_writer: Arc<dyn LoopCapabilityResultWriter>,
    pub(super) model_gateway: Arc<dyn HostManagedModelGateway>,
    pub(super) display_previews: Arc<CapabilityDisplayPreviewStore>,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn capability_wiring(
    services: &RebornServices,
    thread_service: Arc<dyn SessionThreadService>,
    thread_scope: ThreadScope,
    fallback_user_id: UserId,
    policy: Arc<LocalDevCapabilityPolicy>,
    model_gateway: Arc<dyn HostManagedModelGateway>,
    milestone_sink: Arc<dyn LoopHostMilestoneSink>,
    skill_activation_source: Option<Arc<LocalDevSelectableSkillContextSource>>,
) -> Option<LocalDevCapabilityWiring> {
    let runtime = services.host_runtime.clone()?;
    let local_runtime = services.local_runtime.as_ref()?;
    let workspace_mounts = local_runtime.workspace_mounts.clone();
    let memory_mounts = local_runtime.memory_mounts.clone();
    let extension_surface_source =
        LocalDevExtensionSurfaceSource::new(local_runtime.extension_management.clone());
    let display_previews = Arc::new(CapabilityDisplayPreviewStore::default());
    let capability_io = Arc::new(LocalDevCapabilityIo::new_with_durable_previews(
        Arc::clone(&display_previews),
        thread_service,
        thread_scope,
    ));
    let capability_input_resolver: Arc<dyn LoopCapabilityInputResolver> = capability_io.clone();
    let capability_result_writer: Arc<dyn LoopCapabilityResultWriter> = capability_io.clone();
    let capability_factory: Arc<dyn LoopCapabilityPortFactory> =
        Arc::new(LocalDevLoopCapabilityPortFactory {
            runtime,
            fallback_user_id,
            policy,
            workspace_mounts,
            memory_mounts,
            extension_surface_source,
            input_resolver: Arc::clone(&capability_input_resolver),
            result_writer: Arc::clone(&capability_result_writer),
            milestone_sink,
            skill_activation_source,
        });
    let model_gateway: Arc<dyn HostManagedModelGateway> = Arc::new(
        LocalDevResultHydratingModelGateway::new(model_gateway, capability_io),
    );

    Some(LocalDevCapabilityWiring {
        capability_factory,
        capability_input_resolver,
        capability_result_writer,
        model_gateway,
        display_previews,
    })
}

#[derive(Clone)]
struct LocalDevLoopCapabilityPortFactory {
    runtime: Arc<dyn HostRuntime>,
    fallback_user_id: UserId,
    policy: Arc<LocalDevCapabilityPolicy>,
    workspace_mounts: MountView,
    memory_mounts: MountView,
    extension_surface_source: LocalDevExtensionSurfaceSource,
    input_resolver: Arc<dyn LoopCapabilityInputResolver>,
    result_writer: Arc<dyn LoopCapabilityResultWriter>,
    milestone_sink: Arc<dyn LoopHostMilestoneSink>,
    skill_activation_source: Option<Arc<LocalDevSelectableSkillContextSource>>,
}

#[async_trait::async_trait]
impl LoopCapabilityPortFactory for LocalDevLoopCapabilityPortFactory {
    async fn create_capability_port(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
        let skill_mounts = scoped_skill_management_mount_view(&local_dev_resource_scope_for_run(
            run_context,
            &self.fallback_user_id,
        ))
        .map_err(host_api_agent_loop_error)?;
        create_refreshing_local_dev_capability_port(RefreshingLocalDevCapabilityPortConfig {
            runtime: Arc::clone(&self.runtime),
            run_context: run_context.clone(),
            fallback_user_id: self.fallback_user_id.clone(),
            policy: Arc::clone(&self.policy),
            workspace_mounts: self.workspace_mounts.clone(),
            skill_mounts,
            memory_mounts: self.memory_mounts.clone(),
            extension_surface_source: self.extension_surface_source.clone(),
            input_resolver: Arc::clone(&self.input_resolver),
            result_writer: Arc::clone(&self.result_writer),
            milestone_sink: Arc::clone(&self.milestone_sink),
            skill_activation_source: self.skill_activation_source.clone(),
        })
        .await
    }
}

const LOCAL_DEV_CAPABILITY_IO_MAX_STAGED_REFS: usize = 1024;
const LOCAL_DEV_CAPABILITY_IO_MAX_STAGED_BYTES: usize = 4 * 1024 * 1024;
// Replay payload cap for provider calls. This is a model-window guard, not a
// safe-summary formatter; the staged result remains available for follow-up.
const LOCAL_DEV_MODEL_VISIBLE_TOOL_RESULT_MAX_BYTES: usize = 100_000;

struct LocalDevCapabilityIo {
    inputs: StdMutex<StagedValueStore>,
    results: StdMutex<StagedValueStore>,
    display_previews: Arc<CapabilityDisplayPreviewStore>,
    durable_previews: Option<DurableCapabilityDisplayPreviewSink>,
}

#[derive(Clone)]
struct DurableCapabilityDisplayPreviewSink {
    thread_service: Arc<dyn SessionThreadService>,
    thread_scope: ThreadScope,
}

impl Default for LocalDevCapabilityIo {
    fn default() -> Self {
        Self::new(Arc::new(CapabilityDisplayPreviewStore::default()))
    }
}

impl LocalDevCapabilityIo {
    fn new(display_previews: Arc<CapabilityDisplayPreviewStore>) -> Self {
        Self {
            inputs: StdMutex::new(StagedValueStore::default()),
            results: StdMutex::new(StagedValueStore::default()),
            display_previews,
            durable_previews: None,
        }
    }

    fn new_with_durable_previews(
        display_previews: Arc<CapabilityDisplayPreviewStore>,
        thread_service: Arc<dyn SessionThreadService>,
        thread_scope: ThreadScope,
    ) -> Self {
        Self {
            inputs: StdMutex::new(StagedValueStore::default()),
            results: StdMutex::new(StagedValueStore::default()),
            display_previews,
            durable_previews: Some(DurableCapabilityDisplayPreviewSink {
                thread_service,
                thread_scope,
            }),
        }
    }

    fn result_output(
        &self,
        result_ref: &str,
    ) -> Result<Option<serde_json::Value>, AgentLoopHostError> {
        self.results
            .lock()
            .map_err(|_| capability_io_error())
            .map(|results| results.get(result_ref).cloned())
    }

    async fn try_append_durable_display_preview(
        &self,
        run_context: &LoopRunContext,
        invocation_id: InvocationId,
        capability_id: &CapabilityId,
    ) -> Option<ThreadMessageId> {
        let Some(durable_previews) = &self.durable_previews else {
            return None;
        };
        let Some(record) = self.display_previews.record_for_invocation(invocation_id) else {
            tracing::debug!(
                invocation_id = %invocation_id,
                capability_id = capability_id.as_str(),
                "capability display preview record missing after result staging"
            );
            return None;
        };
        let preview =
            match CapabilityDisplayPreviewEnvelope::new(CapabilityDisplayPreviewEnvelopeInput {
                invocation_id,
                capability_id: capability_id.clone(),
                status: CapabilityDisplayPreviewStatus::Completed,
                title: record.title,
                subtitle: record.subtitle,
                input_summary: record.input_summary,
                output_summary: record.output_summary,
                output_preview: record.output_preview,
                output_kind: record.output_kind,
                output_bytes: record.output_bytes,
                result_ref: record.result_ref,
                truncated: record.truncated,
                updated_at: Utc::now(),
            }) {
                Ok(preview) => preview,
                Err(error) => {
                    tracing::debug!(
                        invocation_id = %invocation_id,
                        capability_id = capability_id.as_str(),
                        error,
                        "capability display preview envelope validation failed"
                    );
                    return None;
                }
            };
        let message = match durable_previews
            .thread_service
            .append_capability_display_preview(AppendCapabilityDisplayPreviewRequest {
                scope: durable_previews.thread_scope.clone(),
                thread_id: run_context.thread_id.clone(),
                turn_run_id: run_context.run_id.to_string(),
                preview,
            })
            .await
        {
            Ok(message) => message,
            Err(error) => {
                tracing::debug!(
                    invocation_id = %invocation_id,
                    capability_id = capability_id.as_str(),
                    error = %error,
                    "capability display preview durable append failed; continuing with staged capability result"
                );
                return None;
            }
        };
        Some(message.message_id)
    }
}

#[derive(Default)]
struct StagedValueStore {
    values: HashMap<String, StagedValue>,
    // Eviction index only, not an execution queue. Inputs fail closed and never
    // evict; results use this to drop oldest staged refs under byte pressure.
    oldest_refs: VecDeque<String>,
    total_bytes: usize,
}

struct StagedValue {
    value: serde_json::Value,
    bytes: usize,
}

impl StagedValueStore {
    fn get(&self, reference: &str) -> Option<&serde_json::Value> {
        self.values.get(reference).map(|staged| &staged.value)
    }

    fn insert_without_eviction(
        &mut self,
        reference: String,
        value: serde_json::Value,
    ) -> Result<(), AgentLoopHostError> {
        let bytes = staged_value_bytes(&value)?;
        if self.values.len() >= LOCAL_DEV_CAPABILITY_IO_MAX_STAGED_REFS
            || self.total_bytes.saturating_add(bytes) > LOCAL_DEV_CAPABILITY_IO_MAX_STAGED_BYTES
        {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::BudgetExceeded,
                "local-dev capability staging is full",
            ));
        }
        self.insert_measured(reference, value, bytes);
        Ok(())
    }

    fn insert_with_oldest_eviction(
        &mut self,
        reference: String,
        value: serde_json::Value,
    ) -> Result<(), AgentLoopHostError> {
        let bytes = staged_value_bytes(&value)?;
        if bytes > LOCAL_DEV_CAPABILITY_IO_MAX_STAGED_BYTES {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::BudgetExceeded,
                "local-dev capability result exceeds staging budget",
            ));
        }
        while self.values.len() >= LOCAL_DEV_CAPABILITY_IO_MAX_STAGED_REFS
            || self.total_bytes.saturating_add(bytes) > LOCAL_DEV_CAPABILITY_IO_MAX_STAGED_BYTES
        {
            self.evict_oldest();
        }
        self.insert_measured(reference, value, bytes);
        Ok(())
    }

    fn insert_measured(&mut self, reference: String, value: serde_json::Value, bytes: usize) {
        if let Some(previous) = self.values.remove(&reference) {
            self.total_bytes = self.total_bytes.saturating_sub(previous.bytes);
            self.oldest_refs.retain(|candidate| candidate != &reference);
        }
        self.total_bytes = self.total_bytes.saturating_add(bytes);
        self.oldest_refs.push_back(reference.clone());
        self.values.insert(reference, StagedValue { value, bytes });
    }

    fn evict_oldest(&mut self) {
        while let Some(reference) = self.oldest_refs.pop_front() {
            if let Some(previous) = self.values.remove(&reference) {
                self.total_bytes = self.total_bytes.saturating_sub(previous.bytes);
                return;
            }
        }
    }

    fn remove(&mut self, reference: &str) {
        if let Some(previous) = self.values.remove(reference) {
            self.total_bytes = self.total_bytes.saturating_sub(previous.bytes);
            self.oldest_refs.retain(|candidate| candidate != reference);
        }
    }
}

fn staged_value_bytes(value: &serde_json::Value) -> Result<usize, AgentLoopHostError> {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .map_err(|error| {
            ironclaw_loop_support::raw_agent_loop_host_error(
                "local_dev_capability_io",
                "measure_payload",
                AgentLoopHostErrorKind::InvalidInvocation,
                "capability payload could not be measured",
                error,
            )
        })
}

#[async_trait::async_trait]
impl LoopCapabilityInputResolver for LocalDevCapabilityIo {
    async fn resolve_capability_input(
        &self,
        run_context: &LoopRunContext,
        input_ref: &CapabilityInputRef,
    ) -> Result<serde_json::Value, AgentLoopHostError> {
        ensure_local_dev_ref_scope("input", input_ref.as_str(), run_context)?;
        let inputs = self.inputs.lock().map_err(|_| capability_io_error())?;
        inputs.get(input_ref.as_str()).cloned().ok_or_else(|| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "capability input ref was not staged for this loop run",
            )
        })
    }

    async fn register_provider_tool_call_input(
        &self,
        run_context: &LoopRunContext,
        tool_call: &ProviderToolCall,
    ) -> Result<CapabilityInputRef, AgentLoopHostError> {
        let input_ref =
            CapabilityInputRef::new(format!("input:{}:{}", run_context.run_id, Uuid::new_v4()))
                .map_err(|_| {
                    AgentLoopHostError::new(
                        AgentLoopHostErrorKind::Internal,
                        "capability input ref could not be represented",
                    )
                })?;
        let mut inputs = self.inputs.lock().map_err(|_| capability_io_error())?;
        inputs
            .insert_without_eviction(input_ref.as_str().to_string(), tool_call.arguments.clone())?;
        self.display_previews.record_input(
            &run_context.run_id.to_string(),
            &input_ref,
            &tool_call.name,
            &tool_call.arguments,
        );
        Ok(input_ref)
    }
}

#[async_trait::async_trait]
impl LoopCapabilityResultWriter for LocalDevCapabilityIo {
    async fn write_capability_result(
        &self,
        write: CapabilityResultWrite<'_>,
    ) -> Result<(LoopResultRef, u64), AgentLoopHostError> {
        let CapabilityResultWrite {
            run_context,
            input_ref,
            invocation_id,
            capability_id,
            output,
            display_preview,
        } = write;
        let result_ref =
            LoopResultRef::new(format!("result:{}.{}", run_context.run_id, Uuid::new_v4()))
                .map_err(|_| {
                    AgentLoopHostError::new(
                        AgentLoopHostErrorKind::Internal,
                        "capability result ref could not be represented",
                    )
                })?;
        let output_bytes = staged_value_bytes(&output)?.try_into().unwrap_or(u64::MAX);
        {
            let mut results = self.results.lock().map_err(|_| capability_io_error())?;
            results.insert_with_oldest_eviction(result_ref.as_str().to_string(), output.clone())?;
        }
        self.display_previews.record_result_with_preview(
            CapabilityDisplayPreviewResult {
                run_id: &run_context.run_id.to_string(),
                input_ref,
                invocation_id,
                capability_id,
                result_ref: result_ref.as_str(),
                output: &output,
                output_bytes,
            },
            display_preview.as_ref(),
        );
        if let Some(message_id) = self
            .try_append_durable_display_preview(run_context, invocation_id, capability_id)
            .await
        {
            self.display_previews
                .attach_timeline_message_id(invocation_id, message_id);
        }
        Ok((result_ref, output_bytes))
    }

    async fn update_capability_result(
        &self,
        run_context: &LoopRunContext,
        result_ref: &LoopResultRef,
        output: serde_json::Value,
    ) -> Result<u64, AgentLoopHostError> {
        ensure_local_dev_ref_scope("result", result_ref.as_str(), run_context)?;
        let bytes = staged_value_bytes(&output)?;
        let mut results = self.results.lock().map_err(|_| capability_io_error())?;
        let previous_bytes = results
            .values
            .get(result_ref.as_str())
            .map(|previous| previous.bytes)
            .unwrap_or(0);
        let next_total = results
            .total_bytes
            .saturating_sub(previous_bytes)
            .saturating_add(bytes);
        if next_total > LOCAL_DEV_CAPABILITY_IO_MAX_STAGED_BYTES
            || (previous_bytes == 0
                && results.values.len() >= LOCAL_DEV_CAPABILITY_IO_MAX_STAGED_REFS)
        {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::BudgetExceeded,
                "local-dev capability result exceeds staging budget",
            ));
        }
        results.insert_measured(result_ref.as_str().to_string(), output, bytes);
        Ok(bytes as u64)
    }

    async fn delete_capability_result(
        &self,
        run_context: &LoopRunContext,
        result_ref: &LoopResultRef,
    ) -> Result<(), AgentLoopHostError> {
        ensure_local_dev_ref_scope("result", result_ref.as_str(), run_context)?;
        self.results
            .lock()
            .map_err(|_| capability_io_error())?
            .remove(result_ref.as_str());
        Ok(())
    }
}

/// Local-dev replay shim for model-visible tool results.
///
/// Thread transcripts store safe result refs. This runtime-local shim dereferences outputs staged
/// by `LocalDevCapabilityIo` before delegating to the selected model gateway, so REPL follow-up
/// turns see actual host-runtime tool output without making CLI own capability storage.
#[derive(Clone)]
struct LocalDevResultHydratingModelGateway {
    inner: Arc<dyn HostManagedModelGateway>,
    capability_io: Arc<LocalDevCapabilityIo>,
}

impl LocalDevResultHydratingModelGateway {
    fn new(
        inner: Arc<dyn HostManagedModelGateway>,
        capability_io: Arc<LocalDevCapabilityIo>,
    ) -> Self {
        Self {
            inner,
            capability_io,
        }
    }

    fn hydrate_request(
        &self,
        request: HostManagedModelRequest,
    ) -> Result<HostManagedModelRequest, HostManagedModelError> {
        hydrate_tool_result_messages(request, self.capability_io.as_ref())
    }
}

#[async_trait::async_trait]
impl HostManagedModelGateway for LocalDevResultHydratingModelGateway {
    async fn stream_model(
        &self,
        request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        self.inner
            .stream_model(self.hydrate_request(request)?)
            .await
    }

    async fn stream_model_with_capabilities(
        &self,
        request: HostManagedModelRequest,
        capabilities: Arc<dyn LoopCapabilityPort>,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        self.inner
            .stream_model_with_capabilities(self.hydrate_request(request)?, capabilities)
            .await
    }
}

fn hydrate_tool_result_messages(
    mut request: HostManagedModelRequest,
    capability_io: &LocalDevCapabilityIo,
) -> Result<HostManagedModelRequest, HostManagedModelError> {
    for message in &mut request.messages {
        if message.role != HostManagedModelMessageRole::ToolResult {
            continue;
        }
        let envelope = match message.tool_result_content.as_ref() {
            Some(HostManagedToolResultContent::Reference { envelope }) => envelope,
            Some(HostManagedToolResultContent::Resolved { .. }) => continue,
            None => {
                return Err(HostManagedModelError::safe(
                    HostManagedModelErrorKind::InvalidRequest,
                    "tool result replay content is missing",
                ));
            }
        };
        let output = capability_io
            .result_output(&envelope.result_ref)
            .map_err(model_capability_io_error)?;
        let Some(output) = output else {
            continue;
        };
        message.content = model_visible_tool_result_content(&output)?;
        message.tool_result_content = Some(HostManagedToolResultContent::Resolved {
            safe_summary: envelope.safe_summary.clone(),
        });
    }
    Ok(request)
}

fn model_visible_tool_result_content(
    output: &serde_json::Value,
) -> Result<String, HostManagedModelError> {
    let mut content =
        String::with_capacity(LOCAL_DEV_MODEL_VISIBLE_TOOL_RESULT_MAX_BYTES.min(4096));
    let truncated = append_model_visible_value(output, &mut content);
    if content.is_empty() {
        return Err(HostManagedModelError::safe(
            HostManagedModelErrorKind::InvalidRequest,
            "tool result output could not be represented for model replay",
        ));
    }
    if truncated {
        content.push_str("\n\n[... truncated: showing first ");
        content.push_str(&LOCAL_DEV_MODEL_VISIBLE_TOOL_RESULT_MAX_BYTES.to_string());
        content.push_str(" bytes. Use a follow-up tool call to inspect the full result.]");
    }
    Ok(content)
}

fn append_model_visible_value(value: &serde_json::Value, output: &mut String) -> bool {
    if output.len() >= LOCAL_DEV_MODEL_VISIBLE_TOOL_RESULT_MAX_BYTES {
        return true;
    }
    match value {
        serde_json::Value::Null => append_sanitized_capped(" null", output),
        serde_json::Value::Bool(value) => append_sanitized_capped(&format!(" {value}"), output),
        serde_json::Value::Number(value) => append_sanitized_capped(&format!(" {value}"), output),
        serde_json::Value::String(value) => append_sanitized_capped(&format!(" {value}"), output),
        serde_json::Value::Array(values) => {
            if append_sanitized_capped(" array", output) {
                return true;
            }
            for value in values {
                if append_model_visible_value(value, output) {
                    return true;
                }
            }
            false
        }
        serde_json::Value::Object(values) => {
            if append_sanitized_capped(" object", output) {
                return true;
            }
            for (key, value) in values {
                if append_sanitized_capped(&format!(" {key}"), output)
                    || append_model_visible_value(value, output)
                {
                    return true;
                }
            }
            false
        }
    }
}

fn append_sanitized_capped(value: &str, output: &mut String) -> bool {
    let value = sanitize_model_visible_text(value.to_string());
    for character in value.chars() {
        if output.len() >= LOCAL_DEV_MODEL_VISIBLE_TOOL_RESULT_MAX_BYTES {
            return true;
        }
        let character = if character.is_control()
            || matches!(
                character,
                '{' | '}' | '[' | ']' | '`' | '<' | '>' | '/' | '\\'
            ) {
            ' '
        } else {
            character
        };
        if output.len() + character.len_utf8() > LOCAL_DEV_MODEL_VISIBLE_TOOL_RESULT_MAX_BYTES {
            return true;
        }
        output.push(character);
    }
    false
}

fn model_capability_io_error(error: AgentLoopHostError) -> HostManagedModelError {
    HostManagedModelError::safe(HostManagedModelErrorKind::Unavailable, error.safe_summary)
}

fn local_dev_resource_scope_for_run(
    run_context: &LoopRunContext,
    fallback_user_id: &UserId,
) -> ResourceScope {
    let mut scope = run_context.scope.to_resource_scope();
    scope.user_id = run_context
        .scope
        .explicit_owner_user_id()
        .cloned()
        .or_else(|| run_context.actor().map(|actor| actor.user_id.clone()))
        .unwrap_or_else(|| fallback_user_id.clone());
    scope
}

fn local_dev_visible_capability_request(
    run_context: &LoopRunContext,
    fallback_user_id: &UserId,
    workspace_mounts: MountView,
    skill_mounts: MountView,
    memory_mounts: MountView,
    policy: &LocalDevCapabilityPolicy,
    extension_surface: &LocalDevExtensionSurface,
) -> Result<HostVisibleCapabilityRequest, AgentLoopHostError> {
    let extension_id = loop_driver_execution_extension_id(run_context)?;
    let mut grants = policy.builtin_grants(
        &extension_id,
        &workspace_mounts,
        &skill_mounts,
        &memory_mounts,
    );
    grants
        .grants
        .extend(extension_surface.grants(&extension_id));
    let user_id = run_context
        .scope
        .explicit_owner_user_id()
        .cloned()
        .or_else(|| run_context.actor().map(|actor| actor.user_id.clone()))
        .unwrap_or_else(|| fallback_user_id.clone());
    let mut context = ExecutionContext::local_default(
        user_id,
        extension_id,
        RuntimeKind::FirstParty,
        TrustClass::UserTrusted,
        grants,
        MountView::default(),
    )
    .map_err(host_api_agent_loop_error)?;
    context.tenant_id = run_context.scope.tenant_id.clone();
    context.agent_id = run_context.scope.agent_id.clone();
    context.project_id = run_context.scope.project_id.clone();
    context.thread_id = Some(run_context.thread_id.clone());
    context.resource_scope.tenant_id = context.tenant_id.clone();
    context.resource_scope.agent_id = context.agent_id.clone();
    context.resource_scope.project_id = context.project_id.clone();
    context.resource_scope.thread_id = context.thread_id.clone();
    context.validate().map_err(host_api_agent_loop_error)?;

    let builtin_provider =
        ExtensionId::new(policy.provider.id.as_str()).map_err(host_api_agent_loop_error)?;
    let mut provider_trust = BTreeMap::new();
    provider_trust.insert(
        builtin_provider,
        TrustDecision {
            effective_trust: EffectiveTrustClass::user_trusted(),
            authority_ceiling: AuthorityCeiling {
                allowed_effects: policy.provider.authority_effects.clone(),
                max_resource_ceiling: None,
            },
            provenance: TrustProvenance::AdminConfig,
            evaluated_at: Utc::now(),
        },
    );
    provider_trust.extend(extension_surface.provider_trust());

    Ok(HostVisibleCapabilityRequest::new(
        context,
        SurfaceKind::new("agent_loop").map_err(host_api_agent_loop_error)?,
    )
    .with_policy(CapabilitySurfacePolicy::allow_all())
    .with_provider_trust(provider_trust))
}

fn ensure_local_dev_ref_scope(
    prefix: &str,
    reference: &str,
    run_context: &LoopRunContext,
) -> Result<(), AgentLoopHostError> {
    // Match product_live_adapters' convention: result refs are
    // `result:<run_id>.<uuid>` (dot) so they tokenize cleanly when a uuid
    // contains hyphens, while input refs stay `input:<run_id>:<n>` (colon).
    // Keep this in sync with `ensure_ref_scoped_to_run` in
    // `product_live_adapters.rs`.
    let separator = if prefix == "result" { "." } else { ":" };
    let expected_prefix = format!("{prefix}:{}{separator}", run_context.run_id);
    if reference.starts_with(&expected_prefix) {
        Ok(())
    } else {
        Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::ScopeMismatch,
            "capability input ref is not scoped to this loop run",
        ))
    }
}

fn capability_io_error() -> AgentLoopHostError {
    AgentLoopHostError::new(
        AgentLoopHostErrorKind::Internal,
        "capability io store is unavailable",
    )
}

fn host_api_agent_loop_error(
    error: impl std::fmt::Debug + std::fmt::Display,
) -> AgentLoopHostError {
    let safe_summary = error.to_string();
    ironclaw_loop_support::raw_agent_loop_host_error(
        "local_dev_host_api",
        "validate_local_dev_runtime_input",
        AgentLoopHostErrorKind::InvalidInvocation,
        safe_summary,
        format!("{error:?}"),
    )
}

#[cfg(test)]
mod tests;
