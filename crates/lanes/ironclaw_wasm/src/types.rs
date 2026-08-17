use ironclaw_host_api::resource::ResourceUsage;

use crate::wasm_sandbox_core::SandboxLimits;

/// Which generated WIT bindings a prepared component instantiates against.
///
/// Resolved once at [`PreparedWitTool`] preparation time (the first
/// instantiation, done to extract `description`/`schema`) so `execute`
/// doesn't re-probe both worlds on every call.
///
/// Legacy fallback — removed in PR 4, once every guest has migrated to the
/// current (near:agent@0.4.0) world.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WitBindingVersion {
    /// near:agent@0.4.0 — the typed success/failure response variant.
    Current,
    /// near:agent@0.3.0 — the frozen `option<string>`/`option<string>`
    /// response record, decoded through the pre-existing
    /// `ironclaw_host_runtime` string-decode path. Removed in PR 4.
    Legacy,
}

/// Compiled WIT tool component plus metadata extracted from its WIT exports.
pub struct PreparedWitTool {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) schema: serde_json::Value,
    pub(crate) component: wasmtime::component::Component,
    pub(crate) limits: SandboxLimits,
    pub(crate) binding_version: WitBindingVersion,
}

impl PreparedWitTool {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn description(&self) -> &str {
        &self.description
    }

    pub fn schema(&self) -> &serde_json::Value {
        &self.schema
    }

    pub fn limits(&self) -> &SandboxLimits {
        &self.limits
    }
}

impl std::fmt::Debug for PreparedWitTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedWitTool")
            .field("name", &self.name)
            .field("description", &self.description)
            .field("schema", &self.schema)
            .field("limits", &self.limits)
            .field("binding_version", &self.binding_version)
            .finish_non_exhaustive()
    }
}

/// Request passed to a WIT tool component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WitToolRequest {
    pub params_json: String,
    pub context_json: Option<String>,
}

impl WitToolRequest {
    pub fn new(params_json: impl Into<String>) -> Self {
        Self {
            params_json: params_json.into(),
            context_json: None,
        }
    }

    pub fn with_context(mut self, context_json: impl Into<String>) -> Self {
        self.context_json = Some(context_json.into());
        self
    }
}

/// Log level captured from the WIT host `log` import.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WasmLogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

/// One guest-emitted log message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmLogRecord {
    pub level: WasmLogLevel,
    pub message: String,
}

/// Closed vocabulary for a typed guest failure's category, mirroring the WIT
/// `error-kind` enum (near:agent@0.4.0) and the host-side
/// `StructuredWasmGuestErrorKind` vocabulary in `ironclaw_host_runtime`
/// exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WitErrorKind {
    AuthRequired,
    Input,
    OutputTooLarge,
    Executor,
    NetworkDenied,
    Client,
    OperationFailed,
}

/// Typed guest-reported failure (WIT `guest-failure`, near:agent@0.4.0).
///
/// `code` and `message` are free-text carriers scrubbed for secret-shaped
/// values at the sandbox-exit chokepoint (`runtime::scrub_guest_error`)
/// before this value is constructed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WitGuestFailure {
    pub kind: WitErrorKind,
    pub code: Option<String>,
    pub message: Option<String>,
    pub retry_after_ms: Option<u64>,
}

/// Outcome of one WIT tool `execute` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WitToolOutcome {
    /// JSON-encoded output on success.
    Success(String),
    /// Typed guest failure (near:agent@0.4.0 `guest-failure`).
    Failure(WitGuestFailure),
    /// Legacy 0.3.0 binding fallback: the raw guest error string, decoded
    /// through the pre-existing structured/plain-string decode path in
    /// `ironclaw_host_runtime::services::wasm_execution`
    /// (`structured_wasm_guest_error` / `wasm_guest_error_kind`). Removed in
    /// PR 4.
    LegacyFailure(String),
    /// Legacy 0.3.0 binding fallback: the guest returned neither `output`
    /// nor `error`. Removed in PR 4.
    LegacyMissingOutput,
}

/// Result of one WIT tool execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WitToolExecution {
    pub outcome: WitToolOutcome,
    pub usage: ResourceUsage,
    pub logs: Vec<WasmLogRecord>,
}
