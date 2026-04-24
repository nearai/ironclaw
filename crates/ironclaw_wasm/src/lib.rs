//! WASM runtime contracts for IronClaw Reborn.
//!
//! `ironclaw_wasm` validates and invokes portable WASM capabilities. Modules
//! receive no ambient host authority: every privileged effect must eventually
//! cross an explicit host import checked by IronClaw host API contracts.

use std::{
    collections::HashMap,
    net::IpAddr,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use futures::executor::block_on;
use ironclaw_extensions::{ExtensionError, ExtensionPackage, ExtensionRuntime};
use ironclaw_filesystem::{FilesystemError, RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{
    CapabilityDescriptor, CapabilityId, ExtensionId, MountView, NetworkMethod, NetworkPolicy,
    NetworkScheme, NetworkTarget, NetworkTargetPattern, ResourceEstimate, ResourceReservationId,
    ResourceScope, ResourceUsage, RuntimeKind, ScopedPath, VirtualPath,
};
use ironclaw_resources::{ResourceError, ResourceGovernor, ResourceReceipt, ResourceReservation};
use rust_decimal::Decimal;
use serde_json::Value;
use thiserror::Error;
use wasmtime::{Cache, Caller, Config, Engine, Instance, Linker, Module, ResourceLimiter, Store};

const DEFAULT_FUEL: u64 = 500_000;
const DEFAULT_OUTPUT_BYTES: u64 = 1024 * 1024;
const DEFAULT_MEMORY_BYTES: u64 = 10 * 1024 * 1024;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);
const DEFAULT_EPOCH_TICK_INTERVAL: Duration = Duration::from_millis(500);
const CACHE_ABI_VERSION: &str = "json-v1";
const CORE_IMPORT_MODULE: &str = "host";
const CORE_LOG_IMPORT: &str = "log_utf8";
const CORE_TIME_IMPORT: &str = "time_unix_ms";
const FS_READ_IMPORT: &str = "fs_read_utf8";
const FS_WRITE_IMPORT: &str = "fs_write_utf8";
const FS_LIST_IMPORT: &str = "fs_list_utf8";
const FS_STAT_LEN_IMPORT: &str = "fs_stat_len";
const HTTP_REQUEST_IMPORT: &str = "http_request_utf8";
const MAX_LOG_ENTRIES: usize = 1_000;
const MAX_LOG_MESSAGE_BYTES: usize = 4 * 1024;

/// WASM runtime configuration for the V1 runtime lane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmRuntimeConfig {
    pub fuel: u64,
    pub max_output_bytes: u64,
    pub max_memory_bytes: u64,
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
            timeout: Duration::from_secs(5),
            cache_compiled_modules: false,
            cache_dir: None,
            epoch_tick_interval: Duration::from_millis(10),
        }
    }
}

/// Synchronous filesystem surface exposed to WASM host imports.
pub trait WasmHostFilesystem: Send + Sync {
    fn read_utf8(&self, path: &str) -> Result<String, String>;
    fn write_utf8(&self, path: &str, contents: &str) -> Result<(), String>;
    fn list_utf8(&self, path: &str) -> Result<String, String>;
    fn stat_len(&self, path: &str) -> Result<u64, String>;
}

/// Scoped filesystem adapter for WASM filesystem imports.
#[derive(Debug, Clone)]
pub struct WasmScopedFilesystem<F> {
    scoped: ScopedFilesystem<F>,
}

impl<F> WasmScopedFilesystem<F>
where
    F: RootFilesystem,
{
    pub fn new(root: Arc<F>, mounts: MountView) -> Self {
        Self {
            scoped: ScopedFilesystem::new(root, mounts),
        }
    }

    pub fn scoped(&self) -> &ScopedFilesystem<F> {
        &self.scoped
    }
}

impl<F> WasmHostFilesystem for WasmScopedFilesystem<F>
where
    F: RootFilesystem,
{
    fn read_utf8(&self, path: &str) -> Result<String, String> {
        let path = ScopedPath::new(path.to_string()).map_err(|error| error.to_string())?;
        let bytes = block_on(self.scoped.read_file(&path)).map_err(|error| error.to_string())?;
        String::from_utf8(bytes).map_err(|error| error.to_string())
    }

    fn write_utf8(&self, path: &str, contents: &str) -> Result<(), String> {
        let path = ScopedPath::new(path.to_string()).map_err(|error| error.to_string())?;
        block_on(self.scoped.write_file(&path, contents.as_bytes()))
            .map_err(|error| error.to_string())
    }

    fn list_utf8(&self, path: &str) -> Result<String, String> {
        let path = ScopedPath::new(path.to_string()).map_err(|error| error.to_string())?;
        let entries = block_on(self.scoped.list_dir(&path)).map_err(|error| error.to_string())?;
        let names = entries
            .into_iter()
            .map(|entry| entry.name)
            .collect::<Vec<_>>();
        serde_json::to_string(&names).map_err(|error| error.to_string())
    }

    fn stat_len(&self, path: &str) -> Result<u64, String> {
        let path = ScopedPath::new(path.to_string()).map_err(|error| error.to_string())?;
        block_on(self.scoped.stat(&path))
            .map(|stat| stat.len)
            .map_err(|error| error.to_string())
    }
}

/// Host-mediated HTTP request issued by a WASM network import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmHttpRequest {
    pub method: NetworkMethod,
    pub url: String,
    pub body: String,
}

/// Host-mediated HTTP response returned to a WASM network import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmHttpResponse {
    pub status: u16,
    pub body: String,
}

/// Synchronous host HTTP surface exposed to WASM network imports.
pub trait WasmHostHttp: Send + Sync {
    fn request_utf8(&self, request: WasmHttpRequest) -> Result<WasmHttpResponse, String>;
}

/// Network-policy enforcing wrapper around a host-provided HTTP client.
#[derive(Debug, Clone)]
pub struct WasmPolicyHttpClient<C> {
    client: C,
    policy: NetworkPolicy,
}

impl<C> WasmPolicyHttpClient<C> {
    pub fn new(client: C, policy: NetworkPolicy) -> Self {
        Self { client, policy }
    }

    pub fn policy(&self) -> &NetworkPolicy {
        &self.policy
    }
}

impl<C> WasmHostHttp for WasmPolicyHttpClient<C>
where
    C: WasmHostHttp,
{
    fn request_utf8(&self, request: WasmHttpRequest) -> Result<WasmHttpResponse, String> {
        let target = network_target_for_url(&request.url)?;
        if !network_policy_allows(&self.policy, &target) {
            return Err("network target denied by policy".to_string());
        }
        if let Some(max) = self.policy.max_egress_bytes
            && request.body.len() as u64 > max
        {
            return Err("network request exceeds egress limit".to_string());
        }
        let response = self.client.request_utf8(request)?;
        if let Some(max) = self.policy.max_egress_bytes
            && response.body.len() as u64 > max
        {
            return Err("network response exceeds body limit".to_string());
        }
        Ok(response)
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

/// Core host log levels accepted by the low-risk logging import.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WasmLogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl WasmLogLevel {
    fn from_i32(value: i32) -> Self {
        match value {
            0 => Self::Trace,
            1 => Self::Debug,
            3 => Self::Warn,
            4 => Self::Error,
            _ => Self::Info,
        }
    }
}

/// Guest log entry captured through the core `host.log_utf8` import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmLogEntry {
    pub level: WasmLogLevel,
    pub message: String,
    pub timestamp_unix_ms: u64,
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
    pub logs: Vec<WasmLogEntry>,
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
    #[error("extension {extension} uses runtime {actual:?}, not RuntimeKind::Wasm")]
    ExtensionRuntimeMismatch {
        extension: ExtensionId,
        actual: RuntimeKind,
    },
    #[error("capability {capability} is not declared by this extension package")]
    CapabilityNotDeclared { capability: CapabilityId },
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

        validate_module_imports(&module)?;

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
        let bytes = fs
            .read_file(&module_path)
            .await
            .map_err(|error| WasmError::Filesystem(Box::new(error)))?;
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
            Some(&reservation),
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
        reservation: Option<&ResourceReservation>,
        invocation: CapabilityInvocation,
    ) -> Result<CapabilityResult, WasmError> {
        self.invoke_json_inner(module, descriptor, reservation, invocation, None, None)
    }

    pub fn invoke_json_with_filesystem(
        &self,
        module: &PreparedWasmModule,
        descriptor: &CapabilityDescriptor,
        reservation: Option<&ResourceReservation>,
        invocation: CapabilityInvocation,
        filesystem: Arc<dyn WasmHostFilesystem>,
    ) -> Result<CapabilityResult, WasmError> {
        self.invoke_json_inner(
            module,
            descriptor,
            reservation,
            invocation,
            Some(filesystem),
            None,
        )
    }

    pub fn invoke_json_with_network(
        &self,
        module: &PreparedWasmModule,
        descriptor: &CapabilityDescriptor,
        reservation: Option<&ResourceReservation>,
        invocation: CapabilityInvocation,
        http: Arc<dyn WasmHostHttp>,
    ) -> Result<CapabilityResult, WasmError> {
        self.invoke_json_inner(
            module,
            descriptor,
            reservation,
            invocation,
            None,
            Some(http),
        )
    }

    fn invoke_json_inner(
        &self,
        module: &PreparedWasmModule,
        descriptor: &CapabilityDescriptor,
        reservation: Option<&ResourceReservation>,
        invocation: CapabilityInvocation,
        filesystem: Option<Arc<dyn WasmHostFilesystem>>,
        http: Option<Arc<dyn WasmHostHttp>>,
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
        let mut store = self.fueled_store_with_context(filesystem, http)?;
        let instance = self.instantiate_module(&mut store, module)?;
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

        let logs = store.data().logs.clone();

        Ok(CapabilityResult {
            output,
            reservation_id: reservation.id,
            usage,
            fuel_consumed,
            output_bytes: output_byte_count,
            logs,
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
        let mut store = self.fueled_store()?;

        let instance = self.instantiate_module(&mut store, module)?;
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
            reservation_id: reservation.id,
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

    fn instantiate_module(
        &self,
        store: &mut Store<RuntimeStoreData>,
        module: &PreparedWasmModule,
    ) -> Result<Instance, WasmError> {
        let mut linker = Linker::new(&self.engine);
        add_core_host_imports(&mut linker)?;
        linker
            .instantiate(store, &module.module)
            .map_err(|error| self.classify_wasmtime_error(error))
    }

    fn fueled_store(&self) -> Result<Store<RuntimeStoreData>, WasmError> {
        self.fueled_store_with_context(None, None)
    }

    fn fueled_store_with_context(
        &self,
        filesystem: Option<Arc<dyn WasmHostFilesystem>>,
        http: Option<Arc<dyn WasmHostHttp>>,
    ) -> Result<Store<RuntimeStoreData>, WasmError> {
        let mut store = Store::new(
            &self.engine,
            RuntimeStoreData::new(self.config.max_memory_bytes, filesystem, http),
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
        if message.contains("ironclaw memory limit exceeded") {
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

struct RuntimeStoreData {
    limiter: WasmRuntimeLimiter,
    logs: Vec<WasmLogEntry>,
    filesystem: Option<Arc<dyn WasmHostFilesystem>>,
    http: Option<Arc<dyn WasmHostHttp>>,
}

impl RuntimeStoreData {
    fn new(
        memory_limit: u64,
        filesystem: Option<Arc<dyn WasmHostFilesystem>>,
        http: Option<Arc<dyn WasmHostHttp>>,
    ) -> Self {
        Self {
            limiter: WasmRuntimeLimiter::new(memory_limit),
            logs: Vec::new(),
            filesystem,
            http,
        }
    }

    fn push_log(&mut self, level: WasmLogLevel, message: String) {
        if self.logs.len() >= MAX_LOG_ENTRIES {
            return;
        }
        self.logs.push(WasmLogEntry {
            level,
            message,
            timestamp_unix_ms: unix_time_ms(),
        });
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

fn validate_module_imports(module: &Module) -> Result<(), WasmError> {
    for import in module.imports() {
        if !is_supported_core_import(import.module(), import.name()) {
            return Err(WasmError::UnsupportedImport {
                module: import.module().to_string(),
                name: import.name().to_string(),
            });
        }
    }
    Ok(())
}

fn is_supported_core_import(module: &str, name: &str) -> bool {
    module == CORE_IMPORT_MODULE
        && matches!(
            name,
            CORE_LOG_IMPORT
                | CORE_TIME_IMPORT
                | FS_READ_IMPORT
                | FS_WRITE_IMPORT
                | FS_LIST_IMPORT
                | FS_STAT_LEN_IMPORT
                | HTTP_REQUEST_IMPORT
        )
}

fn add_core_host_imports(linker: &mut Linker<RuntimeStoreData>) -> Result<(), WasmError> {
    linker
        .func_wrap(CORE_IMPORT_MODULE, CORE_TIME_IMPORT, || -> i64 {
            unix_time_ms() as i64
        })
        .map_err(|error| WasmError::Engine {
            reason: error.to_string(),
        })?;
    linker
        .func_wrap(
            CORE_IMPORT_MODULE,
            CORE_LOG_IMPORT,
            |mut caller: Caller<'_, RuntimeStoreData>, level: i32, ptr: i32, len: i32| -> i32 {
                host_log_utf8(&mut caller, level, ptr, len)
            },
        )
        .map_err(|error| WasmError::Engine {
            reason: error.to_string(),
        })?;
    linker
        .func_wrap(
            CORE_IMPORT_MODULE,
            FS_READ_IMPORT,
            |mut caller: Caller<'_, RuntimeStoreData>,
             path_ptr: i32,
             path_len: i32,
             out_ptr: i32,
             out_cap: i32|
             -> i32 {
                host_fs_read_utf8(&mut caller, path_ptr, path_len, out_ptr, out_cap)
            },
        )
        .map_err(|error| WasmError::Engine {
            reason: error.to_string(),
        })?;
    linker
        .func_wrap(
            CORE_IMPORT_MODULE,
            FS_WRITE_IMPORT,
            |mut caller: Caller<'_, RuntimeStoreData>,
             path_ptr: i32,
             path_len: i32,
             data_ptr: i32,
             data_len: i32|
             -> i32 {
                host_fs_write_utf8(&mut caller, path_ptr, path_len, data_ptr, data_len)
            },
        )
        .map_err(|error| WasmError::Engine {
            reason: error.to_string(),
        })?;
    linker
        .func_wrap(
            CORE_IMPORT_MODULE,
            FS_LIST_IMPORT,
            |mut caller: Caller<'_, RuntimeStoreData>,
             path_ptr: i32,
             path_len: i32,
             out_ptr: i32,
             out_cap: i32|
             -> i32 {
                host_fs_list_utf8(&mut caller, path_ptr, path_len, out_ptr, out_cap)
            },
        )
        .map_err(|error| WasmError::Engine {
            reason: error.to_string(),
        })?;
    linker
        .func_wrap(
            CORE_IMPORT_MODULE,
            FS_STAT_LEN_IMPORT,
            |mut caller: Caller<'_, RuntimeStoreData>, path_ptr: i32, path_len: i32| -> i64 {
                host_fs_stat_len(&mut caller, path_ptr, path_len)
            },
        )
        .map_err(|error| WasmError::Engine {
            reason: error.to_string(),
        })?;
    linker
        .func_wrap(
            CORE_IMPORT_MODULE,
            HTTP_REQUEST_IMPORT,
            |mut caller: Caller<'_, RuntimeStoreData>,
             method: i32,
             url_ptr: i32,
             url_len: i32,
             body_ptr: i32,
             body_len: i32,
             out_ptr: i32,
             out_cap: i32|
             -> i32 {
                host_http_request_utf8(
                    &mut caller,
                    HttpImportArgs {
                        method,
                        url_ptr,
                        url_len,
                        body_ptr,
                        body_len,
                        out_ptr,
                        out_cap,
                    },
                )
            },
        )
        .map_err(|error| WasmError::Engine {
            reason: error.to_string(),
        })?;
    Ok(())
}

fn host_log_utf8(caller: &mut Caller<'_, RuntimeStoreData>, level: i32, ptr: i32, len: i32) -> i32 {
    let Ok(offset) = usize::try_from(ptr) else {
        return -1;
    };
    let Ok(len) = usize::try_from(len) else {
        return -1;
    };
    if len > MAX_LOG_MESSAGE_BYTES {
        return -2;
    }
    let Some(memory) = caller
        .get_export("memory")
        .and_then(|item| item.into_memory())
    else {
        return -3;
    };
    let mut bytes = vec![0_u8; len];
    if memory.read(&*caller, offset, &mut bytes).is_err() {
        return -4;
    }
    let Ok(message) = String::from_utf8(bytes) else {
        return -5;
    };
    caller
        .data_mut()
        .push_log(WasmLogLevel::from_i32(level), message);
    0
}

fn host_fs_read_utf8(
    caller: &mut Caller<'_, RuntimeStoreData>,
    path_ptr: i32,
    path_len: i32,
    out_ptr: i32,
    out_cap: i32,
) -> i32 {
    let Ok(path) = read_guest_utf8(caller, path_ptr, path_len) else {
        return -1;
    };
    let Some(filesystem) = caller.data().filesystem.clone() else {
        return -10;
    };
    match filesystem.read_utf8(&path) {
        Ok(contents) => write_guest_bytes(caller, out_ptr, out_cap, contents.as_bytes()),
        Err(_) => -11,
    }
}

fn host_fs_write_utf8(
    caller: &mut Caller<'_, RuntimeStoreData>,
    path_ptr: i32,
    path_len: i32,
    data_ptr: i32,
    data_len: i32,
) -> i32 {
    let Ok(path) = read_guest_utf8(caller, path_ptr, path_len) else {
        return -1;
    };
    let Ok(contents) = read_guest_utf8(caller, data_ptr, data_len) else {
        return -2;
    };
    let Some(filesystem) = caller.data().filesystem.clone() else {
        return -10;
    };
    match filesystem.write_utf8(&path, &contents) {
        Ok(()) => 0,
        Err(_) => -11,
    }
}

fn host_fs_list_utf8(
    caller: &mut Caller<'_, RuntimeStoreData>,
    path_ptr: i32,
    path_len: i32,
    out_ptr: i32,
    out_cap: i32,
) -> i32 {
    let Ok(path) = read_guest_utf8(caller, path_ptr, path_len) else {
        return -1;
    };
    let Some(filesystem) = caller.data().filesystem.clone() else {
        return -10;
    };
    match filesystem.list_utf8(&path) {
        Ok(contents) => write_guest_bytes(caller, out_ptr, out_cap, contents.as_bytes()),
        Err(_) => -11,
    }
}

fn host_fs_stat_len(
    caller: &mut Caller<'_, RuntimeStoreData>,
    path_ptr: i32,
    path_len: i32,
) -> i64 {
    let Ok(path) = read_guest_utf8(caller, path_ptr, path_len) else {
        return -1;
    };
    let Some(filesystem) = caller.data().filesystem.clone() else {
        return -10;
    };
    filesystem
        .stat_len(&path)
        .map(|len| len.min(i64::MAX as u64) as i64)
        .unwrap_or(-11)
}

fn read_guest_utf8(
    caller: &mut Caller<'_, RuntimeStoreData>,
    ptr: i32,
    len: i32,
) -> Result<String, i32> {
    let offset = usize::try_from(ptr).map_err(|_| -1)?;
    let len = usize::try_from(len).map_err(|_| -1)?;
    let Some(memory) = caller
        .get_export("memory")
        .and_then(|item| item.into_memory())
    else {
        return Err(-3);
    };
    let mut bytes = vec![0_u8; len];
    memory.read(&*caller, offset, &mut bytes).map_err(|_| -4)?;
    String::from_utf8(bytes).map_err(|_| -5)
}

fn write_guest_bytes(
    caller: &mut Caller<'_, RuntimeStoreData>,
    out_ptr: i32,
    out_cap: i32,
    bytes: &[u8],
) -> i32 {
    let Ok(offset) = usize::try_from(out_ptr) else {
        return -1;
    };
    let Ok(capacity) = usize::try_from(out_cap) else {
        return -1;
    };
    if bytes.len() > capacity {
        return -6;
    }
    let Some(memory) = caller
        .get_export("memory")
        .and_then(|item| item.into_memory())
    else {
        return -3;
    };
    if memory.write(caller, offset, bytes).is_err() {
        return -4;
    }
    i32::try_from(bytes.len()).unwrap_or(-6)
}

struct HttpImportArgs {
    method: i32,
    url_ptr: i32,
    url_len: i32,
    body_ptr: i32,
    body_len: i32,
    out_ptr: i32,
    out_cap: i32,
}

fn host_http_request_utf8(caller: &mut Caller<'_, RuntimeStoreData>, args: HttpImportArgs) -> i32 {
    let Some(method) = network_method_from_i32(args.method) else {
        return -1;
    };
    let Ok(url) = read_guest_utf8(caller, args.url_ptr, args.url_len) else {
        return -2;
    };
    let body = if args.body_len == 0 {
        String::new()
    } else {
        match read_guest_utf8(caller, args.body_ptr, args.body_len) {
            Ok(body) => body,
            Err(_) => return -3,
        }
    };
    let Some(http) = caller.data().http.clone() else {
        return -10;
    };
    match http.request_utf8(WasmHttpRequest { method, url, body }) {
        Ok(response) => {
            write_guest_bytes(caller, args.out_ptr, args.out_cap, response.body.as_bytes())
        }
        Err(_) => -11,
    }
}

fn network_method_from_i32(value: i32) -> Option<NetworkMethod> {
    Some(match value {
        0 => NetworkMethod::Get,
        1 => NetworkMethod::Post,
        2 => NetworkMethod::Put,
        3 => NetworkMethod::Patch,
        4 => NetworkMethod::Delete,
        5 => NetworkMethod::Head,
        _ => return None,
    })
}

fn network_target_for_url(raw: &str) -> Result<NetworkTarget, String> {
    let url = url::Url::parse(raw).map_err(|error| error.to_string())?;
    let scheme = match url.scheme() {
        "http" => NetworkScheme::Http,
        "https" => NetworkScheme::Https,
        other => return Err(format!("unsupported URL scheme {other}")),
    };
    let host = url
        .host_str()
        .filter(|host| !host.trim().is_empty())
        .ok_or_else(|| "URL host is required".to_string())?
        .to_ascii_lowercase();
    Ok(NetworkTarget {
        scheme,
        host,
        port: url.port(),
    })
}

fn network_policy_allows(policy: &NetworkPolicy, target: &NetworkTarget) -> bool {
    if policy.allowed_targets.is_empty() {
        return false;
    }
    if policy.deny_private_ip_ranges
        && let Ok(ip) = target.host.parse::<IpAddr>()
        && is_private_or_loopback_ip(ip)
    {
        return false;
    }
    policy
        .allowed_targets
        .iter()
        .any(|pattern| target_matches_pattern(target, pattern))
}

fn target_matches_pattern(target: &NetworkTarget, pattern: &NetworkTargetPattern) -> bool {
    if let Some(scheme) = pattern.scheme
        && scheme != target.scheme
    {
        return false;
    }
    if let Some(port) = pattern.port
        && Some(port) != target.port
    {
        return false;
    }
    host_matches_pattern(&target.host, &pattern.host_pattern.to_ascii_lowercase())
}

fn host_matches_pattern(host: &str, pattern: &str) -> bool {
    if let Some(suffix) = pattern.strip_prefix("*.") {
        host.ends_with(&format!(".{suffix}")) && host != suffix
    } else {
        host == pattern
    }
}

fn is_private_or_loopback_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_documentation()
                || ip.octets()[0] == 0
        }
        IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
        }
    }
}

fn unix_time_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
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

fn enable_compilation_cache(config: &mut Config, cache_dir: &Path) -> Result<(), WasmError> {
    std::fs::create_dir_all(cache_dir).map_err(|error| WasmError::Cache {
        reason: error.to_string(),
    })?;
    let toml_path = cache_dir.join("wasmtime-cache.toml");
    let escaped = cache_dir
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    std::fs::write(&toml_path, format!("[cache]\ndirectory = \"{escaped}\"\n")).map_err(
        |error| WasmError::Cache {
            reason: error.to_string(),
        },
    )?;
    let cache = Cache::from_file(Some(&toml_path)).map_err(|error| WasmError::Cache {
        reason: error.to_string(),
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
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| "guest returned an error without a valid message".to_string());
    WasmError::GuestError { status, message }
}

fn validate_invocation_schema(schema: &Value, input: &Value) -> Result<(), WasmError> {
    if schema.is_null() {
        return Ok(());
    }

    if let Some(expected_type) = schema.get("type").and_then(Value::as_str) {
        validate_json_type(input, expected_type, "input")?;
    }

    let Some(input_object) = input.as_object() else {
        return Ok(());
    };

    if let Some(required) = schema.get("required").and_then(Value::as_array) {
        for field in required {
            let Some(field) = field.as_str() else {
                continue;
            };
            if !input_object.contains_key(field) {
                return Err(WasmError::InvalidInvocation {
                    reason: format!("missing required input field '{field}'"),
                });
            }
        }
    }

    if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
        for (field, property_schema) in properties {
            let Some(value) = input_object.get(field) else {
                continue;
            };
            if let Some(expected_type) = property_schema.get("type").and_then(Value::as_str) {
                validate_json_type(value, expected_type, field)?;
            }
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
