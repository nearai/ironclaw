//! OMEMO encryption manager backed by persisted Signal sessions.

#![allow(dead_code)]

pub mod store;

use std::path::PathBuf;
use std::sync::Arc;

use aes_gcm::{
    Aes128Gcm, Nonce,
    aead::{AeadInPlace, KeyInit, generic_array::GenericArray},
};
use libsignal_protocol::Serializable;
use libsignal_protocol::messages::{CiphertextType, PreKeySignalMessage, SignalMessage};
use libsignal_protocol::{Address, PreKeyBundle, SessionBuilder, SessionCipher};
use rand::RngCore;
use thiserror::Error;
use tokio::sync::RwLock;
use xmpp_parsers::legacy_omemo::{Bundle, Device, DeviceList, Encrypted, Header, IV, Key, Payload};

use crate::config::XmppConfig;
use store::{OmemoStore, OmemoStoreError, StoreMetadata, registration_id_from_device_id};

#[derive(Debug, Error)]
pub enum OmemoError {
    #[error("store error: {0}")]
    Store(#[from] OmemoStoreError),
    #[error("signal error: {0}")]
    Signal(String),
    #[error("encryption error: {0}")]
    Crypto(String),
    #[error("missing OMEMO bundle data for {0}")]
    MissingBundleData(String),
    #[error("no OMEMO key for local device")]
    NoKeyForDevice,
    #[error("encrypted payload is missing")]
    MissingPayload,
    #[error("invalid transport key length")]
    InvalidTransportKey,
    #[error("background task failed: {0}")]
    Task(String),
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct OmemoDiagnostics {
    pub omemo_enabled: bool,
    pub device_id: Option<u32>,
    pub fingerprint: Option<String>,
    pub bundle_published: bool,
    pub prekeys_available: usize,
    pub migration_state: String,
    pub last_omemo_error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RemoteDeviceBundle {
    pub jid: String,
    pub device_id: u32,
    pub bundle: Bundle,
}

#[derive(Debug, Clone)]
pub struct DecryptResult {
    pub plaintext: String,
    pub used_prekey: bool,
}

#[derive(Debug, Clone)]
pub struct OmemoManager {
    store_dir: PathBuf,
    bare_jid: String,
    requested_device_id: u32,
    diagnostics: Arc<RwLock<OmemoDiagnostics>>,
}

impl OmemoManager {
    pub async fn new(config: &XmppConfig) -> Result<Self, OmemoError> {
        let store_dir = config.omemo_store_dir.clone();
        let requested_device_id = config.device_id;
        let metadata = run_blocking(store_dir.clone(), requested_device_id, move |store| {
            Ok(store.initialize(requested_device_id)?)
        })
        .await?;

        Ok(Self {
            store_dir,
            bare_jid: bare_jid(&config.jid).to_string(),
            requested_device_id,
            diagnostics: Arc::new(RwLock::new(OmemoDiagnostics {
                omemo_enabled: true,
                device_id: Some(metadata.device_id),
                fingerprint: Some(metadata.fingerprint),
                bundle_published: false,
                prekeys_available: metadata.prekeys_available,
                migration_state: metadata.migration_state.as_str().to_string(),
                last_omemo_error: None,
            })),
        })
    }

    pub async fn device_id(&self) -> u32 {
        self.diagnostics.read().await.device_id.unwrap_or(0)
    }

    pub async fn device_list(&self) -> Result<DeviceList, OmemoError> {
        Ok(DeviceList {
            devices: vec![Device {
                id: self.device_id().await,
            }],
        })
    }

    pub async fn bundle(&self) -> Result<Bundle, OmemoError> {
        let requested_device_id = self.requested_device_id;
        let (bundle, metadata) =
            run_blocking(self.store_dir.clone(), requested_device_id, move |store| {
                Ok(store.build_local_bundle(requested_device_id)?)
            })
            .await?;
        self.apply_metadata(metadata).await;
        Ok(bundle)
    }

    pub async fn has_session(&self, jid: &str, device_id: u32) -> Result<bool, OmemoError> {
        let store_dir = self.store_dir.clone();
        let requested_device_id = self.requested_device_id;
        let jid = jid.to_string();
        run_blocking(store_dir, requested_device_id, move |store| {
            let open = store.open_signal_state(requested_device_id)?;
            let address = Address::new(jid.as_bytes(), device_id as i32);
            open.store_context
                .contains_session(&address)
                .map_err(|e| OmemoError::Signal(e.to_string()))
        })
        .await
    }

    pub async fn encrypt(
        &self,
        plaintext: &str,
        recipients: Vec<RemoteDeviceBundle>,
    ) -> Result<Encrypted, OmemoError> {
        let store_dir = self.store_dir.clone();
        let requested_device_id = self.requested_device_id;
        let plaintext = plaintext.to_string();
        let result = run_blocking(store_dir, requested_device_id, move |store| {
            let open = store.open_signal_state(requested_device_id)?;
            let local_device_id = open.local.device_id;

            let mut key_bytes = [0u8; 16];
            let mut iv = [0u8; 12];
            rand::thread_rng().fill_bytes(&mut key_bytes);
            rand::thread_rng().fill_bytes(&mut iv);

            let cipher = Aes128Gcm::new_from_slice(&key_bytes)
                .map_err(|e| OmemoError::Crypto(e.to_string()))?;
            let mut payload_bytes = plaintext.into_bytes();
            let tag = cipher
                .encrypt_in_place_detached(Nonce::from_slice(&iv), b"", &mut payload_bytes)
                .map_err(|e| OmemoError::Crypto(e.to_string()))?;

            let mut transport = Vec::with_capacity(32);
            transport.extend_from_slice(&key_bytes);
            transport.extend_from_slice(tag.as_slice());

            let mut header_keys = Vec::new();
            let mut skipped_recipients = Vec::new();
            for recipient in dedupe_recipients(recipients) {
                let address = Address::new(recipient.jid.as_bytes(), recipient.device_id as i32);
                if !open
                    .store_context
                    .contains_session(&address)
                    .map_err(|e| OmemoError::Signal(e.to_string()))?
                {
                    let bundle = match build_pre_key_bundle(&open.context, &recipient) {
                        Ok(bundle) => bundle,
                        Err(err) => {
                            tracing::warn!(
                                jid = %recipient.jid,
                                device_id = recipient.device_id,
                                error = %err,
                                "Skipping invalid OMEMO recipient bundle"
                            );
                            skipped_recipients.push(format!(
                                "{}#{} (bundle: {})",
                                recipient.jid, recipient.device_id, err
                            ));
                            continue;
                        }
                    };
                    if let Err(err) =
                        SessionBuilder::new(&open.context, &open.store_context, &address)
                            .process_pre_key_bundle(&bundle)
                    {
                        let err = OmemoError::Signal(err.to_string());
                        tracing::warn!(
                            jid = %recipient.jid,
                            device_id = recipient.device_id,
                            error = %err,
                            "Skipping OMEMO recipient after prekey bundle processing failure"
                        );
                        skipped_recipients.push(format!(
                            "{}#{} (session bootstrap: {})",
                            recipient.jid, recipient.device_id, err
                        ));
                        continue;
                    }
                }

                let session_cipher =
                    match SessionCipher::new(&open.context, &open.store_context, &address) {
                        Ok(cipher) => cipher,
                        Err(err) => {
                            let err = OmemoError::Signal(err.to_string());
                            tracing::warn!(
                                jid = %recipient.jid,
                                device_id = recipient.device_id,
                                error = %err,
                                "Skipping OMEMO recipient after session cipher creation failure"
                            );
                            skipped_recipients.push(format!(
                                "{}#{} (session cipher: {})",
                                recipient.jid, recipient.device_id, err
                            ));
                            continue;
                        }
                    };
                let ciphertext = match session_cipher.encrypt(&transport) {
                    Ok(ciphertext) => ciphertext,
                    Err(err) => {
                        let err = OmemoError::Signal(err.to_string());
                        tracing::warn!(
                            jid = %recipient.jid,
                            device_id = recipient.device_id,
                            error = %err,
                            "Skipping OMEMO recipient after encrypt failure"
                        );
                        skipped_recipients.push(format!(
                            "{}#{} (encrypt: {})",
                            recipient.jid, recipient.device_id, err
                        ));
                        continue;
                    }
                };
                let serialized = ciphertext
                    .serialize()
                    .map_err(|e| OmemoError::Signal(e.to_string()))?;
                let is_prekey = matches!(
                    ciphertext
                        .get_type()
                        .map_err(|e| OmemoError::Signal(e.to_string()))?,
                    CiphertextType::PreKey
                );
                header_keys.push(Key {
                    rid: recipient.device_id,
                    prekey: is_prekey,
                    data: serialized.as_slice().to_vec(),
                });
            }

            if header_keys.is_empty() {
                let details = if skipped_recipients.is_empty() {
                    "no usable OMEMO recipients".to_string()
                } else {
                    format!(
                        "no usable OMEMO recipients; skipped {}",
                        skipped_recipients.join(", ")
                    )
                };
                return Err(OmemoError::Signal(details));
            }

            Ok((
                Encrypted {
                    header: Header {
                        sid: local_device_id,
                        keys: header_keys,
                        iv: IV { data: iv.to_vec() },
                    },
                    payload: Some(Payload {
                        data: payload_bytes,
                    }),
                },
                StoreMetadata {
                    device_id: open.local.device_id,
                    fingerprint: local_fingerprint(&open.local)?,
                    migration_state: open.local.migration_state,
                    prekeys_available: open.prekeys_available,
                },
            ))
        })
        .await;

        match result {
            Ok((encrypted, metadata)) => {
                self.apply_metadata(metadata).await;
                self.clear_error().await;
                Ok(encrypted)
            }
            Err(err) => {
                self.record_error(err.to_string()).await;
                Err(err)
            }
        }
    }

    pub async fn decrypt(
        &self,
        sender_jid: &str,
        sender_device_id: u32,
        encrypted: Encrypted,
    ) -> Result<DecryptResult, OmemoError> {
        let store_dir = self.store_dir.clone();
        let requested_device_id = self.requested_device_id;
        let sender_jid = sender_jid.to_string();
        let result = run_blocking(store_dir, requested_device_id, move |store| {
            let open = store.open_signal_state(requested_device_id)?;
            let local_device_id = open.local.device_id;
            let local_key = encrypted
                .header
                .keys
                .iter()
                .find(|key| key.rid == local_device_id)
                .cloned()
                .ok_or(OmemoError::NoKeyForDevice)?;
            let payload = encrypted.payload.ok_or(OmemoError::MissingPayload)?;

            let address = Address::new(sender_jid.as_bytes(), sender_device_id as i32);
            let session_cipher = SessionCipher::new(&open.context, &open.store_context, &address)
                .map_err(|e| OmemoError::Signal(e.to_string()))?;

            let transport = if local_key.prekey {
                let message =
                    PreKeySignalMessage::deserialize_with_context(&open.context, &local_key.data)
                        .map_err(|e| OmemoError::Signal(e.to_string()))?;
                session_cipher
                    .decrypt_pre_key_signal_message(&message)
                    .map_err(|e| OmemoError::Signal(e.to_string()))?
            } else {
                let message =
                    SignalMessage::deserialize_with_context(&open.context, &local_key.data)
                        .map_err(|e| OmemoError::Signal(e.to_string()))?;
                session_cipher
                    .decrypt_signal_message(&message)
                    .map_err(|e| OmemoError::Signal(e.to_string()))?
            };

            let transport_bytes = transport.as_slice();
            if transport_bytes.len() < 32 {
                return Err(OmemoError::InvalidTransportKey);
            }

            let cipher = Aes128Gcm::new_from_slice(&transport_bytes[..16])
                .map_err(|e| OmemoError::Crypto(e.to_string()))?;
            let tag = GenericArray::clone_from_slice(&transport_bytes[16..32]);
            let mut plaintext = payload.data;
            cipher
                .decrypt_in_place_detached(
                    Nonce::from_slice(&encrypted.header.iv.data),
                    b"",
                    &mut plaintext,
                    &tag,
                )
                .map_err(|e| OmemoError::Crypto(e.to_string()))?;

            let text =
                String::from_utf8(plaintext).map_err(|e| OmemoError::Crypto(e.to_string()))?;
            Ok((
                DecryptResult {
                    plaintext: text,
                    used_prekey: local_key.prekey,
                },
                StoreMetadata {
                    device_id: open.local.device_id,
                    fingerprint: local_fingerprint(&open.local)?,
                    migration_state: open.local.migration_state,
                    prekeys_available: open.prekeys_available,
                },
            ))
        })
        .await;

        match result {
            Ok((decrypted, metadata)) => {
                self.apply_metadata(metadata).await;
                if decrypted.used_prekey {
                    self.mark_bundle_needs_publish().await;
                } else {
                    self.clear_error().await;
                }
                Ok(decrypted)
            }
            Err(err) => {
                self.record_error(err.to_string()).await;
                Err(err)
            }
        }
    }

    pub async fn save_remote_device_list(
        &self,
        jid: &str,
        devices: &[u32],
    ) -> Result<(), OmemoError> {
        let jid = jid.to_string();
        let devices = devices.to_vec();
        run_blocking(
            self.store_dir.clone(),
            self.requested_device_id,
            move |store| Ok(store.save_remote_device_list(&jid, &devices)?),
        )
        .await
    }

    pub async fn save_remote_bundle(
        &self,
        jid: &str,
        device_id: u32,
        bundle: &Bundle,
    ) -> Result<(), OmemoError> {
        let jid = jid.to_string();
        let bundle_xml = {
            let element = xmpp_parsers::minidom::Element::from(bundle.clone());
            let mut bytes = Vec::new();
            element
                .write_to(&mut bytes)
                .map_err(|e| OmemoError::Signal(e.to_string()))?;
            String::from_utf8(bytes).map_err(|e| OmemoError::Signal(e.to_string()))?
        };
        run_blocking(
            self.store_dir.clone(),
            self.requested_device_id,
            move |store| Ok(store.save_remote_bundle(&jid, device_id, &bundle_xml)?),
        )
        .await
    }

    pub async fn mark_bundle_published(&self) {
        let mut diagnostics = self.diagnostics.write().await;
        diagnostics.bundle_published = true;
        diagnostics.last_omemo_error = None;
    }

    pub async fn mark_bundle_needs_publish(&self) {
        self.diagnostics.write().await.bundle_published = false;
    }

    pub async fn record_error(&self, message: String) {
        self.diagnostics.write().await.last_omemo_error = Some(message);
    }

    pub async fn clear_error(&self) {
        self.diagnostics.write().await.last_omemo_error = None;
    }

    pub async fn diagnostics(&self) -> OmemoDiagnostics {
        self.diagnostics.read().await.clone()
    }

    async fn apply_metadata(&self, metadata: StoreMetadata) {
        let mut diagnostics = self.diagnostics.write().await;
        diagnostics.device_id = Some(metadata.device_id);
        diagnostics.fingerprint = Some(metadata.fingerprint);
        diagnostics.prekeys_available = metadata.prekeys_available;
        diagnostics.migration_state = metadata.migration_state.as_str().to_string();
    }
}

fn local_fingerprint(local: &store::LocalSignalState) -> Result<String, OmemoError> {
    let public = local
        .identity_key_pair
        .public()
        .serialize()
        .map_err(|e| OmemoError::Signal(e.to_string()))?;
    Ok(hex::encode(public.as_slice().get(1..).unwrap_or(&[])))
}

async fn run_blocking<F, R>(
    store_dir: PathBuf,
    requested_device_id: u32,
    f: F,
) -> Result<R, OmemoError>
where
    F: FnOnce(OmemoStore) -> Result<R, OmemoError> + Send + 'static,
    R: Send + 'static,
{
    tokio::task::spawn_blocking(move || {
        let store = OmemoStore::new(store_dir);
        let _ = requested_device_id;
        f(store)
    })
    .await
    .map_err(|e| OmemoError::Task(e.to_string()))?
}

fn build_pre_key_bundle(
    context: &libsignal_protocol::Context,
    remote: &RemoteDeviceBundle,
) -> Result<PreKeyBundle, OmemoError> {
    let identity_key =
        remote.bundle.identity_key.as_ref().ok_or_else(|| {
            OmemoError::MissingBundleData(format!("identity_key for {}", remote.jid))
        })?;
    let signed_pre_key_public = remote
        .bundle
        .signed_pre_key_public
        .as_ref()
        .ok_or_else(|| {
            OmemoError::MissingBundleData(format!("signed_pre_key for {}", remote.jid))
        })?;
    let signature = remote
        .bundle
        .signed_pre_key_signature
        .as_ref()
        .ok_or_else(|| {
            OmemoError::MissingBundleData(format!("signed_pre_key_signature for {}", remote.jid))
        })?;
    let pre_key = remote
        .bundle
        .prekeys
        .as_ref()
        .and_then(|prekeys| prekeys.keys.first())
        .ok_or_else(|| OmemoError::MissingBundleData(format!("prekeys for {}", remote.jid)))?;

    let identity_key =
        libsignal_protocol::keys::PublicKey::decode_point(context, &identity_key.data)
            .map_err(|e| OmemoError::Signal(e.to_string()))?;
    let signed_pre_key =
        libsignal_protocol::keys::PublicKey::decode_point(context, &signed_pre_key_public.data)
            .map_err(|e| OmemoError::Signal(e.to_string()))?;
    let pre_key_public = libsignal_protocol::keys::PublicKey::decode_point(context, &pre_key.data)
        .map_err(|e| OmemoError::Signal(e.to_string()))?;

    PreKeyBundle::builder()
        .registration_id(registration_id_from_device_id(remote.device_id))
        .device_id(remote.device_id as i32)
        .identity_key(&identity_key)
        .pre_key(pre_key.pre_key_id, &pre_key_public)
        .signed_pre_key(
            signed_pre_key_public.signed_pre_key_id.unwrap_or(1),
            &signed_pre_key,
        )
        .signature(&signature.data)
        .build()
        .map_err(|e| OmemoError::Signal(e.to_string()))
}

fn dedupe_recipients(recipients: Vec<RemoteDeviceBundle>) -> Vec<RemoteDeviceBundle> {
    let mut seen = std::collections::HashSet::new();
    recipients
        .into_iter()
        .filter(|recipient| seen.insert((recipient.jid.clone(), recipient.device_id)))
        .collect()
}

fn bare_jid(jid: &str) -> &str {
    jid.split('/').next().unwrap_or(jid)
}

#[cfg(test)]
mod tests {
    use secrecy::SecretString;
    use tempfile::TempDir;

    use super::*;

    fn config_for(jid: &str, store_dir: PathBuf) -> XmppConfig {
        XmppConfig {
            jid: jid.to_string(),
            password: SecretString::from("password".to_string()),
            allow_from: vec!["*".to_string()],
            dm_policy: "open".to_string(),
            allow_rooms: vec![],
            encrypted_rooms: vec![],
            device_id: 0,
            omemo_store_dir: store_dir,
            allow_plaintext_fallback: false,
            max_messages_per_hour: 0,
        }
    }

    fn remote_bundle(jid: &str, device_id: u32, bundle: Bundle) -> RemoteDeviceBundle {
        RemoteDeviceBundle {
            jid: jid.to_string(),
            device_id,
            bundle,
        }
    }

    #[tokio::test]
    async fn bundle_and_device_list_are_available_after_init() {
        let dir = TempDir::new().unwrap();
        let manager = OmemoManager::new(&config_for(
            "bot@example.com/resource",
            dir.path().to_path_buf(),
        ))
        .await
        .unwrap();

        let device_list = manager.device_list().await.unwrap();
        let bundle = manager.bundle().await.unwrap();

        assert_eq!(device_list.devices.len(), 1);
        assert!(bundle.identity_key.is_some());
        assert!(bundle.signed_pre_key_public.is_some());
        assert!(
            bundle
                .prekeys
                .as_ref()
                .is_some_and(|value| !value.keys.is_empty())
        );
    }

    #[tokio::test]
    async fn first_encrypted_dm_bootstraps_session_and_decrypts() {
        let alice_dir = TempDir::new().unwrap();
        let bob_dir = TempDir::new().unwrap();

        let alice = OmemoManager::new(&config_for(
            "alice@example.com/laptop",
            alice_dir.path().to_path_buf(),
        ))
        .await
        .unwrap();
        let bob = OmemoManager::new(&config_for(
            "bob@example.com/phone",
            bob_dir.path().to_path_buf(),
        ))
        .await
        .unwrap();

        let bob_device_id = bob.device_id().await;
        let bob_bundle = bob.bundle().await.unwrap();

        let encrypted = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            alice.encrypt(
                "hello bob",
                vec![remote_bundle("bob@example.com", bob_device_id, bob_bundle)],
            ),
        )
        .await
        .expect("alice encrypt should complete")
        .unwrap();

        let bob_key = encrypted
            .header
            .keys
            .iter()
            .find(|key| key.rid == bob_device_id)
            .unwrap();
        assert!(
            bob_key.prekey,
            "first message should bootstrap with a prekey"
        );

        let decrypted = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            bob.decrypt("alice@example.com", alice.device_id().await, encrypted),
        )
        .await
        .expect("bob decrypt should complete")
        .unwrap();

        assert_eq!(decrypted.plaintext, "hello bob");
        assert!(decrypted.used_prekey);
    }

    #[tokio::test]
    async fn follow_up_message_uses_persisted_session_after_restart() {
        let alice_dir = TempDir::new().unwrap();
        let bob_dir = TempDir::new().unwrap();

        let alice_config = config_for("alice@example.com/laptop", alice_dir.path().to_path_buf());
        let bob_config = config_for("bob@example.com/phone", bob_dir.path().to_path_buf());

        let alice = OmemoManager::new(&alice_config).await.unwrap();
        let bob = OmemoManager::new(&bob_config).await.unwrap();

        let alice_device_id = alice.device_id().await;
        let alice_bundle = alice.bundle().await.unwrap();
        let bob_device_id = bob.device_id().await;
        let bob_bundle = bob.bundle().await.unwrap();

        let first = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            alice.encrypt(
                "initial session",
                vec![remote_bundle(
                    "bob@example.com",
                    bob_device_id,
                    bob_bundle.clone(),
                )],
            ),
        )
        .await
        .expect("initial alice encrypt should complete")
        .unwrap();
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            bob.decrypt("alice@example.com", alice_device_id, first),
        )
        .await
        .expect("initial bob decrypt should complete")
        .unwrap();

        let reply = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            bob.encrypt(
                "reply",
                vec![remote_bundle(
                    "alice@example.com",
                    alice_device_id,
                    alice_bundle,
                )],
            ),
        )
        .await
        .expect("bob reply encrypt should complete")
        .unwrap();
        let alice_key = reply
            .header
            .keys
            .iter()
            .find(|key| key.rid == alice_device_id)
            .unwrap();
        assert!(
            !alice_key.prekey,
            "reply after decrypt should use the established session",
        );
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            alice.decrypt("bob@example.com", bob_device_id, reply),
        )
        .await
        .expect("alice reply decrypt should complete")
        .unwrap();

        let alice_restarted = OmemoManager::new(&alice_config).await.unwrap();
        let bob_restarted = OmemoManager::new(&bob_config).await.unwrap();
        let follow_up = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            alice_restarted.encrypt(
                "follow up",
                vec![remote_bundle("bob@example.com", bob_device_id, bob_bundle)],
            ),
        )
        .await
        .expect("follow-up alice encrypt should complete")
        .unwrap();

        let bob_key = follow_up
            .header
            .keys
            .iter()
            .find(|key| key.rid == bob_device_id)
            .unwrap();
        assert!(
            !bob_key.prekey,
            "follow-up message should use the persisted Signal session",
        );

        let decrypted = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            bob_restarted.decrypt(
                "alice@example.com",
                alice_restarted.device_id().await,
                follow_up,
            ),
        )
        .await
        .expect("follow-up bob decrypt should complete")
        .unwrap();

        assert_eq!(decrypted.plaintext, "follow up");
        assert!(!decrypted.used_prekey);
    }

    #[tokio::test]
    async fn decrypt_bootstrap_creates_session_for_sender_device() {
        let alice_dir = TempDir::new().unwrap();
        let bob_dir = TempDir::new().unwrap();

        let alice = OmemoManager::new(&config_for(
            "alice@example.com/laptop",
            alice_dir.path().to_path_buf(),
        ))
        .await
        .unwrap();
        let bob = OmemoManager::new(&config_for(
            "bob@example.com/phone",
            bob_dir.path().to_path_buf(),
        ))
        .await
        .unwrap();

        let bob_device_id = bob.device_id().await;
        let bob_bundle = bob.bundle().await.unwrap();

        let encrypted = alice
            .encrypt(
                "hello bob",
                vec![remote_bundle("bob@example.com", bob_device_id, bob_bundle)],
            )
            .await
            .unwrap();
        let alice_device_id = alice.device_id().await;

        bob.decrypt("alice@example.com", alice_device_id, encrypted)
            .await
            .unwrap();

        assert!(
            bob.has_session("alice@example.com", alice_device_id)
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn remote_prekey_bundle_uses_zero_registration_id_for_legacy_omemo() {
        let bob_dir = TempDir::new().unwrap();
        let bob = OmemoManager::new(&config_for(
            "bob@example.com/phone",
            bob_dir.path().to_path_buf(),
        ))
        .await
        .unwrap();

        let remote = remote_bundle(
            "bob@example.com",
            bob.device_id().await,
            bob.bundle().await.unwrap(),
        );
        let context =
            libsignal_protocol::Context::new(libsignal_protocol::crypto::DefaultCrypto::default())
                .unwrap();
        let bundle = build_pre_key_bundle(&context, &remote).unwrap();

        assert_eq!(bundle.registration_id(), 0);
    }

    #[tokio::test]
    async fn encrypt_skips_invalid_recipient_bundle_if_another_device_is_valid() {
        let alice_dir = TempDir::new().unwrap();
        let bob_dir = TempDir::new().unwrap();

        let alice = OmemoManager::new(&config_for(
            "alice@example.com/laptop",
            alice_dir.path().to_path_buf(),
        ))
        .await
        .unwrap();
        let bob = OmemoManager::new(&config_for(
            "bob@example.com/phone",
            bob_dir.path().to_path_buf(),
        ))
        .await
        .unwrap();

        let mut invalid_bundle = bob.bundle().await.unwrap();
        invalid_bundle
            .signed_pre_key_signature
            .as_mut()
            .expect("signed prekey signature")
            .data[0] ^= 0xFF;

        let valid_device_id = bob.device_id().await;
        let encrypted = alice
            .encrypt(
                "hello bob",
                vec![
                    remote_bundle("bob@example.com", 999_999, invalid_bundle),
                    remote_bundle(
                        "bob@example.com",
                        valid_device_id,
                        bob.bundle().await.unwrap(),
                    ),
                ],
            )
            .await
            .unwrap();

        assert!(
            encrypted
                .header
                .keys
                .iter()
                .any(|key| key.rid == valid_device_id)
        );
        assert!(encrypted.header.keys.iter().all(|key| key.rid != 999_999));
    }
}
