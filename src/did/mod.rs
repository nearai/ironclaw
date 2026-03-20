//! Persistent instance DID support.

pub mod did_key;
pub mod document;
mod store;

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use fs4::FileExt;
use zeroize::Zeroizing;

pub use document::DidDocument;

/// Errors from instance DID loading or persistence.
#[derive(Debug, thiserror::Error)]
pub enum DidError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Unsupported DID method in stored identity: {0}")]
    UnsupportedMethod(String),

    #[error("Invalid secret key encoding: {0}")]
    InvalidSecretKey(String),

    #[error("Secret encryption error: {0}")]
    Secret(#[from] crate::secrets::SecretError),
}

/// Stable instance identity for the current IronClaw installation.
#[derive(Debug)]
pub struct InstanceIdentity {
    did: String,
    public_key_multibase: String,
    created_at: DateTime<Utc>,
    secret_key: Zeroizing<[u8; 32]>,
}

impl InstanceIdentity {
    fn generate() -> Self {
        let secret_key = did_key::generate_secret_key();
        Self::from_secret_key(secret_key, Utc::now())
    }

    pub(crate) fn from_secret_key(secret_key: [u8; 32], created_at: DateTime<Utc>) -> Self {
        let public_key_multibase = did_key::public_key_multibase(&secret_key);
        let did = did_key::did_from_public_key_multibase(&public_key_multibase);
        Self {
            did,
            public_key_multibase,
            created_at,
            secret_key: Zeroizing::new(secret_key),
        }
    }

    fn from_stored_at(
        path: &Path,
        stored: store::StoredInstanceIdentity,
    ) -> Result<Self, DidError> {
        if stored.method != did_key::DID_KEY_METHOD {
            return Err(DidError::UnsupportedMethod(stored.method));
        }

        let secret_key = store::decrypt_secret_key(path, &stored)?;
        Ok(Self::from_secret_key(secret_key, stored.created_at))
    }

    fn to_stored(&self, path: &Path) -> Result<store::StoredInstanceIdentity, DidError> {
        let (encrypted_secret_key_hex, key_salt_hex) =
            store::encrypt_secret_key(path, &self.secret_key)?;
        Ok(store::StoredInstanceIdentity::new_encrypted(
            did_key::DID_KEY_METHOD,
            self.created_at,
            encrypted_secret_key_hex,
            key_salt_hex,
        ))
    }

    /// The DID string for this instance.
    pub fn did(&self) -> &str {
        &self.did
    }

    /// The DID method in use.
    pub fn method(&self) -> &'static str {
        did_key::DID_KEY_METHOD
    }

    /// Creation timestamp for the current identity.
    pub fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }

    /// Verification method ID for the current key.
    pub fn key_id(&self) -> String {
        did_key::key_id(&self.did, &self.public_key_multibase)
    }

    /// DID document for the current identity.
    pub fn document(&self) -> DidDocument {
        document::did_key_document(&self.did, &self.public_key_multibase)
    }
}

/// Default on-disk path for the instance identity.
pub fn default_identity_path() -> PathBuf {
    store::default_identity_path()
}

/// Load the default identity if it exists.
pub fn load_default() -> Result<Option<InstanceIdentity>, DidError> {
    load_at(&default_identity_path())
}

/// Load or create the default identity.
pub fn load_or_create_default() -> Result<InstanceIdentity, DidError> {
    load_or_create_at(&default_identity_path())
}

/// Load an identity from an arbitrary path if it exists.
pub fn load_at(path: &Path) -> Result<Option<InstanceIdentity>, DidError> {
    match store::load(path)? {
        Some(stored) => Ok(Some(InstanceIdentity::from_stored_at(path, stored)?)),
        None => Ok(None),
    }
}

/// Load an identity from an arbitrary path, creating it if missing.
pub fn load_or_create_at(path: &Path) -> Result<InstanceIdentity, DidError> {
    let _lock = DidFileLock::acquire(path)?;

    if let Some(stored) = store::load(path)? {
        let identity = InstanceIdentity::from_stored_at(path, stored.clone())?;
        if stored.is_legacy_plaintext() {
            store::save(path, &identity.to_stored(path)?)?;
        }
        return Ok(identity);
    }

    let identity = InstanceIdentity::generate();
    match store::save_new(path, &identity.to_stored(path)?) {
        Ok(()) => Ok(identity),
        Err(DidError::Io(err)) if err.kind() == std::io::ErrorKind::AlreadyExists => load_at(path)?
            .ok_or_else(|| {
                DidError::InvalidSecretKey(
                    "identity file created concurrently but could not be loaded".to_string(),
                )
            }),
        Err(err) => Err(err),
    }
}

struct DidFileLock {
    _file: std::fs::File,
}

impl DidFileLock {
    fn acquire(identity_path: &Path) -> Result<Self, DidError> {
        let lock_path = identity_path.with_extension("lock");
        if let Some(parent) = lock_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)?;
        file.lock_exclusive()?;
        Ok(Self { _file: file })
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn load_or_create_persists_identity() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("identity.json");

        let first = load_or_create_at(&path).expect("create identity");
        let second = load_or_create_at(&path).expect("load identity");

        assert_eq!(first.did(), second.did());
        assert_eq!(first.key_id(), second.key_id());
    }

    #[test]
    fn did_document_matches_identity() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("identity.json");
        let identity = load_or_create_at(&path).expect("identity");
        let document = identity.document();

        assert_eq!(document.id, identity.did());
        assert_eq!(document.authentication[0], identity.key_id());
    }

    #[test]
    fn concurrent_load_or_create_converges_on_one_identity() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("identity.json");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let mut handles = Vec::new();

        for _ in 0..8 {
            let barrier = std::sync::Arc::clone(&barrier);
            let path = path.clone();
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                load_or_create_at(&path)
                    .expect("identity")
                    .did()
                    .to_string()
            }));
        }

        let dids: std::collections::HashSet<_> = handles
            .into_iter()
            .map(|handle| handle.join().expect("thread"))
            .collect();
        assert_eq!(dids.len(), 1);
    }

    #[test]
    fn load_or_create_migrates_legacy_plaintext_identity() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("identity.json");
        let legacy = store::StoredInstanceIdentity::legacy_plaintext(
            did_key::DID_KEY_METHOD,
            Utc::now(),
            "33".repeat(32),
        );
        store::save(&path, &legacy).expect("save legacy");

        let identity = load_or_create_at(&path).expect("migrate identity");
        assert!(identity.did().starts_with("did:key:"));

        let stored = store::load(&path)
            .expect("load stored")
            .expect("stored identity");
        assert!(stored.secret_key_hex.is_none());
        assert!(stored.encrypted_secret_key_hex.is_some());
        assert!(stored.key_salt_hex.is_some());
    }
}
