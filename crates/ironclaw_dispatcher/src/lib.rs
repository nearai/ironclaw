//! Composition-only runtime dispatch contracts for IronClaw Reborn.
//!
//! `ironclaw_dispatcher` wires validated extension descriptors to runtime lanes. It
//! does not parse extension manifests, implement sandbox policy, reserve budget
//! itself, or execute product workflows. Those responsibilities stay in the
//! owning service crates.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_events::{EventSink, RuntimeEvent};
use ironclaw_extensions::{ExtensionPackage, ExtensionRegistry, SharedExtensionRegistry};
use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::{
    CapabilityDescriptor, CapabilityId, ExtensionId, MountView, ResourceEstimate, ResourceReceipt,
    ResourceReservation, ResourceScope, ResourceUsage, RuntimeKind, RuntimeLane,
    runtime_policy::{
        ApprovalPolicy, AuditMode, DeploymentMode, EffectiveRuntimePolicy, FilesystemBackendKind,
        NetworkMode, ProcessBackendKind, RuntimeProfile, SecretMode,
    },
};
pub use ironclaw_host_api::{
    CapabilityDispatchRequest, CapabilityDispatchResult, CapabilityDispatcher,
    CapabilityDisplayOutputPreview, DispatchError, RuntimeDispatchErrorKind,
};
use ironclaw_resources::ResourceGovernor;
use serde_json::Value;

enum ServiceHandle<'a, T>
where
    T: ?Sized,
{
    Borrowed(&'a T),
    Shared(Arc<T>),
}

impl<T> ServiceHandle<'_, T>
where
    T: ?Sized,
{
    fn as_ref(&self) -> &T {
        match self {
            Self::Borrowed(value) => value,
            Self::Shared(value) => value.as_ref(),
        }
    }
}

/// Runtime-specific execution request handed to a registered adapter.
///
/// The dispatcher has already validated the capability descriptor, provider
/// package, runtime kind, and configured backend presence before building this
/// request. Adapters own concrete runtime semantics and resource accounting.
/// If `resource_reservation` is present, the adapter must reconcile or release
/// that prepared reservation instead of creating a second reservation.
pub struct RuntimeAdapterRequest<'a, F, G>
where
    F: RootFilesystem,
    G: ResourceGovernor,
{
    pub package: &'a ExtensionPackage,
    pub descriptor: &'a CapabilityDescriptor,
    pub filesystem: &'a F,
    pub governor: &'a G,
    pub runtime_policy: &'a EffectiveRuntimePolicy,
    pub capability_id: &'a CapabilityId,
    pub scope: ResourceScope,
    pub authenticated_actor_user_id: Option<ironclaw_host_api::UserId>,
    /// Loop turn-run identity forwarded from the dispatch request. `None`
    /// for non-loop callers.
    pub run_id: Option<ironclaw_host_api::RunId>,
    pub estimate: ResourceEstimate,
    pub mounts: Option<MountView>,
    pub resource_reservation: Option<ResourceReservation>,
    pub input: Value,
}

/// Runtime-normalized adapter result before dispatcher adds stable identity fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeAdapterResult {
    pub output: Value,
    pub display_preview: Option<CapabilityDisplayOutputPreview>,
    pub usage: ResourceUsage,
    pub receipt: ResourceReceipt,
    pub output_bytes: u64,
}

/// Per-lane runtime backend execution shape.
///
/// This is the concrete-execution contract each runtime lane
/// (`FirstParty`/`Wasm`/`Mcp`/`Process`) satisfies host-runtime-side. Unlike the
/// former design it is **not** used as a trait object in a per-lane registry:
/// routing now goes through the closed [`RuntimeExecutor`] (arch-simplification
/// §4.2 — a `dyn`→enum collapse of the capability hot path). Implementations are
/// called by static dispatch from the host-runtime lane executor; they must not
/// perform caller-facing authorization or approval resolution, may
/// reserve/reconcile resources through the provided governor, and must surface
/// only redacted [`DispatchError`] categories.
#[async_trait]
pub trait RuntimeAdapter<F, G>: Send + Sync
where
    F: RootFilesystem,
    G: ResourceGovernor,
{
    async fn dispatch_json(
        &self,
        request: RuntimeAdapterRequest<'_, F, G>,
    ) -> Result<RuntimeAdapterResult, DispatchError>;
}

/// Closed runtime-lane router held by [`RuntimeDispatcher`].
///
/// This replaces the former `HashMap<RuntimeKind, dyn RuntimeAdapter>` per-lane
/// vtable registry with a single monomorphized executor resolved once at
/// composition (arch-simplification §4.2, "a single generic parameter resolved
/// once at composition"). The dispatcher holds one `E: RuntimeExecutor` and
/// never dynamic-dispatches per lane. The executor's own `dispatch_json` is the
/// one place a [`RuntimeLane`] is matched exhaustively — adding a lane is a
/// compile error until every arm handles it (§4.2 safety property).
///
/// The dependency boundary is preserved: the executor set (WASM/Script/MCP/
/// first-party) lives host-runtime-side, so this crate gains no concrete-runtime
/// dependency. This crate only owns the port.
#[async_trait]
pub trait RuntimeExecutor<F, G>: Send + Sync
where
    F: RootFilesystem,
    G: ResourceGovernor,
{
    /// True when a backend is wired for `lane`. The dispatcher consults this
    /// **before** emitting `RuntimeSelected`, so an unconfigured lane fails with
    /// a redacted [`DispatchError::MissingRuntimeBackend`] and no reservation —
    /// exactly as the former registry-miss path did.
    fn supports_lane(&self, lane: RuntimeLane) -> bool;

    /// Execute an already-validated request on the resolved `lane`. Called only
    /// after [`Self::supports_lane`] returned true.
    async fn dispatch_json(
        &self,
        lane: RuntimeLane,
        request: RuntimeAdapterRequest<'_, F, G>,
    ) -> Result<RuntimeAdapterResult, DispatchError>;
}

/// Smart-pointer delegation so an `Arc`-shared executor (e.g. one a caller keeps
/// a handle to for inspection) satisfies the bound directly.
#[async_trait]
impl<F, G, T> RuntimeExecutor<F, G> for Arc<T>
where
    F: RootFilesystem,
    G: ResourceGovernor,
    T: RuntimeExecutor<F, G> + ?Sized,
{
    fn supports_lane(&self, lane: RuntimeLane) -> bool {
        (**self).supports_lane(lane)
    }

    async fn dispatch_json(
        &self,
        lane: RuntimeLane,
        request: RuntimeAdapterRequest<'_, F, G>,
    ) -> Result<RuntimeAdapterResult, DispatchError> {
        (**self).dispatch_json(lane, request).await
    }
}

/// Narrow runtime dispatcher over already-discovered extensions and services.
///
/// `E` is the closed [`RuntimeExecutor`] resolved once at composition — the
/// dispatcher routes every capability through it by static dispatch, with no
/// per-lane trait object or vtable map (arch-simplification §4.2).
pub struct RuntimeDispatcher<'a, F, G, E>
where
    F: RootFilesystem,
    G: ResourceGovernor,
    E: RuntimeExecutor<F, G>,
{
    registry: Arc<SharedExtensionRegistry>,
    filesystem: ServiceHandle<'a, F>,
    governor: ServiceHandle<'a, G>,
    runtime_policy: EffectiveRuntimePolicy,
    executor: E,
    event_sink: Option<ServiceHandle<'a, dyn EventSink + 'a>>,
}

impl<'a, F, G, E> RuntimeDispatcher<'a, F, G, E>
where
    F: RootFilesystem,
    G: ResourceGovernor,
    E: RuntimeExecutor<F, G>,
{
    pub fn new(
        registry: &'a ExtensionRegistry,
        filesystem: &'a F,
        governor: &'a G,
        executor: E,
    ) -> Self {
        Self {
            registry: Arc::new(SharedExtensionRegistry::new(registry.clone())),
            filesystem: ServiceHandle::Borrowed(filesystem),
            governor: ServiceHandle::Borrowed(governor),
            runtime_policy: default_runtime_policy(),
            executor,
            event_sink: None,
        }
    }

    pub fn from_arcs(
        registry: Arc<ExtensionRegistry>,
        filesystem: Arc<F>,
        governor: Arc<G>,
        executor: E,
    ) -> RuntimeDispatcher<'static, F, G, E>
    where
        F: 'static,
        G: 'static,
    {
        Self::from_shared_registry(
            Arc::new(SharedExtensionRegistry::new((*registry).clone())),
            filesystem,
            governor,
            executor,
        )
    }

    pub fn from_shared_registry(
        registry: Arc<SharedExtensionRegistry>,
        filesystem: Arc<F>,
        governor: Arc<G>,
        executor: E,
    ) -> RuntimeDispatcher<'static, F, G, E>
    where
        F: 'static,
        G: 'static,
    {
        RuntimeDispatcher {
            registry,
            filesystem: ServiceHandle::Shared(filesystem),
            governor: ServiceHandle::Shared(governor),
            runtime_policy: default_runtime_policy(),
            executor,
            event_sink: None,
        }
    }

    /// Replaces the effective runtime policy passed to runtime adapters.
    ///
    /// The dispatcher does not resolve profiles; callers must supply the
    /// concrete policy that should govern each dispatch request.
    pub fn with_runtime_policy(mut self, runtime_policy: EffectiveRuntimePolicy) -> Self {
        self.runtime_policy = runtime_policy;
        self
    }

    pub fn with_event_sink(mut self, sink: &'a dyn EventSink) -> Self {
        self.event_sink = Some(ServiceHandle::Borrowed(sink));
        self
    }

    pub fn with_event_sink_arc(mut self, sink: Arc<dyn EventSink>) -> Self {
        self.event_sink = Some(ServiceHandle::Shared(sink));
        self
    }

    #[tracing::instrument(
        level = "debug",
        skip(self, request),
        fields(
            capability_id = %request.capability_id,
            scope = ?request.scope,
        )
    )]
    pub async fn dispatch_json(
        &self,
        request: CapabilityDispatchRequest,
    ) -> Result<CapabilityDispatchResult, DispatchError> {
        let mut request = request;
        let scope = request.scope.clone();
        let capability_id = request.capability_id.clone();
        let mut reservation_guard = DispatchReservationGuard::new(
            self.governor.as_ref(),
            request.resource_reservation.take(),
        );
        self.emit_event(RuntimeEvent::dispatch_requested(
            scope.clone(),
            capability_id.clone(),
        ))
        .await?;

        let registry = self.registry.snapshot();
        let descriptor = match registry.get_capability(&request.capability_id).cloned() {
            Some(descriptor) => descriptor,
            None => {
                let error = DispatchError::UnknownCapability {
                    capability: capability_id.clone(),
                };
                self.emit_dispatch_failure(scope, capability_id, None, None, &error)
                    .await?;
                return Err(error);
            }
        };
        let package = match registry.get_extension(&descriptor.provider).cloned() {
            Some(package) => package,
            None => {
                let error = DispatchError::UnknownProvider {
                    capability: capability_id.clone(),
                    provider: descriptor.provider.clone(),
                };
                self.emit_dispatch_failure(
                    scope,
                    capability_id,
                    Some(descriptor.provider.clone()),
                    Some(descriptor.runtime),
                    &error,
                )
                .await?;
                return Err(error);
            }
        };
        let package_runtime = package.manifest.runtime_kind();
        if descriptor.runtime != package_runtime {
            let error = DispatchError::RuntimeMismatch {
                capability: capability_id.clone(),
                descriptor_runtime: descriptor.runtime,
                package_runtime,
            };
            self.emit_dispatch_failure(
                scope,
                capability_id,
                Some(descriptor.provider.clone()),
                Some(descriptor.runtime),
                &error,
            )
            .await?;
            return Err(error);
        }

        let runtime = descriptor.runtime;
        // Resolve the runtime kind (loading taxonomy) to a closed execution
        // lane, then confirm the executor has that lane wired. `System` maps to
        // `None` (host-internal, never an untrusted lane) and an unconfigured
        // lane both fail closed here — before any `RuntimeSelected` event or
        // reservation, so the prepared reservation guard drops and releases.
        let lane = match RuntimeLane::from_runtime_kind(runtime) {
            Some(lane) if self.executor.supports_lane(lane) => lane,
            _ => {
                let error = DispatchError::MissingRuntimeBackend { runtime };
                self.emit_dispatch_failure(
                    scope,
                    capability_id,
                    Some(descriptor.provider.clone()),
                    Some(runtime),
                    &error,
                )
                .await?;
                return Err(error);
            }
        };

        self.emit_event(RuntimeEvent::runtime_selected(
            scope.clone(),
            capability_id.clone(),
            descriptor.provider.clone(),
            runtime,
        ))
        .await?;

        let execution = match self
            .executor
            .dispatch_json(
                lane,
                RuntimeAdapterRequest {
                    package: &package,
                    descriptor: &descriptor,
                    filesystem: self.filesystem.as_ref(),
                    governor: self.governor.as_ref(),
                    runtime_policy: &self.runtime_policy,
                    capability_id: &request.capability_id,
                    scope: request.scope,
                    authenticated_actor_user_id: request.authenticated_actor_user_id,
                    run_id: request.run_id,
                    estimate: request.estimate,
                    mounts: request.mounts,
                    resource_reservation: reservation_guard.take(),
                    input: request.input,
                },
            )
            .await
        {
            Ok(execution) => execution,
            Err(error) => {
                self.emit_dispatch_failure(
                    scope,
                    capability_id,
                    Some(descriptor.provider.clone()),
                    Some(runtime),
                    &error,
                )
                .await?;
                return Err(error);
            }
        };

        self.emit_event(RuntimeEvent::dispatch_succeeded(
            scope,
            capability_id.clone(),
            descriptor.provider.clone(),
            runtime,
            execution.output_bytes,
        ))
        .await?;

        Ok(CapabilityDispatchResult {
            capability_id,
            provider: descriptor.provider.clone(),
            runtime,
            output: execution.output,
            display_preview: execution.display_preview,
            usage: execution.usage,
            receipt: execution.receipt,
        })
    }

    async fn emit_dispatch_failure(
        &self,
        scope: ResourceScope,
        capability_id: CapabilityId,
        provider: Option<ExtensionId>,
        runtime: Option<RuntimeKind>,
        error: &DispatchError,
    ) -> Result<(), DispatchError> {
        self.emit_event(RuntimeEvent::dispatch_failed(
            scope,
            capability_id,
            provider,
            runtime,
            error.event_kind(),
        ))
        .await
    }

    async fn emit_event(&self, event: RuntimeEvent) -> Result<(), DispatchError> {
        tracing::debug!(
            event_kind = ?event.kind,
            capability_id = %event.capability_id,
            provider = event.provider.as_ref().map(|provider| provider.as_str()).unwrap_or(""),
            runtime = ?event.runtime,
            output_bytes = event.output_bytes,
            error_kind = event.error_kind.as_deref().unwrap_or(""),
            "runtime dispatcher observed event"
        );
        if let Some(sink) = self.event_sink.as_ref() {
            let _ = sink.as_ref().emit(event).await;
        }
        Ok(())
    }
}

struct DispatchReservationGuard<'a, G>
where
    G: ResourceGovernor,
{
    governor: &'a G,
    reservation: Option<ResourceReservation>,
}

impl<'a, G> DispatchReservationGuard<'a, G>
where
    G: ResourceGovernor,
{
    fn new(governor: &'a G, reservation: Option<ResourceReservation>) -> Self {
        Self {
            governor,
            reservation,
        }
    }

    fn take(&mut self) -> Option<ResourceReservation> {
        self.reservation.take()
    }
}

impl<G> Drop for DispatchReservationGuard<'_, G>
where
    G: ResourceGovernor,
{
    fn drop(&mut self) {
        if let Some(reservation) = &self.reservation
            && let Err(error) = self.governor.release(reservation.id)
        {
            tracing::warn!(
                reservation_id = %reservation.id,
                error = %error,
                "failed to release prepared resource reservation after dispatcher validation failure"
            );
        }
    }
}

fn default_runtime_policy() -> EffectiveRuntimePolicy {
    EffectiveRuntimePolicy {
        deployment: DeploymentMode::LocalSingleUser,
        requested_profile: RuntimeProfile::SecureDefault,
        resolved_profile: RuntimeProfile::SecureDefault,
        filesystem_backend: FilesystemBackendKind::ScopedVirtual,
        process_backend: ProcessBackendKind::None,
        network_mode: NetworkMode::Deny,
        secret_mode: SecretMode::BrokeredHandles,
        approval_policy: ApprovalPolicy::AskAlways,
        audit_mode: AuditMode::LocalMinimal,
    }
}

#[async_trait]
impl<F, G, E> CapabilityDispatcher for RuntimeDispatcher<'_, F, G, E>
where
    F: RootFilesystem,
    G: ResourceGovernor,
    E: RuntimeExecutor<F, G>,
{
    async fn dispatch_json(
        &self,
        request: CapabilityDispatchRequest,
    ) -> Result<CapabilityDispatchResult, DispatchError> {
        RuntimeDispatcher::dispatch_json(self, request).await
    }
}

// Removed: dispatch_error_kind was a local copy of DispatchError::event_kind() from ironclaw_host_api.
// Call error.event_kind() directly instead.
