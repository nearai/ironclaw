//! Composition-only runtime dispatch contracts for IronClaw Reborn.
//!
//! `ironclaw_kernel` wires validated extension descriptors to runtime lanes. It
//! does not parse extension manifests, implement sandbox policy, reserve budget
//! itself, or execute product workflows. Those responsibilities stay in the
//! owning service crates.

use ironclaw_extensions::ExtensionRegistry;
use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::{
    CapabilityId, ExtensionId, ResourceEstimate, ResourceScope, ResourceUsage, RuntimeKind,
};
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
    #[error("script dispatch failed: {0}")]
    Script(Box<ScriptError>),
    #[error("WASM dispatch failed: {0}")]
    Wasm(Box<WasmError>),
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

    pub async fn dispatch_json(
        &self,
        request: CapabilityDispatchRequest,
    ) -> Result<CapabilityDispatchResult, DispatchError> {
        let descriptor = self
            .registry
            .get_capability(&request.capability_id)
            .ok_or_else(|| DispatchError::UnknownCapability {
                capability: request.capability_id.clone(),
            })?;
        let package = self
            .registry
            .get_extension(&descriptor.provider)
            .ok_or_else(|| DispatchError::UnknownProvider {
                capability: request.capability_id.clone(),
                provider: descriptor.provider.clone(),
            })?;
        let package_runtime = package.manifest.runtime_kind();
        if descriptor.runtime != package_runtime {
            return Err(DispatchError::RuntimeMismatch {
                capability: request.capability_id,
                descriptor_runtime: descriptor.runtime,
                package_runtime,
            });
        }

        match descriptor.runtime {
            RuntimeKind::Wasm => {
                let wasm_runtime =
                    self.wasm_runtime
                        .ok_or(DispatchError::MissingRuntimeBackend {
                            runtime: RuntimeKind::Wasm,
                        })?;
                let capability_id = request.capability_id.clone();
                let execution = wasm_runtime
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
                    .await?;

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
                let script_runtime =
                    self.script_runtime
                        .ok_or(DispatchError::MissingRuntimeBackend {
                            runtime: RuntimeKind::Script,
                        })?;
                let capability_id = request.capability_id.clone();
                let execution = script_runtime.execute_extension_json(
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
                )?;

                Ok(CapabilityDispatchResult {
                    capability_id,
                    provider: descriptor.provider.clone(),
                    runtime: RuntimeKind::Script,
                    output: execution.result.output,
                    usage: execution.result.usage,
                    receipt: execution.receipt,
                })
            }
            runtime @ (RuntimeKind::Mcp | RuntimeKind::FirstParty | RuntimeKind::System) => {
                Err(DispatchError::UnsupportedRuntime {
                    capability: request.capability_id,
                    runtime,
                })
            }
        }
    }
}
