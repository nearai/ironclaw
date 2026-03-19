use crate::config::helpers::{optional_env, parse_bool_env, parse_option_env, parse_optional_env};
use crate::error::ConfigError;
use crate::settings::Settings;

/// Heartbeat configuration.
#[derive(Debug, Clone)]
pub struct HeartbeatConfig {
    /// Whether heartbeat is enabled.
    pub enabled: bool,
    /// Interval between heartbeat checks in seconds (used when fire_at is not set).
    pub interval_secs: u64,
    /// Channel to notify on heartbeat findings.
    pub notify_channel: Option<String>,
    /// User ID to notify on heartbeat findings.
    pub notify_user: Option<String>,
    /// Fixed time-of-day to fire (HH:MM, 24h). When set, interval_secs is ignored.
    pub fire_at: Option<chrono::NaiveTime>,
    /// Hour (0-23) when quiet hours start.
    pub quiet_hours_start: Option<u32>,
    /// Hour (0-23) when quiet hours end.
    pub quiet_hours_end: Option<u32>,
    /// Timezone for fire_at and quiet hours evaluation (IANA name).
    pub timezone: Option<String>,
}

impl Default for HeartbeatConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_secs: 1800, // 30 minutes
            notify_channel: None,
            notify_user: None,
            fire_at: None,
            quiet_hours_start: None,
            quiet_hours_end: None,
            timezone: None,
        }
    }
}

impl HeartbeatConfig {
    pub(crate) fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
        let fire_at_str =
            optional_env("HEARTBEAT_FIRE_AT")?.or_else(|| settings.heartbeat.fire_at.clone());
        let fire_at = fire_at_str
            .map(|s| {
                chrono::NaiveTime::parse_from_str(&s, "%H:%M").map_err(|e| {
                    ConfigError::InvalidValue {
                        key: "HEARTBEAT_FIRE_AT".to_string(),
                        message: format!("must be HH:MM (24h), e.g. '14:00': {e}"),
                    }
                })
            })
            .transpose()?;

        let interval_secs_set = optional_env("HEARTBEAT_INTERVAL_SECS")?.is_some()
            || settings.heartbeat.interval_secs != 1800;
        if fire_at.is_some() && interval_secs_set {
            tracing::warn!(
                "both heartbeat fire_at and interval_secs are configured; \
                 interval_secs is ignored when fire_at is active"
            );
        }

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
            fire_at,
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
            timezone: {
                let tz = optional_env("HEARTBEAT_TIMEZONE")?
                    .or_else(|| settings.heartbeat.timezone.clone());
                if let Some(ref tz_str) = tz
                    && crate::timezone::parse_timezone(tz_str).is_none()
                {
                    return Err(ConfigError::InvalidValue {
                        key: "HEARTBEAT_TIMEZONE".into(),
                        message: format!("invalid IANA timezone: '{tz_str}'"),
                    });
                }
                tz
            },
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

    #[test]
    fn test_heartbeat_timezone_rejects_invalid() {
        let mut settings = Settings::default();
        settings.heartbeat.timezone = Some("Fake/Zone".to_string());

        let result = HeartbeatConfig::resolve(&settings);
        assert!(result.is_err(), "invalid IANA timezone should be rejected");
    }

    #[test]
    fn test_heartbeat_timezone_accepts_valid() {
        let mut settings = Settings::default();
        settings.heartbeat.timezone = Some("America/New_York".to_string());

        let config = HeartbeatConfig::resolve(&settings).expect("resolve");
        assert_eq!(config.timezone.as_deref(), Some("America/New_York"));
    }
}
