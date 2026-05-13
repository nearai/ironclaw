use std::{
    collections::{HashMap, VecDeque},
    error::Error,
    fmt,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use ironclaw_host_api::{
    CapabilityDescriptor, CapabilityId, CorrelationId, EffectKind, ExecutionContext, ExtensionId,
    InvocationId, PermissionMode, ResourceEstimate, sha256_digest_token,
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
    CheckpointStateStore, GetCheckpointStateRequest, LoopCheckpointStore, LoopGateRef,
    LoopResultRef, PutCheckpointStateRequest, PutLoopCheckpointRequest, RunProfileId,
    TurnCheckpointId, TurnError, TurnStatus,
    run_profile::{
        AgentLoopHostError, AgentLoopHostErrorKind, AppendCapabilityResultRef,
        BatchExecutionPolicy, BeginAssistantDraft, CapabilityBatchInvocation,
        CapabilityBatchOutcome, CapabilityConcurrency, CapabilityDenied,
        CapabilityDeniedReasonKind, CapabilityDescriptorView, CapabilityFailure,
        CapabilityInvocation, CapabilityOutcome, CapabilityResultMessage, CapabilitySurfaceVersion,
        FinalizeAssistantMessage, LoopCapabilityPort, LoopCheckpointPort, LoopCheckpointRequest,
        LoopCheckpointStateRef, LoopContextBundle, LoopContextPort, LoopContextRequest,
        LoopHostMilestoneEmitter, LoopHostMilestoneSink, LoopInputBatch, LoopInputCursor,
        LoopInputPort, LoopModelMessage, LoopModelPort, LoopModelRequest, LoopModelResponse,
        LoopProcessRef, LoopProgressEvent, LoopProgressPort, LoopPromptBundle, LoopPromptBundleRef,
        LoopPromptBundleRequest, LoopPromptPort, LoopRunContext, LoopRunInfoPort, LoopSafeSummary,
        LoopTranscriptPort, ProcessHandleSummary, PromptMode, PromptSkillContextMetadata,
        StoreLoopCheckpointPayload, UpdateAssistantDraft, VisibleCapabilityRequest,
        VisibleCapabilitySurface, skill_snippet_model_message_ref,
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

const DEFAULT_TEXT_ONLY_MESSAGE_LIMIT: usize = 32;
const MAX_TEXT_ONLY_MESSAGE_LIMIT: usize = 128;

#[derive(Default)]
struct CapabilitySurfaceState {
    current: Mutex<Option<CapabilitySurfaceVersion>>,
}

impl CapabilitySurfaceState {
    fn set_current(&self, version: CapabilitySurfaceVersion) -> Result<(), AgentLoopHostError> {
        let mut current = self.current.lock().map_err(|_| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "capability surface state is unavailable",
            )
        })?;
        *current = Some(version);
        Ok(())
    }

    fn current(&self) -> Result<Option<CapabilitySurfaceVersion>, AgentLoopHostError> {
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
        self.surface_state.set_current(surface.version.clone())?;
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
                let concurrency = capability_concurrency_for(&capability.descriptor);
                CapabilityDescriptorView {
                    capability_id,
                    provider: Some(capability.descriptor.provider),
                    runtime: capability.descriptor.runtime,
                    safe_name: capability.descriptor.id.as_str().to_string(),
                    safe_description: capability.descriptor.description,
                    concurrency,
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
        // Decide between serial and concurrent dispatch. `Parallel` is
        // only honored when `stop_on_first_suspension == false`; the
        // executor today always sets it to `true` (see
        // `executor::capability::handle_capability_calls` — suspension
        // outcomes are dynamic host state and can arise even for
        // `SafeForParallel` descriptors). Cancelling in-flight
        // concurrent invocations on first-suspension would require a
        // `JoinSet`-with-abort pattern; keeping the fallback to the
        // serial loop in that case is strictly better than today's
        // "always serial" state — the contract surface is correct, and
        // when WS-7+ flips a profile to `stop_on_first_suspension =
        // false` the parallel branch becomes live with no further
        // wiring. See PR #3586 follow-up.
        let want_parallel = matches!(request.policy, BatchExecutionPolicy::Parallel)
            && !request.stop_on_first_suspension;
        if want_parallel {
            let invocations = request.invocations;
            let futures = invocations
                .into_iter()
                .map(|invocation| self.invoke_capability(invocation));
            let results: Vec<Result<CapabilityOutcome, AgentLoopHostError>> =
                futures_util::future::join_all(futures).await;
            let mut outcomes = Vec::with_capacity(results.len());
            for result in results {
                outcomes.push(result?);
            }
            return Ok(CapabilityBatchOutcome {
                outcomes,
                stopped_on_suspension: false,
            });
        }

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

/// Derive a `CapabilityConcurrency` from a registry `CapabilityDescriptor`.
///
/// Any side-effecting effect or `default_permission = Ask` forces `Exclusive`
/// so the framework's batch policy stops on first suspension. Read-only
/// descriptors keep `SafeForParallel` so simple read fan-outs still parallelize.
fn capability_concurrency_for(descriptor: &CapabilityDescriptor) -> CapabilityConcurrency {
    if matches!(descriptor.default_permission, PermissionMode::Ask) {
        return CapabilityConcurrency::Exclusive;
    }
    let exclusive_effect = descriptor.effects.iter().any(|effect| {
        matches!(
            effect,
            EffectKind::WriteFilesystem
                | EffectKind::DeleteFilesystem
                | EffectKind::SpawnProcess
                | EffectKind::ExecuteCode
                | EffectKind::ExternalWrite
                | EffectKind::Financial
                | EffectKind::ModifyExtension
                | EffectKind::ModifyApproval
                | EffectKind::ModifyBudget
                | EffectKind::DispatchCapability
        )
    });
    if exclusive_effect {
        CapabilityConcurrency::Exclusive
    } else {
        CapabilityConcurrency::SafeForParallel
    }
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
                    error_kind: failure.kind.as_str().to_string(),
                    safe_summary: runtime_safe_summary(
                        failure.message,
                        "capability invocation failed",
                    ),
                })
            }
        }
        RuntimeCapabilityOutcome::Unknown(unknown) => {
            CapabilityOutcome::Failed(CapabilityFailure {
                error_kind: unknown.kind,
                safe_summary: runtime_safe_summary(
                    unknown.message,
                    "capability invocation returned an unknown outcome",
                ),
            })
        }
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

struct SurfaceAwareLoopPromptPort {
    context: LoopRunContext,
    context_port: Arc<dyn LoopContextPort>,
    milestone_sink: Arc<dyn LoopHostMilestoneSink>,
    default_message_limit: usize,
    surface_state: Arc<CapabilitySurfaceState>,
}

impl SurfaceAwareLoopPromptPort {
    fn new(
        context: LoopRunContext,
        context_port: Arc<dyn LoopContextPort>,
        milestone_sink: Arc<dyn LoopHostMilestoneSink>,
        surface_state: Arc<CapabilitySurfaceState>,
    ) -> Self {
        Self {
            context,
            context_port,
            milestone_sink,
            default_message_limit: DEFAULT_TEXT_ONLY_MESSAGE_LIMIT,
            surface_state,
        }
    }

    fn with_default_message_limit(mut self, default_message_limit: usize) -> Self {
        self.default_message_limit = default_message_limit.clamp(1, MAX_TEXT_ONLY_MESSAGE_LIMIT);
        self
    }

    fn validate_request(
        &self,
        request: &LoopPromptBundleRequest,
    ) -> Result<(), AgentLoopHostError> {
        if request.mode != PromptMode::TextOnly {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::PolicyDenied,
                "prompt mode is not supported by the text-only prompt port",
            ));
        }

        if !request.inline_messages.is_empty() {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::PolicyDenied,
                "inline_messages not yet supported by this prompt builder",
            ));
        }

        if request
            .context_cursor
            .as_ref()
            .is_some_and(|cursor| !cursor.is_for_run(&self.context))
        {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::ScopeMismatch,
                "prompt context cursor is not scoped to this loop run",
            ));
        }

        if let Some(surface_version) = request.surface_version.as_ref() {
            let Some(current_surface_version) = self.surface_state.current()? else {
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::InvalidInvocation,
                    "prompt surface version cannot be validated by this prompt port",
                ));
            };
            if surface_version != &current_surface_version {
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::StaleSurface,
                    "prompt surface version is stale or unknown",
                ));
            }
        }

        if let Some(state_ref) = request.checkpoint_state_ref.as_ref() {
            let run_prefix = format!("checkpoint:{}:", self.context.run_id);
            if !state_ref.as_str().starts_with(&run_prefix) {
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::ScopeMismatch,
                    "prompt checkpoint state ref is not scoped to this loop run",
                ));
            }
            if !state_ref.is_for_run(&self.context) {
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::InvalidInvocation,
                    "prompt checkpoint state ref is malformed",
                ));
            }
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "checkpoint prompt state is not supported by the text-only prompt port",
            ));
        }

        if matches!(request.max_messages, Some(0)) {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::BudgetExceeded,
                "prompt message limit must be greater than zero",
            ));
        }

        Ok(())
    }

    fn message_limit(&self, request: &LoopPromptBundleRequest) -> usize {
        request
            .max_messages
            .map(|messages| messages as usize)
            .unwrap_or(self.default_message_limit)
            .clamp(1, MAX_TEXT_ONLY_MESSAGE_LIMIT)
    }

    fn ensure_supported_context_shape(
        context: &LoopContextBundle,
    ) -> Result<(), AgentLoopHostError> {
        if !context.memory_snippets.is_empty() {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::PolicyDenied,
                "text-only prompt port cannot materialize memory snippets",
            ));
        }
        Ok(())
    }
}

#[async_trait]
impl LoopPromptPort for SurfaceAwareLoopPromptPort {
    async fn build_prompt_bundle(
        &self,
        request: LoopPromptBundleRequest,
    ) -> Result<LoopPromptBundle, AgentLoopHostError> {
        self.validate_request(&request)?;
        let context = self
            .context_port
            .load_loop_context(LoopContextRequest {
                after: request.context_cursor.clone(),
                limit: self.message_limit(&request),
            })
            .await?;
        Self::ensure_supported_context_shape(&context)?;
        let mut messages =
            Vec::with_capacity(context.instruction_snippets.len() + context.messages.len());
        let mut skill_context = Vec::with_capacity(context.instruction_snippets.len());
        for (ordinal, snippet) in context.instruction_snippets.into_iter().enumerate() {
            let content_ref = skill_snippet_model_message_ref(
                &snippet.snippet_ref,
                &snippet.safe_summary,
                ordinal,
            )?;
            match snippet.metadata.as_ref() {
                Some(metadata) => skill_context.push(PromptSkillContextMetadata {
                    ordinal,
                    source_name: metadata.source_name.clone(),
                    trust_level: metadata.trust_level.clone(),
                }),
                None if snippet.snippet_ref.starts_with("skill:") => {
                    return Err(AgentLoopHostError::new(
                        AgentLoopHostErrorKind::Internal,
                        "skill instruction snippet metadata is missing",
                    ));
                }
                None => {}
            }
            messages.push(LoopModelMessage {
                role: "system".to_string(),
                content_ref,
            });
        }
        messages.extend(
            context
                .messages
                .into_iter()
                .map(|message| LoopModelMessage {
                    role: message.role,
                    content_ref: message.message_ref,
                }),
        );
        let bundle = LoopPromptBundle {
            bundle_ref: LoopPromptBundleRef::for_run(
                &self.context,
                CorrelationId::new().to_string(),
            )
            .map_err(|_| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Internal,
                    "loop prompt bundle ref could not be represented",
                )
            })?,
            messages,
            surface_version: request.surface_version.clone(),
        };
        LoopHostMilestoneEmitter::new(self.context.clone(), Arc::clone(&self.milestone_sink))
            .prompt_bundle_built(
                bundle.bundle_ref.clone(),
                request.mode,
                bundle.surface_version.clone(),
                bundle.messages.len(),
                skill_context,
            )
            .await?;
        Ok(bundle)
    }
}

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
    config: TextOnlyLoopHostConfig,
    skill_context_source: Option<Arc<dyn HostSkillContextSource>>,
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
            config,
            skill_context_source: None,
        }
    }

    pub fn with_skill_context_source(mut self, source: Arc<dyn HostSkillContextSource>) -> Self {
        self.skill_context_source = Some(source);
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
        let surface_state = Arc::new(CapabilitySurfaceState::default());
        let capabilities: Arc<dyn LoopCapabilityPort> = Arc::new(
            SurfaceTrackingLoopCapabilityPort::new(capabilities, Arc::clone(&surface_state)),
        );
        capabilities
            .visible_capabilities(VisibleCapabilityRequest)
            .await
            .map_err(|error| RebornLoopDriverHostError::InvalidRequest {
                reason: error.safe_summary,
            })?;
        let prompt: Arc<dyn LoopPromptPort> = Arc::new(
            SurfaceAwareLoopPromptPort::new(
                run_context.clone(),
                Arc::clone(&context),
                Arc::clone(&self.milestone_sink),
                surface_state,
            )
            .with_default_message_limit(max_messages),
        );
        let input: Arc<dyn LoopInputPort> =
            Arc::new(NoExtraLoopInputPort::new(run_context.clone()));
        let mut model_adapter = ThreadBackedLoopModelPort::with_milestone_sink(
            Arc::clone(&self.thread_service),
            self.thread_scope.clone(),
            run_context.clone(),
            Arc::clone(&self.model_gateway),
            max_messages,
            Arc::clone(&self.milestone_sink),
        );
        if let Some(source) = self.skill_context_source.as_ref() {
            model_adapter = model_adapter.with_skill_context_source(source.clone());
        }
        let model: Arc<dyn LoopModelPort> = Arc::new(model_adapter);
        let checkpoint: Arc<dyn LoopCheckpointPort> = Arc::new(HostManagedLoopCheckpointPort::new(
            run_context.clone(),
            Arc::clone(&self.checkpoint_state_store),
            Arc::clone(&self.loop_checkpoint_store),
            Arc::clone(&self.milestone_sink),
        ));
        let transcript: Arc<dyn LoopTranscriptPort> =
            Arc::new(ThreadBackedLoopTranscriptPort::with_milestone_sink(
                Arc::clone(&self.thread_service),
                self.thread_scope.clone(),
                run_context.clone(),
                Arc::clone(&self.milestone_sink),
            ));
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

    async fn store_checkpoint_payload(
        &self,
        request: StoreLoopCheckpointPayload,
    ) -> Result<LoopCheckpointStateRef, AgentLoopHostError> {
        self.checkpoint.store_checkpoint_payload(request).await
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
        let loaded = self
            .checkpoint_state_store
            .get_checkpoint_state(GetCheckpointStateRequest {
                scope: self.run_context.scope.clone(),
                turn_id: self.run_context.turn_id,
                run_id: self.run_context.run_id,
                state_ref: request.state_ref.clone(),
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

    async fn store_checkpoint_payload(
        &self,
        request: StoreLoopCheckpointPayload,
    ) -> Result<LoopCheckpointStateRef, AgentLoopHostError> {
        // Thread the active run profile's `max_checkpoint_bytes` cap into
        // the store so profiles that opt into larger checkpoints (durable
        // mission = 256 KiB) aren't silently rejected by the store's legacy
        // default. The store still enforces an absolute system ceiling on
        // top of this.
        let profile_cap = self
            .run_context
            .resolved_run_profile
            .checkpoint_policy
            .max_checkpoint_bytes;
        let record = self
            .checkpoint_state_store
            .put_checkpoint_state(
                PutCheckpointStateRequest::new(
                    self.run_context.scope.clone(),
                    self.run_context.turn_id,
                    self.run_context.run_id,
                    self.run_context.checkpoint_schema_id.clone(),
                    self.run_context.checkpoint_schema_version,
                    request.kind,
                    request.payload,
                )
                .with_max_payload_bytes(profile_cap),
            )
            .await
            .map_err(turn_error_to_host_error)?;
        Ok(record.state_ref)
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
        match event {
            LoopProgressEvent::DriverNote { kind, safe_summary } => {
                LoopHostMilestoneEmitter::new(
                    self.run_context.clone(),
                    Arc::clone(&self.milestone_sink),
                )
                .driver_note(kind, safe_summary)
                .await
            }
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
