//! WASM runtime contracts for IronClaw Reborn.
//!
//! `ironclaw_wasm` validates and invokes portable WASM capabilities. Modules
//! receive no ambient host authority: every privileged effect must eventually
//! cross an explicit host import checked by IronClaw host API contracts.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use ironclaw_extensions::{ExtensionError, ExtensionPackage, ExtensionRuntime};
use ironclaw_filesystem::{FilesystemError, RootFilesystem};
use ironclaw_host_api::{
    CapabilityDescriptor, CapabilityId, ExtensionId, ResourceEstimate, ResourceReservationId,
    ResourceScope, ResourceUsage, RuntimeKind, VirtualPath,
};
use ironclaw_resources::{
    ActiveResourceReservation, ResourceError, ResourceGovernor, ResourceReceipt,
};
use rust_decimal::Decimal;
use serde_json::Value;
use thiserror::Error;
use wasmtime::{Cache, Config, Engine, Instance, Module, ResourceLimiter, Store};

const DEFAULT_FUEL: u64 = 500_000;
const DEFAULT_OUTPUT_BYTES: u64 = 1024 * 1024;
const DEFAULT_MEMORY_BYTES: u64 = 10 * 1024 * 1024;
const DEFAULT_MODULE_BYTES: u64 = 50 * 1024 * 1024;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);
const DEFAULT_EPOCH_TICK_INTERVAL: Duration = Duration::from_millis(500);
const CACHE_ABI_VERSION: &str = "json-v1";

/// WASM runtime configuration for the V1 runtime lane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmRuntimeConfig {
    pub fuel: u64,
    pub max_output_bytes: u64,
    pub max_memory_bytes: u64,
    pub max_module_bytes: u64,
    pub timeout: Duration,
    pub cache_compiled_modules: bool,
    pub cache_dir: Option<PathBuf>,
    pub epoch_tick_interval: Duration,
}

impl Default for WasmRuntimeConfig {
    fn default() -> Self {
        Self {
            fuel: DEFAULT_FUEL,
            max_output_bytes: DEFAULT_OUTPUT_BYTES,
            max_memory_bytes: DEFAULT_MEMORY_BYTES,
            max_module_bytes: DEFAULT_MODULE_BYTES,
            timeout: DEFAULT_TIMEOUT,
            cache_compiled_modules: true,
            cache_dir: None,
            epoch_tick_interval: DEFAULT_EPOCH_TICK_INTERVAL,
        }
    }
}

impl WasmRuntimeConfig {
    pub fn for_testing() -> Self {
        Self {
            fuel: 100_000,
            max_output_bytes: 1024,
            max_memory_bytes: 1024 * 1024,
            max_module_bytes: 1024 * 1024,
            timeout: Duration::from_secs(5),
            cache_compiled_modules: false,
            cache_dir: None,
            epoch_tick_interval: Duration::from_millis(10),
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
    content_hash: String,
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

    pub fn content_hash(&self) -> &str {
        &self.content_hash
    }
}

impl std::fmt::Debug for PreparedWasmModule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedWasmModule")
            .field("provider", &self.provider)
            .field("capability", &self.capability)
            .field("export", &self.export)
            .field("content_hash", &self.content_hash)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ModuleCacheKey {
    provider: ExtensionId,
    capability: CapabilityId,
    export: String,
    content_hash: String,
    abi_version: &'static str,
}

impl ModuleCacheKey {
    fn new(spec: &WasmModuleSpec) -> Self {
        Self {
            provider: spec.provider.clone(),
            capability: spec.capability.clone(),
            export: spec.export.clone(),
            content_hash: wasm_content_hash(&spec.bytes),
            abi_version: CACHE_ABI_VERSION,
        }
    }
}

/// Prepared WASM module plus the descriptor and package-local module path it came from.
#[derive(Debug, Clone)]
pub struct PreparedWasmCapability {
    pub descriptor: CapabilityDescriptor,
    pub module: Arc<PreparedWasmModule>,
    pub module_path: VirtualPath,
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

/// Full resource-governed execution request.
#[derive(Debug)]
pub struct WasmExecutionRequest<'a> {
    pub package: &'a ExtensionPackage,
    pub capability_id: &'a CapabilityId,
    pub scope: ResourceScope,
    pub estimate: ResourceEstimate,
    pub invocation: CapabilityInvocation,
}

/// Full resource-governed execution result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmExecutionResult {
    pub result: CapabilityResult,
    pub receipt: ResourceReceipt,
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
    #[error("invalid WASM runtime configuration: {reason}")]
    InvalidConfig { reason: String },
    #[error("WASM runtime cache error: {reason}")]
    Cache { reason: String },
    #[error("extension package error: {0}")]
    Extension(Box<ExtensionError>),
    #[error("filesystem error: {0}")]
    Filesystem(Box<FilesystemError>),
    #[error("resource governor error: {0}")]
    Resource(Box<ResourceError>),
    #[error("invalid WASM module: {reason}")]
    InvalidModule { reason: String },
    #[error("unsupported WASM import {module}.{name}; no privileged host imports are registered")]
    UnsupportedImport { module: String, name: String },
    #[error("WASM descriptor mismatch: {reason}")]
    DescriptorMismatch { reason: String },
    #[error("extension {extension} package root {root} does not match expected extension root")]
    PackageRootMismatch {
        extension: ExtensionId,
        root: String,
    },
    #[error("extension {extension} uses runtime {actual:?}, not RuntimeKind::Wasm")]
    ExtensionRuntimeMismatch {
        extension: ExtensionId,
        actual: RuntimeKind,
    },
    #[error("capability {capability} is not declared by this extension package")]
    CapabilityNotDeclared { capability: CapabilityId },
    #[error("invalid WASM invocation: {reason}")]
    InvalidInvocation { reason: String },
    #[error("WASM module asset exceeds configured size limit: limit {limit}, actual {actual}")]
    ModuleTooLarge { limit: u64, actual: u64 },
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
    #[error("WASM memory limit exceeded: {used} bytes requested, {limit} bytes allowed")]
    MemoryExceeded { used: u64, limit: u64 },
    #[error("WASM execution timed out after {timeout:?}")]
    Timeout { timeout: Duration },
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
    prepared_modules: Arc<Mutex<HashMap<ModuleCacheKey, Arc<PreparedWasmModule>>>>,
}

impl WasmRuntime {
    pub fn new(config: WasmRuntimeConfig) -> Result<Self, WasmError> {
        validate_runtime_config(&config)?;
        let mut wasmtime_config = Config::new();
        wasmtime_config.consume_fuel(true);
        wasmtime_config.epoch_interruption(true);
        wasmtime_config.wasm_threads(false);
        wasmtime_config.debug_info(false);
        if let Some(cache_dir) = &config.cache_dir {
            enable_compilation_cache(&mut wasmtime_config, cache_dir)?;
        }
        let engine = Engine::new(&wasmtime_config).map_err(|error| WasmError::Engine {
            reason: error.to_string(),
        })?;
        spawn_epoch_ticker(engine.clone(), config.epoch_tick_interval)?;
        Ok(Self {
            engine,
            config,
            prepared_modules: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub fn for_testing() -> Result<Self, WasmError> {
        Self::new(WasmRuntimeConfig::for_testing())
    }

    pub fn config(&self) -> &WasmRuntimeConfig {
        &self.config
    }

    pub fn prepared_module_count(&self) -> usize {
        self.prepared_modules
            .lock()
            .map(|cache| cache.len())
            .unwrap_or(0)
    }

    pub fn prepare(&self, spec: WasmModuleSpec) -> Result<PreparedWasmModule, WasmError> {
        self.prepare_uncached(spec)
    }

    pub fn prepare_cached(
        &self,
        spec: WasmModuleSpec,
    ) -> Result<Arc<PreparedWasmModule>, WasmError> {
        if !self.config.cache_compiled_modules {
            return self.prepare_uncached(spec).map(Arc::new);
        }

        let key = ModuleCacheKey::new(&spec);
        {
            let cache = self.prepared_modules.lock().map_err(|_| WasmError::Cache {
                reason: "prepared module cache lock poisoned".to_string(),
            })?;
            if let Some(module) = cache.get(&key) {
                return Ok(Arc::clone(module));
            }
        }

        let prepared = Arc::new(self.prepare_uncached(spec)?);
        let mut cache = self.prepared_modules.lock().map_err(|_| WasmError::Cache {
            reason: "prepared module cache lock poisoned".to_string(),
        })?;
        Ok(Arc::clone(cache.entry(key).or_insert(prepared)))
    }

    fn prepare_uncached(&self, spec: WasmModuleSpec) -> Result<PreparedWasmModule, WasmError> {
        let content_hash = wasm_content_hash(&spec.bytes);
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

        if spec.export.trim().is_empty()
            || !module.exports().any(|export| export.name() == spec.export)
        {
            return Err(WasmError::MissingExport {
                export: spec.export,
            });
        }

        Ok(PreparedWasmModule {
            provider: spec.provider,
            capability: spec.capability,
            export: spec.export,
            content_hash,
            module,
        })
    }

    /// Prepare a WASM capability from a validated extension package manifest.
    pub async fn prepare_extension_capability<F>(
        &self,
        fs: &F,
        package: &ExtensionPackage,
        capability_id: &CapabilityId,
    ) -> Result<PreparedWasmCapability, WasmError>
    where
        F: RootFilesystem,
    {
        validate_package_root(package)?;

        let descriptor = package
            .capabilities
            .iter()
            .find(|descriptor| &descriptor.id == capability_id)
            .cloned()
            .ok_or_else(|| WasmError::CapabilityNotDeclared {
                capability: capability_id.clone(),
            })?;

        if descriptor.runtime != RuntimeKind::Wasm {
            return Err(WasmError::ExtensionRuntimeMismatch {
                extension: package.id.clone(),
                actual: descriptor.runtime,
            });
        }
        if descriptor.provider != package.id {
            return Err(WasmError::DescriptorMismatch {
                reason: format!(
                    "descriptor {} provider {} does not match package {}",
                    descriptor.id, descriptor.provider, package.id
                ),
            });
        }

        let module_asset = match &package.manifest.runtime {
            ExtensionRuntime::Wasm { module } => module,
            other => {
                return Err(WasmError::ExtensionRuntimeMismatch {
                    extension: package.id.clone(),
                    actual: other.kind(),
                });
            }
        };
        let module_path = module_asset
            .resolve_under(&package.root)
            .map_err(|error| WasmError::Extension(Box::new(error)))?;
        let stat = fs
            .stat(&module_path)
            .await
            .map_err(|error| WasmError::Filesystem(Box::new(error)))?;
        ensure_module_size_within_limit(stat.len, self.config.max_module_bytes)?;
        let bytes = fs
            .read_file(&module_path)
            .await
            .map_err(|error| WasmError::Filesystem(Box::new(error)))?;
        ensure_module_size_within_limit(bytes.len() as u64, self.config.max_module_bytes)?;
        let export = capability_export_name(&package.id, capability_id)?;
        let module = self.prepare_cached(WasmModuleSpec {
            provider: package.id.clone(),
            capability: capability_id.clone(),
            export,
            bytes,
        })?;

        Ok(PreparedWasmCapability {
            descriptor,
            module,
            module_path,
        })
    }

    /// Execute a WASM extension capability with resource reserve/reconcile semantics.
    pub async fn execute_extension_json<F, G>(
        &self,
        fs: &F,
        governor: &G,
        request: WasmExecutionRequest<'_>,
    ) -> Result<WasmExecutionResult, WasmError>
    where
        F: RootFilesystem,
        G: ResourceGovernor,
    {
        let reservation = governor
            .reserve(request.scope, request.estimate)
            .map_err(|error| WasmError::Resource(Box::new(error)))?;

        let active_reservation = match governor.active_reservation(reservation.id) {
            Ok(active) => active,
            Err(error) => {
                return Err(release_after_failure(
                    governor,
                    reservation.id,
                    WasmError::Resource(Box::new(error)),
                ));
            }
        };

        let prepared = match self
            .prepare_extension_capability(fs, request.package, request.capability_id)
            .await
        {
            Ok(prepared) => prepared,
            Err(error) => return Err(release_after_failure(governor, reservation.id, error)),
        };

        let result = match self.invoke_json(
            prepared.module.as_ref(),
            &prepared.descriptor,
            Some(&active_reservation),
            request.invocation,
        ) {
            Ok(result) => result,
            Err(error) => return Err(release_after_failure(governor, reservation.id, error)),
        };

        let receipt = governor
            .reconcile(reservation.id, result.usage.clone())
            .map_err(|error| WasmError::Resource(Box::new(error)))?;
        Ok(WasmExecutionResult { result, receipt })
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
        let instance = Instance::new(&mut store, &module.module, &[])
            .map_err(|error| self.classify_wasmtime_error(error))?;
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

        let input_ptr = alloc
            .call(&mut store, input_len)
            .map_err(|error| self.classify_wasmtime_error(error))?;
        let input_offset = positive_offset(input_ptr, "alloc returned a negative input pointer")?;
        memory
            .write(&mut store, input_offset, &input_bytes)
            .map_err(|error| WasmError::GuestAllocation {
                reason: error.to_string(),
            })?;

        let status = run
            .call(&mut store, (input_ptr, input_len))
            .map_err(|error| self.classify_wasmtime_error(error))?;
        self.ensure_no_memory_denial(&store)?;
        let output_ptr_value = output_ptr
            .call(&mut store, ())
            .map_err(|error| self.classify_wasmtime_error(error))?;
        let output_len_value = output_len
            .call(&mut store, ())
            .map_err(|error| self.classify_wasmtime_error(error))?;
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

        let instance = Instance::new(&mut store, &module.module, &[])
            .map_err(|error| self.classify_wasmtime_error(error))?;
        let func = instance
            .get_typed_func::<i32, i32>(&mut store, module.export())
            .map_err(|_| WasmError::MissingExport {
                export: module.export().to_string(),
            })?;

        let value = func
            .call(&mut store, input)
            .map_err(|error| self.classify_wasmtime_error(error))?;
        self.ensure_no_memory_denial(&store)?;

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

    fn fueled_store(&self) -> Result<Store<RuntimeStoreData>, WasmError> {
        let mut store = Store::new(
            &self.engine,
            RuntimeStoreData::new(self.config.max_memory_bytes),
        );
        store.limiter(|data| &mut data.limiter);
        store.epoch_deadline_trap();
        store.set_epoch_deadline(epoch_deadline_ticks(&self.config));
        store
            .set_fuel(self.config.fuel)
            .map_err(|error| WasmError::Trap {
                reason: error.to_string(),
            })?;
        Ok(store)
    }

    fn fuel_consumed(&self, store: &Store<RuntimeStoreData>) -> u64 {
        self.config
            .fuel
            .saturating_sub(store.get_fuel().unwrap_or(0))
    }

    fn ensure_no_memory_denial(&self, store: &Store<RuntimeStoreData>) -> Result<(), WasmError> {
        if let Some((used, limit)) = store.data().limiter.denied_memory_growth {
            Err(WasmError::MemoryExceeded { used, limit })
        } else {
            Ok(())
        }
    }

    fn classify_wasmtime_error(&self, error: wasmtime::Error) -> WasmError {
        if matches!(
            error.downcast_ref::<wasmtime::Trap>(),
            Some(wasmtime::Trap::OutOfFuel)
        ) {
            return WasmError::FuelExhausted {
                limit: self.config.fuel,
            };
        }
        if matches!(
            error.downcast_ref::<wasmtime::Trap>(),
            Some(wasmtime::Trap::Interrupt)
        ) {
            return WasmError::Timeout {
                timeout: self.config.timeout,
            };
        }
        let message = error.to_string();
        if message.contains("ironclaw memory limit exceeded")
            || message.contains("memory")
            || message.contains("Memory")
        {
            return WasmError::MemoryExceeded {
                used: parse_marker_u64(&message, "desired=")
                    .unwrap_or(self.config.max_memory_bytes.saturating_add(1)),
                limit: parse_marker_u64(&message, "limit=").unwrap_or(self.config.max_memory_bytes),
            };
        }
        if message.contains("all fuel consumed") || message.contains("out of fuel") {
            WasmError::FuelExhausted {
                limit: self.config.fuel,
            }
        } else if message.contains("interrupt") {
            WasmError::Timeout {
                timeout: self.config.timeout,
            }
        } else {
            WasmError::Trap { reason: message }
        }
    }
}

#[derive(Debug)]
struct RuntimeStoreData {
    limiter: WasmRuntimeLimiter,
}

impl RuntimeStoreData {
    fn new(memory_limit: u64) -> Self {
        Self {
            limiter: WasmRuntimeLimiter::new(memory_limit),
        }
    }
}

#[derive(Debug)]
struct WasmRuntimeLimiter {
    memory_limit: u64,
    memory_used: u64,
    denied_memory_growth: Option<(u64, u64)>,
}

impl WasmRuntimeLimiter {
    fn new(memory_limit: u64) -> Self {
        Self {
            memory_limit,
            memory_used: 0,
            denied_memory_growth: None,
        }
    }
}

impl ResourceLimiter for WasmRuntimeLimiter {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> Result<bool, wasmtime::Error> {
        let desired = desired as u64;
        if desired > self.memory_limit {
            self.denied_memory_growth = Some((desired, self.memory_limit));
            return Ok(false);
        }
        self.memory_used = desired;
        Ok(true)
    }

    fn table_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> Result<bool, wasmtime::Error> {
        Ok(desired <= 10_000)
    }

    fn instances(&self) -> usize {
        10
    }

    fn tables(&self) -> usize {
        10
    }

    fn memories(&self) -> usize {
        10
    }
}

fn validate_package_root(package: &ExtensionPackage) -> Result<(), WasmError> {
    let expected_root = VirtualPath::new(format!("/system/extensions/{}", package.id.as_str()))
        .map_err(|error| WasmError::InvalidInvocation {
            reason: error.to_string(),
        })?;
    if package.root != expected_root {
        return Err(WasmError::PackageRootMismatch {
            extension: package.id.clone(),
            root: package.root.as_str().to_string(),
        });
    }
    Ok(())
}

fn ensure_module_size_within_limit(actual: u64, limit: u64) -> Result<(), WasmError> {
    if actual > limit {
        Err(WasmError::ModuleTooLarge { limit, actual })
    } else {
        Ok(())
    }
}

fn release_after_failure<G>(
    governor: &G,
    reservation_id: ResourceReservationId,
    original: WasmError,
) -> WasmError
where
    G: ResourceGovernor,
{
    match governor.release(reservation_id) {
        Ok(_) => original,
        Err(error) => WasmError::Resource(Box::new(error)),
    }
}

fn capability_export_name(
    package_id: &ExtensionId,
    capability_id: &CapabilityId,
) -> Result<String, WasmError> {
    let expected_prefix = format!("{}.", package_id.as_str());
    capability_id
        .as_str()
        .strip_prefix(&expected_prefix)
        .filter(|suffix| !suffix.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| WasmError::DescriptorMismatch {
            reason: format!(
                "capability {} is not prefixed by package {}",
                capability_id, package_id
            ),
        })
}

fn validate_runtime_config(config: &WasmRuntimeConfig) -> Result<(), WasmError> {
    if config.fuel == 0 {
        return Err(WasmError::InvalidConfig {
            reason: "fuel must be greater than zero".to_string(),
        });
    }
    if config.max_memory_bytes == 0 {
        return Err(WasmError::InvalidConfig {
            reason: "max_memory_bytes must be greater than zero".to_string(),
        });
    }
    if config.max_module_bytes == 0 {
        return Err(WasmError::InvalidConfig {
            reason: "max_module_bytes must be greater than zero".to_string(),
        });
    }
    if config.timeout.is_zero() {
        return Err(WasmError::InvalidConfig {
            reason: "timeout must be greater than zero".to_string(),
        });
    }
    if config.epoch_tick_interval.is_zero() {
        return Err(WasmError::InvalidConfig {
            reason: "epoch_tick_interval must be greater than zero".to_string(),
        });
    }
    Ok(())
}

fn enable_compilation_cache(config: &mut Config, cache_dir: &Path) -> Result<(), WasmError> {
    std::fs::create_dir_all(cache_dir).map_err(|_| WasmError::Cache {
        reason: "failed to create compilation cache directory".to_string(),
    })?;
    let toml_path = cache_dir.join("wasmtime-cache.toml");
    let escaped = cache_dir
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    std::fs::write(&toml_path, format!("[cache]\ndirectory = \"{escaped}\"\n")).map_err(|_| {
        WasmError::Cache {
            reason: "failed to write compilation cache configuration".to_string(),
        }
    })?;
    let cache = Cache::from_file(Some(&toml_path)).map_err(|_| WasmError::Cache {
        reason: "failed to load compilation cache configuration".to_string(),
    })?;
    config.cache(Some(cache));
    Ok(())
}

fn spawn_epoch_ticker(engine: Engine, tick_interval: Duration) -> Result<(), WasmError> {
    if tick_interval.is_zero() {
        return Ok(());
    }
    std::thread::Builder::new()
        .name("ironclaw-wasm-epoch-ticker".to_string())
        .spawn(move || {
            loop {
                std::thread::sleep(tick_interval);
                engine.increment_epoch();
            }
        })
        .map(|_| ())
        .map_err(|error| WasmError::Engine {
            reason: format!("failed to spawn epoch ticker thread: {error}"),
        })
}

fn epoch_deadline_ticks(config: &WasmRuntimeConfig) -> u64 {
    if config.timeout.is_zero() || config.epoch_tick_interval.is_zero() {
        return u64::MAX;
    }
    let timeout_ms = config.timeout.as_millis();
    let interval_ms = config.epoch_tick_interval.as_millis().max(1);
    (timeout_ms / interval_ms).max(1) as u64
}

fn wasm_content_hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn parse_marker_u64(message: &str, marker: &str) -> Option<u64> {
    let start = message.find(marker)? + marker.len();
    let digits: String = message[start..]
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect();
    digits.parse().ok()
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
