use std::collections::HashMap;
use std::path::PathBuf;

use secrecy::SecretString;
use serde::Deserialize;

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
    pub xmpp: Option<XmppConfig>,
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
    /// Additional user scopes for workspace reads.
    ///
    /// When set, the workspace will be able to read (search, read, list) from
    /// these additional user scopes while writes remain isolated to `user_id`.
    /// Parsed from `WORKSPACE_READ_SCOPES` (comma-separated).
    pub workspace_read_scopes: Vec<String>,
    /// Memory layer definitions (JSON in env var, or from external config).
    pub memory_layers: Vec<crate::workspace::layer::MemoryLayer>,
    /// Multi-user token map. When set, each token maps to a user identity.
    /// Parsed from `GATEWAY_USER_TOKENS` (JSON string). When absent, falls back
    /// to single-user mode via `auth_token` + `user_id`.
    pub user_tokens: Option<HashMap<String, UserTokenConfig>>,
}

/// Per-user token configuration for multi-user mode.
#[derive(Debug, Clone, Deserialize)]
pub struct UserTokenConfig {
    pub user_id: String,
    #[serde(default)]
    pub workspace_read_scopes: Vec<String>,
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

/// XMPP channel configuration with OMEMO encryption.
#[derive(Debug, Clone)]
pub struct XmppConfig {
    /// XMPP JID, e.g. `bot@example.com`.
    pub jid: String,
    /// Account password.
    pub password: secrecy::SecretString,
    /// Allowed sender bare JIDs for DMs. `*` = open. Default: own JID only.
    pub allow_from: Vec<String>,
    /// DM policy: "allowlist", "open", or "pairing". Default: "allowlist".
    pub dm_policy: String,
    /// Allowed MUC room JIDs. `*` = open. Empty = deny all group messages.
    pub allow_rooms: Vec<String>,
    /// Rooms that must use encrypted groupchat handling.
    pub encrypted_rooms: Vec<String>,
    /// OMEMO device ID (0 = auto-generate on first run).
    pub device_id: u32,
    /// Directory for OMEMO key/session persistence (default: ~/.ironclaw/xmpp/).
    pub omemo_store_dir: std::path::PathBuf,
    /// Fall back to plaintext when no OMEMO bundle is available.
    pub allow_plaintext_fallback: bool,
    /// Maximum outbound XMPP messages per hour (0 = unlimited).
    pub max_messages_per_hour: u32,
}

impl ChannelsConfig {
    pub(crate) fn resolve(settings: &Settings, owner_id: &str) -> Result<Self, ConfigError> {
        let cs = &settings.channels;

        let http_enabled_by_env =
            optional_env("HTTP_PORT")?.is_some() || optional_env("HTTP_HOST")?.is_some();
        let http = if http_enabled_by_env || cs.http_enabled {
            Some(HttpConfig {
                host: optional_env("HTTP_HOST")?
                    .or_else(|| cs.http_host.clone())
                    .unwrap_or_else(|| "0.0.0.0".to_string()),
                port: parse_optional_env("HTTP_PORT", cs.http_port.unwrap_or(8080))?,
                webhook_secret: optional_env("HTTP_WEBHOOK_SECRET")?.map(SecretString::from),
                user_id: owner_id.to_string(),
            })
        } else {
            None
        };

        let gateway_enabled = parse_bool_env("GATEWAY_ENABLED", cs.gateway_enabled)?;
        let gateway = if gateway_enabled {
            let user_id = optional_env("GATEWAY_USER_ID")?
                .or_else(|| cs.gateway_user_id.clone())
                .unwrap_or_else(|| owner_id.to_string());

            let memory_layers: Vec<crate::workspace::layer::MemoryLayer> =
                match optional_env("MEMORY_LAYERS")? {
                    Some(json_str) => {
                        serde_json::from_str(&json_str).map_err(|e| ConfigError::InvalidValue {
                            key: "MEMORY_LAYERS".to_string(),
                            message: format!("must be valid JSON array of layer objects: {e}"),
                        })?
                    }
                    None => crate::workspace::layer::MemoryLayer::default_for_user(&user_id),
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

            let user_tokens: Option<HashMap<String, UserTokenConfig>> =
                match optional_env("GATEWAY_USER_TOKENS")? {
                    Some(json_str) => {
                        let tokens: HashMap<String, UserTokenConfig> = serde_json::from_str(
                            &json_str,
                        )
                        .map_err(|e| ConfigError::InvalidValue {
                            key: "GATEWAY_USER_TOKENS".to_string(),
                            message: format!(
                                "must be valid JSON object mapping tokens to user configs: {e}"
                            ),
                        })?;
                        if tokens.is_empty() {
                            return Err(ConfigError::InvalidValue {
                            key: "GATEWAY_USER_TOKENS".to_string(),
                            message:
                                "token map is empty — remove the variable to use single-user mode"
                                    .to_string(),
                        });
                        }
                        for (tok, cfg) in &tokens {
                            if cfg.user_id.trim().is_empty() {
                                return Err(ConfigError::InvalidValue {
                                    key: "GATEWAY_USER_TOKENS".to_string(),
                                    message: format!(
                                        "token '{}...' has an empty user_id",
                                        &tok[..tok.len().min(8)]
                                    ),
                                });
                            }
                        }
                        Some(tokens)
                    }
                    None => None,
                };
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
                        message: format!("scope '{}...' exceeds 128 characters", &scope[..32]),
                    });
                }
            }
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
                workspace_read_scopes,
                memory_layers,
                user_tokens,
            })
        } else {
            None
        };

        let signal_url = optional_env("SIGNAL_HTTP_URL")?.or_else(|| cs.signal_http_url.clone());
        let signal = if let Some(http_url) = signal_url {
            let account = optional_env("SIGNAL_ACCOUNT")?
                .or_else(|| cs.signal_account.clone())
                .ok_or(ConfigError::InvalidValue {
                    key: "SIGNAL_ACCOUNT".to_string(),
                    message: "SIGNAL_ACCOUNT is required when SIGNAL_HTTP_URL is set".to_string(),
                })?;
            let allow_from =
                match optional_env("SIGNAL_ALLOW_FROM")?.or_else(|| cs.signal_allow_from.clone()) {
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

        let xmpp_jid = optional_env("XMPP_JID")?.or_else(|| cs.xmpp_jid.clone());
        let xmpp = if let Some(jid) = xmpp_jid {
            let password = optional_env("XMPP_PASSWORD")?
                .or_else(|| cs.xmpp_password.clone())
                .ok_or(ConfigError::InvalidValue {
                    key: "XMPP_PASSWORD".to_string(),
                    message: "XMPP_PASSWORD is required when XMPP_JID is set".to_string(),
                })?;
            let device_id: u32 = parse_optional_env("XMPP_OMEMO_DEVICE_ID", 0u32)?;
            let omemo_store_dir = optional_env("XMPP_OMEMO_STORE_DIR")?
                .map(std::path::PathBuf::from)
                .or_else(|| cs.xmpp_omemo_store_dir.clone())
                .unwrap_or_else(|| ironclaw_base_dir().join("xmpp"));
            let allow_from = optional_env("XMPP_ALLOW_FROM")?
                .or_else(|| cs.xmpp_allow_from.clone())
                .map(|s| {
                    s.split(',')
                        .map(|e| e.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_else(|| vec![jid.clone()]);
            let dm_policy = optional_env("XMPP_DM_POLICY")?
                .or_else(|| cs.xmpp_dm_policy.clone())
                .unwrap_or_else(|| "allowlist".to_string());
            let allow_rooms: Vec<String> = optional_env("XMPP_ALLOW_ROOMS")?
                .or_else(|| cs.xmpp_allow_rooms.clone())
                .map(|s| {
                    s.split(',')
                        .map(|e| e.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default();
            let encrypted_rooms: Vec<String> = optional_env("XMPP_ENCRYPTED_ROOMS")?
                .or_else(|| cs.xmpp_encrypted_rooms.clone())
                .map(|s| {
                    s.split(',')
                        .map(|e| e.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let mut allow_rooms = allow_rooms;
            if !allow_rooms.iter().any(|room| room == "*") {
                for room in &encrypted_rooms {
                    if !allow_rooms
                        .iter()
                        .any(|value: &String| value.eq_ignore_ascii_case(room))
                    {
                        allow_rooms.push(room.clone());
                    }
                }
            }
            let allow_plaintext_fallback = optional_env("XMPP_ALLOW_PLAINTEXT_FALLBACK")?
                .map(|s| s.eq_ignore_ascii_case("true") || s == "1")
                .or(cs.xmpp_allow_plaintext_fallback)
                .unwrap_or(true);
            let max_messages_per_hour = parse_optional_env(
                "XMPP_MAX_MESSAGES_PER_HOUR",
                cs.xmpp_max_messages_per_hour.unwrap_or(0u32),
            )?;
            Some(XmppConfig {
                jid,
                password: secrecy::SecretString::from(password),
                allow_from,
                dm_policy,
                allow_rooms,
                encrypted_rooms,
                device_id,
                omemo_store_dir,
                allow_plaintext_fallback,
                max_messages_per_hour,
            })
        } else {
            None
        };

        let cli_enabled = parse_bool_env("CLI_ENABLED", cs.cli_enabled)?;

        Ok(Self {
            cli: CliConfig {
                enabled: cli_enabled,
            },
            http,
            gateway,
            signal,
            xmpp,
            wasm_channels_dir: optional_env("WASM_CHANNELS_DIR")?
                .map(PathBuf::from)
                .or_else(|| cs.wasm_channels_dir.clone())
                .unwrap_or_else(default_channels_dir),
            wasm_channels_enabled: parse_bool_env(
                "WASM_CHANNELS_ENABLED",
                cs.wasm_channels_enabled,
            )?,
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
    use crate::config::helpers::lock_env;
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
            workspace_read_scopes: vec![],
            memory_layers: vec![],
            user_tokens: None,
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
            workspace_read_scopes: vec![],
            memory_layers: vec![],
            user_tokens: None,
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
            xmpp: None,
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
            xmpp: None,
            wasm_channels_dir: PathBuf::from("/opt/channels"),
            wasm_channels_enabled: false,
            wasm_channel_owner_ids: ids,
        };
        assert_eq!(cfg.wasm_channel_owner_ids.get("telegram"), Some(&12345));
        assert_eq!(cfg.wasm_channel_owner_ids.get("slack"), Some(&67890));
        assert!(!cfg.wasm_channels_enabled);
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
    fn resolve_uses_settings_channel_values_with_owner_scope_user_ids() {
        let _guard = lock_env();
        let mut settings = Settings::default();
        settings.channels.http_enabled = true;
        settings.channels.http_host = Some("127.0.0.2".to_string());
        settings.channels.http_port = Some(8181);
        settings.channels.gateway_enabled = true;
        settings.channels.gateway_host = Some("127.0.0.3".to_string());
        settings.channels.gateway_port = Some(9191);
        settings.channels.gateway_auth_token = Some("tok".to_string());
        settings.channels.signal_http_url = Some("http://127.0.0.1:8080".to_string());
        settings.channels.signal_account = Some("+15551234567".to_string());
        settings.channels.signal_allow_from = Some("+15551234567,+15557654321".to_string());
        settings.channels.wasm_channels_dir = Some(PathBuf::from("/tmp/settings-channels"));
        settings.channels.wasm_channels_enabled = false;

        let cfg = ChannelsConfig::resolve(&settings, "owner-scope").expect("resolve");

        let http = cfg.http.expect("http config");
        assert_eq!(http.host, "127.0.0.2");
        assert_eq!(http.port, 8181);
        assert_eq!(http.user_id, "owner-scope");

        let gateway = cfg.gateway.expect("gateway config");
        assert_eq!(gateway.host, "127.0.0.3");
        assert_eq!(gateway.port, 9191);
        assert_eq!(gateway.auth_token.as_deref(), Some("tok"));
        assert_eq!(gateway.user_id, "owner-scope");

        let signal = cfg.signal.expect("signal config");
        assert_eq!(signal.account, "+15551234567");
        assert_eq!(signal.allow_from, vec!["+15551234567", "+15557654321"]);

        assert_eq!(
            cfg.wasm_channels_dir,
            PathBuf::from("/tmp/settings-channels")
        );
        assert!(!cfg.wasm_channels_enabled);
    }

    #[test]
    fn resolve_xmpp_dm_policy_from_settings() {
        let _guard = lock_env();
        let mut settings = Settings::default();
        settings.channels.xmpp_jid = Some("bot@example.com".to_string());
        settings.channels.xmpp_password = Some("secret".to_string());
        settings.channels.xmpp_dm_policy = Some("pairing".to_string());
        settings.channels.xmpp_allow_from = Some("alice@example.com".to_string());
        settings.channels.xmpp_encrypted_rooms = Some("room@conference.example.com".to_string());
        settings.channels.xmpp_max_messages_per_hour = Some(6);

        let cfg = ChannelsConfig::resolve(&settings, "owner-scope").expect("resolve");
        let xmpp = cfg.xmpp.expect("xmpp config");

        assert_eq!(xmpp.jid, "bot@example.com");
        assert_eq!(xmpp.dm_policy, "pairing");
        assert_eq!(xmpp.allow_from, vec!["alice@example.com"]);
        assert_eq!(xmpp.allow_rooms, vec!["room@conference.example.com"]);
        assert_eq!(xmpp.encrypted_rooms, vec!["room@conference.example.com"]);
        assert_eq!(xmpp.max_messages_per_hour, 6);
    }
}
