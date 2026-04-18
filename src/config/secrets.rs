use std::path::Path;

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
    /// Resolve the secrets master key.
    ///
    /// Order:
    /// 1. `SECRETS_MASTER_KEY` env var (process env or `.env` overlay)
    /// 2. OS keychain entry
    /// 3. Auto-generate and persist: OS keychain first, `~/.ironclaw/.env`
    ///    fallback when the keychain is unavailable.
    ///
    /// Running this on every startup (not only onboarding) closes #1820:
    /// users who skipped or partially completed onboarding, or who run on
    /// headless Linux without secret-service, previously ended up with no
    /// secrets store and saw "secrets store is not available" when
    /// configuring API keys. The generate-and-persist step is the same path
    /// the onboarding wizard's quick mode already uses; calling it from
    /// here means there is exactly one place that writes the master key.
    pub(crate) async fn resolve() -> Result<Self, ConfigError> {
        Self::resolve_with_env_path(&crate::bootstrap::ironclaw_env_path()).await
    }

    /// Testable variant of [`Self::resolve`] that writes its `.env`
    /// fallback to an explicit path. Production code calls
    /// [`Self::resolve`], which targets `~/.ironclaw/.env`.
    pub(crate) async fn resolve_with_env_path(env_path: &Path) -> Result<Self, ConfigError> {
        use crate::settings::KeySource;

        if let Some(env_key) = optional_env("SECRETS_MASTER_KEY")? {
            return Self::build(Some(SecretString::from(env_key)), KeySource::Env);
        }

        if let Ok(key_bytes) = crate::secrets::keychain::get_master_key().await {
            let key_hex: String = key_bytes.iter().map(|b| format!("{:02x}", b)).collect();
            return Self::build(Some(SecretString::from(key_hex)), KeySource::Keychain);
        }

        let (key_hex, source) = Self::auto_generate_and_persist(env_path).await?;
        Self::build(Some(SecretString::from(key_hex)), source)
    }

    /// Generate a new master key and persist it.
    ///
    /// Tries the OS keychain first. If the keychain is unavailable
    /// (headless Linux, CI without secret-service, macOS without keychain
    /// access), writes the key to `env_path` as `SECRETS_MASTER_KEY=…`
    /// via `upsert_bootstrap_vars_to` (the same writer the onboarding
    /// wizard uses) and injects it into the process env overlay so the
    /// current run sees it immediately.
    async fn auto_generate_and_persist(
        env_path: &Path,
    ) -> Result<(String, crate::settings::KeySource), ConfigError> {
        use crate::settings::KeySource;

        let key_bytes = crate::secrets::keychain::generate_master_key();
        let key_hex: String = key_bytes.iter().map(|b| format!("{:02x}", b)).collect();

        if crate::secrets::keychain::store_master_key(&key_bytes)
            .await
            .is_ok()
        {
            tracing::debug!("Auto-generated secrets master key; stored in OS keychain");
            return Ok((key_hex, KeySource::Keychain));
        }

        crate::bootstrap::upsert_bootstrap_vars_to(env_path, &[("SECRETS_MASTER_KEY", &key_hex)])
            .map_err(|e| ConfigError::InvalidValue {
            key: "SECRETS_MASTER_KEY".to_string(),
            message: format!(
                "failed to persist auto-generated master key to {}: {e}",
                env_path.display()
            ),
        })?;
        crate::config::inject_single_var("SECRETS_MASTER_KEY", &key_hex);
        tracing::debug!(
            "Auto-generated secrets master key; stored in {}",
            env_path.display()
        );
        Ok((key_hex, KeySource::Env))
    }

    fn build(
        master_key: Option<SecretString>,
        source: crate::settings::KeySource,
    ) -> Result<Self, ConfigError> {
        if let Some(ref key) = master_key
            && key.expose_secret().len() < 32
        {
            return Err(ConfigError::InvalidValue {
                key: "SECRETS_MASTER_KEY".to_string(),
                message: "must be at least 32 bytes for AES-256-GCM".to_string(),
            });
        }
        let enabled = master_key.is_some();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::helpers::lock_env;
    use crate::settings::KeySource;

    /// Regression test for #1820: when neither the env var nor the OS
    /// keychain yield a key, `resolve_with_env_path` must auto-generate
    /// one and persist it so the caller gets a usable secrets store
    /// without requiring onboarding.
    ///
    /// The test gracefully skips when the host keychain already holds
    /// a master key for the `ironclaw` service — that would make the
    /// generate-and-persist branch unreachable and we'd otherwise
    /// wipe/overwrite a developer's real key.
    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // env guard must span the entire test
    async fn resolve_persists_generated_key_when_keychain_empty() {
        let _guard = lock_env();
        // SAFETY: serialized via ENV_MUTEX (lock_env).
        let prior = std::env::var("SECRETS_MASTER_KEY").ok();
        unsafe {
            std::env::remove_var("SECRETS_MASTER_KEY");
        }

        let result = async {
            if crate::secrets::keychain::get_master_key().await.is_ok() {
                eprintln!(
                    "skipping: host keychain already holds a master key; \
                     cannot exercise the generate-and-persist path"
                );
                return;
            }

            let dir = tempfile::tempdir().unwrap();
            let env_path = dir.path().join(".env");

            let cfg = SecretsConfig::resolve_with_env_path(&env_path)
                .await
                .expect("resolve must succeed and auto-generate a key");

            assert!(cfg.enabled, "secrets must be enabled after auto-generate");
            assert!(cfg.master_key.is_some(), "master key must be populated");

            match cfg.source {
                KeySource::Env => {
                    let contents = std::fs::read_to_string(&env_path).unwrap();
                    assert!(
                        contents.contains("SECRETS_MASTER_KEY="),
                        ".env must carry the generated key; got: {contents}"
                    );
                    let key = cfg.master_key.unwrap().expose_secret().to_string();
                    assert_eq!(key.len(), 64, "32-byte AES-256 key = 64 hex chars");
                    assert!(
                        contents.contains(&key),
                        "persisted value must match the returned master key"
                    );
                }
                KeySource::Keychain => {
                    // Keychain accepted the write. Clean up so we don't
                    // leave a generated key in the dev machine's real
                    // keychain.
                    let _ = crate::secrets::keychain::delete_master_key().await;
                }
                other => panic!("unexpected source after auto-generate: {other:?}"),
            }
        }
        .await;

        // SAFETY: serialized via ENV_MUTEX (lock_env).
        unsafe {
            if let Some(ref v) = prior {
                std::env::set_var("SECRETS_MASTER_KEY", v);
            } else {
                std::env::remove_var("SECRETS_MASTER_KEY");
            }
        }
        result
    }

    /// When `SECRETS_MASTER_KEY` is set, resolve must not touch the
    /// keychain and must not write to `.env`.
    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // env guard must span the entire test
    async fn env_var_is_used_directly_without_generating() {
        let _guard = lock_env();
        let prior = std::env::var("SECRETS_MASTER_KEY").ok();
        let key_hex = "a".repeat(64);
        // SAFETY: serialized via ENV_MUTEX (lock_env).
        unsafe {
            std::env::set_var("SECRETS_MASTER_KEY", &key_hex);
        }

        let dir = tempfile::tempdir().unwrap();
        let env_path = dir.path().join(".env");

        let cfg = SecretsConfig::resolve_with_env_path(&env_path)
            .await
            .expect("env-var path must succeed");

        assert_eq!(cfg.source, KeySource::Env);
        assert_eq!(
            cfg.master_key
                .as_ref()
                .map(|k| k.expose_secret().to_string()),
            Some(key_hex)
        );
        assert!(
            !env_path.exists(),
            "resolve must not create .env when the env var is already set"
        );

        // SAFETY: serialized via ENV_MUTEX (lock_env).
        unsafe {
            if let Some(ref v) = prior {
                std::env::set_var("SECRETS_MASTER_KEY", v);
            } else {
                std::env::remove_var("SECRETS_MASTER_KEY");
            }
        }
    }

    /// A too-short master key is rejected even when supplied via env.
    /// AES-256-GCM requires 32 bytes; accepting a shorter key would
    /// silently break decryption.
    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // env guard must span the entire test
    async fn short_env_key_is_rejected() {
        let _guard = lock_env();
        let prior = std::env::var("SECRETS_MASTER_KEY").ok();
        // SAFETY: serialized via ENV_MUTEX (lock_env).
        unsafe {
            std::env::set_var("SECRETS_MASTER_KEY", "too-short");
        }

        let dir = tempfile::tempdir().unwrap();
        let env_path = dir.path().join(".env");

        let err = SecretsConfig::resolve_with_env_path(&env_path)
            .await
            .expect_err("short key must fail");
        assert!(err.to_string().contains("32 bytes"));

        // SAFETY: serialized via ENV_MUTEX (lock_env).
        unsafe {
            if let Some(ref v) = prior {
                std::env::set_var("SECRETS_MASTER_KEY", v);
            } else {
                std::env::remove_var("SECRETS_MASTER_KEY");
            }
        }
    }
}
