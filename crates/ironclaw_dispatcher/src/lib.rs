//! Composition-only runtime dispatch contracts for IronClaw Reborn.
//!
//! `ironclaw_dispatcher` routes already-authorized capability invocations to
//! prebound adapters resolved by capability id through the injected
//! [`ToolResolver`]. It does not select packages or runtime kinds, parse
//! extension manifests, implement sandbox policy, reserve budget itself, or
//! execute product workflows. Binding construction (which adapter serves a
//! capability, with which package, plan, and ports) happens at
//! activation/registration time in the resolver implementations: the host
//! built-in registry resolver in `ironclaw_host_runtime` and the
//! active-snapshot resolver over `ironclaw_extension_host`.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_events::{EventSink, RuntimeEvent};
pub use ironclaw_host_api::{
    CapabilityDispatchRequest, CapabilityDispatchResult, CapabilityDispatcher,
    CapabilityDisplayOutputPreview, DispatchError, RuntimeDispatchErrorKind,
};
use ironclaw_host_api::{
    CapabilityId, ExtensionId, ResourceReceipt, ResourceReservation, ResourceScope, ResourceUsage,
    RuntimeKind,
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

/// Resolve a prebound capability binding by capability id (TOOL-1).
///
/// Implementations are snapshot-shaped: resolution is a lookup into bindings
/// constructed at activation/registration time, never a per-invocation
/// package/runtime-kind selection. An unknown id returns `None` and dispatch
/// fails before any adapter work (TOOL-2).
pub trait ToolResolver: Send + Sync {
    fn resolve(&self, capability_id: &CapabilityId) -> Option<ResolvedCapability>;
}

/// One prebound, ready-to-invoke capability binding.
#[derive(Clone)]
pub struct ResolvedCapability {
    /// The owning extension (host built-ins resolve as the synthetic
    /// `builtin` provider).
    pub provider: ExtensionId,
    /// The implementation lane, carried for dispatch events and results;
    /// selection already happened when the binding was constructed.
    pub runtime: RuntimeKind,
    pub adapter: Arc<dyn BoundCapabilityAdapter>,
}

/// A prebound capability implementation behind [`ToolResolver`].
///
/// The adapter receives the already-authorized [`CapabilityDispatchRequest`]
/// unchanged — everything static (package, descriptor, execution plan,
/// filesystem, ports) was captured when the binding was constructed. If
/// `resource_reservation` is present, the binding owns the
/// reconcile-or-release leg for it (same legs as the runtime lanes always
/// had); the dispatcher only releases a reservation when resolution fails
/// before any binding takes it.
///
/// Implementations must not perform caller-facing authorization or approval
/// resolution and must surface only redacted [`DispatchError`] categories.
#[async_trait]
pub trait BoundCapabilityAdapter: Send + Sync {
    async fn dispatch_json(
        &self,
        request: CapabilityDispatchRequest,
    ) -> Result<RuntimeAdapterResult, DispatchError>;
}

/// Runtime-normalized adapter result before the dispatcher adds stable
/// identity fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeAdapterResult {
    pub output: Value,
    pub display_preview: Option<CapabilityDisplayOutputPreview>,
    pub usage: ResourceUsage,
    pub receipt: ResourceReceipt,
    pub output_bytes: u64,
}

/// First-`Some`-wins composition of resolvers (host built-ins chained with
/// the active extension snapshot).
pub struct ChainToolResolver {
    resolvers: Vec<Arc<dyn ToolResolver>>,
}

impl ChainToolResolver {
    pub fn new(resolvers: Vec<Arc<dyn ToolResolver>>) -> Self {
        Self { resolvers }
    }
}

impl ToolResolver for ChainToolResolver {
    fn resolve(&self, capability_id: &CapabilityId) -> Option<ResolvedCapability> {
        self.resolvers
            .iter()
            .find_map(|resolver| resolver.resolve(capability_id))
    }
}

/// Narrow runtime dispatcher over prebound capability bindings.
pub struct RuntimeDispatcher<'a, G>
where
    G: ResourceGovernor,
{
    resolver: ServiceHandle<'a, dyn ToolResolver + 'a>,
    governor: ServiceHandle<'a, G>,
    event_sink: Option<ServiceHandle<'a, dyn EventSink + 'a>>,
}

impl<'a, G> RuntimeDispatcher<'a, G>
where
    G: ResourceGovernor,
{
    pub fn new(resolver: &'a dyn ToolResolver, governor: &'a G) -> Self {
        Self {
            resolver: ServiceHandle::Borrowed(resolver),
            governor: ServiceHandle::Borrowed(governor),
            event_sink: None,
        }
    }

    pub fn from_arcs(
        resolver: Arc<dyn ToolResolver>,
        governor: Arc<G>,
    ) -> RuntimeDispatcher<'static, G>
    where
        G: 'static,
    {
        RuntimeDispatcher {
            resolver: ServiceHandle::Shared(resolver),
            governor: ServiceHandle::Shared(governor),
            event_sink: None,
        }
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

        let Some(resolved) = self.resolver.as_ref().resolve(&request.capability_id) else {
            let error = DispatchError::UnknownCapability {
                capability: capability_id.clone(),
            };
            self.emit_dispatch_failure(scope, capability_id, None, None, &error)
                .await?;
            return Err(error);
        };
        let provider = resolved.provider.clone();
        let runtime = resolved.runtime;

        // Bind dispatch to the lane the authorization pinned into the witness
        // (§5.3.2, mutable-registry TOCTOU). The descriptor + lane resolved above
        // came from a FRESH `self.registry.snapshot()`, so an extension update
        // between authorize and dispatch can move the descriptor onto a different
        // runtime lane. If the authorization pinned a lane, fail closed on a
        // mismatch — never execute a replacement runtime on mounts, a reservation,
        // and trust prepared for the old lane. A `None` pin (witness-free/legacy
        // dispatch, or a host-internal `System` descriptor with no lane) skips the
        // check, so the common path is byte-identical. Same-lane descriptor-content
        // swaps (which keep this lane equal) are the residual gap tracked in #6434.
        if let Some(pinned) = request.pinned_lane
            && pinned != lane
        {
            let error = DispatchError::LaneMismatch {
                capability: capability_id.clone(),
                authorized: pinned,
                resolved: lane,
            };
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

        self.emit_event(RuntimeEvent::runtime_selected(
            scope.clone(),
            capability_id.clone(),
            provider.clone(),
            runtime,
        ))
        .await?;

        let execution = match resolved
            .adapter
            .dispatch_json(CapabilityDispatchRequest {
                resource_reservation: reservation_guard.take(),
                ..request
            })
            .await
        {
            Ok(execution) => execution,
            Err(error) => {
                self.emit_dispatch_failure(
                    scope,
                    capability_id,
                    Some(provider),
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
            provider.clone(),
            runtime,
            execution.output_bytes,
        ))
        .await?;

        Ok(CapabilityDispatchResult {
            capability_id,
            provider,
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

#[async_trait]
impl<G> CapabilityDispatcher for RuntimeDispatcher<'_, G>
where
    G: ResourceGovernor,
{
    async fn dispatch_json(
        &self,
        request: CapabilityDispatchRequest,
    ) -> Result<CapabilityDispatchResult, DispatchError> {
        RuntimeDispatcher::dispatch_json(self, request).await
    }
}
