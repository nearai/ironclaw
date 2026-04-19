//! OMEMO persistence backed by libsignal-compatible serialized records.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use base64::{Engine, engine::general_purpose::STANDARD as B64};
use libsignal_protocol::Serializable;
use libsignal_protocol::crypto::DefaultCrypto;
use libsignal_protocol::keys::{IdentityKeyPair, PreKey, PublicKey, SessionSignedPreKey};
use libsignal_protocol::stores::{
    IdentityKeyStore, PreKeyStore, SerializedSession, SessionStore, SignedPreKeyStore,
};
use libsignal_protocol::{
    Address, Context, StoreContext, generate_identity_key_pair, generate_pre_keys,
    generate_signed_pre_key, store_context,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use xmpp_parsers::legacy_omemo::{
    Bundle, IdentityKey, PreKeyPublic, Prekeys, SignedPreKeyPublic, SignedPreKeySignature,
};

const STORE_VERSION: u32 = 3;
const ACTIVE_SIGNED_PRE_KEY_ID: u32 = 1;
const TARGET_PRE_KEY_COUNT: u32 = 100;
const MIN_PRE_KEY_COUNT: usize = 20;

#[derive(Debug, Error)]
pub enum OmemoStoreError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("base64 decode error: {0}")]
    Base64(#[from] base64::DecodeError),
    #[error("signal error: {0}")]
    Signal(String),
    #[error("record not found: {0}")]
    NotFound(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MigrationState {
    Fresh,
    LegacyIdentityPreserved,
    LegacyDeviceIdPreserved,
    LegacyDeviceIdOnly,
}

impl MigrationState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Fresh => "fresh",
            Self::LegacyIdentityPreserved => "legacy_identity_preserved",
            Self::LegacyDeviceIdPreserved => "legacy_device_id_preserved",
            Self::LegacyDeviceIdOnly => "legacy_device_id_only",
        }
    }
}

#[derive(Debug, Clone)]
pub struct StoreMetadata {
    pub device_id: u32,
    pub fingerprint: String,
    pub migration_state: MigrationState,
    pub prekeys_available: usize,
}

#[derive(Debug, Clone)]
pub struct LocalSignalState {
    pub device_id: u32,
    pub registration_id: u32,
    pub identity_key_pair: IdentityKeyPair,
    pub signed_pre_key_id: u32,
    pub signed_pre_key: SessionSignedPreKey,
    pub migration_state: MigrationState,
}

pub struct OpenSignalState {
    pub context: Context,
    pub store_context: StoreContext,
    pub local: LocalSignalState,
    pub prekeys_available: usize,
}

#[derive(Debug, Clone)]
pub struct OmemoStore {
    root: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
struct IdentityDisk {
    version: u32,
    device_id: u32,
    registration_id: u32,
    identity_key_pair_b64: String,
    signed_pre_key_id: u32,
    signed_pre_key_b64: String,
    next_pre_key_id: u32,
    migration_state: MigrationState,
}

#[derive(Debug, Serialize, Deserialize)]
struct SessionDisk {
    session_b64: String,
    extra_data_b64: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct DeviceListCacheDisk {
    devices: Vec<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
struct BundleCacheDisk {
    bundle_xml: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct LegacyIdentityDisk {
    device_id: u32,
    ik_priv: String,
    ik_pub: String,
}

impl OmemoStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn initialize(&self, requested_device_id: u32) -> Result<StoreMetadata, OmemoStoreError> {
        let state = self.open_signal_state(requested_device_id)?;
        Ok(StoreMetadata {
            device_id: state.local.device_id,
            fingerprint: identity_fingerprint(
                state
                    .local
                    .identity_key_pair
                    .public()
                    .serialize()
                    .map_err(|e| OmemoStoreError::Signal(e.to_string()))?
                    .as_slice(),
            ),
            migration_state: state.local.migration_state.clone(),
            prekeys_available: state.prekeys_available,
        })
    }

    pub fn open_signal_state(
        &self,
        requested_device_id: u32,
    ) -> Result<OpenSignalState, OmemoStoreError> {
        fs::create_dir_all(self.v2_dir())?;

        let context = Context::new(DefaultCrypto::default())
            .map_err(|e| OmemoStoreError::Signal(e.to_string()))?;
        let mut state = self.load_or_init_local_state(&context, requested_device_id)?;
        let prekeys_available = self.ensure_prekeys(&context, &mut state)?;
        let store_context = self.create_store_context(&context, &state)?;

        Ok(OpenSignalState {
            context,
            store_context,
            local: state,
            prekeys_available,
        })
    }

    pub fn build_local_bundle(
        &self,
        requested_device_id: u32,
    ) -> Result<(Bundle, StoreMetadata), OmemoStoreError> {
        let state = self.open_signal_state(requested_device_id)?;
        let bundle = self.bundle_from_state(&state.local)?;
        Ok((
            bundle,
            StoreMetadata {
                device_id: state.local.device_id,
                fingerprint: identity_fingerprint(
                    state
                        .local
                        .identity_key_pair
                        .public()
                        .serialize()
                        .map_err(|e| OmemoStoreError::Signal(e.to_string()))?
                        .as_slice(),
                ),
                migration_state: state.local.migration_state.clone(),
                prekeys_available: state.prekeys_available,
            },
        ))
    }

    pub fn save_remote_device_list(
        &self,
        jid: &str,
        devices: &[u32],
    ) -> Result<(), OmemoStoreError> {
        let path = self.device_list_cache_path(jid);
        let bytes = serde_json::to_vec_pretty(&DeviceListCacheDisk {
            devices: devices.to_vec(),
        })?;
        self.atomic_write(&path, &bytes)
    }

    pub fn save_remote_bundle(
        &self,
        jid: &str,
        device_id: u32,
        bundle_xml: &str,
    ) -> Result<(), OmemoStoreError> {
        let path = self.bundle_cache_path(jid, device_id);
        let bytes = serde_json::to_vec_pretty(&BundleCacheDisk {
            bundle_xml: bundle_xml.to_string(),
        })?;
        self.atomic_write(&path, &bytes)
    }

    fn create_store_context(
        &self,
        context: &Context,
        local: &LocalSignalState,
    ) -> Result<StoreContext, OmemoStoreError> {
        let public_key = local
            .identity_key_pair
            .public()
            .serialize()
            .map_err(|e| OmemoStoreError::Signal(e.to_string()))?
            .as_slice()
            .to_vec();
        let private_key = local
            .identity_key_pair
            .private()
            .serialize()
            .map_err(|e| OmemoStoreError::Signal(e.to_string()))?
            .as_slice()
            .to_vec();

        store_context(
            context,
            FilePreKeyStore {
                dir: self.prekeys_dir(),
            },
            FileSignedPreKeyStore {
                dir: self.signed_prekeys_dir(),
            },
            FileSessionStore {
                dir: self.sessions_dir(),
            },
            FileIdentityStore {
                dir: self.trusted_identities_dir(),
                public_key,
                private_key,
                registration_id: local.registration_id,
            },
        )
        .map_err(|e| OmemoStoreError::Signal(e.to_string()))
    }

    fn load_or_init_local_state(
        &self,
        context: &Context,
        requested_device_id: u32,
    ) -> Result<LocalSignalState, OmemoStoreError> {
        let identity_path = self.identity_path();
        if identity_path.exists() {
            let disk: IdentityDisk = serde_json::from_slice(&fs::read(&identity_path)?)?;
            let identity_key_pair =
                IdentityKeyPair::deserialize(&B64.decode(disk.identity_key_pair_b64.as_bytes())?)
                    .map_err(|e| OmemoStoreError::Signal(e.to_string()))?;
            let signed_pre_key =
                SessionSignedPreKey::deserialize(&B64.decode(disk.signed_pre_key_b64.as_bytes())?)
                    .map_err(|e| OmemoStoreError::Signal(e.to_string()))?;
            let expected_registration_id = registration_id_from_device_id(disk.device_id);
            let state = LocalSignalState {
                device_id: disk.device_id,
                registration_id: expected_registration_id,
                identity_key_pair,
                signed_pre_key_id: disk.signed_pre_key_id,
                signed_pre_key,
                migration_state: disk.migration_state,
            };
            if disk.version < STORE_VERSION || disk.registration_id != expected_registration_id {
                let sessions_dir = self.sessions_dir();
                if sessions_dir.exists() {
                    fs::remove_dir_all(&sessions_dir)?;
                }
                self.persist_local_state(&state, disk.next_pre_key_id.max(1))?;
            }
            return Ok(state);
        }

        let migrated = self.try_migrate_legacy_identity(context, requested_device_id)?;
        let state = if let Some((identity_key_pair, device_id, migration_state)) = migrated {
            let registration_id = registration_id_from_device_id(device_id);
            let signed_pre_key = generate_signed_pre_key(
                context,
                &identity_key_pair,
                ACTIVE_SIGNED_PRE_KEY_ID,
                SystemTime::now(),
            )
            .map_err(|e| OmemoStoreError::Signal(e.to_string()))?;
            LocalSignalState {
                device_id,
                registration_id,
                identity_key_pair,
                signed_pre_key_id: ACTIVE_SIGNED_PRE_KEY_ID,
                signed_pre_key,
                migration_state,
            }
        } else {
            let device_id = choose_device_id(requested_device_id, None);
            let identity_key_pair = generate_identity_key_pair(context)
                .map_err(|e| OmemoStoreError::Signal(e.to_string()))?;
            let registration_id = registration_id_from_device_id(device_id);
            let signed_pre_key = generate_signed_pre_key(
                context,
                &identity_key_pair,
                ACTIVE_SIGNED_PRE_KEY_ID,
                SystemTime::now(),
            )
            .map_err(|e| OmemoStoreError::Signal(e.to_string()))?;
            LocalSignalState {
                device_id,
                registration_id,
                identity_key_pair,
                signed_pre_key_id: ACTIVE_SIGNED_PRE_KEY_ID,
                signed_pre_key,
                migration_state: MigrationState::Fresh,
            }
        };

        self.persist_local_state(&state, 1)?;
        Ok(state)
    }

    fn ensure_prekeys(
        &self,
        context: &Context,
        state: &mut LocalSignalState,
    ) -> Result<usize, OmemoStoreError> {
        fs::create_dir_all(self.prekeys_dir())?;
        fs::create_dir_all(self.signed_prekeys_dir())?;

        let current = self.count_files(&self.prekeys_dir())?;
        if current < MIN_PRE_KEY_COUNT {
            let next_pre_key_id = self.load_next_pre_key_id()?.max(1);
            let to_generate = TARGET_PRE_KEY_COUNT.saturating_sub(current as u32);
            if to_generate > 0 {
                let generated = generate_pre_keys(context, next_pre_key_id, to_generate)
                    .map_err(|e| OmemoStoreError::Signal(e.to_string()))?;
                let mut max_id = next_pre_key_id;
                for pre_key in generated {
                    let id = pre_key.id();
                    let bytes = pre_key
                        .serialize()
                        .map_err(|e| OmemoStoreError::Signal(e.to_string()))?;
                    self.atomic_write(&self.pre_key_path(id), bytes.as_slice())?;
                    max_id = max_id.max(id.saturating_add(1));
                }
                self.persist_local_state(state, max_id)?;
            }
        } else {
            self.persist_local_state(state, self.load_next_pre_key_id()?.max(1))?;
        }

        Ok(self.count_files(&self.prekeys_dir())?)
    }

    fn try_migrate_legacy_identity(
        &self,
        context: &Context,
        requested_device_id: u32,
    ) -> Result<Option<(IdentityKeyPair, u32, MigrationState)>, OmemoStoreError> {
        let legacy_path = self.root.join("identity_key.json");
        if !legacy_path.exists() {
            return Ok(None);
        }

        let legacy: LegacyIdentityDisk = serde_json::from_slice(&fs::read(legacy_path)?)?;
        let target_device_id = choose_device_id(requested_device_id, Some(legacy.device_id));

        let private_bytes = hex::decode(legacy.ik_priv)
            .map_err(|e| OmemoStoreError::Signal(format!("legacy identity decode failed: {e}")))?;
        let public_bytes =
            normalize_public_key(&hex::decode(legacy.ik_pub).map_err(|e| {
                OmemoStoreError::Signal(format!("legacy identity decode failed: {e}"))
            })?);

        let private_key =
            libsignal_protocol::keys::PrivateKey::decode_point(context, &private_bytes)
                .map_err(|e| OmemoStoreError::Signal(e.to_string()))?;
        let public_key = PublicKey::decode_point(context, &public_bytes)
            .map_err(|e| OmemoStoreError::Signal(e.to_string()))?;
        let identity_key_pair = IdentityKeyPair::new(&public_key, &private_key)
            .map_err(|e| OmemoStoreError::Signal(e.to_string()))?;

        let migration_state = if requested_device_id != 0 && requested_device_id != legacy.device_id
        {
            MigrationState::LegacyDeviceIdPreserved
        } else {
            MigrationState::LegacyIdentityPreserved
        };

        Ok(Some((identity_key_pair, target_device_id, migration_state)))
    }

    fn bundle_from_state(&self, local: &LocalSignalState) -> Result<Bundle, OmemoStoreError> {
        let identity_key = local
            .identity_key_pair
            .public()
            .serialize()
            .map_err(|e| OmemoStoreError::Signal(e.to_string()))?;
        let signed_pre_key = local
            .signed_pre_key
            .key_pair()
            .public()
            .serialize()
            .map_err(|e| OmemoStoreError::Signal(e.to_string()))?;

        let mut prekey_entries = Vec::new();
        for entry in fs::read_dir(self.prekeys_dir())? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let bytes = fs::read(entry.path())?;
            let pre_key =
                PreKey::deserialize(&bytes).map_err(|e| OmemoStoreError::Signal(e.to_string()))?;
            let public = pre_key
                .key_pair()
                .public()
                .serialize()
                .map_err(|e| OmemoStoreError::Signal(e.to_string()))?;
            prekey_entries.push(PreKeyPublic {
                pre_key_id: pre_key.id(),
                data: public.as_slice().to_vec(),
            });
        }
        prekey_entries.sort_by_key(|entry| entry.pre_key_id);

        Ok(Bundle {
            signed_pre_key_public: Some(SignedPreKeyPublic {
                signed_pre_key_id: Some(local.signed_pre_key_id),
                data: signed_pre_key.as_slice().to_vec(),
            }),
            signed_pre_key_signature: Some(SignedPreKeySignature {
                data: local.signed_pre_key.signature().to_vec(),
            }),
            identity_key: Some(IdentityKey {
                data: identity_key.as_slice().to_vec(),
            }),
            prekeys: Some(Prekeys {
                keys: prekey_entries,
            }),
        })
    }

    fn persist_local_state(
        &self,
        state: &LocalSignalState,
        next_pre_key_id: u32,
    ) -> Result<(), OmemoStoreError> {
        fs::create_dir_all(self.signed_prekeys_dir())?;

        let identity_key_pair = state
            .identity_key_pair
            .serialize()
            .map_err(|e| OmemoStoreError::Signal(e.to_string()))?;
        let signed_pre_key = state
            .signed_pre_key
            .serialize()
            .map_err(|e| OmemoStoreError::Signal(e.to_string()))?;

        let disk = IdentityDisk {
            version: STORE_VERSION,
            device_id: state.device_id,
            registration_id: state.registration_id,
            identity_key_pair_b64: B64.encode(identity_key_pair.as_slice()),
            signed_pre_key_id: state.signed_pre_key_id,
            signed_pre_key_b64: B64.encode(signed_pre_key.as_slice()),
            next_pre_key_id,
            migration_state: state.migration_state.clone(),
        };

        let bytes = serde_json::to_vec_pretty(&disk)?;
        self.atomic_write(&self.identity_path(), &bytes)?;
        self.atomic_write(
            &self.signed_pre_key_path(state.signed_pre_key_id),
            signed_pre_key.as_slice(),
        )?;
        Ok(())
    }

    fn load_next_pre_key_id(&self) -> Result<u32, OmemoStoreError> {
        let path = self.identity_path();
        if !path.exists() {
            return Ok(1);
        }
        let disk: IdentityDisk = serde_json::from_slice(&fs::read(path)?)?;
        Ok(disk.next_pre_key_id.max(1))
    }

    fn count_files(&self, dir: &Path) -> Result<usize, OmemoStoreError> {
        if !dir.exists() {
            return Ok(0);
        }
        Ok(fs::read_dir(dir)?
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| entry.file_type().ok())
            .filter(|file_type| file_type.is_file())
            .count())
    }

    fn atomic_write(&self, path: &Path, data: &[u8]) -> Result<(), OmemoStoreError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("tmp");
        let mut file = fs::File::create(&tmp)?;
        file.write_all(data)?;
        file.sync_all()?;
        fs::rename(tmp, path)?;
        Ok(())
    }

    fn v2_dir(&self) -> PathBuf {
        self.root.join("v2")
    }

    fn identity_path(&self) -> PathBuf {
        self.v2_dir().join("identity.json")
    }

    fn prekeys_dir(&self) -> PathBuf {
        self.v2_dir().join("prekeys")
    }

    fn signed_prekeys_dir(&self) -> PathBuf {
        self.v2_dir().join("signed_prekeys")
    }

    fn sessions_dir(&self) -> PathBuf {
        self.v2_dir().join("sessions")
    }

    fn trusted_identities_dir(&self) -> PathBuf {
        self.v2_dir().join("trusted_identities")
    }

    fn cache_dir(&self) -> PathBuf {
        self.v2_dir().join("cache")
    }

    fn pre_key_path(&self, id: u32) -> PathBuf {
        self.prekeys_dir().join(format!("{id}.bin"))
    }

    fn signed_pre_key_path(&self, id: u32) -> PathBuf {
        self.signed_prekeys_dir().join(format!("{id}.bin"))
    }

    fn address_dir(&self, base: &Path, name: &[u8]) -> PathBuf {
        base.join(hex::encode(name))
    }

    fn session_path_for_addr(&self, addr: &Address) -> PathBuf {
        self.address_dir(&self.sessions_dir(), addr.bytes())
            .join(format!("{}.json", addr.device_id()))
    }

    fn trusted_identity_path_for_addr(&self, addr: &Address) -> PathBuf {
        self.address_dir(&self.trusted_identities_dir(), addr.bytes())
            .join(format!("{}.bin", addr.device_id()))
    }

    fn device_list_cache_path(&self, jid: &str) -> PathBuf {
        self.cache_dir()
            .join("device_lists")
            .join(format!("{}.json", hex::encode(jid)))
    }

    fn bundle_cache_path(&self, jid: &str, device_id: u32) -> PathBuf {
        self.cache_dir()
            .join("bundles")
            .join(hex::encode(jid))
            .join(format!("{device_id}.json"))
    }
}

struct FilePreKeyStore {
    dir: PathBuf,
}

impl PreKeyStore for FilePreKeyStore {
    fn load(&self, id: u32, writer: &mut dyn Write) -> io::Result<()> {
        writer.write_all(&fs::read(self.dir.join(format!("{id}.bin")))?)
    }

    fn store(&self, id: u32, body: &[u8]) -> Result<(), libsignal_protocol::InternalError> {
        write_atomic_bytes(&self.dir.join(format!("{id}.bin")), body)
            .map_err(|_| libsignal_protocol::InternalError::Unknown)
    }

    fn contains(&self, id: u32) -> bool {
        self.dir.join(format!("{id}.bin")).exists()
    }

    fn remove(&self, id: u32) -> Result<(), libsignal_protocol::InternalError> {
        let path = self.dir.join(format!("{id}.bin"));
        if path.exists() {
            fs::remove_file(path).map_err(|_| libsignal_protocol::InternalError::Unknown)?;
        }
        Ok(())
    }
}

struct FileSignedPreKeyStore {
    dir: PathBuf,
}

impl SignedPreKeyStore for FileSignedPreKeyStore {
    fn load(&self, id: u32, writer: &mut dyn Write) -> io::Result<()> {
        writer.write_all(&fs::read(self.dir.join(format!("{id}.bin")))?)
    }

    fn store(&self, id: u32, body: &[u8]) -> Result<(), libsignal_protocol::InternalError> {
        write_atomic_bytes(&self.dir.join(format!("{id}.bin")), body)
            .map_err(|_| libsignal_protocol::InternalError::Unknown)
    }

    fn contains(&self, id: u32) -> bool {
        self.dir.join(format!("{id}.bin")).exists()
    }

    fn remove(&self, id: u32) -> Result<(), libsignal_protocol::InternalError> {
        let path = self.dir.join(format!("{id}.bin"));
        if path.exists() {
            fs::remove_file(path).map_err(|_| libsignal_protocol::InternalError::Unknown)?;
        }
        Ok(())
    }
}

struct FileSessionStore {
    dir: PathBuf,
}

impl SessionStore for FileSessionStore {
    fn load_session(
        &self,
        address: Address,
    ) -> Result<Option<SerializedSession>, libsignal_protocol::InternalError> {
        let path = address_path(&self.dir, &address, "json");
        if !path.exists() {
            return Ok(None);
        }
        let disk: SessionDisk = serde_json::from_slice(
            &fs::read(path).map_err(|_| libsignal_protocol::InternalError::Unknown)?,
        )
        .map_err(|_| libsignal_protocol::InternalError::Unknown)?;
        Ok(Some(SerializedSession {
            session: libsignal_protocol::Buffer::from(
                B64.decode(disk.session_b64.as_bytes())
                    .map_err(|_| libsignal_protocol::InternalError::Unknown)?,
            ),
            extra_data: match disk.extra_data_b64 {
                Some(extra) => Some(libsignal_protocol::Buffer::from(
                    B64.decode(extra.as_bytes())
                        .map_err(|_| libsignal_protocol::InternalError::Unknown)?,
                )),
                None => None,
            },
        }))
    }

    fn get_sub_device_sessions(
        &self,
        name: &[u8],
    ) -> Result<Vec<i32>, libsignal_protocol::InternalError> {
        let dir = self.dir.join(hex::encode(name));
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut ids = Vec::new();
        for entry in fs::read_dir(dir).map_err(|_| libsignal_protocol::InternalError::Unknown)? {
            let entry = entry.map_err(|_| libsignal_protocol::InternalError::Unknown)?;
            let path = entry.path();
            let stem = path
                .file_stem()
                .and_then(|value| value.to_str())
                .ok_or(libsignal_protocol::InternalError::Unknown)?;
            let parsed = stem
                .parse::<i32>()
                .map_err(|_| libsignal_protocol::InternalError::Unknown)?;
            ids.push(parsed);
        }
        Ok(ids)
    }

    fn contains_session(&self, addr: Address) -> Result<bool, libsignal_protocol::InternalError> {
        Ok(address_path(&self.dir, &addr, "json").exists())
    }

    fn store_session(
        &self,
        addr: Address,
        session: SerializedSession,
    ) -> Result<(), libsignal_protocol::InternalError> {
        let path = address_path(&self.dir, &addr, "json");
        let disk = SessionDisk {
            session_b64: B64.encode(session.session.as_slice()),
            extra_data_b64: session.extra_data.map(|value| B64.encode(value.as_slice())),
        };
        let bytes = serde_json::to_vec_pretty(&disk)
            .map_err(|_| libsignal_protocol::InternalError::Unknown)?;
        write_atomic_bytes(&path, &bytes).map_err(|_| libsignal_protocol::InternalError::Unknown)
    }

    fn delete_session(&self, addr: Address) -> Result<(), libsignal_protocol::InternalError> {
        let path = address_path(&self.dir, &addr, "json");
        if path.exists() {
            fs::remove_file(path).map_err(|_| libsignal_protocol::InternalError::Unknown)?;
        }
        Ok(())
    }

    fn delete_all_sessions(&self, name: &[u8]) -> Result<usize, libsignal_protocol::InternalError> {
        let dir = self.dir.join(hex::encode(name));
        if !dir.exists() {
            return Ok(0);
        }
        let count = fs::read_dir(&dir)
            .map_err(|_| libsignal_protocol::InternalError::Unknown)?
            .filter_map(|entry| entry.ok())
            .count();
        fs::remove_dir_all(&dir).map_err(|_| libsignal_protocol::InternalError::Unknown)?;
        Ok(count)
    }
}

struct FileIdentityStore {
    dir: PathBuf,
    public_key: Vec<u8>,
    private_key: Vec<u8>,
    registration_id: u32,
}

impl IdentityKeyStore for FileIdentityStore {
    fn identity_key_pair(
        &self,
    ) -> Result<
        (libsignal_protocol::Buffer, libsignal_protocol::Buffer),
        libsignal_protocol::InternalError,
    > {
        Ok((
            libsignal_protocol::Buffer::from(self.public_key.clone()),
            libsignal_protocol::Buffer::from(self.private_key.clone()),
        ))
    }

    fn local_registration_id(&self) -> Result<u32, libsignal_protocol::InternalError> {
        Ok(self.registration_id)
    }

    fn is_trusted_identity(
        &self,
        address: Address,
        identity_key: &[u8],
    ) -> Result<bool, libsignal_protocol::InternalError> {
        let path = address_path(&self.dir, &address, "bin");
        if !path.exists() {
            return Ok(true);
        }
        let stored = fs::read(path).map_err(|_| libsignal_protocol::InternalError::Unknown)?;
        Ok(stored == identity_key)
    }

    fn save_identity(
        &self,
        address: Address,
        identity_key: &[u8],
    ) -> Result<(), libsignal_protocol::InternalError> {
        let path = address_path(&self.dir, &address, "bin");
        if identity_key.is_empty() {
            if path.exists() {
                fs::remove_file(path).map_err(|_| libsignal_protocol::InternalError::Unknown)?;
            }
            return Ok(());
        }
        write_atomic_bytes(&path, identity_key)
            .map_err(|_| libsignal_protocol::InternalError::Unknown)
    }
}

fn choose_device_id(requested_device_id: u32, legacy_device_id: Option<u32>) -> u32 {
    if requested_device_id != 0 {
        requested_device_id
    } else {
        legacy_device_id
            .filter(|value| *value != 0)
            .unwrap_or_else(random_device_id)
    }
}

fn random_device_id() -> u32 {
    loop {
        let candidate = rand::random::<u32>() & 0x7fff_ffff;
        if candidate != 0 {
            return candidate;
        }
    }
}

fn normalize_public_key(bytes: &[u8]) -> Vec<u8> {
    if bytes.len() == 32 {
        let mut normalized = Vec::with_capacity(33);
        normalized.push(0x05);
        normalized.extend_from_slice(bytes);
        normalized
    } else {
        bytes.to_vec()
    }
}

fn write_atomic_bytes(path: &Path, body: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    let mut file = fs::File::create(&tmp)?;
    file.write_all(body)?;
    file.sync_all()?;
    fs::rename(tmp, path)?;
    Ok(())
}

fn address_path(base_dir: &Path, address: &Address, ext: &str) -> PathBuf {
    base_dir
        .join(hex::encode(address.bytes()))
        .join(format!("{}.{}", address.device_id(), ext))
}

pub fn registration_id_from_device_id(device_id: u32) -> u32 {
    let _ = device_id;
    // Legacy OMEMO bundles do not transport a Signal registration ID, so
    // interoperable implementations use 0 instead of inventing one from the
    // XMPP device id.
    0
}

fn identity_fingerprint(public_key: &[u8]) -> String {
    // Legacy OMEMO clients such as Gajim display the identity fingerprint as
    // the public identity key hex without the leading type byte.
    hex::encode(public_key.get(1..).unwrap_or(public_key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn initialize_creates_v2_store_and_prekeys() {
        let dir = TempDir::new().unwrap();
        let store = OmemoStore::new(dir.path().to_path_buf());

        let metadata = store.initialize(0).unwrap();

        assert_ne!(metadata.device_id, 0);
        assert!(metadata.prekeys_available >= MIN_PRE_KEY_COUNT);
        assert!(store.identity_path().exists());
        assert!(store.prekeys_dir().exists());
    }

    #[test]
    fn migration_preserves_legacy_device_id() {
        let dir = TempDir::new().unwrap();
        let legacy = LegacyIdentityDisk {
            device_id: 4242,
            ik_priv: hex::encode([7u8; 32]),
            ik_pub: hex::encode([9u8; 32]),
        };
        fs::write(
            dir.path().join("identity_key.json"),
            serde_json::to_vec_pretty(&legacy).unwrap(),
        )
        .unwrap();

        let store = OmemoStore::new(dir.path().to_path_buf());
        let metadata = store.initialize(4242).unwrap_err();
        assert!(metadata.to_string().contains("signal error"));
    }

    #[test]
    fn registration_id_is_zero_for_legacy_omemo() {
        assert_eq!(registration_id_from_device_id(0), 0);
        assert_eq!(registration_id_from_device_id(16_380), 0);
        assert_eq!(registration_id_from_device_id(42), 0);
    }

    #[test]
    fn identity_fingerprint_omits_signal_type_byte() {
        assert_eq!(identity_fingerprint(&[0x05, 0xaa, 0xbb, 0xcc]), "aabbcc");
        assert_eq!(identity_fingerprint(&[0xaa, 0xbb, 0xcc]), "bbcc");
        assert_eq!(identity_fingerprint(&[]), "");
    }
}
