//! Reborn WASM component runtime lane.
//!
//! This crate owns the Reborn-only WASM runtime surface. It intentionally uses
//! the canonical WIT/component-model contract in `wit/tool.wit` instead of the
//! temporary JSON pointer/length ABI that was abandoned before landing.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ironclaw_host_api::ResourceUsage;
use thiserror::Error;
use wasmtime::component::Linker;
use wasmtime::{Config, Engine, ResourceLimiter, Store};
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

#[allow(clippy::all)]
mod bindings {
    wasmtime::component::bindgen!({
        path: "../../wit/tool.wit",
        world: "sandboxed-tool",
        with: {},
    });
}

/// WIT package version supported by the Reborn WASM tool runtime.
pub const WIT_TOOL_VERSION: &str = "0.3.0";

const DEFAULT_MEMORY_BYTES: u64 = 10 * 1024 * 1024;
const DEFAULT_FUEL: u64 = 500_000_000;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);
const EPOCH_TICK_INTERVAL: Duration = Duration::from_millis(500);
const MAX_LOGS_PER_EXECUTION: usize = 1_000;
const MAX_LOG_MESSAGE_BYTES: usize = 4 * 1024;

/// Errors returned by the Reborn WASM runtime.
#[derive(Debug, Error)]
pub enum WasmError {
    #[error("failed to create WASM engine: {0}")]
    EngineCreationFailed(String),
    #[error("failed to compile WIT component: {0}")]
    CompilationFailed(String),
    #[error("failed to configure WASM store: {0}")]
    StoreConfiguration(String),
    #[error("failed to configure WASM linker: {0}")]
    LinkerConfiguration(String),
    #[error("failed to instantiate WIT component: {0}")]
    InstantiationFailed(String),
    #[error("failed to execute WIT component: {0}")]
    ExecutionFailed(String),
    #[error("tool schema export did not return a valid JSON object: {0}")]
    InvalidSchema(String),
}

/// Errors returned by injected host services.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WasmHostError {
    #[error("{0}")]
    Denied(String),
    #[error("{0}")]
    Unavailable(String),
    #[error("{0}")]
    Failed(String),
    #[error("{0}")]
    FailedAfterRequestSent(String),
}

impl WasmHostError {
    fn request_was_sent(&self) -> bool {
        matches!(self, Self::FailedAfterRequestSent(_))
    }
}

/// HTTP request shape exposed through the WIT `host.http-request` import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmHttpRequest {
    pub method: String,
    pub url: String,
    pub headers_json: String,
    pub body: Option<Vec<u8>>,
    pub timeout_ms: Option<u32>,
}

/// HTTP response shape returned to a WASM guest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmHttpResponse {
    pub status: u16,
    pub headers_json: String,
    pub body: Vec<u8>,
}

/// Host HTTP seam used by the WIT runtime.
///
/// Production composition should wire this to the shared Reborn runtime egress
/// service. Until that service exists, the default implementation denies every
/// request so WASM cannot perform direct network I/O.
pub trait WasmHostHttp: Send + Sync {
    fn request(&self, request: WasmHttpRequest) -> Result<WasmHttpResponse, WasmHostError>;
}

/// Fail-closed HTTP host service.
#[derive(Debug, Default)]
pub struct DenyWasmHostHttp;

impl WasmHostHttp for DenyWasmHostHttp {
    fn request(&self, _request: WasmHttpRequest) -> Result<WasmHttpResponse, WasmHostError> {
        Err(WasmHostError::Unavailable(
            "WASM HTTP egress is not configured".to_string(),
        ))
    }
}

/// Recording HTTP host service for tests and development fixtures.
#[derive(Debug)]
pub struct RecordingWasmHostHttp {
    requests: Mutex<Vec<WasmHttpRequest>>,
    response: Result<WasmHttpResponse, WasmHostError>,
}

impl RecordingWasmHostHttp {
    pub fn ok(response: WasmHttpResponse) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            response: Ok(response),
        }
    }

    pub fn err(error: WasmHostError) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            response: Err(error),
        }
    }

    pub fn requests(&self) -> Result<Vec<WasmHttpRequest>, WasmHostError> {
        self.requests
            .lock()
            .map(|requests| requests.clone())
            .map_err(|_| WasmHostError::Failed("recording HTTP request log is poisoned".into()))
    }
}

impl WasmHostHttp for RecordingWasmHostHttp {
    fn request(&self, request: WasmHttpRequest) -> Result<WasmHttpResponse, WasmHostError> {
        self.requests
            .lock()
            .map_err(|_| WasmHostError::Failed("recording HTTP request log is poisoned".into()))?
            .push(request);
        self.response.clone()
    }
}

pub trait WasmHostWorkspace: Send + Sync {
    fn read(&self, path: &str) -> Option<String>;
}

#[derive(Debug, Default)]
pub struct DenyWasmHostWorkspace;

impl WasmHostWorkspace for DenyWasmHostWorkspace {
    fn read(&self, _path: &str) -> Option<String> {
        None
    }
}

pub trait WasmHostSecrets: Send + Sync {
    fn exists(&self, name: &str) -> bool;
}

#[derive(Debug, Default)]
pub struct DenyWasmHostSecrets;

impl WasmHostSecrets for DenyWasmHostSecrets {
    fn exists(&self, _name: &str) -> bool {
        false
    }
}

pub trait WasmHostTools: Send + Sync {
    fn invoke(&self, alias: &str, params_json: &str) -> Result<String, WasmHostError>;
}

#[derive(Debug, Default)]
pub struct DenyWasmHostTools;

impl WasmHostTools for DenyWasmHostTools {
    fn invoke(&self, _alias: &str, _params_json: &str) -> Result<String, WasmHostError> {
        Err(WasmHostError::Unavailable(
            "WASM tool invocation is not configured".to_string(),
        ))
    }
}

pub trait WasmHostClock: Send + Sync {
    fn now_millis(&self) -> u64;
}

#[derive(Debug, Default)]
pub struct SystemWasmHostClock;

impl WasmHostClock for SystemWasmHostClock {
    fn now_millis(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
            .unwrap_or(0)
    }
}

/// Host services made available to one WASM tool execution.
#[derive(Clone)]
pub struct WitToolHost {
    http: Arc<dyn WasmHostHttp>,
    workspace: Arc<dyn WasmHostWorkspace>,
    secrets: Arc<dyn WasmHostSecrets>,
    tools: Arc<dyn WasmHostTools>,
    clock: Arc<dyn WasmHostClock>,
}

impl WitToolHost {
    pub fn deny_all() -> Self {
        Self {
            http: Arc::new(DenyWasmHostHttp),
            workspace: Arc::new(DenyWasmHostWorkspace),
            secrets: Arc::new(DenyWasmHostSecrets),
            tools: Arc::new(DenyWasmHostTools),
            clock: Arc::new(SystemWasmHostClock),
        }
    }

    pub fn with_http<T>(mut self, http: Arc<T>) -> Self
    where
        T: WasmHostHttp + 'static,
    {
        self.http = http;
        self
    }

    pub fn with_workspace<T>(mut self, workspace: Arc<T>) -> Self
    where
        T: WasmHostWorkspace + 'static,
    {
        self.workspace = workspace;
        self
    }

    pub fn with_secrets<T>(mut self, secrets: Arc<T>) -> Self
    where
        T: WasmHostSecrets + 'static,
    {
        self.secrets = secrets;
        self
    }

    pub fn with_tools<T>(mut self, tools: Arc<T>) -> Self
    where
        T: WasmHostTools + 'static,
    {
        self.tools = tools;
        self
    }

    pub fn with_clock<T>(mut self, clock: Arc<T>) -> Self
    where
        T: WasmHostClock + 'static,
    {
        self.clock = clock;
        self
    }
}

impl Default for WitToolHost {
    fn default() -> Self {
        Self::deny_all()
    }
}

/// Resource limits for one WIT tool execution.
#[derive(Debug, Clone)]
pub struct WitToolLimits {
    pub memory_bytes: u64,
    pub fuel: u64,
    pub timeout: Duration,
}

impl Default for WitToolLimits {
    fn default() -> Self {
        Self {
            memory_bytes: DEFAULT_MEMORY_BYTES,
            fuel: DEFAULT_FUEL,
            timeout: DEFAULT_TIMEOUT,
        }
    }
}

impl WitToolLimits {
    pub fn with_memory_bytes(mut self, memory_bytes: u64) -> Self {
        self.memory_bytes = memory_bytes;
        self
    }

    pub fn with_fuel(mut self, fuel: u64) -> Self {
        self.fuel = fuel;
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

/// Configuration for the Reborn WIT tool runtime.
#[derive(Debug, Clone, Default)]
pub struct WitToolRuntimeConfig {
    pub default_limits: WitToolLimits,
}

impl WitToolRuntimeConfig {
    pub fn for_testing() -> Self {
        Self {
            default_limits: WitToolLimits::default()
                .with_memory_bytes(1024 * 1024)
                .with_fuel(100_000)
                .with_timeout(Duration::from_secs(5)),
        }
    }
}

/// Compiled WIT tool component plus metadata extracted from its WIT exports.
pub struct PreparedWitTool {
    name: String,
    description: String,
    schema: serde_json::Value,
    component: wasmtime::component::Component,
    limits: WitToolLimits,
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

    pub fn limits(&self) -> &WitToolLimits {
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

/// Result of one WIT tool execution.
#[derive(Debug, Clone, PartialEq)]
pub struct WitToolExecution {
    pub output_json: Option<String>,
    pub error: Option<String>,
    pub usage: ResourceUsage,
    pub logs: Vec<WasmLogRecord>,
}

/// Reborn WIT-compatible WASM tool runtime.
pub struct WitToolRuntime {
    engine: Engine,
    config: WitToolRuntimeConfig,
}

impl WitToolRuntime {
    pub fn new(config: WitToolRuntimeConfig) -> Result<Self, WasmError> {
        let mut wasmtime_config = Config::new();
        wasmtime_config.wasm_component_model(true);
        wasmtime_config.wasm_threads(false);
        wasmtime_config.consume_fuel(true);
        wasmtime_config.epoch_interruption(true);
        wasmtime_config.debug_info(false);

        let engine = Engine::new(&wasmtime_config)
            .map_err(|error| WasmError::EngineCreationFailed(error.to_string()))?;
        spawn_epoch_ticker(engine.clone())?;

        Ok(Self { engine, config })
    }

    pub fn config(&self) -> &WitToolRuntimeConfig {
        &self.config
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    pub fn prepare(&self, name: &str, wasm_bytes: &[u8]) -> Result<PreparedWitTool, WasmError> {
        let component = wasmtime::component::Component::new(&self.engine, wasm_bytes)
            .map_err(|error| WasmError::CompilationFailed(error.to_string()))?;
        let limits = self.config.default_limits.clone();
        let (description, schema) = self.extract_metadata(&component, &limits)?;

        Ok(PreparedWitTool {
            name: name.to_string(),
            description,
            schema,
            component,
            limits,
        })
    }

    pub fn execute(
        &self,
        prepared: &PreparedWitTool,
        host: WitToolHost,
        request: WitToolRequest,
    ) -> Result<WitToolExecution, WasmError> {
        let started = Instant::now();
        let (mut store, instance) =
            self.instantiate(&prepared.component, host, &prepared.limits)?;
        let tool = instance.near_agent_tool();
        let request = bindings::exports::near::agent::tool::Request {
            params: request.params_json,
            context: request.context_json,
        };
        let response = tool
            .call_execute(&mut store, &request)
            .map_err(|error| WasmError::ExecutionFailed(error.to_string()))?;

        let mut usage = store.data().usage.clone();
        usage.wall_clock_ms = elapsed_millis(started);
        usage.output_bytes = response
            .output
            .as_deref()
            .map(|output| output.len().min(u64::MAX as usize) as u64)
            .unwrap_or(0);
        let logs = store.data().logs.clone();

        Ok(WitToolExecution {
            output_json: response.output,
            error: response.error,
            usage,
            logs,
        })
    }

    fn extract_metadata(
        &self,
        component: &wasmtime::component::Component,
        limits: &WitToolLimits,
    ) -> Result<(String, serde_json::Value), WasmError> {
        let (mut store, instance) = self.instantiate(component, WitToolHost::deny_all(), limits)?;
        let tool = instance.near_agent_tool();
        let description = tool
            .call_description(&mut store)
            .map_err(|error| WasmError::ExecutionFailed(error.to_string()))?;
        let schema_json = tool
            .call_schema(&mut store)
            .map_err(|error| WasmError::ExecutionFailed(error.to_string()))?;
        let schema = serde_json::from_str::<serde_json::Value>(&schema_json)
            .map_err(|error| WasmError::InvalidSchema(error.to_string()))?;
        if !schema.is_object() {
            return Err(WasmError::InvalidSchema(
                "schema export must return a JSON object".to_string(),
            ));
        }
        Ok((description, schema))
    }

    fn instantiate(
        &self,
        component: &wasmtime::component::Component,
        host: WitToolHost,
        limits: &WitToolLimits,
    ) -> Result<(Store<StoreData>, bindings::SandboxedTool), WasmError> {
        let mut store = Store::new(&self.engine, StoreData::new(host, limits.memory_bytes));
        configure_store(&mut store, limits)?;
        let linker = create_linker(&self.engine)?;
        let instance = bindings::SandboxedTool::instantiate(&mut store, component, &linker)
            .map_err(|error| classify_instantiation_error(error.to_string()))?;
        Ok((store, instance))
    }
}

impl std::fmt::Debug for WitToolRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WitToolRuntime")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

fn spawn_epoch_ticker(engine: Engine) -> Result<(), WasmError> {
    std::thread::Builder::new()
        .name("reborn-wasm-epoch-ticker".into())
        .spawn(move || {
            loop {
                std::thread::sleep(EPOCH_TICK_INTERVAL);
                engine.increment_epoch();
            }
        })
        .map(|_| ())
        .map_err(|error| WasmError::EngineCreationFailed(error.to_string()))
}

fn elapsed_millis(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

fn configure_store(store: &mut Store<StoreData>, limits: &WitToolLimits) -> Result<(), WasmError> {
    store
        .set_fuel(limits.fuel)
        .map_err(|error| WasmError::StoreConfiguration(error.to_string()))?;
    store.epoch_deadline_trap();
    let ticks = (limits.timeout.as_millis() / EPOCH_TICK_INTERVAL.as_millis()).max(1) as u64;
    store.set_epoch_deadline(ticks);
    store.limiter(|data| &mut data.limiter);
    Ok(())
}

fn create_linker(engine: &Engine) -> Result<Linker<StoreData>, WasmError> {
    let mut linker = Linker::new(engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker)
        .map_err(|error| WasmError::LinkerConfiguration(error.to_string()))?;
    bindings::SandboxedTool::add_to_linker::<_, wasmtime::component::HasSelf<_>>(
        &mut linker,
        |state: &mut StoreData| state,
    )
    .map_err(|error| WasmError::LinkerConfiguration(error.to_string()))?;
    Ok(linker)
}

fn classify_instantiation_error(message: String) -> WasmError {
    if message.contains("near:agent") || message.contains("import") {
        WasmError::InstantiationFailed(format!(
            "{message}. This usually means the component was compiled against a different WIT version than the host supports (host: {WIT_TOOL_VERSION})."
        ))
    } else {
        WasmError::InstantiationFailed(message)
    }
}

struct StoreData {
    host: WitToolHost,
    limiter: WasmResourceLimiter,
    wasi: WasiCtx,
    table: ResourceTable,
    usage: ResourceUsage,
    logs: Vec<WasmLogRecord>,
}

impl StoreData {
    fn new(host: WitToolHost, memory_limit: u64) -> Self {
        Self {
            host,
            limiter: WasmResourceLimiter::new(memory_limit),
            wasi: WasiCtxBuilder::new().build(),
            table: ResourceTable::new(),
            usage: ResourceUsage::default(),
            logs: Vec::new(),
        }
    }
}

impl WasiView for StoreData {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl bindings::near::agent::host::Host for StoreData {
    fn log(&mut self, level: bindings::near::agent::host::LogLevel, message: String) {
        if self.logs.len() >= MAX_LOGS_PER_EXECUTION {
            return;
        }
        let message = truncate_log_message(message);
        let level = match level {
            bindings::near::agent::host::LogLevel::Trace => WasmLogLevel::Trace,
            bindings::near::agent::host::LogLevel::Debug => WasmLogLevel::Debug,
            bindings::near::agent::host::LogLevel::Info => WasmLogLevel::Info,
            bindings::near::agent::host::LogLevel::Warn => WasmLogLevel::Warn,
            bindings::near::agent::host::LogLevel::Error => WasmLogLevel::Error,
        };
        self.logs.push(WasmLogRecord { level, message });
    }

    fn now_millis(&mut self) -> u64 {
        self.host.clock.now_millis()
    }

    fn workspace_read(&mut self, path: String) -> Option<String> {
        self.host.workspace.read(&path)
    }

    fn http_request(
        &mut self,
        method: String,
        url: String,
        headers_json: String,
        body: Option<Vec<u8>>,
        timeout_ms: Option<u32>,
    ) -> Result<bindings::near::agent::host::HttpResponse, String> {
        let request_body_bytes = body.as_ref().map(|body| body.len() as u64).unwrap_or(0);
        let response = self.host.http.request(WasmHttpRequest {
            method,
            url,
            headers_json,
            body,
            timeout_ms,
        });
        match response {
            Ok(response) => {
                self.record_network_egress(request_body_bytes);
                Ok(bindings::near::agent::host::HttpResponse {
                    status: response.status,
                    headers_json: response.headers_json,
                    body: response.body,
                })
            }
            Err(error) => {
                if error.request_was_sent() {
                    self.record_network_egress(request_body_bytes);
                }
                Err(error.to_string())
            }
        }
    }

    fn tool_invoke(&mut self, alias: String, params_json: String) -> Result<String, String> {
        self.host
            .tools
            .invoke(&alias, &params_json)
            .map_err(|error| error.to_string())
    }

    fn secret_exists(&mut self, name: String) -> bool {
        self.host.secrets.exists(&name)
    }
}

impl StoreData {
    fn record_network_egress(&mut self, request_body_bytes: u64) {
        self.usage.network_egress_bytes = self
            .usage
            .network_egress_bytes
            .saturating_add(request_body_bytes);
    }
}

fn truncate_log_message(message: String) -> String {
    if message.len() <= MAX_LOG_MESSAGE_BYTES {
        return message;
    }

    let mut end = MAX_LOG_MESSAGE_BYTES;
    while !message.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    message[..end].to_string()
}

#[derive(Debug)]
struct WasmResourceLimiter {
    memory_limit: u64,
    memory_used: u64,
    max_tables: u32,
    max_instances: u32,
}

impl WasmResourceLimiter {
    fn new(memory_limit: u64) -> Self {
        Self {
            memory_limit,
            memory_used: 0,
            max_tables: 10,
            max_instances: 10,
        }
    }
}

impl ResourceLimiter for WasmResourceLimiter {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> Result<bool, wasmtime::Error> {
        let desired = desired as u64;
        if desired > self.memory_limit {
            tracing::warn!(
                current,
                desired,
                limit = self.memory_limit,
                "WASM memory growth denied"
            );
            return Ok(false);
        }
        self.memory_used = desired;
        Ok(true)
    }

    fn table_growing(
        &mut self,
        current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> Result<bool, wasmtime::Error> {
        if desired > 10_000 {
            tracing::warn!(current, desired, "WASM table growth denied");
            return Ok(false);
        }
        Ok(true)
    }

    fn instances(&self) -> usize {
        self.max_instances as usize
    }

    fn tables(&self) -> usize {
        self.max_tables as usize
    }

    fn memories(&self) -> usize {
        self.max_instances as usize
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_LOG_MESSAGE_BYTES, truncate_log_message};

    #[test]
    fn truncate_log_message_respects_utf8_boundaries() {
        let message = "é".repeat(MAX_LOG_MESSAGE_BYTES);
        let truncated = truncate_log_message(message);
        assert!(truncated.len() <= MAX_LOG_MESSAGE_BYTES);
        assert!(truncated.is_char_boundary(truncated.len()));
    }
}
