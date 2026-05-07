use std::time::Duration;

use crate::config::helpers::{db_first_bool, db_first_or_default};
use crate::error::ConfigError;
use crate::tools::builtin::TirithConfig;

pub use ironclaw_safety::SafetyConfig;

pub(crate) fn resolve_safety_config(
    settings: &crate::settings::Settings,
) -> Result<SafetyConfig, ConfigError> {
    let ss = &settings.safety;
    let defaults = crate::settings::SafetySettings::default();
    Ok(SafetyConfig {
        max_output_length: db_first_or_default(
            &ss.max_output_length,
            &defaults.max_output_length,
            "SAFETY_MAX_OUTPUT_LENGTH",
        )?,
        injection_check_enabled: db_first_bool(
            ss.injection_check_enabled,
            defaults.injection_check_enabled,
            "SAFETY_INJECTION_CHECK_ENABLED",
        )?,
    })
}

/// Resolve `TirithConfig` using the same DB-first → env → default precedence
/// as the rest of the safety subsystem. See [`crate::tools::builtin::TirithConfig`].
pub(crate) fn resolve_tirith_config(
    settings: &crate::settings::Settings,
) -> Result<TirithConfig, ConfigError> {
    let ss = &settings.safety;
    let defaults = crate::settings::SafetySettings::default();
    let timeout_ms = db_first_or_default(
        &ss.tirith_timeout_ms,
        &defaults.tirith_timeout_ms,
        "SAFETY_TIRITH_TIMEOUT_MS",
    )?;
    Ok(TirithConfig {
        enabled: db_first_bool(
            ss.tirith_enabled,
            defaults.tirith_enabled,
            "SAFETY_TIRITH_ENABLED",
        )?,
        bin: db_first_or_default(&ss.tirith_bin, &defaults.tirith_bin, "SAFETY_TIRITH_BIN")?,
        timeout: Duration::from_millis(timeout_ms),
        fail_open: db_first_bool(
            ss.tirith_fail_open,
            defaults.tirith_fail_open,
            "SAFETY_TIRITH_FAIL_OPEN",
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
    fn db_settings_override_env() {
        let _guard = lock_env();
        let mut settings = Settings::default();
        // Non-default value simulates an explicit DB/TOML setting
        settings.safety.max_output_length = 42;

        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe { std::env::set_var("SAFETY_MAX_OUTPUT_LENGTH", "7") };
        let cfg = resolve_safety_config(&settings).expect("resolve");
        unsafe { std::env::remove_var("SAFETY_MAX_OUTPUT_LENGTH") };

        // DB value (42) wins over env value (7)
        assert_eq!(cfg.max_output_length, 42);
    }

    #[test]
    fn env_used_when_no_db_setting() {
        let _guard = lock_env();
        // Settings left at defaults — no explicit DB/TOML override
        let settings = Settings::default();

        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe { std::env::set_var("SAFETY_MAX_OUTPUT_LENGTH", "7") };
        unsafe { std::env::set_var("SAFETY_INJECTION_CHECK_ENABLED", "false") };
        let cfg = resolve_safety_config(&settings).expect("resolve");
        unsafe { std::env::remove_var("SAFETY_MAX_OUTPUT_LENGTH") };
        unsafe { std::env::remove_var("SAFETY_INJECTION_CHECK_ENABLED") };

        // Env values win when settings are at their defaults
        assert_eq!(cfg.max_output_length, 7);
        assert!(!cfg.injection_check_enabled);
    }
}
