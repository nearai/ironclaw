use std::{
    collections::{HashMap, VecDeque},
    error::Error,
    fmt,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use ironclaw_hooks::dispatch::{HookDispatcher, HookDispatcherBuilder};
use ironclaw_hooks::middleware::{
    CapabilityInputResolver as HookCapabilityInputResolver,
    CapabilityProviderResolver as HookCapabilityProviderResolver, HookPromptMaterializationSink,
    HookedLoopCapabilityPort, HookedLoopCheckpointPort, HookedLoopModelPort, HookedLoopPromptPort,
    HookedLoopTranscriptPort,
};
use ironclaw_host_api::{
    CapabilityId, CorrelationId, ExecutionContext, ExtensionId, InvocationId, ResourceEstimate,
    sha256_digest_token,
};
use ironclaw_host_runtime::{
    HostRuntime, HostRuntimeError, IdempotencyKey, RuntimeBlockedReason, RuntimeCapabilityOutcome,
    RuntimeCapabilityRequest, RuntimeFailureKind,
};
use ironclaw_loop_support::{
    EmptyLoopCapabilityPort, HostManagedModelGateway, HostSkillContextSource,
    ThreadBackedLoopContextPort, ThreadBackedLoopModelPort, ThreadBackedLoopTranscriptPort,
};
use ironclaw_threads::{SessionThreadService, ThreadScope};

use crate::model_routes::{ModelRouteError, ModelRouteResolver, ModelSlot};

use ironclaw_turns::{
    CheckpointStateStore, GetCheckpointStateRequest, LoopCheckpointStateRef, LoopCheckpointStore,
    LoopGateRef, LoopResultRef, PutCheckpointStateRequest, PutLoopCheckpointRequest, RunProfileId,
    TurnCheckpointId, TurnError, TurnStatus,
    run_profile::{
        AgentLoopHostError, AgentLoopHostErrorKind, AppendCapabilityResultRef, BeginAssistantDraft,
        CapabilityBatchInvocation, CapabilityBatchOutcome, CapabilityDenied,
        CapabilityDeniedReasonKind, CapabilityDescriptorView, CapabilityFailure,
        CapabilityFailureKind, CapabilityInvocation, CapabilityOutcome, CapabilityResultMessage,
        ConcurrencyHint, FinalizeAssistantMessage, HookMilestoneSink, HostManagedLoopModelPort,
        HostManagedLoopPromptPort, InMemoryInstructionMaterializationStore,
        InstructionBundleMaterializedMessage, InstructionMaterializationStore,
        InstructionSafetyContext, LoopCapabilityPort, LoopCheckpointPort, LoopCheckpointRequest,
        LoopContextBundle, LoopContextPort, LoopContextRequest, LoopHostMilestoneEmitter,
        LoopHostMilestoneSink, LoopInputBatch, LoopInputCursor, LoopInputPort,
        LoopModelBudgetAccountant, LoopModelGateway, LoopModelGatewayError,
        LoopModelGatewayRequest, LoopModelPolicyGuard, LoopModelPort, LoopModelRequest,
        LoopModelResponse, LoopProcessRef, LoopProgressEvent, LoopProgressPort, LoopPromptBundle,
        LoopPromptBundleAuthority, LoopPromptBundleRequest, LoopPromptPort, LoopRunContext,
        LoopRunInfoPort, LoopSafeSummary, LoopTranscriptPort, NoOpBudgetAccountant,
        NoOpPolicyGuard, ProcessHandleSummary, RunScopedHookMilestoneSink,
        StageCheckpointPayloadRequest, UpdateAssistantDraft, VisibleCapabilityRequest,
        VisibleCapabilitySurface,
    },
    runner::ClaimedTurnRun,
};
use tokio::sync::Notify;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextOnlyLoopHostConfig {
    pub max_messages: usize,
    pub require_model_route_snapshot: bool,
}

impl Default for TextOnlyLoopHostConfig {
    fn default() -> Self {
        Self {
            max_messages: 16,
            require_model_route_snapshot: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RebornLoopDriverHostError {
    ScopeMismatch { reason: String },
    InvalidRequest { reason: String },
}

impl fmt::Display for RebornLoopDriverHostError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ScopeMismatch { reason } => {
                write!(formatter, "loop driver host scope mismatch: {reason}")
            }
            Self::InvalidRequest { reason } => {
                write!(formatter, "invalid loop driver host request: {reason}")
            }
        }
    }
}

impl Error for RebornLoopDriverHostError {}

#[derive(Debug, Clone)]
pub struct RebornLoopDriverHostRequest {
    pub claimed_run: ClaimedTurnRun,
    pub loop_run_context: LoopRunContext,
}

/// Provider resolver that consults the current visible-capability surface
/// to map `capability_id` → `provider: ExtensionId`. Wires the hook
/// middleware to the same surface the inner port already tracks via
/// `SurfaceTrackingLoopCapabilityPort`. Without this, `OwnCapabilities`-
/// scoped hooks never fire because `ctx.provider` stays `None`
/// (henrypark133 Critical #2).
struct SurfaceBackedProviderResolver {
    surface_state: Arc<CapabilitySurfaceState>,
}

#[async_trait]
impl HookCapabilityProviderResolver for SurfaceBackedProviderResolver {
    async fn provider_for(&self, capability_id: &str) -> Option<ExtensionId> {
        let surface = self.surface_state.current().ok().flatten()?;
        surface
            .descriptors
            .iter()
            .find(|d| d.capability_id.as_str() == capability_id)
            .and_then(|d| d.provider.clone())
    }
}

/// Adapter that lets `HookedLoopPromptPort` write hook-emitted
/// `msg:hook.*` content into the host's `InstructionMaterializationStore`
/// so the downstream model resolver can resolve those refs. Captures the
/// `LoopRunContext` at construction time so the hook prompt port doesn't
/// need to know about run-profile types.
///
/// Threat-model + henrypark133 Critical #1: without this adapter wired
/// through the factory, hook prompt patches produce unresolvable refs and
/// the request fails with `model message reference is unavailable`.
struct InstructionStoreBackedHookSink {
    store: Arc<dyn InstructionMaterializationStore>,
    run_context: LoopRunContext,
}

impl HookPromptMaterializationSink for InstructionStoreBackedHookSink {
    fn put(
        &self,
        role: &str,
        content_ref: &ironclaw_turns::LoopMessageRef,
        safe_content: String,
    ) -> Result<(), AgentLoopHostError> {
        self.store.put_materialized_messages(
            &self.run_context,
            vec![InstructionBundleMaterializedMessage {
                role: role.to_string(),
                content_ref: content_ref.clone(),
                safe_content,
            }],
        )
    }
}

#[derive(Default)]
struct CapabilitySurfaceState {
    current: Mutex<Option<VisibleCapabilitySurface>>,
}

impl CapabilitySurfaceState {
    fn set_current(&self, surface: VisibleCapabilitySurface) -> Result<(), AgentLoopHostError> {
        let mut current = self.current.lock().map_err(|_| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "capability surface state is unavailable",
            )
        })?;
        *current = Some(surface);
        Ok(())
    }

    fn current(&self) -> Result<Option<VisibleCapabilitySurface>, AgentLoopHostError> {
        self.current
            .lock()
            .map(|current| current.clone())
            .map_err(|_| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Unavailable,
                    "capability surface state is unavailable",
                )
            })
    }
}

struct SurfaceTrackingLoopCapabilityPort {
    inner: Arc<dyn LoopCapabilityPort>,
    surface_state: Arc<CapabilitySurfaceState>,
}

impl SurfaceTrackingLoopCapabilityPort {
    fn new(inner: Arc<dyn LoopCapabilityPort>, surface_state: Arc<CapabilitySurfaceState>) -> Self {
        Self {
            inner,
            surface_state,
        }
    }
}

#[async_trait]
impl LoopCapabilityPort for SurfaceTrackingLoopCapabilityPort {
    async fn visible_capabilities(
        &self,
        request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        let surface = self.inner.visible_capabilities(request).await?;
        self.surface_state.set_current(surface.clone())?;
        Ok(surface)
    }

    async fn invoke_capability(
        &self,
        request: CapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        self.inner.invoke_capability(request).await
    }

    async fn invoke_capability_batch(
        &self,
        request: CapabilityBatchInvocation,
    ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
        self.inner.invoke_capability_batch(request).await
    }
}

#[async_trait]
pub trait LoopCapabilityInputResolver: Send + Sync {
    async fn resolve_capability_input(
        &self,
        run_context: &LoopRunContext,
        input_ref: &ironclaw_turns::run_profile::CapabilityInputRef,
    ) -> Result<serde_json::Value, AgentLoopHostError>;
}

/// Default upper bound (in bytes of UTF-8 JSON-serialized form) above which
/// [`HookCapabilityInputResolverAdapter`] refuses to forward resolved input to
/// `before_capability` hook predicates. The hook crate's
/// [`crate::ironclaw_hooks::points::SanitizedArguments`] already caps per-string
/// length and nesting depth, but does not bound total byte size; the adapter
/// rejects oversized payloads up front so an unexpectedly large blob doesn't
/// reach predicate evaluation. The default is intentionally generous (64 KiB)
/// to cover normal capability inputs; callers can tighten it via
/// [`HookCapabilityInputResolverAdapter::with_max_input_bytes`].
pub const DEFAULT_HOOK_CAPABILITY_INPUT_MAX_BYTES: usize = 64 * 1024;

/// Adapter that exposes a [`LoopCapabilityInputResolver`] to the
/// `ironclaw_hooks` middleware as a
/// [`ironclaw_hooks::middleware::CapabilityInputResolver`].
///
/// Production capability dispatch already requires a
/// [`LoopCapabilityInputResolver`] (used by [`HostRuntimeLoopCapabilityPort`]
/// to convert opaque `CapabilityInputRef`s into JSON inputs for the host
/// runtime). This adapter reuses that same resolver — and the same
/// `LoopRunContext` it was built for — to feed sanitized arguments to hook
/// predicate evaluators. Sharing the resolver guarantees that the hook
/// framework and the dispatch path see the same logical input for a given
/// `(run, input_ref)` pair.
///
/// Fail-closed semantics:
///
/// - If the inner resolver returns an error, the adapter returns `None`. The
///   hook framework treats `None` as "unresolved" and `NumericSum`-style
///   predicates fail closed (deny / pause) per the evaluator's existing
///   semantics.
/// - If the resolved JSON value exceeds the configured byte budget once
///   serialized, the adapter returns `None`. The framework's per-string
///   truncation and depth cap (in
///   [`ironclaw_hooks::points::SanitizedArguments`]) apply to predicate
///   evaluation, but the total payload size guard lives here so an oversized
///   body cannot reach the sanitizer at all.
pub struct HookCapabilityInputResolverAdapter {
    inner: Arc<dyn LoopCapabilityInputResolver>,
    run_context: LoopRunContext,
    max_input_bytes: usize,
}

impl HookCapabilityInputResolverAdapter {
    pub fn new(inner: Arc<dyn LoopCapabilityInputResolver>, run_context: LoopRunContext) -> Self {
        Self {
            inner,
            run_context,
            max_input_bytes: DEFAULT_HOOK_CAPABILITY_INPUT_MAX_BYTES,
        }
    }

    /// Override the maximum serialized-byte budget. Inputs whose serialized
    /// JSON exceeds this size resolve to `None` (predicate evaluators that
    /// depend on argument contents fail closed).
    #[must_use]
    pub fn with_max_input_bytes(mut self, max_input_bytes: usize) -> Self {
        self.max_input_bytes = max_input_bytes;
        self
    }
}

#[async_trait]
impl HookCapabilityInputResolver for HookCapabilityInputResolverAdapter {
    async fn resolve(
        &self,
        invocation: &ironclaw_turns::run_profile::CapabilityInvocation,
    ) -> Option<serde_json::Value> {
        let value = match self
            .inner
            .resolve_capability_input(&self.run_context, &invocation.input_ref)
            .await
        {
            Ok(value) => value,
            Err(error) => {
                tracing::debug!(
                    capability = %invocation.capability_id,
                    input_ref = %invocation.input_ref,
                    kind = ?error.kind,
                    safe_summary = %error.safe_summary,
                    "hook capability input resolution failed; treating as unresolved"
                );
                return None;
            }
        };
        let serialized_len = match serde_json::to_vec(&value) {
            Ok(bytes) => bytes.len(),
            Err(error) => {
                tracing::debug!(
                    capability = %invocation.capability_id,
                    input_ref = %invocation.input_ref,
                    error = %error,
                    "hook capability input could not be re-serialized; treating as unresolved"
                );
                return None;
            }
        };
        if serialized_len > self.max_input_bytes {
            tracing::debug!(
                capability = %invocation.capability_id,
                input_ref = %invocation.input_ref,
                serialized_len,
                max_input_bytes = self.max_input_bytes,
                "hook capability input exceeded byte budget; treating as unresolved"
            );
            return None;
        }
        Some(value)
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
struct SurfaceCapabilitySnapshot {
    provider: ExtensionId,
    estimate: ResourceEstimate,
}

#[derive(Clone, Default)]
struct SurfaceSnapshot {
    capabilities: HashMap<CapabilityId, SurfaceCapabilitySnapshot>,
}

const MAX_IN_MEMORY_DISPATCH_RECORDS: usize = 128;

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

pub struct HostRuntimeLoopCapabilityPort {
    runtime: Arc<dyn HostRuntime>,
    run_context: LoopRunContext,
    visible_request: ironclaw_host_runtime::VisibleCapabilityRequest,
    input_resolver: Arc<dyn LoopCapabilityInputResolver>,
    result_writer: Arc<dyn LoopCapabilityResultWriter>,
    milestone_sink: Option<Arc<dyn LoopHostMilestoneSink>>,
    snapshots: Mutex<HashMap<String, SurfaceSnapshot>>,
    dispatch_records: Mutex<DispatchRecordStore>,
}

impl HostRuntimeLoopCapabilityPort {
    pub fn new(
        runtime: Arc<dyn HostRuntime>,
        run_context: LoopRunContext,
        visible_request: ironclaw_host_runtime::VisibleCapabilityRequest,
        input_resolver: Arc<dyn LoopCapabilityInputResolver>,
        result_writer: Arc<dyn LoopCapabilityResultWriter>,
    ) -> Self {
        Self {
            runtime,
            run_context,
            visible_request,
            input_resolver,
            result_writer,
            milestone_sink: None,
            snapshots: Mutex::new(HashMap::new()),
            dispatch_records: Mutex::new(DispatchRecordStore::default()),
        }
    }

    pub fn with_milestone_sink(mut self, sink: Arc<dyn LoopHostMilestoneSink>) -> Self {
        self.milestone_sink = Some(sink);
        self
    }

    fn snapshot_for(
        &self,
        version: &ironclaw_turns::run_profile::CapabilitySurfaceVersion,
    ) -> Result<SurfaceSnapshot, AgentLoopHostError> {
        let snapshots = self.snapshots.lock().map_err(|_| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "capability surface snapshot store is unavailable",
            )
        })?;
        snapshots.get(version.as_str()).cloned().ok_or_else(|| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::StaleSurface,
                "capability surface is stale or unknown",
            )
        })
    }

    fn reserve_dispatch(
        &self,
        key: &IdempotencyKey,
    ) -> Result<DispatchReservation, AgentLoopHostError> {
        self.dispatch_records
            .lock()
            .map_err(|_| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Unavailable,
                    "capability dispatch record store is unavailable",
                )
            })?
            .reserve(key)
    }

    fn dispatch_in_flight_matches(
        &self,
        key: &IdempotencyKey,
        notify: &Arc<Notify>,
    ) -> Result<bool, AgentLoopHostError> {
        self.dispatch_records
            .lock()
            .map(|records| records.in_flight_matches(key, notify))
            .map_err(|_| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Unavailable,
                    "capability dispatch record store is unavailable",
                )
            })
    }

    fn record_runtime_completed(
        &self,
        key: &IdempotencyKey,
        requested_capability_id: CapabilityId,
        outcome: RuntimeCapabilityOutcome,
    ) -> Result<(), AgentLoopHostError> {
        let notify = self
            .dispatch_records
            .lock()
            .map_err(|_| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Unavailable,
                    "capability dispatch record store is unavailable",
                )
            })?
            .record(
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
        let notify = self
            .dispatch_records
            .lock()
            .map_err(|_| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Unavailable,
                    "capability dispatch record store is unavailable",
                )
            })?
            .record(key, DispatchRecord::LoopCompleted(result));
        if let Some(notify) = notify {
            notify.notify_waiters();
        }
        Ok(())
    }

    fn clear_dispatch(&self, key: &IdempotencyKey) -> Result<(), AgentLoopHostError> {
        let notify = self
            .dispatch_records
            .lock()
            .map_err(|_| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Unavailable,
                    "capability dispatch record store is unavailable",
                )
            })?
            .remove(key);
        if let Some(notify) = notify {
            notify.notify_waiters();
        }
        Ok(())
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
        if let Some(milestone_sink) = &self.milestone_sink {
            let milestones =
                LoopHostMilestoneEmitter::new(self.run_context.clone(), Arc::clone(milestone_sink));
            milestones.capability_invoked(capability_id).await?;
        }
        Ok(())
    }
}

#[async_trait]
impl LoopCapabilityPort for HostRuntimeLoopCapabilityPort {
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
                snapshot.capabilities.insert(
                    capability_id.clone(),
                    SurfaceCapabilitySnapshot {
                        provider: capability.descriptor.provider.clone(),
                        estimate: capability.estimated_resources.clone(),
                    },
                );
                CapabilityDescriptorView {
                    capability_id,
                    provider: Some(capability.descriptor.provider),
                    runtime: capability.descriptor.runtime,
                    safe_name: capability.descriptor.id.as_str().to_string(),
                    safe_description: capability.descriptor.description,
                    // WS-9 derives this from the underlying
                    // `CapabilityDescriptor.effects` Vec. Until that landing,
                    // default to `Exclusive` (the conservative choice — forces
                    // serial invocation and never accidentally enables
                    // parallel side effects).
                    concurrency_hint: ConcurrencyHint::Exclusive,
                }
            })
            .collect();

        let mut snapshots = self.snapshots.lock().map_err(|_| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "capability surface snapshot store is unavailable",
            )
        })?;
        snapshots.clear();
        snapshots.insert(version.as_str().to_string(), snapshot);

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
        let input = match self
            .input_resolver
            .resolve_capability_input(&self.run_context, &request.input_ref)
            .await
        {
            Ok(input) => input,
            Err(error) => {
                if let Err(clear_error) = self.clear_dispatch(&idempotency_key) {
                    tracing::warn!(
                        clear_error = %clear_error,
                        "failed to clear capability dispatch record after input resolution error"
                    );
                    return Err(clear_error);
                }
                return Err(error);
            }
        };
        let requested_capability_id = request.capability_id.clone();

        if let Err(error) = self
            .emit_capability_invoked(request.capability_id.clone())
            .await
        {
            if let Err(clear_error) = self.clear_dispatch(&idempotency_key) {
                tracing::warn!(
                    clear_error = %clear_error,
                    "failed to clear capability dispatch record after milestone emission error"
                );
                return Err(clear_error);
            }
            return Err(error);
        }
        let outcome = match self
            .runtime
            .invoke_capability(
                RuntimeCapabilityRequest::new(
                    invocation_context_from_visible(&self.visible_request.context),
                    request.capability_id,
                    capability.estimate,
                    input,
                    trust_decision,
                )
                .with_idempotency_key(idempotency_key.clone()),
            )
            .await
        {
            Ok(outcome) => outcome,
            Err(error) => {
                if let Err(clear_error) = self.clear_dispatch(&idempotency_key) {
                    tracing::warn!(
                        clear_error = %clear_error,
                        "failed to clear capability dispatch record after host runtime error"
                    );
                    return Err(clear_error);
                }
                return Err(host_runtime_error(error));
            }
        };
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

fn invocation_context_from_visible(base: &ExecutionContext) -> ExecutionContext {
    // The visible-capability request is the host-vetted authority envelope for
    // this loop run. Each dispatch gets fresh invocation/correlation ids and no
    // inherited process parentage, while identity, trust, grants, mounts, and
    // run scope remain bound to the visible surface that the model cited.
    let mut context = base.clone();
    let invocation_id = InvocationId::new();
    context.invocation_id = invocation_id;
    context.correlation_id = CorrelationId::new();
    context.process_id = None;
    context.parent_process_id = None;
    context.resource_scope.invocation_id = invocation_id;
    context
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
                    error_kind: runtime_failure_kind_to_loop(failure.kind),
                    safe_summary: runtime_safe_summary(
                        failure.message,
                        "capability invocation failed",
                    ),
                })
            }
        }
        RuntimeCapabilityOutcome::Unknown(unknown) => {
            CapabilityOutcome::Failed(CapabilityFailure {
                error_kind: CapabilityFailureKind::unknown(unknown.kind).map_err(|_| {
                    AgentLoopHostError::new(
                        AgentLoopHostErrorKind::Internal,
                        "unknown capability outcome kind could not be represented",
                    )
                })?,
                safe_summary: runtime_safe_summary(
                    unknown.message,
                    "capability invocation returned an unknown outcome",
                ),
            })
        }
    })
}

fn runtime_failure_kind_to_loop(kind: RuntimeFailureKind) -> CapabilityFailureKind {
    match kind {
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
        RuntimeFailureKind::Unknown => CapabilityFailureKind::Permanent,
        _ => CapabilityFailureKind::Permanent,
    }
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

/// Factory closure that produces an `Arc<HookDispatcher>` for each host
/// build. The closure is invoked once per `build_text_only_host*` call.
///
/// To get full per-run isolation of dispatcher-owned mutable state (poisoned
/// slots, in-process registry edits, timeout overrides), the closure should
/// construct a **fresh** `HookDispatcher` on every call (e.g.
/// `Arc::new(build_my_dispatcher())`). To opt into the legacy shared-state
/// behavior, return clones of the same `Arc<HookDispatcher>`.
///
/// `Fn` (not `FnOnce`) — invoked once per build, potentially many times over
/// the factory's lifetime. `Send + Sync + 'static` so the factory can be held
/// across `.await` points and shared across tokio tasks.
pub type HookDispatcherFactory = Arc<dyn Fn() -> Arc<HookDispatcher> + Send + Sync + 'static>;

/// Per-build hook dispatcher *builder* factory. Unlike
/// [`HookDispatcherFactory`] which returns a finalized `Arc<HookDispatcher>`
/// and forces the caller to wire telemetry sinks themselves (a doc-only
/// contract that henrypark133 Critical #4 flagged as silently losing
/// telemetry when callers forgot), this variant returns a builder so the
/// host factory can attach a `RunScopedHookMilestoneSink` keyed to the
/// current `LoopRunContext` before sealing — telemetry is then
/// run-scope-correct without caller bookkeeping.
pub type HookDispatcherBuilderFactory =
    Arc<dyn Fn() -> HookDispatcherBuilder + Send + Sync + 'static>;

pub struct RebornLoopDriverHostFactory<S, G>
where
    S: SessionThreadService + ?Sized,
    G: HostManagedModelGateway + ?Sized,
{
    thread_service: Arc<S>,
    thread_scope: ThreadScope,
    model_gateway: Arc<G>,
    model_route_resolver: Option<Arc<dyn ModelRouteResolver>>,
    checkpoint_state_store: Arc<dyn CheckpointStateStore>,
    loop_checkpoint_store: Arc<dyn LoopCheckpointStore>,
    milestone_sink: Arc<dyn LoopHostMilestoneSink>,
    model_accountant: Arc<dyn LoopModelBudgetAccountant>,
    model_policy_guard: Arc<dyn LoopModelPolicyGuard>,
    config: TextOnlyLoopHostConfig,
    skill_context_source: Option<Arc<dyn HostSkillContextSource>>,
    /// Optional hook dispatcher factory. When set, the factory invokes the
    /// closure on every `build_text_only_host*` call to obtain a fresh
    /// `HookDispatcher`, wraps it in `Arc`, and then plumbs it through
    /// `HookedLoopCapabilityPort` / `HookedLoopPromptPort`. Building a fresh
    /// dispatcher per host build means slot-poisoning state, the per-tenant
    /// predicate counter, and any registry mutations done while a run is
    /// active do not leak into the next run. Default behavior (no factory) is
    /// unchanged from the pre-hooks shape.
    hook_dispatcher_factory: Option<HookDispatcherFactory>,
    /// Per-build builder factory. Preferred over `hook_dispatcher_factory`
    /// because it lets the host factory attach the run-scoped milestone
    /// sink internally (henrypark133 Critical #4). Exactly one of these
    /// two should be set; if both are, the builder factory wins.
    hook_dispatcher_builder_factory: Option<HookDispatcherBuilderFactory>,
    /// Optional capability-input resolver. When the hook dispatcher is set
    /// and a resolver is configured, the factory wraps it in a
    /// [`HookCapabilityInputResolverAdapter`] (bound to the current
    /// `LoopRunContext`) and threads it into `HookedLoopCapabilityPort` so
    /// argument-dependent predicates (e.g., `NumericSum`) evaluate against
    /// real capability arguments instead of failing closed.
    capability_input_resolver: Option<Arc<dyn LoopCapabilityInputResolver>>,
    /// Optional gate-ref factory for hook `PauseApproval` / `PauseAuth`
    /// decisions. Default behavior (no factory) is fail-closed — the hook
    /// suspension surfaces as `Denied`. Production deployments must install
    /// a factory that talks to the host's approval/auth router; tests can
    /// install `UuidHookGateRefFactory` to exercise the affirmative path.
    /// See [`Self::with_hook_gate_ref_factory`].
    hook_gate_ref_factory: Option<Arc<dyn ironclaw_hooks::middleware::HookGateRefFactory>>,
    safety_context: Option<InstructionSafetyContext>,
}

impl<S, G> RebornLoopDriverHostFactory<S, G>
where
    S: SessionThreadService + ?Sized + Send + Sync + 'static,
    G: HostManagedModelGateway + ?Sized + Send + Sync + 'static,
{
    pub fn new(
        thread_service: Arc<S>,
        thread_scope: ThreadScope,
        model_gateway: Arc<G>,
        checkpoint_state_store: Arc<dyn CheckpointStateStore>,
        loop_checkpoint_store: Arc<dyn LoopCheckpointStore>,
        milestone_sink: Arc<dyn LoopHostMilestoneSink>,
        config: TextOnlyLoopHostConfig,
    ) -> Self {
        Self {
            thread_service,
            thread_scope,
            model_gateway,
            model_route_resolver: None,
            checkpoint_state_store,
            loop_checkpoint_store,
            milestone_sink,
            model_accountant: Arc::new(NoOpBudgetAccountant),
            model_policy_guard: Arc::new(NoOpPolicyGuard),
            config,
            skill_context_source: None,
            hook_dispatcher_factory: None,
            hook_dispatcher_builder_factory: None,
            capability_input_resolver: None,
            hook_gate_ref_factory: None,
            safety_context: None,
        }
    }

    pub fn with_skill_context_source(mut self, source: Arc<dyn HostSkillContextSource>) -> Self {
        self.skill_context_source = Some(source);
        self
    }

    /// Install a hook dispatcher factory closure. The closure is invoked once
    /// on every `build_text_only_host*` call to mint a fresh
    /// [`HookDispatcher`], which the factory then wraps in `Arc` and threads
    /// through `HookedLoopCapabilityPort` / `HookedLoopPromptPort`.
    ///
    /// This is the recommended hook installation path: per-build construction
    /// gives each host its own dispatcher, so slot poisoning, registry
    /// mutations, and any other dispatcher-owned state are scoped to a single
    /// run rather than shared across every host the factory ever produces.
    ///
    /// **Hook telemetry**: to surface hook dispatch in the host's milestone
    /// stream, the closure itself should attach a
    /// [`ironclaw_turns::run_profile::HookMilestoneSink`] (typically a
    /// [`ironclaw_turns::run_profile::RunScopedHookMilestoneSink`] wrapping
    /// the factory's `LoopHostMilestoneSink`) before returning the
    /// dispatcher. The wrapping happens inside the closure so each run gets a
    /// dispatcher already configured for telemetry. Hook activity is
    /// invisible to observers when no sink is attached.
    pub fn with_hook_dispatcher_factory<F>(mut self, factory: F) -> Self
    where
        F: Fn() -> Arc<HookDispatcher> + Send + Sync + 'static,
    {
        self.hook_dispatcher_factory = Some(Arc::new(factory));
        self
    }

    /// Install a capability-input resolver for hook predicate evaluation.
    /// When set alongside a hook dispatcher, hook predicates that depend on
    /// argument contents (`ValueOrRateBound::NumericSum`, etc.) see real,
    /// sanitized input values; otherwise they fail closed because the hooks
    /// middleware defaults to the framework's `NullCapabilityInputResolver`.
    ///
    /// The resolver is the same trait used by [`HostRuntimeLoopCapabilityPort`]
    /// to convert opaque capability input refs into JSON arguments; production
    /// callers typically share a single implementation between dispatch and
    /// hook evaluation so both observe the same logical input.
    /// Install a hook dispatcher *builder* factory. **Preferred over
    /// [`Self::with_hook_dispatcher_factory`]** because the host factory
    /// can attach a `RunScopedHookMilestoneSink` keyed to the current
    /// `LoopRunContext` *internally*, before the builder is sealed —
    /// guaranteeing hook telemetry carries the right run/thread scope
    /// without caller bookkeeping (henrypark133 Critical #4).
    ///
    /// The closure is invoked once per `build_text_only_host*` call. It
    /// should construct a clean builder (no pre-attached milestone sink —
    /// the host wires one). Manifest-driven hook installations happen
    /// inside the closure exactly as with the legacy factory.
    pub fn with_hook_dispatcher_builder_factory<F>(mut self, factory: F) -> Self
    where
        F: Fn() -> HookDispatcherBuilder + Send + Sync + 'static,
    {
        self.hook_dispatcher_builder_factory = Some(Arc::new(factory));
        self
    }

    pub fn with_capability_input_resolver(
        mut self,
        resolver: Arc<dyn LoopCapabilityInputResolver>,
    ) -> Self {
        self.capability_input_resolver = Some(resolver);
        self
    }

    /// Install a `HookGateRefFactory` for hook-emitted `PauseApproval` /
    /// `PauseAuth` decisions. The default (no factory) is fail-closed: the
    /// suspension surfaces as `Denied` so the loop doesn't park on an
    /// unresolvable ref.
    ///
    /// Production: install a factory that reserves a gate through the
    /// host's real approval/auth router so the ref carries lease + one-shot
    /// semantics. Tests/dev: install `UuidHookGateRefFactory` to exercise
    /// the affirmative `ApprovalRequired { gate_ref }` shape (the refs are
    /// locally unique but not router-registered — production must not use
    /// this).
    pub fn with_hook_gate_ref_factory(
        mut self,
        factory: Arc<dyn ironclaw_hooks::middleware::HookGateRefFactory>,
    ) -> Self {
        self.hook_gate_ref_factory = Some(factory);
        self
    }

    /// Install a shared [`HookDispatcher`] that wraps the capability and
    /// prompt ports for every host built by this factory.
    ///
    /// **Deprecated for production use.** This is preserved as a thin
    /// backward-compat wrapper that adapts a single `Arc<HookDispatcher>`
    /// into a factory closure cloning the same instance on every build. As a
    /// result, dispatcher-owned mutable state (poisoned slots, predicate
    /// counters, registry mutations) is **shared across every run** the
    /// factory produces — a hook poisoned in run N stays poisoned for runs
    /// N+1, N+2, …
    ///
    /// New callers should prefer [`Self::with_hook_dispatcher_factory`],
    /// which mints a fresh dispatcher per host build and provides full
    /// per-run isolation of hook state.
    pub fn with_hook_dispatcher(self, dispatcher: Arc<HookDispatcher>) -> Self {
        // Single-instance Arc cloning preserves the legacy shared-state shape
        // so existing call sites and tests behave identically. New code paths
        // should reach for `with_hook_dispatcher_factory` instead.
        self.with_hook_dispatcher_factory(move || Arc::clone(&dispatcher))
    }

    /// **DEPRECATED.** Earlier docs claimed this would defer `.build_arc()`
    /// until the host factory finalizes wiring, but the implementation
    /// always called it eagerly and routed through the legacy shared-
    /// dispatcher adapter (serrrfirat P2 #3 on PR #3573). Callers using
    /// this path lost per-run dispatcher isolation and the run-scoped
    /// milestone sink. Use [`Self::with_hook_dispatcher_builder_factory`]
    /// — pass a closure that builds a fresh builder per host — for the
    /// behavior this method's name implied, or
    /// [`Self::with_hook_dispatcher`] if you actually want the shared
    /// dispatcher.
    #[deprecated(
        since = "0.1.0",
        note = "this method always called build_arc() eagerly and routed \
                through the shared-dispatcher adapter, contradicting the \
                doc-claimed deferred semantics. Use \
                `with_hook_dispatcher_builder_factory(|| builder())` for \
                per-build isolation, or `with_hook_dispatcher(...)` if you \
                meant the shared adapter explicitly."
    )]
    pub fn with_hook_dispatcher_builder(self, builder: HookDispatcherBuilder) -> Self {
        self.with_hook_dispatcher(builder.build_arc())
    }

    pub fn with_safety_context(mut self, safety_context: InstructionSafetyContext) -> Self {
        self.safety_context = Some(safety_context);
        self
    }

    pub fn with_model_route_resolver<R>(mut self, resolver: Arc<R>) -> Self
    where
        R: ModelRouteResolver + 'static,
    {
        let resolver: Arc<dyn ModelRouteResolver> = resolver;
        self.model_route_resolver = Some(resolver);
        self
    }

    pub fn with_model_budget_accountant(
        mut self,
        accountant: Arc<dyn LoopModelBudgetAccountant>,
    ) -> Self {
        self.model_accountant = accountant;
        self
    }

    pub fn with_model_policy_guard(mut self, policy_guard: Arc<dyn LoopModelPolicyGuard>) -> Self {
        self.model_policy_guard = policy_guard;
        self
    }

    pub async fn build_text_only_host(
        &self,
        request: RebornLoopDriverHostRequest,
    ) -> Result<RebornLoopDriverHost, RebornLoopDriverHostError> {
        self.build_text_only_host_with_capabilities(request, Arc::new(EmptyLoopCapabilityPort))
            .await
    }

    pub async fn build_text_only_host_with_capabilities(
        &self,
        request: RebornLoopDriverHostRequest,
        capabilities: Arc<dyn LoopCapabilityPort>,
    ) -> Result<RebornLoopDriverHost, RebornLoopDriverHostError> {
        validate_claimed_run_context(&request.claimed_run, &request.loop_run_context)?;
        validate_thread_scope(&self.thread_scope, &request.loop_run_context)?;

        let max_messages = self.config.max_messages.max(1);
        let run_context = self.attach_model_route_snapshot(request.loop_run_context)?;
        let mut context_adapter = ThreadBackedLoopContextPort::new(
            Arc::clone(&self.thread_service),
            self.thread_scope.clone(),
            run_context.clone(),
            max_messages,
        );
        if let Some(source) = self.skill_context_source.as_ref() {
            context_adapter = context_adapter.with_skill_context_source(source.clone());
        }
        let context: Arc<dyn LoopContextPort> = Arc::new(context_adapter);
        // Mint a fresh dispatcher per build when a factory is installed. This
        // localizes dispatcher-owned state (slot poisoning, registry edits,
        // predicate counters) to this one host so it cannot leak into the
        // next run that shares this factory.
        //
        // Builder-factory path (preferred): the factory hands back a
        // mutable builder; we attach a `RunScopedHookMilestoneSink` keyed
        // to *this* run's `LoopRunContext` *before* sealing. Hook
        // telemetry then carries the correct run/thread scope without
        // depending on the closure capturing the right context (which
        // would silently misattribute across reuses — henrypark133
        // Critical #4).
        let per_build_dispatcher = match (
            self.hook_dispatcher_builder_factory.as_ref(),
            self.hook_dispatcher_factory.as_ref(),
        ) {
            (Some(builder_factory), _) => {
                let builder = builder_factory();
                let run_scoped: Arc<dyn HookMilestoneSink> =
                    Arc::new(RunScopedHookMilestoneSink::new(
                        run_context.clone(),
                        Arc::clone(&self.milestone_sink) as _,
                    ));
                Some(builder.with_milestone_sink(run_scoped).build_arc())
            }
            (None, Some(factory)) => Some(factory()),
            (None, None) => None,
        };
        let instruction_materialization_store: Arc<dyn InstructionMaterializationStore> =
            Arc::new(InMemoryInstructionMaterializationStore::default());
        let surface_state = Arc::new(CapabilitySurfaceState::default());
        let mut capabilities: Arc<dyn LoopCapabilityPort> = Arc::new(
            SurfaceTrackingLoopCapabilityPort::new(capabilities, Arc::clone(&surface_state)),
        );
        if let Some(dispatcher) = per_build_dispatcher.as_ref() {
            // Wire a surface-backed provider resolver so OwnCapabilities-
            // scoped hooks can see `ctx.provider` (henrypark133 Critical #2).
            // Without this, the middleware keeps NullCapabilityProviderResolver
            // and every OwnCapabilities hook is inert because provider stays
            // None.
            let provider_resolver: Arc<dyn HookCapabilityProviderResolver> =
                Arc::new(SurfaceBackedProviderResolver {
                    surface_state: Arc::clone(&surface_state),
                });
            let mut hooked = HookedLoopCapabilityPort::new(
                Arc::clone(&capabilities),
                Arc::clone(dispatcher),
                run_context.scope.tenant_id.clone(),
            )
            .with_provider_resolver(provider_resolver);
            if let Some(factory) = self.hook_gate_ref_factory.as_ref() {
                hooked = hooked.with_gate_ref_factory(Arc::clone(factory));
            }
            if let Some(input_resolver) = self.capability_input_resolver.as_ref() {
                let adapter: Arc<dyn HookCapabilityInputResolver> =
                    Arc::new(HookCapabilityInputResolverAdapter::new(
                        Arc::clone(input_resolver),
                        run_context.clone(),
                    ));
                hooked = hooked.with_resolver(adapter);
            }
            capabilities = Arc::new(hooked);
        }
        capabilities
            .visible_capabilities(VisibleCapabilityRequest)
            .await
            .map_err(|error| RebornLoopDriverHostError::InvalidRequest {
                reason: error.safe_summary,
            })?;
        let prompt_authority = LoopPromptBundleAuthority::shared();
        let surface_state_for_prompt = Arc::clone(&surface_state);
        let mut prompt_port = HostManagedLoopPromptPort::new(
            run_context.clone(),
            Arc::clone(&context),
            Arc::clone(&self.milestone_sink),
        )
        .with_prompt_bundle_authority(prompt_authority.clone())
        .with_default_message_limit(max_messages)
        .with_current_surface_lookup(move || surface_state_for_prompt.current())
        .with_instruction_materialization_store(Arc::clone(&instruction_materialization_store));
        if let Some(safety_context) = self.safety_context.clone() {
            prompt_port = prompt_port.with_safety_context(safety_context);
        }
        let mut prompt: Arc<dyn LoopPromptPort> = Arc::new(prompt_port);
        if let Some(dispatcher) = per_build_dispatcher.as_ref() {
            // Pass a sink backed by the host's instruction materialization
            // store so hook-emitted `msg:hook.*` refs are resolvable by the
            // downstream model resolver. Without this the resolver fails
            // the request with `model message reference is unavailable`
            // (henrypark133 review Critical #1).
            let sink: Arc<dyn HookPromptMaterializationSink> =
                Arc::new(InstructionStoreBackedHookSink {
                    store: Arc::clone(&instruction_materialization_store),
                    run_context: run_context.clone(),
                });
            prompt = Arc::new(
                HookedLoopPromptPort::new(
                    Arc::clone(&prompt),
                    Arc::clone(dispatcher),
                    run_context.scope.tenant_id.clone(),
                )
                .with_materialization_sink(sink),
            );
        }
        let input: Arc<dyn LoopInputPort> =
            Arc::new(NoExtraLoopInputPort::new(run_context.clone()));
        let model_gateway = Arc::new(ThreadResolvingLoopModelGateway::new(
            Arc::clone(&self.thread_service),
            self.thread_scope.clone(),
            Arc::clone(&self.model_gateway),
            max_messages,
            self.skill_context_source.clone(),
            Some(Arc::clone(&instruction_materialization_store)),
            prompt_authority,
        ));
        let mut model: Arc<dyn LoopModelPort> = Arc::new(HostManagedLoopModelPort::with_guards(
            run_context.clone(),
            model_gateway,
            Arc::clone(&self.milestone_sink),
            Arc::clone(&self.model_accountant),
            Arc::clone(&self.model_policy_guard),
        ));
        let mut checkpoint: Arc<dyn LoopCheckpointPort> =
            Arc::new(HostManagedLoopCheckpointPort::new(
                run_context.clone(),
                Arc::clone(&self.checkpoint_state_store),
                Arc::clone(&self.loop_checkpoint_store),
                Arc::clone(&self.milestone_sink),
            ));
        let mut transcript: Arc<dyn LoopTranscriptPort> =
            Arc::new(ThreadBackedLoopTranscriptPort::with_milestone_sink(
                Arc::clone(&self.thread_service),
                self.thread_scope.clone(),
                run_context.clone(),
                Arc::clone(&self.milestone_sink),
            ));
        if let Some(dispatcher) = per_build_dispatcher.as_ref() {
            model = Arc::new(HookedLoopModelPort::new(
                Arc::clone(&model),
                Arc::clone(dispatcher),
                run_context.scope.tenant_id.clone(),
            ));
            transcript = Arc::new(HookedLoopTranscriptPort::new(
                Arc::clone(&transcript),
                Arc::clone(dispatcher),
                run_context.scope.tenant_id.clone(),
            ));
            checkpoint = Arc::new(HookedLoopCheckpointPort::new(
                Arc::clone(&checkpoint),
                Arc::clone(dispatcher),
                run_context.scope.tenant_id.clone(),
            ));
        }
        let progress: Arc<dyn LoopProgressPort> = Arc::new(HostManagedLoopProgressPort::new(
            run_context.clone(),
            Arc::clone(&self.milestone_sink),
        ));

        Ok(RebornLoopDriverHost {
            run_context,
            context,
            prompt,
            input,
            model,
            checkpoint,
            capabilities,
            transcript,
            progress,
        })
    }

    fn attach_model_route_snapshot(
        &self,
        run_context: LoopRunContext,
    ) -> Result<LoopRunContext, RebornLoopDriverHostError> {
        if let Some(snapshot) = &run_context.resolved_model_route {
            snapshot
                .validate()
                .map_err(|reason| RebornLoopDriverHostError::InvalidRequest { reason })?;
            let Some(resolver) = &self.model_route_resolver else {
                return Err(RebornLoopDriverHostError::InvalidRequest {
                    reason: "model route resolver is required for this host".to_string(),
                });
            };
            let slot = slot_for_model_profile(&run_context)?;
            let route = crate::model_routes::ModelRoute::new(
                snapshot.provider_id.clone(),
                snapshot.model_id.clone(),
            )
            .map_err(model_route_error_to_host_error)?;
            resolver
                .validate_model_route(slot, &route)
                .map_err(model_route_error_to_host_error)?;
            return Ok(run_context);
        }
        let Some(resolver) = &self.model_route_resolver else {
            if self.config.require_model_route_snapshot {
                return Err(RebornLoopDriverHostError::InvalidRequest {
                    reason: "model route resolver is required for this host".to_string(),
                });
            }
            return Ok(run_context);
        };
        let slot = slot_for_model_profile(&run_context)?;
        let snapshot = resolver
            .resolve_model_route(slot)
            .map_err(model_route_error_to_host_error)?;
        Ok(run_context.with_resolved_model_route(snapshot.to_loop_model_route_snapshot()))
    }
}

struct ThreadResolvingLoopModelGateway<S, G>
where
    S: SessionThreadService + ?Sized,
    G: HostManagedModelGateway + ?Sized,
{
    thread_service: Arc<S>,
    thread_scope: ThreadScope,
    host_gateway: Arc<G>,
    max_messages: usize,
    skill_context_source: Option<Arc<dyn HostSkillContextSource>>,
    instruction_materialization_store: Option<Arc<dyn InstructionMaterializationStore>>,
    prompt_authority: LoopPromptBundleAuthority,
}

impl<S, G> ThreadResolvingLoopModelGateway<S, G>
where
    S: SessionThreadService + ?Sized,
    G: HostManagedModelGateway + ?Sized,
{
    fn new(
        thread_service: Arc<S>,
        thread_scope: ThreadScope,
        host_gateway: Arc<G>,
        max_messages: usize,
        skill_context_source: Option<Arc<dyn HostSkillContextSource>>,
        instruction_materialization_store: Option<Arc<dyn InstructionMaterializationStore>>,
        prompt_authority: LoopPromptBundleAuthority,
    ) -> Self {
        Self {
            thread_service,
            thread_scope,
            host_gateway,
            max_messages,
            skill_context_source,
            instruction_materialization_store,
            prompt_authority,
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
        if let Some(store) = self.instruction_materialization_store.as_ref() {
            model_port = model_port.with_instruction_materialization_store(Arc::clone(store));
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

pub struct RebornLoopDriverHost {
    run_context: LoopRunContext,
    context: Arc<dyn LoopContextPort>,
    prompt: Arc<dyn LoopPromptPort>,
    input: Arc<dyn LoopInputPort>,
    model: Arc<dyn LoopModelPort>,
    checkpoint: Arc<dyn LoopCheckpointPort>,
    capabilities: Arc<dyn LoopCapabilityPort>,
    transcript: Arc<dyn LoopTranscriptPort>,
    progress: Arc<dyn LoopProgressPort>,
}

impl fmt::Debug for RebornLoopDriverHost {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RebornLoopDriverHost")
            .field("scope", &self.run_context.scope)
            .field("turn_id", &self.run_context.turn_id)
            .field("run_id", &self.run_context.run_id)
            .field("loop_driver_id", &self.run_context.loop_driver_id)
            .finish()
    }
}

impl LoopRunInfoPort for RebornLoopDriverHost {
    fn run_context(&self) -> &LoopRunContext {
        &self.run_context
    }
}

#[async_trait]
impl LoopContextPort for RebornLoopDriverHost {
    async fn load_loop_context(
        &self,
        request: LoopContextRequest,
    ) -> Result<LoopContextBundle, AgentLoopHostError> {
        self.context.load_loop_context(request).await
    }
}

#[async_trait]
impl LoopPromptPort for RebornLoopDriverHost {
    async fn build_prompt_bundle(
        &self,
        request: LoopPromptBundleRequest,
    ) -> Result<LoopPromptBundle, AgentLoopHostError> {
        self.prompt.build_prompt_bundle(request).await
    }
}

#[async_trait]
impl LoopInputPort for RebornLoopDriverHost {
    async fn poll_inputs(
        &self,
        after: LoopInputCursor,
        limit: usize,
    ) -> Result<LoopInputBatch, AgentLoopHostError> {
        self.input.poll_inputs(after, limit).await
    }

    async fn ack_inputs(&self, cursor: LoopInputCursor) -> Result<(), AgentLoopHostError> {
        self.input.ack_inputs(cursor).await
    }
}

#[async_trait]
impl LoopModelPort for RebornLoopDriverHost {
    async fn stream_model(
        &self,
        request: LoopModelRequest,
    ) -> Result<LoopModelResponse, AgentLoopHostError> {
        self.model.stream_model(request).await
    }
}

#[async_trait]
impl LoopCapabilityPort for RebornLoopDriverHost {
    async fn visible_capabilities(
        &self,
        request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        self.capabilities.visible_capabilities(request).await
    }

    async fn invoke_capability(
        &self,
        request: CapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        self.capabilities.invoke_capability(request).await
    }

    async fn invoke_capability_batch(
        &self,
        request: CapabilityBatchInvocation,
    ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
        self.capabilities.invoke_capability_batch(request).await
    }
}

#[async_trait]
impl LoopTranscriptPort for RebornLoopDriverHost {
    async fn begin_assistant_draft(
        &self,
        request: BeginAssistantDraft,
    ) -> Result<ironclaw_turns::LoopMessageRef, AgentLoopHostError> {
        self.transcript.begin_assistant_draft(request).await
    }

    async fn update_assistant_draft(
        &self,
        request: UpdateAssistantDraft,
    ) -> Result<(), AgentLoopHostError> {
        self.transcript.update_assistant_draft(request).await
    }

    async fn finalize_assistant_message(
        &self,
        request: FinalizeAssistantMessage,
    ) -> Result<ironclaw_turns::LoopMessageRef, AgentLoopHostError> {
        self.transcript.finalize_assistant_message(request).await
    }

    async fn append_capability_result_ref(
        &self,
        request: AppendCapabilityResultRef,
    ) -> Result<ironclaw_turns::LoopMessageRef, AgentLoopHostError> {
        self.transcript.append_capability_result_ref(request).await
    }
}

#[async_trait]
impl LoopCheckpointPort for RebornLoopDriverHost {
    async fn checkpoint(
        &self,
        request: LoopCheckpointRequest,
    ) -> Result<TurnCheckpointId, AgentLoopHostError> {
        self.checkpoint.checkpoint(request).await
    }

    async fn stage_checkpoint_payload(
        &self,
        request: StageCheckpointPayloadRequest,
    ) -> Result<LoopCheckpointStateRef, AgentLoopHostError> {
        self.checkpoint.stage_checkpoint_payload(request).await
    }
}

#[async_trait]
impl LoopProgressPort for RebornLoopDriverHost {
    async fn emit_loop_progress(&self, event: LoopProgressEvent) -> Result<(), AgentLoopHostError> {
        self.progress.emit_loop_progress(event).await
    }
}

#[derive(Clone)]
pub struct NoExtraLoopInputPort {
    run_context: LoopRunContext,
}

impl NoExtraLoopInputPort {
    pub fn new(run_context: LoopRunContext) -> Self {
        Self { run_context }
    }

    fn validate_cursor(&self, cursor: &LoopInputCursor) -> Result<(), AgentLoopHostError> {
        if cursor.is_for_run(&self.run_context) {
            Ok(())
        } else {
            Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::ScopeMismatch,
                "input cursor is not scoped to this loop run",
            ))
        }
    }
}

impl LoopRunInfoPort for NoExtraLoopInputPort {
    fn run_context(&self) -> &LoopRunContext {
        &self.run_context
    }
}

#[async_trait]
impl LoopInputPort for NoExtraLoopInputPort {
    async fn poll_inputs(
        &self,
        after: LoopInputCursor,
        _limit: usize,
    ) -> Result<LoopInputBatch, AgentLoopHostError> {
        self.validate_cursor(&after)?;
        Ok(LoopInputBatch {
            inputs: Vec::new(),
            next_cursor: after,
        })
    }

    async fn ack_inputs(&self, cursor: LoopInputCursor) -> Result<(), AgentLoopHostError> {
        self.validate_cursor(&cursor)
    }
}

#[derive(Clone)]
pub struct HostManagedLoopCheckpointPort {
    run_context: LoopRunContext,
    checkpoint_state_store: Arc<dyn CheckpointStateStore>,
    loop_checkpoint_store: Arc<dyn LoopCheckpointStore>,
    milestone_sink: Arc<dyn LoopHostMilestoneSink>,
}

impl HostManagedLoopCheckpointPort {
    pub fn new(
        run_context: LoopRunContext,
        checkpoint_state_store: Arc<dyn CheckpointStateStore>,
        loop_checkpoint_store: Arc<dyn LoopCheckpointStore>,
        milestone_sink: Arc<dyn LoopHostMilestoneSink>,
    ) -> Self {
        Self {
            run_context,
            checkpoint_state_store,
            loop_checkpoint_store,
            milestone_sink,
        }
    }
}

impl LoopRunInfoPort for HostManagedLoopCheckpointPort {
    fn run_context(&self) -> &LoopRunContext {
        &self.run_context
    }
}

#[async_trait]
impl LoopCheckpointPort for HostManagedLoopCheckpointPort {
    async fn checkpoint(
        &self,
        request: LoopCheckpointRequest,
    ) -> Result<TurnCheckpointId, AgentLoopHostError> {
        // `stage_checkpoint_payload` returns a run-scoped ref of the form
        // `checkpoint:{run_id}:{token}`. The underlying store indexed the payload
        // under the original `checkpoint:{token}` key (which `new_state_ref()`
        // generated). Unwrap to the store key so the look-up succeeds, then pass
        // the caller-supplied (run-scoped) ref through to the loop-checkpoint
        // record so `is_for_run` validators see the correct form.
        let run_scoped_prefix = format!("checkpoint:{}:", self.run_context.run_id);
        let store_ref = if let Some(token) =
            request.state_ref.as_str().strip_prefix(&run_scoped_prefix)
        {
            // Run-scoped ref → rebuild the store's original `checkpoint:{token}`.
            LoopCheckpointStateRef::new(format!("checkpoint:{token}")).map_err(|reason| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Internal,
                    format!("could not rebuild store key from run-scoped checkpoint ref: {reason}"),
                )
            })?
        } else {
            request.state_ref.clone()
        };

        let loaded = self
            .checkpoint_state_store
            .get_checkpoint_state(GetCheckpointStateRequest {
                scope: self.run_context.scope.clone(),
                turn_id: self.run_context.turn_id,
                run_id: self.run_context.run_id,
                state_ref: store_ref,
                schema_id: self.run_context.checkpoint_schema_id.clone(),
                schema_version: self.run_context.checkpoint_schema_version,
                kind: request.kind,
            })
            .await
            .map_err(turn_error_to_host_error)?;
        if loaded.is_none() {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::CheckpointRejected,
                "checkpoint state ref is unavailable for this loop run",
            ));
        }

        let checkpoint = self
            .loop_checkpoint_store
            .put_loop_checkpoint(PutLoopCheckpointRequest {
                scope: self.run_context.scope.clone(),
                turn_id: self.run_context.turn_id,
                run_id: self.run_context.run_id,
                state_ref: request.state_ref,
                schema_id: self.run_context.checkpoint_schema_id.clone(),
                schema_version: self.run_context.checkpoint_schema_version,
                kind: request.kind,
            })
            .await
            .map_err(turn_error_to_host_error)?;
        LoopHostMilestoneEmitter::new(self.run_context.clone(), Arc::clone(&self.milestone_sink))
            .checkpoint_created(checkpoint.checkpoint_id, request.kind)
            .await?;
        Ok(checkpoint.checkpoint_id)
    }

    async fn stage_checkpoint_payload(
        &self,
        request: StageCheckpointPayloadRequest,
    ) -> Result<LoopCheckpointStateRef, AgentLoopHostError> {
        // Reject staged payloads whose schema_id disagrees with the run
        // profile's resolved checkpoint schema — the read-side
        // `get_checkpoint_state` checks `(state_ref, schema_id, kind)` as a
        // unit, so mismatches here would lead to phantom resume rejections.
        if request.schema_id != self.run_context.checkpoint_schema_id.as_str() {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::CheckpointRejected,
                "staged checkpoint payload schema_id does not match the run profile's checkpoint schema",
            ));
        }

        let record = self
            .checkpoint_state_store
            .put_checkpoint_state(PutCheckpointStateRequest::new(
                self.run_context.scope.clone(),
                self.run_context.turn_id,
                self.run_context.run_id,
                self.run_context.checkpoint_schema_id.clone(),
                self.run_context.checkpoint_schema_version,
                request.kind,
                request.payload,
            ))
            .await
            .map_err(turn_error_to_host_error)?;

        // The store produces `checkpoint:{uuid}` refs. Wrap into the run-scoped
        // form `checkpoint:{run_id}:{token}` so that `LoopCheckpointStateRef::
        // is_for_run` validators accept the returned ref without treating it as
        // a cross-run ref. The token is the opaque UUID the store already minted.
        let raw = record.state_ref.as_str();
        let token = raw.strip_prefix("checkpoint:").ok_or_else(|| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                "checkpoint state store returned ref without expected `checkpoint:` prefix",
            )
        })?;
        LoopCheckpointStateRef::for_run(&self.run_context, token).map_err(|reason| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                format!("could not build run-scoped checkpoint state ref: {reason}"),
            )
        })
    }
}

#[derive(Clone)]
pub struct HostManagedLoopProgressPort {
    run_context: LoopRunContext,
    milestone_sink: Arc<dyn LoopHostMilestoneSink>,
}

impl HostManagedLoopProgressPort {
    pub fn new(
        run_context: LoopRunContext,
        milestone_sink: Arc<dyn LoopHostMilestoneSink>,
    ) -> Self {
        Self {
            run_context,
            milestone_sink,
        }
    }
}

impl LoopRunInfoPort for HostManagedLoopProgressPort {
    fn run_context(&self) -> &LoopRunContext {
        &self.run_context
    }
}

#[async_trait]
impl LoopProgressPort for HostManagedLoopProgressPort {
    async fn emit_loop_progress(&self, event: LoopProgressEvent) -> Result<(), AgentLoopHostError> {
        let emitter = LoopHostMilestoneEmitter::new(
            self.run_context.clone(),
            Arc::clone(&self.milestone_sink),
        );
        match event {
            LoopProgressEvent::DriverNote { kind, safe_summary } => {
                emitter.driver_note(kind, safe_summary).await
            }
            LoopProgressEvent::IterationStarted { iteration } => {
                emitter.iteration_started(iteration).await
            }
            // Prompt construction already emits the canonical
            // `PromptBundleBuilt` milestone from `HostManagedLoopPromptPort`,
            // including the bundle ref and redacted skill-context metadata.
            // Treat the executor progress echo as advisory to avoid duplicate
            // prompt milestones for the same bundle.
            LoopProgressEvent::PromptBundleBuilt { .. } => Ok(()),
            LoopProgressEvent::CapabilityBatchStarted {
                iteration,
                call_count,
                policy,
            } => {
                emitter
                    .capability_batch_started(iteration, call_count, policy)
                    .await
            }
            LoopProgressEvent::CapabilityBatchCompleted {
                iteration,
                result_count,
                denied_count,
                gated_count,
                failed_count,
            } => {
                emitter
                    .capability_batch_completed(
                        iteration,
                        result_count,
                        denied_count,
                        gated_count,
                        failed_count,
                    )
                    .await
            }
            LoopProgressEvent::GateBlocked {
                iteration,
                gate_kind,
            } => emitter.gate_blocked(iteration, gate_kind).await,
            // `HostManagedLoopCheckpointPort::checkpoint` publishes the
            // canonical checkpoint milestone with the durable checkpoint id.
            // `CheckpointWritten` carries only the checkpoint kind/iteration,
            // so emitting it here would either duplicate or weaken that record.
            LoopProgressEvent::CheckpointWritten { .. } => Ok(()),
        }
    }
}

fn validate_claimed_run_context(
    claimed_run: &ClaimedTurnRun,
    run_context: &LoopRunContext,
) -> Result<(), RebornLoopDriverHostError> {
    if claimed_run.state.status != TurnStatus::Running {
        return Err(RebornLoopDriverHostError::InvalidRequest {
            reason: "claimed run must be running".to_string(),
        });
    }
    if claimed_run.state.scope != run_context.scope
        || claimed_run.state.turn_id != run_context.turn_id
        || claimed_run.state.run_id != run_context.run_id
    {
        return Err(RebornLoopDriverHostError::ScopeMismatch {
            reason: "claimed run state does not match loop run context".to_string(),
        });
    }
    if claimed_run.resolved_run_profile != run_context.resolved_run_profile {
        return Err(RebornLoopDriverHostError::ScopeMismatch {
            reason: "claimed run profile does not match loop run context".to_string(),
        });
    }
    match (
        &claimed_run.state.resolved_model_route,
        &run_context.resolved_model_route,
    ) {
        (Some(expected), Some(actual)) if expected != actual => {
            return Err(RebornLoopDriverHostError::ScopeMismatch {
                reason: "loop run context model route does not match claimed run".to_string(),
            });
        }
        (Some(_), None) => {
            return Err(RebornLoopDriverHostError::ScopeMismatch {
                reason: "loop run context is missing claimed run model route".to_string(),
            });
        }
        (None, Some(_)) => {
            return Err(RebornLoopDriverHostError::ScopeMismatch {
                reason: "loop run context model route was not persisted on claimed run".to_string(),
            });
        }
        _ => {}
    }
    let expected_profile_id = persisted_profile_id(&run_context.resolved_run_profile.profile_id);
    if claimed_run.state.resolved_run_profile_id != expected_profile_id
        || claimed_run.state.resolved_run_profile_version
            != run_context.resolved_run_profile.profile_version
    {
        return Err(RebornLoopDriverHostError::ScopeMismatch {
            reason: "claimed run persisted profile identity does not match loop run context"
                .to_string(),
        });
    }
    if run_context.loop_driver_id != run_context.resolved_run_profile.loop_driver.id
        || run_context.loop_driver_version != run_context.resolved_run_profile.loop_driver.version
    {
        return Err(RebornLoopDriverHostError::ScopeMismatch {
            reason: "loop driver identity does not match resolved profile".to_string(),
        });
    }
    if run_context.thread_id != run_context.scope.thread_id {
        return Err(RebornLoopDriverHostError::ScopeMismatch {
            reason: "loop run context thread does not match scope thread".to_string(),
        });
    }
    if run_context.checkpoint_schema_id != run_context.resolved_run_profile.checkpoint_schema_id
        || run_context.checkpoint_schema_version
            != run_context.resolved_run_profile.checkpoint_schema_version
    {
        return Err(RebornLoopDriverHostError::ScopeMismatch {
            reason: "loop run context checkpoint identity does not match resolved profile"
                .to_string(),
        });
    }
    Ok(())
}

#[async_trait]
impl<S, G> crate::turn_runner::HostFactory for RebornLoopDriverHostFactory<S, G>
where
    S: SessionThreadService + ?Sized + Send + Sync + 'static,
    G: HostManagedModelGateway + ?Sized + Send + Sync + 'static,
{
    async fn create_host(
        &self,
        claimed: &ClaimedTurnRun,
    ) -> Result<
        Box<dyn ironclaw_turns::run_profile::AgentLoopDriverHost + Send + Sync>,
        crate::turn_runner::HostFactoryError,
    > {
        let mut loop_run_context = LoopRunContext::new(
            claimed.state.scope.clone(),
            claimed.state.turn_id,
            claimed.state.run_id,
            claimed.resolved_run_profile.clone(),
        );
        if let Some(snapshot) = claimed.state.resolved_model_route.clone() {
            loop_run_context = loop_run_context.with_resolved_model_route(snapshot);
        }
        self.build_text_only_host(RebornLoopDriverHostRequest {
            claimed_run: claimed.clone(),
            loop_run_context,
        })
        .await
        .map(|host| {
            Box::new(host)
                as Box<dyn ironclaw_turns::run_profile::AgentLoopDriverHost + Send + Sync>
        })
        .map_err(|error| crate::turn_runner::HostFactoryError::new(error.to_string()))
    }
}

fn model_route_error_to_host_error(error: ModelRouteError) -> RebornLoopDriverHostError {
    RebornLoopDriverHostError::InvalidRequest {
        reason: format!("model route resolution failed: {}", error.kind().as_str()),
    }
}

fn slot_for_model_profile(
    run_context: &LoopRunContext,
) -> Result<ModelSlot, RebornLoopDriverHostError> {
    ModelSlot::from_model_profile_id(&run_context.resolved_run_profile.model_profile_id).ok_or_else(
        || RebornLoopDriverHostError::InvalidRequest {
            reason: "model profile is not supported by the model route resolver".to_string(),
        },
    )
}

fn persisted_profile_id(profile_id: &RunProfileId) -> RunProfileId {
    if profile_id.is_interactive_default() {
        RunProfileId::default_profile()
    } else {
        profile_id.clone()
    }
}

fn validate_thread_scope(
    thread_scope: &ThreadScope,
    run_context: &LoopRunContext,
) -> Result<(), RebornLoopDriverHostError> {
    // Reborn text-only hosts currently wrap `ironclaw_threads::ThreadScope`,
    // whose production transcript boundary is agent-scoped. Agentless turn
    // scopes are rejected here until that lower thread boundary grows an
    // explicit agentless thread scope.
    if run_context.scope.agent_id.as_ref() != Some(&thread_scope.agent_id) {
        return Err(RebornLoopDriverHostError::ScopeMismatch {
            reason: "text-only loop host requires a matching agent-scoped thread".to_string(),
        });
    }
    if thread_scope.tenant_id != run_context.scope.tenant_id
        || thread_scope.project_id != run_context.scope.project_id
    {
        return Err(RebornLoopDriverHostError::ScopeMismatch {
            reason: "thread scope does not match loop run scope".to_string(),
        });
    }
    Ok(())
}

fn turn_error_to_host_error(error: TurnError) -> AgentLoopHostError {
    match error {
        TurnError::Unauthorized => AgentLoopHostError::new(
            AgentLoopHostErrorKind::Unauthorized,
            "checkpoint state access was unauthorized",
        ),
        TurnError::InvalidRequest { .. } => AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            "checkpoint state request is invalid",
        ),
        TurnError::Unavailable { .. } => AgentLoopHostError::new(
            AgentLoopHostErrorKind::Unavailable,
            "checkpoint state store is unavailable",
        ),
        TurnError::ScopeNotFound => AgentLoopHostError::new(
            AgentLoopHostErrorKind::CheckpointRejected,
            "checkpoint state scope was not found for this loop run",
        ),
        TurnError::Conflict { .. } => AgentLoopHostError::new(
            AgentLoopHostErrorKind::CheckpointRejected,
            "checkpoint state write conflicted with current turn state",
        ),
        TurnError::InvalidTransition { .. } => AgentLoopHostError::new(
            AgentLoopHostErrorKind::CheckpointRejected,
            "checkpoint state write was invalid for current turn state",
        ),
        TurnError::LeaseMismatch => AgentLoopHostError::new(
            AgentLoopHostErrorKind::CheckpointRejected,
            "checkpoint state write lease no longer matches current run",
        ),
        TurnError::ThreadBusy(_) | TurnError::AdmissionRejected(_) => AgentLoopHostError::new(
            AgentLoopHostErrorKind::Unavailable,
            "checkpoint state store returned unsupported turn admission status",
        ),
    }
}

#[cfg(test)]
mod hook_resolver_adapter_tests {
    //! Unit coverage for [`HookCapabilityInputResolverAdapter`]. These tests
    //! drive the adapter directly (not through the factory) so they can
    //! exercise every error branch without standing up a full Reborn host.

    use super::*;
    use ironclaw_host_api::{AgentId, CapabilityId, ProjectId, TenantId, ThreadId};
    use ironclaw_turns::run_profile::{
        AgentLoopHostError, AgentLoopHostErrorKind, CapabilityInputRef, CapabilityInvocation,
        CapabilitySurfaceVersion,
    };
    use ironclaw_turns::{
        InMemoryRunProfileResolver, RunProfileResolutionRequest, RunProfileResolver, TurnId,
        TurnRunId, TurnScope,
    };
    use std::sync::Mutex;

    fn tenant() -> TenantId {
        TenantId::new("hook-resolver-tests").expect("tenant id literal valid")
    }

    async fn run_context() -> LoopRunContext {
        let tenant_id = tenant();
        let agent_id = AgentId::new("agent-hook-resolver").expect("agent id literal valid");
        let project_id = ProjectId::new("project-hook-resolver").expect("project id literal valid");
        let thread_id = ThreadId::new("thread-hook-resolver").expect("thread id literal valid");
        let scope = TurnScope::new(tenant_id, Some(agent_id), Some(project_id), thread_id);
        let resolved = InMemoryRunProfileResolver::default()
            .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
            .await
            .expect("interactive default run profile resolves");
        LoopRunContext::new(scope, TurnId::new(), TurnRunId::new(), resolved)
    }

    fn invocation(input_ref: &str) -> CapabilityInvocation {
        CapabilityInvocation {
            surface_version: CapabilitySurfaceVersion::new("v1")
                .expect("surface version literal valid"),
            capability_id: CapabilityId::new("cap.test").expect("capability id literal valid"),
            input_ref: CapabilityInputRef::new(input_ref).expect("input ref literal valid"),
        }
    }

    /// Test double for [`LoopCapabilityInputResolver`] that returns a queued
    /// `Result` per call; lets us cover both Ok and Err branches.
    struct StubInputResolver {
        responses: Mutex<Vec<Result<serde_json::Value, AgentLoopHostError>>>,
    }

    impl StubInputResolver {
        fn new(responses: Vec<Result<serde_json::Value, AgentLoopHostError>>) -> Self {
            Self {
                responses: Mutex::new(responses),
            }
        }
    }

    #[async_trait]
    impl LoopCapabilityInputResolver for StubInputResolver {
        async fn resolve_capability_input(
            &self,
            _run_context: &LoopRunContext,
            _input_ref: &CapabilityInputRef,
        ) -> Result<serde_json::Value, AgentLoopHostError> {
            self.responses
                .lock()
                .expect("stub responses mutex not poisoned")
                .remove(0)
        }
    }

    #[tokio::test]
    async fn adapter_extracts_json_body_when_inner_resolves() {
        let inner = Arc::new(StubInputResolver::new(vec![Ok(serde_json::json!({
            "amount": "50"
        }))]));
        let adapter = HookCapabilityInputResolverAdapter::new(inner, run_context().await);

        let resolved = adapter.resolve(&invocation("input:cap.test")).await;
        let value = resolved.expect("adapter returns Some when inner resolves");
        assert_eq!(value, serde_json::json!({"amount": "50"}));
    }

    #[tokio::test]
    async fn adapter_returns_none_when_inner_errors() {
        let inner = Arc::new(StubInputResolver::new(vec![Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            "input ref is unknown",
        ))]));
        let adapter = HookCapabilityInputResolverAdapter::new(inner, run_context().await);

        let resolved = adapter.resolve(&invocation("input:missing")).await;
        assert!(
            resolved.is_none(),
            "adapter must fail closed when inner resolver returns an error"
        );
    }

    #[tokio::test]
    async fn adapter_passes_through_non_object_json_unchanged() {
        // The trait already returns `serde_json::Value`, so "non-JSON-shaped"
        // input cannot reach the adapter — anything the inner returns is
        // already typed JSON. The fail-closed case for non-JSON bodies lives
        // in the inner resolver's implementation (it surfaces an
        // `AgentLoopHostError` on decode failure, covered by the
        // `returns_none_when_inner_errors` test above). This test pins down
        // the adapter's contract for non-object inputs: it must forward them
        // verbatim so predicate evaluators see the raw shape and decide for
        // themselves whether to fail closed.
        let inner = Arc::new(StubInputResolver::new(vec![Ok(serde_json::Value::String(
            "not-an-object".to_string(),
        ))]));
        let adapter = HookCapabilityInputResolverAdapter::new(inner, run_context().await);

        let resolved = adapter.resolve(&invocation("input:non-object")).await;
        assert_eq!(
            resolved,
            Some(serde_json::Value::String("not-an-object".to_string())),
            "adapter forwards non-object JSON verbatim"
        );
    }

    #[tokio::test]
    async fn adapter_returns_none_when_body_exceeds_byte_budget() {
        // Build a value whose serialized form deliberately exceeds the
        // adapter's configured budget. The hooks crate's
        // `SanitizedArguments::from_json` caps individual string lengths and
        // nesting depth, but does not bound total payload bytes — this guard
        // is the adapter's defense in depth.
        let oversized_string = "x".repeat(2048);
        let inner = Arc::new(StubInputResolver::new(vec![Ok(serde_json::json!({
            "blob": oversized_string,
        }))]));
        let adapter = HookCapabilityInputResolverAdapter::new(inner, run_context().await)
            .with_max_input_bytes(512);

        let resolved = adapter.resolve(&invocation("input:oversized")).await;
        assert!(
            resolved.is_none(),
            "adapter must refuse payloads above the configured byte budget"
        );
    }
}
