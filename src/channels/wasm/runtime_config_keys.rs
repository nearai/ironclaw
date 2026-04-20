/// Host-injected runtime config keys for WASM channels.
///
/// Secret-to-config mappings from channel manifests must not be able to
/// override these values, because the host owns their provenance.
pub(crate) const RUNTIME_CONFIG_KEY_WEBHOOK_SECRET: &str = "webhook_secret";
pub(crate) const RUNTIME_CONFIG_KEY_TUNNEL_URL: &str = "tunnel_url";
pub(crate) const RUNTIME_CONFIG_KEY_OWNER_ID: &str = "owner_id";
pub(crate) const RUNTIME_CONFIG_KEY_BOT_USERNAME: &str = "bot_username";

pub(crate) const RESERVED_RUNTIME_CONFIG_KEYS: &[&str] = &[
    RUNTIME_CONFIG_KEY_WEBHOOK_SECRET,
    RUNTIME_CONFIG_KEY_TUNNEL_URL,
    RUNTIME_CONFIG_KEY_OWNER_ID,
    RUNTIME_CONFIG_KEY_BOT_USERNAME,
];

pub(crate) fn is_reserved_runtime_config_key(config_key: &str) -> bool {
    let trimmed = config_key.trim();
    RESERVED_RUNTIME_CONFIG_KEYS
        .iter()
        .any(|reserved| trimmed.eq_ignore_ascii_case(reserved))
}
