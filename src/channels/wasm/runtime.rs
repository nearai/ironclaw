//! WASM channel runtime for managing compiled channel components.
//!
//! Similar to tool runtime, follows the principle: compile once at registration,
//! instantiate fresh per callback execution.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;
use wasmtime::{Config, Engine, OptLevel};

use crate::channels::wasm::error::WasmChannelError;
use crate::tools::wasm::{FuelConfig, ResourceLimits};

/// Configuration for the WASM channel runtime.
#[derive(Debug, Clone)]
pub struct WasmChannelRuntimeConfig {
    /// Default resource limits for channels.
    pub default_limits: ResourceLimits,
    /// Fuel configuration.
    pub fuel_config: FuelConfig,
    /// Whether to cache compiled modules.
    pub cache_compiled: bool,
    /// Directory for compiled module cache.
    pub cache_dir: Option<PathBuf>,
    /// Cranelift optimization level.
    pub optimization_level: OptLevel,
    /// Default callback timeout.
    pub callback_timeout: Duration,
}

impl Default for WasmChannelRuntimeConfig {
    fn default() -> Self {
        Self {
            default_limits: ResourceLimits {
                // Channels may need more memory for message buffering
                memory_bytes: 50 * 1024 * 1024, // 50 MB
                fuel: 10_000_000,
                timeout: Duration::from_secs(60),
            },
            fuel_config: FuelConfig::default(),
            cache_compiled: true,
            cache_dir: None,
            optimization_level: OptLevel::Speed,
            callback_timeout: Duration::from_secs(30),
        }
    }
}

impl WasmChannelRuntimeConfig {
    /// Create a minimal config for testing.
    pub fn for_testing() -> Self {
        Self {
            default_limits: ResourceLimits {
                memory_bytes: 5 * 1024 * 1024, // 5 MB
                fuel: 1_000_000,
                timeout: Duration::from_secs(5),
            },
            fuel_config: FuelConfig::with_limit(1_000_000),
            cache_compiled: false,
            cache_dir: None,
            optimization_level: OptLevel::None, // Faster compilation for tests
            callback_timeout: Duration::from_secs(5),
        }
    }
}

/// A compiled WASM channel component ready for instantiation.
///
/// Stores the pre-compiled `Component` directly so instantiation
/// doesn't require recompilation.
pub struct PreparedChannelModule {
    /// Channel name.
    pub name: String,
    /// Channel description.
    pub description: String,
    /// Pre-compiled component (cheaply cloneable via internal Arc).
    pub(crate) component: Option<wasmtime::component::Component>,
    /// Resource limits for this channel.
    pub limits: ResourceLimits,
}

impl PreparedChannelModule {
    /// Get the pre-compiled component for instantiation.
    pub fn component(&self) -> Option<&wasmtime::component::Component> {
        self.component.as_ref()
    }

    /// Create a PreparedChannelModule for testing purposes.
    ///
    /// Creates a module with no actual WASM component, suitable for testing
    /// channel infrastructure without requiring a real WASM component.
    pub fn for_testing(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            component: None,
            limits: ResourceLimits::default(),
        }
    }
}

impl std::fmt::Debug for PreparedChannelModule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedChannelModule")
            .field("name", &self.name)
            .field("description", &self.description)
            .field("has_component", &self.component.is_some())
            .field("limits", &self.limits)
            .finish()
    }
}

/// WASM channel runtime.
///
/// Manages the Wasmtime engine and a cache of prepared channel modules.
pub struct WasmChannelRuntime {
    /// Wasmtime engine with configured settings.
    engine: Engine,
    /// Runtime configuration.
    config: WasmChannelRuntimeConfig,
    /// Cache of prepared modules by name.
    modules: RwLock<HashMap<String, Arc<PreparedChannelModule>>>,
}

impl WasmChannelRuntime {
    /// Create a new runtime with the given configuration.
    pub fn new(config: WasmChannelRuntimeConfig) -> Result<Self, WasmChannelError> {
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

        // Disable debug info in production
        wasmtime_config.debug_info(false);

        // Enable persistent compilation cache. Wasmtime serializes compiled native
        // code to disk (~/.cache/wasmtime by default), so subsequent startups
        // deserialize instead of recompiling — typically 10-50x faster.
        if let Err(e) = wasmtime_config.cache_config_load_default() {
            tracing::warn!("Failed to enable wasmtime compilation cache: {}", e);
        }

        let engine = Engine::new(&wasmtime_config).map_err(|e| {
            WasmChannelError::Config(format!("Failed to create Wasmtime engine: {}", e))
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
    pub fn config(&self) -> &WasmChannelRuntimeConfig {
        &self.config
    }

    /// Prepare a WASM channel component for execution.
    ///
    /// This validates and compiles the component.
    /// The compiled component is cached for fast instantiation.
    pub async fn prepare(
        &self,
        name: &str,
        wasm_bytes: &[u8],
        limits: Option<ResourceLimits>,
        description: Option<String>,
    ) -> Result<Arc<PreparedChannelModule>, WasmChannelError> {
        // Check if already prepared
        if let Some(module) = self.modules.read().await.get(name) {
            return Ok(Arc::clone(module));
        }

        let name = name.to_string();
        let wasm_bytes = wasm_bytes.to_vec();
        let engine = self.engine.clone();
        let default_limits = self.config.default_limits.clone();
        let desc = description.unwrap_or_else(|| format!("WASM channel: {}", name));

        // Compile in blocking task (Wasmtime compilation is synchronous)
        let prepared = tokio::task::spawn_blocking(move || {
            // Validate and compile the component
            let component = wasmtime::component::Component::new(&engine, &wasm_bytes)
                .map_err(|e| WasmChannelError::Compilation(e.to_string()))?;

            Ok::<_, WasmChannelError>(PreparedChannelModule {
                name: name.clone(),
                description: desc,
                component: Some(component),
                limits: limits.unwrap_or(default_limits),
            })
        })
        .await
        .map_err(|e| {
            WasmChannelError::Compilation(format!("Preparation task panicked: {}", e))
        })??;

        let prepared = Arc::new(prepared);

        // Cache the prepared module
        if self.config.cache_compiled {
            self.modules
                .write()
                .await
                .insert(prepared.name.clone(), Arc::clone(&prepared));
        }

        tracing::info!(
            name = %prepared.name,
            "Prepared WASM channel for execution"
        );

        Ok(prepared)
    }

    /// Get a prepared module by name.
    pub async fn get(&self, name: &str) -> Option<Arc<PreparedChannelModule>> {
        self.modules.read().await.get(name).cloned()
    }

    /// Remove a prepared module from the cache.
    pub async fn remove(&self, name: &str) -> Option<Arc<PreparedChannelModule>> {
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

impl std::fmt::Debug for WasmChannelRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmChannelRuntime")
            .field("config", &self.config)
            .field("modules", &"<RwLock<HashMap>>")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use crate::channels::wasm::runtime::{WasmChannelRuntime, WasmChannelRuntimeConfig};

    #[test]
    fn test_runtime_config_default() {
        let config = WasmChannelRuntimeConfig::default();
        assert!(config.cache_compiled);
        assert!(config.fuel_config.enabled);
        // Channels get more memory than tools
        assert_eq!(config.default_limits.memory_bytes, 50 * 1024 * 1024);
    }

    #[test]
    fn test_runtime_config_for_testing() {
        let config = WasmChannelRuntimeConfig::for_testing();
        assert!(!config.cache_compiled);
        assert_eq!(config.default_limits.memory_bytes, 5 * 1024 * 1024);
    }

    #[test]
    fn test_runtime_creation() {
        let config = WasmChannelRuntimeConfig::for_testing();
        let runtime = WasmChannelRuntime::new(config).unwrap();
        assert!(runtime.config().fuel_config.enabled);
    }

    #[tokio::test]
    async fn test_module_cache_operations() {
        let config = WasmChannelRuntimeConfig::for_testing();
        let runtime = WasmChannelRuntime::new(config).unwrap();

        // Initially empty
        assert!(runtime.list().await.is_empty());
        assert!(runtime.get("test").await.is_none());
    }
}
