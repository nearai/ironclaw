use crate::config::helpers::{parse_bool_env, parse_optional_env};
use crate::error::ConfigError;

pub use ironclaw_safety::SafetyConfig;

pub(crate) fn resolve_safety_config() -> Result<SafetyConfig, ConfigError> {
    Ok(SafetyConfig {
        max_output_length: parse_optional_env("SAFETY_MAX_OUTPUT_LENGTH", 100_000)?,
        injection_check_enabled: parse_bool_env("SAFETY_INJECTION_CHECK_ENABLED", true)?,
    })
}
