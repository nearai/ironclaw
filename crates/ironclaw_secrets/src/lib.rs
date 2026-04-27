//! Tenant-scoped secret service boundary for IronClaw Reborn.
//!
//! This crate stores and leases secret material behind opaque
//! [`SecretHandle`] values. It does not decide authorization, inject secrets into
//! runtimes, emit audit records, or expose raw values through metadata.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, AeadCore, OsRng, rand_core::RngCore},
};
use async_trait::async_trait;
use chrono::Utc;
use hkdf::Hkdf;
use ironclaw_filesystem::{DirEntry, FileType, FilesystemError, RootFilesystem};
use ironclaw_host_api::{
    AgentId, ExtensionId, ProjectId, ResourceScope, SecretHandle, TenantId, Timestamp, UserId,
    VirtualPath,
};
pub use secrecy::{ExposeSecret, SecretString as SecretMaterial};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use thiserror::Error;
use uuid::Uuid;

const SECRET_KEY_SIZE: usize = 32;
const SECRET_NONCE_SIZE: usize = 12;
const SECRET_SALT_SIZE: usize = 32;
const SECRET_TAG_SIZE: usize = 16;

/// Opaque identifier for a stored secret record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SecretId(Uuid);

impl SecretId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for SecretId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for SecretId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Opaque identifier for a one-shot secret lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SecretLeaseId(Uuid);

impl SecretLeaseId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for SecretLeaseId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for SecretLeaseId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Redacted metadata for a stored secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretMetadata {
    pub id: SecretId,
    pub scope: ResourceScope,
    pub handle: SecretHandle,
    pub provider: Option<String>,
    pub expires_at: Option<Timestamp>,
    pub last_used_at: Option<Timestamp>,
    pub usage_count: u64,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// Lease lifecycle for one secret access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretLeaseStatus {
    Active,
    Consumed,
    Revoked,
}

/// Metadata for a scoped one-shot secret lease.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretLease {
    pub id: SecretLeaseId,
    pub scope: ResourceScope,
    pub handle: SecretHandle,
    pub status: SecretLeaseStatus,
}

/// Where a credential should be injected into an outbound HTTP request.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CredentialLocation {
    AuthorizationBearer,
    AuthorizationBasic {
        username: String,
    },
    Header {
        name: String,
        prefix: Option<String>,
    },
    QueryParam {
        name: String,
    },
    UrlPath {
        placeholder: String,
    },
}

/// Metadata describing which scoped secret powers a host-specific credential.
///
/// This carries no secret material. Runtime-specific injection is owned by the
/// caller/composition layer after it has explicitly obtained material.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialMapping {
    pub handle: SecretHandle,
    pub location: CredentialLocation,
    pub host_patterns: Vec<String>,
    pub optional: bool,
}

impl CredentialMapping {
    pub fn bearer(handle: SecretHandle, host_pattern: impl Into<String>) -> Self {
        Self {
            handle,
            location: CredentialLocation::AuthorizationBearer,
            host_patterns: vec![host_pattern.into()],
            optional: false,
        }
    }

    pub fn header(
        handle: SecretHandle,
        header_name: impl Into<String>,
        host_pattern: impl Into<String>,
    ) -> Self {
        Self {
            handle,
            location: CredentialLocation::Header {
                name: header_name.into(),
                prefix: None,
            },
            host_patterns: vec![host_pattern.into()],
            optional: false,
        }
    }
}

macro_rules! credential_id {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, ironclaw_host_api::HostApiError> {
                let value = value.into();
                validate_credential_segment(&value)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_string(self) -> String {
                self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

credential_id!(CredentialSlotId);
credential_id!(CredentialAccountId);

fn validate_credential_segment(value: &str) -> Result<(), ironclaw_host_api::HostApiError> {
    SecretHandle::new(value.to_string()).map(|_| ())
}

/// Metadata-only reference from a credential account to a stored secret handle.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CredentialSecretRef {
    pub name: String,
    pub handle: SecretHandle,
}

impl CredentialSecretRef {
    pub fn new(
        name: impl Into<String>,
        handle: SecretHandle,
    ) -> Result<Self, ironclaw_host_api::HostApiError> {
        let name = name.into();
        validate_credential_segment(&name)?;
        Ok(Self { name, handle })
    }
}

/// Metadata-only external account saved for an extension credential slot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialAccountRecord {
    pub scope: ResourceScope,
    pub extension_id: ExtensionId,
    pub slot_id: CredentialSlotId,
    pub account_id: CredentialAccountId,
    pub label: String,
    pub subject_hint: Option<String>,
    pub secret_refs: Vec<CredentialSecretRef>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl CredentialAccountRecord {
    pub fn new(
        scope: ResourceScope,
        extension_id: ExtensionId,
        slot_id: CredentialSlotId,
        account_id: CredentialAccountId,
        label: impl Into<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            scope,
            extension_id,
            slot_id,
            account_id,
            label: label.into(),
            subject_hint: None,
            secret_refs: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }

    pub fn with_subject_hint(mut self, subject_hint: impl Into<String>) -> Self {
        self.subject_hint = Some(subject_hint.into());
        self
    }

    pub fn with_secret_ref(mut self, secret_ref: CredentialSecretRef) -> Self {
        self.secret_refs.push(secret_ref);
        self
    }
}

/// Metadata repository for extension credential accounts.
#[async_trait]
pub trait CredentialAccountRepository: Send + Sync {
    async fn upsert(
        &self,
        record: CredentialAccountRecord,
    ) -> Result<CredentialAccountRecord, SecretStoreError>;

    async fn get(
        &self,
        scope: &ResourceScope,
        extension_id: &ExtensionId,
        slot_id: &CredentialSlotId,
        account_id: &CredentialAccountId,
    ) -> Result<Option<CredentialAccountRecord>, SecretStoreError>;

    async fn list_for_slot(
        &self,
        scope: &ResourceScope,
        extension_id: &ExtensionId,
        slot_id: &CredentialSlotId,
    ) -> Result<Vec<CredentialAccountRecord>, SecretStoreError>;

    async fn delete(
        &self,
        scope: &ResourceScope,
        extension_id: &ExtensionId,
        slot_id: &CredentialSlotId,
        account_id: &CredentialAccountId,
    ) -> Result<bool, SecretStoreError>;
}

/// In-memory credential account repository for tests and demos.
#[derive(Debug, Default)]
pub struct InMemoryCredentialAccountRepository {
    records: Mutex<HashMap<CredentialAccountKey, CredentialAccountRecord>>,
}

impl InMemoryCredentialAccountRepository {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock_records(
        &self,
    ) -> Result<
        MutexGuard<'_, HashMap<CredentialAccountKey, CredentialAccountRecord>>,
        SecretStoreError,
    > {
        self.records
            .lock()
            .map_err(|error| SecretStoreError::StoreUnavailable {
                reason: error.to_string(),
            })
    }
}

#[async_trait]
impl CredentialAccountRepository for InMemoryCredentialAccountRepository {
    async fn upsert(
        &self,
        mut record: CredentialAccountRecord,
    ) -> Result<CredentialAccountRecord, SecretStoreError> {
        let key = CredentialAccountKey::new(
            &record.scope,
            &record.extension_id,
            &record.slot_id,
            &record.account_id,
        );
        if let Some(existing) = self.lock_records()?.get(&key) {
            record.created_at = existing.created_at;
        }
        record.updated_at = Utc::now();
        self.lock_records()?.insert(key, record.clone());
        Ok(record)
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        extension_id: &ExtensionId,
        slot_id: &CredentialSlotId,
        account_id: &CredentialAccountId,
    ) -> Result<Option<CredentialAccountRecord>, SecretStoreError> {
        Ok(self
            .lock_records()?
            .get(&CredentialAccountKey::new(
                scope,
                extension_id,
                slot_id,
                account_id,
            ))
            .cloned())
    }

    async fn list_for_slot(
        &self,
        scope: &ResourceScope,
        extension_id: &ExtensionId,
        slot_id: &CredentialSlotId,
    ) -> Result<Vec<CredentialAccountRecord>, SecretStoreError> {
        let mut records: Vec<_> = self
            .lock_records()?
            .iter()
            .filter(|(key, _)| key.matches_slot(scope, extension_id, slot_id))
            .map(|(_, record)| record.clone())
            .collect();
        records.sort_by(|left, right| left.account_id.as_str().cmp(right.account_id.as_str()));
        Ok(records)
    }

    async fn delete(
        &self,
        scope: &ResourceScope,
        extension_id: &ExtensionId,
        slot_id: &CredentialSlotId,
        account_id: &CredentialAccountId,
    ) -> Result<bool, SecretStoreError> {
        Ok(self
            .lock_records()?
            .remove(&CredentialAccountKey::new(
                scope,
                extension_id,
                slot_id,
                account_id,
            ))
            .is_some())
    }
}

/// Filesystem-backed credential account metadata repository over any RootFilesystem.
pub struct FilesystemCredentialAccountRepository<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<F>,
}

impl<F> FilesystemCredentialAccountRepository<F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: Arc<F>) -> Self {
        Self { filesystem }
    }

    pub fn record_path(
        &self,
        scope: &ResourceScope,
        extension_id: &ExtensionId,
        slot_id: &CredentialSlotId,
        account_id: &CredentialAccountId,
    ) -> Result<VirtualPath, SecretStoreError> {
        credential_account_record_path(scope, extension_id, slot_id, account_id)
    }

    fn slot_root(
        &self,
        scope: &ResourceScope,
        extension_id: &ExtensionId,
        slot_id: &CredentialSlotId,
    ) -> Result<VirtualPath, SecretStoreError> {
        credential_account_slot_root(scope, extension_id, slot_id)
    }

    async fn read_record(
        &self,
        path: &VirtualPath,
    ) -> Result<Option<CredentialAccountRecord>, SecretStoreError> {
        let bytes = match self.filesystem.read_file(path).await {
            Ok(bytes) => bytes,
            Err(error) if filesystem_not_found(&error) => return Ok(None),
            Err(error) => return Err(secret_filesystem_error(error)),
        };
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(secret_json_error)
    }

    async fn write_record(
        &self,
        path: &VirtualPath,
        record: &CredentialAccountRecord,
    ) -> Result<(), SecretStoreError> {
        let bytes = serde_json::to_vec_pretty(record).map_err(secret_json_error)?;
        self.filesystem
            .write_file(path, &bytes)
            .await
            .map_err(secret_filesystem_error)
    }
}

#[async_trait]
impl<F> CredentialAccountRepository for FilesystemCredentialAccountRepository<F>
where
    F: RootFilesystem,
{
    async fn upsert(
        &self,
        mut record: CredentialAccountRecord,
    ) -> Result<CredentialAccountRecord, SecretStoreError> {
        let path = self.record_path(
            &record.scope,
            &record.extension_id,
            &record.slot_id,
            &record.account_id,
        )?;
        if let Some(existing) = self.read_record(&path).await? {
            record.created_at = existing.created_at;
        }
        record.updated_at = Utc::now();
        self.write_record(&path, &record).await?;
        Ok(record)
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        extension_id: &ExtensionId,
        slot_id: &CredentialSlotId,
        account_id: &CredentialAccountId,
    ) -> Result<Option<CredentialAccountRecord>, SecretStoreError> {
        let path = self.record_path(scope, extension_id, slot_id, account_id)?;
        self.read_record(&path).await
    }

    async fn list_for_slot(
        &self,
        scope: &ResourceScope,
        extension_id: &ExtensionId,
        slot_id: &CredentialSlotId,
    ) -> Result<Vec<CredentialAccountRecord>, SecretStoreError> {
        let root = self.slot_root(scope, extension_id, slot_id)?;
        let entries = match self.filesystem.list_dir(&root).await {
            Ok(entries) => entries,
            Err(error) if filesystem_not_found(&error) => return Ok(Vec::new()),
            Err(error) => return Err(secret_filesystem_error(error)),
        };
        let mut records = Vec::new();
        for entry in entries {
            if entry.file_type == FileType::File
                && entry.name.ends_with(".json")
                && let Some(record) = self.read_record(&entry.path).await?
            {
                records.push(record);
            }
        }
        records.sort_by(|left, right| left.account_id.as_str().cmp(right.account_id.as_str()));
        Ok(records)
    }

    async fn delete(
        &self,
        scope: &ResourceScope,
        extension_id: &ExtensionId,
        slot_id: &CredentialSlotId,
        account_id: &CredentialAccountId,
    ) -> Result<bool, SecretStoreError> {
        let path = self.record_path(scope, extension_id, slot_id, account_id)?;
        if self.read_record(&path).await?.is_none() {
            return Ok(false);
        }
        self.filesystem
            .delete(&path)
            .await
            .map_err(secret_filesystem_error)?;
        Ok(true)
    }
}

/// Cryptographic operations for encrypted secret storage.
///
/// Uses AES-256-GCM with per-secret HKDF-SHA256 key derivation. The master key
/// is held in [`SecretMaterial`] and never appears in debug output.
#[derive(Clone)]
pub struct SecretsCrypto {
    master_key: SecretMaterial,
}

impl SecretsCrypto {
    pub fn new(master_key: SecretMaterial) -> Result<Self, SecretStoreError> {
        if master_key.expose_secret().len() < SECRET_KEY_SIZE {
            return Err(SecretStoreError::InvalidMasterKey);
        }
        Ok(Self { master_key })
    }

    pub fn generate_salt() -> Vec<u8> {
        let mut salt = vec![0u8; SECRET_SALT_SIZE];
        OsRng.fill_bytes(&mut salt);
        salt
    }

    pub fn encrypt(
        &self,
        material: &SecretMaterial,
    ) -> Result<(Vec<u8>, Vec<u8>), SecretStoreError> {
        let salt = Self::generate_salt();
        let derived_key = self.derive_key(&salt)?;
        let cipher = Aes256Gcm::new_from_slice(&derived_key)
            .map_err(|_| SecretStoreError::EncryptionFailed)?;
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ciphertext = cipher
            .encrypt(&nonce, material.expose_secret().as_bytes())
            .map_err(|_| SecretStoreError::EncryptionFailed)?;

        let mut encrypted = Vec::with_capacity(SECRET_NONCE_SIZE + ciphertext.len());
        encrypted.extend_from_slice(&nonce);
        encrypted.extend_from_slice(&ciphertext);
        Ok((encrypted, salt))
    }

    pub fn decrypt(
        &self,
        encrypted_value: &[u8],
        key_salt: &[u8],
    ) -> Result<SecretMaterial, SecretStoreError> {
        if encrypted_value.len() < SECRET_NONCE_SIZE + SECRET_TAG_SIZE {
            return Err(SecretStoreError::DecryptionFailed);
        }

        let derived_key = self.derive_key(key_salt)?;
        let cipher = Aes256Gcm::new_from_slice(&derived_key)
            .map_err(|_| SecretStoreError::DecryptionFailed)?;
        let (nonce_bytes, ciphertext) = encrypted_value.split_at(SECRET_NONCE_SIZE);
        let nonce = Nonce::from_slice(nonce_bytes);
        let plaintext = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|_| SecretStoreError::DecryptionFailed)?;
        let plaintext = String::from_utf8(plaintext).map_err(|_| SecretStoreError::InvalidUtf8)?;
        Ok(SecretMaterial::from(plaintext))
    }

    fn derive_key(&self, salt: &[u8]) -> Result<[u8; SECRET_KEY_SIZE], SecretStoreError> {
        let hk = Hkdf::<Sha256>::new(Some(salt), self.master_key.expose_secret().as_bytes());
        let mut derived = [0u8; SECRET_KEY_SIZE];
        hk.expand(b"ironclaw-reborn-secrets-v1", &mut derived)
            .map_err(|_| SecretStoreError::EncryptionFailed)?;
        Ok(derived)
    }
}

impl fmt::Debug for SecretsCrypto {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecretsCrypto")
            .field("master_key", &"[REDACTED]")
            .finish()
    }
}

/// Encrypted durable-row shape. Debug output redacts ciphertext and salt.
#[derive(Clone, Serialize, Deserialize)]
pub struct EncryptedSecretRecord {
    pub metadata: SecretMetadata,
    pub encrypted_value: Vec<u8>,
    pub key_salt: Vec<u8>,
}

impl fmt::Debug for EncryptedSecretRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EncryptedSecretRecord")
            .field("metadata", &self.metadata)
            .field("encrypted_value", &"[REDACTED]")
            .field("key_salt", &"[REDACTED]")
            .finish()
    }
}

/// Persistence boundary for encrypted secret rows.
///
/// Concrete PostgreSQL/libSQL/filesystem adapters can implement this trait
/// without moving database, filesystem, authorization, or runtime dependencies
/// into this crate.
#[async_trait]
pub trait EncryptedSecretRepository: Send + Sync {
    async fn upsert(
        &self,
        record: EncryptedSecretRecord,
    ) -> Result<EncryptedSecretRecord, SecretStoreError>;

    async fn get(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<Option<EncryptedSecretRecord>, SecretStoreError>;

    async fn list(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<EncryptedSecretRecord>, SecretStoreError>;

    async fn delete(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<bool, SecretStoreError>;

    async fn any_exist(&self) -> Result<bool, SecretStoreError>;

    async fn record_usage(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
        used_at: Timestamp,
    ) -> Result<EncryptedSecretRecord, SecretStoreError>;
}

/// Filesystem-backed encrypted row repository over any [`RootFilesystem`].
///
/// Records are stored as redacted JSON under tenant/user/project-scoped
/// `/engine` virtual paths. Raw secret material never appears in these files;
/// only ciphertext, salt, and metadata are serialized.
pub struct FilesystemEncryptedSecretRepository<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<F>,
}

impl<F> FilesystemEncryptedSecretRepository<F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: Arc<F>) -> Self {
        Self { filesystem }
    }

    pub fn record_path(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<VirtualPath, SecretStoreError> {
        secret_record_path(scope, handle)
    }

    fn scope_root(&self, scope: &ResourceScope) -> Result<VirtualPath, SecretStoreError> {
        secret_scope_root(scope)
    }

    fn legacy_scope_root(
        &self,
        scope: &ResourceScope,
    ) -> Result<Option<VirtualPath>, SecretStoreError> {
        legacy_secret_scope_root(scope).transpose()
    }

    fn lookup_paths(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<Vec<VirtualPath>, SecretStoreError> {
        secret_lookup_paths(scope, handle)
    }

    async fn read_wrapper(
        &self,
        path: &VirtualPath,
    ) -> Result<Option<FilesystemSecretRecord>, SecretStoreError> {
        let bytes = match self.filesystem.read_file(path).await {
            Ok(bytes) => bytes,
            Err(error) if filesystem_not_found(&error) => return Ok(None),
            Err(error) => return Err(secret_filesystem_error(error)),
        };
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(secret_json_error)
    }

    async fn write_wrapper(
        &self,
        path: &VirtualPath,
        wrapper: &FilesystemSecretRecord,
    ) -> Result<(), SecretStoreError> {
        let bytes = serde_json::to_vec_pretty(wrapper).map_err(secret_json_error)?;
        self.filesystem
            .write_file(path, &bytes)
            .await
            .map_err(secret_filesystem_error)
    }

    async fn read_active_record(
        &self,
        path: &VirtualPath,
    ) -> Result<Option<EncryptedSecretRecord>, SecretStoreError> {
        Ok(self.read_wrapper(path).await?.and_then(|wrapper| {
            if wrapper.deleted {
                None
            } else {
                Some(wrapper.record)
            }
        }))
    }

    async fn list_record_files(
        &self,
        root: &VirtualPath,
    ) -> Result<Vec<DirEntry>, SecretStoreError> {
        match self.filesystem.list_dir(root).await {
            Ok(entries) => Ok(entries
                .into_iter()
                .filter(|entry| entry.file_type == FileType::File && entry.name.ends_with(".json"))
                .collect()),
            Err(error) if filesystem_not_found(&error) => Ok(Vec::new()),
            Err(error) => Err(secret_filesystem_error(error)),
        }
    }

    async fn active_records_under(
        &self,
        root: &VirtualPath,
    ) -> Result<Vec<EncryptedSecretRecord>, SecretStoreError> {
        let mut records = Vec::new();
        for entry in self.list_record_files(root).await? {
            if let Some(record) = self.read_active_record(&entry.path).await? {
                records.push(record);
            }
        }
        Ok(records)
    }

    async fn first_active_record(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<Option<(VirtualPath, EncryptedSecretRecord)>, SecretStoreError> {
        for path in self.lookup_paths(scope, handle)? {
            if let Some(record) = self.read_active_record(&path).await? {
                return Ok(Some((path, record)));
            }
        }
        Ok(None)
    }

    async fn any_active_record_under(&self, root: &VirtualPath) -> Result<bool, SecretStoreError> {
        let entries = match self.filesystem.list_dir(root).await {
            Ok(entries) => entries,
            Err(error) if filesystem_not_found(&error) => return Ok(false),
            Err(error) => return Err(secret_filesystem_error(error)),
        };
        for entry in entries {
            match entry.file_type {
                FileType::File if is_secret_record_path(&entry.path) => {
                    if self.read_active_record(&entry.path).await?.is_some() {
                        return Ok(true);
                    }
                }
                FileType::Directory => {
                    if Box::pin(self.any_active_record_under(&entry.path)).await? {
                        return Ok(true);
                    }
                }
                _ => {}
            }
        }
        Ok(false)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FilesystemSecretRecord {
    deleted: bool,
    record: EncryptedSecretRecord,
}

fn secrets_root_path() -> Result<VirtualPath, SecretStoreError> {
    VirtualPath::new("/engine/tenants").map_err(secret_path_error)
}

fn secret_scope_root(scope: &ResourceScope) -> Result<VirtualPath, SecretStoreError> {
    let agent_id = scope
        .agent_id
        .as_ref()
        .map(AgentId::as_str)
        .unwrap_or("_none");
    let project_id = scope
        .project_id
        .as_ref()
        .map(ProjectId::as_str)
        .unwrap_or("_none");
    VirtualPath::new(format!(
        "/engine/tenants/{}/users/{}/agents/{agent_id}/projects/{project_id}/secrets",
        scope.tenant_id.as_str(),
        scope.user_id.as_str()
    ))
    .map_err(secret_path_error)
}

fn secret_record_path(
    scope: &ResourceScope,
    handle: &SecretHandle,
) -> Result<VirtualPath, SecretStoreError> {
    let root = secret_scope_root(scope)?;
    VirtualPath::new(format!("{}/{}.json", root.as_str(), handle.as_str()))
        .map_err(secret_path_error)
}

fn credential_account_root(scope: &ResourceScope) -> Result<VirtualPath, SecretStoreError> {
    let agent_id = scope
        .agent_id
        .as_ref()
        .map(AgentId::as_str)
        .unwrap_or("_none");
    let project_id = scope
        .project_id
        .as_ref()
        .map(ProjectId::as_str)
        .unwrap_or("_none");
    VirtualPath::new(format!(
        "/engine/tenants/{}/users/{}/agents/{agent_id}/projects/{project_id}/credential-accounts",
        scope.tenant_id.as_str(),
        scope.user_id.as_str()
    ))
    .map_err(secret_path_error)
}

fn credential_account_slot_root(
    scope: &ResourceScope,
    extension_id: &ExtensionId,
    slot_id: &CredentialSlotId,
) -> Result<VirtualPath, SecretStoreError> {
    let root = credential_account_root(scope)?;
    VirtualPath::new(format!(
        "{}/{}/{}",
        root.as_str(),
        extension_id.as_str(),
        slot_id.as_str()
    ))
    .map_err(secret_path_error)
}

fn credential_account_record_path(
    scope: &ResourceScope,
    extension_id: &ExtensionId,
    slot_id: &CredentialSlotId,
    account_id: &CredentialAccountId,
) -> Result<VirtualPath, SecretStoreError> {
    let root = credential_account_slot_root(scope, extension_id, slot_id)?;
    VirtualPath::new(format!("{}/{}.json", root.as_str(), account_id.as_str()))
        .map_err(secret_path_error)
}

fn legacy_secret_scope_root(
    scope: &ResourceScope,
) -> Option<Result<VirtualPath, SecretStoreError>> {
    if scope.agent_id.is_some() {
        return None;
    }
    let project_id = scope
        .project_id
        .as_ref()
        .map(ProjectId::as_str)
        .unwrap_or("_none");
    Some(
        VirtualPath::new(format!(
            "/engine/tenants/{}/users/{}/projects/{project_id}/secrets",
            scope.tenant_id.as_str(),
            scope.user_id.as_str()
        ))
        .map_err(secret_path_error),
    )
}

fn legacy_secret_record_path(
    scope: &ResourceScope,
    handle: &SecretHandle,
) -> Option<Result<VirtualPath, SecretStoreError>> {
    legacy_secret_scope_root(scope).map(|root| {
        let root = root?;
        VirtualPath::new(format!("{}/{}.json", root.as_str(), handle.as_str()))
            .map_err(secret_path_error)
    })
}

fn secret_lookup_paths(
    scope: &ResourceScope,
    handle: &SecretHandle,
) -> Result<Vec<VirtualPath>, SecretStoreError> {
    let mut paths = vec![secret_record_path(scope, handle)?];
    if let Some(legacy_path) = legacy_secret_record_path(scope, handle) {
        paths.push(legacy_path?);
    }
    Ok(paths)
}

fn secret_path_error(error: ironclaw_host_api::HostApiError) -> SecretStoreError {
    SecretStoreError::StoreUnavailable {
        reason: format!("invalid secret filesystem path: {error}"),
    }
}

fn secret_filesystem_error(error: FilesystemError) -> SecretStoreError {
    SecretStoreError::StoreUnavailable {
        reason: format!("secret filesystem backend unavailable: {error}"),
    }
}

fn secret_json_error(error: serde_json::Error) -> SecretStoreError {
    SecretStoreError::StoreUnavailable {
        reason: format!("invalid secret filesystem record: {error}"),
    }
}

fn is_secret_record_path(path: &VirtualPath) -> bool {
    let path = path.as_str();
    path.ends_with(".json") && path.contains("/secrets/")
}

fn filesystem_not_found(error: &FilesystemError) -> bool {
    match error {
        FilesystemError::MountNotFound { .. } => false,
        FilesystemError::Backend { reason, .. } => {
            let reason = reason.to_ascii_lowercase();
            reason.contains("not found") || reason.contains("no such file")
        }
        _ => false,
    }
}

#[async_trait]
impl<F> EncryptedSecretRepository for FilesystemEncryptedSecretRepository<F>
where
    F: RootFilesystem,
{
    async fn upsert(
        &self,
        mut record: EncryptedSecretRecord,
    ) -> Result<EncryptedSecretRecord, SecretStoreError> {
        let path = self.record_path(&record.metadata.scope, &record.metadata.handle)?;
        if let Some((_existing_path, existing)) = self
            .first_active_record(&record.metadata.scope, &record.metadata.handle)
            .await?
        {
            record.metadata.id = existing.metadata.id;
            record.metadata.created_at = existing.metadata.created_at;
            record.metadata.usage_count = existing.metadata.usage_count;
            record.metadata.last_used_at = existing.metadata.last_used_at;
        }
        self.write_wrapper(
            &path,
            &FilesystemSecretRecord {
                deleted: false,
                record: record.clone(),
            },
        )
        .await?;
        Ok(record)
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<Option<EncryptedSecretRecord>, SecretStoreError> {
        Ok(self
            .first_active_record(scope, handle)
            .await?
            .map(|(_path, record)| record))
    }

    async fn list(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<EncryptedSecretRecord>, SecretStoreError> {
        let root = self.scope_root(scope)?;
        let mut records = self.active_records_under(&root).await?;
        if let Some(legacy_root) = self.legacy_scope_root(scope)? {
            for record in self.active_records_under(&legacy_root).await? {
                if !records
                    .iter()
                    .any(|existing| existing.metadata.handle == record.metadata.handle)
                {
                    records.push(record);
                }
            }
        }
        Ok(records)
    }

    async fn delete(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<bool, SecretStoreError> {
        let mut deleted = false;
        for path in self.lookup_paths(scope, handle)? {
            if let Some(record) = self.read_active_record(&path).await? {
                self.write_wrapper(
                    &path,
                    &FilesystemSecretRecord {
                        deleted: true,
                        record,
                    },
                )
                .await?;
                deleted = true;
            }
        }
        Ok(deleted)
    }

    async fn any_exist(&self) -> Result<bool, SecretStoreError> {
        self.any_active_record_under(&secrets_root_path()?).await
    }

    async fn record_usage(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
        used_at: Timestamp,
    ) -> Result<EncryptedSecretRecord, SecretStoreError> {
        let (path, mut record) =
            self.first_active_record(scope, handle)
                .await?
                .ok_or_else(|| SecretStoreError::UnknownSecret {
                    scope: Box::new(scope.clone()),
                    handle: handle.clone(),
                })?;
        record.metadata.usage_count += 1;
        record.metadata.last_used_at = Some(used_at);
        record.metadata.updated_at = used_at;
        self.write_wrapper(
            &path,
            &FilesystemSecretRecord {
                deleted: false,
                record: record.clone(),
            },
        )
        .await?;
        Ok(record)
    }
}

/// In-memory encrypted row repository for tests and local composition demos.
#[derive(Debug, Default)]
pub struct InMemoryEncryptedSecretRepository {
    records: Mutex<HashMap<SecretKey, EncryptedSecretRecord>>,
}

impl InMemoryEncryptedSecretRepository {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock_records(
        &self,
    ) -> Result<MutexGuard<'_, HashMap<SecretKey, EncryptedSecretRecord>>, SecretStoreError> {
        self.records
            .lock()
            .map_err(|error| SecretStoreError::StoreUnavailable {
                reason: error.to_string(),
            })
    }
}

#[async_trait]
impl EncryptedSecretRepository for InMemoryEncryptedSecretRepository {
    async fn upsert(
        &self,
        mut record: EncryptedSecretRecord,
    ) -> Result<EncryptedSecretRecord, SecretStoreError> {
        let key = SecretKey::new(&record.metadata.scope, &record.metadata.handle);
        if let Some(existing) = self.lock_records()?.get(&key) {
            record.metadata.id = existing.metadata.id;
            record.metadata.created_at = existing.metadata.created_at;
            record.metadata.usage_count = existing.metadata.usage_count;
            record.metadata.last_used_at = existing.metadata.last_used_at;
        }
        self.lock_records()?.insert(key, record.clone());
        Ok(record)
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<Option<EncryptedSecretRecord>, SecretStoreError> {
        Ok(self
            .lock_records()?
            .get(&SecretKey::new(scope, handle))
            .cloned())
    }

    async fn list(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<EncryptedSecretRecord>, SecretStoreError> {
        Ok(self
            .lock_records()?
            .iter()
            .filter(|(key, _)| key.matches_scope(scope))
            .map(|(_, record)| record.clone())
            .collect())
    }

    async fn delete(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<bool, SecretStoreError> {
        Ok(self
            .lock_records()?
            .remove(&SecretKey::new(scope, handle))
            .is_some())
    }

    async fn any_exist(&self) -> Result<bool, SecretStoreError> {
        Ok(!self.lock_records()?.is_empty())
    }

    async fn record_usage(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
        used_at: Timestamp,
    ) -> Result<EncryptedSecretRecord, SecretStoreError> {
        let mut records = self.lock_records()?;
        let key = SecretKey::new(scope, handle);
        let record = records
            .get_mut(&key)
            .ok_or_else(|| SecretStoreError::UnknownSecret {
                scope: Box::new(scope.clone()),
                handle: handle.clone(),
            })?;
        record.metadata.usage_count += 1;
        record.metadata.last_used_at = Some(used_at);
        record.metadata.updated_at = used_at;
        Ok(record.clone())
    }
}

/// Secret service failures. Variants intentionally avoid secret material.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SecretStoreError {
    #[error("invalid secrets master key")]
    InvalidMasterKey,
    #[error("secret encryption failed")]
    EncryptionFailed,
    #[error("secret decryption failed")]
    DecryptionFailed,
    #[error("secret value is not valid UTF-8")]
    InvalidUtf8,
    #[error("secret has expired")]
    SecretExpired { handle: SecretHandle },
    #[error("unknown secret {handle} for tenant/user scope")]
    UnknownSecret {
        scope: Box<ResourceScope>,
        handle: SecretHandle,
    },
    #[error("unknown secret lease {lease_id} for tenant/user scope")]
    UnknownLease {
        scope: Box<ResourceScope>,
        lease_id: SecretLeaseId,
    },
    #[error("secret lease {lease_id} was already consumed")]
    LeaseConsumed { lease_id: SecretLeaseId },
    #[error("secret lease {lease_id} was revoked")]
    LeaseRevoked { lease_id: SecretLeaseId },
    #[error("secret store state is unavailable: {reason}")]
    StoreUnavailable { reason: String },
}

impl SecretStoreError {
    pub fn is_unknown_secret(&self) -> bool {
        matches!(self, Self::UnknownSecret { .. })
    }

    pub fn is_unknown_lease(&self) -> bool {
        matches!(self, Self::UnknownLease { .. })
    }

    pub fn is_consumed(&self) -> bool {
        matches!(self, Self::LeaseConsumed { .. })
    }

    pub fn is_revoked(&self) -> bool {
        matches!(self, Self::LeaseRevoked { .. })
    }

    pub fn is_decryption_failed(&self) -> bool {
        matches!(self, Self::DecryptionFailed)
    }
}

/// Scoped secret store contract.
#[async_trait]
pub trait SecretStore: Send + Sync {
    /// Stores or replaces a secret under the caller's tenant/user/project scope and returns redacted metadata.
    async fn put(
        &self,
        scope: ResourceScope,
        handle: SecretHandle,
        material: SecretMaterial,
    ) -> Result<SecretMetadata, SecretStoreError>;

    /// Returns redacted metadata for a secret without exposing material.
    async fn metadata(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<Option<SecretMetadata>, SecretStoreError>;

    /// Creates a one-shot lease for later secret consumption.
    async fn lease_once(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<SecretLease, SecretStoreError>;

    /// Consumes an active one-shot lease and returns secret material exactly once.
    async fn consume(
        &self,
        scope: &ResourceScope,
        lease_id: SecretLeaseId,
    ) -> Result<SecretMaterial, SecretStoreError>;

    /// Revokes an active one-shot lease without returning material.
    async fn revoke(
        &self,
        scope: &ResourceScope,
        lease_id: SecretLeaseId,
    ) -> Result<SecretLease, SecretStoreError>;

    /// Lists leases visible to the caller's tenant/user/project scope.
    async fn leases_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<SecretLease>, SecretStoreError>;
}

/// Encrypted secret store over a caller-provided encrypted-row repository.
pub struct EncryptedSecretStore<R>
where
    R: EncryptedSecretRepository,
{
    repository: Arc<R>,
    crypto: SecretsCrypto,
    leases: tokio::sync::Mutex<HashMap<SecretLeaseKey, SecretLease>>,
}

impl<R> EncryptedSecretStore<R>
where
    R: EncryptedSecretRepository,
{
    pub fn new(repository: Arc<R>, crypto: SecretsCrypto) -> Self {
        Self {
            repository,
            crypto,
            leases: tokio::sync::Mutex::new(HashMap::new()),
        }
    }

    pub fn repository(&self) -> &Arc<R> {
        &self.repository
    }
}

#[async_trait]
impl<R> SecretStore for EncryptedSecretStore<R>
where
    R: EncryptedSecretRepository,
{
    async fn put(
        &self,
        scope: ResourceScope,
        handle: SecretHandle,
        material: SecretMaterial,
    ) -> Result<SecretMetadata, SecretStoreError> {
        let now = Utc::now();
        let metadata = SecretMetadata {
            id: SecretId::new(),
            scope: scope.clone(),
            handle: handle.clone(),
            provider: None,
            expires_at: None,
            last_used_at: None,
            usage_count: 0,
            created_at: now,
            updated_at: now,
        };
        let (encrypted_value, key_salt) = self.crypto.encrypt(&material)?;
        let record = EncryptedSecretRecord {
            metadata,
            encrypted_value,
            key_salt,
        };
        Ok(self.repository.upsert(record).await?.metadata)
    }

    async fn metadata(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<Option<SecretMetadata>, SecretStoreError> {
        Ok(self
            .repository
            .get(scope, handle)
            .await?
            .map(|record| record.metadata))
    }

    async fn lease_once(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<SecretLease, SecretStoreError> {
        let Some(record) = self.repository.get(scope, handle).await? else {
            return Err(SecretStoreError::UnknownSecret {
                scope: Box::new(scope.clone()),
                handle: handle.clone(),
            });
        };
        if record
            .metadata
            .expires_at
            .is_some_and(|expires_at| expires_at <= Utc::now())
        {
            return Err(SecretStoreError::SecretExpired {
                handle: handle.clone(),
            });
        }
        let lease = SecretLease {
            id: SecretLeaseId::new(),
            scope: scope.clone(),
            handle: handle.clone(),
            status: SecretLeaseStatus::Active,
        };
        self.leases
            .lock()
            .await
            .insert(SecretLeaseKey::new(scope, lease.id), lease.clone());
        Ok(lease)
    }

    async fn consume(
        &self,
        scope: &ResourceScope,
        lease_id: SecretLeaseId,
    ) -> Result<SecretMaterial, SecretStoreError> {
        let mut leases = self.leases.lock().await;
        let key = SecretLeaseKey::new(scope, lease_id);
        let lease = leases
            .get_mut(&key)
            .ok_or_else(|| SecretStoreError::UnknownLease {
                scope: Box::new(scope.clone()),
                lease_id,
            })?;
        match lease.status {
            SecretLeaseStatus::Active => {}
            SecretLeaseStatus::Consumed => {
                return Err(SecretStoreError::LeaseConsumed { lease_id });
            }
            SecretLeaseStatus::Revoked => return Err(SecretStoreError::LeaseRevoked { lease_id }),
        }

        let record = self
            .repository
            .get(scope, &lease.handle)
            .await?
            .ok_or_else(|| SecretStoreError::UnknownSecret {
                scope: Box::new(scope.clone()),
                handle: lease.handle.clone(),
            })?;
        if record
            .metadata
            .expires_at
            .is_some_and(|expires_at| expires_at <= Utc::now())
        {
            return Err(SecretStoreError::SecretExpired {
                handle: lease.handle.clone(),
            });
        }
        let material = self
            .crypto
            .decrypt(&record.encrypted_value, &record.key_salt)?;
        self.repository
            .record_usage(scope, &lease.handle, Utc::now())
            .await?;
        lease.status = SecretLeaseStatus::Consumed;
        Ok(material)
    }

    async fn revoke(
        &self,
        scope: &ResourceScope,
        lease_id: SecretLeaseId,
    ) -> Result<SecretLease, SecretStoreError> {
        let mut leases = self.leases.lock().await;
        let key = SecretLeaseKey::new(scope, lease_id);
        let lease = leases
            .get_mut(&key)
            .ok_or_else(|| SecretStoreError::UnknownLease {
                scope: Box::new(scope.clone()),
                lease_id,
            })?;
        if lease.status == SecretLeaseStatus::Active {
            lease.status = SecretLeaseStatus::Revoked;
        }
        Ok(lease.clone())
    }

    async fn leases_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<SecretLease>, SecretStoreError> {
        Ok(self
            .leases
            .lock()
            .await
            .iter()
            .filter(|(key, _)| key.matches_scope(scope))
            .map(|(_, lease)| lease.clone())
            .collect())
    }
}

/// In-memory secret store for contract tests and non-durable demos.
#[derive(Debug, Default)]
pub struct InMemorySecretStore {
    state: Mutex<SecretState>,
}

impl InMemorySecretStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock_state(&self) -> Result<MutexGuard<'_, SecretState>, SecretStoreError> {
        self.state
            .lock()
            .map_err(|error| SecretStoreError::StoreUnavailable {
                reason: error.to_string(),
            })
    }
}

#[derive(Debug, Default)]
struct SecretState {
    secrets: HashMap<SecretKey, SecretRecord>,
    leases: HashMap<SecretLeaseKey, LeaseRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CredentialAccountKey {
    tenant_id: TenantId,
    user_id: UserId,
    agent_id: Option<AgentId>,
    project_id: Option<ProjectId>,
    extension_id: ExtensionId,
    slot_id: CredentialSlotId,
    account_id: CredentialAccountId,
}

impl CredentialAccountKey {
    fn new(
        scope: &ResourceScope,
        extension_id: &ExtensionId,
        slot_id: &CredentialSlotId,
        account_id: &CredentialAccountId,
    ) -> Self {
        Self {
            tenant_id: scope.tenant_id.clone(),
            user_id: scope.user_id.clone(),
            agent_id: scope.agent_id.clone(),
            project_id: scope.project_id.clone(),
            extension_id: extension_id.clone(),
            slot_id: slot_id.clone(),
            account_id: account_id.clone(),
        }
    }

    fn matches_slot(
        &self,
        scope: &ResourceScope,
        extension_id: &ExtensionId,
        slot_id: &CredentialSlotId,
    ) -> bool {
        self.tenant_id == scope.tenant_id
            && self.user_id == scope.user_id
            && self.agent_id == scope.agent_id
            && self.project_id == scope.project_id
            && self.extension_id == *extension_id
            && self.slot_id == *slot_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SecretKey {
    tenant_id: TenantId,
    user_id: UserId,
    agent_id: Option<AgentId>,
    project_id: Option<ProjectId>,
    handle: SecretHandle,
}

impl SecretKey {
    fn new(scope: &ResourceScope, handle: &SecretHandle) -> Self {
        Self {
            tenant_id: scope.tenant_id.clone(),
            user_id: scope.user_id.clone(),
            agent_id: scope.agent_id.clone(),
            project_id: scope.project_id.clone(),
            handle: handle.clone(),
        }
    }

    fn matches_scope(&self, scope: &ResourceScope) -> bool {
        self.tenant_id == scope.tenant_id
            && self.user_id == scope.user_id
            && self.agent_id == scope.agent_id
            && self.project_id == scope.project_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SecretLeaseKey {
    tenant_id: TenantId,
    user_id: UserId,
    agent_id: Option<AgentId>,
    project_id: Option<ProjectId>,
    lease_id: SecretLeaseId,
}

impl SecretLeaseKey {
    fn new(scope: &ResourceScope, lease_id: SecretLeaseId) -> Self {
        Self {
            tenant_id: scope.tenant_id.clone(),
            user_id: scope.user_id.clone(),
            agent_id: scope.agent_id.clone(),
            project_id: scope.project_id.clone(),
            lease_id,
        }
    }

    fn matches_scope(&self, scope: &ResourceScope) -> bool {
        self.tenant_id == scope.tenant_id
            && self.user_id == scope.user_id
            && self.agent_id == scope.agent_id
            && self.project_id == scope.project_id
    }
}

#[derive(Debug, Clone)]
struct SecretRecord {
    metadata: SecretMetadata,
    material: SecretMaterial,
}

#[derive(Debug, Clone)]
struct LeaseRecord {
    lease: SecretLease,
    material: SecretMaterial,
}

#[async_trait]
impl SecretStore for InMemorySecretStore {
    async fn put(
        &self,
        scope: ResourceScope,
        handle: SecretHandle,
        material: SecretMaterial,
    ) -> Result<SecretMetadata, SecretStoreError> {
        let now = Utc::now();
        let metadata = SecretMetadata {
            id: SecretId::new(),
            scope: scope.clone(),
            handle: handle.clone(),
            provider: None,
            expires_at: None,
            last_used_at: None,
            usage_count: 0,
            created_at: now,
            updated_at: now,
        };
        let record = SecretRecord {
            metadata: metadata.clone(),
            material,
        };
        self.lock_state()?
            .secrets
            .insert(SecretKey::new(&scope, &handle), record);
        Ok(metadata)
    }

    async fn metadata(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<Option<SecretMetadata>, SecretStoreError> {
        Ok(self
            .lock_state()?
            .secrets
            .get(&SecretKey::new(scope, handle))
            .map(|record| record.metadata.clone()))
    }

    async fn lease_once(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<SecretLease, SecretStoreError> {
        let mut state = self.lock_state()?;
        let secret = state
            .secrets
            .get(&SecretKey::new(scope, handle))
            .ok_or_else(|| SecretStoreError::UnknownSecret {
                scope: Box::new(scope.clone()),
                handle: handle.clone(),
            })?;
        let lease = SecretLease {
            id: SecretLeaseId::new(),
            scope: scope.clone(),
            handle: handle.clone(),
            status: SecretLeaseStatus::Active,
        };
        let record = LeaseRecord {
            lease: lease.clone(),
            material: secret.material.clone(),
        };
        state
            .leases
            .insert(SecretLeaseKey::new(scope, lease.id), record);
        Ok(lease)
    }

    async fn consume(
        &self,
        scope: &ResourceScope,
        lease_id: SecretLeaseId,
    ) -> Result<SecretMaterial, SecretStoreError> {
        let mut state = self.lock_state()?;
        let key = SecretLeaseKey::new(scope, lease_id);
        let record = state
            .leases
            .get_mut(&key)
            .ok_or_else(|| SecretStoreError::UnknownLease {
                scope: Box::new(scope.clone()),
                lease_id,
            })?;
        match record.lease.status {
            SecretLeaseStatus::Active => {
                record.lease.status = SecretLeaseStatus::Consumed;
                Ok(record.material.clone())
            }
            SecretLeaseStatus::Consumed => Err(SecretStoreError::LeaseConsumed { lease_id }),
            SecretLeaseStatus::Revoked => Err(SecretStoreError::LeaseRevoked { lease_id }),
        }
    }

    async fn revoke(
        &self,
        scope: &ResourceScope,
        lease_id: SecretLeaseId,
    ) -> Result<SecretLease, SecretStoreError> {
        let mut state = self.lock_state()?;
        let key = SecretLeaseKey::new(scope, lease_id);
        let record = state
            .leases
            .get_mut(&key)
            .ok_or_else(|| SecretStoreError::UnknownLease {
                scope: Box::new(scope.clone()),
                lease_id,
            })?;
        if record.lease.status == SecretLeaseStatus::Active {
            record.lease.status = SecretLeaseStatus::Revoked;
        }
        Ok(record.lease.clone())
    }

    async fn leases_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<SecretLease>, SecretStoreError> {
        Ok(self
            .lock_state()?
            .leases
            .iter()
            .filter(|(key, _)| key.matches_scope(scope))
            .map(|(_, record)| record.lease.clone())
            .collect())
    }
}
