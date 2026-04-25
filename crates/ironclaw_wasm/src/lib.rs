//! WASM runtime contracts for IronClaw Reborn.
//!
//! `ironclaw_wasm` validates and invokes portable WASM capabilities. Modules
//! receive no ambient host authority: every privileged effect must eventually
//! cross an explicit host import checked by IronClaw host API contracts.

use std::time::Instant;

use ironclaw_host_api::{
    CapabilityDescriptor, CapabilityId, ExtensionId, ResourceReservationId, ResourceUsage,
    RuntimeKind,
};
use ironclaw_resources::ActiveResourceReservation;
use rust_decimal::Decimal;
use serde_json::Value;
use thiserror::Error;
use wasmtime::{Config, Engine, Instance, Module, Store, StoreLimits, StoreLimitsBuilder};

/// WASM runtime configuration for the narrow V1 vertical slice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmRuntimeConfig {
    pub fuel: u64,
    pub max_output_bytes: u64,
    pub max_memory_bytes: usize,
}

impl Default for WasmRuntimeConfig {
    fn default() -> Self {
        Self {
            fuel: 500_000,
            max_output_bytes: 1024 * 1024,
            max_memory_bytes: 64 * 1024 * 1024,
        }
    }
}

impl WasmRuntimeConfig {
    pub fn for_testing() -> Self {
        Self {
            fuel: 100_000,
            max_output_bytes: 1024,
            max_memory_bytes: 1024 * 1024,
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

/// JSON capability invocation payload for the initial Reborn WASM ABI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityInvocation {
    pub input: Value,
}

/// Structured JSON result returned by the initial Reborn WASM ABI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityResult {
    pub output: Value,
    pub reservation_id: ResourceReservationId,
    pub usage: ResourceUsage,
    pub fuel_consumed: u64,
    pub output_bytes: u64,
}

/// WASM invocation result with usage data for resource reconciliation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmInvocationResult<T> {
    pub value: T,
    pub reservation_id: ResourceReservationId,
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
    #[error("invalid WASM invocation: {reason}")]
    InvalidInvocation { reason: String },
    #[error("WASM invocation requires an active resource reservation")]
    MissingReservation,
    #[error("WASM export '{export}' was not found or has the wrong signature")]
    MissingExport { export: String },
    #[error("WASM JSON ABI requires an exported memory named 'memory'")]
    MissingMemory,
    #[error("WASM guest allocation failed: {reason}")]
    GuestAllocation { reason: String },
    #[error("WASM guest returned status {status}: {message}")]
    GuestError { status: i32, message: String },
    #[error("WASM guest output is invalid: {reason}")]
    InvalidGuestOutput { reason: String },
    #[error("WASM fuel exhausted after limit {limit}")]
    FuelExhausted { limit: u64 },
    #[error("WASM output limit exceeded: limit {limit}, actual {actual}")]
    OutputLimitExceeded { limit: u64, actual: u64 },
    #[error("WASM memory limit exceeded: limit {limit} bytes")]
    MemoryLimitExceeded { limit: usize },
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

    /// Invoke a capability through the initial JSON pointer/length ABI.
    ///
    /// The guest module must export:
    ///
    /// - `memory`
    /// - `alloc(len: i32) -> i32`
    /// - the module's configured capability export as `(ptr: i32, len: i32) -> i32`
    /// - `output_ptr() -> i32`
    /// - `output_len() -> i32`
    ///
    /// A zero status means the output buffer contains JSON success output. Any
    /// non-zero status means the output buffer contains a JSON error object and
    /// is surfaced as [`WasmError::GuestError`].
    pub fn invoke_json(
        &self,
        module: &PreparedWasmModule,
        descriptor: &CapabilityDescriptor,
        reservation: Option<&ActiveResourceReservation>,
        invocation: CapabilityInvocation,
    ) -> Result<CapabilityResult, WasmError> {
        let reservation = reservation.ok_or(WasmError::MissingReservation)?;
        self.validate_descriptor(module, descriptor)?;
        validate_invocation_schema(&descriptor.parameters_schema, &invocation.input)?;

        let input_bytes = serde_json::to_vec(&invocation.input).map_err(|error| {
            WasmError::InvalidInvocation {
                reason: error.to_string(),
            }
        })?;
        let input_len =
            i32::try_from(input_bytes.len()).map_err(|_| WasmError::InvalidInvocation {
                reason: "input JSON is too large for the V1 WASM ABI".to_string(),
            })?;

        let start = Instant::now();
        let mut store = self.fueled_store()?;
        let instance = Instance::new(&mut store, &module.module, &[]).map_err(|error| {
            classify_wasmtime_error(error, self.config.fuel, self.config.max_memory_bytes)
        })?;
        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or(WasmError::MissingMemory)?;
        let alloc = instance
            .get_typed_func::<i32, i32>(&mut store, "alloc")
            .map_err(|_| WasmError::MissingExport {
                export: "alloc".to_string(),
            })?;
        let run = instance
            .get_typed_func::<(i32, i32), i32>(&mut store, module.export())
            .map_err(|_| WasmError::MissingExport {
                export: module.export().to_string(),
            })?;
        let output_ptr = instance
            .get_typed_func::<(), i32>(&mut store, "output_ptr")
            .map_err(|_| WasmError::MissingExport {
                export: "output_ptr".to_string(),
            })?;
        let output_len = instance
            .get_typed_func::<(), i32>(&mut store, "output_len")
            .map_err(|_| WasmError::MissingExport {
                export: "output_len".to_string(),
            })?;

        let input_ptr = alloc.call(&mut store, input_len).map_err(|error| {
            classify_wasmtime_error(error, self.config.fuel, self.config.max_memory_bytes)
        })?;
        let input_offset = positive_offset(input_ptr, "alloc returned a negative input pointer")?;
        memory
            .write(&mut store, input_offset, &input_bytes)
            .map_err(|error| WasmError::GuestAllocation {
                reason: error.to_string(),
            })?;

        let status = run
            .call(&mut store, (input_ptr, input_len))
            .map_err(|error| {
                classify_wasmtime_error(error, self.config.fuel, self.config.max_memory_bytes)
            })?;
        let output_ptr_value = output_ptr.call(&mut store, ()).map_err(|error| {
            classify_wasmtime_error(error, self.config.fuel, self.config.max_memory_bytes)
        })?;
        let output_len_value = output_len.call(&mut store, ()).map_err(|error| {
            classify_wasmtime_error(error, self.config.fuel, self.config.max_memory_bytes)
        })?;
        let output_offset =
            positive_offset(output_ptr_value, "output_ptr returned a negative pointer")?;
        let output_len = positive_len(output_len_value)?;
        if output_len as u64 > self.config.max_output_bytes {
            return Err(WasmError::OutputLimitExceeded {
                limit: self.config.max_output_bytes,
                actual: output_len as u64,
            });
        }

        let mut output_bytes = vec![0_u8; output_len];
        memory
            .read(&store, output_offset, &mut output_bytes)
            .map_err(|error| WasmError::InvalidGuestOutput {
                reason: error.to_string(),
            })?;

        if status != 0 {
            return Err(guest_error(status, &output_bytes));
        }

        let output = serde_json::from_slice(&output_bytes).map_err(|error| {
            WasmError::InvalidGuestOutput {
                reason: error.to_string(),
            }
        })?;
        let output_byte_count = output_bytes.len() as u64;
        let fuel_consumed = self.fuel_consumed(&store);
        let usage = resource_usage(start, output_byte_count);

        Ok(CapabilityResult {
            output,
            reservation_id: reservation.id(),
            usage,
            fuel_consumed,
            output_bytes: output_byte_count,
        })
    }

    pub fn invoke_i32(
        &self,
        module: &PreparedWasmModule,
        descriptor: &CapabilityDescriptor,
        reservation: Option<&ActiveResourceReservation>,
        input: i32,
    ) -> Result<WasmInvocationResult<i32>, WasmError> {
        let reservation = reservation.ok_or(WasmError::MissingReservation)?;
        self.validate_descriptor(module, descriptor)?;

        let start = Instant::now();
        let mut store = self.fueled_store()?;

        let instance = Instance::new(&mut store, &module.module, &[]).map_err(|error| {
            classify_wasmtime_error(error, self.config.fuel, self.config.max_memory_bytes)
        })?;
        let func = instance
            .get_typed_func::<i32, i32>(&mut store, module.export())
            .map_err(|_| WasmError::MissingExport {
                export: module.export().to_string(),
            })?;

        let value = func.call(&mut store, input).map_err(|error| {
            classify_wasmtime_error(error, self.config.fuel, self.config.max_memory_bytes)
        })?;

        let output_bytes = value.to_string().len() as u64;
        if output_bytes > self.config.max_output_bytes {
            return Err(WasmError::OutputLimitExceeded {
                limit: self.config.max_output_bytes,
                actual: output_bytes,
            });
        }

        Ok(WasmInvocationResult {
            value,
            reservation_id: reservation.id(),
            usage: resource_usage(start, output_bytes),
            fuel_consumed: self.fuel_consumed(&store),
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

    fn fueled_store(&self) -> Result<Store<WasmStoreState>, WasmError> {
        let mut store = Store::new(
            &self.engine,
            WasmStoreState {
                limits: StoreLimitsBuilder::new()
                    .memory_size(self.config.max_memory_bytes)
                    .trap_on_grow_failure(true)
                    .build(),
            },
        );
        store.limiter(|state| &mut state.limits);
        store
            .set_fuel(self.config.fuel)
            .map_err(|error| WasmError::Trap {
                reason: error.to_string(),
            })?;
        Ok(store)
    }

    fn fuel_consumed<T>(&self, store: &Store<T>) -> u64 {
        self.config
            .fuel
            .saturating_sub(store.get_fuel().unwrap_or(0))
    }
}

fn positive_offset(value: i32, reason: &str) -> Result<usize, WasmError> {
    usize::try_from(value).map_err(|_| WasmError::InvalidGuestOutput {
        reason: reason.to_string(),
    })
}

fn positive_len(value: i32) -> Result<usize, WasmError> {
    usize::try_from(value).map_err(|_| WasmError::InvalidGuestOutput {
        reason: "output_len returned a negative length".to_string(),
    })
}

fn guest_error(status: i32, output_bytes: &[u8]) -> WasmError {
    let message = serde_json::from_slice::<Value>(output_bytes)
        .ok()
        .and_then(|value| {
            value
                .get("message")
                .and_then(Value::as_str)
                .map(sanitize_guest_message)
        })
        .unwrap_or_else(|| "guest returned an error without a valid message".to_string());
    WasmError::GuestError { status, message }
}

fn sanitize_guest_message(message: &str) -> String {
    let sanitized = message
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect::<String>();
    let collapsed = sanitized.split_whitespace().collect::<Vec<_>>().join(" ");
    const MAX_GUEST_ERROR_CHARS: usize = 256;
    collapsed.chars().take(MAX_GUEST_ERROR_CHARS).collect()
}

fn validate_invocation_schema(schema: &Value, input: &Value) -> Result<(), WasmError> {
    validate_schema_at(schema, input, "input")
}

fn validate_schema_at(schema: &Value, input: &Value, field: &str) -> Result<(), WasmError> {
    if schema.is_null() {
        return Ok(());
    }

    if let Some(expected_type) = schema.get("type").and_then(Value::as_str) {
        validate_json_type(input, expected_type, field)?;
    }

    if let Some(required) = schema.get("required").and_then(Value::as_array) {
        let input_object = input
            .as_object()
            .ok_or_else(|| WasmError::InvalidInvocation {
                reason: format!("input field '{field}' must be object"),
            })?;
        for required_field in required {
            let Some(required_field) = required_field.as_str() else {
                continue;
            };
            if !input_object.contains_key(required_field) {
                return Err(WasmError::InvalidInvocation {
                    reason: format!("missing required input field '{field}.{required_field}'"),
                });
            }
        }
    }

    if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
        let Some(input_object) = input.as_object() else {
            return Ok(());
        };
        for (property, property_schema) in properties {
            let Some(value) = input_object.get(property) else {
                continue;
            };
            validate_schema_at(property_schema, value, &format!("{field}.{property}"))?;
        }
    }

    if let Some(item_schema) = schema.get("items") {
        let Some(items) = input.as_array() else {
            return Ok(());
        };
        for (index, value) in items.iter().enumerate() {
            validate_schema_at(item_schema, value, &format!("{field}[{index}]"))?;
        }
    }

    Ok(())
}

fn validate_json_type(value: &Value, expected_type: &str, field: &str) -> Result<(), WasmError> {
    let valid = match expected_type {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        _ => true,
    };

    if valid {
        Ok(())
    } else {
        Err(WasmError::InvalidInvocation {
            reason: format!("input field '{field}' must be {expected_type}"),
        })
    }
}

fn resource_usage(start: Instant, output_bytes: u64) -> ResourceUsage {
    ResourceUsage {
        usd: Decimal::ZERO,
        input_tokens: 0,
        output_tokens: 0,
        wall_clock_ms: start.elapsed().as_millis().max(1) as u64,
        output_bytes,
        network_egress_bytes: 0,
        process_count: 1,
    }
}

struct WasmStoreState {
    limits: StoreLimits,
}

fn classify_wasmtime_error(
    error: wasmtime::Error,
    fuel_limit: u64,
    memory_limit: usize,
) -> WasmError {
    if matches!(
        error.downcast_ref::<wasmtime::Trap>(),
        Some(wasmtime::Trap::OutOfFuel)
    ) {
        return WasmError::FuelExhausted { limit: fuel_limit };
    }
    let message = error.to_string();
    if message.contains("all fuel consumed") || message.contains("out of fuel") {
        WasmError::FuelExhausted { limit: fuel_limit }
    } else if message.contains("memory") || message.contains("Memory") {
        WasmError::MemoryLimitExceeded {
            limit: memory_limit,
        }
    } else {
        WasmError::Trap { reason: message }
    }
}
