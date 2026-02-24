//! WASM channel wrapper implementing the Channel trait.
//!
//! Wraps a prepared WASM channel module and provides the Channel interface.
//! Each callback (on_start, on_http_request, on_poll, on_respond) creates
//! a fresh WASM instance for isolation.
//!
//! # Architecture
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────────┐
//! │                    WasmChannel                               │
//! │                                                              │
//! │   ┌─────────────┐   call_on_*   ┌──────────────────────┐    │
//! │   │   Channel   │ ────────────> │   execute_callback   │    │
//! │   │    Trait    │               │   (fresh instance)   │    │
//! │   └─────────────┘               └──────────┬───────────┘    │
//! │                                            │                 │
//! │                                            ▼                 │
//! │   ┌──────────────────────────────────────────────────────┐  │
//! │   │               ChannelStoreData                       │  │
//! │   │  ┌─────────────┐  ┌──────────────────────────────┐   │  │
//! │   │  │   limiter   │  │      ChannelHostState        │   │  │
//! │   │  └─────────────┘  │  - emitted_messages          │   │  │
//! │   │                   │  - pending_writes            │   │  │
//! │   │                   │  - base HostState (logging)  │   │  │
//! │   │                   └──────────────────────────────┘   │  │
//! │   └──────────────────────────────────────────────────────┘  │
//! └──────────────────────────────────────────────────────────────┘
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::{RwLock, mpsc, oneshot};
use tokio_stream::wrappers::ReceiverStream;
use uuid::Uuid;
use wasmtime::Store;
use wasmtime::component::Linker;
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiView};

use crate::channels::wasm::capabilities::ChannelCapabilities;
use crate::channels::wasm::error::WasmChannelError;
use crate::channels::wasm::host::{
    ChannelEmitRateLimiter, ChannelHostState, ChannelWorkspaceStore, EmittedMessage,
};
use crate::channels::wasm::router::RegisteredEndpoint;
use crate::channels::wasm::runtime::{PreparedChannelModule, WasmChannelRuntime};
use crate::channels::wasm::schema::ChannelConfig;
use crate::channels::{Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate};
use crate::error::ChannelError;
use crate::pairing::PairingStore;
use crate::safety::LeakDetector;
use crate::tools::wasm::LogLevel;
use crate::tools::wasm::WasmResourceLimiter;

// Generate component model bindings from the WIT file
wasmtime::component::bindgen!({
    path: "wit/channel.wit",
    world: "sandboxed-channel",
    async: false,
    with: {
        // Use our own store data type
    },
});

/// Store data for WASM channel execution.
///
/// Contains the resource limiter, channel-specific host state, and WASI context.
struct ChannelStoreData {
    limiter: WasmResourceLimiter,
    host_state: ChannelHostState,
    wasi: WasiCtx,
    table: ResourceTable,
    /// Injected credentials for URL substitution (e.g., bot tokens).
    /// Keys are placeholder names like "TELEGRAM_BOT_TOKEN".
    credentials: HashMap<String, String>,
    /// Pairing store for DM pairing (guest access control).
    pairing_store: Arc<PairingStore>,
    /// Dedicated tokio runtime for HTTP requests, lazily initialized.
    /// Reused across multiple `http_request` calls within one execution.
    http_runtime: Option<tokio::runtime::Runtime>,
}

impl ChannelStoreData {
    fn new(
        memory_limit: u64,
        channel_name: &str,
        capabilities: ChannelCapabilities,
        credentials: HashMap<String, String>,
        pairing_store: Arc<PairingStore>,
    ) -> Self {
        // Create a minimal WASI context (no filesystem, no env vars for security)
        let wasi = WasiCtxBuilder::new().build();

        Self {
            limiter: WasmResourceLimiter::new(memory_limit),
            host_state: ChannelHostState::new(channel_name, capabilities),
            wasi,
            table: ResourceTable::new(),
            credentials,
            pairing_store,
            http_runtime: None,
        }
    }

    /// Inject credentials into a string by replacing placeholders.
    ///
    /// Replaces patterns like `{TELEGRAM_BOT_TOKEN}` or `{WHATSAPP_ACCESS_TOKEN}`
    /// with actual values from the injected credentials map. This allows WASM
    /// channels to reference credentials without ever seeing the actual values.
    ///
    /// Works on URLs, headers, or any string with credential placeholders.
    fn inject_credentials(&self, input: &str, context: &str) -> String {
        let mut result = input.to_string();

        tracing::debug!(
            input_preview = %input.chars().take(100).collect::<String>(),
            context = %context,
            credential_count = self.credentials.len(),
            credential_names = ?self.credentials.keys().collect::<Vec<_>>(),
            "Injecting credentials"
        );

        // Replace all known placeholders from the credentials map
        for (name, value) in &self.credentials {
            let placeholder = format!("{{{}}}", name);
            if result.contains(&placeholder) {
                tracing::debug!(
                    placeholder = %placeholder,
                    context = %context,
                    "Found and replacing credential placeholder"
                );
                result = result.replace(&placeholder, value);
            }
        }

        // Check if any placeholders remain (indicates missing credential)
        if result.contains('{') && result.contains('}') {
            // Only warn if it looks like an unresolved placeholder (not JSON braces)
            let brace_pattern = regex::Regex::new(r"\{[A-Z_]+\}").ok();
            if let Some(re) = brace_pattern
                && re.is_match(&result)
            {
                tracing::warn!(
                    context = %context,
                    "String may contain unresolved credential placeholders"
                );
            }
        }

        result
    }

    /// Replace injected credential values with `[REDACTED]` in text.
    ///
    /// Prevents credentials from leaking through error messages, logs, or
    /// return values to WASM. reqwest::Error includes the full URL in its
    /// Display output, so any error from an injected-URL request will
    /// contain the raw credential unless we scrub it.
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

// Implement WasiView to provide WASI context and resource table
impl WasiView for ChannelStoreData {
    fn ctx(&mut self) -> &mut WasiCtx {
        &mut self.wasi
    }

    fn table(&mut self) -> &mut ResourceTable {
        &mut self.table
    }
}

// Implement the generated Host trait for channel-host interface
impl near::agent::channel_host::Host for ChannelStoreData {
    fn log(&mut self, level: near::agent::channel_host::LogLevel, message: String) {
        let log_level = match level {
            near::agent::channel_host::LogLevel::Trace => LogLevel::Trace,
            near::agent::channel_host::LogLevel::Debug => LogLevel::Debug,
            near::agent::channel_host::LogLevel::Info => LogLevel::Info,
            near::agent::channel_host::LogLevel::Warn => LogLevel::Warn,
            near::agent::channel_host::LogLevel::Error => LogLevel::Error,
        };
        let _ = self.host_state.log(log_level, message);
    }

    fn now_millis(&mut self) -> u64 {
        self.host_state.now_millis()
    }

    fn workspace_read(&mut self, path: String) -> Option<String> {
        self.host_state.workspace_read(&path).ok().flatten()
    }

    fn workspace_write(&mut self, path: String, content: String) -> Result<(), String> {
        self.host_state
            .workspace_write(&path, content)
            .map_err(|e| e.to_string())
    }

    fn http_request(
        &mut self,
        method: String,
        url: String,
        headers_json: String,
        body: Option<Vec<u8>>,
        timeout_ms: Option<u32>,
    ) -> Result<near::agent::channel_host::HttpResponse, String> {
        tracing::info!(
            method = %method,
            original_url = %url,
            body_len = body.as_ref().map(|b| b.len()).unwrap_or(0),
            "WASM http_request called"
        );

        // Inject credentials into URL (e.g., replace {TELEGRAM_BOT_TOKEN} with actual token)
        let injected_url = self.inject_credentials(&url, "url");

        // Log whether injection happened (without revealing the token)
        let url_changed = injected_url != url;
        tracing::info!(url_changed = url_changed, "URL after credential injection");

        // Check if HTTP is allowed for this URL
        self.host_state
            .check_http_allowed(&injected_url, &method)
            .map_err(|e| {
                tracing::error!(error = %e, "HTTP not allowed");
                format!("HTTP not allowed: {}", e)
            })?;

        // Record the request for rate limiting
        self.host_state.record_http_request().map_err(|e| {
            tracing::error!(error = %e, "Rate limit exceeded");
            format!("Rate limit exceeded: {}", e)
        })?;

        // Parse headers and inject credentials into header values
        // This allows patterns like "Authorization": "Bearer {WHATSAPP_ACCESS_TOKEN}"
        let raw_headers: std::collections::HashMap<String, String> =
            serde_json::from_str(&headers_json).unwrap_or_default();

        let headers: std::collections::HashMap<String, String> = raw_headers
            .into_iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    self.inject_credentials(&v, &format!("header:{}", k)),
                )
            })
            .collect();

        let headers_changed = headers
            .values()
            .any(|v| v.contains("Bearer ") && !v.contains('{'));
        tracing::debug!(
            header_count = headers.len(),
            headers_changed = headers_changed,
            "Parsed and injected request headers"
        );

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
            .tool_capabilities
            .http
            .as_ref()
            .map(|h| h.max_response_bytes)
            .unwrap_or(10 * 1024 * 1024);

        // Make the HTTP request using a dedicated single-threaded runtime.
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
        let rt = self.http_runtime.as_ref().expect("just initialized");
        let result = rt.block_on(async {
            let client = reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(10))
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

            // Add headers
            for (key, value) in headers {
                request = request.header(&key, &value);
            }

            // Add body if present
            if let Some(body_bytes) = body {
                request = request.body(body_bytes);
            }

            // Send request with caller-specified timeout (default 30s, max 5min).
            let timeout_ms = timeout_ms.unwrap_or(30_000).min(300_000) as u64;
            let timeout = std::time::Duration::from_millis(timeout_ms);
            let response = request.timeout(timeout).send().await.map_err(|e| {
                // Walk the full error chain so we get the actual root cause
                // (DNS, TLS, connection refused, etc.) instead of just
                // "error sending request for url (...)".
                let mut chain = format!("HTTP request failed: {}", e);
                let mut source = std::error::Error::source(&e);
                while let Some(cause) = source {
                    chain.push_str(&format!(" -> {}", cause));
                    source = cause.source();
                }
                chain
            })?;

            let status = response.status().as_u16();
            let response_headers: std::collections::HashMap<String, String> = response
                .headers()
                .iter()
                .filter_map(|(k, v)| {
                    v.to_str()
                        .ok()
                        .map(|v| (k.as_str().to_string(), v.to_string()))
                })
                .collect();
            let headers_json = serde_json::to_string(&response_headers).unwrap_or_default();

            // Enforce max response body size to prevent memory exhaustion.
            let max_response = max_response_bytes;
            if let Some(cl) = response.content_length()
                && cl as usize > max_response
            {
                return Err(format!(
                    "Response body too large: {} bytes exceeds limit of {} bytes",
                    cl, max_response
                ));
            }
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

            tracing::info!(
                status = status,
                body_len = body.len(),
                "HTTP response received"
            );

            // Log response body for debugging (truncated at char boundary)
            if let Ok(body_str) = std::str::from_utf8(&body) {
                let truncated = if body_str.chars().count() > 500 {
                    format!("{}...", body_str.chars().take(500).collect::<String>())
                } else {
                    body_str.to_string()
                };
                tracing::debug!(body = %truncated, "Response body");
            }

            // Leak detection on response body (best-effort)
            if let Ok(body_str) = std::str::from_utf8(&body) {
                leak_detector
                    .scan_and_clean(body_str)
                    .map_err(|e| format!("Potential secret leak in response: {}", e))?;
            }

            Ok(near::agent::channel_host::HttpResponse {
                status,
                headers_json,
                body,
            })
        });

        // Scrub credential values from error messages before logging or returning
        // to WASM. reqwest::Error includes the full URL (with injected credentials)
        // in its Display output.
        let result = result.map_err(|e| self.redact_credentials(&e));

        match &result {
            Ok(resp) => {
                tracing::info!(status = resp.status, "http_request completed successfully");
            }
            Err(e) => {
                tracing::error!(error = %e, "http_request failed");
            }
        }

        result
    }

    fn secret_exists(&mut self, name: String) -> bool {
        self.host_state.secret_exists(&name)
    }

    fn emit_message(&mut self, msg: near::agent::channel_host::EmittedMessage) {
        tracing::info!(
            user_id = %msg.user_id,
            user_name = ?msg.user_name,
            content_len = msg.content.len(),
            "WASM emit_message called"
        );

        let mut emitted = EmittedMessage::new(msg.user_id.clone(), msg.content.clone());
        if let Some(name) = msg.user_name {
            emitted = emitted.with_user_name(name);
        }
        if let Some(tid) = msg.thread_id {
            emitted = emitted.with_thread_id(tid);
        }
        emitted = emitted.with_metadata(msg.metadata_json);

        match self.host_state.emit_message(emitted) {
            Ok(()) => {
                tracing::info!("Message emitted to host state successfully");
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to emit message to host state");
            }
        }
    }

    fn pairing_upsert_request(
        &mut self,
        channel: String,
        id: String,
        meta_json: String,
    ) -> Result<near::agent::channel_host::PairingUpsertResult, String> {
        let meta = if meta_json.is_empty() {
            None
        } else {
            serde_json::from_str(&meta_json).ok()
        };
        match self.pairing_store.upsert_request(&channel, &id, meta) {
            Ok(r) => Ok(near::agent::channel_host::PairingUpsertResult {
                code: r.code,
                created: r.created,
            }),
            Err(e) => Err(e.to_string()),
        }
    }

    fn pairing_is_allowed(
        &mut self,
        channel: String,
        id: String,
        username: Option<String>,
    ) -> Result<bool, String> {
        self.pairing_store
            .is_sender_allowed(&channel, &id, username.as_deref())
            .map_err(|e| e.to_string())
    }

    fn pairing_read_allow_from(&mut self, channel: String) -> Result<Vec<String>, String> {
        self.pairing_store
            .read_allow_from(&channel)
            .map_err(|e| e.to_string())
    }
}

/// A WASM-based channel implementing the Channel trait.
#[allow(dead_code)]
pub struct WasmChannel {
    /// Channel name.
    name: String,

    /// Runtime for WASM execution.
    runtime: Arc<WasmChannelRuntime>,

    /// Prepared module (compiled WASM).
    prepared: Arc<PreparedChannelModule>,

    /// Channel capabilities.
    capabilities: ChannelCapabilities,

    /// Channel configuration JSON (passed to on_start).
    /// Wrapped in RwLock to allow updating before start.
    config_json: RwLock<String>,

    /// Channel configuration returned by on_start.
    channel_config: RwLock<Option<ChannelConfig>>,

    /// Message sender (for emitting messages to the stream).
    /// Wrapped in Arc for sharing with the polling task.
    message_tx: Arc<RwLock<Option<mpsc::Sender<IncomingMessage>>>>,

    /// Pending responses (for synchronous response handling).
    pending_responses: RwLock<HashMap<Uuid, oneshot::Sender<String>>>,

    /// Rate limiter for message emission.
    /// Wrapped in Arc for sharing with the polling task.
    rate_limiter: Arc<RwLock<ChannelEmitRateLimiter>>,

    /// Shutdown signal sender.
    shutdown_tx: RwLock<Option<oneshot::Sender<()>>>,

    /// Polling shutdown signal sender (keeps polling alive while held).
    poll_shutdown_tx: RwLock<Option<oneshot::Sender<()>>>,

    /// Registered HTTP endpoints.
    endpoints: RwLock<Vec<RegisteredEndpoint>>,

    /// Injected credentials for HTTP requests (e.g., bot tokens).
    /// Keys are placeholder names like "TELEGRAM_BOT_TOKEN".
    /// Wrapped in Arc for sharing with the polling task.
    credentials: Arc<RwLock<HashMap<String, String>>>,

    /// Background task that repeats typing indicators every 4 seconds.
    /// Telegram's "typing..." indicator expires after ~5s, so we refresh it.
    typing_task: RwLock<Option<tokio::task::JoinHandle<()>>>,

    /// Pairing store for DM pairing (guest access control).
    pairing_store: Arc<PairingStore>,

    /// In-memory workspace store persisting writes across callback invocations.
    /// Ensures WASM channels can maintain state (e.g., polling offsets) between ticks.
    workspace_store: Arc<ChannelWorkspaceStore>,
}

impl WasmChannel {
    /// Create a new WASM channel.
    pub fn new(
        runtime: Arc<WasmChannelRuntime>,
        prepared: Arc<PreparedChannelModule>,
        capabilities: ChannelCapabilities,
        config_json: String,
        pairing_store: Arc<PairingStore>,
    ) -> Self {
        let name = prepared.name.clone();
        let rate_limiter = ChannelEmitRateLimiter::new(capabilities.emit_rate_limit.clone());

        Self {
            name,
            runtime,
            prepared,
            capabilities,
            config_json: RwLock::new(config_json),
            channel_config: RwLock::new(None),
            message_tx: Arc::new(RwLock::new(None)),
            pending_responses: RwLock::new(HashMap::new()),
            rate_limiter: Arc::new(RwLock::new(rate_limiter)),
            shutdown_tx: RwLock::new(None),
            poll_shutdown_tx: RwLock::new(None),
            endpoints: RwLock::new(Vec::new()),
            credentials: Arc::new(RwLock::new(HashMap::new())),
            typing_task: RwLock::new(None),
            pairing_store,
            workspace_store: Arc::new(ChannelWorkspaceStore::new()),
        }
    }

    /// Update the channel config before starting.
    ///
    /// Merges the provided values into the existing config JSON.
    /// Call this before `start()` to inject runtime values like tunnel_url.
    pub async fn update_config(&self, updates: HashMap<String, serde_json::Value>) {
        let mut config_guard = self.config_json.write().await;

        // Parse existing config
        let mut config: HashMap<String, serde_json::Value> =
            serde_json::from_str(&config_guard).unwrap_or_default();

        // Merge updates
        for (key, value) in updates {
            config.insert(key, value);
        }

        // Serialize back
        *config_guard = serde_json::to_string(&config).unwrap_or_else(|_| "{}".to_string());

        tracing::debug!(
            channel = %self.name,
            config = %*config_guard,
            "Updated channel config"
        );
    }

    /// Set a credential for URL injection.
    pub async fn set_credential(&self, name: &str, value: String) {
        self.credentials
            .write()
            .await
            .insert(name.to_string(), value);
    }

    /// Get a snapshot of credentials for use in callbacks.
    pub async fn get_credentials(&self) -> HashMap<String, String> {
        self.credentials.read().await.clone()
    }

    /// Get the channel name.
    pub fn channel_name(&self) -> &str {
        &self.name
    }

    /// Get the channel capabilities.
    pub fn capabilities(&self) -> &ChannelCapabilities {
        &self.capabilities
    }

    /// Get the registered endpoints.
    pub async fn endpoints(&self) -> Vec<RegisteredEndpoint> {
        self.endpoints.read().await.clone()
    }

    /// Inject the workspace store as the reader into a capabilities clone.
    ///
    /// Ensures `workspace_read` capability is present with the store as its reader,
    /// so WASM callbacks can read previously written workspace state.
    fn inject_workspace_reader(
        capabilities: &ChannelCapabilities,
        store: &Arc<ChannelWorkspaceStore>,
    ) -> ChannelCapabilities {
        let mut caps = capabilities.clone();
        let ws_cap = caps
            .tool_capabilities
            .workspace_read
            .get_or_insert_with(|| crate::tools::wasm::WorkspaceCapability {
                allowed_prefixes: Vec::new(),
                reader: None,
            });
        ws_cap.reader = Some(Arc::clone(store) as Arc<dyn crate::tools::wasm::WorkspaceReader>);
        caps
    }

    /// Add channel host functions to the linker using generated bindings.
    ///
    /// Uses the wasmtime::component::bindgen! generated `add_to_linker` function
    /// to properly register all host functions with correct component model signatures.
    fn add_host_functions(linker: &mut Linker<ChannelStoreData>) -> Result<(), WasmChannelError> {
        // Add WASI support (required by the component adapter)
        wasmtime_wasi::add_to_linker_sync(linker).map_err(|e| {
            WasmChannelError::Config(format!("Failed to add WASI functions: {}", e))
        })?;

        // Use the generated add_to_linker function from bindgen for our custom interface
        near::agent::channel_host::add_to_linker(linker, |state| state).map_err(|e| {
            WasmChannelError::Config(format!("Failed to add host functions: {}", e))
        })?;

        Ok(())
    }

    /// Create a fresh store configured for WASM execution.
    fn create_store(
        runtime: &WasmChannelRuntime,
        prepared: &PreparedChannelModule,
        capabilities: &ChannelCapabilities,
        credentials: HashMap<String, String>,
        pairing_store: Arc<PairingStore>,
    ) -> Result<Store<ChannelStoreData>, WasmChannelError> {
        let engine = runtime.engine();
        let limits = &prepared.limits;

        // Create fresh store with channel state (NEAR pattern: fresh instance per call)
        let store_data = ChannelStoreData::new(
            limits.memory_bytes,
            &prepared.name,
            capabilities.clone(),
            credentials,
            pairing_store,
        );
        let mut store = Store::new(engine, store_data);

        // Configure fuel if enabled
        if runtime.config().fuel_config.enabled {
            store
                .set_fuel(limits.fuel)
                .map_err(|e| WasmChannelError::Config(format!("Failed to set fuel: {}", e)))?;
        }

        // Configure epoch deadline for timeout backup
        store.epoch_deadline_trap();
        store.set_epoch_deadline(1);

        // Set up resource limiter
        store.limiter(|data| &mut data.limiter);

        Ok(store)
    }

    /// Instantiate the WASM component using generated bindings.
    fn instantiate_component(
        runtime: &WasmChannelRuntime,
        prepared: &PreparedChannelModule,
        store: &mut Store<ChannelStoreData>,
    ) -> Result<SandboxedChannel, WasmChannelError> {
        let engine = runtime.engine();

        // Use the pre-compiled component (no recompilation needed)
        let component = prepared
            .component()
            .ok_or_else(|| {
                WasmChannelError::Compilation("No compiled component available".to_string())
            })?
            .clone();

        // Create linker and add host functions
        let mut linker = Linker::new(engine);
        Self::add_host_functions(&mut linker)?;

        // Instantiate using the generated bindings
        let instance = SandboxedChannel::instantiate(store, &component, &linker)
            .map_err(|e| WasmChannelError::Instantiation(e.to_string()))?;

        Ok(instance)
    }

    /// Map WASM execution errors to our error types.
    fn map_wasm_error(e: anyhow::Error, name: &str, fuel_limit: u64) -> WasmChannelError {
        let error_str = e.to_string();
        if error_str.contains("out of fuel") {
            WasmChannelError::FuelExhausted {
                name: name.to_string(),
                limit: fuel_limit,
            }
        } else if error_str.contains("unreachable") {
            WasmChannelError::Trapped {
                name: name.to_string(),
                reason: "unreachable code executed".to_string(),
            }
        } else {
            WasmChannelError::Trapped {
                name: name.to_string(),
                reason: error_str,
            }
        }
    }

    /// Extract host state after callback execution.
    fn extract_host_state(
        store: &mut Store<ChannelStoreData>,
        channel_name: &str,
        capabilities: &ChannelCapabilities,
    ) -> ChannelHostState {
        std::mem::replace(
            &mut store.data_mut().host_state,
            ChannelHostState::new(channel_name, capabilities.clone()),
        )
    }

    /// Execute the on_start callback.
    ///
    /// Returns the channel configuration for HTTP endpoint registration.
    /// Call the WASM module's `on_start` callback.
    ///
    /// Typically called once during `start()`, but can be called again after
    /// credentials are refreshed to re-trigger webhook registration and
    /// other one-time setup that depends on credentials.
    pub async fn call_on_start(&self) -> Result<ChannelConfig, WasmChannelError> {
        // If no WASM bytes, return default config (for testing)
        if self.prepared.component().is_none() {
            tracing::info!(
                channel = %self.name,
                "WASM channel on_start called (no WASM module, returning defaults)"
            );
            return Ok(ChannelConfig {
                display_name: self.prepared.description.clone(),
                http_endpoints: Vec::new(),
                poll: None,
            });
        }

        let runtime = Arc::clone(&self.runtime);
        let prepared = Arc::clone(&self.prepared);
        let capabilities = Self::inject_workspace_reader(&self.capabilities, &self.workspace_store);
        let config_json = self.config_json.read().await.clone();
        let timeout = self.runtime.config().callback_timeout;
        let channel_name = self.name.clone();
        let credentials = self.get_credentials().await;
        let pairing_store = self.pairing_store.clone();
        let workspace_store = self.workspace_store.clone();

        // Execute in blocking task with timeout
        let result = tokio::time::timeout(timeout, async move {
            tokio::task::spawn_blocking(move || {
                let mut store = Self::create_store(
                    &runtime,
                    &prepared,
                    &capabilities,
                    credentials,
                    pairing_store,
                )?;
                let instance = Self::instantiate_component(&runtime, &prepared, &mut store)?;

                // Call on_start using the generated typed interface
                let channel_iface = instance.near_agent_channel();
                let wasm_result = channel_iface
                    .call_on_start(&mut store, &config_json)
                    .map_err(|e| Self::map_wasm_error(e, &prepared.name, prepared.limits.fuel))?;

                // Convert the result
                let config = match wasm_result {
                    Ok(wit_config) => convert_channel_config(wit_config),
                    Err(err_msg) => {
                        return Err(WasmChannelError::CallbackFailed {
                            name: prepared.name.clone(),
                            reason: err_msg,
                        });
                    }
                };

                let mut host_state =
                    Self::extract_host_state(&mut store, &prepared.name, &capabilities);

                // Commit pending workspace writes to the persistent store
                let pending_writes = host_state.take_pending_writes();
                workspace_store.commit_writes(&pending_writes);

                Ok((config, host_state))
            })
            .await
            .map_err(|e| WasmChannelError::ExecutionPanicked {
                name: channel_name.clone(),
                reason: e.to_string(),
            })?
        })
        .await;

        match result {
            Ok(Ok((config, mut host_state))) => {
                // Surface WASM guest logs (errors/warnings from webhook setup, etc.)
                for entry in host_state.take_logs() {
                    match entry.level {
                        crate::tools::wasm::LogLevel::Error => {
                            tracing::error!(channel = %self.name, "{}", entry.message);
                        }
                        crate::tools::wasm::LogLevel::Warn => {
                            tracing::warn!(channel = %self.name, "{}", entry.message);
                        }
                        _ => {
                            tracing::debug!(channel = %self.name, "{}", entry.message);
                        }
                    }
                }
                tracing::info!(
                    channel = %self.name,
                    display_name = %config.display_name,
                    endpoints = config.http_endpoints.len(),
                    "WASM channel on_start completed"
                );
                Ok(config)
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(WasmChannelError::Timeout {
                name: self.name.clone(),
                callback: "on_start".to_string(),
            }),
        }
    }

    /// Execute the on_http_request callback.
    ///
    /// Called when an HTTP request arrives at a registered endpoint.
    pub async fn call_on_http_request(
        &self,
        method: &str,
        path: &str,
        headers: &HashMap<String, String>,
        query: &HashMap<String, String>,
        body: &[u8],
        secret_validated: bool,
    ) -> Result<HttpResponse, WasmChannelError> {
        tracing::info!(
            channel = %self.name,
            method = method,
            path = path,
            body_len = body.len(),
            secret_validated = secret_validated,
            "call_on_http_request invoked (webhook received)"
        );

        // Log the body for debugging (truncated at char boundary)
        if let Ok(body_str) = std::str::from_utf8(body) {
            let truncated = if body_str.chars().count() > 1000 {
                format!("{}...", body_str.chars().take(1000).collect::<String>())
            } else {
                body_str.to_string()
            };
            tracing::debug!(body = %truncated, "Webhook request body");
        }

        // Log credentials state (without values)
        let creds = self.get_credentials().await;
        tracing::info!(
            credential_count = creds.len(),
            credential_names = ?creds.keys().collect::<Vec<_>>(),
            "Credentials available for on_http_request"
        );

        // If no WASM bytes, return 200 OK (for testing)
        if self.prepared.component().is_none() {
            tracing::debug!(
                channel = %self.name,
                method = method,
                path = path,
                "WASM channel on_http_request called (no WASM module)"
            );
            return Ok(HttpResponse::ok());
        }

        let runtime = Arc::clone(&self.runtime);
        let prepared = Arc::clone(&self.prepared);
        let capabilities = Self::inject_workspace_reader(&self.capabilities, &self.workspace_store);
        let timeout = self.runtime.config().callback_timeout;
        let credentials = self.get_credentials().await;
        let pairing_store = self.pairing_store.clone();
        let workspace_store = self.workspace_store.clone();

        // Prepare request data
        let method = method.to_string();
        let path = path.to_string();
        let headers_json = serde_json::to_string(&headers).unwrap_or_default();
        let query_json = serde_json::to_string(&query).unwrap_or_default();
        let body = body.to_vec();

        let channel_name = self.name.clone();

        // Execute in blocking task with timeout
        let result = tokio::time::timeout(timeout, async move {
            tokio::task::spawn_blocking(move || {
                let mut store = Self::create_store(
                    &runtime,
                    &prepared,
                    &capabilities,
                    credentials,
                    pairing_store,
                )?;
                let instance = Self::instantiate_component(&runtime, &prepared, &mut store)?;

                // Build the WIT request type
                let wit_request = wit_channel::IncomingHttpRequest {
                    method,
                    path,
                    headers_json,
                    query_json,
                    body,
                    secret_validated,
                };

                // Call on_http_request using the generated typed interface
                let channel_iface = instance.near_agent_channel();
                let wit_response = channel_iface
                    .call_on_http_request(&mut store, &wit_request)
                    .map_err(|e| Self::map_wasm_error(e, &prepared.name, prepared.limits.fuel))?;

                let response = convert_http_response(wit_response);
                let mut host_state =
                    Self::extract_host_state(&mut store, &prepared.name, &capabilities);

                // Commit pending workspace writes to the persistent store
                let pending_writes = host_state.take_pending_writes();
                workspace_store.commit_writes(&pending_writes);

                Ok((response, host_state))
            })
            .await
            .map_err(|e| WasmChannelError::ExecutionPanicked {
                name: channel_name.clone(),
                reason: e.to_string(),
            })?
        })
        .await;

        let channel_name = self.name.clone();
        match result {
            Ok(Ok((response, mut host_state))) => {
                // Process emitted messages
                let emitted = host_state.take_emitted_messages();
                self.process_emitted_messages(emitted).await?;

                tracing::debug!(
                    channel = %channel_name,
                    status = response.status,
                    "WASM channel on_http_request completed"
                );
                Ok(response)
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(WasmChannelError::Timeout {
                name: channel_name,
                callback: "on_http_request".to_string(),
            }),
        }
    }

    /// Execute the on_poll callback.
    ///
    /// Called periodically if polling is configured.
    pub async fn call_on_poll(&self) -> Result<(), WasmChannelError> {
        // If no WASM bytes, do nothing (for testing)
        if self.prepared.component().is_none() {
            tracing::debug!(
                channel = %self.name,
                "WASM channel on_poll called (no WASM module)"
            );
            return Ok(());
        }

        let runtime = Arc::clone(&self.runtime);
        let prepared = Arc::clone(&self.prepared);
        let capabilities = Self::inject_workspace_reader(&self.capabilities, &self.workspace_store);
        let timeout = self.runtime.config().callback_timeout;
        let channel_name = self.name.clone();
        let credentials = self.get_credentials().await;
        let pairing_store = self.pairing_store.clone();
        let workspace_store = self.workspace_store.clone();

        // Execute in blocking task with timeout
        let result = tokio::time::timeout(timeout, async move {
            tokio::task::spawn_blocking(move || {
                let mut store = Self::create_store(
                    &runtime,
                    &prepared,
                    &capabilities,
                    credentials,
                    pairing_store,
                )?;
                let instance = Self::instantiate_component(&runtime, &prepared, &mut store)?;

                // Call on_poll using the generated typed interface
                let channel_iface = instance.near_agent_channel();
                channel_iface
                    .call_on_poll(&mut store)
                    .map_err(|e| Self::map_wasm_error(e, &prepared.name, prepared.limits.fuel))?;

                let mut host_state =
                    Self::extract_host_state(&mut store, &prepared.name, &capabilities);

                // Commit pending workspace writes to the persistent store
                let pending_writes = host_state.take_pending_writes();
                workspace_store.commit_writes(&pending_writes);

                Ok(((), host_state))
            })
            .await
            .map_err(|e| WasmChannelError::ExecutionPanicked {
                name: channel_name.clone(),
                reason: e.to_string(),
            })?
        })
        .await;

        let channel_name = self.name.clone();
        match result {
            Ok(Ok(((), mut host_state))) => {
                // Process emitted messages
                let emitted = host_state.take_emitted_messages();
                self.process_emitted_messages(emitted).await?;

                tracing::debug!(
                    channel = %channel_name,
                    "WASM channel on_poll completed"
                );
                Ok(())
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(WasmChannelError::Timeout {
                name: channel_name,
                callback: "on_poll".to_string(),
            }),
        }
    }

    /// Execute the on_respond callback.
    ///
    /// Called when the agent has a response to send back.
    pub async fn call_on_respond(
        &self,
        message_id: Uuid,
        content: &str,
        thread_id: Option<&str>,
        metadata_json: &str,
    ) -> Result<(), WasmChannelError> {
        tracing::info!(
            channel = %self.name,
            message_id = %message_id,
            content_len = content.len(),
            thread_id = ?thread_id,
            "call_on_respond invoked"
        );

        // Log credentials state (without values)
        let creds = self.get_credentials().await;
        tracing::info!(
            credential_count = creds.len(),
            credential_names = ?creds.keys().collect::<Vec<_>>(),
            "Credentials available for on_respond"
        );

        // If no WASM bytes, do nothing (for testing)
        if self.prepared.component().is_none() {
            tracing::debug!(
                channel = %self.name,
                message_id = %message_id,
                "WASM channel on_respond called (no WASM module)"
            );
            return Ok(());
        }

        let runtime = Arc::clone(&self.runtime);
        let prepared = Arc::clone(&self.prepared);
        let capabilities = self.capabilities.clone();
        let timeout = self.runtime.config().callback_timeout;
        let channel_name = self.name.clone();
        let credentials = self.get_credentials().await;
        let pairing_store = self.pairing_store.clone();

        // Prepare response data
        let message_id_str = message_id.to_string();
        let content = content.to_string();
        let thread_id = thread_id.map(|s| s.to_string());
        let metadata_json = metadata_json.to_string();

        // Execute in blocking task with timeout
        tracing::info!(channel = %channel_name, "Starting on_respond WASM execution");

        let result = tokio::time::timeout(timeout, async move {
            tokio::task::spawn_blocking(move || {
                tracing::info!("Creating WASM store for on_respond");
                let mut store = Self::create_store(
                    &runtime,
                    &prepared,
                    &capabilities,
                    credentials,
                    pairing_store,
                )?;

                tracing::info!("Instantiating WASM component for on_respond");
                let instance = Self::instantiate_component(&runtime, &prepared, &mut store)?;

                // Build the WIT response type
                let wit_response = wit_channel::AgentResponse {
                    message_id: message_id_str,
                    content: content.clone(),
                    thread_id,
                    metadata_json,
                };

                // Truncate at char boundary for logging (avoid panic on multi-byte UTF-8)
                let content_preview: String = content.chars().take(50).collect();
                tracing::info!(
                    content_preview = %content_preview,
                    "Calling WASM on_respond"
                );

                // Call on_respond using the generated typed interface
                let channel_iface = instance.near_agent_channel();
                let wasm_result = channel_iface
                    .call_on_respond(&mut store, &wit_response)
                    .map_err(|e| {
                        tracing::error!(error = %e, "WASM on_respond call failed");
                        Self::map_wasm_error(e, &prepared.name, prepared.limits.fuel)
                    })?;

                tracing::info!(wasm_result = ?wasm_result, "WASM on_respond returned");

                // Check for WASM-level errors
                if let Err(ref err_msg) = wasm_result {
                    tracing::error!(error = %err_msg, "WASM on_respond returned error");
                    return Err(WasmChannelError::CallbackFailed {
                        name: prepared.name.clone(),
                        reason: err_msg.clone(),
                    });
                }

                let host_state =
                    Self::extract_host_state(&mut store, &prepared.name, &capabilities);
                tracing::info!("on_respond WASM execution completed successfully");
                Ok(((), host_state))
            })
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "spawn_blocking panicked");
                WasmChannelError::ExecutionPanicked {
                    name: channel_name.clone(),
                    reason: e.to_string(),
                }
            })?
        })
        .await;

        let channel_name = self.name.clone();
        match result {
            Ok(Ok(((), _host_state))) => {
                tracing::debug!(
                    channel = %channel_name,
                    message_id = %message_id,
                    "WASM channel on_respond completed"
                );
                Ok(())
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(WasmChannelError::Timeout {
                name: channel_name,
                callback: "on_respond".to_string(),
            }),
        }
    }

    /// Execute the on_status callback.
    ///
    /// Called to notify the WASM channel of agent status changes (e.g., typing).
    pub async fn call_on_status(
        &self,
        status: &StatusUpdate,
        metadata: &serde_json::Value,
    ) -> Result<(), WasmChannelError> {
        // If no WASM bytes, do nothing (for testing)
        if self.prepared.component().is_none() {
            return Ok(());
        }

        let runtime = Arc::clone(&self.runtime);
        let prepared = Arc::clone(&self.prepared);
        let capabilities = self.capabilities.clone();
        let timeout = self.runtime.config().callback_timeout;
        let channel_name = self.name.clone();
        let credentials = self.get_credentials().await;
        let pairing_store = self.pairing_store.clone();

        let wit_update = status_to_wit(status, metadata);

        let result = tokio::time::timeout(timeout, async move {
            tokio::task::spawn_blocking(move || {
                let mut store = Self::create_store(
                    &runtime,
                    &prepared,
                    &capabilities,
                    credentials,
                    pairing_store,
                )?;
                let instance = Self::instantiate_component(&runtime, &prepared, &mut store)?;

                let channel_iface = instance.near_agent_channel();
                channel_iface
                    .call_on_status(&mut store, &wit_update)
                    .map_err(|e| Self::map_wasm_error(e, &prepared.name, prepared.limits.fuel))?;

                Ok(())
            })
            .await
            .map_err(|e| WasmChannelError::ExecutionPanicked {
                name: channel_name.clone(),
                reason: e.to_string(),
            })?
        })
        .await;

        match result {
            Ok(Ok(())) => {
                tracing::debug!(
                    channel = %self.name,
                    "WASM channel on_status completed"
                );
                Ok(())
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(WasmChannelError::Timeout {
                name: self.name.clone(),
                callback: "on_status".to_string(),
            }),
        }
    }

    /// Execute a single on_status callback with a fresh WASM instance.
    ///
    /// Static method for use by the background typing repeat task (which
    /// doesn't have access to `&self`).
    #[allow(clippy::too_many_arguments)]
    async fn execute_status(
        channel_name: &str,
        runtime: &Arc<WasmChannelRuntime>,
        prepared: &Arc<PreparedChannelModule>,
        capabilities: &ChannelCapabilities,
        credentials: &RwLock<HashMap<String, String>>,
        pairing_store: Arc<PairingStore>,
        timeout: Duration,
        wit_update: wit_channel::StatusUpdate,
    ) -> Result<(), WasmChannelError> {
        if prepared.component().is_none() {
            return Ok(());
        }

        let runtime = Arc::clone(runtime);
        let prepared = Arc::clone(prepared);
        let capabilities = capabilities.clone();
        let credentials_snapshot = credentials.read().await.clone();
        let channel_name_owned = channel_name.to_string();

        let result = tokio::time::timeout(timeout, async move {
            tokio::task::spawn_blocking(move || {
                let mut store = Self::create_store(
                    &runtime,
                    &prepared,
                    &capabilities,
                    credentials_snapshot,
                    pairing_store,
                )?;
                let instance = Self::instantiate_component(&runtime, &prepared, &mut store)?;

                let channel_iface = instance.near_agent_channel();
                channel_iface
                    .call_on_status(&mut store, &wit_update)
                    .map_err(|e| Self::map_wasm_error(e, &prepared.name, prepared.limits.fuel))?;

                Ok(())
            })
            .await
            .map_err(|e| WasmChannelError::ExecutionPanicked {
                name: channel_name_owned.clone(),
                reason: e.to_string(),
            })?
        })
        .await;

        match result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(WasmChannelError::Timeout {
                name: channel_name.to_string(),
                callback: "on_status".to_string(),
            }),
        }
    }

    /// Cancel the background typing indicator task if running.
    async fn cancel_typing_task(&self) {
        if let Some(handle) = self.typing_task.write().await.take() {
            handle.abort();
        }
    }

    /// Handle a status update, managing the typing repeat timer.
    ///
    /// On Thinking: fires on_status once, then spawns a background task
    /// that repeats the call every 4 seconds (Telegram's typing indicator
    /// expires after ~5s).
    ///
    /// On terminal or user-action-required states: cancels the repeat task,
    /// then fires on_status once.
    ///
    /// On intermediate progress states (tool/auth/job/status updates), keeps
    /// the typing repeater running and fires on_status once.
    /// On StreamChunk: no-op (too noisy).
    async fn handle_status_update(
        &self,
        status: StatusUpdate,
        metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        fn is_terminal_text_status(msg: &str) -> bool {
            let trimmed = msg.trim();
            trimmed.eq_ignore_ascii_case("done")
                || trimmed.eq_ignore_ascii_case("interrupted")
                || trimmed.eq_ignore_ascii_case("awaiting approval")
                || trimmed.eq_ignore_ascii_case("rejected")
        }

        match &status {
            StatusUpdate::Thinking(_) => {
                // Cancel any existing typing task
                self.cancel_typing_task().await;

                // Fire once immediately
                if let Err(e) = self.call_on_status(&status, metadata).await {
                    tracing::debug!(
                        channel = %self.name,
                        error = %e,
                        "on_status(Thinking) failed (best-effort)"
                    );
                }

                // Spawn background repeater
                let channel_name = self.name.clone();
                let runtime = Arc::clone(&self.runtime);
                let prepared = Arc::clone(&self.prepared);
                let capabilities = self.capabilities.clone();
                let credentials = self.credentials.clone();
                let pairing_store = self.pairing_store.clone();
                let callback_timeout = self.runtime.config().callback_timeout;
                let wit_update = status_to_wit(&status, metadata);

                let handle = tokio::spawn(async move {
                    let mut interval = tokio::time::interval(Duration::from_secs(4));
                    // Skip the first tick (we already fired above)
                    interval.tick().await;

                    loop {
                        interval.tick().await;

                        let wit_update_clone = clone_wit_status_update(&wit_update);

                        if let Err(e) = Self::execute_status(
                            &channel_name,
                            &runtime,
                            &prepared,
                            &capabilities,
                            &credentials,
                            pairing_store.clone(),
                            callback_timeout,
                            wit_update_clone,
                        )
                        .await
                        {
                            tracing::debug!(
                                channel = %channel_name,
                                error = %e,
                                "Typing repeat on_status failed (best-effort)"
                            );
                        }
                    }
                });

                *self.typing_task.write().await = Some(handle);
            }
            StatusUpdate::StreamChunk(_) => {
                // No-op, too noisy
            }
            StatusUpdate::ApprovalNeeded {
                tool_name,
                description,
                parameters,
                ..
            } => {
                // WASM channels (Telegram, Slack, etc.) cannot render
                // interactive approval overlays.  Send the approval prompt
                // as an actual message so the user can reply yes/no.
                self.cancel_typing_task().await;

                let params_preview = parameters
                    .as_object()
                    .map(|obj| {
                        obj.iter()
                            .map(|(k, v)| {
                                let val = match v {
                                    serde_json::Value::String(s) => {
                                        if s.chars().count() > 80 {
                                            let truncated: String = s.chars().take(77).collect();
                                            format!("\"{}...\"", truncated)
                                        } else {
                                            format!("\"{}\"", s)
                                        }
                                    }
                                    other => {
                                        let s = other.to_string();
                                        if s.chars().count() > 80 {
                                            let truncated: String = s.chars().take(77).collect();
                                            format!("{}...", truncated)
                                        } else {
                                            s
                                        }
                                    }
                                };
                                format!("  {}: {}", k, val)
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .unwrap_or_default();

                let prompt = format!(
                    "Approval needed: {tool_name}\n\
                     {description}\n\
                     \n\
                     Parameters:\n\
                     {params_preview}\n\
                     \n\
                     Reply \"yes\" to approve, \"no\" to deny, or \"always\" to auto-approve."
                );

                let metadata_json = serde_json::to_string(metadata).unwrap_or_default();
                if let Err(e) = self
                    .call_on_respond(uuid::Uuid::new_v4(), &prompt, None, &metadata_json)
                    .await
                {
                    tracing::warn!(
                        channel = %self.name,
                        error = %e,
                        "Failed to send approval prompt via on_respond, falling back to on_status"
                    );
                    // Fall back to status update (typing indicator)
                    let _ = self.call_on_status(&status, metadata).await;
                }
            }
            StatusUpdate::AuthRequired { .. } => {
                // Waiting on user action: stop typing and fire once.
                self.cancel_typing_task().await;

                if let Err(e) = self.call_on_status(&status, metadata).await {
                    tracing::debug!(
                        channel = %self.name,
                        error = %e,
                        "on_status failed (best-effort)"
                    );
                }
            }
            StatusUpdate::Status(msg) if is_terminal_text_status(msg) => {
                // Waiting on user or terminal states: stop typing and fire once.
                self.cancel_typing_task().await;

                if let Err(e) = self.call_on_status(&status, metadata).await {
                    tracing::debug!(
                        channel = %self.name,
                        error = %e,
                        "on_status failed (best-effort)"
                    );
                }
            }
            _ => {
                // Intermediate progress status: keep any existing typing task alive.
                if let Err(e) = self.call_on_status(&status, metadata).await {
                    tracing::debug!(
                        channel = %self.name,
                        error = %e,
                        "on_status failed (best-effort)"
                    );
                }
            }
        }

        Ok(())
    }

    /// Process emitted messages from a callback.
    async fn process_emitted_messages(
        &self,
        messages: Vec<EmittedMessage>,
    ) -> Result<(), WasmChannelError> {
        tracing::info!(
            channel = %self.name,
            message_count = messages.len(),
            "Processing emitted messages from WASM callback"
        );

        if messages.is_empty() {
            tracing::debug!(channel = %self.name, "No messages emitted");
            return Ok(());
        }

        let tx_guard = self.message_tx.read().await;
        let Some(tx) = tx_guard.as_ref() else {
            tracing::error!(
                channel = %self.name,
                count = messages.len(),
                "Messages emitted but no sender available - channel may not be started!"
            );
            return Ok(());
        };

        let mut rate_limiter = self.rate_limiter.write().await;

        for emitted in messages {
            // Check rate limit
            if !rate_limiter.check_and_record() {
                tracing::warn!(
                    channel = %self.name,
                    "Message emission rate limited"
                );
                return Err(WasmChannelError::EmitRateLimited {
                    name: self.name.clone(),
                });
            }

            // Convert to IncomingMessage
            let mut msg = IncomingMessage::new(&self.name, &emitted.user_id, &emitted.content);

            if let Some(name) = emitted.user_name {
                msg = msg.with_user_name(name);
            }

            if let Some(thread_id) = emitted.thread_id {
                msg = msg.with_thread(thread_id);
            }

            // Parse metadata JSON
            if let Ok(metadata) = serde_json::from_str(&emitted.metadata_json) {
                msg = msg.with_metadata(metadata);
            }

            // Send to stream
            tracing::info!(
                channel = %self.name,
                user_id = %emitted.user_id,
                content_len = emitted.content.len(),
                "Sending emitted message to agent"
            );

            if tx.send(msg).await.is_err() {
                tracing::error!(
                    channel = %self.name,
                    "Failed to send emitted message, channel closed"
                );
                break;
            }

            tracing::info!(
                channel = %self.name,
                "Message successfully sent to agent queue"
            );
        }

        Ok(())
    }

    /// Start the polling loop if configured.
    ///
    /// Since we can't hold `Arc<Self>` from `&self`, we pass all the components
    /// needed for polling to a spawned task. Each poll tick creates a fresh WASM
    /// instance (matching our "fresh instance per callback" pattern).
    fn start_polling(&self, interval: Duration, shutdown_rx: oneshot::Receiver<()>) {
        let channel_name = self.name.clone();
        let runtime = Arc::clone(&self.runtime);
        let prepared = Arc::clone(&self.prepared);
        let capabilities = self.capabilities.clone();
        let message_tx = self.message_tx.clone();
        let rate_limiter = self.rate_limiter.clone();
        let credentials = self.credentials.clone();
        let pairing_store = self.pairing_store.clone();
        let callback_timeout = self.runtime.config().callback_timeout;
        let workspace_store = self.workspace_store.clone();

        tokio::spawn(async move {
            let mut interval_timer = tokio::time::interval(interval);
            let mut shutdown = std::pin::pin!(shutdown_rx);

            loop {
                tokio::select! {
                    _ = interval_timer.tick() => {
                        tracing::debug!(
                            channel = %channel_name,
                            "Polling tick - calling on_poll"
                        );

                        // Execute on_poll with fresh WASM instance
                        let result = Self::execute_poll(
                            &channel_name,
                            &runtime,
                            &prepared,
                            &capabilities,
                            &credentials,
                            pairing_store.clone(),
                            callback_timeout,
                            &workspace_store,
                        ).await;

                        match result {
                            Ok(emitted_messages) => {
                                // Process any emitted messages
                                if !emitted_messages.is_empty()
                                    && let Err(e) = Self::dispatch_emitted_messages(
                                        &channel_name,
                                        emitted_messages,
                                        &message_tx,
                                        &rate_limiter,
                                    ).await {
                                        tracing::warn!(
                                            channel = %channel_name,
                                            error = %e,
                                            "Failed to dispatch emitted messages from poll"
                                        );
                                    }
                            }
                            Err(e) => {
                                tracing::warn!(
                                    channel = %channel_name,
                                    error = %e,
                                    "Polling callback failed"
                                );
                            }
                        }
                    }
                    _ = &mut shutdown => {
                        tracing::info!(
                            channel = %channel_name,
                            "Polling stopped"
                        );
                        break;
                    }
                }
            }
        });
    }

    /// Execute a single poll callback with a fresh WASM instance.
    ///
    /// Returns any emitted messages from the callback. Pending workspace writes
    /// are committed to the shared `ChannelWorkspaceStore` so state persists
    /// across poll ticks (e.g., Telegram polling offset).
    #[allow(clippy::too_many_arguments)]
    async fn execute_poll(
        channel_name: &str,
        runtime: &Arc<WasmChannelRuntime>,
        prepared: &Arc<PreparedChannelModule>,
        capabilities: &ChannelCapabilities,
        credentials: &RwLock<HashMap<String, String>>,
        pairing_store: Arc<PairingStore>,
        timeout: Duration,
        workspace_store: &Arc<ChannelWorkspaceStore>,
    ) -> Result<Vec<EmittedMessage>, WasmChannelError> {
        // Skip if no WASM bytes (testing mode)
        if prepared.component().is_none() {
            tracing::debug!(
                channel = %channel_name,
                "WASM channel on_poll called (no WASM module)"
            );
            return Ok(Vec::new());
        }

        let runtime = Arc::clone(runtime);
        let prepared = Arc::clone(prepared);
        let capabilities = Self::inject_workspace_reader(capabilities, workspace_store);
        let credentials_snapshot = credentials.read().await.clone();
        let channel_name_owned = channel_name.to_string();
        let workspace_store = Arc::clone(workspace_store);

        // Execute in blocking task with timeout
        let result = tokio::time::timeout(timeout, async move {
            tokio::task::spawn_blocking(move || {
                let mut store = Self::create_store(
                    &runtime,
                    &prepared,
                    &capabilities,
                    credentials_snapshot,
                    pairing_store,
                )?;
                let instance = Self::instantiate_component(&runtime, &prepared, &mut store)?;

                // Call on_poll using the generated typed interface
                let channel_iface = instance.near_agent_channel();
                channel_iface
                    .call_on_poll(&mut store)
                    .map_err(|e| Self::map_wasm_error(e, &prepared.name, prepared.limits.fuel))?;

                let mut host_state =
                    Self::extract_host_state(&mut store, &prepared.name, &capabilities);

                // Commit pending workspace writes to the persistent store
                let pending_writes = host_state.take_pending_writes();
                workspace_store.commit_writes(&pending_writes);

                Ok(host_state)
            })
            .await
            .map_err(|e| WasmChannelError::ExecutionPanicked {
                name: channel_name_owned.clone(),
                reason: e.to_string(),
            })?
        })
        .await;

        match result {
            Ok(Ok(mut host_state)) => {
                let emitted = host_state.take_emitted_messages();
                tracing::debug!(
                    channel = %channel_name,
                    emitted_count = emitted.len(),
                    "WASM channel on_poll completed"
                );
                Ok(emitted)
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(WasmChannelError::Timeout {
                name: channel_name.to_string(),
                callback: "on_poll".to_string(),
            }),
        }
    }

    /// Dispatch emitted messages to the message channel.
    ///
    /// This is a static helper used by the polling loop since it doesn't have
    /// access to `&self`.
    async fn dispatch_emitted_messages(
        channel_name: &str,
        messages: Vec<EmittedMessage>,
        message_tx: &RwLock<Option<mpsc::Sender<IncomingMessage>>>,
        rate_limiter: &RwLock<ChannelEmitRateLimiter>,
    ) -> Result<(), WasmChannelError> {
        tracing::info!(
            channel = %channel_name,
            message_count = messages.len(),
            "Processing emitted messages from polling callback"
        );

        let tx_guard = message_tx.read().await;
        let Some(tx) = tx_guard.as_ref() else {
            tracing::error!(
                channel = %channel_name,
                count = messages.len(),
                "Messages emitted but no sender available - channel may not be started!"
            );
            return Ok(());
        };

        let mut limiter = rate_limiter.write().await;

        for emitted in messages {
            // Check rate limit
            if !limiter.check_and_record() {
                tracing::warn!(
                    channel = %channel_name,
                    "Message emission rate limited"
                );
                return Err(WasmChannelError::EmitRateLimited {
                    name: channel_name.to_string(),
                });
            }

            // Convert to IncomingMessage
            let mut msg = IncomingMessage::new(channel_name, &emitted.user_id, &emitted.content);

            if let Some(name) = emitted.user_name {
                msg = msg.with_user_name(name);
            }

            if let Some(thread_id) = emitted.thread_id {
                msg = msg.with_thread(thread_id);
            }

            // Parse metadata JSON
            if let Ok(metadata) = serde_json::from_str(&emitted.metadata_json) {
                msg = msg.with_metadata(metadata);
            }

            // Send to stream
            tracing::info!(
                channel = %channel_name,
                user_id = %emitted.user_id,
                content_len = emitted.content.len(),
                "Sending polled message to agent"
            );

            if tx.send(msg).await.is_err() {
                tracing::error!(
                    channel = %channel_name,
                    "Failed to send polled message, channel closed"
                );
                break;
            }

            tracing::info!(
                channel = %channel_name,
                "Message successfully sent to agent queue"
            );
        }

        Ok(())
    }
}

#[async_trait]
impl Channel for WasmChannel {
    fn name(&self) -> &str {
        &self.name
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        // Create message channel
        let (tx, rx) = mpsc::channel(256);
        *self.message_tx.write().await = Some(tx);

        // Create shutdown channel
        let (shutdown_tx, _shutdown_rx) = oneshot::channel();
        *self.shutdown_tx.write().await = Some(shutdown_tx);

        // Call on_start to get configuration
        let config = self
            .call_on_start()
            .await
            .map_err(|e| ChannelError::StartupFailed {
                name: self.name.clone(),
                reason: e.to_string(),
            })?;

        // Store the config
        *self.channel_config.write().await = Some(config.clone());

        // Register HTTP endpoints
        let mut endpoints = Vec::new();
        for endpoint in &config.http_endpoints {
            // Validate path is allowed
            if !self.capabilities.is_path_allowed(&endpoint.path) {
                tracing::warn!(
                    channel = %self.name,
                    path = %endpoint.path,
                    "HTTP endpoint path not allowed by capabilities"
                );
                continue;
            }

            endpoints.push(RegisteredEndpoint {
                channel_name: self.name.clone(),
                path: endpoint.path.clone(),
                methods: endpoint.methods.clone(),
                require_secret: endpoint.require_secret,
            });
        }
        *self.endpoints.write().await = endpoints;

        // Start polling if configured
        if let Some(poll_config) = &config.poll
            && poll_config.enabled
        {
            let interval = self
                .capabilities
                .validate_poll_interval(poll_config.interval_ms)
                .map_err(|e| ChannelError::StartupFailed {
                    name: self.name.clone(),
                    reason: e,
                })?;

            // Create shutdown channel for polling and store the sender to keep it alive
            let (poll_shutdown_tx, poll_shutdown_rx) = oneshot::channel();
            *self.poll_shutdown_tx.write().await = Some(poll_shutdown_tx);

            self.start_polling(Duration::from_millis(interval as u64), poll_shutdown_rx);
        }

        tracing::info!(
            channel = %self.name,
            display_name = %config.display_name,
            endpoints = config.http_endpoints.len(),
            "WASM channel started"
        );

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn respond(
        &self,
        msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        // Stop the typing indicator, we're about to send the actual response
        self.cancel_typing_task().await;

        // Check if there's a pending synchronous response waiter
        if let Some(tx) = self.pending_responses.write().await.remove(&msg.id) {
            let _ = tx.send(response.content.clone());
        }

        // Call WASM on_respond
        // IMPORTANT: Use the ORIGINAL message's metadata, not the response's metadata.
        // The original metadata contains channel-specific routing info (e.g., Telegram chat_id)
        // that the WASM channel needs to send the reply to the correct destination.
        let metadata_json = serde_json::to_string(&msg.metadata).unwrap_or_default();
        self.call_on_respond(
            msg.id,
            &response.content,
            response.thread_id.as_deref(),
            &metadata_json,
        )
        .await
        .map_err(|e| ChannelError::SendFailed {
            name: self.name.clone(),
            reason: e.to_string(),
        })?;

        Ok(())
    }

    async fn send_status(
        &self,
        status: StatusUpdate,
        metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        // Delegate to the typing indicator implementation
        self.handle_status_update(status, metadata).await
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        // Check if we have an active message sender
        if self.message_tx.read().await.is_some() {
            Ok(())
        } else {
            Err(ChannelError::HealthCheckFailed {
                name: self.name.clone(),
            })
        }
    }

    async fn shutdown(&self) -> Result<(), ChannelError> {
        // Cancel typing indicator
        self.cancel_typing_task().await;

        // Send shutdown signal
        if let Some(tx) = self.shutdown_tx.write().await.take() {
            let _ = tx.send(());
        }

        // Stop polling by dropping the sender (receiver will complete)
        let _ = self.poll_shutdown_tx.write().await.take();

        // Clear the message sender
        *self.message_tx.write().await = None;

        tracing::info!(
            channel = %self.name,
            "WASM channel shut down"
        );

        Ok(())
    }
}

impl std::fmt::Debug for WasmChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmChannel")
            .field("name", &self.name)
            .field("prepared", &self.prepared.name)
            .finish()
    }
}

// ============================================================================
// Shared Channel Wrapper
// ============================================================================

/// A wrapper around `Arc<WasmChannel>` that implements `Channel`.
///
/// This allows sharing the same WasmChannel instance between:
/// - The WasmChannelRouter (for webhook handling)
/// - The ChannelManager (for message streaming and responses)
pub struct SharedWasmChannel {
    inner: Arc<WasmChannel>,
}

impl SharedWasmChannel {
    /// Create a new shared wrapper.
    pub fn new(channel: Arc<WasmChannel>) -> Self {
        Self { inner: channel }
    }

    /// Get the inner Arc.
    pub fn inner(&self) -> &Arc<WasmChannel> {
        &self.inner
    }
}

impl std::fmt::Debug for SharedWasmChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedWasmChannel")
            .field("inner", &self.inner)
            .finish()
    }
}

#[async_trait]
impl Channel for SharedWasmChannel {
    fn name(&self) -> &str {
        self.inner.name()
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        self.inner.start().await
    }

    async fn respond(
        &self,
        msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        self.inner.respond(msg, response).await
    }

    async fn send_status(
        &self,
        status: StatusUpdate,
        metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        self.inner.send_status(status, metadata).await
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        self.inner.health_check().await
    }

    async fn shutdown(&self) -> Result<(), ChannelError> {
        self.inner.shutdown().await
    }
}

// ============================================================================
// WIT Type Conversion Helpers
// ============================================================================

// Type aliases for the generated WIT types (exported interface)
use exports::near::agent::channel as wit_channel;

/// Convert WIT-generated ChannelConfig to our internal type.
fn convert_channel_config(wit: wit_channel::ChannelConfig) -> ChannelConfig {
    ChannelConfig {
        display_name: wit.display_name,
        http_endpoints: wit
            .http_endpoints
            .into_iter()
            .map(
                |ep| crate::channels::wasm::schema::HttpEndpointConfigSchema {
                    path: ep.path,
                    methods: ep.methods,
                    require_secret: ep.require_secret,
                },
            )
            .collect(),
        poll: wit
            .poll
            .map(|p| crate::channels::wasm::schema::PollConfigSchema {
                interval_ms: p.interval_ms,
                enabled: p.enabled,
            }),
    }
}

/// Convert WIT-generated OutgoingHttpResponse to our HttpResponse type.
fn convert_http_response(wit: wit_channel::OutgoingHttpResponse) -> HttpResponse {
    let headers = serde_json::from_str(&wit.headers_json).unwrap_or_default();
    HttpResponse {
        status: wit.status,
        headers,
        body: wit.body,
    }
}

/// Convert a StatusUpdate + metadata into the WIT StatusUpdate type.
fn truncate_status_text(input: &str, max_chars: usize) -> String {
    let mut iter = input.chars();
    let truncated: String = iter.by_ref().take(max_chars).collect();
    if iter.next().is_some() {
        format!("{}...", truncated)
    } else {
        truncated
    }
}

fn status_to_wit(status: &StatusUpdate, metadata: &serde_json::Value) -> wit_channel::StatusUpdate {
    let metadata_json = serde_json::to_string(metadata).unwrap_or_default();

    match status {
        StatusUpdate::Thinking(msg) => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::Thinking,
            message: msg.clone(),
            metadata_json,
        },
        StatusUpdate::ToolStarted { name } => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::ToolStarted,
            message: format!("Tool started: {}", name),
            metadata_json,
        },
        StatusUpdate::ToolCompleted { name, success } => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::ToolCompleted,
            message: format!(
                "Tool completed: {} ({})",
                name,
                if *success { "ok" } else { "failed" }
            ),
            metadata_json,
        },
        StatusUpdate::ToolResult { name, preview } => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::ToolResult,
            message: format!(
                "Tool result: {}\n{}",
                name,
                truncate_status_text(preview, 280)
            ),
            metadata_json,
        },
        StatusUpdate::StreamChunk(chunk) => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::Thinking,
            message: chunk.clone(),
            metadata_json,
        },
        StatusUpdate::Status(msg) => {
            // Map well-known status strings to WIT types (case-insensitive
            // to stay consistent with is_terminal_text_status and the
            // Telegram-side classify_status_update).
            let trimmed = msg.trim();
            let status_type = if trimmed.eq_ignore_ascii_case("done") {
                wit_channel::StatusType::Done
            } else if trimmed.eq_ignore_ascii_case("interrupted") {
                wit_channel::StatusType::Interrupted
            } else {
                wit_channel::StatusType::Status
            };
            wit_channel::StatusUpdate {
                status: status_type,
                message: msg.clone(),
                metadata_json,
            }
        }
        StatusUpdate::ApprovalNeeded {
            request_id,
            tool_name,
            description,
            ..
        } => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::ApprovalNeeded,
            message: format!(
                "Approval needed for tool '{}'. {}\nRequest ID: {}\nReply with: yes (or /approve), no (or /deny), or always (or /always).",
                tool_name, description, request_id
            ),
            metadata_json,
        },
        StatusUpdate::JobStarted {
            job_id,
            title,
            browse_url,
        } => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::JobStarted,
            message: format!("Job started: {} ({})\n{}", title, job_id, browse_url),
            metadata_json,
        },
        StatusUpdate::AuthRequired {
            extension_name,
            instructions,
            auth_url,
            setup_url,
        } => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::AuthRequired,
            message: {
                let mut lines = vec![format!("Authentication required for {}.", extension_name)];
                if let Some(text) = instructions
                    && !text.trim().is_empty()
                {
                    lines.push(text.trim().to_string());
                }
                if let Some(url) = auth_url {
                    lines.push(format!("Auth URL: {}", url));
                }
                if let Some(url) = setup_url {
                    lines.push(format!("Setup URL: {}", url));
                }
                lines.join("\n")
            },
            metadata_json,
        },
        StatusUpdate::AuthCompleted {
            extension_name,
            success,
            message,
        } => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::AuthCompleted,
            message: format!(
                "Authentication {} for {}. {}",
                if *success { "completed" } else { "failed" },
                extension_name,
                message
            ),
            metadata_json,
        },
    }
}

/// Clone a WIT StatusUpdate (the generated type doesn't derive Clone).
fn clone_wit_status_update(update: &wit_channel::StatusUpdate) -> wit_channel::StatusUpdate {
    wit_channel::StatusUpdate {
        status: match update.status {
            wit_channel::StatusType::Thinking => wit_channel::StatusType::Thinking,
            wit_channel::StatusType::Done => wit_channel::StatusType::Done,
            wit_channel::StatusType::Interrupted => wit_channel::StatusType::Interrupted,
            wit_channel::StatusType::ToolStarted => wit_channel::StatusType::ToolStarted,
            wit_channel::StatusType::ToolCompleted => wit_channel::StatusType::ToolCompleted,
            wit_channel::StatusType::ToolResult => wit_channel::StatusType::ToolResult,
            wit_channel::StatusType::ApprovalNeeded => wit_channel::StatusType::ApprovalNeeded,
            wit_channel::StatusType::Status => wit_channel::StatusType::Status,
            wit_channel::StatusType::JobStarted => wit_channel::StatusType::JobStarted,
            wit_channel::StatusType::AuthRequired => wit_channel::StatusType::AuthRequired,
            wit_channel::StatusType::AuthCompleted => wit_channel::StatusType::AuthCompleted,
        },
        message: update.message.clone(),
        metadata_json: update.metadata_json.clone(),
    }
}

/// HTTP response from a WASM channel callback.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    /// HTTP status code.
    pub status: u16,
    /// Response headers.
    pub headers: HashMap<String, String>,
    /// Response body.
    pub body: Vec<u8>,
}

impl HttpResponse {
    /// Create an OK response.
    pub fn ok() -> Self {
        Self {
            status: 200,
            headers: HashMap::new(),
            body: Vec::new(),
        }
    }

    /// Create a JSON response.
    pub fn json(value: serde_json::Value) -> Self {
        let body = serde_json::to_vec(&value).unwrap_or_default();
        let mut headers = HashMap::new();
        headers.insert("Content-Type".to_string(), "application/json".to_string());
        Self {
            status: 200,
            headers,
            body,
        }
    }

    /// Create an error response.
    pub fn error(status: u16, message: &str) -> Self {
        Self {
            status,
            headers: HashMap::new(),
            body: message.as_bytes().to_vec(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::channels::Channel;
    use crate::channels::wasm::capabilities::ChannelCapabilities;
    use crate::channels::wasm::runtime::{
        PreparedChannelModule, WasmChannelRuntime, WasmChannelRuntimeConfig,
    };
    use crate::channels::wasm::wrapper::{HttpResponse, WasmChannel};
    use crate::pairing::PairingStore;
    use crate::tools::wasm::ResourceLimits;

    fn create_test_channel() -> WasmChannel {
        let config = WasmChannelRuntimeConfig::for_testing();
        let runtime = Arc::new(WasmChannelRuntime::new(config).unwrap());

        let prepared = Arc::new(PreparedChannelModule {
            name: "test".to_string(),
            description: "Test channel".to_string(),
            component: None,
            limits: ResourceLimits::default(),
        });

        let capabilities = ChannelCapabilities::for_channel("test").with_path("/webhook/test");

        WasmChannel::new(
            runtime,
            prepared,
            capabilities,
            "{}".to_string(),
            Arc::new(PairingStore::new()),
        )
    }

    #[test]
    fn test_channel_name() {
        let channel = create_test_channel();
        assert_eq!(channel.name(), "test");
    }

    #[test]
    fn test_http_response_ok() {
        let response = HttpResponse::ok();
        assert_eq!(response.status, 200);
        assert!(response.body.is_empty());
    }

    #[test]
    fn test_http_response_json() {
        let response = HttpResponse::json(serde_json::json!({"key": "value"}));
        assert_eq!(response.status, 200);
        assert_eq!(
            response.headers.get("Content-Type"),
            Some(&"application/json".to_string())
        );
    }

    #[test]
    fn test_http_response_error() {
        let response = HttpResponse::error(400, "Bad request");
        assert_eq!(response.status, 400);
        assert_eq!(response.body, b"Bad request");
    }

    #[tokio::test]
    async fn test_channel_start_and_shutdown() {
        let channel = create_test_channel();

        // Start should succeed
        let stream = channel.start().await;
        assert!(stream.is_ok());

        // Health check should pass
        assert!(channel.health_check().await.is_ok());

        // Shutdown should succeed
        assert!(channel.shutdown().await.is_ok());

        // Health check should fail after shutdown
        assert!(channel.health_check().await.is_err());
    }

    #[tokio::test]
    async fn test_execute_poll_no_wasm_returns_empty() {
        // When there's no WASM module (None component), execute_poll
        // should return an empty vector of messages
        let config = WasmChannelRuntimeConfig::for_testing();
        let runtime = Arc::new(WasmChannelRuntime::new(config).unwrap());

        let prepared = Arc::new(PreparedChannelModule {
            name: "poll-test".to_string(),
            description: "Test channel".to_string(),
            component: None, // No WASM module
            limits: ResourceLimits::default(),
        });

        let capabilities = ChannelCapabilities::for_channel("poll-test").with_polling(1000);
        let credentials = Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
        let timeout = std::time::Duration::from_secs(5);

        let workspace_store = Arc::new(crate::channels::wasm::host::ChannelWorkspaceStore::new());

        let result = WasmChannel::execute_poll(
            "poll-test",
            &runtime,
            &prepared,
            &capabilities,
            &credentials,
            Arc::new(PairingStore::new()),
            timeout,
            &workspace_store,
        )
        .await;

        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_dispatch_emitted_messages_sends_to_channel() {
        use crate::channels::wasm::host::EmittedMessage;

        let (tx, mut rx) = tokio::sync::mpsc::channel(10);
        let message_tx = Arc::new(tokio::sync::RwLock::new(Some(tx)));

        let rate_limiter = Arc::new(tokio::sync::RwLock::new(
            crate::channels::wasm::host::ChannelEmitRateLimiter::new(
                crate::channels::wasm::capabilities::EmitRateLimitConfig::default(),
            ),
        ));

        let messages = vec![
            EmittedMessage::new("user1", "Hello from polling!"),
            EmittedMessage::new("user2", "Another message"),
        ];

        let result = WasmChannel::dispatch_emitted_messages(
            "test-channel",
            messages,
            &message_tx,
            &rate_limiter,
        )
        .await;

        assert!(result.is_ok());

        // Verify messages were sent
        let msg1 = rx.try_recv().expect("Should receive first message");
        assert_eq!(msg1.user_id, "user1");
        assert_eq!(msg1.content, "Hello from polling!");

        let msg2 = rx.try_recv().expect("Should receive second message");
        assert_eq!(msg2.user_id, "user2");
        assert_eq!(msg2.content, "Another message");

        // No more messages
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_dispatch_emitted_messages_no_sender_returns_ok() {
        use crate::channels::wasm::host::EmittedMessage;

        // No sender available (channel not started)
        let message_tx = Arc::new(tokio::sync::RwLock::new(None));
        let rate_limiter = Arc::new(tokio::sync::RwLock::new(
            crate::channels::wasm::host::ChannelEmitRateLimiter::new(
                crate::channels::wasm::capabilities::EmitRateLimitConfig::default(),
            ),
        ));

        let messages = vec![EmittedMessage::new("user1", "Hello!")];

        // Should return Ok even without a sender (logs warning but doesn't fail)
        let result = WasmChannel::dispatch_emitted_messages(
            "test-channel",
            messages,
            &message_tx,
            &rate_limiter,
        )
        .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_channel_with_polling_stores_shutdown_sender() {
        // Create a channel with polling capabilities
        let config = WasmChannelRuntimeConfig::for_testing();
        let runtime = Arc::new(WasmChannelRuntime::new(config).unwrap());

        let prepared = Arc::new(PreparedChannelModule {
            name: "poll-channel".to_string(),
            description: "Polling test channel".to_string(),
            component: None,
            limits: ResourceLimits::default(),
        });

        // Enable polling with a 1 second minimum interval
        let capabilities = ChannelCapabilities::for_channel("poll-channel")
            .with_path("/webhook/poll")
            .with_polling(1000);

        let channel = WasmChannel::new(
            runtime,
            prepared,
            capabilities,
            "{}".to_string(),
            Arc::new(PairingStore::new()),
        );

        // Start the channel
        let _stream = channel.start().await.expect("Channel should start");

        // Verify poll_shutdown_tx is set (polling was started)
        // Note: For testing channels without WASM, on_start returns no poll config,
        // so polling won't actually be started. This verifies the basic lifecycle.
        assert!(channel.health_check().await.is_ok());

        // Shutdown should clean up properly
        channel.shutdown().await.expect("Shutdown should succeed");
        assert!(channel.health_check().await.is_err());
    }

    #[tokio::test]
    async fn test_call_on_poll_no_wasm_succeeds() {
        // Verify call_on_poll returns Ok when there's no WASM module
        let channel = create_test_channel();

        // Start the channel first to set up message_tx
        let _stream = channel.start().await.expect("Channel should start");

        // call_on_poll should succeed (no-op for no WASM)
        let result = channel.call_on_poll().await;
        assert!(result.is_ok());

        channel.shutdown().await.expect("Shutdown should succeed");
    }

    #[tokio::test]
    async fn test_typing_task_starts_on_thinking() {
        let channel = create_test_channel();
        let _stream = channel.start().await.expect("Channel should start");

        let metadata = serde_json::json!({"chat_id": 123});

        // Sending Thinking should succeed (no-op for no WASM)
        let result = channel
            .send_status(
                crate::channels::StatusUpdate::Thinking("Processing...".into()),
                &metadata,
            )
            .await;
        assert!(result.is_ok());

        // A typing task should have been spawned
        assert!(channel.typing_task.read().await.is_some());

        // Shutdown should cancel the typing task
        channel.shutdown().await.expect("Shutdown should succeed");
        assert!(channel.typing_task.read().await.is_none());
    }

    #[tokio::test]
    async fn test_typing_task_cancelled_on_done() {
        let channel = create_test_channel();
        let _stream = channel.start().await.expect("Channel should start");

        let metadata = serde_json::json!({"chat_id": 123});

        // Start typing
        let _ = channel
            .send_status(
                crate::channels::StatusUpdate::Thinking("Processing...".into()),
                &metadata,
            )
            .await;
        assert!(channel.typing_task.read().await.is_some());

        // Send Done status
        let _ = channel
            .send_status(
                crate::channels::StatusUpdate::Status("Done".into()),
                &metadata,
            )
            .await;

        // Typing task should be cancelled
        assert!(channel.typing_task.read().await.is_none());

        channel.shutdown().await.expect("Shutdown should succeed");
    }

    #[tokio::test]
    async fn test_typing_task_persists_on_tool_started() {
        let channel = create_test_channel();
        let _stream = channel.start().await.expect("Channel should start");

        let metadata = serde_json::json!({"chat_id": 123});

        // Start typing
        let _ = channel
            .send_status(
                crate::channels::StatusUpdate::Thinking("Processing...".into()),
                &metadata,
            )
            .await;
        assert!(channel.typing_task.read().await.is_some());

        // Intermediate tool status should not cancel typing
        let _ = channel
            .send_status(
                crate::channels::StatusUpdate::ToolStarted {
                    name: "http_request".into(),
                },
                &metadata,
            )
            .await;

        assert!(channel.typing_task.read().await.is_some());

        channel.shutdown().await.expect("Shutdown should succeed");
    }

    #[tokio::test]
    async fn test_typing_task_cancelled_on_approval_needed() {
        let channel = create_test_channel();
        let _stream = channel.start().await.expect("Channel should start");

        let metadata = serde_json::json!({"chat_id": 123});

        // Start typing
        let _ = channel
            .send_status(
                crate::channels::StatusUpdate::Thinking("Processing...".into()),
                &metadata,
            )
            .await;
        assert!(channel.typing_task.read().await.is_some());

        // Approval-needed should stop typing while waiting for user action
        let _ = channel
            .send_status(
                crate::channels::StatusUpdate::ApprovalNeeded {
                    request_id: "req-1".into(),
                    tool_name: "http_request".into(),
                    description: "Fetch weather".into(),
                    parameters: serde_json::json!({"url": "https://wttr.in"}),
                },
                &metadata,
            )
            .await;

        assert!(channel.typing_task.read().await.is_none());

        channel.shutdown().await.expect("Shutdown should succeed");
    }

    #[tokio::test]
    async fn test_typing_task_cancelled_on_awaiting_approval_status() {
        let channel = create_test_channel();
        let _stream = channel.start().await.expect("Channel should start");

        let metadata = serde_json::json!({"chat_id": 123});

        // Start typing
        let _ = channel
            .send_status(
                crate::channels::StatusUpdate::Thinking("Processing...".into()),
                &metadata,
            )
            .await;
        assert!(channel.typing_task.read().await.is_some());

        // Legacy terminal status string should also cancel typing
        let _ = channel
            .send_status(
                crate::channels::StatusUpdate::Status("Awaiting approval".into()),
                &metadata,
            )
            .await;

        assert!(channel.typing_task.read().await.is_none());

        channel.shutdown().await.expect("Shutdown should succeed");
    }

    #[tokio::test]
    async fn test_typing_task_replaced_on_new_thinking() {
        let channel = create_test_channel();
        let _stream = channel.start().await.expect("Channel should start");

        let metadata = serde_json::json!({"chat_id": 123});

        // Start typing
        let _ = channel
            .send_status(
                crate::channels::StatusUpdate::Thinking("First...".into()),
                &metadata,
            )
            .await;

        // Get handle of first task
        let first_handle = {
            let guard = channel.typing_task.read().await;
            guard.as_ref().map(|h| h.id())
        };
        assert!(first_handle.is_some());

        // Start typing again (should replace the previous task)
        let _ = channel
            .send_status(
                crate::channels::StatusUpdate::Thinking("Second...".into()),
                &metadata,
            )
            .await;

        // Should still have a typing task, but it's a new one
        let second_handle = {
            let guard = channel.typing_task.read().await;
            guard.as_ref().map(|h| h.id())
        };
        assert!(second_handle.is_some());
        // The task IDs should differ (old one was aborted, new one spawned)
        assert_ne!(first_handle, second_handle);

        channel.shutdown().await.expect("Shutdown should succeed");
    }

    #[tokio::test]
    async fn test_respond_cancels_typing_task() {
        use crate::channels::IncomingMessage;

        let channel = create_test_channel();
        let _stream = channel.start().await.expect("Channel should start");

        let metadata = serde_json::json!({"chat_id": 123});

        // Start typing
        let _ = channel
            .send_status(
                crate::channels::StatusUpdate::Thinking("Processing...".into()),
                &metadata,
            )
            .await;
        assert!(channel.typing_task.read().await.is_some());

        // Respond should cancel the typing task
        let msg = IncomingMessage::new("test", "user1", "hello").with_metadata(metadata);
        let _ = channel
            .respond(&msg, crate::channels::OutgoingResponse::text("response"))
            .await;

        // Typing task should be gone
        assert!(channel.typing_task.read().await.is_none());

        channel.shutdown().await.expect("Shutdown should succeed");
    }

    #[tokio::test]
    async fn test_stream_chunk_is_noop() {
        let channel = create_test_channel();
        let _stream = channel.start().await.expect("Channel should start");

        let metadata = serde_json::json!({"chat_id": 123});

        // StreamChunk should not start a typing task
        let result = channel
            .send_status(
                crate::channels::StatusUpdate::StreamChunk("chunk".into()),
                &metadata,
            )
            .await;
        assert!(result.is_ok());
        assert!(channel.typing_task.read().await.is_none());

        channel.shutdown().await.expect("Shutdown should succeed");
    }

    #[test]
    fn test_status_to_wit_thinking() {
        use super::status_to_wit;

        let metadata = serde_json::json!({"chat_id": 42});
        let wit = status_to_wit(
            &crate::channels::StatusUpdate::Thinking("Processing...".into()),
            &metadata,
        );

        assert!(matches!(
            wit.status,
            super::wit_channel::StatusType::Thinking
        ));
        assert_eq!(wit.message, "Processing...");
        assert!(wit.metadata_json.contains("42"));
    }

    #[test]
    fn test_status_to_wit_done() {
        use super::status_to_wit;

        let metadata = serde_json::json!(null);
        let wit = status_to_wit(
            &crate::channels::StatusUpdate::Status("Done".into()),
            &metadata,
        );

        assert!(matches!(wit.status, super::wit_channel::StatusType::Done));
    }

    #[test]
    fn test_status_to_wit_done_case_insensitive() {
        use super::status_to_wit;

        let metadata = serde_json::json!(null);

        // lowercase
        let wit = status_to_wit(
            &crate::channels::StatusUpdate::Status("done".into()),
            &metadata,
        );
        assert!(matches!(wit.status, super::wit_channel::StatusType::Done));

        // with whitespace
        let wit = status_to_wit(
            &crate::channels::StatusUpdate::Status(" Done ".into()),
            &metadata,
        );
        assert!(matches!(wit.status, super::wit_channel::StatusType::Done));
    }

    #[test]
    fn test_status_to_wit_interrupted() {
        use super::status_to_wit;

        let metadata = serde_json::json!(null);
        let wit = status_to_wit(
            &crate::channels::StatusUpdate::Status("Interrupted".into()),
            &metadata,
        );

        assert!(matches!(
            wit.status,
            super::wit_channel::StatusType::Interrupted
        ));
    }

    #[test]
    fn test_status_to_wit_interrupted_case_insensitive() {
        use super::status_to_wit;

        let metadata = serde_json::json!(null);

        // lowercase
        let wit = status_to_wit(
            &crate::channels::StatusUpdate::Status("interrupted".into()),
            &metadata,
        );
        assert!(matches!(
            wit.status,
            super::wit_channel::StatusType::Interrupted
        ));

        // with whitespace
        let wit = status_to_wit(
            &crate::channels::StatusUpdate::Status(" Interrupted ".into()),
            &metadata,
        );
        assert!(matches!(
            wit.status,
            super::wit_channel::StatusType::Interrupted
        ));
    }

    #[test]
    fn test_status_to_wit_generic_status() {
        use super::status_to_wit;

        let metadata = serde_json::json!(null);
        let wit = status_to_wit(
            &crate::channels::StatusUpdate::Status("Awaiting approval".into()),
            &metadata,
        );

        assert!(matches!(wit.status, super::wit_channel::StatusType::Status));
        assert_eq!(wit.message, "Awaiting approval");
    }

    #[test]
    fn test_status_to_wit_auth_required() {
        use super::status_to_wit;

        let metadata = serde_json::json!({"chat_id": 42});
        let wit = status_to_wit(
            &crate::channels::StatusUpdate::AuthRequired {
                extension_name: "weather".to_string(),
                instructions: Some("Paste your token".to_string()),
                auth_url: Some("https://example.com/auth".to_string()),
                setup_url: None,
            },
            &metadata,
        );

        assert!(matches!(
            wit.status,
            super::wit_channel::StatusType::AuthRequired
        ));
        assert!(wit.message.contains("Authentication required for weather"));
        assert!(wit.message.contains("Paste your token"));
    }

    #[test]
    fn test_status_to_wit_tool_started() {
        use super::status_to_wit;

        let metadata = serde_json::json!({"chat_id": 7});
        let wit = status_to_wit(
            &crate::channels::StatusUpdate::ToolStarted {
                name: "http_request".to_string(),
            },
            &metadata,
        );

        assert!(matches!(
            wit.status,
            super::wit_channel::StatusType::ToolStarted
        ));
        assert_eq!(wit.message, "Tool started: http_request");
    }

    #[test]
    fn test_status_to_wit_tool_completed_success() {
        use super::status_to_wit;

        let metadata = serde_json::json!(null);
        let wit = status_to_wit(
            &crate::channels::StatusUpdate::ToolCompleted {
                name: "http_request".to_string(),
                success: true,
            },
            &metadata,
        );

        assert!(matches!(
            wit.status,
            super::wit_channel::StatusType::ToolCompleted
        ));
        assert_eq!(wit.message, "Tool completed: http_request (ok)");
    }

    #[test]
    fn test_status_to_wit_tool_completed_failure() {
        use super::status_to_wit;

        let metadata = serde_json::json!(null);
        let wit = status_to_wit(
            &crate::channels::StatusUpdate::ToolCompleted {
                name: "http_request".to_string(),
                success: false,
            },
            &metadata,
        );

        assert!(matches!(
            wit.status,
            super::wit_channel::StatusType::ToolCompleted
        ));
        assert_eq!(wit.message, "Tool completed: http_request (failed)");
    }

    #[test]
    fn test_status_to_wit_tool_result() {
        use super::status_to_wit;

        let metadata = serde_json::json!(null);
        let wit = status_to_wit(
            &crate::channels::StatusUpdate::ToolResult {
                name: "http_request".to_string(),
                preview: "{".to_string() + "\"temperature\": 22}",
            },
            &metadata,
        );

        assert!(matches!(
            wit.status,
            super::wit_channel::StatusType::ToolResult
        ));
        assert!(wit.message.starts_with("Tool result: http_request\n"));
    }

    #[test]
    fn test_status_to_wit_tool_result_truncates_preview() {
        use super::status_to_wit;

        let metadata = serde_json::json!(null);
        let long_preview = "x".repeat(400);
        let wit = status_to_wit(
            &crate::channels::StatusUpdate::ToolResult {
                name: "big_tool".to_string(),
                preview: long_preview,
            },
            &metadata,
        );

        assert!(matches!(
            wit.status,
            super::wit_channel::StatusType::ToolResult
        ));
        assert!(wit.message.ends_with("..."));
    }

    #[test]
    fn test_status_to_wit_job_started() {
        use super::status_to_wit;

        let metadata = serde_json::json!({"chat_id": 1});
        let wit = status_to_wit(
            &crate::channels::StatusUpdate::JobStarted {
                job_id: "job-1".to_string(),
                title: "Daily sync".to_string(),
                browse_url: "https://example.com/jobs/job-1".to_string(),
            },
            &metadata,
        );

        assert!(matches!(
            wit.status,
            super::wit_channel::StatusType::JobStarted
        ));
        assert!(wit.message.contains("Daily sync"));
        assert!(wit.message.contains("https://example.com/jobs/job-1"));
    }

    #[test]
    fn test_status_to_wit_auth_completed_success() {
        use super::status_to_wit;

        let metadata = serde_json::json!(null);
        let wit = status_to_wit(
            &crate::channels::StatusUpdate::AuthCompleted {
                extension_name: "weather".to_string(),
                success: true,
                message: "Token saved".to_string(),
            },
            &metadata,
        );

        assert!(matches!(
            wit.status,
            super::wit_channel::StatusType::AuthCompleted
        ));
        assert!(wit.message.contains("Authentication completed"));
        assert!(wit.message.contains("Token saved"));
    }

    #[test]
    fn test_status_to_wit_auth_completed_failure() {
        use super::status_to_wit;

        let metadata = serde_json::json!(null);
        let wit = status_to_wit(
            &crate::channels::StatusUpdate::AuthCompleted {
                extension_name: "weather".to_string(),
                success: false,
                message: "Invalid token".to_string(),
            },
            &metadata,
        );

        assert!(matches!(
            wit.status,
            super::wit_channel::StatusType::AuthCompleted
        ));
        assert!(wit.message.contains("Authentication failed"));
        assert!(wit.message.contains("Invalid token"));
    }

    #[test]
    fn test_status_to_wit_approval_needed() {
        use super::status_to_wit;

        let metadata = serde_json::json!({"chat_id": 42});
        let wit = status_to_wit(
            &crate::channels::StatusUpdate::ApprovalNeeded {
                request_id: "req-123".to_string(),
                tool_name: "http_request".to_string(),
                description: "Fetch weather data".to_string(),
                parameters: serde_json::json!({"url": "https://api.weather.test"}),
            },
            &metadata,
        );

        assert!(matches!(
            wit.status,
            super::wit_channel::StatusType::ApprovalNeeded
        ));
        assert!(wit.message.contains("http_request"));
        assert!(wit.message.contains("/approve"));
    }

    #[test]
    fn test_approval_prompt_roundtrip_submission_aliases() {
        use super::status_to_wit;
        use crate::agent::submission::{Submission, SubmissionParser};

        let metadata = serde_json::json!({"chat_id": 42});
        let wit = status_to_wit(
            &crate::channels::StatusUpdate::ApprovalNeeded {
                request_id: "req-321".to_string(),
                tool_name: "http_request".to_string(),
                description: "Fetch weather data".to_string(),
                parameters: serde_json::json!({"url": "https://api.weather.test"}),
            },
            &metadata,
        );

        assert!(matches!(
            wit.status,
            super::wit_channel::StatusType::ApprovalNeeded
        ));
        assert!(wit.message.contains("/approve"));
        assert!(wit.message.contains("/deny"));
        assert!(wit.message.contains("/always"));

        let approve = SubmissionParser::parse("/approve");
        assert!(matches!(
            approve,
            Submission::ApprovalResponse {
                approved: true,
                always: false
            }
        ));

        let deny = SubmissionParser::parse("/deny");
        assert!(matches!(
            deny,
            Submission::ApprovalResponse {
                approved: false,
                always: false
            }
        ));

        let always = SubmissionParser::parse("/always");
        assert!(matches!(
            always,
            Submission::ApprovalResponse {
                approved: true,
                always: true
            }
        ));
    }

    #[test]
    fn test_clone_wit_status_update() {
        use super::{clone_wit_status_update, wit_channel};

        let original = wit_channel::StatusUpdate {
            status: wit_channel::StatusType::Thinking,
            message: "hello".to_string(),
            metadata_json: "{\"a\":1}".to_string(),
        };

        let cloned = clone_wit_status_update(&original);
        assert!(matches!(cloned.status, wit_channel::StatusType::Thinking));
        assert_eq!(cloned.message, "hello");
        assert_eq!(cloned.metadata_json, "{\"a\":1}");
    }

    #[test]
    fn test_clone_wit_status_update_approval_needed() {
        use super::{clone_wit_status_update, wit_channel};

        let original = wit_channel::StatusUpdate {
            status: wit_channel::StatusType::ApprovalNeeded,
            message: "approval needed".to_string(),
            metadata_json: "{\"chat_id\":42}".to_string(),
        };

        let cloned = clone_wit_status_update(&original);
        assert!(matches!(
            cloned.status,
            wit_channel::StatusType::ApprovalNeeded
        ));
        assert_eq!(cloned.message, "approval needed");
        assert_eq!(cloned.metadata_json, "{\"chat_id\":42}");
    }

    #[test]
    fn test_clone_wit_status_update_auth_completed() {
        use super::{clone_wit_status_update, wit_channel};

        let original = wit_channel::StatusUpdate {
            status: wit_channel::StatusType::AuthCompleted,
            message: "auth complete".to_string(),
            metadata_json: "{}".to_string(),
        };

        let cloned = clone_wit_status_update(&original);
        assert!(matches!(
            cloned.status,
            wit_channel::StatusType::AuthCompleted
        ));
        assert_eq!(cloned.message, "auth complete");
    }

    #[test]
    fn test_clone_wit_status_update_all_variants() {
        use super::{clone_wit_status_update, wit_channel};

        let variants = vec![
            wit_channel::StatusType::Thinking,
            wit_channel::StatusType::Done,
            wit_channel::StatusType::Interrupted,
            wit_channel::StatusType::ToolStarted,
            wit_channel::StatusType::ToolCompleted,
            wit_channel::StatusType::ToolResult,
            wit_channel::StatusType::ApprovalNeeded,
            wit_channel::StatusType::Status,
            wit_channel::StatusType::JobStarted,
            wit_channel::StatusType::AuthRequired,
            wit_channel::StatusType::AuthCompleted,
        ];

        for status in variants {
            let original = wit_channel::StatusUpdate {
                status,
                message: "sample".to_string(),
                metadata_json: "{}".to_string(),
            };
            let cloned = clone_wit_status_update(&original);

            assert_eq!(
                std::mem::discriminant(&cloned.status),
                std::mem::discriminant(&original.status)
            );
            assert_eq!(cloned.message, "sample");
            assert_eq!(cloned.metadata_json, "{}");
        }
    }

    #[test]
    fn test_redact_credentials_replaces_values() {
        use super::ChannelStoreData;

        let mut creds = std::collections::HashMap::new();
        creds.insert(
            "TELEGRAM_BOT_TOKEN".to_string(),
            "8218490433:AAEZeUxwqZ5OO3mOCXv7fKvpdhDgsmBBNis".to_string(),
        );
        creds.insert("OTHER_SECRET".to_string(), "s3cret".to_string());

        let store = ChannelStoreData::new(
            1024 * 1024,
            "test",
            ChannelCapabilities::default(),
            creds,
            Arc::new(PairingStore::new()),
        );

        let error = "HTTP request failed: error sending request for url \
            (https://api.telegram.org/bot8218490433:AAEZeUxwqZ5OO3mOCXv7fKvpdhDgsmBBNis/getUpdates)";

        let redacted = store.redact_credentials(error);

        assert!(
            !redacted.contains("8218490433:AAEZeUxwqZ5OO3mOCXv7fKvpdhDgsmBBNis"),
            "credential value should be redacted"
        );
        assert!(
            redacted.contains("[REDACTED:TELEGRAM_BOT_TOKEN]"),
            "redacted text should contain placeholder name"
        );
        assert!(
            !redacted.contains("s3cret"),
            "other credentials should also be redacted"
        );
    }

    #[test]
    fn test_redact_credentials_no_op_without_credentials() {
        use super::ChannelStoreData;

        let store = ChannelStoreData::new(
            1024 * 1024,
            "test",
            ChannelCapabilities::default(),
            std::collections::HashMap::new(),
            Arc::new(PairingStore::new()),
        );

        let input = "some error message";
        assert_eq!(store.redact_credentials(input), input);
    }

    #[test]
    fn test_redact_credentials_skips_empty_values() {
        use super::ChannelStoreData;

        let mut creds = std::collections::HashMap::new();
        creds.insert("EMPTY_TOKEN".to_string(), String::new());

        let store = ChannelStoreData::new(
            1024 * 1024,
            "test",
            ChannelCapabilities::default(),
            creds,
            Arc::new(PairingStore::new()),
        );

        let input = "should not match anything";
        assert_eq!(store.redact_credentials(input), input);
    }

    /// Verify that WASM HTTP host functions work using a dedicated
    /// current-thread runtime inside spawn_blocking.
    #[tokio::test]
    async fn test_dedicated_runtime_inside_spawn_blocking() {
        let result = tokio::task::spawn_blocking(|| {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to build runtime");
            rt.block_on(async { 42 })
        })
        .await
        .expect("spawn_blocking panicked");
        assert_eq!(result, 42);
    }

    /// Verify a real HTTP request works using the dedicated-runtime pattern.
    /// This catches DNS, TLS, and I/O driver issues that trivial tests miss.
    #[tokio::test]
    #[ignore] // requires network
    async fn test_dedicated_runtime_real_http() {
        let result = tokio::task::spawn_blocking(|| {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to build runtime");
            rt.block_on(async {
                let client = reqwest::Client::builder()
                    .connect_timeout(std::time::Duration::from_secs(10))
                    .build()
                    .expect("failed to build client");
                let resp = client
                    .get("https://api.telegram.org/bot000/getMe")
                    .timeout(std::time::Duration::from_secs(10))
                    .send()
                    .await;
                match resp {
                    Ok(r) => r.status().as_u16(),
                    Err(e) if e.is_timeout() => panic!("request timed out: {e}"),
                    Err(e) => panic!("unexpected error: {e}"),
                }
            })
        })
        .await
        .expect("spawn_blocking panicked");
        // 404 because "000" is not a valid bot token
        assert_eq!(result, 404);
    }
}
