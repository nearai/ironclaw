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

#[cfg(test)]
mod tests {
    use super::*;

    /// Every named RUNTIME_CONFIG_KEY_* constant must appear in the reserved
    /// set. If a new host-injected key is added without updating the reserved
    /// list, a manifest's `secret_config_mappings` could silently override it.
    /// Host-insertion call sites all reference these constants (not raw
    /// strings), so asserting `const ⇔ RESERVED_*` gives compile-time +
    /// test-time coupling between "what the host inserts" and "what the
    /// manifest validator blocks."
    #[test]
    fn reserved_set_matches_named_constants() {
        let named = [
            RUNTIME_CONFIG_KEY_WEBHOOK_SECRET,
            RUNTIME_CONFIG_KEY_TUNNEL_URL,
            RUNTIME_CONFIG_KEY_OWNER_ID,
            RUNTIME_CONFIG_KEY_BOT_USERNAME,
        ];
        for key in &named {
            assert!(
                is_reserved_runtime_config_key(key),
                "host-injected runtime key {key:?} must be in RESERVED_RUNTIME_CONFIG_KEYS",
            );
        }
        assert_eq!(
            RESERVED_RUNTIME_CONFIG_KEYS.len(),
            named.len(),
            "RESERVED_RUNTIME_CONFIG_KEYS must equal the set of named host-injected constants",
        );
    }

    #[test]
    fn reserved_check_is_case_and_whitespace_insensitive() {
        assert!(is_reserved_runtime_config_key("Webhook_Secret"));
        assert!(is_reserved_runtime_config_key("  tunnel_url  "));
        assert!(!is_reserved_runtime_config_key("not_reserved"));
    }
}
