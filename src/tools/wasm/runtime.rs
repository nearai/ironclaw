//! WASM tool runtime for managing compiled components.
//!
//! Follows the principle: compile once at registration, instantiate fresh per execution.
//! This matches NEAR blockchain patterns for deterministic, isolated execution.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;
use wasmtime::{Cache, Config, Engine, OptLevel};

use crate::tools::wasm::error::WasmError;
use crate::tools::wasm::limits::{FuelConfig, ResourceLimits};

/// Default epoch tick interval. Each tick increments the engine's epoch counter,
/// which causes any store with an expired epoch deadline to trap.
pub const EPOCH_TICK_INTERVAL: Duration = Duration::from_millis(500);

/// Enable Wasmtime's persistent compilation cache for a [`Config`].
///
/// If `explicit_dir` is `Some`, this writes a small cache TOML file pointing
/// at that directory and loads it with [`Cache::from_file`].
///
/// If `explicit_dir` is `None`, we load Wasmtime's default cache configuration
/// by calling `Cache::from_file(None)`.
///
/// On Windows, the caller typically passes an engine-specific directory keyed
/// by `label` to avoid OS error 33 (`ERROR_LOCK_VIOLATION`) when multiple
/// engines memory-map files in the same cache directory. See #448.
pub fn enable_compilation_cache(
    wasmtime_config: &mut Config,
    label: &str,
    explicit_dir: Option<&Path>,
) -> anyhow::Result<()> {
    // If the caller provided an explicit directory, or we're on Windows and
    // need per-engine isolation, write a TOML config with a custom directory.
    let custom_dir = match explicit_dir {
        Some(dir) => Some(dir.to_path_buf()),
        #[cfg(windows)]
        None => {
            let base = dirs::cache_dir()
                .unwrap_or_else(std::env::temp_dir)
                .join("ironclaw");
            Some(base.join(format!("wasmtime-{}", label)))
        }
        #[cfg(not(windows))]
        None => {
            let _ = label;
            None
        }
    };

    match custom_dir {
        Some(dir) => {
            std::fs::create_dir_all(&dir)?;
            let toml_path = dir.join("wasmtime-cache.toml");
            let escaped = dir
                .to_string_lossy()
                .replace('\\', "\\\\")
                .replace('"', "\\\"");
            let toml_content = format!("[cache]\ndirectory = \"{}\"\n", escaped);
            std::fs::write(&toml_path, toml_content)?;
            wasmtime_config.cache(Some(Cache::from_file(Some(&toml_path))?));
            Ok(())
        }
        None => {
            wasmtime_config.cache(Some(Cache::from_file(None)?));
            Ok(())
        }
    }
}

/// Configuration for the WASM runtime.
#[derive(Debug, Clone)]
pub struct WasmRuntimeConfig {
    /// Default resource limits for tools.
    pub default_limits: ResourceLimits,
    /// Fuel configuration.
    pub fuel_config: FuelConfig,
    /// Whether to cache compiled modules.
    pub cache_compiled: bool,
    /// Directory for compiled module cache.
    pub cache_dir: Option<PathBuf>,
    /// Cranelift optimization level.
    pub optimization_level: OptLevel,
}

impl Default for WasmRuntimeConfig {
    fn default() -> Self {
        Self {
            default_limits: ResourceLimits::default(),
            fuel_config: FuelConfig::default(),
            cache_compiled: true,
            cache_dir: None,
            optimization_level: OptLevel::Speed,
        }
    }
}

impl WasmRuntimeConfig {
    /// Create a minimal config for testing.
    pub fn for_testing() -> Self {
        Self {
            default_limits: ResourceLimits::default()
                .with_memory(1024 * 1024) // 1 MB
                .with_fuel(100_000)
                .with_timeout(Duration::from_secs(5)),
            fuel_config: FuelConfig::with_limit(100_000),
            cache_compiled: false,
            cache_dir: None,
            optimization_level: OptLevel::None, // Faster compilation for tests
        }
    }
}

/// A compiled WASM component ready for instantiation.
///
/// Contains the pre-compiled component plus cached metadata extracted
/// from the component during preparation. Stores the compiled `Component`
/// directly so instantiation doesn't require recompilation.
pub struct PreparedModule {
    /// Tool name.
    pub name: String,
    /// Tool description (cached from component).
    pub description: String,
    /// Full parameter schema JSON extracted from the component.
    /// Used for discovery and coercion, not necessarily for the compact
    /// schema advertised in the main tools array.
    pub schema: serde_json::Value,
    /// Pre-compiled component (cheaply cloneable via internal Arc).
    component: wasmtime::component::Component,
    /// Resource limits for this tool.
    pub limits: ResourceLimits,
}

impl PreparedModule {
    /// Get the pre-compiled component for instantiation.
    pub fn component(&self) -> &wasmtime::component::Component {
        &self.component
    }
}

impl std::fmt::Debug for PreparedModule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedModule")
            .field("name", &self.name)
            .field("description", &self.description)
            .field("limits", &self.limits)
            .finish()
    }
}

/// WASM tool runtime.
///
/// Manages the Wasmtime engine and a cache of prepared modules.
pub struct WasmToolRuntime {
    /// Wasmtime engine with configured settings.
    engine: Engine,
    /// Runtime configuration.
    config: WasmRuntimeConfig,
    /// Cache of prepared modules by name.
    modules: RwLock<HashMap<String, Arc<PreparedModule>>>,
}

impl WasmToolRuntime {
    /// Create a new runtime with the given configuration.
    pub fn new(config: WasmRuntimeConfig) -> Result<Self, WasmError> {
        let mut wasmtime_config = Config::new();

        // Enable fuel consumption for CPU limiting
        if config.fuel_config.enabled {
            wasmtime_config.consume_fuel(true);
        }

        // Enable epoch interruption as a backup timeout mechanism
        wasmtime_config.epoch_interruption(true);

        // Enable component model (WASI Preview 2)
        wasmtime_config.wasm_component_model(true);

        // Disable threads (simplifies security model)
        wasmtime_config.wasm_threads(false);

        // Set optimization level
        wasmtime_config.cranelift_opt_level(config.optimization_level);

        // Disable debug info in production for smaller modules
        wasmtime_config.debug_info(false);

        // Enable persistent compilation cache. Wasmtime serializes compiled native
        // code to disk (~/.cache/wasmtime by default), so subsequent startups
        // deserialize instead of recompiling — typically 10-50x faster.
        //
        // On Windows, each Engine gets its own cache subdirectory to avoid
        // OS error 33 (ERROR_LOCK_VIOLATION) when multiple engines share the
        // default cache and Windows holds exclusive locks on memory-mapped
        // files. See #448.
        if let Err(e) =
            enable_compilation_cache(&mut wasmtime_config, "tools", config.cache_dir.as_deref())
        {
            tracing::warn!("Failed to enable wasmtime compilation cache: {}", e);
        }

        let engine = Engine::new(&wasmtime_config).map_err(|e| {
            WasmError::EngineCreationFailed(format!("Failed to create Wasmtime engine: {}", e))
        })?;

        // Spawn a background thread that periodically increments the engine's
        // epoch counter. Without this, epoch_deadline_trap() never fires and
        // WASM modules can spin indefinitely even with a deadline set.
        let ticker_engine = engine.clone();
        std::thread::Builder::new()
            .name("wasm-epoch-ticker".into())
            .spawn(move || {
                loop {
                    std::thread::sleep(EPOCH_TICK_INTERVAL);
                    ticker_engine.increment_epoch();
                }
            })
            .map_err(|e| {
                WasmError::EngineCreationFailed(format!(
                    "Failed to spawn epoch ticker thread: {}",
                    e
                ))
            })?;

        Ok(Self {
            engine,
            config,
            modules: RwLock::new(HashMap::new()),
        })
    }

    /// Get the Wasmtime engine.
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Get the runtime configuration.
    pub fn config(&self) -> &WasmRuntimeConfig {
        &self.config
    }

    /// Prepare a WASM component for execution.
    ///
    /// This validates and compiles the component, extracting metadata.
    /// The compiled component is cached for fast instantiation.
    pub async fn prepare(
        &self,
        name: &str,
        wasm_bytes: &[u8],
        limits: Option<ResourceLimits>,
    ) -> Result<Arc<PreparedModule>, WasmError> {
        // Check if already prepared
        if let Some(module) = self.modules.read().await.get(name) {
            return Ok(Arc::clone(module));
        }

        let name = name.to_string();
        let wasm_bytes = wasm_bytes.to_vec();
        let engine = self.engine.clone();
        let default_limits = self.config.default_limits.clone();

        // Compile in blocking task (Wasmtime compilation is synchronous)
        let prepared = tokio::task::spawn_blocking(move || {
            // Validate and compile the component
            let component = wasmtime::component::Component::new(&engine, &wasm_bytes)
                .map_err(|e| WasmError::CompilationFailed(e.to_string()))?;

            // Briefly instantiate to extract metadata (description + schema)
            // from the tool's exports, analogous to MCP's list_tools().
            let effective_limits = limits.clone().unwrap_or(default_limits.clone());
            let (description, schema) = crate::tools::wasm::wrapper::extract_wasm_metadata(
                &engine,
                &component,
                &effective_limits,
            )
            .unwrap_or_else(|e| {
                tracing::warn!(
                    name = %name,
                    error = %e,
                    "WASM metadata extraction failed, using fallbacks"
                );
                (
                    "WASM sandboxed tool".to_string(),
                    serde_json::json!({
                        "type": "object",
                        "properties": {},
                        "additionalProperties": true
                    }),
                )
            });

            Ok::<_, WasmError>(PreparedModule {
                name: name.clone(),
                description,
                schema,
                component,
                limits: limits.unwrap_or(default_limits),
            })
        })
        .await
        .map_err(|e| WasmError::ExecutionPanicked(format!("Preparation task panicked: {}", e)))??;

        let prepared = Arc::new(prepared);

        // Cache the prepared module
        if self.config.cache_compiled {
            self.modules
                .write()
                .await
                .insert(prepared.name.clone(), Arc::clone(&prepared));
        }

        tracing::debug!(
            name = %prepared.name,
            "Prepared WASM tool for execution"
        );

        Ok(prepared)
    }

    /// Get a prepared module by name.
    pub async fn get(&self, name: &str) -> Option<Arc<PreparedModule>> {
        self.modules.read().await.get(name).cloned()
    }

    /// Remove a prepared module from the cache.
    pub async fn remove(&self, name: &str) -> Option<Arc<PreparedModule>> {
        self.modules.write().await.remove(name)
    }

    /// List all prepared module names.
    pub async fn list(&self) -> Vec<String> {
        self.modules.read().await.keys().cloned().collect()
    }

    /// Clear all cached modules.
    pub async fn clear(&self) {
        self.modules.write().await.clear();
    }
}

impl std::fmt::Debug for WasmToolRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmToolRuntime")
            .field("config", &self.config)
            .field("modules", &"<RwLock<HashMap>>")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use crate::tools::wasm::limits::ResourceLimits;
    use crate::tools::wasm::runtime::{WasmRuntimeConfig, WasmToolRuntime};

    #[test]
    fn test_runtime_config_default() {
        let config = WasmRuntimeConfig::default();
        assert!(config.cache_compiled);
        assert!(config.fuel_config.enabled);
    }

    #[test]
    fn test_runtime_config_for_testing() {
        let config = WasmRuntimeConfig::for_testing();
        assert!(!config.cache_compiled);
        assert_eq!(config.default_limits.memory_bytes, 1024 * 1024);
    }

    #[test]
    fn test_runtime_creation() {
        let config = WasmRuntimeConfig::for_testing();
        let runtime = WasmToolRuntime::new(config).unwrap();
        // Engine was created successfully, which validates the config
        assert!(runtime.config().fuel_config.enabled);
    }

    #[tokio::test]
    async fn test_module_cache_operations() {
        let config = WasmRuntimeConfig::for_testing();
        let runtime = WasmToolRuntime::new(config).unwrap();

        // Initially empty
        assert!(runtime.list().await.is_empty());
        assert!(runtime.get("test").await.is_none());
    }

    #[test]
    fn test_prepared_module_limits() {
        let limits = ResourceLimits::default()
            .with_memory(5 * 1024 * 1024)
            .with_fuel(500_000);

        assert_eq!(limits.memory_bytes, 5 * 1024 * 1024);
        assert_eq!(limits.fuel, 500_000);
    }

    /// Per-engine cache directories must work correctly to avoid file lock
    /// conflicts on Windows where multiple engines sharing a single cache
    /// directory triggers OS error 33 (ERROR_LOCK_VIOLATION). Regression test
    /// for #448: `enable_compilation_cache` must create a subdirectory and
    /// produce a valid TOML config that wasmtime can load.
    #[test]
    fn test_enable_compilation_cache_with_explicit_dir() {
        use crate::tools::wasm::runtime::enable_compilation_cache;

        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let cache_dir = tmp.path().join("custom-cache");

        let mut config = wasmtime::Config::new();
        enable_compilation_cache(&mut config, "test-engine", Some(cache_dir.as_path()))
            .expect("enable_compilation_cache should succeed with explicit dir");

        // The cache directory should have been created.
        assert!(cache_dir.exists(), "cache directory should be created");

        // A TOML config file should have been written inside.
        let toml_path = cache_dir.join("wasmtime-cache.toml");
        assert!(toml_path.exists(), "TOML config should be written");

        let content = std::fs::read_to_string(&toml_path).unwrap();
        assert!(
            content.contains("[cache]"),
            "TOML must contain [cache] section"
        );
        assert!(
            content.contains("directory = "),
            "TOML must specify a cache directory"
        );
    }

    /// Two engines with different labels must get independent cache directories
    /// so that their file locks do not conflict. Regression test for #448.
    #[test]
    fn test_enable_compilation_cache_label_isolation() {
        use crate::tools::wasm::runtime::enable_compilation_cache;

        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let base = tmp.path().join("isolation");

        let dir_a = base.join("engine-a");
        let dir_b = base.join("engine-b");

        let mut config_a = wasmtime::Config::new();
        enable_compilation_cache(&mut config_a, "a", Some(dir_a.as_path()))
            .expect("cache A should succeed");

        let mut config_b = wasmtime::Config::new();
        enable_compilation_cache(&mut config_b, "b", Some(dir_b.as_path()))
            .expect("cache B should succeed");

        // Both directories must exist and be distinct.
        assert!(dir_a.exists());
        assert!(dir_b.exists());
        assert_ne!(dir_a, dir_b);
    }

    /// The WASM runtime (Wasmtime engine) must initialise successfully even
    /// when no tools directory exists on disk. The engine only configures the
    /// compiler and epoch ticker — loading modules from a directory is a
    /// separate step. Regression test for a bug where the runtime was gated
    /// on `tools_dir.exists()`, causing extensions installed after startup
    /// (e.g. via the web UI) to fail with "WASM runtime not available".
    #[test]
    fn test_runtime_creation_without_tools_dir() {
        let config = WasmRuntimeConfig::for_testing();
        // Runtime should succeed even though no tools directory exists.
        let runtime = WasmToolRuntime::new(config).expect("runtime should init without tools dir");
        assert!(runtime.config().fuel_config.enabled);
    }
}
