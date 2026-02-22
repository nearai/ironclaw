use crate::config::helpers::{parse_bool_env, parse_optional_env};
use crate::error::ConfigError;

/// Safety configuration.
#[derive(Debug, Clone)]
pub struct SafetyConfig {
    pub max_output_length: usize,
    pub injection_check_enabled: bool,
}

impl SafetyConfig {
    pub(crate) fn resolve() -> Result<Self, ConfigError> {
        Ok(Self {
            max_output_length: parse_optional_env("SAFETY_MAX_OUTPUT_LENGTH", 100_000)?,
            injection_check_enabled: parse_bool_env("SAFETY_INJECTION_CHECK_ENABLED", true)?,
        })
    }
}
