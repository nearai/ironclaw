use secrecy::{ExposeSecret, SecretString};

use crate::config::helpers::optional_env;
use crate::error::ConfigError;

/// Secrets management configuration.
#[derive(Clone, Default)]
pub struct SecretsConfig {
    /// Master key for encrypting secrets.
    pub master_key: Option<SecretString>,
    /// Whether secrets management is enabled.
    pub enabled: bool,
    /// Source of the master key.
    pub source: crate::settings::KeySource,
}

impl std::fmt::Debug for SecretsConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretsConfig")
            .field("master_key", &self.master_key.is_some())
            .field("enabled", &self.enabled)
            .field("source", &self.source)
            .finish()
    }
}

/// Process-wide cache for the keychain master key.
///
/// Avoids re-prompting the OS keychain on every `SecretsConfig::resolve()` call
/// (e.g. `Config::from_env()` then `Config::from_db()`). Thread-safe alternative
/// to caching in a process env var.
impl SecretsConfig {
    /// Auto-detect secrets master key from env var, then OS keychain.
    ///
    /// Sequential probe: SECRETS_MASTER_KEY env var first, then OS keychain.
    /// No saved "source" needed; just try each source in order.
    pub(crate) async fn resolve() -> Result<Self, ConfigError> {
        use crate::settings::KeySource;

        let (master_key, source) = if let Some(env_key) = optional_env("SECRETS_MASTER_KEY")? {
            (Some(SecretString::from(env_key)), KeySource::Env)
        } else {
            // Probe the OS keychain; if a key is stored, use it
            match crate::secrets::keychain::get_master_key().await {
                Ok(key_bytes) => {
                    let key_hex: String = key_bytes.iter().map(|b| format!("{:02x}", b)).collect();
                    (Some(SecretString::from(key_hex)), KeySource::Keychain)
                }
                Err(_) => (None, KeySource::None),
            }
        };

        let enabled = master_key.is_some();

        if let Some(ref key) = master_key
            && key.expose_secret().len() < 32
        {
            return Err(ConfigError::InvalidValue {
                key: "SECRETS_MASTER_KEY".to_string(),
                message: "must be at least 32 bytes for AES-256-GCM".to_string(),
            });
        }

        Ok(Self {
            master_key,
            enabled,
            source,
        })
    }

    /// Get the master key if configured.
    pub fn master_key(&self) -> Option<&SecretString> {
        self.master_key.as_ref()
    }
}
