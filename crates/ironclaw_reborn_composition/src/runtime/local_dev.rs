use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    sync::{Arc, Mutex as StdMutex},
};

use chrono::Utc;
use uuid::Uuid;

use ironclaw_host_api::{
    CapabilityGrant, CapabilityGrantId, CapabilityId, CapabilitySet, EffectKind, ExecutionContext,
    ExtensionId, GrantConstraints, InvocationId, MountView, NetworkPolicy, NetworkTargetPattern,
    Principal, RuntimeKind, TrustClass, UserId,
};
use ironclaw_host_runtime::{
    APPLY_PATCH_CAPABILITY_ID, CapabilitySurfacePolicy, ECHO_CAPABILITY_ID, GLOB_CAPABILITY_ID,
    GREP_CAPABILITY_ID, HTTP_CAPABILITY_ID, HostRuntime, JSON_CAPABILITY_ID,
    LIST_DIR_CAPABILITY_ID, READ_FILE_CAPABILITY_ID, SHELL_CAPABILITY_ID,
    SKILL_INSTALL_CAPABILITY_ID, SKILL_LIST_CAPABILITY_ID, SKILL_REMOVE_CAPABILITY_ID, SurfaceKind,
    TIME_CAPABILITY_ID, VisibleCapabilityRequest as HostVisibleCapabilityRequest,
    WRITE_FILE_CAPABILITY_ID,
};
use ironclaw_loop_support::{
    HostManagedModelError, HostManagedModelErrorKind, HostManagedModelGateway,
    HostManagedModelMessageRole, HostManagedModelRequest, HostManagedModelResponse,
    HostRuntimeLoopCapabilityPortFactory, LoopCapabilityInputResolver, LoopCapabilityResultWriter,
    loop_driver_execution_extension_id,
};
use ironclaw_reborn::loop_driver_host::LoopCapabilityPortFactory;
use ironclaw_threads::{
    AppendCapabilityDisplayPreviewRequest, CapabilityDisplayPreviewEnvelope,
    CapabilityDisplayPreviewEnvelopeInput, CapabilityDisplayPreviewStatus, SessionThreadService,
    ThreadScope, ToolResultReferenceEnvelope, ToolResultSafeSummary,
};
use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};
use ironclaw_turns::{
    LoopResultRef,
    run_profile::{
        AgentLoopHostError, AgentLoopHostErrorKind, CapabilityInputRef, LoopCapabilityPort,
        LoopHostMilestoneSink, LoopRunContext, ProviderToolCall,
    },
};

use crate::local_dev_mounts::skill_management_mount_view;
use crate::{
    RebornServices,
    projection::{CapabilityDisplayPreviewResult, CapabilityDisplayPreviewStore},
};

pub(super) struct LocalDevCapabilityWiring {
    pub(super) capability_factory: Arc<dyn LoopCapabilityPortFactory>,
    pub(super) capability_input_resolver: Arc<dyn LoopCapabilityInputResolver>,
    pub(super) capability_result_writer: Arc<dyn LoopCapabilityResultWriter>,
    pub(super) model_gateway: Arc<dyn HostManagedModelGateway>,
    pub(super) display_previews: Arc<CapabilityDisplayPreviewStore>,
}

pub(super) fn capability_wiring(
    services: &RebornServices,
    thread_service: Arc<dyn SessionThreadService>,
    thread_scope: ThreadScope,
    user_id: UserId,
    model_gateway: Arc<dyn HostManagedModelGateway>,
    milestone_sink: Arc<dyn LoopHostMilestoneSink>,
) -> Option<LocalDevCapabilityWiring> {
    let runtime = services.host_runtime.clone()?;
    let workspace_mounts = services
        .local_runtime
        .as_ref()
        .map(|runtime| runtime.workspace_mounts.clone())?;
    let display_previews = Arc::new(CapabilityDisplayPreviewStore::default());
    let capability_io = Arc::new(LocalDevCapabilityIo::new_with_durable_previews(
        Arc::clone(&display_previews),
        thread_service,
        thread_scope,
    ));
    let capability_input_resolver: Arc<dyn LoopCapabilityInputResolver> = capability_io.clone();
    let capability_result_writer: Arc<dyn LoopCapabilityResultWriter> = capability_io.clone();
    let capability_factory: Arc<dyn LoopCapabilityPortFactory> =
        Arc::new(LocalDevLoopCapabilityPortFactory::new(
            runtime,
            user_id,
            workspace_mounts,
            Arc::clone(&capability_input_resolver),
            Arc::clone(&capability_result_writer),
            milestone_sink,
        ));
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
    user_id: UserId,
    workspace_mounts: MountView,
    input_resolver: Arc<dyn LoopCapabilityInputResolver>,
    result_writer: Arc<dyn LoopCapabilityResultWriter>,
    milestone_sink: Arc<dyn LoopHostMilestoneSink>,
}

impl LocalDevLoopCapabilityPortFactory {
    fn new(
        runtime: Arc<dyn HostRuntime>,
        user_id: UserId,
        workspace_mounts: MountView,
        input_resolver: Arc<dyn LoopCapabilityInputResolver>,
        result_writer: Arc<dyn LoopCapabilityResultWriter>,
        milestone_sink: Arc<dyn LoopHostMilestoneSink>,
    ) -> Self {
        Self {
            runtime,
            user_id,
            workspace_mounts,
            input_resolver,
            result_writer,
            milestone_sink,
        }
    }
}

#[async_trait::async_trait]
impl LoopCapabilityPortFactory for LocalDevLoopCapabilityPortFactory {
    async fn create_capability_port(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
        let workspace_mounts = self.workspace_mounts.clone();
        let skill_mounts = skill_management_mount_view().map_err(host_api_agent_loop_error)?;
        let visible_request = local_dev_visible_capability_request(
            run_context,
            self.user_id.clone(),
            workspace_mounts.clone(),
            skill_mounts.clone(),
        )?;
        let mut factory = HostRuntimeLoopCapabilityPortFactory::new(
            Arc::clone(&self.runtime),
            visible_request,
            Arc::clone(&self.input_resolver),
            Arc::clone(&self.result_writer),
            Arc::clone(&self.milestone_sink),
        )
        .with_execution_mounts(workspace_mounts);
        for capability_id in local_dev_skill_management_capability_ids() {
            factory = factory.with_capability_execution_mount(
                CapabilityId::new(capability_id).map_err(host_api_agent_loop_error)?,
                skill_mounts.clone(),
            );
        }
        Ok(factory.for_run_context(run_context.clone()))
    }
}

const LOCAL_DEV_CAPABILITY_IO_MAX_STAGED_REFS: usize = 1024;
const LOCAL_DEV_CAPABILITY_IO_MAX_STAGED_BYTES: usize = 4 * 1024 * 1024;
const MODEL_VISIBLE_TOOL_OUTPUT_MAX_BYTES: usize = 480;

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

    async fn append_durable_display_preview(
        &self,
        run_context: &LoopRunContext,
        invocation_id: InvocationId,
        capability_id: &CapabilityId,
    ) -> Result<(), AgentLoopHostError> {
        let Some(durable_previews) = &self.durable_previews else {
            return Ok(());
        };
        let Some(record) = self.display_previews.record_for_invocation(invocation_id) else {
            tracing::debug!(
                invocation_id = %invocation_id,
                capability_id = capability_id.as_str(),
                "capability display preview record missing after result staging"
            );
            return Ok(());
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
                    return Err(capability_io_error());
                }
            };
        let message = durable_previews
            .thread_service
            .append_capability_display_preview(AppendCapabilityDisplayPreviewRequest {
                scope: durable_previews.thread_scope.clone(),
                thread_id: run_context.thread_id.clone(),
                turn_run_id: run_context.run_id.to_string(),
                preview,
            })
            .await
            .map_err(|error| {
                tracing::debug!(
                    invocation_id = %invocation_id,
                    capability_id = capability_id.as_str(),
                    error = %error,
                    "capability display preview durable append failed"
                );
                capability_io_error()
            })?;
        self.display_previews
            .attach_timeline_message_id(invocation_id, message.message_id);
        Ok(())
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
        run_context: &LoopRunContext,
        input_ref: &CapabilityInputRef,
        invocation_id: InvocationId,
        _capability_id: &CapabilityId,
        output: serde_json::Value,
    ) -> Result<LoopResultRef, AgentLoopHostError> {
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
        self.display_previews
            .record_result(CapabilityDisplayPreviewResult {
                run_id: &run_context.run_id.to_string(),
                input_ref,
                invocation_id,
                capability_id: _capability_id,
                result_ref: result_ref.as_str(),
                output: &output,
                output_bytes,
            });
        self.append_durable_display_preview(run_context, invocation_id, _capability_id)
            .await?;
        Ok(result_ref)
    }

    async fn update_capability_result(
        &self,
        run_context: &LoopRunContext,
        result_ref: &LoopResultRef,
        output: serde_json::Value,
    ) -> Result<(), AgentLoopHostError> {
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
        Ok(())
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
        let mut envelope: ToolResultReferenceEnvelope = serde_json::from_str(&message.content)
            .map_err(|error| {
                ironclaw_loop_support::raw_host_managed_model_error(
                    "local_dev_model_replay",
                    "decode_tool_result_envelope",
                    HostManagedModelErrorKind::InvalidRequest,
                    "tool result reference transcript content is invalid",
                    error,
                )
            })?;
        let output = capability_io
            .result_output(&envelope.result_ref)
            .map_err(model_capability_io_error)?;
        let Some(output) = output else {
            continue;
        };
        envelope.safe_summary = ToolResultSafeSummary::new(model_visible_tool_output(&output))
            .map_err(|error| {
                ironclaw_loop_support::raw_host_managed_model_error(
                    "local_dev_model_replay",
                    "sanitize_tool_result",
                    HostManagedModelErrorKind::InvalidRequest,
                    "tool result output could not be represented safely for model replay",
                    error,
                )
            })?;
        message.content = serde_json::to_string(&envelope).map_err(|error| {
            let safe_summary = error.to_string();
            ironclaw_loop_support::raw_host_managed_model_error(
                "local_dev_model_replay",
                "encode_tool_result_envelope",
                HostManagedModelErrorKind::InvalidRequest,
                safe_summary,
                error,
            )
        })?;
    }
    Ok(request)
}

/// Convert local-dev tool output into a `ToolResultSafeSummary`-compatible replay string.
/// This is not product-live canonical result storage; it is a bounded local-dev bridge so provider
/// follow-up calls receive useful output while preserving the transcript safe-summary contract.
fn model_visible_tool_output(output: &serde_json::Value) -> String {
    let mut sanitized = String::from("tool output");
    append_model_visible_value(output, &mut sanitized);
    let sanitized = sanitized.split_whitespace().collect::<Vec<_>>().join(" ");
    if ToolResultSafeSummary::new(sanitized.clone()).is_ok() {
        sanitized
    } else {
        "tool output available".to_string()
    }
}

fn append_model_visible_value(value: &serde_json::Value, output: &mut String) {
    if output.len() >= MODEL_VISIBLE_TOOL_OUTPUT_MAX_BYTES {
        return;
    }
    match value {
        serde_json::Value::Null => append_sanitized_capped(" null", output),
        serde_json::Value::Bool(value) => append_sanitized_capped(&format!(" {value}"), output),
        serde_json::Value::Number(value) => append_sanitized_capped(&format!(" {value}"), output),
        serde_json::Value::String(value) => append_sanitized_capped(&format!(" {value}"), output),
        serde_json::Value::Array(values) => {
            for value in values {
                append_model_visible_value(value, output);
                if output.len() >= MODEL_VISIBLE_TOOL_OUTPUT_MAX_BYTES {
                    break;
                }
            }
        }
        serde_json::Value::Object(values) => {
            for (key, value) in values {
                append_sanitized_capped(&format!(" {key}"), output);
                append_model_visible_value(value, output);
                if output.len() >= MODEL_VISIBLE_TOOL_OUTPUT_MAX_BYTES {
                    break;
                }
            }
        }
    }
}

fn append_sanitized_capped(value: &str, output: &mut String) {
    for character in value.chars() {
        if output.len() >= MODEL_VISIBLE_TOOL_OUTPUT_MAX_BYTES {
            break;
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
        if output.len() + character.len_utf8() > MODEL_VISIBLE_TOOL_OUTPUT_MAX_BYTES {
            break;
        }
        output.push(character);
    }
}

fn model_capability_io_error(error: AgentLoopHostError) -> HostManagedModelError {
    HostManagedModelError::safe(HostManagedModelErrorKind::Unavailable, error.safe_summary)
}

fn local_dev_visible_capability_request(
    run_context: &LoopRunContext,
    user_id: UserId,
    workspace_mounts: MountView,
    skill_mounts: MountView,
) -> Result<HostVisibleCapabilityRequest, AgentLoopHostError> {
    let extension_id = loop_driver_execution_extension_id(run_context)?;
    let grants = local_dev_builtin_grants(&extension_id, &workspace_mounts, &skill_mounts)?;
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

    let builtin_provider = ExtensionId::new("builtin").map_err(host_api_agent_loop_error)?;
    let mut provider_trust = BTreeMap::new();
    provider_trust.insert(
        builtin_provider,
        TrustDecision {
            effective_trust: EffectiveTrustClass::user_trusted(),
            authority_ceiling: AuthorityCeiling {
                allowed_effects: local_dev_provider_allowed_effects(),
                max_resource_ceiling: None,
            },
            provenance: TrustProvenance::AdminConfig,
            evaluated_at: Utc::now(),
        },
    );

    Ok(HostVisibleCapabilityRequest::new(
        context,
        SurfaceKind::new("agent_loop").map_err(host_api_agent_loop_error)?,
    )
    .with_policy(CapabilitySurfacePolicy::allow_all())
    .with_provider_trust(provider_trust))
}

fn local_dev_builtin_grants(
    grantee: &ExtensionId,
    workspace_mounts: &MountView,
    skill_mounts: &MountView,
) -> Result<CapabilitySet, AgentLoopHostError> {
    let mut grants = Vec::new();
    for capability_id in local_dev_builtin_capability_ids() {
        grants.push(CapabilityGrant {
            id: CapabilityGrantId::new(),
            capability: CapabilityId::new(capability_id).map_err(host_api_agent_loop_error)?,
            grantee: Principal::Extension(grantee.clone()),
            issued_by: Principal::HostRuntime,
            constraints: local_dev_grant_constraints(capability_id, workspace_mounts, skill_mounts),
        });
    }
    Ok(CapabilitySet { grants })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LocalDevCapabilityKind {
    Workspace,
    AmbientShell,
    Network,
    SkillManagement,
}

fn local_dev_capability_kind(capability_id: &str) -> LocalDevCapabilityKind {
    if capability_id == SHELL_CAPABILITY_ID {
        LocalDevCapabilityKind::AmbientShell
    } else if capability_id == HTTP_CAPABILITY_ID {
        LocalDevCapabilityKind::Network
    } else if capability_id == SKILL_LIST_CAPABILITY_ID
        || capability_id == SKILL_INSTALL_CAPABILITY_ID
        || capability_id == SKILL_REMOVE_CAPABILITY_ID
    {
        LocalDevCapabilityKind::SkillManagement
    } else {
        LocalDevCapabilityKind::Workspace
    }
}

fn local_dev_skill_management_capability_ids() -> impl Iterator<Item = &'static str> {
    local_dev_builtin_capability_ids()
        .into_iter()
        .filter(|capability_id| {
            local_dev_capability_kind(capability_id) == LocalDevCapabilityKind::SkillManagement
        })
}

pub(super) fn local_dev_grant_constraints(
    capability_id: &str,
    workspace_mounts: &MountView,
    skill_mounts: &MountView,
) -> GrantConstraints {
    match local_dev_capability_kind(capability_id) {
        LocalDevCapabilityKind::AmbientShell => GrantConstraints {
            allowed_effects: local_dev_shell_allowed_effects(),
            // The first-party shell handler still uses direct host process
            // execution. It fails closed when scoped mounts are attached
            // because it cannot safely translate virtual cwd values like
            // `/workspace` to host paths yet. Local-dev exposes shell as an
            // explicitly ambient developer escape hatch until mount-aware
            // process execution lands.
            mounts: MountView::default(),
            network: local_dev_shell_network_policy(),
            secrets: Vec::new(),
            resource_ceiling: None,
            expires_at: None,
            max_invocations: None,
        },
        LocalDevCapabilityKind::Network => GrantConstraints {
            allowed_effects: local_dev_network_allowed_effects(),
            mounts: MountView::default(),
            network: local_dev_shell_network_policy(),
            secrets: Vec::new(),
            resource_ceiling: None,
            expires_at: None,
            max_invocations: None,
        },
        LocalDevCapabilityKind::Workspace => GrantConstraints {
            allowed_effects: local_dev_allowed_effects(),
            mounts: workspace_mounts.clone(),
            network: NetworkPolicy::default(),
            secrets: Vec::new(),
            resource_ceiling: None,
            expires_at: None,
            max_invocations: None,
        },
        LocalDevCapabilityKind::SkillManagement => GrantConstraints {
            allowed_effects: local_dev_allowed_effects(),
            mounts: skill_mounts.clone(),
            network: NetworkPolicy::default(),
            secrets: Vec::new(),
            resource_ceiling: None,
            expires_at: None,
            max_invocations: None,
        },
    }
}

fn local_dev_builtin_capability_ids() -> [&'static str; 14] {
    [
        ECHO_CAPABILITY_ID,
        TIME_CAPABILITY_ID,
        JSON_CAPABILITY_ID,
        HTTP_CAPABILITY_ID,
        SHELL_CAPABILITY_ID,
        READ_FILE_CAPABILITY_ID,
        WRITE_FILE_CAPABILITY_ID,
        LIST_DIR_CAPABILITY_ID,
        GLOB_CAPABILITY_ID,
        GREP_CAPABILITY_ID,
        APPLY_PATCH_CAPABILITY_ID,
        SKILL_LIST_CAPABILITY_ID,
        SKILL_INSTALL_CAPABILITY_ID,
        SKILL_REMOVE_CAPABILITY_ID,
    ]
}

fn local_dev_network_allowed_effects() -> Vec<EffectKind> {
    vec![EffectKind::DispatchCapability, EffectKind::Network]
}

fn local_dev_allowed_effects() -> Vec<EffectKind> {
    vec![
        EffectKind::DispatchCapability,
        EffectKind::ReadFilesystem,
        EffectKind::WriteFilesystem,
    ]
}

fn local_dev_shell_allowed_effects() -> Vec<EffectKind> {
    let mut effects = local_dev_allowed_effects();
    effects.extend([
        EffectKind::SpawnProcess,
        EffectKind::ExecuteCode,
        EffectKind::Network,
    ]);
    effects
}

fn local_dev_provider_allowed_effects() -> Vec<EffectKind> {
    local_dev_shell_allowed_effects()
}

fn local_dev_shell_network_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: None,
            host_pattern: "*".to_string(),
            port: None,
        }],
        // Local-dev shell is intentionally broad for developer CLI workflows,
        // but it still uses the coarse host-local guard so cloud metadata,
        // link-local, multicast, loopback, and private IP targets remain
        // blocked by the shared network policy enforcer.
        deny_private_ip_ranges: true,
        max_egress_bytes: None,
    }
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
