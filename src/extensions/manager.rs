//! Central extension manager that dispatches operations by ExtensionKind.
//!
//! Holds references to MCP infrastructure, WASM tool runtime, secrets store,
//! and tool registry. All extension operations (search, install, auth, activate,
//! list, remove) flow through here.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::extensions::discovery::OnlineDiscovery;
use crate::extensions::registry::ExtensionRegistry;
use crate::extensions::{
    ActivateResult, AuthResult, ExtensionError, ExtensionKind, ExtensionSource, InstallResult,
    InstalledExtension, RegistryEntry, ResultSource, SearchResult,
};
use crate::hooks::HookRegistry;
use crate::secrets::{CreateSecretParams, SecretsStore};
use crate::tools::ToolRegistry;
use crate::tools::mcp::McpClient;
use crate::tools::mcp::auth::{
    PkceChallenge, authorize_mcp_server, build_authorization_url, discover_full_oauth_metadata,
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
}

/// Central manager for extension lifecycle operations.
pub struct ExtensionManager {
    registry: ExtensionRegistry,
    discovery: OnlineDiscovery,

    // MCP infrastructure
    mcp_session_manager: Arc<McpSessionManager>,
    /// Active MCP clients keyed by server name.
    mcp_clients: RwLock<HashMap<String, Arc<McpClient>>>,

    // WASM tool infrastructure
    wasm_tool_runtime: Option<Arc<WasmToolRuntime>>,
    wasm_tools_dir: PathBuf,
    wasm_channels_dir: PathBuf,

    // Shared
    secrets: Arc<dyn SecretsStore + Send + Sync>,
    tool_registry: Arc<ToolRegistry>,
    hooks: Option<Arc<HookRegistry>>,
    pending_auth: RwLock<HashMap<String, PendingAuth>>,
    /// Tunnel URL for remote OAuth callbacks (used in future iterations).
    _tunnel_url: Option<String>,
    user_id: String,
    /// Optional database store for DB-backed MCP config.
    store: Option<Arc<dyn crate::db::Database>>,
}

impl ExtensionManager {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        mcp_session_manager: Arc<McpSessionManager>,
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
            mcp_clients: RwLock::new(HashMap::new()),
            wasm_tool_runtime,
            wasm_tools_dir,
            wasm_channels_dir,
            secrets,
            tool_registry,
            hooks,
            pending_auth: RwLock::new(HashMap::new()),
            _tunnel_url: tunnel_url,
            user_id,
            store,
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
        tracing::info!(extension = %name, url = ?url, kind = ?kind_hint, "Installing extension");

        // If we have a registry entry, use it
        if let Some(entry) = self.registry.get(name).await {
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
            }
            .map_err(|e| {
                tracing::error!(extension = %name, url = %url, error = %e, "Extension install from URL failed");
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

    /// Authenticate an installed extension.
    pub async fn auth(
        &self,
        name: &str,
        token: Option<&str>,
    ) -> Result<AuthResult, ExtensionError> {
        // Clean up expired pending auths
        self.cleanup_expired_auths().await;

        // Determine what kind of extension this is
        let kind = self.determine_installed_kind(name).await?;

        match kind {
            ExtensionKind::McpServer => self.auth_mcp(name, token).await,
            ExtensionKind::WasmTool => self.auth_wasm_tool(name, token).await,
            ExtensionKind::WasmChannel => self.auth_wasm_tool(name, token).await,
        }
    }

    /// Activate an installed (and optionally authenticated) extension.
    pub async fn activate(&self, name: &str) -> Result<ActivateResult, ExtensionError> {
        let kind = self.determine_installed_kind(name).await?;

        match kind {
            ExtensionKind::McpServer => self.activate_mcp(name).await,
            ExtensionKind::WasmTool => self.activate_wasm_tool(name).await,
            ExtensionKind::WasmChannel => Err(ExtensionError::ChannelNeedsRestart),
        }
    }

    /// List all installed extensions with their status.
    pub async fn list(
        &self,
        kind_filter: Option<ExtensionKind>,
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

                        extensions.push(InstalledExtension {
                            name: server.name.clone(),
                            kind: ExtensionKind::McpServer,
                            description: server.description.clone(),
                            url: Some(server.url.clone()),
                            authenticated,
                            active,
                            tools,
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
                    for (name, _discovered) in tools {
                        let active = self.tool_registry.has(&name).await;

                        extensions.push(InstalledExtension {
                            name: name.clone(),
                            kind: ExtensionKind::WasmTool,
                            description: None,
                            url: None,
                            authenticated: true, // WASM tools don't always need auth
                            active,
                            tools: if active { vec![name] } else { Vec::new() },
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
                    for (name, _discovered) in channels {
                        extensions.push(InstalledExtension {
                            name,
                            kind: ExtensionKind::WasmChannel,
                            description: None,
                            url: None,
                            authenticated: true,
                            active: true, // If loaded at startup, they're active
                            tools: Vec::new(),
                        });
                    }
                }
                Err(e) => {
                    tracing::debug!("Failed to discover WASM channels for listing: {}", e);
                }
            }
        }

        Ok(extensions)
    }

    /// Remove an installed extension.
    pub async fn remove(&self, name: &str) -> Result<String, ExtensionError> {
        let kind = self.determine_installed_kind(name).await?;

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
                let cap_path = self
                    .wasm_tools_dir
                    .join(format!("{}.capabilities.json", name));

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
            ExtensionKind::WasmChannel => Err(ExtensionError::Other(
                "Channel removal requires restart. Delete the .wasm file from ~/.ironclaw/channels/ and restart."
                    .to_string(),
            )),
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
        match entry.kind {
            ExtensionKind::McpServer => {
                let url = match &entry.source {
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
            ExtensionKind::WasmTool => match &entry.source {
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
                ExtensionSource::WasmBuildable { .. } => {
                    Err(ExtensionError::InstallFailed(format!(
                        "'{}' requires building from source. Run `ironclaw registry install {}` \
                         from the CLI (requires cargo-component).",
                        entry.name, entry.name
                    )))
                }
                _ => Err(ExtensionError::InstallFailed(
                    "WASM tool entry has no download URL".to_string(),
                )),
            },
            ExtensionKind::WasmChannel => match &entry.source {
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
                ExtensionSource::WasmBuildable { .. } => {
                    Err(ExtensionError::InstallFailed(format!(
                        "'{}' requires building from source. Run `ironclaw registry install {}` \
                         from the CLI (requires cargo-component).",
                        entry.name, entry.name
                    )))
                }
                _ => Err(ExtensionError::InstallFailed(
                    "WASM channel entry has no download URL".to_string(),
                )),
            },
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
                "WASM channel '{}' installed to {}. Restart to activate.",
                name,
                self.wasm_channels_dir.display()
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

        tracing::debug!(extension = %name, url = %url, "Downloading WASM extension");

        let response = client.get(url).send().await.map_err(|e| {
            tracing::error!(extension = %name, url = %url, error = %e, "Download request failed");
            ExtensionError::DownloadFailed(e.to_string())
        })?;

        if !response.status().is_success() {
            let status = response.status();
            tracing::error!(
                extension = %name,
                url = %url,
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
                match client.get(caps_url).send().await {
                    Ok(resp) if resp.status().is_success() => {
                        if let Ok(caps_bytes) = resp.bytes().await {
                            let _ = tokio::fs::write(&caps_path, &caps_bytes).await;
                        }
                    }
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

        let decoder = GzDecoder::new(bytes);
        let mut archive = Archive::new(decoder);

        let wasm_filename = format!("{}.wasm", name);
        let caps_filename = format!("{}.capabilities.json", name);
        let mut found_wasm = false;

        let entries = archive
            .entries()
            .map_err(|e| ExtensionError::InstallFailed(format!("Bad tar.gz archive: {}", e)))?;

        for entry in entries {
            let mut entry = entry
                .map_err(|e| ExtensionError::InstallFailed(format!("Bad tar.gz entry: {}", e)))?;

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
                let mut data = Vec::new();
                std::io::Read::read_to_end(&mut entry, &mut data)
                    .map_err(|e| ExtensionError::InstallFailed(e.to_string()))?;
                std::fs::write(target_wasm, &data)
                    .map_err(|e| ExtensionError::InstallFailed(e.to_string()))?;
                found_wasm = true;
            } else if filename == caps_filename {
                let mut data = Vec::new();
                std::io::Read::read_to_end(&mut entry, &mut data)
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

    async fn auth_mcp(
        &self,
        name: &str,
        token: Option<&str>,
    ) -> Result<AuthResult, ExtensionError> {
        let server = self
            .get_mcp_server(name)
            .await
            .map_err(|e| ExtensionError::NotInstalled(e.to_string()))?;

        // If a token was provided directly, store it and we're done.
        if let Some(token_value) = token {
            let secret_name = server.token_secret_name();
            let params =
                CreateSecretParams::new(&secret_name, token_value).with_provider(name.to_string());
            self.secrets
                .create(&self.user_id, params)
                .await
                .map_err(|e| ExtensionError::AuthFailed(e.to_string()))?;

            tracing::info!("MCP server '{}' authenticated via manual token", name);
            return Ok(AuthResult {
                name: name.to_string(),
                kind: ExtensionKind::McpServer,
                auth_url: None,
                callback_type: None,
                instructions: None,
                setup_url: None,
                awaiting_token: false,
                status: "authenticated".to_string(),
            });
        }

        // Check if already authenticated
        if is_authenticated(&server, &self.secrets, &self.user_id).await {
            return Ok(AuthResult {
                name: name.to_string(),
                kind: ExtensionKind::McpServer,
                auth_url: None,
                callback_type: None,
                instructions: None,
                setup_url: None,
                awaiting_token: false,
                status: "authenticated".to_string(),
            });
        }

        // Run the full OAuth flow (opens browser, waits for callback)
        match authorize_mcp_server(&server, &self.secrets, &self.user_id).await {
            Ok(_token) => {
                tracing::info!("MCP server '{}' authenticated via OAuth", name);
                Ok(AuthResult {
                    name: name.to_string(),
                    kind: ExtensionKind::McpServer,
                    auth_url: None,
                    callback_type: None,
                    instructions: None,
                    setup_url: None,
                    awaiting_token: false,
                    status: "authenticated".to_string(),
                })
            }
            Err(crate::tools::mcp::auth::AuthError::NotSupported) => {
                // Server doesn't support OAuth, try building a URL first
                match self.auth_mcp_build_url(name, &server).await {
                    Ok(result) => Ok(result),
                    Err(_) => {
                        // No OAuth, no DCR: fall back to manual token entry
                        Ok(AuthResult {
                            name: name.to_string(),
                            kind: ExtensionKind::McpServer,
                            auth_url: None,
                            callback_type: None,
                            instructions: Some(format!(
                                "Server '{}' does not support OAuth. \
                                 Please provide an API token/key for this server.",
                                name
                            )),
                            setup_url: None,
                            awaiting_token: true,
                            status: "awaiting_token".to_string(),
                        })
                    }
                }
            }
            Err(e) => {
                // OAuth failed for some other reason, fall back to manual token
                Ok(AuthResult {
                    name: name.to_string(),
                    kind: ExtensionKind::McpServer,
                    auth_url: None,
                    callback_type: None,
                    instructions: Some(format!(
                        "OAuth failed for '{}': {}. \
                         Please provide an API token/key manually.",
                        name, e
                    )),
                    setup_url: None,
                    awaiting_token: true,
                    status: "awaiting_token".to_string(),
                })
            }
        }
    }

    /// Build an auth URL for cases where non-interactive auth is needed
    /// (e.g., running via Telegram where we can't open a browser).
    async fn auth_mcp_build_url(
        &self,
        name: &str,
        server: &McpServerConfig,
    ) -> Result<AuthResult, ExtensionError> {
        // Try to discover OAuth metadata and build a URL the user can open manually
        let metadata = discover_full_oauth_metadata(&server.url)
            .await
            .map_err(|e| ExtensionError::AuthFailed(e.to_string()))?;

        // Try DCR if no client_id configured
        let (client_id, redirect_uri) = if let Some(ref oauth) = server.oauth {
            let port = find_available_port()
                .await
                .map_err(|e| ExtensionError::AuthFailed(e.to_string()))?;
            let redirect = format!("http://localhost:{}/callback", port.1);
            (oauth.client_id.clone(), redirect)
        } else if let Some(ref reg_endpoint) = metadata.registration_endpoint {
            let port = find_available_port()
                .await
                .map_err(|e| ExtensionError::AuthFailed(e.to_string()))?;
            let redirect = format!("http://localhost:{}/callback", port.1);

            let registration = register_client(reg_endpoint, &redirect)
                .await
                .map_err(|e| ExtensionError::AuthFailed(e.to_string()))?;

            (registration.client_id, redirect)
        } else {
            return Err(ExtensionError::AuthFailed(
                "Server doesn't support OAuth or Dynamic Client Registration".to_string(),
            ));
        };

        let pkce = PkceChallenge::generate();
        let auth_url = build_authorization_url(
            &metadata.authorization_endpoint,
            &client_id,
            &redirect_uri,
            &metadata.scopes_supported,
            Some(&pkce),
            &std::collections::HashMap::new(),
        );

        // Store pending auth for later callback handling
        self.pending_auth.write().await.insert(
            name.to_string(),
            PendingAuth {
                _name: name.to_string(),
                _kind: ExtensionKind::McpServer,
                created_at: std::time::Instant::now(),
            },
        );

        Ok(AuthResult {
            name: name.to_string(),
            kind: ExtensionKind::McpServer,
            auth_url: Some(auth_url),
            callback_type: Some("local".to_string()),
            instructions: None,
            setup_url: None,
            awaiting_token: false,
            status: "awaiting_authorization".to_string(),
        })
    }

    async fn auth_wasm_tool(
        &self,
        name: &str,
        token: Option<&str>,
    ) -> Result<AuthResult, ExtensionError> {
        // Read the capabilities file to get auth config
        let cap_path = self
            .wasm_tools_dir
            .join(format!("{}.capabilities.json", name));

        if !cap_path.exists() {
            // No capabilities = no auth needed
            return Ok(AuthResult {
                name: name.to_string(),
                kind: ExtensionKind::WasmTool,
                auth_url: None,
                callback_type: None,
                instructions: None,
                setup_url: None,
                awaiting_token: false,
                status: "no_auth_required".to_string(),
            });
        }

        let cap_bytes = tokio::fs::read(&cap_path)
            .await
            .map_err(|e| ExtensionError::Other(e.to_string()))?;

        let cap_file = crate::tools::wasm::CapabilitiesFile::from_bytes(&cap_bytes)
            .map_err(|e| ExtensionError::Other(e.to_string()))?;

        let auth = match cap_file.auth {
            Some(auth) => auth,
            None => {
                return Ok(AuthResult {
                    name: name.to_string(),
                    kind: ExtensionKind::WasmTool,
                    auth_url: None,
                    callback_type: None,
                    instructions: None,
                    setup_url: None,
                    awaiting_token: false,
                    status: "no_auth_required".to_string(),
                });
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

            return Ok(AuthResult {
                name: name.to_string(),
                kind: ExtensionKind::WasmTool,
                auth_url: None,
                callback_type: None,
                instructions: None,
                setup_url: None,
                awaiting_token: false,
                status: "authenticated".to_string(),
            });
        }

        // Check if already authenticated
        if self
            .secrets
            .exists(&self.user_id, &auth.secret_name)
            .await
            .unwrap_or(false)
        {
            return Ok(AuthResult {
                name: name.to_string(),
                kind: ExtensionKind::WasmTool,
                auth_url: None,
                callback_type: None,
                instructions: None,
                setup_url: None,
                awaiting_token: false,
                status: "authenticated".to_string(),
            });
        }

        // If a token was provided, store it
        if let Some(token_value) = token {
            let params = CreateSecretParams::new(&auth.secret_name, token_value)
                .with_provider(name.to_string());
            self.secrets
                .create(&self.user_id, params)
                .await
                .map_err(|e| ExtensionError::AuthFailed(e.to_string()))?;

            return Ok(AuthResult {
                name: name.to_string(),
                kind: ExtensionKind::WasmTool,
                auth_url: None,
                callback_type: None,
                instructions: None,
                setup_url: None,
                awaiting_token: false,
                status: "authenticated".to_string(),
            });
        }

        // Return instructions for manual token entry
        let display = auth.display_name.unwrap_or_else(|| name.to_string());
        let instructions = auth
            .instructions
            .unwrap_or_else(|| format!("Please provide your {} API token/key.", display));

        Ok(AuthResult {
            name: name.to_string(),
            kind: ExtensionKind::WasmTool,
            auth_url: None,
            callback_type: None,
            instructions: Some(instructions),
            setup_url: auth.setup_url,
            awaiting_token: true,
            status: "awaiting_token".to_string(),
        })
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

        let has_tokens = is_authenticated(&server, &self.secrets, &self.user_id).await;

        let client = if has_tokens || server.requires_auth() {
            McpClient::new_authenticated(
                server.clone(),
                Arc::clone(&self.mcp_session_manager),
                Arc::clone(&self.secrets),
                &self.user_id,
            )
        } else {
            McpClient::new_with_name(&server.name, &server.url)
        };

        // Try to list and create tools
        let mcp_tools = client
            .list_tools()
            .await
            .map_err(|e| ExtensionError::ActivationFailed(e.to_string()))?;

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

        let loader = WasmToolLoader::new(Arc::clone(runtime), Arc::clone(&self.tool_registry));
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

    /// Determine what kind of installed extension this is.
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

        Err(ExtensionError::NotInstalled(format!(
            "'{}' is not installed as an MCP server, WASM tool, or WASM channel",
            name
        )))
    }

    async fn cleanup_expired_auths(&self) {
        let mut pending = self.pending_auth.write().await;
        pending.retain(|_, auth| auth.created_at.elapsed() < std::time::Duration::from_secs(300));
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

/// Infer the extension kind from a URL.
fn infer_kind_from_url(url: &str) -> ExtensionKind {
    if url.ends_with(".wasm") || url.ends_with(".tar.gz") {
        ExtensionKind::WasmTool
    } else {
        ExtensionKind::McpServer
    }
}

#[cfg(test)]
mod tests {
    use crate::extensions::ExtensionKind;
    use crate::extensions::manager::infer_kind_from_url;

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
}
