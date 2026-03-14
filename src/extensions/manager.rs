//! Central extension manager that dispatches operations by ExtensionKind.
//!
//! Holds references to channel runtime, WASM tool runtime, MCP infrastructure,
//! secrets store, and tool registry. All extension operations (search, install,
//! auth, activate, list, remove) flow through here.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::channels::ChannelManager;
use crate::channels::wasm::{
    RegisteredEndpoint, SharedWasmChannel, WasmChannelLoader, WasmChannelRouter, WasmChannelRuntime,
};
use crate::extensions::discovery::OnlineDiscovery;
use crate::extensions::registry::ExtensionRegistry;
use crate::extensions::{
    ActivateResult, AuthResult, ConfigureResult, ExtensionError, ExtensionKind, ExtensionSource,
    InstallResult, InstalledExtension, RegistryEntry, ResultSource, SearchResult, ToolAuthState,
    UpgradeOutcome, UpgradeResult,
};
use crate::hooks::HookRegistry;
use crate::pairing::PairingStore;
use crate::secrets::{CreateSecretParams, SecretsStore};
use crate::tools::ToolRegistry;
use crate::tools::mcp::McpClient;
use crate::tools::mcp::auth::{
    authorize_mcp_server, canonical_resource_uri, discover_full_oauth_metadata,
    find_available_port, is_authenticated, register_client,
};
use crate::tools::mcp::config::McpServerConfig;
use crate::tools::mcp::session::McpSessionManager;
use crate::tools::wasm::{WasmToolLoader, WasmToolRuntime, discover_tools};

/// Pending OAuth authorization state.
struct PendingAuth {
    _name: String,
    _kind: ExtensionKind,
    created_at: std::time::Instant,
    /// Background task listening for the OAuth callback.
    /// Aborted when a new auth flow starts for the same extension.
    task_handle: Option<tokio::task::JoinHandle<()>>,
}

/// Runtime infrastructure needed for hot-activating WASM channels.
///
/// Set after construction via [`ExtensionManager::set_channel_runtime`] once the
/// channel manager, WASM runtime, pairing store, and webhook router are available.
struct ChannelRuntimeState {
    channel_manager: Arc<ChannelManager>,
    wasm_channel_runtime: Arc<WasmChannelRuntime>,
    pairing_store: Arc<PairingStore>,
    wasm_channel_router: Arc<WasmChannelRouter>,
    wasm_channel_owner_ids: std::collections::HashMap<String, i64>,
}

/// Central manager for extension lifecycle operations.
pub struct ExtensionManager {
    registry: ExtensionRegistry,
    discovery: OnlineDiscovery,

    // MCP infrastructure
    mcp_session_manager: Arc<McpSessionManager>,
    mcp_process_manager: Arc<crate::tools::mcp::process::McpProcessManager>,
    /// Active MCP clients keyed by server name.
    mcp_clients: RwLock<HashMap<String, Arc<McpClient>>>,

    // WASM tool infrastructure
    wasm_tool_runtime: Option<Arc<WasmToolRuntime>>,
    wasm_tools_dir: PathBuf,
    wasm_channels_dir: PathBuf,

    // WASM channel hot-activation infrastructure (set post-construction)
    channel_runtime: RwLock<Option<ChannelRuntimeState>>,
    /// Channel manager for hot-adding relay channels (set independently of WASM runtime).
    relay_channel_manager: RwLock<Option<Arc<ChannelManager>>>,

    // Shared
    secrets: Arc<dyn SecretsStore + Send + Sync>,
    tool_registry: Arc<ToolRegistry>,
    hooks: Option<Arc<HookRegistry>>,
    pending_auth: RwLock<HashMap<String, PendingAuth>>,
    /// Tunnel URL for webhook configuration and remote OAuth callbacks.
    tunnel_url: Option<String>,
    user_id: String,
    /// Optional database store for DB-backed MCP config.
    store: Option<Arc<dyn crate::db::Database>>,
    /// Names of WASM channels that were successfully loaded at startup.
    active_channel_names: RwLock<HashSet<String>>,
    /// Installed channel-relay extensions (no on-disk artifact, tracked in memory).
    installed_relay_extensions: RwLock<HashSet<String>>,
    /// Last activation error for each WASM channel (ephemeral, cleared on success).
    activation_errors: RwLock<HashMap<String, String>>,
    /// SSE broadcast sender (set post-construction via `set_sse_sender()`).
    sse_sender:
        RwLock<Option<tokio::sync::broadcast::Sender<crate::channels::web::types::SseEvent>>>,
    /// Shared registry of pending OAuth flows for gateway-routed callbacks.
    ///
    /// Keyed by CSRF `state` parameter. Populated in `start_wasm_oauth()`
    /// when running in gateway mode, consumed by the web gateway's
    /// `/oauth/callback` handler.
    pending_oauth_flows: crate::cli::oauth_defaults::PendingOAuthRegistry,
    /// Gateway auth token for authenticating with the platform token exchange proxy.
    /// Read once at construction from `GATEWAY_AUTH_TOKEN` env var.
    gateway_token: Option<String>,
    /// Relay config captured at startup. Used by `auth_channel_relay` and
    /// `activate_channel_relay` instead of re-reading env vars.
    relay_config: Option<crate::config::RelayConfig>,
    /// When `true`, OAuth flows always return an auth URL to the caller
    /// instead of opening a browser on the server via `open::that()`.
    /// Set by the web gateway at startup via `enable_gateway_mode()`.
    gateway_mode: std::sync::atomic::AtomicBool,
    /// The gateway's own base URL for building OAuth redirect URIs.
    /// Set by the web gateway at startup via `enable_gateway_mode()`.
    gateway_base_url: RwLock<Option<String>>,
}

/// Sanitize a URL for logging by removing query parameters and credentials.
/// Prevents accidental logging of API keys, OAuth tokens, or other sensitive data in URLs.
fn sanitize_url_for_logging(url: &str) -> String {
    // If URL is very short or doesn't look like a URL, just use as-is
    if url.len() < 10 || !url.contains("://") {
        return url.to_string();
    }

    // Try to parse and remove sensitive components
    if let Ok(mut parsed) = url::Url::parse(url) {
        // Remove query string and fragment
        parsed.set_query(None);
        parsed.set_fragment(None);

        // Remove userinfo (username and password) if present
        let _ = parsed.set_username("");
        let _ = parsed.set_password(None);

        parsed.to_string()
    } else {
        // Fallback: strip after ? or #
        url.split(['?', '#']).next().unwrap_or(url).to_string()
    }
}

impl ExtensionManager {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        mcp_session_manager: Arc<McpSessionManager>,
        mcp_process_manager: Arc<crate::tools::mcp::process::McpProcessManager>,
        secrets: Arc<dyn SecretsStore + Send + Sync>,
        tool_registry: Arc<ToolRegistry>,
        hooks: Option<Arc<HookRegistry>>,
        wasm_tool_runtime: Option<Arc<WasmToolRuntime>>,
        wasm_tools_dir: PathBuf,
        wasm_channels_dir: PathBuf,
        tunnel_url: Option<String>,
        user_id: String,
        store: Option<Arc<dyn crate::db::Database>>,
        catalog_entries: Vec<RegistryEntry>,
    ) -> Self {
        let registry = if catalog_entries.is_empty() {
            ExtensionRegistry::new()
        } else {
            ExtensionRegistry::new_with_catalog(catalog_entries)
        };
        Self {
            registry,
            discovery: OnlineDiscovery::new(),
            mcp_session_manager,
            mcp_process_manager,
            mcp_clients: RwLock::new(HashMap::new()),
            wasm_tool_runtime,
            wasm_tools_dir,
            wasm_channels_dir,
            channel_runtime: RwLock::new(None),
            relay_channel_manager: RwLock::new(None),
            secrets,
            tool_registry,
            hooks,
            pending_auth: RwLock::new(HashMap::new()),
            tunnel_url,
            user_id,
            store,
            active_channel_names: RwLock::new(HashSet::new()),
            installed_relay_extensions: RwLock::new(HashSet::new()),
            activation_errors: RwLock::new(HashMap::new()),
            sse_sender: RwLock::new(None),
            pending_oauth_flows: crate::cli::oauth_defaults::new_pending_oauth_registry(),
            gateway_token: std::env::var("GATEWAY_AUTH_TOKEN").ok(),
            relay_config: crate::config::RelayConfig::from_env(),
            gateway_mode: std::sync::atomic::AtomicBool::new(false),
            gateway_base_url: RwLock::new(None),
        }
    }

    /// Enable gateway mode so OAuth flows return auth URLs to the frontend
    /// instead of calling `open::that()` on the server.
    ///
    /// `base_url` is the gateway's own public URL (e.g. `https://my-gateway.example.com`),
    /// used to build OAuth redirect URIs when `IRONCLAW_OAUTH_CALLBACK_URL` is not set.
    pub async fn enable_gateway_mode(&self, base_url: String) {
        self.gateway_mode
            .store(true, std::sync::atomic::Ordering::Release);
        *self.gateway_base_url.write().await = Some(base_url);
    }

    /// Returns `true` if OAuth should use gateway mode (return auth URL to
    /// frontend) rather than CLI mode (open browser on server via `open::that`).
    ///
    /// Gateway mode is active when any of:
    /// - `enable_gateway_mode()` was called (web gateway is running), OR
    /// - `IRONCLAW_OAUTH_CALLBACK_URL` is set to a non-loopback URL, OR
    /// - `self.tunnel_url` is set to a non-loopback URL
    pub fn should_use_gateway_mode(&self) -> bool {
        if self.gateway_mode.load(std::sync::atomic::Ordering::Acquire) {
            return true;
        }
        if crate::cli::oauth_defaults::use_gateway_callback() {
            return true;
        }
        self.tunnel_url
            .as_ref()
            .filter(|u| !u.is_empty())
            .and_then(|raw| url::Url::parse(raw).ok())
            .and_then(|u| u.host_str().map(String::from))
            .map(|host| !crate::cli::oauth_defaults::is_loopback_host(&host))
            .unwrap_or(false)
    }

    /// Returns the OAuth redirect URI for gateway mode, or `None` for local mode.
    ///
    /// Priority:
    /// 1. `IRONCLAW_OAUTH_CALLBACK_URL` env var (via `callback_url()`)
    /// 2. `gateway_base_url` (set by `enable_gateway_mode()`)
    /// 3. `tunnel_url` (from config)
    /// 4. `None` (local/CLI mode)
    async fn gateway_callback_redirect_uri(&self) -> Option<String> {
        use crate::cli::oauth_defaults;
        if oauth_defaults::use_gateway_callback() {
            return Some(format!("{}/oauth/callback", oauth_defaults::callback_url()));
        }
        // Use gateway_base_url from enable_gateway_mode()
        if let Some(ref base) = *self.gateway_base_url.read().await {
            let base = base.trim_end_matches('/');
            return Some(format!("{}/oauth/callback", base));
        }
        // Fall back to tunnel_url
        self.tunnel_url
            .as_ref()
            .filter(|u| !u.is_empty())
            .and_then(|raw| url::Url::parse(raw).ok())
            .and_then(|u| u.host_str().map(String::from))
            .filter(|host| !oauth_defaults::is_loopback_host(host))
            .map(|_| {
                let base = self.tunnel_url.as_ref().unwrap().trim_end_matches('/');
                format!("{}/oauth/callback", base)
            })
    }

    /// Get the relay config stored at startup.
    fn relay_config(&self) -> Result<&crate::config::RelayConfig, ExtensionError> {
        self.relay_config.as_ref().ok_or_else(|| {
            ExtensionError::Config(
                "CHANNEL_RELAY_URL and CHANNEL_RELAY_API_KEY must be set".to_string(),
            )
        })
    }

    /// Inject a registry entry for testing. The entry is added to the discovery
    /// cache so it appears in search results alongside built-in entries.
    pub async fn inject_registry_entry(&self, entry: crate::extensions::RegistryEntry) {
        self.registry.cache_discovered(vec![entry]).await;
    }

    /// Configure the channel runtime infrastructure for hot-activating WASM channels.
    ///
    /// Call after construction (and after wrapping in `Arc`) once the channel
    /// manager, WASM runtime, pairing store, and webhook router are available.
    /// Without this, channel activation returns an error.
    pub async fn set_channel_runtime(
        &self,
        channel_manager: Arc<ChannelManager>,
        wasm_channel_runtime: Arc<WasmChannelRuntime>,
        pairing_store: Arc<PairingStore>,
        wasm_channel_router: Arc<WasmChannelRouter>,
        wasm_channel_owner_ids: std::collections::HashMap<String, i64>,
    ) {
        // Also store the channel manager for relay channel activation.
        *self.relay_channel_manager.write().await = Some(Arc::clone(&channel_manager));
        *self.channel_runtime.write().await = Some(ChannelRuntimeState {
            channel_manager,
            wasm_channel_runtime,
            pairing_store,
            wasm_channel_router,
            wasm_channel_owner_ids,
        });
    }

    /// Set just the channel manager for relay channel hot-activation.
    ///
    /// Call this when WASM channel runtime is not available but relay channels
    /// still need to be hot-added.
    pub async fn set_relay_channel_manager(&self, channel_manager: Arc<ChannelManager>) {
        *self.relay_channel_manager.write().await = Some(channel_manager);
    }

    async fn current_channel_owner_id(&self, name: &str) -> Option<i64> {
        {
            let rt_guard = self.channel_runtime.read().await;
            if let Some(owner_id) = rt_guard
                .as_ref()
                .and_then(|rt| rt.wasm_channel_owner_ids.get(name).copied())
            {
                return Some(owner_id);
            }
        }

        let store = self.store.as_ref()?;
        let key = format!("channels.wasm_channel_owner_ids.{name}");
        match store.get_setting(&self.user_id, &key).await {
            Ok(Some(serde_json::Value::Number(n))) => n.as_i64(),
            Ok(Some(serde_json::Value::String(s))) => s.parse::<i64>().ok(),
            Ok(Some(_)) | Ok(None) => None,
            Err(e) => {
                tracing::debug!(
                    channel = %name,
                    error = %e,
                    "Failed to read persisted wasm channel owner id"
                );
                None
            }
        }
    }

    /// Check if a channel name corresponds to a relay extension (has stored stream token).
    pub async fn is_relay_channel(&self, name: &str) -> bool {
        self.secrets
            .exists(&self.user_id, &format!("relay:{}:stream_token", name))
            .await
            .unwrap_or(false)
    }

    /// Restore persisted relay channels after startup.
    ///
    /// Loads the persisted active channel list, filters to relay types (those with
    /// a stored stream token), and activates each via `activate_stored_relay()`.
    /// Skips channels that are already active. Call this after `set_relay_channel_manager()`.
    pub async fn restore_relay_channels(&self) {
        let persisted = self.load_persisted_active_channels().await;
        let already_active = self.active_channel_names.read().await.clone();

        for name in &persisted {
            if already_active.contains(name) {
                continue;
            }
            if !self.is_relay_channel(name).await {
                continue;
            }
            match self.activate_stored_relay(name).await {
                Ok(_) => {
                    tracing::debug!(channel = %name, "Restored persisted relay channel");
                }
                Err(e) => {
                    tracing::warn!(
                        channel = %name,
                        error = %e,
                        "Failed to restore persisted relay channel"
                    );
                }
            }
        }
    }

    /// Access the secrets store (used by OAuth callback handlers).
    pub fn secrets(&self) -> &Arc<dyn SecretsStore + Send + Sync> {
        &self.secrets
    }

    /// Register channel names that were loaded at startup.
    /// Called after WASM channels are loaded so `list()` reports accurate active status.
    pub async fn set_active_channels(&self, names: Vec<String>) {
        let mut active = self.active_channel_names.write().await;
        active.extend(names);
    }

    /// Persist the set of active channel names to the settings store.
    ///
    /// Saved under key `activated_channels` so channels auto-activate on restart.
    async fn persist_active_channels(&self) {
        let Some(ref store) = self.store else {
            return;
        };
        let names: Vec<String> = self
            .active_channel_names
            .read()
            .await
            .iter()
            .cloned()
            .collect();
        let value = serde_json::json!(names);
        if let Err(e) = store
            .set_setting(&self.user_id, "activated_channels", &value)
            .await
        {
            tracing::warn!(error = %e, "Failed to persist activated_channels setting");
        }
    }

    /// Load previously activated channel names from the settings store.
    ///
    /// Returns channel names that were activated in a prior session so they can
    /// be auto-activated at startup.
    pub async fn load_persisted_active_channels(&self) -> Vec<String> {
        let Some(ref store) = self.store else {
            return Vec::new();
        };
        match store.get_setting(&self.user_id, "activated_channels").await {
            Ok(Some(value)) => match serde_json::from_value(value) {
                Ok(names) => names,
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to deserialize activated_channels");
                    Vec::new()
                }
            },
            Ok(None) => Vec::new(),
            Err(e) => {
                tracing::warn!(error = %e, "Failed to load activated_channels setting");
                Vec::new()
            }
        }
    }

    /// Set the SSE broadcast sender for pushing extension status events to the web UI.
    pub async fn set_sse_sender(
        &self,
        sender: tokio::sync::broadcast::Sender<crate::channels::web::types::SseEvent>,
    ) {
        *self.sse_sender.write().await = Some(sender);
    }

    /// Returns the pending OAuth flow registry for sharing with the web gateway.
    ///
    /// The gateway's `/oauth/callback` handler uses this to look up pending flows
    /// by CSRF `state` parameter and complete the token exchange.
    pub fn pending_oauth_flows(&self) -> &crate::cli::oauth_defaults::PendingOAuthRegistry {
        &self.pending_oauth_flows
    }

    /// Broadcast an extension status change to the web UI via SSE.
    async fn broadcast_extension_status(&self, name: &str, status: &str, message: Option<&str>) {
        if let Some(ref sender) = *self.sse_sender.read().await {
            let _ = sender.send(crate::channels::web::types::SseEvent::ExtensionStatus {
                extension_name: name.to_string(),
                status: status.to_string(),
                message: message.map(|m| m.to_string()),
            });
        }
    }

    /// Search for extensions. If `discover` is true, also searches online.
    pub async fn search(
        &self,
        query: &str,
        discover: bool,
    ) -> Result<Vec<SearchResult>, ExtensionError> {
        let mut results = self.registry.search(query).await;

        if discover && results.is_empty() {
            tracing::info!("No built-in results for '{}', searching online...", query);
            let discovered = self.discovery.discover(query).await;

            if !discovered.is_empty() {
                // Cache for future lookups
                self.registry.cache_discovered(discovered.clone()).await;

                // Add to results
                for entry in discovered {
                    results.push(SearchResult {
                        entry,
                        source: ResultSource::Discovered,
                        validated: true,
                    });
                }
            }
        }

        Ok(results)
    }

    /// Install an extension by name (from registry) or by explicit URL.
    pub async fn install(
        &self,
        name: &str,
        url: Option<&str>,
        kind_hint: Option<ExtensionKind>,
    ) -> Result<InstallResult, ExtensionError> {
        let sanitized_url = url.map(sanitize_url_for_logging);
        tracing::info!(extension = %name, url = ?sanitized_url, kind = ?kind_hint, "Installing extension");
        Self::validate_extension_name(name)?;

        // If we have a registry entry, use it (prefer kind_hint to resolve collisions)
        if let Some(entry) = self.registry.get_with_kind(name, kind_hint).await {
            return self.install_from_entry(&entry).await.map_err(|e| {
                tracing::error!(extension = %name, error = %e, "Extension install failed");
                e
            });
        }

        // If a URL was provided, determine kind and install
        if let Some(url) = url {
            let kind = kind_hint.unwrap_or_else(|| infer_kind_from_url(url));
            return match kind {
                ExtensionKind::McpServer => self.install_mcp_from_url(name, url).await,
                ExtensionKind::WasmTool => self.install_wasm_tool_from_url(name, url).await,
                ExtensionKind::WasmChannel => {
                    self.install_wasm_channel_from_url(name, url, None).await
                }
                ExtensionKind::ChannelRelay => {
                    // ChannelRelay extensions are installed from registry, not by URL
                    Err(ExtensionError::InstallFailed(
                        "Channel relay extensions cannot be installed by URL".to_string(),
                    ))
                }
            }
            .map_err(|e| {
                let sanitized = sanitize_url_for_logging(url);
                tracing::error!(extension = %name, url = %sanitized, error = %e, "Extension install from URL failed");
                e
            });
        }

        let err = ExtensionError::NotFound(format!(
            "'{}' not found in registry. Try searching with discover:true or provide a URL.",
            name
        ));
        tracing::warn!(extension = %name, "Extension not found in registry");
        Err(err)
    }

    /// Check auth status for an installed extension.
    ///
    /// Read-only for WASM extensions; may initiate OAuth for MCP servers.
    /// To provide secrets, use [`configure()`] instead.
    pub async fn auth(&self, name: &str) -> Result<AuthResult, ExtensionError> {
        // Clean up expired pending auths
        self.cleanup_expired_auths().await;

        // Determine what kind of extension this is
        let kind = self.determine_installed_kind(name).await?;

        match kind {
            ExtensionKind::McpServer => self.auth_mcp(name).await,
            ExtensionKind::WasmTool => self.auth_wasm_tool(name).await,
            ExtensionKind::WasmChannel => self.auth_wasm_channel_status(name).await,
            ExtensionKind::ChannelRelay => self.auth_channel_relay(name).await,
        }
    }

    /// Activate an installed (and optionally authenticated) extension.
    pub async fn activate(&self, name: &str) -> Result<ActivateResult, ExtensionError> {
        Self::validate_extension_name(name)?;
        let kind = self.determine_installed_kind(name).await?;

        match kind {
            ExtensionKind::McpServer => self.activate_mcp(name).await,
            ExtensionKind::WasmTool => self.activate_wasm_tool(name).await,
            ExtensionKind::WasmChannel => self.activate_wasm_channel(name).await,
            ExtensionKind::ChannelRelay => self.activate_channel_relay(name).await,
        }
    }

    /// List extensions with their status.
    ///
    /// When `include_available` is `true`, registry entries that are not yet
    /// installed are appended with `installed: false`.
    pub async fn list(
        &self,
        kind_filter: Option<ExtensionKind>,
        include_available: bool,
    ) -> Result<Vec<InstalledExtension>, ExtensionError> {
        let mut extensions = Vec::new();

        // List MCP servers
        if kind_filter.is_none() || kind_filter == Some(ExtensionKind::McpServer) {
            match self.load_mcp_servers().await {
                Ok(servers) => {
                    for server in &servers.servers {
                        let authenticated =
                            is_authenticated(server, &self.secrets, &self.user_id).await;
                        let clients = self.mcp_clients.read().await;
                        let active = clients.contains_key(&server.name);

                        // Get tool names if active
                        let tools = if active {
                            self.tool_registry
                                .list()
                                .await
                                .into_iter()
                                .filter(|t| t.starts_with(&format!("{}_", server.name)))
                                .collect()
                        } else {
                            Vec::new()
                        };

                        let display_name = self
                            .registry
                            .get_with_kind(&server.name, Some(ExtensionKind::McpServer))
                            .await
                            .map(|e| e.display_name);
                        extensions.push(InstalledExtension {
                            name: server.name.clone(),
                            kind: ExtensionKind::McpServer,
                            display_name,
                            description: server.description.clone(),
                            url: Some(server.url.clone()),
                            authenticated,
                            active,
                            tools,
                            needs_setup: false,
                            has_auth: false,
                            installed: true,
                            activation_error: None,
                            version: None,
                        });
                    }
                }
                Err(e) => {
                    tracing::debug!("Failed to load MCP servers for listing: {}", e);
                }
            }
        }

        // List WASM tools
        if (kind_filter.is_none() || kind_filter == Some(ExtensionKind::WasmTool))
            && self.wasm_tools_dir.exists()
        {
            match discover_tools(&self.wasm_tools_dir).await {
                Ok(tools) => {
                    for (name, discovered) in tools {
                        let active = self.tool_registry.has(&name).await;

                        let registry_entry = self
                            .registry
                            .get_with_kind(&name, Some(ExtensionKind::WasmTool))
                            .await;
                        let display_name = registry_entry.as_ref().map(|e| e.display_name.clone());
                        let auth_state = self.check_tool_auth_status(&name).await;
                        let version = if let Some(ref cap_path) = discovered.capabilities_path {
                            tokio::fs::read(cap_path)
                                .await
                                .ok()
                                .and_then(|bytes| {
                                    crate::tools::wasm::CapabilitiesFile::from_bytes(&bytes).ok()
                                })
                                .and_then(|cap| cap.version)
                        } else {
                            None
                        };
                        let version =
                            version.or_else(|| registry_entry.and_then(|e| e.version.clone()));
                        extensions.push(InstalledExtension {
                            name: name.clone(),
                            kind: ExtensionKind::WasmTool,
                            display_name,
                            description: None,
                            url: None,
                            authenticated: auth_state == ToolAuthState::Ready,
                            active,
                            tools: if active { vec![name] } else { Vec::new() },
                            needs_setup: auth_state == ToolAuthState::NeedsSetup,
                            has_auth: auth_state != ToolAuthState::NoAuth,
                            installed: true,
                            activation_error: None,
                            version,
                        });
                    }
                }
                Err(e) => {
                    tracing::debug!("Failed to discover WASM tools for listing: {}", e);
                }
            }
        }

        // List WASM channels
        if (kind_filter.is_none() || kind_filter == Some(ExtensionKind::WasmChannel))
            && self.wasm_channels_dir.exists()
        {
            match crate::channels::wasm::discover_channels(&self.wasm_channels_dir).await {
                Ok(channels) => {
                    let active_names = self.active_channel_names.read().await;
                    let errors = self.activation_errors.read().await;
                    for (name, discovered) in channels {
                        let active = active_names.contains(&name);
                        let auth_state = self.check_channel_auth_status(&name).await;
                        let activation_error = errors.get(&name).cloned();
                        let registry_entry = self
                            .registry
                            .get_with_kind(&name, Some(ExtensionKind::WasmChannel))
                            .await;
                        let display_name = registry_entry.as_ref().map(|e| e.display_name.clone());
                        let version = if let Some(ref cap_path) = discovered.capabilities_path {
                            tokio::fs::read(cap_path)
                                .await
                                .ok()
                                .and_then(|bytes| {
                                    crate::channels::wasm::ChannelCapabilitiesFile::from_bytes(
                                        &bytes,
                                    )
                                    .ok()
                                })
                                .and_then(|cap| cap.version)
                        } else {
                            None
                        };
                        let version =
                            version.or_else(|| registry_entry.and_then(|e| e.version.clone()));
                        extensions.push(InstalledExtension {
                            name,
                            kind: ExtensionKind::WasmChannel,
                            display_name,
                            description: None,
                            url: None,
                            authenticated: auth_state == ToolAuthState::Ready,
                            active,
                            tools: Vec::new(),
                            needs_setup: auth_state == ToolAuthState::NeedsSetup,
                            has_auth: false,
                            installed: true,
                            activation_error,
                            version,
                        });
                    }
                }
                Err(e) => {
                    tracing::debug!("Failed to discover WASM channels for listing: {}", e);
                }
            }
        }

        // List channel-relay extensions
        if kind_filter.is_none() || kind_filter == Some(ExtensionKind::ChannelRelay) {
            let installed = self.installed_relay_extensions.read().await;
            let active_names = self.active_channel_names.read().await;
            for name in installed.iter() {
                let active = active_names.contains(name);
                let has_token = self
                    .secrets
                    .exists(&self.user_id, &format!("relay:{}:stream_token", name))
                    .await
                    .unwrap_or(false);
                let registry_entry = self
                    .registry
                    .get_with_kind(name, Some(ExtensionKind::ChannelRelay))
                    .await;
                let display_name = registry_entry.as_ref().map(|e| e.display_name.clone());
                let description = registry_entry.as_ref().map(|e| e.description.clone());
                extensions.push(InstalledExtension {
                    name: name.clone(),
                    kind: ExtensionKind::ChannelRelay,
                    display_name,
                    description,
                    url: None,
                    authenticated: has_token,
                    active,
                    tools: Vec::new(),
                    needs_setup: false,
                    has_auth: true,
                    installed: true,
                    activation_error: None,
                    version: None,
                });
            }
        }

        // Append available-but-not-installed registry entries
        if include_available {
            let installed_names: std::collections::HashSet<(String, ExtensionKind)> = extensions
                .iter()
                .map(|e| (e.name.clone(), e.kind))
                .collect();

            for entry in self.registry.all_entries().await {
                if let Some(filter) = kind_filter
                    && entry.kind != filter
                {
                    continue;
                }
                if installed_names.contains(&(entry.name.clone(), entry.kind)) {
                    continue;
                }
                extensions.push(InstalledExtension {
                    name: entry.name,
                    kind: entry.kind,
                    display_name: Some(entry.display_name),
                    description: Some(entry.description),
                    url: None,
                    authenticated: false,
                    active: false,
                    tools: Vec::new(),
                    needs_setup: false,
                    has_auth: false,
                    installed: false,
                    activation_error: None,
                    version: entry.version,
                });
            }
        }

        Ok(extensions)
    }

    /// Remove an installed extension.
    pub async fn remove(&self, name: &str) -> Result<String, ExtensionError> {
        Self::validate_extension_name(name)?;
        let kind = self.determine_installed_kind(name).await?;

        // Clean up any in-progress OAuth flows for this extension.
        // TCP mode: abort the listener task so port 9876 is freed immediately.
        // Gateway mode: remove stale pending flow entries.
        if let Some(pending) = self.pending_auth.write().await.remove(name)
            && let Some(handle) = pending.task_handle
        {
            handle.abort();
        }
        self.pending_oauth_flows
            .write()
            .await
            .retain(|_, flow| flow.extension_name != name);

        match kind {
            ExtensionKind::McpServer => {
                // Unregister tools with this server's prefix
                let tool_names: Vec<String> = self
                    .tool_registry
                    .list()
                    .await
                    .into_iter()
                    .filter(|t| t.starts_with(&format!("{}_", name)))
                    .collect();

                for tool_name in &tool_names {
                    self.tool_registry.unregister(tool_name).await;
                }

                // Remove MCP client
                self.mcp_clients.write().await.remove(name);

                // Remove from config
                self.remove_mcp_server(name)
                    .await
                    .map_err(|e| ExtensionError::Config(e.to_string()))?;

                Ok(format!(
                    "Removed MCP server '{}' and {} tool(s)",
                    name,
                    tool_names.len()
                ))
            }
            ExtensionKind::WasmTool => {
                // Unregister from tool registry
                self.tool_registry.unregister(name).await;

                // Evict compiled module from runtime cache so reinstall uses fresh binary
                if let Some(ref rt) = self.wasm_tool_runtime {
                    rt.remove(name).await;
                }

                // Clear stale activation errors so reinstall starts clean
                self.activation_errors.write().await.remove(name);

                // Revoke credential mappings from the shared registry
                let cap_path = self
                    .wasm_tools_dir
                    .join(format!("{}.capabilities.json", name));
                self.revoke_credential_mappings(&cap_path).await;

                // Unregister hooks registered from this plugin source.
                let removed_hooks = self
                    .unregister_hook_prefix(&format!("plugin.tool:{}::", name))
                    .await
                    + self
                        .unregister_hook_prefix(&format!("plugin.dev_tool:{}::", name))
                        .await;
                if removed_hooks > 0 {
                    tracing::info!(
                        extension = name,
                        removed_hooks = removed_hooks,
                        "Removed plugin hooks for WASM tool"
                    );
                }

                // Delete files
                let wasm_path = self.wasm_tools_dir.join(format!("{}.wasm", name));

                if wasm_path.exists() {
                    tokio::fs::remove_file(&wasm_path)
                        .await
                        .map_err(|e| ExtensionError::Other(e.to_string()))?;
                }
                if cap_path.exists() {
                    let _ = tokio::fs::remove_file(&cap_path).await;
                }

                Ok(format!("Removed WASM tool '{}'", name))
            }
            ExtensionKind::WasmChannel => {
                // Remove from active set and persist
                self.active_channel_names.write().await.remove(name);
                self.persist_active_channels().await;

                // Clear stale activation errors so reinstall starts clean
                self.activation_errors.write().await.remove(name);

                // Delete channel files
                let wasm_path = self.wasm_channels_dir.join(format!("{}.wasm", name));
                let cap_path = self
                    .wasm_channels_dir
                    .join(format!("{}.capabilities.json", name));

                // Revoke credential mappings before deleting the capabilities file
                self.revoke_credential_mappings(&cap_path).await;

                if wasm_path.exists() {
                    tokio::fs::remove_file(&wasm_path)
                        .await
                        .map_err(|e| ExtensionError::Other(e.to_string()))?;
                }
                if cap_path.exists() {
                    let _ = tokio::fs::remove_file(&cap_path).await;
                }

                Ok(format!(
                    "Removed channel '{}'. Restart IronClaw for the change to take effect.",
                    name
                ))
            }
            ExtensionKind::ChannelRelay => {
                // Remove from installed set
                self.installed_relay_extensions.write().await.remove(name);

                // Remove from active channels
                self.active_channel_names.write().await.remove(name);
                self.persist_active_channels().await;

                // Remove stored stream token
                let _ = self
                    .secrets
                    .delete(&self.user_id, &format!("relay:{}:stream_token", name))
                    .await;

                // Shut down the channel (check both runtime paths for WASM+relay and relay-only modes)
                let mut shut_down = false;
                if let Some(ref rt) = *self.channel_runtime.read().await
                    && let Some(channel) = rt.channel_manager.get_channel(name).await
                {
                    let _ = channel.shutdown().await;
                    shut_down = true;
                }
                if !shut_down
                    && let Some(ref cm) = *self.relay_channel_manager.read().await
                    && let Some(channel) = cm.get_channel(name).await
                {
                    let _ = channel.shutdown().await;
                }

                Ok(format!("Removed channel relay '{}'", name))
            }
        }
    }

    /// Upgrade installed WASM extensions to match the current host WIT version.
    ///
    /// If `name` is `Some`, upgrades only that extension.  If `None`, checks all
    /// installed WASM tools and channels and upgrades any that are outdated.
    ///
    /// The upgrade preserves authentication secrets — only the `.wasm` binary
    /// (and `.capabilities.json`) are replaced.
    pub async fn upgrade(&self, name: Option<&str>) -> Result<UpgradeResult, ExtensionError> {
        // Collect extensions to check
        let mut candidates: Vec<(String, ExtensionKind)> = Vec::new();

        if let Some(name) = name {
            Self::validate_extension_name(name)?;
            let kind = self.determine_installed_kind(name).await?;
            if kind == ExtensionKind::McpServer {
                return Err(ExtensionError::Other(
                    "MCP servers don't have WIT versions and cannot be upgraded this way"
                        .to_string(),
                ));
            }
            candidates.push((name.to_string(), kind));
        } else {
            // Discover all installed WASM tools
            if self.wasm_tools_dir.exists()
                && let Ok(tools) = discover_tools(&self.wasm_tools_dir).await
            {
                for (tool_name, _) in tools {
                    candidates.push((tool_name, ExtensionKind::WasmTool));
                }
            }
            // Discover all installed WASM channels
            if self.wasm_channels_dir.exists()
                && let Ok(channels) =
                    crate::channels::wasm::discover_channels(&self.wasm_channels_dir).await
            {
                for (ch_name, _) in channels {
                    candidates.push((ch_name, ExtensionKind::WasmChannel));
                }
            }
        }

        if candidates.is_empty() {
            return Ok(UpgradeResult {
                results: Vec::new(),
                message: "No WASM extensions installed.".to_string(),
            });
        }

        let mut outcomes = Vec::new();

        for (ext_name, kind) in &candidates {
            let outcome = self.upgrade_one(ext_name, *kind).await;
            outcomes.push(outcome);
        }

        let upgraded = outcomes.iter().filter(|o| o.status == "upgraded").count();
        let up_to_date = outcomes
            .iter()
            .filter(|o| o.status == "already_up_to_date")
            .count();
        let failed = outcomes.iter().filter(|o| o.status == "failed").count();

        let message = format!(
            "{} extension(s) checked: {} upgraded, {} already up to date, {} failed",
            outcomes.len(),
            upgraded,
            up_to_date,
            failed
        );

        Ok(UpgradeResult {
            results: outcomes,
            message,
        })
    }

    /// Upgrade a single WASM extension if its WIT version is outdated.
    async fn upgrade_one(&self, name: &str, kind: ExtensionKind) -> UpgradeOutcome {
        let (cap_dir, host_wit) = match kind {
            ExtensionKind::WasmTool => (&self.wasm_tools_dir, crate::tools::wasm::WIT_TOOL_VERSION),
            ExtensionKind::WasmChannel => (
                &self.wasm_channels_dir,
                crate::tools::wasm::WIT_CHANNEL_VERSION,
            ),
            ExtensionKind::McpServer | ExtensionKind::ChannelRelay => {
                return UpgradeOutcome {
                    name: name.to_string(),
                    kind,
                    status: "failed".to_string(),
                    detail: "This extension type cannot be upgraded this way".to_string(),
                };
            }
        };

        // Read current WIT version from capabilities
        let cap_path = cap_dir.join(format!("{}.capabilities.json", name));
        let declared_wit = if cap_path.exists() {
            match tokio::fs::read(&cap_path).await {
                Ok(bytes) => {
                    let wit: Option<String> = match kind {
                        ExtensionKind::WasmTool => {
                            crate::tools::wasm::CapabilitiesFile::from_bytes(&bytes)
                                .ok()
                                .and_then(|c| c.wit_version)
                        }
                        ExtensionKind::WasmChannel => {
                            crate::channels::wasm::ChannelCapabilitiesFile::from_bytes(&bytes)
                                .ok()
                                .and_then(|c| c.wit_version)
                        }
                        ExtensionKind::McpServer | ExtensionKind::ChannelRelay => None,
                    };
                    wit
                }
                Err(_) => None,
            }
        } else {
            None
        };

        // Check if upgrade is needed
        let needs_upgrade =
            crate::tools::wasm::check_wit_version_compat(name, declared_wit.as_deref(), host_wit)
                .is_err();

        if !needs_upgrade {
            return UpgradeOutcome {
                name: name.to_string(),
                kind,
                status: "already_up_to_date".to_string(),
                detail: format!(
                    "WIT {} matches host WIT {}",
                    declared_wit.as_deref().unwrap_or("unknown"),
                    host_wit
                ),
            };
        }

        // Check registry for a newer version
        let entry = self.registry.get_with_kind(name, Some(kind)).await;
        let Some(entry) = entry else {
            return UpgradeOutcome {
                name: name.to_string(),
                kind,
                status: "not_in_registry".to_string(),
                detail: format!(
                    "Extension '{}' has outdated WIT {} (host: {}), \
                     but is not in the registry. Reinstall manually with a URL.",
                    name,
                    declared_wit.as_deref().unwrap_or("unknown"),
                    host_wit
                ),
            };
        };

        // Delete old .wasm file (keep secrets intact)
        let wasm_path = cap_dir.join(format!("{}.wasm", name));
        if wasm_path.exists()
            && let Err(e) = tokio::fs::remove_file(&wasm_path).await
        {
            return UpgradeOutcome {
                name: name.to_string(),
                kind,
                status: "failed".to_string(),
                detail: format!("Failed to remove old WASM binary: {}", e),
            };
        }
        // Also remove old capabilities so install_from_entry can write the new one
        if cap_path.exists() {
            let _ = tokio::fs::remove_file(&cap_path).await;
        }

        // Reinstall from registry
        match self.install_from_entry(&entry).await {
            Ok(_) => {
                tracing::info!(
                    extension = %name,
                    old_wit = ?declared_wit,
                    new_host_wit = %host_wit,
                    "Upgraded WASM extension"
                );
                UpgradeOutcome {
                    name: name.to_string(),
                    kind,
                    status: "upgraded".to_string(),
                    detail: format!(
                        "Upgraded from WIT {} to host WIT {}. Restart to activate.",
                        declared_wit.as_deref().unwrap_or("unknown"),
                        host_wit
                    ),
                }
            }
            Err(e) => UpgradeOutcome {
                name: name.to_string(),
                kind,
                status: "failed".to_string(),
                detail: format!("Reinstall failed: {}. Old files were removed.", e),
            },
        }
    }

    /// Get detailed info about an installed extension (version, wit_version, host compatibility).
    pub async fn extension_info(&self, name: &str) -> Result<serde_json::Value, ExtensionError> {
        Self::validate_extension_name(name)?;
        let kind = self.determine_installed_kind(name).await?;

        match kind {
            ExtensionKind::WasmTool => {
                let cap_path = self
                    .wasm_tools_dir
                    .join(format!("{}.capabilities.json", name));
                let wasm_path = self.wasm_tools_dir.join(format!("{}.wasm", name));

                let mut info = serde_json::json!({
                    "name": name,
                    "kind": "wasm_tool",
                    "installed": wasm_path.exists(),
                });

                if cap_path.exists()
                    && let Ok(bytes) = tokio::fs::read(&cap_path).await
                    && let Ok(cap) = crate::tools::wasm::CapabilitiesFile::from_bytes(&bytes)
                {
                    info["version"] =
                        serde_json::json!(cap.version.unwrap_or_else(|| "unknown".into()));
                    info["wit_version"] =
                        serde_json::json!(cap.wit_version.unwrap_or_else(|| "unknown".into()));
                }

                info["host_wit_version"] = serde_json::json!(crate::tools::wasm::WIT_TOOL_VERSION);

                Ok(info)
            }
            ExtensionKind::WasmChannel => {
                let cap_path = self
                    .wasm_channels_dir
                    .join(format!("{}.capabilities.json", name));
                let wasm_path = self.wasm_channels_dir.join(format!("{}.wasm", name));

                let mut info = serde_json::json!({
                    "name": name,
                    "kind": "wasm_channel",
                    "installed": wasm_path.exists(),
                    "active": self.active_channel_names.read().await.contains(name),
                });

                if cap_path.exists()
                    && let Ok(bytes) = tokio::fs::read(&cap_path).await
                    && let Ok(cap) =
                        crate::channels::wasm::ChannelCapabilitiesFile::from_bytes(&bytes)
                {
                    info["version"] =
                        serde_json::json!(cap.version.unwrap_or_else(|| "unknown".into()));
                    info["wit_version"] =
                        serde_json::json!(cap.wit_version.unwrap_or_else(|| "unknown".into()));
                }

                info["host_wit_version"] =
                    serde_json::json!(crate::tools::wasm::WIT_CHANNEL_VERSION);

                Ok(info)
            }
            ExtensionKind::McpServer => {
                let info = serde_json::json!({
                    "name": name,
                    "kind": "mcp_server",
                    "connected": self.mcp_clients.read().await.contains_key(name),
                });
                Ok(info)
            }
            ExtensionKind::ChannelRelay => {
                let info = serde_json::json!({
                    "name": name,
                    "kind": "channel_relay",
                    "active": self.active_channel_names.read().await.contains(name),
                });
                Ok(info)
            }
        }
    }

    // ── MCP config helpers (DB with disk fallback) ─────────────────────

    async fn load_mcp_servers(
        &self,
    ) -> Result<crate::tools::mcp::config::McpServersFile, crate::tools::mcp::config::ConfigError>
    {
        if let Some(ref store) = self.store {
            crate::tools::mcp::config::load_mcp_servers_from_db(store.as_ref(), &self.user_id).await
        } else {
            crate::tools::mcp::config::load_mcp_servers().await
        }
    }

    async fn get_mcp_server(
        &self,
        name: &str,
    ) -> Result<McpServerConfig, crate::tools::mcp::config::ConfigError> {
        let servers = self.load_mcp_servers().await?;
        servers.get(name).cloned().ok_or_else(|| {
            crate::tools::mcp::config::ConfigError::ServerNotFound {
                name: name.to_string(),
            }
        })
    }

    async fn add_mcp_server(
        &self,
        config: McpServerConfig,
    ) -> Result<(), crate::tools::mcp::config::ConfigError> {
        config.validate()?;
        if let Some(ref store) = self.store {
            crate::tools::mcp::config::add_mcp_server_db(store.as_ref(), &self.user_id, config)
                .await
        } else {
            crate::tools::mcp::config::add_mcp_server(config).await
        }
    }

    async fn remove_mcp_server(
        &self,
        name: &str,
    ) -> Result<(), crate::tools::mcp::config::ConfigError> {
        if let Some(ref store) = self.store {
            crate::tools::mcp::config::remove_mcp_server_db(store.as_ref(), &self.user_id, name)
                .await
        } else {
            crate::tools::mcp::config::remove_mcp_server(name).await
        }
    }

    // ── Private helpers ──────────────────────────────────────────────────

    async fn install_from_entry(
        &self,
        entry: &RegistryEntry,
    ) -> Result<InstallResult, ExtensionError> {
        let primary_result = self.try_install_from_source(entry, &entry.source).await;
        match fallback_decision(&primary_result, &entry.fallback_source) {
            FallbackDecision::Return => primary_result,
            FallbackDecision::TryFallback => {
                let primary_err = primary_result.unwrap_err();
                let fallback = entry.fallback_source.as_ref().unwrap();
                tracing::info!(
                    extension = %entry.name,
                    primary_error = %primary_err,
                    "Primary install failed, trying fallback source"
                );
                match self.try_install_from_source(entry, fallback).await {
                    Ok(result) => Ok(result),
                    Err(fallback_err) => {
                        tracing::error!(
                            extension = %entry.name,
                            fallback_error = %fallback_err,
                            "Fallback install also failed"
                        );
                        Err(combine_install_errors(primary_err, fallback_err))
                    }
                }
            }
        }
    }

    /// Attempt to install an extension using a specific source.
    async fn try_install_from_source(
        &self,
        entry: &RegistryEntry,
        source: &ExtensionSource,
    ) -> Result<InstallResult, ExtensionError> {
        match entry.kind {
            ExtensionKind::McpServer => {
                let url = match source {
                    ExtensionSource::McpUrl { url } => url.clone(),
                    ExtensionSource::Discovered { url } => url.clone(),
                    _ => {
                        return Err(ExtensionError::InstallFailed(
                            "Registry entry for MCP server has no URL".to_string(),
                        ));
                    }
                };
                self.install_mcp_from_url(&entry.name, &url).await
            }
            ExtensionKind::WasmTool => match source {
                ExtensionSource::WasmDownload {
                    wasm_url,
                    capabilities_url,
                } => {
                    self.install_wasm_tool_from_url_with_caps(
                        &entry.name,
                        wasm_url,
                        capabilities_url.as_deref(),
                    )
                    .await
                }
                ExtensionSource::WasmBuildable {
                    build_dir,
                    crate_name,
                    ..
                } => {
                    self.install_wasm_from_buildable(
                        &entry.name,
                        build_dir.as_deref(),
                        crate_name.as_deref(),
                        &self.wasm_tools_dir,
                        ExtensionKind::WasmTool,
                    )
                    .await
                }
                _ => Err(ExtensionError::InstallFailed(
                    "WASM tool entry has no download URL or build info".to_string(),
                )),
            },
            ExtensionKind::WasmChannel => match source {
                ExtensionSource::WasmDownload {
                    wasm_url,
                    capabilities_url,
                } => {
                    self.install_wasm_channel_from_url(
                        &entry.name,
                        wasm_url,
                        capabilities_url.as_deref(),
                    )
                    .await
                }
                ExtensionSource::WasmBuildable {
                    build_dir,
                    crate_name,
                    ..
                } => {
                    self.install_wasm_from_buildable(
                        &entry.name,
                        build_dir.as_deref(),
                        crate_name.as_deref(),
                        &self.wasm_channels_dir,
                        ExtensionKind::WasmChannel,
                    )
                    .await
                }
                _ => Err(ExtensionError::InstallFailed(
                    "WASM channel entry has no download URL or build info".to_string(),
                )),
            },
            ExtensionKind::ChannelRelay => {
                // No download needed — just mark as installed.
                self.installed_relay_extensions
                    .write()
                    .await
                    .insert(entry.name.clone());
                Ok(InstallResult {
                    name: entry.name.clone(),
                    kind: ExtensionKind::ChannelRelay,
                    message: format!(
                        "'{}' installed. Click Activate to connect your workspace.",
                        entry.display_name
                    ),
                })
            }
        }
    }

    async fn install_mcp_from_url(
        &self,
        name: &str,
        url: &str,
    ) -> Result<InstallResult, ExtensionError> {
        // Check if already installed
        if self.get_mcp_server(name).await.is_ok() {
            return Err(ExtensionError::AlreadyInstalled(name.to_string()));
        }

        let config = McpServerConfig::new(name, url);
        config
            .validate()
            .map_err(|e| ExtensionError::InvalidUrl(e.to_string()))?;

        self.add_mcp_server(config)
            .await
            .map_err(|e| ExtensionError::Config(e.to_string()))?;

        tracing::info!("Installed MCP server '{}' at {}", name, url);

        Ok(InstallResult {
            name: name.to_string(),
            kind: ExtensionKind::McpServer,
            message: format!(
                "MCP server '{}' installed. Run auth next to authenticate.",
                name
            ),
        })
    }

    async fn install_wasm_tool_from_url(
        &self,
        name: &str,
        url: &str,
    ) -> Result<InstallResult, ExtensionError> {
        self.install_wasm_tool_from_url_with_caps(name, url, None)
            .await
    }

    async fn install_wasm_tool_from_url_with_caps(
        &self,
        name: &str,
        url: &str,
        capabilities_url: Option<&str>,
    ) -> Result<InstallResult, ExtensionError> {
        self.download_and_install_wasm(name, url, capabilities_url, &self.wasm_tools_dir)
            .await?;

        Ok(InstallResult {
            name: name.to_string(),
            kind: ExtensionKind::WasmTool,
            message: format!("WASM tool '{}' installed. Run activate to load it.", name),
        })
    }

    async fn install_wasm_channel_from_url(
        &self,
        name: &str,
        url: &str,
        capabilities_url: Option<&str>,
    ) -> Result<InstallResult, ExtensionError> {
        self.download_and_install_wasm(name, url, capabilities_url, &self.wasm_channels_dir)
            .await?;

        Ok(InstallResult {
            name: name.to_string(),
            kind: ExtensionKind::WasmChannel,
            message: format!(
                "WASM channel '{}' installed. Run activate to start it.",
                name,
            ),
        })
    }

    /// Download a WASM extension (tool or channel) from URL and install to target directory.
    ///
    /// Handles both tar.gz bundles (containing `.wasm` + `.capabilities.json`) and bare
    /// `.wasm` files. Validates HTTPS, size limits, and file format.
    async fn download_and_install_wasm(
        &self,
        name: &str,
        url: &str,
        capabilities_url: Option<&str>,
        target_dir: &std::path::Path,
    ) -> Result<(), ExtensionError> {
        // Require HTTPS to prevent downgrade attacks
        if !url.starts_with("https://") {
            return Err(ExtensionError::InstallFailed(
                "Only HTTPS URLs are allowed for extension downloads".to_string(),
            ));
        }

        // 50 MB cap to prevent disk-fill DoS
        const MAX_DOWNLOAD_SIZE: usize = 50 * 1024 * 1024;

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(|e| ExtensionError::DownloadFailed(e.to_string()))?;

        let sanitized_url = sanitize_url_for_logging(url);
        tracing::debug!(extension = %name, url = %sanitized_url, "Downloading WASM extension");

        let response = client.get(url).send().await.map_err(|e| {
            tracing::error!(extension = %name, url = %sanitized_url, error = %e, "Download request failed");
            ExtensionError::DownloadFailed(e.to_string())
        })?;

        if !response.status().is_success() {
            let status = response.status();
            tracing::error!(
                extension = %name,
                url = %sanitized_url,
                status = %status,
                "Download returned non-success HTTP status"
            );
            return Err(ExtensionError::DownloadFailed(format!(
                "HTTP {} from {}",
                status, url
            )));
        }

        // Check Content-Length header before downloading the full body
        if let Some(len) = response.content_length()
            && len as usize > MAX_DOWNLOAD_SIZE
        {
            return Err(ExtensionError::InstallFailed(format!(
                "Download too large ({} bytes, max {} bytes)",
                len, MAX_DOWNLOAD_SIZE
            )));
        }

        let bytes = response
            .bytes()
            .await
            .map_err(|e| ExtensionError::DownloadFailed(e.to_string()))?;

        if bytes.len() > MAX_DOWNLOAD_SIZE {
            return Err(ExtensionError::InstallFailed(format!(
                "Download too large ({} bytes, max {} bytes)",
                bytes.len(),
                MAX_DOWNLOAD_SIZE
            )));
        }

        // Ensure target directory exists
        tokio::fs::create_dir_all(target_dir)
            .await
            .map_err(|e| ExtensionError::InstallFailed(e.to_string()))?;

        let wasm_path = target_dir.join(format!("{}.wasm", name));
        let caps_path = target_dir.join(format!("{}.capabilities.json", name));

        // Detect format: gzip (tar.gz bundle) or bare WASM
        if bytes.len() >= 2 && bytes[0] == 0x1f && bytes[1] == 0x8b {
            // tar.gz bundle: extract {name}.wasm and {name}.capabilities.json
            self.extract_wasm_tar_gz(name, &bytes, &wasm_path, &caps_path)?;
        } else {
            // Bare WASM file: validate magic number
            if bytes.len() < 4 || &bytes[..4] != b"\0asm" {
                return Err(ExtensionError::InstallFailed(
                    "Downloaded file is not a valid WASM binary (bad magic number)".to_string(),
                ));
            }

            tokio::fs::write(&wasm_path, &bytes)
                .await
                .map_err(|e| ExtensionError::InstallFailed(e.to_string()))?;

            // Download capabilities separately if URL provided
            if let Some(caps_url) = capabilities_url {
                const MAX_CAPS_SIZE: usize = 1024 * 1024; // 1 MB
                match client.get(caps_url).send().await {
                    Ok(resp) if resp.status().is_success() => match resp.bytes().await {
                        Ok(caps_bytes) if caps_bytes.len() <= MAX_CAPS_SIZE => {
                            if let Err(e) = tokio::fs::write(&caps_path, &caps_bytes).await {
                                tracing::warn!(
                                    "Failed to write capabilities for '{}': {}",
                                    name,
                                    e
                                );
                            }
                        }
                        Ok(caps_bytes) => {
                            tracing::warn!(
                                "Capabilities file for '{}' too large ({} bytes, max {})",
                                name,
                                caps_bytes.len(),
                                MAX_CAPS_SIZE
                            );
                        }
                        Err(e) => {
                            tracing::warn!("Failed to download capabilities for '{}': {}", name, e);
                        }
                    },
                    _ => {
                        tracing::warn!(
                            "Failed to download capabilities for '{}' from {}",
                            name,
                            caps_url
                        );
                    }
                }
            }
        }

        tracing::info!(
            "Installed WASM extension '{}' from {} to {}",
            name,
            url,
            wasm_path.display()
        );

        Ok(())
    }

    /// Extract a tar.gz bundle into the WASM tools directory.
    fn extract_wasm_tar_gz(
        &self,
        name: &str,
        bytes: &[u8],
        target_wasm: &std::path::Path,
        target_caps: &std::path::Path,
    ) -> Result<(), ExtensionError> {
        use flate2::read::GzDecoder;
        use tar::Archive;

        use std::io::Read as _;

        let decoder = GzDecoder::new(bytes);
        let mut archive = Archive::new(decoder);
        // Defense-in-depth: do not preserve permissions or extended attributes
        archive.set_preserve_permissions(false);
        #[cfg(any(unix, target_os = "redox"))]
        archive.set_unpack_xattrs(false);

        // 100 MB cap on decompressed entry size to prevent decompression bombs
        const MAX_ENTRY_SIZE: u64 = 100 * 1024 * 1024;

        let wasm_filename = format!("{}.wasm", name);
        let caps_filename = format!("{}.capabilities.json", name);
        let mut found_wasm = false;

        let entries = archive
            .entries()
            .map_err(|e| ExtensionError::InstallFailed(format!("Bad tar.gz archive: {}", e)))?;

        for entry in entries {
            let mut entry = entry
                .map_err(|e| ExtensionError::InstallFailed(format!("Bad tar.gz entry: {}", e)))?;

            if entry.size() > MAX_ENTRY_SIZE {
                return Err(ExtensionError::InstallFailed(format!(
                    "Archive entry too large ({} bytes, max {} bytes)",
                    entry.size(),
                    MAX_ENTRY_SIZE
                )));
            }

            let entry_path = entry
                .path()
                .map_err(|e| {
                    ExtensionError::InstallFailed(format!("Invalid path in tar.gz: {}", e))
                })?
                .to_path_buf();

            let filename = entry_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");

            if filename == wasm_filename {
                let mut data = Vec::with_capacity(entry.size() as usize);
                std::io::Read::read_to_end(&mut entry.by_ref().take(MAX_ENTRY_SIZE), &mut data)
                    .map_err(|e| ExtensionError::InstallFailed(e.to_string()))?;
                std::fs::write(target_wasm, &data)
                    .map_err(|e| ExtensionError::InstallFailed(e.to_string()))?;
                found_wasm = true;
            } else if filename == caps_filename {
                let mut data = Vec::with_capacity(entry.size() as usize);
                std::io::Read::read_to_end(&mut entry.by_ref().take(MAX_ENTRY_SIZE), &mut data)
                    .map_err(|e| ExtensionError::InstallFailed(e.to_string()))?;
                std::fs::write(target_caps, &data)
                    .map_err(|e| ExtensionError::InstallFailed(e.to_string()))?;
            }
        }

        if !found_wasm {
            return Err(ExtensionError::InstallFailed(format!(
                "tar.gz archive does not contain '{}'",
                wasm_filename
            )));
        }

        Ok(())
    }

    /// Install a WASM extension from local build artifacts (WasmBuildable source).
    ///
    /// Resolves the build directory (relative to `CARGO_MANIFEST_DIR` or absolute),
    /// looks for the compiled WASM artifact, and copies it (plus capabilities.json)
    /// to the install directory. Falls back to an error if artifacts don't exist.
    async fn install_wasm_from_buildable(
        &self,
        name: &str,
        build_dir: Option<&str>,
        crate_name: Option<&str>,
        target_dir: &std::path::Path,
        kind: ExtensionKind,
    ) -> Result<InstallResult, ExtensionError> {
        let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));

        // Resolve build directory
        let resolved_dir = match build_dir {
            Some(dir) => {
                let p = std::path::Path::new(dir);
                if p.is_absolute() {
                    p.to_path_buf()
                } else {
                    manifest_dir.join(dir)
                }
            }
            None => manifest_dir.to_path_buf(),
        };

        // Determine the binary name to look for
        let binary_name = crate_name.unwrap_or(name);

        let wasm_src =
            crate::registry::artifacts::find_wasm_artifact(&resolved_dir, binary_name, "release")
                .ok_or_else(|| {
                ExtensionError::InstallFailed(format!(
                    "'{}' requires building from source. Build artifact not found. \
                         Run `cargo component build --release` in {} first, \
                         or use `ironclaw registry install {}`.",
                    name,
                    resolved_dir.display(),
                    name,
                ))
            })?;

        let wasm_dst = crate::registry::artifacts::install_wasm_files(
            &wasm_src,
            &resolved_dir,
            name,
            target_dir,
            true,
        )
        .await
        .map_err(|e| ExtensionError::InstallFailed(e.to_string()))?;

        let kind_label = match kind {
            ExtensionKind::WasmTool => "WASM tool",
            ExtensionKind::WasmChannel => "WASM channel",
            ExtensionKind::McpServer => "MCP server",
            ExtensionKind::ChannelRelay => "channel relay",
        };

        tracing::info!(
            "Installed {} '{}' from build artifacts at {}",
            kind_label,
            name,
            wasm_dst.display(),
        );

        Ok(InstallResult {
            name: name.to_string(),
            kind,
            message: format!(
                "{} '{}' installed from local build artifacts. Run activate to load it.",
                kind_label, name,
            ),
        })
    }

    async fn auth_mcp(&self, name: &str) -> Result<AuthResult, ExtensionError> {
        let server = self
            .get_mcp_server(name)
            .await
            .map_err(|e| ExtensionError::NotInstalled(e.to_string()))?;

        // Check if already authenticated
        if is_authenticated(&server, &self.secrets, &self.user_id).await {
            return Ok(AuthResult::authenticated(name, ExtensionKind::McpServer));
        }

        // In gateway mode, build an auth URL and return it for the frontend to
        // open in the same browser. The gateway's /oauth/callback handler will
        // complete the token exchange.
        if self.should_use_gateway_mode() {
            return match self.auth_mcp_build_url(name, &server).await {
                Ok(result) => Ok(result),
                Err(ExtensionError::AuthNotSupported(_)) => Ok(AuthResult::awaiting_token(
                    name,
                    ExtensionKind::McpServer,
                    format!(
                        "Server '{}' does not support OAuth. \
                         Please provide an API token/key for this server.",
                        name
                    ),
                    None,
                )),
                Err(e) => Err(e),
            };
        }

        // CLI/local mode: run the full blocking OAuth flow (opens browser, waits for callback)
        match authorize_mcp_server(&server, &self.secrets, &self.user_id).await {
            Ok(_token) => {
                tracing::info!("MCP server '{}' authenticated via OAuth", name);
                Ok(AuthResult::authenticated(name, ExtensionKind::McpServer))
            }
            Err(crate::tools::mcp::auth::AuthError::NotSupported) => {
                // Server doesn't support OAuth, try building a URL
                match self.auth_mcp_build_url(name, &server).await {
                    Ok(result) => Ok(result),
                    Err(_) => Ok(AuthResult::awaiting_token(
                        name,
                        ExtensionKind::McpServer,
                        format!(
                            "Server '{}' does not support OAuth. \
                             Please provide an API token/key for this server.",
                            name
                        ),
                        None,
                    )),
                }
            }
            Err(e) => {
                // OAuth failed for some other reason, fall back to manual token
                Ok(AuthResult::awaiting_token(
                    name,
                    ExtensionKind::McpServer,
                    format!(
                        "OAuth failed for '{}': {}. \
                         Please provide an API token/key manually.",
                        name, e
                    ),
                    None,
                ))
            }
        }
    }

    /// Build an auth URL for MCP OAuth.
    ///
    /// In gateway mode, stores a `PendingOAuthFlow` so the web gateway's
    /// `/oauth/callback` handler can complete the token exchange — the auth
    /// URL is sent to the frontend which opens it in the same browser.
    /// In local/CLI mode, builds the URL for the user to open manually.
    async fn auth_mcp_build_url(
        &self,
        name: &str,
        server: &McpServerConfig,
    ) -> Result<AuthResult, ExtensionError> {
        // Try to discover OAuth metadata and build a URL the user can open manually
        let metadata = discover_full_oauth_metadata(&server.url)
            .await
            .map_err(|e| match e {
                crate::tools::mcp::auth::AuthError::NotSupported => {
                    ExtensionError::AuthNotSupported(e.to_string())
                }
                _ => ExtensionError::AuthFailed(e.to_string()),
            })?;

        use crate::cli::oauth_defaults;

        let is_gateway = self.should_use_gateway_mode();

        // Build redirect URI: gateway uses the public callback URL,
        // local mode binds a random port.
        let redirect_uri = if let Some(uri) = self.gateway_callback_redirect_uri().await {
            uri
        } else {
            let port = find_available_port()
                .await
                .map_err(|e| ExtensionError::AuthFailed(e.to_string()))?;
            format!("http://localhost:{}/callback", port.1)
        };

        // Try DCR if no client_id configured
        let (client_id, client_secret) = if let Some(ref oauth) = server.oauth {
            (oauth.client_id.clone(), None)
        } else if let Some(ref reg_endpoint) = metadata.registration_endpoint {
            let registration = register_client(reg_endpoint, &redirect_uri)
                .await
                .map_err(|e| ExtensionError::AuthFailed(e.to_string()))?;

            (registration.client_id, None)
        } else {
            return Err(ExtensionError::AuthNotSupported(
                "Server doesn't support OAuth or Dynamic Client Registration".to_string(),
            ));
        };

        // RFC 8707: resource parameter to scope the token to this MCP server
        let resource = canonical_resource_uri(&server.url);

        // Build authorization URL with CSRF state using the shared oauth_defaults
        // builder, which generates PKCE + state for us.
        let mut extra_params = server
            .oauth
            .as_ref()
            .map(|o| o.extra_params.clone())
            .unwrap_or_default();
        extra_params.insert("resource".to_string(), resource.clone());

        let scopes = server
            .oauth
            .as_ref()
            .map(|o| o.scopes.clone())
            .unwrap_or_else(|| metadata.scopes_supported.clone());

        let oauth_result = oauth_defaults::build_oauth_url(
            &metadata.authorization_endpoint,
            &client_id,
            &redirect_uri,
            &scopes,
            true, // Always use PKCE for MCP
            &extra_params,
        );
        let expected_state = oauth_result.state;
        let code_verifier = oauth_result.code_verifier;

        if is_gateway {
            // Gateway mode: store pending flow for the /oauth/callback handler.
            oauth_defaults::sweep_expired_flows(&self.pending_oauth_flows).await;

            // Platform routing: prepend instance name to state
            let platform_state = oauth_defaults::build_platform_state(&expected_state);
            let auth_url = if platform_state != expected_state {
                oauth_result.url.replace(
                    &format!("state={}", urlencoding::encode(&expected_state)),
                    &format!("state={}", urlencoding::encode(&platform_state)),
                )
            } else {
                oauth_result.url
            };

            let flow = oauth_defaults::PendingOAuthFlow {
                extension_name: name.to_string(),
                display_name: server.name.clone(),
                token_url: metadata.token_endpoint,
                client_id,
                client_secret,
                redirect_uri,
                code_verifier,
                access_token_field: "access_token".to_string(),
                secret_name: server.token_secret_name(),
                provider: Some(format!("mcp:{}", name)),
                validation_endpoint: None,
                scopes,
                user_id: self.user_id.clone(),
                secrets: Arc::clone(&self.secrets),
                sse_sender: self.sse_sender.read().await.clone(),
                gateway_token: self.gateway_token.clone(),
                resource: Some(resource),
                client_id_secret_name: if server.oauth.is_none() {
                    Some(server.client_id_secret_name())
                } else {
                    None
                },
                created_at: std::time::Instant::now(),
            };

            self.pending_oauth_flows
                .write()
                .await
                .insert(expected_state, flow);

            self.pending_auth.write().await.insert(
                name.to_string(),
                PendingAuth {
                    _name: name.to_string(),
                    _kind: ExtensionKind::McpServer,
                    created_at: std::time::Instant::now(),
                    task_handle: None,
                },
            );

            Ok(AuthResult::awaiting_authorization(
                name,
                ExtensionKind::McpServer,
                auth_url,
                "gateway".to_string(),
            ))
        } else {
            // Local mode: return URL for manual opening
            self.pending_auth.write().await.insert(
                name.to_string(),
                PendingAuth {
                    _name: name.to_string(),
                    _kind: ExtensionKind::McpServer,
                    created_at: std::time::Instant::now(),
                    task_handle: None,
                },
            );

            Ok(AuthResult::awaiting_authorization(
                name,
                ExtensionKind::McpServer,
                oauth_result.url,
                "local".to_string(),
            ))
        }
    }

    async fn auth_wasm_tool(&self, name: &str) -> Result<AuthResult, ExtensionError> {
        // Read the capabilities file to get auth config
        let cap_path = self
            .wasm_tools_dir
            .join(format!("{}.capabilities.json", name));

        if !cap_path.exists() {
            return Ok(AuthResult::no_auth_required(name, ExtensionKind::WasmTool));
        }

        let cap_bytes = tokio::fs::read(&cap_path)
            .await
            .map_err(|e| ExtensionError::Other(e.to_string()))?;

        let cap_file = crate::tools::wasm::CapabilitiesFile::from_bytes(&cap_bytes)
            .map_err(|e| ExtensionError::Other(e.to_string()))?;

        let auth = match cap_file.auth {
            Some(auth) => auth,
            None => {
                return Ok(AuthResult::no_auth_required(name, ExtensionKind::WasmTool));
            }
        };

        // Check env var first
        if let Some(ref env_var) = auth.env_var
            && let Ok(value) = std::env::var(env_var)
        {
            // Store the env var value as a secret
            let params =
                CreateSecretParams::new(&auth.secret_name, &value).with_provider(name.to_string());
            self.secrets
                .create(&self.user_id, params)
                .await
                .map_err(|e| ExtensionError::AuthFailed(e.to_string()))?;

            return Ok(AuthResult::authenticated(name, ExtensionKind::WasmTool));
        }

        // Check if already authenticated (with scope expansion detection)
        let token_exists = self
            .secrets
            .exists(&self.user_id, &auth.secret_name)
            .await
            .unwrap_or(false);

        if token_exists {
            // If this tool has OAuth config, check whether new scopes are needed
            let needs_reauth = if let Some(ref oauth) = auth.oauth {
                let merged = self
                    .collect_shared_scopes(&auth.secret_name, &oauth.scopes)
                    .await;
                let needs = self.needs_scope_expansion(&auth.secret_name, &merged).await;
                tracing::debug!(
                    tool = name,
                    secret_name = %auth.secret_name,
                    merged_scopes = ?merged,
                    needs_reauth = needs,
                    "Scope expansion check"
                );
                needs
            } else {
                false
            };

            if !needs_reauth {
                return Ok(AuthResult::authenticated(name, ExtensionKind::WasmTool));
            }
            // Fall through to OAuth branch for scope expansion
        }

        // OAuth flow: if the tool has OAuth config, start the browser-based flow.
        // But only if credentials are available — if the tool has setup secrets
        // for client_id/secret that aren't configured yet, return needs_setup.
        if let Some(ref oauth) = auth.oauth {
            if self.needs_setup_credentials(name, &auth, oauth).await {
                let display = auth.display_name.as_deref().unwrap_or(name);
                return Ok(AuthResult::needs_setup(
                    name,
                    ExtensionKind::WasmTool,
                    format!(
                        "Configure OAuth credentials for {} in the Setup tab.",
                        display
                    ),
                    auth.setup_url.clone(),
                ));
            }

            return self
                .start_wasm_oauth(name, &auth, oauth)
                .await
                .map_err(|e| ExtensionError::AuthFailed(e.to_string()));
        }

        // Return instructions for manual token entry
        let display = auth.display_name.unwrap_or_else(|| name.to_string());
        let instructions = auth
            .instructions
            .unwrap_or_else(|| format!("Please provide your {} API token/key.", display));

        Ok(AuthResult::awaiting_token(
            name,
            ExtensionKind::WasmTool,
            instructions,
            auth.setup_url,
        ))
    }

    /// Determine the auth readiness of a WASM channel.
    async fn check_channel_auth_status(&self, name: &str) -> ToolAuthState {
        let cap_path = self
            .wasm_channels_dir
            .join(format!("{}.capabilities.json", name));
        let Ok(cap_bytes) = tokio::fs::read(&cap_path).await else {
            return ToolAuthState::NoAuth;
        };
        let Ok(cap_file) = crate::channels::wasm::ChannelCapabilitiesFile::from_bytes(&cap_bytes)
        else {
            return ToolAuthState::NoAuth;
        };

        let required: Vec<_> = cap_file
            .setup
            .required_secrets
            .iter()
            .filter(|s| !s.optional)
            .collect();
        if required.is_empty() {
            return ToolAuthState::NoAuth;
        }

        let all_provided = futures::future::join_all(
            required
                .iter()
                .map(|s| self.secrets.exists(&self.user_id, &s.name)),
        )
        .await
        .into_iter()
        .all(|r| r.unwrap_or(false));

        if all_provided {
            ToolAuthState::Ready
        } else {
            ToolAuthState::NeedsSetup
        }
    }

    /// Load and parse a WASM tool's capabilities file.
    ///
    /// Returns `None` if the file doesn't exist or can't be parsed.
    async fn load_tool_capabilities(
        &self,
        name: &str,
    ) -> Option<crate::tools::wasm::CapabilitiesFile> {
        let cap_path = self
            .wasm_tools_dir
            .join(format!("{}.capabilities.json", name));
        let cap_bytes = tokio::fs::read(&cap_path).await.ok()?;
        crate::tools::wasm::CapabilitiesFile::from_bytes(&cap_bytes).ok()
    }

    /// Collect merged OAuth scopes from all installed tools sharing the same secret_name.
    ///
    /// When multiple tools share an OAuth provider (e.g., google-calendar and google-drive
    /// both use `google_oauth_token`), we request all their scopes in a single OAuth flow
    /// so one login covers everything.
    async fn collect_shared_scopes(
        &self,
        secret_name: &str,
        base_scopes: &[String],
    ) -> Vec<String> {
        let mut all_scopes: std::collections::BTreeSet<String> =
            base_scopes.iter().cloned().collect();

        if let Ok(tools) = discover_tools(&self.wasm_tools_dir).await {
            for tool_name in tools.keys() {
                if let Some(cap) = self.load_tool_capabilities(tool_name).await
                    && let Some(auth) = &cap.auth
                    && auth.secret_name == secret_name
                    && let Some(oauth) = &auth.oauth
                {
                    all_scopes.extend(oauth.scopes.iter().cloned());
                }
            }
        }

        all_scopes.into_iter().collect()
    }

    /// Check whether the stored scopes are insufficient for the merged scopes.
    async fn needs_scope_expansion(&self, secret_name: &str, merged_scopes: &[String]) -> bool {
        if merged_scopes.is_empty() {
            return false;
        }

        let scopes_key = format!("{}_scopes", secret_name);
        let stored_scopes: std::collections::HashSet<String> =
            match self.secrets.get_decrypted(&self.user_id, &scopes_key).await {
                Ok(secret) => {
                    let scopes: std::collections::HashSet<String> = secret
                        .expose()
                        .split_whitespace()
                        .map(String::from)
                        .collect();
                    tracing::debug!(
                        secret_name,
                        stored_scopes = ?scopes,
                        "Loaded stored scopes for expansion check"
                    );
                    scopes
                }
                Err(_) => {
                    // No stored scopes record — this is a legacy token created before
                    // scope tracking. Force re-auth to ensure all required scopes are granted.
                    tracing::debug!(
                        secret_name,
                        "No stored scopes record, forcing re-auth for legacy token"
                    );
                    return true;
                }
            };

        // Check if any merged scope is missing from stored scopes
        merged_scopes
            .iter()
            .any(|scope| !stored_scopes.contains(scope))
    }

    /// Find the setup secret names for OAuth client_id and client_secret.
    ///
    /// Scans `setup.required_secrets` for names containing "client_id" and "client_secret".
    /// Returns `(Option<(name, optional)>, Option<(name, optional)>)`.
    async fn find_setup_credential_names(
        &self,
        tool_name: &str,
    ) -> (Option<(String, bool)>, Option<(String, bool)>) {
        let Some(cap) = self.load_tool_capabilities(tool_name).await else {
            return (None, None);
        };
        let Some(setup) = &cap.setup else {
            return (None, None);
        };

        let mut client_id_entry = None;
        let mut client_secret_entry = None;
        for secret in &setup.required_secrets {
            let lower = secret.name.to_lowercase();
            if lower.ends_with("client_id") || lower == "client_id" {
                client_id_entry = Some((secret.name.clone(), secret.optional));
            } else if lower.ends_with("client_secret") || lower == "client_secret" {
                client_secret_entry = Some((secret.name.clone(), secret.optional));
            }
        }
        (client_id_entry, client_secret_entry)
    }

    /// Check if OAuth client credentials (client_id / client_secret) require
    /// user input via the Setup tab. Returns `true` when at least one required
    /// credential cannot be resolved through the full chain:
    /// secrets store → inline → env var → builtin.
    async fn needs_setup_credentials(
        &self,
        name: &str,
        auth: &crate::tools::wasm::AuthCapabilitySchema,
        oauth: &crate::tools::wasm::OAuthConfigSchema,
    ) -> bool {
        let builtin = crate::cli::oauth_defaults::builtin_credentials(&auth.secret_name);
        let (id_entry, secret_entry) = self.find_setup_credential_names(name).await;

        for (entry, inline, env, fallback) in [
            (
                &id_entry,
                &oauth.client_id,
                &oauth.client_id_env,
                builtin.as_ref().map(|c| c.client_id),
            ),
            (
                &secret_entry,
                &oauth.client_secret,
                &oauth.client_secret_env,
                builtin.as_ref().map(|c| c.client_secret),
            ),
        ] {
            let Some((ref setup_name, optional)) = *entry else {
                continue;
            };
            if optional {
                continue;
            }
            let resolved = self
                .resolve_oauth_credential(inline, env, fallback, Some(setup_name))
                .await
                .is_some();
            if !resolved {
                return true;
            }
        }
        false
    }

    /// Resolve an OAuth credential value via: secrets store → inline → env var → builtin.
    ///
    /// For web gateway users, the secrets store is checked first because client_id/secret
    /// may have been entered via the Setup tab (stored as setup secrets).
    async fn resolve_oauth_credential(
        &self,
        inline_value: &Option<String>,
        env_var_name: &Option<String>,
        builtin_value: Option<&str>,
        setup_secret_name: Option<&str>,
    ) -> Option<String> {
        // 1. Check secrets store (entered via Setup tab)
        if let Some(secret_name) = setup_secret_name
            && let Ok(secret) = self.secrets.get_decrypted(&self.user_id, secret_name).await
        {
            let val = secret.expose();
            if !val.is_empty() {
                return Some(val.to_string());
            }
        }

        // 2. Inline value from capabilities.json
        if let Some(val) = inline_value {
            return Some(val.clone());
        }

        // 3. Runtime environment variable
        if let Some(env) = env_var_name
            && let Ok(val) = std::env::var(env)
        {
            return Some(val);
        }

        // 4. Built-in defaults
        builtin_value.map(String::from)
    }

    /// Start the OAuth browser flow for a WASM tool.
    ///
    /// Binds a callback listener, builds the authorization URL, spawns a background
    /// task to wait for the callback and exchange the code, then returns the auth URL
    /// immediately so the web UI can open it.
    async fn start_wasm_oauth(
        &self,
        name: &str,
        auth: &crate::tools::wasm::AuthCapabilitySchema,
        oauth: &crate::tools::wasm::OAuthConfigSchema,
    ) -> Result<AuthResult, String> {
        use crate::cli::oauth_defaults;

        let builtin = oauth_defaults::builtin_credentials(&auth.secret_name);

        // Find setup secret names for client_id and client_secret from capabilities.
        // These are the actual names used in the Setup tab (e.g., "google_oauth_client_id"),
        // which may differ from "{secret_name}_client_id".
        let (setup_client_id_entry, setup_client_secret_entry) =
            self.find_setup_credential_names(name).await;
        let setup_client_id_name = setup_client_id_entry.map(|(n, _)| n);
        let setup_client_secret_name = setup_client_secret_entry.map(|(n, _)| n);

        // Resolve client_id: setup secrets → inline → env var → builtin
        let client_id = self
            .resolve_oauth_credential(
                &oauth.client_id,
                &oauth.client_id_env,
                builtin.as_ref().map(|c| c.client_id),
                setup_client_id_name.as_deref(),
            )
            .await
            .ok_or_else(|| {
                let env_name = oauth
                    .client_id_env
                    .as_deref()
                    .unwrap_or("the client_id env var");
                let mut msg = format!(
                    "OAuth client_id not configured for '{}'. \
                     Enter it in the Setup tab or set {} env var",
                    name, env_name
                );
                // Only mention the Google-specific build flag for Google providers
                if auth.secret_name.to_lowercase().contains("google") {
                    msg.push_str(", or build with IRONCLAW_GOOGLE_CLIENT_ID");
                }
                msg.push('.');
                msg
            })?;

        // Resolve client_secret (optional for PKCE-only flows)
        let client_secret = self
            .resolve_oauth_credential(
                &oauth.client_secret,
                &oauth.client_secret_env,
                builtin.as_ref().map(|c| c.client_secret),
                setup_client_secret_name.as_deref(),
            )
            .await;

        // Cancel any existing pending auth for this tool (frees port 9876 in TCP mode)
        {
            let mut pending = self.pending_auth.write().await;
            if let Some(old) = pending.remove(name)
                && let Some(handle) = old.task_handle
            {
                handle.abort();
            }
        }
        // Also clean up any gateway-mode pending flows for this tool
        {
            let mut flows = self.pending_oauth_flows.write().await;
            flows.retain(|_, flow| flow.extension_name != name);
        }

        let redirect_uri = self
            .gateway_callback_redirect_uri()
            .await
            .unwrap_or_else(|| format!("{}/callback", oauth_defaults::callback_url()));

        // Merge scopes from all tools sharing this provider
        let merged_scopes = self
            .collect_shared_scopes(&auth.secret_name, &oauth.scopes)
            .await;

        // Build authorization URL with CSRF state
        let oauth_result = oauth_defaults::build_oauth_url(
            &oauth.authorization_url,
            &client_id,
            &redirect_uri,
            &merged_scopes,
            oauth.use_pkce,
            &oauth.extra_params,
        );
        let auth_url = oauth_result.url.clone();
        let code_verifier = oauth_result.code_verifier;
        let expected_state = oauth_result.state;

        let display_name = auth
            .display_name
            .clone()
            .unwrap_or_else(|| name.to_string());

        if self.should_use_gateway_mode() {
            // Gateway mode: store pending flow state for the web gateway's
            // `/oauth/callback` handler to complete the exchange. No TCP listener
            // needed — the OAuth provider redirects to the gateway URL.
            oauth_defaults::sweep_expired_flows(&self.pending_oauth_flows).await;

            // Wrap the CSRF nonce with instance name for platform routing.
            // Nginx at auth.DOMAIN parses `instance:nonce` to route the callback
            // to the correct container. The flow is keyed by the raw nonce.
            let platform_state = oauth_defaults::build_platform_state(&expected_state);
            let auth_url = if platform_state != expected_state {
                auth_url.replace(
                    &format!("state={}", urlencoding::encode(&expected_state)),
                    &format!("state={}", urlencoding::encode(&platform_state)),
                )
            } else {
                auth_url
            };

            let flow = oauth_defaults::PendingOAuthFlow {
                extension_name: name.to_string(),
                display_name: display_name.clone(),
                token_url: oauth.token_url.clone(),
                client_id: client_id.clone(),
                client_secret: client_secret.clone(),
                redirect_uri: redirect_uri.clone(),
                code_verifier,
                access_token_field: oauth.access_token_field.clone(),
                secret_name: auth.secret_name.clone(),
                provider: auth.provider.clone(),
                validation_endpoint: auth.validation_endpoint.clone(),
                scopes: merged_scopes,
                user_id: self.user_id.clone(),
                secrets: Arc::clone(&self.secrets),
                sse_sender: self.sse_sender.read().await.clone(),
                gateway_token: self.gateway_token.clone(),
                resource: None,
                client_id_secret_name: None,
                created_at: std::time::Instant::now(),
            };

            // Key by raw nonce (without instance prefix) — the callback handler
            // strips the prefix before lookup.
            self.pending_oauth_flows
                .write()
                .await
                .insert(expected_state, flow);

            // Register pending auth without a task handle (gateway handles completion)
            self.pending_auth.write().await.insert(
                name.to_string(),
                PendingAuth {
                    _name: name.to_string(),
                    _kind: ExtensionKind::WasmTool,
                    created_at: std::time::Instant::now(),
                    task_handle: None,
                },
            );

            Ok(AuthResult::awaiting_authorization(
                name,
                ExtensionKind::WasmTool,
                auth_url,
                "gateway".to_string(),
            ))
        } else {
            // TCP listener mode: bind port 9876 and spawn a background task
            // to wait for the callback. This is the original flow for local/desktop use.
            let listener = oauth_defaults::bind_callback_listener()
                .await
                .map_err(|e| format!("Failed to start OAuth callback listener: {}", e))?;

            let token_url = oauth.token_url.clone();
            let access_token_field = oauth.access_token_field.clone();
            let secret_name = auth.secret_name.clone();
            let provider = auth.provider.clone();
            let validation_endpoint = auth.validation_endpoint.clone();
            let user_id = self.user_id.clone();
            let secrets = Arc::clone(&self.secrets);
            let sse_sender = self.sse_sender.read().await.clone();
            let ext_name = name.to_string();

            let task_handle = tokio::spawn(async move {
                let result: Result<(), String> = async {
                    let code = oauth_defaults::wait_for_callback(
                        listener,
                        "/callback",
                        "code",
                        &display_name,
                        Some(&expected_state),
                    )
                    .await
                    .map_err(|e| e.to_string())?;

                    let token_response = oauth_defaults::exchange_oauth_code(
                        &token_url,
                        &client_id,
                        client_secret.as_deref(),
                        &code,
                        &redirect_uri,
                        code_verifier.as_deref(),
                        &access_token_field,
                    )
                    .await
                    .map_err(|e| e.to_string())?;

                    // Validate the token before storing (catches wrong account, etc.)
                    if let Some(ref validation) = validation_endpoint {
                        oauth_defaults::validate_oauth_token(
                            &token_response.access_token,
                            validation,
                        )
                        .await
                        .map_err(|e| e.to_string())?;
                    }

                    oauth_defaults::store_oauth_tokens(
                        secrets.as_ref(),
                        &user_id,
                        &secret_name,
                        provider.as_deref(),
                        &token_response.access_token,
                        token_response.refresh_token.as_deref(),
                        token_response.expires_in,
                        &merged_scopes,
                    )
                    .await
                    .map_err(|e| e.to_string())?;

                    Ok(())
                }
                .await;

                // Broadcast SSE event
                let (success, message) = match result {
                    Ok(()) => (true, format!("{} authenticated successfully", display_name)),
                    Err(ref e) => (
                        false,
                        format!("{} authentication failed: {}", display_name, e),
                    ),
                };

                match &result {
                    Ok(()) => {
                        tracing::info!(
                            tool = %ext_name,
                            "OAuth completed successfully"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            tool = %ext_name,
                            error = %e,
                            "WASM tool OAuth failed"
                        );
                    }
                }

                if let Some(ref sender) = sse_sender {
                    let _ = sender.send(crate::channels::web::types::SseEvent::AuthCompleted {
                        extension_name: ext_name,
                        success,
                        message,
                    });
                }
            });

            // Store pending auth with task handle
            self.pending_auth.write().await.insert(
                name.to_string(),
                PendingAuth {
                    _name: name.to_string(),
                    _kind: ExtensionKind::WasmTool,
                    created_at: std::time::Instant::now(),
                    task_handle: Some(task_handle),
                },
            );

            Ok(AuthResult::awaiting_authorization(
                name,
                ExtensionKind::WasmTool,
                auth_url,
                "local".to_string(),
            ))
        }
    }

    /// Returns `true` if a setup secret is an OAuth credential (client_id or client_secret)
    /// that can be resolved without user input — via inline capabilities, env var, or
    /// builtin defaults.
    ///
    /// Used by `check_tool_auth_status()` and `get_setup_schema()` to hide setup fields
    /// that the user doesn't need to fill (e.g., Google tools with builtin credentials).
    fn is_auto_resolved_oauth_field(
        secret_name: &str,
        cap_file: &crate::tools::wasm::CapabilitiesFile,
    ) -> bool {
        let lower = secret_name.to_lowercase();
        let is_client_id = lower.ends_with("client_id") || lower == "client_id";
        let is_client_secret = lower.ends_with("client_secret") || lower == "client_secret";
        if !is_client_id && !is_client_secret {
            return false;
        }
        let Some(ref auth) = cap_file.auth else {
            return false;
        };
        let Some(ref oauth) = auth.oauth else {
            return false;
        };
        let builtin = crate::cli::oauth_defaults::builtin_credentials(&auth.secret_name);

        if is_client_id {
            oauth.client_id.is_some()
                || oauth
                    .client_id_env
                    .as_ref()
                    .is_some_and(|e| std::env::var(e).is_ok())
                || builtin.is_some()
        } else {
            oauth.client_secret.is_some()
                || oauth
                    .client_secret_env
                    .as_ref()
                    .is_some_and(|e| std::env::var(e).is_ok())
                || builtin.is_some()
        }
    }

    /// Determine the auth readiness of a WASM tool.
    async fn check_tool_auth_status(&self, name: &str) -> ToolAuthState {
        let Some(cap_file) = self.load_tool_capabilities(name).await else {
            return ToolAuthState::NoAuth;
        };

        // If the tool declares an auth section, the access token is the
        // authoritative signal — setup secrets (client_id/secret) are
        // intermediate and may be auto-resolved via builtins.
        if let Some(ref auth) = cap_file.auth {
            let has_token = self
                .secrets
                .exists(&self.user_id, &auth.secret_name)
                .await
                .unwrap_or(false)
                || auth
                    .env_var
                    .as_ref()
                    .is_some_and(|v| std::env::var(v).is_ok());
            return if has_token {
                ToolAuthState::Ready
            } else if auth.oauth.is_some() {
                ToolAuthState::NeedsAuth
            } else {
                ToolAuthState::NeedsSetup
            };
        }

        // No auth section — fall back to checking setup.required_secrets.
        let Some(setup) = &cap_file.setup else {
            return ToolAuthState::NoAuth;
        };
        if setup.required_secrets.is_empty() {
            return ToolAuthState::NoAuth;
        }

        let all_provided = futures::future::join_all(
            setup
                .required_secrets
                .iter()
                .filter(|s| !s.optional)
                .filter(|s| !Self::is_auto_resolved_oauth_field(&s.name, &cap_file))
                .map(|s| self.secrets.exists(&self.user_id, &s.name)),
        )
        .await
        .into_iter()
        .all(|r| r.unwrap_or(false));

        if all_provided {
            ToolAuthState::Ready
        } else {
            ToolAuthState::NeedsSetup
        }
    }

    /// Check auth status for a WASM channel (read-only).
    async fn auth_wasm_channel_status(&self, name: &str) -> Result<AuthResult, ExtensionError> {
        let cap_path = self
            .wasm_channels_dir
            .join(format!("{}.capabilities.json", name));

        if !cap_path.exists() {
            return Ok(AuthResult::no_auth_required(
                name,
                ExtensionKind::WasmChannel,
            ));
        }

        let cap_bytes = tokio::fs::read(&cap_path)
            .await
            .map_err(|e| ExtensionError::Other(e.to_string()))?;

        let cap_file = crate::channels::wasm::ChannelCapabilitiesFile::from_bytes(&cap_bytes)
            .map_err(|e| ExtensionError::Other(e.to_string()))?;

        let required_secrets = &cap_file.setup.required_secrets;
        if required_secrets.is_empty() {
            return Ok(AuthResult::no_auth_required(
                name,
                ExtensionKind::WasmChannel,
            ));
        }

        // Find non-optional secrets that aren't yet stored
        let mut missing = Vec::new();
        for secret in required_secrets {
            if secret.optional {
                continue;
            }
            if !self
                .secrets
                .exists(&self.user_id, &secret.name)
                .await
                .unwrap_or(false)
            {
                missing.push(secret);
            }
        }

        if missing.is_empty() {
            return Ok(AuthResult::authenticated(name, ExtensionKind::WasmChannel));
        }

        // Prompt for the first missing secret
        let secret = &missing[0];
        Ok(AuthResult::awaiting_token(
            name,
            ExtensionKind::WasmChannel,
            secret.prompt.clone(),
            cap_file.setup.setup_url.clone(),
        ))
    }

    async fn activate_mcp(&self, name: &str) -> Result<ActivateResult, ExtensionError> {
        // Check if already activated
        {
            let clients = self.mcp_clients.read().await;
            if clients.contains_key(name) {
                // Already connected, just return the tool names
                let tools: Vec<String> = self
                    .tool_registry
                    .list()
                    .await
                    .into_iter()
                    .filter(|t| t.starts_with(&format!("{}_", name)))
                    .collect();

                return Ok(ActivateResult {
                    name: name.to_string(),
                    kind: ExtensionKind::McpServer,
                    tools_loaded: tools,
                    message: format!("MCP server '{}' already active", name),
                });
            }
        }

        let server = self
            .get_mcp_server(name)
            .await
            .map_err(|e| ExtensionError::NotInstalled(e.to_string()))?;

        let client = crate::tools::mcp::create_client_from_config(
            server.clone(),
            &self.mcp_session_manager,
            &self.mcp_process_manager,
            Some(Arc::clone(&self.secrets)),
            &self.user_id,
        )
        .await
        .map_err(|e| ExtensionError::ActivationFailed(e.to_string()))?;

        // Try to list and create tools.
        // A 401/auth error means the server requires OAuth — surface as
        // AuthRequired so the activate handler triggers the OAuth flow.
        let mcp_tools = client.list_tools().await.map_err(|e| {
            let msg = e.to_string();
            if msg.contains("requires authentication") || msg.contains("401") {
                ExtensionError::AuthRequired
            } else {
                ExtensionError::ActivationFailed(msg)
            }
        })?;

        let tool_impls = client
            .create_tools()
            .await
            .map_err(|e| ExtensionError::ActivationFailed(e.to_string()))?;

        let tool_names: Vec<String> = mcp_tools
            .iter()
            .map(|t| format!("{}_{}", name, t.name))
            .collect();

        for tool in tool_impls {
            self.tool_registry.register(tool).await;
        }

        // Store the client
        self.mcp_clients
            .write()
            .await
            .insert(name.to_string(), Arc::new(client));

        tracing::info!(
            "Activated MCP server '{}' with {} tools",
            name,
            tool_names.len()
        );

        Ok(ActivateResult {
            name: name.to_string(),
            kind: ExtensionKind::McpServer,
            tools_loaded: tool_names,
            message: format!("Connected to '{}' and loaded tools", name),
        })
    }

    async fn activate_wasm_tool(&self, name: &str) -> Result<ActivateResult, ExtensionError> {
        // Check if already active
        if self.tool_registry.has(name).await {
            return Ok(ActivateResult {
                name: name.to_string(),
                kind: ExtensionKind::WasmTool,
                tools_loaded: vec![name.to_string()],
                message: format!("WASM tool '{}' already active", name),
            });
        }

        // Check auth status — block activation if required secrets are missing.
        // NeedsAuth (OAuth not yet completed) is allowed because configure() loads
        // the tool first, then starts the OAuth flow to obtain the token.
        let auth_state = self.check_tool_auth_status(name).await;
        if auth_state == ToolAuthState::NeedsSetup {
            return Err(ExtensionError::ActivationFailed(format!(
                "Tool '{}' requires configuration. Use the setup form to provide credentials.",
                name
            )));
        }

        let runtime = self.wasm_tool_runtime.as_ref().ok_or_else(|| {
            ExtensionError::ActivationFailed("WASM runtime not available".to_string())
        })?;

        let wasm_path = self.wasm_tools_dir.join(format!("{}.wasm", name));
        if !wasm_path.exists() {
            return Err(ExtensionError::NotInstalled(format!(
                "WASM tool '{}' not found at {}",
                name,
                wasm_path.display()
            )));
        }

        let cap_path = self
            .wasm_tools_dir
            .join(format!("{}.capabilities.json", name));
        let cap_path_option = if cap_path.exists() {
            Some(cap_path.as_path())
        } else {
            None
        };

        let loader = WasmToolLoader::new(Arc::clone(runtime), Arc::clone(&self.tool_registry))
            .with_secrets_store(Arc::clone(&self.secrets));
        loader
            .load_from_files(name, &wasm_path, cap_path_option)
            .await
            .map_err(|e| ExtensionError::ActivationFailed(e.to_string()))?;

        if let Some(ref hooks) = self.hooks
            && let Some(cap_path) = cap_path_option
        {
            let source = format!("plugin.tool:{}", name);
            let registration =
                crate::hooks::bootstrap::register_plugin_bundle_from_capabilities_file(
                    hooks, &source, cap_path,
                )
                .await;

            if registration.total_registered() > 0 {
                tracing::info!(
                    extension = name,
                    hooks = registration.hooks,
                    outbound_webhooks = registration.outbound_webhooks,
                    "Registered plugin hooks for activated WASM tool"
                );
            }

            if registration.errors > 0 {
                tracing::warn!(
                    extension = name,
                    errors = registration.errors,
                    "Some plugin hooks failed to register"
                );
            }
        }

        tracing::info!("Activated WASM tool '{}'", name);

        Ok(ActivateResult {
            name: name.to_string(),
            kind: ExtensionKind::WasmTool,
            tools_loaded: vec![name.to_string()],
            message: format!("WASM tool '{}' loaded and ready", name),
        })
    }

    /// Activate a WASM channel at runtime without restarting.
    ///
    /// Loads the channel from its WASM file, injects credentials and config,
    /// registers it with the webhook router, and hot-adds it to the channel manager
    /// so its stream feeds into the agent loop.
    async fn activate_wasm_channel(&self, name: &str) -> Result<ActivateResult, ExtensionError> {
        // If already active, re-inject credentials and refresh webhook secret.
        // Handles the case where a channel was loaded at startup before the
        // user saved secrets via the web UI.
        {
            let active = self.active_channel_names.read().await;
            if active.contains(name) {
                return self.refresh_active_channel(name).await;
            }
        }

        // Verify runtime infrastructure is available and clone Arcs so we don't
        // hold the RwLock guard across awaits.
        let (channel_runtime, channel_manager, pairing_store, wasm_channel_router) = {
            let rt_guard = self.channel_runtime.read().await;
            let rt = rt_guard.as_ref().ok_or_else(|| {
                ExtensionError::ActivationFailed("WASM channel runtime not configured".to_string())
            })?;
            (
                Arc::clone(&rt.wasm_channel_runtime),
                Arc::clone(&rt.channel_manager),
                Arc::clone(&rt.pairing_store),
                Arc::clone(&rt.wasm_channel_router),
            )
        };

        // Check auth status first
        let auth_state = self.check_channel_auth_status(name).await;
        if auth_state != ToolAuthState::Ready && auth_state != ToolAuthState::NoAuth {
            return Err(ExtensionError::ActivationFailed(format!(
                "Channel '{}' requires configuration. Use the setup form to provide credentials.",
                name
            )));
        }

        // Load the channel from files
        let wasm_path = self.wasm_channels_dir.join(format!("{}.wasm", name));
        let cap_path = self
            .wasm_channels_dir
            .join(format!("{}.capabilities.json", name));
        let cap_path_option = if cap_path.exists() {
            Some(cap_path.as_path())
        } else {
            None
        };

        let settings_store: Option<Arc<dyn crate::db::SettingsStore>> =
            self.store.as_ref().map(|db| Arc::clone(db) as _);
        let loader = WasmChannelLoader::new(
            Arc::clone(&channel_runtime),
            Arc::clone(&pairing_store),
            settings_store,
        )
        .with_secrets_store(Arc::clone(&self.secrets));
        let loaded = loader
            .load_from_files(name, &wasm_path, cap_path_option)
            .await
            .map_err(|e| ExtensionError::ActivationFailed(e.to_string()))?;

        let channel_name = loaded.name().to_string();
        let webhook_secret_name = loaded.webhook_secret_name();
        let secret_header = loaded.webhook_secret_header().map(|s| s.to_string());
        let sig_key_secret_name = loaded.signature_key_secret_name();
        let hmac_secret_name = loaded.hmac_secret_name();

        // Get webhook secret from secrets store
        let webhook_secret = self
            .secrets
            .get_decrypted(&self.user_id, &webhook_secret_name)
            .await
            .ok()
            .map(|s| s.expose().to_string());

        let channel_arc = Arc::new(loaded.channel);

        // Inject runtime config (tunnel_url, webhook_secret, owner_id)
        {
            let mut config_updates = std::collections::HashMap::new();

            if let Some(ref tunnel_url) = self.tunnel_url {
                config_updates.insert(
                    "tunnel_url".to_string(),
                    serde_json::Value::String(tunnel_url.clone()),
                );
            }

            if let Some(ref secret) = webhook_secret {
                config_updates.insert(
                    "webhook_secret".to_string(),
                    serde_json::Value::String(secret.clone()),
                );
            }

            if let Some(owner_id) = self.current_channel_owner_id(&channel_name).await {
                config_updates.insert("owner_id".to_string(), serde_json::json!(owner_id));
            }

            if !config_updates.is_empty() {
                channel_arc.update_config(config_updates).await;
                tracing::info!(
                    channel = %channel_name,
                    has_tunnel = self.tunnel_url.is_some(),
                    has_webhook_secret = webhook_secret.is_some(),
                    "Injected runtime config into hot-activated channel"
                );
            }
        }

        // Register with webhook router
        {
            let webhook_path = format!("/webhook/{}", channel_name);
            let endpoints = vec![RegisteredEndpoint {
                channel_name: channel_name.clone(),
                path: webhook_path,
                methods: vec!["POST".to_string()],
                require_secret: webhook_secret.is_some(),
            }];

            wasm_channel_router
                .register(
                    Arc::clone(&channel_arc),
                    endpoints,
                    webhook_secret,
                    secret_header,
                )
                .await;
            tracing::info!(channel = %channel_name, "Registered hot-activated channel with webhook router");

            // Register Ed25519 signature key if declared in capabilities
            if let Some(ref sig_key_name) = sig_key_secret_name
                && let Ok(key_secret) = self
                    .secrets
                    .get_decrypted(&self.user_id, sig_key_name)
                    .await
            {
                match wasm_channel_router
                    .register_signature_key(&channel_name, key_secret.expose())
                    .await
                {
                    Ok(()) => {
                        tracing::info!(channel = %channel_name, "Registered signature key for hot-activated channel")
                    }
                    Err(e) => {
                        tracing::error!(channel = %channel_name, error = %e, "Failed to register signature key")
                    }
                }
            }

            // Register HMAC signing secret if declared in capabilities
            if let Some(hmac_name) = &hmac_secret_name {
                match self.secrets.get_decrypted(&self.user_id, hmac_name).await {
                    Ok(secret) => {
                        wasm_channel_router
                            .register_hmac_secret(&channel_name, secret.expose())
                            .await;
                        tracing::info!(channel = %channel_name, "Registered HMAC signing secret for hot-activated channel");
                    }
                    Err(e) => {
                        tracing::warn!(channel = %channel_name, error = %e, "HMAC secret not found");
                    }
                }
            }
        }

        // Inject credentials
        match inject_channel_credentials_from_secrets(
            &channel_arc,
            Some(self.secrets.as_ref()),
            &channel_name,
            &self.user_id,
        )
        .await
        {
            Ok(count) => {
                if count > 0 {
                    tracing::info!(
                        channel = %channel_name,
                        credentials_injected = count,
                        "Credentials injected into hot-activated channel"
                    );
                }
            }
            Err(e) => {
                tracing::error!(
                    channel = %channel_name,
                    error = %e,
                    "Failed to inject credentials into hot-activated channel"
                );
            }
        }

        // Hot-add the channel to the running agent
        channel_manager
            .hot_add(Box::new(SharedWasmChannel::new(channel_arc)))
            .await
            .map_err(|e| ExtensionError::ActivationFailed(e.to_string()))?;

        // Mark as active
        self.active_channel_names
            .write()
            .await
            .insert(channel_name.clone());

        // Persist activation state so the channel auto-activates on restart
        self.persist_active_channels().await;

        tracing::info!(channel = %channel_name, "Hot-activated WASM channel");

        Ok(ActivateResult {
            name: channel_name,
            kind: ExtensionKind::WasmChannel,
            tools_loaded: Vec::new(),
            message: format!("Channel '{}' activated and running", name),
        })
    }

    /// Refresh credentials and webhook secret on an already-active channel.
    ///
    /// Called when the user saves new secrets via the setup form for a channel
    /// that was loaded at startup (possibly without credentials).
    async fn refresh_active_channel(&self, name: &str) -> Result<ActivateResult, ExtensionError> {
        let router = {
            let rt_guard = self.channel_runtime.read().await;
            match rt_guard.as_ref() {
                Some(rt) => Arc::clone(&rt.wasm_channel_router),
                None => {
                    return Ok(ActivateResult {
                        name: name.to_string(),
                        kind: ExtensionKind::WasmChannel,
                        tools_loaded: Vec::new(),
                        message: format!("Channel '{}' is already active", name),
                    });
                }
            }
        };

        let webhook_path = format!("/webhook/{}", name);
        let existing_channel = match router.get_channel_for_path(&webhook_path).await {
            Some(ch) => ch,
            None => {
                return Ok(ActivateResult {
                    name: name.to_string(),
                    kind: ExtensionKind::WasmChannel,
                    tools_loaded: Vec::new(),
                    message: format!("Channel '{}' is already active", name),
                });
            }
        };

        // Re-inject credentials from secrets store into the running channel
        let cred_count = match inject_channel_credentials_from_secrets(
            &existing_channel,
            Some(self.secrets.as_ref()),
            name,
            &self.user_id,
        )
        .await
        {
            Ok(count) => count,
            Err(e) => {
                tracing::warn!(
                    channel = %name,
                    error = %e,
                    "Failed to refresh credentials on already-active channel"
                );
                0
            }
        };

        // Load capabilities file once to extract all secret names
        let cap_path = self
            .wasm_channels_dir
            .join(format!("{}.capabilities.json", name));
        let capabilities_file = match tokio::fs::read(&cap_path).await {
            Ok(bytes) => crate::channels::wasm::ChannelCapabilitiesFile::from_bytes(&bytes).ok(),
            Err(_) => None,
        };

        // Extract all secret names from the capabilities file
        let webhook_secret_name = capabilities_file
            .as_ref()
            .map(|f| f.webhook_secret_name())
            .unwrap_or_else(|| format!("{}_webhook_secret", name));

        let sig_key_secret_name = capabilities_file
            .as_ref()
            .and_then(|f| f.signature_key_secret_name().map(|s| s.to_string()));

        let hmac_secret_name = capabilities_file
            .as_ref()
            .and_then(|f| f.hmac_secret_name().map(|s| s.to_string()));

        // Refresh webhook secret
        if let Ok(secret) = self
            .secrets
            .get_decrypted(&self.user_id, &webhook_secret_name)
            .await
        {
            router
                .update_secret(name, secret.expose().to_string())
                .await;

            // Also inject the webhook_secret into the channel's runtime config
            let mut config_updates = std::collections::HashMap::new();
            config_updates.insert(
                "webhook_secret".to_string(),
                serde_json::Value::String(secret.expose().to_string()),
            );
            existing_channel.update_config(config_updates).await;
        }

        // Refresh signature key
        if let Some(ref sig_key_name) = sig_key_secret_name
            && let Ok(key_secret) = self
                .secrets
                .get_decrypted(&self.user_id, sig_key_name)
                .await
        {
            match router
                .register_signature_key(name, key_secret.expose())
                .await
            {
                Ok(()) => {
                    tracing::info!(channel = %name, "Refreshed signature verification key")
                }
                Err(e) => {
                    tracing::error!(channel = %name, error = %e, "Failed to refresh signature key")
                }
            }
        }

        // Refresh HMAC signing secret
        if let Some(ref hmac_secret_name_ref) = hmac_secret_name {
            match self
                .secrets
                .get_decrypted(&self.user_id, hmac_secret_name_ref)
                .await
            {
                Ok(secret) => {
                    router.register_hmac_secret(name, secret.expose()).await;
                    tracing::info!(channel = %name, "Refreshed HMAC signing secret");
                }
                Err(e) => {
                    tracing::warn!(channel = %name, error = %e, "HMAC secret not found");
                }
            }
        }

        // Refresh tunnel_url in case it wasn't set at startup
        if let Some(ref tunnel_url) = self.tunnel_url {
            let mut config_updates = std::collections::HashMap::new();
            config_updates.insert(
                "tunnel_url".to_string(),
                serde_json::Value::String(tunnel_url.clone()),
            );
            existing_channel.update_config(config_updates).await;
        }

        // Re-call on_start() to trigger webhook registration with the
        // now-available credentials (e.g., setWebhook for Telegram).
        if cred_count > 0 {
            match existing_channel.call_on_start().await {
                Ok(_config) => {
                    tracing::info!(
                        channel = %name,
                        "Re-ran on_start after credential refresh (webhook re-registered)"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        channel = %name,
                        error = %e,
                        "on_start failed after credential refresh"
                    );
                }
            }
        }

        tracing::info!(
            channel = %name,
            credentials_refreshed = cred_count,
            "Refreshed credentials and config on already-active channel"
        );

        Ok(ActivateResult {
            name: name.to_string(),
            kind: ExtensionKind::WasmChannel,
            tools_loaded: Vec::new(),
            message: format!(
                "Channel '{}' is already active; refreshed {} credential(s)",
                name, cred_count
            ),
        })
    }

    // ── Channel-relay extension methods ──────────────────────────────────

    /// Derive a stable instance ID from the relay config and user_id.
    fn relay_instance_id(&self, config: &crate::config::RelayConfig) -> String {
        config.instance_id.clone().unwrap_or_else(|| {
            uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_DNS, self.user_id.as_bytes()).to_string()
        })
    }

    /// Authenticate a channel-relay extension.
    ///
    /// For Slack: initiates OAuth flow (redirect-based).
    /// For Telegram: accepts a bot token, registers it with channel-relay,
    /// and stores the returned stream token.
    async fn auth_channel_relay(&self, name: &str) -> Result<AuthResult, ExtensionError> {
        // Check if already authenticated (stream token exists)
        let token_key = format!("relay:{}:stream_token", name);
        if self
            .secrets
            .exists(&self.user_id, &token_key)
            .await
            .unwrap_or(false)
        {
            return Ok(AuthResult::authenticated(name, ExtensionKind::ChannelRelay));
        }

        // Use relay config captured at startup
        let relay_config = self.relay_config()?;

        let instance_id = self.relay_instance_id(relay_config);
        let user_id_uuid = std::env::var("IRONCLAW_USER_ID").unwrap_or_else(|_| {
            uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_DNS, self.user_id.as_bytes()).to_string()
        });

        let client = crate::channels::relay::RelayClient::new(
            relay_config.url.clone(),
            relay_config.api_key.clone(),
            relay_config.request_timeout_secs,
        )
        .map_err(|e| ExtensionError::Config(e.to_string()))?;

        // OAuth redirect flow
        let callback_base = self
            .tunnel_url
            .clone()
            .or_else(|| relay_config.callback_url.clone())
            .unwrap_or_else(|| {
                let host = std::env::var("GATEWAY_HOST").unwrap_or_else(|_| "127.0.0.1".into());
                let port = std::env::var("GATEWAY_PORT").unwrap_or_else(|_| "3001".into());
                format!("http://{}:{}", host, port)
            });

        // Generate CSRF nonce for OAuth state parameter
        let state_nonce = uuid::Uuid::new_v4().to_string();
        let state_key = format!("relay:{}:oauth_state", name);
        // Delete any stale nonce before storing the new one
        let _ = self.secrets.delete(&self.user_id, &state_key).await;
        self.secrets
            .create(
                &self.user_id,
                CreateSecretParams::new(&state_key, &state_nonce),
            )
            .await
            .map_err(|e| ExtensionError::AuthFailed(format!("Failed to store OAuth state: {e}")))?;

        let callback_url = format!(
            "{}/oauth/slack/callback?state={}",
            callback_base, state_nonce
        );

        match client
            .initiate_oauth(&instance_id, &user_id_uuid, &callback_url)
            .await
        {
            Ok(auth_url) => Ok(AuthResult::awaiting_authorization(
                name,
                ExtensionKind::ChannelRelay,
                auth_url,
                "redirect".to_string(),
            )),
            Err(e) => Err(ExtensionError::AuthFailed(e.to_string())),
        }
    }

    /// Activate a channel-relay extension.
    async fn activate_channel_relay(&self, name: &str) -> Result<ActivateResult, ExtensionError> {
        let token_key = format!("relay:{}:stream_token", name);
        let team_id_key = format!("relay:{}:team_id", name);

        // Check if we have a stream token
        let stream_token = match self.secrets.get_decrypted(&self.user_id, &token_key).await {
            Ok(secret) => secret.expose().to_string(),
            Err(_) => {
                return Err(ExtensionError::AuthRequired);
            }
        };

        // Get team_id from settings
        let team_id = if let Some(ref store) = self.store {
            store
                .get_setting(&self.user_id, &team_id_key)
                .await
                .ok()
                .flatten()
                .and_then(|v| v.as_str().map(|s| s.to_string()))
                .unwrap_or_default()
        } else {
            String::new()
        };

        // Use relay config captured at startup
        let relay_config = self.relay_config()?;

        let instance_id = self.relay_instance_id(relay_config);

        let client = crate::channels::relay::RelayClient::new(
            relay_config.url.clone(),
            relay_config.api_key.clone(),
            relay_config.request_timeout_secs,
        )
        .map_err(|e| ExtensionError::ActivationFailed(e.to_string()))?;

        let channel = crate::channels::relay::RelayChannel::new_with_provider(
            client,
            crate::channels::relay::channel::RelayProvider::Slack,
            stream_token,
            team_id,
            instance_id,
            self.user_id.clone(),
        )
        .with_timeouts(
            relay_config.stream_timeout_secs,
            relay_config.backoff_initial_ms,
            relay_config.backoff_max_ms,
        );

        // Hot-add to channel manager
        let cm_guard = self.relay_channel_manager.read().await;
        let channel_mgr = cm_guard.as_ref().ok_or_else(|| {
            ExtensionError::ActivationFailed("Channel manager not initialized".to_string())
        })?;

        channel_mgr
            .hot_add(Box::new(channel))
            .await
            .map_err(|e| ExtensionError::ActivationFailed(e.to_string()))?;

        // Mark as active
        self.active_channel_names
            .write()
            .await
            .insert(name.to_string());
        self.persist_active_channels().await;

        // Broadcast status
        let status_msg = "Slack connected via channel relay".to_string();
        self.broadcast_extension_status(name, "active", Some(&status_msg))
            .await;

        Ok(ActivateResult {
            name: name.to_string(),
            kind: ExtensionKind::ChannelRelay,
            tools_loaded: Vec::new(),
            message: status_msg,
        })
    }

    /// Activate a channel-relay extension from stored credentials (for startup reconnect).
    pub async fn activate_stored_relay(&self, name: &str) -> Result<(), ExtensionError> {
        self.installed_relay_extensions
            .write()
            .await
            .insert(name.to_string());
        self.activate_channel_relay(name).await?;
        Ok(())
    }

    /// Determine what kind of installed extension this is.
    ///
    /// This is a read-only check — it never modifies `installed_relay_extensions`.
    /// To mark a relay extension as installed, use `activate_stored_relay()` or
    /// the explicit install flow.
    async fn determine_installed_kind(&self, name: &str) -> Result<ExtensionKind, ExtensionError> {
        // Check MCP servers first
        if self.get_mcp_server(name).await.is_ok() {
            return Ok(ExtensionKind::McpServer);
        }

        // Check WASM tools
        let wasm_path = self.wasm_tools_dir.join(format!("{}.wasm", name));
        if wasm_path.exists() {
            return Ok(ExtensionKind::WasmTool);
        }

        // Check WASM channels
        let channel_path = self.wasm_channels_dir.join(format!("{}.wasm", name));
        if channel_path.exists() {
            return Ok(ExtensionKind::WasmChannel);
        }

        // Check channel-relay extensions (installed in memory or has stored token)
        if self.installed_relay_extensions.read().await.contains(name) {
            return Ok(ExtensionKind::ChannelRelay);
        }
        // Also check if there's a stored stream token (persisted across restarts)
        if self
            .secrets
            .exists(&self.user_id, &format!("relay:{}:stream_token", name))
            .await
            .unwrap_or(false)
        {
            return Ok(ExtensionKind::ChannelRelay);
        }

        Err(ExtensionError::NotInstalled(format!(
            "'{}' is not installed as an MCP server, WASM tool, WASM channel, or channel relay",
            name
        )))
    }

    /// Reject names containing path separators or traversal sequences.
    fn validate_extension_name(name: &str) -> Result<(), ExtensionError> {
        if name.contains('/') || name.contains('\\') || name.contains("..") || name.contains('\0') {
            return Err(ExtensionError::InstallFailed(format!(
                "Invalid extension name '{}': contains path separator or traversal characters",
                name
            )));
        }
        Ok(())
    }

    async fn cleanup_expired_auths(&self) {
        let mut pending = self.pending_auth.write().await;
        pending.retain(|_, auth| {
            let expired = auth.created_at.elapsed() >= std::time::Duration::from_secs(300);
            if expired {
                // Abort the background listener task to free port 9876
                if let Some(ref handle) = auth.task_handle {
                    handle.abort();
                }
            }
            !expired
        });
    }

    /// Get the setup schema for an extension (secret fields and their status).
    pub async fn get_setup_schema(
        &self,
        name: &str,
    ) -> Result<Vec<crate::channels::web::types::SecretFieldInfo>, ExtensionError> {
        let kind = self.determine_installed_kind(name).await?;
        match kind {
            ExtensionKind::WasmChannel => {
                let cap_path = self
                    .wasm_channels_dir
                    .join(format!("{}.capabilities.json", name));
                if !cap_path.exists() {
                    return Ok(Vec::new());
                }
                let cap_bytes = tokio::fs::read(&cap_path)
                    .await
                    .map_err(|e| ExtensionError::Other(e.to_string()))?;
                let cap_file =
                    crate::channels::wasm::ChannelCapabilitiesFile::from_bytes(&cap_bytes)
                        .map_err(|e| ExtensionError::Other(e.to_string()))?;

                let mut fields = Vec::new();
                for secret in &cap_file.setup.required_secrets {
                    let provided = self
                        .secrets
                        .exists(&self.user_id, &secret.name)
                        .await
                        .unwrap_or(false);
                    fields.push(crate::channels::web::types::SecretFieldInfo {
                        name: secret.name.clone(),
                        prompt: secret.prompt.clone(),
                        optional: secret.optional,
                        provided,
                        auto_generate: secret.auto_generate.is_some(),
                    });
                }
                Ok(fields)
            }
            ExtensionKind::WasmTool => {
                let Some(cap_file) = self.load_tool_capabilities(name).await else {
                    return Ok(Vec::new());
                };

                let mut fields = Vec::new();
                if let Some(setup) = &cap_file.setup {
                    for secret in &setup.required_secrets {
                        // Skip OAuth client_id/secret fields that resolve automatically
                        if Self::is_auto_resolved_oauth_field(&secret.name, &cap_file) {
                            continue;
                        }
                        let provided = self
                            .secrets
                            .exists(&self.user_id, &secret.name)
                            .await
                            .unwrap_or(false);
                        fields.push(crate::channels::web::types::SecretFieldInfo {
                            name: secret.name.clone(),
                            prompt: secret.prompt.clone(),
                            optional: secret.optional,
                            provided,
                            auto_generate: false,
                        });
                    }
                }
                Ok(fields)
            }
            _ => Ok(Vec::new()),
        }
    }

    /// Save setup secrets for an extension, validating names against the capabilities schema.
    ///
    /// Configure secrets for an extension: validate, store, auto-generate, and activate.
    ///
    /// This is the single entrypoint for providing secrets to any extension.
    /// Both the chat auth flow and the Extensions tab setup form call this method.
    ///
    /// - Validates tokens against `validation_endpoint` (if declared in capabilities)
    /// - Stores secrets in the encrypted secrets store
    /// - Auto-generates missing secrets (e.g., webhook keys)
    /// - Activates the extension after configuration
    pub async fn configure(
        &self,
        name: &str,
        secrets: &std::collections::HashMap<String, String>,
    ) -> Result<ConfigureResult, ExtensionError> {
        let kind = self.determine_installed_kind(name).await?;

        // Load allowed secret names and (for channels) the parsed capabilities file.
        // The capabilities file is parsed once here and reused for validation_endpoint
        // and auto-generation below, avoiding redundant I/O + JSON parsing.
        let mut channel_cap_file: Option<crate::channels::wasm::ChannelCapabilitiesFile> = None;
        let allowed: std::collections::HashSet<String> = match kind {
            ExtensionKind::WasmChannel => {
                let cap_path = self
                    .wasm_channels_dir
                    .join(format!("{}.capabilities.json", name));
                if !cap_path.exists() {
                    return Err(ExtensionError::Other(format!(
                        "Capabilities file not found for '{}'",
                        name
                    )));
                }
                let cap_bytes = tokio::fs::read(&cap_path)
                    .await
                    .map_err(|e| ExtensionError::Other(e.to_string()))?;
                let cap_file =
                    crate::channels::wasm::ChannelCapabilitiesFile::from_bytes(&cap_bytes)
                        .map_err(|e| ExtensionError::Other(e.to_string()))?;
                let names = cap_file
                    .setup
                    .required_secrets
                    .iter()
                    .map(|s| s.name.clone())
                    .collect();
                channel_cap_file = Some(cap_file);
                names
            }
            ExtensionKind::WasmTool => {
                let cap_file = self.load_tool_capabilities(name).await.ok_or_else(|| {
                    ExtensionError::Other(format!("Capabilities file not found for '{}'", name))
                })?;
                let mut names: std::collections::HashSet<String> = std::collections::HashSet::new();
                if let Some(ref s) = cap_file.setup {
                    names.extend(s.required_secrets.iter().map(|s| s.name.clone()));
                }
                // Also allow storing the auth token secret directly
                if let Some(ref auth) = cap_file.auth {
                    names.insert(auth.secret_name.clone());
                }
                if names.is_empty() {
                    return Err(ExtensionError::Other(format!(
                        "Tool '{}' has no setup or auth schema — no secrets to configure",
                        name
                    )));
                }
                names
            }
            ExtensionKind::McpServer => {
                let server = self
                    .get_mcp_server(name)
                    .await
                    .map_err(|e| ExtensionError::NotInstalled(e.to_string()))?;
                let mut names = std::collections::HashSet::new();
                names.insert(server.token_secret_name());
                names
            }
            ExtensionKind::ChannelRelay => {
                let mut names = std::collections::HashSet::new();
                names.insert(format!("relay:{}:stream_token", name));
                names
            }
        };

        // Validate secrets against the validation_endpoint if declared in capabilities.
        // The endpoint URL template uses {secret_name} placeholders that are
        // substituted with the provided secret value before making the request.
        if let Some(ref cap_file) = channel_cap_file
            && let Some(ref endpoint_template) = cap_file.setup.validation_endpoint
            && let Some(secret_def) = cap_file
                .setup
                .required_secrets
                .iter()
                .find(|s| !s.optional && secrets.contains_key(&s.name))
            && let Some(token_value) = secrets.get(&secret_def.name)
        {
            let token = token_value.trim();
            if !token.is_empty() {
                let encoded =
                    url::form_urlencoded::byte_serialize(token.as_bytes()).collect::<String>();
                let url = endpoint_template.replace(&format!("{{{}}}", secret_def.name), &encoded);
                // SSRF defense: block private IPs, localhost, cloud metadata endpoints
                crate::tools::builtin::skill_tools::validate_fetch_url(&url)
                    .map_err(|e| ExtensionError::Other(format!("SSRF blocked: {}", e)))?;
                let resp = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(10))
                    .build()
                    .map_err(|e| ExtensionError::Other(e.to_string()))?
                    .get(&url)
                    .send()
                    .await
                    // Transport errors are infrastructure failures, not token issues
                    .map_err(|e| {
                        ExtensionError::Other(format!("Token validation request failed: {}", e))
                    })?;
                if !resp.status().is_success() {
                    return Err(ExtensionError::ValidationFailed(format!(
                        "Invalid token (API returned {})",
                        resp.status()
                    )));
                }
            }
        }

        // Validate and store each submitted secret
        for (secret_name, secret_value) in secrets {
            if !allowed.contains(secret_name.as_str()) {
                return Err(ExtensionError::Other(format!(
                    "Unknown secret '{}' for extension '{}'",
                    secret_name, name
                )));
            }
            if secret_value.trim().is_empty() {
                continue;
            }
            let params =
                CreateSecretParams::new(secret_name, secret_value).with_provider(name.to_string());
            self.secrets
                .create(&self.user_id, params)
                .await
                .map_err(|e| ExtensionError::AuthFailed(e.to_string()))?;
        }

        // Auto-generate any missing secrets (channel-only feature)
        if let Some(ref cap_file) = channel_cap_file {
            for secret_def in &cap_file.setup.required_secrets {
                if let Some(ref auto_gen) = secret_def.auto_generate {
                    let already_provided = secrets
                        .get(&secret_def.name)
                        .is_some_and(|v| !v.trim().is_empty());
                    let already_stored = self
                        .secrets
                        .exists(&self.user_id, &secret_def.name)
                        .await
                        .unwrap_or(false);
                    if !already_provided && !already_stored {
                        use rand::RngCore;
                        use rand::rngs::OsRng;
                        let mut bytes = vec![0u8; auto_gen.length];
                        OsRng.fill_bytes(&mut bytes);
                        let hex_value: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
                        let params = CreateSecretParams::new(&secret_def.name, &hex_value)
                            .with_provider(name.to_string());
                        self.secrets
                            .create(&self.user_id, params)
                            .await
                            .map_err(|e| ExtensionError::AuthFailed(e.to_string()))?;
                        tracing::info!(
                            "Auto-generated secret '{}' for channel '{}'",
                            secret_def.name,
                            name
                        );
                    }
                }
            }
        }

        // For tools, save and attempt auto-activation, then check auth.
        if kind == ExtensionKind::WasmTool {
            match self.activate_wasm_tool(name).await {
                Ok(result) => {
                    // Delete existing OAuth token so auth() starts a fresh flow.
                    // Done AFTER activation succeeds to avoid losing tokens on failure.
                    // This covers Reconfigure: user wants to re-auth (switch account, update creds).
                    if let Some(cap) = self.load_tool_capabilities(name).await
                        && let Some(ref auth_cfg) = cap.auth
                        && auth_cfg.oauth.is_some()
                    {
                        let _ = self
                            .secrets
                            .delete(&self.user_id, &auth_cfg.secret_name)
                            .await;
                        let _ = self
                            .secrets
                            .delete(&self.user_id, &format!("{}_scopes", auth_cfg.secret_name))
                            .await;
                        let _ = self
                            .secrets
                            .delete(
                                &self.user_id,
                                &format!("{}_refresh_token", auth_cfg.secret_name),
                            )
                            .await;
                    }

                    // Check if auth is needed (OAuth or manual token).
                    // This is safe to call here — cancel-and-retry prevents port conflicts.
                    let mut auth_url = None;
                    // Box::pin breaks the async recursion cycle:
                    // auth() → auth_wasm_tool() → (OAuth) → configure() → auth()
                    if let Ok(auth_result) = Box::pin(self.auth(name)).await {
                        auth_url = auth_result.auth_url().map(String::from);
                    }
                    let message = if auth_url.is_some() {
                        format!(
                            "Configuration saved and tool '{}' activated. Complete OAuth in your browser.",
                            name
                        )
                    } else {
                        format!(
                            "Configuration saved and tool '{}' activated. {}",
                            name, result.message
                        )
                    };
                    return Ok(ConfigureResult {
                        message,
                        activated: true,
                        auth_url,
                    });
                }
                Err(e) => {
                    tracing::debug!(
                        "Auto-activation of tool '{}' after setup failed: {}",
                        name,
                        e
                    );
                    return Ok(ConfigureResult {
                        message: format!("Configuration saved for '{}'.", name),
                        activated: false,
                        auth_url: None,
                    });
                }
            }
        }

        // Activate the extension now that secrets are saved.
        // Dispatch by kind — WasmTool was already handled above with an early return.
        let activate_result = match kind {
            ExtensionKind::WasmChannel => self.activate_wasm_channel(name).await,
            ExtensionKind::McpServer => self.activate_mcp(name).await,
            ExtensionKind::ChannelRelay => self.activate_channel_relay(name).await,
            ExtensionKind::WasmTool => {
                // WasmTool is handled above and returns early; this branch is unreachable.
                return Ok(ConfigureResult {
                    message: format!("Configuration saved for '{}'.", name),
                    activated: false,
                    auth_url: None,
                });
            }
        };

        match activate_result {
            Ok(result) => {
                self.activation_errors.write().await.remove(name);
                self.broadcast_extension_status(name, "active", None).await;
                Ok(ConfigureResult {
                    message: format!(
                        "Configuration saved and '{}' activated. {}",
                        name, result.message
                    ),
                    activated: true,
                    auth_url: None,
                })
            }
            Err(e) => {
                let error_msg = e.to_string();
                tracing::warn!(
                    extension = name,
                    error = %e,
                    "Saved configuration but activation failed"
                );
                self.activation_errors
                    .write()
                    .await
                    .insert(name.to_string(), error_msg.clone());
                self.broadcast_extension_status(name, "failed", Some(&error_msg))
                    .await;
                Ok(ConfigureResult {
                    message: format!(
                        "Configuration saved for '{}'. Activation failed: {}",
                        name, e
                    ),
                    activated: false,
                    auth_url: None,
                })
            }
        }
    }

    /// Convenience wrapper: configure a single token for an extension.
    ///
    /// Determines the primary secret name from the extension's capabilities,
    /// then delegates to [`configure()`]. Use this when the caller only has
    /// a bare token value (e.g., from the chat auth card or WebSocket auth).
    pub async fn configure_token(
        &self,
        name: &str,
        token: &str,
    ) -> Result<ConfigureResult, ExtensionError> {
        let kind = self.determine_installed_kind(name).await?;
        let secret_name = match kind {
            ExtensionKind::WasmChannel => {
                let cap_path = self
                    .wasm_channels_dir
                    .join(format!("{}.capabilities.json", name));
                let cap_bytes = tokio::fs::read(&cap_path)
                    .await
                    .map_err(|e| ExtensionError::Other(e.to_string()))?;
                let cap_file =
                    crate::channels::wasm::ChannelCapabilitiesFile::from_bytes(&cap_bytes)
                        .map_err(|e| ExtensionError::Other(e.to_string()))?;
                // Pick the first *missing* non-optional secret so re-configure
                // of a second secret works for multi-secret channels.
                let mut target = None;
                for s in &cap_file.setup.required_secrets {
                    if s.optional {
                        continue;
                    }
                    if !self
                        .secrets
                        .exists(&self.user_id, &s.name)
                        .await
                        .unwrap_or(false)
                    {
                        target = Some(s.name.clone());
                        break;
                    }
                }
                // Fall back to first non-optional if all exist (overwrite)
                target
                    .or_else(|| {
                        cap_file
                            .setup
                            .required_secrets
                            .iter()
                            .find(|s| !s.optional)
                            .map(|s| s.name.clone())
                    })
                    .ok_or_else(|| {
                        ExtensionError::Other(format!("Channel '{}' has no required secrets", name))
                    })?
            }
            ExtensionKind::WasmTool => {
                let cap = self.load_tool_capabilities(name).await.ok_or_else(|| {
                    ExtensionError::Other(format!("Capabilities not found for '{}'", name))
                })?;
                // Prefer auth secret, then first missing setup secret
                if let Some(ref auth) = cap.auth {
                    if !self
                        .secrets
                        .exists(&self.user_id, &auth.secret_name)
                        .await
                        .unwrap_or(false)
                    {
                        auth.secret_name.clone()
                    } else if let Some(ref setup) = cap.setup {
                        // Auth secret exists, find first missing setup secret
                        let mut found = None;
                        for s in &setup.required_secrets {
                            if !self
                                .secrets
                                .exists(&self.user_id, &s.name)
                                .await
                                .unwrap_or(false)
                            {
                                found = Some(s.name.clone());
                                break;
                            }
                        }
                        found.unwrap_or_else(|| auth.secret_name.clone())
                    } else {
                        auth.secret_name.clone()
                    }
                } else {
                    cap.setup
                        .as_ref()
                        .and_then(|s| s.required_secrets.first())
                        .map(|s| s.name.clone())
                        .ok_or_else(|| {
                            ExtensionError::Other(format!(
                                "Tool '{}' has no auth or setup secrets",
                                name
                            ))
                        })?
                }
            }
            ExtensionKind::McpServer => {
                let server = self
                    .get_mcp_server(name)
                    .await
                    .map_err(|e| ExtensionError::NotInstalled(e.to_string()))?;
                server.token_secret_name()
            }
            ExtensionKind::ChannelRelay => format!("relay:{}:stream_token", name),
        };

        let mut secrets = std::collections::HashMap::new();
        secrets.insert(secret_name, token.to_string());
        self.configure(name, &secrets).await
    }

    /// Read a capabilities.json file and revoke its credential mappings from
    /// the shared credential registry, so removed extensions lose injection
    /// authority immediately.
    async fn revoke_credential_mappings(&self, cap_path: &std::path::Path) {
        if !cap_path.exists() {
            return;
        }
        let Ok(bytes) = tokio::fs::read(cap_path).await else {
            return;
        };
        // Extract secret names from the capabilities JSON.
        // Structure: { "http": { "credentials": { "<key>": { "secret_name": "..." } } } }
        let Ok(json) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            return;
        };
        let secret_names: Vec<String> = json
            .get("http")
            .and_then(|h| h.get("credentials"))
            .and_then(|c| c.as_object())
            .map(|creds| {
                creds
                    .values()
                    .filter_map(|v| v.get("secret_name").and_then(|s| s.as_str()))
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default();

        if secret_names.is_empty() {
            return;
        }

        if let Some(cr) = self.tool_registry.credential_registry() {
            cr.remove_mappings_for_secrets(&secret_names);
            tracing::info!(
                secrets = ?secret_names,
                "Revoked credential mappings for removed extension"
            );
        }
    }

    async fn unregister_hook_prefix(&self, prefix: &str) -> usize {
        let Some(ref hooks) = self.hooks else {
            return 0;
        };

        let names = hooks.list().await;
        let mut removed = 0;
        for hook_name in names {
            if hook_name.starts_with(prefix) && hooks.unregister(&hook_name).await {
                removed += 1;
            }
        }
        removed
    }
}

/// Inject credentials for a channel based on naming convention.
///
/// Looks for secrets matching the pattern `{channel_name}_*` and injects them
/// as credential placeholders (e.g., `telegram_bot_token` -> `{TELEGRAM_BOT_TOKEN}`).
///
/// Falls back to environment variables starting with the uppercase channel name
/// prefix (e.g., `TELEGRAM_` for channel `telegram`) for missing credentials.
///
/// Returns the number of credentials injected.
async fn inject_channel_credentials_from_secrets(
    channel: &Arc<crate::channels::wasm::WasmChannel>,
    secrets: Option<&dyn SecretsStore>,
    channel_name: &str,
    user_id: &str,
) -> Result<usize, String> {
    let mut count = 0;
    let mut injected_placeholders = std::collections::HashSet::new();

    // 1. Try injecting from persistent secrets store if available
    if let Some(secrets) = secrets {
        let all_secrets = secrets
            .list(user_id)
            .await
            .map_err(|e| format!("Failed to list secrets: {}", e))?;

        let prefix = format!("{}_", channel_name.to_ascii_lowercase());

        for secret_meta in all_secrets {
            if !secret_meta.name.to_ascii_lowercase().starts_with(&prefix) {
                continue;
            }

            let decrypted = match secrets.get_decrypted(user_id, &secret_meta.name).await {
                Ok(d) => d,
                Err(e) => {
                    tracing::warn!(
                        secret = %secret_meta.name,
                        error = %e,
                        "Failed to decrypt secret for channel credential injection"
                    );
                    continue;
                }
            };

            let placeholder = secret_meta.name.to_uppercase();
            channel
                .set_credential(&placeholder, decrypted.expose().to_string())
                .await;
            injected_placeholders.insert(placeholder);
            count += 1;
        }
    }

    // 2. Fallback to environment variables for missing credentials
    count += inject_env_credentials(channel, channel_name, &injected_placeholders).await;

    Ok(count)
}

/// Inject missing credentials from environment variables.
///
/// Only environment variables starting with the uppercase channel name prefix
/// (e.g., `TELEGRAM_` for channel `telegram`) are considered for security.
async fn inject_env_credentials(
    channel: &Arc<crate::channels::wasm::WasmChannel>,
    channel_name: &str,
    already_injected: &std::collections::HashSet<String>,
) -> usize {
    if channel_name.trim().is_empty() {
        return 0;
    }

    let caps = channel.capabilities();
    let Some(ref http_cap) = caps.tool_capabilities.http else {
        return 0;
    };

    let placeholders: Vec<String> = http_cap
        .credentials
        .values()
        .map(|m| m.secret_name.to_uppercase())
        .collect();

    let resolved = resolve_env_credentials(&placeholders, channel_name, already_injected);
    let count = resolved.len();
    for (placeholder, value) in resolved {
        channel.set_credential(&placeholder, value).await;
    }
    count
}

/// Pure helper: from a list of credential placeholder names, return those that
/// pass the channel-prefix security check and have a non-empty env var value.
///
/// Placeholders already covered by the secrets store (`already_injected`) are
/// skipped. Only names starting with `{CHANNEL_NAME}_` are allowed to prevent
/// a WASM channel from reading unrelated host credentials (e.g. `AWS_SECRET_ACCESS_KEY`).
pub(crate) fn resolve_env_credentials(
    placeholders: &[String],
    channel_name: &str,
    already_injected: &std::collections::HashSet<String>,
) -> Vec<(String, String)> {
    if channel_name.trim().is_empty() {
        return Vec::new();
    }

    let prefix = format!("{}_", channel_name.to_ascii_uppercase());
    let mut out = Vec::new();

    for placeholder in placeholders {
        if already_injected.contains(placeholder) {
            continue;
        }
        if !placeholder.starts_with(&prefix) {
            tracing::warn!(
                channel = %channel_name,
                placeholder = %placeholder,
                "Ignoring non-prefixed credential placeholder in environment fallback"
            );
            continue;
        }
        if let Ok(value) = std::env::var(placeholder)
            && !value.is_empty()
        {
            out.push((placeholder.clone(), value));
        }
    }
    out
}

/// Infer the extension kind from a URL.
fn infer_kind_from_url(url: &str) -> ExtensionKind {
    if url.ends_with(".wasm") || url.ends_with(".tar.gz") {
        ExtensionKind::WasmTool
    } else {
        ExtensionKind::McpServer
    }
}

/// Decision from `fallback_decision`: should we try the fallback source or
/// return the primary result as-is?
enum FallbackDecision {
    /// Return the primary result directly (success or non-retriable error).
    Return,
    /// Primary failed with a retriable error and a fallback source is available.
    TryFallback,
}

/// Decide whether to attempt a fallback install based on the primary result
/// and the availability of a fallback source.
fn fallback_decision(
    primary_result: &Result<InstallResult, ExtensionError>,
    fallback_source: &Option<Box<ExtensionSource>>,
) -> FallbackDecision {
    match (primary_result, fallback_source) {
        // Success — no fallback needed
        (Ok(_), _) => FallbackDecision::Return,
        // AlreadyInstalled — don't try building from source
        (Err(ExtensionError::AlreadyInstalled(_)), _) => FallbackDecision::Return,
        // Failed with a fallback available — try it
        (Err(_), Some(_)) => FallbackDecision::TryFallback,
        // Failed with no fallback — return the error
        (Err(_), None) => FallbackDecision::Return,
    }
}

/// Combine primary and fallback errors into a single error.
///
/// Preserves `AlreadyInstalled` from the fallback directly; otherwise wraps
/// both errors into the structured `ExtensionError::FallbackFailed` variant.
fn combine_install_errors(
    primary_err: ExtensionError,
    fallback_err: ExtensionError,
) -> ExtensionError {
    if matches!(fallback_err, ExtensionError::AlreadyInstalled(_)) {
        return fallback_err;
    }
    ExtensionError::FallbackFailed {
        primary: Box::new(primary_err),
        fallback: Box::new(fallback_err),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::extensions::ExtensionManager;
    use crate::extensions::manager::{
        FallbackDecision, combine_install_errors, fallback_decision, infer_kind_from_url,
    };
    use crate::extensions::{ExtensionError, ExtensionKind, ExtensionSource, InstallResult};

    #[test]
    fn test_infer_kind_from_url() {
        assert_eq!(
            infer_kind_from_url("https://example.com/tool.wasm"),
            ExtensionKind::WasmTool
        );
        assert_eq!(
            infer_kind_from_url("https://example.com/tool-wasm32-wasip2.tar.gz"),
            ExtensionKind::WasmTool
        );
        assert_eq!(
            infer_kind_from_url("https://mcp.notion.com"),
            ExtensionKind::McpServer
        );
        assert_eq!(
            infer_kind_from_url("https://example.com/mcp"),
            ExtensionKind::McpServer
        );
    }

    // ---- fallback install logic tests ----

    fn make_ok_result() -> Result<InstallResult, ExtensionError> {
        Ok(InstallResult {
            name: "test".to_string(),
            kind: ExtensionKind::WasmTool,
            message: "Installed".to_string(),
        })
    }

    fn make_fallback_source() -> Option<Box<ExtensionSource>> {
        Some(Box::new(ExtensionSource::WasmBuildable {
            source_dir: "tools-src/test".to_string(),
            build_dir: Some("tools-src/test".to_string()),
            crate_name: Some("test-tool".to_string()),
        }))
    }

    #[test]
    fn test_fallback_decision_success_returns_directly() {
        let result = make_ok_result();
        let fallback = make_fallback_source();
        assert!(matches!(
            fallback_decision(&result, &fallback),
            FallbackDecision::Return
        ));
    }

    #[test]
    fn test_fallback_decision_already_installed_skips_fallback() {
        let result: Result<InstallResult, ExtensionError> =
            Err(ExtensionError::AlreadyInstalled("test".to_string()));
        let fallback = make_fallback_source();
        assert!(matches!(
            fallback_decision(&result, &fallback),
            FallbackDecision::Return
        ));
    }

    #[test]
    fn test_fallback_decision_download_failed_triggers_fallback() {
        let result: Result<InstallResult, ExtensionError> =
            Err(ExtensionError::DownloadFailed("404 Not Found".to_string()));
        let fallback = make_fallback_source();
        assert!(matches!(
            fallback_decision(&result, &fallback),
            FallbackDecision::TryFallback
        ));
    }

    #[test]
    fn test_fallback_decision_error_without_fallback_returns() {
        let result: Result<InstallResult, ExtensionError> =
            Err(ExtensionError::DownloadFailed("404 Not Found".to_string()));
        let fallback = None;
        assert!(matches!(
            fallback_decision(&result, &fallback),
            FallbackDecision::Return
        ));
    }

    #[test]
    fn test_combine_errors_includes_both_messages() {
        let primary = ExtensionError::DownloadFailed("404 Not Found".to_string());
        let fallback = ExtensionError::InstallFailed("cargo not found".to_string());
        let combined = combine_install_errors(primary, fallback);
        assert!(
            matches!(combined, ExtensionError::FallbackFailed { .. }),
            "Expected FallbackFailed, got: {combined:?}"
        );
        let msg = combined.to_string();
        assert!(msg.contains("404 Not Found"), "missing primary: {msg}");
        assert!(msg.contains("cargo not found"), "missing fallback: {msg}");
    }

    #[test]
    fn test_combine_errors_forwards_already_installed_from_fallback() {
        let primary = ExtensionError::DownloadFailed("404".to_string());
        let fallback = ExtensionError::AlreadyInstalled("test".to_string());
        let combined = combine_install_errors(primary, fallback);
        assert!(
            matches!(combined, ExtensionError::AlreadyInstalled(ref name) if name == "test"),
            "Expected AlreadyInstalled, got: {combined:?}"
        );
    }

    // === QA Plan P2 - 2.4: Extension registry collision tests (filesystem) ===

    #[test]
    fn test_tool_and_channel_paths_are_separate() {
        // Verify that a WASM tool named "telegram" and a WASM channel named
        // "telegram" use different filesystem paths and don't overwrite each other.
        let dir = tempfile::tempdir().expect("temp dir");
        let tools_dir = dir.path().join("tools");
        let channels_dir = dir.path().join("channels");
        std::fs::create_dir_all(&tools_dir).unwrap();
        std::fs::create_dir_all(&channels_dir).unwrap();

        let name = "telegram";
        let tool_wasm = tools_dir.join(format!("{}.wasm", name));
        let channel_wasm = channels_dir.join(format!("{}.wasm", name));

        // Simulate installing both.
        std::fs::write(&tool_wasm, b"tool-payload").unwrap();
        std::fs::write(&channel_wasm, b"channel-payload").unwrap();

        // Both files exist and contain distinct content.
        assert!(tool_wasm.exists());
        assert!(channel_wasm.exists());
        assert_ne!(
            std::fs::read(&tool_wasm).unwrap(),
            std::fs::read(&channel_wasm).unwrap(),
            "Tool and channel files must be independent"
        );

        // Removing one doesn't affect the other.
        std::fs::remove_file(&tool_wasm).unwrap();
        assert!(!tool_wasm.exists());
        assert!(
            channel_wasm.exists(),
            "Removing tool must not affect channel"
        );
    }

    #[test]
    fn test_determine_kind_priority_tools_before_channels() {
        // When a name exists in both tools and channels dirs,
        // determine_installed_kind checks tools first (wasm_tools_dir).
        // This test documents the priority order.
        let dir = tempfile::tempdir().expect("temp dir");
        let tools_dir = dir.path().join("tools");
        let channels_dir = dir.path().join("channels");
        std::fs::create_dir_all(&tools_dir).unwrap();
        std::fs::create_dir_all(&channels_dir).unwrap();

        let name = "ambiguous";
        let tool_wasm = tools_dir.join(format!("{}.wasm", name));
        let channel_wasm = channels_dir.join(format!("{}.wasm", name));

        // Only channel exists → channel kind.
        std::fs::write(&channel_wasm, b"channel").unwrap();
        assert!(!tool_wasm.exists());
        assert!(channel_wasm.exists());

        // Both exist → tools dir checked first.
        std::fs::write(&tool_wasm, b"tool").unwrap();
        assert!(tool_wasm.exists());
        assert!(channel_wasm.exists());
        // This documents the determine_installed_kind priority:
        // tools are checked before channels.

        // Only tool exists → tool kind.
        std::fs::remove_file(&channel_wasm).unwrap();
        assert!(tool_wasm.exists());
        assert!(!channel_wasm.exists());
    }

    // === WASM runtime availability tests ===
    //
    // Regression tests for a bug where the WASM runtime was only created at
    // startup when the tools directory already existed. Extensions installed
    // after startup (e.g. via the web UI) would fail with "WASM runtime not
    // available" because the ExtensionManager had `wasm_tool_runtime: None`.

    /// Build a minimal ExtensionManager suitable for unit tests.
    fn make_test_manager_with_dirs(
        wasm_runtime: Option<Arc<crate::tools::wasm::WasmToolRuntime>>,
        tools_dir: std::path::PathBuf,
        channels_dir: std::path::PathBuf,
    ) -> crate::extensions::manager::ExtensionManager {
        use crate::secrets::{InMemorySecretsStore, SecretsCrypto};
        use crate::tools::mcp::process::McpProcessManager;
        use crate::tools::mcp::session::McpSessionManager;

        std::fs::create_dir_all(&tools_dir).ok();
        std::fs::create_dir_all(&channels_dir).ok();

        let key = secrecy::SecretString::from(crate::secrets::keychain::generate_master_key_hex());
        let crypto = Arc::new(SecretsCrypto::new(key).expect("crypto"));
        let secrets: Arc<dyn crate::secrets::SecretsStore + Send + Sync> =
            Arc::new(InMemorySecretsStore::new(crypto));
        let tools = Arc::new(crate::tools::ToolRegistry::new());
        let mcp = Arc::new(McpSessionManager::new());

        crate::extensions::manager::ExtensionManager::new(
            mcp,
            Arc::new(McpProcessManager::new()),
            secrets,
            tools,
            None, // hooks
            wasm_runtime,
            tools_dir,
            channels_dir,
            None, // tunnel_url
            "test".to_string(),
            None, // db
            vec![],
        )
    }

    fn make_test_manager(
        wasm_runtime: Option<Arc<crate::tools::wasm::WasmToolRuntime>>,
        tools_dir: std::path::PathBuf,
    ) -> crate::extensions::manager::ExtensionManager {
        make_test_manager_with_dirs(wasm_runtime, tools_dir.clone(), tools_dir)
    }

    #[tokio::test]
    async fn test_activate_wasm_tool_with_runtime_passes_runtime_check() {
        // When the ExtensionManager has a WASM runtime, activation should get
        // past the "WASM runtime not available" check. It will still fail
        // because no .wasm file exists on disk — but the error message should
        // be "not found", NOT "WASM runtime not available".
        let dir = tempfile::tempdir().expect("temp dir");
        let config = crate::tools::wasm::WasmRuntimeConfig::for_testing();
        let runtime = Arc::new(crate::tools::wasm::WasmToolRuntime::new(config).expect("runtime"));
        let mgr = make_test_manager(Some(runtime), dir.path().to_path_buf());

        let err = mgr.activate("nonexistent").await.unwrap_err();
        let msg = err.to_string();
        assert!(
            !msg.contains("WASM runtime not available"),
            "Should not fail on runtime check, got: {msg}"
        );
        assert!(
            msg.contains("not found")
                || msg.contains("not installed")
                || msg.contains("Not installed"),
            "Should fail on missing file, got: {msg}"
        );
    }

    #[tokio::test]
    async fn test_activate_wasm_tool_without_runtime_fails_with_runtime_error() {
        // When the ExtensionManager has no WASM runtime (None), activation
        // must fail with the "WASM runtime not available" message.
        let dir = tempfile::tempdir().expect("temp dir");
        // Write a fake .wasm file so we don't fail on "not found" first.
        std::fs::write(dir.path().join("fake.wasm"), b"not-a-real-wasm").unwrap();

        let mgr = make_test_manager(None, dir.path().to_path_buf());

        let err = mgr.activate("fake").await.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("WASM runtime not available"),
            "Expected runtime not available error, got: {msg}"
        );
    }

    #[test]
    fn test_capabilities_files_also_separate() {
        // capabilities.json files for tools and channels should also be separate.
        let dir = tempfile::tempdir().expect("temp dir");
        let tools_dir = dir.path().join("tools");
        let channels_dir = dir.path().join("channels");
        std::fs::create_dir_all(&tools_dir).unwrap();
        std::fs::create_dir_all(&channels_dir).unwrap();

        let name = "telegram";
        let tool_cap = tools_dir.join(format!("{}.capabilities.json", name));
        let channel_cap = channels_dir.join(format!("{}.capabilities.json", name));

        let tool_caps = r#"{"required_secrets":["TELEGRAM_API_KEY"]}"#;
        let channel_caps = r#"{"required_secrets":["TELEGRAM_BOT_TOKEN"]}"#;

        std::fs::write(&tool_cap, tool_caps).unwrap();
        std::fs::write(&channel_cap, channel_caps).unwrap();

        // Both exist with distinct content.
        assert_eq!(std::fs::read_to_string(&tool_cap).unwrap(), tool_caps);
        assert_eq!(std::fs::read_to_string(&channel_cap).unwrap(), channel_caps);
    }

    #[tokio::test]
    async fn test_upgrade_no_installed_extensions() {
        let manager = make_manager_with_temp_dirs();
        let result = manager.upgrade(None).await.unwrap();
        assert!(result.results.is_empty());
        assert!(result.message.contains("No WASM extensions installed"));
    }

    #[tokio::test]
    async fn test_upgrade_mcp_server_rejected() {
        let manager = make_manager_with_temp_dirs();
        // MCP servers can't be upgraded via tool_upgrade
        let err = manager.upgrade(Some("some-mcp")).await;
        // It will fail with NotInstalled because there's no MCP server named "some-mcp",
        // but if it were installed, the MCP code path would be rejected.
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn test_upgrade_up_to_date_extension() {
        let dir = tempfile::tempdir().expect("temp dir");
        let channels_dir = dir.path().join("channels");
        std::fs::create_dir_all(&channels_dir).unwrap();

        // Write a fake .wasm file and capabilities with current WIT version
        let wasm_path = channels_dir.join("test-channel.wasm");
        std::fs::write(&wasm_path, b"\0asm fake").unwrap();

        let cap_path = channels_dir.join("test-channel.capabilities.json");
        let caps = serde_json::json!({
            "type": "channel",
            "name": "test-channel",
            "wit_version": crate::tools::wasm::WIT_CHANNEL_VERSION,
        });
        std::fs::write(&cap_path, serde_json::to_string(&caps).unwrap()).unwrap();

        let manager = make_manager_custom_dirs(dir.path().join("tools"), channels_dir);

        let result = manager.upgrade(Some("test-channel")).await.unwrap();
        assert_eq!(result.results.len(), 1);
        assert_eq!(result.results[0].status, "already_up_to_date");
    }

    #[tokio::test]
    async fn test_upgrade_outdated_not_in_registry() {
        let dir = tempfile::tempdir().expect("temp dir");
        let channels_dir = dir.path().join("channels");
        std::fs::create_dir_all(&channels_dir).unwrap();

        // Write a fake .wasm file and capabilities with OLD WIT version
        let wasm_path = channels_dir.join("custom-channel.wasm");
        std::fs::write(&wasm_path, b"\0asm fake").unwrap();

        let cap_path = channels_dir.join("custom-channel.capabilities.json");
        let caps = serde_json::json!({
            "type": "channel",
            "name": "custom-channel",
            "wit_version": "0.1.0",
        });
        std::fs::write(&cap_path, serde_json::to_string(&caps).unwrap()).unwrap();

        let manager = make_manager_custom_dirs(dir.path().join("tools"), channels_dir);

        let result = manager.upgrade(Some("custom-channel")).await.unwrap();
        assert_eq!(result.results.len(), 1);
        assert_eq!(result.results[0].status, "not_in_registry");
    }

    fn make_manager_with_temp_dirs() -> ExtensionManager {
        let dir = tempfile::tempdir().expect("temp dir");
        make_manager_custom_dirs(dir.path().join("tools"), dir.path().join("channels"))
    }

    fn make_manager_custom_dirs(
        tools_dir: std::path::PathBuf,
        channels_dir: std::path::PathBuf,
    ) -> ExtensionManager {
        use crate::secrets::{InMemorySecretsStore, SecretsCrypto};
        use crate::testing::credentials::TEST_CRYPTO_KEY;
        use crate::tools::ToolRegistry;
        use crate::tools::mcp::process::McpProcessManager;
        use crate::tools::mcp::session::McpSessionManager;

        std::fs::create_dir_all(&tools_dir).ok();
        std::fs::create_dir_all(&channels_dir).ok();

        let master_key = secrecy::SecretString::from(TEST_CRYPTO_KEY.to_string());
        let crypto = Arc::new(SecretsCrypto::new(master_key).unwrap());

        ExtensionManager::new(
            Arc::new(McpSessionManager::new()),
            Arc::new(McpProcessManager::new()),
            Arc::new(InMemorySecretsStore::new(crypto)),
            Arc::new(ToolRegistry::new()),
            None,
            None,
            tools_dir,
            channels_dir,
            None,
            "test".to_string(),
            None,
            Vec::new(),
        )
    }

    #[tokio::test]
    async fn test_current_channel_owner_id_uses_runtime_state() -> Result<(), String> {
        let manager = make_manager_with_temp_dirs();
        if manager.current_channel_owner_id("telegram").await.is_some() {
            return Err("expected no owner id for telegram before runtime setup".to_string());
        }

        let channels = Arc::new(crate::channels::ChannelManager::new());
        let runtime = Arc::new(
            crate::channels::wasm::WasmChannelRuntime::new(
                crate::channels::wasm::WasmChannelRuntimeConfig::default(),
            )
            .map_err(|e| format!("runtime init failed: {e}"))?,
        );
        let pairing_store = Arc::new(crate::pairing::PairingStore::new());
        let router = Arc::new(crate::channels::wasm::WasmChannelRouter::new());
        let mut owner_ids = std::collections::HashMap::new();
        owner_ids.insert("telegram".to_string(), 12345_i64);

        manager
            .set_channel_runtime(channels, runtime, pairing_store, router, owner_ids)
            .await;

        if manager.current_channel_owner_id("telegram").await != Some(12345_i64) {
            return Err("expected runtime owner id fast-path for telegram".to_string());
        }
        if manager.current_channel_owner_id("slack").await.is_some() {
            return Err("expected no owner id for slack".to_string());
        }

        Ok(())
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_current_channel_owner_id_uses_store_fallback() -> Result<(), String> {
        use crate::db::{Database, SettingsStore};

        let dir = tempfile::tempdir().map_err(|e| format!("tempdir failed: {e}"))?;
        let db_path = dir.path().join("owner-id.db");

        let db = Arc::new(
            crate::db::libsql::LibSqlBackend::new_local(&db_path)
                .await
                .map_err(|e| format!("create local libsql backend failed: {e}"))?,
        );
        db.run_migrations()
            .await
            .map_err(|e| format!("run libsql migrations failed: {e}"))?;

        let tools_dir = dir.path().join("tools");
        let channels_dir = dir.path().join("channels");
        std::fs::create_dir_all(&tools_dir).ok();
        std::fs::create_dir_all(&channels_dir).ok();

        use crate::secrets::{InMemorySecretsStore, SecretsCrypto};
        use crate::testing::credentials::TEST_CRYPTO_KEY;
        use crate::tools::ToolRegistry;
        use crate::tools::mcp::process::McpProcessManager;
        use crate::tools::mcp::session::McpSessionManager;

        let master_key = secrecy::SecretString::from(TEST_CRYPTO_KEY.to_string());
        let crypto = Arc::new(
            SecretsCrypto::new(master_key)
                .map_err(|e| format!("create secrets crypto failed: {e}"))?,
        );

        let manager = ExtensionManager::new(
            Arc::new(McpSessionManager::new()),
            Arc::new(McpProcessManager::new()),
            Arc::new(InMemorySecretsStore::new(crypto)),
            Arc::new(ToolRegistry::new()),
            None,
            None,
            tools_dir,
            channels_dir,
            None,
            "test".to_string(),
            Some(db.clone() as Arc<dyn crate::db::Database>),
            Vec::new(),
        );

        if manager.current_channel_owner_id("telegram").await.is_some() {
            return Err("expected no owner id before settings seed".to_string());
        }

        db.set_setting(
            "test",
            "channels.wasm_channel_owner_ids.telegram",
            &serde_json::json!(54321_i64),
        )
        .await
        .map_err(|e| format!("persist owner id in settings failed: {e}"))?;

        if manager.current_channel_owner_id("telegram").await != Some(54321_i64) {
            return Err("expected store fallback owner id for telegram".to_string());
        }

        let channels = Arc::new(crate::channels::ChannelManager::new());
        let runtime = Arc::new(
            crate::channels::wasm::WasmChannelRuntime::new(
                crate::channels::wasm::WasmChannelRuntimeConfig::default(),
            )
            .map_err(|e| format!("runtime init failed: {e}"))?,
        );
        let pairing_store = Arc::new(crate::pairing::PairingStore::new());
        let router = Arc::new(crate::channels::wasm::WasmChannelRouter::new());
        let mut owner_ids = std::collections::HashMap::new();
        owner_ids.insert("telegram".to_string(), 12345_i64);
        manager
            .set_channel_runtime(channels, runtime, pairing_store, router, owner_ids)
            .await;

        if manager.current_channel_owner_id("telegram").await != Some(12345_i64) {
            return Err("expected runtime fast-path owner id precedence".to_string());
        }

        Ok(())
    }

    // ── resolve_env_credentials tests ────────────────────────────────────

    #[test]
    fn test_security_prefix_check() {
        // Placeholders that don't start with the channel prefix must be rejected.
        // All env var names are prefixed with ICTEST1_ to avoid CI collisions.
        let placeholders = vec![
            "ICTEST1_BOT_TOKEN".to_string(), // valid: matches channel prefix
            "ICTEST2_TOKEN".to_string(),     // invalid: wrong channel prefix
            "ICTEST1_UNRELATED_OTHER".to_string(), // valid prefix, but env var not set — not injected
        ];
        let already_injected = std::collections::HashSet::new();

        unsafe { std::env::set_var("ICTEST1_BOT_TOKEN", "good-secret") };
        unsafe { std::env::set_var("ICTEST2_TOKEN", "bad-secret") };
        // ICTEST1_UNRELATED_OTHER intentionally not set — tests both prefix rejection and absence

        let resolved = super::resolve_env_credentials(&placeholders, "ictest1", &already_injected);

        // Only ICTEST1_BOT_TOKEN passes the prefix check for channel "ictest1"
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].0, "ICTEST1_BOT_TOKEN");
        assert_eq!(resolved[0].1, "good-secret");

        unsafe { std::env::remove_var("ICTEST1_BOT_TOKEN") };
        unsafe { std::env::remove_var("ICTEST2_TOKEN") };
    }

    #[test]
    fn test_already_injected_skipped() {
        // Use unique env var names (ictest3_*) to avoid interference with other tests.
        let placeholders = vec!["ICTEST3_TOKEN".to_string()];
        let mut already_injected = std::collections::HashSet::new();
        already_injected.insert("ICTEST3_TOKEN".to_string());

        unsafe { std::env::set_var("ICTEST3_TOKEN", "secret") };

        let resolved = super::resolve_env_credentials(&placeholders, "ictest3", &already_injected);

        // Already covered by secrets store — env var must be skipped
        assert!(resolved.is_empty());

        unsafe { std::env::remove_var("ICTEST3_TOKEN") };
    }

    #[test]
    fn test_missing_env_var_not_injected() {
        // Use unique env var names (ictest4_*) to avoid interference with other tests.
        let placeholders = vec!["ICTEST4_TOKEN".to_string()];
        let already_injected = std::collections::HashSet::new();

        unsafe { std::env::remove_var("ICTEST4_TOKEN") };

        let resolved = super::resolve_env_credentials(&placeholders, "ictest4", &already_injected);

        assert!(resolved.is_empty());
    }

    #[test]
    fn test_empty_env_var_not_injected() {
        // An env var that exists but is empty must not be injected.
        // Use unique env var names (ictest5_*) to avoid interference with other tests.
        let placeholders = vec!["ICTEST5_TOKEN".to_string()];
        let already_injected = std::collections::HashSet::new();

        unsafe { std::env::set_var("ICTEST5_TOKEN", "") };

        let resolved = super::resolve_env_credentials(&placeholders, "ictest5", &already_injected);

        assert!(resolved.is_empty());

        unsafe { std::env::remove_var("ICTEST5_TOKEN") };
    }

    #[test]
    fn test_empty_channel_name_returns_nothing() {
        // An empty channel name must never match any env var (prefix would be "_").
        let placeholders = vec!["_TOKEN".to_string(), "ICTEST6_TOKEN".to_string()];
        let already_injected = std::collections::HashSet::new();

        unsafe { std::env::set_var("_TOKEN", "bad") };
        unsafe { std::env::set_var("ICTEST6_TOKEN", "bad") };

        let resolved = super::resolve_env_credentials(&placeholders, "", &already_injected);

        assert!(resolved.is_empty(), "empty channel name must match nothing");

        unsafe { std::env::remove_var("_TOKEN") };
        unsafe { std::env::remove_var("ICTEST6_TOKEN") };
    }

    #[tokio::test]
    async fn test_determine_installed_kind_does_not_auto_install_relay() {
        // Regression: determine_installed_kind used to auto-insert into
        // installed_relay_extensions when a ChannelRelay registry entry existed,
        // even though the user never installed it. It should be read-only.
        let dir = tempfile::tempdir().expect("temp dir");
        let mgr = make_test_manager(None, dir.path().to_path_buf());

        // The manager has no relay extensions installed
        assert!(
            mgr.installed_relay_extensions.read().await.is_empty(),
            "Should start with no installed relay extensions"
        );

        // Calling determine_installed_kind for a non-installed name returns NotInstalled
        let result = mgr.determine_installed_kind("slack-relay").await;
        assert!(result.is_err(), "Should return NotInstalled");

        // Crucially: installed_relay_extensions must still be empty
        assert!(
            mgr.installed_relay_extensions.read().await.is_empty(),
            "determine_installed_kind must not modify installed_relay_extensions"
        );
    }

    #[tokio::test]
    async fn test_is_relay_channel_detects_stored_token() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mgr = make_test_manager(None, dir.path().to_path_buf());

        // No token stored → not a relay channel
        assert!(!mgr.is_relay_channel("slack-relay").await);

        // Store a stream token
        mgr.secrets
            .create(
                "test",
                crate::secrets::CreateSecretParams::new("relay:slack-relay:stream_token", "tok123"),
            )
            .await
            .expect("store token");

        // Now it's detected as a relay channel
        assert!(mgr.is_relay_channel("slack-relay").await);
    }

    #[tokio::test]
    async fn test_remove_relay_shuts_down_via_relay_channel_manager() {
        // Regression: remove() only checked channel_runtime for shutdown, missing
        // relay-only mode where only relay_channel_manager is set.
        let dir = tempfile::tempdir().expect("temp dir");
        let mgr = make_test_manager(None, dir.path().to_path_buf());

        // Set up relay channel manager with a stub channel
        let cm = Arc::new(crate::channels::ChannelManager::new());
        let (stub, _tx) = crate::testing::StubChannel::new("slack-relay");
        cm.add(Box::new(stub)).await;
        mgr.set_relay_channel_manager(Arc::clone(&cm)).await;

        // Mark as installed + store a token so determine_installed_kind finds it
        mgr.installed_relay_extensions
            .write()
            .await
            .insert("slack-relay".to_string());
        mgr.secrets
            .create(
                "test",
                crate::secrets::CreateSecretParams::new("relay:slack-relay:stream_token", "tok123"),
            )
            .await
            .expect("store token");

        // Verify channel exists before removal
        assert!(cm.get_channel("slack-relay").await.is_some());

        // Remove should succeed and shut down the channel
        let result = mgr.remove("slack-relay").await;
        assert!(result.is_ok(), "remove should succeed: {:?}", result.err());

        // installed_relay_extensions should be cleared
        assert!(
            !mgr.installed_relay_extensions
                .read()
                .await
                .contains("slack-relay"),
            "Should be removed from installed set"
        );
    }

    #[tokio::test]
    async fn test_remove_wasm_tool_clears_pending_oauth_state_and_activation_error() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mgr = make_test_manager(None, dir.path().to_path_buf());

        std::fs::write(dir.path().join("gmail.wasm"), b"fake-tool").expect("write tool");

        let listener = tokio::spawn(async {
            std::future::pending::<()>().await;
        });
        let abort_handle = listener.abort_handle();
        mgr.pending_auth.write().await.insert(
            "gmail".to_string(),
            super::PendingAuth {
                _name: "gmail".to_string(),
                _kind: ExtensionKind::WasmTool,
                created_at: std::time::Instant::now(),
                task_handle: Some(listener),
            },
        );

        mgr.activation_errors
            .write()
            .await
            .insert("gmail".to_string(), "cached failure".to_string());

        let secrets = Arc::clone(&mgr.secrets);
        mgr.pending_oauth_flows().write().await.insert(
            "gmail-state".to_string(),
            crate::cli::oauth_defaults::PendingOAuthFlow {
                extension_name: "gmail".to_string(),
                display_name: "Gmail".to_string(),
                token_url: "https://example.com/token".to_string(),
                client_id: "client123".to_string(),
                client_secret: None,
                redirect_uri: "https://example.com/oauth/callback".to_string(),
                code_verifier: None,
                access_token_field: "access_token".to_string(),
                secret_name: "google_oauth_token".to_string(),
                provider: None,
                validation_endpoint: None,
                scopes: vec![],
                user_id: "test".to_string(),
                secrets: Arc::clone(&secrets),
                sse_sender: None,
                gateway_token: None,
                resource: None,
                client_id_secret_name: None,
                created_at: std::time::Instant::now(),
            },
        );
        mgr.pending_oauth_flows().write().await.insert(
            "other-state".to_string(),
            crate::cli::oauth_defaults::PendingOAuthFlow {
                extension_name: "web-search".to_string(),
                display_name: "Web Search".to_string(),
                token_url: "https://example.com/token".to_string(),
                client_id: "client456".to_string(),
                client_secret: None,
                redirect_uri: "https://example.com/oauth/callback".to_string(),
                code_verifier: None,
                access_token_field: "access_token".to_string(),
                secret_name: "other_token".to_string(),
                provider: None,
                validation_endpoint: None,
                scopes: vec![],
                user_id: "test".to_string(),
                secrets,
                sse_sender: None,
                gateway_token: None,
                resource: None,
                client_id_secret_name: None,
                created_at: std::time::Instant::now(),
            },
        );

        let result = mgr.remove("gmail").await;
        assert!(result.is_ok(), "remove should succeed: {:?}", result.err());

        tokio::task::yield_now().await;

        assert!(
            mgr.pending_auth.read().await.get("gmail").is_none(),
            "pending auth entry should be removed"
        );
        assert!(
            abort_handle.is_finished(),
            "pending auth listener should be aborted"
        );
        assert!(
            !mgr.activation_errors.read().await.contains_key("gmail"),
            "stale activation error should be cleared"
        );

        let flows = mgr.pending_oauth_flows().read().await;
        assert!(
            !flows.contains_key("gmail-state"),
            "gateway OAuth flow for removed extension should be cleared"
        );
        assert!(
            flows.contains_key("other-state"),
            "unrelated pending OAuth flows should be retained"
        );
    }

    #[tokio::test]
    async fn test_remove_wasm_channel_clears_activation_error_and_deletes_files() {
        let dir = tempfile::tempdir().expect("temp dir");
        let tools_dir = dir.path().join("tools");
        let channels_dir = dir.path().join("channels");
        let mgr = make_test_manager_with_dirs(None, tools_dir, channels_dir.clone());

        let wasm_path = channels_dir.join("telegram.wasm");
        let cap_path = channels_dir.join("telegram.capabilities.json");
        std::fs::write(&wasm_path, b"fake-channel").expect("write channel");
        std::fs::write(&cap_path, b"{}").expect("write capabilities");

        mgr.activation_errors
            .write()
            .await
            .insert("telegram".to_string(), "channel failed".to_string());

        let result = mgr.remove("telegram").await;
        assert!(result.is_ok(), "remove should succeed: {:?}", result.err());

        assert!(
            !mgr.activation_errors.read().await.contains_key("telegram"),
            "channel activation error should be cleared on remove"
        );
        assert!(
            !wasm_path.exists(),
            "channel wasm file should be deleted on remove"
        );
        assert!(
            !cap_path.exists(),
            "channel capabilities file should be deleted on remove"
        );
    }

    #[test]
    fn test_sanitize_url_with_query_params() {
        let url = "https://api.example.com/path?api_key=secret123&token=abc";
        let result = super::sanitize_url_for_logging(url);
        assert_eq!(result, "https://api.example.com/path");
        assert!(!result.contains("api_key"));
        assert!(!result.contains("secret123"));
        assert!(!result.contains("token"));
    }

    #[test]
    fn test_sanitize_url_with_credentials() {
        let url = "https://user:password@api.example.com:8080/path";
        let result = super::sanitize_url_for_logging(url);
        assert!(!result.contains("user"));
        assert!(!result.contains("password"));
        assert!(!result.contains("@"));
        assert!(result.contains("api.example.com"));
        assert!(result.contains(":8080"));
    }

    #[test]
    fn test_sanitize_url_with_fragment() {
        let url = "https://api.example.com/path#section";
        let result = super::sanitize_url_for_logging(url);
        assert_eq!(result, "https://api.example.com/path");
        assert!(!result.contains("#"));
        assert!(!result.contains("section"));
    }

    #[test]
    fn test_sanitize_url_with_port() {
        let url = "https://api.example.com:9443/path?key=value";
        let result = super::sanitize_url_for_logging(url);
        assert_eq!(result, "https://api.example.com:9443/path");
        assert!(result.contains(":9443"));
        assert!(!result.contains("key"));
    }

    #[test]
    fn test_sanitize_url_with_all_components() {
        let url = "https://admin:secret@api.example.com:8080/v1/data?api_key=xyz#results";
        let result = super::sanitize_url_for_logging(url);
        assert!(!result.contains("admin"));
        assert!(!result.contains("secret"));
        assert!(!result.contains("@"));
        assert!(!result.contains("api_key"));
        assert!(!result.contains("xyz"));
        assert!(!result.contains("#"));
        assert!(!result.contains("results"));
        assert!(result.contains("api.example.com:8080"));
        assert!(result.contains("/v1/data"));
    }

    #[test]
    fn test_sanitize_url_malformed() {
        // Malformed URL should fallback to string splitting
        let url = "https://[invalid-url";
        let result = super::sanitize_url_for_logging(url);
        // Malformed URL without query should return as-is via fallback
        assert_eq!(result, url);

        // Should still strip query params via fallback
        let url_with_query = "https://[invalid-url?key=secret";
        let result_with_query = super::sanitize_url_for_logging(url_with_query);
        assert_eq!(result_with_query, "https://[invalid-url");
        assert!(!result_with_query.contains("?"));
        assert!(!result_with_query.contains("secret"));
    }

    #[test]
    fn test_sanitize_url_short_string() {
        let url = "short";
        let result = super::sanitize_url_for_logging(url);
        assert_eq!(result, "short");
    }

    #[test]
    fn test_sanitize_url_not_url_like() {
        let input = "this is not a url";
        let result = super::sanitize_url_for_logging(input);
        assert_eq!(result, input);
    }

    #[test]
    fn test_sanitize_url_preserves_path() {
        let url = "https://api.example.com/v1/users/123/profile";
        let result = super::sanitize_url_for_logging(url);
        assert_eq!(result, url);
        assert!(result.contains("/v1/users/123/profile"));
    }

    // ---- gateway mode detection tests ----
    // Regression tests for a bug where MCP OAuth called `open::that()` on the
    // server machine instead of returning an auth URL to the gateway frontend.
    // The root cause was that `should_use_gateway_mode()` only checked the
    // `IRONCLAW_OAUTH_CALLBACK_URL` env var, ignoring `self.tunnel_url`.

    /// Serializes env-mutating tests to prevent parallel races.
    static GATEWAY_ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Build a minimal ExtensionManager with a custom tunnel_url.
    fn make_manager_with_tunnel(tunnel_url: Option<String>) -> ExtensionManager {
        use crate::secrets::{InMemorySecretsStore, SecretsCrypto};
        use crate::tools::mcp::process::McpProcessManager;
        use crate::tools::mcp::session::McpSessionManager;

        let key = secrecy::SecretString::from(crate::secrets::keychain::generate_master_key_hex());
        let crypto = Arc::new(SecretsCrypto::new(key).expect("crypto"));
        let secrets: Arc<dyn crate::secrets::SecretsStore + Send + Sync> =
            Arc::new(InMemorySecretsStore::new(crypto));
        let tools = Arc::new(crate::tools::ToolRegistry::new());
        let mcp = Arc::new(McpSessionManager::new());
        let dir = std::env::temp_dir().join("ironclaw-test-gateway-mode");

        ExtensionManager::new(
            mcp,
            Arc::new(McpProcessManager::new()),
            secrets,
            tools,
            None,
            None,
            dir.clone(),
            dir,
            tunnel_url,
            "test".to_string(),
            None,
            vec![],
        )
    }

    #[test]
    fn should_use_gateway_mode_true_for_tunnel_url() {
        let _guard = GATEWAY_ENV_MUTEX.lock().expect("env mutex poisoned");
        let original = std::env::var("IRONCLAW_OAUTH_CALLBACK_URL").ok();
        // SAFETY: Under GATEWAY_ENV_MUTEX, no concurrent env access.
        unsafe {
            std::env::remove_var("IRONCLAW_OAUTH_CALLBACK_URL");
        }

        let mgr = make_manager_with_tunnel(Some("https://my-gateway.example.com".into()));
        assert!(
            mgr.should_use_gateway_mode(),
            "should detect gateway mode from tunnel_url"
        );

        unsafe {
            if let Some(val) = original {
                std::env::set_var("IRONCLAW_OAUTH_CALLBACK_URL", val);
            }
        }
    }

    #[test]
    fn should_use_gateway_mode_false_without_tunnel() {
        let _guard = GATEWAY_ENV_MUTEX.lock().expect("env mutex poisoned");
        let original = std::env::var("IRONCLAW_OAUTH_CALLBACK_URL").ok();
        unsafe {
            std::env::remove_var("IRONCLAW_OAUTH_CALLBACK_URL");
        }

        let mgr = make_manager_with_tunnel(None);
        assert!(
            !mgr.should_use_gateway_mode(),
            "should not detect gateway mode without tunnel_url or env var"
        );

        unsafe {
            if let Some(val) = original {
                std::env::set_var("IRONCLAW_OAUTH_CALLBACK_URL", val);
            }
        }
    }

    #[test]
    fn should_use_gateway_mode_false_for_loopback_tunnel() {
        let _guard = GATEWAY_ENV_MUTEX.lock().expect("env mutex poisoned");
        let original = std::env::var("IRONCLAW_OAUTH_CALLBACK_URL").ok();
        unsafe {
            std::env::remove_var("IRONCLAW_OAUTH_CALLBACK_URL");
        }

        let mgr = make_manager_with_tunnel(Some("http://127.0.0.1:3001".into()));
        assert!(
            !mgr.should_use_gateway_mode(),
            "should not detect gateway mode for loopback tunnel_url"
        );

        unsafe {
            if let Some(val) = original {
                std::env::set_var("IRONCLAW_OAUTH_CALLBACK_URL", val);
            }
        }
    }

    /// Helper to run an async test body while holding the env mutex.
    /// Clears `IRONCLAW_OAUTH_CALLBACK_URL` for the duration, restoring on drop.
    struct EnvGuard {
        original: Option<String>,
        _mutex: std::sync::MutexGuard<'static, ()>,
    }

    impl EnvGuard {
        fn new() -> Self {
            let guard = GATEWAY_ENV_MUTEX.lock().expect("env mutex poisoned");
            let original = std::env::var("IRONCLAW_OAUTH_CALLBACK_URL").ok();
            // SAFETY: Under GATEWAY_ENV_MUTEX, no concurrent env access.
            unsafe {
                std::env::remove_var("IRONCLAW_OAUTH_CALLBACK_URL");
            }
            Self {
                original,
                _mutex: guard,
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // SAFETY: Under GATEWAY_ENV_MUTEX (still held by _mutex), no concurrent env access.
            unsafe {
                if let Some(ref val) = self.original {
                    std::env::set_var("IRONCLAW_OAUTH_CALLBACK_URL", val);
                } else {
                    std::env::remove_var("IRONCLAW_OAUTH_CALLBACK_URL");
                }
            }
        }
    }

    #[tokio::test]
    async fn gateway_callback_redirect_uri_from_tunnel_url() {
        let _env = EnvGuard::new();

        let mgr = make_manager_with_tunnel(Some("https://my-gateway.example.com".into()));
        assert_eq!(
            mgr.gateway_callback_redirect_uri().await,
            Some("https://my-gateway.example.com/oauth/callback".to_string()),
        );
    }

    #[tokio::test]
    async fn gateway_callback_redirect_uri_none_without_tunnel() {
        let _env = EnvGuard::new();

        let mgr = make_manager_with_tunnel(None);
        assert_eq!(mgr.gateway_callback_redirect_uri().await, None);
    }

    #[tokio::test]
    async fn gateway_callback_redirect_uri_trims_trailing_slash() {
        let _env = EnvGuard::new();

        let mgr = make_manager_with_tunnel(Some("https://my-gateway.example.com/".into()));
        assert_eq!(
            mgr.gateway_callback_redirect_uri().await,
            Some("https://my-gateway.example.com/oauth/callback".to_string()),
        );
    }

    #[tokio::test]
    async fn gateway_mode_enabled_explicitly() {
        let _env = EnvGuard::new();

        let mgr = make_manager_with_tunnel(None);
        assert!(!mgr.should_use_gateway_mode());

        mgr.enable_gateway_mode("https://my-gateway.example.com".into())
            .await;
        assert!(mgr.should_use_gateway_mode());
        assert_eq!(
            mgr.gateway_callback_redirect_uri().await,
            Some("https://my-gateway.example.com/oauth/callback".to_string()),
        );
    }
    // ── Regression tests for PR #677 (unify-extension-lifecycle) ─────────

    #[tokio::test]
    async fn test_configure_token_picks_first_missing_secret() {
        // Regression: configure_token() must pick the first *missing* secret,
        // not the first non-optional one. This allows multi-secret channels
        // to be configured one secret at a time.
        let dir = tempfile::tempdir().expect("temp dir");
        let channels_dir = dir.path().join("channels");
        std::fs::create_dir_all(&channels_dir).unwrap();

        // Write a fake channel WASM + capabilities with two required secrets
        std::fs::write(channels_dir.join("multi.wasm"), b"\0asm fake").unwrap();
        let caps = serde_json::json!({
            "type": "channel",
            "name": "multi",
            "setup": {
                "required_secrets": [
                    {"name": "SECRET_A", "prompt": "Enter secret A (at least 30 chars for validation)"},
                    {"name": "SECRET_B", "prompt": "Enter secret B (at least 30 chars for validation)"}
                ]
            }
        });
        std::fs::write(
            channels_dir.join("multi.capabilities.json"),
            serde_json::to_string(&caps).unwrap(),
        )
        .unwrap();

        let mgr = make_manager_custom_dirs(dir.path().join("tools"), channels_dir);

        // Pre-store SECRET_A so it's no longer missing
        mgr.secrets
            .create(
                "test",
                crate::secrets::CreateSecretParams::new("SECRET_A", "value-a"),
            )
            .await
            .expect("store SECRET_A");

        // configure_token should target SECRET_B (the first missing one)
        let _result = mgr.configure_token("multi", "value-b").await;
        // configure will fail at activation (no real WASM runtime), but the
        // secret should still have been stored before activation was attempted.
        // Check that SECRET_B was stored.
        assert!(
            mgr.secrets
                .exists("test", "SECRET_B")
                .await
                .unwrap_or(false),
            "configure_token should have stored SECRET_B (the first missing secret)"
        );
    }

    #[tokio::test]
    async fn test_auth_is_read_only_for_wasm_channel() {
        // Regression: auth() must be a pure status check — it must not store
        // any secrets or modify state. The old API accepted a token parameter.
        let dir = tempfile::tempdir().expect("temp dir");
        let channels_dir = dir.path().join("channels");
        std::fs::create_dir_all(&channels_dir).unwrap();

        std::fs::write(channels_dir.join("test-ch.wasm"), b"\0asm fake").unwrap();
        let caps = serde_json::json!({
            "type": "channel",
            "name": "test-ch",
            "setup": {
                "required_secrets": [
                    {"name": "BOT_TOKEN", "prompt": "Enter bot token (at least 30 chars for prompt validation)"}
                ]
            }
        });
        std::fs::write(
            channels_dir.join("test-ch.capabilities.json"),
            serde_json::to_string(&caps).unwrap(),
        )
        .unwrap();

        let mgr = make_manager_custom_dirs(dir.path().join("tools"), channels_dir);

        // auth() should return a result without storing anything
        let result = mgr.auth("test-ch").await;
        assert!(result.is_ok(), "auth should succeed: {:?}", result.err());

        // No secrets should have been created
        assert!(
            !mgr.secrets
                .exists("test", "BOT_TOKEN")
                .await
                .unwrap_or(true),
            "auth() must not create any secrets — it should be read-only"
        );
    }

    #[tokio::test]
    async fn test_configure_dispatches_activation_by_kind() {
        // Regression: configure() must dispatch to the correct activation method
        // by kind. Previously it unconditionally called activate_wasm_channel()
        // for all non-WasmTool types, which would fail with a channel-specific
        // error for MCP servers and channel relays.
        let dir = tempfile::tempdir().expect("temp dir");
        let channels_dir = dir.path().join("channels");
        std::fs::create_dir_all(&channels_dir).unwrap();

        let mgr = make_manager_custom_dirs(dir.path().join("tools"), channels_dir);

        // Register a channel relay extension (in-memory)
        mgr.installed_relay_extensions
            .write()
            .await
            .insert("test-relay".to_string());

        // configure() should dispatch to activate_channel_relay(), not
        // activate_wasm_channel(). Both will fail (no runtime configured),
        // but the error should be about relay config, not WASM channels.
        let mut secrets = std::collections::HashMap::new();
        secrets.insert(
            "relay:test-relay:stream_token".to_string(),
            "tok".to_string(),
        );

        let result = mgr.configure("test-relay", &secrets).await;
        assert!(
            result.is_ok(),
            "configure should return Ok: {:?}",
            result.err()
        );

        let result = result.unwrap();
        // Activation will fail (no relay config), but secrets should still be stored
        assert!(
            !result.activated,
            "activation should fail without relay config"
        );
        assert!(
            !result.message.contains("WASM"),
            "error should not mention WASM — got: {}",
            result.message
        );

        // Verify the secret was stored
        assert!(
            mgr.secrets
                .exists("test", "relay:test-relay:stream_token")
                .await
                .unwrap_or(false),
            "configure should have stored the relay stream token"
        );
    }
    #[test]
    fn test_validation_failed_is_distinct_error_variant() {
        // Regression: ValidationFailed must be a distinct error variant so
        // callers can match on it instead of parsing error message strings.
        let err = ExtensionError::ValidationFailed("Invalid token".to_string());

        assert!(
            matches!(err, ExtensionError::ValidationFailed(_)),
            "Should match ValidationFailed variant"
        );
        assert!(
            !matches!(err, ExtensionError::Other(_)),
            "Must NOT match Other variant"
        );
        assert!(
            !matches!(err, ExtensionError::AuthFailed(_)),
            "Must NOT match AuthFailed variant"
        );

        let msg = err.to_string();
        assert!(
            msg.contains("validation failed"),
            "Display should contain 'validation failed', got: {msg}"
        );
    }
}
