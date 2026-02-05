//! Generic WASM tool loader for loading tools from files or directories.
//!
//! This module provides a way to load WASM tools dynamically at runtime from:
//! - A directory containing `<name>.wasm` and `<name>.capabilities.json`
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
//! # Security
//!
//! Tools loaded from files are assigned `TrustLevel::User` by default, meaning
//! they run with the most restrictive permissions. Only tools explicitly marked
//! as `verified` or `system` in the database get elevated trust.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::fs;

use crate::tools::registry::{ToolRegistry, WasmRegistrationError, WasmToolRegistration};
use crate::tools::wasm::capabilities_schema::CapabilitiesFile;
use crate::tools::wasm::{
    Capabilities, WasmError, WasmStorageError, WasmToolRuntime, WasmToolStore,
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
}

impl WasmToolLoader {
    /// Create a new loader with the given runtime and registry.
    pub fn new(runtime: Arc<WasmToolRuntime>, registry: Arc<ToolRegistry>) -> Self {
        Self { runtime, registry }
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

        // Read capabilities (optional)
        let capabilities = if let Some(cap_path) = capabilities_path {
            if cap_path.exists() {
                let cap_bytes = fs::read(cap_path).await?;
                let cap_file = CapabilitiesFile::from_bytes(&cap_bytes)
                    .map_err(|e| WasmLoadError::InvalidCapabilities(e.to_string()))?;
                cap_file.to_capabilities()
            } else {
                tracing::warn!(
                    path = %cap_path.display(),
                    "Capabilities file not found, using default (no permissions)"
                );
                Capabilities::default()
            }
        } else {
            Capabilities::default()
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
        let mut entries = fs::read_dir(dir).await?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();

            // Only process .wasm files
            if path.extension().and_then(|e| e.to_str()) != Some("wasm") {
                continue;
            }

            // Extract tool name from filename
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

            // Look for sidecar capabilities file
            let cap_path = path.with_extension("capabilities.json");
            let cap_path_option = if cap_path.exists() {
                Some(cap_path.as_path())
            } else {
                None
            };

            match self.load_from_files(&name, &path, cap_path_option).await {
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
}
