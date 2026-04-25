//! Composition-only runtime dispatch contracts for IronClaw Reborn.
//!
//! `ironclaw_dispatcher` wires validated extension descriptors to runtime lanes. It
//! does not parse extension manifests, implement sandbox policy, reserve budget
//! itself, or execute product workflows. Those responsibilities stay in the
//! owning service crates.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_events::{EventSink, RuntimeEvent};
use ironclaw_extensions::ExtensionRegistry;
use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::{
    CapabilityDispatchError, CapabilityDispatchFailureKind, CapabilityDispatchRequest,
    CapabilityDispatchResult, CapabilityDispatcher, CapabilityId, ExtensionId, ResourceScope,
    RuntimeKind,
};
use ironclaw_mcp::{McpExecutionRequest, McpExecutor, McpInvocation};
use ironclaw_resources::ResourceGovernor;
use ironclaw_scripts::{ScriptExecutionRequest, ScriptExecutor, ScriptInvocation};
use ironclaw_wasm::{CapabilityInvocation, WasmExecutionRequest, WasmRuntime};

pub type DispatchError = CapabilityDispatchError;

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

/// Narrow runtime dispatcher over already-discovered extensions and services.
pub struct RuntimeDispatcher<'a, F, G>
where
    F: RootFilesystem,
    G: ResourceGovernor,
{
    registry: ServiceHandle<'a, ExtensionRegistry>,
    filesystem: ServiceHandle<'a, F>,
    governor: ServiceHandle<'a, G>,
    wasm_runtime: Option<ServiceHandle<'a, WasmRuntime>>,
    script_runtime: Option<ServiceHandle<'a, dyn ScriptExecutor + 'a>>,
    mcp_runtime: Option<ServiceHandle<'a, dyn McpExecutor + 'a>>,
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
            wasm_runtime: None,
            script_runtime: None,
            mcp_runtime: None,
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
            wasm_runtime: None,
            script_runtime: None,
            mcp_runtime: None,
            event_sink: None,
        }
    }

    pub fn with_wasm_runtime(mut self, runtime: &'a WasmRuntime) -> Self {
        self.wasm_runtime = Some(ServiceHandle::Borrowed(runtime));
        self
    }

    pub fn with_wasm_runtime_arc(mut self, runtime: Arc<WasmRuntime>) -> Self {
        self.wasm_runtime = Some(ServiceHandle::Shared(runtime));
        self
    }

    pub fn with_script_runtime(mut self, runtime: &'a dyn ScriptExecutor) -> Self {
        self.script_runtime = Some(ServiceHandle::Borrowed(runtime));
        self
    }

    pub fn with_script_runtime_arc(mut self, runtime: Arc<dyn ScriptExecutor>) -> Self {
        self.script_runtime = Some(ServiceHandle::Shared(runtime));
        self
    }

    pub fn with_mcp_runtime(mut self, runtime: &'a dyn McpExecutor) -> Self {
        self.mcp_runtime = Some(ServiceHandle::Borrowed(runtime));
        self
    }

    pub fn with_mcp_runtime_arc(mut self, runtime: Arc<dyn McpExecutor>) -> Self {
        self.mcp_runtime = Some(ServiceHandle::Shared(runtime));
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
    ) -> Result<CapabilityDispatchResult, CapabilityDispatchError> {
        let scope = request.scope.clone();
        let capability_id = request.capability_id.clone();
        self.emit_event(RuntimeEvent::dispatch_requested(
            scope.clone(),
            capability_id.clone(),
        ))
        .await;

        let descriptor = match self
            .registry
            .as_ref()
            .get_capability(&request.capability_id)
        {
            Some(descriptor) => descriptor,
            None => {
                let error = CapabilityDispatchError::new(
                    CapabilityDispatchFailureKind::UnknownCapability,
                    request.capability_id,
                    None,
                    None,
                );
                self.emit_dispatch_failure(scope, capability_id, None, None, &error)
                    .await;
                return Err(error);
            }
        };
        let package = match self.registry.as_ref().get_extension(&descriptor.provider) {
            Some(package) => package,
            None => {
                let error = CapabilityDispatchError::new(
                    CapabilityDispatchFailureKind::UnknownProvider,
                    request.capability_id,
                    Some(descriptor.provider.clone()),
                    Some(descriptor.runtime),
                );
                self.emit_dispatch_failure(
                    scope,
                    capability_id,
                    Some(descriptor.provider.clone()),
                    Some(descriptor.runtime),
                    &error,
                )
                .await;
                return Err(error);
            }
        };
        let package_runtime = package.manifest.runtime_kind();
        if descriptor.runtime != package_runtime {
            let error = CapabilityDispatchError::new(
                CapabilityDispatchFailureKind::RuntimeMismatch,
                request.capability_id,
                Some(descriptor.provider.clone()),
                Some(descriptor.runtime),
            );
            self.emit_dispatch_failure(
                scope,
                capability_id,
                Some(descriptor.provider.clone()),
                Some(descriptor.runtime),
                &error,
            )
            .await;
            return Err(error);
        }

        match descriptor.runtime {
            RuntimeKind::Wasm => {
                let Some(wasm_runtime) = self.wasm_runtime.as_ref() else {
                    let error = CapabilityDispatchError::new(
                        CapabilityDispatchFailureKind::MissingRuntimeBackend,
                        request.capability_id.clone(),
                        Some(descriptor.provider.clone()),
                        Some(RuntimeKind::Wasm),
                    );
                    self.emit_dispatch_failure(
                        scope,
                        capability_id,
                        Some(descriptor.provider.clone()),
                        Some(RuntimeKind::Wasm),
                        &error,
                    )
                    .await;
                    return Err(error);
                };
                self.emit_event(RuntimeEvent::runtime_selected(
                    scope.clone(),
                    capability_id.clone(),
                    descriptor.provider.clone(),
                    RuntimeKind::Wasm,
                ))
                .await;

                let execution = match wasm_runtime
                    .as_ref()
                    .execute_extension_json(
                        self.filesystem.as_ref(),
                        self.governor.as_ref(),
                        WasmExecutionRequest {
                            package,
                            capability_id: &request.capability_id,
                            scope: request.scope,
                            estimate: request.estimate,
                            invocation: CapabilityInvocation {
                                input: request.input,
                            },
                        },
                    )
                    .await
                {
                    Ok(execution) => execution,
                    Err(_error) => {
                        let error = runtime_dispatch_error(
                            &capability_id,
                            &descriptor.provider,
                            RuntimeKind::Wasm,
                            CapabilityDispatchFailureKind::Wasm,
                        );
                        self.emit_dispatch_failure(
                            scope,
                            capability_id,
                            Some(descriptor.provider.clone()),
                            Some(RuntimeKind::Wasm),
                            &error,
                        )
                        .await;
                        return Err(error);
                    }
                };
                let output_bytes = execution.result.output_bytes;
                self.emit_event(RuntimeEvent::dispatch_succeeded(
                    scope,
                    capability_id.clone(),
                    descriptor.provider.clone(),
                    RuntimeKind::Wasm,
                    output_bytes,
                ))
                .await;

                Ok(CapabilityDispatchResult {
                    capability_id,
                    provider: descriptor.provider.clone(),
                    runtime: RuntimeKind::Wasm,
                    output: execution.result.output,
                    usage: execution.result.usage,
                    receipt: execution.receipt,
                })
            }
            RuntimeKind::Script => {
                let Some(script_runtime) = self.script_runtime.as_ref() else {
                    let error = CapabilityDispatchError::new(
                        CapabilityDispatchFailureKind::MissingRuntimeBackend,
                        request.capability_id.clone(),
                        Some(descriptor.provider.clone()),
                        Some(RuntimeKind::Script),
                    );
                    self.emit_dispatch_failure(
                        scope,
                        capability_id,
                        Some(descriptor.provider.clone()),
                        Some(RuntimeKind::Script),
                        &error,
                    )
                    .await;
                    return Err(error);
                };
                self.emit_event(RuntimeEvent::runtime_selected(
                    scope.clone(),
                    capability_id.clone(),
                    descriptor.provider.clone(),
                    RuntimeKind::Script,
                ))
                .await;

                let execution = match script_runtime.as_ref().execute_extension_json(
                    self.governor.as_ref(),
                    ScriptExecutionRequest {
                        package,
                        capability_id: &request.capability_id,
                        scope: request.scope,
                        estimate: request.estimate,
                        invocation: ScriptInvocation {
                            input: request.input,
                        },
                    },
                ) {
                    Ok(execution) => execution,
                    Err(_error) => {
                        let error = runtime_dispatch_error(
                            &capability_id,
                            &descriptor.provider,
                            RuntimeKind::Script,
                            CapabilityDispatchFailureKind::Script,
                        );
                        self.emit_dispatch_failure(
                            scope,
                            capability_id,
                            Some(descriptor.provider.clone()),
                            Some(RuntimeKind::Script),
                            &error,
                        )
                        .await;
                        return Err(error);
                    }
                };
                let output_bytes = execution.result.output_bytes;
                self.emit_event(RuntimeEvent::dispatch_succeeded(
                    scope,
                    capability_id.clone(),
                    descriptor.provider.clone(),
                    RuntimeKind::Script,
                    output_bytes,
                ))
                .await;

                Ok(CapabilityDispatchResult {
                    capability_id,
                    provider: descriptor.provider.clone(),
                    runtime: RuntimeKind::Script,
                    output: execution.result.output,
                    usage: execution.result.usage,
                    receipt: execution.receipt,
                })
            }
            RuntimeKind::Mcp => {
                let Some(mcp_runtime) = self.mcp_runtime.as_ref() else {
                    let error = CapabilityDispatchError::new(
                        CapabilityDispatchFailureKind::MissingRuntimeBackend,
                        request.capability_id.clone(),
                        Some(descriptor.provider.clone()),
                        Some(RuntimeKind::Mcp),
                    );
                    self.emit_dispatch_failure(
                        scope,
                        capability_id,
                        Some(descriptor.provider.clone()),
                        Some(RuntimeKind::Mcp),
                        &error,
                    )
                    .await;
                    return Err(error);
                };
                self.emit_event(RuntimeEvent::runtime_selected(
                    scope.clone(),
                    capability_id.clone(),
                    descriptor.provider.clone(),
                    RuntimeKind::Mcp,
                ))
                .await;

                let execution = match mcp_runtime
                    .as_ref()
                    .execute_extension_json(
                        self.governor.as_ref(),
                        McpExecutionRequest {
                            package,
                            capability_id: &request.capability_id,
                            scope: request.scope,
                            estimate: request.estimate,
                            invocation: McpInvocation {
                                input: request.input,
                            },
                        },
                    )
                    .await
                {
                    Ok(execution) => execution,
                    Err(_error) => {
                        let error = runtime_dispatch_error(
                            &capability_id,
                            &descriptor.provider,
                            RuntimeKind::Mcp,
                            CapabilityDispatchFailureKind::Mcp,
                        );
                        self.emit_dispatch_failure(
                            scope,
                            capability_id,
                            Some(descriptor.provider.clone()),
                            Some(RuntimeKind::Mcp),
                            &error,
                        )
                        .await;
                        return Err(error);
                    }
                };
                let output_bytes = execution.result.output_bytes;
                self.emit_event(RuntimeEvent::dispatch_succeeded(
                    scope,
                    capability_id.clone(),
                    descriptor.provider.clone(),
                    RuntimeKind::Mcp,
                    output_bytes,
                ))
                .await;

                Ok(CapabilityDispatchResult {
                    capability_id,
                    provider: descriptor.provider.clone(),
                    runtime: RuntimeKind::Mcp,
                    output: execution.result.output,
                    usage: execution.result.usage,
                    receipt: execution.receipt,
                })
            }
            runtime @ (RuntimeKind::FirstParty | RuntimeKind::System) => {
                let error = CapabilityDispatchError::new(
                    CapabilityDispatchFailureKind::UnsupportedRuntime,
                    request.capability_id,
                    Some(descriptor.provider.clone()),
                    Some(runtime),
                );
                self.emit_dispatch_failure(
                    scope,
                    capability_id,
                    Some(descriptor.provider.clone()),
                    Some(runtime),
                    &error,
                )
                .await;
                Err(error)
            }
        }
    }

    async fn emit_dispatch_failure(
        &self,
        scope: ResourceScope,
        capability_id: CapabilityId,
        provider: Option<ExtensionId>,
        runtime: Option<RuntimeKind>,
        error: &DispatchError,
    ) {
        self.emit_event(RuntimeEvent::dispatch_failed(
            scope,
            capability_id,
            provider,
            runtime,
            error.error_kind(),
        ))
        .await;
    }

    async fn emit_event(&self, event: RuntimeEvent) {
        if let Some(sink) = self.event_sink.as_ref() {
            let _ = sink.as_ref().emit(event).await;
        }
    }
}

fn runtime_dispatch_error(
    capability_id: &CapabilityId,
    provider: &ExtensionId,
    runtime: RuntimeKind,
    kind: CapabilityDispatchFailureKind,
) -> CapabilityDispatchError {
    CapabilityDispatchError::new(
        kind,
        capability_id.clone(),
        Some(provider.clone()),
        Some(runtime),
    )
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
    ) -> Result<CapabilityDispatchResult, CapabilityDispatchError> {
        RuntimeDispatcher::dispatch_json(self, request).await
    }
}
