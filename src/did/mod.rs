//! Persistent instance DID support.

pub mod did_key;
pub mod document;
mod store;

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

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
}

/// Stable instance identity for the current IronClaw installation.
#[derive(Debug, Clone)]
pub struct InstanceIdentity {
    did: String,
    public_key_multibase: String,
    created_at: DateTime<Utc>,
    secret_key: [u8; 32],
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
            secret_key,
        }
    }

    fn from_stored(stored: store::StoredInstanceIdentity) -> Result<Self, DidError> {
        if stored.method != did_key::DID_KEY_METHOD {
            return Err(DidError::UnsupportedMethod(stored.method));
        }

        let raw = hex::decode(&stored.secret_key_hex)
            .map_err(|e| DidError::InvalidSecretKey(e.to_string()))?;
        let secret_key: [u8; 32] = raw
            .try_into()
            .map_err(|_| DidError::InvalidSecretKey("expected 32 bytes".to_string()))?;

        Ok(Self::from_secret_key(secret_key, stored.created_at))
    }

    fn to_stored(&self) -> store::StoredInstanceIdentity {
        store::StoredInstanceIdentity::new(
            did_key::DID_KEY_METHOD,
            self.created_at,
            hex::encode(self.secret_key),
        )
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
    store::load(path)?
        .map(InstanceIdentity::from_stored)
        .transpose()
}

/// Load an identity from an arbitrary path, creating it if missing.
pub fn load_or_create_at(path: &Path) -> Result<InstanceIdentity, DidError> {
    if let Some(existing) = load_at(path)? {
        return Ok(existing);
    }

    let identity = InstanceIdentity::generate();
    store::save(path, &identity.to_stored())?;
    Ok(identity)
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
}
