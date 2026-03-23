//! Filesystem persistence for instance DID state.

use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::bootstrap::ironclaw_base_dir;
use crate::secrets::{SecretsCrypto, crypto_from_hex, keychain};

use super::DidError;

/// On-disk representation of the instance identity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StoredInstanceIdentity {
    pub version: u8,
    pub method: String,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub secret_key_hex: Option<String>,
    #[serde(default)]
    pub encrypted_secret_key_hex: Option<String>,
    #[serde(default)]
    pub key_salt_hex: Option<String>,
}

impl StoredInstanceIdentity {
    pub fn new_encrypted(
        method: &str,
        created_at: DateTime<Utc>,
        encrypted_secret_key_hex: String,
        key_salt_hex: String,
    ) -> Self {
        Self {
            version: 2,
            method: method.to_string(),
            created_at,
            secret_key_hex: None,
            encrypted_secret_key_hex: Some(encrypted_secret_key_hex),
            key_salt_hex: Some(key_salt_hex),
        }
    }

    #[cfg(test)]
    pub fn legacy_plaintext(
        method: &str,
        created_at: DateTime<Utc>,
        secret_key_hex: String,
    ) -> Self {
        Self {
            version: 1,
            method: method.to_string(),
            created_at,
            secret_key_hex: Some(secret_key_hex),
            encrypted_secret_key_hex: None,
            key_salt_hex: None,
        }
    }

    pub fn is_legacy_plaintext(&self) -> bool {
        self.secret_key_hex.is_some()
    }
}

/// Default on-disk path for the instance identity.
pub fn default_identity_path() -> PathBuf {
    ironclaw_base_dir().join("identity").join("instance.json")
}

pub(crate) fn identity_master_key_path(identity_path: &Path) -> PathBuf {
    identity_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("master.key")
}

pub(crate) fn load(path: &Path) -> Result<Option<StoredInstanceIdentity>, DidError> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err.into()),
    };

    let stored = serde_json::from_str(&content)?;
    Ok(Some(stored))
}

pub(crate) fn save(path: &Path, stored: &StoredInstanceIdentity) -> Result<(), DidError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let content = serde_json::to_vec_pretty(stored)?;
    std::fs::write(path, content)?;
    restrict_file_permissions(path)?;
    Ok(())
}

pub(crate) fn save_new(path: &Path, stored: &StoredInstanceIdentity) -> Result<(), DidError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let content = serde_json::to_vec_pretty(stored)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(&content)?;
    file.sync_all()?;
    drop(file);
    restrict_file_permissions(path)?;
    Ok(())
}

pub(crate) fn encrypt_secret_key(
    identity_path: &Path,
    secret_key: &[u8; 32],
) -> Result<(String, String), DidError> {
    let crypto = identity_crypto(identity_path)?;
    let secret_key_hex = hex::encode(secret_key);
    let (encrypted, salt) = crypto.encrypt(secret_key_hex.as_bytes())?;
    Ok((hex::encode(encrypted), hex::encode(salt)))
}

pub(crate) fn decrypt_secret_key(
    identity_path: &Path,
    stored: &StoredInstanceIdentity,
) -> Result<[u8; 32], DidError> {
    if let Some(secret_key_hex) = &stored.secret_key_hex {
        return decode_secret_key_hex(secret_key_hex);
    }

    let encrypted_hex = stored
        .encrypted_secret_key_hex
        .as_deref()
        .ok_or_else(|| DidError::InvalidSecretKey("missing encrypted secret key".to_string()))?;
    let salt_hex = stored
        .key_salt_hex
        .as_deref()
        .ok_or_else(|| DidError::InvalidSecretKey("missing key salt".to_string()))?;

    let encrypted = hex::decode(encrypted_hex)
        .map_err(|e| DidError::InvalidSecretKey(format!("invalid encrypted secret key: {e}")))?;
    let salt = hex::decode(salt_hex)
        .map_err(|e| DidError::InvalidSecretKey(format!("invalid key salt: {e}")))?;
    let crypto = identity_crypto(identity_path)?;
    let decrypted = crypto.decrypt(&encrypted, &salt)?;
    decode_secret_key_hex(decrypted.expose())
}

fn decode_secret_key_hex(secret_key_hex: &str) -> Result<[u8; 32], DidError> {
    let raw = hex::decode(secret_key_hex).map_err(|e| DidError::InvalidSecretKey(e.to_string()))?;
    raw.try_into()
        .map_err(|_| DidError::InvalidSecretKey("expected 32 bytes".to_string()))
}

fn identity_crypto(identity_path: &Path) -> Result<std::sync::Arc<SecretsCrypto>, DidError> {
    let key_hex = match std::env::var("SECRETS_MASTER_KEY") {
        Ok(key) if !key.is_empty() => key,
        _ => load_or_create_master_key(&identity_master_key_path(identity_path))?,
    };

    Ok(crypto_from_hex(&key_hex)?)
}

fn load_or_create_master_key(path: &Path) -> Result<String, DidError> {
    match std::fs::read_to_string(path) {
        Ok(key) => return Ok(key.trim().to_string()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err.into()),
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let generated = keychain::generate_master_key_hex();
    match write_new_restricted(path, generated.as_bytes()) {
        Ok(()) => Ok(generated),
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            Ok(std::fs::read_to_string(path)?.trim().to_string())
        }
        Err(err) => Err(err.into()),
    }
}

fn write_new_restricted(path: &Path, content: &[u8]) -> std::io::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(content)?;
    file.sync_all()?;
    drop(file);
    restrict_file_permissions(path)?;
    Ok(())
}

#[cfg(unix)]
fn restrict_file_permissions(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(0o600);
    std::fs::set_permissions(path, perms)
}

#[cfg(not(unix))]
fn restrict_file_permissions(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn save_and_load_round_trip() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("instance.json");
        let (encrypted, salt) = encrypt_secret_key(&path, &[0x11; 32]).expect("encrypt");
        let stored = StoredInstanceIdentity::new_encrypted("did:key", Utc::now(), encrypted, salt);

        save(&path, &stored).expect("save");
        let loaded = load(&path).expect("load").expect("stored identity");

        assert_eq!(loaded.method, "did:key");
        assert!(loaded.secret_key_hex.is_none());
        let decrypted = decrypt_secret_key(&path, &loaded).expect("decrypt");
        assert_eq!(decrypted, [0x11; 32]);
    }

    #[cfg(unix)]
    #[test]
    fn stored_identity_file_is_0600() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("instance.json");
        let (encrypted, salt) = encrypt_secret_key(&path, &[0x22; 32]).expect("encrypt");
        let stored = StoredInstanceIdentity::new_encrypted("did:key", Utc::now(), encrypted, salt);

        save(&path, &stored).expect("save");
        let mode = std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
}
