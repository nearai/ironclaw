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
use wasmtime::component::Linker;
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiView};

use crate::context::JobContext;
use crate::safety::LeakDetector;
use crate::secrets::SecretsStore;
use crate::tools::tool::{Tool, ToolError, ToolOutput};
use crate::tools::wasm::capabilities::Capabilities;
use crate::tools::wasm::credential_injector::{
    InjectedCredentials, host_matches_pattern, inject_credential,
};
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

/// Configuration needed to refresh an expired OAuth access token.
///
/// Extracted at tool load time from the capabilities file's `auth.oauth` section.
/// Passed into `resolve_host_credentials()` so it can transparently refresh
/// tokens before WASM execution.
#[derive(Debug, Clone)]
pub struct OAuthRefreshConfig {
    /// OAuth token exchange URL (e.g., "https://oauth2.googleapis.com/token").
    pub token_url: String,
    /// OAuth client_id.
    pub client_id: String,
    /// OAuth client_secret (optional, some providers use PKCE without a secret).
    pub client_secret: Option<String>,
    /// Secret name of the access token (e.g., "google_oauth_token").
    /// The refresh token lives at `{secret_name}_refresh_token`.
    pub secret_name: String,
    /// Provider hint stored alongside the refreshed secret.
    pub provider: Option<String>,
}

/// Pre-resolved credential for host-based injection.
///
/// Built before each WASM execution by decrypting secrets from the store.
/// Applied per-request by matching the URL host against `host_patterns`.
/// WASM tools never see the raw secret values.
struct ResolvedHostCredential {
    /// Host patterns this credential applies to (e.g., "www.googleapis.com").
    host_patterns: Vec<String>,
    /// Headers to add to matching requests (e.g., "Authorization: Bearer ...").
    headers: HashMap<String, String>,
    /// Query parameters to add to matching requests.
    query_params: HashMap<String, String>,
    /// Raw secret value for redaction in error messages.
    secret_value: String,
}

/// Store data for WASM tool execution.
///
/// Contains the resource limiter, host state, WASI context, and injected
/// credentials. Fresh instance created per execution (NEAR pattern).
struct StoreData {
    limiter: WasmResourceLimiter,
    host_state: HostState,
    wasi: WasiCtx,
    table: ResourceTable,
    /// Injected credentials for URL/header placeholder substitution.
    /// Keys are placeholder names like "TELEGRAM_BOT_TOKEN".
    credentials: HashMap<String, String>,
    /// Pre-resolved credentials for automatic host-based injection.
    /// Applied by matching URL host against each credential's host_patterns.
    host_credentials: Vec<ResolvedHostCredential>,
    /// Dedicated tokio runtime for HTTP requests, lazily initialized.
    /// Reused across multiple `http_request` calls within one execution.
    http_runtime: Option<tokio::runtime::Runtime>,
}

impl StoreData {
    fn new(
        memory_limit: u64,
        capabilities: Capabilities,
        credentials: HashMap<String, String>,
        host_credentials: Vec<ResolvedHostCredential>,
    ) -> Self {
        // Minimal WASI context: no filesystem, no env vars (security)
        let wasi = WasiCtxBuilder::new().build();

        Self {
            limiter: WasmResourceLimiter::new(memory_limit),
            host_state: HostState::new(capabilities),
            wasi,
            table: ResourceTable::new(),
            credentials,
            host_credentials,
            http_runtime: None,
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
        for cred in &self.host_credentials {
            if !cred.secret_value.is_empty() {
                result = result.replace(&cred.secret_value, "[REDACTED:host_credential]");
            }
        }
        result
    }

    /// Inject pre-resolved host credentials into the request.
    ///
    /// Matches the URL host against each resolved credential's host_patterns.
    /// Matching credentials have their headers merged and query params appended.
    fn inject_host_credentials(
        &self,
        url_host: &str,
        headers: &mut HashMap<String, String>,
        url: &mut String,
    ) {
        for cred in &self.host_credentials {
            let matches = cred
                .host_patterns
                .iter()
                .any(|pattern| host_matches_pattern(url_host, pattern));

            if !matches {
                continue;
            }

            // Merge injected headers (host credentials take precedence)
            for (key, value) in &cred.headers {
                headers.insert(key.clone(), value.clone());
            }

            // Append query parameters to URL (insert before fragment if present)
            if !cred.query_params.is_empty() {
                let (base, fragment) = match url.find('#') {
                    Some(i) => (url[..i].to_string(), Some(url[i..].to_string())),
                    None => (url.clone(), None),
                };
                *url = base;

                let separator = if url.contains('?') { '&' } else { '?' };
                for (i, (name, value)) in cred.query_params.iter().enumerate() {
                    if i == 0 {
                        url.push(separator);
                    } else {
                        url.push('&');
                    }
                    url.push_str(&urlencoding::encode(name));
                    url.push('=');
                    url.push_str(&urlencoding::encode(value));
                }

                if let Some(frag) = fragment {
                    url.push_str(&frag);
                }
            }
        }
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

        // Leak scan runs on WASM-provided values BEFORE host credential injection.
        // This prevents false positives where the host-injected Bearer token
        // (e.g., xoxb- Slack token) triggers the leak detector — WASM never saw
        // the real value, so scanning the pre-injection state is correct.
        // Inline the scan to avoid allocating a Vec of cloned headers.
        let leak_detector = LeakDetector::new();
        leak_detector
            .scan_and_clean(&injected_url)
            .map_err(|e| format!("Potential secret leak in URL blocked: {}", e))?;
        for (name, value) in &raw_headers {
            leak_detector.scan_and_clean(value).map_err(|e| {
                format!("Potential secret leak in header '{}' blocked: {}", name, e)
            })?;
        }
        if let Some(body_bytes) = body.as_deref() {
            let body_str = String::from_utf8_lossy(body_bytes);
            leak_detector
                .scan_and_clean(&body_str)
                .map_err(|e| format!("Potential secret leak in body blocked: {}", e))?;
        }

        let mut headers: HashMap<String, String> = raw_headers
            .into_iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    self.inject_credentials(&v, &format!("header:{}", k)),
                )
            })
            .collect();

        let mut url = injected_url;

        // Inject pre-resolved host credentials (Bearer tokens, API keys, etc.)
        // based on the request's target host.
        if let Some(host) = extract_host_from_url(&url) {
            self.inject_host_credentials(&host, &mut headers, &mut url);
        }

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

        // Make HTTP request using a dedicated single-threaded runtime.
        // We're inside spawn_blocking, so we can't rely on the main runtime's
        // I/O driver (it may be busy with WASM compilation or other startup work).
        // A dedicated runtime gives us our own I/O driver and avoids contention.
        // The runtime is lazily created and reused across calls within one execution.
        if self.http_runtime.is_none() {
            self.http_runtime = Some(
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|e| format!("Failed to create HTTP runtime: {e}"))?,
            );
        }
        let rt = self.http_runtime.as_ref().expect("just initialized"); // safety: is_none branch above guarantees Some
        let result = rt.block_on(async {
            let client = reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

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

            // Caller-specified timeout (default 30s, max 5min)
            let timeout_ms = timeout_ms.unwrap_or(30_000).min(300_000) as u64;
            let timeout = Duration::from_millis(timeout_ms);
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
            if let Some(cl) = response.content_length()
                && cl as usize > max_response
            {
                return Err(format!(
                    "Response body too large: {} bytes exceeds limit of {} bytes",
                    cl, max_response
                ));
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
    /// Stored without any tool_info hints — hints are composed at display time.
    description: String,
    /// Compact and discovery schemas for this tool.
    schemas: WasmToolSchemas,
    /// Injected credentials for HTTP requests (e.g., OAuth tokens).
    /// Keys are placeholder names like "GOOGLE_ACCESS_TOKEN".
    credentials: HashMap<String, String>,
    /// Secrets store for resolving host-based credential injection.
    /// Used in execute() to pre-decrypt secrets before WASM runs.
    secrets_store: Option<Arc<dyn SecretsStore + Send + Sync>>,
    /// OAuth refresh configuration for auto-refreshing expired tokens.
    oauth_refresh: Option<OAuthRefreshConfig>,
}

#[derive(Debug, Clone)]
struct WasmToolSchemas {
    /// Compact schema advertised in the main tools array.
    ///
    /// This stays permissive by default to avoid serializing full exported
    /// WASM schemas on every LLM call. Sidecars can override it explicitly.
    advertised: serde_json::Value,
    /// Full schema available for discovery and runtime parameter preparation.
    ///
    /// Seeded from the WASM `schema()` export at registration time, unless a
    /// sidecar explicitly overrides it.
    discovery: serde_json::Value,
}

impl WasmToolSchemas {
    fn permissive_schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
            "additionalProperties": true
        })
    }

    fn is_permissive_schema(schema: &serde_json::Value) -> bool {
        schema
            .get("properties")
            .and_then(|p| p.as_object())
            .is_none_or(|p| p.is_empty())
    }

    fn typed_property_count(schema: &serde_json::Value) -> usize {
        schema
            .get("properties")
            .and_then(|p| p.as_object())
            .map(|props| {
                props
                    .values()
                    .filter(|prop| schema_is_typed_property(prop))
                    .count()
            })
            .unwrap_or(0)
    }

    fn new(discovery: serde_json::Value) -> Self {
        Self {
            advertised: Self::permissive_schema(),
            discovery,
        }
    }

    fn with_override(&self, schema: serde_json::Value) -> Self {
        Self {
            advertised: schema.clone(),
            discovery: schema,
        }
    }

    fn is_advertised_permissive(&self) -> bool {
        Self::is_permissive_schema(&self.advertised)
    }

    fn advertised(&self) -> serde_json::Value {
        self.advertised.clone()
    }

    fn discovery(&self) -> serde_json::Value {
        self.discovery.clone()
    }
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
            schemas: WasmToolSchemas::new(prepared.schema.clone()),
            runtime,
            prepared,
            capabilities,
            credentials: HashMap::new(),
            secrets_store: None,
            oauth_refresh: None,
        }
    }

    /// Override the tool description.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// Override the parameter schema.
    pub fn with_schema(mut self, schema: serde_json::Value) -> Self {
        let override_typed = WasmToolSchemas::typed_property_count(&schema);
        let prepared_typed = WasmToolSchemas::typed_property_count(&self.prepared.schema);

        if override_typed == 0 && prepared_typed > 0 {
            tracing::warn!(
                tool = %self.prepared.name,
                "Ignoring untyped schema override for discovery/runtime preparation and preserving extracted WASM schema"
            );
            self.schemas = WasmToolSchemas {
                advertised: schema,
                discovery: self.prepared.schema.clone(),
            };
        } else {
            self.schemas = self.schemas.with_override(schema);
        }
        self
    }

    /// Set credentials for HTTP request placeholder injection.
    pub fn with_credentials(mut self, credentials: HashMap<String, String>) -> Self {
        self.credentials = credentials;
        self
    }

    /// Set the secrets store for host-based credential injection.
    ///
    /// When set, credentials declared in the tool's capabilities are
    /// automatically decrypted and injected into HTTP requests based
    /// on the target host (e.g., Bearer token for www.googleapis.com).
    pub fn with_secrets_store(mut self, store: Arc<dyn SecretsStore + Send + Sync>) -> Self {
        self.secrets_store = Some(store);
        self
    }

    /// Set OAuth refresh configuration for auto-refreshing expired tokens.
    ///
    /// When set, `execute()` checks the access token's `expires_at` before
    /// each call and silently refreshes it using the stored refresh token.
    pub fn with_oauth_refresh(mut self, config: OAuthRefreshConfig) -> Self {
        self.oauth_refresh = Some(config);
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
        host_credentials: Vec<ResolvedHostCredential>,
    ) -> Result<(String, Vec<crate::tools::wasm::host::LogEntry>), WasmError> {
        let engine = self.runtime.engine();
        let limits = &self.prepared.limits;

        // Create store with fresh state (NEAR pattern: fresh instance per call)
        let store_data = StoreData::new(
            limits.memory_bytes,
            self.capabilities.clone(),
            self.credentials.clone(),
            host_credentials,
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

        // Use the pre-compiled component (no recompilation needed)
        let component = self.prepared.component().clone();

        // Create linker with all host functions properly namespaced
        let mut linker = Linker::new(engine);
        Self::add_host_functions(&mut linker)?;

        // Instantiate using the generated bindings
        let instance =
            SandboxedTool::instantiate(&mut store, &component, &linker).map_err(|e| {
                let msg = e.to_string();
                if msg.contains("near:agent") || msg.contains("import") {
                    WasmError::InstantiationFailed(format!(
                        "{msg}. This usually means the extension was compiled against \
                         a different WIT version than the host supports. \
                         Rebuild the extension against the current WIT (host: {}).",
                        crate::tools::wasm::WIT_TOOL_VERSION
                    ))
                } else {
                    WasmError::InstantiationFailed(msg)
                }
            })?;

        // Get typed interface — used for execute.
        let tool_iface = instance.near_agent_tool();

        // Prepare the request
        let params_json = serde_json::to_string(&params)
            .map_err(|e| WasmError::InvalidResponseJson(e.to_string()))?;

        let request = wit_tool::Request {
            params: params_json,
            context: context_json,
        };

        // Call execute using the generated typed interface
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

        // Check for tool-level error — point the LLM to tool_info for the
        // full schema instead of dumping ~3.5KB inline.
        if let Some(err) = response.error {
            let hint = build_tool_usage_hint(&self.prepared.name, &self.schemas.discovery());
            return Err(WasmError::ToolReturnedError { message: err, hint });
        }

        // Return result (or empty string if none)
        Ok((response.output.unwrap_or_default(), logs))
    }
}

/// Extract metadata (description + schema) from a WASM tool by briefly
/// instantiating it and calling its `description()` and `schema()` exports.
/// Analogous to MCP's `list_tools()` — discovers tool capabilities at load time.
///
/// Falls back to generic description and permissive schema on failure.
pub(super) fn extract_wasm_metadata(
    engine: &wasmtime::Engine,
    component: &wasmtime::component::Component,
    limits: &ResourceLimits,
) -> Result<(String, serde_json::Value), WasmError> {
    let store_data = StoreData::new(
        limits.memory_bytes,
        Capabilities::default(),
        HashMap::new(),
        vec![],
    );
    let mut store = Store::new(engine, store_data);

    // Configure fuel + epoch deadline so extraction can't hang
    if let Err(e) = store.set_fuel(limits.fuel) {
        tracing::debug!("Fuel not enabled for metadata extraction: {e}");
    }
    store.epoch_deadline_trap();
    let ticks = (limits.timeout.as_millis() / EPOCH_TICK_INTERVAL.as_millis()).max(1) as u64;
    store.set_epoch_deadline(ticks);
    store.limiter(|data| &mut data.limiter);

    // Instantiate with minimal linker
    let mut linker = Linker::new(engine);
    WasmToolWrapper::add_host_functions(&mut linker)?;
    let instance = SandboxedTool::instantiate(&mut store, component, &linker)
        .map_err(|e| WasmError::InstantiationFailed(e.to_string()))?;
    let tool_iface = instance.near_agent_tool();

    // Extract description (fall back to generic)
    let description = tool_iface
        .call_description(&mut store)
        .unwrap_or_else(|_| "WASM sandboxed tool".to_string());

    // Extract and parse schema (fall back to permissive)
    let schema = tool_iface
        .call_schema(&mut store)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .unwrap_or_else(|| {
            serde_json::json!({"type": "object", "properties": {}, "additionalProperties": true})
        });

    Ok((description, schema))
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
        self.schemas.advertised()
    }

    fn discovery_schema(&self) -> serde_json::Value {
        self.schemas.discovery()
    }

    /// Compose the tool schema for LLM function calling.
    ///
    /// When the advertised schema is permissive (no typed properties), appends
    /// a hint to the description directing the LLM to call `tool_info` for the
    /// full parameter schema. This keeps the raw description clean while still
    /// guiding the LLM.
    fn schema(&self) -> crate::tools::tool::ToolSchema {
        let description = if self.schemas.is_advertised_permissive() {
            format!(
                "{} (call tool_info(name: \"{}\", include_schema: true) for parameter schema)",
                self.description, self.prepared.name
            )
        } else {
            self.description.clone()
        };
        crate::tools::tool::ToolSchema {
            name: self.prepared.name.clone(),
            description,
            parameters: self.schemas.advertised(),
        }
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let timeout = self.prepared.limits.timeout;

        // Pre-resolve host credentials from secrets store (async, before blocking task).
        // This decrypts the secrets once so the sync http_request() host function
        // can inject them without needing async access.
        let credential_user_id = &ctx.user_id;
        let host_credentials = resolve_host_credentials(
            &self.capabilities,
            self.secrets_store.as_deref(),
            credential_user_id,
            self.oauth_refresh.as_ref(),
        )
        .await;

        // Serialize context for WASM
        let context_json = serde_json::to_string(ctx).ok();

        // Clone what we need for the blocking task
        let runtime = Arc::clone(&self.runtime);
        let prepared = Arc::clone(&self.prepared);
        let capabilities = self.capabilities.clone();
        let description = self.description.clone();
        let schemas = self.schemas.clone();
        let credentials = self.credentials.clone();

        // Execute in blocking task with timeout
        let result = tokio::time::timeout(timeout, async move {
            let wrapper = WasmToolWrapper {
                runtime,
                prepared,
                capabilities,
                description,
                schemas,
                credentials,
                secrets_store: None, // Not needed in blocking task
                oauth_refresh: None, // Already used above for pre-refresh
            };

            tokio::task::spawn_blocking(move || {
                wrapper.execute_sync(params, context_json, host_credentials)
            })
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

    fn webhook_capability(&self) -> Option<crate::tools::wasm::WebhookCapability> {
        self.capabilities.webhook.clone()
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

/// Refresh an expired OAuth access token using the stored refresh token.
///
/// Posts to the provider's token endpoint with `grant_type=refresh_token`,
/// then stores the new access token (with expiry) and rotated refresh token
/// (if the provider returns one).
///
/// SSRF defense: `token_url` originates from a tool's capabilities JSON, so
/// a malicious tool could point it at an internal service to exfiltrate the
/// refresh token. We require HTTPS, reject private/loopback IPs (including
/// DNS-resolved), and disable redirects.
///
/// Returns `true` if the refresh succeeded, `false` otherwise.
async fn refresh_oauth_token(
    store: &(dyn SecretsStore + Send + Sync),
    user_id: &str,
    config: &OAuthRefreshConfig,
) -> bool {
    // SSRF defense: token_url comes from the tool's capabilities file.
    if !config.token_url.starts_with("https://") {
        tracing::warn!(
            token_url = %config.token_url,
            "OAuth token_url must use HTTPS, refusing token refresh"
        );
        return false;
    }
    if let Err(reason) = reject_private_ip(&config.token_url) {
        tracing::warn!(
            token_url = %config.token_url,
            reason = %reason,
            "OAuth token_url points to a private/internal IP, refusing token refresh"
        );
        return false;
    }

    let refresh_name = format!("{}_refresh_token", config.secret_name);
    let refresh_secret = match store.get_decrypted(user_id, &refresh_name).await {
        Ok(s) => s,
        Err(e) => {
            tracing::debug!(
                secret_name = %refresh_name,
                error = %e,
                "No refresh token available, skipping token refresh"
            );
            return false;
        }
    };

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "Failed to build HTTP client for token refresh");
            return false;
        }
    };

    let mut params = vec![
        ("grant_type", "refresh_token".to_string()),
        ("refresh_token", refresh_secret.expose().to_string()),
        ("client_id", config.client_id.clone()),
    ];
    if let Some(ref secret) = config.client_secret {
        params.push(("client_secret", secret.clone()));
    }

    let response = match client.post(&config.token_url).form(&params).send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "OAuth token refresh request failed");
            return false;
        }
    };

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        tracing::warn!(
            status = %status,
            body = %body,
            "OAuth token refresh returned non-success status"
        );
        return false;
    }

    let token_data: serde_json::Value = match response.json().await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "Failed to parse token refresh response");
            return false;
        }
    };

    let new_access_token = match token_data.get("access_token").and_then(|v| v.as_str()) {
        Some(t) => t,
        None => {
            tracing::warn!("Token refresh response missing access_token field");
            return false;
        }
    };

    // Store the new access token with expiry
    let mut access_params =
        crate::secrets::CreateSecretParams::new(&config.secret_name, new_access_token);
    if let Some(ref provider) = config.provider {
        access_params = access_params.with_provider(provider);
    }
    if let Some(expires_in) = token_data.get("expires_in").and_then(|v| v.as_u64()) {
        let expires_at = chrono::Utc::now() + chrono::Duration::seconds(expires_in as i64);
        access_params = access_params.with_expiry(expires_at);
    }

    if let Err(e) = store.create(user_id, access_params).await {
        tracing::warn!(error = %e, "Failed to store refreshed access token");
        return false;
    }

    // Store rotated refresh token if the provider sent a new one
    if let Some(new_refresh) = token_data.get("refresh_token").and_then(|v| v.as_str()) {
        let mut refresh_params =
            crate::secrets::CreateSecretParams::new(&refresh_name, new_refresh);
        if let Some(ref provider) = config.provider {
            refresh_params = refresh_params.with_provider(provider);
        }
        if let Err(e) = store.create(user_id, refresh_params).await {
            tracing::warn!(error = %e, "Failed to store rotated refresh token");
        }
    }

    tracing::info!(
        secret_name = %config.secret_name,
        "OAuth access token refreshed successfully"
    );
    true
}

/// Pre-resolve credentials for all HTTP capability mappings.
///
/// Called once per tool execution (in async context, before spawn_blocking)
/// so that the synchronous WASM host function can inject credentials
/// without needing async access to the secrets store.
///
/// If an `OAuthRefreshConfig` is provided and the access token is expired
/// (or within 5 minutes of expiry), attempts a transparent refresh first.
///
/// Silently skips credentials that can't be resolved (e.g., missing secrets).
/// The tool will get a 401/403 from the API, which is the expected UX when
/// auth hasn't been configured yet.
async fn resolve_host_credentials(
    capabilities: &Capabilities,
    store: Option<&(dyn SecretsStore + Send + Sync)>,
    user_id: &str,
    oauth_refresh: Option<&OAuthRefreshConfig>,
) -> Vec<ResolvedHostCredential> {
    let store = match store {
        Some(s) => s,
        None => {
            // If tool requires credentials but has no secrets store, this is a configuration error
            if let Some(http_cap) = &capabilities.http
                && !http_cap.credentials.is_empty()
            {
                tracing::warn!(
                    user_id = %user_id,
                    "WASM tool requires credentials but secrets_store is not configured - authentication will fail"
                );
            }
            return Vec::new();
        }
    };

    // Check if the access token needs refreshing before resolving credentials.
    // This runs once per tool execution, keeping the hot path (credential injection
    // inside WASM) synchronous and allocation-free.
    if let Some(config) = oauth_refresh {
        let needs_refresh = match store.get(user_id, &config.secret_name).await {
            Ok(secret) => match secret.expires_at {
                Some(expires_at) => {
                    let buffer = chrono::Duration::minutes(5);
                    expires_at - buffer < chrono::Utc::now()
                }
                // No expires_at means legacy token, don't try to refresh
                None => false,
            },
            // Expired error from store means we definitely need to refresh
            Err(crate::secrets::SecretError::Expired) => true,
            // Not found or other errors: skip refresh, let the normal flow handle it
            Err(_) => false,
        };

        if needs_refresh {
            tracing::debug!(
                secret_name = %config.secret_name,
                "Access token expired or near expiry, attempting refresh"
            );
            refresh_oauth_token(store, user_id, config).await;
        }
    }

    let http_cap = match &capabilities.http {
        Some(cap) => cap,
        None => return Vec::new(),
    };

    if http_cap.credentials.is_empty() {
        return Vec::new();
    }

    let mut resolved = Vec::new();

    for mapping in http_cap.credentials.values() {
        // Skip UrlPath credentials, they're handled by placeholder substitution
        if matches!(
            mapping.location,
            crate::secrets::CredentialLocation::UrlPath { .. }
        ) {
            continue;
        }

        // Try to get credential under the provided user_id first.
        // If not found and user_id != "default", fallback to "default" (global credentials).
        // This handles OAuth tokens stored globally under "default" but accessed from routine contexts.
        let secret = match store.get_decrypted(user_id, &mapping.secret_name).await {
            Ok(s) => Some(s),
            Err(e) => {
                tracing::trace!(
                    user_id = %user_id,
                    secret_name = %mapping.secret_name,
                    error = %e,
                    "No matching host credential resolved for WASM tool in the requested scope"
                );

                // If lookup fails and we're not already looking up "default", try "default" as fallback
                if user_id != "default" {
                    tracing::debug!(
                        secret_name = %mapping.secret_name,
                        user_id = %user_id,
                        error = %e,
                        "Credential not found for user, trying default global credentials"
                    );
                    store
                        .get_decrypted("default", &mapping.secret_name)
                        .await
                        .ok()
                } else {
                    None
                }
            }
        };

        let secret = match secret {
            Some(s) => s,
            None => {
                tracing::warn!(
                    secret_name = %mapping.secret_name,
                    user_id = %user_id,
                    "Could not resolve credential for WASM tool (not found in user context or default)"
                );
                continue;
            }
        };

        let mut injected = InjectedCredentials::empty();
        inject_credential(&mut injected, &mapping.location, &secret);

        if injected.is_empty() {
            continue;
        }

        resolved.push(ResolvedHostCredential {
            host_patterns: mapping.host_patterns.clone(),
            headers: injected.headers,
            query_params: injected.query_params,
            secret_value: secret.expose().to_string(),
        });
    }

    if !resolved.is_empty() {
        tracing::debug!(
            count = resolved.len(),
            "Pre-resolved host credentials for WASM tool execution"
        );
    }

    resolved
}

/// Extract the hostname from a URL string.
///
/// Handles `https://host:port/path`, stripping scheme, port, and path.
/// Also handles IPv6 bracket notation like `http://[::1]:8080/path`.
/// Returns None for malformed URLs.
fn extract_host_from_url(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    parsed.host_str().map(|h| {
        h.strip_prefix('[')
            .and_then(|v| v.strip_suffix(']'))
            .unwrap_or(h)
            .to_lowercase()
    })
}

/// Resolve the URL's hostname and reject connections to private/internal IP addresses.
/// This prevents DNS rebinding attacks where an attacker's domain resolves to an
/// internal IP after passing the allowlist check.
fn reject_private_ip(url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|e| format!("Failed to parse URL: {e}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(format!("Unsupported URL scheme: {}", parsed.scheme()));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("URL contains userinfo (@) which is not allowed".to_string());
    }

    let host = parsed
        .host_str()
        .map(|h| {
            h.strip_prefix('[')
                .and_then(|v| v.strip_suffix(']'))
                .unwrap_or(h)
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

fn schema_contains_container_properties(schema: &serde_json::Value) -> bool {
    schema
        .get("properties")
        .and_then(|p| p.as_object())
        .map(|props| {
            props.values().any(|prop| {
                schema_declares_type(prop, "array") || schema_declares_type(prop, "object")
            })
        })
        .unwrap_or(false)
}

fn schema_declares_type(schema: &serde_json::Value, expected: &str) -> bool {
    match schema.get("type") {
        Some(serde_json::Value::String(t)) => t == expected,
        Some(serde_json::Value::Array(types)) => types.iter().any(|t| t.as_str() == Some(expected)),
        _ => match expected {
            "object" => {
                schema
                    .get("properties")
                    .and_then(|p| p.as_object())
                    .is_some()
                    || schema
                        .get("additionalProperties")
                        .is_some_and(serde_json::Value::is_object)
            }
            "array" => schema.get("items").is_some(),
            _ => false,
        },
    }
}

fn schema_is_typed_property(schema: &serde_json::Value) -> bool {
    matches!(
        schema.get("type"),
        Some(serde_json::Value::String(_)) | Some(serde_json::Value::Array(_))
    ) || schema.get("$ref").is_some()
        || schema.get("anyOf").is_some()
        || schema.get("oneOf").is_some()
        || schema.get("allOf").is_some()
        || schema.get("items").is_some()
        || schema
            .get("properties")
            .and_then(|p| p.as_object())
            .is_some()
        || schema
            .get("additionalProperties")
            .is_some_and(serde_json::Value::is_object)
}

fn build_tool_usage_hint(tool_name: &str, schema: &serde_json::Value) -> String {
    let mut hint = format!(
        "Tip: call tool_info(name: \"{}\", include_schema: true) for the full parameter schema.",
        tool_name
    );

    if schema_contains_container_properties(schema) {
        hint.push_str(
            " For array/object fields, pass native JSON arrays/objects, not quoted JSON strings.",
        );
    }

    hint
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::testing::credentials::{
        TEST_BEARER_TOKEN_123, TEST_GOOGLE_OAUTH_FRESH, TEST_GOOGLE_OAUTH_LEGACY,
        TEST_GOOGLE_OAUTH_TOKEN, TEST_OAUTH_CLIENT_ID, TEST_OAUTH_CLIENT_SECRET,
        test_secrets_store,
    };
    use crate::tools::tool::Tool;
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

    #[tokio::test]
    async fn test_advertised_schema_stays_permissive_until_sidecar_override() {
        let discovery_schema = serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "limit": { "type": "integer" }
            },
            "required": ["query"]
        });

        let runtime = Arc::new(WasmToolRuntime::new(WasmRuntimeConfig::for_testing()).unwrap());
        let prepared = runtime
            .prepare("search", b"\0asm\x0d\0\x01\0", None)
            .await
            .unwrap();
        let mut wrapper =
            super::WasmToolWrapper::new(Arc::clone(&runtime), prepared, Capabilities::default());
        wrapper.schemas = super::WasmToolSchemas::new(discovery_schema.clone());
        wrapper.description = "Search documents".to_string();

        // Advertised schema stays permissive; discovery holds the typed schema
        assert_eq!(
            wrapper.parameters_schema(),
            serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": true
            })
        );
        assert_eq!(wrapper.discovery_schema(), discovery_schema);

        // Raw description is clean — no tool_info hint baked in
        assert!(!wrapper.description().contains("tool_info"));

        // But schema() composes the hint at display time when advertised is permissive
        let schema = wrapper.schema();
        assert!(
            schema.description.contains("tool_info"),
            "schema().description should contain tool_info hint: {}",
            schema.description
        );
        assert!(
            schema.description.contains("include_schema: true"),
            "hint should mention include_schema: true: {}",
            schema.description
        );

        // After sidecar override, both schemas match and hint disappears
        let wrapper = wrapper.with_schema(serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" }
            },
            "required": ["query"]
        }));

        assert_eq!(
            wrapper.parameters_schema(),
            serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" }
                },
                "required": ["query"]
            })
        );
        assert_eq!(wrapper.discovery_schema(), wrapper.parameters_schema());

        // With typed schema, schema() should NOT include tool_info hint
        let schema = wrapper.schema();
        assert!(
            !schema.description.contains("tool_info"),
            "schema().description should not contain tool_info hint when typed: {}",
            schema.description
        );
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
    fn test_extract_host_from_url() {
        use crate::tools::wasm::wrapper::extract_host_from_url;

        assert_eq!(
            extract_host_from_url("https://www.googleapis.com/calendar/v3/events"),
            Some("www.googleapis.com".to_string())
        );
        assert_eq!(
            extract_host_from_url("https://api.example.com:443/v1/foo"),
            Some("api.example.com".to_string())
        );
        assert_eq!(
            extract_host_from_url("http://localhost:8080/test?q=1"),
            Some("localhost".to_string())
        );
        assert_eq!(
            extract_host_from_url("https://user:pass@host.com/path"),
            Some("host.com".to_string())
        );
        assert_eq!(extract_host_from_url("ftp://bad.com"), None);
        assert_eq!(extract_host_from_url("not a url"), None);
        // IPv6
        assert_eq!(
            extract_host_from_url("http://[::1]:8080/test"),
            Some("::1".to_string())
        );
        assert_eq!(
            extract_host_from_url("https://[2001:db8::1]/path"),
            Some("2001:db8::1".to_string())
        );
    }

    #[test]
    fn test_inject_host_credentials_bearer() {
        use crate::tools::wasm::wrapper::{ResolvedHostCredential, StoreData};
        use std::collections::HashMap;

        let host_credentials = vec![ResolvedHostCredential {
            host_patterns: vec!["www.googleapis.com".to_string()],
            headers: {
                let mut h = HashMap::new();
                h.insert(
                    "Authorization".to_string(),
                    format!("Bearer {TEST_BEARER_TOKEN_123}"),
                );
                h
            },
            query_params: HashMap::new(),
            secret_value: TEST_BEARER_TOKEN_123.to_string(),
        }];

        let store_data = StoreData::new(
            1024 * 1024,
            Capabilities::default(),
            HashMap::new(),
            host_credentials,
        );

        // Should inject for matching host
        let mut headers = HashMap::new();
        let mut url = "https://www.googleapis.com/calendar/v3/events".to_string();
        store_data.inject_host_credentials("www.googleapis.com", &mut headers, &mut url);
        assert_eq!(
            headers.get("Authorization"),
            Some(&format!("Bearer {TEST_BEARER_TOKEN_123}"))
        );

        // Should not inject for non-matching host
        let mut headers2 = HashMap::new();
        let mut url2 = "https://other.com/api".to_string();
        store_data.inject_host_credentials("other.com", &mut headers2, &mut url2);
        assert!(!headers2.contains_key("Authorization"));
    }

    #[test]
    fn test_inject_host_credentials_query_params() {
        use crate::tools::wasm::wrapper::{ResolvedHostCredential, StoreData};
        use std::collections::HashMap;

        let host_credentials = vec![ResolvedHostCredential {
            host_patterns: vec!["api.example.com".to_string()],
            headers: HashMap::new(),
            query_params: {
                let mut q = HashMap::new();
                q.insert("api_key".to_string(), "secret123".to_string());
                q
            },
            secret_value: "secret123".to_string(),
        }];

        let store_data = StoreData::new(
            1024 * 1024,
            Capabilities::default(),
            HashMap::new(),
            host_credentials,
        );

        let mut headers = HashMap::new();
        let mut url = "https://api.example.com/v1/data".to_string();
        store_data.inject_host_credentials("api.example.com", &mut headers, &mut url);
        assert!(url.contains("api_key=secret123"));
        assert!(url.contains('?'));
    }

    #[test]
    fn test_redact_credentials_includes_host_credentials() {
        use crate::tools::wasm::wrapper::{ResolvedHostCredential, StoreData};
        use std::collections::HashMap;

        let host_credentials = vec![ResolvedHostCredential {
            host_patterns: vec!["api.example.com".to_string()],
            headers: HashMap::new(),
            query_params: HashMap::new(),
            secret_value: "super-secret-token".to_string(),
        }];

        let store_data = StoreData::new(
            1024 * 1024,
            Capabilities::default(),
            HashMap::new(),
            host_credentials,
        );

        let text = "Error: request to https://api.example.com?key=super-secret-token failed";
        let redacted = store_data.redact_credentials(text);
        assert!(!redacted.contains("super-secret-token"));
        assert!(redacted.contains("[REDACTED:host_credential]"));
    }

    #[tokio::test]
    async fn test_resolve_host_credentials_no_store() {
        use crate::tools::wasm::wrapper::resolve_host_credentials;

        let caps = Capabilities::default();
        let result = resolve_host_credentials(&caps, None, "user1", None).await;
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_resolve_host_credentials_no_http_cap() {
        use crate::tools::wasm::wrapper::resolve_host_credentials;

        let store = test_secrets_store();

        let caps = Capabilities::default();
        let result = resolve_host_credentials(&caps, Some(&store), "user1", None).await;
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_resolve_host_credentials_bearer() {
        use std::collections::HashMap;

        use crate::secrets::{
            CreateSecretParams, CredentialLocation, CredentialMapping, SecretsStore,
        };
        use crate::tools::wasm::capabilities::HttpCapability;
        use crate::tools::wasm::wrapper::resolve_host_credentials;

        let store = test_secrets_store();

        store
            .create(
                "user1",
                CreateSecretParams::new("google_oauth_token", TEST_GOOGLE_OAUTH_TOKEN),
            )
            .await
            .unwrap();

        let mut credentials = HashMap::new();
        credentials.insert(
            "google_oauth_token".to_string(),
            CredentialMapping {
                secret_name: "google_oauth_token".to_string(),
                location: CredentialLocation::AuthorizationBearer,
                host_patterns: vec!["www.googleapis.com".to_string()],
            },
        );

        let caps = Capabilities {
            http: Some(HttpCapability {
                credentials,
                ..Default::default()
            }),
            ..Default::default()
        };

        let result = resolve_host_credentials(&caps, Some(&store), "user1", None).await;
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].host_patterns, vec!["www.googleapis.com"]);
        assert_eq!(
            result[0].headers.get("Authorization"),
            Some(&format!("Bearer {TEST_GOOGLE_OAUTH_TOKEN}"))
        );
    }

    #[tokio::test]
    async fn test_resolve_host_credentials_owner_scope_bearer() {
        use std::collections::HashMap;

        use crate::context::JobContext;
        use crate::secrets::{
            CreateSecretParams, CredentialLocation, CredentialMapping, SecretsStore,
        };
        use crate::tools::wasm::capabilities::HttpCapability;
        use crate::tools::wasm::wrapper::resolve_host_credentials;

        let store = test_secrets_store();
        let ctx = JobContext::with_user("owner-scope", "owner-scope test", "owner-scope test");

        store
            .create(
                &ctx.user_id,
                CreateSecretParams::new("google_oauth_token", TEST_GOOGLE_OAUTH_TOKEN),
            )
            .await
            .unwrap();

        let mut credentials = HashMap::new();
        credentials.insert(
            "google_oauth_token".to_string(),
            CredentialMapping {
                secret_name: "google_oauth_token".to_string(),
                location: CredentialLocation::AuthorizationBearer,
                host_patterns: vec!["www.googleapis.com".to_string()],
            },
        );

        let caps = Capabilities {
            http: Some(HttpCapability {
                credentials,
                ..Default::default()
            }),
            ..Default::default()
        };

        let result = resolve_host_credentials(&caps, Some(&store), &ctx.user_id, None).await;
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0].headers.get("Authorization"),
            Some(&format!("Bearer {TEST_GOOGLE_OAUTH_TOKEN}"))
        );
    }

    #[tokio::test]
    async fn test_resolve_host_credentials_missing_secret() {
        use std::collections::HashMap;

        use crate::secrets::{CredentialLocation, CredentialMapping};
        use crate::tools::wasm::capabilities::HttpCapability;
        use crate::tools::wasm::wrapper::resolve_host_credentials;

        let store = test_secrets_store();

        // No secret stored, should silently skip
        let mut credentials = HashMap::new();
        credentials.insert(
            "missing_token".to_string(),
            CredentialMapping {
                secret_name: "missing_token".to_string(),
                location: CredentialLocation::AuthorizationBearer,
                host_patterns: vec!["api.example.com".to_string()],
            },
        );

        let caps = Capabilities {
            http: Some(HttpCapability {
                credentials,
                ..Default::default()
            }),
            ..Default::default()
        };

        let result = resolve_host_credentials(&caps, Some(&store), "user1", None).await;
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_resolve_host_credentials_skips_refresh_when_not_expired() {
        use std::collections::HashMap;

        use crate::secrets::{
            CreateSecretParams, CredentialLocation, CredentialMapping, SecretsStore,
        };
        use crate::tools::wasm::capabilities::HttpCapability;
        use crate::tools::wasm::wrapper::{OAuthRefreshConfig, resolve_host_credentials};

        let store = test_secrets_store();

        // Store a token that expires 2 hours from now (well within buffer)
        let expires_at = chrono::Utc::now() + chrono::Duration::hours(2);
        store
            .create(
                "user1",
                CreateSecretParams::new("google_oauth_token", TEST_GOOGLE_OAUTH_FRESH)
                    .with_expiry(expires_at),
            )
            .await
            .unwrap();

        let mut credentials = HashMap::new();
        credentials.insert(
            "google_oauth_token".to_string(),
            CredentialMapping {
                secret_name: "google_oauth_token".to_string(),
                location: CredentialLocation::AuthorizationBearer,
                host_patterns: vec!["www.googleapis.com".to_string()],
            },
        );

        let caps = Capabilities {
            http: Some(HttpCapability {
                credentials,
                ..Default::default()
            }),
            ..Default::default()
        };

        let oauth_config = OAuthRefreshConfig {
            token_url: "https://oauth2.googleapis.com/token".to_string(),
            client_id: TEST_OAUTH_CLIENT_ID.to_string(),
            client_secret: Some(TEST_OAUTH_CLIENT_SECRET.to_string()),
            secret_name: "google_oauth_token".to_string(),
            provider: Some("google".to_string()),
        };

        // Should resolve the existing fresh token without attempting refresh
        let result =
            resolve_host_credentials(&caps, Some(&store), "user1", Some(&oauth_config)).await;
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0].headers.get("Authorization"),
            Some(&format!("Bearer {TEST_GOOGLE_OAUTH_FRESH}"))
        );
    }

    #[tokio::test]
    async fn test_resolve_host_credentials_skips_refresh_no_config() {
        use std::collections::HashMap;

        use crate::secrets::{
            CreateSecretParams, CredentialLocation, CredentialMapping, SecretsStore,
        };
        use crate::tools::wasm::capabilities::HttpCapability;
        use crate::tools::wasm::wrapper::resolve_host_credentials;

        let store = test_secrets_store();

        // Store an expired token
        let expires_at = chrono::Utc::now() - chrono::Duration::hours(1);
        store
            .create(
                "user1",
                CreateSecretParams::new("my_token", "expired-value").with_expiry(expires_at),
            )
            .await
            .unwrap();

        let mut credentials = HashMap::new();
        credentials.insert(
            "my_token".to_string(),
            CredentialMapping {
                secret_name: "my_token".to_string(),
                location: CredentialLocation::AuthorizationBearer,
                host_patterns: vec!["api.example.com".to_string()],
            },
        );

        let caps = Capabilities {
            http: Some(HttpCapability {
                credentials,
                ..Default::default()
            }),
            ..Default::default()
        };

        // No OAuth config, expired token can't be resolved (get_decrypted returns Expired)
        let result = resolve_host_credentials(&caps, Some(&store), "user1", None).await;
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_resolve_host_credentials_skips_refresh_no_expires_at() {
        use std::collections::HashMap;

        use crate::secrets::{
            CreateSecretParams, CredentialLocation, CredentialMapping, SecretsStore,
        };
        use crate::tools::wasm::capabilities::HttpCapability;
        use crate::tools::wasm::wrapper::{OAuthRefreshConfig, resolve_host_credentials};

        let store = test_secrets_store();

        // Legacy token: no expires_at set
        store
            .create(
                "user1",
                CreateSecretParams::new("google_oauth_token", TEST_GOOGLE_OAUTH_LEGACY),
            )
            .await
            .unwrap();

        let mut credentials = HashMap::new();
        credentials.insert(
            "google_oauth_token".to_string(),
            CredentialMapping {
                secret_name: "google_oauth_token".to_string(),
                location: CredentialLocation::AuthorizationBearer,
                host_patterns: vec!["www.googleapis.com".to_string()],
            },
        );

        let caps = Capabilities {
            http: Some(HttpCapability {
                credentials,
                ..Default::default()
            }),
            ..Default::default()
        };

        let oauth_config = OAuthRefreshConfig {
            token_url: "https://oauth2.googleapis.com/token".to_string(),
            client_id: TEST_OAUTH_CLIENT_ID.to_string(),
            client_secret: Some(TEST_OAUTH_CLIENT_SECRET.to_string()),
            secret_name: "google_oauth_token".to_string(),
            provider: Some("google".to_string()),
        };

        // Should use the legacy token directly without attempting refresh
        let result =
            resolve_host_credentials(&caps, Some(&store), "user1", Some(&oauth_config)).await;
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0].headers.get("Authorization"),
            Some(&format!("Bearer {TEST_GOOGLE_OAUTH_LEGACY}"))
        );
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

    #[tokio::test]
    async fn test_untyped_override_preserves_extracted_discovery_schema() {
        let typed_schema = serde_json::json!({
            "type": "object",
            "properties": {
                "values": {
                    "type": ["array", "null"],
                    "items": { "type": "array" }
                }
            }
        });

        let runtime = Arc::new(WasmToolRuntime::new(WasmRuntimeConfig::for_testing()).unwrap()); // safety: test-only setup
        let mut prepared = runtime
            .prepare("sheets", b"\0asm\x0d\0\x01\0", None)
            .await
            .unwrap(); // safety: test-only setup
        Arc::get_mut(&mut prepared).unwrap().schema = typed_schema.clone(); // safety: test-only setup

        let wrapper =
            super::WasmToolWrapper::new(Arc::clone(&runtime), prepared, Capabilities::default())
                .with_schema(serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "additionalProperties": true
                }));

        #[rustfmt::skip]
        assert_eq!( // safety: test-only assertion
            wrapper.parameters_schema(),
            serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": true
            })
        );
        assert_eq!(wrapper.discovery_schema(), typed_schema); // safety: test-only assertion
    }

    #[test]
    fn test_build_tool_usage_hint_detects_nullable_container_properties() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "requests": {
                    "type": ["array", "null"],
                    "items": { "type": "object" }
                }
            }
        });

        let hint = super::build_tool_usage_hint("google_docs", &schema);

        assert!(hint.contains("native JSON arrays/objects")); // safety: test-only assertion
    }

    /// Regression test: leak scan must run on raw headers (before credential
    /// injection), not after. If it ran post-injection, the host-injected
    /// Slack bot token (`xoxb-...`) would trigger a Block and reject the
    /// tool's own legitimate outbound request.
    #[test]
    fn test_leak_scan_runs_before_credential_injection() {
        use crate::safety::LeakDetector;

        // Simulate pre-injection headers: WASM only sees the placeholder, not the real token.
        let raw_headers: Vec<(String, String)> = vec![
            (
                "Authorization".to_string(),
                "Bearer {SLACK_BOT_TOKEN}".to_string(),
            ),
            ("Content-Type".to_string(), "application/json".to_string()),
        ];

        let detector = LeakDetector::new();

        // Pre-injection scan should pass — placeholders are not secrets.
        let pre_result = detector.scan_http_request(
            "https://slack.com/api/chat.postMessage",
            &raw_headers,
            None,
        );
        assert!(
            pre_result.is_ok(),
            "Leak scan on pre-injection headers should pass, but got: {:?}",
            pre_result
        );

        // Post-injection headers would contain a real Slack token.
        let post_injection_headers: Vec<(String, String)> = vec![
            (
                "Authorization".to_string(),
                "Bearer xoxb-1234567890-abcdefghij".to_string(),
            ),
            ("Content-Type".to_string(), "application/json".to_string()),
        ];

        // Post-injection scan WOULD block — this is the false positive
        // that the pre-injection ordering prevents.
        let post_result = detector.scan_http_request(
            "https://slack.com/api/chat.postMessage",
            &post_injection_headers,
            None,
        );
        assert!(
            post_result.is_err(),
            "Leak scan on post-injection headers should block the Slack token"
        );
    }

    #[tokio::test]
    async fn test_resolve_host_credentials_fallback_to_default_user() {
        use crate::secrets::{CredentialLocation, CredentialMapping, SecretsStore};
        use crate::tools::wasm::capabilities::HttpCapability;
        use crate::tools::wasm::wrapper::resolve_host_credentials;

        let store = test_secrets_store();

        // Store a token under the "default" global user
        store
            .create(
                "default",
                crate::secrets::CreateSecretParams::new("google_oauth_token", "global_token_value"),
            )
            .await
            .expect("Failed to store global token"); // safety: test code only

        // Create capabilities requiring this credential
        let mut creds = std::collections::HashMap::new();
        creds.insert(
            "google_oauth_token".to_string(),
            CredentialMapping {
                secret_name: "google_oauth_token".to_string(),
                location: CredentialLocation::AuthorizationBearer,
                host_patterns: vec!["sheets.googleapis.com".to_string()],
            },
        );
        let caps = Capabilities {
            http: Some(HttpCapability {
                allowlist: vec![],
                credentials: creds,
                rate_limit: crate::tools::wasm::capabilities::RateLimitConfig::default(),
                max_request_bytes: 1024 * 1024,
                max_response_bytes: 10 * 1024 * 1024,
                timeout: std::time::Duration::from_secs(30),
            }),
            ..Default::default()
        };

        // Resolve credentials for a different user (routine context)
        // Should fallback to "default" and find the token
        let result = resolve_host_credentials(&caps, Some(&store), "routine_user_123", None).await;

        assert!(!result.is_empty(), "fallback to default"); // safety: test code only
        assert_eq!(result[0].secret_value, "global_token_value"); // safety: test code only
    }

    fn test_capabilities_with_google_oauth() -> Capabilities {
        use crate::secrets::{CredentialLocation, CredentialMapping};
        use crate::tools::wasm::capabilities::HttpCapability;

        let mut creds = std::collections::HashMap::new();
        creds.insert(
            "google_oauth_token".to_string(),
            CredentialMapping {
                secret_name: "google_oauth_token".to_string(),
                location: CredentialLocation::AuthorizationBearer,
                host_patterns: vec!["sheets.googleapis.com".to_string()],
            },
        );
        Capabilities {
            http: Some(HttpCapability {
                allowlist: vec![],
                credentials: creds,
                rate_limit: crate::tools::wasm::capabilities::RateLimitConfig::default(),
                max_request_bytes: 1024 * 1024,
                max_response_bytes: 10 * 1024 * 1024,
                timeout: std::time::Duration::from_secs(30),
            }),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn test_resolve_host_credentials_prefers_user_specific_over_default() {
        use crate::secrets::SecretsStore;
        use crate::tools::wasm::wrapper::resolve_host_credentials;

        let store = test_secrets_store();

        // Store token under "default" (global)
        store
            .create(
                "default",
                crate::secrets::CreateSecretParams::new("google_oauth_token", "global_token"),
            )
            .await
            .expect("Failed to store global token"); // safety: test code only

        // Store token under user_123 (user-specific)
        store
            .create(
                "user_123",
                crate::secrets::CreateSecretParams::new(
                    "google_oauth_token",
                    "user_specific_token",
                ),
            )
            .await
            .expect("Failed to store user token"); // safety: test code only

        // Create capabilities
        let caps = test_capabilities_with_google_oauth();

        // Resolve credentials for user_123
        // Should prefer user_123's token over default
        let result = resolve_host_credentials(&caps, Some(&store), "user_123", None).await;

        assert!(!result.is_empty(), "has user credentials"); // safety: test code only
        assert_eq!(result[0].secret_value, "user_specific_token", "user token"); // safety: test code only
    }

    #[tokio::test]
    async fn test_resolve_host_credentials_no_fallback_when_already_default() {
        use crate::secrets::SecretsStore;
        use crate::tools::wasm::wrapper::resolve_host_credentials;

        let store = test_secrets_store();

        // Only store token under "default" (not a duplicate)
        store
            .create(
                "default",
                crate::secrets::CreateSecretParams::new("google_oauth_token", "default_token"),
            )
            .await
            .expect("Failed to store default token"); // safety: test code only

        // Create capabilities
        let caps = test_capabilities_with_google_oauth();

        // Resolve credentials for "default" user
        // Should NOT attempt fallback (already looking up default)
        let result = resolve_host_credentials(&caps, Some(&store), "default", None).await;

        assert!(!result.is_empty(), "Should find default token"); // safety: test code only
        assert_eq!(result[0].secret_value, "default_token"); // safety: test code only
    }

    #[tokio::test]
    async fn test_resolve_host_credentials_missing_secret_warns() {
        use crate::tools::wasm::wrapper::resolve_host_credentials;

        let store = test_secrets_store();

        // Don't store any token

        // Create capabilities expecting a credential
        let caps = test_capabilities_with_google_oauth();

        // Resolve credentials when neither user nor default has the token
        let result = resolve_host_credentials(&caps, Some(&store), "user_456", None).await;

        // Should return empty since credential can't be found anywhere
        assert!(result.is_empty(), "no credentials found"); // safety: test code only
    }
}
