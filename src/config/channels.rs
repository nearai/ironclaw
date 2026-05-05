use std::collections::HashMap;
use std::path::PathBuf;

use crate::bootstrap::ironclaw_base_dir;
use crate::channels::web::sse::{DEFAULT_BROADCAST_BUFFER, DEFAULT_MAX_CONNECTIONS};
use crate::config::helpers::{
    db_first_bool, db_first_optional_string, db_first_or_default, optional_env, parse_bool_env,
    parse_option_env, parse_optional_env,
};
use crate::error::ConfigError;
use crate::settings::{ChannelSettings, Settings};
use secrecy::SecretString;

/// Channel configurations.
#[derive(Debug, Clone)]
pub struct ChannelsConfig {
    pub cli: CliConfig,
    pub http: Option<HttpConfig>,
    pub webhook_listener: WebhookListenerConfig,
    pub gateway: Option<GatewayConfig>,
    pub signal: Option<SignalConfig>,
    pub tui: Option<TuiChannelConfig>,
    /// Directory containing WASM channel modules (default: ~/.ironclaw/channels/).
    pub wasm_channels_dir: std::path::PathBuf,
    /// Whether WASM channels are enabled.
    pub wasm_channels_enabled: bool,
    /// Channel names that the setup wizard explicitly configured for startup.
    ///
    /// This is separate from runtime `activated_channels`, which is managed by
    /// extension activation flows. Startup uses this list only as a fallback
    /// before any runtime activation state has been persisted.
    pub configured_wasm_channels: Vec<String>,
    /// Per-channel owner user IDs. When set, the channel only responds to this user.
    /// Key: channel name (e.g., "telegram"), Value: owner user ID.
    pub wasm_channel_owner_ids: HashMap<String, i64>,
}

#[derive(Debug, Clone)]
pub struct CliConfig {
    pub enabled: bool,
}

#[derive(Debug, Clone)]
pub struct TuiChannelConfig {
    pub theme: String,
    pub sidebar_visible: bool,
}

#[derive(Debug, Clone)]
pub struct HttpConfig {
    pub host: String,
    pub port: u16,
    pub webhook_secret: Option<SecretString>,
    pub user_id: String,
}

#[derive(Debug, Clone)]
pub struct WebhookListenerConfig {
    pub host: String,
    pub port: u16,
}

pub const DEFAULT_WEBHOOK_LISTENER_HOST: &str = "127.0.0.1";
pub const DEFAULT_WEBHOOK_LISTENER_PORT: u16 = 8080;

fn required_legacy_http_bind<'a>(
    legacy_http_bind: Option<&'a WebhookListenerConfig>,
    context: &str,
) -> Result<&'a WebhookListenerConfig, ConfigError> {
    legacy_http_bind
        .ok_or_else(|| ConfigError::ParseError(format!("missing legacy HTTP bind for {context}")))
}

fn validate_listener_port(key: &str, port: u16) -> Result<u16, ConfigError> {
    if port == 0 {
        return Err(ConfigError::InvalidValue {
            key: key.to_string(),
            message: "must be between 1 and 65535".to_string(),
        });
    }
    Ok(port)
}

/// Maximum allowed broadcast buffer size to prevent OOM from misconfiguration.
///
/// Memory impact: `buffer_size × max_receivers × avg_event_size`.
/// Worst case at max: 65,536 slots × 100 connections × ~200 bytes ≈ 1.3 GB.
/// The default (`DEFAULT_BROADCAST_BUFFER = 1024`) keeps worst case at ~20 MB.
const MAX_BROADCAST_BUFFER: usize = 65_536;

/// Web gateway configuration.
#[derive(Debug, Clone)]
pub struct GatewayConfig {
    pub host: String,
    pub port: u16,
    /// Bearer token for authentication. Random hex generated at startup if unset.
    pub auth_token: Option<String>,
    /// Maximum number of concurrent SSE/WebSocket connections.
    pub max_connections: u64,
    /// SSE broadcast channel buffer size. Clamped to `MAX_BROADCAST_BUFFER`.
    pub broadcast_buffer: usize,
    /// Additional user scopes for workspace reads.
    ///
    /// When set, the workspace will be able to read (search, read, list) from
    /// these additional user scopes while writes remain isolated to the
    /// authenticated user's own scope.
    /// Parsed from `WORKSPACE_READ_SCOPES` (comma-separated).
    pub workspace_read_scopes: Vec<String>,
    /// Memory layer definitions (JSON in env var, or from external config).
    pub memory_layers: Vec<crate::workspace::layer::MemoryLayer>,
    /// OIDC JWT authentication (e.g., behind AWS ALB with Okta).
    pub oidc: Option<GatewayOidcConfig>,
}

/// OIDC JWT authentication configuration for the web gateway.
///
/// When enabled, the gateway accepts signed JWTs from a configurable HTTP
/// header (e.g., `x-amzn-oidc-data` from AWS ALB). Keys are fetched from
/// a JWKS endpoint and cached for 1 hour.
#[derive(Debug, Clone)]
pub struct GatewayOidcConfig {
    /// HTTP header containing the JWT (default: `x-amzn-oidc-data`).
    pub header: String,
    /// JWKS URL for key discovery. Supports `{kid}` placeholder for
    /// ALB-style per-key PEM endpoints, and standard `/.well-known/jwks.json`.
    pub jwks_url: String,
    /// Expected `iss` claim. Validated if set.
    pub issuer: Option<String>,
    /// Expected `aud` claim. Validated if set.
    pub audience: Option<String>,
}

/// Signal channel configuration (signal-cli daemon HTTP/JSON-RPC).
#[derive(Debug, Clone)]
pub struct SignalConfig {
    /// Base URL of the signal-cli daemon HTTP endpoint (e.g. `http://127.0.0.1:8080`).
    pub http_url: String,
    /// Signal account identifier (E.164 phone number, e.g. `+1234567890`).
    pub account: String,
    /// Users allowed to interact with the bot in DMs.
    ///
    /// Each entry is one of:
    /// - `*` — allow everyone
    /// - E.164 phone number (e.g. `+1234567890`)
    /// - bare UUID (e.g. `a1b2c3d4-e5f6-7890-abcd-ef1234567890`)
    /// - `uuid:<id>` prefix form (e.g. `uuid:a1b2c3d4-e5f6-7890-abcd-ef1234567890`)
    ///
    /// An empty list denies all senders (secure by default).
    pub allow_from: Vec<String>,
    /// Groups allowed to interact with the bot.
    ///
    /// - Empty list — deny all group messages (DMs only, secure by default).
    /// - `*` — allow all groups.
    /// - Specific group IDs — allow only those groups.
    pub allow_from_groups: Vec<String>,
    /// DM policy: "open", "allowlist", or "pairing". Default: "pairing".
    ///
    /// - "open" — allow all DM senders (ignores allow_from for DMs)
    /// - "allowlist" — only allow senders in allow_from list
    /// - "pairing" — allowlist + send pairing reply to unknown users
    pub dm_policy: String,
    /// Group policy: "allowlist", "open", or "disabled". Default: "allowlist".
    ///
    /// - "disabled" — deny all group messages
    /// - "allowlist" — check allow_from_groups and group_allow_from
    /// - "open" — accept all group messages (respects allow_from_groups for group ID)
    pub group_policy: String,
    /// Allow list for group message senders. If empty, inherits from allow_from.
    pub group_allow_from: Vec<String>,
    /// Skip messages that contain only attachments (no text).
    pub ignore_attachments: bool,
    /// Skip story messages.
    pub ignore_stories: bool,
}

impl ChannelsConfig {
    pub(crate) fn resolve(settings: &Settings, owner_id: &str) -> Result<Self, ConfigError> {
        let cs = &settings.channels;
        let defaults = ChannelSettings::default();

        let legacy_http_host_from_env = optional_env("HTTP_HOST")?;
        let legacy_http_port_from_env = optional_env("HTTP_PORT")?;
        let legacy_http_enable_by_env =
            legacy_http_host_from_env.is_some() || legacy_http_port_from_env.is_some();
        let legacy_http_bind_configured =
            legacy_http_enable_by_env || cs.http_host.is_some() || cs.http_port.is_some();
        let explicit_http_enabled = if let Some(enabled) = cs.http_enabled {
            if optional_env("HTTP_ENABLED")?.is_some() {
                tracing::warn!("HTTP_ENABLED from settings is overriding the environment value.");
            }
            Some(enabled)
        } else if optional_env("HTTP_ENABLED")?.is_some() {
            Some(parse_bool_env("HTTP_ENABLED", false)?)
        } else {
            None
        };
        let webhook_host = optional_env("WEBHOOK_HOST")?;
        let webhook_port = parse_option_env("WEBHOOK_PORT")?
            .map(|port| validate_listener_port("WEBHOOK_PORT", port))
            .transpose()?;
        let http_enabled = explicit_http_enabled == Some(true)
            || (explicit_http_enabled.is_none() && legacy_http_enable_by_env);
        let webhook_host_uses_legacy_env_fallback =
            webhook_host.is_none() && legacy_http_bind_configured;
        let webhook_port_uses_legacy_env_fallback =
            webhook_port.is_none() && legacy_http_bind_configured;
        let legacy_http_bind = (http_enabled
            || webhook_host_uses_legacy_env_fallback
            || webhook_port_uses_legacy_env_fallback)
            .then(|| resolve_legacy_http_bind(cs))
            .transpose()?;
        let http = if explicit_http_enabled == Some(true)
            || (explicit_http_enabled.is_none() && legacy_http_enable_by_env)
        {
            let legacy_http_bind = required_legacy_http_bind(
                legacy_http_bind.as_ref(),
                "named HTTP channel enablement",
            )?;
            if explicit_http_enabled.is_none() && legacy_http_enable_by_env {
                tracing::warn!(
                    "HTTP_HOST/HTTP_PORT enabled the named HTTP webhook channel without \
                     HTTP_ENABLED=true. This compatibility path is deprecated; move the listener \
                     bind to WEBHOOK_HOST/WEBHOOK_PORT and set HTTP_ENABLED=true explicitly to \
                     keep the named HTTP webhook channel enabled."
                );
            }
            Some(HttpConfig {
                host: legacy_http_bind.host.clone(),
                port: legacy_http_bind.port,
                webhook_secret: optional_env("HTTP_WEBHOOK_SECRET")?.map(SecretString::from),
                user_id: owner_id.to_string(),
            })
        } else {
            None
        };
        if explicit_http_enabled == Some(false)
            && legacy_http_bind_configured
            && (webhook_host.is_none() || webhook_port.is_none())
        {
            tracing::warn!(
                "WEBHOOK_HOST and/or WEBHOOK_PORT are unset; using legacy HTTP bind settings as a \
                 compatibility fallback for the unified webhook listener. Migrate to \
                 WEBHOOK_HOST/WEBHOOK_PORT, then either set HTTP_ENABLED=true to keep the named \
                 HTTP webhook channel or HTTP_ENABLED=false to disable it explicitly."
            );
        }
        let webhook_listener_host = match (webhook_host, webhook_host_uses_legacy_env_fallback) {
            (Some(host), _) => host,
            (None, true) => required_legacy_http_bind(
                legacy_http_bind.as_ref(),
                "webhook listener host fallback",
            )?
            .host
            .clone(),
            (None, false) => DEFAULT_WEBHOOK_LISTENER_HOST.to_string(),
        };
        let webhook_listener_port = match (webhook_port, webhook_port_uses_legacy_env_fallback) {
            (Some(port), _) => port,
            (None, true) => {
                required_legacy_http_bind(
                    legacy_http_bind.as_ref(),
                    "webhook listener port fallback",
                )?
                .port
            }
            (None, false) => DEFAULT_WEBHOOK_LISTENER_PORT,
        };
        let webhook_listener = WebhookListenerConfig {
            host: webhook_listener_host,
            port: webhook_listener_port,
        };

        let gateway_enabled = db_first_bool(
            cs.gateway_enabled,
            defaults.gateway_enabled,
            "GATEWAY_ENABLED",
        )?;
        let gateway = if gateway_enabled {
            let memory_layers: Vec<crate::workspace::layer::MemoryLayer> =
                match optional_env("MEMORY_LAYERS")? {
                    Some(json_str) => {
                        serde_json::from_str(&json_str).map_err(|e| ConfigError::InvalidValue {
                            key: "MEMORY_LAYERS".to_string(),
                            message: format!("must be valid JSON array of layer objects: {e}"),
                        })?
                    }
                    None => crate::workspace::layer::MemoryLayer::default_for_user(owner_id),
                };

            // Validate layer names and scopes
            for layer in &memory_layers {
                if layer.name.trim().is_empty() {
                    return Err(ConfigError::InvalidValue {
                        key: "MEMORY_LAYERS".to_string(),
                        message: "layer name must not be empty".to_string(),
                    });
                }
                if layer.name.len() > 64 {
                    return Err(ConfigError::InvalidValue {
                        key: "MEMORY_LAYERS".to_string(),
                        message: format!("layer name '{}' exceeds 64 characters", layer.name),
                    });
                }
                if !layer
                    .name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                {
                    return Err(ConfigError::InvalidValue {
                        key: "MEMORY_LAYERS".to_string(),
                        message: format!(
                            "layer name '{}' contains invalid characters \
                             (allowed: a-z, A-Z, 0-9, _, -)",
                            layer.name
                        ),
                    });
                }
                if layer.scope.trim().is_empty() {
                    return Err(ConfigError::InvalidValue {
                        key: "MEMORY_LAYERS".to_string(),
                        message: format!("layer '{}' has an empty scope", layer.name),
                    });
                }
            }

            // Check for duplicate layer names
            {
                let mut seen = std::collections::HashSet::new();
                for layer in &memory_layers {
                    if !seen.insert(&layer.name) {
                        return Err(ConfigError::InvalidValue {
                            key: "MEMORY_LAYERS".to_string(),
                            message: format!("duplicate layer name '{}'", layer.name),
                        });
                    }
                }
            }

            let workspace_read_scopes: Vec<String> = optional_env("WORKSPACE_READ_SCOPES")?
                .map(|s| {
                    s.split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default();

            for scope in &workspace_read_scopes {
                if scope.len() > 128 {
                    return Err(ConfigError::InvalidValue {
                        key: "WORKSPACE_READ_SCOPES".to_string(),
                        message: format!(
                            "scope '{}...' exceeds 128 characters",
                            &scope[..crate::util::floor_char_boundary(scope, 32)]
                        ),
                    });
                }
            }
            let oidc_enabled = parse_bool_env("GATEWAY_OIDC_ENABLED", false)?;
            let oidc = if oidc_enabled {
                let jwks_url =
                    optional_env("GATEWAY_OIDC_JWKS_URL")?.ok_or(ConfigError::InvalidValue {
                        key: "GATEWAY_OIDC_JWKS_URL".to_string(),
                        message: "required when GATEWAY_OIDC_ENABLED=true".to_string(),
                    })?;
                Some(GatewayOidcConfig {
                    header: optional_env("GATEWAY_OIDC_HEADER")?
                        .unwrap_or_else(|| "x-amzn-oidc-data".to_string()),
                    jwks_url,
                    issuer: optional_env("GATEWAY_OIDC_ISSUER")?,
                    audience: optional_env("GATEWAY_OIDC_AUDIENCE")?,
                })
            } else {
                None
            };

            Some(GatewayConfig {
                host: db_first_optional_string(&cs.gateway_host, "GATEWAY_HOST")?
                    .unwrap_or_else(|| "127.0.0.1".to_string()),
                port: {
                    // defaults.gateway_port is None, so any Some(..) is an explicit DB override.
                    if let Some(ref db_port) = cs.gateway_port {
                        db_first_or_default(db_port, &DEFAULT_GATEWAY_PORT, "GATEWAY_PORT")?
                    } else {
                        parse_optional_env("GATEWAY_PORT", DEFAULT_GATEWAY_PORT)?
                    }
                },
                // Security: auth token is env-only — never read from DB settings.
                auth_token: {
                    if cs.gateway_auth_token.is_some() {
                        tracing::warn!(
                            "gateway_auth_token is set in DB/TOML but is now env-only \
                             (GATEWAY_AUTH_TOKEN). Remove it from DB/TOML settings."
                        );
                    }
                    optional_env("GATEWAY_AUTH_TOKEN")?
                },
                max_connections: {
                    let max =
                        parse_optional_env("GATEWAY_MAX_CONNECTIONS", DEFAULT_MAX_CONNECTIONS)?;
                    if max == 0 {
                        return Err(ConfigError::InvalidValue {
                            key: "GATEWAY_MAX_CONNECTIONS".to_string(),
                            message: "must be greater than 0".to_string(),
                        });
                    }
                    max
                },
                broadcast_buffer: {
                    let buf: usize =
                        parse_optional_env("SSE_BROADCAST_BUFFER", DEFAULT_BROADCAST_BUFFER)?;
                    if buf == 0 {
                        return Err(ConfigError::InvalidValue {
                            key: "SSE_BROADCAST_BUFFER".to_string(),
                            message: "must be greater than 0".to_string(),
                        });
                    }
                    buf.min(MAX_BROADCAST_BUFFER)
                },
                workspace_read_scopes,
                memory_layers,
                oidc,
            })
        } else {
            None
        };

        let signal_enabled =
            db_first_bool(cs.signal_enabled, defaults.signal_enabled, "SIGNAL_ENABLED")?;
        let signal_url = db_first_optional_string(&cs.signal_http_url, "SIGNAL_HTTP_URL")?;
        let signal = if signal_enabled || signal_url.is_some() {
            let http_url = signal_url.ok_or(ConfigError::InvalidValue {
                key: "SIGNAL_HTTP_URL".to_string(),
                message: "SIGNAL_HTTP_URL is required when signal_enabled is set in DB/TOML \
                         or SIGNAL_ENABLED env var is true"
                    .to_string(),
            })?;
            let account = db_first_optional_string(&cs.signal_account, "SIGNAL_ACCOUNT")?.ok_or(
                ConfigError::InvalidValue {
                    key: "SIGNAL_ACCOUNT".to_string(),
                    message: "SIGNAL_ACCOUNT is required when SIGNAL_HTTP_URL is set".to_string(),
                },
            )?;
            let allow_from =
                match db_first_optional_string(&cs.signal_allow_from, "SIGNAL_ALLOW_FROM")? {
                    None => vec![account.clone()],
                    Some(s) => s
                        .split(',')
                        .map(|e| e.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect(),
                };
            let dm_policy = db_first_optional_string(&cs.signal_dm_policy, "SIGNAL_DM_POLICY")?
                .unwrap_or_else(|| "pairing".to_string());
            let group_policy =
                db_first_optional_string(&cs.signal_group_policy, "SIGNAL_GROUP_POLICY")?
                    .unwrap_or_else(|| "allowlist".to_string());
            Some(SignalConfig {
                http_url,
                account,
                allow_from,
                allow_from_groups: db_first_optional_string(
                    &cs.signal_allow_from_groups,
                    "SIGNAL_ALLOW_FROM_GROUPS",
                )?
                .map(|s| {
                    s.split(',')
                        .map(|e| e.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default(),
                dm_policy,
                group_policy,
                group_allow_from: db_first_optional_string(
                    &cs.signal_group_allow_from,
                    "SIGNAL_GROUP_ALLOW_FROM",
                )?
                .map(|s| {
                    s.split(',')
                        .map(|e| e.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default(),
                ignore_attachments: optional_env("SIGNAL_IGNORE_ATTACHMENTS")?
                    .map(|s| s.to_lowercase() == "true" || s == "1")
                    .unwrap_or(false),
                ignore_stories: optional_env("SIGNAL_IGNORE_STORIES")?
                    .map(|s| s.to_lowercase() == "true" || s == "1")
                    .unwrap_or(true),
            })
        } else {
            None
        };

        let cli_enabled = db_first_bool(cs.cli_enabled, defaults.cli_enabled, "CLI_ENABLED")?;
        let cli_mode = db_first_optional_string(&cs.cli_mode, "CLI_MODE")?
            .unwrap_or_else(|| "tui".to_string());
        let tui = if cli_mode.eq_ignore_ascii_case("tui") {
            Some(TuiChannelConfig {
                theme: optional_env("TUI_THEME")?.unwrap_or_else(|| "dark".to_string()),
                sidebar_visible: parse_bool_env("TUI_SIDEBAR", true)?,
            })
        } else {
            None
        };

        Ok(Self {
            cli: CliConfig {
                enabled: cli_enabled,
            },
            http,
            webhook_listener,
            gateway,
            signal,
            tui,
            wasm_channels_dir: {
                // DB-first: use settings if explicitly set, else env, else default.
                // defaults.wasm_channels_dir is None, so any Some(..) is an explicit DB override.
                if let Some(ref db_dir) = cs.wasm_channels_dir {
                    db_dir.clone()
                } else {
                    optional_env("WASM_CHANNELS_DIR")?
                        .map(PathBuf::from)
                        .unwrap_or_else(default_channels_dir)
                }
            },
            wasm_channels_enabled: db_first_bool(
                cs.wasm_channels_enabled,
                defaults.wasm_channels_enabled,
                "WASM_CHANNELS_ENABLED",
            )?,
            configured_wasm_channels: cs.wasm_channels.clone(),
            wasm_channel_owner_ids: {
                let mut ids = cs.wasm_channel_owner_ids.clone();
                // Backwards compat: TELEGRAM_OWNER_ID env var
                if let Some(id_str) = optional_env("TELEGRAM_OWNER_ID")? {
                    let id: i64 = id_str.parse().map_err(|e: std::num::ParseIntError| {
                        ConfigError::InvalidValue {
                            key: "TELEGRAM_OWNER_ID".to_string(),
                            message: format!("must be an integer: {e}"),
                        }
                    })?;
                    ids.insert("telegram".to_string(), id);
                }
                ids
            },
        })
    }
}

/// Default gateway port — used both in `resolve()` and as the fallback in
/// other modules that need to construct a gateway URL.
pub const DEFAULT_GATEWAY_PORT: u16 = 3000;

fn resolve_legacy_http_bind(cs: &ChannelSettings) -> Result<WebhookListenerConfig, ConfigError> {
    Ok(WebhookListenerConfig {
        host: db_first_optional_string(&cs.http_host, "HTTP_HOST")?
            .unwrap_or_else(|| DEFAULT_WEBHOOK_LISTENER_HOST.to_string()),
        port: {
            if let Some(ref db_port) = cs.http_port {
                validate_listener_port(
                    "HTTP_PORT",
                    db_first_or_default(db_port, &DEFAULT_WEBHOOK_LISTENER_PORT, "HTTP_PORT")?,
                )?
            } else {
                validate_listener_port(
                    "HTTP_PORT",
                    parse_optional_env("HTTP_PORT", DEFAULT_WEBHOOK_LISTENER_PORT)?,
                )?
            }
        },
    })
}

/// Get the default channels directory (~/.ironclaw/channels/).
fn default_channels_dir() -> PathBuf {
    ironclaw_base_dir().join("channels")
}

#[cfg(test)]
mod tests {
    use crate::config::channels::*;
    use crate::config::helpers::lock_env;
    use crate::error::ConfigError;
    use crate::settings::Settings;

    #[test]
    fn cli_config_fields() {
        let cfg = CliConfig { enabled: true };
        assert!(cfg.enabled);

        let disabled = CliConfig { enabled: false };
        assert!(!disabled.enabled);
    }

    #[test]
    fn http_config_fields() {
        let cfg = HttpConfig {
            host: "127.0.0.1".to_string(),
            port: 8080,
            webhook_secret: None,
            user_id: "http".to_string(),
        };
        assert_eq!(cfg.host, "127.0.0.1");
        assert_eq!(cfg.port, 8080);
        assert!(cfg.webhook_secret.is_none());
        assert_eq!(cfg.user_id, "http");
    }

    #[test]
    fn http_config_with_secret() {
        let cfg = HttpConfig {
            host: "127.0.0.1".to_string(),
            port: 9090,
            webhook_secret: Some(secrecy::SecretString::from("s3cret".to_string())),
            user_id: "webhook-bot".to_string(),
        };
        assert!(cfg.webhook_secret.is_some());
        assert_eq!(cfg.port, 9090);
    }

    #[test]
    fn webhook_listener_config_fields() {
        let cfg = WebhookListenerConfig {
            host: "127.0.0.1".to_string(),
            port: 8080,
        };
        assert_eq!(cfg.host, "127.0.0.1");
        assert_eq!(cfg.port, 8080);
    }

    #[test]
    fn gateway_config_fields() {
        let cfg = GatewayConfig {
            host: "127.0.0.1".to_string(),
            port: 3000,
            auth_token: Some("tok-abc".to_string()),
            max_connections: 100,
            broadcast_buffer: DEFAULT_BROADCAST_BUFFER,
            workspace_read_scopes: vec![],
            memory_layers: vec![],
            oidc: None,
        };
        assert_eq!(cfg.host, "127.0.0.1");
        assert_eq!(cfg.port, 3000);
        assert_eq!(cfg.auth_token.as_deref(), Some("tok-abc"));
    }

    #[test]
    fn gateway_config_no_auth_token() {
        let cfg = GatewayConfig {
            host: "0.0.0.0".to_string(),
            port: 3001,
            auth_token: None,
            max_connections: 100,
            broadcast_buffer: DEFAULT_BROADCAST_BUFFER,
            workspace_read_scopes: vec![],
            memory_layers: vec![],
            oidc: None,
        };
        assert!(cfg.auth_token.is_none());
    }

    #[test]
    fn broadcast_buffer_defaults_and_clamps() {
        let _guard = lock_env();
        let settings = Settings::default();

        // SAFETY: under ENV_MUTEX
        unsafe {
            std::env::set_var("GATEWAY_ENABLED", "true");
            std::env::remove_var("SSE_BROADCAST_BUFFER");
        }
        let cfg = ChannelsConfig::resolve(&settings, "owner").expect("resolve");
        let gw = cfg.gateway.expect("gateway");
        assert_eq!(gw.broadcast_buffer, DEFAULT_BROADCAST_BUFFER);

        // Custom value
        unsafe { std::env::set_var("SSE_BROADCAST_BUFFER", "2048") };
        let cfg = ChannelsConfig::resolve(&settings, "owner").expect("resolve");
        let gw = cfg.gateway.expect("gateway");
        assert_eq!(gw.broadcast_buffer, 2048);

        // Clamped to MAX_BROADCAST_BUFFER
        unsafe { std::env::set_var("SSE_BROADCAST_BUFFER", "999999") };
        let cfg = ChannelsConfig::resolve(&settings, "owner").expect("resolve");
        let gw = cfg.gateway.expect("gateway");
        assert_eq!(gw.broadcast_buffer, MAX_BROADCAST_BUFFER);

        // Zero is rejected
        unsafe { std::env::set_var("SSE_BROADCAST_BUFFER", "0") };
        let err = ChannelsConfig::resolve(&settings, "owner");
        assert!(err.is_err());

        // SAFETY: under ENV_MUTEX
        unsafe {
            std::env::remove_var("GATEWAY_ENABLED");
            std::env::remove_var("SSE_BROADCAST_BUFFER");
        }
    }

    #[test]
    fn signal_config_fields_and_defaults() {
        let cfg = SignalConfig {
            http_url: "http://127.0.0.1:8080".to_string(),
            account: "+1234567890".to_string(),
            allow_from: vec!["+1234567890".to_string()],
            allow_from_groups: vec![],
            dm_policy: "pairing".to_string(),
            group_policy: "allowlist".to_string(),
            group_allow_from: vec![],
            ignore_attachments: false,
            ignore_stories: true,
        };
        assert_eq!(cfg.http_url, "http://127.0.0.1:8080");
        assert_eq!(cfg.account, "+1234567890");
        assert_eq!(cfg.allow_from, vec!["+1234567890"]);
        assert!(cfg.allow_from_groups.is_empty());
        assert_eq!(cfg.dm_policy, "pairing");
        assert_eq!(cfg.group_policy, "allowlist");
        assert!(cfg.group_allow_from.is_empty());
        assert!(!cfg.ignore_attachments);
        assert!(cfg.ignore_stories);
    }

    #[test]
    fn signal_config_open_policies() {
        let cfg = SignalConfig {
            http_url: "http://localhost:7583".to_string(),
            account: "+0000000000".to_string(),
            allow_from: vec!["*".to_string()],
            allow_from_groups: vec!["*".to_string()],
            dm_policy: "open".to_string(),
            group_policy: "open".to_string(),
            group_allow_from: vec![],
            ignore_attachments: true,
            ignore_stories: false,
        };
        assert_eq!(cfg.allow_from, vec!["*"]);
        assert_eq!(cfg.allow_from_groups, vec!["*"]);
        assert_eq!(cfg.dm_policy, "open");
        assert_eq!(cfg.group_policy, "open");
        assert!(cfg.ignore_attachments);
        assert!(!cfg.ignore_stories);
    }

    #[test]
    fn channels_config_fields() {
        let cfg = ChannelsConfig {
            cli: CliConfig { enabled: true },
            http: None,
            webhook_listener: WebhookListenerConfig {
                host: DEFAULT_WEBHOOK_LISTENER_HOST.to_string(),
                port: DEFAULT_WEBHOOK_LISTENER_PORT,
            },
            gateway: None,
            signal: None,
            tui: None,
            wasm_channels_dir: PathBuf::from("/tmp/channels"),
            wasm_channels_enabled: true,
            configured_wasm_channels: Vec::new(),
            wasm_channel_owner_ids: HashMap::new(),
        };
        assert!(cfg.cli.enabled);
        assert!(cfg.http.is_none());
        assert_eq!(cfg.webhook_listener.host, DEFAULT_WEBHOOK_LISTENER_HOST);
        assert_eq!(cfg.webhook_listener.port, DEFAULT_WEBHOOK_LISTENER_PORT);
        assert!(cfg.gateway.is_none());
        assert!(cfg.signal.is_none());
        assert_eq!(cfg.wasm_channels_dir, PathBuf::from("/tmp/channels"));
        assert!(cfg.wasm_channels_enabled);
        assert!(cfg.wasm_channel_owner_ids.is_empty());
    }

    #[test]
    fn channels_config_with_owner_ids() {
        let mut ids = HashMap::new();
        ids.insert("telegram".to_string(), 12345_i64);
        ids.insert("slack".to_string(), 67890_i64);

        let cfg = ChannelsConfig {
            cli: CliConfig { enabled: false },
            http: None,
            webhook_listener: WebhookListenerConfig {
                host: DEFAULT_WEBHOOK_LISTENER_HOST.to_string(),
                port: DEFAULT_WEBHOOK_LISTENER_PORT,
            },
            gateway: None,
            signal: None,
            tui: None,
            wasm_channels_dir: PathBuf::from("/opt/channels"),
            wasm_channels_enabled: false,
            configured_wasm_channels: vec!["telegram".to_string()],
            wasm_channel_owner_ids: ids,
        };
        assert_eq!(cfg.wasm_channel_owner_ids.get("telegram"), Some(&12345));
        assert_eq!(cfg.wasm_channel_owner_ids.get("slack"), Some(&67890));
        assert!(!cfg.wasm_channels_enabled);
        assert_eq!(cfg.configured_wasm_channels, vec!["telegram"]);
    }

    #[test]
    fn default_channels_dir_ends_with_channels() {
        let dir = default_channels_dir();
        assert!(
            dir.ends_with("channels"),
            "expected path ending in 'channels', got: {dir:?}"
        );
    }

    #[test]
    fn resolve_populates_dedicated_webhook_listener_from_webhook_env_vars() {
        let _guard = lock_env();
        let settings = Settings::default();

        // SAFETY: under ENV_MUTEX
        unsafe {
            std::env::remove_var("HTTP_ENABLED");
            std::env::remove_var("HTTP_HOST");
            std::env::remove_var("HTTP_PORT");
            std::env::set_var("WEBHOOK_HOST", "127.0.0.1");
            std::env::set_var("WEBHOOK_PORT", "9091");
        }

        let cfg = ChannelsConfig::resolve(&settings, "owner-scope").expect("resolve");

        assert_eq!(cfg.webhook_listener.host, "127.0.0.1");
        assert_eq!(cfg.webhook_listener.port, 9091);

        // SAFETY: under ENV_MUTEX
        unsafe {
            std::env::remove_var("HTTP_ENABLED");
            std::env::remove_var("HTTP_HOST");
            std::env::remove_var("HTTP_PORT");
            std::env::remove_var("WEBHOOK_HOST");
            std::env::remove_var("WEBHOOK_PORT");
        }
    }

    #[test]
    fn resolve_defaults_dedicated_webhook_listener_to_localhost_8080() {
        let _guard = lock_env();
        let settings = Settings::default();

        // SAFETY: under ENV_MUTEX
        unsafe {
            std::env::remove_var("HTTP_ENABLED");
            std::env::remove_var("HTTP_HOST");
            std::env::remove_var("HTTP_PORT");
            std::env::remove_var("WEBHOOK_HOST");
            std::env::remove_var("WEBHOOK_PORT");
        }

        let cfg = ChannelsConfig::resolve(&settings, "owner-scope").expect("resolve");

        assert_eq!(cfg.webhook_listener.host, "127.0.0.1");
        assert_eq!(cfg.webhook_listener.port, 8080);
    }

    #[test]
    fn resolve_does_not_enable_named_http_channel_from_legacy_host_port_when_http_enabled_is_false()
    {
        let _guard = lock_env();
        let settings = Settings::default();

        // SAFETY: under ENV_MUTEX
        unsafe {
            std::env::set_var("HTTP_ENABLED", "false");
            std::env::set_var("HTTP_HOST", "0.0.0.0");
            std::env::set_var("HTTP_PORT", "8089");
            std::env::set_var("WEBHOOK_HOST", "127.0.0.1");
            std::env::set_var("WEBHOOK_PORT", "9091");
        }

        let cfg = ChannelsConfig::resolve(&settings, "owner-scope").expect("resolve");

        assert!(
            cfg.http.is_none(),
            "legacy HTTP_HOST/HTTP_PORT must not re-enable the named HTTP channel when HTTP_ENABLED=false"
        );
        assert_eq!(cfg.webhook_listener.host, "127.0.0.1");
        assert_eq!(cfg.webhook_listener.port, 9091);

        // SAFETY: under ENV_MUTEX
        unsafe {
            std::env::remove_var("HTTP_ENABLED");
            std::env::remove_var("HTTP_HOST");
            std::env::remove_var("HTTP_PORT");
            std::env::remove_var("WEBHOOK_HOST");
            std::env::remove_var("WEBHOOK_PORT");
        }
    }

    #[test]
    fn resolve_preserves_named_http_channel_when_http_enabled_is_true() {
        let _guard = lock_env();
        let settings = Settings::default();

        // SAFETY: under ENV_MUTEX
        unsafe {
            std::env::set_var("HTTP_ENABLED", "true");
            std::env::set_var("HTTP_HOST", "0.0.0.0");
            std::env::set_var("HTTP_PORT", "8089");
            std::env::remove_var("WEBHOOK_HOST");
            std::env::remove_var("WEBHOOK_PORT");
        }

        let cfg = ChannelsConfig::resolve(&settings, "owner-scope").expect("resolve");
        let http = cfg.http.expect("http config");

        assert_eq!(http.host, "0.0.0.0");
        assert_eq!(http.port, 8089);
        assert_eq!(http.user_id, "owner-scope");
        assert_eq!(cfg.webhook_listener.host, "0.0.0.0");
        assert_eq!(cfg.webhook_listener.port, 8089);

        // SAFETY: under ENV_MUTEX
        unsafe {
            std::env::remove_var("HTTP_ENABLED");
            std::env::remove_var("HTTP_HOST");
            std::env::remove_var("HTTP_PORT");
        }
    }

    fn clear_webhook_channel_env() {
        // SAFETY: callers hold ENV_MUTEX via lock_env()
        unsafe {
            std::env::remove_var("HTTP_ENABLED");
            std::env::remove_var("HTTP_HOST");
            std::env::remove_var("HTTP_PORT");
            std::env::remove_var("WEBHOOK_HOST");
            std::env::remove_var("WEBHOOK_PORT");
        }
    }

    fn set_webhook_channel_env(entries: &[(&str, &str)]) {
        clear_webhook_channel_env();
        for (key, value) in entries {
            // SAFETY: callers hold ENV_MUTEX via lock_env()
            unsafe { std::env::set_var(key, value) };
        }
    }

    #[test]
    fn resolve_webhook_listener_compatibility_matrix() {
        struct Case<'a> {
            name: &'a str,
            env: &'a [(&'a str, &'a str)],
            expected_http: Option<(&'a str, u16)>,
            expected_webhook: (&'a str, u16),
        }

        let _guard = lock_env();
        let settings = Settings::default();
        let cases = [
            Case {
                name: "no_config_defaults_to_local_listener",
                env: &[],
                expected_http: None,
                expected_webhook: (DEFAULT_WEBHOOK_LISTENER_HOST, DEFAULT_WEBHOOK_LISTENER_PORT),
            },
            Case {
                name: "legacy_http_env_unset_http_enabled_keeps_compatibility_path",
                env: &[("HTTP_HOST", "0.0.0.0"), ("HTTP_PORT", "8089")],
                expected_http: Some(("0.0.0.0", 8089)),
                expected_webhook: ("0.0.0.0", 8089),
            },
            Case {
                name: "explicit_http_enabled_true_preserves_compatibility_path",
                env: &[
                    ("HTTP_ENABLED", "true"),
                    ("HTTP_HOST", "0.0.0.0"),
                    ("HTTP_PORT", "8089"),
                ],
                expected_http: Some(("0.0.0.0", 8089)),
                expected_webhook: ("0.0.0.0", 8089),
            },
            Case {
                name: "explicit_http_enabled_false_without_webhook_or_legacy_bind_uses_default_listener",
                env: &[("HTTP_ENABLED", "false")],
                expected_http: None,
                expected_webhook: (DEFAULT_WEBHOOK_LISTENER_HOST, DEFAULT_WEBHOOK_LISTENER_PORT),
            },
            Case {
                name: "explicit_webhook_env_overrides_http_compatibility_bind",
                env: &[
                    ("HTTP_ENABLED", "true"),
                    ("HTTP_HOST", "0.0.0.0"),
                    ("HTTP_PORT", "8089"),
                    ("WEBHOOK_HOST", "127.0.0.9"),
                    ("WEBHOOK_PORT", "9091"),
                ],
                expected_http: Some(("0.0.0.0", 8089)),
                expected_webhook: ("127.0.0.9", 9091),
            },
            Case {
                name: "partial_migration_webhook_host_overrides_legacy_host_only",
                env: &[
                    ("HTTP_ENABLED", "false"),
                    ("HTTP_HOST", "0.0.0.0"),
                    ("HTTP_PORT", "8089"),
                    ("WEBHOOK_HOST", "127.0.0.9"),
                ],
                expected_http: None,
                expected_webhook: ("127.0.0.9", 8089),
            },
            Case {
                name: "partial_migration_webhook_port_overrides_legacy_port_only",
                env: &[
                    ("HTTP_ENABLED", "false"),
                    ("HTTP_HOST", "0.0.0.0"),
                    ("HTTP_PORT", "8089"),
                    ("WEBHOOK_PORT", "9091"),
                ],
                expected_http: None,
                expected_webhook: ("0.0.0.0", 9091),
            },
        ];

        for case in cases {
            set_webhook_channel_env(case.env);
            let cfg = ChannelsConfig::resolve(&settings, "owner-scope")
                .unwrap_or_else(|err| panic!("case {} should resolve: {err}", case.name));

            match (cfg.http.as_ref(), case.expected_http) {
                (Some(http), Some((expected_host, expected_port))) => {
                    assert_eq!(http.host, expected_host, "case {}", case.name);
                    assert_eq!(http.port, expected_port, "case {}", case.name);
                }
                (None, None) => {}
                (Some(http), None) => panic!(
                    "case {} expected no HTTP config, got {}:{}",
                    case.name, http.host, http.port
                ),
                (None, Some((expected_host, expected_port))) => panic!(
                    "case {} expected HTTP config {}:{}, got none",
                    case.name, expected_host, expected_port
                ),
            }

            assert_eq!(
                cfg.webhook_listener.host, case.expected_webhook.0,
                "case {}",
                case.name
            );
            assert_eq!(
                cfg.webhook_listener.port, case.expected_webhook.1,
                "case {}",
                case.name
            );
        }

        clear_webhook_channel_env();
    }

    #[test]
    fn resolve_invalid_legacy_http_port_for_webhook_listener_returns_config_error() {
        let _guard = lock_env();
        let settings = Settings::default();

        set_webhook_channel_env(&[
            ("HTTP_ENABLED", "false"),
            ("HTTP_HOST", "0.0.0.0"),
            ("HTTP_PORT", "not-a-port"),
        ]);

        let err = ChannelsConfig::resolve(&settings, "owner-scope")
            .expect_err("invalid legacy HTTP_PORT should surface as a config error");

        match err {
            ConfigError::InvalidValue { key, .. } => {
                assert_eq!(key, "HTTP_PORT");
            }
            other => panic!("expected ConfigError::InvalidValue for HTTP_PORT, got {other}"),
        }

        clear_webhook_channel_env();
    }

    #[test]
    fn resolve_rejects_zero_listener_ports_for_webhook_and_legacy_fallback_sources() {
        let _guard = lock_env();

        let webhook_port_err = {
            let settings = Settings::default();
            set_webhook_channel_env(&[("WEBHOOK_PORT", "0")]);
            ChannelsConfig::resolve(&settings, "owner-scope")
                .expect_err("WEBHOOK_PORT=0 should be rejected")
        };
        match webhook_port_err {
            ConfigError::InvalidValue { key, .. } => assert_eq!(key, "WEBHOOK_PORT"),
            other => panic!("expected invalid WEBHOOK_PORT error, got {other}"),
        }

        let legacy_env_port_err = {
            let settings = Settings::default();
            set_webhook_channel_env(&[("HTTP_HOST", "0.0.0.0"), ("HTTP_PORT", "0")]);
            ChannelsConfig::resolve(&settings, "owner-scope")
                .expect_err("HTTP_PORT=0 should be rejected for legacy fallback")
        };
        match legacy_env_port_err {
            ConfigError::InvalidValue { key, .. } => assert_eq!(key, "HTTP_PORT"),
            other => panic!("expected invalid HTTP_PORT error, got {other}"),
        }

        let persisted_legacy_port_err = {
            clear_webhook_channel_env();
            let mut settings = Settings::default();
            settings.channels.http_enabled = Some(false);
            settings.channels.http_host = Some("127.0.0.9".to_string());
            settings.channels.http_port = Some(0);
            ChannelsConfig::resolve(&settings, "owner-scope")
                .expect_err("persisted HTTP_PORT=0 should be rejected for legacy fallback")
        };
        match persisted_legacy_port_err {
            ConfigError::InvalidValue { key, .. } => assert_eq!(key, "HTTP_PORT"),
            other => panic!("expected invalid persisted HTTP_PORT error, got {other}"),
        }

        clear_webhook_channel_env();
    }

    #[test]
    fn resolve_persisted_legacy_http_bind_keeps_http_disabled_but_sets_webhook_listener() {
        let _guard = lock_env();
        clear_webhook_channel_env();

        let mut settings = Settings::default();
        settings.channels.http_enabled = Some(false);
        settings.channels.http_host = Some("127.0.0.9".to_string());
        settings.channels.http_port = Some(9091);

        let cfg = ChannelsConfig::resolve(&settings, "owner-scope").expect("resolve");

        assert!(
            cfg.http.is_none(),
            "persisted legacy HTTP settings must not re-enable the named HTTP channel when http_enabled=false"
        );
        assert_eq!(cfg.webhook_listener.host, "127.0.0.9");
        assert_eq!(cfg.webhook_listener.port, 9091);
    }

    #[test]
    fn resolve_persisted_http_enabled_false_after_db_round_trip_does_not_allow_legacy_env_reenablement()
     {
        let _guard = lock_env();

        let mut persisted = Settings::default();
        persisted.channels.http_enabled = Some(false);
        let persisted = Settings::from_db_map(&persisted.to_db_map());

        set_webhook_channel_env(&[("HTTP_HOST", "0.0.0.0"), ("HTTP_PORT", "8089")]);

        let cfg = ChannelsConfig::resolve(&persisted, "owner-scope").expect("resolve");

        assert_eq!(persisted.channels.http_enabled, Some(false));
        assert!(
            cfg.http.is_none(),
            "persisted HTTP disablement must survive DB round-trip and block legacy env re-enable"
        );
        assert_eq!(cfg.webhook_listener.host, "0.0.0.0");
        assert_eq!(cfg.webhook_listener.port, 8089);

        clear_webhook_channel_env();
    }

    #[test]
    fn resolve_uses_settings_channel_values_with_owner_scope_user_ids() {
        let _guard = lock_env();
        let mut settings = Settings::default();
        settings.channels.http_enabled = Some(true);
        settings.channels.http_host = Some("127.0.0.2".to_string());
        settings.channels.http_port = Some(8181);
        settings.channels.gateway_enabled = true;
        settings.channels.gateway_host = Some("127.0.0.3".to_string());
        settings.channels.gateway_port = Some(9191);
        // auth_token is env-only (security), set via env var
        // SAFETY: under ENV_MUTEX
        unsafe { std::env::set_var("GATEWAY_AUTH_TOKEN", "tok") };
        settings.channels.signal_http_url = Some("http://127.0.0.1:8080".to_string());
        settings.channels.signal_account = Some("+15551234567".to_string());
        settings.channels.signal_allow_from = Some("+15551234567,+15557654321".to_string());
        settings.channels.wasm_channels_dir = Some(PathBuf::from("/tmp/settings-channels"));
        settings.channels.wasm_channels_enabled = false;
        settings.channels.wasm_channels = vec!["telegram".to_string(), "discord".to_string()];

        let cfg = ChannelsConfig::resolve(&settings, "owner-scope").expect("resolve");

        let http = cfg.http.expect("http config");
        assert_eq!(http.host, "127.0.0.2");
        assert_eq!(http.port, 8181);
        assert_eq!(http.user_id, "owner-scope");

        let gateway = cfg.gateway.expect("gateway config");
        assert_eq!(gateway.host, "127.0.0.3");
        assert_eq!(gateway.port, 9191);
        assert_eq!(gateway.auth_token.as_deref(), Some("tok"));

        let signal = cfg.signal.expect("signal config");
        assert_eq!(signal.account, "+15551234567");
        assert_eq!(signal.allow_from, vec!["+15551234567", "+15557654321"]);

        assert_eq!(
            cfg.wasm_channels_dir,
            PathBuf::from("/tmp/settings-channels")
        );
        assert!(!cfg.wasm_channels_enabled);
        assert_eq!(
            cfg.configured_wasm_channels,
            vec!["telegram".to_string(), "discord".to_string()]
        );

        // SAFETY: under ENV_MUTEX
        unsafe { std::env::remove_var("GATEWAY_AUTH_TOKEN") };
    }

    #[test]
    fn resolve_enables_tui_mode_from_env() {
        let _guard = lock_env();
        let settings = Settings::default();

        // SAFETY: under ENV_MUTEX
        unsafe {
            std::env::set_var("CLI_MODE", "tui");
            std::env::set_var("TUI_THEME", "light");
            std::env::set_var("TUI_SIDEBAR", "false");
        }

        let cfg = ChannelsConfig::resolve(&settings, "owner-scope").expect("resolve");
        let tui = cfg.tui.expect("tui config");
        assert_eq!(tui.theme, "light");
        assert!(!tui.sidebar_visible);

        // SAFETY: under ENV_MUTEX
        unsafe {
            std::env::remove_var("CLI_MODE");
            std::env::remove_var("TUI_THEME");
            std::env::remove_var("TUI_SIDEBAR");
        }
    }
}
