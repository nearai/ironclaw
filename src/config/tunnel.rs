use crate::config::helpers::{db_first_bool, db_first_optional_string, optional_env};
use crate::error::ConfigError;
use crate::settings::{Settings, TunnelSettings};

/// Tunnel configuration for exposing the agent to the internet.
///
/// Used by channels and tools that need public webhook endpoints.
/// The tunnel URL is shared across all channels (Telegram, Slack, etc.).
///
/// Resolution priority: DB/settings > env var > default.
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
        let defaults = TunnelSettings::default();

        // Priority: DB/settings > env > default.
        let public_url = db_first_optional_string(&settings.tunnel.public_url, "TUNNEL_URL")?;

        if let Some(ref url) = public_url
            && !url.starts_with("https://")
        {
            return Err(ConfigError::InvalidValue {
                key: "TUNNEL_URL".to_string(),
                message: "must start with https:// (webhooks require HTTPS)".to_string(),
            });
        }

        // Resolve managed tunnel provider config.
        // Priority: DB/settings > env > default (none).
        let provider_name = db_first_optional_string(&settings.tunnel.provider, "TUNNEL_PROVIDER")?
            .unwrap_or_default();

        let provider = if provider_name.is_empty() || provider_name == "none" {
            None
        } else {
            Some(crate::tunnel::TunnelProviderConfig {
                provider: provider_name.clone(),
                // Security: tunnel auth tokens are env-only (sensitive credentials).
                cloudflare: {
                    if settings.tunnel.cf_token.is_some() {
                        tracing::warn!(
                            "tunnel.cf_token is set in DB/TOML but is now env-only \
                             (TUNNEL_CF_TOKEN). Remove it from DB settings."
                        );
                    }
                    optional_env("TUNNEL_CF_TOKEN")?
                        .map(|token| crate::tunnel::CloudflareTunnelConfig { token })
                },
                tailscale: Some(crate::tunnel::TailscaleTunnelConfig {
                    funnel: db_first_bool(
                        settings.tunnel.ts_funnel,
                        defaults.ts_funnel,
                        "TUNNEL_TS_FUNNEL",
                    )?,
                    hostname: db_first_optional_string(
                        &settings.tunnel.ts_hostname,
                        "TUNNEL_TS_HOSTNAME",
                    )?,
                }),
                ngrok: {
                    let ngrok_domain = db_first_optional_string(
                        &settings.tunnel.ngrok_domain,
                        "TUNNEL_NGROK_DOMAIN",
                    )?;
                    if settings.tunnel.ngrok_token.is_some() {
                        tracing::warn!(
                            "tunnel.ngrok_token is set in DB/TOML but is now env-only \
                             (TUNNEL_NGROK_TOKEN). Remove it from DB settings."
                        );
                    }
                    optional_env("TUNNEL_NGROK_TOKEN")?.map(|auth_token| {
                        crate::tunnel::NgrokTunnelConfig {
                            auth_token,
                            domain: ngrok_domain,
                        }
                    })
                },
                custom: {
                    let health_url = db_first_optional_string(
                        &settings.tunnel.custom_health_url,
                        "TUNNEL_CUSTOM_HEALTH_URL",
                    )?;
                    let url_pattern = db_first_optional_string(
                        &settings.tunnel.custom_url_pattern,
                        "TUNNEL_CUSTOM_URL_PATTERN",
                    )?;
                    db_first_optional_string(
                        &settings.tunnel.custom_command,
                        "TUNNEL_CUSTOM_COMMAND",
                    )?
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

#[cfg(test)]
mod tests {
    use crate::config::tunnel::TunnelConfig;
    use crate::tunnel::{
        CloudflareTunnelConfig, CustomTunnelConfig, NgrokTunnelConfig, TailscaleTunnelConfig,
        TunnelProviderConfig,
    };

    // ── Default ─────────────────────────────────────────────────────

    #[test]
    fn default_is_disabled() {
        let cfg = TunnelConfig::default();
        assert!(cfg.public_url.is_none());
        assert!(cfg.provider.is_none());
        assert!(!cfg.is_enabled());
    }

    // ── is_enabled ──────────────────────────────────────────────────

    #[test]
    fn is_enabled_with_static_url() {
        let cfg = TunnelConfig {
            public_url: Some("https://tunnel.example.com".to_string()),
            provider: None,
        };
        assert!(cfg.is_enabled());
    }

    #[test]
    fn is_enabled_with_provider() {
        let cfg = TunnelConfig {
            public_url: None,
            provider: Some(TunnelProviderConfig {
                provider: "cloudflare".to_string(),
                cloudflare: Some(CloudflareTunnelConfig {
                    token: "cf-tok".to_string(),
                }),
                tailscale: None,
                ngrok: None,
                custom: None,
            }),
        };
        assert!(cfg.is_enabled());
    }

    #[test]
    fn is_enabled_with_both() {
        let cfg = TunnelConfig {
            public_url: Some("https://example.com".to_string()),
            provider: Some(TunnelProviderConfig {
                provider: "ngrok".to_string(),
                cloudflare: None,
                tailscale: None,
                ngrok: Some(NgrokTunnelConfig {
                    auth_token: "ngrok-tok".to_string(),
                    domain: None,
                }),
                custom: None,
            }),
        };
        assert!(cfg.is_enabled());
    }

    // ── webhook_url ─────────────────────────────────────────────────

    #[test]
    fn webhook_url_none_when_no_public_url() {
        let cfg = TunnelConfig::default();
        assert!(cfg.webhook_url("/hook").is_none());
    }

    #[test]
    fn webhook_url_basic() {
        let cfg = TunnelConfig {
            public_url: Some("https://abc.ngrok.io".to_string()),
            provider: None,
        };
        assert_eq!(
            cfg.webhook_url("/webhook/telegram"),
            Some("https://abc.ngrok.io/webhook/telegram".to_string())
        );
    }

    #[test]
    fn webhook_url_trims_trailing_slash_on_base() {
        let cfg = TunnelConfig {
            public_url: Some("https://abc.ngrok.io/".to_string()),
            provider: None,
        };
        assert_eq!(
            cfg.webhook_url("/hook"),
            Some("https://abc.ngrok.io/hook".to_string())
        );
    }

    #[test]
    fn webhook_url_trims_leading_slash_on_path() {
        let cfg = TunnelConfig {
            public_url: Some("https://abc.ngrok.io".to_string()),
            provider: None,
        };
        // Path without leading slash should also work
        assert_eq!(
            cfg.webhook_url("hook"),
            Some("https://abc.ngrok.io/hook".to_string())
        );
    }

    #[test]
    fn webhook_url_double_slash_normalization() {
        let cfg = TunnelConfig {
            public_url: Some("https://abc.ngrok.io/".to_string()),
            provider: None,
        };
        // Both base trailing and path leading slashes trimmed
        assert_eq!(
            cfg.webhook_url("/api/webhook"),
            Some("https://abc.ngrok.io/api/webhook".to_string())
        );
    }

    #[test]
    fn webhook_url_empty_path() {
        let cfg = TunnelConfig {
            public_url: Some("https://abc.ngrok.io".to_string()),
            provider: None,
        };
        assert_eq!(
            cfg.webhook_url(""),
            Some("https://abc.ngrok.io/".to_string())
        );
    }

    // ── TunnelProviderConfig field coverage ─────────────────────────

    #[test]
    fn provider_config_cloudflare() {
        let p = TunnelProviderConfig {
            provider: "cloudflare".to_string(),
            cloudflare: Some(CloudflareTunnelConfig {
                token: "cf-secret".to_string(),
            }),
            tailscale: None,
            ngrok: None,
            custom: None,
        };
        assert_eq!(p.provider, "cloudflare");
        assert_eq!(p.cloudflare.as_ref().unwrap().token, "cf-secret");
    }

    #[test]
    fn provider_config_tailscale() {
        let ts = TailscaleTunnelConfig {
            funnel: true,
            hostname: Some("my-host".to_string()),
        };
        assert!(ts.funnel);
        assert_eq!(ts.hostname.as_deref(), Some("my-host"));
    }

    #[test]
    fn provider_config_tailscale_defaults() {
        let ts = TailscaleTunnelConfig::default();
        assert!(!ts.funnel);
        assert!(ts.hostname.is_none());
    }

    #[test]
    fn provider_config_ngrok() {
        let ng = NgrokTunnelConfig {
            auth_token: "ng-tok".to_string(),
            domain: Some("custom.ngrok.dev".to_string()),
        };
        assert_eq!(ng.auth_token, "ng-tok");
        assert_eq!(ng.domain.as_deref(), Some("custom.ngrok.dev"));
    }

    #[test]
    fn provider_config_ngrok_defaults() {
        let ng = NgrokTunnelConfig::default();
        assert!(ng.auth_token.is_empty());
        assert!(ng.domain.is_none());
    }

    #[test]
    fn provider_config_custom() {
        let c = CustomTunnelConfig {
            start_command: "bore local {port}".to_string(),
            health_url: Some("http://localhost:8080/health".to_string()),
            url_pattern: Some("https://bore.pub".to_string()),
        };
        assert_eq!(c.start_command, "bore local {port}");
        assert!(c.health_url.is_some());
        assert!(c.url_pattern.is_some());
    }

    #[test]
    fn provider_config_custom_defaults() {
        let c = CustomTunnelConfig::default();
        assert!(c.start_command.is_empty());
        assert!(c.health_url.is_none());
        assert!(c.url_pattern.is_none());
    }

    #[test]
    fn cloudflare_config_defaults() {
        let cf = CloudflareTunnelConfig::default();
        assert!(cf.token.is_empty());
    }
}
