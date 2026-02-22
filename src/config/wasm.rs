use std::path::PathBuf;
use std::time::Duration;

use crate::config::helpers::{optional_env, parse_bool_env, parse_optional_env};
use crate::error::ConfigError;

/// WASM sandbox configuration.
#[derive(Debug, Clone)]
pub struct WasmConfig {
    /// Whether WASM tool execution is enabled.
    pub enabled: bool,
    /// Directory containing installed WASM tools (default: ~/.ironclaw/tools/).
    pub tools_dir: PathBuf,
    /// Default memory limit in bytes (default: 10 MB).
    pub default_memory_limit: u64,
    /// Default execution timeout in seconds (default: 60).
    pub default_timeout_secs: u64,
    /// Default fuel limit for CPU metering (default: 10M).
    pub default_fuel_limit: u64,
    /// Whether to cache compiled modules.
    pub cache_compiled: bool,
    /// Directory for compiled module cache.
    pub cache_dir: Option<PathBuf>,
}

impl Default for WasmConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            tools_dir: default_tools_dir(),
            default_memory_limit: 10 * 1024 * 1024, // 10 MB
            default_timeout_secs: 60,
            default_fuel_limit: 10_000_000,
            cache_compiled: true,
            cache_dir: None,
        }
    }
}

/// Get the default tools directory (~/.ironclaw/tools/).
fn default_tools_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ironclaw")
        .join("tools")
}

impl WasmConfig {
    pub(crate) fn resolve() -> Result<Self, ConfigError> {
        Ok(Self {
            enabled: parse_bool_env("WASM_ENABLED", true)?,
            tools_dir: optional_env("WASM_TOOLS_DIR")?
                .map(PathBuf::from)
                .unwrap_or_else(default_tools_dir),
            default_memory_limit: parse_optional_env(
                "WASM_DEFAULT_MEMORY_LIMIT",
                10 * 1024 * 1024,
            )?,
            default_timeout_secs: parse_optional_env("WASM_DEFAULT_TIMEOUT_SECS", 60)?,
            default_fuel_limit: parse_optional_env("WASM_DEFAULT_FUEL_LIMIT", 10_000_000)?,
            cache_compiled: parse_bool_env("WASM_CACHE_COMPILED", true)?,
            cache_dir: optional_env("WASM_CACHE_DIR")?.map(PathBuf::from),
        })
    }

    /// Convert to WasmRuntimeConfig.
    pub fn to_runtime_config(&self) -> crate::tools::wasm::WasmRuntimeConfig {
        use crate::tools::wasm::{FuelConfig, ResourceLimits, WasmRuntimeConfig};

        WasmRuntimeConfig {
            default_limits: ResourceLimits {
                memory_bytes: self.default_memory_limit,
                fuel: self.default_fuel_limit,
                timeout: Duration::from_secs(self.default_timeout_secs),
            },
            fuel_config: FuelConfig {
                initial_fuel: self.default_fuel_limit,
                enabled: true,
            },
            cache_compiled: self.cache_compiled,
            cache_dir: self.cache_dir.clone(),
            optimization_level: wasmtime::OptLevel::Speed,
        }
    }
}
