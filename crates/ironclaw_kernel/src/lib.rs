//! Composition-only runtime dispatch contracts for IronClaw Reborn.
//!
//! `ironclaw_kernel` wires validated extension descriptors to runtime lanes. It
//! does not parse extension manifests, implement sandbox policy, reserve budget
//! itself, or execute product workflows. Those responsibilities stay in the
//! owning service crates.

use ironclaw_events::{EventError, EventSink, RuntimeEvent};
use ironclaw_extensions::ExtensionRegistry;
use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::{
    CapabilityId, ExtensionId, ResourceEstimate, ResourceScope, ResourceUsage, RuntimeKind,
};
use ironclaw_mcp::{McpError, McpExecutionRequest, McpExecutor, McpInvocation};
use ironclaw_resources::{ResourceGovernor, ResourceReceipt};
use ironclaw_scripts::{ScriptError, ScriptExecutionRequest, ScriptExecutor, ScriptInvocation};
use ironclaw_wasm::{CapabilityInvocation, WasmError, WasmExecutionRequest, WasmRuntime};
use serde_json::Value;
use thiserror::Error;

/// Request/response dispatch request for one declared capability.
#[derive(Debug, Clone, PartialEq)]
pub struct CapabilityDispatchRequest {
    pub capability_id: CapabilityId,
    pub scope: ResourceScope,
    pub estimate: ResourceEstimate,
    pub input: Value,
}

/// Normalized dispatch result returned by the composition layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityDispatchResult {
    pub capability_id: CapabilityId,
    pub provider: ExtensionId,
    pub runtime: RuntimeKind,
    pub output: Value,
    pub usage: ResourceUsage,
    pub receipt: ResourceReceipt,
}

/// Runtime dispatch failures.
#[derive(Debug, Error)]
pub enum DispatchError {
    #[error("unknown capability {capability}")]
    UnknownCapability { capability: CapabilityId },
    #[error("capability {capability} provider {provider} is not registered")]
    UnknownProvider {
        capability: CapabilityId,
        provider: ExtensionId,
    },
    #[error(
        "capability {capability} descriptor runtime {descriptor_runtime:?} does not match package runtime {package_runtime:?}"
    )]
    RuntimeMismatch {
        capability: CapabilityId,
        descriptor_runtime: RuntimeKind,
        package_runtime: RuntimeKind,
    },
    #[error("runtime backend {runtime:?} is not configured")]
    MissingRuntimeBackend { runtime: RuntimeKind },
    #[error(
        "runtime {runtime:?} is recognized but not supported by this dispatcher yet for capability {capability}"
    )]
    UnsupportedRuntime {
        capability: CapabilityId,
        runtime: RuntimeKind,
    },
    #[error("event sink failed: {0}")]
    Event(Box<EventError>),
    #[error("MCP dispatch failed: {0}")]
    Mcp(Box<McpError>),
    #[error("script dispatch failed: {0}")]
    Script(Box<ScriptError>),
    #[error("WASM dispatch failed: {0}")]
    Wasm(Box<WasmError>),
}

impl From<EventError> for DispatchError {
    fn from(error: EventError) -> Self {
        Self::Event(Box::new(error))
    }
}

impl From<McpError> for DispatchError {
    fn from(error: McpError) -> Self {
        Self::Mcp(Box::new(error))
    }
}

impl From<ScriptError> for DispatchError {
    fn from(error: ScriptError) -> Self {
        Self::Script(Box::new(error))
    }
}

impl From<WasmError> for DispatchError {
    fn from(error: WasmError) -> Self {
        Self::Wasm(Box::new(error))
    }
}

/// Narrow runtime dispatcher over already-discovered extensions and services.
pub struct RuntimeDispatcher<'a, F, G>
where
    F: RootFilesystem,
    G: ResourceGovernor,
{
    registry: &'a ExtensionRegistry,
    filesystem: &'a F,
    governor: &'a G,
    wasm_runtime: Option<&'a WasmRuntime>,
    script_runtime: Option<&'a dyn ScriptExecutor>,
    mcp_runtime: Option<&'a dyn McpExecutor>,
    event_sink: Option<&'a dyn EventSink>,
}

impl<'a, F, G> RuntimeDispatcher<'a, F, G>
where
    F: RootFilesystem,
    G: ResourceGovernor,
{
    pub fn new(registry: &'a ExtensionRegistry, filesystem: &'a F, governor: &'a G) -> Self {
        Self {
            registry,
            filesystem,
            governor,
            wasm_runtime: None,
            script_runtime: None,
            mcp_runtime: None,
            event_sink: None,
        }
    }

    pub fn with_wasm_runtime(mut self, runtime: &'a WasmRuntime) -> Self {
        self.wasm_runtime = Some(runtime);
        self
    }

    pub fn with_script_runtime(mut self, runtime: &'a dyn ScriptExecutor) -> Self {
        self.script_runtime = Some(runtime);
        self
    }

    pub fn with_mcp_runtime(mut self, runtime: &'a dyn McpExecutor) -> Self {
        self.mcp_runtime = Some(runtime);
        self
    }

    pub fn with_event_sink(mut self, sink: &'a dyn EventSink) -> Self {
        self.event_sink = Some(sink);
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
        .await;

        let descriptor = match self.registry.get_capability(&request.capability_id) {
            Some(descriptor) => descriptor,
            None => {
                let error = DispatchError::UnknownCapability {
                    capability: request.capability_id,
                };
                self.emit_dispatch_failure(scope, capability_id, None, None, &error)
                    .await;
                return Err(error);
            }
        };
        let package = match self.registry.get_extension(&descriptor.provider) {
            Some(package) => package,
            None => {
                let error = DispatchError::UnknownProvider {
                    capability: request.capability_id,
                    provider: descriptor.provider.clone(),
                };
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
            let error = DispatchError::RuntimeMismatch {
                capability: request.capability_id,
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
            .await;
            return Err(error);
        }

        match descriptor.runtime {
            RuntimeKind::Wasm => {
                let Some(wasm_runtime) = self.wasm_runtime else {
                    let error = DispatchError::MissingRuntimeBackend {
                        runtime: RuntimeKind::Wasm,
                    };
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
                    .execute_extension_json(
                        self.filesystem,
                        self.governor,
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
                    Err(error) => {
                        let error = DispatchError::from(error);
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
                let Some(script_runtime) = self.script_runtime else {
                    let error = DispatchError::MissingRuntimeBackend {
                        runtime: RuntimeKind::Script,
                    };
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

                let execution = match script_runtime.execute_extension_json(
                    self.governor,
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
                    Err(error) => {
                        let error = DispatchError::from(error);
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
                let Some(mcp_runtime) = self.mcp_runtime else {
                    let error = DispatchError::MissingRuntimeBackend {
                        runtime: RuntimeKind::Mcp,
                    };
                    self.emit_dispatch_failure(
                        scope,
                        capability_id,
                        Some(descriptor.provider.clone()),
                        Some(RuntimeKind::Mcp),
                        &error,
                    )
                    .await?;
                    return Err(error);
                };
                self.emit_event(RuntimeEvent::runtime_selected(
                    scope.clone(),
                    capability_id.clone(),
                    descriptor.provider.clone(),
                    RuntimeKind::Mcp,
                ))
                .await?;

                let execution = match mcp_runtime
                    .execute_extension_json(
                        self.governor,
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
                    Err(error) => {
                        let error = DispatchError::from(error);
                        self.emit_dispatch_failure(
                            scope,
                            capability_id,
                            Some(descriptor.provider.clone()),
                            Some(RuntimeKind::Mcp),
                            &error,
                        )
                        .await?;
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
                .await?;

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
                let error = DispatchError::UnsupportedRuntime {
                    capability: request.capability_id,
                    runtime,
                };
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
            dispatch_error_kind(error),
        ))
        .await;
    }

    async fn emit_event(&self, event: RuntimeEvent) {
        if let Some(sink) = self.event_sink {
            let _ = sink.emit(event).await;
        }
    }
}

fn dispatch_error_kind(error: &DispatchError) -> &'static str {
    match error {
        DispatchError::UnknownCapability { .. } => "UnknownCapability",
        DispatchError::UnknownProvider { .. } => "UnknownProvider",
        DispatchError::RuntimeMismatch { .. } => "RuntimeMismatch",
        DispatchError::MissingRuntimeBackend { .. } => "MissingRuntimeBackend",
        DispatchError::UnsupportedRuntime { .. } => "UnsupportedRuntime",
        DispatchError::Event(_) => "Event",
        DispatchError::Mcp(_) => "Mcp",
        DispatchError::Script(_) => "Script",
        DispatchError::Wasm(_) => "Wasm",
    }
}
