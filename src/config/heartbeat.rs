use crate::config::helpers::{optional_env, parse_bool_env, parse_option_env, parse_optional_env};
use crate::error::ConfigError;
use crate::settings::Settings;

/// Heartbeat configuration.
#[derive(Debug, Clone)]
pub struct HeartbeatConfig {
    /// Whether heartbeat is enabled.
    pub enabled: bool,
    /// Interval between heartbeat checks in seconds.
    pub interval_secs: u64,
    /// Channel to notify on heartbeat findings.
    pub notify_channel: Option<String>,
    /// User ID to notify on heartbeat findings.
    pub notify_user: Option<String>,
    /// Hour (0-23) when quiet hours start.
    pub quiet_hours_start: Option<u32>,
    /// Hour (0-23) when quiet hours end.
    pub quiet_hours_end: Option<u32>,
}

impl Default for HeartbeatConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_secs: 1800, // 30 minutes
            notify_channel: None,
            notify_user: None,
            quiet_hours_start: None,
            quiet_hours_end: None,
        }
    }
}

impl HeartbeatConfig {
    pub(crate) fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
        Ok(Self {
            enabled: parse_bool_env("HEARTBEAT_ENABLED", settings.heartbeat.enabled)?,
            interval_secs: parse_optional_env(
                "HEARTBEAT_INTERVAL_SECS",
                settings.heartbeat.interval_secs,
            )?,
            notify_channel: optional_env("HEARTBEAT_NOTIFY_CHANNEL")?
                .or_else(|| settings.heartbeat.notify_channel.clone()),
            notify_user: optional_env("HEARTBEAT_NOTIFY_USER")?
                .or_else(|| settings.heartbeat.notify_user.clone()),
            quiet_hours_start: parse_option_env::<u32>("HEARTBEAT_QUIET_START")?
                .or(settings.heartbeat.quiet_hours_start)
                .map(|h| {
                    if h > 23 {
                        return Err(ConfigError::InvalidValue {
                            key: "HEARTBEAT_QUIET_START".into(),
                            message: "must be 0-23".into(),
                        });
                    }
                    Ok(h)
                })
                .transpose()?,
            quiet_hours_end: parse_option_env::<u32>("HEARTBEAT_QUIET_END")?
                .or(settings.heartbeat.quiet_hours_end)
                .map(|h| {
                    if h > 23 {
                        return Err(ConfigError::InvalidValue {
                            key: "HEARTBEAT_QUIET_END".into(),
                            message: "must be 0-23".into(),
                        });
                    }
                    Ok(h)
                })
                .transpose()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quiet_hours_settings_fallback() {
        // When env vars are not set, settings values should be used
        let mut settings = Settings::default();
        settings.heartbeat.quiet_hours_start = Some(22);
        settings.heartbeat.quiet_hours_end = Some(6);

        let config = HeartbeatConfig::resolve(&settings).expect("resolve");
        assert_eq!(config.quiet_hours_start, Some(22));
        assert_eq!(config.quiet_hours_end, Some(6));
    }

    #[test]
    fn test_quiet_hours_rejects_invalid_hour() {
        let mut settings = Settings::default();
        settings.heartbeat.quiet_hours_start = Some(24);

        let result = HeartbeatConfig::resolve(&settings);
        assert!(result.is_err());
    }

    #[test]
    fn test_quiet_hours_accepts_boundary_values() {
        let mut settings = Settings::default();
        settings.heartbeat.quiet_hours_start = Some(0);
        settings.heartbeat.quiet_hours_end = Some(23);

        let config = HeartbeatConfig::resolve(&settings).expect("resolve");
        assert_eq!(config.quiet_hours_start, Some(0));
        assert_eq!(config.quiet_hours_end, Some(23));
    }
}
