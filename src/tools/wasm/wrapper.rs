//! WASM tool wrapper implementing the Tool trait.
//!
//! Uses wasmtime::component::bindgen! to generate typed bindings from the WIT
//! interface, ensuring all host functions are properly registered under the
//! correct `near:agent/host` namespace.
//!
//! Each execution creates a fresh instance (NEAR pattern) to ensure
//! isolation and deterministic behavior.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use wasmtime::Store;
use wasmtime::component::{Component, Linker};
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiView};

use crate::context::JobContext;
use crate::safety::LeakDetector;
use crate::tools::tool::{Tool, ToolError, ToolOutput};
use crate::tools::wasm::capabilities::Capabilities;
use crate::tools::wasm::error::WasmError;
use crate::tools::wasm::host::{HostState, LogLevel};
use crate::tools::wasm::limits::{ResourceLimits, WasmResourceLimiter};
use crate::tools::wasm::runtime::{EPOCH_TICK_INTERVAL, PreparedModule, WasmToolRuntime};

// Generate component model bindings from the WIT file.
//
// This creates:
// - `near::agent::host::Host` trait + `add_to_linker()` for the import interface
// - `SandboxedTool` struct with `instantiate()` for the world
// - `exports::near::agent::tool::*` types for the export interface
wasmtime::component::bindgen!({
    path: "wit/tool.wit",
    world: "sandboxed-tool",
    async: false,
    with: {},
});

// Alias the export interface types for convenience.
use exports::near::agent::tool as wit_tool;

/// Store data for WASM tool execution.
///
/// Contains the resource limiter, host state, WASI context, and injected
/// credentials. Fresh instance created per execution (NEAR pattern).
struct StoreData {
    limiter: WasmResourceLimiter,
    host_state: HostState,
    wasi: WasiCtx,
    table: ResourceTable,
    /// Injected credentials for URL/header substitution.
    /// Keys are placeholder names like "GOOGLE_ACCESS_TOKEN".
    credentials: HashMap<String, String>,
}

impl StoreData {
    fn new(
        memory_limit: u64,
        capabilities: Capabilities,
        credentials: HashMap<String, String>,
    ) -> Self {
        // Minimal WASI context: no filesystem, no env vars (security)
        let wasi = WasiCtxBuilder::new().build();

        Self {
            limiter: WasmResourceLimiter::new(memory_limit),
            host_state: HostState::new(capabilities),
            wasi,
            table: ResourceTable::new(),
            credentials,
        }
    }

    /// Inject credentials into a string by replacing placeholders.
    ///
    /// Replaces patterns like `{GOOGLE_ACCESS_TOKEN}` with actual values.
    /// WASM tools reference credentials by placeholder, never seeing real values.
    fn inject_credentials(&self, input: &str, context: &str) -> String {
        let mut result = input.to_string();

        for (name, value) in &self.credentials {
            let placeholder = format!("{{{}}}", name);
            if result.contains(&placeholder) {
                tracing::debug!(
                    placeholder = %placeholder,
                    context = %context,
                    "Replacing credential placeholder in tool request"
                );
                result = result.replace(&placeholder, value);
            }
        }

        result
    }

    /// Replace injected credential values with `[REDACTED]` in text.
    ///
    /// Prevents credentials from leaking through error messages or logs.
    /// reqwest::Error includes the full URL in its Display output, so any
    /// error from an injected-URL request will contain the raw credential
    /// unless we scrub it.
    fn redact_credentials(&self, text: &str) -> String {
        let mut result = text.to_string();
        for (name, value) in &self.credentials {
            if !value.is_empty() {
                result = result.replace(value, &format!("[REDACTED:{}]", name));
            }
        }
        result
    }
}

// Provide WASI context for the WASM component.
// Required because tools are compiled with wasm32-wasip2 target.
impl WasiView for StoreData {
    fn ctx(&mut self) -> &mut WasiCtx {
        &mut self.wasi
    }

    fn table(&mut self) -> &mut ResourceTable {
        &mut self.table
    }
}

// Implement the generated Host trait from bindgen.
//
// This registers all 6 host functions under the `near:agent/host` namespace:
// log, now-millis, workspace-read, http-request, secret-exists, tool-invoke
impl near::agent::host::Host for StoreData {
    fn log(&mut self, level: near::agent::host::LogLevel, message: String) {
        let log_level = match level {
            near::agent::host::LogLevel::Trace => LogLevel::Trace,
            near::agent::host::LogLevel::Debug => LogLevel::Debug,
            near::agent::host::LogLevel::Info => LogLevel::Info,
            near::agent::host::LogLevel::Warn => LogLevel::Warn,
            near::agent::host::LogLevel::Error => LogLevel::Error,
        };
        let _ = self.host_state.log(log_level, message);
    }

    fn now_millis(&mut self) -> u64 {
        self.host_state.now_millis()
    }

    fn workspace_read(&mut self, path: String) -> Option<String> {
        self.host_state.workspace_read(&path).ok().flatten()
    }

    fn http_request(
        &mut self,
        method: String,
        url: String,
        headers_json: String,
        body: Option<Vec<u8>>,
        timeout_ms: Option<u32>,
    ) -> Result<near::agent::host::HttpResponse, String> {
        // Inject credentials into URL (e.g., replace {TELEGRAM_BOT_TOKEN})
        let injected_url = self.inject_credentials(&url, "url");

        // Check HTTP allowlist
        self.host_state
            .check_http_allowed(&injected_url, &method)
            .map_err(|e| format!("HTTP not allowed: {}", e))?;

        // Record for rate limiting
        self.host_state
            .record_http_request()
            .map_err(|e| format!("Rate limit exceeded: {}", e))?;

        // Parse headers and inject credentials into header values
        let raw_headers: HashMap<String, String> =
            serde_json::from_str(&headers_json).unwrap_or_default();

        let headers: HashMap<String, String> = raw_headers
            .into_iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    self.inject_credentials(&v, &format!("header:{}", k)),
                )
            })
            .collect();

        let url = injected_url;
        let leak_detector = LeakDetector::new();
        let header_vec: Vec<(String, String)> = headers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        leak_detector
            .scan_http_request(&url, &header_vec, body.as_deref())
            .map_err(|e| format!("Potential secret leak blocked: {}", e))?;

        // Get the max response size from capabilities (default 10MB).
        let max_response_bytes = self
            .host_state
            .capabilities()
            .http
            .as_ref()
            .map(|h| h.max_response_bytes)
            .unwrap_or(10 * 1024 * 1024);

        // Resolve hostname and reject private/internal IPs to prevent DNS rebinding.
        reject_private_ip(&url)?;

        // Make HTTP request using blocking I/O.
        // We're inside a spawn_blocking context, so use block_on.
        let result = tokio::runtime::Handle::current().block_on(async {
            let client = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|e| format!("failed to create HTTP client: {e}"))?;

            let mut request = match method.to_uppercase().as_str() {
                "GET" => client.get(&url),
                "POST" => client.post(&url),
                "PUT" => client.put(&url),
                "DELETE" => client.delete(&url),
                "PATCH" => client.patch(&url),
                "HEAD" => client.head(&url),
                _ => return Err(format!("Unsupported HTTP method: {}", method)),
            };

            for (key, value) in headers {
                request = request.header(&key, &value);
            }

            if let Some(body_bytes) = body {
                request = request.body(body_bytes);
            }

            // Caller-specified timeout (default 30s)
            let timeout = Duration::from_millis(timeout_ms.unwrap_or(30_000) as u64);
            let response = request.timeout(timeout).send().await.map_err(|e| {
                // Walk the full error chain for the actual root cause
                let mut chain = format!("HTTP request failed: {}", e);
                let mut source = std::error::Error::source(&e);
                while let Some(cause) = source {
                    chain.push_str(&format!(" -> {}", cause));
                    source = cause.source();
                }
                chain
            })?;

            let status = response.status().as_u16();
            let response_headers: HashMap<String, String> = response
                .headers()
                .iter()
                .filter_map(|(k, v)| {
                    v.to_str()
                        .ok()
                        .map(|v| (k.as_str().to_string(), v.to_string()))
                })
                .collect();
            let headers_json = serde_json::to_string(&response_headers).unwrap_or_default();

            // Check Content-Length header for early rejection of oversized responses.
            let max_response = max_response_bytes;
            if let Some(cl) = response.content_length() {
                if cl as usize > max_response {
                    return Err(format!(
                        "Response body too large: {} bytes exceeds limit of {} bytes",
                        cl, max_response
                    ));
                }
            }

            // Read body with a size cap to prevent memory exhaustion.
            let body = response
                .bytes()
                .await
                .map_err(|e| format!("Failed to read response body: {}", e))?;
            if body.len() > max_response {
                return Err(format!(
                    "Response body too large: {} bytes exceeds limit of {} bytes",
                    body.len(),
                    max_response
                ));
            }
            let body = body.to_vec();

            // Leak detection on response body
            if let Ok(body_str) = std::str::from_utf8(&body) {
                leak_detector
                    .scan_and_clean(body_str)
                    .map_err(|e| format!("Potential secret leak in response: {}", e))?;
            }

            Ok(near::agent::host::HttpResponse {
                status,
                headers_json,
                body,
            })
        });

        // Redact credentials from error messages before returning to WASM
        result.map_err(|e| self.redact_credentials(&e))
    }

    fn tool_invoke(&mut self, alias: String, _params_json: String) -> Result<String, String> {
        // Validate capability and resolve alias
        let _real_name = self.host_state.check_tool_invoke_allowed(&alias)?;
        self.host_state.record_tool_invoke()?;

        // Tool invocation requires async context and access to the tool registry,
        // which aren't available inside a synchronous WASM callback.
        Err("Tool invocation from WASM tools is not yet supported".to_string())
    }

    fn secret_exists(&mut self, name: String) -> bool {
        self.host_state.secret_exists(&name)
    }
}

/// A Tool implementation backed by a WASM component.
///
/// Each call to `execute` creates a fresh instance for isolation.
pub struct WasmToolWrapper {
    /// Runtime for engine access.
    runtime: Arc<WasmToolRuntime>,
    /// Prepared module with compiled component.
    prepared: Arc<PreparedModule>,
    /// Capabilities to grant to this tool.
    capabilities: Capabilities,
    /// Cached description (from PreparedModule or override).
    description: String,
    /// Cached schema (from PreparedModule or override).
    schema: serde_json::Value,
    /// Injected credentials for HTTP requests (e.g., OAuth tokens).
    /// Keys are placeholder names like "GOOGLE_ACCESS_TOKEN".
    credentials: HashMap<String, String>,
}

impl WasmToolWrapper {
    /// Create a new WASM tool wrapper.
    pub fn new(
        runtime: Arc<WasmToolRuntime>,
        prepared: Arc<PreparedModule>,
        capabilities: Capabilities,
    ) -> Self {
        Self {
            description: prepared.description.clone(),
            schema: prepared.schema.clone(),
            runtime,
            prepared,
            capabilities,
            credentials: HashMap::new(),
        }
    }

    /// Override the tool description.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// Override the parameter schema.
    pub fn with_schema(mut self, schema: serde_json::Value) -> Self {
        self.schema = schema;
        self
    }

    /// Set credentials for HTTP request injection.
    pub fn with_credentials(mut self, credentials: HashMap<String, String>) -> Self {
        self.credentials = credentials;
        self
    }

    /// Get the resource limits for this tool.
    pub fn limits(&self) -> &ResourceLimits {
        &self.prepared.limits
    }

    /// Add all host functions to the linker using generated bindings.
    ///
    /// Uses the bindgen-generated `add_to_linker` function to properly register
    /// all host functions with correct component model signatures under the
    /// `near:agent/host` namespace.
    fn add_host_functions(linker: &mut Linker<StoreData>) -> Result<(), WasmError> {
        // Add WASI support (required by components built with wasm32-wasip2)
        wasmtime_wasi::add_to_linker_sync(linker)
            .map_err(|e| WasmError::ConfigError(format!("Failed to add WASI functions: {}", e)))?;

        // Add our custom host interface using the generated add_to_linker
        near::agent::host::add_to_linker(linker, |state| state)
            .map_err(|e| WasmError::ConfigError(format!("Failed to add host functions: {}", e)))?;

        Ok(())
    }

    /// Execute the WASM tool synchronously (called from spawn_blocking).
    fn execute_sync(
        &self,
        params: serde_json::Value,
        context_json: Option<String>,
    ) -> Result<(String, Vec<crate::tools::wasm::host::LogEntry>), WasmError> {
        let engine = self.runtime.engine();
        let limits = &self.prepared.limits;

        // Create store with fresh state (NEAR pattern: fresh instance per call)
        let store_data = StoreData::new(
            limits.memory_bytes,
            self.capabilities.clone(),
            self.credentials.clone(),
        );
        let mut store = Store::new(engine, store_data);

        // Configure fuel if enabled
        if self.runtime.config().fuel_config.enabled {
            store
                .set_fuel(limits.fuel)
                .map_err(|e| WasmError::ConfigError(format!("Failed to set fuel: {}", e)))?;
        }

        // Configure epoch deadline as a hard timeout backup.
        // The epoch ticker thread increments the engine epoch every EPOCH_TICK_INTERVAL.
        // Setting deadline to N means "trap after N ticks", so we compute the number
        // of ticks that fit in the tool's timeout. Minimum 1 to always have a backstop.
        store.epoch_deadline_trap();
        let ticks = (limits.timeout.as_millis() / EPOCH_TICK_INTERVAL.as_millis()).max(1) as u64;
        store.set_epoch_deadline(ticks);

        // Set up resource limiter
        store.limiter(|data| &mut data.limiter);

        // Compile the component (uses cached bytes)
        let component = Component::new(engine, self.prepared.component_bytes())
            .map_err(|e| WasmError::CompilationFailed(e.to_string()))?;

        // Create linker with all host functions properly namespaced
        let mut linker = Linker::new(engine);
        Self::add_host_functions(&mut linker)?;

        // Instantiate using the generated bindings
        let instance = SandboxedTool::instantiate(&mut store, &component, &linker)
            .map_err(|e| WasmError::InstantiationFailed(e.to_string()))?;

        // Prepare the request
        let params_json = serde_json::to_string(&params)
            .map_err(|e| WasmError::InvalidResponseJson(e.to_string()))?;

        let request = wit_tool::Request {
            params: params_json,
            context: context_json,
        };

        // Call execute using the generated typed interface
        let tool_iface = instance.near_agent_tool();
        let response = tool_iface.call_execute(&mut store, &request).map_err(|e| {
            let error_str = e.to_string();
            if error_str.contains("out of fuel") {
                WasmError::FuelExhausted { limit: limits.fuel }
            } else if error_str.contains("unreachable") {
                WasmError::Trapped("unreachable code executed".to_string())
            } else {
                WasmError::Trapped(error_str)
            }
        })?;

        // Get logs from host state
        let logs = store.data_mut().host_state.take_logs();

        // Check for tool-level error
        if let Some(err) = response.error {
            return Err(WasmError::ToolReturnedError(err));
        }

        // Return result (or empty string if none)
        Ok((response.output.unwrap_or_default(), logs))
    }
}

#[async_trait]
impl Tool for WasmToolWrapper {
    fn name(&self) -> &str {
        &self.prepared.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> serde_json::Value {
        self.schema.clone()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let timeout = self.prepared.limits.timeout;

        // Serialize context for WASM
        let context_json = serde_json::to_string(ctx).ok();

        // Clone what we need for the blocking task
        let runtime = Arc::clone(&self.runtime);
        let prepared = Arc::clone(&self.prepared);
        let capabilities = self.capabilities.clone();
        let description = self.description.clone();
        let schema = self.schema.clone();
        let credentials = self.credentials.clone();

        // Execute in blocking task with timeout
        let result = tokio::time::timeout(timeout, async move {
            let wrapper = WasmToolWrapper {
                runtime,
                prepared,
                capabilities,
                description,
                schema,
                credentials,
            };

            tokio::task::spawn_blocking(move || wrapper.execute_sync(params, context_json))
                .await
                .map_err(|e| WasmError::ExecutionPanicked(e.to_string()))?
        })
        .await;

        let duration = start.elapsed();

        match result {
            Ok(Ok((result_json, logs))) => {
                // Emit collected logs
                for log in logs {
                    match log.level {
                        LogLevel::Trace => tracing::trace!(target: "wasm_tool", "{}", log.message),
                        LogLevel::Debug => tracing::debug!(target: "wasm_tool", "{}", log.message),
                        LogLevel::Info => tracing::info!(target: "wasm_tool", "{}", log.message),
                        LogLevel::Warn => tracing::warn!(target: "wasm_tool", "{}", log.message),
                        LogLevel::Error => tracing::error!(target: "wasm_tool", "{}", log.message),
                    }
                }

                // Parse result JSON
                let result: serde_json::Value = serde_json::from_str(&result_json)
                    .unwrap_or(serde_json::Value::String(result_json));

                Ok(ToolOutput::success(result, duration))
            }
            Ok(Err(wasm_err)) => Err(wasm_err.into()),
            Err(_) => Err(WasmError::Timeout(timeout).into()),
        }
    }

    fn requires_sanitization(&self) -> bool {
        // WASM tools always require sanitization, they're untrusted by definition
        true
    }

    fn estimated_duration(&self, _params: &serde_json::Value) -> Option<Duration> {
        // Use the timeout as a conservative estimate
        Some(self.prepared.limits.timeout)
    }
}

impl std::fmt::Debug for WasmToolWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmToolWrapper")
            .field("name", &self.prepared.name)
            .field("description", &self.description)
            .field("limits", &self.prepared.limits)
            .finish()
    }
}

/// Resolve the URL's hostname and reject connections to private/internal IP addresses.
/// This prevents DNS rebinding attacks where an attacker's domain resolves to an
/// internal IP after passing the allowlist check.
fn reject_private_ip(url: &str) -> Result<(), String> {
    let host = url
        .split("://")
        .nth(1)
        .and_then(|rest| {
            let host_and_port = rest.split('/').next().unwrap_or(rest);
            // Strip port
            if host_and_port.starts_with('[') {
                // IPv6
                host_and_port.find(']').map(|i| &host_and_port[1..i])
            } else {
                Some(
                    host_and_port
                        .rfind(':')
                        .map_or(host_and_port, |i| &host_and_port[..i]),
                )
            }
        })
        .ok_or_else(|| "Failed to parse host from URL".to_string())?;

    // If the host is already an IP, check it directly
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return if is_private_ip(ip) {
            Err(format!(
                "HTTP request to private/internal IP {} is not allowed",
                ip
            ))
        } else {
            Ok(())
        };
    }

    // Resolve DNS and check all addresses
    use std::net::ToSocketAddrs;
    // Port 0 is a placeholder; ToSocketAddrs needs host:port but the port
    // doesn't affect which IPs the hostname resolves to.
    let addrs: Vec<_> = format!("{}:0", host)
        .to_socket_addrs()
        .map_err(|e| format!("DNS resolution failed for {}: {}", host, e))?
        .collect();

    if addrs.is_empty() {
        return Err(format!("DNS resolution returned no addresses for {}", host));
    }

    for addr in &addrs {
        if is_private_ip(addr.ip()) {
            return Err(format!(
                "DNS rebinding detected: {} resolved to private IP {}",
                host,
                addr.ip()
            ));
        }
    }

    Ok(())
}

/// Check if an IP address belongs to a private/internal range.
fn is_private_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            v4.is_loopback()           // 127.0.0.0/8
            || v4.is_private()         // 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16
            || v4.is_link_local()      // 169.254.0.0/16
            || v4.is_unspecified()     // 0.0.0.0
            || v4.octets()[0] == 100 && (v4.octets()[1] & 0xC0) == 64 // 100.64.0.0/10 (CGNAT)
        }
        std::net::IpAddr::V6(v6) => {
            v6.is_loopback()           // ::1
            || v6.is_unspecified()     // ::
            // fc00::/7 (unique local)
            || (v6.segments()[0] & 0xFE00) == 0xFC00
            // fe80::/10 (link-local)
            || (v6.segments()[0] & 0xFFC0) == 0xFE80
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::tools::wasm::capabilities::Capabilities;
    use crate::tools::wasm::runtime::{WasmRuntimeConfig, WasmToolRuntime};

    #[test]
    fn test_wrapper_creation() {
        // This test verifies the runtime can be created
        // Actual execution tests require a valid WASM component
        let config = WasmRuntimeConfig::for_testing();
        let runtime = Arc::new(WasmToolRuntime::new(config).unwrap());

        // Runtime was created successfully
        assert!(runtime.config().fuel_config.enabled);
    }

    #[test]
    fn test_capabilities_default() {
        let caps = Capabilities::default();
        assert!(caps.workspace_read.is_none());
        assert!(caps.http.is_none());
        assert!(caps.tool_invoke.is_none());
        assert!(caps.secrets.is_none());
    }

    #[test]
    fn test_is_private_ip_v4() {
        use std::net::IpAddr;
        // Private ranges
        assert!(super::is_private_ip("127.0.0.1".parse::<IpAddr>().unwrap()));
        assert!(super::is_private_ip("10.0.0.1".parse::<IpAddr>().unwrap()));
        assert!(super::is_private_ip(
            "172.16.0.1".parse::<IpAddr>().unwrap()
        ));
        assert!(super::is_private_ip(
            "192.168.1.1".parse::<IpAddr>().unwrap()
        ));
        assert!(super::is_private_ip(
            "169.254.1.1".parse::<IpAddr>().unwrap()
        ));
        assert!(super::is_private_ip("0.0.0.0".parse::<IpAddr>().unwrap()));
        // CGNAT
        assert!(super::is_private_ip(
            "100.64.0.1".parse::<IpAddr>().unwrap()
        ));

        // Public IPs
        assert!(!super::is_private_ip("8.8.8.8".parse::<IpAddr>().unwrap()));
        assert!(!super::is_private_ip("1.1.1.1".parse::<IpAddr>().unwrap()));
        assert!(!super::is_private_ip(
            "93.184.216.34".parse::<IpAddr>().unwrap()
        ));
    }

    #[test]
    fn test_is_private_ip_v6() {
        use std::net::IpAddr;
        assert!(super::is_private_ip("::1".parse::<IpAddr>().unwrap()));
        assert!(super::is_private_ip("::".parse::<IpAddr>().unwrap()));
        assert!(super::is_private_ip("fc00::1".parse::<IpAddr>().unwrap()));
        assert!(super::is_private_ip("fe80::1".parse::<IpAddr>().unwrap()));

        // Public
        assert!(!super::is_private_ip(
            "2606:4700::1111".parse::<IpAddr>().unwrap()
        ));
    }

    #[test]
    fn test_reject_private_ip_loopback() {
        let result = super::reject_private_ip("https://127.0.0.1:8080/api");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("private/internal IP"));
    }

    #[test]
    fn test_reject_private_ip_internal() {
        let result = super::reject_private_ip("https://192.168.1.1/admin");
        assert!(result.is_err());
    }

    #[test]
    fn test_reject_private_ip_public_ok() {
        // 8.8.8.8 (Google DNS) is public
        let result = super::reject_private_ip("https://8.8.8.8/dns-query");
        assert!(result.is_ok());
    }
}
