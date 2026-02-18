use crate::config::helpers::optional_env;
use crate::error::ConfigError;
use crate::settings::Settings;

/// Tunnel configuration for exposing the agent to the internet.
///
/// Used by channels and tools that need public webhook endpoints.
/// The tunnel URL is shared across all channels (Telegram, Slack, etc.).
///
/// Two modes:
/// - **Static URL** (`TUNNEL_URL`): set the public URL directly (manual tunnel)
/// - **Managed provider** (`TUNNEL_PROVIDER`): lifecycle-managed tunnel process
///
/// When a managed provider is configured _and_ no static URL is set,
/// the gateway starts the tunnel on boot and populates `public_url`.
#[derive(Debug, Clone, Default)]
pub struct TunnelConfig {
    /// Public URL from tunnel provider (e.g., "https://abc123.ngrok.io").
    /// Set statically via `TUNNEL_URL` or populated at runtime by a managed tunnel.
    pub public_url: Option<String>,
    /// Provider configuration for lifecycle-managed tunnels.
    /// `None` when using a static URL or no tunnel at all.
    pub provider: Option<crate::tunnel::TunnelProviderConfig>,
}

impl TunnelConfig {
    pub(crate) fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
        let public_url = optional_env("TUNNEL_URL")?
            .or_else(|| settings.tunnel.public_url.clone().filter(|s| !s.is_empty()));

        if let Some(ref url) = public_url
            && !url.starts_with("https://")
        {
            return Err(ConfigError::InvalidValue {
                key: "TUNNEL_URL".to_string(),
                message: "must start with https:// (webhooks require HTTPS)".to_string(),
            });
        }

        // Resolve managed tunnel provider config.
        // Priority: env var > settings > default (none).
        let provider_name = optional_env("TUNNEL_PROVIDER")?
            .or_else(|| settings.tunnel.provider.clone())
            .unwrap_or_default();

        let provider = if provider_name.is_empty() || provider_name == "none" {
            None
        } else {
            Some(crate::tunnel::TunnelProviderConfig {
                provider: provider_name.clone(),
                cloudflare: optional_env("TUNNEL_CF_TOKEN")?
                    .or_else(|| settings.tunnel.cf_token.clone())
                    .map(|token| crate::tunnel::CloudflareTunnelConfig { token }),
                tailscale: Some(crate::tunnel::TailscaleTunnelConfig {
                    funnel: optional_env("TUNNEL_TS_FUNNEL")?
                        .map(|s| s == "true" || s == "1")
                        .unwrap_or(settings.tunnel.ts_funnel),
                    hostname: optional_env("TUNNEL_TS_HOSTNAME")?
                        .or_else(|| settings.tunnel.ts_hostname.clone()),
                }),
                ngrok: {
                    let ngrok_domain = optional_env("TUNNEL_NGROK_DOMAIN")?
                        .or_else(|| settings.tunnel.ngrok_domain.clone());
                    optional_env("TUNNEL_NGROK_TOKEN")?
                        .or_else(|| settings.tunnel.ngrok_token.clone())
                        .map(|auth_token| crate::tunnel::NgrokTunnelConfig {
                            auth_token,
                            domain: ngrok_domain,
                        })
                },
                custom: {
                    let health_url = optional_env("TUNNEL_CUSTOM_HEALTH_URL")?
                        .or_else(|| settings.tunnel.custom_health_url.clone());
                    let url_pattern = optional_env("TUNNEL_CUSTOM_URL_PATTERN")?
                        .or_else(|| settings.tunnel.custom_url_pattern.clone());
                    optional_env("TUNNEL_CUSTOM_COMMAND")?
                        .or_else(|| settings.tunnel.custom_command.clone())
                        .map(|start_command| crate::tunnel::CustomTunnelConfig {
                            start_command,
                            health_url,
                            url_pattern,
                        })
                },
            })
        };

        Ok(Self {
            public_url,
            provider,
        })
    }

    /// Check if a tunnel is configured (static URL or managed provider).
    pub fn is_enabled(&self) -> bool {
        self.public_url.is_some() || self.provider.is_some()
    }

    /// Get the webhook URL for a given path.
    pub fn webhook_url(&self, path: &str) -> Option<String> {
        self.public_url.as_ref().map(|base| {
            let base = base.trim_end_matches('/');
            let path = path.trim_start_matches('/');
            format!("{}/{}", base, path)
        })
    }
}
