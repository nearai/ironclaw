use crate::config::helpers::{db_first_option, parse_optional_env};
use crate::error::ConfigError;
use crate::settings::Settings;

/// Mission-related configuration resolved from environment variables.
#[derive(Debug, Clone)]
pub struct MissionsConfig {
    /// Conversation insights extraction interval (every N completed threads).
    /// Default: 5. Minimum: 1.
    pub insights_interval: u32,
}

impl Default for MissionsConfig {
    fn default() -> Self {
        Self {
            insights_interval: 5,
        }
    }
}

impl MissionsConfig {
    pub(crate) fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
        let interval = db_first_option(
            &settings.missions.insights_interval,
            "MISSION_INSIGHTS_INTERVAL",
        )?
        .unwrap_or(parse_optional_env("MISSION_INSIGHTS_INTERVAL", 5u32)?);
        let interval = interval.max(1);

        Ok(Self {
            insights_interval: interval,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::helpers::lock_env;

    fn clear_missions_env() {
        // SAFETY: Only called under ENV_MUTEX in tests.
        unsafe {
            std::env::remove_var("MISSION_INSIGHTS_INTERVAL");
        }
    }

    #[test]
    fn defaults_when_no_env() {
        let _guard = lock_env();
        clear_missions_env();

        let settings = Settings::default();
        let config = MissionsConfig::resolve(&settings).expect("should resolve");
        assert_eq!(config.insights_interval, 5);
    }

    #[test]
    fn env_override() {
        let _guard = lock_env();
        clear_missions_env();

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::set_var("MISSION_INSIGHTS_INTERVAL", "10");
        }

        let settings = Settings::default();
        let config = MissionsConfig::resolve(&settings).expect("should resolve");
        assert_eq!(config.insights_interval, 10);

        clear_missions_env();
    }

    #[test]
    fn minimum_clamped_to_one() {
        let _guard = lock_env();
        clear_missions_env();

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::set_var("MISSION_INSIGHTS_INTERVAL", "0");
        }

        let settings = Settings::default();
        let config = MissionsConfig::resolve(&settings).expect("should resolve");
        assert_eq!(config.insights_interval, 1);

        clear_missions_env();
    }

    #[test]
    fn db_settings_override_env() {
        let _guard = lock_env();
        clear_missions_env();

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::set_var("MISSION_INSIGHTS_INTERVAL", "10");
        }

        let mut settings = Settings::default();
        settings.missions.insights_interval = Some(3);

        let config = MissionsConfig::resolve(&settings).expect("should resolve");
        assert_eq!(config.insights_interval, 3);

        clear_missions_env();
    }
}
