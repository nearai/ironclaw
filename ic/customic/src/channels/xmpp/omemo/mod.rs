//! OMEMO encryption manager — high-level encrypt/decrypt API.

#![allow(dead_code)]

pub mod ratchet;
pub mod store;
pub mod x3dh;

use std::collections::HashMap;
use std::sync::Arc;

use aes_gcm::{aead::Aead, Aes256Gcm, KeyInit, Nonce};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use rand::RngCore;
use secrecy::ExposeSecret;
use thiserror::Error;
use tokio::sync::RwLock;

use crate::config::XmppConfig;
use ratchet::{ratchet_decrypt, ratchet_encrypt, RatchetState};
use store::{IdentityBundle, OmemoStore, OmemoStoreError, PreKey};
use x3dh::{x3dh_init_sender, RemoteBundle, X3dhError};

const OMEMO_NAMESPACE: &str = "eu.siacs.conversations.axolotl";
const INITIAL_PREKEY_COUNT: usize = 20;

#[derive(Debug, Error)]
pub enum OmemoError {
    #[error("Store error: {0}")]
    Store(#[from] OmemoStoreError),
    #[error("X3DH error: {0}")]
    X3dh(#[from] X3dhError),
    #[error("Ratchet error: {0}")]
    Ratchet(#[from] ratchet::RatchetError),
    #[error("XML error: {0}")]
    Xml(String),
    #[error("No OMEMO bundle available for {0}")]
    NoBundleAvailable(String),
    #[error("Decryption failed: no matching key for our device")]
    NoKeyForDevice,
    #[error("Base64 error: {0}")]
    Base64(String),
    #[error("AEAD error")]
    Aead,
}

/// Session key: "bare_jid/device_id"
fn session_key(jid: &str, device_id: u32) -> String {
    format!("{jid}/{device_id}")
}

pub struct OmemoManager {
    store: Arc<OmemoStore>,
    identity: Arc<RwLock<IdentityBundle>>,
    sessions: Arc<RwLock<HashMap<String, RatchetState>>>,
}

impl OmemoManager {
    pub async fn new(config: &XmppConfig) -> Result<Self, OmemoError> {
        let store = Arc::new(OmemoStore::new(config.omemo_store_dir.clone()));
        let identity = store.load_or_init_identity(config.device_id).await?;

        // Ensure we have prekeys
        let existing = store.load_prekeys().await?;
        if existing.is_empty() {
            for i in 0..INITIAL_PREKEY_COUNT {
                let priv_key = x25519_dalek::StaticSecret::random_from_rng(rand::thread_rng());
                let pub_key = x25519_dalek::PublicKey::from(&priv_key);
                let pk = PreKey {
                    id: (i + 1) as u32,
                    priv_key: hex::encode(priv_key.as_bytes()),
                    pub_key: hex::encode(pub_key.to_bytes()),
                };
                store.save_prekey(&pk).await?;
            }
        }

        Ok(Self {
            store,
            identity: Arc::new(RwLock::new(identity)),
            sessions: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    pub async fn device_id(&self) -> u32 {
        self.identity.read().await.device_id
    }

    /// Returns the PEP bundle XML for publishing.
    pub async fn own_bundle_xml(&self) -> Result<String, OmemoError> {
        let id = self.identity.read().await;
        let prekeys = self.store.load_prekeys().await?;
        let mut prekey_elems = String::new();
        for pk in &prekeys {
            prekey_elems.push_str(&format!(
                r#"<preKeyPublic preKeyId='{}'>{}</preKeyPublic>"#,
                pk.id,
                B64.encode(pk.pub_bytes().map_err(OmemoError::Store)?)
            ));
        }
        Ok(format!(
            r#"<bundle xmlns='{ns}'>
  <signedPreKeyPublic signedPreKeyId='{spk_id}'>{spk_pub}</signedPreKeyPublic>
  <signedPreKeySignature>{spk_sig}</signedPreKeySignature>
  <identityKey>{ik_pub}</identityKey>
  <identitySigningKey>{ik_sig_pub}</identitySigningKey>
  <prekeys>{prekeys}</prekeys>
</bundle>"#,
            ns = OMEMO_NAMESPACE,
            spk_id = id.spk_id,
            spk_pub = B64.encode(id.spk_pub),
            spk_sig = B64.encode(id.spk_sig),
            ik_pub = B64.encode(id.ik_pub),
            ik_sig_pub = B64.encode(id.ik_sig_pub),
            prekeys = prekey_elems,
        ))
    }

    /// Returns the device list XML for publishing.
    pub async fn device_list_xml(&self) -> Result<String, OmemoError> {
        let id = self.identity.read().await;
        Ok(format!(
            r#"<list xmlns='{}'><device id='{}'/></list>"#,
            OMEMO_NAMESPACE, id.device_id
        ))
    }

    /// Encrypt plaintext for the given recipients.
    /// Each entry in `recipients` is (bare_jid, Vec<RemoteBundle>).
    /// Returns the `<encrypted>` XML string.
    pub async fn encrypt(
        &self,
        plaintext: &str,
        recipients: &[(String, Vec<RemoteBundle>)],
    ) -> Result<String, OmemoError> {
        let our_device_id = {
            let id = self.identity.read().await;
            id.device_id
        };

        // Generate random AES-256 key and 12-byte IV
        let mut aes_key = [0u8; 32];
        let mut iv = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut aes_key);
        rand::thread_rng().fill_bytes(&mut iv);

        // Encrypt plaintext with AES-256-GCM
        let cipher = Aes256Gcm::new_from_slice(&aes_key).map_err(|_| OmemoError::Aead)?;
        let nonce = Nonce::from_slice(&iv);
        let payload = cipher
            .encrypt(nonce, plaintext.as_bytes())
            .map_err(|_| OmemoError::Aead)?;

        let mut key_elems = String::new();
        let mut sessions = self.sessions.write().await;

        for (jid, bundles) in recipients {
            for bundle in bundles {
                let key = session_key(jid, bundle.device_id);
                // Get or create session
                if !sessions.contains_key(&key) {
                    // Load from store
                    let loaded = self.store.load_session(jid, bundle.device_id).await?;
                    if let Some(state) = loaded {
                        sessions.insert(key.clone(), state);
                    } else {
                        // X3DH initiation
                        let id_guard = self.identity.read().await;
                        let out = x3dh_init_sender(&id_guard, bundle)?;
                        let state = RatchetState::init_sender(out.shared_secret, bundle.spk_pub);
                        sessions.insert(key.clone(), state);
                    }
                }

                let state = sessions
                    .get_mut(&key)
                    .expect("session was just inserted");
                let (ct, _ratchet_pub, msg_n) = ratchet_encrypt(state, &aes_key)?;
                let is_prekey = msg_n == 0;
                key_elems.push_str(&format!(
                    r#"<key{}rid='{}'>{}</key>"#,
                    if is_prekey {
                        " preKeyWhisper='true' "
                    } else {
                        " "
                    },
                    bundle.device_id,
                    B64.encode(ct)
                ));
            }
        }

        // Persist sessions
        for (jid, bundles) in recipients {
            for bundle in bundles {
                let key = session_key(jid, bundle.device_id);
                if let Some(state) = sessions.get(&key) {
                    self.store
                        .save_session(jid, bundle.device_id, state)
                        .await?;
                }
            }
        }

        Ok(format!(
            r#"<encrypted xmlns='{ns}'><header sid='{sid}'>{keys}<iv>{iv}</iv></header><payload>{payload}</payload></encrypted>"#,
            ns = OMEMO_NAMESPACE,
            sid = our_device_id,
            keys = key_elems,
            iv = B64.encode(iv),
            payload = B64.encode(&payload),
        ))
    }

    /// Decrypt an incoming `<encrypted>` XML element.
    pub async fn decrypt(
        &self,
        sender_jid: &str,
        sender_device_id: u32,
        encrypted_xml: &str,
    ) -> Result<String, OmemoError> {
        let id_guard = self.identity.read().await;
        let our_device_id = id_guard.device_id;

        // Parse XML manually (simple pattern matching for OMEMO elements)
        let iv_b64 = extract_element_content(encrypted_xml, "iv")
            .ok_or_else(|| OmemoError::Xml("missing <iv>".into()))?;
        let payload_b64 = extract_element_content(encrypted_xml, "payload")
            .ok_or_else(|| OmemoError::Xml("missing <payload>".into()))?;

        // Find our key
        let our_key_b64 = find_key_for_device(encrypted_xml, our_device_id)
            .ok_or(OmemoError::NoKeyForDevice)?;

        let iv = B64
            .decode(iv_b64)
            .map_err(|e| OmemoError::Base64(e.to_string()))?;
        let payload = B64
            .decode(payload_b64)
            .map_err(|e| OmemoError::Base64(e.to_string()))?;
        let key_ct_bytes = B64
            .decode(&our_key_b64)
            .map_err(|e| OmemoError::Base64(e.to_string()))?;

        if key_ct_bytes.len() != 48 || iv.len() != 12 {
            return Err(OmemoError::Xml("invalid key or IV length".into()));
        }

        let mut key_ct = [0u8; 48];
        key_ct.copy_from_slice(&key_ct_bytes);

        // Get ratchet state for sender
        let session_k = session_key(sender_jid, sender_device_id);
        let mut sessions = self.sessions.write().await;

        // Placeholder ratchet_pub — real implementation would extract from PreKeyWhisper header
        let ratchet_pub = [0u8; 32];

        if !sessions.contains_key(&session_k) {
            let loaded = self
                .store
                .load_session(sender_jid, sender_device_id)
                .await?;
            if let Some(state) = loaded {
                sessions.insert(session_k.clone(), state);
            } else {
                // New session from X3DH receiver side — requires ek_pub from PreKeyWhisper
                return Err(OmemoError::Xml(
                    "no session exists; PreKeyWhisper handling required".into(),
                ));
            }
        }

        let state = sessions
            .get_mut(&session_k)
            .expect("session was just inserted");
        let recv_msg_n = state.recv_msg_n;
        let aes_key = ratchet_decrypt(
            state,
            &key_ct,
            &ratchet_pub,
            recv_msg_n,
            id_guard.spk_priv.expose_secret(),
        )?;

        // Persist updated session
        self.store
            .save_session(sender_jid, sender_device_id, state)
            .await?;
        drop(id_guard);

        // Decrypt payload
        let cipher = Aes256Gcm::new_from_slice(&aes_key).map_err(|_| OmemoError::Aead)?;
        let nonce = Nonce::from_slice(&iv);
        let plaintext = cipher
            .decrypt(nonce, payload.as_ref())
            .map_err(|_| OmemoError::Aead)?;

        String::from_utf8(plaintext).map_err(|e| OmemoError::Xml(e.to_string()))
    }
}

/// Extract text content of a simple XML element like `<tag>content</tag>`.
fn extract_element_content<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)?;
    Some(xml[start..start + end].trim())
}

/// Find `<key rid='device_id'>content</key>` in header XML.
fn find_key_for_device(xml: &str, device_id: u32) -> Option<String> {
    // Look for rid='device_id' or rid="device_id"
    let patterns = [
        format!("rid='{device_id}'"),
        format!("rid=\"{device_id}\""),
    ];
    for pat in &patterns {
        if let Some(pos) = xml.find(pat.as_str()) {
            // Find the end of this <key ...> element
            let before = &xml[..pos];
            let key_start = before.rfind('<')?;
            let tag_end = xml[key_start..].find('>')?;
            let content_start = key_start + tag_end + 1;
            let content_end = xml[content_start..].find("</key>")?;
            return Some(
                xml[content_start..content_start + content_end]
                    .trim()
                    .to_string(),
            );
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_key_for_device() {
        let xml =
            r#"<header sid='1'><key rid='42'>AAEC</key><key rid='99'>BBBB</key></header>"#;
        assert_eq!(find_key_for_device(xml, 42).as_deref(), Some("AAEC"));
        assert_eq!(find_key_for_device(xml, 99).as_deref(), Some("BBBB"));
        assert!(find_key_for_device(xml, 7).is_none());
    }

    #[test]
    fn test_extract_element_content() {
        let xml = r#"<encrypted><header><iv>abc123</iv></header><payload>xyz789</payload></encrypted>"#;
        assert_eq!(extract_element_content(xml, "iv"), Some("abc123"));
        assert_eq!(extract_element_content(xml, "payload"), Some("xyz789"));
    }
}
