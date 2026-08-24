use ironclaw_host_api::resource::ResourceUsage;

use crate::wasm_sandbox_core::SandboxLimits;

/// Compiled WIT tool component plus metadata extracted from its WIT exports.
pub struct PreparedWitTool {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) schema: serde_json::Value,
    pub(crate) component: wasmtime::component::Component,
    pub(crate) limits: SandboxLimits,
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
/// `error-kind` enum (near:agent@0.4.1).
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

/// Typed guest-reported failure (WIT `guest-failure`, near:agent@0.4.1).
///
/// `code` and `message` are free-text carriers scrubbed for secret-shaped
/// values at the sandbox-exit chokepoint (`runtime::scrub_guest_error`)
/// before this value is constructed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WitGuestFailure {
    pub kind: WitErrorKind,
    pub code: Option<String>,
    pub message: Option<String>,
}

/// Outcome of one WIT tool `execute` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WitToolOutcome {
    /// JSON-encoded output on success.
    Success(String),
    /// Typed guest failure (near:agent@0.4.1 `guest-failure`).
    Failure(WitGuestFailure),
}

/// Result of one WIT tool execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WitToolExecution {
    pub outcome: WitToolOutcome,
    pub usage: ResourceUsage,
    pub logs: Vec<WasmLogRecord>,
}
