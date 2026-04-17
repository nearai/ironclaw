use std::path::PathBuf;
use std::time::Duration;

use crate::config::helpers::{db_first_bool, db_first_or_default, optional_env};
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
    pub(crate) fn resolve(settings: &crate::settings::Settings) -> Result<Self, ConfigError> {
        let bs = &settings.builder;
        let defaults = crate::settings::BuilderSettings::default();
        Ok(Self {
            enabled: db_first_bool(bs.enabled, defaults.enabled, "BUILDER_ENABLED")?,
            build_dir: if let Some(ref dir) = bs.build_dir {
                Some(dir.clone())
            } else {
                optional_env("BUILDER_DIR")?.map(PathBuf::from)
            },
            max_iterations: db_first_or_default(
                &bs.max_iterations,
                &defaults.max_iterations,
                "BUILDER_MAX_ITERATIONS",
            )?,
            timeout_secs: db_first_or_default(
                &bs.timeout_secs,
                &defaults.timeout_secs,
                "BUILDER_TIMEOUT_SECS",
            )?,
            auto_register: db_first_bool(
                bs.auto_register,
                defaults.auto_register,
                "BUILDER_AUTO_REGISTER",
            )?,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::helpers::lock_env;
    use crate::settings::Settings;

    #[test]
    fn resolve_falls_back_to_settings() {
        let _guard = lock_env();
        let mut settings = Settings::default();
        settings.builder.max_iterations = 99;
        settings.builder.auto_register = false;

        let cfg = BuilderModeConfig::resolve(&settings).expect("resolve");
        assert_eq!(cfg.max_iterations, 99);
        assert!(!cfg.auto_register);
    }

    #[test]
    fn db_settings_override_env() {
        let _guard = lock_env();
        let mut settings = Settings::default();
        settings.builder.timeout_secs = 123;

        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe { std::env::set_var("BUILDER_TIMEOUT_SECS", "3") };
        let cfg = BuilderModeConfig::resolve(&settings).expect("resolve");
        unsafe { std::env::remove_var("BUILDER_TIMEOUT_SECS") };

        assert_eq!(cfg.timeout_secs, 123, "DB setting should win over env");
    }

    #[test]
    fn env_used_when_no_db_setting() {
        let _guard = lock_env();
        let settings = Settings::default();

        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe { std::env::set_var("BUILDER_TIMEOUT_SECS", "42") };
        let cfg = BuilderModeConfig::resolve(&settings).expect("resolve");
        unsafe { std::env::remove_var("BUILDER_TIMEOUT_SECS") };

        assert_eq!(
            cfg.timeout_secs, 42,
            "env should be used when DB has the default value"
        );
    }
}
