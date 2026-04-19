use crate::config::helpers::{
    db_first_bool, db_first_option, db_first_optional_string, db_first_or_default, optional_env,
    parse_bool_env,
};
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
    /// When true, cycle through all users with routines. Controlled via
    /// HEARTBEAT_MULTI_TENANT env var; defaults to false.
    pub multi_tenant: bool,
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
            multi_tenant: false,
        }
    }
}

impl HeartbeatConfig {
    pub(crate) fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
        let defaults = crate::settings::HeartbeatSettings::default();

        // fire_at: DB > env, then parse into NaiveTime
        let fire_at_str =
            db_first_optional_string(&settings.heartbeat.fire_at, "HEARTBEAT_FIRE_AT")?;
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

        // quiet_hours: DB > env (using db_first_option for shadow warnings)
        let quiet_hours_start = db_first_option(
            &settings.heartbeat.quiet_hours_start,
            "HEARTBEAT_QUIET_START",
        )?
        .map(|h| {
            if h > 23 {
                return Err(ConfigError::InvalidValue {
                    key: "HEARTBEAT_QUIET_START".into(),
                    message: "must be 0-23".into(),
                });
            }
            Ok(h)
        })
        .transpose()?;

        let quiet_hours_end =
            db_first_option(&settings.heartbeat.quiet_hours_end, "HEARTBEAT_QUIET_END")?
                .map(|h| {
                    if h > 23 {
                        return Err(ConfigError::InvalidValue {
                            key: "HEARTBEAT_QUIET_END".into(),
                            message: "must be 0-23".into(),
                        });
                    }
                    Ok(h)
                })
                .transpose()?;

        Ok(Self {
            enabled: db_first_bool(
                settings.heartbeat.enabled,
                defaults.enabled,
                "HEARTBEAT_ENABLED",
            )?,
            interval_secs: db_first_or_default(
                &settings.heartbeat.interval_secs,
                &defaults.interval_secs,
                "HEARTBEAT_INTERVAL_SECS",
            )?,
            notify_channel: db_first_optional_string(
                &settings.heartbeat.notify_channel,
                "HEARTBEAT_NOTIFY_CHANNEL",
            )?,
            notify_user: db_first_optional_string(
                &settings.heartbeat.notify_user,
                "HEARTBEAT_NOTIFY_USER",
            )?,
            fire_at,
            quiet_hours_start,
            quiet_hours_end,
            timezone: {
                let tz =
                    db_first_optional_string(&settings.heartbeat.timezone, "HEARTBEAT_TIMEZONE")?;
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
            // Auto-detect multi-tenant mode from GATEWAY_USER_TOKENS presence,
            // or allow explicit override via HEARTBEAT_MULTI_TENANT. Stays env-only.
            multi_tenant: parse_bool_env(
                "HEARTBEAT_MULTI_TENANT",
                optional_env("GATEWAY_USER_TOKENS")?.is_some(),
            )?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::helpers::lock_env;

    #[test]
    fn test_quiet_hours_settings_have_priority() {
        // DB/settings values should take priority over env
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

    #[test]
    fn test_db_first_enabled_beats_env() {
        let _guard = lock_env();
        // SAFETY: under ENV_MUTEX
        unsafe { std::env::set_var("HEARTBEAT_ENABLED", "false") };

        let mut settings = Settings::default();
        settings.heartbeat.enabled = true; // DB says enabled

        let config = HeartbeatConfig::resolve(&settings).expect("resolve");
        assert!(config.enabled, "DB value (true) should beat env (false)");

        unsafe { std::env::remove_var("HEARTBEAT_ENABLED") };
    }

    #[test]
    fn test_db_first_interval_beats_env() {
        let _guard = lock_env();
        unsafe { std::env::set_var("HEARTBEAT_INTERVAL_SECS", "999") };

        let mut settings = Settings::default();
        settings.heartbeat.interval_secs = 600; // DB says 600

        let config = HeartbeatConfig::resolve(&settings).expect("resolve");
        assert_eq!(config.interval_secs, 600, "DB value should beat env");

        unsafe { std::env::remove_var("HEARTBEAT_INTERVAL_SECS") };
    }

    #[test]
    fn test_db_first_notify_channel_beats_env() {
        let _guard = lock_env();
        unsafe { std::env::set_var("HEARTBEAT_NOTIFY_CHANNEL", "env-channel") };

        let mut settings = Settings::default();
        settings.heartbeat.notify_channel = Some("db-channel".to_string());

        let config = HeartbeatConfig::resolve(&settings).expect("resolve");
        assert_eq!(
            config.notify_channel.as_deref(),
            Some("db-channel"),
            "DB value should beat env"
        );

        unsafe { std::env::remove_var("HEARTBEAT_NOTIFY_CHANNEL") };
    }

    #[test]
    fn test_env_fallback_when_db_at_default() {
        let _guard = lock_env();
        unsafe { std::env::set_var("HEARTBEAT_INTERVAL_SECS", "999") };

        // Settings at default => env should win
        let settings = Settings::default();

        let config = HeartbeatConfig::resolve(&settings).expect("resolve");
        assert_eq!(
            config.interval_secs, 999,
            "env should win when DB at default"
        );

        unsafe { std::env::remove_var("HEARTBEAT_INTERVAL_SECS") };
    }

    #[test]
    fn test_fire_at_db_first() {
        let _guard = lock_env();
        unsafe { std::env::set_var("HEARTBEAT_FIRE_AT", "08:00") };

        let mut settings = Settings::default();
        settings.heartbeat.fire_at = Some("14:30".to_string());

        let config = HeartbeatConfig::resolve(&settings).expect("resolve");
        assert_eq!(
            config.fire_at,
            Some(chrono::NaiveTime::from_hms_opt(14, 30, 0).unwrap()),
            "DB fire_at should beat env"
        );

        unsafe { std::env::remove_var("HEARTBEAT_FIRE_AT") };
    }

    #[test]
    fn test_timezone_db_first() {
        let _guard = lock_env();
        unsafe { std::env::set_var("HEARTBEAT_TIMEZONE", "UTC") };

        let mut settings = Settings::default();
        settings.heartbeat.timezone = Some("America/New_York".to_string());

        let config = HeartbeatConfig::resolve(&settings).expect("resolve");
        assert_eq!(
            config.timezone.as_deref(),
            Some("America/New_York"),
            "DB timezone should beat env"
        );

        unsafe { std::env::remove_var("HEARTBEAT_TIMEZONE") };
    }

    #[test]
    fn test_multi_tenant_stays_env_only() {
        let _guard = lock_env();
        unsafe { std::env::set_var("HEARTBEAT_MULTI_TENANT", "true") };

        let settings = Settings::default();
        let config = HeartbeatConfig::resolve(&settings).expect("resolve");
        assert!(config.multi_tenant, "multi_tenant should read from env");

        unsafe { std::env::remove_var("HEARTBEAT_MULTI_TENANT") };
    }
}
