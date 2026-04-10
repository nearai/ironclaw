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

impl SecretsConfig {
    /// Auto-detect secrets master key from env var, then OS keychain.
    ///
    /// Sequential probe: SECRETS_MASTER_KEY env var first, then OS keychain.
    /// If neither source has a key, auto-generate one and persist it to the
    /// keychain (preferred) or a file at `~/.ironclaw/master.key` (fallback).
    /// This ensures the secrets store is always available without manual setup.
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
                Err(_) => {
                    // Try loading from the file-based fallback
                    match Self::load_master_key_file() {
                        Some(key_hex) => (Some(SecretString::from(key_hex)), KeySource::File),
                        None => {
                            // No key anywhere — auto-generate and persist
                            Self::auto_generate_master_key().await
                        }
                    }
                }
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

    /// Auto-generate a master key and try to persist it.
    ///
    /// Tries the OS keychain first; falls back to `~/.ironclaw/master.key`.
    async fn auto_generate_master_key() -> (Option<SecretString>, crate::settings::KeySource) {
        use crate::settings::KeySource;

        let key_bytes = crate::secrets::keychain::generate_master_key();
        let key_hex: String = key_bytes.iter().map(|b| format!("{:02x}", b)).collect();

        // Try storing in the OS keychain
        if crate::secrets::keychain::store_master_key(&key_bytes)
            .await
            .is_ok()
        {
            tracing::debug!("Auto-generated secrets master key and stored in OS keychain");
            return (Some(SecretString::from(key_hex)), KeySource::Keychain);
        }

        // Keychain unavailable — persist to file
        if Self::save_master_key_file(&key_hex) {
            tracing::debug!(
                "Auto-generated secrets master key and stored in {}",
                Self::master_key_file_path().display()
            );
            return (Some(SecretString::from(key_hex)), KeySource::File);
        }

        tracing::warn!(
            "Failed to persist auto-generated master key to keychain or file. \
             Set SECRETS_MASTER_KEY env var to enable the secrets store."
        );
        (None, KeySource::None)
    }

    /// Path to the file-based master key fallback.
    fn master_key_file_path() -> std::path::PathBuf {
        crate::bootstrap::ironclaw_base_dir().join("master.key")
    }

    /// Load the master key from the file-based fallback.
    fn load_master_key_file() -> Option<String> {
        Self::load_master_key_from_path(&Self::master_key_file_path())
    }

    /// Load and validate a master key from a specific file path.
    fn load_master_key_from_path(path: &std::path::Path) -> Option<String> {
        match std::fs::read_to_string(path) {
            Ok(contents) => {
                let trimmed = contents.trim().to_string();
                if trimmed.len() >= 64 && trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
                    Some(trimmed)
                } else {
                    tracing::warn!(
                        "master.key file exists but contains invalid data (expected 64+ hex chars)"
                    );
                    None
                }
            }
            Err(_) => None,
        }
    }

    /// Save the master key to the file-based fallback with restrictive permissions.
    fn save_master_key_file(key_hex: &str) -> bool {
        Self::save_master_key_to_path(key_hex, &Self::master_key_file_path())
    }

    /// Save the master key to a specific file path with restrictive permissions.
    fn save_master_key_to_path(key_hex: &str, path: &std::path::Path) -> bool {
        // Ensure parent directory exists
        if let Some(parent) = path.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            tracing::debug!("Cannot create directory for master.key: {e}");
            return false;
        }

        // Write with restrictive permissions (owner read/write only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            match std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(path)
            {
                Ok(mut file) => {
                    use std::io::Write;
                    if let Err(e) = file.write_all(key_hex.as_bytes()) {
                        tracing::debug!("Failed to write master.key: {e}");
                        return false;
                    }
                    true
                }
                Err(e) => {
                    tracing::debug!("Failed to create master.key: {e}");
                    false
                }
            }
        }

        #[cfg(not(unix))]
        {
            match std::fs::write(path, key_hex) {
                Ok(()) => true,
                Err(e) => {
                    tracing::debug!("Failed to write master.key: {e}");
                    false
                }
            }
        }
    }

    /// Get the master key if configured.
    pub fn master_key(&self) -> Option<&SecretString> {
        self.master_key.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_save_and_load_master_key_file_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("master.key");

        let key_hex = "a".repeat(64); // valid 32-byte hex key
        assert!(SecretsConfig::save_master_key_to_path(&key_hex, &path));

        let loaded = SecretsConfig::load_master_key_from_path(&path);
        assert_eq!(loaded, Some(key_hex));
    }

    #[test]
    fn test_load_master_key_file_rejects_short_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("master.key");
        std::fs::write(&path, "abcd1234").unwrap();

        let loaded = SecretsConfig::load_master_key_from_path(&path);
        assert!(loaded.is_none());
    }

    #[test]
    fn test_load_master_key_file_rejects_non_hex() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("master.key");
        // 64 chars but not all hex
        let bad_key = format!("{}zzzz", "a".repeat(60));
        std::fs::write(&path, &bad_key).unwrap();

        let loaded = SecretsConfig::load_master_key_from_path(&path);
        assert!(loaded.is_none());
    }

    #[test]
    fn test_load_master_key_file_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nonexistent.key");

        let loaded = SecretsConfig::load_master_key_from_path(&path);
        assert!(loaded.is_none());
    }

    #[test]
    fn test_load_master_key_file_trims_whitespace() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("master.key");
        let key_hex = "b".repeat(64);
        std::fs::write(&path, format!("  {key_hex}  \n")).unwrap();

        let loaded = SecretsConfig::load_master_key_from_path(&path);
        assert_eq!(loaded, Some(key_hex));
    }

    #[cfg(unix)]
    #[test]
    fn test_save_master_key_file_permissions() {
        use std::os::unix::fs::MetadataExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("master.key");

        let key_hex = "c".repeat(64);
        assert!(SecretsConfig::save_master_key_to_path(&key_hex, &path));

        let metadata = std::fs::metadata(&path).unwrap();
        assert_eq!(metadata.mode() & 0o777, 0o600);
    }

    #[test]
    fn test_save_master_key_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("dir").join("master.key");

        let key_hex = "d".repeat(64);
        assert!(SecretsConfig::save_master_key_to_path(&key_hex, &path));

        let loaded = SecretsConfig::load_master_key_from_path(&path);
        assert_eq!(loaded, Some(key_hex));
    }
}
