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
    /// `true` when this process had to fall through to
    /// `auto_generate_and_persist` — i.e. no pre-existing
    /// `SECRETS_MASTER_KEY`, no keychain entry, and no on-disk `.env`
    /// key was available when we resolved.
    ///
    /// Consumed by `AppBuilder::init_secrets` as a safety gate: if the
    /// secrets table already has rows, a freshly-generated key can't
    /// decrypt them and startup must fail loudly rather than silently
    /// shadow unrecoverable data.
    pub generated: bool,
}

impl std::fmt::Debug for SecretsConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretsConfig")
            .field("master_key", &self.master_key.is_some())
            .field("enabled", &self.enabled)
            .field("source", &self.source)
            .field("generated", &self.generated)
            .finish()
    }
}

impl SecretsConfig {
    /// Resolve the secrets master key.
    ///
    /// Order:
    /// 1. `SECRETS_MASTER_KEY` env var — explicit user intent and the
    ///    canonical escape hatch for CI/Docker/headless environments.
    ///    Matches the probe order used by `SetupWizard::step_security`,
    ///    `SetupWizard::init_secrets_context`,
    ///    `crate::secrets::resolve_master_key`, and `cli import`, so the
    ///    key a newly-written row is encrypted with always matches the
    ///    key a subsequent startup decrypts with.
    /// 2. OS keychain entry — the managed-storage path for local
    ///    installs that ran the onboarding wizard. Only probed when the
    ///    env var is unset, so setting `SECRETS_MASTER_KEY` explicitly
    ///    does not trigger a macOS Keychain Access dialog.
    /// 3. Auto-generate and persist: try keychain first, fall back to
    ///    `~/.ironclaw/.env` as `SECRETS_MASTER_KEY=…`.
    ///
    /// Running this on every startup (not only onboarding) closes #1820:
    /// users who skipped or partially completed onboarding, or who run
    /// on headless Linux without secret-service, previously ended up
    /// with no secrets store and saw "secrets store is not available"
    /// when configuring API keys.
    ///
    /// Note on zeroization: generated hex keys flow through `String`
    /// before landing in `SecretString`. That intermediate is not
    /// zeroized, but the key ultimately lives in `~/.ironclaw/.env`
    /// (0o600) in plaintext when the keychain is unavailable — the
    /// durable leak surface is the file, not heap fragments.
    pub(crate) async fn resolve() -> Result<Self, ConfigError> {
        Self::resolve_with_env_path(&crate::bootstrap::ironclaw_env_path()).await
    }

    /// Testable variant of [`Self::resolve`] that writes its `.env`
    /// fallback to an explicit path. Production code calls
    /// [`Self::resolve`], which targets `~/.ironclaw/.env`.
    pub(crate) async fn resolve_with_env_path(env_path: &Path) -> Result<Self, ConfigError> {
        // Lazy keychain probe: only touch the OS keychain when the env
        // var is unset. Eager probing would trigger macOS Keychain
        // Access dialogs on every startup even when the user has
        // explicitly set SECRETS_MASTER_KEY.
        let keychain_probe = if optional_env("SECRETS_MASTER_KEY")?.is_some() {
            None
        } else {
            crate::secrets::keychain::get_master_key().await.ok()
        };
        Self::resolve_inner(keychain_probe, env_path, true).await
    }

    /// Resolution core. Takes the keychain probe result and an
    /// `allow_keychain_persist` flag as inputs so tests can exercise
    /// the env-wins and keychain-fallback branches deterministically
    /// without touching the real OS keychain.
    ///
    /// `allow_keychain_persist = false` skips the keychain write in
    /// the auto-generate path, forcing `.env` persistence. Tests use
    /// this to avoid macOS keychain dialogs that would block on dev
    /// machines.
    async fn resolve_inner(
        keychain_key: Option<Vec<u8>>,
        env_path: &Path,
        allow_keychain_persist: bool,
    ) -> Result<Self, ConfigError> {
        use crate::settings::KeySource;

        if let Some(env_key) = optional_env("SECRETS_MASTER_KEY")? {
            return Self::build(Some(SecretString::from(env_key)), KeySource::Env, false);
        }

        if let Some(key_bytes) = keychain_key {
            let key_hex: String = key_bytes.iter().map(|b| format!("{:02x}", b)).collect();
            return Self::build(
                Some(SecretString::from(key_hex)),
                KeySource::Keychain,
                false,
            );
        }

        let (key_hex, source, generated) =
            Self::auto_generate_and_persist(env_path, allow_keychain_persist).await?;
        Self::build(Some(SecretString::from(key_hex)), source, generated)
    }

    /// Generate a new master key and persist it.
    ///
    /// If `allow_keychain_persist` is true and the OS keychain accepts
    /// the write, stores there. Otherwise (keychain disabled for
    /// tests, unavailable on headless Linux, or denied), writes to
    /// `env_path` as `SECRETS_MASTER_KEY=…` via
    /// `upsert_bootstrap_vars_to` (the same writer the onboarding
    /// wizard uses) and injects it into the process env overlay so
    /// the current run sees it immediately.
    ///
    /// Returns `(key_hex, source, generated)`. `generated` is `true`
    /// when this process's freshly-generated key was persisted, and
    /// `false` when the TOCTOU re-check picked up a key another
    /// process wrote concurrently — that key is legitimate (same
    /// round of generation as ours), so the startup safety gate in
    /// `AppBuilder::init_secrets` must not treat it as stale.
    ///
    /// TOCTOU note: before writing to `.env`, we re-read the file to
    /// catch the case where a concurrently-started process already
    /// persisted a key. If we see one there, we use it instead of
    /// overwriting it. This closes the common-case race (P1 writes
    /// while P2 is mid-generate). A residual microsecond-wide race
    /// between our re-check and our write still exists; a full fix
    /// would require a file lock.
    async fn auto_generate_and_persist(
        env_path: &Path,
        allow_keychain_persist: bool,
    ) -> Result<(String, crate::settings::KeySource, bool), ConfigError> {
        use crate::settings::KeySource;

        let key_bytes = crate::secrets::keychain::generate_master_key();
        let key_hex: String = key_bytes.iter().map(|b| format!("{:02x}", b)).collect();

        if allow_keychain_persist
            && crate::secrets::keychain::store_master_key(&key_bytes)
                .await
                .is_ok()
        {
            tracing::debug!("Auto-generated secrets master key; stored in OS keychain");
            return Ok((key_hex, KeySource::Keychain, true));
        }

        // Re-check for a concurrently-written key BEFORE persisting ours.
        // Keeps us from overwriting a winner of the generate race that
        // sits between resolve() and this function. When we reuse that
        // key, we return `generated = false` — the key came from a
        // sibling process that is also fresh-generating, and any rows
        // it has encrypted so far are encrypted with *this same key*.
        // Marking it `generated` would cause the safety gate in
        // `init_secrets` to fire spuriously.
        if let Some(existing) = read_secrets_master_key(env_path) {
            tracing::debug!(
                "Found concurrent master key in {}; using that instead of generated",
                env_path.display()
            );
            crate::config::inject_single_var("SECRETS_MASTER_KEY", &existing);
            return Ok((existing, KeySource::Env, false));
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
        Ok((key_hex, KeySource::Env, true))
    }

    fn build(
        master_key: Option<SecretString>,
        source: crate::settings::KeySource,
        generated: bool,
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
            generated,
        })
    }

    /// Get the master key if configured.
    pub fn master_key(&self) -> Option<&SecretString> {
        self.master_key.as_ref()
    }
}

/// Extract a valid 32-byte hex `SECRETS_MASTER_KEY` value from an
/// ironclaw `.env` file. Returns `None` if the file is missing, the
/// key isn't present, or the value isn't exactly 64 hex characters.
/// Used for TOCTOU detection in [`SecretsConfig::auto_generate_and_persist`].
fn read_secrets_master_key(env_path: &Path) -> Option<String> {
    let contents = std::fs::read_to_string(env_path).ok()?;
    for line in contents.lines() {
        // Skip blank lines and comments rather than bailing out of the
        // whole scan — a normal `.env` often has a leading comment or
        // blank line, and `?` on `split_once` would abort before we
        // ever reach `SECRETS_MASTER_KEY` further down the file.
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != "SECRETS_MASTER_KEY" {
            continue;
        }
        let stripped = value.trim().trim_matches('"');
        if stripped.len() == 64 && stripped.chars().all(|c| c.is_ascii_hexdigit()) {
            return Some(stripped.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::helpers::lock_env;
    use crate::settings::KeySource;

    /// Probe order: when `SECRETS_MASTER_KEY` is set AND the keychain
    /// also yields a key, the env var wins. The env var is the explicit
    /// user-intent / CI escape hatch, and matches the order used by
    /// every other master-key reader (`SetupWizard::step_security`,
    /// `SetupWizard::init_secrets_context`,
    /// `crate::secrets::resolve_master_key`, `cli import`). If the
    /// startup path disagreed, onboarding could encrypt new secrets
    /// with one key and a subsequent boot could read them with another.
    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // env guard must span the entire test
    async fn env_wins_over_keychain_when_both_present() {
        let _guard = lock_env();
        let prior = std::env::var("SECRETS_MASTER_KEY").ok();
        let env_hex = "a".repeat(64);
        // SAFETY: serialized via ENV_MUTEX (lock_env).
        unsafe {
            std::env::set_var("SECRETS_MASTER_KEY", &env_hex);
        }

        let dir = tempfile::tempdir().unwrap();
        let env_path = dir.path().join(".env");

        let keychain_bytes = vec![0xbbu8; 32];

        let cfg = SecretsConfig::resolve_inner(Some(keychain_bytes), &env_path, false)
            .await
            .expect("resolve must succeed with env var present");

        assert_eq!(cfg.source, KeySource::Env);
        assert_eq!(
            cfg.master_key
                .as_ref()
                .map(|k| k.expose_secret().to_string()),
            Some(env_hex),
            "env-var key must be returned, not the keychain key"
        );
        assert!(!env_path.exists(), "env-wins path must not touch .env");

        // SAFETY: serialized via ENV_MUTEX (lock_env).
        unsafe {
            if let Some(ref v) = prior {
                std::env::set_var("SECRETS_MASTER_KEY", v);
            } else {
                std::env::remove_var("SECRETS_MASTER_KEY");
            }
        }
    }

    /// `generated` must be false when the key came from env or
    /// keychain, and true only when we fell through to
    /// `auto_generate_and_persist`. The flag is read in
    /// `AppBuilder::init_secrets` to gate the "DB already populated"
    /// safety check — a false positive (marking an env-supplied key as
    /// generated) would spuriously abort a correctly-configured
    /// startup; a false negative would silently shadow undecryptable
    /// data.
    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // env guard must span the entire test
    async fn generated_flag_tracks_auto_generate_path() {
        let _guard = lock_env();
        let prior = std::env::var("SECRETS_MASTER_KEY").ok();
        let dir = tempfile::tempdir().unwrap();

        // Defensive: clear any overlay entry left by a prior test so
        // the env-branch read below doesn't spuriously succeed on a
        // leaked injection instead of the `set_var` we're about to do.
        crate::config::clear_injected_var("SECRETS_MASTER_KEY");

        // env-first branch: generated = false.
        // SAFETY: serialized via ENV_MUTEX (lock_env).
        unsafe {
            std::env::set_var("SECRETS_MASTER_KEY", "b".repeat(64));
        }
        let cfg = SecretsConfig::resolve_inner(None, &dir.path().join(".env-env"), false)
            .await
            .expect("env branch");
        assert!(
            !cfg.generated,
            "env-supplied key must not be marked generated"
        );

        // keychain branch: generated = false.
        // SAFETY: serialized via ENV_MUTEX (lock_env).
        unsafe {
            std::env::remove_var("SECRETS_MASTER_KEY");
        }
        crate::config::clear_injected_var("SECRETS_MASTER_KEY");
        let cfg = SecretsConfig::resolve_inner(
            Some(vec![0xaau8; 32]),
            &dir.path().join(".env-kc"),
            false,
        )
        .await
        .expect("keychain branch");
        assert!(
            !cfg.generated,
            "keychain-supplied key must not be marked generated"
        );

        // auto-generate branch: generated = true.
        let cfg = SecretsConfig::resolve_inner(None, &dir.path().join(".env-gen"), false)
            .await
            .expect("auto-generate branch");
        assert!(
            cfg.generated,
            "auto-generated key must be marked generated so init_secrets can \
             run the store-already-populated safety check"
        );
        crate::config::clear_injected_var("SECRETS_MASTER_KEY");

        // SAFETY: serialized via ENV_MUTEX (lock_env).
        unsafe {
            if let Some(ref v) = prior {
                std::env::set_var("SECRETS_MASTER_KEY", v);
            } else {
                std::env::remove_var("SECRETS_MASTER_KEY");
            }
        }
    }

    /// When `SECRETS_MASTER_KEY` is unset but the keychain has a key,
    /// the keychain fallback path must succeed. This is the typical
    /// macOS flow after the onboarding wizard stores a generated key.
    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // env guard must span the entire test
    async fn keychain_wins_when_env_unset() {
        let _guard = lock_env();
        let prior = std::env::var("SECRETS_MASTER_KEY").ok();
        // SAFETY: serialized via ENV_MUTEX (lock_env).
        unsafe {
            std::env::remove_var("SECRETS_MASTER_KEY");
        }

        let dir = tempfile::tempdir().unwrap();
        let env_path = dir.path().join(".env");

        let keychain_bytes = vec![0xbbu8; 32];
        let expected_hex: String = keychain_bytes
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect();

        let cfg = SecretsConfig::resolve_inner(Some(keychain_bytes), &env_path, false)
            .await
            .expect("keychain path must succeed when env var unset");

        assert_eq!(cfg.source, KeySource::Keychain);
        assert_eq!(
            cfg.master_key
                .as_ref()
                .map(|k| k.expose_secret().to_string()),
            Some(expected_hex)
        );
        assert!(!env_path.exists(), "keychain-hit path must not create .env");

        // SAFETY: serialized via ENV_MUTEX (lock_env).
        unsafe {
            if let Some(ref v) = prior {
                std::env::set_var("SECRETS_MASTER_KEY", v);
            } else {
                std::env::remove_var("SECRETS_MASTER_KEY");
            }
        }
    }

    /// When the keychain is empty but `SECRETS_MASTER_KEY` is set, the
    /// env-var path is the CI/Docker escape hatch and must still work.
    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // env guard must span the entire test
    async fn env_var_wins_when_keychain_empty() {
        let _guard = lock_env();
        let prior = std::env::var("SECRETS_MASTER_KEY").ok();
        let env_hex = "c".repeat(64);
        // SAFETY: serialized via ENV_MUTEX (lock_env).
        unsafe {
            std::env::set_var("SECRETS_MASTER_KEY", &env_hex);
        }

        let dir = tempfile::tempdir().unwrap();
        let env_path = dir.path().join(".env");

        let cfg = SecretsConfig::resolve_inner(None, &env_path, false)
            .await
            .expect("env-var path must succeed when keychain empty");

        assert_eq!(cfg.source, KeySource::Env);
        assert_eq!(
            cfg.master_key
                .as_ref()
                .map(|k| k.expose_secret().to_string()),
            Some(env_hex)
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

    /// Regression test for #1820: when neither source yields a key,
    /// resolve must auto-generate one and persist via the `.env`
    /// fallback. Driven deterministically through `resolve_inner` with
    /// `allow_keychain_persist = false` so the test never touches the
    /// real OS keychain (avoids macOS permission dialogs on dev
    /// machines).
    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // env guard must span the entire test
    async fn resolve_persists_generated_key_when_nothing_available() {
        let _guard = lock_env();
        let prior = std::env::var("SECRETS_MASTER_KEY").ok();
        // SAFETY: serialized via ENV_MUTEX (lock_env).
        unsafe {
            std::env::remove_var("SECRETS_MASTER_KEY");
        }

        let dir = tempfile::tempdir().unwrap();
        let env_path = dir.path().join(".env");

        let cfg = SecretsConfig::resolve_inner(None, &env_path, false)
            .await
            .expect("resolve must succeed and auto-generate a key");

        assert!(cfg.enabled);
        assert_eq!(cfg.source, KeySource::Env, "must fall back to .env");

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

        // Clear the overlay slot `auto_generate_and_persist` populated
        // so the next test's `optional_env` sees a clean env.
        crate::config::clear_injected_var("SECRETS_MASTER_KEY");

        // SAFETY: serialized via ENV_MUTEX (lock_env).
        unsafe {
            if let Some(ref v) = prior {
                std::env::set_var("SECRETS_MASTER_KEY", v);
            } else {
                std::env::remove_var("SECRETS_MASTER_KEY");
            }
        }
    }

    /// TOCTOU safety: if a concurrently-started process has already
    /// written `SECRETS_MASTER_KEY` to `.env` by the time our
    /// `auto_generate_and_persist` is about to write, we must pick up
    /// their key rather than overwrite with ours.
    ///
    /// Simulates the race by pre-populating `.env` with a key, then
    /// calling `resolve_inner` with `SECRETS_MASTER_KEY` cleared (so
    /// the env branch doesn't short-circuit) and both the keychain
    /// probe and keychain persist disabled (so the generate-and-persist
    /// path is reached deterministically).
    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // env guard must span the entire test
    async fn toctou_picks_up_concurrent_writer() {
        let _guard = lock_env();
        let prior = std::env::var("SECRETS_MASTER_KEY").ok();
        // SAFETY: serialized via ENV_MUTEX (lock_env).
        unsafe {
            std::env::remove_var("SECRETS_MASTER_KEY");
        }

        let dir = tempfile::tempdir().unwrap();
        let env_path = dir.path().join(".env");

        // Pre-populate .env with a "winner" key as if a concurrent
        // process wrote it after our resolve() started.
        let winner_hex = "e".repeat(64);
        std::fs::write(&env_path, format!("SECRETS_MASTER_KEY=\"{winner_hex}\"\n")).unwrap();

        let cfg = SecretsConfig::resolve_inner(None, &env_path, false)
            .await
            .expect("resolve must succeed by picking up the concurrent key");

        assert_eq!(cfg.source, KeySource::Env);
        assert_eq!(
            cfg.master_key
                .as_ref()
                .map(|k| k.expose_secret().to_string()),
            Some(winner_hex.clone()),
            "must reuse the concurrent writer's key, not generate a new one"
        );
        assert!(
            !cfg.generated,
            "TOCTOU-reuse key came from a sibling process's fresh generation and \
             matches whatever rows that sibling has encrypted; marking it generated \
             would cause init_secrets to spuriously bail when the store is already \
             populated by the sibling"
        );

        // The .env file should still contain only the winner's key
        // (our generate path should not have overwritten it).
        let contents = std::fs::read_to_string(&env_path).unwrap();
        let occurrences = contents.matches("SECRETS_MASTER_KEY=").count();
        assert_eq!(
            occurrences, 1,
            ".env must retain exactly one SECRETS_MASTER_KEY line, \
             got: {contents}"
        );
        assert!(
            contents.contains(&winner_hex),
            "winner key must be preserved; got: {contents}"
        );

        // Clear the overlay slot populated when TOCTOU found the
        // concurrent writer's key, so the next test's `optional_env`
        // sees a clean env.
        crate::config::clear_injected_var("SECRETS_MASTER_KEY");

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

        let err = SecretsConfig::resolve_inner(None, &env_path, false)
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

    #[test]
    fn read_secrets_master_key_extracts_valid_hex() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env");
        let key = "f".repeat(64);
        std::fs::write(
            &path,
            format!("DATABASE_URL=\"x\"\nSECRETS_MASTER_KEY=\"{key}\"\n"),
        )
        .unwrap();

        assert_eq!(read_secrets_master_key(&path), Some(key));
    }

    #[test]
    fn read_secrets_master_key_rejects_short_value() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env");
        std::fs::write(&path, "SECRETS_MASTER_KEY=\"too-short\"\n").unwrap();

        assert_eq!(read_secrets_master_key(&path), None);
    }

    #[test]
    fn read_secrets_master_key_rejects_non_hex_value() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env");
        // 64 chars but not all hex.
        let bad = format!("zzz{}", "0".repeat(61));
        std::fs::write(&path, format!("SECRETS_MASTER_KEY=\"{bad}\"\n")).unwrap();

        assert_eq!(read_secrets_master_key(&path), None);
    }

    /// Regression: a `.env` whose `SECRETS_MASTER_KEY` line is preceded
    /// by blank lines or comments must still be parsed. An earlier
    /// version used `split_once('=')?`, which bailed out of the entire
    /// scan on the first non-`KEY=value` line and silently defeated the
    /// TOCTOU re-check.
    #[test]
    fn read_secrets_master_key_skips_blank_and_comment_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env");
        let key = "d".repeat(64);
        std::fs::write(
            &path,
            format!("\n# a comment line\n\n# another comment\nSECRETS_MASTER_KEY=\"{key}\"\n"),
        )
        .unwrap();

        assert_eq!(read_secrets_master_key(&path), Some(key));
    }

    #[test]
    fn read_secrets_master_key_returns_none_for_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nope.env");
        assert_eq!(read_secrets_master_key(&path), None);
    }
}
