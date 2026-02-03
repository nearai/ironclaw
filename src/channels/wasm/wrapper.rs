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
use wasmtime::component::{Component, Linker, Val};

use crate::channels::wasm::capabilities::ChannelCapabilities;
use crate::channels::wasm::error::WasmChannelError;
use crate::channels::wasm::host::{ChannelEmitRateLimiter, ChannelHostState, EmittedMessage};
use crate::channels::wasm::router::RegisteredEndpoint;
use crate::channels::wasm::runtime::{PreparedChannelModule, WasmChannelRuntime};
use crate::channels::wasm::schema::ChannelConfig;
use crate::channels::{Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate};
use crate::error::ChannelError;
use crate::tools::wasm::LogLevel;
use crate::tools::wasm::WasmResourceLimiter;

/// Store data for WASM channel execution.
///
/// Contains the resource limiter and channel-specific host state.
struct ChannelStoreData {
    limiter: WasmResourceLimiter,
    host_state: ChannelHostState,
}

impl ChannelStoreData {
    fn new(memory_limit: u64, channel_name: &str, capabilities: ChannelCapabilities) -> Self {
        Self {
            limiter: WasmResourceLimiter::new(memory_limit),
            host_state: ChannelHostState::new(channel_name, capabilities),
        }
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
    config_json: String,

    /// Channel configuration returned by on_start.
    channel_config: RwLock<Option<ChannelConfig>>,

    /// Message sender (for emitting messages to the stream).
    message_tx: RwLock<Option<mpsc::Sender<IncomingMessage>>>,

    /// Pending responses (for synchronous response handling).
    pending_responses: RwLock<HashMap<Uuid, oneshot::Sender<String>>>,

    /// Rate limiter for message emission.
    rate_limiter: RwLock<ChannelEmitRateLimiter>,

    /// Shutdown signal sender.
    shutdown_tx: RwLock<Option<oneshot::Sender<()>>>,

    /// Registered HTTP endpoints.
    endpoints: RwLock<Vec<RegisteredEndpoint>>,
}

impl WasmChannel {
    /// Create a new WASM channel.
    pub fn new(
        runtime: Arc<WasmChannelRuntime>,
        prepared: Arc<PreparedChannelModule>,
        capabilities: ChannelCapabilities,
        config_json: String,
    ) -> Self {
        let name = prepared.name.clone();
        let rate_limiter = ChannelEmitRateLimiter::new(capabilities.emit_rate_limit.clone());

        Self {
            name,
            runtime,
            prepared,
            capabilities,
            config_json,
            channel_config: RwLock::new(None),
            message_tx: RwLock::new(None),
            pending_responses: RwLock::new(HashMap::new()),
            rate_limiter: RwLock::new(rate_limiter),
            shutdown_tx: RwLock::new(None),
            endpoints: RwLock::new(Vec::new()),
        }
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

    /// Add channel host functions to the linker.
    ///
    /// These functions are imported by the WASM channel module.
    fn add_host_functions(linker: &mut Linker<ChannelStoreData>) -> Result<(), WasmChannelError> {
        // host.log(level: log-level, message: string)
        linker
            .root()
            .func_wrap(
                "log",
                |mut ctx: wasmtime::StoreContextMut<'_, ChannelStoreData>,
                 (level, message): (i32, String)| {
                    let log_level = match level {
                        0 => LogLevel::Trace,
                        1 => LogLevel::Debug,
                        2 => LogLevel::Info,
                        3 => LogLevel::Warn,
                        4 => LogLevel::Error,
                        _ => LogLevel::Info,
                    };
                    // Ignore errors from logging (rate limiting)
                    let _ = ctx.data_mut().host_state.log(log_level, message);
                    Ok(())
                },
            )
            .map_err(|e| WasmChannelError::Config(format!("Failed to add log function: {}", e)))?;

        // host.now-millis() -> u64
        linker
            .root()
            .func_wrap(
                "now-millis",
                |ctx: wasmtime::StoreContextMut<'_, ChannelStoreData>,
                 (): ()|
                 -> anyhow::Result<(u64,)> {
                    Ok((ctx.data().host_state.now_millis(),))
                },
            )
            .map_err(|e| {
                WasmChannelError::Config(format!("Failed to add now-millis function: {}", e))
            })?;

        // host.workspace-read(path: string) -> option<string>
        linker
            .root()
            .func_wrap(
                "workspace-read",
                |ctx: wasmtime::StoreContextMut<'_, ChannelStoreData>,
                 (path,): (String,)|
                 -> anyhow::Result<(Option<String>,)> {
                    let result = ctx.data().host_state.workspace_read(&path).ok().flatten();
                    Ok((result,))
                },
            )
            .map_err(|e| {
                WasmChannelError::Config(format!("Failed to add workspace-read function: {}", e))
            })?;

        // host.workspace-write(path: string, content: string) -> result<_, string>
        linker
            .root()
            .func_wrap(
                "workspace-write",
                |mut ctx: wasmtime::StoreContextMut<'_, ChannelStoreData>,
                 (path, content): (String, String)|
                 -> anyhow::Result<(Result<(), String>,)> {
                    let result = ctx
                        .data_mut()
                        .host_state
                        .workspace_write(&path, content)
                        .map_err(|e| e.to_string());
                    Ok((result,))
                },
            )
            .map_err(|e| {
                WasmChannelError::Config(format!("Failed to add workspace-write function: {}", e))
            })?;

        // host.emit-message(msg: emitted-message)
        // The message is passed as a record with fields: user-id, user-name, content, thread-id, metadata-json
        linker
            .root()
            .func_wrap(
                "emit-message",
                |mut ctx: wasmtime::StoreContextMut<'_, ChannelStoreData>,
                 (user_id, user_name, content, thread_id, metadata_json): (
                    String,
                    Option<String>,
                    String,
                    Option<String>,
                    String,
                )| {
                    let mut msg = EmittedMessage::new(user_id, content);
                    if let Some(name) = user_name {
                        msg = msg.with_user_name(name);
                    }
                    if let Some(tid) = thread_id {
                        msg = msg.with_thread_id(tid);
                    }
                    msg = msg.with_metadata(metadata_json);

                    // Ignore errors (rate limiting just drops messages)
                    let _ = ctx.data_mut().host_state.emit_message(msg);
                    Ok(())
                },
            )
            .map_err(|e| {
                WasmChannelError::Config(format!("Failed to add emit-message function: {}", e))
            })?;

        // host.secret-exists(name: string) -> bool
        linker
            .root()
            .func_wrap(
                "secret-exists",
                |ctx: wasmtime::StoreContextMut<'_, ChannelStoreData>,
                 (name,): (String,)|
                 -> anyhow::Result<(bool,)> {
                    Ok((ctx.data().host_state.secret_exists(&name),))
                },
            )
            .map_err(|e| {
                WasmChannelError::Config(format!("Failed to add secret-exists function: {}", e))
            })?;

        Ok(())
    }

    /// Execute a WASM callback synchronously (called from spawn_blocking).
    ///
    /// This is the core execution logic shared by all callbacks.
    fn execute_callback_sync<F, R>(
        runtime: &WasmChannelRuntime,
        prepared: &PreparedChannelModule,
        capabilities: &ChannelCapabilities,
        export_name: &str,
        build_args: F,
    ) -> Result<(R, ChannelHostState), WasmChannelError>
    where
        F: FnOnce() -> (
            Vec<Val>,
            Box<dyn FnOnce(&Val) -> Result<R, WasmChannelError>>,
        ),
    {
        let engine = runtime.engine();
        let limits = &prepared.limits;

        // Create fresh store with channel state (NEAR pattern: fresh instance per call)
        let store_data =
            ChannelStoreData::new(limits.memory_bytes, &prepared.name, capabilities.clone());
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

        // Compile the component (uses cached bytes)
        let component = Component::new(engine, prepared.component_bytes())
            .map_err(|e| WasmChannelError::Compilation(e.to_string()))?;

        // Create linker and add host functions
        let mut linker = Linker::new(engine);
        Self::add_host_functions(&mut linker)?;

        // Instantiate the component
        let instance = linker
            .instantiate(&mut store, &component)
            .map_err(|e| WasmChannelError::Instantiation(e.to_string()))?;

        // Get the export function
        let func = instance
            .get_func(&mut store, export_name)
            .ok_or_else(|| WasmChannelError::MissingExport(export_name.to_string()))?;

        // Build arguments and result extractor
        let (args, extract_result) = build_args();

        // Call the function
        let mut results = vec![Val::Bool(false)]; // Placeholder
        func.call(&mut store, &args, &mut results).map_err(|e| {
            let error_str = e.to_string();
            if error_str.contains("out of fuel") {
                WasmChannelError::FuelExhausted {
                    name: prepared.name.clone(),
                    limit: limits.fuel,
                }
            } else if error_str.contains("unreachable") {
                WasmChannelError::Trapped {
                    name: prepared.name.clone(),
                    reason: "unreachable code executed".to_string(),
                }
            } else {
                WasmChannelError::Trapped {
                    name: prepared.name.clone(),
                    reason: error_str,
                }
            }
        })?;

        // Post-call completion (cleanup)
        func.post_return(&mut store)
            .map_err(|e| WasmChannelError::Trapped {
                name: prepared.name.clone(),
                reason: format!("post_return failed: {}", e),
            })?;

        // Extract result
        let result = extract_result(&results[0])?;

        // Get host state with emitted messages and pending writes
        let host_state = std::mem::replace(
            &mut store.data_mut().host_state,
            ChannelHostState::new(&prepared.name, capabilities.clone()),
        );

        Ok((result, host_state))
    }

    /// Execute the on_start callback.
    ///
    /// Returns the channel configuration for HTTP endpoint registration.
    async fn call_on_start(&self) -> Result<ChannelConfig, WasmChannelError> {
        // If no WASM bytes, return default config (for testing)
        if self.prepared.component_bytes.is_empty() {
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
        let capabilities = self.capabilities.clone();
        let config_json = self.config_json.clone();
        let timeout = self.runtime.config().callback_timeout;
        let channel_name_for_error = self.name.clone();

        // Execute in blocking task with timeout
        let result = tokio::time::timeout(timeout, async move {
            tokio::task::spawn_blocking(move || {
                Self::execute_callback_sync(&runtime, &prepared, &capabilities, "on-start", || {
                    let args = vec![Val::String(config_json)];
                    let extract = Box::new(|result: &Val| extract_channel_config(result));
                    (args, extract)
                })
            })
            .await
            .map_err(|e| WasmChannelError::ExecutionPanicked {
                name: channel_name_for_error.clone(),
                reason: e.to_string(),
            })?
        })
        .await;

        match result {
            Ok(Ok((config, _host_state))) => {
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
        // If no WASM bytes, return 200 OK (for testing)
        if self.prepared.component_bytes.is_empty() {
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
        let capabilities = self.capabilities.clone();
        let timeout = self.runtime.config().callback_timeout;

        // Prepare request data
        let method = method.to_string();
        let path = path.to_string();
        let headers_json = serde_json::to_string(&headers).unwrap_or_default();
        let query_json = serde_json::to_string(&query).unwrap_or_default();
        let body = body.to_vec();

        // Clone name for error handling before moving into closure
        let channel_name_for_error = self.name.clone();

        // Execute in blocking task with timeout
        let result = tokio::time::timeout(timeout, async move {
            tokio::task::spawn_blocking(move || {
                Self::execute_callback_sync(
                    &runtime,
                    &prepared,
                    &capabilities,
                    "on-http-request",
                    || {
                        // Build incoming-http-request record
                        let request = Val::Record(vec![
                            ("method".to_string(), Val::String(method)),
                            ("path".to_string(), Val::String(path)),
                            ("headers-json".to_string(), Val::String(headers_json)),
                            ("query-json".to_string(), Val::String(query_json)),
                            (
                                "body".to_string(),
                                Val::List(body.into_iter().map(Val::U8).collect()),
                            ),
                            ("secret-validated".to_string(), Val::Bool(secret_validated)),
                        ]);
                        let args = vec![request];
                        let extract = Box::new(|result: &Val| extract_http_response(result));
                        (args, extract)
                    },
                )
            })
            .await
            .map_err(|e| WasmChannelError::ExecutionPanicked {
                name: channel_name_for_error.clone(),
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
        if self.prepared.component_bytes.is_empty() {
            tracing::debug!(
                channel = %self.name,
                "WASM channel on_poll called (no WASM module)"
            );
            return Ok(());
        }

        let runtime = Arc::clone(&self.runtime);
        let prepared = Arc::clone(&self.prepared);
        let capabilities = self.capabilities.clone();
        let timeout = self.runtime.config().callback_timeout;
        let channel_name = self.name.clone();

        // Execute in blocking task with timeout
        let result = tokio::time::timeout(timeout, async move {
            tokio::task::spawn_blocking(move || {
                Self::execute_callback_sync(&runtime, &prepared, &capabilities, "on-poll", || {
                    let args = vec![];
                    let extract = Box::new(|_result: &Val| Ok(()));
                    (args, extract)
                })
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
        // If no WASM bytes, do nothing (for testing)
        if self.prepared.component_bytes.is_empty() {
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

        // Prepare response data
        let message_id_str = message_id.to_string();
        let content = content.to_string();
        let thread_id = thread_id.map(|s| s.to_string());
        let metadata_json = metadata_json.to_string();

        // Execute in blocking task with timeout
        let result = tokio::time::timeout(timeout, async move {
            tokio::task::spawn_blocking(move || {
                Self::execute_callback_sync(
                    &runtime,
                    &prepared,
                    &capabilities,
                    "on-respond",
                    || {
                        // Build agent-response record
                        let response = Val::Record(vec![
                            ("message-id".to_string(), Val::String(message_id_str)),
                            ("content".to_string(), Val::String(content)),
                            (
                                "thread-id".to_string(),
                                match thread_id {
                                    Some(tid) => Val::Option(Some(Box::new(Val::String(tid)))),
                                    None => Val::Option(None),
                                },
                            ),
                            ("metadata-json".to_string(), Val::String(metadata_json)),
                        ]);
                        let args = vec![response];
                        let extract = Box::new(|result: &Val| extract_result_unit(result));
                        (args, extract)
                    },
                )
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

    /// Process emitted messages from a callback.
    async fn process_emitted_messages(
        &self,
        messages: Vec<EmittedMessage>,
    ) -> Result<(), WasmChannelError> {
        if messages.is_empty() {
            return Ok(());
        }

        let tx_guard = self.message_tx.read().await;
        let Some(tx) = tx_guard.as_ref() else {
            tracing::warn!(
                channel = %self.name,
                count = messages.len(),
                "Messages emitted but no sender available"
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
            if tx.send(msg).await.is_err() {
                tracing::warn!(
                    channel = %self.name,
                    "Failed to send emitted message, channel closed"
                );
                break;
            }
        }

        Ok(())
    }

    /// Start the polling loop if configured.
    fn start_polling(&self, interval: Duration, shutdown_rx: oneshot::Receiver<()>) {
        let channel_name = self.name.clone();

        // Clone self reference for the async block
        // In a real implementation, we'd hold an Arc<Self>
        tokio::spawn(async move {
            let mut interval_timer = tokio::time::interval(interval);
            let mut shutdown = std::pin::pin!(shutdown_rx);

            loop {
                tokio::select! {
                    _ = interval_timer.tick() => {
                        tracing::debug!(
                            channel = %channel_name,
                            "Polling tick (stub - would call on_poll)"
                        );
                        // In real implementation: self.call_on_poll().await
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
        if let Some(poll_config) = &config.poll {
            if poll_config.enabled {
                let interval = self
                    .capabilities
                    .validate_poll_interval(poll_config.interval_ms)
                    .map_err(|e| ChannelError::StartupFailed {
                        name: self.name.clone(),
                        reason: e,
                    })?;

                // Create a new shutdown receiver for polling
                let (_poll_shutdown_tx, poll_shutdown_rx) = oneshot::channel();

                self.start_polling(Duration::from_millis(interval as u64), poll_shutdown_rx);
            }
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
        // Check if there's a pending synchronous response waiter
        if let Some(tx) = self.pending_responses.write().await.remove(&msg.id) {
            let _ = tx.send(response.content.clone());
        }

        // Call WASM on_respond
        let metadata_json = serde_json::to_string(&response.metadata).unwrap_or_default();
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

    async fn send_status(&self, status: StatusUpdate) -> Result<(), ChannelError> {
        // WASM channels don't support status updates by default
        // Could be extended with an optional on_status callback
        let _ = status;
        Ok(())
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
        // Send shutdown signal
        if let Some(tx) = self.shutdown_tx.write().await.take() {
            let _ = tx.send(());
        }

        // Clear the message sender
        *self.message_tx.write().await = None;

        // TODO: Call WASM on_shutdown if we add that callback

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
// Value Extraction Helpers
// ============================================================================

/// Extract ChannelConfig from a WIT result<channel-config, string>.
fn extract_channel_config(val: &Val) -> Result<ChannelConfig, WasmChannelError> {
    // Result is (ok: option<channel-config>, err: option<string>)
    match val {
        Val::Result(result) => match result.as_ref() {
            Ok(Some(config_val)) => extract_channel_config_inner(config_val),
            Ok(None) => Err(WasmChannelError::InvalidResponse(
                "on-start returned empty Ok".to_string(),
            )),
            Err(Some(err_val)) => {
                if let Val::String(err) = err_val.as_ref() {
                    Err(WasmChannelError::CallbackFailed {
                        name: "channel".to_string(),
                        reason: err.clone(),
                    })
                } else {
                    Err(WasmChannelError::InvalidResponse(
                        "on-start error is not a string".to_string(),
                    ))
                }
            }
            Err(None) => Err(WasmChannelError::InvalidResponse(
                "on-start returned empty Err".to_string(),
            )),
        },
        // Fallback: try to parse as record directly (for simpler implementations)
        Val::Record(_) => extract_channel_config_inner(val),
        _ => Err(WasmChannelError::InvalidResponse(format!(
            "Expected result or record, got {:?}",
            std::mem::discriminant(val)
        ))),
    }
}

/// Extract ChannelConfig from a channel-config record.
fn extract_channel_config_inner(val: &Val) -> Result<ChannelConfig, WasmChannelError> {
    match val {
        Val::Record(fields) => {
            let mut display_name = String::new();
            let mut http_endpoints = Vec::new();
            let mut poll = None;

            for (name, field_val) in fields {
                match name.as_str() {
                    "display-name" => {
                        if let Val::String(s) = field_val {
                            display_name = s.clone();
                        }
                    }
                    "http-endpoints" => {
                        if let Val::List(endpoints) = field_val {
                            for ep in endpoints {
                                if let Ok(endpoint) = extract_http_endpoint_config(ep) {
                                    http_endpoints.push(endpoint);
                                }
                            }
                        }
                    }
                    "poll" => {
                        if let Val::Option(Some(poll_val)) = field_val {
                            poll = extract_poll_config(poll_val).ok();
                        }
                    }
                    _ => {}
                }
            }

            Ok(ChannelConfig {
                display_name,
                http_endpoints,
                poll,
            })
        }
        _ => Err(WasmChannelError::InvalidResponse(
            "Expected record for channel-config".to_string(),
        )),
    }
}

/// Extract HttpEndpointConfigSchema from a record.
fn extract_http_endpoint_config(
    val: &Val,
) -> Result<crate::channels::wasm::schema::HttpEndpointConfigSchema, WasmChannelError> {
    match val {
        Val::Record(fields) => {
            let mut path = String::new();
            let mut methods = Vec::new();
            let mut require_secret = false;

            for (name, field_val) in fields {
                match name.as_str() {
                    "path" => {
                        if let Val::String(s) = field_val {
                            path = s.clone();
                        }
                    }
                    "methods" => {
                        if let Val::List(list) = field_val {
                            for item in list {
                                if let Val::String(s) = item {
                                    methods.push(s.clone());
                                }
                            }
                        }
                    }
                    "require-secret" => {
                        if let Val::Bool(b) = field_val {
                            require_secret = *b;
                        }
                    }
                    _ => {}
                }
            }

            Ok(crate::channels::wasm::schema::HttpEndpointConfigSchema {
                path,
                methods,
                require_secret,
            })
        }
        _ => Err(WasmChannelError::InvalidResponse(
            "Expected record for http-endpoint-config".to_string(),
        )),
    }
}

/// Extract PollConfigSchema from a record.
fn extract_poll_config(
    val: &Val,
) -> Result<crate::channels::wasm::schema::PollConfigSchema, WasmChannelError> {
    match val {
        Val::Record(fields) => {
            let mut interval_ms = 30_000;
            let mut enabled = false;

            for (name, field_val) in fields {
                match name.as_str() {
                    "interval-ms" => {
                        if let Val::U32(n) = field_val {
                            interval_ms = *n;
                        }
                    }
                    "enabled" => {
                        if let Val::Bool(b) = field_val {
                            enabled = *b;
                        }
                    }
                    _ => {}
                }
            }

            Ok(crate::channels::wasm::schema::PollConfigSchema {
                interval_ms,
                enabled,
            })
        }
        _ => Err(WasmChannelError::InvalidResponse(
            "Expected record for poll-config".to_string(),
        )),
    }
}

/// Extract HttpResponse from a WIT outgoing-http-response record.
fn extract_http_response(val: &Val) -> Result<HttpResponse, WasmChannelError> {
    match val {
        Val::Record(fields) => {
            let mut status = 200u16;
            let mut headers = HashMap::new();
            let mut body = Vec::new();

            for (name, field_val) in fields {
                match name.as_str() {
                    "status" => {
                        if let Val::U16(s) = field_val {
                            status = *s;
                        }
                    }
                    "headers-json" => {
                        if let Val::String(s) = field_val {
                            if let Ok(h) = serde_json::from_str::<HashMap<String, String>>(s) {
                                headers = h;
                            }
                        }
                    }
                    "body" => {
                        if let Val::List(bytes) = field_val {
                            body = bytes
                                .iter()
                                .filter_map(|v| if let Val::U8(b) = v { Some(*b) } else { None })
                                .collect();
                        }
                    }
                    _ => {}
                }
            }

            Ok(HttpResponse {
                status,
                headers,
                body,
            })
        }
        _ => Err(WasmChannelError::InvalidResponse(
            "Expected record for http-response".to_string(),
        )),
    }
}

/// Extract unit result from a WIT result<_, string>.
fn extract_result_unit(val: &Val) -> Result<(), WasmChannelError> {
    match val {
        Val::Result(result) => match result.as_ref() {
            Ok(_) => Ok(()),
            Err(Some(err_val)) => {
                if let Val::String(err) = err_val.as_ref() {
                    Err(WasmChannelError::CallbackFailed {
                        name: "channel".to_string(),
                        reason: err.clone(),
                    })
                } else {
                    Err(WasmChannelError::InvalidResponse(
                        "Error is not a string".to_string(),
                    ))
                }
            }
            Err(None) => Err(WasmChannelError::InvalidResponse(
                "Returned empty Err".to_string(),
            )),
        },
        // Unit return (for on-poll which returns nothing)
        Val::Tuple(items) if items.is_empty() => Ok(()),
        _ => Ok(()), // Treat anything else as success for unit-returning callbacks
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
    use crate::tools::wasm::ResourceLimits;

    fn create_test_channel() -> WasmChannel {
        let config = WasmChannelRuntimeConfig::for_testing();
        let runtime = Arc::new(WasmChannelRuntime::new(config).unwrap());

        let prepared = Arc::new(PreparedChannelModule {
            name: "test".to_string(),
            description: "Test channel".to_string(),
            component_bytes: Vec::new(),
            limits: ResourceLimits::default(),
        });

        let capabilities = ChannelCapabilities::for_channel("test").with_path("/webhook/test");

        WasmChannel::new(runtime, prepared, capabilities, "{}".to_string())
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
}
