//! OMEMO key and session persistence.
//!
//! Stores identity keys, prekeys, and per-device ratchet sessions as JSON
//! files under the configured `omemo_store_dir`.
//!
//! All writes are atomic: data is written to a `.tmp` file then renamed.
#![allow(dead_code)]

use secrecy::{ExposeSecret, SecretBox};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;
use tokio::fs;

#[derive(Debug, Error)]
pub enum OmemoStoreError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Key not found: {0}")]
    KeyNotFound(String),
}

/// Serializable form of IdentityBundle (keys as hex strings for JSON).
#[derive(Serialize, Deserialize)]
struct IdentityBundleDisk {
    device_id: u32,
    ik_priv: String,  // hex
    ik_pub: String,   // hex
    spk_priv: String, // hex
    spk_pub: String,  // hex
    spk_sig: String,  // hex (64 bytes)
    spk_id: u32,
}

/// The bot's own OMEMO identity: identity key pair + signed prekey pair.
pub struct IdentityBundle {
    pub device_id: u32,
    pub ik_priv: SecretBox<[u8; 32]>,
    /// X25519 Diffie-Hellman public key.
    pub ik_pub: [u8; 32],
    /// Ed25519 verifying key derived from `ik_priv` via HKDF — used to verify SPK signatures.
    pub ik_sig_pub: [u8; 32],
    pub spk_priv: SecretBox<[u8; 32]>,
    pub spk_pub: [u8; 32],
    pub spk_sig: [u8; 64],
    pub spk_id: u32,
}

/// A one-time prekey.
#[derive(Clone, Serialize, Deserialize)]
pub struct PreKey {
    pub id: u32,
    pub priv_key: String, // hex (stored as hex for JSON)
    pub pub_key: String,  // hex
}

impl PreKey {
    pub fn priv_bytes(&self) -> Result<[u8; 32], OmemoStoreError> {
        let bytes = hex::decode(&self.priv_key)
            .map_err(|e| OmemoStoreError::KeyNotFound(format!("hex decode: {e}")))?;
        let mut arr = [0u8; 32];
        if bytes.len() != 32 {
            return Err(OmemoStoreError::KeyNotFound(
                "prekey priv not 32 bytes".into(),
            ));
        }
        arr.copy_from_slice(&bytes);
        Ok(arr)
    }

    pub fn pub_bytes(&self) -> Result<[u8; 32], OmemoStoreError> {
        let bytes = hex::decode(&self.pub_key)
            .map_err(|e| OmemoStoreError::KeyNotFound(format!("hex decode: {e}")))?;
        let mut arr = [0u8; 32];
        if bytes.len() != 32 {
            return Err(OmemoStoreError::KeyNotFound(
                "prekey pub not 32 bytes".into(),
            ));
        }
        arr.copy_from_slice(&bytes);
        Ok(arr)
    }
}

pub struct OmemoStore {
    dir: PathBuf,
}

impl OmemoStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Atomically write bytes to path via temp file.
    async fn atomic_write(path: &Path, data: &[u8]) -> Result<(), OmemoStoreError> {
        let tmp = path.with_extension("tmp");
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::write(&tmp, data).await?;
        fs::rename(&tmp, path).await?;
        Ok(())
    }

    /// Load or generate identity bundle. If device_id is 0, a random one is generated.
    pub async fn load_or_init_identity(
        &self,
        device_id: u32,
    ) -> Result<IdentityBundle, OmemoStoreError> {
        let path = self.dir.join("identity_key.json");
        if path.exists() {
            let data = fs::read_to_string(&path).await?;
            let disk: IdentityBundleDisk = serde_json::from_str(&data)?;
            return Self::bundle_from_disk(disk);
        }
        // Generate new identity
        let mut rng = rand::thread_rng();
        use rand::RngCore;
        let did = if device_id == 0 {
            rng.next_u32()
        } else {
            device_id
        };
        let ik_priv_bytes = {
            let secret = x25519_dalek::StaticSecret::random_from_rng(&mut rng);
            *secret.as_bytes()
        };
        let ik_pub_bytes =
            x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from(ik_priv_bytes))
                .to_bytes();
        let spk_priv_bytes = {
            let secret = x25519_dalek::StaticSecret::random_from_rng(&mut rng);
            *secret.as_bytes()
        };
        let spk_pub_bytes =
            x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from(spk_priv_bytes))
                .to_bytes();
        let spk_id = rng.next_u32();

        // Sign spk_pub with Ed25519 key derived from ik_priv via HKDF
        let signing_key = Self::derive_signing_key(&ik_priv_bytes);
        let ik_sig_pub = signing_key.verifying_key().to_bytes();
        use ed25519_dalek::Signer;
        let sig = signing_key.sign(&spk_pub_bytes);
        let spk_sig = sig.to_bytes();

        let bundle = IdentityBundle {
            device_id: did,
            ik_priv: SecretBox::new(Box::new(ik_priv_bytes)),
            ik_pub: ik_pub_bytes,
            ik_sig_pub,
            spk_priv: SecretBox::new(Box::new(spk_priv_bytes)),
            spk_pub: spk_pub_bytes,
            spk_sig,
            spk_id,
        };
        self.save_identity(&bundle).await?;
        Ok(bundle)
    }

    /// Derive an Ed25519 signing key from an X25519 private key using HKDF.
    pub fn derive_signing_key(ik_priv: &[u8; 32]) -> ed25519_dalek::SigningKey {
        use hkdf::Hkdf;
        use sha2::Sha256;
        let hk = Hkdf::<Sha256>::new(None, ik_priv);
        let mut okm = [0u8; 32];
        hk.expand(b"OMEMO identity signing key", &mut okm)
            .expect("HKDF expand for signing key");
        ed25519_dalek::SigningKey::from_bytes(&okm)
    }

    fn bundle_from_disk(d: IdentityBundleDisk) -> Result<IdentityBundle, OmemoStoreError> {
        let decode32 = |s: &str, name: &str| -> Result<[u8; 32], OmemoStoreError> {
            let bytes = hex::decode(s)
                .map_err(|e| OmemoStoreError::KeyNotFound(format!("{name} hex: {e}")))?;
            if bytes.len() != 32 {
                return Err(OmemoStoreError::KeyNotFound(format!(
                    "{name} wrong length"
                )));
            }
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            Ok(arr)
        };
        let decode64 = |s: &str, name: &str| -> Result<[u8; 64], OmemoStoreError> {
            let bytes = hex::decode(s)
                .map_err(|e| OmemoStoreError::KeyNotFound(format!("{name} hex: {e}")))?;
            if bytes.len() != 64 {
                return Err(OmemoStoreError::KeyNotFound(format!(
                    "{name} wrong length"
                )));
            }
            let mut arr = [0u8; 64];
            arr.copy_from_slice(&bytes);
            Ok(arr)
        };
        let ik_priv_bytes = decode32(&d.ik_priv, "ik_priv")?;
        let ik_sig_pub = Self::derive_signing_key(&ik_priv_bytes)
            .verifying_key()
            .to_bytes();
        Ok(IdentityBundle {
            device_id: d.device_id,
            ik_priv: SecretBox::new(Box::new(ik_priv_bytes)),
            ik_pub: decode32(&d.ik_pub, "ik_pub")?,
            ik_sig_pub,
            spk_priv: SecretBox::new(Box::new(decode32(&d.spk_priv, "spk_priv")?)),
            spk_pub: decode32(&d.spk_pub, "spk_pub")?,
            spk_sig: decode64(&d.spk_sig, "spk_sig")?,
            spk_id: d.spk_id,
        })
    }

    pub async fn save_identity(&self, bundle: &IdentityBundle) -> Result<(), OmemoStoreError> {
        let disk = IdentityBundleDisk {
            device_id: bundle.device_id,
            ik_priv: hex::encode(bundle.ik_priv.expose_secret()),
            ik_pub: hex::encode(bundle.ik_pub),
            spk_priv: hex::encode(bundle.spk_priv.expose_secret()),
            spk_pub: hex::encode(bundle.spk_pub),
            spk_sig: hex::encode(bundle.spk_sig),
            spk_id: bundle.spk_id,
        };
        let path = self.dir.join("identity_key.json");
        let data = serde_json::to_vec_pretty(&disk)?;
        Self::atomic_write(&path, &data).await
    }

    pub async fn load_prekeys(&self) -> Result<Vec<PreKey>, OmemoStoreError> {
        let dir = self.dir.join("prekeys");
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut entries = fs::read_dir(&dir).await?;
        let mut keys = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            let data = fs::read_to_string(entry.path()).await?;
            let pk: PreKey = serde_json::from_str(&data)?;
            keys.push(pk);
        }
        Ok(keys)
    }

    pub async fn save_prekey(&self, pk: &PreKey) -> Result<(), OmemoStoreError> {
        let path = self
            .dir
            .join("prekeys")
            .join(format!("{}.json", pk.id));
        let data = serde_json::to_vec_pretty(pk)?;
        Self::atomic_write(&path, &data).await
    }

    /// Remove prekey from disk (used when consumed in X3DH).
    pub async fn consume_prekey(&self, id: u32) -> Result<PreKey, OmemoStoreError> {
        let path = self
            .dir
            .join("prekeys")
            .join(format!("{id}.json"));
        let data = fs::read_to_string(&path)
            .await
            .map_err(|_| OmemoStoreError::KeyNotFound(format!("prekey {id}")))?;
        let pk: PreKey = serde_json::from_str(&data)?;
        fs::remove_file(&path).await?;
        Ok(pk)
    }

    pub async fn load_session(
        &self,
        jid: &str,
        device_id: u32,
    ) -> Result<
        Option<crate::channels::xmpp::omemo::ratchet::RatchetState>,
        OmemoStoreError,
    > {
        let safe_jid = jid.replace('/', "_").replace('@', "_at_");
        let path = self
            .dir
            .join("sessions")
            .join(&safe_jid)
            .join(format!("{device_id}.json"));
        if !path.exists() {
            return Ok(None);
        }
        let data = fs::read_to_string(&path).await?;
        let state = serde_json::from_str(&data)?;
        Ok(Some(state))
    }

    pub async fn save_session(
        &self,
        jid: &str,
        device_id: u32,
        state: &crate::channels::xmpp::omemo::ratchet::RatchetState,
    ) -> Result<(), OmemoStoreError> {
        let safe_jid = jid.replace('/', "_").replace('@', "_at_");
        let path = self
            .dir
            .join("sessions")
            .join(&safe_jid)
            .join(format!("{device_id}.json"));
        let data = serde_json::to_vec_pretty(state)?;
        Self::atomic_write(&path, &data).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_store_roundtrip_identity() {
        let dir = TempDir::new().unwrap();
        let store = OmemoStore::new(dir.path().to_path_buf());
        let bundle = store.load_or_init_identity(0).await.unwrap();
        assert_ne!(bundle.device_id, 0);
        // Load again — should return same identity
        let bundle2 = store.load_or_init_identity(0).await.unwrap();
        assert_eq!(bundle.device_id, bundle2.device_id);
        assert_eq!(bundle.ik_pub, bundle2.ik_pub);
    }

    #[tokio::test]
    async fn test_prekey_consume() {
        let dir = TempDir::new().unwrap();
        let store = OmemoStore::new(dir.path().to_path_buf());
        let pk = PreKey {
            id: 42,
            priv_key: hex::encode([1u8; 32]),
            pub_key: hex::encode([2u8; 32]),
        };
        store.save_prekey(&pk).await.unwrap();
        let loaded = store.consume_prekey(42).await.unwrap();
        assert_eq!(loaded.id, 42);
        // Second consume should fail
        assert!(store.consume_prekey(42).await.is_err());
    }
}
