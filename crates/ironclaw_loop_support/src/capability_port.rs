use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use ironclaw_host_api::{
    CapabilityDisplayOutputPreview, CapabilityId, CapabilitySet, CorrelationId, EffectKind,
    ExecutionContext, ExtensionId, InvocationId, MountView, Principal, ResourceEstimate,
    RuntimeKind, sha256_digest_token,
};
use ironclaw_host_runtime::{
    CapabilityFailureDisposition, HostRuntime, HostRuntimeError, IdempotencyKey,
    RuntimeBlockedReason, RuntimeCapabilityAuthResumeRequest, RuntimeCapabilityFailure,
    RuntimeCapabilityOutcome, RuntimeCapabilityRequest, RuntimeCapabilityResumeRequest,
    RuntimeFailureKind,
};
use ironclaw_process_sandbox::{SandboxProcessPlan, ValidatedSandboxProcessPlan};
use ironclaw_turns::{
    CapabilityActivityId, LoopGateRef, LoopResultRef,
    run_profile::{
        AgentLoopHostError, AgentLoopHostErrorKind, CapabilityApprovalResume, CapabilityAuthResume,
        CapabilityBatchInvocation, CapabilityBatchOutcome, CapabilityDenied,
        CapabilityDeniedReasonKind, CapabilityDescriptorView, CapabilityFailure,
        CapabilityFailureKind, CapabilityInputRef, CapabilityInvocation, CapabilityOutcome,
        CapabilityResultMessage, CapabilityResumeToken, ConcurrencyHint, LoopCapabilityPort,
        LoopHostMilestone, LoopHostMilestoneKind, LoopHostMilestoneSink, LoopProcessRef,
        LoopRunContext, LoopSafeSummary, ProcessHandleSummary, ProviderToolCall,
        ProviderToolCallCapabilityIds, ProviderToolCallReplay, ProviderToolDefinition,
        VisibleCapabilityRequest, VisibleCapabilitySurface,
    },
};
use serde_json::Value;
use tokio::sync::Notify;

mod provider_input;
mod provider_validation;
mod surface_snapshot;

use self::provider_input::{
    normalize_provider_arguments, prepare_provider_arguments,
    prepare_provider_arguments_with_detail, schema_contains_external_ref,
};
use self::provider_validation::{
    PROVIDER_TOOL_NAME_MAX_BYTES, validate_provider_arguments, validate_provider_tool_call,
};
use self::surface_snapshot::{
    RuntimeSurfaceCapabilitySnapshot, SurfaceCapabilitySnapshot, SurfaceSnapshot,
    SyntheticSurfaceCapabilitySnapshot,
};

// arch-exempt: large_file, tracked in #3988; this PR keeps new synthetic surface
// snapshot logic in `capability_port/surface_snapshot.rs` while preserving the
// existing adapter boundary.
const PROVIDER_TOOL_NAME_DIGEST_BYTES: usize = 32;
const PROVIDER_TOOL_CALL_INPUT_REF_PREFIX: &str = "input:provider-tool-";
const MAX_IN_MEMORY_PROVIDER_TOOL_CALL_EFFECTIVE_CAPABILITY_IDS: usize = 128;

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
    /// Write the result of a completed capability invocation.
    ///
    /// Returns a tuple of `(LoopResultRef, u64)` where the `u64` is the
    /// serialized byte length of the staged result JSON, for downstream
    /// per-capability byte accounting (no PII; pure size).
    async fn write_capability_result(
        &self,
        write: CapabilityResultWrite<'_>,
    ) -> Result<(LoopResultRef, u64), AgentLoopHostError>;

    async fn update_capability_result(
        &self,
        _run_context: &LoopRunContext,
        _result_ref: &LoopResultRef,
        _output: serde_json::Value,
    ) -> Result<u64, AgentLoopHostError> {
        Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            "capability result updates are not supported by this writer",
        ))
    }

    async fn delete_capability_result(
        &self,
        _run_context: &LoopRunContext,
        _result_ref: &LoopResultRef,
    ) -> Result<(), AgentLoopHostError> {
        Ok(())
    }
}

pub struct CapabilityResultWrite<'a> {
    pub run_context: &'a LoopRunContext,
    pub input_ref: &'a CapabilityInputRef,
    pub invocation_id: InvocationId,
    pub capability_id: &'a CapabilityId,
    pub output: serde_json::Value,
    pub display_preview: Option<CapabilityDisplayOutputPreview>,
}

#[async_trait]
pub trait LoopCapabilityPortFactory: Send + Sync {
    async fn create_capability_port(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError>;
}

pub trait LoopCapabilityPortDecorator: Send + Sync {
    fn decorate(
        &self,
        run_context: &LoopRunContext,
        inner: Arc<dyn LoopCapabilityPort>,
    ) -> Arc<dyn LoopCapabilityPort>;
}

pub struct DecoratingLoopCapabilityPortFactory {
    inner: Arc<dyn LoopCapabilityPortFactory>,
    decorators: Vec<Arc<dyn LoopCapabilityPortDecorator>>,
}

impl DecoratingLoopCapabilityPortFactory {
    pub fn new(inner: Arc<dyn LoopCapabilityPortFactory>) -> Self {
        Self {
            inner,
            decorators: Vec::new(),
        }
    }

    pub fn with_decorator(mut self, decorator: Arc<dyn LoopCapabilityPortDecorator>) -> Self {
        self.decorators.push(decorator);
        self
    }
}

#[async_trait]
impl LoopCapabilityPortFactory for DecoratingLoopCapabilityPortFactory {
    async fn create_capability_port(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
        let mut port = self.inner.create_capability_port(run_context).await?;
        for decorator in &self.decorators {
            port = decorator.decorate(run_context, port);
        }
        Ok(port)
    }
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

#[async_trait]
impl LoopCapabilityPortFactory for HostRuntimeLoopCapabilityPortFactory {
    async fn create_capability_port(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
        Ok(self.for_run_context(run_context.clone()))
    }
}

struct PreparedProviderToolCall {
    surface_version: ironclaw_turns::run_profile::CapabilitySurfaceVersion,
    capability_id: CapabilityId,
    provider_turn_id: String,
    normalized_arguments: serde_json::Value,
    effective_capability_ids: Vec<CapabilityId>,
    capability_info_target_missing: bool,
}

const MAX_IN_MEMORY_DISPATCH_RECORDS: usize = 128;

#[derive(Clone)]
enum DispatchRecord {
    InFlight {
        notify: Arc<Notify>,
    },
    RuntimeCompleted {
        invocation_id: InvocationId,
        correlation_id: CorrelationId,
        requested_capability_id: CapabilityId,
        outcome: RuntimeCapabilityOutcome,
    },
    TerminalMilestonePending {
        result: Result<CapabilityOutcome, AgentLoopHostError>,
        milestone: LoopHostMilestoneKind,
    },
    LoopCompleted(Result<CapabilityOutcome, AgentLoopHostError>),
}

struct RuntimeOutcomeCompletion<'a> {
    input_ref: &'a CapabilityInputRef,
    input: Option<&'a Value>,
    estimate: Option<&'a ResourceEstimate>,
    invocation_id: InvocationId,
    correlation_id: CorrelationId,
    requested_capability_id: &'a CapabilityId,
    provider: ExtensionId,
    runtime: RuntimeKind,
    outcome: RuntimeCapabilityOutcome,
}

struct RuntimeOutcomeConversion<'a> {
    input_ref: &'a CapabilityInputRef,
    input: Option<&'a Value>,
    estimate: Option<&'a ResourceEstimate>,
    invocation_id: InvocationId,
    correlation_id: CorrelationId,
    requested_capability_id: &'a CapabilityId,
    outcome: RuntimeCapabilityOutcome,
}

impl<'a> RuntimeOutcomeCompletion<'a> {
    fn conversion(&self) -> RuntimeOutcomeConversion<'a> {
        RuntimeOutcomeConversion {
            input_ref: self.input_ref,
            input: self.input,
            estimate: self.estimate,
            invocation_id: self.invocation_id,
            correlation_id: self.correlation_id,
            requested_capability_id: self.requested_capability_id,
            outcome: self.outcome.clone(),
        }
    }
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
                invocation_id,
                correlation_id,
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
                    invocation_id,
                    correlation_id,
                    requested_capability_id,
                    outcome,
                })
            }
            Some(DispatchRecord::TerminalMilestonePending { result, milestone }) => {
                self.records.insert(
                    key_value,
                    DispatchRecord::InFlight {
                        notify: Arc::new(Notify::new()),
                    },
                );
                Ok(DispatchReservation::TerminalMilestonePending { result, milestone })
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
                | Some(DispatchRecord::TerminalMilestonePending { .. })
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
        invocation_id: InvocationId,
        correlation_id: CorrelationId,
        requested_capability_id: CapabilityId,
        outcome: RuntimeCapabilityOutcome,
    },
    TerminalMilestonePending {
        result: Result<CapabilityOutcome, AgentLoopHostError>,
        milestone: LoopHostMilestoneKind,
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

#[derive(Default)]
struct ProviderToolCallEffectiveCapabilityIdStore {
    records: HashMap<String, HashSet<CapabilityId>>,
    insertion_order: VecDeque<String>,
}

impl ProviderToolCallEffectiveCapabilityIdStore {
    fn record(
        &mut self,
        input_ref: &CapabilityInputRef,
        capability_ids: HashSet<CapabilityId>,
    ) -> Result<(), AgentLoopHostError> {
        let key = input_ref.as_str().to_string();
        if !self.records.contains_key(input_ref.as_str()) {
            self.evict_until_below_limit()?;
            self.insertion_order.push_back(key.clone());
        }
        self.records.insert(key, capability_ids);
        Ok(())
    }

    fn staged_effective_capability_ids_for(
        &self,
        input_ref: &CapabilityInputRef,
    ) -> HashSet<CapabilityId> {
        self.records
            .get(input_ref.as_str())
            .cloned()
            .unwrap_or_default()
    }

    fn evict_until_below_limit(&mut self) -> Result<(), AgentLoopHostError> {
        let mut scanned = 0;
        let scan_limit = self.insertion_order.len();
        while self.records.len() >= MAX_IN_MEMORY_PROVIDER_TOOL_CALL_EFFECTIVE_CAPABILITY_IDS
            && scanned < scan_limit
        {
            let Some(candidate) = self.insertion_order.pop_front() else {
                break;
            };
            scanned += 1;
            self.records.remove(&candidate);
        }
        if self.records.len() >= MAX_IN_MEMORY_PROVIDER_TOOL_CALL_EFFECTIVE_CAPABILITY_IDS {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "provider tool-call effective capability id store is unavailable",
            ));
        }
        Ok(())
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
    provider_tool_call_effective_capability_ids: Mutex<ProviderToolCallEffectiveCapabilityIdStore>,
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
            provider_tool_call_effective_capability_ids: Mutex::new(
                ProviderToolCallEffectiveCapabilityIdStore::default(),
            ),
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
        invocation_id: InvocationId,
        correlation_id: CorrelationId,
        requested_capability_id: CapabilityId,
        outcome: RuntimeCapabilityOutcome,
    ) -> Result<(), AgentLoopHostError> {
        let notify = lock_mut(&self.dispatch_records, "capability dispatch record store")?.record(
            key,
            DispatchRecord::RuntimeCompleted {
                invocation_id,
                correlation_id,
                requested_capability_id,
                outcome,
            },
        );
        if let Some(notify) = notify {
            notify.notify_waiters();
        }
        Ok(())
    }

    fn record_terminal_milestone_pending(
        &self,
        key: &IdempotencyKey,
        result: Result<CapabilityOutcome, AgentLoopHostError>,
        milestone: LoopHostMilestoneKind,
    ) -> Result<(), AgentLoopHostError> {
        let notify = lock_mut(&self.dispatch_records, "capability dispatch record store")?.record(
            key,
            DispatchRecord::TerminalMilestonePending { result, milestone },
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

    fn record_provider_tool_call_effective_capability_ids(
        &self,
        input_ref: &CapabilityInputRef,
        capability_ids: HashSet<CapabilityId>,
    ) -> Result<(), AgentLoopHostError> {
        lock_mut(
            &self.provider_tool_call_effective_capability_ids,
            "provider tool-call effective capability id store",
        )?
        .record(input_ref, capability_ids)?;
        Ok(())
    }

    fn staged_effective_capability_ids_for(
        &self,
        input_ref: &CapabilityInputRef,
    ) -> Result<HashSet<CapabilityId>, AgentLoopHostError> {
        Ok(lock_mut(
            &self.provider_tool_call_effective_capability_ids,
            "provider tool-call effective capability id store",
        )?
        .staged_effective_capability_ids_for(input_ref))
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
        completion: RuntimeOutcomeCompletion<'_>,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        let result = runtime_outcome_to_loop(
            &self.run_context,
            self.result_writer.as_ref(),
            completion.conversion(),
        )
        .await;
        if should_retry_result_write(&completion.outcome, &result) {
            self.record_runtime_completed(
                key,
                completion.invocation_id,
                completion.correlation_id,
                completion.requested_capability_id.clone(),
                completion.outcome,
            )?;
            return result;
        }
        if result.is_err() {
            self.record_loop_completed(key, result.clone())?;
            return result;
        }
        let terminal_milestone = match runtime_terminal_milestone(
            CapabilityActivityId::from_uuid(completion.invocation_id.as_uuid()),
            completion.provider,
            completion.runtime,
            &completion.outcome,
        ) {
            Ok(milestone) => milestone,
            Err(error) => {
                let result = Err(error);
                self.record_loop_completed(key, result.clone())?;
                return result;
            }
        };
        self.complete_terminal_milestone(key, result, terminal_milestone)
            .await
    }

    async fn complete_terminal_milestone(
        &self,
        key: &IdempotencyKey,
        result: Result<CapabilityOutcome, AgentLoopHostError>,
        terminal_milestone: Option<LoopHostMilestoneKind>,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        if let Some(milestone) = terminal_milestone
            && let Err(error) = self.emit_capability_milestone(milestone.clone()).await
        {
            self.record_terminal_milestone_pending(key, result.clone(), milestone)?;
            return Err(error);
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

    async fn emit_capability_milestone(
        &self,
        kind: LoopHostMilestoneKind,
    ) -> Result<(), AgentLoopHostError> {
        self.milestone_sink
            .publish_loop_milestone(LoopHostMilestone {
                scope: self.run_context.scope.clone(),
                actor: self.run_context.actor.clone(),
                turn_id: self.run_context.turn_id,
                run_id: self.run_context.run_id,
                loop_driver_id: self.run_context.loop_driver_id.clone(),
                kind,
            })
            .await
    }

    async fn invoke_synthetic_capability(
        &self,
        request: CapabilityInvocation,
        capability: SyntheticSurfaceCapabilitySnapshot,
        snapshot: SurfaceSnapshot,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        let input = self
            .input_resolver
            .resolve_capability_input(&self.run_context, &request.input_ref)
            .await?;
        let effective_capability_ids =
            self.staged_effective_capability_ids_for(&request.input_ref)?;
        let output = match capability.output(&input, |requested| {
            let capability = snapshot.capability_info(requested)?;
            if !effective_capability_ids.contains(capability.capability_id) {
                return None;
            }
            Some(capability)
        }) {
            Ok(output) => output,
            Err(error) if error.kind == AgentLoopHostErrorKind::InvalidInvocation => {
                // Synthetic capability InvalidInvocation errors are model-side input failures
                // such as bad arguments or an unknown capability_info target. Keep those
                // model-visible so the driver can retry instead of terminalizing the host.
                // INVARIANT: synthetic capabilities must not use InvalidInvocation for
                // internal or host-fatal conditions.
                return Ok(CapabilityOutcome::Failed(CapabilityFailure {
                    error_kind: CapabilityFailureKind::InvalidInput,
                    safe_summary: error.safe_summary,
                    detail: None,
                }));
            }
            Err(error) => return Err(error),
        };
        let (result_ref, byte_len) = self
            .result_writer
            .write_capability_result(CapabilityResultWrite {
                run_context: &self.run_context,
                input_ref: &request.input_ref,
                invocation_id: InvocationId::new(),
                capability_id: &request.capability_id,
                output,
                display_preview: None,
            })
            .await?;
        Ok(CapabilityOutcome::Completed(CapabilityResultMessage {
            result_ref,
            safe_summary: "capability info returned".to_string(),
            progress: ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
            terminate_hint: false,
            byte_len,
        }))
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
        let (capability_id, capability) = snapshot.provider_capability(&tool_call.name)?;
        let prepared =
            capability.prepare_provider_tool_call(capability_id, &snapshot, tool_call)?;
        Ok(PreparedProviderToolCall {
            surface_version: loop_surface_version(&version)?,
            capability_id: prepared.capability_id,
            provider_turn_id,
            normalized_arguments: prepared.normalized_arguments,
            effective_capability_ids: prepared.effective_capability_ids,
            capability_info_target_missing: prepared.capability_info_target_missing,
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
        let mut definitions = Vec::new();
        for (capability_id, capability) in &snapshot.capabilities {
            if let Some(definition) = capability.tool_definition(capability_id)? {
                definitions.push(definition);
            }
        }
        definitions.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(definitions)
    }

    fn provider_tool_call_capability_ids(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<ProviderToolCallCapabilityIds, AgentLoopHostError> {
        let prepared = self.prepare_provider_tool_call(tool_call)?;
        if prepared.capability_id.as_str() == crate::capability_info::CAPABILITY_ID
            && prepared.capability_info_target_missing
        {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "capability_info target is not on the visible surface",
            ));
        }
        Ok(ProviderToolCallCapabilityIds {
            provider_capability_id: prepared.capability_id,
            effective_capability_ids: prepared.effective_capability_ids,
        })
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
        if prepared.capability_id.as_str() == crate::capability_info::CAPABILITY_ID {
            self.record_provider_tool_call_effective_capability_ids(
                &input_ref,
                prepared.effective_capability_ids.iter().cloned().collect(),
            )?;
        }
        Ok(ironclaw_turns::run_profile::CapabilityCallCandidate {
            surface_version: prepared.surface_version,
            capability_id: prepared.capability_id,
            input_ref,
            effective_capability_ids: prepared.effective_capability_ids,
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
        let mut snapshot = SurfaceSnapshot::with_synthetic_capabilities()?;
        let mut descriptors = runtime_surface
            .capabilities
            .into_iter()
            .map(|capability| {
                let capability_id = capability.descriptor.id.clone();
                if snapshot.capabilities.contains_key(&capability_id) {
                    return Err(AgentLoopHostError::new(
                        AgentLoopHostErrorKind::InvalidInvocation,
                        "host runtime capability id is reserved for a synthetic loop capability",
                    ));
                }
                let provider_tool_name =
                    provider_tool_name(&capability.descriptor.id, &snapshot.provider_names);
                snapshot
                    .provider_names
                    .insert(provider_tool_name.clone(), capability_id.clone());
                snapshot.capabilities.insert(
                    capability_id.clone(),
                    SurfaceCapabilitySnapshot::Runtime(Box::new(
                        RuntimeSurfaceCapabilitySnapshot {
                            provider: capability.descriptor.provider.clone(),
                            runtime: capability.descriptor.runtime,
                            estimate: capability.estimated_resources.clone(),
                            safe_description: capability.descriptor.description.clone(),
                            parameters_schema: capability.descriptor.parameters_schema.clone(),
                            effects: capability.descriptor.effects.clone(),
                            provider_tool_name,
                        },
                    )),
                );
                Ok(CapabilityDescriptorView {
                    capability_id,
                    provider: Some(capability.descriptor.provider),
                    runtime: capability.descriptor.runtime,
                    safe_name: capability.descriptor.id.as_str().to_string(),
                    safe_description: capability.descriptor.description,
                    concurrency_hint: concurrency_hint_from_effects(&capability.descriptor.effects),
                    parameters_schema: capability.descriptor.parameters_schema,
                })
            })
            .collect::<Result<Vec<_>, AgentLoopHostError>>()?;
        descriptors.extend(snapshot.synthetic_descriptor_views()?);

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
        let effective_input_ref = request
            .approval_resume
            .as_ref()
            .map(|resume| &resume.input_ref)
            .unwrap_or(&request.input_ref);
        let snapshot = self.snapshot_for(&request.surface_version)?;
        let Some(capability) = snapshot.capabilities.get(&request.capability_id).cloned() else {
            return Ok(CapabilityOutcome::Denied(CapabilityDenied {
                reason_kind: capability_denied_reason_kind("outside_visible_surface")?,
                safe_summary: "capability was not visible on the cited surface".to_string(),
            }));
        };
        let idempotency_key =
            invocation_idempotency_key(&self.run_context, &request, effective_input_ref)?;
        loop {
            match self.reserve_dispatch(&idempotency_key)? {
                DispatchReservation::Reserved => break,
                DispatchReservation::Wait(notify) => {
                    self.wait_for_dispatch_completion(&idempotency_key, notify)
                        .await?;
                }
                DispatchReservation::RuntimeCompleted {
                    invocation_id,
                    correlation_id,
                    requested_capability_id,
                    outcome,
                } => {
                    if let SurfaceCapabilitySnapshot::Runtime(capability) = &capability {
                        return self
                            .finish_runtime_outcome(
                                &idempotency_key,
                                RuntimeOutcomeCompletion {
                                    input_ref: effective_input_ref,
                                    input: None,
                                    estimate: None,
                                    invocation_id,
                                    correlation_id,
                                    requested_capability_id: &requested_capability_id,
                                    provider: capability.provider.clone(),
                                    runtime: capability.runtime,
                                    outcome,
                                },
                            )
                            .await;
                    }
                    let result = runtime_outcome_to_loop(
                        &self.run_context,
                        self.result_writer.as_ref(),
                        RuntimeOutcomeConversion {
                            input_ref: effective_input_ref,
                            input: None,
                            estimate: None,
                            invocation_id,
                            correlation_id,
                            requested_capability_id: &requested_capability_id,
                            outcome,
                        },
                    )
                    .await;
                    self.record_loop_completed(&idempotency_key, result.clone())?;
                    return result;
                }
                DispatchReservation::TerminalMilestonePending { result, milestone } => {
                    return self
                        .complete_terminal_milestone(&idempotency_key, result, Some(milestone))
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

        let capability = match capability {
            SurfaceCapabilitySnapshot::Runtime(capability) => capability,
            SurfaceCapabilitySnapshot::Synthetic(capability) => {
                let result = self
                    .invoke_synthetic_capability(request, capability, snapshot)
                    .await;
                if result.is_ok() {
                    guard.commit();
                    self.record_loop_completed(&idempotency_key, result.clone())?;
                }
                return result;
            }
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
        let (input, estimate) = if let Some(resume) = request.approval_resume.as_ref() {
            (resume.input.clone(), resume.estimate.clone())
        } else {
            let input = self
                .input_resolver
                .resolve_capability_input(&self.run_context, effective_input_ref)
                .await?;
            let input = match prepare_provider_arguments_with_detail(
                &input,
                &capability.parameters_schema,
                "capability input",
            ) {
                Ok(input) => input,
                Err(error)
                    if error.error.kind == AgentLoopHostErrorKind::InvalidInvocation
                        && is_provider_tool_call_input_ref(effective_input_ref) =>
                {
                    let result = Ok(CapabilityOutcome::Failed(CapabilityFailure {
                        error_kind: CapabilityFailureKind::InvalidInput,
                        safe_summary: error.error.safe_summary,
                        detail: error.detail,
                    }));
                    guard.commit();
                    self.record_loop_completed(&idempotency_key, result.clone())?;
                    return result;
                }
                Err(error) => return Err(error.error),
            };
            (
                host_runtime_input_for_capability(&request.capability_id, input)?,
                capability.estimate.clone(),
            )
        };
        let mut invocation_context = invocation_context_from_visible(
            &self.visible_request.context,
            &self.run_context,
            &request.capability_id,
            &capability,
            trust_decision.effective_trust.class(),
            &trust_decision.authority_ceiling.allowed_effects,
            self.execution_mounts_for(&request.capability_id),
        )?;
        // Normalize the two mutually-exclusive resume fields into a single
        // local value BEFORE touching `invocation_context`, so an illegal
        // both-set invocation is rejected before any state mutation occurs.
        enum ResolvedResumeMode<'a> {
            Approval(&'a CapabilityApprovalResume),
            Auth(&'a CapabilityAuthResume),
            None,
        }
        let resume_mode = match (
            request.approval_resume.as_ref(),
            request.auth_resume.as_ref(),
        ) {
            (Some(_), Some(_)) => {
                // Both resume modes set simultaneously is an illegal invocation:
                // approval_resume and auth_resume are mutually exclusive paths.
                // Fail closed — do not dispatch, and do not mutate context.
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::InvalidInvocation,
                    "capability invocation has both approval_resume and auth_resume set; \
                     these resume modes are mutually exclusive",
                ));
            }
            (Some(resume), _) => ResolvedResumeMode::Approval(resume),
            (_, Some(auth_resume)) => ResolvedResumeMode::Auth(auth_resume),
            (Option::None, Option::None) => ResolvedResumeMode::None,
        };
        match &resume_mode {
            ResolvedResumeMode::Approval(resume) => {
                let resume_invocation_id = invocation_id_from_resume_token(&resume.resume_token)?;
                invocation_context.invocation_id = resume_invocation_id;
                invocation_context.correlation_id = resume.correlation_id;
                invocation_context.resource_scope.invocation_id = resume_invocation_id;
                invocation_context.validate().map_err(|_| {
                    AgentLoopHostError::new(
                        AgentLoopHostErrorKind::InvalidInvocation,
                        "capability approval resume context is invalid",
                    )
                })?;
            }
            ResolvedResumeMode::Auth(auth_resume) => {
                // Reuse original invocation identifier so the fingerprinted
                // approval lease (scoped to that identifier) can still be matched
                // and claimed.
                let resume_invocation_id =
                    invocation_id_from_resume_token(&auth_resume.resume_token)?;
                invocation_context.invocation_id = resume_invocation_id;
                invocation_context.resource_scope.invocation_id = resume_invocation_id;
                // Restore original correlation identifier when a prior approval is
                // present so the same trace-correlation identifier flows through
                // the full capability lifecycle (mirrors the approval-resume path).
                if let Some(pa) = auth_resume.prior_approval.as_ref() {
                    invocation_context.correlation_id = pa.correlation_id;
                }
                invocation_context.validate().map_err(|_| {
                    AgentLoopHostError::new(
                        AgentLoopHostErrorKind::InvalidInvocation,
                        "capability auth resume context is invalid",
                    )
                })?;
            }
            ResolvedResumeMode::None => {}
        }
        let invocation_id = invocation_context.invocation_id;
        let correlation_id = invocation_context.correlation_id;
        let requested_capability_id = request.capability_id.clone();
        let provider = capability.provider.clone();
        let runtime = capability.runtime;
        let capability_activity_id = CapabilityActivityId::from_uuid(invocation_id.as_uuid());
        self.emit_capability_milestone(LoopHostMilestoneKind::CapabilityInvoked {
            activity_id: capability_activity_id,
            capability_id: request.capability_id.clone(),
        })
        .await?;
        let outcome = match resume_mode {
            ResolvedResumeMode::Approval(resume) => {
                let runtime_request = RuntimeCapabilityResumeRequest::new(
                    invocation_context,
                    resume.approval_request_id,
                    request.capability_id,
                    estimate.clone(),
                    input.clone(),
                    trust_decision,
                )
                .with_idempotency_key(idempotency_key.clone());
                dispatch_runtime_capability_resume(self.runtime.as_ref(), runtime_request).await
            }
            ResolvedResumeMode::Auth(auth_resume) => {
                let prior_approval_id = auth_resume
                    .prior_approval
                    .as_ref()
                    .map(|pa| pa.approval_request_id);
                tracing::debug!(
                    invocation_id = %invocation_id,
                    auth_resume = true,
                    approval_request_id = prior_approval_id.map(|id| id.to_string()).as_deref().unwrap_or("none"),
                    "capability auth-resume re-dispatch with preserved invocation identity"
                );
                let runtime_request = RuntimeCapabilityAuthResumeRequest::new(
                    invocation_context,
                    request.capability_id,
                    estimate.clone(),
                    input.clone(),
                    trust_decision,
                    prior_approval_id,
                )
                .with_idempotency_key(idempotency_key.clone());
                dispatch_runtime_capability_auth_resume(self.runtime.as_ref(), runtime_request)
                    .await
            }
            ResolvedResumeMode::None => {
                let runtime_request = RuntimeCapabilityRequest::new(
                    invocation_context,
                    request.capability_id,
                    estimate.clone(),
                    input.clone(),
                    trust_decision,
                )
                .with_idempotency_key(idempotency_key.clone());
                dispatch_runtime_capability(self.runtime.as_ref(), runtime_request).await
            }
        };
        let outcome = match outcome {
            Ok(outcome) => outcome,
            Err(HostRuntimeError::Unavailable { reason }) => {
                runtime_failed_outcome_for_host_runtime_unavailable(
                    requested_capability_id.clone(),
                    reason,
                )
            }
            Err(error) => {
                let host_error = host_runtime_error(error);
                let terminal_milestone = LoopHostMilestoneKind::CapabilityFailed {
                    activity_id: capability_activity_id,
                    capability_id: requested_capability_id.clone(),
                    provider: Some(provider),
                    runtime: Some(runtime),
                    reason_kind: capability_failure_kind(host_error.kind.as_str())?,
                };
                guard.commit();
                return self
                    .complete_terminal_milestone(
                        &idempotency_key,
                        Err(host_error),
                        Some(terminal_milestone),
                    )
                    .await;
            }
        };
        guard.commit();
        self.finish_runtime_outcome(
            &idempotency_key,
            RuntimeOutcomeCompletion {
                input_ref: effective_input_ref,
                input: Some(&input),
                estimate: Some(&estimate),
                invocation_id,
                correlation_id,
                requested_capability_id: &requested_capability_id,
                provider,
                runtime,
                outcome,
            },
        )
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

async fn dispatch_runtime_capability(
    runtime: &(dyn HostRuntime + Send + Sync),
    request: RuntimeCapabilityRequest,
) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
    if is_process_sandbox_capability(&request.capability_id) {
        runtime.spawn_capability(request).await
    } else {
        runtime.invoke_capability(request).await
    }
}

async fn dispatch_runtime_capability_resume(
    runtime: &(dyn HostRuntime + Send + Sync),
    request: RuntimeCapabilityResumeRequest,
) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
    if is_process_sandbox_capability(&request.capability_id) {
        runtime.resume_spawn_capability(request).await
    } else {
        runtime.resume_capability(request).await
    }
}

/// Auth-resume dispatch: always uses `auth_resume_capability` (no spawn
/// variant; sandbox spawns do not go through approval/auth gates).
async fn dispatch_runtime_capability_auth_resume(
    runtime: &(dyn HostRuntime + Send + Sync),
    request: RuntimeCapabilityAuthResumeRequest,
) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
    runtime.auth_resume_capability(request).await
}

fn host_runtime_input_for_capability(
    capability_id: &CapabilityId,
    input: serde_json::Value,
) -> Result<serde_json::Value, AgentLoopHostError> {
    if is_process_sandbox_capability(capability_id) {
        let plan = serde_json::from_value::<SandboxProcessPlan>(input).map_err(|_| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "process sandbox capability input must be a SandboxProcessPlan",
            )
        })?;
        let plan = ValidatedSandboxProcessPlan::new(plan).map_err(|_| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "process sandbox capability input failed SandboxProcessPlan validation",
            )
        })?;
        return serde_json::to_value(plan.into_plan()).map_err(|error| {
            let safe_summary = error.to_string();
            crate::raw_agent_loop_host_error(
                "capability_runtime_input",
                "serialize_process_sandbox_plan",
                AgentLoopHostErrorKind::Internal,
                safe_summary,
                error,
            )
        });
    }
    Ok(input)
}

fn is_process_sandbox_capability(capability_id: &CapabilityId) -> bool {
    capability_id.as_str() == ironclaw_process_sandbox::PROCESS_SANDBOX_CAPABILITY_ID
}

fn provider_schema_is_usable(schema: &serde_json::Value) -> bool {
    let Some(object) = schema.as_object() else {
        return false;
    };
    if schema_contains_external_ref(schema, 0) {
        return false;
    }
    if object
        .get("$ref")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|reference| reference.starts_with('#'))
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
    capability: &RuntimeSurfaceCapabilitySnapshot,
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
    input_ref: &CapabilityInputRef,
) -> Result<IdempotencyKey, AgentLoopHostError> {
    // Each mode must hash to a distinct key: a colliding key would replay the
    // prior mode's recorded outcome (e.g. an auth re-dispatch receiving the
    // original cached ApprovalRequired gate) instead of dispatching.
    let resume_scope = match (
        request.approval_resume.as_ref(),
        request.auth_resume.as_ref(),
    ) {
        (Some(resume), _) => format!(
            "resume:{}:{}",
            resume.approval_request_id, resume.resume_token
        ),
        (None, Some(auth_resume)) => format!(
            "auth-resume:{}:{}",
            auth_resume
                .prior_approval
                .as_ref()
                .map(|pa| pa.approval_request_id.to_string())
                .unwrap_or_else(|| "none".to_string()),
            auth_resume.resume_token
        ),
        (None, None) => "dispatch".to_string(),
    };
    let payload = format!(
        "loop-capability\nrun={}\nsurface={}\ncapability={}\ninput={}\nmode={}",
        run_context.run_id,
        request.surface_version.as_str(),
        request.capability_id.as_str(),
        input_ref.as_str(),
        resume_scope
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
        let safe_summary = error.to_string();
        crate::raw_agent_loop_host_error(
            "capability_provider_tool_call",
            "serialize_arguments",
            AgentLoopHostErrorKind::InvalidInvocation,
            safe_summary,
            error,
        )
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
    CapabilityInputRef::new(format!("{PROVIDER_TOOL_CALL_INPUT_REF_PREFIX}{digest}")).map_err(
        |_| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                "provider tool-call input ref could not be represented",
            )
        },
    )
}

fn is_provider_tool_call_input_ref(input_ref: &CapabilityInputRef) -> bool {
    input_ref
        .as_str()
        .starts_with(PROVIDER_TOOL_CALL_INPUT_REF_PREFIX)
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
    conversion: RuntimeOutcomeConversion<'_>,
) -> Result<CapabilityOutcome, AgentLoopHostError> {
    ensure_runtime_outcome_matches(conversion.requested_capability_id, &conversion.outcome)?;
    Ok(match conversion.outcome {
        RuntimeCapabilityOutcome::Completed(completed) => {
            let (result_ref, byte_len) = result_writer
                .write_capability_result(CapabilityResultWrite {
                    run_context,
                    input_ref: conversion.input_ref,
                    invocation_id: conversion.invocation_id,
                    capability_id: &completed.capability_id,
                    output: completed.output.clone(),
                    display_preview: completed.display_preview.clone(),
                })
                .await?;
            CapabilityOutcome::Completed(CapabilityResultMessage {
                result_ref,
                safe_summary: "capability completed".to_string(),
                progress: ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                terminate_hint: false,
                byte_len,
            })
        }
        RuntimeCapabilityOutcome::ApprovalRequired(gate) => {
            let input = conversion.input.ok_or_else(|| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Internal,
                    "approval resume replay input is unavailable",
                )
            })?;
            let estimate = conversion.estimate.ok_or_else(|| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Internal,
                    "approval resume replay estimate is unavailable",
                )
            })?;
            CapabilityOutcome::ApprovalRequired {
                gate_ref: loop_gate_ref("approval", gate.approval_request_id.to_string())?,
                safe_summary: blocked_summary(gate.reason).to_string(),
                approval_resume: Some(ironclaw_turns::run_profile::CapabilityApprovalResume {
                    approval_request_id: gate.approval_request_id,
                    resume_token: resume_token_from_invocation_id(conversion.invocation_id)?,
                    correlation_id: conversion.correlation_id,
                    input_ref: conversion.input_ref.clone(),
                    input: input.clone(),
                    estimate: estimate.clone(),
                }),
            }
        }
        RuntimeCapabilityOutcome::AuthRequired(gate) => CapabilityOutcome::AuthRequired {
            gate_ref: loop_gate_ref("auth", gate.gate_id.to_string())?,
            credential_requirements: gate.credential_requirements,
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
        RuntimeCapabilityOutcome::Failed(failure) => runtime_failure_to_loop(failure)?,
        RuntimeCapabilityOutcome::Unknown(unknown) => {
            CapabilityOutcome::Failed(CapabilityFailure {
                error_kind: capability_failure_kind(unknown.kind)?,
                safe_summary: runtime_safe_summary(
                    unknown.message,
                    "capability invocation returned an unknown outcome",
                ),
                detail: None,
            })
        }
    })
}

fn runtime_terminal_milestone(
    activity_id: CapabilityActivityId,
    provider: ExtensionId,
    runtime: RuntimeKind,
    outcome: &RuntimeCapabilityOutcome,
) -> Result<Option<LoopHostMilestoneKind>, AgentLoopHostError> {
    Ok(match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => {
            Some(LoopHostMilestoneKind::CapabilityCompleted {
                activity_id,
                capability_id: completed.capability_id.clone(),
                provider,
                runtime,
                output_bytes: completed.usage.output_bytes,
            })
        }
        RuntimeCapabilityOutcome::Failed(failure) => {
            Some(LoopHostMilestoneKind::CapabilityFailed {
                activity_id,
                capability_id: failure.capability_id.clone(),
                provider: Some(provider),
                runtime: Some(runtime),
                reason_kind: runtime_failure_kind_to_loop(failure.kind)?,
            })
        }
        RuntimeCapabilityOutcome::Unknown(unknown) => {
            Some(LoopHostMilestoneKind::CapabilityFailed {
                activity_id,
                capability_id: unknown.capability_id.clone(),
                provider: Some(provider),
                runtime: Some(runtime),
                reason_kind: capability_failure_kind(unknown.kind.clone())?,
            })
        }
        RuntimeCapabilityOutcome::ApprovalRequired(_)
        | RuntimeCapabilityOutcome::AuthRequired(_)
        | RuntimeCapabilityOutcome::ResourceBlocked(_)
        | RuntimeCapabilityOutcome::SpawnedProcess(_) => None,
    })
}

fn runtime_failure_to_loop(
    failure: RuntimeCapabilityFailure,
) -> Result<CapabilityOutcome, AgentLoopHostError> {
    match failure.disposition() {
        CapabilityFailureDisposition::ModelVisibleToolError => {
            runtime_model_visible_failure_to_loop(failure)
        }
        CapabilityFailureDisposition::RetrySameCall => {
            Ok(CapabilityOutcome::Failed(CapabilityFailure {
                error_kind: runtime_failure_kind_to_loop(failure.kind)?,
                safe_summary: runtime_failure_safe_summary(
                    &failure,
                    "capability invocation failed",
                ),
                detail: None,
            }))
        }
    }
}

fn runtime_model_visible_failure_to_loop(
    failure: RuntimeCapabilityFailure,
) -> Result<CapabilityOutcome, AgentLoopHostError> {
    if matches!(
        failure.kind,
        RuntimeFailureKind::Authorization | RuntimeFailureKind::PolicyDenied
    ) {
        return Ok(CapabilityOutcome::Denied(CapabilityDenied {
            reason_kind: capability_denied_reason_kind(failure.kind.as_str())?,
            safe_summary: runtime_failure_safe_summary(&failure, "capability authorization denied"),
        }));
    }

    Ok(CapabilityOutcome::Failed(CapabilityFailure {
        error_kind: model_visible_runtime_failure_kind_to_loop(failure.kind)?,
        safe_summary: runtime_failure_safe_summary(&failure, "capability invocation failed"),
        detail: None,
    }))
}

fn runtime_failure_kind_to_loop(
    kind: RuntimeFailureKind,
) -> Result<CapabilityFailureKind, AgentLoopHostError> {
    Ok(match kind {
        RuntimeFailureKind::Authorization => CapabilityFailureKind::Authorization,
        RuntimeFailureKind::Backend => CapabilityFailureKind::Backend,
        RuntimeFailureKind::Cancelled => CapabilityFailureKind::Cancelled,
        RuntimeFailureKind::Dispatcher => CapabilityFailureKind::Dispatcher,
        RuntimeFailureKind::Internal => CapabilityFailureKind::Internal,
        RuntimeFailureKind::InvalidInput => CapabilityFailureKind::InvalidInput,
        RuntimeFailureKind::InvalidOutput => CapabilityFailureKind::InvalidOutput,
        RuntimeFailureKind::MissingRuntime => CapabilityFailureKind::MissingRuntime,
        RuntimeFailureKind::Network => CapabilityFailureKind::Network,
        RuntimeFailureKind::OperationFailed => CapabilityFailureKind::OperationFailed,
        RuntimeFailureKind::OutputTooLarge => CapabilityFailureKind::OutputTooLarge,
        RuntimeFailureKind::PolicyDenied => CapabilityFailureKind::PolicyDenied,
        RuntimeFailureKind::Process => CapabilityFailureKind::Process,
        RuntimeFailureKind::Resource => CapabilityFailureKind::Resource,
        RuntimeFailureKind::Transient => CapabilityFailureKind::Transient,
        RuntimeFailureKind::Unavailable => CapabilityFailureKind::Unavailable,
        RuntimeFailureKind::Unknown => capability_failure_kind("unknown")?,
        _ => capability_failure_kind(kind.as_str())?,
    })
}

fn runtime_failed_outcome_for_host_runtime_unavailable(
    capability_id: CapabilityId,
    reason: String,
) -> RuntimeCapabilityOutcome {
    let host_error = host_runtime_error(HostRuntimeError::Unavailable { reason });
    RuntimeCapabilityOutcome::Failed(RuntimeCapabilityFailure::new(
        capability_id,
        RuntimeFailureKind::Unavailable,
        Some(host_error.safe_summary),
    ))
}

fn model_visible_runtime_failure_kind_to_loop(
    kind: RuntimeFailureKind,
) -> Result<CapabilityFailureKind, AgentLoopHostError> {
    runtime_failure_kind_to_loop(kind)
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

fn runtime_failure_safe_summary(
    failure: &RuntimeCapabilityFailure,
    fallback: &'static str,
) -> String {
    failure
        .safe_summary()
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

fn resume_token_from_invocation_id(
    invocation_id: InvocationId,
) -> Result<CapabilityResumeToken, AgentLoopHostError> {
    CapabilityResumeToken::new(invocation_id.to_string()).map_err(|reason| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::Internal,
            format!("capability resume token is invalid: {reason}"),
        )
    })
}

fn invocation_id_from_resume_token(
    resume_token: &CapabilityResumeToken,
) -> Result<InvocationId, AgentLoopHostError> {
    InvocationId::parse(resume_token.as_str()).map_err(|_| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            "capability approval resume token is invalid",
        )
    })
}

fn host_runtime_error(error: HostRuntimeError) -> AgentLoopHostError {
    match error {
        HostRuntimeError::InvalidRequest { reason } => crate::raw_agent_loop_host_error(
            "host_runtime_capability",
            "invoke",
            AgentLoopHostErrorKind::InvalidInvocation,
            runtime_safe_summary(
                Some(reason.clone()),
                "host runtime rejected capability request",
            ),
            reason,
        ),
        HostRuntimeError::Unavailable { reason } => crate::raw_agent_loop_host_error(
            "host_runtime_capability",
            "invoke",
            AgentLoopHostErrorKind::Unavailable,
            runtime_safe_summary(
                Some(reason.clone()),
                "host runtime capability service is unavailable",
            ),
            reason,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    mod runtime_lifecycle_tests;

    use std::{
        collections::VecDeque,
        sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use async_trait::async_trait;
    use ironclaw_host_api::{
        AgentId, CapabilityDescriptor, CapabilityGrant, CapabilityGrantId, GrantConstraints,
        MountAlias, MountGrant, MountPermissions, NetworkPolicy, PermissionMode, ProjectId,
        ResourceEstimate, ResourceUsage, RuntimeKind, TenantId, TrustClass, UserId, VirtualPath,
    };
    use ironclaw_host_runtime::{
        CancelRuntimeWorkOutcome, CancelRuntimeWorkRequest, CapabilitySurfaceVersion,
        HostRuntimeHealth, HostRuntimeStatus, RuntimeCapabilityCompleted, RuntimeCapabilityFailure,
        RuntimeCapabilityResumeRequest, RuntimeCapabilityUnknown, RuntimeStatusRequest,
        SurfaceKind, VisibleCapability, VisibleCapabilityAccess, VisibleCapabilitySurface,
    };
    use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};
    use ironclaw_turns::{
        InMemoryRunProfileResolver, LoopDriverId, RunProfileResolutionRequest, RunProfileResolver,
        TurnId, TurnRunId, TurnScope,
    };

    use crate::{capability_info, capability_surface_filter::CapabilitySurfaceVisibleFilter};

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

    #[tokio::test]
    async fn decorating_factory_with_no_decorators_delegates_to_inner() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let inner = Arc::new(DecoratorTestPort {
            label: "inner",
            log: Arc::clone(&log),
        });
        let factory = DecoratingLoopCapabilityPortFactory::new(Arc::new(DecoratorTestFactory {
            port: inner,
        }));

        let port = factory
            .create_capability_port(&loop_run_context(&execution_context("decorator-empty")).await)
            .await
            .expect("decorated port");

        let error = port
            .visible_capabilities(VisibleCapabilityRequest {})
            .await
            .expect_err("test inner port should fail");

        assert_eq!(error.kind, AgentLoopHostErrorKind::Unavailable);
        assert_eq!(&*log.lock().expect("log lock"), &["inner"]);
    }

    #[tokio::test]
    async fn decorating_factory_applies_decorators_in_declared_order() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let inner = Arc::new(DecoratorTestPort {
            label: "inner",
            log: Arc::clone(&log),
        });
        let factory = DecoratingLoopCapabilityPortFactory::new(Arc::new(DecoratorTestFactory {
            port: inner,
        }))
        .with_decorator(Arc::new(LoggingDecorator {
            label: "first",
            log: Arc::clone(&log),
        }))
        .with_decorator(Arc::new(LoggingDecorator {
            label: "second",
            log: Arc::clone(&log),
        }));

        let port = factory
            .create_capability_port(&loop_run_context(&execution_context("decorator-order")).await)
            .await
            .expect("decorated port");

        let error = port
            .visible_capabilities(VisibleCapabilityRequest {})
            .await
            .expect_err("test inner port should fail");

        assert_eq!(error.kind, AgentLoopHostErrorKind::Unavailable);
        assert_eq!(
            &*log.lock().expect("log lock"),
            &["second", "first", "inner"]
        );
    }

    #[tokio::test]
    async fn decorating_factory_propagates_inner_error() {
        let decorate_calls = Arc::new(AtomicUsize::new(0));
        let factory = DecoratingLoopCapabilityPortFactory::new(Arc::new(FailingDecoratorFactory {
            error: AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "inner factory failed",
            ),
        }))
        .with_decorator(Arc::new(NoopDecorator {
            decorate_calls: Arc::clone(&decorate_calls),
        }));

        let error = match factory
            .create_capability_port(&loop_run_context(&execution_context("decorator-error")).await)
            .await
        {
            Ok(_) => panic!("inner factory error should propagate"),
            Err(error) => error,
        };

        assert_eq!(error.kind, AgentLoopHostErrorKind::Unavailable);
        assert_eq!(error.safe_summary, "inner factory failed");
        assert_eq!(decorate_calls.load(Ordering::SeqCst), 0);
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
                RuntimeFailureKind::Internal,
                CapabilityFailureKind::Internal,
            ),
            (
                RuntimeFailureKind::InvalidInput,
                CapabilityFailureKind::InvalidInput,
            ),
            (
                RuntimeFailureKind::InvalidOutput,
                CapabilityFailureKind::InvalidOutput,
            ),
            (
                RuntimeFailureKind::MissingRuntime,
                CapabilityFailureKind::MissingRuntime,
            ),
            (RuntimeFailureKind::Network, CapabilityFailureKind::Network),
            (
                RuntimeFailureKind::OperationFailed,
                CapabilityFailureKind::OperationFailed,
            ),
            (
                RuntimeFailureKind::OutputTooLarge,
                CapabilityFailureKind::OutputTooLarge,
            ),
            (
                RuntimeFailureKind::PolicyDenied,
                CapabilityFailureKind::PolicyDenied,
            ),
            (RuntimeFailureKind::Process, CapabilityFailureKind::Process),
            (
                RuntimeFailureKind::Resource,
                CapabilityFailureKind::Resource,
            ),
            (
                RuntimeFailureKind::Transient,
                CapabilityFailureKind::Transient,
            ),
            (
                RuntimeFailureKind::Unavailable,
                CapabilityFailureKind::Unavailable,
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
    fn runtime_failure_to_loop_honors_model_visible_disposition() {
        let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
        let invalid_input = runtime_failure_to_loop(RuntimeCapabilityFailure::new(
            capability_id.clone(),
            RuntimeFailureKind::InvalidInput,
            None,
        ))
        .expect("convert invalid input without runtime detail");
        assert!(matches!(
            invalid_input,
            CapabilityOutcome::Failed(failure)
                if failure.error_kind == CapabilityFailureKind::InvalidInput
                    && failure.safe_summary == "capability invocation failed"
        ));

        let denied = runtime_failure_to_loop(RuntimeCapabilityFailure::new(
            capability_id.clone(),
            RuntimeFailureKind::PolicyDenied,
            Some("policy denied request".to_string()),
        ))
        .expect("convert policy denial");
        assert!(matches!(
            denied,
            CapabilityOutcome::Denied(denied)
                if denied.reason_kind.as_str() == "policy_denied"
                    && denied.safe_summary == "policy denied request"
        ));

        let operation_failed = runtime_failure_to_loop(RuntimeCapabilityFailure::new(
            capability_id.clone(),
            RuntimeFailureKind::OperationFailed,
            Some(
                "apply_patch failed for path workspace main.rs: old_string matched 0 times"
                    .to_string(),
            ),
        ))
        .expect("convert operation failure");
        assert!(matches!(
            operation_failed,
            CapabilityOutcome::Failed(failure)
                if failure.error_kind == CapabilityFailureKind::OperationFailed
                    && failure.safe_summary == "apply_patch failed for path workspace main.rs: old_string matched 0 times"
        ));

        let missing_runtime = runtime_failure_to_loop(RuntimeCapabilityFailure::new(
            capability_id,
            RuntimeFailureKind::MissingRuntime,
            Some("tool runtime is missing".to_string()),
        ))
        .expect("convert missing runtime");
        assert!(matches!(
            missing_runtime,
            CapabilityOutcome::Failed(failure)
                if failure.error_kind == CapabilityFailureKind::MissingRuntime
                    && failure.safe_summary == "tool runtime is missing"
        ));
    }

    #[test]
    fn runtime_failure_to_loop_routes_retryable_failures_to_retry_classes() {
        let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
        let retry = runtime_failure_to_loop(RuntimeCapabilityFailure::new(
            capability_id,
            RuntimeFailureKind::Transient,
            Some("temporary outage".to_string()),
        ))
        .expect("convert retryable failure");
        assert!(matches!(
            retry,
            CapabilityOutcome::Failed(failure)
                if failure.error_kind == CapabilityFailureKind::Transient
                    && failure.safe_summary == "temporary outage"
        ));
    }

    #[test]
    fn runtime_failure_to_loop_keeps_recoverable_failures_out_of_tool_error_path() {
        let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
        let invalid_output = runtime_failure_to_loop(RuntimeCapabilityFailure::new(
            capability_id.clone(),
            RuntimeFailureKind::InvalidOutput,
            Some("runtime returned malformed output".to_string()),
        ))
        .expect("convert invalid output");
        assert!(matches!(
            invalid_output,
            CapabilityOutcome::Failed(failure)
                if failure.error_kind == CapabilityFailureKind::InvalidOutput
                    && failure.safe_summary == "runtime returned malformed output"
        ));

        let cancelled = runtime_failure_to_loop(RuntimeCapabilityFailure::new(
            capability_id,
            RuntimeFailureKind::Cancelled,
            Some("capability cancelled".to_string()),
        ))
        .expect("convert cancelled failure");
        assert!(matches!(
            cancelled,
            CapabilityOutcome::Failed(failure)
                if failure.error_kind == CapabilityFailureKind::Cancelled
                    && failure.safe_summary == "capability cancelled"
        ));
    }

    #[test]
    fn provider_schema_accepts_zero_arg_object_tools() {
        assert!(provider_schema_is_usable(
            &serde_json::json!({"type":"object"})
        ));
        assert!(provider_schema_is_usable(
            &serde_json::json!({"type":"object","properties":{}})
        ));
        assert!(!provider_schema_is_usable(&serde_json::json!({
            "$ref": "schemas/builtin/write-file.input.v1.json"
        })));
        assert!(provider_schema_is_usable(&serde_json::json!({
            "$ref": "#/$defs/input",
            "$defs": {
                "input": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" }
                    }
                }
            }
        })));
        assert!(!provider_schema_is_usable(&serde_json::json!({
            "type": "object",
            "properties": {
                "payload": {
                    "$ref": "schemas/builtin/write-file.input.v1.json"
                }
            }
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
        provider_validation::validate_provider_tool_name(&name).expect("provider-safe name");
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

    /// Regression: schemas like `headers` in `builtin.http` declare
    /// `{ oneOf: [{type:object}, {type:array}] }` and have no top-level
    /// `type`. Without `oneOf` handling, the normalizer's type-matched
    /// branches never fire and a stringified array is forwarded raw to the
    /// tool, which then rejects it with `InputEncode`.
    #[test]
    fn provider_argument_normalization_coerces_stringified_array_into_oneof_variant() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "headers": {
                    "oneOf": [
                        { "type": "object", "additionalProperties": { "type": "string" } },
                        {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "name": { "type": "string" },
                                    "value": { "type": "string" }
                                },
                                "required": ["name", "value"]
                            }
                        }
                    ]
                }
            }
        });

        let normalized = normalize_provider_arguments(
            &serde_json::json!({
                "headers": "[{\"name\":\"User-Agent\",\"value\":\"IronClaw/1.0\"}]"
            }),
            &schema,
            "provider arguments",
        )
        .expect("oneOf array variant should accept stringified array");

        assert_eq!(
            normalized,
            serde_json::json!({
                "headers": [{ "name": "User-Agent", "value": "IronClaw/1.0" }]
            })
        );
    }

    #[test]
    fn provider_argument_normalization_coerces_stringified_object_into_oneof_variant() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "headers": {
                    "oneOf": [
                        { "type": "object", "additionalProperties": { "type": "string" } },
                        { "type": "array", "items": { "type": "object" } }
                    ]
                }
            }
        });

        let normalized = normalize_provider_arguments(
            &serde_json::json!({
                "headers": "{\"User-Agent\":\"IronClaw/1.0\"}"
            }),
            &schema,
            "provider arguments",
        )
        .expect("oneOf object variant should accept stringified object");

        assert_eq!(
            normalized,
            serde_json::json!({
                "headers": { "User-Agent": "IronClaw/1.0" }
            })
        );
    }

    #[test]
    fn provider_argument_normalization_passes_through_oneof_when_value_already_matches_variant() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "headers": {
                    "oneOf": [
                        { "type": "object", "additionalProperties": { "type": "string" } },
                        { "type": "array", "items": { "type": "object" } }
                    ]
                }
            }
        });

        let input = serde_json::json!({
            "headers": [{ "name": "X", "value": "y" }]
        });
        let normalized = normalize_provider_arguments(&input, &schema, "provider arguments")
            .expect("real array value should pass oneOf normalization unchanged");

        assert_eq!(normalized, input);
    }

    #[test]
    fn provider_argument_normalization_anyof_behaves_like_oneof() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "payload": {
                    "anyOf": [
                        { "type": "object" },
                        { "type": "array", "items": { "type": "string" } }
                    ]
                }
            }
        });

        let normalized = normalize_provider_arguments(
            &serde_json::json!({ "payload": "[\"a\",\"b\"]" }),
            &schema,
            "provider arguments",
        )
        .expect("anyOf array variant should accept stringified array");

        assert_eq!(normalized, serde_json::json!({ "payload": ["a", "b"] }));
    }

    #[test]
    fn provider_argument_preparation_validates_required_fields_before_dispatch() {
        let schema = serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "owner": { "type": "string" },
                "repo": { "type": "string" },
                "pr_number": { "type": "integer", "minimum": 1 }
            },
            "required": ["owner", "repo", "pr_number"]
        });

        let error = prepare_provider_arguments(
            &serde_json::json!({ "owner": "nearai", "repo": "ironclaw" }),
            &schema,
            "provider arguments",
        )
        .expect_err("missing required fields should fail before dispatch");

        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
        assert!(error.safe_summary.contains("schema validation"));
        assert!(
            ironclaw_turns::run_profile::LoopSafeSummary::new(error.safe_summary.clone()).is_ok()
        );
    }

    #[test]
    fn provider_argument_preparation_rejects_unresolved_ref_schema() {
        let schema = serde_json::json!({
            "$ref": "schemas/demo/echo.input.v1.json"
        });

        let error = prepare_provider_arguments(
            &serde_json::json!({ "message": "hello" }),
            &schema,
            "provider arguments",
        )
        .expect_err("unresolved ref schemas must fail closed");

        assert_eq!(error.kind, AgentLoopHostErrorKind::StaleSurface);
    }

    #[test]
    fn provider_argument_preparation_rejects_nested_unresolved_ref_schema() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "payload": {
                    "type": "object",
                    "properties": {
                        "tool_input": {
                            "$ref": "schemas/demo/echo.input.v1.json"
                        }
                    }
                }
            }
        });

        let error = prepare_provider_arguments(
            &serde_json::json!({
                "payload": {
                    "tool_input": {
                        "message": "hello"
                    }
                }
            }),
            &schema,
            "provider arguments",
        )
        .expect_err("nested unresolved refs must fail closed");

        assert_eq!(error.kind, AgentLoopHostErrorKind::StaleSurface);
    }

    #[test]
    fn provider_argument_preparation_allows_internal_ref_schema() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "payload": {
                    "$ref": "#/$defs/payload"
                }
            },
            "$defs": {
                "payload": {
                    "type": "object",
                    "properties": {
                        "message": { "type": "string" }
                    },
                    "required": ["message"],
                    "additionalProperties": false
                }
            }
        });

        let normalized = prepare_provider_arguments(
            &serde_json::json!({
                "payload": {
                    "message": "hello"
                }
            }),
            &schema,
            "provider arguments",
        )
        .expect("internal refs should be allowed");

        assert_eq!(
            normalized,
            serde_json::json!({
                "payload": {
                    "message": "hello"
                }
            })
        );
    }

    #[test]
    fn provider_argument_preparation_rejects_excessive_schema_ref_scan_depth() {
        fn wrap_unknown_keyword(inner_schema: serde_json::Value) -> serde_json::Value {
            serde_json::json!({
                "x-next": inner_schema
            })
        }

        let mut deep_annotation = serde_json::json!({ "type": "null" });
        for _ in 0..40 {
            deep_annotation = wrap_unknown_keyword(deep_annotation);
        }
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "message": { "type": "string" }
            },
            "required": ["message"],
            "x-adversarial-depth": deep_annotation
        });

        let error = prepare_provider_arguments(
            &serde_json::json!({ "message": "hello" }),
            &schema,
            "provider arguments",
        )
        .expect_err("excessively deep schema ref scans should fail closed");

        assert_eq!(error.kind, AgentLoopHostErrorKind::StaleSurface);
    }

    #[test]
    fn provider_argument_depth_limit_allows_exact_boundary() {
        fn wrap_object_property(
            name: String,
            inner_schema: serde_json::Value,
        ) -> serde_json::Value {
            let mut properties = serde_json::Map::new();
            properties.insert(name, inner_schema);
            let mut schema = serde_json::Map::new();
            schema.insert("type".to_string(), serde_json::json!("object"));
            schema.insert(
                "properties".to_string(),
                serde_json::Value::Object(properties),
            );
            serde_json::Value::Object(schema)
        }

        fn wrap_object_value(name: String, inner_value: serde_json::Value) -> serde_json::Value {
            let mut object = serde_json::Map::new();
            object.insert(name, inner_value);
            serde_json::Value::Object(object)
        }

        fn wrap_unknown_keyword(inner_schema: serde_json::Value) -> serde_json::Value {
            serde_json::json!({
                "x-next": inner_schema
            })
        }

        let mut schema = serde_json::json!({ "type": "integer" });
        let mut value = serde_json::json!("1");
        for depth in (0..provider_input::MAX_PROVIDER_NORMALIZATION_DEPTH).rev() {
            let property = format!("level_{depth}");
            schema = wrap_object_property(property.clone(), schema);
            value = wrap_object_value(property, value);
        }

        let normalized = normalize_provider_arguments(&value, &schema, "provider arguments")
            .expect("exact normalization depth boundary should pass");

        assert_eq!(normalized, {
            let mut expected = serde_json::json!(1);
            for depth in (0..provider_input::MAX_PROVIDER_NORMALIZATION_DEPTH).rev() {
                expected = wrap_object_value(format!("level_{depth}"), expected);
            }
            expected
        });

        let mut deep_annotation = serde_json::json!({ "type": "null" });
        for _ in 2..provider_input::MAX_PROVIDER_NORMALIZATION_DEPTH {
            deep_annotation = wrap_unknown_keyword(deep_annotation);
        }
        let ref_scan_schema = serde_json::json!({
            "type": "object",
            "properties": {
                "message": { "type": "string" }
            },
            "required": ["message"],
            "x-depth-boundary": deep_annotation
        });

        prepare_provider_arguments(
            &serde_json::json!({ "message": "hello" }),
            &ref_scan_schema,
            "provider arguments",
        )
        .expect("exact schema ref-scan depth boundary should pass");
    }

    #[test]
    fn provider_argument_normalization_rejects_excessive_schema_depth() {
        fn wrap_object_property(
            name: String,
            inner_schema: serde_json::Value,
        ) -> serde_json::Value {
            let mut properties = serde_json::Map::new();
            properties.insert(name, inner_schema);
            let mut schema = serde_json::Map::new();
            schema.insert("type".to_string(), serde_json::json!("object"));
            schema.insert(
                "properties".to_string(),
                serde_json::Value::Object(properties),
            );
            serde_json::Value::Object(schema)
        }

        fn wrap_object_value(name: String, inner_value: serde_json::Value) -> serde_json::Value {
            let mut object = serde_json::Map::new();
            object.insert(name, inner_value);
            serde_json::Value::Object(object)
        }

        let mut schema = serde_json::json!({ "type": "integer" });
        let mut value = serde_json::json!("1");
        for depth in (0..40).rev() {
            let property = format!("level_{depth}");
            schema = wrap_object_property(property.clone(), schema);
            value = wrap_object_value(property, value);
        }

        let error = normalize_provider_arguments(&value, &schema, "provider arguments")
            .expect_err("excessively deep schema normalization should fail closed");

        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    }

    #[test]
    fn provider_argument_normalization_rejects_excessive_array_items_schema_depth() {
        fn wrap_array_schema(inner_schema: serde_json::Value) -> serde_json::Value {
            serde_json::json!({
                "type": "array",
                "items": inner_schema
            })
        }

        fn wrap_array_value(inner_value: serde_json::Value) -> serde_json::Value {
            serde_json::Value::Array(vec![inner_value])
        }

        let mut schema = serde_json::json!({ "type": "integer" });
        let mut value = serde_json::json!("1");
        for _ in 0..40 {
            schema = wrap_array_schema(schema);
            value = wrap_array_value(value);
        }

        let error = normalize_provider_arguments(&value, &schema, "provider arguments")
            .expect_err("excessively deep array item normalization should fail closed");

        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    }

    #[test]
    fn provider_argument_preparation_rejects_unknown_fields_before_dispatch() {
        let schema = serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "owner": { "type": "string" },
                "repo": { "type": "string" },
                "pr_number": { "type": "integer" }
            },
            "required": ["owner", "repo", "pr_number"]
        });

        let error = prepare_provider_arguments(
            &serde_json::json!({
                "owner": "nearai",
                "repo": "ironclaw",
                "pr_number": 4286,
                "number": 4286
            }),
            &schema,
            "provider arguments",
        )
        .expect_err("additional properties should fail before dispatch");

        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
        assert!(error.safe_summary.contains("schema validation"));
        assert!(
            ironclaw_turns::run_profile::LoopSafeSummary::new(error.safe_summary.clone()).is_ok()
        );
    }

    #[test]
    fn provider_argument_preparation_validates_composed_object_schema_after_normalization() {
        let schema = serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "query": { "type": "string" },
                "page": { "type": "integer", "minimum": 1 },
                "owner": { "type": "string" },
                "repo": { "type": "string" }
            },
            "allOf": [
                {
                    "if": { "required": ["owner"] },
                    "then": { "required": ["repo"] }
                }
            ],
            "anyOf": [
                { "required": ["query"] },
                { "required": ["owner", "repo"] }
            ]
        });

        let normalized = prepare_provider_arguments(
            &serde_json::json!({ "query": "repo:nearai/ironclaw", "page": "2" }),
            &schema,
            "provider arguments",
        )
        .expect("top-level anyOf object schema should still normalize properties");
        assert_eq!(
            normalized,
            serde_json::json!({ "query": "repo:nearai/ironclaw", "page": 2 })
        );

        let error = prepare_provider_arguments(
            &serde_json::json!({ "owner": "nearai" }),
            &schema,
            "provider arguments",
        )
        .expect_err("composed schema constraints should fail before dispatch");
        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    }

    #[test]
    fn provider_argument_schema_failure_sanitizes_sensitive_path_markers() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "secret_api_key": { "type": "integer" }
            }
        });

        let error = prepare_provider_arguments(
            &serde_json::json!({ "secret_api_key": "not an integer" }),
            &schema,
            "provider arguments",
        )
        .expect_err("schema failure should remain a model-visible invocation error");

        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
        assert!(!error.safe_summary.contains("secret"));
        assert!(!error.safe_summary.contains("api_key"));
    }

    /// Regression for Gemini review comment: a plain string that starts with
    /// `{` or `[` but is not valid JSON must not cause an `InvalidInvocation`
    /// error when a `string` variant is available. The coercion attempt should
    /// fail gracefully and fall through to the string branch.
    #[test]
    fn provider_argument_normalization_oneof_string_variant_accepts_non_json_string() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "oneOf": [
                        { "type": "object" },
                        { "type": "string" }
                    ]
                }
            }
        });

        // Looks like JSON but is malformed — must not error; string variant matches.
        let normalized = normalize_provider_arguments(
            &serde_json::json!({ "query": "{not valid json" }),
            &schema,
            "provider arguments",
        )
        .expect("malformed JSON-like string should fall through to the string variant");

        assert_eq!(
            normalized,
            serde_json::json!({ "query": "{not valid json" })
        );
    }

    /// Regression for Gemini review comment: JSON Schema treats every integer
    /// as a valid number, so an integer-shaped value must match a `number`
    /// variant in a `oneOf`/`anyOf` schema.
    #[test]
    fn provider_argument_normalization_oneof_integer_matches_number_variant() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "value": {
                    "oneOf": [
                        { "type": "string" },
                        { "type": "number" }
                    ]
                }
            }
        });

        let normalized = normalize_provider_arguments(
            &serde_json::json!({ "value": 42 }),
            &schema,
            "provider arguments",
        )
        .expect("integer value should match the number variant");

        assert_eq!(normalized, serde_json::json!({ "value": 42 }));
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
    async fn runtime_capability_invocation_emits_dispatch_lifecycle_milestones() {
        let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
        let provider_id = ExtensionId::new("demo").expect("valid provider id");
        let milestone_sink =
            Arc::new(ironclaw_turns::run_profile::InMemoryLoopHostMilestoneSink::default());
        let port = runtime_capability_port(
            &capability_id,
            &provider_id,
            Arc::new(RecordingHostRuntime::new(vec![visible_capability(
                capability_id.clone(),
                provider_id.clone(),
            )])),
            Arc::new(RecordingResultWriter::default()),
            milestone_sink.clone(),
            "thread-runtime-capability-milestones",
        )
        .await;

        let outcome = invoke_visible_runtime_capability(&port)
            .await
            .expect("capability invocation succeeds");

        assert!(matches!(outcome, CapabilityOutcome::Completed(_)));
        let milestones = milestone_sink.milestones();
        assert!(matches!(
            &milestones[0].kind,
            ironclaw_turns::run_profile::LoopHostMilestoneKind::CapabilityInvoked {
                capability_id: actual,
                ..
            } if actual == &capability_id
        ));
        assert!(matches!(
            &milestones[1].kind,
            ironclaw_turns::run_profile::LoopHostMilestoneKind::CapabilityCompleted {
                capability_id: actual,
                provider,
                runtime: RuntimeKind::FirstParty,
                output_bytes,
                ..
            } if actual == &capability_id && provider == &provider_id && *output_bytes == RECORDING_OUTPUT_BYTES
        ));
    }

    #[tokio::test]
    async fn runtime_capability_emits_completion_after_result_write_retry_succeeds() {
        let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
        let provider_id = ExtensionId::new("demo").expect("valid provider id");
        let milestone_sink =
            Arc::new(ironclaw_turns::run_profile::InMemoryLoopHostMilestoneSink::default());
        let result_writer = Arc::new(FailOnceResultWriter::default());
        let port = runtime_capability_port(
            &capability_id,
            &provider_id,
            Arc::new(RecordingHostRuntime::new(vec![visible_capability(
                capability_id.clone(),
                provider_id.clone(),
            )])),
            result_writer.clone(),
            milestone_sink.clone(),
            "thread-runtime-capability-milestone-retry",
        )
        .await;
        let invocation = visible_runtime_invocation(&port).await;

        let first_error = port
            .invoke_capability(invocation.clone())
            .await
            .expect_err("first result write fails");
        assert_eq!(
            first_error.kind,
            AgentLoopHostErrorKind::TranscriptWriteFailed
        );
        assert_eq!(milestone_sink.milestones().len(), 1);

        let outcome = port
            .invoke_capability(invocation)
            .await
            .expect("cached runtime outcome writes on retry");
        assert!(matches!(outcome, CapabilityOutcome::Completed(_)));
        assert_eq!(result_writer.attempts(), 2);
        let milestones = milestone_sink.milestones();
        assert_eq!(milestones.len(), 2);
        assert!(matches!(
            &milestones[1].kind,
            ironclaw_turns::run_profile::LoopHostMilestoneKind::CapabilityCompleted {
                capability_id: actual,
                provider,
                runtime: RuntimeKind::FirstParty,
                output_bytes,
                ..
            } if actual == &capability_id && provider == &provider_id && *output_bytes == RECORDING_OUTPUT_BYTES
        ));
    }

    #[tokio::test]
    async fn runtime_capability_terminal_milestone_failure_is_retryable_without_rewriting_result() {
        let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
        let provider_id = ExtensionId::new("demo").expect("valid provider id");
        let runtime = Arc::new(RecordingHostRuntime::new(vec![visible_capability(
            capability_id.clone(),
            provider_id.clone(),
        )]));
        let result_writer = Arc::new(RecordingResultWriter::default());
        let milestone_sink = Arc::new(FailOnceTerminalMilestoneSink::default());
        let port = runtime_capability_port(
            &capability_id,
            &provider_id,
            runtime.clone(),
            result_writer.clone(),
            milestone_sink.clone(),
            "thread-runtime-capability-milestone-fail-retry",
        )
        .await;
        let invocation = visible_runtime_invocation(&port).await;

        let first_error = port
            .invoke_capability(invocation.clone())
            .await
            .expect_err("terminal milestone publish fails first");
        assert_eq!(first_error.kind, AgentLoopHostErrorKind::Unavailable);
        assert_eq!(runtime.take_requests().len(), 1);
        assert_eq!(result_writer.records().len(), 1);

        let outcome = port
            .invoke_capability(invocation)
            .await
            .expect("pending terminal milestone publishes on retry");

        assert!(matches!(outcome, CapabilityOutcome::Completed(_)));
        assert_eq!(runtime.take_requests().len(), 1);
        assert_eq!(result_writer.records().len(), 1);
        let milestones = milestone_sink.milestones();
        assert_eq!(milestones.len(), 2);
        assert!(matches!(
            &milestones[1].kind,
            ironclaw_turns::run_profile::LoopHostMilestoneKind::CapabilityCompleted {
                capability_id: actual,
                provider,
                runtime: RuntimeKind::FirstParty,
                output_bytes,
                ..
            } if actual == &capability_id && provider == &provider_id && *output_bytes == RECORDING_OUTPUT_BYTES
        ));
    }

    #[tokio::test]
    async fn runtime_capability_failed_and_unknown_outcomes_emit_failure_milestones() {
        let cases = [
            (
                RuntimeCapabilityOutcome::Failed(RuntimeCapabilityFailure {
                    capability_id: CapabilityId::new("demo.echo").expect("valid capability id"),
                    kind: RuntimeFailureKind::InvalidInput,
                    message: Some("invalid input".to_string()),
                }),
                CapabilityFailureKind::InvalidInput,
            ),
            (
                RuntimeCapabilityOutcome::Unknown(RuntimeCapabilityUnknown {
                    capability_id: CapabilityId::new("demo.echo").expect("valid capability id"),
                    kind: "custom_failure".to_string(),
                    message: Some("custom failure".to_string()),
                }),
                capability_failure_kind("custom_failure").expect("valid custom failure kind"),
            ),
        ];

        for (outcome, expected_kind) in cases {
            let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
            let provider_id = ExtensionId::new("demo").expect("valid provider id");
            let milestone_sink =
                Arc::new(ironclaw_turns::run_profile::InMemoryLoopHostMilestoneSink::default());
            let port = runtime_capability_port(
                &capability_id,
                &provider_id,
                Arc::new(QueuedHostRuntime::new(
                    vec![visible_capability(
                        capability_id.clone(),
                        provider_id.clone(),
                    )],
                    vec![Ok(outcome)],
                )),
                Arc::new(RecordingResultWriter::default()),
                milestone_sink.clone(),
                "thread-runtime-capability-failure-milestone",
            )
            .await;

            let outcome = invoke_visible_runtime_capability(&port)
                .await
                .expect("runtime failure outcome maps to loop outcome");

            assert!(matches!(outcome, CapabilityOutcome::Failed(_)));
            let milestones = milestone_sink.milestones();
            assert_eq!(milestones.len(), 2);
            assert!(matches!(
                &milestones[1].kind,
                ironclaw_turns::run_profile::LoopHostMilestoneKind::CapabilityFailed {
                    capability_id: actual,
                    provider: Some(provider),
                    runtime: Some(RuntimeKind::FirstParty),
                    reason_kind,
                    ..
                } if actual == &capability_id && provider == &provider_id && reason_kind == &expected_kind
            ));
        }
    }

    #[tokio::test]
    async fn runtime_capability_unavailable_returns_failed_outcome_and_emits_failure_milestone() {
        let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
        let provider_id = ExtensionId::new("demo").expect("valid provider id");
        let milestone_sink =
            Arc::new(ironclaw_turns::run_profile::InMemoryLoopHostMilestoneSink::default());
        let port = runtime_capability_port(
            &capability_id,
            &provider_id,
            Arc::new(QueuedHostRuntime::new(
                vec![visible_capability(
                    capability_id.clone(),
                    provider_id.clone(),
                )],
                vec![Err(HostRuntimeError::unavailable("runtime unavailable"))],
            )),
            Arc::new(RecordingResultWriter::default()),
            milestone_sink.clone(),
            "thread-runtime-capability-unavailable-milestone",
        )
        .await;

        let outcome = invoke_visible_runtime_capability(&port)
            .await
            .expect("host runtime unavailability should become a capability failure");

        assert!(matches!(
            outcome,
            CapabilityOutcome::Failed(failure)
                if failure.error_kind == CapabilityFailureKind::Unavailable
        ));
        let milestones = milestone_sink.milestones();
        assert_eq!(milestones.len(), 2);
        assert!(matches!(
            &milestones[1].kind,
            ironclaw_turns::run_profile::LoopHostMilestoneKind::CapabilityFailed {
                capability_id: actual,
                provider: Some(provider),
                runtime: Some(RuntimeKind::FirstParty),
                reason_kind,
                ..
            } if actual == &capability_id
                && provider == &provider_id
                && reason_kind == &CapabilityFailureKind::Unavailable
        ));
    }

    #[tokio::test]
    async fn runtime_capability_invalid_request_preserves_host_error_and_emits_failure_milestone() {
        let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
        let provider_id = ExtensionId::new("demo").expect("valid provider id");
        let milestone_sink =
            Arc::new(ironclaw_turns::run_profile::InMemoryLoopHostMilestoneSink::default());
        let port = runtime_capability_port(
            &capability_id,
            &provider_id,
            Arc::new(QueuedHostRuntime::new(
                vec![visible_capability(
                    capability_id.clone(),
                    provider_id.clone(),
                )],
                vec![Err(HostRuntimeError::invalid_request("bad request"))],
            )),
            Arc::new(RecordingResultWriter::default()),
            milestone_sink.clone(),
            "thread-runtime-capability-invalid-request-milestone",
        )
        .await;

        let error = invoke_visible_runtime_capability(&port)
            .await
            .expect_err("host runtime invalid request should remain a host error");

        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
        let milestones = milestone_sink.milestones();
        assert_eq!(milestones.len(), 2);
        assert!(matches!(
            &milestones[1].kind,
            ironclaw_turns::run_profile::LoopHostMilestoneKind::CapabilityFailed {
                capability_id: actual,
                provider: Some(provider),
                runtime: Some(RuntimeKind::FirstParty),
                reason_kind,
                ..
            } if actual == &capability_id
                && provider == &provider_id
                && reason_kind.as_str() == AgentLoopHostErrorKind::InvalidInvocation.as_str()
        ));
    }

    #[tokio::test]
    async fn capability_info_is_advertised_and_returns_lazy_schema_on_request() {
        let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
        let provider_id = ExtensionId::new("demo").expect("valid provider id");
        let context = execution_context("thread-capability-info");
        let run_context = loop_run_context(&context).await;
        let runtime = Arc::new(RecordingHostRuntime::new(vec![visible_capability(
            capability_id.clone(),
            provider_id,
        )]));
        let result_writer = Arc::new(RecordingResultWriter::default());
        let port = Arc::new(
            HostRuntimeLoopCapabilityPortFactory::new(
                runtime.clone(),
                visible_request(context),
                dummy_input_resolver(),
                result_writer.clone(),
                dummy_milestone_sink(),
            )
            .port_for_run_context(run_context),
        );

        let surface = port
            .visible_capabilities(VisibleCapabilityRequest {})
            .await
            .expect("visible capabilities load");
        assert!(surface.descriptors.iter().any(|descriptor| {
            descriptor.capability_id.as_str() == capability_info::CAPABILITY_ID
        }));
        let visible_filter = CapabilitySurfaceVisibleFilter::new(
            port.clone(),
            surface
                .descriptors
                .iter()
                .map(|descriptor| descriptor.capability_id.clone()),
        );
        let filtered_tool_definitions = visible_filter
            .tool_definitions()
            .expect("filtered tool definitions");
        assert!(
            filtered_tool_definitions
                .iter()
                .any(|definition| definition.name == capability_info::TOOL_NAME),
            "capability_info must survive the ordinary model-visible capability filter"
        );
        let tool_definitions = port.tool_definitions().expect("tool definitions");
        assert!(
            tool_definitions
                .iter()
                .any(|definition| definition.name == capability_info::TOOL_NAME)
        );
        let capability_info_definition = tool_definitions
            .iter()
            .find(|definition| definition.name == capability_info::TOOL_NAME)
            .expect("capability_info definition is advertised");
        assert_eq!(
            capability_info_definition.parameters["required"],
            serde_json::json!(["name"])
        );
        assert!(
            tool_definitions
                .iter()
                .any(|definition| definition.capability_id == capability_id)
        );

        let mut call = provider_tool_call();
        call.name = capability_info::TOOL_NAME.to_string();
        call.arguments = serde_json::json!({
            "capability_id": capability_id.as_str(),
            "include_schema": true
        });
        let candidate = port
            .register_provider_tool_call(call)
            .await
            .expect("capability_info call should register");
        assert_eq!(
            candidate.capability_id.as_str(),
            capability_info::CAPABILITY_ID
        );

        let invocation = CapabilityInvocation {
            surface_version: surface.version,
            capability_id: candidate.capability_id,
            input_ref: candidate.input_ref,
            approval_resume: None,
            auth_resume: None,
        };
        let outcome = port
            .invoke_capability(invocation.clone())
            .await
            .expect("capability_info invocation succeeds");
        let replayed_outcome = port
            .invoke_capability(CapabilityInvocation {
                surface_version: invocation.surface_version,
                capability_id: invocation.capability_id,
                input_ref: invocation.input_ref,
                approval_resume: None,
                auth_resume: None,
            })
            .await
            .expect("capability_info invocation replays");

        assert!(matches!(outcome, CapabilityOutcome::Completed(_)));
        assert!(matches!(replayed_outcome, CapabilityOutcome::Completed(_)));
        let records = result_writer.records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].0.as_str(), capability_info::CAPABILITY_ID);
        assert_eq!(records[0].1["capability_id"], capability_id.as_str());
        assert_eq!(records[0].1["schema"], serde_json::json!({"type":"object"}));
        assert!(
            runtime.take_requests().is_empty(),
            "capability_info must be served by the loop port without dispatching to the host runtime"
        );
    }

    #[tokio::test]
    async fn capability_info_result_write_failure_is_retryable() {
        let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
        let provider_id = ExtensionId::new("demo").expect("valid provider id");
        let context = execution_context("thread-capability-info-retry-result-write");
        let run_context = loop_run_context(&context).await;
        let result_writer = Arc::new(FailOnceResultWriter::default());
        let port = HostRuntimeLoopCapabilityPortFactory::new(
            Arc::new(RecordingHostRuntime::new(vec![visible_capability(
                capability_id.clone(),
                provider_id,
            )])),
            visible_request(context),
            dummy_input_resolver(),
            result_writer.clone(),
            dummy_milestone_sink(),
        )
        .port_for_run_context(run_context);
        let surface = port
            .visible_capabilities(VisibleCapabilityRequest {})
            .await
            .expect("visible capabilities load");
        let mut call = provider_tool_call();
        call.name = capability_info::TOOL_NAME.to_string();
        call.arguments = serde_json::json!({ "name": capability_id.as_str() });
        let candidate = port
            .register_provider_tool_call(call)
            .await
            .expect("capability_info call should register");
        let invocation = CapabilityInvocation {
            surface_version: surface.version,
            capability_id: candidate.capability_id,
            input_ref: candidate.input_ref,
            approval_resume: None,
            auth_resume: None,
        };

        let error = port
            .invoke_capability(invocation.clone())
            .await
            .expect_err("first result write should fail");
        assert_eq!(error.kind, AgentLoopHostErrorKind::TranscriptWriteFailed);
        let retried_outcome = port
            .invoke_capability(invocation)
            .await
            .expect("second invocation should retry the write");

        assert!(matches!(retried_outcome, CapabilityOutcome::Completed(_)));
        assert_eq!(result_writer.attempts(), 2);
    }

    #[tokio::test]
    async fn capability_info_accepts_visible_provider_tool_name() {
        let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
        let provider_id = ExtensionId::new("demo").expect("valid provider id");
        let context = execution_context("thread-capability-info-provider-name");
        let run_context = loop_run_context(&context).await;
        let result_writer = Arc::new(RecordingResultWriter::default());
        let port = HostRuntimeLoopCapabilityPortFactory::new(
            Arc::new(RecordingHostRuntime::new(vec![visible_capability(
                capability_id.clone(),
                provider_id,
            )])),
            visible_request(context),
            dummy_input_resolver(),
            result_writer.clone(),
            dummy_milestone_sink(),
        )
        .port_for_run_context(run_context);
        let surface = port
            .visible_capabilities(VisibleCapabilityRequest {})
            .await
            .expect("visible capabilities load");
        let provider_tool_name = port
            .tool_definitions()
            .expect("tool definitions")
            .into_iter()
            .find(|definition| definition.capability_id == capability_id)
            .expect("runtime capability is advertised")
            .name;

        let mut call = provider_tool_call();
        call.name = capability_info::TOOL_NAME.to_string();
        call.arguments = serde_json::json!({
            "name": provider_tool_name,
            "detail": "summary"
        });
        let candidate = port
            .register_provider_tool_call(call)
            .await
            .expect("capability_info call should register by provider tool name");
        assert_eq!(
            candidate.effective_capability_ids,
            vec![
                CapabilityId::new(capability_info::CAPABILITY_ID).expect("synthetic id"),
                capability_id.clone(),
            ],
            "known target should include both capability_info and target ids"
        );
        port.invoke_capability(CapabilityInvocation {
            surface_version: surface.version,
            capability_id: candidate.capability_id,
            input_ref: candidate.input_ref,
            approval_resume: None,
            auth_resume: None,
        })
        .await
        .expect("capability_info invocation succeeds");

        let records = result_writer.records();
        assert_eq!(records[0].1["capability_id"], capability_id.as_str());
        assert_eq!(
            records[0].1["summary"]["notes"],
            serde_json::json!(["runtime: first_party", "effects: dispatch_capability"])
        );
    }

    #[tokio::test]
    async fn capability_info_reports_invalid_detail_arguments_as_model_visible_failure() {
        let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
        let provider_id = ExtensionId::new("demo").expect("valid provider id");
        let context = execution_context("thread-capability-info-invalid-detail");
        let run_context = loop_run_context(&context).await;
        let runtime = Arc::new(RecordingHostRuntime::new(vec![visible_capability(
            capability_id.clone(),
            provider_id,
        )]));
        let result_writer = Arc::new(RecordingResultWriter::default());
        let port = HostRuntimeLoopCapabilityPortFactory::new(
            runtime.clone(),
            visible_request(context),
            dummy_input_resolver(),
            result_writer.clone(),
            dummy_milestone_sink(),
        )
        .port_for_run_context(run_context);
        let surface = port
            .visible_capabilities(VisibleCapabilityRequest {})
            .await
            .expect("visible capabilities load");

        for (index, (arguments, expected_summary)) in [
            (
                serde_json::json!({ "name": capability_id.as_str(), "include_schema": 1 }),
                "capability_info include_schema must be boolean",
            ),
            (
                serde_json::json!({ "name": capability_id.as_str(), "detail": "everything" }),
                "capability_info detail must be names, summary, or schema",
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let mut call = provider_tool_call();
            call.id = format!("call_invalid_detail_{index}");
            call.name = capability_info::TOOL_NAME.to_string();
            call.arguments = arguments;

            port.validate_provider_tool_call(&call).expect(
                "invalid capability_info arguments should be staged for model-visible failure",
            );
            let candidate = port
                .register_provider_tool_call(call)
                .await
                .expect("invalid capability_info arguments should stage");

            assert_eq!(
                candidate.effective_capability_ids,
                vec![
                    CapabilityId::new(capability_info::CAPABILITY_ID).expect("synthetic id"),
                    capability_id.clone()
                ]
            );

            let outcome = port
                .invoke_capability(CapabilityInvocation {
                    surface_version: surface.version.clone(),
                    capability_id: candidate.capability_id,
                    input_ref: candidate.input_ref,
                    approval_resume: None,
                    auth_resume: None,
                })
                .await
                .expect("invalid arguments should return a capability failure, not a host error");

            assert!(matches!(
                outcome,
                CapabilityOutcome::Failed(CapabilityFailure {
                    error_kind: CapabilityFailureKind::InvalidInput,
                    safe_summary,
                    ..
                }) if safe_summary == expected_summary
            ));
        }
        assert!(
            result_writer.records().is_empty(),
            "failed capability_info calls are reported through the provider error-result path"
        );
        assert!(
            runtime.take_requests().is_empty(),
            "capability_info failure must not dispatch to the host runtime"
        );
    }

    #[tokio::test]
    async fn capability_info_reports_invalid_name_inputs_as_model_visible_failure() {
        let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
        let provider_id = ExtensionId::new("demo").expect("valid provider id");
        let context = execution_context("thread-capability-info-invalid-name");
        let run_context = loop_run_context(&context).await;
        let runtime = Arc::new(RecordingHostRuntime::new(vec![visible_capability(
            capability_id,
            provider_id,
        )]));
        let result_writer = Arc::new(RecordingResultWriter::default());
        let port = HostRuntimeLoopCapabilityPortFactory::new(
            runtime.clone(),
            visible_request(context),
            dummy_input_resolver(),
            result_writer.clone(),
            dummy_milestone_sink(),
        )
        .port_for_run_context(run_context);
        let surface = port
            .visible_capabilities(VisibleCapabilityRequest {})
            .await
            .expect("visible capabilities load");

        for (index, arguments) in [
            serde_json::json!({}),
            serde_json::json!({ "name": "" }),
            serde_json::json!({ "name": "demo echo" }),
            serde_json::json!({ "name": "demo.echo!" }),
            serde_json::json!({ "name": "demo.écho" }),
            serde_json::json!({ "name": "a".repeat(161) }),
        ]
        .into_iter()
        .enumerate()
        {
            let mut call = provider_tool_call();
            call.id = format!("call_invalid_name_{index}");
            call.name = capability_info::TOOL_NAME.to_string();
            call.arguments = arguments;

            port.validate_provider_tool_call(&call)
                .expect("invalid capability_info names should be staged for model-visible failure");
            let candidate = port
                .register_provider_tool_call(call)
                .await
                .expect("invalid capability_info name should stage");

            assert_eq!(
                candidate.effective_capability_ids,
                vec![CapabilityId::new(capability_info::CAPABILITY_ID).expect("synthetic id")]
            );

            let outcome = port
                .invoke_capability(CapabilityInvocation {
                    surface_version: surface.version.clone(),
                    capability_id: candidate.capability_id,
                    input_ref: candidate.input_ref,
                    approval_resume: None,
                    auth_resume: None,
                })
                .await
                .expect("invalid name should return a capability failure, not a host error");

            assert!(matches!(
                outcome,
                CapabilityOutcome::Failed(CapabilityFailure {
                    error_kind: CapabilityFailureKind::InvalidInput,
                    ..
                })
            ));
        }
        assert!(
            result_writer.records().is_empty(),
            "failed capability_info calls are reported through the provider error-result path"
        );
        assert!(
            runtime.take_requests().is_empty(),
            "capability_info failure must not dispatch to the host runtime"
        );
    }

    #[tokio::test]
    async fn capability_info_reports_unknown_targets_as_model_visible_failure() {
        let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
        let provider_id = ExtensionId::new("demo").expect("valid provider id");
        let context = execution_context("thread-capability-info-unknown-target");
        let run_context = loop_run_context(&context).await;
        let runtime = Arc::new(RecordingHostRuntime::new(vec![visible_capability(
            capability_id,
            provider_id,
        )]));
        let result_writer = Arc::new(RecordingResultWriter::default());
        let port = HostRuntimeLoopCapabilityPortFactory::new(
            runtime.clone(),
            visible_request(context),
            dummy_input_resolver(),
            result_writer.clone(),
            dummy_milestone_sink(),
        )
        .port_for_run_context(run_context);
        let surface = port
            .visible_capabilities(VisibleCapabilityRequest {})
            .await
            .expect("visible capabilities load");

        let mut call = provider_tool_call();
        call.name = capability_info::TOOL_NAME.to_string();
        call.arguments = serde_json::json!({ "name": "demo.missing" });
        let error = port
            .provider_tool_call_capability_ids(&call)
            .expect_err("approval-time capability id lookup should reject unknown targets");
        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);

        let mut malformed_call = provider_tool_call();
        malformed_call.id = "call_malformed_unknown_target".to_string();
        malformed_call.name = capability_info::TOOL_NAME.to_string();
        malformed_call.arguments =
            serde_json::json!({ "name": "demo.missing", "detail": "everything" });
        let error = port
            .provider_tool_call_capability_ids(&malformed_call)
            .expect_err("approval-time target lookup should still reject unknown targets");
        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);

        let candidate = port
            .register_provider_tool_call(call)
            .await
            .expect("unknown target should stage so the model can observe the tool error");

        assert_eq!(
            candidate.effective_capability_ids,
            vec![CapabilityId::new(capability_info::CAPABILITY_ID).expect("synthetic id")]
        );

        let outcome = port
            .invoke_capability(CapabilityInvocation {
                surface_version: surface.version,
                capability_id: candidate.capability_id,
                input_ref: candidate.input_ref,
                approval_resume: None,
                auth_resume: None,
            })
            .await
            .expect("unknown target should return a capability failure, not a host error");

        assert!(matches!(
            outcome,
            CapabilityOutcome::Failed(CapabilityFailure {
                error_kind: CapabilityFailureKind::InvalidInput,
                safe_summary,
                ..
            }) if safe_summary == "capability_info target is not on the visible surface"
        ));
        assert!(
            result_writer.records().is_empty(),
            "failed capability_info calls are reported through the provider error-result path"
        );
        assert!(
            runtime.take_requests().is_empty(),
            "capability_info failure must not dispatch to the host runtime"
        );
    }

    #[tokio::test]
    async fn capability_info_output_requires_staged_effective_target_for_visible_target() {
        let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
        let provider_id = ExtensionId::new("demo").expect("valid provider id");
        let context = execution_context("thread-capability-info-unstaged-target");
        let run_context = loop_run_context(&context).await;
        let runtime = Arc::new(RecordingHostRuntime::new(vec![visible_capability(
            capability_id.clone(),
            provider_id,
        )]));
        let result_writer = Arc::new(RecordingResultWriter::default());
        let port = HostRuntimeLoopCapabilityPortFactory::new(
            runtime.clone(),
            visible_request(context),
            Arc::new(JsonInputResolver(serde_json::json!({
                "name": capability_id.as_str(),
                "detail": "schema"
            }))),
            result_writer.clone(),
            dummy_milestone_sink(),
        )
        .port_for_run_context(run_context);
        let surface = port
            .visible_capabilities(VisibleCapabilityRequest {})
            .await
            .expect("visible capabilities load");
        assert!(
            surface
                .descriptors
                .iter()
                .any(|descriptor| descriptor.capability_id == capability_id),
            "target should be visible even when the synthetic capability_info call is unstaged"
        );

        let outcome = port
            .invoke_capability(CapabilityInvocation {
                surface_version: surface.version,
                capability_id: CapabilityId::new(capability_info::CAPABILITY_ID)
                    .expect("synthetic capability id"),
                input_ref: CapabilityInputRef::new("input:direct-capability-info")
                    .expect("test input ref"),
                approval_resume: None,
                auth_resume: None,
            })
            .await
            .expect("unstaged synthetic invocation should return a model-visible failure");

        assert!(matches!(
            outcome,
            CapabilityOutcome::Failed(CapabilityFailure {
                error_kind: CapabilityFailureKind::InvalidInput,
                safe_summary,
                ..
            }) if safe_summary == "capability_info target is not on the visible surface"
        ));
        assert!(
            result_writer.records().is_empty(),
            "unstaged capability_info calls must not write hidden schema output"
        );
        assert!(
            runtime.take_requests().is_empty(),
            "capability_info failure must not dispatch to the host runtime"
        );
    }

    #[tokio::test]
    async fn capability_info_output_rejects_visible_target_excluded_from_staged_effective_ids() {
        let allowed_capability_id =
            CapabilityId::new("demo.allowed").expect("valid allowed capability id");
        let denied_capability_id =
            CapabilityId::new("demo.denied").expect("valid denied capability id");
        let provider_id = ExtensionId::new("demo").expect("valid provider id");
        let context = execution_context("thread-capability-info-excluded-visible-target");
        let run_context = loop_run_context(&context).await;
        let runtime = Arc::new(RecordingHostRuntime::new(vec![
            visible_capability(allowed_capability_id.clone(), provider_id.clone()),
            visible_capability(denied_capability_id.clone(), provider_id),
        ]));
        let result_writer = Arc::new(RecordingResultWriter::default());
        let port = HostRuntimeLoopCapabilityPortFactory::new(
            runtime.clone(),
            visible_request(context),
            Arc::new(JsonInputResolver(serde_json::json!({
                "name": denied_capability_id.as_str(),
                "detail": "schema"
            }))),
            result_writer.clone(),
            dummy_milestone_sink(),
        )
        .port_for_run_context(run_context);
        let surface = port
            .visible_capabilities(VisibleCapabilityRequest {})
            .await
            .expect("visible capabilities load");
        assert!(
            surface
                .descriptors
                .iter()
                .any(|descriptor| descriptor.capability_id == denied_capability_id),
            "target should be visible on the raw surface"
        );

        let input_ref = CapabilityInputRef::new("input:capability-info-excluded-target")
            .expect("test input ref");
        port.record_provider_tool_call_effective_capability_ids(
            &input_ref,
            [
                CapabilityId::new(capability_info::CAPABILITY_ID).expect("synthetic id"),
                allowed_capability_id,
            ]
            .into_iter()
            .collect(),
        )
        .expect("staged effective capability ids");

        let outcome = port
            .invoke_capability(CapabilityInvocation {
                surface_version: surface.version,
                capability_id: CapabilityId::new(capability_info::CAPABILITY_ID)
                    .expect("synthetic capability id"),
                input_ref,
                approval_resume: None,
                auth_resume: None,
            })
            .await
            .expect("excluded target should return a model-visible failure");

        assert!(matches!(
            outcome,
            CapabilityOutcome::Failed(CapabilityFailure {
                error_kind: CapabilityFailureKind::InvalidInput,
                safe_summary,
                ..
            }) if safe_summary == "capability_info target is not on the visible surface"
        ));
        assert!(
            result_writer.records().is_empty(),
            "excluded capability_info calls must not write schema output"
        );
        assert!(
            runtime.take_requests().is_empty(),
            "capability_info failure must not dispatch to the host runtime"
        );
    }

    #[test]
    fn provider_tool_call_effective_capability_id_store_returns_unavailable_when_full() {
        let mut records = HashMap::new();
        for index in 0..MAX_IN_MEMORY_PROVIDER_TOOL_CALL_EFFECTIVE_CAPABILITY_IDS {
            records.insert(format!("input:staged-capability-{index}"), HashSet::new());
        }
        let mut store = ProviderToolCallEffectiveCapabilityIdStore {
            records,
            insertion_order: VecDeque::new(),
        };
        let input_ref =
            CapabilityInputRef::new("input:staged-capability-new").expect("valid input ref");

        let error = store
            .record(&input_ref, HashSet::new())
            .expect_err("full store with exhausted insertion order should fail closed");

        assert_eq!(error.kind, AgentLoopHostErrorKind::Unavailable);
    }

    /// Regression: `capability_info` previously used `as_runtime()` for
    /// surface lookup, which excluded synthetic capabilities. A model calling
    /// `capability_info { name: "capability_info" }` (to introspect the tool
    /// itself before using it) got `target is not on the visible surface` →
    /// `InvalidInvocation` → terminal run failure instead of a helpful schema
    /// response.
    #[tokio::test]
    async fn capability_info_can_describe_itself() {
        let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
        let provider_id = ExtensionId::new("demo").expect("valid provider id");
        let context = execution_context("thread-capability-info-self-lookup");
        let run_context = loop_run_context(&context).await;
        let runtime = Arc::new(RecordingHostRuntime::new(vec![visible_capability(
            capability_id,
            provider_id,
        )]));
        let port = HostRuntimeLoopCapabilityPortFactory::new(
            runtime,
            visible_request(context),
            dummy_input_resolver(),
            dummy_result_writer(),
            dummy_milestone_sink(),
        )
        .port_for_run_context(run_context);
        port.visible_capabilities(VisibleCapabilityRequest {})
            .await
            .expect("visible capabilities load");

        // Query by provider tool name
        let mut call = provider_tool_call();
        call.name = capability_info::TOOL_NAME.to_string();
        call.arguments = serde_json::json!({ "name": capability_info::TOOL_NAME });
        port.register_provider_tool_call(call)
            .await
            .expect("capability_info should be able to describe itself by tool name");

        // Query by canonical capability id
        let mut call2 = provider_tool_call();
        call2.id = "call_2".to_string();
        call2.name = capability_info::TOOL_NAME.to_string();
        call2.arguments = serde_json::json!({ "name": capability_info::CAPABILITY_ID });
        port.register_provider_tool_call(call2)
            .await
            .expect("capability_info should be able to describe itself by capability id");
    }

    #[tokio::test]
    async fn capability_info_returns_names_and_summary_details() {
        let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
        let provider_id = ExtensionId::new("demo").expect("valid provider id");
        let context = execution_context("thread-capability-info-detail-modes");
        let run_context = loop_run_context(&context).await;
        let mut visible = visible_capability(capability_id.clone(), provider_id);
        visible.descriptor.parameters_schema = serde_json::json!({
            "type": "object",
            "properties": {
                "count": { "type": "integer" },
                "message": { "type": "string" }
            },
            "required": ["message"],
            "allOf": [{
                "properties": {
                    "limit": { "type": "integer" }
                },
                "required": ["limit"]
            }],
            "anyOf": [{
                "properties": {
                    "mode": { "type": "string" }
                },
                "required": ["mode"]
            }]
        });
        let result_writer = Arc::new(RecordingResultWriter::default());
        let port = HostRuntimeLoopCapabilityPortFactory::new(
            Arc::new(RecordingHostRuntime::new(vec![visible])),
            visible_request(context),
            dummy_input_resolver(),
            result_writer.clone(),
            dummy_milestone_sink(),
        )
        .port_for_run_context(run_context);
        let surface = port
            .visible_capabilities(VisibleCapabilityRequest {})
            .await
            .expect("visible capabilities load");

        for (detail, expected_summary) in [(None, false), (Some("summary"), true)] {
            let mut call = provider_tool_call();
            call.name = capability_info::TOOL_NAME.to_string();
            call.arguments = serde_json::json!({ "name": capability_id.as_str() });
            if let Some(detail) = detail {
                call.arguments["detail"] = serde_json::json!(detail);
            }
            let candidate = port
                .register_provider_tool_call(call)
                .await
                .expect("capability_info call should register");
            port.invoke_capability(CapabilityInvocation {
                surface_version: surface.version.clone(),
                capability_id: candidate.capability_id,
                input_ref: candidate.input_ref,
                approval_resume: None,
                auth_resume: None,
            })
            .await
            .expect("capability_info invocation succeeds");

            let records = result_writer.records();
            let output = &records.last().expect("result was written").1;
            assert_eq!(
                output["parameters"],
                serde_json::json!(["count", "limit", "message", "mode"])
            );
            assert_eq!(output.get("summary").is_some(), expected_summary);
            if expected_summary {
                assert_eq!(
                    output["summary"]["always_required"],
                    serde_json::json!(["limit", "message"])
                );
                assert_eq!(
                    output["summary"]["notes"],
                    serde_json::json!(["runtime: first_party", "effects: dispatch_capability"])
                );
            }
        }
    }

    #[tokio::test]
    async fn runtime_capability_can_use_old_builtin_capability_info_id_without_synthetic_intercept()
    {
        let capability_id =
            CapabilityId::new("builtin.capability_info").expect("valid capability id");
        let provider_id = ExtensionId::new("builtin").expect("valid provider id");
        let mut context = execution_context("thread-capability-info-id-collision");
        let run_context = loop_run_context(&context).await;
        let loop_driver_extension =
            loop_driver_execution_extension_id(&run_context).expect("valid extension id");
        context.grants.grants.push(dispatch_capability_grant(
            &capability_id,
            &loop_driver_extension,
        ));

        let runtime = Arc::new(RecordingHostRuntime::new(vec![visible_capability(
            capability_id.clone(),
            provider_id.clone(),
        )]));
        let visible_request = visible_request(context).with_provider_trust(
            std::collections::BTreeMap::from([(provider_id, dispatch_trust_decision())]),
        );
        let port = HostRuntimeLoopCapabilityPortFactory::new(
            runtime.clone(),
            visible_request,
            Arc::new(StaticInputResolver),
            Arc::new(StaticResultWriter),
            dummy_milestone_sink(),
        )
        .port_for_run_context(run_context);

        let surface = port
            .visible_capabilities(VisibleCapabilityRequest {})
            .await
            .expect("visible capabilities load");
        port.invoke_capability(CapabilityInvocation {
            surface_version: surface.version,
            capability_id: capability_id.clone(),
            input_ref: CapabilityInputRef::new("input:old-builtin-capability-info")
                .expect("valid input ref"),
            approval_resume: None,
            auth_resume: None,
        })
        .await
        .expect("runtime capability invocation succeeds");

        let requests = runtime.take_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].capability_id, capability_id);
    }

    #[tokio::test]
    async fn runtime_capability_with_reserved_synthetic_id_is_rejected_from_surface() {
        let capability_id =
            CapabilityId::new(capability_info::CAPABILITY_ID).expect("valid capability id");
        let provider_id = ExtensionId::new("ironclaw.loop").expect("valid provider id");
        let context = execution_context("thread-capability-info-reserved-id");
        let run_context = loop_run_context(&context).await;
        let runtime = Arc::new(RecordingHostRuntime::new(vec![visible_capability(
            capability_id,
            provider_id,
        )]));
        let port = HostRuntimeLoopCapabilityPortFactory::new(
            runtime,
            visible_request(context),
            dummy_input_resolver(),
            dummy_result_writer(),
            dummy_milestone_sink(),
        )
        .port_for_run_context(run_context);

        let error = port
            .visible_capabilities(VisibleCapabilityRequest {})
            .await
            .expect_err("reserved synthetic capability id should be rejected");

        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
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
            approval_resume: None,
            auth_resume: None,
        })
        .await
        .expect("override invocation succeeds");
        port.invoke_capability(CapabilityInvocation {
            surface_version: surface.version,
            capability_id: default_id.clone(),
            input_ref,
            approval_resume: None,
            auth_resume: None,
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
    async fn process_sandbox_capability_invocation_uses_spawn_with_validated_plan() {
        let capability_id =
            CapabilityId::new(ironclaw_process_sandbox::PROCESS_SANDBOX_CAPABILITY_ID)
                .expect("valid capability id");
        let provider_id = ExtensionId::new("system.process_sandbox").expect("valid provider id");
        let mut context = execution_context("thread-process-sandbox-spawn");
        let run_context = loop_run_context(&context).await;
        let loop_driver_extension =
            loop_driver_execution_extension_id(&run_context).expect("valid extension id");
        let effects = vec![EffectKind::ExecuteCode, EffectKind::SpawnProcess];
        context.grants.grants.push(capability_grant_with_effects(
            &capability_id,
            &loop_driver_extension,
            effects.clone(),
        ));

        let runtime = Arc::new(RecordingHostRuntime::new(vec![
            visible_capability_with_runtime_effects(
                capability_id.clone(),
                provider_id.clone(),
                RuntimeKind::System,
                effects.clone(),
            ),
        ]));
        let visible_request = visible_request(context).with_provider_trust(
            std::collections::BTreeMap::from([(provider_id, trust_decision_with_effects(effects))]),
        );
        let port = HostRuntimeLoopCapabilityPortFactory::new(
            runtime.clone(),
            visible_request,
            Arc::new(ProcessSandboxPlanInputResolver),
            Arc::new(StaticResultWriter),
            dummy_milestone_sink(),
        )
        .port_for_run_context(run_context);
        let surface = port
            .visible_capabilities(VisibleCapabilityRequest {})
            .await
            .expect("visible capabilities load");

        let outcome = port
            .invoke_capability(CapabilityInvocation {
                surface_version: surface.version,
                capability_id: capability_id.clone(),
                input_ref: CapabilityInputRef::new("input:process-sandbox-plan")
                    .expect("valid input ref"),
                approval_resume: None,
                auth_resume: None,
            })
            .await
            .expect("process sandbox invocation succeeds");

        assert!(matches!(outcome, CapabilityOutcome::SpawnedProcess(_)));
        assert!(
            runtime.take_requests().is_empty(),
            "process sandbox capability must not use foreground invoke"
        );
        let spawn_requests = runtime.take_spawn_requests();
        assert_eq!(spawn_requests.len(), 1);
        assert_eq!(spawn_requests[0].capability_id, capability_id);
        assert_eq!(
            serde_json::from_value::<SandboxProcessPlan>(spawn_requests[0].input.clone())
                .expect("spawn input is a typed sandbox process plan")
                .run
                .command,
            "echo"
        );
    }

    #[tokio::test]
    async fn non_sandbox_capability_invocation_still_uses_invoke_capability() {
        let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
        let provider_id = ExtensionId::new("demo").expect("valid provider id");
        let runtime = Arc::new(RecordingHostRuntime::new(vec![visible_capability(
            capability_id.clone(),
            provider_id.clone(),
        )]));
        let port = runtime_capability_port(
            &capability_id,
            &provider_id,
            runtime.clone(),
            Arc::new(RecordingResultWriter::default()),
            dummy_milestone_sink(),
            "thread-non-sandbox-invoke-path",
        )
        .await;

        let outcome = invoke_visible_runtime_capability(&port)
            .await
            .expect("non-sandbox capability invocation succeeds");

        assert!(matches!(outcome, CapabilityOutcome::Completed(_)));
        let requests = runtime.take_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].capability_id, capability_id);
        assert!(
            runtime.take_spawn_requests().is_empty(),
            "non-sandbox capability must not use spawn dispatch"
        );
    }

    #[tokio::test]
    async fn runtime_capability_invocation_validates_schema_before_dispatch() {
        let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
        let provider_id = ExtensionId::new("demo").expect("valid provider id");
        let mut visible = visible_capability(capability_id.clone(), provider_id.clone());
        visible.descriptor.parameters_schema = serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "message": { "type": "string" }
            },
            "required": ["message"]
        });
        let runtime = Arc::new(RecordingHostRuntime::new(vec![visible]));
        let mut context = execution_context("thread-runtime-schema-validation");
        let run_context = loop_run_context(&context).await;
        let loop_driver_extension =
            loop_driver_execution_extension_id(&run_context).expect("valid extension id");
        context.grants.grants.push(dispatch_capability_grant(
            &capability_id,
            &loop_driver_extension,
        ));
        let port = HostRuntimeLoopCapabilityPortFactory::new(
            runtime.clone(),
            visible_request(context).with_provider_trust(std::collections::BTreeMap::from([(
                provider_id.clone(),
                dispatch_trust_decision(),
            )])),
            Arc::new(JsonInputResolver(serde_json::json!({"number": 4286}))),
            Arc::new(RecordingResultWriter::default()),
            dummy_milestone_sink(),
        )
        .port_for_run_context(run_context);
        let surface = port
            .visible_capabilities(VisibleCapabilityRequest {})
            .await
            .expect("visible capabilities load");

        let error = port
            .invoke_capability(CapabilityInvocation {
                surface_version: surface.version,
                capability_id,
                input_ref: CapabilityInputRef::new("input:direct-invalid")
                    .expect("valid input ref"),
                approval_resume: None,
                auth_resume: None,
            })
            .await
            .expect_err("invalid direct input should fail before runtime dispatch");

        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
        assert!(error.safe_summary.contains("schema validation"));
        assert!(
            runtime.take_requests().is_empty(),
            "invalid direct input must not reach the runtime"
        );
    }

    #[tokio::test]
    async fn provider_runtime_tool_call_schema_failure_is_model_visible() {
        let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
        let provider_id = ExtensionId::new("demo").expect("valid provider id");
        let mut visible = visible_capability(capability_id.clone(), provider_id.clone());
        visible.descriptor.parameters_schema = serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "message": { "type": "string" }
            },
            "required": ["message"]
        });
        let runtime = Arc::new(RecordingHostRuntime::new(vec![visible]));
        let result_writer = Arc::new(RecordingResultWriter::default());
        let context = execution_context("thread-provider-runtime-schema-validation");
        let run_context = loop_run_context(&context).await;
        let port = HostRuntimeLoopCapabilityPortFactory::new(
            runtime.clone(),
            visible_request(context).with_provider_trust(std::collections::BTreeMap::from([(
                provider_id,
                dispatch_trust_decision(),
            )])),
            dummy_input_resolver(),
            result_writer.clone(),
            dummy_milestone_sink(),
        )
        .port_for_run_context(run_context);
        let surface = port
            .visible_capabilities(VisibleCapabilityRequest {})
            .await
            .expect("visible capabilities load");
        let tool_definition = port
            .tool_definitions()
            .expect("tool definitions")
            .into_iter()
            .find(|definition| definition.capability_id == capability_id)
            .expect("runtime capability advertised to provider");

        let mut call = provider_tool_call();
        call.name = tool_definition.name;
        call.arguments = serde_json::json!({});
        port.validate_provider_tool_call(&call)
            .expect("schema-invalid provider calls should stage for model-visible failure");
        let candidate = port
            .register_provider_tool_call(call)
            .await
            .expect("schema-invalid provider calls should register");
        assert!(
            candidate
                .input_ref
                .as_str()
                .starts_with("input:provider-tool-")
        );

        let outcome = port
            .invoke_capability(CapabilityInvocation {
                surface_version: surface.version,
                capability_id,
                input_ref: candidate.input_ref,
                approval_resume: None,
                auth_resume: None,
            })
            .await
            .expect("schema-invalid provider calls should produce a capability failure");

        let CapabilityOutcome::Failed(CapabilityFailure {
            error_kind,
            safe_summary,
            detail,
        }) = outcome
        else {
            panic!("expected schema-invalid provider call to fail");
        };
        assert_eq!(error_kind, CapabilityFailureKind::InvalidInput);
        assert!(safe_summary.contains("schema validation"));
        let Some(ironclaw_turns::run_profile::CapabilityFailureDetail::InvalidInput { issues }) =
            detail
        else {
            panic!("schema-invalid provider call should include invalid input detail");
        };
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].path, "message");
        assert_eq!(
            issues[0].code,
            ironclaw_turns::run_profile::CapabilityInputIssueCode::MissingRequired
        );
        assert_eq!(issues[0].expected.as_deref(), Some("required field"));
        assert!(
            runtime.take_requests().is_empty(),
            "schema-invalid provider input must not reach the runtime"
        );
        assert!(
            result_writer.records().is_empty(),
            "schema-invalid provider calls should report through the provider error-result path"
        );
    }

    #[tokio::test]
    async fn provider_runtime_tool_call_schema_failure_preserves_type_mismatch_detail() {
        let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
        let provider_id = ExtensionId::new("demo").expect("valid provider id");
        let mut visible = visible_capability(capability_id.clone(), provider_id.clone());
        visible.descriptor.parameters_schema = serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "message": { "type": "string" },
                "limit": { "type": "integer" }
            },
            "required": ["message"]
        });
        let runtime = Arc::new(RecordingHostRuntime::new(vec![visible]));
        let result_writer = Arc::new(RecordingResultWriter::default());
        let context = execution_context("thread-provider-runtime-schema-detail-validation");
        let run_context = loop_run_context(&context).await;
        let port = HostRuntimeLoopCapabilityPortFactory::new(
            runtime.clone(),
            visible_request(context).with_provider_trust(std::collections::BTreeMap::from([(
                provider_id,
                dispatch_trust_decision(),
            )])),
            dummy_input_resolver(),
            result_writer.clone(),
            dummy_milestone_sink(),
        )
        .port_for_run_context(run_context);
        let surface = port
            .visible_capabilities(VisibleCapabilityRequest {})
            .await
            .expect("visible capabilities load");
        let tool_definition = port
            .tool_definitions()
            .expect("tool definitions")
            .into_iter()
            .find(|definition| definition.capability_id == capability_id)
            .expect("runtime capability advertised to provider");

        let mut call = provider_tool_call();
        call.name = tool_definition.name;
        call.arguments = serde_json::json!({
            "message": 123
        });
        port.validate_provider_tool_call(&call)
            .expect("schema-invalid provider calls should stage for model-visible failure");
        let candidate = port
            .register_provider_tool_call(call)
            .await
            .expect("schema-invalid provider calls should register");

        let outcome = port
            .invoke_capability(CapabilityInvocation {
                surface_version: surface.version,
                capability_id,
                input_ref: candidate.input_ref,
                approval_resume: None,
                auth_resume: None,
            })
            .await
            .expect("schema-invalid provider calls should produce a capability failure");

        let CapabilityOutcome::Failed(CapabilityFailure {
            error_kind, detail, ..
        }) = outcome
        else {
            panic!("expected schema-invalid provider call to fail");
        };
        assert_eq!(error_kind, CapabilityFailureKind::InvalidInput);
        let Some(ironclaw_turns::run_profile::CapabilityFailureDetail::InvalidInput { issues }) =
            detail
        else {
            panic!("schema-invalid provider call should include invalid input detail");
        };
        assert!(
            issues.iter().any(|issue| {
                issue.path == "message"
                    && issue.code
                        == ironclaw_turns::run_profile::CapabilityInputIssueCode::TypeMismatch
                    && issue.expected.as_deref() == Some("string")
                    && issue.received.as_deref() == Some("integer")
            }),
            "type mismatch issue should identify the mismatched field"
        );
        assert!(
            runtime.take_requests().is_empty(),
            "schema-invalid provider input must not reach the runtime"
        );
        assert!(
            result_writer.records().is_empty(),
            "schema-invalid provider calls should report through the provider error-result path"
        );
    }

    #[tokio::test]
    async fn provider_runtime_tool_call_schema_failure_preserves_unexpected_field_detail() {
        let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
        let provider_id = ExtensionId::new("demo").expect("valid provider id");
        let mut visible = visible_capability(capability_id.clone(), provider_id.clone());
        visible.descriptor.parameters_schema = serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "message": { "type": "string" }
            },
            "required": ["message"]
        });
        let runtime = Arc::new(RecordingHostRuntime::new(vec![visible]));
        let result_writer = Arc::new(RecordingResultWriter::default());
        let context = execution_context("thread-provider-runtime-unexpected-field-validation");
        let run_context = loop_run_context(&context).await;
        let port = HostRuntimeLoopCapabilityPortFactory::new(
            runtime.clone(),
            visible_request(context).with_provider_trust(std::collections::BTreeMap::from([(
                provider_id,
                dispatch_trust_decision(),
            )])),
            dummy_input_resolver(),
            result_writer.clone(),
            dummy_milestone_sink(),
        )
        .port_for_run_context(run_context);
        let surface = port
            .visible_capabilities(VisibleCapabilityRequest {})
            .await
            .expect("visible capabilities load");
        let tool_definition = port
            .tool_definitions()
            .expect("tool definitions")
            .into_iter()
            .find(|definition| definition.capability_id == capability_id)
            .expect("runtime capability advertised to provider");

        let mut call = provider_tool_call();
        call.name = tool_definition.name;
        call.arguments = serde_json::json!({
            "message": "hello",
            "unexpected": true
        });
        port.validate_provider_tool_call(&call)
            .expect("schema-invalid provider calls should stage for model-visible failure");
        let candidate = port
            .register_provider_tool_call(call)
            .await
            .expect("schema-invalid provider calls should register");

        let outcome = port
            .invoke_capability(CapabilityInvocation {
                surface_version: surface.version,
                capability_id,
                input_ref: candidate.input_ref,
                approval_resume: None,
                auth_resume: None,
            })
            .await
            .expect("schema-invalid provider calls should produce a capability failure");

        let CapabilityOutcome::Failed(CapabilityFailure {
            error_kind, detail, ..
        }) = outcome
        else {
            panic!("expected schema-invalid provider call to fail");
        };
        assert_eq!(error_kind, CapabilityFailureKind::InvalidInput);
        let Some(ironclaw_turns::run_profile::CapabilityFailureDetail::InvalidInput { issues }) =
            detail
        else {
            panic!("schema-invalid provider call should include invalid input detail");
        };
        assert!(
            issues.iter().any(|issue| {
                issue.path == "unexpected"
                    && issue.code
                        == ironclaw_turns::run_profile::CapabilityInputIssueCode::UnexpectedField
            }),
            "unexpected field issue should identify the field to remove"
        );
        assert!(
            runtime.take_requests().is_empty(),
            "schema-invalid provider input must not reach the runtime"
        );
        assert!(
            result_writer.records().is_empty(),
            "schema-invalid provider calls should report through the provider error-result path"
        );
    }

    #[tokio::test]
    async fn runtime_capability_invocation_normalizes_input_before_dispatch() {
        let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
        let provider_id = ExtensionId::new("demo").expect("valid provider id");
        let mut visible = visible_capability(capability_id.clone(), provider_id.clone());
        visible.descriptor.parameters_schema = serde_json::json!({
            "type": "object",
            "properties": {
                "limit": { "type": "integer" }
            },
            "required": ["limit"]
        });
        let runtime = Arc::new(RecordingHostRuntime::new(vec![visible]));
        let mut context = execution_context("thread-runtime-input-normalization");
        let run_context = loop_run_context(&context).await;
        let loop_driver_extension =
            loop_driver_execution_extension_id(&run_context).expect("valid extension id");
        context.grants.grants.push(dispatch_capability_grant(
            &capability_id,
            &loop_driver_extension,
        ));
        let port = HostRuntimeLoopCapabilityPortFactory::new(
            runtime.clone(),
            visible_request(context).with_provider_trust(std::collections::BTreeMap::from([(
                provider_id.clone(),
                dispatch_trust_decision(),
            )])),
            Arc::new(JsonInputResolver(serde_json::json!({"limit": "10"}))),
            Arc::new(RecordingResultWriter::default()),
            dummy_milestone_sink(),
        )
        .port_for_run_context(run_context);
        let surface = port
            .visible_capabilities(VisibleCapabilityRequest {})
            .await
            .expect("visible capabilities load");

        port.invoke_capability(CapabilityInvocation {
            surface_version: surface.version,
            capability_id,
            input_ref: CapabilityInputRef::new("input:direct-normalized").expect("valid input ref"),
            approval_resume: None,
            auth_resume: None,
        })
        .await
        .expect("valid direct input should dispatch");

        let requests = runtime.take_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].input, serde_json::json!({"limit": 10}));
    }

    #[tokio::test]
    async fn process_sandbox_capability_rejects_invalid_plan_before_runtime_spawn() {
        let capability_id =
            CapabilityId::new(ironclaw_process_sandbox::PROCESS_SANDBOX_CAPABILITY_ID)
                .expect("valid capability id");
        let provider_id = ExtensionId::new("system.process_sandbox").expect("valid provider id");
        let mut context = execution_context("thread-process-sandbox-invalid-plan");
        let run_context = loop_run_context(&context).await;
        let loop_driver_extension =
            loop_driver_execution_extension_id(&run_context).expect("valid extension id");
        let effects = vec![EffectKind::ExecuteCode, EffectKind::SpawnProcess];
        context.grants.grants.push(capability_grant_with_effects(
            &capability_id,
            &loop_driver_extension,
            effects.clone(),
        ));
        let runtime = Arc::new(RecordingHostRuntime::new(vec![
            visible_capability_with_runtime_effects(
                capability_id.clone(),
                provider_id.clone(),
                RuntimeKind::System,
                effects.clone(),
            ),
        ]));
        let visible_request = visible_request(context).with_provider_trust(
            std::collections::BTreeMap::from([(provider_id, trust_decision_with_effects(effects))]),
        );
        let port = HostRuntimeLoopCapabilityPortFactory::new(
            runtime.clone(),
            visible_request,
            Arc::new(InvalidProcessSandboxPlanInputResolver),
            Arc::new(StaticResultWriter),
            dummy_milestone_sink(),
        )
        .port_for_run_context(run_context);
        let surface = port
            .visible_capabilities(VisibleCapabilityRequest {})
            .await
            .expect("visible capabilities load");

        let error = port
            .invoke_capability(CapabilityInvocation {
                surface_version: surface.version,
                capability_id,
                input_ref: CapabilityInputRef::new("input:invalid-process-sandbox-plan")
                    .expect("valid input ref"),
                approval_resume: None,
                auth_resume: None,
            })
            .await
            .expect_err("invalid process sandbox plan must fail before runtime dispatch");

        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
        assert!(runtime.take_requests().is_empty());
        assert!(runtime.take_spawn_requests().is_empty());
    }

    #[tokio::test]
    async fn process_sandbox_capability_rejects_malformed_plan_before_runtime_spawn() {
        let capability_id =
            CapabilityId::new(ironclaw_process_sandbox::PROCESS_SANDBOX_CAPABILITY_ID)
                .expect("valid capability id");
        let provider_id = ExtensionId::new("system.process_sandbox").expect("valid provider id");
        let mut context = execution_context("thread-process-sandbox-malformed-plan");
        let run_context = loop_run_context(&context).await;
        let loop_driver_extension =
            loop_driver_execution_extension_id(&run_context).expect("valid extension id");
        let effects = vec![EffectKind::ExecuteCode, EffectKind::SpawnProcess];
        context.grants.grants.push(capability_grant_with_effects(
            &capability_id,
            &loop_driver_extension,
            effects.clone(),
        ));
        let runtime = Arc::new(RecordingHostRuntime::new(vec![
            visible_capability_with_runtime_effects(
                capability_id.clone(),
                provider_id.clone(),
                RuntimeKind::System,
                effects.clone(),
            ),
        ]));
        let visible_request = visible_request(context).with_provider_trust(
            std::collections::BTreeMap::from([(provider_id, trust_decision_with_effects(effects))]),
        );
        let port = HostRuntimeLoopCapabilityPortFactory::new(
            runtime.clone(),
            visible_request,
            Arc::new(MalformedProcessSandboxPlanInputResolver),
            Arc::new(StaticResultWriter),
            dummy_milestone_sink(),
        )
        .port_for_run_context(run_context);
        let surface = port
            .visible_capabilities(VisibleCapabilityRequest {})
            .await
            .expect("visible capabilities load");

        let error = port
            .invoke_capability(CapabilityInvocation {
                surface_version: surface.version,
                capability_id,
                input_ref: CapabilityInputRef::new("input:malformed-process-sandbox-plan")
                    .expect("valid input ref"),
                approval_resume: None,
                auth_resume: None,
            })
            .await
            .expect_err("malformed process sandbox plan must fail before runtime dispatch");

        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
        assert!(runtime.take_requests().is_empty());
        assert!(runtime.take_spawn_requests().is_empty());
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
        let capability = RuntimeSurfaceCapabilitySnapshot {
            provider: ExtensionId::new("demo").expect("valid provider"),
            runtime: RuntimeKind::Wasm,
            estimate: ResourceEstimate::default(),
            safe_description: "demo capability".to_string(),
            parameters_schema: serde_json::json!({"type":"object"}),
            effects: vec![EffectKind::ReadFilesystem],
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
        let capability = RuntimeSurfaceCapabilitySnapshot {
            provider: ExtensionId::new("demo").expect("valid provider"),
            runtime: RuntimeKind::Wasm,
            estimate: ResourceEstimate::default(),
            safe_description: "demo capability".to_string(),
            parameters_schema: serde_json::json!({"type":"object"}),
            effects: vec![EffectKind::ReadFilesystem],
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
        let capability = RuntimeSurfaceCapabilitySnapshot {
            provider: ExtensionId::new("demo").expect("valid provider"),
            runtime: RuntimeKind::Wasm,
            estimate: ResourceEstimate::default(),
            safe_description: "demo capability".to_string(),
            parameters_schema: serde_json::json!({"type":"object"}),
            effects: vec![EffectKind::ReadFilesystem],
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
        let capability = RuntimeSurfaceCapabilitySnapshot {
            provider: ExtensionId::new("demo").expect("valid provider"),
            runtime: RuntimeKind::FirstParty,
            estimate: ResourceEstimate::default(),
            safe_description: "demo echo".to_string(),
            parameters_schema: serde_json::json!({ "type": "object" }),
            effects: vec![EffectKind::DispatchCapability],
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
        let capability = RuntimeSurfaceCapabilitySnapshot {
            provider: ExtensionId::new("demo").expect("valid provider"),
            runtime: RuntimeKind::Script,
            estimate: ResourceEstimate::default(),
            safe_description: "demo capability".to_string(),
            parameters_schema: serde_json::json!({"type":"object"}),
            effects: vec![EffectKind::ExecuteCode],
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

    /// Guard: a `CapabilityInvocation` with both `approval_resume` and `auth_resume` set
    /// must be rejected fail-closed with `InvalidInvocation` — the two resume modes are
    /// mutually exclusive and simultaneous presence indicates a malformed invocation.
    #[tokio::test]
    async fn invoke_capability_rejects_both_resume_modes_set() {
        use ironclaw_host_api::ApprovalRequestId;
        use ironclaw_turns::run_profile::{CapabilityApprovalResume, CapabilityAuthResume};

        let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
        let provider_id = ExtensionId::new("demo").expect("valid provider id");
        let port = runtime_capability_port(
            &capability_id,
            &provider_id,
            Arc::new(RecordingHostRuntime::new(vec![visible_capability(
                capability_id.clone(),
                provider_id.clone(),
            )])),
            dummy_result_writer(),
            dummy_milestone_sink(),
            "thread-both-resume-modes-set",
        )
        .await;

        // Obtain a valid surface_version and input_ref so the invocation
        // reaches the dispatch match — the guard fires there.
        let invocation = visible_runtime_invocation(&port).await;

        let resume_token =
            CapabilityResumeToken::new(InvocationId::new().to_string()).expect("valid token");
        let dual_resume_invocation = CapabilityInvocation {
            surface_version: invocation.surface_version,
            capability_id: invocation.capability_id,
            input_ref: invocation.input_ref,
            approval_resume: Some(CapabilityApprovalResume {
                approval_request_id: ApprovalRequestId::new(),
                resume_token: resume_token.clone(),
                correlation_id: CorrelationId::new(),
                input_ref: CapabilityInputRef::new("input:test-dual-resume")
                    .expect("valid input ref"),
                input: serde_json::json!({}),
                estimate: ResourceEstimate::default(),
            }),
            auth_resume: Some(CapabilityAuthResume {
                resume_token,
                prior_approval: None,
            }),
        };

        let err = port
            .invoke_capability(dual_resume_invocation)
            .await
            .expect_err("dual-resume invocation must be rejected");

        assert_eq!(
            err.kind,
            AgentLoopHostErrorKind::InvalidInvocation,
            "expected InvalidInvocation, got {:?}",
            err.kind
        );
        assert!(
            err.safe_summary.contains("mutually exclusive"),
            "error message should name the mutual-exclusion constraint: {:?}",
            err.safe_summary
        );
    }

    fn visible_request(
        context: ExecutionContext,
    ) -> ironclaw_host_runtime::VisibleCapabilityRequest {
        ironclaw_host_runtime::VisibleCapabilityRequest::new(
            context,
            SurfaceKind::new("test").expect("valid surface kind"),
        )
    }

    struct DecoratorTestFactory {
        port: Arc<dyn LoopCapabilityPort>,
    }

    #[async_trait]
    impl LoopCapabilityPortFactory for DecoratorTestFactory {
        async fn create_capability_port(
            &self,
            _run_context: &LoopRunContext,
        ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
            Ok(Arc::clone(&self.port))
        }
    }

    struct FailingDecoratorFactory {
        error: AgentLoopHostError,
    }

    #[async_trait]
    impl LoopCapabilityPortFactory for FailingDecoratorFactory {
        async fn create_capability_port(
            &self,
            _run_context: &LoopRunContext,
        ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
            Err(self.error.clone())
        }
    }

    struct DecoratorTestPort {
        label: &'static str,
        log: Arc<Mutex<Vec<&'static str>>>,
    }

    #[async_trait]
    impl LoopCapabilityPort for DecoratorTestPort {
        async fn visible_capabilities(
            &self,
            _request: VisibleCapabilityRequest,
        ) -> Result<ironclaw_turns::run_profile::VisibleCapabilitySurface, AgentLoopHostError>
        {
            self.log.lock().expect("log lock").push(self.label);
            Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                format!("{label} failed", label = self.label),
            ))
        }

        async fn invoke_capability(
            &self,
            _request: CapabilityInvocation,
        ) -> Result<CapabilityOutcome, AgentLoopHostError> {
            Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                format!("{label} unused", label = self.label),
            ))
        }

        async fn invoke_capability_batch(
            &self,
            _request: CapabilityBatchInvocation,
        ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
            Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                format!("{label} unused", label = self.label),
            ))
        }
    }

    struct LoggingDecorator {
        label: &'static str,
        log: Arc<Mutex<Vec<&'static str>>>,
    }

    impl LoopCapabilityPortDecorator for LoggingDecorator {
        fn decorate(
            &self,
            _run_context: &LoopRunContext,
            inner: Arc<dyn LoopCapabilityPort>,
        ) -> Arc<dyn LoopCapabilityPort> {
            Arc::new(LoggingDecoratorPort {
                label: self.label,
                log: Arc::clone(&self.log),
                inner,
            })
        }
    }

    struct LoggingDecoratorPort {
        label: &'static str,
        log: Arc<Mutex<Vec<&'static str>>>,
        inner: Arc<dyn LoopCapabilityPort>,
    }

    #[async_trait]
    impl LoopCapabilityPort for LoggingDecoratorPort {
        async fn visible_capabilities(
            &self,
            request: VisibleCapabilityRequest,
        ) -> Result<ironclaw_turns::run_profile::VisibleCapabilitySurface, AgentLoopHostError>
        {
            self.log.lock().expect("log lock").push(self.label);
            self.inner.visible_capabilities(request).await
        }

        async fn invoke_capability(
            &self,
            request: CapabilityInvocation,
        ) -> Result<CapabilityOutcome, AgentLoopHostError> {
            self.log.lock().expect("log lock").push(self.label);
            self.inner.invoke_capability(request).await
        }

        async fn invoke_capability_batch(
            &self,
            request: CapabilityBatchInvocation,
        ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
            self.log.lock().expect("log lock").push(self.label);
            self.inner.invoke_capability_batch(request).await
        }
    }

    struct NoopDecorator {
        decorate_calls: Arc<AtomicUsize>,
    }

    impl LoopCapabilityPortDecorator for NoopDecorator {
        fn decorate(
            &self,
            _run_context: &LoopRunContext,
            inner: Arc<dyn LoopCapabilityPort>,
        ) -> Arc<dyn LoopCapabilityPort> {
            self.decorate_calls.fetch_add(1, Ordering::SeqCst);
            inner
        }
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
        capability_grant_with_effects(capability_id, grantee, vec![EffectKind::DispatchCapability])
    }

    fn capability_grant_with_effects(
        capability_id: &CapabilityId,
        grantee: &ExtensionId,
        allowed_effects: Vec<EffectKind>,
    ) -> CapabilityGrant {
        CapabilityGrant {
            id: CapabilityGrantId::new(),
            capability: capability_id.clone(),
            grantee: Principal::Extension(grantee.clone()),
            issued_by: Principal::HostRuntime,
            constraints: GrantConstraints {
                allowed_effects,
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
        trust_decision_with_effects(vec![EffectKind::DispatchCapability])
    }

    fn trust_decision_with_effects(allowed_effects: Vec<EffectKind>) -> TrustDecision {
        TrustDecision {
            effective_trust: EffectiveTrustClass::user_trusted(),
            authority_ceiling: AuthorityCeiling {
                allowed_effects,
                max_resource_ceiling: None,
            },
            provenance: TrustProvenance::Default,
            evaluated_at: chrono::Utc::now(),
        }
    }

    fn visible_capability(id: CapabilityId, provider: ExtensionId) -> VisibleCapability {
        visible_capability_with_runtime_effects(
            id,
            provider,
            RuntimeKind::FirstParty,
            vec![EffectKind::DispatchCapability],
        )
    }

    fn visible_capability_with_runtime_effects(
        id: CapabilityId,
        provider: ExtensionId,
        runtime: RuntimeKind,
        effects: Vec<EffectKind>,
    ) -> VisibleCapability {
        VisibleCapability {
            descriptor: CapabilityDescriptor {
                id,
                provider,
                runtime,
                trust_ceiling: TrustClass::UserTrusted,
                description: "demo capability".to_string(),
                parameters_schema: serde_json::json!({"type":"object"}),
                effects,
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

    const RECORDING_OUTPUT_BYTES: u64 = 12;

    async fn runtime_capability_port(
        capability_id: &CapabilityId,
        provider_id: &ExtensionId,
        runtime: Arc<dyn HostRuntime>,
        result_writer: Arc<dyn LoopCapabilityResultWriter>,
        milestone_sink: Arc<dyn LoopHostMilestoneSink>,
        thread_id: &str,
    ) -> HostRuntimeLoopCapabilityPort {
        let mut context = execution_context(thread_id);
        let run_context = loop_run_context(&context).await;
        let loop_driver_extension =
            loop_driver_execution_extension_id(&run_context).expect("valid extension id");
        context.grants.grants.push(dispatch_capability_grant(
            capability_id,
            &loop_driver_extension,
        ));
        HostRuntimeLoopCapabilityPortFactory::new(
            runtime,
            visible_request(context).with_provider_trust(std::collections::BTreeMap::from([(
                provider_id.clone(),
                dispatch_trust_decision(),
            )])),
            dummy_input_resolver(),
            result_writer,
            milestone_sink,
        )
        .port_for_run_context(run_context)
    }

    async fn visible_runtime_invocation(
        port: &HostRuntimeLoopCapabilityPort,
    ) -> CapabilityInvocation {
        let surface = port
            .visible_capabilities(VisibleCapabilityRequest {})
            .await
            .expect("visible capabilities load");
        let candidate = port
            .register_provider_tool_call(provider_tool_call())
            .await
            .expect("provider tool call registers");
        CapabilityInvocation {
            surface_version: surface.version,
            capability_id: candidate.capability_id,
            input_ref: candidate.input_ref,
            approval_resume: None,
            auth_resume: None,
        }
    }

    async fn invoke_visible_runtime_capability(
        port: &HostRuntimeLoopCapabilityPort,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        port.invoke_capability(visible_runtime_invocation(port).await)
            .await
    }

    struct RecordingHostRuntime {
        capabilities: Vec<VisibleCapability>,
        requests: Mutex<Vec<RuntimeCapabilityRequest>>,
        spawn_requests: Mutex<Vec<RuntimeCapabilityRequest>>,
    }

    impl RecordingHostRuntime {
        fn new(capabilities: Vec<VisibleCapability>) -> Self {
            Self {
                capabilities,
                requests: Mutex::new(Vec::new()),
                spawn_requests: Mutex::new(Vec::new()),
            }
        }

        fn take_requests(&self) -> Vec<RuntimeCapabilityRequest> {
            self.requests.lock().expect("requests lock").clone()
        }

        fn take_spawn_requests(&self) -> Vec<RuntimeCapabilityRequest> {
            self.spawn_requests
                .lock()
                .expect("spawn requests lock")
                .clone()
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
                    display_preview: None,
                    usage: ResourceUsage {
                        output_bytes: RECORDING_OUTPUT_BYTES,
                        ..ResourceUsage::default()
                    },
                },
            )))
        }

        async fn spawn_capability(
            &self,
            request: RuntimeCapabilityRequest,
        ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
            self.spawn_requests
                .lock()
                .expect("spawn requests lock")
                .push(request.clone());
            Ok(RuntimeCapabilityOutcome::SpawnedProcess(
                ironclaw_host_runtime::RuntimeProcessHandle {
                    process_id: ironclaw_host_api::ProcessId::new(),
                    capability_id: request.capability_id,
                },
            ))
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

    struct QueuedHostRuntime {
        capabilities: Vec<VisibleCapability>,
        outcomes: Mutex<VecDeque<Result<RuntimeCapabilityOutcome, HostRuntimeError>>>,
    }

    impl QueuedHostRuntime {
        fn new(
            capabilities: Vec<VisibleCapability>,
            outcomes: Vec<Result<RuntimeCapabilityOutcome, HostRuntimeError>>,
        ) -> Self {
            Self {
                capabilities,
                outcomes: Mutex::new(VecDeque::from(outcomes)),
            }
        }
    }

    #[async_trait]
    impl HostRuntime for QueuedHostRuntime {
        async fn invoke_capability(
            &self,
            _request: RuntimeCapabilityRequest,
        ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
            self.outcomes
                .lock()
                .expect("outcomes lock")
                .pop_front()
                .expect("queued host runtime outcome")
        }

        async fn resume_capability(
            &self,
            _request: RuntimeCapabilityResumeRequest,
        ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
            unreachable!("queued host runtime should not resume")
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
            unreachable!("queued host runtime should not cancel work")
        }

        async fn runtime_status(
            &self,
            _request: RuntimeStatusRequest,
        ) -> Result<HostRuntimeStatus, HostRuntimeError> {
            unreachable!("queued host runtime should not report status")
        }

        async fn health(&self) -> Result<HostRuntimeHealth, HostRuntimeError> {
            unreachable!("queued host runtime should not report health")
        }
    }

    #[derive(Default)]
    struct FailOnceTerminalMilestoneSink {
        failures: AtomicUsize,
        milestones: Mutex<Vec<ironclaw_turns::run_profile::LoopHostMilestone>>,
    }

    impl FailOnceTerminalMilestoneSink {
        fn milestones(&self) -> Vec<ironclaw_turns::run_profile::LoopHostMilestone> {
            self.milestones.lock().expect("milestones lock").clone()
        }
    }

    #[async_trait]
    impl LoopHostMilestoneSink for FailOnceTerminalMilestoneSink {
        async fn publish_loop_milestone(
            &self,
            milestone: ironclaw_turns::run_profile::LoopHostMilestone,
        ) -> Result<(), AgentLoopHostError> {
            let is_terminal = matches!(
                &milestone.kind,
                ironclaw_turns::run_profile::LoopHostMilestoneKind::CapabilityCompleted { .. }
                    | ironclaw_turns::run_profile::LoopHostMilestoneKind::CapabilityFailed { .. }
            );
            if is_terminal && self.failures.fetch_add(1, Ordering::SeqCst) == 0 {
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Unavailable,
                    "terminal milestone sink unavailable",
                ));
            }
            self.milestones
                .lock()
                .expect("milestones lock")
                .push(milestone);
            Ok(())
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

    struct JsonInputResolver(serde_json::Value);

    #[async_trait]
    impl LoopCapabilityInputResolver for JsonInputResolver {
        async fn resolve_capability_input(
            &self,
            _run_context: &LoopRunContext,
            _input_ref: &CapabilityInputRef,
        ) -> Result<serde_json::Value, AgentLoopHostError> {
            Ok(self.0.clone())
        }
    }

    struct ProcessSandboxPlanInputResolver;

    #[async_trait]
    impl LoopCapabilityInputResolver for ProcessSandboxPlanInputResolver {
        async fn resolve_capability_input(
            &self,
            _run_context: &LoopRunContext,
            _input_ref: &CapabilityInputRef,
        ) -> Result<serde_json::Value, AgentLoopHostError> {
            Ok(serde_json::json!({
                "run": {
                    "command": "echo",
                    "args": ["ok"]
                }
            }))
        }
    }

    struct InvalidProcessSandboxPlanInputResolver;

    #[async_trait]
    impl LoopCapabilityInputResolver for InvalidProcessSandboxPlanInputResolver {
        async fn resolve_capability_input(
            &self,
            _run_context: &LoopRunContext,
            _input_ref: &CapabilityInputRef,
        ) -> Result<serde_json::Value, AgentLoopHostError> {
            Ok(serde_json::json!({
                "run": {
                    "command": ""
                }
            }))
        }
    }

    struct MalformedProcessSandboxPlanInputResolver;

    #[async_trait]
    impl LoopCapabilityInputResolver for MalformedProcessSandboxPlanInputResolver {
        async fn resolve_capability_input(
            &self,
            _run_context: &LoopRunContext,
            _input_ref: &CapabilityInputRef,
        ) -> Result<serde_json::Value, AgentLoopHostError> {
            Ok(serde_json::json!({
                "not_run": true
            }))
        }
    }

    struct StaticResultWriter;

    #[async_trait]
    impl LoopCapabilityResultWriter for StaticResultWriter {
        async fn write_capability_result(
            &self,
            _write: CapabilityResultWrite<'_>,
        ) -> Result<(LoopResultRef, u64), AgentLoopHostError> {
            let result_ref = LoopResultRef::new("result:mount-test").map_err(|_| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Internal,
                    "result ref could not be built",
                )
            })?;
            Ok((result_ref, 0))
        }
    }

    #[derive(Default)]
    struct FailOnceResultWriter {
        attempts: AtomicUsize,
    }

    impl FailOnceResultWriter {
        fn attempts(&self) -> usize {
            self.attempts.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl LoopCapabilityResultWriter for FailOnceResultWriter {
        async fn write_capability_result(
            &self,
            _write: CapabilityResultWrite<'_>,
        ) -> Result<(LoopResultRef, u64), AgentLoopHostError> {
            if self.attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::TranscriptWriteFailed,
                    "transient result write failure",
                ));
            }
            let result_ref = LoopResultRef::new("result:capability-info-retry").map_err(|_| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Internal,
                    "result ref could not be built",
                )
            })?;
            Ok((result_ref, 0))
        }
    }

    #[derive(Default)]
    struct RecordingResultWriter {
        records: Mutex<Vec<(CapabilityId, serde_json::Value)>>,
        display_previews: Mutex<Vec<Option<CapabilityDisplayOutputPreview>>>,
    }

    impl RecordingResultWriter {
        fn records(&self) -> Vec<(CapabilityId, serde_json::Value)> {
            self.records.lock().expect("records lock").clone()
        }

        fn display_previews(&self) -> Vec<Option<CapabilityDisplayOutputPreview>> {
            self.display_previews
                .lock()
                .expect("display previews lock")
                .clone()
        }
    }

    #[async_trait]
    impl LoopCapabilityResultWriter for RecordingResultWriter {
        async fn write_capability_result(
            &self,
            write: CapabilityResultWrite<'_>,
        ) -> Result<(LoopResultRef, u64), AgentLoopHostError> {
            self.records
                .lock()
                .expect("records lock")
                .push((write.capability_id.clone(), write.output));
            self.display_previews
                .lock()
                .expect("display previews lock")
                .push(write.display_preview);
            let result_ref = LoopResultRef::new("result:capability-info").map_err(|_| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Internal,
                    "result ref could not be built",
                )
            })?;
            Ok((result_ref, 0))
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
            _write: CapabilityResultWrite<'_>,
        ) -> Result<(LoopResultRef, u64), AgentLoopHostError> {
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
