use crate::config::helpers::{optional_env, parse_bool_env, parse_optional_env};
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
}

impl Default for HeartbeatConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_secs: 1800, // 30 minutes
            notify_channel: None,
            notify_user: None,
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
        })
    }
}
