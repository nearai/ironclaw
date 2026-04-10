use std::path::PathBuf;
use std::time::Duration;

use crate::bootstrap::ironclaw_base_dir;
use crate::config::helpers::{db_first_bool, db_first_or_default, optional_env};
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
    ironclaw_base_dir().join("tools")
}

impl WasmConfig {
    pub(crate) fn resolve(settings: &crate::settings::Settings) -> Result<Self, ConfigError> {
        let ws = &settings.wasm;
        let defaults = crate::settings::WasmSettings::default();
        Ok(Self {
            enabled: db_first_bool(ws.enabled, defaults.enabled, "WASM_ENABLED")?,
            tools_dir: if let Some(ref dir) = ws.tools_dir {
                dir.clone()
            } else {
                optional_env("WASM_TOOLS_DIR")?
                    .map(PathBuf::from)
                    .unwrap_or_else(default_tools_dir)
            },
            default_memory_limit: db_first_or_default(
                &ws.default_memory_limit,
                &defaults.default_memory_limit,
                "WASM_DEFAULT_MEMORY_LIMIT",
            )?,
            default_timeout_secs: db_first_or_default(
                &ws.default_timeout_secs,
                &defaults.default_timeout_secs,
                "WASM_DEFAULT_TIMEOUT_SECS",
            )?,
            default_fuel_limit: db_first_or_default(
                &ws.default_fuel_limit,
                &defaults.default_fuel_limit,
                "WASM_DEFAULT_FUEL_LIMIT",
            )?,
            cache_compiled: db_first_bool(
                ws.cache_compiled,
                defaults.cache_compiled,
                "WASM_CACHE_COMPILED",
            )?,
            cache_dir: if let Some(ref dir) = ws.cache_dir {
                Some(dir.clone())
            } else {
                optional_env("WASM_CACHE_DIR")?.map(PathBuf::from)
            },
        })
    }

    /// Convert to WasmRuntimeConfig.
    #[cfg(feature = "wasm-sandbox")]
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::helpers::lock_env;
    use crate::settings::Settings;

    #[test]
    fn resolve_falls_back_to_settings() {
        let _guard = lock_env();
        let mut settings = Settings::default();
        settings.wasm.default_memory_limit = 42;
        settings.wasm.cache_compiled = false;

        let cfg = WasmConfig::resolve(&settings).expect("resolve");
        assert_eq!(cfg.default_memory_limit, 42);
        assert!(!cfg.cache_compiled);
    }

    #[test]
    fn db_settings_override_env() {
        let _guard = lock_env();
        let mut settings = Settings::default();
        settings.wasm.default_fuel_limit = 42;

        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe { std::env::set_var("WASM_DEFAULT_FUEL_LIMIT", "7") };
        let cfg = WasmConfig::resolve(&settings).expect("resolve");
        unsafe { std::env::remove_var("WASM_DEFAULT_FUEL_LIMIT") };

        assert_eq!(cfg.default_fuel_limit, 42);
    }

    #[test]
    fn env_used_when_no_db_setting() {
        let _guard = lock_env();
        let settings = Settings::default();

        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe { std::env::set_var("WASM_DEFAULT_FUEL_LIMIT", "7") };
        let cfg = WasmConfig::resolve(&settings).expect("resolve");
        unsafe { std::env::remove_var("WASM_DEFAULT_FUEL_LIMIT") };

        assert_eq!(cfg.default_fuel_limit, 7);
    }
}
