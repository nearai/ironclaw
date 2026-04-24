//! WASM runtime contracts for IronClaw Reborn.
//!
//! `ironclaw_wasm` validates and invokes portable WASM capabilities. Modules
//! receive no ambient host authority: every privileged effect must eventually
//! cross an explicit host import checked by IronClaw host API contracts.

use std::time::Instant;

use ironclaw_host_api::{
    CapabilityDescriptor, CapabilityId, ExtensionId, ResourceUsage, RuntimeKind,
};
use ironclaw_resources::ResourceReservation;
use rust_decimal::Decimal;
use thiserror::Error;
use wasmtime::{Config, Engine, Instance, Module, Store};

/// WASM runtime configuration for the narrow V1 vertical slice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmRuntimeConfig {
    pub fuel: u64,
    pub max_output_bytes: u64,
}

impl Default for WasmRuntimeConfig {
    fn default() -> Self {
        Self {
            fuel: 500_000,
            max_output_bytes: 1024 * 1024,
        }
    }
}

impl WasmRuntimeConfig {
    pub fn for_testing() -> Self {
        Self {
            fuel: 100_000,
            max_output_bytes: 1024,
        }
    }
}

/// Request to prepare one WASM export for one capability descriptor.
#[derive(Debug, Clone)]
pub struct WasmModuleSpec {
    pub provider: ExtensionId,
    pub capability: CapabilityId,
    pub export: String,
    pub bytes: Vec<u8>,
}

/// Prepared, validated WASM module.
#[derive(Clone)]
pub struct PreparedWasmModule {
    provider: ExtensionId,
    capability: CapabilityId,
    export: String,
    module: Module,
}

impl PreparedWasmModule {
    pub fn provider(&self) -> &ExtensionId {
        &self.provider
    }

    pub fn capability(&self) -> &CapabilityId {
        &self.capability
    }

    pub fn export(&self) -> &str {
        &self.export
    }
}

impl std::fmt::Debug for PreparedWasmModule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedWasmModule")
            .field("provider", &self.provider)
            .field("capability", &self.capability)
            .field("export", &self.export)
            .finish_non_exhaustive()
    }
}

/// WASM invocation result with usage data for resource reconciliation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmInvocationResult<T> {
    pub value: T,
    pub reservation_id: ironclaw_host_api::ResourceReservationId,
    pub usage: ResourceUsage,
    pub fuel_consumed: u64,
    pub output_bytes: u64,
}

/// WASM runtime errors.
#[derive(Debug, Error)]
pub enum WasmError {
    #[error("failed to create WASM engine: {reason}")]
    Engine { reason: String },
    #[error("invalid WASM module: {reason}")]
    InvalidModule { reason: String },
    #[error("unsupported WASM import {module}.{name}; no privileged host imports are registered")]
    UnsupportedImport { module: String, name: String },
    #[error("WASM descriptor mismatch: {reason}")]
    DescriptorMismatch { reason: String },
    #[error("WASM invocation requires an active resource reservation")]
    MissingReservation,
    #[error("WASM export '{export}' was not found or has the wrong signature")]
    MissingExport { export: String },
    #[error("WASM fuel exhausted after limit {limit}")]
    FuelExhausted { limit: u64 },
    #[error("WASM output limit exceeded: limit {limit}, actual {actual}")]
    OutputLimitExceeded { limit: u64, actual: u64 },
    #[error("WASM trap: {reason}")]
    Trap { reason: String },
}

/// Minimal Wasmtime-backed runtime.
#[derive(Debug, Clone)]
pub struct WasmRuntime {
    engine: Engine,
    config: WasmRuntimeConfig,
}

impl WasmRuntime {
    pub fn new(config: WasmRuntimeConfig) -> Result<Self, WasmError> {
        let mut wasmtime_config = Config::new();
        wasmtime_config.consume_fuel(true);
        wasmtime_config.wasm_threads(false);
        wasmtime_config.debug_info(false);
        let engine = Engine::new(&wasmtime_config).map_err(|error| WasmError::Engine {
            reason: error.to_string(),
        })?;
        Ok(Self { engine, config })
    }

    pub fn for_testing() -> Result<Self, WasmError> {
        Self::new(WasmRuntimeConfig::for_testing())
    }

    pub fn config(&self) -> &WasmRuntimeConfig {
        &self.config
    }

    pub fn prepare(&self, spec: WasmModuleSpec) -> Result<PreparedWasmModule, WasmError> {
        let module = Module::from_binary(&self.engine, &spec.bytes).map_err(|error| {
            WasmError::InvalidModule {
                reason: error.to_string(),
            }
        })?;

        if let Some(import) = module.imports().next() {
            return Err(WasmError::UnsupportedImport {
                module: import.module().to_string(),
                name: import.name().to_string(),
            });
        }

        if spec.export.trim().is_empty() {
            return Err(WasmError::MissingExport {
                export: spec.export,
            });
        }

        Ok(PreparedWasmModule {
            provider: spec.provider,
            capability: spec.capability,
            export: spec.export,
            module,
        })
    }

    pub fn invoke_i32(
        &self,
        module: &PreparedWasmModule,
        descriptor: &CapabilityDescriptor,
        reservation: Option<&ResourceReservation>,
        input: i32,
    ) -> Result<WasmInvocationResult<i32>, WasmError> {
        let reservation = reservation.ok_or(WasmError::MissingReservation)?;
        self.validate_descriptor(module, descriptor)?;

        let start = Instant::now();
        let mut store = Store::new(&self.engine, ());
        store
            .set_fuel(self.config.fuel)
            .map_err(|error| WasmError::Trap {
                reason: error.to_string(),
            })?;

        let instance = Instance::new(&mut store, &module.module, &[])
            .map_err(|error| classify_wasmtime_error(error, self.config.fuel))?;
        let func = instance
            .get_typed_func::<i32, i32>(&mut store, module.export())
            .map_err(|_| WasmError::MissingExport {
                export: module.export().to_string(),
            })?;

        let value = func
            .call(&mut store, input)
            .map_err(|error| classify_wasmtime_error(error, self.config.fuel))?;

        let output_bytes = value.to_string().len() as u64;
        if output_bytes > self.config.max_output_bytes {
            return Err(WasmError::OutputLimitExceeded {
                limit: self.config.max_output_bytes,
                actual: output_bytes,
            });
        }

        let fuel_remaining = store.get_fuel().unwrap_or(0);
        let fuel_consumed = self.config.fuel.saturating_sub(fuel_remaining);
        let wall_clock_ms = start.elapsed().as_millis().max(1) as u64;

        Ok(WasmInvocationResult {
            value,
            reservation_id: reservation.id,
            usage: ResourceUsage {
                usd: Decimal::ZERO,
                input_tokens: 0,
                output_tokens: 0,
                wall_clock_ms,
                output_bytes,
                network_egress_bytes: 0,
                process_count: 1,
            },
            fuel_consumed,
            output_bytes,
        })
    }

    fn validate_descriptor(
        &self,
        module: &PreparedWasmModule,
        descriptor: &CapabilityDescriptor,
    ) -> Result<(), WasmError> {
        if descriptor.runtime != RuntimeKind::Wasm {
            return Err(WasmError::DescriptorMismatch {
                reason: "descriptor runtime must be RuntimeKind::Wasm".to_string(),
            });
        }
        if descriptor.provider != module.provider {
            return Err(WasmError::DescriptorMismatch {
                reason: format!(
                    "descriptor provider {} does not match module provider {}",
                    descriptor.provider, module.provider
                ),
            });
        }
        if descriptor.id != module.capability {
            return Err(WasmError::DescriptorMismatch {
                reason: format!(
                    "descriptor capability {} does not match module capability {}",
                    descriptor.id, module.capability
                ),
            });
        }
        Ok(())
    }
}

fn classify_wasmtime_error(error: wasmtime::Error, fuel_limit: u64) -> WasmError {
    if matches!(
        error.downcast_ref::<wasmtime::Trap>(),
        Some(wasmtime::Trap::OutOfFuel)
    ) {
        return WasmError::FuelExhausted { limit: fuel_limit };
    }
    let message = error.to_string();
    if message.contains("all fuel consumed") || message.contains("out of fuel") {
        WasmError::FuelExhausted { limit: fuel_limit }
    } else {
        WasmError::Trap { reason: message }
    }
}
