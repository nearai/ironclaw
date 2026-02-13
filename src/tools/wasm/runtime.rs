//! WASM tool runtime for managing compiled components.
//!
//! Follows the principle: compile once at registration, instantiate fresh per execution.
//! This matches NEAR blockchain patterns for deterministic, isolated execution.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;
use wasmtime::{Config, Engine, OptLevel};

use crate::tools::wasm::error::WasmError;
use crate::tools::wasm::limits::{FuelConfig, ResourceLimits};

/// Default epoch tick interval. Each tick increments the engine's epoch counter,
/// which causes any store with an expired epoch deadline to trap.
pub const EPOCH_TICK_INTERVAL: Duration = Duration::from_millis(500);

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
/// from the component during preparation.
#[derive(Debug)]
pub struct PreparedModule {
    /// Tool name.
    pub name: String,
    /// Tool description (cached from component).
    pub description: String,
    /// Parameter schema JSON (cached from component).
    pub schema: serde_json::Value,
    /// Compiled component bytes (can be serialized for caching).
    component_bytes: Vec<u8>,
    /// Resource limits for this tool.
    pub limits: ResourceLimits,
}

impl PreparedModule {
    /// Get the compiled component bytes.
    pub fn component_bytes(&self) -> &[u8] {
        &self.component_bytes
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

            // We need to instantiate briefly to extract metadata.
            // In a full implementation, we'd use WIT bindgen to get typed access.
            // For now, we extract what we can from the component.
            let description = extract_tool_description(&engine, &component)?;
            let schema = extract_tool_schema(&engine, &component)?;

            Ok::<_, WasmError>(PreparedModule {
                name: name.clone(),
                description,
                schema,
                component_bytes: wasm_bytes,
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

        tracing::info!(
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

/// Extract tool description from a compiled component.
///
/// In a full implementation, this would use WIT bindgen to call the description() export.
/// For now, we return a placeholder since we can't easily introspect without more setup.
fn extract_tool_description(
    _engine: &Engine,
    _component: &wasmtime::component::Component,
) -> Result<String, WasmError> {
    // TODO: Use WIT bindgen to properly extract description
    // This requires instantiating with a linker, which needs host functions.
    // For now, tools should have their description set externally.
    Ok("WASM sandboxed tool".to_string())
}

/// Extract tool schema from a compiled component.
///
/// In a full implementation, this would use WIT bindgen to call the schema() export.
fn extract_tool_schema(
    _engine: &Engine,
    _component: &wasmtime::component::Component,
) -> Result<serde_json::Value, WasmError> {
    // TODO: Use WIT bindgen to properly extract schema
    // For now, return a minimal schema that accepts any object.
    Ok(serde_json::json!({
        "type": "object",
        "properties": {},
        "additionalProperties": true
    }))
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
}
