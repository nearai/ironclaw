use std::collections::HashMap;
use std::path::PathBuf;

use secrecy::SecretString;

use crate::bootstrap::ironclaw_base_dir;
use crate::config::helpers::{optional_env, parse_bool_env, parse_optional_env};
use crate::error::ConfigError;
use crate::settings::Settings;

/// Channel configurations.
#[derive(Debug, Clone)]
pub struct ChannelsConfig {
    pub cli: CliConfig,
    pub http: Option<HttpConfig>,
    pub gateway: Option<GatewayConfig>,
    pub signal: Option<SignalConfig>,
    /// Directory containing WASM channel modules (default: ~/.ironclaw/channels/).
    pub wasm_channels_dir: std::path::PathBuf,
    /// Whether WASM channels are enabled.
    pub wasm_channels_enabled: bool,
    /// Per-channel owner user IDs. When set, the channel only responds to this user.
    /// Key: channel name (e.g., "telegram"), Value: owner user ID.
    pub wasm_channel_owner_ids: HashMap<String, i64>,
}

#[derive(Debug, Clone)]
pub struct CliConfig {
    pub enabled: bool,
}

#[derive(Debug, Clone)]
pub struct HttpConfig {
    pub host: String,
    pub port: u16,
    pub webhook_secret: Option<SecretString>,
    pub user_id: String,
}

/// Web gateway configuration.
#[derive(Debug, Clone)]
pub struct GatewayConfig {
    pub host: String,
    pub port: u16,
    /// Bearer token for authentication. Random hex generated at startup if unset.
    pub auth_token: Option<String>,
    pub user_id: String,
    /// Memory layer definitions (JSON in env var, or from external config).
    pub memory_layers: Vec<crate::workspace::layer::MemoryLayer>,
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
    /// Resolve channels config following `env > settings > default` for every field.
    pub(crate) fn resolve(settings: &Settings, tunnel_enabled: bool) -> Result<Self, ConfigError> {
        let cs = &settings.channels;

        // --- HTTP webhook ---
        // HTTP is enabled when env vars are set OR settings has it enabled.
        let http_enabled_by_env =
            optional_env("HTTP_PORT")?.is_some() || optional_env("HTTP_HOST")?.is_some();
        // When a tunnel is configured, default to loopback since external
        // traffic arrives through the tunnel. Without a tunnel the webhook
        // server needs to accept connections from the network directly.
        let default_host = if tunnel_enabled {
            "127.0.0.1"
        } else {
            "0.0.0.0"
        };
        let http = if http_enabled_by_env || cs.http_enabled {
            Some(HttpConfig {
                host: optional_env("HTTP_HOST")?
                    .or_else(|| cs.http_host.clone())
                    .unwrap_or_else(|| default_host.to_string()),
                port: parse_optional_env("HTTP_PORT", cs.http_port.unwrap_or(8080))?,
                webhook_secret: optional_env("HTTP_WEBHOOK_SECRET")?.map(SecretString::from),
                user_id: optional_env("HTTP_USER_ID")?.unwrap_or_else(|| "http".to_string()),
            })
        } else {
            None
        };

        // --- Web gateway ---
        let gateway_enabled = parse_bool_env("GATEWAY_ENABLED", cs.gateway_enabled)?;
        let gateway = if gateway_enabled {
            let user_id = optional_env("GATEWAY_USER_ID")?
                .or_else(|| cs.gateway_user_id.clone())
                .unwrap_or_else(|| "default".to_string());

            let memory_layers = match optional_env("MEMORY_LAYERS")? {
                Some(json_str) => {
                    serde_json::from_str(&json_str).map_err(|e| ConfigError::InvalidValue {
                        key: "MEMORY_LAYERS".to_string(),
                        message: format!("must be valid JSON array of layer objects: {e}"),
                    })?
                }
                None => crate::workspace::layer::MemoryLayer::default_for_user(&user_id),
            };

            Some(GatewayConfig {
                host: optional_env("GATEWAY_HOST")?
                    .or_else(|| cs.gateway_host.clone())
                    .unwrap_or_else(|| "127.0.0.1".to_string()),
                port: parse_optional_env(
                    "GATEWAY_PORT",
                    cs.gateway_port.unwrap_or(DEFAULT_GATEWAY_PORT),
                )?,
                auth_token: optional_env("GATEWAY_AUTH_TOKEN")?
                    .or_else(|| cs.gateway_auth_token.clone()),
                user_id,
                memory_layers,
            })
        } else {
            None
        };

        // --- Signal ---
        let signal_url = optional_env("SIGNAL_HTTP_URL")?.or_else(|| cs.signal_http_url.clone());
        let signal = if let Some(http_url) = signal_url {
            let account = optional_env("SIGNAL_ACCOUNT")?
                .or_else(|| cs.signal_account.clone())
                .ok_or(ConfigError::InvalidValue {
                    key: "SIGNAL_ACCOUNT".to_string(),
                    message: "SIGNAL_ACCOUNT is required when Signal is enabled".to_string(),
                })?;
            let allow_from_str =
                optional_env("SIGNAL_ALLOW_FROM")?.or_else(|| cs.signal_allow_from.clone());
            let allow_from = match allow_from_str {
                None => vec![account.clone()],
                Some(s) => s
                    .split(',')
                    .map(|e| e.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect(),
            };
            let dm_policy = optional_env("SIGNAL_DM_POLICY")?
                .or_else(|| cs.signal_dm_policy.clone())
                .unwrap_or_else(|| "pairing".to_string());
            let group_policy = optional_env("SIGNAL_GROUP_POLICY")?
                .or_else(|| cs.signal_group_policy.clone())
                .unwrap_or_else(|| "allowlist".to_string());
            Some(SignalConfig {
                http_url,
                account,
                allow_from,
                allow_from_groups: optional_env("SIGNAL_ALLOW_FROM_GROUPS")?
                    .or_else(|| cs.signal_allow_from_groups.clone())
                    .map(|s| {
                        s.split(',')
                            .map(|e| e.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect()
                    })
                    .unwrap_or_default(),
                dm_policy,
                group_policy,
                group_allow_from: optional_env("SIGNAL_GROUP_ALLOW_FROM")?
                    .or_else(|| cs.signal_group_allow_from.clone())
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

        // --- CLI ---
        let cli_enabled = parse_bool_env("CLI_ENABLED", cs.cli_enabled)?;

        // --- WASM channels ---
        let wasm_channels_dir = optional_env("WASM_CHANNELS_DIR")?
            .map(PathBuf::from)
            .or_else(|| cs.wasm_channels_dir.clone())
            .unwrap_or_else(default_channels_dir);

        let wasm_channels_enabled =
            parse_bool_env("WASM_CHANNELS_ENABLED", cs.wasm_channels_enabled)?;

        Ok(Self {
            cli: CliConfig {
                enabled: cli_enabled,
            },
            http,
            gateway,
            signal,
            wasm_channels_dir,
            wasm_channels_enabled,
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

/// Get the default channels directory (~/.ironclaw/channels/).
fn default_channels_dir() -> PathBuf {
    ironclaw_base_dir().join("channels")
}

#[cfg(test)]
mod tests {
    use crate::config::channels::*;

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
            host: "0.0.0.0".to_string(),
            port: 8080,
            webhook_secret: None,
            user_id: "http".to_string(),
        };
        assert_eq!(cfg.host, "0.0.0.0");
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
    fn gateway_config_fields() {
        let cfg = GatewayConfig {
            host: "127.0.0.1".to_string(),
            port: 3000,
            auth_token: Some("tok-abc".to_string()),
            user_id: "default".to_string(),
            memory_layers: vec![],
        };
        assert_eq!(cfg.host, "127.0.0.1");
        assert_eq!(cfg.port, 3000);
        assert_eq!(cfg.auth_token.as_deref(), Some("tok-abc"));
        assert_eq!(cfg.user_id, "default");
    }

    #[test]
    fn gateway_config_no_auth_token() {
        let cfg = GatewayConfig {
            host: "0.0.0.0".to_string(),
            port: 3001,
            auth_token: None,
            user_id: "anon".to_string(),
            memory_layers: vec![],
        };
        assert!(cfg.auth_token.is_none());
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
            gateway: None,
            signal: None,
            wasm_channels_dir: PathBuf::from("/tmp/channels"),
            wasm_channels_enabled: true,
            wasm_channel_owner_ids: HashMap::new(),
        };
        assert!(cfg.cli.enabled);
        assert!(cfg.http.is_none());
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
            gateway: None,
            signal: None,
            wasm_channels_dir: PathBuf::from("/opt/channels"),
            wasm_channels_enabled: false,
            wasm_channel_owner_ids: ids,
        };
        assert_eq!(cfg.wasm_channel_owner_ids.get("telegram"), Some(&12345));
        assert_eq!(cfg.wasm_channel_owner_ids.get("slack"), Some(&67890));
        assert!(!cfg.wasm_channels_enabled);
    }

    /// When a tunnel is active and HTTP_HOST is not explicitly set, the
    /// webhook server should default to loopback to avoid unnecessary exposure.
    #[test]
    fn http_host_defaults_to_loopback_with_tunnel() {
        // Set HTTP_PORT to trigger HttpConfig creation, but leave HTTP_HOST unset
        // so the default kicks in.
        unsafe {
            std::env::set_var("HTTP_PORT", "9999");
            std::env::remove_var("HTTP_HOST");
        }
        let settings = crate::settings::Settings::default();
        let cfg = ChannelsConfig::resolve(&settings, true).unwrap();
        unsafe {
            std::env::remove_var("HTTP_PORT");
        }
        let http = cfg.http.expect("HttpConfig should be present");
        assert_eq!(
            http.host, "127.0.0.1",
            "tunnel active should default to loopback"
        );
        assert_eq!(http.port, 9999);
    }

    /// Without a tunnel, the webhook server defaults to 0.0.0.0 so external
    /// services can reach it directly.
    #[test]
    fn http_host_defaults_to_all_interfaces_without_tunnel() {
        unsafe {
            std::env::set_var("HTTP_PORT", "9998");
            std::env::remove_var("HTTP_HOST");
        }
        let settings = crate::settings::Settings::default();
        let cfg = ChannelsConfig::resolve(&settings, false).unwrap();
        unsafe {
            std::env::remove_var("HTTP_PORT");
        }
        let http = cfg.http.expect("HttpConfig should be present");
        assert_eq!(
            http.host, "0.0.0.0",
            "no tunnel should default to all interfaces"
        );
    }

    /// An explicit HTTP_HOST always wins regardless of tunnel state.
    #[test]
    fn explicit_http_host_overrides_tunnel_default() {
        unsafe {
            std::env::set_var("HTTP_PORT", "9997");
            std::env::set_var("HTTP_HOST", "192.168.1.50");
        }
        let settings = crate::settings::Settings::default();
        let cfg = ChannelsConfig::resolve(&settings, true).unwrap();
        unsafe {
            std::env::remove_var("HTTP_PORT");
            std::env::remove_var("HTTP_HOST");
        }
        let http = cfg.http.expect("HttpConfig should be present");
        assert_eq!(
            http.host, "192.168.1.50",
            "explicit host should override tunnel default"
        );
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
    fn default_gateway_port_constant() {
        assert_eq!(DEFAULT_GATEWAY_PORT, 3000);
    }

    /// With default settings and no env vars, gateway should use defaults.
    #[test]
    fn resolve_gateway_defaults_from_settings() {
        let _lock = crate::config::helpers::ENV_MUTEX.lock();
        // Clear env vars that would interfere
        unsafe {
            std::env::remove_var("GATEWAY_ENABLED");
            std::env::remove_var("GATEWAY_HOST");
            std::env::remove_var("GATEWAY_PORT");
            std::env::remove_var("GATEWAY_AUTH_TOKEN");
            std::env::remove_var("GATEWAY_USER_ID");
            std::env::remove_var("HTTP_PORT");
            std::env::remove_var("HTTP_HOST");
            std::env::remove_var("SIGNAL_HTTP_URL");
            std::env::remove_var("CLI_ENABLED");
            std::env::remove_var("WASM_CHANNELS_DIR");
            std::env::remove_var("WASM_CHANNELS_ENABLED");
            std::env::remove_var("TELEGRAM_OWNER_ID");
        }

        let settings = crate::settings::Settings::default();
        let cfg = ChannelsConfig::resolve(&settings, false).unwrap();

        let gw = cfg.gateway.expect("gateway should be enabled by default");
        assert_eq!(gw.host, "127.0.0.1");
        assert_eq!(gw.port, DEFAULT_GATEWAY_PORT);
        assert!(gw.auth_token.is_none());
        assert_eq!(gw.user_id, "default");
    }

    /// Settings values should be used when no env vars are set.
    #[test]
    fn resolve_gateway_from_settings() {
        let _lock = crate::config::helpers::ENV_MUTEX.lock();
        unsafe {
            std::env::remove_var("GATEWAY_ENABLED");
            std::env::remove_var("GATEWAY_HOST");
            std::env::remove_var("GATEWAY_PORT");
            std::env::remove_var("GATEWAY_AUTH_TOKEN");
            std::env::remove_var("GATEWAY_USER_ID");
            std::env::remove_var("HTTP_PORT");
            std::env::remove_var("HTTP_HOST");
            std::env::remove_var("SIGNAL_HTTP_URL");
            std::env::remove_var("CLI_ENABLED");
            std::env::remove_var("WASM_CHANNELS_DIR");
            std::env::remove_var("WASM_CHANNELS_ENABLED");
            std::env::remove_var("TELEGRAM_OWNER_ID");
        }

        let mut settings = crate::settings::Settings::default();
        settings.channels.gateway_port = Some(4000);
        settings.channels.gateway_host = Some("0.0.0.0".to_string());
        settings.channels.gateway_auth_token = Some("db-token-123".to_string());
        settings.channels.gateway_user_id = Some("myuser".to_string());

        let cfg = ChannelsConfig::resolve(&settings, false).unwrap();
        let gw = cfg.gateway.expect("gateway should be enabled");
        assert_eq!(gw.port, 4000);
        assert_eq!(gw.host, "0.0.0.0");
        assert_eq!(gw.auth_token.as_deref(), Some("db-token-123"));
        assert_eq!(gw.user_id, "myuser");
    }

    /// Env vars should override settings values.
    #[test]
    fn resolve_env_overrides_settings() {
        let _lock = crate::config::helpers::ENV_MUTEX.lock();
        unsafe {
            std::env::set_var("GATEWAY_PORT", "5000");
            std::env::set_var("GATEWAY_HOST", "10.0.0.1");
            std::env::set_var("GATEWAY_AUTH_TOKEN", "env-token");
            std::env::remove_var("GATEWAY_ENABLED");
            std::env::remove_var("GATEWAY_USER_ID");
            std::env::remove_var("HTTP_PORT");
            std::env::remove_var("HTTP_HOST");
            std::env::remove_var("SIGNAL_HTTP_URL");
            std::env::remove_var("CLI_ENABLED");
            std::env::remove_var("WASM_CHANNELS_DIR");
            std::env::remove_var("WASM_CHANNELS_ENABLED");
            std::env::remove_var("TELEGRAM_OWNER_ID");
        }

        let mut settings = crate::settings::Settings::default();
        settings.channels.gateway_port = Some(4000);
        settings.channels.gateway_host = Some("0.0.0.0".to_string());
        settings.channels.gateway_auth_token = Some("db-token".to_string());

        let cfg = ChannelsConfig::resolve(&settings, false).unwrap();
        let gw = cfg.gateway.expect("gateway should be enabled");
        assert_eq!(gw.port, 5000, "env should override settings");
        assert_eq!(gw.host, "10.0.0.1", "env should override settings");
        assert_eq!(
            gw.auth_token.as_deref(),
            Some("env-token"),
            "env should override settings"
        );

        // Cleanup
        unsafe {
            std::env::remove_var("GATEWAY_PORT");
            std::env::remove_var("GATEWAY_HOST");
            std::env::remove_var("GATEWAY_AUTH_TOKEN");
        }
    }

    /// CLI enabled should fall back to settings.
    #[test]
    fn resolve_cli_enabled_from_settings() {
        let _lock = crate::config::helpers::ENV_MUTEX.lock();
        unsafe {
            std::env::remove_var("CLI_ENABLED");
            std::env::remove_var("GATEWAY_ENABLED");
            std::env::remove_var("GATEWAY_HOST");
            std::env::remove_var("GATEWAY_PORT");
            std::env::remove_var("GATEWAY_AUTH_TOKEN");
            std::env::remove_var("GATEWAY_USER_ID");
            std::env::remove_var("HTTP_PORT");
            std::env::remove_var("HTTP_HOST");
            std::env::remove_var("SIGNAL_HTTP_URL");
            std::env::remove_var("WASM_CHANNELS_DIR");
            std::env::remove_var("WASM_CHANNELS_ENABLED");
            std::env::remove_var("TELEGRAM_OWNER_ID");
        }

        let mut settings = crate::settings::Settings::default();
        settings.channels.cli_enabled = false;

        let cfg = ChannelsConfig::resolve(&settings, false).unwrap();
        assert!(!cfg.cli.enabled, "settings should disable CLI");
    }

    /// HTTP channel should activate when settings has it enabled.
    #[test]
    fn resolve_http_from_settings() {
        let _lock = crate::config::helpers::ENV_MUTEX.lock();
        unsafe {
            std::env::remove_var("HTTP_PORT");
            std::env::remove_var("HTTP_HOST");
            std::env::remove_var("HTTP_WEBHOOK_SECRET");
            std::env::remove_var("HTTP_USER_ID");
            std::env::remove_var("GATEWAY_ENABLED");
            std::env::remove_var("GATEWAY_HOST");
            std::env::remove_var("GATEWAY_PORT");
            std::env::remove_var("GATEWAY_AUTH_TOKEN");
            std::env::remove_var("GATEWAY_USER_ID");
            std::env::remove_var("SIGNAL_HTTP_URL");
            std::env::remove_var("CLI_ENABLED");
            std::env::remove_var("WASM_CHANNELS_DIR");
            std::env::remove_var("WASM_CHANNELS_ENABLED");
            std::env::remove_var("TELEGRAM_OWNER_ID");
        }

        let mut settings = crate::settings::Settings::default();
        settings.channels.http_enabled = true;
        settings.channels.http_port = Some(9090);
        settings.channels.http_host = Some("10.0.0.1".to_string());

        let cfg = ChannelsConfig::resolve(&settings, false).unwrap();
        let http = cfg.http.expect("HTTP should be enabled from settings");
        assert_eq!(http.port, 9090);
        assert_eq!(http.host, "10.0.0.1");
    }

    /// Settings round-trip through DB map for new gateway fields.
    #[test]
    fn settings_gateway_fields_db_roundtrip() {
        let mut settings = crate::settings::Settings::default();
        settings.channels.gateway_port = Some(4000);
        settings.channels.gateway_host = Some("0.0.0.0".to_string());
        settings.channels.gateway_auth_token = Some("tok-abc".to_string());
        settings.channels.gateway_user_id = Some("myuser".to_string());
        settings.channels.cli_enabled = false;

        let map = settings.to_db_map();
        let restored = crate::settings::Settings::from_db_map(&map);

        assert_eq!(restored.channels.gateway_port, Some(4000));
        assert_eq!(restored.channels.gateway_host.as_deref(), Some("0.0.0.0"));
        assert_eq!(
            restored.channels.gateway_auth_token.as_deref(),
            Some("tok-abc")
        );
        assert_eq!(restored.channels.gateway_user_id.as_deref(), Some("myuser"));
        assert!(!restored.channels.cli_enabled);
    }

    /// Invalid boolean env values must produce errors, not silently degrade.
    #[test]
    fn resolve_rejects_invalid_bool_env() {
        let _lock = crate::config::helpers::ENV_MUTEX.lock();
        let settings = crate::settings::Settings::default();

        // GATEWAY_ENABLED=maybe should error
        unsafe {
            std::env::set_var("GATEWAY_ENABLED", "maybe");
            std::env::remove_var("HTTP_PORT");
            std::env::remove_var("HTTP_HOST");
            std::env::remove_var("SIGNAL_HTTP_URL");
            std::env::remove_var("CLI_ENABLED");
            std::env::remove_var("WASM_CHANNELS_ENABLED");
            std::env::remove_var("GATEWAY_PORT");
            std::env::remove_var("GATEWAY_HOST");
            std::env::remove_var("GATEWAY_AUTH_TOKEN");
            std::env::remove_var("GATEWAY_USER_ID");
            std::env::remove_var("WASM_CHANNELS_DIR");
            std::env::remove_var("TELEGRAM_OWNER_ID");
        }
        let result = ChannelsConfig::resolve(&settings, false);
        assert!(result.is_err(), "GATEWAY_ENABLED=maybe should be rejected");

        // CLI_ENABLED=on should error
        unsafe {
            std::env::remove_var("GATEWAY_ENABLED");
            std::env::set_var("CLI_ENABLED", "on");
        }
        let result = ChannelsConfig::resolve(&settings, false);
        assert!(result.is_err(), "CLI_ENABLED=on should be rejected");

        // WASM_CHANNELS_ENABLED=yes should error
        unsafe {
            std::env::remove_var("CLI_ENABLED");
            std::env::set_var("WASM_CHANNELS_ENABLED", "yes");
        }
        let result = ChannelsConfig::resolve(&settings, false);
        assert!(
            result.is_err(),
            "WASM_CHANNELS_ENABLED=yes should be rejected"
        );

        // Cleanup
        unsafe {
            std::env::remove_var("WASM_CHANNELS_ENABLED");
        }
    }
}
