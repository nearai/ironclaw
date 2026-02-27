//! WASM channel loader for loading channels from files or directories.
//!
//! Loads WASM channel modules from the filesystem (default: ~/.ironclaw/channels/).
//! Each channel consists of:
//! - `<name>.wasm` - The compiled WASM component
//! - `<name>.capabilities.json` - Channel capabilities and configuration

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::fs;

use crate::channels::wasm::capabilities::ChannelCapabilities;
use crate::channels::wasm::error::WasmChannelError;
use crate::channels::wasm::runtime::WasmChannelRuntime;
use crate::channels::wasm::schema::ChannelCapabilitiesFile;
use crate::channels::wasm::wrapper::WasmChannel;
use crate::pairing::PairingStore;

/// Loads WASM channels from the filesystem.
pub struct WasmChannelLoader {
    runtime: Arc<WasmChannelRuntime>,
    pairing_store: Arc<PairingStore>,
}

impl WasmChannelLoader {
    /// Create a new loader with the given runtime and pairing store.
    pub fn new(runtime: Arc<WasmChannelRuntime>, pairing_store: Arc<PairingStore>) -> Self {
        Self {
            runtime,
            pairing_store,
        }
    }

    /// Load a single WASM channel from a file pair.
    ///
    /// Expects:
    /// - `wasm_path`: Path to the `.wasm` file
    /// - `capabilities_path`: Path to the `.capabilities.json` file (optional)
    ///
    /// If no capabilities file is provided, the channel gets minimal capabilities.
    pub async fn load_from_files(
        &self,
        name: &str,
        wasm_path: &Path,
        capabilities_path: Option<&Path>,
    ) -> Result<LoadedChannel, WasmChannelError> {
        // Validate name
        if name.is_empty() || name.contains('/') || name.contains('\\') || name.contains("..") {
            return Err(WasmChannelError::InvalidName(name.to_string()));
        }

        // Read WASM bytes
        if !wasm_path.exists() {
            return Err(WasmChannelError::WasmNotFound(wasm_path.to_path_buf()));
        }
        let wasm_bytes = fs::read(wasm_path).await?;

        // Read capabilities file
        let (capabilities, config_json, description, cap_file) =
            if let Some(cap_path) = capabilities_path {
                if cap_path.exists() {
                    let cap_bytes = fs::read(cap_path).await?;
                    let cap_file = ChannelCapabilitiesFile::from_bytes(&cap_bytes)
                        .map_err(|e| WasmChannelError::InvalidCapabilities(e.to_string()))?;

                    // Debug: log raw capabilities
                    tracing::debug!(
                        channel = name,
                        raw_capabilities = ?cap_file.capabilities,
                        "Parsed capabilities file"
                    );

                    let caps = cap_file.to_capabilities();

                    // Debug: log resulting capabilities
                    tracing::info!(
                        channel = name,
                        http_allowed = caps.tool_capabilities.http.is_some(),
                        http_allowlist_count = caps
                            .tool_capabilities
                            .http
                            .as_ref()
                            .map(|h| h.allowlist.len())
                            .unwrap_or(0),
                        "Channel capabilities loaded"
                    );

                    let config = cap_file.config_json();
                    let desc = cap_file.description.clone();

                    (caps, config, desc, Some(cap_file))
                } else {
                    tracing::warn!(
                        path = %cap_path.display(),
                        "Capabilities file not found, using defaults"
                    );
                    (
                        ChannelCapabilities::for_channel(name),
                        "{}".to_string(),
                        None,
                        None,
                    )
                }
            } else {
                (
                    ChannelCapabilities::for_channel(name),
                    "{}".to_string(),
                    None,
                    None,
                )
            };

        // Prepare the module
        let prepared = self
            .runtime
            .prepare(name, &wasm_bytes, None, description)
            .await?;

        // Create the channel
        let channel = WasmChannel::new(
            self.runtime.clone(),
            prepared,
            capabilities,
            config_json,
            self.pairing_store.clone(),
        );

        tracing::info!(
            name = name,
            wasm_path = %wasm_path.display(),
            "Loaded WASM channel from file"
        );

        Ok(LoadedChannel {
            channel,
            capabilities_file: cap_file,
        })
    }

    /// Load all WASM channels from a directory.
    ///
    /// Scans the directory for `*.wasm` files and loads each one, looking for
    /// a matching `*.capabilities.json` sidecar file.
    ///
    /// # Directory Layout
    ///
    /// ```text
    /// channels/
    /// ├── slack.wasm                  <- Channel WASM component
    /// ├── slack.capabilities.json     <- Capabilities (optional)
    /// ├── telegram.wasm
    /// └── telegram.capabilities.json
    /// ```
    pub async fn load_from_dir(&self, dir: &Path) -> Result<LoadResults, WasmChannelError> {
        if !dir.is_dir() {
            return Err(WasmChannelError::Io(std::io::Error::new(
                std::io::ErrorKind::NotADirectory,
                format!("{} is not a directory", dir.display()),
            )));
        }

        let mut results = LoadResults::default();

        // Collect all .wasm entries first, then load in parallel
        let mut channel_entries = Vec::new();
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
                        WasmChannelError::InvalidName("invalid filename".to_string()),
                    ));
                    continue;
                }
            };

            let cap_path = path.with_extension("capabilities.json");
            let has_cap = cap_path.exists();
            channel_entries.push((name, path, if has_cap { Some(cap_path) } else { None }));
        }

        // Load all channels in parallel (file I/O + WASM compilation)
        let load_futures = channel_entries
            .iter()
            .map(|(name, path, cap_path)| self.load_from_files(name, path, cap_path.as_deref()));

        let load_results = futures::future::join_all(load_futures).await;

        for ((name, path, _), result) in channel_entries.into_iter().zip(load_results) {
            match result {
                Ok(loaded) => {
                    results.loaded.push(loaded);
                }
                Err(e) => {
                    tracing::error!(
                        name = name,
                        path = %path.display(),
                        error = %e,
                        "Failed to load WASM channel"
                    );
                    results.errors.push((path, e));
                }
            }
        }

        if !results.loaded.is_empty() {
            tracing::info!(
                count = results.loaded.len(),
                channels = ?results.loaded.iter().map(|c| c.name()).collect::<Vec<_>>(),
                "Loaded WASM channels from directory"
            );
        }

        Ok(results)
    }
}

/// A loaded WASM channel with its capabilities file.
pub struct LoadedChannel {
    /// The loaded channel.
    pub channel: WasmChannel,

    /// The parsed capabilities file (if present).
    pub capabilities_file: Option<ChannelCapabilitiesFile>,
}

impl LoadedChannel {
    /// Get the channel name.
    pub fn name(&self) -> &str {
        self.channel.channel_name()
    }

    /// Get the webhook secret header name from capabilities.
    pub fn webhook_secret_header(&self) -> Option<&str> {
        self.capabilities_file
            .as_ref()
            .and_then(|f| f.webhook_secret_header())
    }

    /// Get the signature verification key secret name from capabilities.
    pub fn signature_key_secret_name(&self) -> Option<String> {
        self.capabilities_file
            .as_ref()
            .and_then(|f| f.signature_key_secret_name().map(|s| s.to_string()))
    }

    /// Get the webhook secret name from capabilities.
    pub fn webhook_secret_name(&self) -> String {
        self.capabilities_file
            .as_ref()
            .map(|f| f.webhook_secret_name())
            .unwrap_or_else(|| format!("{}_webhook_secret", self.channel.channel_name()))
    }
}

/// Results from loading multiple channels.
#[derive(Default)]
pub struct LoadResults {
    /// Successfully loaded channels with their capabilities.
    pub loaded: Vec<LoadedChannel>,

    /// Errors encountered (path, error).
    pub errors: Vec<(PathBuf, WasmChannelError)>,
}

impl LoadResults {
    /// Check if all channels loaded successfully.
    pub fn all_succeeded(&self) -> bool {
        self.errors.is_empty()
    }

    /// Get the count of successfully loaded channels.
    pub fn success_count(&self) -> usize {
        self.loaded.len()
    }

    /// Get the count of failed channels.
    pub fn error_count(&self) -> usize {
        self.errors.len()
    }

    /// Take ownership of loaded channels (extracts just the WasmChannel).
    pub fn take_channels(self) -> Vec<WasmChannel> {
        self.loaded.into_iter().map(|l| l.channel).collect()
    }
}

/// Discover WASM channel files in a directory without loading them.
///
/// Returns a map of channel name -> (wasm_path, capabilities_path).
#[allow(dead_code)]
pub async fn discover_channels(
    dir: &Path,
) -> Result<HashMap<String, DiscoveredChannel>, std::io::Error> {
    let mut channels = HashMap::new();

    if !dir.is_dir() {
        return Ok(channels);
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

        channels.insert(
            name,
            DiscoveredChannel {
                wasm_path: path,
                capabilities_path: if cap_path.exists() {
                    Some(cap_path)
                } else {
                    None
                },
            },
        );
    }

    Ok(channels)
}

/// A discovered WASM channel (not yet loaded).
#[derive(Debug)]
pub struct DiscoveredChannel {
    /// Path to the WASM file.
    pub wasm_path: PathBuf,

    /// Path to the capabilities file (if present).
    pub capabilities_path: Option<PathBuf>,
}

/// Get the default channels directory path.
///
/// Returns ~/.ironclaw/channels/
#[allow(dead_code)]
pub fn default_channels_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ironclaw")
        .join("channels")
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::TempDir;

    use crate::channels::wasm::loader::{WasmChannelLoader, discover_channels};
    use crate::channels::wasm::runtime::{WasmChannelRuntime, WasmChannelRuntimeConfig};
    use crate::pairing::PairingStore;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_discover_channels_empty_dir() {
        let dir = TempDir::new().unwrap();
        let channels = discover_channels(dir.path()).await.unwrap();
        assert!(channels.is_empty());
    }

    #[tokio::test]
    async fn test_discover_channels_with_wasm() {
        let dir = TempDir::new().unwrap();

        // Create a fake .wasm file
        let wasm_path = dir.path().join("slack.wasm");
        std::fs::File::create(&wasm_path).unwrap();

        let channels = discover_channels(dir.path()).await.unwrap();
        assert_eq!(channels.len(), 1);
        assert!(channels.contains_key("slack"));
        assert!(channels["slack"].capabilities_path.is_none());
    }

    #[tokio::test]
    async fn test_discover_channels_with_capabilities() {
        let dir = TempDir::new().unwrap();

        // Create wasm and capabilities files
        std::fs::File::create(dir.path().join("telegram.wasm")).unwrap();
        let mut cap_file =
            std::fs::File::create(dir.path().join("telegram.capabilities.json")).unwrap();
        cap_file.write_all(b"{}").unwrap();

        let channels = discover_channels(dir.path()).await.unwrap();
        assert_eq!(channels.len(), 1);
        assert!(channels["telegram"].capabilities_path.is_some());
    }

    #[tokio::test]
    async fn test_discover_channels_ignores_non_wasm() {
        let dir = TempDir::new().unwrap();

        // Create non-wasm files
        std::fs::File::create(dir.path().join("readme.md")).unwrap();
        std::fs::File::create(dir.path().join("config.json")).unwrap();
        std::fs::File::create(dir.path().join("channel.wasm")).unwrap();

        let channels = discover_channels(dir.path()).await.unwrap();
        assert_eq!(channels.len(), 1);
        assert!(channels.contains_key("channel"));
    }

    #[test]
    fn test_loaded_channel_signature_key_none_without_caps() {
        // We can't easily construct a WasmChannel without a runtime, so test
        // the delegation logic directly: when capabilities_file is None, the
        // chain returns None (same logic as LoadedChannel::signature_key_secret_name).
        let cap_file: Option<crate::channels::wasm::schema::ChannelCapabilitiesFile> = None;
        let result = cap_file
            .as_ref()
            .and_then(|f| f.signature_key_secret_name().map(|s| s.to_string()));
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_loader_invalid_name() {
        let config = WasmChannelRuntimeConfig::for_testing();
        let runtime = Arc::new(WasmChannelRuntime::new(config).unwrap());
        let loader = WasmChannelLoader::new(runtime, Arc::new(PairingStore::new()));

        let dir = TempDir::new().unwrap();
        let wasm_path = dir.path().join("test.wasm");

        // Invalid name with path separator
        let result = loader.load_from_files("../escape", &wasm_path, None).await;
        assert!(result.is_err());

        // Empty name
        let result = loader.load_from_files("", &wasm_path, None).await;
        assert!(result.is_err());
    }
}
