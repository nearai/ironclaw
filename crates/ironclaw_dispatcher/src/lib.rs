//! Composition-only runtime dispatch contracts for IronClaw Reborn.
//!
//! `ironclaw_dispatcher` wires validated extension descriptors to runtime lanes. It
//! does not parse extension manifests, implement sandbox policy, reserve budget
//! itself, or execute product workflows. Those responsibilities stay in the
//! owning service crates.

use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use ironclaw_events::{EventSink, RuntimeEvent};
use ironclaw_extensions::{ExtensionPackage, ExtensionRegistry};
use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::{
    CapabilityDescriptor, CapabilityId, ExtensionId, MountView, ResourceEstimate, ResourceReceipt,
    ResourceReservation, ResourceScope, ResourceUsage, RuntimeKind,
};
pub use ironclaw_host_api::{
    CapabilityDispatchRequest, CapabilityDispatchResult, CapabilityDispatcher, DispatchError,
    RuntimeDispatchErrorKind,
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
    pub capability_id: &'a CapabilityId,
    pub scope: ResourceScope,
    pub estimate: ResourceEstimate,
    pub mounts: Option<MountView>,
    pub resource_reservation: Option<ResourceReservation>,
    pub input: Value,
}

/// Runtime-normalized adapter result before dispatcher adds stable identity fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeAdapterResult {
    pub output: Value,
    pub usage: ResourceUsage,
    pub receipt: ResourceReceipt,
    pub output_bytes: u64,
}

/// Runtime backend adapter used by [`RuntimeDispatcher`].
///
/// Implementations must not perform caller-facing authorization or approval
/// resolution. They may reserve/reconcile resources through the provided
/// governor and must surface only redacted [`DispatchError`] categories.
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

/// Narrow runtime dispatcher over already-discovered extensions and services.
pub struct RuntimeDispatcher<'a, F, G>
where
    F: RootFilesystem,
    G: ResourceGovernor,
{
    registry: ServiceHandle<'a, ExtensionRegistry>,
    filesystem: ServiceHandle<'a, F>,
    governor: ServiceHandle<'a, G>,
    runtime_adapters: HashMap<RuntimeKind, ServiceHandle<'a, dyn RuntimeAdapter<F, G> + 'a>>,
    event_sink: Option<ServiceHandle<'a, dyn EventSink + 'a>>,
}

impl<'a, F, G> RuntimeDispatcher<'a, F, G>
where
    F: RootFilesystem,
    G: ResourceGovernor,
{
    pub fn new(registry: &'a ExtensionRegistry, filesystem: &'a F, governor: &'a G) -> Self {
        Self {
            registry: ServiceHandle::Borrowed(registry),
            filesystem: ServiceHandle::Borrowed(filesystem),
            governor: ServiceHandle::Borrowed(governor),
            runtime_adapters: HashMap::new(),
            event_sink: None,
        }
    }

    pub fn from_arcs(
        registry: Arc<ExtensionRegistry>,
        filesystem: Arc<F>,
        governor: Arc<G>,
    ) -> RuntimeDispatcher<'static, F, G>
    where
        F: 'static,
        G: 'static,
    {
        RuntimeDispatcher {
            registry: ServiceHandle::Shared(registry),
            filesystem: ServiceHandle::Shared(filesystem),
            governor: ServiceHandle::Shared(governor),
            runtime_adapters: HashMap::new(),
            event_sink: None,
        }
    }

    pub fn with_runtime_adapter<T>(mut self, runtime: RuntimeKind, adapter: &'a T) -> Self
    where
        T: RuntimeAdapter<F, G> + 'a,
    {
        let adapter: &'a (dyn RuntimeAdapter<F, G> + 'a) = adapter;
        self.runtime_adapters
            .insert(runtime, ServiceHandle::Borrowed(adapter));
        self
    }

    pub fn with_runtime_adapter_arc<T>(mut self, runtime: RuntimeKind, adapter: Arc<T>) -> Self
    where
        T: RuntimeAdapter<F, G> + 'static,
        F: 'static,
        G: 'static,
    {
        let adapter: Arc<dyn RuntimeAdapter<F, G>> = adapter;
        self.runtime_adapters
            .insert(runtime, ServiceHandle::Shared(adapter));
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

    pub async fn dispatch_json(
        &self,
        request: CapabilityDispatchRequest,
    ) -> Result<CapabilityDispatchResult, DispatchError> {
        let scope = request.scope.clone();
        let capability_id = request.capability_id.clone();
        self.emit_event(RuntimeEvent::dispatch_requested(
            scope.clone(),
            capability_id.clone(),
        ))
        .await?;

        let descriptor = match self
            .registry
            .as_ref()
            .get_capability(&request.capability_id)
        {
            Some(descriptor) => descriptor,
            None => {
                let error = DispatchError::UnknownCapability {
                    capability: capability_id.clone(),
                };
                self.release_request_reservation(&request);
                self.emit_dispatch_failure(scope, capability_id, None, None, &error)
                    .await?;
                return Err(error);
            }
        };
        let package = match self.registry.as_ref().get_extension(&descriptor.provider) {
            Some(package) => package,
            None => {
                let error = DispatchError::UnknownProvider {
                    capability: capability_id.clone(),
                    provider: descriptor.provider.clone(),
                };
                self.release_request_reservation(&request);
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
            self.release_request_reservation(&request);
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
        let Some(adapter) = self.runtime_adapters.get(&runtime) else {
            let error = DispatchError::MissingRuntimeBackend { runtime };
            self.release_request_reservation(&request);
            self.emit_dispatch_failure(
                scope,
                capability_id,
                Some(descriptor.provider.clone()),
                Some(runtime),
                &error,
            )
            .await?;
            return Err(error);
        };

        self.emit_event(RuntimeEvent::runtime_selected(
            scope.clone(),
            capability_id.clone(),
            descriptor.provider.clone(),
            runtime,
        ))
        .await?;

        let execution = match adapter
            .as_ref()
            .dispatch_json(RuntimeAdapterRequest {
                package,
                descriptor,
                filesystem: self.filesystem.as_ref(),
                governor: self.governor.as_ref(),
                capability_id: &request.capability_id,
                scope: request.scope,
                estimate: request.estimate,
                mounts: request.mounts,
                resource_reservation: request.resource_reservation,
                input: request.input,
            })
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
            usage: execution.usage,
            receipt: execution.receipt,
        })
    }

    fn release_request_reservation(&self, request: &CapabilityDispatchRequest) {
        if let Some(reservation) = &request.resource_reservation
            && let Err(error) = self.governor.as_ref().release(reservation.id)
        {
            tracing::warn!(
                reservation_id = %reservation.id,
                error = %error,
                "failed to release prepared resource reservation after dispatcher validation failure"
            );
        }
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
            dispatch_error_kind(error),
        ))
        .await
    }

    async fn emit_event(&self, event: RuntimeEvent) -> Result<(), DispatchError> {
        if let Some(sink) = self.event_sink.as_ref() {
            let _ = sink.as_ref().emit(event).await;
        }
        Ok(())
    }
}

#[async_trait]
impl<F, G> CapabilityDispatcher for RuntimeDispatcher<'_, F, G>
where
    F: RootFilesystem,
    G: ResourceGovernor,
{
    async fn dispatch_json(
        &self,
        request: CapabilityDispatchRequest,
    ) -> Result<CapabilityDispatchResult, DispatchError> {
        RuntimeDispatcher::dispatch_json(self, request).await
    }
}

fn dispatch_error_kind(error: &DispatchError) -> &'static str {
    match error {
        DispatchError::UnknownCapability { .. } => "unknown_capability",
        DispatchError::UnknownProvider { .. } => "unknown_provider",
        DispatchError::RuntimeMismatch { .. } => "runtime_mismatch",
        DispatchError::MissingRuntimeBackend { .. } => "missing_runtime_backend",
        DispatchError::UnsupportedRuntime { .. } => "unsupported_runtime",
        DispatchError::Mcp { kind }
        | DispatchError::Script { kind }
        | DispatchError::Wasm { kind } => kind.event_kind(),
    }
}
