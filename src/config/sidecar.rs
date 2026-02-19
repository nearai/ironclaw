use crate::config::helpers::{optional_env, parse_optional_env};
use crate::error::ConfigError;

/// Sidecar configuration for external Docker services.
///
/// Configures persistent sidecar containers that run alongside the main agent,
/// such as browserless for browser automation.
#[derive(Debug, Clone)]
pub struct SidecarConfig {
    /// Whether sidecar support is enabled.
    pub enabled: bool,
    /// Browserless sidecar configuration (for browser automation).
    pub browserless_enabled: bool,
    /// Port for browserless sidecar on host.
    pub browserless_port: u16,
    /// Optional auth token for browserless.
    pub browserless_token: Option<String>,
    /// Whether to keep sidecar containers running on shutdown (for debugging).
    pub keep_on_shutdown: bool,
    /// Timeout in seconds for sidecar startup.
    pub startup_timeout_secs: u64,
}

impl Default for SidecarConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            browserless_enabled: false,
            browserless_port: 9222,
            browserless_token: None,
            keep_on_shutdown: false,
            startup_timeout_secs: 90,
        }
    }
}

impl SidecarConfig {
    pub(crate) fn resolve() -> Result<Self, ConfigError> {
        let defaults = Self::default();

        let browserless_token = optional_env("BROWSERLESS_TOKEN")?.filter(|t| !t.is_empty());

        Ok(Self {
            enabled: optional_env("SIDECAR_ENABLED")?
                .map(|s| s.parse())
                .transpose()
                .map_err(|e| ConfigError::InvalidValue {
                    key: "SIDECAR_ENABLED".to_string(),
                    message: format!("must be 'true' or 'false': {e}"),
                })?
                .unwrap_or(defaults.enabled),
            browserless_enabled: optional_env("BROWSERLESS_ENABLED")?
                .map(|s| s.parse())
                .transpose()
                .map_err(|e| ConfigError::InvalidValue {
                    key: "BROWSERLESS_ENABLED".to_string(),
                    message: format!("must be 'true' or 'false': {e}"),
                })?
                .unwrap_or(defaults.browserless_enabled),
            browserless_port: parse_optional_env("BROWSERLESS_PORT", defaults.browserless_port)?,
            browserless_token,
            keep_on_shutdown: optional_env("SIDECAR_KEEP_ON_SHUTDOWN")?
                .map(|s| s.parse())
                .transpose()
                .map_err(|e| ConfigError::InvalidValue {
                    key: "SIDECAR_KEEP_ON_SHUTDOWN".to_string(),
                    message: format!("must be 'true' or 'false': {e}"),
                })?
                .unwrap_or(defaults.keep_on_shutdown),
            startup_timeout_secs: parse_optional_env(
                "SIDECAR_STARTUP_TIMEOUT_SECS",
                defaults.startup_timeout_secs,
            )?,
        })
    }

    /// Check if any sidecar is enabled.
    pub fn any_enabled(&self) -> bool {
        self.enabled || self.browserless_enabled
    }

    /// Create browserless sidecar configuration.
    pub fn to_browserless_config(&self) -> crate::sidecar::SidecarConfig {
        use std::time::Duration;

        crate::sidecar::SidecarConfig {
            name: "browserless".to_string(),
            image: "ghcr.io/browserless/chromium:latest".to_string(),
            ports: vec![(self.browserless_port.to_string(), "3000".to_string())],
            env: {
                let mut env = vec![
                    ("MAX_CONCURRENT_SESSIONS".to_string(), "4".to_string()),
                    ("MAX_QUEUE_LENGTH".to_string(), "10".to_string()),
                ];
                if let Some(ref token) = self.browserless_token {
                    env.push(("TOKEN".to_string(), token.clone()));
                }
                env
            },
            volumes: Vec::new(),
            health_check: crate::sidecar::HealthCheck::Http {
                path: "/".to_string(),
                port: 3000,
            },
            startup_timeout: Duration::from_secs(self.startup_timeout_secs),
            keep_on_shutdown: self.keep_on_shutdown,
            ..Default::default()
        }
    }
}
