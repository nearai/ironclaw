//! Generic WASM tool loader for loading tools from files or directories.
//!
//! This module provides a way to load WASM tools dynamically at runtime from:
//! - A directory containing `<name>.wasm` and `<name>.capabilities.json`
//! - Build artifacts in `tools-src/` (dev mode, auto-detected)
//! - Database storage (via [`WasmToolStore`])
//!
//! # Example: Loading from Directory
//!
//! ```text
//! ~/.ironclaw/tools/
//! ├── slack.wasm
//! ├── slack.capabilities.json
//! ├── github.wasm
//! └── github.capabilities.json
//! ```
//!
//! ```ignore
//! let loader = WasmToolLoader::new(runtime, registry);
//! loader.load_from_dir(Path::new("~/.ironclaw/tools/")).await?;
//! ```
//!
//! # Dev Mode
//!
//! When `load_dev_tools()` is called, the loader scans `tools-src/*/` for build
//! artifacts. Tools found there are loaded directly from the build output,
//! skipping the install directory. This means during development you just
//! rebuild the WASM and restart the host, no manual copy step needed.
//!
//! # Security
//!
//! Tools loaded from files are assigned `TrustLevel::User` by default, meaning
//! they run with the most restrictive permissions. Only tools explicitly marked
//! as `verified` or `system` in the database get elevated trust.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::fs;

use crate::secrets::SecretsStore;
use crate::tools::registry::{ToolRegistry, WasmRegistrationError, WasmToolRegistration};
use crate::tools::wasm::capabilities_schema::CapabilitiesFile;
use crate::tools::wasm::{
    Capabilities, OAuthRefreshConfig, WasmError, WasmStorageError, WasmToolRuntime, WasmToolStore,
};

/// Error during WASM tool loading.
#[derive(Debug, thiserror::Error)]
pub enum WasmLoadError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("WASM file not found: {0}")]
    WasmNotFound(PathBuf),

    #[error("Capabilities file not found: {0}")]
    CapabilitiesNotFound(PathBuf),

    #[error("Invalid capabilities JSON: {0}")]
    InvalidCapabilities(String),

    #[error("WASM compilation error: {0}")]
    Compilation(#[from] WasmError),

    #[error("Storage error: {0}")]
    Storage(#[from] WasmStorageError),

    #[error("Registration error: {0}")]
    Registration(#[from] WasmRegistrationError),

    #[error("Invalid tool name: {0}")]
    InvalidName(String),
}

/// Loads WASM tools from files or storage into the registry.
pub struct WasmToolLoader {
    runtime: Arc<WasmToolRuntime>,
    registry: Arc<ToolRegistry>,
    secrets_store: Option<Arc<dyn SecretsStore + Send + Sync>>,
}

impl WasmToolLoader {
    /// Create a new loader with the given runtime and registry.
    pub fn new(runtime: Arc<WasmToolRuntime>, registry: Arc<ToolRegistry>) -> Self {
        Self {
            runtime,
            registry,
            secrets_store: None,
        }
    }

    /// Set the secrets store for credential injection in WASM tools.
    pub fn with_secrets_store(mut self, store: Arc<dyn SecretsStore + Send + Sync>) -> Self {
        self.secrets_store = Some(store);
        self
    }

    /// Load a single WASM tool from a file pair.
    ///
    /// Expects:
    /// - `wasm_path`: Path to the `.wasm` file
    /// - `capabilities_path`: Path to the `.capabilities.json` file (optional)
    ///
    /// If no capabilities file is provided, the tool gets no capabilities (default deny).
    pub async fn load_from_files(
        &self,
        name: &str,
        wasm_path: &Path,
        capabilities_path: Option<&Path>,
    ) -> Result<(), WasmLoadError> {
        if name.is_empty() || name.contains('/') || name.contains('\\') {
            return Err(WasmLoadError::InvalidName(name.to_string()));
        }

        // Read WASM bytes
        if !wasm_path.exists() {
            return Err(WasmLoadError::WasmNotFound(wasm_path.to_path_buf()));
        }
        let wasm_bytes = fs::read(wasm_path).await?;

        // Read capabilities (optional) and extract OAuth refresh config
        let (capabilities, oauth_refresh) = if let Some(cap_path) = capabilities_path {
            if cap_path.exists() {
                let cap_bytes = fs::read(cap_path).await?;
                let cap_file = CapabilitiesFile::from_bytes(&cap_bytes)
                    .map_err(|e| WasmLoadError::InvalidCapabilities(e.to_string()))?;
                let caps = cap_file.to_capabilities();
                let oauth = resolve_oauth_refresh_config(&cap_file);
                (caps, oauth)
            } else {
                tracing::warn!(
                    path = %cap_path.display(),
                    "Capabilities file not found, using default (no permissions)"
                );
                (Capabilities::default(), None)
            }
        } else {
            (Capabilities::default(), None)
        };

        // Register the tool
        self.registry
            .register_wasm(WasmToolRegistration {
                name,
                wasm_bytes: &wasm_bytes,
                runtime: &self.runtime,
                capabilities,
                limits: None,
                description: None,
                schema: None,
                secrets_store: self.secrets_store.clone(),
                oauth_refresh,
            })
            .await?;

        tracing::info!(
            name = name,
            wasm_path = %wasm_path.display(),
            "Loaded WASM tool from file"
        );

        Ok(())
    }

    /// Load all WASM tools from a directory.
    ///
    /// Scans the directory for `*.wasm` files and loads each one, looking for
    /// a matching `*.capabilities.json` sidecar file.
    ///
    /// # Directory Layout
    ///
    /// ```text
    /// tools/
    /// ├── slack.wasm                  <- Tool WASM component
    /// ├── slack.capabilities.json     <- Capabilities (optional)
    /// ├── github.wasm
    /// └── github.capabilities.json
    /// ```
    ///
    /// Tools without a capabilities file get no permissions (default deny).
    pub async fn load_from_dir(&self, dir: &Path) -> Result<LoadResults, WasmLoadError> {
        if !dir.is_dir() {
            return Err(WasmLoadError::Io(std::io::Error::new(
                std::io::ErrorKind::NotADirectory,
                format!("{} is not a directory", dir.display()),
            )));
        }

        let mut results = LoadResults::default();

        // Collect all .wasm entries first, then load in parallel
        let mut tool_entries = Vec::new();
        let mut entries = fs::read_dir(dir).await?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();

            if path.extension().and_then(|e| e.to_str()) != Some("wasm") {
                continue;
            }

            let name = match path.file_stem().and_then(|s| s.to_str()) {
                Some(n) => n.to_string(),
                None => {
                    results.errors.push((
                        path.clone(),
                        WasmLoadError::InvalidName("invalid filename".to_string()),
                    ));
                    continue;
                }
            };

            let cap_path = path.with_extension("capabilities.json");
            let has_cap = cap_path.exists();
            tool_entries.push((name, path, if has_cap { Some(cap_path) } else { None }));
        }

        // Load all tools in parallel (file I/O + WASM compilation + registration)
        let load_futures = tool_entries
            .iter()
            .map(|(name, path, cap_path)| self.load_from_files(name, path, cap_path.as_deref()));

        let load_results = futures::future::join_all(load_futures).await;

        for ((name, path, _), result) in tool_entries.into_iter().zip(load_results) {
            match result {
                Ok(()) => {
                    results.loaded.push(name);
                }
                Err(e) => {
                    tracing::error!(
                        name = name,
                        path = %path.display(),
                        error = %e,
                        "Failed to load WASM tool"
                    );
                    results.errors.push((path, e));
                }
            }
        }

        if !results.loaded.is_empty() {
            tracing::info!(
                count = results.loaded.len(),
                tools = ?results.loaded,
                "Loaded WASM tools from directory"
            );
        }

        Ok(results)
    }

    /// Load a WASM tool from database storage.
    ///
    /// This is a convenience wrapper around [`ToolRegistry::register_wasm_from_storage`].
    pub async fn load_from_storage(
        &self,
        store: &dyn WasmToolStore,
        user_id: &str,
        tool_name: &str,
    ) -> Result<(), WasmLoadError> {
        self.registry
            .register_wasm_from_storage(store, &self.runtime, user_id, tool_name)
            .await?;

        tracing::info!(
            user_id = user_id,
            name = tool_name,
            "Loaded WASM tool from storage"
        );

        Ok(())
    }

    /// Load all active WASM tools for a user from storage.
    pub async fn load_all_from_storage(
        &self,
        store: &dyn WasmToolStore,
        user_id: &str,
    ) -> Result<LoadResults, WasmLoadError> {
        let tools = store.list(user_id).await?;
        let mut results = LoadResults::default();

        for tool in tools {
            // Skip non-active tools
            if tool.status != crate::tools::wasm::ToolStatus::Active {
                continue;
            }

            match self.load_from_storage(store, user_id, &tool.name).await {
                Ok(()) => {
                    results.loaded.push(tool.name);
                }
                Err(e) => {
                    tracing::error!(
                        name = tool.name,
                        user_id = user_id,
                        error = %e,
                        "Failed to load WASM tool from storage"
                    );
                    results.errors.push((PathBuf::from(&tool.name), e));
                }
            }
        }

        Ok(results)
    }
}

/// Extract OAuth refresh configuration from a parsed capabilities file.
///
/// Returns `None` if there's no `auth.oauth` section or if the client_id
/// can't be resolved from any source (inline, env var, or built-in defaults).
///
/// Fallback chain for client_id:
///   `oauth.client_id` > env var (`oauth.client_id_env`) > `builtin_credentials()`
fn resolve_oauth_refresh_config(cap_file: &CapabilitiesFile) -> Option<OAuthRefreshConfig> {
    let auth = cap_file.auth.as_ref()?;
    let oauth = auth.oauth.as_ref()?;

    let builtin = crate::cli::oauth_defaults::builtin_credentials(&auth.secret_name);

    let client_id = oauth
        .client_id
        .clone()
        .or_else(|| {
            oauth
                .client_id_env
                .as_ref()
                .and_then(|env| std::env::var(env).ok())
        })
        .or_else(|| builtin.as_ref().map(|c| c.client_id.to_string()))?;

    let client_secret = oauth
        .client_secret
        .clone()
        .or_else(|| {
            oauth
                .client_secret_env
                .as_ref()
                .and_then(|env| std::env::var(env).ok())
        })
        .or_else(|| builtin.as_ref().map(|c| c.client_secret.to_string()));

    Some(OAuthRefreshConfig {
        token_url: oauth.token_url.clone(),
        client_id,
        client_secret,
        secret_name: auth.secret_name.clone(),
        provider: auth.provider.clone(),
    })
}

/// Results from loading multiple tools.
#[derive(Debug, Default)]
pub struct LoadResults {
    /// Names of successfully loaded tools.
    pub loaded: Vec<String>,

    /// Errors encountered (path/name, error).
    pub errors: Vec<(PathBuf, WasmLoadError)>,
}

impl LoadResults {
    /// Check if all tools loaded successfully.
    pub fn all_succeeded(&self) -> bool {
        self.errors.is_empty()
    }

    /// Get the count of successfully loaded tools.
    pub fn success_count(&self) -> usize {
        self.loaded.len()
    }

    /// Get the count of failed tools.
    pub fn error_count(&self) -> usize {
        self.errors.len()
    }
}

/// Compile-time project root, used to locate tools-src/ in dev builds.
const CARGO_MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");

/// Resolve the tools source directory.
///
/// Checks (in order):
/// 1. `IRONCLAW_TOOLS_SRC` env var
/// 2. `<CARGO_MANIFEST_DIR>/tools-src/` (dev builds)
fn tools_src_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("IRONCLAW_TOOLS_SRC") {
        return PathBuf::from(dir);
    }
    PathBuf::from(CARGO_MANIFEST_DIR).join("tools-src")
}

/// Discover WASM tools available as build artifacts in `tools-src/`.
///
/// Scans each subdirectory for:
/// - `tools-src/<name>/target/wasm32-wasip2/release/<crate_name>_tool.wasm`
/// - `tools-src/<name>/<name>-tool.capabilities.json`
///
/// Returns a map of install-name (e.g. "gmail-tool") to paths.
pub async fn discover_dev_tools() -> Result<HashMap<String, DiscoveredTool>, std::io::Error> {
    let src_dir = tools_src_dir();
    let mut tools = HashMap::new();

    if !src_dir.is_dir() {
        return Ok(tools);
    }

    let mut entries = fs::read_dir(&src_dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let dir_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };

        // Convention: crate name uses underscores, directory uses hyphens
        let crate_name = dir_name.replace('-', "_");
        let install_name = format!("{}-tool", dir_name);

        let wasm_path = path
            .join("target/wasm32-wasip2/release")
            .join(format!("{}_tool.wasm", crate_name));

        if !wasm_path.exists() {
            continue;
        }

        let caps_path = path.join(format!("{}-tool.capabilities.json", dir_name));

        tools.insert(
            install_name,
            DiscoveredTool {
                wasm_path,
                capabilities_path: if caps_path.exists() {
                    Some(caps_path)
                } else {
                    None
                },
            },
        );
    }

    Ok(tools)
}

/// Load WASM tools from build artifacts in `tools-src/`.
///
/// In dev mode, tools can be loaded directly from their build output without
/// needing to install them to `~/.ironclaw/tools/` first. Build artifacts
/// that are newer than installed copies take priority.
///
/// Set `IRONCLAW_TOOLS_SRC` env var to override the source directory.
pub async fn load_dev_tools(
    loader: &WasmToolLoader,
    install_dir: &Path,
) -> Result<LoadResults, WasmLoadError> {
    let dev_tools = discover_dev_tools().await?;
    let mut results = LoadResults::default();

    if dev_tools.is_empty() {
        return Ok(results);
    }

    for (name, discovered) in &dev_tools {
        // Check if the build artifact is newer than the installed copy
        let installed_path = install_dir.join(format!("{}.wasm", name));
        let should_load = if installed_path.exists() {
            // Compare modification times: prefer fresher build artifact
            match (
                fs::metadata(&discovered.wasm_path).await,
                fs::metadata(&installed_path).await,
            ) {
                (Ok(dev_meta), Ok(inst_meta)) => {
                    let dev_modified = dev_meta.modified().unwrap_or(std::time::UNIX_EPOCH);
                    let inst_modified = inst_meta.modified().unwrap_or(std::time::UNIX_EPOCH);
                    dev_modified > inst_modified
                }
                _ => true,
            }
        } else {
            true
        };

        if !should_load {
            continue;
        }

        tracing::info!(
            name = name,
            wasm_path = %discovered.wasm_path.display(),
            "Loading dev tool from build artifacts (newer than installed)"
        );

        match loader
            .load_from_files(
                name,
                &discovered.wasm_path,
                discovered.capabilities_path.as_deref(),
            )
            .await
        {
            Ok(()) => {
                results.loaded.push(name.clone());
            }
            Err(e) => {
                tracing::error!(
                    name = name,
                    error = %e,
                    "Failed to load dev tool"
                );
                results.errors.push((discovered.wasm_path.clone(), e));
            }
        }
    }

    if !results.loaded.is_empty() {
        tracing::info!(
            count = results.loaded.len(),
            tools = ?results.loaded,
            "Loaded dev tools from build artifacts"
        );
    }

    Ok(results)
}

/// Discover WASM tool files in a directory without loading them.
///
/// Returns a map of tool name -> (wasm_path, capabilities_path).
pub async fn discover_tools(dir: &Path) -> Result<HashMap<String, DiscoveredTool>, std::io::Error> {
    let mut tools = HashMap::new();

    if !dir.is_dir() {
        return Ok(tools);
    }

    let mut entries = fs::read_dir(dir).await?;

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();

        if path.extension().and_then(|e| e.to_str()) != Some("wasm") {
            continue;
        }

        let name = match path.file_stem().and_then(|s| s.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };

        let cap_path = path.with_extension("capabilities.json");

        tools.insert(
            name,
            DiscoveredTool {
                wasm_path: path,
                capabilities_path: if cap_path.exists() {
                    Some(cap_path)
                } else {
                    None
                },
            },
        );
    }

    Ok(tools)
}

/// A discovered WASM tool (not yet loaded).
#[derive(Debug)]
pub struct DiscoveredTool {
    /// Path to the WASM file.
    pub wasm_path: PathBuf,

    /// Path to the capabilities file (if present).
    pub capabilities_path: Option<PathBuf>,
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::TempDir;

    use crate::tools::wasm::loader::{WasmLoadError, discover_tools};

    #[tokio::test]
    async fn test_discover_tools_empty_dir() {
        let dir = TempDir::new().unwrap();
        let tools = discover_tools(dir.path()).await.unwrap();
        assert!(tools.is_empty());
    }

    #[tokio::test]
    async fn test_discover_tools_with_wasm() {
        let dir = TempDir::new().unwrap();

        // Create a fake .wasm file
        let wasm_path = dir.path().join("test_tool.wasm");
        std::fs::File::create(&wasm_path).unwrap();

        let tools = discover_tools(dir.path()).await.unwrap();
        assert_eq!(tools.len(), 1);
        assert!(tools.contains_key("test_tool"));
        assert!(tools["test_tool"].capabilities_path.is_none());
    }

    #[tokio::test]
    async fn test_discover_tools_with_capabilities() {
        let dir = TempDir::new().unwrap();

        // Create wasm and capabilities files
        std::fs::File::create(dir.path().join("slack.wasm")).unwrap();
        let mut cap_file =
            std::fs::File::create(dir.path().join("slack.capabilities.json")).unwrap();
        cap_file.write_all(b"{}").unwrap();

        let tools = discover_tools(dir.path()).await.unwrap();
        assert_eq!(tools.len(), 1);
        assert!(tools["slack"].capabilities_path.is_some());
    }

    #[tokio::test]
    async fn test_discover_tools_ignores_non_wasm() {
        let dir = TempDir::new().unwrap();

        // Create non-wasm files
        std::fs::File::create(dir.path().join("readme.md")).unwrap();
        std::fs::File::create(dir.path().join("config.json")).unwrap();
        std::fs::File::create(dir.path().join("tool.wasm")).unwrap();

        let tools = discover_tools(dir.path()).await.unwrap();
        assert_eq!(tools.len(), 1);
        assert!(tools.contains_key("tool"));
    }

    #[test]
    fn test_load_error_display() {
        let err = WasmLoadError::InvalidName("bad/name".to_string());
        assert!(err.to_string().contains("bad/name"));

        let err = WasmLoadError::WasmNotFound(std::path::PathBuf::from("/foo/bar.wasm"));
        assert!(err.to_string().contains("/foo/bar.wasm"));
    }

    #[test]
    fn test_tools_src_dir_default() {
        let dir = super::tools_src_dir();
        assert!(dir.ends_with("tools-src"));
    }

    #[tokio::test]
    async fn test_discover_dev_tools_finds_build_artifacts() {
        // This test relies on the actual tools-src/ directory in the repo.
        // If build artifacts exist, they should be discovered.
        let tools = super::discover_dev_tools().await.unwrap();

        // If any tools have been built, they should appear with "-tool" suffix
        for (name, discovered) in &tools {
            assert!(
                name.ends_with("-tool"),
                "Dev tool name should end with -tool: {}",
                name
            );
            assert!(
                discovered.wasm_path.exists(),
                "WASM should exist: {:?}",
                discovered.wasm_path
            );
        }
    }

    #[test]
    fn test_resolve_oauth_refresh_config_with_oauth() {
        use crate::tools::wasm::capabilities_schema::{
            AuthCapabilitySchema, CapabilitiesFile, OAuthConfigSchema,
        };

        let caps = CapabilitiesFile {
            auth: Some(AuthCapabilitySchema {
                secret_name: "google_oauth_token".to_string(),
                provider: Some("google".to_string()),
                oauth: Some(OAuthConfigSchema {
                    authorization_url: "https://accounts.google.com/o/oauth2/v2/auth".to_string(),
                    token_url: "https://oauth2.googleapis.com/token".to_string(),
                    client_id: Some("test-client-id".to_string()),
                    client_secret: Some("test-client-secret".to_string()),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };

        let config = super::resolve_oauth_refresh_config(&caps);
        assert!(config.is_some());

        let config = config.unwrap();
        assert_eq!(config.token_url, "https://oauth2.googleapis.com/token");
        assert_eq!(config.client_id, "test-client-id");
        assert_eq!(config.client_secret, Some("test-client-secret".to_string()));
        assert_eq!(config.secret_name, "google_oauth_token");
        assert_eq!(config.provider, Some("google".to_string()));
    }

    #[test]
    fn test_resolve_oauth_refresh_config_no_auth() {
        use crate::tools::wasm::capabilities_schema::CapabilitiesFile;

        let caps = CapabilitiesFile::default();
        let config = super::resolve_oauth_refresh_config(&caps);
        assert!(config.is_none());
    }

    #[test]
    fn test_resolve_oauth_refresh_config_no_oauth() {
        use crate::tools::wasm::capabilities_schema::{AuthCapabilitySchema, CapabilitiesFile};

        let caps = CapabilitiesFile {
            auth: Some(AuthCapabilitySchema {
                secret_name: "manual_token".to_string(),
                ..Default::default()
            }),
            ..Default::default()
        };

        let config = super::resolve_oauth_refresh_config(&caps);
        assert!(config.is_none());
    }

    #[test]
    fn test_resolve_oauth_refresh_config_no_client_id() {
        use crate::tools::wasm::capabilities_schema::{
            AuthCapabilitySchema, CapabilitiesFile, OAuthConfigSchema,
        };

        // A non-Google provider with no client_id anywhere should return None
        let caps = CapabilitiesFile {
            auth: Some(AuthCapabilitySchema {
                secret_name: "unknown_provider_token".to_string(),
                oauth: Some(OAuthConfigSchema {
                    authorization_url: "https://example.com/auth".to_string(),
                    token_url: "https://example.com/token".to_string(),
                    // No client_id, no client_id_env, no builtin
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };

        let config = super::resolve_oauth_refresh_config(&caps);
        assert!(config.is_none());
    }

    #[test]
    fn test_resolve_oauth_refresh_config_builtin_google() {
        use crate::tools::wasm::capabilities_schema::{
            AuthCapabilitySchema, CapabilitiesFile, OAuthConfigSchema,
        };

        // google_oauth_token should fall back to built-in credentials
        let caps = CapabilitiesFile {
            auth: Some(AuthCapabilitySchema {
                secret_name: "google_oauth_token".to_string(),
                provider: Some("google".to_string()),
                oauth: Some(OAuthConfigSchema {
                    authorization_url: "https://accounts.google.com/o/oauth2/v2/auth".to_string(),
                    token_url: "https://oauth2.googleapis.com/token".to_string(),
                    // No inline client_id, should fall back to builtin
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };

        let config = super::resolve_oauth_refresh_config(&caps);
        assert!(config.is_some());
        let config = config.unwrap();
        assert!(!config.client_id.is_empty());
        assert!(config.client_secret.is_some());
    }
}
