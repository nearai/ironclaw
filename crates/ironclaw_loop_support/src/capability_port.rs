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
        LoopRunContext, LoopSafeSummary, ProcessHandleSummary, VisibleCapabilityRequest,
        VisibleCapabilitySurface,
    },
};
use tokio::sync::Notify;

#[async_trait]
pub trait LoopCapabilityInputResolver: Send + Sync {
    async fn resolve_capability_input(
        &self,
        run_context: &LoopRunContext,
        input_ref: &CapabilityInputRef,
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
pub struct HostRuntimeLoopCapabilityPortFactory {
    runtime: Arc<dyn HostRuntime>,
    visible_request: ironclaw_host_runtime::VisibleCapabilityRequest,
    input_resolver: Arc<dyn LoopCapabilityInputResolver>,
    result_writer: Arc<dyn LoopCapabilityResultWriter>,
    milestone_sink: Option<Arc<dyn LoopHostMilestoneSink>>,
}

impl HostRuntimeLoopCapabilityPortFactory {
    pub fn new(
        runtime: Arc<dyn HostRuntime>,
        visible_request: ironclaw_host_runtime::VisibleCapabilityRequest,
        input_resolver: Arc<dyn LoopCapabilityInputResolver>,
        result_writer: Arc<dyn LoopCapabilityResultWriter>,
        milestone_sink: Option<Arc<dyn LoopHostMilestoneSink>>,
    ) -> Self {
        Self {
            runtime,
            visible_request,
            input_resolver,
            result_writer,
            milestone_sink,
        }
    }

    pub fn without_milestone_sink(
        runtime: Arc<dyn HostRuntime>,
        visible_request: ironclaw_host_runtime::VisibleCapabilityRequest,
        input_resolver: Arc<dyn LoopCapabilityInputResolver>,
        result_writer: Arc<dyn LoopCapabilityResultWriter>,
    ) -> Self {
        Self::new(
            runtime,
            visible_request,
            input_resolver,
            result_writer,
            None,
        )
    }

    pub fn with_milestone_sink(mut self, sink: Arc<dyn LoopHostMilestoneSink>) -> Self {
        self.milestone_sink = Some(sink);
        self
    }

    pub fn for_run_context(&self, run_context: LoopRunContext) -> Arc<dyn LoopCapabilityPort> {
        let mut port = HostRuntimeLoopCapabilityPort::new(
            Arc::clone(&self.runtime),
            run_context,
            self.visible_request.clone(),
            Arc::clone(&self.input_resolver),
            Arc::clone(&self.result_writer),
        );
        if let Some(sink) = &self.milestone_sink {
            port = port.with_milestone_sink(Arc::clone(sink));
        }
        Arc::new(port)
    }
}

#[derive(Clone)]
struct SurfaceCapabilitySnapshot {
    provider: ExtensionId,
    runtime: RuntimeKind,
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
                        runtime: capability.descriptor.runtime,
                        estimate: capability.estimated_resources.clone(),
                    },
                );
                CapabilityDescriptorView {
                    capability_id,
                    provider: Some(capability.descriptor.provider),
                    runtime: capability.descriptor.runtime,
                    safe_name: capability.descriptor.id.as_str().to_string(),
                    safe_description: capability.descriptor.description,
                    concurrency_hint: concurrency_hint_from_effects(&capability.descriptor.effects),
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
                        cleanup_error = %clear_error,
                        original_error = ?error,
                        "failed to clean up state after input resolution failure"
                    );
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
                    cleanup_error = %clear_error,
                    original_error = ?error,
                    "failed to clean up state after milestone emission failure"
                );
            }
            return Err(error);
        }
        let outcome = match self
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
                    )?,
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
                        cleanup_error = %clear_error,
                        original_error = ?error,
                        "failed to clean up state after host runtime failure"
                    );
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
) -> Result<ExecutionContext, AgentLoopHostError> {
    let mut context = base.clone();
    let loop_driver_extension =
        ExtensionId::new(run_context.loop_driver_id.as_str()).map_err(|_| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                "loop driver id could not be represented as an execution extension",
            )
        })?;
    context.extension_id = loop_driver_extension.clone();
    context.runtime = capability.runtime;
    context.trust = trust;
    context.grants = invocation_grants_from_visible(
        base,
        capability_id,
        &loop_driver_extension,
        allowed_effects,
    )?;
    context.mounts = MountView::default();
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
    use ironclaw_host_api::{
        AgentId, CapabilityGrant, CapabilityGrantId, GrantConstraints, MountAlias, MountGrant,
        MountPermissions, NetworkPolicy, ProjectId, TenantId, TrustClass, UserId, VirtualPath,
    };
    use ironclaw_turns::{
        InMemoryRunProfileResolver, RunProfileResolutionRequest, RunProfileResolver, TurnId,
        TurnRunId, TurnScope,
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
        };

        let err = invocation_context_from_visible(
            &context,
            &run_context,
            &capability_id,
            &capability,
            TrustClass::Sandbox,
            &[EffectKind::ReadFilesystem],
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
        };

        let invocation_context = invocation_context_from_visible(
            &context,
            &run_context,
            &capability_id,
            &capability,
            TrustClass::Sandbox,
            &[EffectKind::ReadFilesystem],
        )
        .expect("host-issued mount grant should be preserved");

        assert_eq!(invocation_context.mounts, MountView::default());
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
        };

        let invocation_context = invocation_context_from_visible(
            &context,
            &run_context,
            &capability_id,
            &capability,
            TrustClass::Sandbox,
            &[EffectKind::ReadFilesystem],
        )
        .expect("matching host scope grant should be preserved");

        assert_eq!(invocation_context.grants.grants.len(), 1);
        assert!(matches!(
            &invocation_context.grants.grants[0].grantee,
            Principal::Thread(_)
        ));
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
        };

        let invocation_context = invocation_context_from_visible(
            &context,
            &run_context,
            &capability_id,
            &capability,
            TrustClass::UserTrusted,
            &[EffectKind::DispatchCapability],
        )
        .expect("context");

        assert_eq!(invocation_context.extension_id, loop_driver_extension);
        assert_eq!(invocation_context.runtime, RuntimeKind::Script);
        assert_eq!(invocation_context.trust, TrustClass::UserTrusted);
        assert_eq!(invocation_context.mounts, MountView::default());
        assert_eq!(invocation_context.grants.grants.len(), 1);
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
