use secrecy::{ExposeSecret, SecretString};
use zeroize::Zeroizing;

use crate::config::helpers::optional_env;
use crate::error::ConfigError;
use crate::secrets::keychain::{Keystore, KeystoreError, OsKeystore};

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
    ///
    /// # Keystore failure semantics
    ///
    /// The OS keychain is probed via the [`Keystore`] abstraction, which
    /// distinguishes `NotFound` ("store is reachable but has no entry" --
    /// a legitimate signal to fall back or generate) from `Unavailable`
    /// ("store is locked / D-Bus down / permission denied" -- a transient
    /// failure that MUST NOT be treated as NotFound). A transient failure
    /// propagates as a startup error rather than silently rotating the
    /// master key and stranding any previously-encrypted secrets.
    pub(crate) async fn resolve() -> Result<Self, ConfigError> {
        Self::resolve_with_keystore(
            &OsKeystore,
            &Self::master_key_file_path(),
            /* allow_keystore_persist = */ true,
        )
        .await
    }

    /// Testable core of [`Self::resolve`]. Isolates the keystore
    /// abstraction and file-fallback path so caller-level tests can
    /// simulate first-boot, reuse, NotFound-with-file, and transient
    /// failure modes deterministically.
    ///
    /// `allow_keystore_persist` controls whether newly-generated keys
    /// may be written to the keystore; tests disable this to force the
    /// file-fallback path.
    pub(crate) async fn resolve_with_keystore<K: Keystore + ?Sized>(
        keystore: &K,
        master_key_file: &std::path::Path,
        allow_keystore_persist: bool,
    ) -> Result<Self, ConfigError> {
        use crate::settings::KeySource;

        let (master_key, source) = if let Some(env_key) = optional_env("SECRETS_MASTER_KEY")? {
            (Some(SecretString::from(env_key)), KeySource::Env)
        } else {
            match keystore.get_master_key().await {
                Ok(key_bytes) => {
                    // Key present in keychain -- reuse it. Never rotate.
                    let key_hex: Zeroizing<String> =
                        Zeroizing::new(key_bytes.iter().map(|b| format!("{:02x}", b)).collect());
                    (
                        Some(SecretString::from(key_hex.as_str().to_owned())),
                        KeySource::Keychain,
                    )
                }
                Err(KeystoreError::NotFound) => {
                    // Keychain is reachable and authoritatively reports
                    // no entry. Safe to fall back to file, or generate.
                    match Self::load_master_key_from_path(master_key_file) {
                        Some(key_hex) => (Some(SecretString::from(key_hex)), KeySource::File),
                        None => {
                            Self::auto_generate_master_key(
                                keystore,
                                master_key_file,
                                allow_keystore_persist,
                            )
                            .await
                        }
                    }
                }
                Err(KeystoreError::Unavailable { reason }) => {
                    // Transient failure (locked, D-Bus down, permission
                    // denied, etc.). If a file-based key already exists,
                    // use it -- that's the key we'd regenerate to anyway
                    // and preserving it avoids stranding secrets the
                    // next time the keychain comes back.
                    //
                    // If there is NO existing key anywhere, we MUST NOT
                    // silently generate a new one: doing so could strand
                    // previously-encrypted secrets that were keyed from
                    // the real keychain value we couldn't read. Propagate
                    // the error and fail to start.
                    match Self::load_master_key_from_path(master_key_file) {
                        Some(key_hex) => {
                            tracing::debug!(
                                "OS keystore unavailable ({reason}); \
                                 using existing file-based master key"
                            );
                            (Some(SecretString::from(key_hex)), KeySource::File)
                        }
                        None => {
                            return Err(ConfigError::InvalidValue {
                                key: "SECRETS_MASTER_KEY".to_string(),
                                message: format!(
                                    "OS keystore is unavailable ({reason}) and no \
                                     master.key file fallback exists. Refusing to \
                                     auto-generate a new master key because doing so \
                                     would silently strand any previously-encrypted \
                                     secrets once the keystore becomes reachable \
                                     again. Set SECRETS_MASTER_KEY explicitly, unlock \
                                     the keystore, or place a master.key file."
                                ),
                            });
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
    /// Tries the OS keystore first (when `allow_keystore_persist` is
    /// true); falls back to `master_key_file`.
    async fn auto_generate_master_key<K: Keystore + ?Sized>(
        keystore: &K,
        master_key_file: &std::path::Path,
        allow_keystore_persist: bool,
    ) -> (Option<SecretString>, crate::settings::KeySource) {
        use crate::settings::KeySource;

        let key_bytes = crate::secrets::keychain::generate_master_key();
        let key_hex: Zeroizing<String> =
            Zeroizing::new(key_bytes.iter().map(|b| format!("{:02x}", b)).collect());

        if allow_keystore_persist && keystore.store_master_key(&key_bytes).await.is_ok() {
            tracing::debug!("Auto-generated secrets master key and stored in OS keychain");
            return (
                Some(SecretString::from(key_hex.as_str().to_owned())),
                KeySource::Keychain,
            );
        }

        // Keychain unavailable -- persist to file.
        // save_master_key_to_path uses create_new, so it returns false if
        // another process already created the file (TOCTOU race). In that
        // case, load the winner's key.
        if Self::save_master_key_to_path(&key_hex, master_key_file) {
            tracing::debug!("Auto-generated secrets master key and stored to file");
            return (
                Some(SecretString::from(key_hex.as_str().to_owned())),
                KeySource::File,
            );
        }

        // Check if another process won the race and created the file.
        if let Some(existing_hex) = Self::load_master_key_from_path(master_key_file) {
            tracing::debug!("Loaded master key created by concurrent process");
            return (Some(SecretString::from(existing_hex)), KeySource::File);
        }

        tracing::debug!(
            "Failed to persist auto-generated master key to keychain or file. \
             Set SECRETS_MASTER_KEY env var to enable the secrets store."
        );
        (None, KeySource::None)
    }

    /// Path to the file-based master key fallback.
    fn master_key_file_path() -> std::path::PathBuf {
        crate::bootstrap::ironclaw_base_dir().join("master.key")
    }

    /// Load and validate a master key from a specific file path.
    fn load_master_key_from_path(path: &std::path::Path) -> Option<String> {
        match std::fs::read_to_string(path) {
            Ok(contents) => {
                let trimmed = contents.trim().to_string();
                if trimmed.len() == 64 && trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
                    Some(trimmed)
                } else {
                    tracing::debug!(
                        "master.key file exists but contains invalid data \
                         (expected exactly 64 hex chars)"
                    );
                    None
                }
            }
            Err(_) => None,
        }
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

        // Write with restrictive permissions (owner read/write only).
        // Use create_new to prevent TOCTOU race: if two processes race to
        // generate a key on first boot, only one wins; the loser loads the
        // winner's key instead of overwriting it.
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
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
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    // Another process won the race -- load their key.
                    tracing::debug!(
                        "master.key already exists (concurrent creation); loading existing"
                    );
                    false
                }
                Err(e) => {
                    tracing::debug!("Failed to create master.key: {e}");
                    false
                }
            }
        }

        #[cfg(not(unix))]
        {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
            {
                Ok(mut file) => {
                    use std::io::Write;
                    if let Err(e) = file.write_all(key_hex.as_bytes()) {
                        tracing::debug!("Failed to write master.key: {e}");
                        return false;
                    }
                    tracing::debug!(
                        "File permissions are not restricted on this platform; \
                         consider using SECRETS_MASTER_KEY env var for production"
                    );
                    true
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    tracing::debug!(
                        "master.key already exists (concurrent creation); loading existing"
                    );
                    false
                }
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
    use std::sync::{Mutex, OnceLock};

    use super::*;
    use crate::settings::KeySource;

    /// Serialize all tests in this module that mutate `SECRETS_MASTER_KEY`.
    /// `std::env::set_var`/`remove_var` are process-global, so without
    /// this lock `cargo test`'s parallel scheduler races and flakes the
    /// env-guarded tests.
    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    /// Deterministic keystore stub for caller-level tests. Lets each test
    /// inject a specific response (present / NotFound / transient
    /// Unavailable) and observes whether the resolver attempts to
    /// persist a newly-generated key.
    struct TestKeystore {
        /// Response to return from `get_master_key`.
        get_response: Mutex<Result<Vec<u8>, KeystoreError>>,
        /// If present, `store_master_key` records the stored bytes here.
        /// Tests assert on this to detect silent rotation.
        stored: Mutex<Option<Vec<u8>>>,
        /// If true, `store_master_key` fails with Unavailable.
        store_fails: bool,
    }

    impl TestKeystore {
        fn new(get_response: Result<Vec<u8>, KeystoreError>) -> Self {
            Self {
                get_response: Mutex::new(get_response),
                stored: Mutex::new(None),
                store_fails: false,
            }
        }

        fn with_store_failing(mut self) -> Self {
            self.store_fails = true;
            self
        }

        fn stored_key(&self) -> Option<Vec<u8>> {
            self.stored.lock().ok().and_then(|g| g.clone())
        }
    }

    #[async_trait::async_trait]
    impl Keystore for TestKeystore {
        async fn get_master_key(&self) -> Result<Vec<u8>, KeystoreError> {
            let guard = self
                .get_response
                .lock()
                .map_err(|_| KeystoreError::Unavailable {
                    reason: "test mutex poisoned".to_string(),
                })?;
            guard.clone()
        }

        async fn store_master_key(&self, key: &[u8]) -> Result<(), KeystoreError> {
            if self.store_fails {
                return Err(KeystoreError::Unavailable {
                    reason: "test: store unavailable".to_string(),
                });
            }
            if let Ok(mut guard) = self.stored.lock() {
                *guard = Some(key.to_vec());
            }
            Ok(())
        }
    }

    /// Guard that clears `SECRETS_MASTER_KEY` for the duration of a
    /// test so env-var bleed-through from the host cannot mask bugs in
    /// the keystore fallback path. Restores on drop.
    struct EnvGuard {
        prior: Option<String>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl EnvGuard {
        fn new() -> Self {
            let lock = env_lock().lock().unwrap_or_else(|e| e.into_inner());
            let prior = std::env::var("SECRETS_MASTER_KEY").ok();
            // SAFETY: tests are single-threaded per `#[tokio::test]` task
            // but env mutation is process-global. The `cargo test`
            // harness may run these concurrently with other tests that
            // also touch SECRETS_MASTER_KEY. We accept that risk here
            // because (a) no other test in this module touches this var
            // and (b) we only need it cleared for the body of one test.
            unsafe {
                std::env::remove_var("SECRETS_MASTER_KEY");
            }
            Self { prior, _lock: lock }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // SAFETY: see `EnvGuard::new`.
            unsafe {
                if let Some(ref v) = self.prior {
                    std::env::set_var("SECRETS_MASTER_KEY", v);
                } else {
                    std::env::remove_var("SECRETS_MASTER_KEY");
                }
            }
        }
    }

    // ========================================================================
    // Caller-level tests: drive SecretsConfig::resolve_with_keystore through
    // all the branches that a real startup would hit. These cover the
    // data-loss bug addressed in PR #2312: a transient keychain outage must
    // NEVER silently rotate the master key.
    // ========================================================================

    #[tokio::test]
    async fn first_boot_no_key_anywhere_generates_and_persists() {
        let _env = EnvGuard::new();
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("master.key");

        let keystore = TestKeystore::new(Err(KeystoreError::NotFound));

        let cfg = SecretsConfig::resolve_with_keystore(&keystore, &file_path, true)
            .await
            .expect("first boot should succeed");

        assert!(
            cfg.master_key.is_some(),
            "should generate a key on first boot"
        );
        assert_eq!(cfg.source, KeySource::Keychain);
        // Generated key must have been persisted to the keystore.
        let stored = keystore
            .stored_key()
            .expect("keystore should have been written");
        assert_eq!(stored.len(), 32, "32-byte AES-256 key");
    }

    #[tokio::test]
    async fn key_already_in_keychain_is_reused_not_rotated() {
        let _env = EnvGuard::new();
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("master.key");

        // Pre-existing key in the keychain.
        let existing = vec![0x42u8; 32];
        let keystore = TestKeystore::new(Ok(existing.clone()));

        let cfg = SecretsConfig::resolve_with_keystore(&keystore, &file_path, true)
            .await
            .expect("key reuse should succeed");

        let expected_hex: String = existing.iter().map(|b| format!("{:02x}", b)).collect();
        assert_eq!(
            cfg.master_key
                .as_ref()
                .map(|k| k.expose_secret().to_string()),
            Some(expected_hex)
        );
        assert_eq!(cfg.source, KeySource::Keychain);
        // Reuse path must NOT call store_master_key.
        assert!(
            keystore.stored_key().is_none(),
            "must not overwrite an existing key"
        );
    }

    #[tokio::test]
    async fn notfound_with_file_fallback_present_loads_from_file() {
        let _env = EnvGuard::new();
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("master.key");

        let file_key_hex = "a".repeat(64);
        std::fs::write(&file_path, &file_key_hex).unwrap();

        let keystore = TestKeystore::new(Err(KeystoreError::NotFound));

        let cfg = SecretsConfig::resolve_with_keystore(&keystore, &file_path, true)
            .await
            .expect("file fallback should succeed");

        assert_eq!(
            cfg.master_key
                .as_ref()
                .map(|k| k.expose_secret().to_string()),
            Some(file_key_hex)
        );
        assert_eq!(cfg.source, KeySource::File);
        assert!(
            keystore.stored_key().is_none(),
            "must not regenerate when a file-based key exists"
        );
    }

    #[tokio::test]
    async fn transient_keystore_failure_with_existing_file_loads_file() {
        let _env = EnvGuard::new();
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("master.key");

        // The file-based fallback already holds a key.
        let file_key_hex = "b".repeat(64);
        std::fs::write(&file_path, &file_key_hex).unwrap();

        // Keychain is transiently broken (locked, dbus down, etc).
        let keystore = TestKeystore::new(Err(KeystoreError::Unavailable {
            reason: "secret-service unavailable".to_string(),
        }));

        let cfg = SecretsConfig::resolve_with_keystore(&keystore, &file_path, true)
            .await
            .expect("transient failure with existing file should succeed");

        // MUST use the existing file key -- the bug this test guards
        // against is silently regenerating and losing previously
        // encrypted secrets.
        assert_eq!(
            cfg.master_key
                .as_ref()
                .map(|k| k.expose_secret().to_string()),
            Some(file_key_hex)
        );
        assert_eq!(cfg.source, KeySource::File);
        assert!(
            keystore.stored_key().is_none(),
            "must NOT regenerate or overwrite on transient failure"
        );
    }

    #[tokio::test]
    async fn transient_keystore_failure_with_no_existing_key_errors() {
        let _env = EnvGuard::new();
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("master.key");

        let keystore = TestKeystore::new(Err(KeystoreError::Unavailable {
            reason: "keychain locked".to_string(),
        }));

        let result = SecretsConfig::resolve_with_keystore(&keystore, &file_path, true).await;

        // Must propagate the error. Silently generating a new key here
        // is the data-loss bug: once the keychain comes back online with
        // a pre-existing key, any secrets encrypted under the silently-
        // generated replacement are stranded.
        match result {
            Err(ConfigError::InvalidValue { key, message }) => {
                assert_eq!(key, "SECRETS_MASTER_KEY");
                assert!(message.contains("unavailable"));
            }
            other => panic!("expected InvalidValue error, got {other:?}"),
        }
        // Must not have written a new key to the file fallback either.
        assert!(
            !file_path.exists(),
            "must NOT create a new master.key file on transient failure"
        );
        assert!(
            keystore.stored_key().is_none(),
            "must NOT write a new key to the keystore on transient failure"
        );
    }

    #[tokio::test]
    async fn env_var_override_bypasses_keystore_even_on_transient_failure() {
        // When SECRETS_MASTER_KEY is set, the env var wins unconditionally.
        // This is the CI/Docker escape hatch.
        let _env = EnvGuard::new();
        // SAFETY: single env var write, guarded by EnvGuard::drop.
        unsafe {
            std::env::set_var("SECRETS_MASTER_KEY", "c".repeat(64));
        }

        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("master.key");

        let keystore = TestKeystore::new(Err(KeystoreError::Unavailable {
            reason: "should not be consulted".to_string(),
        }));

        let cfg = SecretsConfig::resolve_with_keystore(&keystore, &file_path, true)
            .await
            .expect("env override should succeed even with a broken keystore");

        assert_eq!(cfg.source, KeySource::Env);
        assert!(cfg.master_key.is_some());
    }

    #[tokio::test]
    async fn first_boot_keystore_persist_denied_falls_back_to_file() {
        // Keystore reports NotFound on read but rejects writes (e.g.
        // macOS keychain in read-only mode). Must persist the newly
        // generated key to the file fallback instead.
        let _env = EnvGuard::new();
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("master.key");

        let keystore = TestKeystore::new(Err(KeystoreError::NotFound)).with_store_failing();

        let cfg = SecretsConfig::resolve_with_keystore(&keystore, &file_path, true)
            .await
            .expect("file fallback persist should succeed");

        assert_eq!(cfg.source, KeySource::File);
        assert!(
            file_path.exists(),
            "master.key file should have been written"
        );
    }

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

    #[test]
    fn test_load_master_key_rejects_oversized_key() {
        // 128 hex chars = 64 bytes, but AES-256 requires exactly 32 bytes = 64 hex chars.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("master.key");
        let oversized = "a".repeat(128);
        std::fs::write(&path, &oversized).unwrap();

        let loaded = SecretsConfig::load_master_key_from_path(&path);
        assert!(
            loaded.is_none(),
            "128 hex chars (64 bytes) should be rejected; only exactly 64 hex chars allowed"
        );
    }

    #[test]
    fn test_concurrent_first_boot_create_new_exclusivity() {
        // Simulate two processes racing to create the master.key file.
        // Only the first writer should succeed; the second gets AlreadyExists.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("master.key");

        let key_a = "a".repeat(64);
        let key_b = "b".repeat(64);

        // First writer wins.
        assert!(
            SecretsConfig::save_master_key_to_path(&key_a, &path),
            "first writer should succeed"
        );
        // Second writer should fail (create_new returns AlreadyExists).
        assert!(
            !SecretsConfig::save_master_key_to_path(&key_b, &path),
            "second writer should fail due to create_new exclusivity"
        );

        // The file should contain the first writer's key, not the second's.
        let loaded = SecretsConfig::load_master_key_from_path(&path).unwrap();
        assert_eq!(loaded, key_a, "first writer's key must survive the race");
    }
}
