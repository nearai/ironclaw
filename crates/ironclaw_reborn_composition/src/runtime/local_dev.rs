use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    sync::{Arc, Mutex as StdMutex},
};

use chrono::Utc;
use uuid::Uuid;

use ironclaw_host_api::{
    CapabilityGrant, CapabilityGrantId, CapabilityId, CapabilitySet, EffectKind, ExecutionContext,
    ExtensionId, GrantConstraints, MountAlias, MountGrant, MountPermissions, MountView,
    NetworkPolicy, NetworkTargetPattern, Principal, RuntimeKind, TrustClass, UserId, VirtualPath,
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
use ironclaw_threads::{ToolResultReferenceEnvelope, ToolResultSafeSummary};
use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};
use ironclaw_turns::{
    LoopResultRef,
    run_profile::{
        AgentLoopHostError, AgentLoopHostErrorKind, CapabilityInputRef, LoopCapabilityPort,
        LoopHostMilestoneSink, LoopRunContext, ProviderToolCall,
    },
};

use crate::RebornServices;

pub(super) struct LocalDevCapabilityWiring {
    pub(super) capability_factory: Arc<dyn LoopCapabilityPortFactory>,
    pub(super) model_gateway: Arc<dyn HostManagedModelGateway>,
}

pub(super) fn capability_wiring(
    services: &RebornServices,
    user_id: UserId,
    model_gateway: Arc<dyn HostManagedModelGateway>,
    milestone_sink: Arc<dyn LoopHostMilestoneSink>,
) -> Option<LocalDevCapabilityWiring> {
    let runtime = services.host_runtime.clone()?;
    let capability_io = Arc::new(LocalDevCapabilityIo::default());
    let capability_input_resolver: Arc<dyn LoopCapabilityInputResolver> = capability_io.clone();
    let capability_result_writer: Arc<dyn LoopCapabilityResultWriter> = capability_io.clone();
    let capability_factory: Arc<dyn LoopCapabilityPortFactory> =
        Arc::new(LocalDevLoopCapabilityPortFactory::new(
            runtime,
            user_id,
            capability_input_resolver,
            capability_result_writer,
            milestone_sink,
        ));
    let model_gateway: Arc<dyn HostManagedModelGateway> = Arc::new(
        LocalDevResultHydratingModelGateway::new(model_gateway, capability_io),
    );

    Some(LocalDevCapabilityWiring {
        capability_factory,
        model_gateway,
    })
}

#[derive(Clone)]
struct LocalDevLoopCapabilityPortFactory {
    runtime: Arc<dyn HostRuntime>,
    user_id: UserId,
    input_resolver: Arc<dyn LoopCapabilityInputResolver>,
    result_writer: Arc<dyn LoopCapabilityResultWriter>,
    milestone_sink: Arc<dyn LoopHostMilestoneSink>,
}

impl LocalDevLoopCapabilityPortFactory {
    fn new(
        runtime: Arc<dyn HostRuntime>,
        user_id: UserId,
        input_resolver: Arc<dyn LoopCapabilityInputResolver>,
        result_writer: Arc<dyn LoopCapabilityResultWriter>,
        milestone_sink: Arc<dyn LoopHostMilestoneSink>,
    ) -> Self {
        Self {
            runtime,
            user_id,
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
        let workspace_mounts = local_dev_workspace_mounts()?;
        let skill_mounts = local_dev_skill_mounts()?;
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

#[derive(Default)]
struct LocalDevCapabilityIo {
    inputs: StdMutex<StagedValueStore>,
    results: StdMutex<StagedValueStore>,
}

impl LocalDevCapabilityIo {
    fn result_output(
        &self,
        result_ref: &str,
    ) -> Result<Option<serde_json::Value>, AgentLoopHostError> {
        self.results
            .lock()
            .map_err(|_| capability_io_error())
            .map(|results| results.get(result_ref).cloned())
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
        Ok(input_ref)
    }
}

#[async_trait::async_trait]
impl LoopCapabilityResultWriter for LocalDevCapabilityIo {
    async fn write_capability_result(
        &self,
        run_context: &LoopRunContext,
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
        let mut results = self.results.lock().map_err(|_| capability_io_error())?;
        results.insert_with_oldest_eviction(result_ref.as_str().to_string(), output)?;
        Ok(result_ref)
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

fn local_dev_grant_constraints(
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

fn local_dev_workspace_mounts() -> Result<MountView, AgentLoopHostError> {
    MountView::new(vec![MountGrant::new(
        MountAlias::new("/workspace").map_err(host_api_agent_loop_error)?,
        VirtualPath::new("/projects/workspace").map_err(host_api_agent_loop_error)?,
        MountPermissions::read_write(),
    )])
    .map_err(host_api_agent_loop_error)
}

fn local_dev_skill_mounts() -> Result<MountView, AgentLoopHostError> {
    MountView::new(vec![
        MountGrant::new(
            MountAlias::new("/skills").map_err(host_api_agent_loop_error)?,
            VirtualPath::new("/projects/skills").map_err(host_api_agent_loop_error)?,
            MountPermissions::read_write_list_delete(),
        ),
        MountGrant::new(
            MountAlias::new("/system/skills").map_err(host_api_agent_loop_error)?,
            VirtualPath::new("/projects/system/skills").map_err(host_api_agent_loop_error)?,
            MountPermissions::read_only(),
        ),
    ])
    .map_err(host_api_agent_loop_error)
}

fn ensure_local_dev_ref_scope(
    prefix: &str,
    reference: &str,
    run_context: &LoopRunContext,
) -> Result<(), AgentLoopHostError> {
    let expected_prefix = format!("{prefix}:{}:", run_context.run_id);
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
mod tests {
    use super::*;

    use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId};
    use ironclaw_turns::{
        RunProfileResolutionRequest, RunProfileResolver, TurnId, TurnRunId, TurnScope,
        run_profile::InMemoryRunProfileResolver,
    };

    async fn run_context(label: &str) -> LoopRunContext {
        let resolved = InMemoryRunProfileResolver::default()
            .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
            .await
            .expect("profile resolves");
        LoopRunContext::new(
            TurnScope::new(
                TenantId::new(format!("tenant-{label}")).expect("tenant id"),
                Some(AgentId::new(format!("agent-{label}")).expect("agent id")),
                Some(ProjectId::new(format!("project-{label}")).expect("project id")),
                ThreadId::new(format!("thread-{label}")).expect("thread id"),
            ),
            TurnId::new(),
            TurnRunId::new(),
            resolved,
        )
    }

    fn provider_tool_call(arguments: serde_json::Value) -> ProviderToolCall {
        ProviderToolCall {
            provider_id: "test-provider".to_string(),
            provider_model_id: "test-model".to_string(),
            turn_id: Some("provider-turn-1".to_string()),
            id: "call-1".to_string(),
            name: "builtin_echo".to_string(),
            arguments,
            response_reasoning: None,
            reasoning: None,
            signature: None,
        }
    }

    #[tokio::test]
    async fn capability_io_resolves_input_refs_repeatedly() {
        let capability_io = LocalDevCapabilityIo::default();
        let run_context = run_context("repeat-input").await;
        let input_ref = capability_io
            .register_provider_tool_call_input(
                &run_context,
                &provider_tool_call(serde_json::json!({"message": "hello"})),
            )
            .await
            .expect("input stages");

        let first = capability_io
            .resolve_capability_input(&run_context, &input_ref)
            .await
            .expect("first resolve succeeds");
        let second = capability_io
            .resolve_capability_input(&run_context, &input_ref)
            .await
            .expect("second resolve succeeds");

        assert_eq!(first, serde_json::json!({"message": "hello"}));
        assert_eq!(second, serde_json::json!({"message": "hello"}));
    }

    #[tokio::test]
    async fn capability_io_rejects_cross_run_and_unstaged_input_refs() {
        let capability_io = LocalDevCapabilityIo::default();
        let current_context = run_context("input-scope-a").await;
        let other_context = run_context("input-scope-b").await;
        let input_ref = capability_io
            .register_provider_tool_call_input(
                &current_context,
                &provider_tool_call(serde_json::json!({"message": "hello"})),
            )
            .await
            .expect("input stages");

        let cross_run = capability_io
            .resolve_capability_input(&other_context, &input_ref)
            .await
            .expect_err("foreign run should fail");
        assert_eq!(cross_run.kind, AgentLoopHostErrorKind::ScopeMismatch);

        let missing_ref =
            CapabilityInputRef::new(format!("input:{}:missing", current_context.run_id))
                .expect("missing ref");
        let missing = capability_io
            .resolve_capability_input(&current_context, &missing_ref)
            .await
            .expect_err("unstaged ref should fail");
        assert_eq!(missing.kind, AgentLoopHostErrorKind::InvalidInvocation);
    }

    #[test]
    fn result_store_evicts_oldest_entries_to_stay_under_byte_cap() {
        let mut store = StagedValueStore::default();
        store
            .insert_with_oldest_eviction(
                "result:first".to_string(),
                serde_json::Value::String("a".repeat(3 * 1024 * 1024)),
            )
            .expect("first result stages");
        store
            .insert_with_oldest_eviction(
                "result:second".to_string(),
                serde_json::Value::String("b".repeat(2 * 1024 * 1024)),
            )
            .expect("second result stages");

        assert!(store.get("result:first").is_none());
        assert!(store.get("result:second").is_some());
        assert!(store.total_bytes <= LOCAL_DEV_CAPABILITY_IO_MAX_STAGED_BYTES);
    }

    #[test]
    fn local_dev_builtin_surface_grants_shell_as_ambient_escape_hatch() {
        let capability_ids = local_dev_builtin_capability_ids();

        assert!(capability_ids.contains(&WRITE_FILE_CAPABILITY_ID));
        assert!(capability_ids.contains(&APPLY_PATCH_CAPABILITY_ID));
        assert!(capability_ids.contains(&SKILL_LIST_CAPABILITY_ID));
        assert!(capability_ids.contains(&SKILL_INSTALL_CAPABILITY_ID));
        assert!(capability_ids.contains(&SKILL_REMOVE_CAPABILITY_ID));
        assert!(capability_ids.contains(&SHELL_CAPABILITY_ID));
        assert!(capability_ids.contains(&HTTP_CAPABILITY_ID));
        assert_eq!(
            local_dev_allowed_effects(),
            vec![
                EffectKind::DispatchCapability,
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem
            ]
        );
        assert_eq!(
            local_dev_provider_allowed_effects(),
            vec![
                EffectKind::DispatchCapability,
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem,
                EffectKind::SpawnProcess,
                EffectKind::ExecuteCode,
                EffectKind::Network
            ]
        );

        let workspace_mounts = local_dev_workspace_mounts().expect("workspace mounts build");
        let skill_mounts = local_dev_skill_mounts().expect("skill mounts build");
        assert!(workspace_mounts.mounts.iter().all(|mount| {
            mount.alias.as_str() != "/skills" && mount.alias.as_str() != "/system/skills"
        }));
        let mount_for = |alias: &str| {
            skill_mounts
                .mounts
                .iter()
                .find(|mount| mount.alias.as_str() == alias)
                .expect("mount exists")
        };
        assert_eq!(
            mount_for("/skills").permissions,
            MountPermissions::read_write_list_delete()
        );
        assert_eq!(
            mount_for("/system/skills").permissions,
            MountPermissions::read_only()
        );
        let grants = local_dev_builtin_grants(
            &ExtensionId::new("loop-driver").expect("valid extension id"),
            &workspace_mounts,
            &skill_mounts,
        )
        .expect("local-dev grants build");
        let grant_for = |capability_id: &str| {
            grants
                .grants
                .iter()
                .find(|grant| grant.capability.as_str() == capability_id)
                .expect("capability grant exists")
        };

        let shell_grant = grant_for(SHELL_CAPABILITY_ID);
        assert_eq!(
            shell_grant.constraints.allowed_effects,
            vec![
                EffectKind::DispatchCapability,
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem,
                EffectKind::SpawnProcess,
                EffectKind::ExecuteCode,
                EffectKind::Network
            ]
        );
        assert!(shell_grant.constraints.mounts.mounts.is_empty());
        assert_eq!(
            shell_grant.constraints.network,
            local_dev_shell_network_policy()
        );

        let http_grant = grant_for(HTTP_CAPABILITY_ID);
        assert_eq!(
            http_grant.constraints.allowed_effects,
            vec![EffectKind::DispatchCapability, EffectKind::Network]
        );
        assert!(http_grant.constraints.mounts.mounts.is_empty());
        assert_eq!(
            http_grant.constraints.network,
            local_dev_shell_network_policy()
        );

        let read_file_grant = grant_for(READ_FILE_CAPABILITY_ID);
        assert_eq!(
            read_file_grant.constraints.allowed_effects,
            local_dev_allowed_effects()
        );
        assert_eq!(read_file_grant.constraints.mounts, workspace_mounts);
        assert_eq!(
            read_file_grant.constraints.network,
            NetworkPolicy::default()
        );

        let skill_install_grant = grant_for(SKILL_INSTALL_CAPABILITY_ID);
        assert_eq!(skill_install_grant.constraints.mounts, skill_mounts);
        assert_eq!(
            skill_install_grant.constraints.network,
            NetworkPolicy::default()
        );
    }

    #[test]
    fn model_visible_tool_output_truncates_at_utf8_boundary() {
        let output = model_visible_tool_output(&serde_json::json!({
            "message": "é".repeat(300),
        }));

        assert!(output.len() <= MODEL_VISIBLE_TOOL_OUTPUT_MAX_BYTES);
        assert!(output.is_char_boundary(output.len()));
        ToolResultSafeSummary::new(output).expect("summary remains safe");
    }
}
