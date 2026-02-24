use std::path::PathBuf;
use std::time::Duration;

use crate::config::helpers::{optional_env, parse_bool_env, parse_optional_env};
use crate::error::ConfigError;

/// Builder mode configuration.
#[derive(Debug, Clone)]
pub struct BuilderModeConfig {
    /// Whether the software builder tool is enabled.
    pub enabled: bool,
    /// Directory for build artifacts (default: temp dir).
    pub build_dir: Option<PathBuf>,
    /// Maximum iterations for the build loop.
    pub max_iterations: u32,
    /// Build timeout in seconds.
    pub timeout_secs: u64,
    /// Whether to automatically register built WASM tools.
    pub auto_register: bool,
}

impl Default for BuilderModeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            build_dir: None,
            max_iterations: 20,
            timeout_secs: 600,
            auto_register: true,
        }
    }
}

impl BuilderModeConfig {
    pub(crate) fn resolve() -> Result<Self, ConfigError> {
        Ok(Self {
            enabled: parse_bool_env("BUILDER_ENABLED", true)?,
            build_dir: optional_env("BUILDER_DIR")?.map(PathBuf::from),
            max_iterations: parse_optional_env("BUILDER_MAX_ITERATIONS", 20)?,
            timeout_secs: parse_optional_env("BUILDER_TIMEOUT_SECS", 600)?,
            auto_register: parse_bool_env("BUILDER_AUTO_REGISTER", true)?,
        })
    }

    /// Convert to BuilderConfig for the builder tool.
    pub fn to_builder_config(&self) -> crate::tools::BuilderConfig {
        crate::tools::BuilderConfig {
            build_dir: self.build_dir.clone().unwrap_or_else(std::env::temp_dir),
            max_iterations: self.max_iterations,
            timeout: Duration::from_secs(self.timeout_secs),
            cleanup_on_failure: true,
            validate_wasm: true,
            run_tests: true,
            auto_register: self.auto_register,
            wasm_output_dir: None,
        }
    }
}
