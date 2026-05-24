use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use ironclaw_host_api::{
    CapabilityId, CapabilitySet, CorrelationId, EffectKind, ExecutionContext, ExtensionId,
    InvocationId, MountView, Principal, ResourceEstimate, RuntimeKind, sha256_digest_token,
};
use ironclaw_host_runtime::{
    HostRuntime, HostRuntimeError, IdempotencyKey, RuntimeBlockedReason, RuntimeCapabilityOutcome,
    RuntimeCapabilityRequest, RuntimeFailureKind,
};
use ironclaw_turns::{
    LoopGateRef, LoopResultRef,
    run_profile::{
        AgentLoopHostError, AgentLoopHostErrorKind, CapabilityBatchInvocation,
        CapabilityBatchOutcome, CapabilityDenied, CapabilityDeniedReasonKind,
        CapabilityDescriptorView, CapabilityFailure, CapabilityFailureKind, CapabilityInputRef,
        CapabilityInvocation, CapabilityOutcome, CapabilityResultMessage, ConcurrencyHint,
        LoopCapabilityPort, LoopHostMilestoneEmitter, LoopHostMilestoneSink, LoopProcessRef,
        LoopRunContext, LoopSafeSummary, ProcessHandleSummary, ProviderToolCall,
        ProviderToolCallReplay, ProviderToolDefinition, VisibleCapabilityRequest,
        VisibleCapabilitySurface,
    },
};
use tokio::sync::Notify;

const PROVIDER_TOOL_NAME_MAX_BYTES: usize = 64;
const PROVIDER_TOOL_NAME_DIGEST_BYTES: usize = 32;

#[async_trait]
pub trait LoopCapabilityInputResolver: Send + Sync {
    async fn resolve_capability_input(
        &self,
        run_context: &LoopRunContext,
        input_ref: &CapabilityInputRef,
    ) -> Result<serde_json::Value, AgentLoopHostError>;

    async fn register_provider_tool_call_input(
        &self,
        _run_context: &LoopRunContext,
        _tool_call: &ProviderToolCall,
    ) -> Result<CapabilityInputRef, AgentLoopHostError> {
        Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            "provider tool-call input registration is not supported",
        ))
    }
}

struct ProviderToolCallInputResolver {
    inner: Arc<dyn LoopCapabilityInputResolver>,
    provider_inputs: Mutex<HashMap<String, serde_json::Value>>,
}

impl ProviderToolCallInputResolver {
    fn new(inner: Arc<dyn LoopCapabilityInputResolver>) -> Self {
        Self {
            inner,
            provider_inputs: Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl LoopCapabilityInputResolver for ProviderToolCallInputResolver {
    async fn resolve_capability_input(
        &self,
        run_context: &LoopRunContext,
        input_ref: &CapabilityInputRef,
    ) -> Result<serde_json::Value, AgentLoopHostError> {
        if let Some(input) = self
            .provider_inputs
            .lock()
            .map_err(|_| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Unavailable,
                    "provider tool-call input store is unavailable",
                )
            })?
            .get(input_ref.as_str())
            .cloned()
        {
            return Ok(input);
        }
        self.inner
            .resolve_capability_input(run_context, input_ref)
            .await
    }

    async fn register_provider_tool_call_input(
        &self,
        run_context: &LoopRunContext,
        tool_call: &ProviderToolCall,
    ) -> Result<CapabilityInputRef, AgentLoopHostError> {
        let input_ref = provider_tool_call_input_ref(run_context, tool_call)?;
        let mut provider_inputs = self.provider_inputs.lock().map_err(|_| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "provider tool-call input store is unavailable",
            )
        })?;
        if let Some(existing) = provider_inputs.get(input_ref.as_str()) {
            if existing != &tool_call.arguments {
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Internal,
                    "provider tool-call input ref collision",
                ));
            }
        } else {
            provider_inputs.insert(input_ref.as_str().to_string(), tool_call.arguments.clone());
        }
        Ok(input_ref)
    }
}

#[async_trait]
pub trait LoopCapabilityResultWriter: Send + Sync {
    async fn write_capability_result(
        &self,
        run_context: &LoopRunContext,
        capability_id: &CapabilityId,
        output: serde_json::Value,
    ) -> Result<LoopResultRef, AgentLoopHostError>;
}

#[derive(Clone)]
pub struct HostRuntimeLoopCapabilityPortFactory {
    runtime: Arc<dyn HostRuntime>,
    visible_request: ironclaw_host_runtime::VisibleCapabilityRequest,
    input_resolver: Arc<dyn LoopCapabilityInputResolver>,
    result_writer: Arc<dyn LoopCapabilityResultWriter>,
    milestone_sink: Arc<dyn LoopHostMilestoneSink>,
    execution_mounts: MountView,
    capability_execution_mounts: HashMap<CapabilityId, MountView>,
}

impl HostRuntimeLoopCapabilityPortFactory {
    pub fn new(
        runtime: Arc<dyn HostRuntime>,
        visible_request: ironclaw_host_runtime::VisibleCapabilityRequest,
        input_resolver: Arc<dyn LoopCapabilityInputResolver>,
        result_writer: Arc<dyn LoopCapabilityResultWriter>,
        milestone_sink: Arc<dyn LoopHostMilestoneSink>,
    ) -> Self {
        Self {
            runtime,
            visible_request,
            input_resolver,
            result_writer,
            milestone_sink,
            execution_mounts: MountView::default(),
            capability_execution_mounts: HashMap::new(),
        }
    }

    pub fn with_execution_mounts(mut self, mounts: MountView) -> Self {
        self.execution_mounts = mounts;
        self
    }

    pub fn with_capability_execution_mount(
        mut self,
        capability_id: CapabilityId,
        mounts: MountView,
    ) -> Self {
        self.capability_execution_mounts
            .insert(capability_id, mounts);
        self
    }

    pub fn for_run_context(&self, run_context: LoopRunContext) -> Arc<dyn LoopCapabilityPort> {
        Arc::new(self.port_for_run_context(run_context))
    }

    fn port_for_run_context(&self, run_context: LoopRunContext) -> HostRuntimeLoopCapabilityPort {
        HostRuntimeLoopCapabilityPort::new(
            Arc::clone(&self.runtime),
            run_context,
            self.visible_request.clone(),
            Arc::clone(&self.input_resolver),
            Arc::clone(&self.result_writer),
            Arc::clone(&self.milestone_sink),
        )
        .with_execution_mounts(self.execution_mounts.clone())
        .with_capability_execution_mounts(self.capability_execution_mounts.clone())
    }
}

#[derive(Clone)]
struct SurfaceCapabilitySnapshot {
    provider: ExtensionId,
    runtime: RuntimeKind,
    estimate: ResourceEstimate,
    safe_description: String,
    parameters_schema: serde_json::Value,
    provider_tool_name: String,
}

#[derive(Clone, Default)]
struct SurfaceSnapshot {
    capabilities: HashMap<CapabilityId, SurfaceCapabilitySnapshot>,
    provider_names: HashMap<String, CapabilityId>,
}

struct PreparedProviderToolCall {
    surface_version: ironclaw_turns::run_profile::CapabilitySurfaceVersion,
    capability_id: CapabilityId,
    provider_turn_id: String,
    normalized_arguments: serde_json::Value,
}

const MAX_IN_MEMORY_DISPATCH_RECORDS: usize = 128;
const SENSITIVE_PROVIDER_TEXT_MARKERS: [&str; 18] = [
    "access token",
    "api key",
    "api_key",
    "apikey",
    "authorization:",
    "bearer ",
    "host path",
    "invalid api key",
    "invalid_api_key",
    "password",
    "passwd",
    "provider error",
    "raw runtime",
    "secret",
    "stack trace",
    "tool input",
    "tool_input",
    "traceback",
];

#[derive(Clone)]
enum DispatchRecord {
    InFlight {
        notify: Arc<Notify>,
    },
    RuntimeCompleted {
        requested_capability_id: CapabilityId,
        outcome: RuntimeCapabilityOutcome,
    },
    LoopCompleted(Result<CapabilityOutcome, AgentLoopHostError>),
}

#[derive(Default)]
struct DispatchRecordStore {
    records: HashMap<String, DispatchRecord>,
    insertion_order: VecDeque<String>,
}

impl DispatchRecordStore {
    fn reserve(&mut self, key: &IdempotencyKey) -> Result<DispatchReservation, AgentLoopHostError> {
        let key_value = key.as_str().to_string();
        match self.records.get(key.as_str()).cloned() {
            Some(DispatchRecord::InFlight { notify }) => Ok(DispatchReservation::Wait(notify)),
            Some(DispatchRecord::RuntimeCompleted {
                requested_capability_id,
                outcome,
            }) => {
                self.records.insert(
                    key_value,
                    DispatchRecord::InFlight {
                        notify: Arc::new(Notify::new()),
                    },
                );
                Ok(DispatchReservation::RuntimeCompleted {
                    requested_capability_id,
                    outcome,
                })
            }
            Some(DispatchRecord::LoopCompleted(result)) => {
                Ok(DispatchReservation::LoopCompleted(result))
            }
            None => {
                self.evict_completed_until_below_limit()?;
                self.insertion_order.push_back(key_value.clone());
                self.records.insert(
                    key_value,
                    DispatchRecord::InFlight {
                        notify: Arc::new(Notify::new()),
                    },
                );
                Ok(DispatchReservation::Reserved)
            }
        }
    }

    fn record(&mut self, key: &IdempotencyKey, record: DispatchRecord) -> Option<Arc<Notify>> {
        let previous = self.records.insert(key.as_str().to_string(), record);
        match previous {
            Some(DispatchRecord::InFlight { notify }) => Some(notify),
            _ => None,
        }
    }

    fn remove(&mut self, key: &IdempotencyKey) -> Option<Arc<Notify>> {
        let removed = self.records.remove(key.as_str());
        self.insertion_order
            .retain(|candidate| candidate != key.as_str());
        match removed {
            Some(DispatchRecord::InFlight { notify }) => Some(notify),
            _ => None,
        }
    }

    fn in_flight_matches(&self, key: &IdempotencyKey, notify: &Arc<Notify>) -> bool {
        matches!(
            self.records.get(key.as_str()),
            Some(DispatchRecord::InFlight { notify: current }) if Arc::ptr_eq(current, notify)
        )
    }

    fn evict_completed_until_below_limit(&mut self) -> Result<(), AgentLoopHostError> {
        let mut scanned = 0;
        let scan_limit = self.insertion_order.len();
        while self.records.len() >= MAX_IN_MEMORY_DISPATCH_RECORDS && scanned < scan_limit {
            let Some(candidate) = self.insertion_order.pop_front() else {
                break;
            };
            scanned += 1;
            match self.records.get(&candidate) {
                None => {}
                Some(DispatchRecord::InFlight { .. }) => self.insertion_order.push_back(candidate),
                Some(DispatchRecord::RuntimeCompleted { .. })
                | Some(DispatchRecord::LoopCompleted(_)) => {
                    self.records.remove(&candidate);
                }
            }
        }
        if self.records.len() >= MAX_IN_MEMORY_DISPATCH_RECORDS {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "capability dispatch record store is full",
            ));
        }
        Ok(())
    }
}

enum DispatchReservation {
    Reserved,
    Wait(Arc<Notify>),
    RuntimeCompleted {
        requested_capability_id: CapabilityId,
        outcome: RuntimeCapabilityOutcome,
    },
    LoopCompleted(Result<CapabilityOutcome, AgentLoopHostError>),
}

/// RAII guard for an `InFlight` dispatch reservation: if the holder drops
/// without calling [`Self::commit`], the reservation is cleared and any
/// waiters are notified. Clearing failures are logged but do not panic, since
/// dropping happens on unwind paths where there's nothing useful to propagate.
struct DispatchReservationGuard<'a> {
    port: &'a HostRuntimeLoopCapabilityPort,
    key: IdempotencyKey,
    committed: bool,
}

impl DispatchReservationGuard<'_> {
    fn commit(mut self) {
        self.committed = true;
    }
}

impl Drop for DispatchReservationGuard<'_> {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        if let Err(error) = self.port.clear_dispatch(&self.key) {
            tracing::warn!(
                cleanup_error = %error,
                "failed to clean up dispatch reservation after early return"
            );
        }
    }
}

pub struct HostRuntimeLoopCapabilityPort {
    runtime: Arc<dyn HostRuntime>,
    run_context: LoopRunContext,
    visible_request: ironclaw_host_runtime::VisibleCapabilityRequest,
    input_resolver: Arc<dyn LoopCapabilityInputResolver>,
    result_writer: Arc<dyn LoopCapabilityResultWriter>,
    milestone_sink: Arc<dyn LoopHostMilestoneSink>,
    execution_mounts: MountView,
    capability_execution_mounts: HashMap<CapabilityId, MountView>,
    snapshots: Mutex<HashMap<String, SurfaceSnapshot>>,
    current_surface_version: Mutex<Option<String>>,
    dispatch_records: Mutex<DispatchRecordStore>,
}

/// Lock a poisoned-aware `Mutex` and wrap a poison error as the canonical
/// "<label> is unavailable" host error. Every store in this module is reached
/// via this helper so the error message stays consistent and the call sites
/// shrink to one line.
fn lock_mut<'a, T>(
    mutex: &'a Mutex<T>,
    label: &'static str,
) -> Result<std::sync::MutexGuard<'a, T>, AgentLoopHostError> {
    mutex.lock().map_err(|_| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::Unavailable,
            format!("{label} is unavailable"),
        )
    })
}

impl HostRuntimeLoopCapabilityPort {
    pub fn new(
        runtime: Arc<dyn HostRuntime>,
        run_context: LoopRunContext,
        visible_request: ironclaw_host_runtime::VisibleCapabilityRequest,
        input_resolver: Arc<dyn LoopCapabilityInputResolver>,
        result_writer: Arc<dyn LoopCapabilityResultWriter>,
        milestone_sink: Arc<dyn LoopHostMilestoneSink>,
    ) -> Self {
        let input_resolver: Arc<dyn LoopCapabilityInputResolver> =
            Arc::new(ProviderToolCallInputResolver::new(input_resolver));
        Self {
            runtime,
            run_context,
            visible_request,
            input_resolver,
            result_writer,
            milestone_sink,
            execution_mounts: MountView::default(),
            capability_execution_mounts: HashMap::new(),
            snapshots: Mutex::new(HashMap::new()),
            current_surface_version: Mutex::new(None),
            dispatch_records: Mutex::new(DispatchRecordStore::default()),
        }
    }

    pub fn with_execution_mounts(mut self, mounts: MountView) -> Self {
        self.execution_mounts = mounts;
        self
    }

    pub fn with_capability_execution_mounts(
        mut self,
        mounts: HashMap<CapabilityId, MountView>,
    ) -> Self {
        self.capability_execution_mounts = mounts;
        self
    }

    fn execution_mounts_for(&self, capability_id: &CapabilityId) -> &MountView {
        self.capability_execution_mounts
            .get(capability_id)
            .unwrap_or(&self.execution_mounts)
    }

    fn snapshot_for(
        &self,
        version: &ironclaw_turns::run_profile::CapabilitySurfaceVersion,
    ) -> Result<SurfaceSnapshot, AgentLoopHostError> {
        let snapshots = lock_mut(&self.snapshots, "capability surface snapshot store")?;
        snapshots.get(version.as_str()).cloned().ok_or_else(|| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::StaleSurface,
                "capability surface is stale or unknown",
            )
        })
    }

    fn current_snapshot(&self) -> Result<Option<(String, SurfaceSnapshot)>, AgentLoopHostError> {
        let snapshots = lock_mut(&self.snapshots, "capability surface snapshot store")?;
        let version = lock_mut(
            &self.current_surface_version,
            "capability surface snapshot pointer",
        )?
        .clone();
        let Some(version) = version else {
            return Ok(None);
        };
        let snapshot = snapshots.get(&version).cloned().ok_or_else(|| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::StaleSurface,
                "current capability surface snapshot is unavailable",
            )
        })?;
        Ok(Some((version, snapshot)))
    }

    fn reserve_dispatch(
        &self,
        key: &IdempotencyKey,
    ) -> Result<DispatchReservation, AgentLoopHostError> {
        lock_mut(&self.dispatch_records, "capability dispatch record store")?.reserve(key)
    }

    fn dispatch_in_flight_matches(
        &self,
        key: &IdempotencyKey,
        notify: &Arc<Notify>,
    ) -> Result<bool, AgentLoopHostError> {
        Ok(
            lock_mut(&self.dispatch_records, "capability dispatch record store")?
                .in_flight_matches(key, notify),
        )
    }

    fn record_runtime_completed(
        &self,
        key: &IdempotencyKey,
        requested_capability_id: CapabilityId,
        outcome: RuntimeCapabilityOutcome,
    ) -> Result<(), AgentLoopHostError> {
        let notify = lock_mut(&self.dispatch_records, "capability dispatch record store")?.record(
            key,
            DispatchRecord::RuntimeCompleted {
                requested_capability_id,
                outcome,
            },
        );
        if let Some(notify) = notify {
            notify.notify_waiters();
        }
        Ok(())
    }

    fn record_loop_completed(
        &self,
        key: &IdempotencyKey,
        result: Result<CapabilityOutcome, AgentLoopHostError>,
    ) -> Result<(), AgentLoopHostError> {
        let notify = lock_mut(&self.dispatch_records, "capability dispatch record store")?
            .record(key, DispatchRecord::LoopCompleted(result));
        if let Some(notify) = notify {
            notify.notify_waiters();
        }
        Ok(())
    }

    fn clear_dispatch(&self, key: &IdempotencyKey) -> Result<(), AgentLoopHostError> {
        let notify =
            lock_mut(&self.dispatch_records, "capability dispatch record store")?.remove(key);
        if let Some(notify) = notify {
            notify.notify_waiters();
        }
        Ok(())
    }

    /// Drop guard for an `InFlight` dispatch reservation. Releases the
    /// reservation (and wakes any waiters) unless [`commit`] is called first.
    /// Use after a successful `reserve_dispatch` returns `Reserved` so any
    /// early-return error path between reservation and outcome recording
    /// unwinds the reservation automatically.
    fn dispatch_reservation_guard<'a>(
        &'a self,
        key: &IdempotencyKey,
    ) -> DispatchReservationGuard<'a> {
        DispatchReservationGuard {
            port: self,
            key: key.clone(),
            committed: false,
        }
    }

    fn validate_visible_request_scope(&self) -> Result<(), AgentLoopHostError> {
        let context = &self.visible_request.context;
        context.validate().map_err(|_| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "capability execution context is invalid",
            )
        })?;
        if context.tenant_id != self.run_context.scope.tenant_id
            || context.agent_id != self.run_context.scope.agent_id
            || context.project_id != self.run_context.scope.project_id
            || context.thread_id.as_ref() != Some(&self.run_context.thread_id)
            || context.resource_scope.tenant_id != self.run_context.scope.tenant_id
            || context.resource_scope.agent_id != self.run_context.scope.agent_id
            || context.resource_scope.project_id != self.run_context.scope.project_id
            || context.resource_scope.thread_id.as_ref() != Some(&self.run_context.thread_id)
        {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::ScopeMismatch,
                "capability execution context is not scoped to this loop run",
            ));
        }
        if context.mounts != MountView::default() {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unauthorized,
                "capability execution context must not carry caller-supplied mounts",
            ));
        }
        Ok(())
    }

    async fn finish_runtime_outcome(
        &self,
        key: &IdempotencyKey,
        requested_capability_id: &CapabilityId,
        outcome: RuntimeCapabilityOutcome,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        let result = runtime_outcome_to_loop(
            &self.run_context,
            self.result_writer.as_ref(),
            requested_capability_id,
            outcome.clone(),
        )
        .await;
        if should_retry_result_write(&outcome, &result) {
            self.record_runtime_completed(key, requested_capability_id.clone(), outcome)?;
            return result;
        }
        self.record_loop_completed(key, result.clone())?;
        result
    }

    async fn wait_for_dispatch_completion(
        &self,
        key: &IdempotencyKey,
        notify: Arc<Notify>,
    ) -> Result<(), AgentLoopHostError> {
        let notified = notify.notified();
        tokio::pin!(notified);
        if self.dispatch_in_flight_matches(key, &notify)? {
            notified.await;
        }
        Ok(())
    }

    async fn emit_capability_invoked(
        &self,
        capability_id: CapabilityId,
    ) -> Result<(), AgentLoopHostError> {
        let milestones = LoopHostMilestoneEmitter::new(
            self.run_context.clone(),
            Arc::clone(&self.milestone_sink),
        );
        milestones.capability_invoked(capability_id).await
    }

    fn prepare_provider_tool_call(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<PreparedProviderToolCall, AgentLoopHostError> {
        self.validate_visible_request_scope()?;
        validate_provider_tool_call(tool_call)?;
        let provider_turn_id = tool_call.turn_id.clone().ok_or_else(|| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "provider tool call is missing a provider turn id",
            )
        })?;
        let Some((version, snapshot)) = self.current_snapshot()? else {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::StaleSurface,
                "capability surface is unavailable",
            ));
        };
        let Some(capability_id) = snapshot.provider_names.get(&tool_call.name).cloned() else {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "provider tool call is outside the visible capability surface",
            ));
        };
        let Some(capability) = snapshot.capabilities.get(&capability_id) else {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::StaleSurface,
                "capability surface snapshot is missing provider metadata",
            ));
        };
        if !provider_schema_is_usable(&capability.parameters_schema) {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "provider tool call was not advertised to the model",
            ));
        }
        let normalized_arguments = normalize_provider_arguments(
            &tool_call.arguments,
            &capability.parameters_schema,
            "provider arguments",
        )?;
        validate_provider_arguments(&normalized_arguments)?;
        Ok(PreparedProviderToolCall {
            surface_version: loop_surface_version(&version)?,
            capability_id,
            provider_turn_id,
            normalized_arguments,
        })
    }
}

#[async_trait]
impl LoopCapabilityPort for HostRuntimeLoopCapabilityPort {
    fn tool_definitions(&self) -> Result<Vec<ProviderToolDefinition>, AgentLoopHostError> {
        self.validate_visible_request_scope()?;
        let Some((_, snapshot)) = self.current_snapshot()? else {
            return Ok(Vec::new());
        };
        let mut definitions = snapshot
            .capabilities
            .iter()
            .filter_map(|(capability_id, capability)| {
                if !provider_schema_is_usable(&capability.parameters_schema) {
                    tracing::debug!(
                        capability_id = capability_id.as_str(),
                        "capability omitted from provider tool definitions because its parameter schema is not provider-usable"
                    );
                    return None;
                }
                Some(ProviderToolDefinition {
                    capability_id: capability_id.clone(),
                    name: capability.provider_tool_name.clone(),
                    description: capability.safe_description.clone(),
                    parameters: capability.parameters_schema.clone(),
                })
            })
            .collect::<Vec<_>>();
        definitions.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(definitions)
    }

    fn validate_provider_tool_call(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<(), AgentLoopHostError> {
        self.prepare_provider_tool_call(tool_call).map(|_| ())
    }

    async fn register_provider_tool_call(
        &self,
        tool_call: ProviderToolCall,
    ) -> Result<ironclaw_turns::run_profile::CapabilityCallCandidate, AgentLoopHostError> {
        let prepared = self.prepare_provider_tool_call(&tool_call)?;
        let mut normalized_tool_call = tool_call.clone();
        normalized_tool_call.arguments = prepared.normalized_arguments;
        let input_ref = self
            .input_resolver
            .register_provider_tool_call_input(&self.run_context, &normalized_tool_call)
            .await?;
        Ok(ironclaw_turns::run_profile::CapabilityCallCandidate {
            surface_version: prepared.surface_version,
            capability_id: prepared.capability_id,
            input_ref,
            provider_replay: Some(ProviderToolCallReplay {
                provider_id: tool_call.provider_id,
                provider_model_id: tool_call.provider_model_id,
                provider_turn_id: prepared.provider_turn_id,
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
        _request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        self.validate_visible_request_scope()?;
        let runtime_surface = self
            .runtime
            .visible_capabilities(self.visible_request.clone())
            .await
            .map_err(host_runtime_error)?;
        let version = loop_surface_version(runtime_surface.version.as_str())?;
        let mut snapshot = SurfaceSnapshot::default();
        let descriptors = runtime_surface
            .capabilities
            .into_iter()
            .map(|capability| {
                let capability_id = capability.descriptor.id.clone();
                let provider_tool_name =
                    provider_tool_name(&capability.descriptor.id, &snapshot.provider_names);
                snapshot
                    .provider_names
                    .insert(provider_tool_name.clone(), capability_id.clone());
                snapshot.capabilities.insert(
                    capability_id.clone(),
                    SurfaceCapabilitySnapshot {
                        provider: capability.descriptor.provider.clone(),
                        runtime: capability.descriptor.runtime,
                        estimate: capability.estimated_resources.clone(),
                        safe_description: capability.descriptor.description.clone(),
                        parameters_schema: capability.descriptor.parameters_schema.clone(),
                        provider_tool_name,
                    },
                );
                CapabilityDescriptorView {
                    capability_id,
                    provider: Some(capability.descriptor.provider),
                    runtime: capability.descriptor.runtime,
                    safe_name: capability.descriptor.id.as_str().to_string(),
                    safe_description: capability.descriptor.description,
                    concurrency_hint: concurrency_hint_from_effects(&capability.descriptor.effects),
                    parameters_schema: capability.descriptor.parameters_schema,
                }
            })
            .collect();

        let mut snapshots = lock_mut(&self.snapshots, "capability surface snapshot store")?;
        snapshots.clear();
        snapshots.insert(version.as_str().to_string(), snapshot);
        *lock_mut(
            &self.current_surface_version,
            "capability surface snapshot pointer",
        )? = Some(version.as_str().to_string());

        Ok(VisibleCapabilitySurface {
            version,
            descriptors,
        })
    }

    async fn invoke_capability(
        &self,
        request: CapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        let snapshot = self.snapshot_for(&request.surface_version)?;
        let Some(capability) = snapshot.capabilities.get(&request.capability_id).cloned() else {
            return Ok(CapabilityOutcome::Denied(CapabilityDenied {
                reason_kind: capability_denied_reason_kind("outside_visible_surface")?,
                safe_summary: "capability was not visible on the cited surface".to_string(),
            }));
        };
        let Some(trust_decision) = self
            .visible_request
            .provider_trust
            .get(&capability.provider)
            .cloned()
        else {
            return Ok(CapabilityOutcome::Denied(CapabilityDenied {
                reason_kind: capability_denied_reason_kind("missing_provider_trust")?,
                safe_summary: "capability provider trust is unavailable".to_string(),
            }));
        };
        let idempotency_key = invocation_idempotency_key(&self.run_context, &request)?;
        loop {
            match self.reserve_dispatch(&idempotency_key)? {
                DispatchReservation::Reserved => break,
                DispatchReservation::Wait(notify) => {
                    self.wait_for_dispatch_completion(&idempotency_key, notify)
                        .await?;
                }
                DispatchReservation::RuntimeCompleted {
                    requested_capability_id,
                    outcome,
                } => {
                    return self
                        .finish_runtime_outcome(&idempotency_key, &requested_capability_id, outcome)
                        .await;
                }
                DispatchReservation::LoopCompleted(result) => return result,
            }
        }

        // Any early `?` between reservation and `finish_runtime_outcome` unwinds
        // the in-flight reservation via the guard's `Drop`. The success path
        // calls `guard.commit()` so the dispatch record is replaced by
        // `finish_runtime_outcome` rather than cleared.
        let guard = self.dispatch_reservation_guard(&idempotency_key);

        let input = self
            .input_resolver
            .resolve_capability_input(&self.run_context, &request.input_ref)
            .await?;
        let requested_capability_id = request.capability_id.clone();
        self.emit_capability_invoked(request.capability_id.clone())
            .await?;
        let outcome = self
            .runtime
            .invoke_capability(
                RuntimeCapabilityRequest::new(
                    invocation_context_from_visible(
                        &self.visible_request.context,
                        &self.run_context,
                        &request.capability_id,
                        &capability,
                        trust_decision.effective_trust.class(),
                        &trust_decision.authority_ceiling.allowed_effects,
                        self.execution_mounts_for(&request.capability_id),
                    )?,
                    request.capability_id,
                    capability.estimate,
                    input,
                    trust_decision,
                )
                .with_idempotency_key(idempotency_key.clone()),
            )
            .await
            .map_err(host_runtime_error)?;

        guard.commit();
        self.finish_runtime_outcome(&idempotency_key, &requested_capability_id, outcome)
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

fn provider_schema_is_usable(schema: &serde_json::Value) -> bool {
    let Some(object) = schema.as_object() else {
        return false;
    };
    if object
        .get("$ref")
        .and_then(serde_json::Value::as_str)
        .is_some()
    {
        return true;
    }
    matches!(
        object.get("type").and_then(serde_json::Value::as_str),
        Some("object")
    ) && object
        .get("properties")
        .is_none_or(serde_json::Value::is_object)
}

fn normalize_provider_arguments(
    arguments: &serde_json::Value,
    schema: &serde_json::Value,
    label: &'static str,
) -> Result<serde_json::Value, AgentLoopHostError> {
    normalize_provider_value(arguments, schema, label)
}

fn normalize_provider_value(
    value: &serde_json::Value,
    schema: &serde_json::Value,
    label: &'static str,
) -> Result<serde_json::Value, AgentLoopHostError> {
    if schema_type_matches(schema, "object") {
        let object_value = coerce_json_string(value, label)?;
        let Some(object) = object_value.as_object() else {
            if is_json_container_string(value) {
                return Err(provider_coercion_error(label, "object"));
            }
            return Ok(object_value);
        };
        let Some(properties) = schema
            .get("properties")
            .and_then(serde_json::Value::as_object)
        else {
            return Ok(object_value);
        };
        let mut normalized = object.clone();
        for (property, property_schema) in properties {
            if let Some(property_value) = normalized.get(property).cloned() {
                normalized.insert(
                    property.clone(),
                    normalize_provider_value(&property_value, property_schema, label)?,
                );
            }
        }
        return Ok(serde_json::Value::Object(normalized));
    }

    if schema_type_matches(schema, "array") {
        let array_value = coerce_json_string(value, label)?;
        let Some(array) = array_value.as_array() else {
            if is_json_container_string(value) {
                return Err(provider_coercion_error(label, "array"));
            }
            return Ok(array_value);
        };
        let Some(items) = schema.get("items") else {
            return Ok(array_value);
        };
        return array
            .iter()
            .map(|item| normalize_provider_value(item, items, label))
            .collect::<Result<Vec<_>, _>>()
            .map(serde_json::Value::Array);
    }

    if schema_type_matches(schema, "integer") {
        return coerce_integer_string(value, label);
    }

    if schema_type_matches(schema, "number") {
        return coerce_number_string(value, label);
    }

    if schema_type_matches(schema, "boolean") {
        return coerce_boolean_string(value, label);
    }

    Ok(value.clone())
}

fn schema_type_matches(schema: &serde_json::Value, expected: &str) -> bool {
    match schema.get("type") {
        Some(serde_json::Value::String(actual)) => actual == expected,
        Some(serde_json::Value::Array(types)) => {
            types.iter().any(|actual| actual.as_str() == Some(expected))
        }
        _ => false,
    }
}

fn is_json_container_string(value: &serde_json::Value) -> bool {
    value
        .as_str()
        .map(str::trim)
        .is_some_and(|text| text.starts_with('{') || text.starts_with('['))
}

fn coerce_json_string(
    value: &serde_json::Value,
    label: &'static str,
) -> Result<serde_json::Value, AgentLoopHostError> {
    let Some(text) = value.as_str() else {
        return Ok(value.clone());
    };
    let trimmed = text.trim();
    if !(trimmed.starts_with('{') || trimmed.starts_with('[')) {
        return Ok(value.clone());
    }
    serde_json::from_str(trimmed).map_err(|_| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            format!("{label} could not be parsed as schema-declared JSON"),
        )
    })
}

fn coerce_integer_string(
    value: &serde_json::Value,
    label: &'static str,
) -> Result<serde_json::Value, AgentLoopHostError> {
    let Some(text) = value.as_str() else {
        return Ok(value.clone());
    };
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.contains('.') || trimmed.contains('e') || trimmed.contains('E')
    {
        return Err(provider_coercion_error(label, "integer"));
    }
    let parsed = trimmed
        .parse::<i64>()
        .map_err(|_| provider_coercion_error(label, "integer"))?;
    Ok(serde_json::Value::Number(parsed.into()))
}

fn coerce_number_string(
    value: &serde_json::Value,
    label: &'static str,
) -> Result<serde_json::Value, AgentLoopHostError> {
    let Some(text) = value.as_str() else {
        return Ok(value.clone());
    };
    let parsed = text
        .trim()
        .parse::<f64>()
        .map_err(|_| provider_coercion_error(label, "number"))?;
    let number = serde_json::Number::from_f64(parsed)
        .ok_or_else(|| provider_coercion_error(label, "number"))?;
    Ok(serde_json::Value::Number(number))
}

fn coerce_boolean_string(
    value: &serde_json::Value,
    label: &'static str,
) -> Result<serde_json::Value, AgentLoopHostError> {
    let Some(text) = value.as_str() else {
        return Ok(value.clone());
    };
    match text.trim().to_ascii_lowercase().as_str() {
        "true" => Ok(serde_json::Value::Bool(true)),
        "false" => Ok(serde_json::Value::Bool(false)),
        _ => Err(provider_coercion_error(label, "boolean")),
    }
}

fn provider_coercion_error(label: &'static str, expected: &'static str) -> AgentLoopHostError {
    AgentLoopHostError::new(
        AgentLoopHostErrorKind::InvalidInvocation,
        format!("{label} could not be coerced to schema-declared {expected}"),
    )
}

fn provider_tool_name(
    capability_id: &CapabilityId,
    existing: &HashMap<String, CapabilityId>,
) -> String {
    let base = provider_tool_name_base(capability_id.as_str());
    if base.len() <= PROVIDER_TOOL_NAME_MAX_BYTES
        && existing
            .get(&base)
            .is_none_or(|existing_id| existing_id == capability_id)
    {
        return base;
    }
    provider_tool_name_with_digest(&base, capability_id.as_str(), existing, 0)
}

fn provider_tool_name_with_digest(
    base: &str,
    capability_id: &str,
    existing: &HashMap<String, CapabilityId>,
    attempt: u16,
) -> String {
    let digest_input = if attempt == 0 {
        capability_id.to_string()
    } else {
        format!("{capability_id}#{attempt}")
    };
    let digest = sha256_digest_token(digest_input.as_bytes());
    let suffix = digest.strip_prefix("sha256:").unwrap_or(&digest);
    let suffix = &suffix[..PROVIDER_TOOL_NAME_DIGEST_BYTES]; // safety: sha256 hex digest is ASCII and longer than the fixed suffix.
    let prefix_len = PROVIDER_TOOL_NAME_MAX_BYTES.saturating_sub("__".len() + suffix.len());
    let prefix = if base.len() <= prefix_len {
        base
    } else {
        let prefix_end = base
            .char_indices()
            .map(|(index, _)| index)
            .take_while(|index| *index <= prefix_len)
            .last()
            .unwrap_or(0);
        &base[..prefix_end] // safety: prefix_end comes from char_indices(), so it is a UTF-8 boundary.
    };
    let candidate = format!("{prefix}__{suffix}");
    if existing
        .get(&candidate)
        .is_none_or(|existing_id| existing_id.as_str() == capability_id)
        || attempt == u16::MAX
    {
        return candidate;
    }
    provider_tool_name_with_digest(base, capability_id, existing, attempt + 1)
}

fn provider_tool_name_base(capability_id: &str) -> String {
    let mut name = String::with_capacity(capability_id.len());
    for character in capability_id.chars() {
        if character.is_ascii_alphanumeric() || matches!(character, '_' | '-') {
            name.push(character);
        } else if character == '.' {
            name.push_str("__");
        } else {
            name.push('_');
        }
    }
    if name.is_empty() {
        "tool".to_string()
    } else {
        name
    }
}

fn validate_provider_tool_call(tool_call: &ProviderToolCall) -> Result<(), AgentLoopHostError> {
    validate_provider_identity(&tool_call.provider_id, "provider id", 512)?;
    validate_provider_identity(&tool_call.provider_model_id, "provider model id", 512)?;
    let turn_id = tool_call.turn_id.as_deref().ok_or_else(|| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            "provider tool call is missing a provider turn id",
        )
    })?;
    validate_provider_token(turn_id, "provider turn id", 512)?;
    validate_provider_token(&tool_call.id, "provider call id", 512)?;
    validate_provider_tool_name(&tool_call.name)?;
    validate_provider_arguments(&tool_call.arguments)?;
    validate_optional_provider_text(
        &tool_call.response_reasoning,
        "provider response reasoning",
        4096,
    )?;
    validate_optional_provider_text(&tool_call.reasoning, "provider reasoning", 4096)?;
    validate_optional_provider_text(&tool_call.signature, "provider signature", 4096)?;
    Ok(())
}

fn validate_provider_tool_name(value: &str) -> Result<(), AgentLoopHostError> {
    if value.is_empty() {
        return Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            "provider tool name must not be empty",
        ));
    }
    if value.len() > PROVIDER_TOOL_NAME_MAX_BYTES {
        return Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            format!("provider tool name exceeds {PROVIDER_TOOL_NAME_MAX_BYTES} bytes"),
        ));
    }
    if !value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
    {
        return Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            "provider tool name must contain only ASCII letters, digits, _, or -",
        ));
    }
    Ok(())
}

fn validate_provider_identity(
    value: &str,
    label: &'static str,
    max_len: usize,
) -> Result<(), AgentLoopHostError> {
    if value.trim().is_empty() {
        return Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            format!("{label} must not be empty"),
        ));
    }
    if value.len() > max_len {
        return Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            format!("{label} exceeds {max_len} bytes"),
        ));
    }
    if value
        .chars()
        .any(|character| character == '\0' || character.is_control())
    {
        return Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            format!("{label} must not contain NUL/control characters"),
        ));
    }
    Ok(())
}

fn validate_provider_token(
    value: &str,
    label: &'static str,
    max_len: usize,
) -> Result<(), AgentLoopHostError> {
    if value.is_empty() {
        return Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            format!("{label} must not be empty"),
        ));
    }
    if value.len() > max_len {
        return Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            format!("{label} exceeds {max_len} bytes"),
        ));
    }
    if !value.chars().all(|character| {
        character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.' | ':')
    }) {
        return Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            format!("{label} must contain only ASCII letters, digits, _, -, ., or :"),
        ));
    }
    Ok(())
}

fn validate_provider_arguments(arguments: &serde_json::Value) -> Result<(), AgentLoopHostError> {
    let arguments_len = serde_json::to_vec(arguments)
        .map_err(|error| {
            AgentLoopHostError::new(AgentLoopHostErrorKind::InvalidInvocation, error.to_string())
        })?
        .len();
    if arguments_len > 16 * 1024 {
        return Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            "provider tool arguments exceed 16384 bytes",
        ));
    }
    validate_provider_json_value(arguments, "provider arguments", 0)
}

fn validate_provider_json_value(
    value: &serde_json::Value,
    label: &'static str,
    depth: usize,
) -> Result<(), AgentLoopHostError> {
    if depth > 16 {
        return Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            format!("{label} exceed maximum nesting depth"),
        ));
    }
    match value {
        serde_json::Value::String(text) => validate_provider_text_content(text, label),
        serde_json::Value::Array(items) => {
            for item in items {
                validate_provider_json_value(item, label, depth + 1)?;
            }
            Ok(())
        }
        serde_json::Value::Object(entries) => {
            for (key, item) in entries {
                validate_provider_json_key(key)?;
                validate_provider_json_value(item, label, depth + 1)?;
            }
            Ok(())
        }
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {
            Ok(())
        }
    }
}

fn validate_provider_json_key(key: &str) -> Result<(), AgentLoopHostError> {
    if key
        .chars()
        .any(|character| character == '\0' || character.is_control())
    {
        return Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            "provider argument key must not contain NUL/control characters",
        ));
    }
    Ok(())
}

fn validate_optional_provider_text(
    value: &Option<String>,
    label: &'static str,
    max_len: usize,
) -> Result<(), AgentLoopHostError> {
    let Some(value) = value else {
        return Ok(());
    };
    if value.len() > max_len {
        return Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            format!("{label} exceeds {max_len} bytes"),
        ));
    }
    validate_provider_text_content(value, label)
}

fn validate_provider_text_content(
    value: &str,
    label: &'static str,
) -> Result<(), AgentLoopHostError> {
    if value
        .chars()
        .any(|character| character == '\0' || character.is_control())
    {
        return Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            format!("{label} must not contain NUL/control characters"),
        ));
    }
    let lower = value.to_ascii_lowercase();
    for forbidden in SENSITIVE_PROVIDER_TEXT_MARKERS {
        if lower.contains(forbidden) {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                format!("{label} must not contain sensitive marker `{forbidden}`"),
            ));
        }
    }
    if lower
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '-')
        .any(|token| token.starts_with("sk-"))
    {
        return Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            format!("{label} must not contain API-key-like tokens"),
        ));
    }
    Ok(())
}

pub fn concurrency_hint_from_effects(effects: &[EffectKind]) -> ConcurrencyHint {
    if effects.is_empty() {
        return ConcurrencyHint::Exclusive;
    }
    if effects
        .iter()
        .all(|effect| matches!(effect, EffectKind::ReadFilesystem | EffectKind::UseSecret))
    {
        ConcurrencyHint::SafeForParallel
    } else {
        ConcurrencyHint::Exclusive
    }
}

fn should_retry_result_write(
    outcome: &RuntimeCapabilityOutcome,
    result: &Result<CapabilityOutcome, AgentLoopHostError>,
) -> bool {
    matches!(outcome, RuntimeCapabilityOutcome::Completed(_))
        && matches!(
            result,
            Err(error)
                if matches!(
                    error.kind,
                    AgentLoopHostErrorKind::Unavailable
                        | AgentLoopHostErrorKind::TranscriptWriteFailed
                )
        )
}

fn invocation_context_from_visible(
    base: &ExecutionContext,
    run_context: &LoopRunContext,
    capability_id: &CapabilityId,
    capability: &SurfaceCapabilitySnapshot,
    trust: ironclaw_host_api::TrustClass,
    allowed_effects: &[EffectKind],
    execution_mounts: &MountView,
) -> Result<ExecutionContext, AgentLoopHostError> {
    let mut context = base.clone();
    let loop_driver_extension = loop_driver_execution_extension_id(run_context)?;
    context.extension_id = loop_driver_extension.clone();
    context.runtime = capability.runtime;
    context.trust = trust;
    context.grants = invocation_grants_from_visible(
        base,
        capability_id,
        &loop_driver_extension,
        allowed_effects,
    )?;
    // Mount propagation is host-authority only: visible-request contexts must arrive with no
    // caller-supplied mounts, while this invocation context receives the execution mounts that the
    // authority resolver selected for the run and capability dispatch.
    context.mounts = execution_mounts.clone();
    let invocation_id = InvocationId::new();
    context.invocation_id = invocation_id;
    context.correlation_id = CorrelationId::new();
    context.process_id = None;
    context.parent_process_id = None;
    context.resource_scope.invocation_id = invocation_id;
    context.validate().map_err(|_| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            "capability execution context is invalid",
        )
    })?;
    Ok(context)
}

/// Derives the execution extension id for a loop driver.
///
/// Valid extension ids are preserved as-is. Other loop-driver ids are sanitized into a lowercase
/// slug, truncated to leave room for entropy, and suffixed with a digest fragment so separators,
/// case changes, non-ASCII input, and other slug collisions remain distinct.
pub fn loop_driver_execution_extension_id(
    run_context: &LoopRunContext,
) -> Result<ExtensionId, AgentLoopHostError> {
    let raw = run_context.loop_driver_id.as_str();
    if let Ok(extension_id) = ExtensionId::new(raw) {
        return Ok(extension_id);
    }

    let digest = sha256_digest_token(raw.as_bytes());
    let digest_hex = digest.strip_prefix("sha256:").unwrap_or(&digest);
    let slug = extension_id_slug(raw);
    let prefix_budget = 128usize
        .saturating_sub("loop-driver-".len())
        .saturating_sub("-".len())
        .saturating_sub(16);
    let mut candidate = slug.chars().take(prefix_budget).collect::<String>();
    if candidate.is_empty() {
        candidate.push_str("driver");
    }
    ExtensionId::new(format!("loop-driver-{candidate}-{}", &digest_hex[..16])).map_err(|_| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::Internal,
            "loop driver id could not be represented as an execution extension",
        )
    })
}

fn extension_id_slug(value: &str) -> String {
    let mut slug = String::new();
    let mut last_separator = false;
    for byte in value.bytes() {
        let next = match byte {
            b'a'..=b'z' | b'0'..=b'9' => {
                last_separator = false;
                byte as char
            }
            b'A'..=b'Z' => {
                last_separator = false;
                byte.to_ascii_lowercase() as char
            }
            b'_' | b'-' => {
                if last_separator {
                    continue;
                }
                last_separator = true;
                '-'
            }
            b'.' => {
                if slug.is_empty() || last_separator {
                    continue;
                }
                last_separator = true;
                '.'
            }
            _ => {
                if last_separator {
                    continue;
                }
                last_separator = true;
                '-'
            }
        };
        slug.push(next);
    }
    while slug.ends_with(['-', '.']) {
        slug.pop();
    }
    if slug
        .as_bytes()
        .first()
        .is_none_or(|first| !(first.is_ascii_lowercase() || first.is_ascii_digit()))
    {
        slug.insert_str(0, "driver");
    }
    slug
}

fn invocation_grants_from_visible(
    base: &ExecutionContext,
    capability_id: &CapabilityId,
    loop_driver_extension: &ExtensionId,
    allowed_effects: &[EffectKind],
) -> Result<CapabilitySet, AgentLoopHostError> {
    let mut filtered = CapabilitySet::default();
    for grant in &base.grants.grants {
        if grant.capability != *capability_id {
            continue;
        }
        if !grant_principal_matches_visible_context(&grant.grantee, base, loop_driver_extension)
            || !matches!(grant.issued_by, Principal::HostRuntime)
            || !effects_are_covered(&grant.constraints.allowed_effects, allowed_effects)
        {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unauthorized,
                "capability execution context carries an untrusted grant",
            ));
        }
        filtered.grants.push(grant.clone());
    }
    Ok(filtered)
}

fn grant_principal_matches_visible_context(
    principal: &Principal,
    context: &ExecutionContext,
    loop_driver_extension: &ExtensionId,
) -> bool {
    match principal {
        Principal::Tenant(id) => id == &context.tenant_id,
        Principal::User(id) => id == &context.user_id,
        Principal::Agent(id) => context.agent_id.as_ref() == Some(id),
        Principal::Project(id) => context.project_id.as_ref() == Some(id),
        Principal::Mission(id) => context.mission_id.as_ref() == Some(id),
        Principal::Thread(id) => context.thread_id.as_ref() == Some(id),
        Principal::Extension(id) => id == loop_driver_extension,
        Principal::HostRuntime | Principal::System(_) => false,
    }
}

fn effects_are_covered(required: &[EffectKind], allowed: &[EffectKind]) -> bool {
    required.iter().all(|effect| allowed.contains(effect))
}

fn invocation_idempotency_key(
    run_context: &LoopRunContext,
    request: &CapabilityInvocation,
) -> Result<IdempotencyKey, AgentLoopHostError> {
    let payload = format!(
        "loop-capability\nrun={}\nsurface={}\ncapability={}\ninput={}",
        run_context.run_id,
        request.surface_version.as_str(),
        request.capability_id.as_str(),
        request.input_ref.as_str()
    );
    IdempotencyKey::new(format!(
        "loop-capability:{}",
        sha256_digest_token(payload.as_bytes())
    ))
    .map_err(host_runtime_error)
}

fn provider_tool_call_input_ref(
    run_context: &LoopRunContext,
    tool_call: &ProviderToolCall,
) -> Result<CapabilityInputRef, AgentLoopHostError> {
    let turn_id = tool_call.turn_id.as_deref().ok_or_else(|| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            "provider tool call is missing a provider turn id",
        )
    })?;
    let arguments = serde_json::to_string(&tool_call.arguments).map_err(|error| {
        AgentLoopHostError::new(AgentLoopHostErrorKind::InvalidInvocation, error.to_string())
    })?;
    let payload = format!(
        "provider-tool-input\nrun={}\nprovider={}\nmodel={}\nturn={}\ncall={}\ntool={}\narguments={}",
        run_context.run_id,
        tool_call.provider_id,
        tool_call.provider_model_id,
        turn_id,
        tool_call.id,
        tool_call.name,
        arguments
    );
    let digest = sha256_digest_token(payload.as_bytes());
    let digest = digest.strip_prefix("sha256:").unwrap_or(&digest);
    CapabilityInputRef::new(format!("input:provider-tool-{digest}")).map_err(|_| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::Internal,
            "provider tool-call input ref could not be represented",
        )
    })
}

fn loop_surface_version(
    version: &str,
) -> Result<ironclaw_turns::run_profile::CapabilitySurfaceVersion, AgentLoopHostError> {
    ironclaw_turns::run_profile::CapabilitySurfaceVersion::new(version).map_err(|_| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::Internal,
            "host runtime capability surface version could not be represented",
        )
    })
}

async fn runtime_outcome_to_loop(
    run_context: &LoopRunContext,
    result_writer: &(dyn LoopCapabilityResultWriter + Send + Sync),
    requested_capability_id: &CapabilityId,
    outcome: RuntimeCapabilityOutcome,
) -> Result<CapabilityOutcome, AgentLoopHostError> {
    ensure_runtime_outcome_matches(requested_capability_id, &outcome)?;
    Ok(match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => {
            let result_ref = result_writer
                .write_capability_result(
                    run_context,
                    &completed.capability_id,
                    completed.output.clone(),
                )
                .await?;
            CapabilityOutcome::Completed(CapabilityResultMessage {
                result_ref,
                safe_summary: "capability completed".to_string(),
                terminate_hint: false,
            })
        }
        RuntimeCapabilityOutcome::ApprovalRequired(gate) => CapabilityOutcome::ApprovalRequired {
            gate_ref: loop_gate_ref("approval", gate.approval_request_id.to_string())?,
            safe_summary: blocked_summary(gate.reason).to_string(),
        },
        RuntimeCapabilityOutcome::AuthRequired(gate) => CapabilityOutcome::AuthRequired {
            gate_ref: loop_gate_ref("auth", gate.gate_id.to_string())?,
            safe_summary: blocked_summary(gate.reason).to_string(),
        },
        RuntimeCapabilityOutcome::ResourceBlocked(gate) => CapabilityOutcome::ResourceBlocked {
            gate_ref: loop_gate_ref("resource", gate.gate_id.to_string())?,
            safe_summary: blocked_summary(gate.reason).to_string(),
        },
        RuntimeCapabilityOutcome::SpawnedProcess(process) => {
            CapabilityOutcome::SpawnedProcess(ProcessHandleSummary {
                process_ref: LoopProcessRef::new(format!("process:{}", process.process_id))
                    .map_err(|_| {
                        AgentLoopHostError::new(
                            AgentLoopHostErrorKind::Internal,
                            "process ref could not be represented",
                        )
                    })?,
                safe_summary: "capability spawned background work".to_string(),
            })
        }
        RuntimeCapabilityOutcome::Failed(failure) => {
            if failure.kind == RuntimeFailureKind::Authorization {
                CapabilityOutcome::Denied(CapabilityDenied {
                    reason_kind: capability_denied_reason_kind(failure.kind.as_str())?,
                    safe_summary: runtime_safe_summary(
                        failure.message,
                        "capability authorization denied",
                    ),
                })
            } else {
                CapabilityOutcome::Failed(CapabilityFailure {
                    error_kind: runtime_failure_kind_to_loop(failure.kind)?,
                    safe_summary: runtime_safe_summary(
                        failure.message,
                        "capability invocation failed",
                    ),
                })
            }
        }
        RuntimeCapabilityOutcome::Unknown(unknown) => {
            CapabilityOutcome::Failed(CapabilityFailure {
                error_kind: capability_failure_kind(unknown.kind)?,
                safe_summary: runtime_safe_summary(
                    unknown.message,
                    "capability invocation returned an unknown outcome",
                ),
            })
        }
    })
}

fn runtime_failure_kind_to_loop(
    kind: RuntimeFailureKind,
) -> Result<CapabilityFailureKind, AgentLoopHostError> {
    Ok(match kind {
        RuntimeFailureKind::Authorization => CapabilityFailureKind::Authorization,
        RuntimeFailureKind::Backend => CapabilityFailureKind::Backend,
        RuntimeFailureKind::Cancelled => CapabilityFailureKind::Cancelled,
        RuntimeFailureKind::Dispatcher => CapabilityFailureKind::Dispatcher,
        RuntimeFailureKind::InvalidInput => CapabilityFailureKind::InvalidInput,
        RuntimeFailureKind::MissingRuntime => CapabilityFailureKind::MissingRuntime,
        RuntimeFailureKind::Network => CapabilityFailureKind::Network,
        RuntimeFailureKind::OutputTooLarge => CapabilityFailureKind::OutputTooLarge,
        RuntimeFailureKind::Process => CapabilityFailureKind::Process,
        RuntimeFailureKind::Resource => CapabilityFailureKind::Resource,
        RuntimeFailureKind::Unknown => capability_failure_kind("unknown")?,
        _ => capability_failure_kind(kind.as_str())?,
    })
}

fn ensure_runtime_outcome_matches(
    expected: &CapabilityId,
    outcome: &RuntimeCapabilityOutcome,
) -> Result<(), AgentLoopHostError> {
    let actual = match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => &completed.capability_id,
        RuntimeCapabilityOutcome::ApprovalRequired(gate) => &gate.capability_id,
        RuntimeCapabilityOutcome::AuthRequired(gate) => &gate.capability_id,
        RuntimeCapabilityOutcome::ResourceBlocked(gate) => &gate.capability_id,
        RuntimeCapabilityOutcome::SpawnedProcess(process) => &process.capability_id,
        RuntimeCapabilityOutcome::Failed(failure) => &failure.capability_id,
        RuntimeCapabilityOutcome::Unknown(unknown) => &unknown.capability_id,
    };
    if actual != expected {
        return Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::Internal,
            "host runtime returned outcome for a different capability",
        ));
    }
    Ok(())
}

fn capability_denied_reason_kind(
    value: impl Into<String>,
) -> Result<CapabilityDeniedReasonKind, AgentLoopHostError> {
    CapabilityDeniedReasonKind::unknown(value).map_err(|_| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::Internal,
            "capability denied reason kind could not be represented",
        )
    })
}

fn capability_failure_kind(
    value: impl Into<String>,
) -> Result<CapabilityFailureKind, AgentLoopHostError> {
    CapabilityFailureKind::unknown(value).map_err(|_| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::Internal,
            "capability failure kind could not be represented",
        )
    })
}

fn runtime_safe_summary(message: Option<String>, fallback: &'static str) -> String {
    message
        .and_then(|summary| LoopSafeSummary::new(summary).ok())
        .map(|summary| summary.to_string())
        .unwrap_or_else(|| fallback.to_string())
}

fn loop_gate_ref(kind: &str, id: String) -> Result<LoopGateRef, AgentLoopHostError> {
    LoopGateRef::new(format!("gate:{kind}-{id}")).map_err(|_| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::Internal,
            "capability gate ref could not be represented",
        )
    })
}

fn blocked_summary(reason: RuntimeBlockedReason) -> &'static str {
    match reason {
        RuntimeBlockedReason::ApprovalRequired => "capability requires approval",
        RuntimeBlockedReason::AuthRequired => "capability requires authentication",
        RuntimeBlockedReason::ResourceLimit => "capability is blocked by resource limits",
        RuntimeBlockedReason::ResourceUnavailable => "capability resources are unavailable",
    }
}

fn host_runtime_error(error: HostRuntimeError) -> AgentLoopHostError {
    match error {
        HostRuntimeError::InvalidRequest { reason } => AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            runtime_safe_summary(Some(reason), "host runtime rejected capability request"),
        ),
        HostRuntimeError::Unavailable { reason } => AgentLoopHostError::new(
            AgentLoopHostErrorKind::Unavailable,
            runtime_safe_summary(
                Some(reason),
                "host runtime capability service is unavailable",
            ),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use ironclaw_host_api::{
        AgentId, CapabilityDescriptor, CapabilityGrant, CapabilityGrantId, GrantConstraints,
        MountAlias, MountGrant, MountPermissions, NetworkPolicy, PermissionMode, ProjectId,
        ResourceUsage, TenantId, TrustClass, UserId, VirtualPath,
    };
    use ironclaw_host_runtime::{
        CancelRuntimeWorkOutcome, CancelRuntimeWorkRequest, CapabilitySurfaceVersion,
        HostRuntimeHealth, HostRuntimeStatus, RuntimeCapabilityCompleted,
        RuntimeCapabilityResumeRequest, RuntimeStatusRequest, SurfaceKind, VisibleCapability,
        VisibleCapabilityAccess, VisibleCapabilitySurface,
    };
    use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};
    use ironclaw_turns::{
        InMemoryRunProfileResolver, LoopDriverId, RunProfileResolutionRequest, RunProfileResolver,
        TurnId, TurnRunId, TurnScope,
    };

    #[test]
    fn concurrency_hint_treats_empty_effects_as_exclusive() {
        assert_eq!(
            concurrency_hint_from_effects(&[]),
            ConcurrencyHint::Exclusive
        );
    }

    #[test]
    fn concurrency_hint_treats_read_and_secret_effects_as_parallel_safe() {
        let effects = vec![EffectKind::ReadFilesystem, EffectKind::UseSecret];

        assert_eq!(
            concurrency_hint_from_effects(&effects),
            ConcurrencyHint::SafeForParallel
        );
    }

    #[test]
    fn concurrency_hint_treats_any_mutating_effect_as_exclusive() {
        let exclusive_effects = [
            EffectKind::WriteFilesystem,
            EffectKind::DeleteFilesystem,
            EffectKind::Network,
            EffectKind::ExecuteCode,
            EffectKind::SpawnProcess,
            EffectKind::DispatchCapability,
            EffectKind::ModifyExtension,
            EffectKind::ModifyApproval,
            EffectKind::ModifyBudget,
            EffectKind::ExternalWrite,
            EffectKind::Financial,
        ];

        for effect in exclusive_effects {
            assert_eq!(
                concurrency_hint_from_effects(&[effect]),
                ConcurrencyHint::Exclusive,
                "{effect:?}"
            );
        }
    }

    #[test]
    fn runtime_failure_kind_mapping_preserves_current_categories() {
        let cases = [
            (
                RuntimeFailureKind::Authorization,
                CapabilityFailureKind::Authorization,
            ),
            (RuntimeFailureKind::Backend, CapabilityFailureKind::Backend),
            (
                RuntimeFailureKind::Cancelled,
                CapabilityFailureKind::Cancelled,
            ),
            (
                RuntimeFailureKind::Dispatcher,
                CapabilityFailureKind::Dispatcher,
            ),
            (
                RuntimeFailureKind::InvalidInput,
                CapabilityFailureKind::InvalidInput,
            ),
            (
                RuntimeFailureKind::MissingRuntime,
                CapabilityFailureKind::MissingRuntime,
            ),
            (RuntimeFailureKind::Network, CapabilityFailureKind::Network),
            (
                RuntimeFailureKind::OutputTooLarge,
                CapabilityFailureKind::OutputTooLarge,
            ),
            (RuntimeFailureKind::Process, CapabilityFailureKind::Process),
            (
                RuntimeFailureKind::Resource,
                CapabilityFailureKind::Resource,
            ),
        ];

        for (runtime, expected) in cases {
            assert_eq!(
                runtime_failure_kind_to_loop(runtime).expect("mapped failure kind"),
                expected,
                "{runtime:?}"
            );
        }

        assert_eq!(
            runtime_failure_kind_to_loop(RuntimeFailureKind::Unknown)
                .expect("unknown failure kind")
                .as_str(),
            "unknown"
        );
    }

    #[test]
    fn provider_schema_accepts_zero_arg_object_tools() {
        assert!(provider_schema_is_usable(
            &serde_json::json!({"type":"object"})
        ));
        assert!(provider_schema_is_usable(
            &serde_json::json!({"type":"object","properties":{}})
        ));
        assert!(provider_schema_is_usable(&serde_json::json!({
            "$ref": "schemas/builtin/write-file.input.v1.json"
        })));
        assert!(!provider_schema_is_usable(
            &serde_json::json!({"type":"string"})
        ));
    }

    #[test]
    fn provider_tool_name_is_bounded_and_uses_digest_entropy() {
        let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
        let mut existing = HashMap::new();
        existing.insert(
            "demo__echo".to_string(),
            CapabilityId::new("demo.other").expect("valid capability id"),
        );
        let name = provider_tool_name(&capability_id, &existing);

        assert!(name.len() <= PROVIDER_TOOL_NAME_MAX_BYTES);
        assert!(
            name.chars().all(
                |character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-')
            )
        );
        let suffix = name.rsplit("__").next().expect("digest suffix");
        assert_eq!(suffix.len(), PROVIDER_TOOL_NAME_DIGEST_BYTES);
        assert!(
            suffix
                .chars()
                .all(|character| character.is_ascii_hexdigit())
        );
    }

    #[test]
    fn provider_tool_name_normalizes_provider_unsafe_characters() {
        let capability_id = CapabilityId::new("demo.echo.v1").expect("valid capability id");
        let name = provider_tool_name(&capability_id, &HashMap::new());

        assert_eq!(name, "demo__echo__v1");
        validate_provider_tool_name(&name).expect("provider-safe name");
    }

    #[test]
    fn provider_tool_call_validation_rejects_provider_unsafe_tool_name() {
        let mut call = provider_tool_call();
        call.name = "demo.echo".to_string();

        let error = validate_provider_tool_call(&call).expect_err("unsafe name rejected");
        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    }

    #[test]
    fn provider_tool_call_validation_requires_turn_id() {
        let mut call = provider_tool_call();
        call.turn_id = None;

        let error = validate_provider_tool_call(&call).expect_err("missing turn id rejected");
        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    }

    #[test]
    fn provider_tool_call_validation_rejects_sensitive_metadata() {
        let mut call = provider_tool_call();
        call.arguments = serde_json::json!({"password":"sk-live-secret"});
        assert!(validate_provider_tool_call(&call).is_err());

        let mut call = provider_tool_call();
        call.reasoning = Some("provider error included traceback".to_string());
        assert!(validate_provider_tool_call(&call).is_err());
    }

    #[test]
    fn provider_argument_normalization_coerces_schema_declared_scalars() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "limit": { "type": "integer" },
                "enabled": { "type": "boolean" },
                "threshold": { "type": "number" },
                "message": { "type": "string" }
            }
        });
        let normalized = normalize_provider_arguments(
            &serde_json::json!({
                "limit": "10",
                "enabled": "true",
                "threshold": "1.5",
                "message": "10"
            }),
            &schema,
            "provider arguments",
        )
        .expect("normalized arguments");

        assert_eq!(
            normalized,
            serde_json::json!({
                "limit": 10,
                "enabled": true,
                "threshold": 1.5,
                "message": "10"
            })
        );
    }

    #[test]
    fn provider_argument_normalization_coerces_stringified_containers() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "rows": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "index": { "type": "integer" },
                            "bold": { "type": "boolean" }
                        }
                    }
                }
            }
        });
        let normalized = normalize_provider_arguments(
            &serde_json::json!({
                "rows": "[{\"index\":\"1\",\"bold\":\"false\"}]"
            }),
            &schema,
            "provider arguments",
        )
        .expect("normalized arguments");

        assert_eq!(
            normalized,
            serde_json::json!({
                "rows": [{ "index": 1, "bold": false }]
            })
        );
    }

    #[test]
    fn provider_argument_normalization_rejects_invalid_schema_declared_integer() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "limit": { "type": "integer" }
            }
        });

        let error = normalize_provider_arguments(
            &serde_json::json!({ "limit": "ten" }),
            &schema,
            "provider arguments",
        )
        .expect_err("invalid integer should fail closed");

        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    }

    #[test]
    fn provider_argument_normalization_rejects_mismatched_stringified_object() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "options": {
                    "type": "object",
                    "properties": {
                        "enabled": { "type": "boolean" }
                    }
                }
            }
        });

        let error = normalize_provider_arguments(
            &serde_json::json!({ "options": "[{\"enabled\":\"true\"}]" }),
            &schema,
            "provider arguments",
        )
        .expect_err("stringified array should not satisfy object schema");

        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    }

    #[test]
    fn provider_argument_normalization_rejects_mismatched_stringified_array() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "rows": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "index": { "type": "integer" }
                        }
                    }
                }
            }
        });

        let error = normalize_provider_arguments(
            &serde_json::json!({ "rows": "{\"index\":\"1\"}" }),
            &schema,
            "provider arguments",
        )
        .expect_err("stringified object should not satisfy array schema");

        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    }

    #[test]
    fn provider_argument_normalization_rejects_mismatched_stringified_array_without_items() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "rows": { "type": "array" }
            }
        });

        let error = normalize_provider_arguments(
            &serde_json::json!({ "rows": "{\"index\":\"1\"}" }),
            &schema,
            "provider arguments",
        )
        .expect_err("stringified object should not satisfy array schema without items");

        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    }

    fn provider_tool_call() -> ProviderToolCall {
        ProviderToolCall {
            provider_id: "provider".to_string(),
            provider_model_id: "model".to_string(),
            turn_id: Some("turn_1".to_string()),
            id: "call_1".to_string(),
            name: "demo__echo".to_string(),
            arguments: serde_json::json!({"message":"hello"}),
            response_reasoning: None,
            reasoning: None,
            signature: None,
        }
    }

    struct FallbackInputResolver;

    #[async_trait]
    impl LoopCapabilityInputResolver for FallbackInputResolver {
        async fn resolve_capability_input(
            &self,
            _run_context: &LoopRunContext,
            _input_ref: &CapabilityInputRef,
        ) -> Result<serde_json::Value, AgentLoopHostError> {
            Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "fallback input resolver should not be used",
            ))
        }
    }

    #[tokio::test]
    async fn provider_tool_call_input_resolver_stages_arguments() {
        let run_context = loop_run_context(&execution_context("thread-provider-input")).await;
        let resolver = ProviderToolCallInputResolver::new(Arc::new(FallbackInputResolver));
        let call = provider_tool_call();

        let input_ref = resolver
            .register_provider_tool_call_input(&run_context, &call)
            .await
            .expect("provider input should stage");
        let resolved = resolver
            .resolve_capability_input(&run_context, &input_ref)
            .await
            .expect("provider input should resolve");

        assert!(input_ref.as_str().starts_with("input:provider-tool-"));
        assert_eq!(resolved, serde_json::json!({"message":"hello"}));
    }

    #[tokio::test]
    async fn factory_with_execution_mounts_propagates_to_port() {
        let context = execution_context("thread-factory-mounts");
        let run_context = loop_run_context(&context).await;
        let execution_mounts = execution_mounts();
        let factory = HostRuntimeLoopCapabilityPortFactory::new(
            dummy_runtime(),
            visible_request(context),
            dummy_input_resolver(),
            dummy_result_writer(),
            dummy_milestone_sink(),
        )
        .with_execution_mounts(execution_mounts.clone());

        let port = factory.port_for_run_context(run_context);

        assert_eq!(port.execution_mounts, execution_mounts);
    }

    #[tokio::test]
    async fn port_with_execution_mounts_sets_field() {
        let context = execution_context("thread-port-mounts");
        let run_context = loop_run_context(&context).await;
        let execution_mounts = execution_mounts();
        let port = HostRuntimeLoopCapabilityPort::new(
            dummy_runtime(),
            run_context,
            visible_request(context),
            dummy_input_resolver(),
            dummy_result_writer(),
            dummy_milestone_sink(),
        )
        .with_execution_mounts(execution_mounts.clone());

        assert_eq!(port.execution_mounts, execution_mounts);
    }

    #[tokio::test]
    async fn invoke_capability_uses_capability_specific_execution_mounts() {
        let default_id = CapabilityId::new("demo.default").expect("valid capability id");
        let override_id = CapabilityId::new("demo.override").expect("valid capability id");
        let provider_id = ExtensionId::new("demo").expect("valid provider id");
        let mut context = execution_context("thread-capability-specific-mounts");
        let run_context = loop_run_context(&context).await;
        let loop_driver_extension =
            loop_driver_execution_extension_id(&run_context).expect("valid extension id");
        context.grants.grants.extend([
            dispatch_capability_grant(&default_id, &loop_driver_extension),
            dispatch_capability_grant(&override_id, &loop_driver_extension),
        ]);

        let runtime = Arc::new(RecordingHostRuntime::new(vec![
            visible_capability(default_id.clone(), provider_id.clone()),
            visible_capability(override_id.clone(), provider_id.clone()),
        ]));
        let visible_request = visible_request(context).with_provider_trust(
            std::collections::BTreeMap::from([(provider_id, dispatch_trust_decision())]),
        );
        let default_mounts = mount_view("/workspace", "/projects/workspace");
        let override_mounts = mount_view("/skills", "/projects/skills");
        let port = HostRuntimeLoopCapabilityPortFactory::new(
            runtime.clone(),
            visible_request,
            Arc::new(StaticInputResolver),
            Arc::new(StaticResultWriter),
            dummy_milestone_sink(),
        )
        .with_execution_mounts(default_mounts.clone())
        .with_capability_execution_mount(override_id.clone(), override_mounts.clone())
        .port_for_run_context(run_context);
        let surface = port
            .visible_capabilities(VisibleCapabilityRequest {})
            .await
            .expect("visible capabilities load");
        let input_ref = CapabilityInputRef::new("input:mount-test").expect("valid input ref");

        port.invoke_capability(CapabilityInvocation {
            surface_version: surface.version.clone(),
            capability_id: override_id.clone(),
            input_ref: input_ref.clone(),
        })
        .await
        .expect("override invocation succeeds");
        port.invoke_capability(CapabilityInvocation {
            surface_version: surface.version,
            capability_id: default_id.clone(),
            input_ref,
        })
        .await
        .expect("default invocation succeeds");

        let requests = runtime.take_requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].capability_id, override_id);
        assert_eq!(requests[0].context.mounts, override_mounts);
        assert_eq!(requests[1].capability_id, default_id);
        assert_eq!(requests[1].context.mounts, default_mounts);
    }

    #[tokio::test]
    async fn invocation_context_rejects_same_scope_elevated_grant() {
        let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
        let mut context = execution_context("thread-elevated-grant");
        let run_context = loop_run_context(&context).await;
        let loop_driver_extension =
            ExtensionId::new(run_context.loop_driver_id.as_str()).expect("valid extension id");
        context.grants.grants.push(CapabilityGrant {
            id: CapabilityGrantId::new(),
            capability: capability_id.clone(),
            grantee: Principal::Extension(loop_driver_extension),
            issued_by: Principal::HostRuntime,
            constraints: GrantConstraints {
                allowed_effects: vec![EffectKind::WriteFilesystem],
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: None,
            },
        });
        let capability = SurfaceCapabilitySnapshot {
            provider: ExtensionId::new("demo").expect("valid provider"),
            runtime: RuntimeKind::Wasm,
            estimate: ResourceEstimate::default(),
            safe_description: "demo capability".to_string(),
            parameters_schema: serde_json::json!({"type":"object"}),
            provider_tool_name: "demo__echo".to_string(),
        };

        let err = invocation_context_from_visible(
            &context,
            &run_context,
            &capability_id,
            &capability,
            TrustClass::Sandbox,
            &[EffectKind::ReadFilesystem],
            &MountView::default(),
        )
        .expect_err("elevated grant must be rejected");

        assert_eq!(err.kind, AgentLoopHostErrorKind::Unauthorized);
    }

    #[tokio::test]
    async fn invocation_context_preserves_host_mount_grants_without_context_mounts() {
        let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
        let mut context = execution_context("thread-host-mount-grant");
        let run_context = loop_run_context(&context).await;
        let loop_driver_extension =
            ExtensionId::new(run_context.loop_driver_id.as_str()).expect("valid extension id");
        let grant_mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/workspace").expect("valid mount alias"),
            VirtualPath::new("/projects/demo").expect("valid virtual path"),
            MountPermissions::read_only(),
        )])
        .expect("valid mount view");
        context.grants.grants.push(CapabilityGrant {
            id: CapabilityGrantId::new(),
            capability: capability_id.clone(),
            grantee: Principal::Extension(loop_driver_extension),
            issued_by: Principal::HostRuntime,
            constraints: GrantConstraints {
                allowed_effects: vec![EffectKind::ReadFilesystem],
                mounts: grant_mounts.clone(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: None,
            },
        });
        let capability = SurfaceCapabilitySnapshot {
            provider: ExtensionId::new("demo").expect("valid provider"),
            runtime: RuntimeKind::Wasm,
            estimate: ResourceEstimate::default(),
            safe_description: "demo capability".to_string(),
            parameters_schema: serde_json::json!({"type":"object"}),
            provider_tool_name: "demo__echo".to_string(),
        };

        let invocation_context = invocation_context_from_visible(
            &context,
            &run_context,
            &capability_id,
            &capability,
            TrustClass::Sandbox,
            &[EffectKind::ReadFilesystem],
            &grant_mounts,
        )
        .expect("host-issued mount grant should be preserved");

        assert_eq!(invocation_context.mounts, grant_mounts);
        assert_eq!(invocation_context.grants.grants.len(), 1);
        assert_eq!(
            invocation_context.grants.grants[0].constraints.mounts,
            grant_mounts
        );
    }

    #[tokio::test]
    async fn invocation_context_preserves_matching_host_scope_grant() {
        let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
        let mut context = execution_context("thread-host-scope-grant");
        let run_context = loop_run_context(&context).await;
        context.grants.grants.push(CapabilityGrant {
            id: CapabilityGrantId::new(),
            capability: capability_id.clone(),
            grantee: Principal::Thread(context.thread_id.clone().expect("thread id")),
            issued_by: Principal::HostRuntime,
            constraints: GrantConstraints {
                allowed_effects: vec![EffectKind::ReadFilesystem],
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: None,
            },
        });
        let capability = SurfaceCapabilitySnapshot {
            provider: ExtensionId::new("demo").expect("valid provider"),
            runtime: RuntimeKind::Wasm,
            estimate: ResourceEstimate::default(),
            safe_description: "demo capability".to_string(),
            parameters_schema: serde_json::json!({"type":"object"}),
            provider_tool_name: "demo__echo".to_string(),
        };

        let invocation_context = invocation_context_from_visible(
            &context,
            &run_context,
            &capability_id,
            &capability,
            TrustClass::Sandbox,
            &[EffectKind::ReadFilesystem],
            &MountView::default(),
        )
        .expect("matching host scope grant should be preserved");

        assert_eq!(invocation_context.grants.grants.len(), 1);
        assert!(matches!(
            &invocation_context.grants.grants[0].grantee,
            Principal::Thread(_)
        ));
    }

    #[tokio::test]
    async fn invocation_context_derives_extension_id_for_planned_driver_namespaced_id() {
        let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
        let mut context = execution_context("thread-planned-driver-id");
        let mut run_context = loop_run_context(&context).await;
        run_context.loop_driver_id =
            LoopDriverId::new("reborn:planned-default").expect("valid loop driver id");
        context.grants.grants.push(CapabilityGrant {
            id: CapabilityGrantId::new(),
            capability: capability_id.clone(),
            grantee: Principal::User(context.user_id.clone()),
            issued_by: Principal::HostRuntime,
            constraints: GrantConstraints {
                allowed_effects: vec![EffectKind::DispatchCapability],
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: None,
            },
        });
        let capability = SurfaceCapabilitySnapshot {
            provider: ExtensionId::new("demo").expect("valid provider"),
            runtime: RuntimeKind::FirstParty,
            estimate: ResourceEstimate::default(),
            safe_description: "demo echo".to_string(),
            parameters_schema: serde_json::json!({ "type": "object" }),
            provider_tool_name: "demo_echo".to_string(),
        };

        let invocation_context = invocation_context_from_visible(
            &context,
            &run_context,
            &capability_id,
            &capability,
            TrustClass::FirstParty,
            &[EffectKind::DispatchCapability],
            &MountView::default(),
        )
        .expect("planned driver id should derive a valid execution principal");

        assert_eq!(
            invocation_context.extension_id,
            loop_driver_execution_extension_id(&run_context).expect("valid extension")
        );
        assert_eq!(invocation_context.grants.grants.len(), 1);
    }

    #[tokio::test]
    async fn loop_driver_execution_extension_id_includes_digest_to_avoid_slug_collisions() {
        let context = execution_context("thread-planned-driver-collisions");
        let mut colon_context = loop_run_context(&context).await;
        colon_context.loop_driver_id =
            LoopDriverId::new("reborn:planned-default").expect("valid loop driver id");
        let mut dash_context = loop_run_context(&context).await;
        dash_context.loop_driver_id =
            LoopDriverId::new("reborn-planned-default").expect("valid loop driver id");

        let colon_id =
            loop_driver_execution_extension_id(&colon_context).expect("valid extension id");
        let dash_id =
            loop_driver_execution_extension_id(&dash_context).expect("valid extension id");

        assert_ne!(colon_id, dash_id);
        assert!(
            colon_id
                .as_str()
                .starts_with("loop-driver-reborn-planned-default-")
        );
        assert_eq!(dash_id.as_str(), "reborn-planned-default");
    }

    #[tokio::test]
    async fn invocation_context_derives_runtime_authority_from_loop_and_surface() {
        let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
        let mut context = execution_context("thread-derived-authority");
        let run_context = loop_run_context(&context).await;
        let loop_driver_extension =
            ExtensionId::new(run_context.loop_driver_id.as_str()).expect("valid extension id");
        context.extension_id = ExtensionId::new("caller-supplied").expect("valid extension id");
        context.runtime = RuntimeKind::System;
        context.trust = TrustClass::System;
        context.mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/workspace").expect("valid mount alias"),
            VirtualPath::new("/projects/demo").expect("valid virtual path"),
            MountPermissions::read_write(),
        )])
        .expect("valid mount view");
        context.grants.grants.push(CapabilityGrant {
            id: CapabilityGrantId::new(),
            capability: capability_id.clone(),
            grantee: Principal::Extension(loop_driver_extension.clone()),
            issued_by: Principal::HostRuntime,
            constraints: GrantConstraints {
                allowed_effects: vec![EffectKind::DispatchCapability],
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: None,
            },
        });
        let capability = SurfaceCapabilitySnapshot {
            provider: ExtensionId::new("demo").expect("valid provider"),
            runtime: RuntimeKind::Script,
            estimate: ResourceEstimate::default(),
            safe_description: "demo capability".to_string(),
            parameters_schema: serde_json::json!({"type":"object"}),
            provider_tool_name: "demo__echo".to_string(),
        };

        let invocation_context = invocation_context_from_visible(
            &context,
            &run_context,
            &capability_id,
            &capability,
            TrustClass::UserTrusted,
            &[EffectKind::DispatchCapability],
            &MountView::default(),
        )
        .expect("context");

        assert_eq!(invocation_context.extension_id, loop_driver_extension);
        assert_eq!(invocation_context.runtime, RuntimeKind::Script);
        assert_eq!(invocation_context.trust, TrustClass::UserTrusted);
        assert_eq!(invocation_context.mounts, MountView::default());
        assert_eq!(invocation_context.grants.grants.len(), 1);
    }

    fn visible_request(
        context: ExecutionContext,
    ) -> ironclaw_host_runtime::VisibleCapabilityRequest {
        ironclaw_host_runtime::VisibleCapabilityRequest::new(
            context,
            SurfaceKind::new("test").expect("valid surface kind"),
        )
    }

    fn execution_mounts() -> MountView {
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/execution").expect("valid mount alias"),
            VirtualPath::new("/projects/execution").expect("valid virtual path"),
            MountPermissions::read_only(),
        )])
        .expect("valid mount view")
    }

    fn mount_view(alias: &str, target: &str) -> MountView {
        MountView::new(vec![MountGrant::new(
            MountAlias::new(alias).expect("valid mount alias"),
            VirtualPath::new(target).expect("valid virtual path"),
            MountPermissions::read_write_list_delete(),
        )])
        .expect("valid mount view")
    }

    fn dispatch_capability_grant(
        capability_id: &CapabilityId,
        grantee: &ExtensionId,
    ) -> CapabilityGrant {
        CapabilityGrant {
            id: CapabilityGrantId::new(),
            capability: capability_id.clone(),
            grantee: Principal::Extension(grantee.clone()),
            issued_by: Principal::HostRuntime,
            constraints: GrantConstraints {
                allowed_effects: vec![EffectKind::DispatchCapability],
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: None,
            },
        }
    }

    fn dispatch_trust_decision() -> TrustDecision {
        TrustDecision {
            effective_trust: EffectiveTrustClass::user_trusted(),
            authority_ceiling: AuthorityCeiling {
                allowed_effects: vec![EffectKind::DispatchCapability],
                max_resource_ceiling: None,
            },
            provenance: TrustProvenance::Default,
            evaluated_at: chrono::Utc::now(),
        }
    }

    fn visible_capability(id: CapabilityId, provider: ExtensionId) -> VisibleCapability {
        VisibleCapability {
            descriptor: CapabilityDescriptor {
                id,
                provider,
                runtime: RuntimeKind::FirstParty,
                trust_ceiling: TrustClass::UserTrusted,
                description: "demo capability".to_string(),
                parameters_schema: serde_json::json!({"type":"object"}),
                effects: vec![EffectKind::DispatchCapability],
                default_permission: PermissionMode::Allow,
                runtime_credentials: Vec::new(),
                resource_profile: None,
            },
            access: VisibleCapabilityAccess::Available,
            estimated_resources: ResourceEstimate::default(),
        }
    }

    fn dummy_runtime() -> Arc<dyn HostRuntime> {
        Arc::new(NoopHostRuntime)
    }

    fn dummy_input_resolver() -> Arc<dyn LoopCapabilityInputResolver> {
        Arc::new(NoopCapabilityIo)
    }

    fn dummy_result_writer() -> Arc<dyn LoopCapabilityResultWriter> {
        Arc::new(NoopCapabilityIo)
    }

    fn dummy_milestone_sink() -> Arc<dyn LoopHostMilestoneSink> {
        Arc::new(ironclaw_turns::run_profile::InMemoryLoopHostMilestoneSink::default())
    }

    struct RecordingHostRuntime {
        capabilities: Vec<VisibleCapability>,
        requests: Mutex<Vec<RuntimeCapabilityRequest>>,
    }

    impl RecordingHostRuntime {
        fn new(capabilities: Vec<VisibleCapability>) -> Self {
            Self {
                capabilities,
                requests: Mutex::new(Vec::new()),
            }
        }

        fn take_requests(&self) -> Vec<RuntimeCapabilityRequest> {
            self.requests.lock().expect("requests lock").clone()
        }
    }

    #[async_trait]
    impl HostRuntime for RecordingHostRuntime {
        async fn invoke_capability(
            &self,
            request: RuntimeCapabilityRequest,
        ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
            self.requests
                .lock()
                .expect("requests lock")
                .push(request.clone());
            Ok(RuntimeCapabilityOutcome::Completed(Box::new(
                RuntimeCapabilityCompleted {
                    capability_id: request.capability_id,
                    output: serde_json::json!({"ok": true}),
                    usage: ResourceUsage::default(),
                },
            )))
        }

        async fn resume_capability(
            &self,
            _request: RuntimeCapabilityResumeRequest,
        ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
            unreachable!("recording host runtime should not resume")
        }

        async fn visible_capabilities(
            &self,
            _request: ironclaw_host_runtime::VisibleCapabilityRequest,
        ) -> Result<VisibleCapabilitySurface, HostRuntimeError> {
            Ok(VisibleCapabilitySurface {
                version: CapabilitySurfaceVersion::new("surface-v1").expect("valid version"),
                capabilities: self.capabilities.clone(),
            })
        }

        async fn cancel_work(
            &self,
            _request: CancelRuntimeWorkRequest,
        ) -> Result<CancelRuntimeWorkOutcome, HostRuntimeError> {
            unreachable!("recording host runtime should not cancel work")
        }

        async fn runtime_status(
            &self,
            _request: RuntimeStatusRequest,
        ) -> Result<HostRuntimeStatus, HostRuntimeError> {
            unreachable!("recording host runtime should not report status")
        }

        async fn health(&self) -> Result<HostRuntimeHealth, HostRuntimeError> {
            unreachable!("recording host runtime should not report health")
        }
    }

    struct StaticInputResolver;

    #[async_trait]
    impl LoopCapabilityInputResolver for StaticInputResolver {
        async fn resolve_capability_input(
            &self,
            _run_context: &LoopRunContext,
            _input_ref: &CapabilityInputRef,
        ) -> Result<serde_json::Value, AgentLoopHostError> {
            Ok(serde_json::json!({"ok": true}))
        }
    }

    struct StaticResultWriter;

    #[async_trait]
    impl LoopCapabilityResultWriter for StaticResultWriter {
        async fn write_capability_result(
            &self,
            _run_context: &LoopRunContext,
            _capability_id: &CapabilityId,
            _output: serde_json::Value,
        ) -> Result<LoopResultRef, AgentLoopHostError> {
            LoopResultRef::new("result:mount-test").map_err(|_| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Internal,
                    "result ref could not be built",
                )
            })
        }
    }

    struct NoopHostRuntime;

    #[async_trait]
    impl HostRuntime for NoopHostRuntime {
        async fn invoke_capability(
            &self,
            _request: RuntimeCapabilityRequest,
        ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
            unreachable!("noop host runtime should not be called")
        }

        async fn resume_capability(
            &self,
            _request: RuntimeCapabilityResumeRequest,
        ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
            unreachable!("noop host runtime should not be called")
        }

        async fn visible_capabilities(
            &self,
            _request: ironclaw_host_runtime::VisibleCapabilityRequest,
        ) -> Result<VisibleCapabilitySurface, HostRuntimeError> {
            unreachable!("noop host runtime should not be called")
        }

        async fn cancel_work(
            &self,
            _request: CancelRuntimeWorkRequest,
        ) -> Result<CancelRuntimeWorkOutcome, HostRuntimeError> {
            unreachable!("noop host runtime should not be called")
        }

        async fn runtime_status(
            &self,
            _request: RuntimeStatusRequest,
        ) -> Result<HostRuntimeStatus, HostRuntimeError> {
            unreachable!("noop host runtime should not be called")
        }

        async fn health(&self) -> Result<HostRuntimeHealth, HostRuntimeError> {
            unreachable!("noop host runtime should not be called")
        }
    }

    struct NoopCapabilityIo;

    #[async_trait]
    impl LoopCapabilityInputResolver for NoopCapabilityIo {
        async fn resolve_capability_input(
            &self,
            _run_context: &LoopRunContext,
            _input_ref: &CapabilityInputRef,
        ) -> Result<serde_json::Value, AgentLoopHostError> {
            unreachable!("noop capability io should not be called")
        }
    }

    #[async_trait]
    impl LoopCapabilityResultWriter for NoopCapabilityIo {
        async fn write_capability_result(
            &self,
            _run_context: &LoopRunContext,
            _capability_id: &CapabilityId,
            _output: serde_json::Value,
        ) -> Result<LoopResultRef, AgentLoopHostError> {
            unreachable!("noop capability io should not be called")
        }
    }

    fn execution_context(thread: &str) -> ExecutionContext {
        let thread_id = ironclaw_host_api::ThreadId::new(thread).expect("valid thread id");
        let mut context = ExecutionContext::local_default(
            UserId::new("user-capability-port").expect("valid user"),
            ExtensionId::new("loop-driver").expect("valid extension"),
            RuntimeKind::FirstParty,
            TrustClass::System,
            CapabilitySet::default(),
            MountView::default(),
        )
        .expect("valid context");
        context.tenant_id = TenantId::new("tenant-capability-port").expect("valid tenant");
        context.agent_id = Some(AgentId::new("agent-capability-port").expect("valid agent"));
        context.project_id =
            Some(ProjectId::new("project-capability-port").expect("valid project"));
        context.thread_id = Some(thread_id.clone());
        context.resource_scope.tenant_id = context.tenant_id.clone();
        context.resource_scope.agent_id = context.agent_id.clone();
        context.resource_scope.project_id = context.project_id.clone();
        context.resource_scope.thread_id = Some(thread_id);
        context
    }

    async fn loop_run_context(context: &ExecutionContext) -> LoopRunContext {
        let resolved = InMemoryRunProfileResolver::default()
            .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
            .await
            .expect("profile resolves");
        LoopRunContext::new(
            TurnScope::new(
                context.tenant_id.clone(),
                context.agent_id.clone(),
                context.project_id.clone(),
                context.thread_id.clone().expect("thread id"),
            ),
            TurnId::new(),
            TurnRunId::new(),
            resolved,
        )
    }
}
