/// Request to prepare one WASM export for one capability descriptor.
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
    pub resource_reservation: Option<ResourceReservation>,
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
