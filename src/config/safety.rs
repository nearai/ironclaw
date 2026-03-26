use crate::config::helpers::{parse_bool_env, parse_optional_env};
use crate::error::ConfigError;

pub use crate::safety::SafetyConfig;

pub(crate) fn resolve_safety_config(
    settings: &crate::settings::Settings,
) -> Result<SafetyConfig, ConfigError> {
    let ss = &settings.safety;
    Ok(SafetyConfig {
        max_output_length: parse_optional_env("SAFETY_MAX_OUTPUT_LENGTH", ss.max_output_length)?,
        injection_check_enabled: parse_bool_env(
            "SAFETY_INJECTION_CHECK_ENABLED",
            ss.injection_check_enabled,
        )?,
    })
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
        settings.safety.max_output_length = 42;
        settings.safety.injection_check_enabled = false;

        let cfg = resolve_safety_config(&settings).expect("resolve");
        assert_eq!(cfg.max_output_length, 42);
        assert!(!cfg.injection_check_enabled);
    }

    #[test]
    fn env_overrides_settings() {
        let _guard = lock_env();
        let mut settings = Settings::default();
        settings.safety.max_output_length = 42;

        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe { std::env::set_var("SAFETY_MAX_OUTPUT_LENGTH", "7") };
        let cfg = resolve_safety_config(&settings).expect("resolve");
        unsafe { std::env::remove_var("SAFETY_MAX_OUTPUT_LENGTH") };

        assert_eq!(cfg.max_output_length, 7);
    }
}
