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
use futures::{SinkExt, StreamExt};
use tokio::sync::{Mutex, RwLock, mpsc, oneshot};
use tokio_stream::wrappers::ReceiverStream;
use tokio_tungstenite::tungstenite::protocol::Message as WebsocketMessage;
use uuid::Uuid;
use wasmtime::Store;
use wasmtime::component::Linker;
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

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
use crate::secrets::SecretsStore;
use crate::tools::wasm::credential_injector::{
    InjectedCredentials, host_matches_pattern, inject_credential,
};
use crate::tools::wasm::{
    LogLevel, WasmResourceLimiter, reject_private_ip, ssrf_safe_client_builder,
};
use ironclaw_safety::LeakDetector;

#[cfg(test)]
const TEST_HTTP_REWRITE_MAP_ENV: &str = "IRONCLAW_TEST_HTTP_REWRITE_MAP";

const WEBSOCKET_EVENT_QUEUE_RELATIVE_PATH: &str = "state/gateway_event_queue";
const WEBSOCKET_EVENT_PROCESSING_QUEUE_RELATIVE_PATH: &str = "state/gateway_event_queue_processing";
const WEBSOCKET_EVENT_QUEUE_MAX_ITEMS: usize = 100;
#[cfg(test)]
const TELEGRAM_TEST_API_BASE_ENV: &str = "IRONCLAW_TEST_TELEGRAM_API_BASE_URL";

// Generate component model bindings from the WIT file
wasmtime::component::bindgen!({
    path: "wit/channel.wit",
    world: "sandboxed-channel",
    with: {
        // Use our own store data type
    },
});

/// Pre-resolved credential for host-based injection.
///
/// Built before each WASM execution by decrypting secrets from the store.
/// Applied per-request by matching the URL host against `host_patterns`.
/// WASM channels never see the raw secret values.
#[derive(Clone)]
struct ResolvedHostCredential {
    /// Host patterns this credential applies to (e.g., "api.slack.com").
    host_patterns: Vec<String>,
    /// Headers to add to matching requests (e.g., "Authorization: Bearer ...").
    headers: HashMap<String, String>,
    /// Query parameters to add to matching requests.
    query_params: HashMap<String, String>,
    /// Raw secret value for redaction in error messages.
    secret_value: String,
}

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
    /// Pre-resolved credentials for automatic host-based injection.
    /// Applied per-request by matching the URL host against host_patterns.
    host_credentials: Vec<ResolvedHostCredential>,
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
        host_credentials: Vec<ResolvedHostCredential>,
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
            host_credentials,
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
    ///
    /// Scrubs raw, URL-encoded, and Base64-encoded forms of each secret
    /// to prevent exfiltration via encoded representations in error strings.
    fn redact_credentials(&self, text: &str) -> String {
        let mut result = text.to_string();
        for (name, value) in &self.credentials {
            if !value.is_empty() {
                let tag = format!("[REDACTED:{}]", name);
                result = result.replace(value, &tag);
                // Also redact URL-encoded form (covers secrets in query strings)
                let encoded = urlencoding::encode(value);
                if encoded != *value {
                    result = result.replace(encoded.as_ref(), &tag);
                }
            }
        }
        for cred in &self.host_credentials {
            if !cred.secret_value.is_empty() {
                let tag = "[REDACTED:host_credential]";
                result = result.replace(&cred.secret_value, tag);
                // Also redact URL-encoded form (covers secrets injected as query params)
                let encoded = urlencoding::encode(&cred.secret_value);
                if encoded.as_ref() != cred.secret_value {
                    result = result.replace(encoded.as_ref(), tag);
                }
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

            // Append query parameters to URL
            if !cred.query_params.is_empty() {
                if let Ok(mut parsed_url) = url::Url::parse(url) {
                    for (name, value) in &cred.query_params {
                        parsed_url.query_pairs_mut().append_pair(name, value);
                    }
                    *url = parsed_url.to_string();
                } else {
                    tracing::warn!(url = %url, "Could not parse URL to inject query parameters; skipping injection");
                }
            }
        }
    }
}

// Implement WasiView to provide WASI context and resource table
impl WasiView for ChannelStoreData {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
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

        let mut headers: std::collections::HashMap<String, String> = raw_headers
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

        let mut logical_url = injected_url;

        // Leak scan runs on WASM-provided values BEFORE host credential injection.
        // This prevents false positives where the host-injected Bearer token
        // (e.g., xoxb- Slack token) triggers the leak detector — WASM never saw
        // the real value, so scanning the pre-injection state is correct.
        let leak_detector = LeakDetector::new();
        let header_vec: Vec<(String, String)> = headers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        leak_detector
            .scan_http_request(&logical_url, &header_vec, body.as_deref())
            .map_err(|e| format!("Potential secret leak blocked: {}", e))?;

        // Inject pre-resolved host credentials (Bearer tokens, API keys, etc.)
        // after the leak scan so host-injected secrets don't trigger false positives.
        if let Some(host) = extract_host_from_url(&logical_url) {
            self.inject_host_credentials(&host, &mut headers, &mut logical_url);
        }

        let (transport_url, allow_private_test_target) = {
            #[cfg(test)]
            {
                let rewritten = rewrite_http_url_for_testing(&logical_url)
                    .or_else(|| rewrite_telegram_api_url_for_testing(&logical_url));
                let is_rewritten = rewritten.is_some();
                let url = rewritten.unwrap_or_else(|| logical_url.clone());
                if url != logical_url {
                    tracing::info!(
                        logical_url = %logical_url,
                        transport_url = %url,
                        "Rewriting outbound HTTP request to test base URL"
                    );
                }
                (url, is_rewritten)
            }
            #[cfg(not(test))]
            {
                (logical_url.clone(), false)
            }
        };

        // Get the max response size from capabilities (default 10MB).
        let max_response_bytes = self
            .host_state
            .capabilities()
            .tool_capabilities
            .http
            .as_ref()
            .map(|h| h.max_response_bytes)
            .unwrap_or(10 * 1024 * 1024);

        // Resolve hostname and reject private/internal IPs to prevent DNS rebinding.
        // Test-only URL rewrites intentionally point at local fake servers.
        if !allow_private_test_target {
            reject_private_ip(&transport_url)?;
        }

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
            let client = ssrf_safe_client_builder()
                .connect_timeout(Duration::from_secs(10))
                .build()
                .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

            let mut request = match method.to_uppercase().as_str() {
                "GET" => client.get(&transport_url),
                "POST" => client.post(&transport_url),
                "PUT" => client.put(&transport_url),
                "DELETE" => client.delete(&transport_url),
                "PATCH" => client.patch(&transport_url),
                "HEAD" => client.head(&transport_url),
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

            // Leak detection on response body (best-effort).
            //
            // Telegram `getUpdates` is special: it is inbound polling data, so
            // user-pasted secrets can legitimately appear in the response body.
            // Those messages are still checked later by the inbound message
            // safety layer before they reach the LLM, so we allow the polling
            // response to continue here to avoid poisoning the offset state.
            if let Ok(body_str) = std::str::from_utf8(&body)
                && !should_skip_response_leak_scan(&logical_url)
            {
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
            attachment_count = msg.attachments.len(),
            "WASM emit_message called"
        );

        let attachments: Vec<crate::channels::wasm::host::Attachment> = msg
            .attachments
            .into_iter()
            .map(|a| {
                // Parse extras-json for well-known fields
                let extras: serde_json::Value = if a.extras_json.is_empty() {
                    serde_json::Value::Null
                } else {
                    serde_json::from_str(&a.extras_json).unwrap_or(serde_json::Value::Null)
                };
                let duration_secs = extras
                    .get("duration_secs")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u32);

                // Merge stored binary data (from store-attachment-data host call)
                let data = self
                    .host_state
                    .remove_attachment_data(&a.id)
                    .unwrap_or_default();

                crate::channels::wasm::host::Attachment {
                    id: a.id,
                    mime_type: a.mime_type,
                    filename: a.filename,
                    size_bytes: a.size_bytes,
                    source_url: a.source_url,
                    storage_key: a.storage_key,
                    extracted_text: a.extracted_text,
                    data,
                    duration_secs,
                }
            })
            .collect();

        let mut emitted = EmittedMessage::new(msg.user_id.clone(), msg.content.clone());
        if let Some(name) = msg.user_name {
            emitted = emitted.with_user_name(name);
        }
        if let Some(tid) = msg.thread_id {
            emitted = emitted.with_thread_id(tid);
        }
        emitted = emitted.with_metadata(msg.metadata_json);
        emitted = emitted.with_attachments(attachments);

        match self.host_state.emit_message(emitted) {
            Ok(()) => {
                tracing::info!("Message emitted to host state successfully");
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to emit message to host state");
            }
        }
    }

    fn store_attachment_data(
        &mut self,
        attachment_id: String,
        data: Vec<u8>,
    ) -> Result<(), String> {
        tracing::debug!(
            attachment_id = %attachment_id,
            size = data.len(),
            "WASM store_attachment_data called"
        );
        self.host_state
            .store_attachment_data(&attachment_id, data)
            .map_err(|e| e.to_string())
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
        let store = self.pairing_store.clone();
        let handle = tokio::runtime::Handle::try_current()
            .map_err(|_| "pairing host callback requires a Tokio runtime".to_string())?;
        if handle.runtime_flavor() != tokio::runtime::RuntimeFlavor::MultiThread {
            return Err("pairing host callback requires a multi-thread Tokio runtime".to_string());
        }
        let result: Result<crate::db::PairingRequestRecord, crate::error::DatabaseError> =
            tokio::task::block_in_place(move || {
                handle.block_on(async move { store.upsert_request(&channel, &id, meta).await })
            });
        match result {
            Ok(req) => Ok(near::agent::channel_host::PairingUpsertResult {
                code: req.code,
                created: req.created,
            }),
            Err(e) => Err(e.to_string()),
        }
    }

    fn pairing_resolve_identity(
        &mut self,
        channel: String,
        external_id: String,
    ) -> Result<Option<String>, String> {
        let store = self.pairing_store.clone();
        let handle = tokio::runtime::Handle::try_current()
            .map_err(|_| "pairing host callback requires a Tokio runtime".to_string())?;
        if handle.runtime_flavor() != tokio::runtime::RuntimeFlavor::MultiThread {
            return Err("pairing host callback requires a multi-thread Tokio runtime".to_string());
        }
        let result: Result<Option<crate::ownership::Identity>, crate::error::DatabaseError> =
            tokio::task::block_in_place(move || {
                handle.block_on(async move { store.resolve_identity(&channel, &external_id).await })
            });
        match result {
            Ok(Some(identity)) => Ok(Some(identity.owner_id.to_string())),
            Ok(None) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    fn pairing_read_allow_from(&mut self, channel: String) -> Result<Vec<String>, String> {
        let store = self.pairing_store.clone();
        let handle = tokio::runtime::Handle::try_current()
            .map_err(|_| "pairing host callback requires a Tokio runtime".to_string())?;
        if handle.runtime_flavor() != tokio::runtime::RuntimeFlavor::MultiThread {
            return Err("pairing host callback requires a multi-thread Tokio runtime".to_string());
        }
        let result: Result<Vec<String>, crate::error::DatabaseError> =
            tokio::task::block_in_place(move || {
                handle.block_on(async move { store.read_allow_from(&channel).await })
            });
        result.map_err(|e| e.to_string())
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

    /// Join handle for the active polling task so restarts can wait for the
    /// previous long-poll to exit before starting a replacement.
    poll_task: RwLock<Option<tokio::task::JoinHandle<()>>>,

    /// Websocket runtime shutdown signal sender.
    websocket_shutdown_tx: RwLock<Option<oneshot::Sender<()>>>,

    /// Serializes websocket-triggered poll executions.
    websocket_poll_lock: Arc<Mutex<()>>,

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

    /// Last-seen message metadata (contains chat_id for broadcast routing).
    /// Populated from incoming messages so `broadcast()` knows where to send.
    last_broadcast_metadata: Arc<tokio::sync::RwLock<Option<String>>>,

    /// Settings store for persisting broadcast metadata across restarts.
    settings_store: Option<Arc<dyn crate::db::SettingsStore>>,

    /// Stable owner scope for persistent data and owner-target routing.
    owner_scope_id: String,

    /// Channel-specific actor ID that maps to the instance owner on this channel.
    /// Wrapped in `Arc` so spawned polling/websocket tasks can read the current
    /// value after pairing approval without capturing a stale clone.
    owner_actor_id: Arc<tokio::sync::RwLock<Option<String>>>,

    /// Secrets store for host-based credential injection.
    /// Used to pre-resolve credentials before each WASM callback.
    secrets_store: Option<Arc<dyn SecretsStore + Send + Sync>>,
}

/// Update broadcast metadata in memory and persist to the settings store when
/// it changes. Extracted as a free function so both the `WasmChannel` instance
/// method and the static polling helper share one implementation.
async fn do_update_broadcast_metadata(
    channel_name: &str,
    owner_scope_id: &str,
    metadata: &str,
    last_broadcast_metadata: &tokio::sync::RwLock<Option<String>>,
    settings_store: Option<&Arc<dyn crate::db::SettingsStore>>,
) {
    let mut guard = last_broadcast_metadata.write().await;
    let changed = guard.as_deref() != Some(metadata);
    *guard = Some(metadata.to_string());
    drop(guard);

    if changed && let Some(store) = settings_store {
        let key = format!("channel_broadcast_metadata_{}", channel_name);
        let value = serde_json::Value::String(metadata.to_string());
        if let Err(e) = store.set_setting(owner_scope_id, &key, &value).await {
            tracing::warn!(
                channel = %channel_name,
                "Failed to persist broadcast metadata: {}",
                e
            );
        }
    }
}

fn resolve_message_scope(
    owner_scope_id: &str,
    owner_actor_id: Option<&str>,
    sender_id: &str,
) -> (String, bool) {
    if owner_actor_id.is_some_and(|owner_actor_id| owner_actor_id == sender_id) {
        (owner_scope_id.to_string(), true)
    } else {
        (sender_id.to_string(), false)
    }
}

async fn resolve_message_scope_with_pairing(
    channel_name: &str,
    owner_scope_id: &str,
    owner_actor_id: Option<&str>,
    sender_id: &str,
    pairing_store: &PairingStore,
) -> (String, bool) {
    if owner_actor_id.is_some_and(|owner_actor_id| owner_actor_id == sender_id) {
        return (owner_scope_id.to_string(), true);
    }

    match pairing_store
        .resolve_identity(channel_name, sender_id)
        .await
    {
        Ok(Some(identity)) => (identity.owner_id.to_string(), false),
        Ok(None) => (sender_id.to_string(), false),
        Err(error) => {
            tracing::warn!(
                channel = %channel_name,
                sender_id = %sender_id,
                "Failed to resolve paired sender identity: {}",
                error
            );
            resolve_message_scope(owner_scope_id, owner_actor_id, sender_id)
        }
    }
}

fn uses_owner_broadcast_target(user_id: &str, owner_scope_id: &str) -> bool {
    user_id == owner_scope_id
}

fn should_update_owner_broadcast_metadata(
    user_id: &str,
    sender_id: &str,
    owner_scope_id: &str,
    owner_actor_id: Option<&str>,
) -> bool {
    owner_actor_id.map_or(user_id == owner_scope_id, |owner_actor_id| {
        sender_id == owner_actor_id
    })
}

fn missing_routing_target_error(name: &str, reason: String) -> ChannelError {
    ChannelError::MissingRoutingTarget {
        name: name.to_string(),
        reason,
    }
}

fn resolve_owner_broadcast_target(
    channel_name: &str,
    metadata: &str,
) -> Result<String, ChannelError> {
    let metadata: serde_json::Value = serde_json::from_str(metadata).map_err(|e| {
        missing_routing_target_error(
            channel_name,
            format!("Invalid stored owner routing metadata: {e}"),
        )
    })?;

    crate::channels::routing_target_from_metadata(&metadata).ok_or_else(|| {
        missing_routing_target_error(
            channel_name,
            format!(
                "Stored owner routing metadata for channel '{}' is missing a delivery target.",
                channel_name
            ),
        )
    })
}

fn apply_emitted_metadata(mut msg: IncomingMessage, metadata_json: &str) -> IncomingMessage {
    if let Ok(metadata) = serde_json::from_str(metadata_json) {
        msg = msg.with_metadata(metadata);
        if msg.conversation_scope().is_none()
            && let Some(scope_id) = crate::channels::routing_target_from_metadata(&msg.metadata)
        {
            msg = msg.with_conversation_scope(scope_id);
        }
    }
    msg
}

impl WasmChannel {
    /// Create a new WASM channel.
    pub fn new(
        runtime: Arc<WasmChannelRuntime>,
        prepared: Arc<PreparedChannelModule>,
        capabilities: ChannelCapabilities,
        owner_scope_id: impl Into<String>,
        config_json: String,
        pairing_store: Arc<PairingStore>,
        settings_store: Option<Arc<dyn crate::db::SettingsStore>>,
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
            poll_task: RwLock::new(None),
            websocket_shutdown_tx: RwLock::new(None),
            websocket_poll_lock: Arc::new(Mutex::new(())),
            endpoints: RwLock::new(Vec::new()),
            credentials: Arc::new(RwLock::new(HashMap::new())),
            typing_task: RwLock::new(None),
            pairing_store,
            workspace_store: Arc::new(ChannelWorkspaceStore::new()),
            last_broadcast_metadata: Arc::new(tokio::sync::RwLock::new(None)),
            settings_store,
            owner_scope_id: owner_scope_id.into(),
            owner_actor_id: Arc::new(tokio::sync::RwLock::new(None)),
            secrets_store: None,
        }
    }

    /// Set the secrets store for host-based credential injection.
    ///
    /// When set, credentials declared in the channel's capabilities are
    /// automatically decrypted and injected into HTTP requests based on
    /// the target host (e.g., Bearer token for api.slack.com).
    pub fn with_secrets_store(mut self, store: Arc<dyn SecretsStore + Send + Sync>) -> Self {
        self.secrets_store = Some(store);
        self
    }

    /// Bind this channel to the external actor that maps to the configured owner.
    pub fn with_owner_actor_id(mut self, owner_actor_id: Option<String>) -> Self {
        self.owner_actor_id = Arc::new(tokio::sync::RwLock::new(owner_actor_id));
        self
    }

    /// Ensure the message channel exists, creating it if needed.
    ///
    /// Returns `Some(stream)` if a new channel was created (caller must wire
    /// up a forwarding task), or `None` if it already exists.
    ///
    /// This handles the case where `Channel::start()` failed at boot (e.g.,
    /// missing credentials) — `message_tx` was never set, so polling tasks
    /// can't deliver messages. `refresh_active_channel` calls this to repair
    /// the channel after credentials become available.
    pub async fn ensure_message_channel(&self) -> Option<MessageStream> {
        let mut guard = self.message_tx.write().await;
        let needs_create = guard.is_none() || guard.as_ref().is_some_and(|tx| tx.is_closed());
        if needs_create {
            let (tx, rx) = mpsc::channel(256);
            *guard = Some(tx);
            tracing::debug!(channel = %self.name, "Created new message_tx (channel was not started or receiver was dropped)");
            Some(Box::pin(ReceiverStream::new(rx)))
        } else {
            None
        }
    }

    /// Update the owner actor ID on a running channel after pairing approval.
    pub async fn set_owner_actor_id(&self, owner_actor_id: Option<String>) {
        *self.owner_actor_id.write().await = owner_actor_id;
    }

    #[cfg(test)]
    pub(crate) async fn owner_actor_id_for_test(&self) -> Option<String> {
        self.owner_actor_id.read().await.clone()
    }

    #[cfg(test)]
    pub(crate) async fn get_config(&self) -> std::collections::HashMap<String, serde_json::Value> {
        let guard = self.config_json.read().await;
        serde_json::from_str(&guard).unwrap_or_default()
    }

    /// Attach a message stream for integration tests.
    ///
    /// This primes any startup-persisted workspace state, but tolerates
    /// callback-level startup failures so tests can exercise webhook parsing
    /// and message emission without depending on external network access.
    #[cfg(feature = "integration")]
    #[doc(hidden)]
    pub async fn start_message_stream_for_test(&self) -> Result<MessageStream, WasmChannelError> {
        self.prime_startup_state_for_test().await?;

        let (tx, rx) = mpsc::channel(256);
        *self.message_tx.write().await = Some(tx);
        let (shutdown_tx, _shutdown_rx) = oneshot::channel();
        *self.shutdown_tx.write().await = Some(shutdown_tx);

        Ok(Box::pin(ReceiverStream::new(rx)))
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

    #[cfg(feature = "integration")]
    async fn prime_startup_state_for_test(&self) -> Result<(), WasmChannelError> {
        if self.prepared.component().is_none() {
            return Ok(());
        }

        let (start_result, mut host_state) = self.execute_on_start_with_state().await?;
        self.log_on_start_host_state(&mut host_state);

        match start_result {
            Ok(_) => Ok(()),
            Err(WasmChannelError::CallbackFailed { reason, .. }) => {
                tracing::warn!(
                    channel = %self.name,
                    reason = %reason,
                    "Ignoring startup callback failure in test-only message stream bootstrap"
                );
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    /// Get the channel name.
    pub fn channel_name(&self) -> &str {
        &self.name
    }

    /// Settings key for persisted broadcast metadata.
    fn broadcast_metadata_key(&self) -> String {
        format!("channel_broadcast_metadata_{}", self.name)
    }

    /// Update broadcast metadata in memory and persist if changed (best-effort).
    ///
    /// Compares with the current value to avoid redundant DB writes on every
    /// incoming message (the chat_id rarely changes).
    async fn update_broadcast_metadata(&self, metadata: &str) {
        do_update_broadcast_metadata(
            &self.name,
            &self.owner_scope_id,
            metadata,
            &self.last_broadcast_metadata,
            self.settings_store.as_ref(),
        )
        .await;
    }

    /// Load broadcast metadata from settings store on startup.
    ///
    /// # Legacy migration (remove after ownership model rollout — tracked in #2100)
    ///
    /// If no metadata is found under `self.owner_scope_id`, a second lookup
    /// under `"default"` is attempted for backward compatibility with instances
    /// that stored broadcast metadata before the ownership model migration.
    /// Remove this fallback once all deployments have run the
    /// `migrate_default_owner` bootstrap step and restarted at least once.
    async fn load_broadcast_metadata(&self) {
        if let Some(ref store) = self.settings_store {
            match store
                .get_setting(&self.owner_scope_id, &self.broadcast_metadata_key())
                .await
            {
                Ok(Some(serde_json::Value::String(meta))) => {
                    *self.last_broadcast_metadata.write().await = Some(meta);
                    tracing::debug!(
                        channel = %self.name,
                        "Restored broadcast metadata from settings"
                    );
                }
                Ok(_) => {
                    // LEGACY MIGRATION: remove after ownership model rollout — tracked in #2100
                    if self.owner_scope_id != "default" {
                        match store
                            .get_setting("default", &self.broadcast_metadata_key())
                            .await
                        {
                            Ok(Some(serde_json::Value::String(meta))) => {
                                *self.last_broadcast_metadata.write().await = Some(meta);
                                tracing::debug!(
                                    channel = %self.name,
                                    "Restored legacy owner broadcast metadata from default scope"
                                );
                            }
                            Ok(_) => {}
                            Err(e) => {
                                tracing::warn!(
                                    channel = %self.name,
                                    "Failed to load legacy broadcast metadata: {}",
                                    e
                                );
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        channel = %self.name,
                        "Failed to load broadcast metadata: {}",
                        e
                    );
                }
            }
        }
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
        wasmtime_wasi::p2::add_to_linker_sync(linker).map_err(|e| {
            WasmChannelError::Config(format!("Failed to add WASI functions: {}", e))
        })?;

        // Use the generated add_to_linker function from bindgen for our custom interface
        SandboxedChannel::add_to_linker::<_, wasmtime::component::HasSelf<_>>(
            linker,
            |state: &mut ChannelStoreData| state,
        )
        .map_err(|e| WasmChannelError::Config(format!("Failed to add host functions: {}", e)))?;

        Ok(())
    }

    fn start_websocket_runtime(
        &self,
        config: WebsocketRuntimeConfig,
        shutdown_rx: oneshot::Receiver<()>,
        owner_actor_id: Arc<tokio::sync::RwLock<Option<String>>>,
    ) {
        let channel_name = self.name.clone();
        let runtime = Arc::clone(&self.runtime);
        let prepared = Arc::clone(&self.prepared);
        let capabilities = Self::inject_workspace_reader(&self.capabilities, &self.workspace_store);
        let poll_capabilities = self.capabilities.clone();
        let message_tx = self.message_tx.clone();
        let rate_limiter = self.rate_limiter.clone();
        let credentials = self.credentials.clone();
        let pairing_store = self.pairing_store.clone();
        let callback_timeout = self.runtime.config().callback_timeout;
        let workspace_store = self.workspace_store.clone();
        let last_broadcast_metadata = self.last_broadcast_metadata.clone();
        let settings_store = self.settings_store.clone();
        let owner_scope_id = self.owner_scope_id.clone();
        let websocket_secrets_store = self.secrets_store.clone();
        let websocket_poll_lock = Arc::clone(&self.websocket_poll_lock);

        tokio::spawn(async move {
            let mut shutdown = std::pin::pin!(shutdown_rx);
            let mut reconnect_attempt = 0u32;
            let (outbound_tx, mut outbound_rx) = mpsc::unbounded_channel::<String>();

            tracing::info!(
                channel = %channel_name,
                url = %config.url,
                "Starting websocket runtime"
            );
            let queue_path = websocket_queue_path(&channel_name);
            let processing_queue_path = websocket_processing_queue_path(&channel_name);
            let identify_payload = resolve_websocket_identify_message(
                &config,
                websocket_secrets_store.as_deref(),
                &owner_scope_id,
            )
            .await;
            let mut session_state = WebsocketSessionState::new(identify_payload.as_deref());

            'reconnect: loop {
                let connect_url = session_state.connect_url(&config.url);
                let connect_result = tokio_tungstenite::connect_async(connect_url).await;
                let (stream, _) = match connect_result {
                    Ok(parts) => {
                        reconnect_attempt = 0;
                        tracing::info!(channel = %channel_name, "Websocket runtime connected");
                        parts
                    }
                    Err(error) => {
                        let backoff = websocket_reconnect_backoff(reconnect_attempt);
                        reconnect_attempt = reconnect_attempt.saturating_add(1);
                        tracing::warn!(
                            channel = %channel_name,
                            url = %config.url,
                            error = %error,
                            backoff_secs = backoff.as_secs(),
                            "Websocket runtime connection failed; retrying"
                        );
                        tokio::select! {
                            _ = tokio::time::sleep(backoff) => continue 'reconnect,
                            _ = &mut shutdown => {
                                tracing::info!(channel = %channel_name, "Stopping websocket runtime");
                                break 'reconnect;
                            }
                        }
                    }
                };

                let (mut write, mut read) = stream.split();
                let mut next_heartbeat: Option<std::pin::Pin<Box<tokio::time::Sleep>>> = None;
                session_state.reset_connection();

                loop {
                    tokio::select! {
                        _ = async {
                            if let Some(sleep) = next_heartbeat.as_mut() {
                                sleep.as_mut().await;
                            } else {
                                std::future::pending::<()>().await;
                            }
                        } => {
                            if let Some(payload) = build_websocket_heartbeat_message(session_state.last_sequence.clone())
                                && let Err(error) = write.send(WebsocketMessage::Text(payload.into())).await
                            {
                                tracing::warn!(channel = %channel_name, error = %error, "Websocket heartbeat send failed");
                                break;
                            }

                            next_heartbeat = session_state.heartbeat_interval_ms
                                .map(|interval_ms| Box::pin(tokio::time::sleep(websocket_heartbeat_sleep_duration(interval_ms))));
                        }
                        outbound = outbound_rx.recv() => {
                            if let Some(payload) = outbound
                                && let Err(error) = write.send(WebsocketMessage::Text(payload.into())).await
                            {
                                tracing::warn!(channel = %channel_name, error = %error, "Websocket outbound control send failed");
                                break;
                            }
                        }
                        _ = &mut shutdown => {
                            tracing::info!(channel = %channel_name, "Stopping websocket runtime");
                            break 'reconnect;
                        }
                        message = read.next() => {
                            match message {
                                Some(Ok(WebsocketMessage::Text(text))) => {
                                    log_websocket_diagnostic(&channel_name, &WebsocketMessage::Text(text.clone()));
                                    let text = text.to_string();

                                    let actions = session_state.process_text_frame(
                                        &text,
                                        &channel_name,
                                        identify_payload.as_deref(),
                                        workspace_store.as_ref(),
                                        pairing_store.as_ref(),
                                    );

                                    let mut should_break = false;
                                    let mut should_reconnect = false;
                                    for action in actions {
                                        match action {
                                            WebsocketFrameAction::SetHeartbeat { interval_ms } => {
                                                next_heartbeat = Some(Box::pin(tokio::time::sleep(
                                                    websocket_heartbeat_sleep_duration(interval_ms),
                                                )));
                                            }
                                            WebsocketFrameAction::Send(payload) => {
                                                if let Err(error) = write.send(WebsocketMessage::Text(payload.into())).await {
                                                    tracing::warn!(channel = %channel_name, error = %error, "Websocket send failed");
                                                    should_break = true;
                                                    break;
                                                }
                                            }
                                            WebsocketFrameAction::Enqueue(raw_text) => {
                                                if let Err(error) = workspace_store.append_json_text_queue(
                                                    &queue_path,
                                                    &raw_text,
                                                    WEBSOCKET_EVENT_QUEUE_MAX_ITEMS,
                                                ) {
                                                    tracing::warn!(channel = %channel_name, error = %error, "Failed to enqueue websocket text frame");
                                                    continue;
                                                }

                                                if let Ok(poll_guard) = Arc::clone(&websocket_poll_lock).try_lock_owned() {
                                                    spawn_websocket_poll(
                                                        poll_guard,
                                                        WebsocketPollContext {
                                                            channel_name: channel_name.clone(),
                                                            runtime: Arc::clone(&runtime),
                                                            prepared: Arc::clone(&prepared),
                                                            capabilities: capabilities.clone(),
                                                            poll_capabilities: poll_capabilities.clone(),
                                                            credentials: Arc::clone(&credentials),
                                                            pairing_store: pairing_store.clone(),
                                                            workspace_store: workspace_store.clone(),
                                                            message_tx: message_tx.clone(),
                                                            rate_limiter: Arc::clone(&rate_limiter),
                                                            last_broadcast_metadata: Arc::clone(&last_broadcast_metadata),
                                                            settings_store: settings_store.clone(),
                                                            owner_scope_id: owner_scope_id.clone(),
                                                            owner_actor_id: owner_actor_id.clone(),
                                                            secrets_store: websocket_secrets_store.clone(),
                                                            outbound_tx: outbound_tx.clone(),
                                                            queue_path: queue_path.clone(),
                                                            processing_queue_path: processing_queue_path.clone(),
                                                            callback_timeout,
                                                        },
                                                    );
                                                }
                                            }
                                            WebsocketFrameAction::InvalidateAndReconnect => {
                                                should_reconnect = true;
                                                break;
                                            }
                                        }
                                    }
                                    if should_reconnect {
                                        break;
                                    }
                                    if should_break {
                                        break;
                                    }
                                }
                                Some(Ok(other)) => {
                                    log_websocket_diagnostic(&channel_name, &other);
                                }
                                Some(Err(error)) => {
                                    tracing::warn!(
                                        channel = %channel_name,
                                        error = %error,
                                        "Websocket runtime receive error"
                                    );
                                    break;
                                }
                                None => {
                                    tracing::info!(channel = %channel_name, "Websocket runtime closed by peer");
                                    break;
                                }
                            }
                        }
                    }
                }

                let backoff = websocket_reconnect_backoff(reconnect_attempt);
                reconnect_attempt = reconnect_attempt.saturating_add(1);
                tracing::info!(
                    channel = %channel_name,
                    backoff_secs = backoff.as_secs(),
                    "Websocket runtime disconnected; reconnect scheduled"
                );
                tokio::select! {
                    _ = tokio::time::sleep(backoff) => {}
                    _ = &mut shutdown => {
                        tracing::info!(channel = %channel_name, "Stopping websocket runtime");
                        break 'reconnect;
                    }
                }
            }
        });
    }

    /// Create a fresh store configured for WASM execution.
    fn create_store(
        runtime: &WasmChannelRuntime,
        prepared: &PreparedChannelModule,
        capabilities: &ChannelCapabilities,
        credentials: HashMap<String, String>,
        host_credentials: Vec<ResolvedHostCredential>,
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
            host_credentials,
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
        let instance = SandboxedChannel::instantiate(store, &component, &linker).map_err(|e| {
            let msg = e.to_string();
            if msg.contains("near:agent") || msg.contains("import") {
                WasmChannelError::Instantiation(format!(
                    "{msg}. This may indicate a WIT version mismatch — \
                         the channel was compiled against a different WIT than the host supports \
                         (host WIT: {}). Rebuild the channel against the current WIT.",
                    crate::tools::wasm::WIT_CHANNEL_VERSION
                ))
            } else {
                WasmChannelError::Instantiation(msg)
            }
        })?;

        Ok(instance)
    }

    /// Map WASM execution errors to our error types.
    fn map_wasm_error(
        e: impl Into<anyhow::Error>,
        name: &str,
        fuel_limit: u64,
    ) -> WasmChannelError {
        let error_str = e.into().to_string();
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

    fn log_on_start_host_state(&self, host_state: &mut ChannelHostState) {
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
    }

    async fn execute_on_start_with_state(
        &self,
    ) -> Result<(Result<ChannelConfig, WasmChannelError>, ChannelHostState), WasmChannelError> {
        let runtime = Arc::clone(&self.runtime);
        let prepared = Arc::clone(&self.prepared);
        let capabilities = Self::inject_workspace_reader(&self.capabilities, &self.workspace_store);
        let config_json = self.config_json.read().await.clone();
        let timeout = self.runtime.config().callback_timeout;
        let channel_name = self.name.clone();
        let credentials = self.get_credentials().await;
        let host_credentials = resolve_channel_host_credentials(
            &self.capabilities,
            self.secrets_store.as_deref(),
            &self.owner_scope_id,
        )
        .await;
        let pairing_store = self.pairing_store.clone();
        let workspace_store = self.workspace_store.clone();

        tokio::time::timeout(timeout, async move {
            tokio::task::spawn_blocking(move || {
                let mut store = Self::create_store(
                    &runtime,
                    &prepared,
                    &capabilities,
                    credentials,
                    host_credentials,
                    pairing_store,
                )?;
                let instance = Self::instantiate_component(&runtime, &prepared, &mut store)?;

                let channel_iface = instance.near_agent_channel();
                let config_result = channel_iface
                    .call_on_start(&mut store, &config_json)
                    .map_err(|e| Self::map_wasm_error(e, &prepared.name, prepared.limits.fuel))
                    .and_then(|wasm_result| match wasm_result {
                        Ok(wit_config) => Ok(convert_channel_config(wit_config)),
                        Err(err_msg) => Err(WasmChannelError::CallbackFailed {
                            name: prepared.name.clone(),
                            reason: err_msg,
                        }),
                    });

                let mut host_state =
                    Self::extract_host_state(&mut store, &prepared.name, &capabilities);
                let pending_writes = host_state.take_pending_writes();
                workspace_store.commit_writes(&pending_writes);

                Ok::<_, WasmChannelError>((config_result, host_state))
            })
            .await
            .map_err(|e| WasmChannelError::ExecutionPanicked {
                name: channel_name.clone(),
                reason: e.to_string(),
            })?
        })
        .await
        .map_err(|_| WasmChannelError::Timeout {
            name: self.name.clone(),
            callback: "on_start".to_string(),
        })?
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

        let (config_result, mut host_state) = self.execute_on_start_with_state().await?;
        self.log_on_start_host_state(&mut host_state);

        let config = config_result?;
        tracing::info!(
            channel = %self.name,
            display_name = %config.display_name,
            endpoints = config.http_endpoints.len(),
            "WASM channel on_start completed"
        );
        Ok(config)
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
        let host_credentials = resolve_channel_host_credentials(
            &self.capabilities,
            self.secrets_store.as_deref(),
            &self.owner_scope_id,
        )
        .await;
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
                    host_credentials,
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
        let host_credentials = resolve_channel_host_credentials(
            &self.capabilities,
            self.secrets_store.as_deref(),
            &self.owner_scope_id,
        )
        .await;
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
                    host_credentials,
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
                let _ = drain_guest_logs(&channel_name, "on_poll", &mut host_state);

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
        attachments: &[String],
    ) -> Result<(), WasmChannelError> {
        tracing::info!(
            channel = %self.name,
            message_id = %message_id,
            content_len = content.len(),
            thread_id = ?thread_id,
            attachment_count = attachments.len(),
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
        let capabilities = Self::inject_workspace_reader(&self.capabilities, &self.workspace_store);
        let timeout = self.runtime.config().callback_timeout;
        let channel_name = self.name.clone();
        let credentials = self.get_credentials().await;
        let host_credentials = resolve_channel_host_credentials(
            &self.capabilities,
            self.secrets_store.as_deref(),
            &self.owner_scope_id,
        )
        .await;
        let pairing_store = self.pairing_store.clone();
        let workspace_store = self.workspace_store.clone();

        // Prepare response data
        let message_id_str = message_id.to_string();
        let content = content.to_string();
        let thread_id = thread_id.map(|s| s.to_string());
        let metadata_json = metadata_json.to_string();
        let attachments = attachments.to_vec();

        // Execute in blocking task with timeout
        tracing::info!(channel = %channel_name, "Starting on_respond WASM execution");

        let result = tokio::time::timeout(timeout, async move {
            tokio::task::spawn_blocking(move || {
                // Read attachment files from disk before entering WASM
                let wit_attachments = read_attachments(&attachments).map_err(|e| {
                    WasmChannelError::CallbackFailed {
                        name: prepared.name.clone(),
                        reason: e,
                    }
                })?;

                tracing::info!("Creating WASM store for on_respond");
                let mut store = Self::create_store(
                    &runtime,
                    &prepared,
                    &capabilities,
                    credentials,
                    host_credentials,
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
                    attachments: wit_attachments,
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

                let mut host_state =
                    Self::extract_host_state(&mut store, &prepared.name, &capabilities);
                let pending_writes = host_state.take_pending_writes();
                workspace_store.commit_writes(&pending_writes);
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

    /// Execute the on_broadcast callback.
    ///
    /// Called to send a proactive message to a user without a prior incoming message.
    pub async fn call_on_broadcast(
        &self,
        user_id: &str,
        content: &str,
        thread_id: Option<&str>,
        attachments: &[String],
    ) -> Result<(), WasmChannelError> {
        tracing::info!(
            channel = %self.name,
            user_id = %user_id,
            content_len = content.len(),
            attachment_count = attachments.len(),
            "call_on_broadcast invoked"
        );

        // If no WASM bytes, do nothing (for testing)
        if self.prepared.component().is_none() {
            tracing::debug!(
                channel = %self.name,
                "WASM channel on_broadcast called (no WASM module)"
            );
            return Ok(());
        }

        let runtime = Arc::clone(&self.runtime);
        let prepared = Arc::clone(&self.prepared);
        let capabilities = Self::inject_workspace_reader(&self.capabilities, &self.workspace_store);
        let timeout = self.runtime.config().callback_timeout;
        let channel_name = self.name.clone();
        let credentials = self.get_credentials().await;
        let host_credentials = resolve_channel_host_credentials(
            &self.capabilities,
            self.secrets_store.as_deref(),
            &self.owner_scope_id,
        )
        .await;
        let pairing_store = self.pairing_store.clone();
        let workspace_store = self.workspace_store.clone();

        let user_id = user_id.to_string();
        let content = content.to_string();
        let thread_id = thread_id.map(|s| s.to_string());
        let attachments = attachments.to_vec();

        let result = tokio::time::timeout(timeout, async move {
            tokio::task::spawn_blocking(move || {
                // Read attachment files from disk
                let wit_attachments = read_attachments(&attachments).map_err(|e| {
                    WasmChannelError::CallbackFailed {
                        name: prepared.name.clone(),
                        reason: e,
                    }
                })?;

                let mut store = Self::create_store(
                    &runtime,
                    &prepared,
                    &capabilities,
                    credentials,
                    host_credentials,
                    pairing_store,
                )?;

                let instance = Self::instantiate_component(&runtime, &prepared, &mut store)?;

                let wit_response = wit_channel::AgentResponse {
                    message_id: String::new(),
                    content: content.clone(),
                    thread_id,
                    metadata_json: String::new(),
                    attachments: wit_attachments,
                };

                let channel_iface = instance.near_agent_channel();
                let wasm_result = channel_iface
                    .call_on_broadcast(&mut store, &user_id, &wit_response)
                    .map_err(|e| {
                        tracing::error!(error = %e, "WASM on_broadcast call failed");
                        Self::map_wasm_error(e, &prepared.name, prepared.limits.fuel)
                    })?;

                if let Err(ref err_msg) = wasm_result {
                    tracing::error!(error = %err_msg, "WASM on_broadcast returned error");
                    return Err(WasmChannelError::CallbackFailed {
                        name: prepared.name.clone(),
                        reason: err_msg.clone(),
                    });
                }

                let mut host_state =
                    Self::extract_host_state(&mut store, &prepared.name, &capabilities);
                let pending_writes = host_state.take_pending_writes();
                workspace_store.commit_writes(&pending_writes);
                tracing::info!("on_broadcast WASM execution completed successfully");
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
            Ok(Ok(((), _host_state))) => {
                tracing::debug!(
                    channel = %channel_name,
                    "WASM channel on_broadcast completed"
                );
                Ok(())
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(WasmChannelError::Timeout {
                name: channel_name,
                callback: "on_broadcast".to_string(),
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
        let capabilities = Self::inject_workspace_reader(&self.capabilities, &self.workspace_store);
        let timeout = self.runtime.config().callback_timeout;
        let channel_name = self.name.clone();
        let credentials = self.get_credentials().await;
        let host_credentials = resolve_channel_host_credentials(
            &self.capabilities,
            self.secrets_store.as_deref(),
            &self.owner_scope_id,
        )
        .await;
        let pairing_store = self.pairing_store.clone();
        let workspace_store = self.workspace_store.clone();

        let Some(wit_update) = status_to_wit(status, metadata) else {
            return Ok(());
        };

        let result = tokio::time::timeout(timeout, async move {
            tokio::task::spawn_blocking(move || {
                let mut store = Self::create_store(
                    &runtime,
                    &prepared,
                    &capabilities,
                    credentials,
                    host_credentials,
                    pairing_store,
                )?;
                let instance = Self::instantiate_component(&runtime, &prepared, &mut store)?;

                let channel_iface = instance.near_agent_channel();
                channel_iface
                    .call_on_status(&mut store, &wit_update)
                    .map_err(|e| Self::map_wasm_error(e, &prepared.name, prepared.limits.fuel))?;

                let mut host_state =
                    Self::extract_host_state(&mut store, &prepared.name, &capabilities);
                let pending_writes = host_state.take_pending_writes();
                workspace_store.commit_writes(&pending_writes);

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
        host_credentials: Vec<ResolvedHostCredential>,
        pairing_store: Arc<PairingStore>,
        timeout: Duration,
        workspace_store: &Arc<ChannelWorkspaceStore>,
        wit_update: wit_channel::StatusUpdate,
    ) -> Result<(), WasmChannelError> {
        if prepared.component().is_none() {
            return Ok(());
        }

        let runtime = Arc::clone(runtime);
        let prepared = Arc::clone(prepared);
        let capabilities = Self::inject_workspace_reader(capabilities, workspace_store);
        let credentials_snapshot = credentials.read().await.clone();
        let channel_name_owned = channel_name.to_string();
        let workspace_store = Arc::clone(workspace_store);

        let result = tokio::time::timeout(timeout, async move {
            tokio::task::spawn_blocking(move || {
                let mut store = Self::create_store(
                    &runtime,
                    &prepared,
                    &capabilities,
                    credentials_snapshot,
                    host_credentials,
                    pairing_store,
                )?;
                let instance = Self::instantiate_component(&runtime, &prepared, &mut store)?;

                let channel_iface = instance.near_agent_channel();
                channel_iface
                    .call_on_status(&mut store, &wit_update)
                    .map_err(|e| Self::map_wasm_error(e, &prepared.name, prepared.limits.fuel))?;

                let mut host_state =
                    Self::extract_host_state(&mut store, &prepared.name, &capabilities);
                let pending_writes = host_state.take_pending_writes();
                workspace_store.commit_writes(&pending_writes);

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
                let workspace_store = self.workspace_store.clone();
                let credentials = self.credentials.clone();
                // Pre-resolve host credentials once for the lifetime of the repeater.
                // Channels tokens rarely change, so a snapshot per-repeater is correct.
                let repeater_host_credentials = resolve_channel_host_credentials(
                    &self.capabilities,
                    self.secrets_store.as_deref(),
                    &self.owner_scope_id,
                )
                .await;
                let pairing_store = self.pairing_store.clone();
                let callback_timeout = self.runtime.config().callback_timeout;
                let Some(wit_update) = status_to_wit(&status, metadata) else {
                    return Ok(());
                };

                let handle = tokio::spawn(async move {
                    let mut interval = tokio::time::interval(Duration::from_secs(4));
                    // Skip the first tick (we already fired above)
                    interval.tick().await;

                    loop {
                        interval.tick().await;

                        let wit_update_clone = clone_wit_status_update(&wit_update);
                        let hc = repeater_host_credentials.clone();

                        if let Err(e) = Self::execute_status(
                            &channel_name,
                            &runtime,
                            &prepared,
                            &capabilities,
                            &credentials,
                            hc,
                            pairing_store.clone(),
                            callback_timeout,
                            &workspace_store,
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
            StatusUpdate::ApprovalNeeded { .. } => {
                // WASM channels (Telegram, Slack, etc.) cannot render
                // interactive approval overlays.  Send the approval prompt
                // as an actual message so the user can reply yes/no.
                self.cancel_typing_task().await;

                let Some(prompt) = crate::channels::ChatApprovalPrompt::from_status(&status) else {
                    return Ok(());
                };
                let prompt = prompt.plain_text_message();

                let metadata_json = serde_json::to_string(metadata).unwrap_or_default();
                if let Err(e) = self
                    .call_on_respond(uuid::Uuid::new_v4(), &prompt, None, &metadata_json, &[])
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

        // Clone sender to avoid holding RwLock read guard across send().await in the loop
        let tx = {
            let tx_guard = self.message_tx.read().await;
            let Some(tx) = tx_guard.as_ref() else {
                tracing::error!(
                    channel = %self.name,
                    count = messages.len(),
                    "Messages emitted but no sender available - channel may not be started!"
                );
                return Ok(());
            };
            tx.clone()
        };

        for emitted in messages {
            // Check rate limit — acquire and release the write lock before send().await
            {
                let mut rate_limiter = self.rate_limiter.write().await;
                if !rate_limiter.check_and_record() {
                    tracing::warn!(
                        channel = %self.name,
                        "Message emission rate limited"
                    );
                    return Err(WasmChannelError::EmitRateLimited {
                        name: self.name.clone(),
                    });
                }
            }

            // Clone the owner_actor_id out of the lock to avoid holding
            // the read guard across the async resolve call below.
            let owner_actor_id = self.owner_actor_id.read().await.clone();
            let (resolved_user_id, is_owner_sender) = resolve_message_scope_with_pairing(
                &self.name,
                &self.owner_scope_id,
                owner_actor_id.as_deref(),
                &emitted.user_id,
                self.pairing_store.as_ref(),
            )
            .await;

            // Convert to IncomingMessage
            let mut msg = IncomingMessage::new(&self.name, &resolved_user_id, &emitted.content)
                .with_sender_id(&emitted.user_id);

            if let Some(name) = emitted.user_name {
                msg = msg.with_user_name(name);
            }

            if let Some(thread_id) = emitted.thread_id {
                msg = msg.with_thread(thread_id);
            }

            // Convert attachments
            if !emitted.attachments.is_empty() {
                let incoming_attachments = emitted
                    .attachments
                    .iter()
                    .map(|a| crate::channels::IncomingAttachment {
                        id: a.id.clone(),
                        kind: crate::channels::AttachmentKind::from_mime_type(&a.mime_type),
                        mime_type: a.mime_type.clone(),
                        filename: a.filename.clone(),
                        size_bytes: a.size_bytes,
                        source_url: a.source_url.clone(),
                        storage_key: a.storage_key.clone(),
                        extracted_text: a.extracted_text.clone(),
                        data: a.data.clone(),
                        duration_secs: a.duration_secs,
                    })
                    .collect();
                msg = msg.with_attachments(incoming_attachments);
            }

            // Parse metadata JSON
            msg = apply_emitted_metadata(msg, &emitted.metadata_json);
            if is_owner_sender {
                // Store for owner-target routing (chat_id etc.).
                self.update_broadcast_metadata(&emitted.metadata_json).await;
            }

            // Send to stream — no locks held across this await
            tracing::info!(
                channel = %self.name,
                user_id = %emitted.user_id,
                content_len = emitted.content.len(),
                attachment_count = msg.attachments.len(),
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

    /// Ensure the polling loop is running with the interval from `config`.
    ///
    /// Stops any existing polling task and starts a fresh one.  Safe to call
    /// multiple times (e.g., from `refresh_active_channel` after re-running
    /// `on_start`).
    pub async fn ensure_polling(&self, config: &ChannelConfig) {
        // Always stop any existing polling task first — if the channel switched
        // from polling to webhook (or polling was disabled), the old task must
        // not keep running. Wait for the old task to finish so two pollers
        // cannot race on the same workspace-backed polling offset.
        self.stop_polling().await;

        if let Some(poll_config) = &config.poll
            && poll_config.enabled
        {
            let interval = match self
                .capabilities
                .validate_poll_interval(poll_config.interval_ms)
            {
                Ok(ms) => ms,
                Err(e) => {
                    tracing::warn!(channel = %self.name, error = %e, "Polling interval rejected");
                    return;
                }
            };

            let (poll_shutdown_tx, poll_shutdown_rx) = oneshot::channel();
            *self.poll_shutdown_tx.write().await = Some(poll_shutdown_tx);

            let handle = self.start_polling(
                Duration::from_millis(interval as u64),
                poll_shutdown_rx,
                Arc::clone(&self.owner_actor_id),
            );
            *self.poll_task.write().await = Some(handle);
            tracing::debug!(channel = %self.name, interval_ms = interval, "Polling loop (re)started");
        }
    }

    /// Start the polling loop if configured.
    ///
    /// Since we can't hold `Arc<Self>` from `&self`, we pass all the components
    /// needed for polling to a spawned task. Each poll tick creates a fresh WASM
    /// instance (matching our "fresh instance per callback" pattern).
    fn start_polling(
        &self,
        interval: Duration,
        shutdown_rx: oneshot::Receiver<()>,
        owner_actor_id: Arc<tokio::sync::RwLock<Option<String>>>,
    ) -> tokio::task::JoinHandle<()> {
        let channel_name = self.name.clone();
        let runtime = Arc::clone(&self.runtime);
        let prepared = Arc::clone(&self.prepared);
        let poll_capabilities = self.capabilities.clone();
        let capabilities = Self::inject_workspace_reader(&self.capabilities, &self.workspace_store);
        let message_tx = self.message_tx.clone();
        let rate_limiter = self.rate_limiter.clone();
        let credentials = self.credentials.clone();
        let pairing_store = self.pairing_store.clone();
        let callback_timeout = self.runtime.config().callback_timeout;
        let workspace_store = self.workspace_store.clone();
        let last_broadcast_metadata = self.last_broadcast_metadata.clone();
        let settings_store = self.settings_store.clone();
        let poll_secrets_store = self.secrets_store.clone();
        let owner_scope_id = self.owner_scope_id.clone();

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

                        // Pre-resolve host credentials for this tick
                        let host_credentials = resolve_channel_host_credentials(
                            &poll_capabilities,
                            poll_secrets_store.as_deref(),
                            &owner_scope_id,
                        )
                        .await;

                        // Execute on_poll with fresh WASM instance
                        let result = Self::execute_poll(
                            &channel_name,
                            &runtime,
                            &prepared,
                            &capabilities,
                            &credentials,
                            host_credentials,
                            pairing_store.clone(),
                            callback_timeout,
                            &workspace_store,
                        ).await;

                        match result {
                            Ok(emitted_messages) => {
                                // Read the current owner on each tick so
                                // post-approval changes are visible immediately.
                                let current_owner = owner_actor_id.read().await.clone();
                                // Process any emitted messages
                                if !emitted_messages.is_empty()
                                    && let Err(e) = Self::dispatch_emitted_messages(
                                        EmitDispatchContext {
                                            channel_name: &channel_name,
                                            owner_scope_id: &owner_scope_id,
                                            owner_actor_id: current_owner.as_deref(),
                                            pairing_store: pairing_store.as_ref(),
                                            message_tx: &message_tx,
                                            rate_limiter: &rate_limiter,
                                            last_broadcast_metadata: &last_broadcast_metadata,
                                            settings_store: settings_store.as_ref(),
                                        },
                                        emitted_messages,
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
        })
    }

    async fn stop_polling(&self) {
        let shutdown_tx = self.poll_shutdown_tx.write().await.take();
        if let Some(tx) = shutdown_tx {
            let _ = tx.send(());
        }

        let poll_task = self.poll_task.write().await.take();
        if let Some(handle) = poll_task
            && let Err(error) = handle.await
        {
            tracing::debug!(channel = %self.name, error = %error, "Polling task join failed");
        }
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
        host_credentials: Vec<ResolvedHostCredential>,
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
                    host_credentials,
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
                let _ = drain_guest_logs(channel_name, "on_poll", &mut host_state);
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
        dispatch: EmitDispatchContext<'_>,
        messages: Vec<EmittedMessage>,
    ) -> Result<(), WasmChannelError> {
        tracing::info!(
            channel = %dispatch.channel_name,
            message_count = messages.len(),
            "Processing emitted messages from polling callback"
        );

        // Clone sender to avoid holding RwLock read guard across send().await in the loop
        let tx = {
            let tx_guard = dispatch.message_tx.read().await;
            let Some(tx) = tx_guard.as_ref() else {
                tracing::error!(
                    channel = %dispatch.channel_name,
                    count = messages.len(),
                    "Messages emitted but no sender available - channel may not be started!"
                );
                return Ok(());
            };
            tx.clone()
        };

        for emitted in messages {
            // Check rate limit — acquire and release the write lock before send().await
            {
                let mut limiter = dispatch.rate_limiter.write().await;
                if !limiter.check_and_record() {
                    tracing::warn!(
                        channel = %dispatch.channel_name,
                        "Message emission rate limited"
                    );
                    return Err(WasmChannelError::EmitRateLimited {
                        name: dispatch.channel_name.to_string(),
                    });
                }
            }

            let (resolved_user_id, is_owner_sender) = resolve_message_scope_with_pairing(
                dispatch.channel_name,
                dispatch.owner_scope_id,
                dispatch.owner_actor_id,
                &emitted.user_id,
                dispatch.pairing_store,
            )
            .await;

            // Convert to IncomingMessage
            let mut msg =
                IncomingMessage::new(dispatch.channel_name, &resolved_user_id, &emitted.content)
                    .with_sender_id(&emitted.user_id);

            if let Some(name) = emitted.user_name {
                msg = msg.with_user_name(name);
            }

            if let Some(thread_id) = emitted.thread_id {
                msg = msg.with_thread(thread_id);
            }

            // Convert attachments
            if !emitted.attachments.is_empty() {
                let incoming_attachments = emitted
                    .attachments
                    .iter()
                    .map(|a| crate::channels::IncomingAttachment {
                        id: a.id.clone(),
                        kind: crate::channels::AttachmentKind::from_mime_type(&a.mime_type),
                        mime_type: a.mime_type.clone(),
                        filename: a.filename.clone(),
                        size_bytes: a.size_bytes,
                        source_url: a.source_url.clone(),
                        storage_key: a.storage_key.clone(),
                        extracted_text: a.extracted_text.clone(),
                        data: a.data.clone(),
                        duration_secs: a.duration_secs,
                    })
                    .collect();
                msg = msg.with_attachments(incoming_attachments);
            }

            msg = apply_emitted_metadata(msg, &emitted.metadata_json);
            if is_owner_sender {
                // Store for owner-target routing (chat_id etc.)
                do_update_broadcast_metadata(
                    dispatch.channel_name,
                    dispatch.owner_scope_id,
                    &emitted.metadata_json,
                    dispatch.last_broadcast_metadata,
                    dispatch.settings_store,
                )
                .await;
            }

            if emitted.content.trim().is_empty() && emitted.attachments.is_empty() {
                tracing::debug!(
                    channel = %dispatch.channel_name,
                    user_id = %emitted.user_id,
                    "Skipping empty emitted message"
                );
                continue;
            }

            // Send to stream — no locks held across this await
            tracing::info!(
                channel = %dispatch.channel_name,
                user_id = %emitted.user_id,
                content_len = emitted.content.len(),
                attachment_count = msg.attachments.len(),
                "Sending polled message to agent"
            );

            if tx.send(msg).await.is_err() {
                tracing::error!(
                    channel = %dispatch.channel_name,
                    "Failed to send polled message, channel closed"
                );
                break;
            }

            tracing::info!(
                channel = %dispatch.channel_name,
                "Message successfully sent to agent queue"
            );
        }

        Ok(())
    }
}

struct EmitDispatchContext<'a> {
    channel_name: &'a str,
    owner_scope_id: &'a str,
    owner_actor_id: Option<&'a str>,
    pairing_store: &'a PairingStore,
    message_tx: &'a RwLock<Option<mpsc::Sender<IncomingMessage>>>,
    rate_limiter: &'a RwLock<ChannelEmitRateLimiter>,
    last_broadcast_metadata: &'a tokio::sync::RwLock<Option<String>>,
    settings_store: Option<&'a Arc<dyn crate::db::SettingsStore>>,
}

#[async_trait]
impl Channel for WasmChannel {
    fn name(&self) -> &str {
        &self.name
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        // Restore broadcast metadata from settings (survives restarts)
        self.load_broadcast_metadata().await;

        // Call on_start BEFORE creating message_tx so a failed start
        // doesn't leave an orphaned sender with a dropped receiver.
        // (If on_start fails, the rx would be dropped on error return
        // but message_tx would keep the tx alive — causing is_closed=true
        // on any subsequent polling attempt.)
        let config = self
            .call_on_start()
            .await
            .map_err(|e| ChannelError::StartupFailed {
                name: self.name.clone(),
                reason: e.to_string(),
            })?;

        // Create message channel — only after on_start succeeds
        let (tx, rx) = mpsc::channel(256);
        *self.message_tx.write().await = Some(tx);

        // Create shutdown channel
        let (shutdown_tx, _shutdown_rx) = oneshot::channel();
        *self.shutdown_tx.write().await = Some(shutdown_tx);

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

            let handle = self.start_polling(
                Duration::from_millis(interval as u64),
                poll_shutdown_rx,
                Arc::clone(&self.owner_actor_id),
            );
            *self.poll_task.write().await = Some(handle);
        }

        if let Some(websocket_config) =
            WebsocketRuntimeConfig::from_capabilities(&self.capabilities)
            && websocket_config.connect_on_start
        {
            let (websocket_shutdown_tx, websocket_shutdown_rx) = oneshot::channel();
            *self.websocket_shutdown_tx.write().await = Some(websocket_shutdown_tx);
            self.start_websocket_runtime(
                websocket_config,
                websocket_shutdown_rx,
                Arc::clone(&self.owner_actor_id),
            );
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
        // Store for owner-target routing (chat_id etc.) only when the configured
        // owner is the actor in this conversation.
        let owner_actor_id = self.owner_actor_id.read().await.clone();
        if should_update_owner_broadcast_metadata(
            &msg.user_id,
            &msg.sender_id,
            &self.owner_scope_id,
            owner_actor_id.as_deref(),
        ) {
            self.update_broadcast_metadata(&metadata_json).await;
        }
        self.call_on_respond(
            msg.id,
            &response.content,
            response.thread_id.as_deref(),
            &metadata_json,
            &response.attachments,
        )
        .await
        .map_err(|e| ChannelError::SendFailed {
            name: self.name.clone(),
            reason: e.to_string(),
        })?;

        Ok(())
    }

    async fn broadcast(
        &self,
        user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        self.cancel_typing_task().await;
        let resolved_target = if uses_owner_broadcast_target(user_id, &self.owner_scope_id) {
            let metadata = self.last_broadcast_metadata.read().await.clone().ok_or_else(|| {
                missing_routing_target_error(
                    &self.name,
                    format!(
                        "No stored owner routing target for channel '{}'. Send a message from the owner on this channel first.",
                        self.name
                    ),
                )
            })?;

            resolve_owner_broadcast_target(&self.name, &metadata)?
        } else {
            user_id.to_string()
        };

        self.call_on_broadcast(
            &resolved_target,
            &response.content,
            response.thread_id.as_deref(),
            &response.attachments,
        )
        .await
        .map_err(|e| ChannelError::SendFailed {
            name: self.name.clone(),
            reason: e.to_string(),
        })
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

        self.stop_polling().await;

        // Stop websocket runtime by dropping the sender (receiver will complete)
        let _ = self.websocket_shutdown_tx.write().await.take();

        // Clear the message sender
        *self.message_tx.write().await = None;

        tracing::info!(
            channel = %self.name,
            "WASM channel shut down"
        );

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct WebsocketRuntimeConfig {
    pub(crate) url: String,
    pub(crate) connect_on_start: bool,
    pub(crate) identify: Option<serde_json::Value>,
    pub(crate) identify_secret_name: Option<String>,
}

impl WebsocketRuntimeConfig {
    pub(crate) fn from_capabilities(capabilities: &ChannelCapabilities) -> Option<Self> {
        let raw = capabilities.tool_capabilities.websocket.as_ref()?;
        let url = raw.get("url")?.as_str()?.trim();
        if url.is_empty() {
            return None;
        }

        let parsed = url::Url::parse(url).ok()?;
        let scheme = parsed.scheme();
        if scheme != "ws" && scheme != "wss" {
            return None;
        }

        let host = parsed.host_str()?;
        let path = parsed.path();
        let http = capabilities.tool_capabilities.http.as_ref()?;
        if !http
            .allowlist
            .iter()
            .any(|pattern| pattern.matches(host, path, "GET"))
        {
            return None;
        }

        Some(Self {
            url: url.to_string(),
            connect_on_start: raw
                .get("connect_on_start")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            identify: raw.get("identify").cloned(),
            identify_secret_name: raw
                .get("identify_secret_name")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned),
        })
    }
}

fn websocket_queue_path(channel_name: &str) -> String {
    format!("channels/{channel_name}/{WEBSOCKET_EVENT_QUEUE_RELATIVE_PATH}")
}

fn websocket_processing_queue_path(channel_name: &str) -> String {
    format!("channels/{channel_name}/{WEBSOCKET_EVENT_PROCESSING_QUEUE_RELATIVE_PATH}")
}

async fn resolve_websocket_identify_message(
    config: &WebsocketRuntimeConfig,
    store: Option<&(dyn SecretsStore + Send + Sync)>,
    owner_scope_id: &str,
) -> Option<String> {
    let identify = config.identify.clone()?;
    let secret_name = config.identify_secret_name.as_ref()?;
    let store = store?;
    // Channel runtime secrets are instance-owned, resolved under the channel's owner scope.
    let secret = store
        .get_decrypted(owner_scope_id, secret_name)
        .await
        .ok()?;
    build_websocket_identify_message(&identify, secret.expose())
}

fn build_websocket_identify_message(identify: &serde_json::Value, token: &str) -> Option<String> {
    let mut payload = identify.as_object()?.clone();
    payload.insert(
        "token".to_string(),
        serde_json::Value::String(token.to_string()),
    );

    serde_json::to_string(&serde_json::json!({
        "op": 2,
        "d": serde_json::Value::Object(payload),
    }))
    .ok()
}

fn build_websocket_heartbeat_message(sequence: Option<serde_json::Value>) -> Option<String> {
    serde_json::to_string(&serde_json::json!({
        "op": 1,
        "d": sequence.unwrap_or(serde_json::Value::Null),
    }))
    .ok()
}

fn build_discord_gateway_presence_update(status: &str) -> Option<String> {
    serde_json::to_string(&serde_json::json!({
        "op": 3,
        "d": {
            "since": serde_json::Value::Null,
            "activities": [],
            "status": status,
            "afk": false
        }
    }))
    .ok()
}

fn build_gateway_presence_update(
    channel_name: &str,
    workspace_store: &crate::channels::wasm::host::ChannelWorkspaceStore,
    pairing_store: &PairingStore,
) -> Option<String> {
    if channel_name != "discord" {
        return None;
    }

    build_discord_gateway_presence_update(discord_gateway_presence_status(
        channel_name,
        workspace_store,
        pairing_store,
    ))
}

fn discord_gateway_presence_status(
    channel_name: &str,
    workspace_store: &crate::channels::wasm::host::ChannelWorkspaceStore,
    _pairing_store: &PairingStore,
) -> &'static str {
    use crate::tools::wasm::WorkspaceReader;

    let owner_key = format!("channels/{}/state/owner_id", channel_name);
    if workspace_store
        .read(&owner_key)
        .filter(|s| !s.is_empty())
        .is_some()
    {
        return "online";
    }

    "dnd"
}

fn parse_websocket_hello_heartbeat_interval_ms(text: &str) -> Option<u64> {
    let payload: serde_json::Value = serde_json::from_str(text).ok()?;
    if payload.get("op")?.as_u64()? != 10 {
        return None;
    }

    payload.get("d")?.get("heartbeat_interval")?.as_u64()
}

fn websocket_reconnect_backoff(attempt: u32) -> Duration {
    use rand::Rng;

    let exponent = attempt.min(6);
    let base_ms = (1u64 << exponent) * 1_000;
    // Add 0-25% jitter per Discord's reconnection recommendations to avoid
    // thundering-herd when many bots reconnect after a Discord deploy.
    let jitter_ms = rand::thread_rng().gen_range(0..=base_ms / 4);
    Duration::from_millis(base_ms + jitter_ms)
}

fn websocket_heartbeat_sleep_duration(interval_ms: u64) -> Duration {
    Duration::from_millis(interval_ms.max(1))
}

fn should_warn_on_heartbeat_interval(interval_ms: u64) -> bool {
    interval_ms < 1_000
}

fn parse_websocket_sequence(text: &str) -> Option<u64> {
    let payload: serde_json::Value = serde_json::from_str(text).ok()?;
    payload.get("s")?.as_u64()
}

fn parse_websocket_ready_session(text: &str) -> Option<(String, Option<String>)> {
    let payload: serde_json::Value = serde_json::from_str(text).ok()?;
    if payload.get("op")?.as_u64()? != 0 {
        return None;
    }
    if payload.get("t")?.as_str()? != "READY" {
        return None;
    }
    let d = payload.get("d")?;
    let sid = d.get("session_id")?.as_str()?.to_string();
    let resume_url = d
        .get("resume_gateway_url")
        .and_then(|v| v.as_str())
        .map(ToOwned::to_owned);
    Some((sid, resume_url))
}

fn build_websocket_resume_message(
    token: &str,
    session_id: &str,
    sequence: Option<&serde_json::Value>,
) -> Option<String> {
    serde_json::to_string(&serde_json::json!({
        "op": 6,
        "d": {
            "token": token,
            "session_id": session_id,
            "seq": sequence.cloned().unwrap_or(serde_json::Value::Null),
        }
    }))
    .ok()
}

fn parse_websocket_invalid_session(text: &str) -> Option<bool> {
    let payload: serde_json::Value = serde_json::from_str(text).ok()?;
    if payload.get("op")?.as_u64()? != 9 {
        return None;
    }
    Some(payload.get("d")?.as_bool().unwrap_or(false))
}

fn extract_token_from_identify_payload(identify_payload: &str) -> Option<String> {
    let payload: serde_json::Value = serde_json::from_str(identify_payload).ok()?;
    payload
        .get("d")?
        .get("token")?
        .as_str()
        .map(ToOwned::to_owned)
}

fn drain_guest_logs(
    channel_name: &str,
    callback: &str,
    host_state: &mut ChannelHostState,
) -> Vec<crate::tools::wasm::LogEntry> {
    let entries = host_state.take_logs();

    for entry in &entries {
        match entry.level {
            crate::tools::wasm::LogLevel::Error => {
                tracing::error!(channel = %channel_name, callback = callback, "{}", entry.message);
            }
            crate::tools::wasm::LogLevel::Warn => {
                tracing::warn!(channel = %channel_name, callback = callback, "{}", entry.message);
            }
            crate::tools::wasm::LogLevel::Info => {
                tracing::info!(channel = %channel_name, callback = callback, "{}", entry.message);
            }
            crate::tools::wasm::LogLevel::Debug => {
                tracing::debug!(channel = %channel_name, callback = callback, "{}", entry.message);
            }
            crate::tools::wasm::LogLevel::Trace => {
                tracing::trace!(channel = %channel_name, callback = callback, "{}", entry.message);
            }
        }
    }

    entries
}

/// Shared state for websocket-triggered poll tasks.
///
/// Groups the many `Arc` handles needed by [`spawn_websocket_poll`] into a
/// single cloneable context so the call site stays readable.
struct WebsocketPollContext {
    channel_name: String,
    runtime: Arc<WasmChannelRuntime>,
    prepared: Arc<PreparedChannelModule>,
    capabilities: ChannelCapabilities,
    poll_capabilities: ChannelCapabilities,
    credentials: Arc<RwLock<HashMap<String, String>>>,
    pairing_store: Arc<PairingStore>,
    workspace_store: Arc<ChannelWorkspaceStore>,
    message_tx: Arc<RwLock<Option<mpsc::Sender<IncomingMessage>>>>,
    rate_limiter: Arc<RwLock<ChannelEmitRateLimiter>>,
    last_broadcast_metadata: Arc<tokio::sync::RwLock<Option<String>>>,
    settings_store: Option<Arc<dyn crate::db::SettingsStore>>,
    owner_scope_id: String,
    owner_actor_id: Arc<tokio::sync::RwLock<Option<String>>>,
    secrets_store: Option<Arc<dyn SecretsStore + Send + Sync>>,
    outbound_tx: mpsc::UnboundedSender<String>,
    queue_path: String,
    processing_queue_path: String,
    callback_timeout: Duration,
}

/// Spawn the websocket-triggered poll task.
///
/// Extracted from the select loop to reduce nesting. Moves items from the
/// event queue to a processing queue and runs the WASM `on_poll` callback.
fn spawn_websocket_poll(poll_guard: tokio::sync::OwnedMutexGuard<()>, ctx: WebsocketPollContext) {
    tokio::spawn(async move {
        let _poll_guard = poll_guard;

        loop {
            let moved = match ctx
                .workspace_store
                .move_json_text_queue(&ctx.queue_path, &ctx.processing_queue_path)
            {
                Ok(value) => value,
                Err(error) => {
                    tracing::warn!(channel = %ctx.channel_name, error = %error, "Failed to snapshot websocket queue for polling");
                    break;
                }
            };

            if !moved {
                break;
            }

            let host_credentials = resolve_channel_host_credentials(
                &ctx.poll_capabilities,
                ctx.secrets_store.as_deref(),
                &ctx.owner_scope_id,
            )
            .await;

            match WasmChannel::execute_poll(
                &ctx.channel_name,
                &ctx.runtime,
                &ctx.prepared,
                &ctx.capabilities,
                &ctx.credentials,
                host_credentials,
                ctx.pairing_store.clone(),
                ctx.callback_timeout,
                &ctx.workspace_store,
            )
            .await
            {
                Ok(emitted_messages) => {
                    // Read the current owner so post-approval changes
                    // are visible immediately.
                    let current_owner = ctx.owner_actor_id.read().await.clone();
                    if !emitted_messages.is_empty()
                        && let Err(error) = WasmChannel::dispatch_emitted_messages(
                            EmitDispatchContext {
                                channel_name: &ctx.channel_name,
                                owner_scope_id: &ctx.owner_scope_id,
                                owner_actor_id: current_owner.as_deref(),
                                pairing_store: ctx.pairing_store.as_ref(),
                                message_tx: &ctx.message_tx,
                                rate_limiter: &ctx.rate_limiter,
                                last_broadcast_metadata: &ctx.last_broadcast_metadata,
                                settings_store: ctx.settings_store.as_ref(),
                            },
                            emitted_messages,
                        )
                        .await
                    {
                        tracing::warn!(channel = %ctx.channel_name, error = %error, "Failed to dispatch emitted websocket poll messages");
                    }
                }
                Err(error) => {
                    tracing::warn!(channel = %ctx.channel_name, error = %error, "Websocket-triggered poll failed");
                }
            }

            if let Some(payload) = build_gateway_presence_update(
                &ctx.channel_name,
                ctx.workspace_store.as_ref(),
                ctx.pairing_store.as_ref(),
            ) {
                let _ = ctx.outbound_tx.send(payload);
            }
        }
    });
}

/// Actions produced by websocket text frame processing.
///
/// Returned from [`WebsocketSessionState::process_text_frame`] so the caller
/// can perform the actual I/O (send messages, break loops) while keeping the
/// parsing logic synchronous and testable.
enum WebsocketFrameAction {
    /// Update the heartbeat timer to fire after `interval_ms` milliseconds.
    SetHeartbeat { interval_ms: u64 },
    /// Send a text payload over the websocket.
    Send(String),
    /// Enqueue the raw text into the workspace event queue.
    Enqueue(String),
    /// Clear session state and reconnect with a fresh identify.
    InvalidateAndReconnect,
}

/// Tracks websocket session state across reconnects.
///
/// Keeps heartbeat interval, sequence counter, and Discord Gateway session
/// resumption fields. The [`process_text_frame`] method parses incoming frames
/// and returns a list of [`WebsocketFrameAction`]s the caller should execute.
struct WebsocketSessionState {
    heartbeat_interval_ms: Option<u64>,
    last_sequence: Option<serde_json::Value>,
    session_id: Option<String>,
    resume_gateway_url: Option<String>,
    /// Raw bot token extracted from the identify payload.
    token: Option<String>,
    /// Whether we attempted a resume on this connection.
    attempted_resume: bool,
}

impl WebsocketSessionState {
    fn new(identify_payload: Option<&str>) -> Self {
        let token = identify_payload.and_then(extract_token_from_identify_payload);
        Self {
            heartbeat_interval_ms: None,
            last_sequence: None,
            session_id: None,
            resume_gateway_url: None,
            token,
            attempted_resume: false,
        }
    }

    /// Determine the URL to use for the next connection attempt.
    fn connect_url<'a>(&'a self, default_url: &'a str) -> &'a str {
        if self.session_id.is_some()
            && let Some(ref url) = self.resume_gateway_url
        {
            return url.as_str();
        }
        default_url
    }

    /// Reset per-connection state when starting a fresh connection.
    fn reset_connection(&mut self) {
        self.heartbeat_interval_ms = None;
        self.attempted_resume = false;
    }

    /// Clear all session state so the next reconnect performs a fresh identify.
    fn invalidate_session(&mut self) {
        self.session_id = None;
        self.resume_gateway_url = None;
        self.last_sequence = None;
    }

    /// Process a text frame and return a list of actions for the caller to
    /// execute. This keeps the select loop thin and the parsing logic testable.
    fn process_text_frame(
        &mut self,
        text: &str,
        channel_name: &str,
        identify_payload: Option<&str>,
        workspace_store: &crate::channels::wasm::host::ChannelWorkspaceStore,
        pairing_store: &PairingStore,
    ) -> Vec<WebsocketFrameAction> {
        let mut actions = Vec::new();

        // OP 10 Hello: extract heartbeat interval, send identify or resume
        if let Some(interval_ms) = parse_websocket_hello_heartbeat_interval_ms(text) {
            if should_warn_on_heartbeat_interval(interval_ms) {
                tracing::warn!(
                    channel = %channel_name,
                    heartbeat_interval_ms = interval_ms,
                    "Websocket hello provided unexpectedly low heartbeat interval"
                );
            }

            self.heartbeat_interval_ms = Some(interval_ms);
            actions.push(WebsocketFrameAction::SetHeartbeat { interval_ms });

            // Try resume if we have a session, otherwise fresh identify
            let sent_resume = if let (Some(token), Some(sid)) = (&self.token, &self.session_id) {
                if let Some(payload) =
                    build_websocket_resume_message(token, sid, self.last_sequence.as_ref())
                {
                    self.attempted_resume = true;
                    actions.push(WebsocketFrameAction::Send(payload));
                    true
                } else {
                    false
                }
            } else {
                false
            };

            if !sent_resume && let Some(payload) = identify_payload {
                actions.push(WebsocketFrameAction::Send(payload.to_string()));
            }
        }

        // OP 0 Dispatch READY: capture session_id and resume_gateway_url.
        // Presence update is sent here (after READY) rather than on Hello,
        // because Discord's gateway protocol requires waiting for READY/RESUMED
        // before sending non-Identify commands.
        if let Some((sid, resume_url)) = parse_websocket_ready_session(text) {
            self.session_id = Some(sid);
            self.resume_gateway_url = resume_url;

            if let Some(payload) =
                build_gateway_presence_update(channel_name, workspace_store, pairing_store)
            {
                actions.push(WebsocketFrameAction::Send(payload));
            }
        }

        // Track sequence number from any dispatch
        if let Some(sequence) = parse_websocket_sequence(text) {
            self.last_sequence = Some(serde_json::Value::Number(sequence.into()));
        }

        // OP 9 Invalid Session: if not resumable, clear state and reconnect
        if let Some(resumable) = parse_websocket_invalid_session(text)
            && !resumable
        {
            tracing::info!(
                channel = %channel_name,
                "Received non-resumable invalid session; will reconnect with fresh identify"
            );
            self.invalidate_session();
            actions.push(WebsocketFrameAction::InvalidateAndReconnect);
            return actions;
        }

        // Always enqueue the raw frame for the poll callback
        actions.push(WebsocketFrameAction::Enqueue(text.to_string()));

        actions
    }
}

fn log_websocket_diagnostic(channel_name: &str, message: &WebsocketMessage) {
    match message {
        WebsocketMessage::Text(text) => {
            tracing::trace!(
                channel = %channel_name,
                bytes = text.len(),
                "Websocket runtime received text frame"
            );
        }
        WebsocketMessage::Binary(bytes) => {
            tracing::debug!(
                channel = %channel_name,
                bytes = bytes.len(),
                "Websocket runtime received binary frame"
            );
        }
        WebsocketMessage::Close(frame) => {
            tracing::info!(
                channel = %channel_name,
                code = ?frame.as_ref().map(|f| f.code),
                reason = ?frame.as_ref().map(|f| f.reason.to_string()),
                "Websocket runtime received close frame"
            );
        }
        WebsocketMessage::Ping(payload) => {
            tracing::trace!(
                channel = %channel_name,
                bytes = payload.len(),
                "Websocket runtime received ping"
            );
        }
        WebsocketMessage::Pong(payload) => {
            tracing::trace!(
                channel = %channel_name,
                bytes = payload.len(),
                "Websocket runtime received pong"
            );
        }
        WebsocketMessage::Frame(_) => {}
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

    async fn broadcast(
        &self,
        user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        self.inner.broadcast(user_id, response).await
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

fn status_to_wit(
    status: &StatusUpdate,
    metadata: &serde_json::Value,
) -> Option<wit_channel::StatusUpdate> {
    let metadata_json = serde_json::to_string(metadata).unwrap_or_default();

    Some(match status {
        StatusUpdate::Thinking(msg) => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::Thinking,
            message: msg.clone(),
            metadata_json,
        },
        StatusUpdate::ToolStarted { name, .. } => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::ToolStarted,
            message: format!("Tool started: {}", name),
            metadata_json,
        },
        StatusUpdate::ToolCompleted { name, success, .. } => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::ToolCompleted,
            message: format!(
                "Tool completed: {} ({})",
                name,
                if *success { "ok" } else { "failed" }
            ),
            metadata_json,
        },
        StatusUpdate::ToolResult { name, preview, .. } => wit_channel::StatusUpdate {
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
        StatusUpdate::ApprovalNeeded { .. } => {
            let prompt = crate::channels::ChatApprovalPrompt::from_status(status)?;
            wit_channel::StatusUpdate {
                status: wit_channel::StatusType::ApprovalNeeded,
                message: prompt.plain_text_message(),
                metadata_json,
            }
        }
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
            ..
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
        StatusUpdate::ImageGenerated { path, .. } => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::Status,
            message: match path {
                Some(p) => format!("[image] {}", p),
                None => "[image generated]".to_string(),
            },
            metadata_json,
        },
        // Suggestions and richer UI/runtime telemetry are handled by the web/TUI surfaces.
        StatusUpdate::Suggestions { .. }
        | StatusUpdate::TurnCost { .. }
        | StatusUpdate::SkillActivated { .. }
        | StatusUpdate::JobStatus { .. }
        | StatusUpdate::JobResult { .. }
        | StatusUpdate::RoutineUpdate { .. }
        | StatusUpdate::ContextPressure { .. }
        | StatusUpdate::SandboxStatus { .. }
        | StatusUpdate::SecretsStatus { .. }
        | StatusUpdate::CostGuard { .. }
        | StatusUpdate::ThreadList { .. }
        | StatusUpdate::EngineThreadList { .. }
        | StatusUpdate::ConversationHistory { .. } => return None,
        StatusUpdate::ReasoningUpdate {
            narrative,
            decisions,
        } => {
            let mut msg = narrative.clone();
            for d in decisions {
                msg.push_str(&format!("\n  → {}: {}", d.tool_name, d.rationale));
            }
            wit_channel::StatusUpdate {
                status: wit_channel::StatusType::Status,
                message: msg,
                metadata_json,
            }
        }
    })
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

/// Extract the hostname from a URL string.
///
/// Returns `None` for malformed URLs or non-HTTP(S) schemes.
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

/// Rewrite outbound HTTP URLs for testing.
///
/// `IRONCLAW_TEST_HTTP_REWRITE_MAP` is a JSON object mapping exact hostnames to
/// replacement base URLs. For example:
/// `{"slack.com":"http://127.0.0.1:8080","files.slack.com":"http://127.0.0.1:8080"}`
///
/// The replacement preserves the original path and query string so tests can
/// point production hosts at local fakes without adding channel-specific code.
#[cfg(test)]
fn rewrite_http_url_for_testing(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }

    let host = parsed.host_str()?.to_lowercase();
    let override_base = std::env::var(TEST_HTTP_REWRITE_MAP_ENV)
        .ok()
        .and_then(|value| parse_test_http_rewrite_map(&value).get(&host).cloned())?;

    let path = parsed.path().trim_start_matches('/');
    let mut rewritten = format!("{override_base}/{path}");
    if let Some(query) = parsed.query() {
        rewritten.push('?');
        rewritten.push_str(query);
    }
    Some(rewritten)
}

#[cfg(test)]
fn parse_test_http_rewrite_map(raw: &str) -> HashMap<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return HashMap::new();
    }

    match serde_json::from_str::<HashMap<String, String>>(trimmed) {
        Ok(map) => map
            .into_iter()
            .filter_map(|(host, base)| {
                let host = host.trim().to_lowercase();
                let base = base.trim().trim_end_matches('/').to_string();
                if host.is_empty() || base.is_empty() {
                    return None;
                }
                Some((host, base))
            })
            .collect(),
        Err(error) => {
            tracing::warn!(
                env_var = TEST_HTTP_REWRITE_MAP_ENV,
                %error,
                "Ignoring invalid test HTTP rewrite map"
            );
            HashMap::new()
        }
    }
}

#[cfg(test)]
fn rewrite_telegram_api_url_for_testing(url: &str) -> Option<String> {
    let override_base = std::env::var(TELEGRAM_TEST_API_BASE_ENV)
        .ok()
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())?;

    let parsed = url::Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }

    let host = parsed.host_str()?;
    if !host.eq_ignore_ascii_case("api.telegram.org") {
        return None;
    }

    let path = parsed.path().trim_start_matches('/');
    let mut rewritten = format!("{override_base}/{path}");
    if let Some(query) = parsed.query() {
        rewritten.push('?');
        rewritten.push_str(query);
    }
    Some(rewritten)
}
fn should_skip_response_leak_scan(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|parsed| {
        matches!(parsed.scheme(), "http" | "https")
            && parsed
                .host_str()
                .is_some_and(|host| host.eq_ignore_ascii_case("api.telegram.org"))
            && parsed
                .path_segments()
                .and_then(|segments| segments.rev().find(|segment| !segment.is_empty()))
                .is_some_and(|segment| segment == "getUpdates")
    })
}

/// Pre-resolve host credentials for all HTTP capability mappings.
///
/// Called once per callback (in async context, before spawn_blocking) so the
/// synchronous WASM host function can inject credentials without needing async
/// access to the secrets store.
///
/// Silently skips credentials that can't be resolved (e.g., missing secrets).
/// The channel will get a 401/403 from the API, which is the expected UX when
/// auth hasn't been configured yet.
async fn resolve_channel_host_credentials(
    capabilities: &ChannelCapabilities,
    store: Option<&(dyn SecretsStore + Send + Sync)>,
    owner_scope_id: &str,
) -> Vec<ResolvedHostCredential> {
    let store = match store {
        Some(s) => s,
        None => return Vec::new(),
    };

    let http_cap = match &capabilities.tool_capabilities.http {
        Some(cap) => cap,
        None => return Vec::new(),
    };

    if http_cap.credentials.is_empty() {
        return Vec::new();
    }

    let mut resolved = Vec::new();

    for mapping in http_cap.credentials.values() {
        // Skip UrlPath credentials; they're handled by placeholder substitution
        if matches!(
            mapping.location,
            crate::secrets::CredentialLocation::UrlPath { .. }
        ) {
            continue;
        }

        let secret = match store
            .get_decrypted(owner_scope_id, &mapping.secret_name)
            .await
        {
            Ok(s) => s,
            Err(e) => {
                tracing::debug!(
                    secret_name = %mapping.secret_name,
                    error = %e,
                    "Could not resolve credential for WASM channel (auth may not be configured)"
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
            "Pre-resolved host credentials for WASM channel execution"
        );
    }

    resolved
}

// ============================================================================
// Attachment Helpers
// ============================================================================

/// Maximum total attachment size (50 MB).
const MAX_TOTAL_ATTACHMENT_BYTES: u64 = 50 * 1024 * 1024;

/// Detect MIME type from file extension using the `mime_guess` crate.
fn mime_from_extension(path: &str) -> String {
    mime_guess::from_path(path)
        .first_or_octet_stream()
        .to_string()
}

/// Read attachment files from disk and build WIT attachment records.
///
/// Validates total size against `MAX_TOTAL_ATTACHMENT_BYTES`.
fn read_attachments(paths: &[String]) -> Result<Vec<wit_channel::Attachment>, String> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }

    let mut attachments = Vec::with_capacity(paths.len());
    let mut total_bytes: u64 = 0;
    let tmp_base = std::path::Path::new("/tmp");
    let home_base = dirs::home_dir()
        .map(|h| h.join(".ironclaw"))
        .unwrap_or_default();

    for path in paths {
        // Validate paths are under /tmp/ or ~/.ironclaw/ to prevent arbitrary file reads
        let validated = crate::tools::builtin::path_utils::validate_path(path, Some(tmp_base))
            .or_else(|_| crate::tools::builtin::path_utils::validate_path(path, Some(&home_base)));
        let validated = validated.map_err(|e| {
            format!(
                "Invalid attachment path '{}': must be under /tmp/ or ~/.ironclaw/: {}",
                path, e
            )
        })?;

        // Pre-check file size before reading into memory to avoid OOM
        let file_size = std::fs::metadata(&validated)
            .map_err(|e| format!("Failed to stat attachment '{}': {}", validated.display(), e))?
            .len();
        total_bytes += file_size;
        if total_bytes > MAX_TOTAL_ATTACHMENT_BYTES {
            return Err(format!(
                "Total attachment size exceeds {} MB limit",
                MAX_TOTAL_ATTACHMENT_BYTES / (1024 * 1024)
            ));
        }

        let data = std::fs::read(&validated)
            .map_err(|e| format!("Failed to read attachment '{}': {}", validated.display(), e))?;

        let filename = validated
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file")
            .to_string();

        let mime_type = mime_from_extension(path);

        attachments.push(wit_channel::Attachment {
            filename,
            mime_type,
            data,
        });
    }

    Ok(attachments)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use secrecy::SecretString;

    use crate::channels::Channel;
    use crate::channels::OutgoingResponse;
    use crate::channels::wasm::capabilities::ChannelCapabilities;
    use crate::channels::wasm::host::{ChannelHostState, PendingWorkspaceWrite};
    use crate::channels::wasm::runtime::{
        PreparedChannelModule, WasmChannelRuntime, WasmChannelRuntimeConfig,
    };
    use crate::channels::wasm::wrapper::{
        EmitDispatchContext, HttpResponse, TELEGRAM_TEST_API_BASE_ENV, TEST_HTTP_REWRITE_MAP_ENV,
        WasmChannel, WebsocketRuntimeConfig, build_discord_gateway_presence_update,
        build_websocket_identify_message, build_websocket_resume_message,
        discord_gateway_presence_status, drain_guest_logs, parse_websocket_invalid_session,
        parse_websocket_ready_session, resolve_websocket_identify_message,
        rewrite_http_url_for_testing, should_warn_on_heartbeat_interval,
        uses_owner_broadcast_target, websocket_heartbeat_sleep_duration,
        websocket_reconnect_backoff,
    };
    use crate::pairing::PairingStore;
    use crate::secrets::{CreateSecretParams, InMemorySecretsStore, SecretsCrypto, SecretsStore};
    use crate::testing::credentials::{TEST_CRYPTO_KEY, TEST_TELEGRAM_BOT_TOKEN};
    use crate::tools::wasm::{
        Capabilities as ToolCapabilities, EndpointPattern, HttpCapability, LogLevel, ResourceLimits,
    };

    #[cfg(feature = "libsql")]
    async fn make_db_backed_pairing_store(owner_id: &str) -> (PairingStore, tempfile::TempDir) {
        use crate::db::libsql::LibSqlBackend;
        use crate::db::{Database, UserRecord};
        use crate::ownership::OwnershipCache;

        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("wrapper-pairing-test.db");
        let db = LibSqlBackend::new_local(&db_path).await.expect("libsql db");
        db.run_migrations().await.expect("migrations");

        let db: Arc<dyn Database> = Arc::new(db);
        db.get_or_create_user(UserRecord {
            id: owner_id.to_string(),
            role: "member".to_string(),
            display_name: owner_id.to_string(),
            status: "active".to_string(),
            email: None,
            last_login_at: None,
            created_by: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata: serde_json::Value::Null,
        })
        .await
        .expect("owner user");

        (PairingStore::new(db, Arc::new(OwnershipCache::new())), dir)
    }

    fn create_test_channel() -> WasmChannel {
        create_test_channel_with_owner_scope("default")
    }

    fn create_test_channel_with_owner_scope(owner_scope_id: &str) -> WasmChannel {
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
            owner_scope_id,
            "{}".to_string(),
            Arc::new(PairingStore::new_noop()),
            None,
        )
    }

    #[test]
    fn test_websocket_runtime_config_reads_capability_payload() {
        let mut tool_capabilities = ToolCapabilities::default();
        let mut http = HttpCapability::new(vec![EndpointPattern::host("gateway.discord.gg")]);
        http.credentials.insert(
            "discord_bot_token".to_string(),
            crate::secrets::CredentialMapping {
                secret_name: "discord_bot_token".to_string(),
                location: crate::secrets::CredentialLocation::Header {
                    name: "Authorization".to_string(),
                    prefix: Some("Bot ".to_string()),
                },
                host_patterns: vec!["discord.com".to_string()],
                optional: false,
            },
        );
        tool_capabilities.http = Some(http);
        tool_capabilities.websocket = Some(serde_json::json!({
            "url": "wss://gateway.discord.gg/?v=10&encoding=json",
            "connect_on_start": true,
            "identify_secret_name": "discord_bot_token",
            "identify": {
                "intents": 513,
                "properties": {
                    "os": "linux",
                    "browser": "ironclaw",
                    "device": "ironclaw"
                }
            }
        }));

        let capabilities =
            ChannelCapabilities::for_channel("discord").with_tool_capabilities(tool_capabilities);

        let config = WebsocketRuntimeConfig::from_capabilities(&capabilities)
            .expect("websocket config should be parsed");

        assert_eq!(config.url, "wss://gateway.discord.gg/?v=10&encoding=json");
        assert!(config.connect_on_start);
        assert_eq!(
            config.identify_secret_name.as_deref(),
            Some("discord_bot_token")
        );
        assert_eq!(
            config.identify,
            Some(serde_json::json!({
                "intents": 513,
                "properties": {
                    "os": "linux",
                    "browser": "ironclaw",
                    "device": "ironclaw"
                }
            }))
        );
    }

    #[test]
    fn test_build_websocket_identify_message_includes_token() {
        let identify = serde_json::json!({
            "intents": 513,
            "properties": {
                "os": "linux",
                "browser": "ironclaw",
                "device": "ironclaw"
            }
        });

        let payload = build_websocket_identify_message(&identify, "bot-token").unwrap();
        let json: serde_json::Value = serde_json::from_str(&payload).unwrap();

        assert_eq!(json["op"], serde_json::json!(2));
        assert_eq!(json["d"]["token"], serde_json::json!("bot-token"));
        assert_eq!(json["d"]["intents"], serde_json::json!(513));
    }

    /// Regression test for #2069: websocket identify must use owner_scope_id,
    /// not hardcoded "default".
    #[tokio::test]
    async fn test_resolve_websocket_identify_message_uses_owner_scope() {
        let crypto =
            Arc::new(SecretsCrypto::new(SecretString::from(TEST_CRYPTO_KEY.to_string())).unwrap());
        let store: Arc<dyn SecretsStore + Send + Sync> =
            Arc::new(InMemorySecretsStore::new(crypto));
        store
            .create(
                "owner_42",
                CreateSecretParams {
                    name: "discord_bot_token".to_string(),
                    value: SecretString::from("real_bot_token".to_string()),
                    provider: None,
                    expires_at: None,
                },
            )
            .await
            .unwrap();
        store
            .create(
                "default",
                CreateSecretParams {
                    name: "discord_bot_token".to_string(),
                    value: SecretString::from("default-bot-token".to_string()),
                    provider: None,
                    expires_at: None,
                },
            )
            .await
            .unwrap();

        let config = WebsocketRuntimeConfig {
            url: "wss://gateway.discord.gg/?v=10&encoding=json".to_string(),
            connect_on_start: true,
            identify: Some(serde_json::json!({
                "intents": 513,
                "properties": { "os": "linux", "browser": "ironclaw", "device": "ironclaw" }
            })),
            identify_secret_name: Some("discord_bot_token".to_string()),
        };

        let payload =
            resolve_websocket_identify_message(&config, Some(store.as_ref()), "owner_42").await;
        assert!(payload.is_some());
        let json: serde_json::Value = serde_json::from_str(payload.as_ref().unwrap()).unwrap();
        assert_eq!(json["d"]["token"], serde_json::json!("real_bot_token"));

        let no_payload =
            resolve_websocket_identify_message(&config, Some(store.as_ref()), "default").await;
        assert!(no_payload.is_some());
        let no_payload_json: serde_json::Value =
            serde_json::from_str(no_payload.as_ref().unwrap()).unwrap();
        assert_eq!(
            no_payload_json["d"]["token"],
            serde_json::json!("default-bot-token")
        );
    }

    #[test]
    fn test_websocket_runtime_config_requires_allowlisted_host() {
        let tool_capabilities = ToolCapabilities {
            http: Some(HttpCapability::new(vec![EndpointPattern::host(
                "discord.com",
            )])),
            websocket: Some(serde_json::json!({
                "url": "wss://gateway.discord.gg/?v=10&encoding=json",
                "connect_on_start": true
            })),
            ..Default::default()
        };

        let capabilities =
            ChannelCapabilities::for_channel("discord").with_tool_capabilities(tool_capabilities);

        assert!(WebsocketRuntimeConfig::from_capabilities(&capabilities).is_none());
    }

    #[test]
    fn test_drain_guest_logs_collects_poll_entries() {
        let mut host_state = ChannelHostState::new("poll-test", ChannelCapabilities::default());
        host_state
            .log(LogLevel::Warn, "poll warning".to_string())
            .expect("log entry should be stored");

        let logs = drain_guest_logs("poll-test", "on_poll", &mut host_state);

        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].message, "poll warning");
        assert_eq!(logs[0].level, LogLevel::Warn);
        assert!(host_state.take_logs().is_empty(), "logs should be drained");
    }

    #[test]
    fn test_websocket_reconnect_backoff_caps_at_sixty_four_seconds_with_jitter() {
        // Backoff = base + 0-25% jitter, so check range [base, base * 1.25].
        let check = |attempt: u32, base_secs: u64| {
            let d = websocket_reconnect_backoff(attempt);
            let base = Duration::from_secs(base_secs);
            let max = base + base / 4;
            assert!(
                d >= base && d <= max,
                "attempt {attempt}: {d:?} not in [{base:?}, {max:?}]"
            );
        };
        check(0, 1);
        check(1, 2);
        check(5, 32);
        check(6, 64);
        check(10, 64); // capped at 2^6
    }

    #[test]
    fn test_websocket_heartbeat_helpers_guard_low_intervals() {
        assert!(should_warn_on_heartbeat_interval(0));
        assert!(should_warn_on_heartbeat_interval(999));
        assert!(!should_warn_on_heartbeat_interval(1_000));
        assert_eq!(
            websocket_heartbeat_sleep_duration(0),
            Duration::from_millis(1)
        );
        assert_eq!(
            websocket_heartbeat_sleep_duration(42),
            Duration::from_millis(42)
        );
    }

    #[test]
    fn test_discord_gateway_presence_defaults_to_dnd() {
        let store = crate::channels::wasm::host::ChannelWorkspaceStore::new();
        let pairing_store = PairingStore::new_noop();

        assert_eq!(
            discord_gateway_presence_status("discord", &store, &pairing_store),
            "dnd"
        );
    }

    #[test]
    fn test_discord_gateway_presence_empty_owner_id_is_dnd() {
        let store = crate::channels::wasm::host::ChannelWorkspaceStore::new();
        let pairing_store = PairingStore::new_noop();
        // Simulate on_start writing empty string when no owner_id is configured
        store.commit_writes(&[PendingWorkspaceWrite {
            path: "channels/discord/state/owner_id".to_string(),
            content: String::new(),
        }]);

        assert_eq!(
            discord_gateway_presence_status("discord", &store, &pairing_store),
            "dnd"
        );
    }

    #[test]
    fn test_discord_gateway_presence_owner_id_is_online() {
        let store = crate::channels::wasm::host::ChannelWorkspaceStore::new();
        let pairing_store = PairingStore::new_noop();
        store.commit_writes(&[PendingWorkspaceWrite {
            path: "channels/discord/state/owner_id".to_string(),
            content: "owner-1".to_string(),
        }]);

        assert_eq!(
            discord_gateway_presence_status("discord", &store, &pairing_store),
            "online"
        );
    }

    #[test]
    fn test_build_discord_gateway_presence_update_uses_status() {
        let payload = build_discord_gateway_presence_update("dnd").unwrap();
        let json: serde_json::Value = serde_json::from_str(&payload).unwrap();

        assert_eq!(json["op"], serde_json::json!(3));
        assert_eq!(json["d"]["status"], serde_json::json!("dnd"));
        assert_eq!(json["d"]["afk"], serde_json::json!(false));
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

    #[test]
    fn test_inject_workspace_reader_adds_missing_reader() {
        let capabilities = ChannelCapabilities::for_channel("test");
        assert!(capabilities.tool_capabilities.workspace_read.is_none());

        let workspace_store = Arc::new(crate::channels::wasm::host::ChannelWorkspaceStore::new());
        let injected = WasmChannel::inject_workspace_reader(&capabilities, &workspace_store);

        assert!(injected.tool_capabilities.workspace_read.is_some());
        assert!(
            injected
                .tool_capabilities
                .workspace_read
                .as_ref()
                .and_then(|cap| cap.reader.as_ref())
                .is_some()
        );
    }

    #[test]
    fn test_inject_workspace_reader_preserves_allowed_prefixes() {
        let tool_capabilities = crate::tools::wasm::Capabilities::default()
            .with_workspace_read(vec!["state/".to_string(), "context/".to_string()]);
        let capabilities =
            ChannelCapabilities::for_channel("test").with_tool_capabilities(tool_capabilities);
        let workspace_store = Arc::new(crate::channels::wasm::host::ChannelWorkspaceStore::new());

        let injected = WasmChannel::inject_workspace_reader(&capabilities, &workspace_store);

        let workspace_read = injected
            .tool_capabilities
            .workspace_read
            .as_ref()
            .expect("workspace_read capability should exist");
        assert_eq!(
            workspace_read.allowed_prefixes,
            vec!["state/".to_string(), "context/".to_string()]
        );
        assert!(workspace_read.reader.is_some());
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
    async fn test_broadcast_delegates_to_call_on_broadcast() {
        let channel = create_test_channel();
        // With `component: None`, call_on_broadcast short-circuits to Ok(()).
        let result = channel
            .broadcast("146032821", OutgoingResponse::text("hello"))
            .await;
        assert!(result.is_ok());
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
            Vec::new(), // no host credentials in test
            Arc::new(PairingStore::new_noop()),
            timeout,
            &workspace_store,
        )
        .await;

        assert!(result.is_ok()); // safety: test-only assertion
        assert!(result.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_dispatch_emitted_messages_sends_to_channel() {
        use crate::channels::wasm::host::EmittedMessage;

        let (tx, mut rx) = tokio::sync::mpsc::channel(10);
        let message_tx = Arc::new(tokio::sync::RwLock::new(Some(tx)));
        let pairing_store = PairingStore::new_noop();

        let rate_limiter = Arc::new(tokio::sync::RwLock::new(
            crate::channels::wasm::host::ChannelEmitRateLimiter::new(
                crate::channels::wasm::capabilities::EmitRateLimitConfig::default(),
            ),
        ));

        let messages = vec![
            EmittedMessage::new("user1", "Hello from polling!"),
            EmittedMessage::new("user2", "Another message"),
        ];

        let last_broadcast_metadata = Arc::new(tokio::sync::RwLock::new(None));
        let result = WasmChannel::dispatch_emitted_messages(
            EmitDispatchContext {
                channel_name: "test-channel",
                owner_scope_id: "default",
                owner_actor_id: None,
                pairing_store: &pairing_store,
                message_tx: &message_tx,
                rate_limiter: &rate_limiter,
                last_broadcast_metadata: &last_broadcast_metadata,
                settings_store: None,
            },
            messages,
        )
        .await;

        assert!(result.is_ok()); // safety: test-only assertion

        // Verify messages were sent
        let msg1 = rx.try_recv().expect("Should receive first message"); // safety: test-only assertion
        assert_eq!(msg1.user_id, "user1"); // safety: test-only assertion
        assert_eq!(msg1.content, "Hello from polling!"); // safety: test-only assertion

        let msg2 = rx.try_recv().expect("Should receive second message"); // safety: test-only assertion
        assert_eq!(msg2.user_id, "user2"); // safety: test-only assertion
        assert_eq!(msg2.content, "Another message"); // safety: test-only assertion

        // No more messages
        assert!(rx.try_recv().is_err()); // safety: test-only assertion
    }

    #[tokio::test]
    async fn test_dispatch_emitted_messages_skips_empty_messages_without_attachments() {
        use crate::channels::wasm::host::EmittedMessage;

        let (tx, mut rx) = tokio::sync::mpsc::channel(10);
        let message_tx = Arc::new(tokio::sync::RwLock::new(Some(tx)));
        let pairing_store = PairingStore::new_noop();

        let rate_limiter = Arc::new(tokio::sync::RwLock::new(
            crate::channels::wasm::host::ChannelEmitRateLimiter::new(
                crate::channels::wasm::capabilities::EmitRateLimitConfig::default(),
            ),
        ));

        let messages = vec![
            EmittedMessage::new("user1", ""),
            EmittedMessage::new("user2", "   "),
            EmittedMessage::new("user3", "real message"),
        ];

        let last_broadcast_metadata = Arc::new(tokio::sync::RwLock::new(None));
        let result = WasmChannel::dispatch_emitted_messages(
            EmitDispatchContext {
                channel_name: "test-channel",
                owner_scope_id: "default",
                owner_actor_id: None,
                pairing_store: &pairing_store,
                message_tx: &message_tx,
                rate_limiter: &rate_limiter,
                last_broadcast_metadata: &last_broadcast_metadata,
                settings_store: None,
            },
            messages,
        )
        .await;

        assert!(result.is_ok()); // safety: test-only assertion

        let msg = rx
            .try_recv()
            .expect("Should receive only the non-empty message");
        assert_eq!(msg.user_id, "user3"); // safety: test-only assertion
        assert_eq!(msg.content, "real message"); // safety: test-only assertion
        assert!(rx.try_recv().is_err()); // safety: test-only assertion
    }

    #[tokio::test]
    async fn test_dispatch_emitted_messages_no_sender_returns_ok() {
        use crate::channels::wasm::host::EmittedMessage;

        // No sender available (channel not started)
        let message_tx = Arc::new(tokio::sync::RwLock::new(None));
        let pairing_store = PairingStore::new_noop();
        let rate_limiter = Arc::new(tokio::sync::RwLock::new(
            crate::channels::wasm::host::ChannelEmitRateLimiter::new(
                crate::channels::wasm::capabilities::EmitRateLimitConfig::default(),
            ),
        ));

        let messages = vec![EmittedMessage::new("user1", "Hello!")];

        // Should return Ok even without a sender (logs warning but doesn't fail)
        let last_broadcast_metadata = Arc::new(tokio::sync::RwLock::new(None));
        let result = WasmChannel::dispatch_emitted_messages(
            EmitDispatchContext {
                channel_name: "test-channel",
                owner_scope_id: "default",
                owner_actor_id: None,
                pairing_store: &pairing_store,
                message_tx: &message_tx,
                rate_limiter: &rate_limiter,
                last_broadcast_metadata: &last_broadcast_metadata,
                settings_store: None,
            },
            messages,
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
            "default",
            "{}".to_string(),
            Arc::new(PairingStore::new_noop()),
            None,
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
    async fn test_ensure_polling_replaces_task_without_leaking_previous_poller() {
        let config = WasmChannelRuntimeConfig::for_testing();
        let runtime = Arc::new(WasmChannelRuntime::new(config).unwrap());

        let prepared = Arc::new(PreparedChannelModule {
            name: "poll-channel".to_string(),
            description: "Polling test channel".to_string(),
            component: None,
            limits: ResourceLimits::default(),
        });

        let capabilities = ChannelCapabilities::for_channel("poll-channel")
            .with_path("/webhook/poll")
            .with_polling(1000);

        let channel = WasmChannel::new(
            runtime,
            prepared,
            capabilities,
            "default",
            "{}".to_string(),
            Arc::new(PairingStore::new_noop()),
            None,
        );

        let poll_config = crate::channels::wasm::schema::ChannelConfig {
            display_name: "poll-channel".to_string(),
            http_endpoints: Vec::new(),
            poll: Some(crate::channels::wasm::schema::PollConfigSchema {
                enabled: true,
                interval_ms: 1000,
            }),
        };

        channel.ensure_polling(&poll_config).await;
        assert!(channel.poll_shutdown_tx.read().await.is_some());
        assert!(channel.poll_task.read().await.is_some());

        channel.ensure_polling(&poll_config).await;
        assert!(channel.poll_shutdown_tx.read().await.is_some());
        assert!(channel.poll_task.read().await.is_some());

        channel.stop_polling().await;
        assert!(channel.poll_shutdown_tx.read().await.is_none());
        assert!(channel.poll_task.read().await.is_none());
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
                    detail: None,
                    call_id: None,
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
                    allow_always: true,
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
    async fn test_respond_paired_guest_does_not_overwrite_owner_broadcast_metadata() {
        use crate::channels::IncomingMessage;

        let channel = create_test_channel_with_owner_scope("owner-scope")
            .with_owner_actor_id(Some("telegram-owner".to_string()));
        let _stream = channel.start().await.expect("Channel should start");

        *channel.last_broadcast_metadata.write().await = Some(r#"{"chat_id":12345}"#.to_string());

        let msg = IncomingMessage::new("test", "owner-scope", "hello from guest")
            .with_sender_id("guest-77")
            .with_metadata(serde_json::json!({"chat_id": 777}));

        channel
            .respond(&msg, crate::channels::OutgoingResponse::text("response"))
            .await
            .expect("respond should succeed");

        let stored_metadata = channel.last_broadcast_metadata.read().await.clone();
        assert_eq!(stored_metadata.as_deref(), Some(r#"{"chat_id":12345}"#));

        channel.shutdown().await.expect("Shutdown should succeed");
    }

    #[tokio::test]
    async fn test_respond_owner_actor_updates_owner_broadcast_metadata() {
        use crate::channels::IncomingMessage;

        let channel = create_test_channel_with_owner_scope("owner-scope")
            .with_owner_actor_id(Some("telegram-owner".to_string()));
        let _stream = channel.start().await.expect("Channel should start");

        let msg = IncomingMessage::new("test", "owner-scope", "hello from owner")
            .with_sender_id("telegram-owner")
            .with_metadata(serde_json::json!({"chat_id": 12345}));

        channel
            .respond(&msg, crate::channels::OutgoingResponse::text("response"))
            .await
            .expect("respond should succeed");

        let stored_metadata = channel.last_broadcast_metadata.read().await.clone();
        assert_eq!(stored_metadata.as_deref(), Some(r#"{"chat_id":12345}"#));

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
        )
        .unwrap(); // safety: test

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
        )
        .unwrap(); // safety: test

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
        )
        .unwrap(); // safety: test
        assert!(matches!(wit.status, super::wit_channel::StatusType::Done));

        // with whitespace
        let wit = status_to_wit(
            &crate::channels::StatusUpdate::Status(" Done ".into()),
            &metadata,
        )
        .unwrap(); // safety: test
        assert!(matches!(wit.status, super::wit_channel::StatusType::Done));
    }

    #[test]
    fn test_status_to_wit_interrupted() {
        use super::status_to_wit;

        let metadata = serde_json::json!(null);
        let wit = status_to_wit(
            &crate::channels::StatusUpdate::Status("Interrupted".into()),
            &metadata,
        )
        .unwrap(); // safety: test

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
        )
        .unwrap(); // safety: test
        assert!(matches!(
            wit.status,
            super::wit_channel::StatusType::Interrupted
        ));

        // with whitespace
        let wit = status_to_wit(
            &crate::channels::StatusUpdate::Status(" Interrupted ".into()),
            &metadata,
        )
        .unwrap(); // safety: test
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
        )
        .unwrap(); // safety: test

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
                request_id: None,
            },
            &metadata,
        )
        .unwrap(); // safety: test

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
                detail: None,
                call_id: None,
            },
            &metadata,
        )
        .unwrap(); // safety: test

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
                error: None,
                parameters: None,
                call_id: None,
            },
            &metadata,
        )
        .unwrap(); // safety: test

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
                error: Some("connection refused".to_string()),
                parameters: None,
                call_id: None,
            },
            &metadata,
        )
        .unwrap(); // safety: test

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
                call_id: None,
            },
            &metadata,
        )
        .unwrap(); // safety: test

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
                call_id: None,
            },
            &metadata,
        )
        .unwrap(); // safety: test

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
        )
        .unwrap(); // safety: test

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
        )
        .unwrap(); // safety: test

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
        )
        .unwrap(); // safety: test

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
                allow_always: true,
            },
            &metadata,
        )
        .unwrap(); // safety: test

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
                allow_always: true,
            },
            &metadata,
        )
        .unwrap(); // safety: test

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
            TEST_TELEGRAM_BOT_TOKEN.to_string(),
        );
        creds.insert("OTHER_SECRET".to_string(), "s3cret".to_string());

        let store = ChannelStoreData::new(
            1024 * 1024,
            "test",
            ChannelCapabilities::default(),
            creds,
            Vec::new(),
            Arc::new(PairingStore::new_noop()),
        );

        let error = format!(
            "HTTP request failed: error sending request for url \
            (https://api.telegram.org/bot{TEST_TELEGRAM_BOT_TOKEN}/getUpdates)"
        );

        let redacted = store.redact_credentials(&error);

        assert!(
            !redacted.contains(TEST_TELEGRAM_BOT_TOKEN),
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
            Vec::new(),
            Arc::new(PairingStore::new_noop()),
        );

        let input = "some error message";
        assert_eq!(store.redact_credentials(input), input);
    }

    #[test]
    fn test_redact_credentials_url_encoded() {
        use super::{ChannelStoreData, ResolvedHostCredential};

        // Credential with characters that get URL-encoded
        let mut creds = std::collections::HashMap::new();
        creds.insert(
            "API_KEY".to_string(),
            "key with spaces&special=chars".to_string(),
        );

        let host_creds = vec![ResolvedHostCredential {
            host_patterns: vec!["api.example.com".to_string()],
            headers: std::collections::HashMap::new(),
            query_params: std::collections::HashMap::new(),
            secret_value: "host secret+value".to_string(),
        }];

        let store = ChannelStoreData::new(
            1024 * 1024,
            "test",
            ChannelCapabilities::default(),
            creds,
            host_creds,
            Arc::new(PairingStore::new_noop()),
        );

        // Error containing URL-encoded form of the credential
        let error = "request failed: https://api.example.com?key=key%20with%20spaces%26special%3Dchars&host=host%20secret%2Bvalue";

        let redacted = store.redact_credentials(error);

        assert!(
            !redacted.contains("key%20with%20spaces"),
            "URL-encoded credential should be redacted, got: {}",
            redacted
        );
        assert!(
            !redacted.contains("host%20secret%2Bvalue"),
            "URL-encoded host credential should be redacted, got: {}",
            redacted
        );
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
            Vec::new(),
            Arc::new(PairingStore::new_noop()),
        );

        let input = "should not match anything";
        assert_eq!(store.redact_credentials(input), input);
    }

    #[test]
    fn test_http_request_rejects_private_ip_targets() {
        let capabilities =
            ChannelCapabilities::for_channel("test").with_tool_capabilities(ToolCapabilities {
                http: Some(HttpCapability::new(vec![EndpointPattern::host(
                    "127.0.0.1",
                )])),
                ..Default::default()
            });
        let mut store = super::ChannelStoreData::new(
            1024 * 1024,
            "test",
            capabilities,
            std::collections::HashMap::new(),
            Vec::new(),
            Arc::new(PairingStore::new_noop()),
        );

        let result = super::near::agent::channel_host::Host::http_request(
            &mut store,
            "GET".to_string(),
            "https://127.0.0.1:1/health".to_string(),
            "{}".to_string(),
            None,
            Some(1_000),
        );

        assert!(result.is_err(), "loopback targets must be rejected");
        assert!(
            result.unwrap_err().contains("private/internal IP"),
            "expected SSRF guard error"
        );
    }

    #[test]
    fn test_http_request_allows_private_target_for_telegram_test_rewrite() {
        let _guard = crate::config::helpers::lock_env();
        let original = std::env::var(TELEGRAM_TEST_API_BASE_ENV).ok();
        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe {
            std::env::set_var(TELEGRAM_TEST_API_BASE_ENV, "http://127.0.0.1:1");
        }

        let capabilities =
            ChannelCapabilities::for_channel("test").with_tool_capabilities(ToolCapabilities {
                http: Some(HttpCapability::new(vec![EndpointPattern::host(
                    "api.telegram.org",
                )])),
                ..Default::default()
            });
        let mut store = super::ChannelStoreData::new(
            1024 * 1024,
            "test",
            capabilities,
            std::collections::HashMap::new(),
            Vec::new(),
            Arc::new(PairingStore::new_noop()),
        );

        let result = super::near::agent::channel_host::Host::http_request(
            &mut store,
            "GET".to_string(),
            "https://api.telegram.org/bot123/getMe".to_string(),
            "{}".to_string(),
            None,
            Some(1_000),
        );

        assert!(
            result.is_err(),
            "test rewrite should still attempt the request"
        );
        assert!(
            !result.unwrap_err().contains("private/internal IP"),
            "test rewrite should bypass private IP guard"
        );

        // SAFETY: Under ENV_MUTEX, restore original state.
        unsafe {
            if let Some(value) = original {
                std::env::set_var(TELEGRAM_TEST_API_BASE_ENV, value);
            } else {
                std::env::remove_var(TELEGRAM_TEST_API_BASE_ENV);
            }
        }
    }

    #[test]
    fn test_should_skip_response_leak_scan_only_for_telegram_getupdates() {
        use super::should_skip_response_leak_scan;

        assert!(should_skip_response_leak_scan(
            "https://api.telegram.org/bot123/getUpdates?offset=1"
        ));
        assert!(!should_skip_response_leak_scan(
            "https://api.telegram.org/bot123/sendMessage"
        ));
        assert!(!should_skip_response_leak_scan(
            "https://api.example.com/getUpdates"
        ));
        assert!(!should_skip_response_leak_scan("not a url"));
    }

    #[test]
    fn test_rewrite_telegram_api_url_for_testing_uses_test_override() {
        use super::rewrite_telegram_api_url_for_testing;

        let _guard = crate::config::helpers::lock_env();
        let original = std::env::var(TELEGRAM_TEST_API_BASE_ENV).ok();
        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe {
            std::env::set_var(TELEGRAM_TEST_API_BASE_ENV, "http://127.0.0.1:19001/");
        }

        let rewritten =
            rewrite_telegram_api_url_for_testing("https://api.telegram.org/bot123/sendMessage")
                .expect("Telegram URL should rewrite");
        assert_eq!(rewritten, "http://127.0.0.1:19001/bot123/sendMessage");

        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe {
            if let Some(value) = original {
                std::env::set_var(TELEGRAM_TEST_API_BASE_ENV, value);
            } else {
                std::env::remove_var(TELEGRAM_TEST_API_BASE_ENV);
            }
        }
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

    #[tokio::test]
    async fn test_dispatch_emitted_messages_preserves_attachments() {
        use crate::channels::wasm::host::{Attachment, EmittedMessage};

        let (tx, mut rx) = tokio::sync::mpsc::channel(10);
        let message_tx = Arc::new(tokio::sync::RwLock::new(Some(tx)));
        let pairing_store = PairingStore::new_noop();

        let rate_limiter = Arc::new(tokio::sync::RwLock::new(
            crate::channels::wasm::host::ChannelEmitRateLimiter::new(
                crate::channels::wasm::capabilities::EmitRateLimitConfig::default(),
            ),
        ));

        let attachments = vec![
            Attachment {
                id: "photo123".to_string(),
                mime_type: "image/jpeg".to_string(),
                filename: Some("cat.jpg".to_string()),
                size_bytes: Some(50_000),
                source_url: Some("https://api.telegram.org/file/photo123".to_string()),
                storage_key: None,
                extracted_text: None,
                data: Vec::new(),
                duration_secs: None,
            },
            Attachment {
                id: "doc456".to_string(),
                mime_type: "application/pdf".to_string(),
                filename: Some("report.pdf".to_string()),
                size_bytes: Some(120_000),
                source_url: None,
                storage_key: Some("store/doc456".to_string()),
                extracted_text: Some("Report contents...".to_string()),
                data: Vec::new(),
                duration_secs: None,
            },
        ];

        let messages =
            vec![EmittedMessage::new("user1", "Check these files").with_attachments(attachments)];

        let last_broadcast_metadata = Arc::new(tokio::sync::RwLock::new(None));
        let result = WasmChannel::dispatch_emitted_messages(
            EmitDispatchContext {
                channel_name: "test-channel",
                owner_scope_id: "default",
                owner_actor_id: None,
                pairing_store: &pairing_store,
                message_tx: &message_tx,
                rate_limiter: &rate_limiter,
                last_broadcast_metadata: &last_broadcast_metadata,
                settings_store: None,
            },
            messages,
        )
        .await;

        assert!(result.is_ok()); // safety: test-only assertion

        let msg = rx.try_recv().expect("Should receive message"); // safety: test-only assertion
        assert_eq!(msg.content, "Check these files"); // safety: test-only assertion
        assert_eq!(msg.attachments.len(), 2); // safety: test-only assertion

        // Verify first attachment
        assert_eq!(msg.attachments[0].id, "photo123"); // safety: test-only assertion
        assert_eq!(msg.attachments[0].mime_type, "image/jpeg"); // safety: test-only assertion
        assert_eq!(msg.attachments[0].filename, Some("cat.jpg".to_string())); // safety: test-only assertion
        assert_eq!(msg.attachments[0].size_bytes, Some(50_000)); // safety: test-only assertion
        assert_eq!(
            msg.attachments[0].source_url,
            Some("https://api.telegram.org/file/photo123".to_string())
        ); // safety: test-only assertion

        // Verify second attachment
        assert_eq!(msg.attachments[1].id, "doc456"); // safety: test-only assertion
        assert_eq!(msg.attachments[1].mime_type, "application/pdf"); // safety: test-only assertion
        assert_eq!(
            msg.attachments[1].extracted_text,
            Some("Report contents...".to_string())
        ); // safety: test-only assertion
        assert_eq!(
            msg.attachments[1].storage_key,
            Some("store/doc456".to_string())
        ); // safety: test-only assertion
    }

    #[tokio::test]
    async fn test_dispatch_emitted_messages_owner_binding_sets_owner_scope() {
        use crate::channels::wasm::host::EmittedMessage;

        let (tx, mut rx) = tokio::sync::mpsc::channel(10);
        let message_tx = Arc::new(tokio::sync::RwLock::new(Some(tx)));
        let pairing_store = PairingStore::new_noop();
        let rate_limiter = Arc::new(tokio::sync::RwLock::new(
            crate::channels::wasm::host::ChannelEmitRateLimiter::new(
                crate::channels::wasm::capabilities::EmitRateLimitConfig::default(),
            ),
        ));
        let last_broadcast_metadata = Arc::new(tokio::sync::RwLock::new(None));

        let messages = vec![
            EmittedMessage::new("telegram-owner", "Hello from owner")
                .with_metadata(r#"{"chat_id":12345}"#),
        ];

        let result = WasmChannel::dispatch_emitted_messages(
            EmitDispatchContext {
                channel_name: "telegram",
                owner_scope_id: "owner-scope",
                owner_actor_id: Some("telegram-owner"),
                pairing_store: &pairing_store,
                message_tx: &message_tx,
                rate_limiter: &rate_limiter,
                last_broadcast_metadata: &last_broadcast_metadata,
                settings_store: None,
            },
            messages,
        )
        .await;

        assert!(result.is_ok()); // safety: test-only assertion

        let msg = rx.try_recv().expect("Should receive message"); // safety: test-only assertion
        assert_eq!(msg.user_id, "owner-scope"); // safety: test-only assertion
        assert_eq!(msg.sender_id, "telegram-owner"); // safety: test-only assertion
        assert_eq!(msg.conversation_scope(), Some("12345")); // safety: test-only assertion
        let stored_metadata = last_broadcast_metadata.read().await.clone();
        assert_eq!(stored_metadata.as_deref(), Some(r#"{"chat_id":12345}"#)); // safety: test-only assertion
    }

    #[tokio::test]
    async fn test_dispatch_emitted_messages_guest_sender_stays_isolated() {
        use crate::channels::wasm::host::EmittedMessage;

        let (tx, mut rx) = tokio::sync::mpsc::channel(10);
        let message_tx = Arc::new(tokio::sync::RwLock::new(Some(tx)));
        let pairing_store = PairingStore::new_noop();
        let rate_limiter = Arc::new(tokio::sync::RwLock::new(
            crate::channels::wasm::host::ChannelEmitRateLimiter::new(
                crate::channels::wasm::capabilities::EmitRateLimitConfig::default(),
            ),
        ));
        let last_broadcast_metadata = Arc::new(tokio::sync::RwLock::new(None));

        let messages = vec![
            EmittedMessage::new("guest-42", "Hello from guest").with_metadata(r#"{"chat_id":999}"#),
        ];

        let result = WasmChannel::dispatch_emitted_messages(
            EmitDispatchContext {
                channel_name: "telegram",
                owner_scope_id: "owner-scope",
                owner_actor_id: Some("telegram-owner"),
                pairing_store: &pairing_store,
                message_tx: &message_tx,
                rate_limiter: &rate_limiter,
                last_broadcast_metadata: &last_broadcast_metadata,
                settings_store: None,
            },
            messages,
        )
        .await;

        assert!(result.is_ok()); // safety: test-only assertion

        let msg = rx.try_recv().expect("Should receive message"); // safety: test-only assertion
        assert_eq!(msg.user_id, "guest-42"); // safety: test-only assertion
        assert_eq!(msg.sender_id, "guest-42"); // safety: test-only assertion
        assert_eq!(msg.conversation_scope(), Some("999")); // safety: test-only assertion
        assert!(last_broadcast_metadata.read().await.is_none()); // safety: test-only assertion
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_dispatch_emitted_messages_paired_sender_sets_owner_scope() {
        use crate::channels::wasm::host::EmittedMessage;
        use crate::ownership::OwnerId;

        let (pairing_store, _dir) = make_db_backed_pairing_store("owner-scope").await;
        let pairing_request = pairing_store
            .upsert_request(
                "telegram",
                "guest-77",
                Some(serde_json::json!({ "chat_id": 777 })),
            )
            .await
            .expect("pairing request");
        pairing_store
            .approve(
                "telegram",
                &pairing_request.code,
                &OwnerId::from("owner-scope"),
            )
            .await
            .expect("pairing approval");

        let (tx, mut rx) = tokio::sync::mpsc::channel(10);
        let message_tx = Arc::new(tokio::sync::RwLock::new(Some(tx)));
        let rate_limiter = Arc::new(tokio::sync::RwLock::new(
            crate::channels::wasm::host::ChannelEmitRateLimiter::new(
                crate::channels::wasm::capabilities::EmitRateLimitConfig::default(),
            ),
        ));
        let last_broadcast_metadata = Arc::new(tokio::sync::RwLock::new(None));

        let result = WasmChannel::dispatch_emitted_messages(
            EmitDispatchContext {
                channel_name: "telegram",
                owner_scope_id: "owner-scope",
                owner_actor_id: Some("telegram-owner"),
                pairing_store: &pairing_store,
                message_tx: &message_tx,
                rate_limiter: &rate_limiter,
                last_broadcast_metadata: &last_broadcast_metadata,
                settings_store: None,
            },
            vec![
                EmittedMessage::new("guest-77", "Hello from paired user")
                    .with_metadata(r#"{"chat_id":777}"#),
            ],
        )
        .await;

        assert!(result.is_ok());

        let msg = rx.try_recv().expect("Should receive message");
        assert_eq!(msg.user_id, "owner-scope");
        assert_eq!(msg.sender_id, "guest-77");
        assert_eq!(msg.conversation_scope(), Some("777"));
        assert!(last_broadcast_metadata.read().await.is_none());
    }

    #[tokio::test]
    async fn test_broadcast_owner_scope_uses_stored_owner_metadata() {
        let channel = create_test_channel_with_owner_scope("owner-scope")
            .with_owner_actor_id(Some("telegram-owner".to_string()));

        *channel.last_broadcast_metadata.write().await = Some(r#"{"chat_id":12345}"#.to_string());

        let result = channel
            .broadcast(
                "owner-scope",
                crate::channels::OutgoingResponse::text("hello owner"),
            )
            .await;

        assert!(result.is_ok()); // safety: test-only assertion
    }

    #[test]
    fn test_default_target_is_not_treated_as_owner_scope() {
        assert!(!uses_owner_broadcast_target("default", "owner-scope")); // safety: test-only assertion
        assert!(uses_owner_broadcast_target("default", "default")); // safety: test-only assertion
    }

    #[tokio::test]
    async fn test_broadcast_owner_scope_requires_stored_metadata() {
        let channel = create_test_channel_with_owner_scope("owner-scope")
            .with_owner_actor_id(Some("telegram-owner".to_string()));

        let result = channel
            .broadcast(
                "owner-scope",
                crate::channels::OutgoingResponse::text("hello owner"),
            )
            .await;

        assert!(result.is_err()); // safety: test-only assertion
        let err = result.unwrap_err().to_string();
        let mentions_missing_owner_route =
            err.contains("Send a message from the owner on this channel first");
        assert!(mentions_missing_owner_route); // safety: test-only assertion
    }

    #[tokio::test]
    async fn test_dispatch_emitted_messages_no_attachments_backward_compat() {
        use crate::channels::wasm::host::EmittedMessage;

        let (tx, mut rx) = tokio::sync::mpsc::channel(10);
        let message_tx = Arc::new(tokio::sync::RwLock::new(Some(tx)));
        let pairing_store = PairingStore::new_noop();

        let rate_limiter = Arc::new(tokio::sync::RwLock::new(
            crate::channels::wasm::host::ChannelEmitRateLimiter::new(
                crate::channels::wasm::capabilities::EmitRateLimitConfig::default(),
            ),
        ));

        let messages = vec![EmittedMessage::new("user1", "Just text, no attachments")];

        let last_broadcast_metadata = Arc::new(tokio::sync::RwLock::new(None));
        let result = WasmChannel::dispatch_emitted_messages(
            EmitDispatchContext {
                channel_name: "test-channel",
                owner_scope_id: "default",
                owner_actor_id: None,
                pairing_store: &pairing_store,
                message_tx: &message_tx,
                rate_limiter: &rate_limiter,
                last_broadcast_metadata: &last_broadcast_metadata,
                settings_store: None,
            },
            messages,
        )
        .await;

        assert!(result.is_ok()); // safety: test-only assertion

        let msg = rx.try_recv().expect("Should receive message"); // safety: test-only assertion
        assert_eq!(msg.content, "Just text, no attachments"); // safety: test-only assertion
        assert!(msg.attachments.is_empty()); // safety: test-only assertion
    }

    #[test]
    fn test_parse_websocket_ready_session() {
        let ready = serde_json::json!({
            "op": 0,
            "s": 1,
            "t": "READY",
            "d": {
                "session_id": "abc123",
                "resume_gateway_url": "wss://gateway-resume.discord.gg",
                "user": {"id": "12345"}
            }
        });
        let (sid, resume_url) = parse_websocket_ready_session(&ready.to_string()).unwrap();
        assert_eq!(sid, "abc123");
        assert_eq!(
            resume_url.as_deref(),
            Some("wss://gateway-resume.discord.gg")
        );

        // Non-READY dispatch returns None
        let message_create = serde_json::json!({
            "op": 0,
            "s": 2,
            "t": "MESSAGE_CREATE",
            "d": {"content": "hello"}
        });
        assert!(parse_websocket_ready_session(&message_create.to_string()).is_none());

        // Non-dispatch opcode returns None
        let hello = serde_json::json!({"op": 10, "d": {"heartbeat_interval": 41250}});
        assert!(parse_websocket_ready_session(&hello.to_string()).is_none());
    }

    #[test]
    fn test_build_websocket_resume_message() {
        let seq = serde_json::Value::Number(42.into());
        let payload = build_websocket_resume_message("bot-token", "session-1", Some(&seq)).unwrap();
        let json: serde_json::Value = serde_json::from_str(&payload).unwrap();

        assert_eq!(json["op"], serde_json::json!(6));
        assert_eq!(json["d"]["token"], serde_json::json!("bot-token"));
        assert_eq!(json["d"]["session_id"], serde_json::json!("session-1"));
        assert_eq!(json["d"]["seq"], serde_json::json!(42));

        // With no sequence, seq should be null
        let payload_null = build_websocket_resume_message("bot-token", "session-1", None).unwrap();
        let json_null: serde_json::Value = serde_json::from_str(&payload_null).unwrap();
        assert!(json_null["d"]["seq"].is_null());
    }

    #[test]
    fn test_parse_websocket_invalid_session() {
        // Non-resumable invalid session (d: false)
        let not_resumable = serde_json::json!({"op": 9, "d": false});
        assert_eq!(
            parse_websocket_invalid_session(&not_resumable.to_string()),
            Some(false)
        );

        // Resumable invalid session (d: true)
        let resumable = serde_json::json!({"op": 9, "d": true});
        assert_eq!(
            parse_websocket_invalid_session(&resumable.to_string()),
            Some(true)
        );

        // Different opcode returns None
        let hello = serde_json::json!({"op": 10, "d": {"heartbeat_interval": 41250}});
        assert!(parse_websocket_invalid_session(&hello.to_string()).is_none());
    }

    #[test]
    fn test_mime_from_extension() {
        use super::mime_from_extension;
        assert_eq!(mime_from_extension("screenshot.png"), "image/png");
        assert_eq!(mime_from_extension("photo.JPG"), "image/jpeg");
        assert_eq!(mime_from_extension("photo.jpeg"), "image/jpeg");
        assert_eq!(mime_from_extension("animation.gif"), "image/gif");
        assert_eq!(mime_from_extension("doc.pdf"), "application/pdf");
        assert_eq!(mime_from_extension("video.mp4"), "video/mp4");
        assert_eq!(mime_from_extension("data.csv"), "text/csv");
        assert_eq!(
            mime_from_extension("unknown.qqqzzz"),
            "application/octet-stream"
        );
        assert_eq!(mime_from_extension("noext"), "application/octet-stream");
        assert_eq!(
            mime_from_extension("/home/user/.ironclaw/screenshot.png"),
            "image/png"
        );
    }

    #[test]
    fn test_rewrite_http_url_for_testing_uses_host_map() {
        use std::sync::{Mutex, OnceLock};

        static ENV_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();
        let _lock = ENV_MUTEX
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("env mutex poisoned");

        let original = std::env::var(TEST_HTTP_REWRITE_MAP_ENV).ok();

        // SAFETY: guarded by ENV_MUTEX — no concurrent env access.
        unsafe {
            std::env::set_var(
                TEST_HTTP_REWRITE_MAP_ENV,
                r#"{"slack.com":"http://localhost:9999","files.slack.com":"http://localhost:9999"}"#,
            );
        }

        // slack.com API call
        let result = rewrite_http_url_for_testing("https://slack.com/api/chat.postMessage");
        assert_eq!(
            result.as_deref(),
            Some("http://localhost:9999/api/chat.postMessage")
        );
        // files.slack.com file download
        let result = rewrite_http_url_for_testing(
            "https://files.slack.com/files-pri/T123/download/test.txt",
        );
        assert_eq!(
            result.as_deref(),
            Some("http://localhost:9999/files-pri/T123/download/test.txt")
        );
        // Non-Slack URL should not be rewritten
        let result = rewrite_http_url_for_testing("https://api.telegram.org/bot123/getMe");
        assert!(result.is_none());

        // SAFETY: guarded by ENV_MUTEX — restore original state.
        unsafe {
            if let Some(ref val) = original {
                std::env::set_var(TEST_HTTP_REWRITE_MAP_ENV, val);
            } else {
                std::env::remove_var(TEST_HTTP_REWRITE_MAP_ENV);
            }
        }
    }

    #[test]
    fn resolve_message_scope_recognizes_owner() {
        let (user_id, is_owner) = super::resolve_message_scope("default", Some("12345"), "12345");
        assert_eq!(user_id, "default");
        assert!(is_owner);
    }

    #[test]
    fn resolve_message_scope_non_owner() {
        let (user_id, is_owner) = super::resolve_message_scope("default", Some("12345"), "99999");
        assert_eq!(user_id, "99999");
        assert!(!is_owner);
    }

    #[test]
    fn resolve_message_scope_no_owner_configured() {
        let (user_id, is_owner) = super::resolve_message_scope("default", None, "12345");
        assert_eq!(user_id, "12345");
        assert!(!is_owner);
    }
}
