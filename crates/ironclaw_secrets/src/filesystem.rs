//! Filesystem-backed durable secret storage.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_filesystem::{
    CasExpectation, Entry, FileType, FilesystemError, Filter, IndexKey, IndexValue, Page,
    RecordKind, RootFilesystem, ScopedFilesystem, VersionedEntry,
};
use ironclaw_host_api::{
    MountAlias, MountGrant, MountPermissions, MountView, ScopedPath, VirtualPath,
};
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::legacy_store::{DecryptedSecret, Secret, SecretRef};
use crate::{CreateSecretParams, SecretConsumeResult, SecretError, SecretsStore};

const SECRET_RECORD_KIND: &str = "secret_record";
const SECRET_KEY_CHECK_KIND: &str = "secret_store_key_check";
const SECRET_STORE_KEY_CHECK_ID: &str = "active";
const SECRET_STORE_KEY_CHECK_PLAINTEXT: &str = "reborn-secret-store-key-check-v1";
const SECRET_ID_INDEX_KEY: &str = "secret_id";

/// Durable [`SecretsStore`] implementation over the unified Reborn filesystem surface.
#[derive(Debug)]
pub struct FilesystemSecretsStore<F> {
    filesystem: ScopedFilesystem<F>,
}

impl<F> FilesystemSecretsStore<F>
where
    F: RootFilesystem + 'static,
{
    pub fn new(filesystem: ScopedFilesystem<F>) -> Self {
        Self { filesystem }
    }

    pub fn over_root(root: Arc<F>) -> Result<Self, SecretError> {
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/secrets").map_err(secret_filesystem_error)?,
            VirtualPath::new("/secrets").map_err(secret_filesystem_error)?,
            MountPermissions {
                read: true,
                write: true,
                delete: true,
                list: true,
                execute: false,
            },
        )])
        .map_err(secret_filesystem_error)?;
        Ok(Self::new(ScopedFilesystem::new(root, mounts)))
    }

    pub async fn verify_can_decrypt_existing_secrets(&self) -> Result<(), SecretError> {
        if let Some(check) = self.get_optional_entry(&key_check_path()?).await? {
            let record: StoredKeyCheck = parse_entry(check)?;
            return verify_secret_store_key_check(&record);
        }

        self.verify_all_secret_payloads().await?;
        let record = build_key_check_record();
        let entry = record_entry(SECRET_KEY_CHECK_KIND, &record)?;
        match self
            .filesystem
            .put(&key_check_path()?, entry, CasExpectation::Absent)
            .await
        {
            Ok(_) => {}
            Err(FilesystemError::VersionMismatch { .. }) => {}
            Err(error) => return Err(secret_filesystem_error(error)),
        }
        let Some(check) = self.get_optional_entry(&key_check_path()?).await? else {
            return Err(SecretError::Database(
                "secret store key check missing after bootstrap".to_string(),
            ));
        };
        let record: StoredKeyCheck = parse_entry(check)?;
        verify_secret_store_key_check(&record)
    }

    async fn get_optional_entry(
        &self,
        path: &ScopedPath,
    ) -> Result<Option<VersionedEntry>, SecretError> {
        match self.filesystem.get(path).await {
            Ok(entry) => Ok(entry),
            Err(FilesystemError::NotFound { .. }) => Ok(None),
            Err(error) => Err(secret_filesystem_error(error)),
        }
    }

    async fn get_secret_entry(
        &self,
        user_id: &str,
        name: &str,
    ) -> Result<Option<VersionedEntry>, SecretError> {
        self.get_optional_entry(&record_path(user_id, name)?).await
    }

    async fn get_stored_secret_record(
        &self,
        user_id: &str,
        name: &str,
    ) -> Result<StoredSecret, SecretError> {
        let name = normalize_secret_name(name);
        let Some(entry) = self.get_secret_entry(user_id, &name).await? else {
            return Err(SecretError::NotFound(name));
        };
        let secret: StoredSecret = parse_entry(entry)?;
        ensure_stored_secret_not_expired(&secret)?;
        Ok(secret)
    }

    async fn get_secret_record(&self, user_id: &str, name: &str) -> Result<Secret, SecretError> {
        Ok(self.get_stored_secret_record(user_id, name).await?.into())
    }

    async fn verify_all_secret_payloads(&self) -> Result<(), SecretError> {
        for user_dir in self.list_dir_or_empty(&records_root_path()?).await? {
            if user_dir.file_type != FileType::Directory {
                continue;
            }
            let user_records_path = scoped_from_virtual(&user_dir.path)?;
            for record_entry in self.list_dir_or_empty(&user_records_path).await? {
                if record_entry.file_type != FileType::File {
                    continue;
                }
                let Some(versioned) = self
                    .get_optional_entry(&scoped_from_virtual(&record_entry.path)?)
                    .await?
                else {
                    continue;
                };
                let _: StoredSecret = parse_entry(versioned)?;
            }
        }
        Ok(())
    }

    async fn list_dir_or_empty(
        &self,
        path: &ScopedPath,
    ) -> Result<Vec<ironclaw_filesystem::DirEntry>, SecretError> {
        match self.filesystem.list_dir(path).await {
            Ok(entries) => Ok(entries),
            Err(FilesystemError::NotFound { .. }) => Ok(Vec::new()),
            Err(error) => Err(secret_filesystem_error(error)),
        }
    }
}

#[async_trait]
impl<F> SecretsStore for FilesystemSecretsStore<F>
where
    F: RootFilesystem + 'static,
{
    async fn create(
        &self,
        user_id: &str,
        params: CreateSecretParams,
    ) -> Result<Secret, SecretError> {
        let name = normalize_secret_name(&params.name);
        let value = params.value.expose_secret().to_string();
        let provider = params.provider;
        let expires_at = params.expires_at;
        let path = record_path(user_id, &name)?;

        loop {
            let existing = self.get_optional_entry(&path).await?;
            let (record, cas) = match existing {
                Some(versioned) => {
                    let existing_record: StoredSecret = parse_entry(versioned.clone())?;
                    (
                        build_stored_secret(
                            user_id,
                            &name,
                            &value,
                            provider.clone(),
                            expires_at,
                            Some(&existing_record),
                        ),
                        CasExpectation::Version(versioned.version),
                    )
                }
                None => (
                    build_stored_secret(user_id, &name, &value, provider.clone(), expires_at, None),
                    CasExpectation::Absent,
                ),
            };
            let entry = secret_record_entry(&record)?;
            match self.filesystem.put(&path, entry, cas).await {
                Ok(_) => return Ok(record.into()),
                Err(FilesystemError::VersionMismatch { .. }) => continue,
                Err(error) => return Err(secret_filesystem_error(error)),
            }
        }
    }

    async fn get(&self, user_id: &str, name: &str) -> Result<Secret, SecretError> {
        self.get_secret_record(user_id, name).await
    }

    async fn get_decrypted(
        &self,
        user_id: &str,
        name: &str,
    ) -> Result<DecryptedSecret, SecretError> {
        let secret = self.get(user_id, name).await?;
        let record = self.get_stored_secret_record(user_id, &secret.name).await?;
        DecryptedSecret::from_bytes(record.value.into_bytes())
    }

    async fn consume_if_matches(
        &self,
        user_id: &str,
        name: &str,
        expected_value: &str,
    ) -> Result<SecretConsumeResult, SecretError> {
        let name = normalize_secret_name(name);
        let Some(entry) = self.get_secret_entry(user_id, &name).await? else {
            return Ok(SecretConsumeResult::NotFound);
        };
        let secret: StoredSecret = parse_entry(entry)?;
        ensure_stored_secret_not_expired(&secret)?;
        if secret.value != expected_value {
            return Ok(SecretConsumeResult::Mismatched);
        }
        self.filesystem
            .delete(&record_path(user_id, &name)?)
            .await
            .map_err(secret_filesystem_error)?;
        Ok(SecretConsumeResult::Matched)
    }

    async fn exists(&self, user_id: &str, name: &str) -> Result<bool, SecretError> {
        Ok(self
            .get_secret_entry(user_id, &normalize_secret_name(name))
            .await?
            .is_some())
    }

    async fn any_exist(&self) -> Result<bool, SecretError> {
        Ok(!self
            .list_dir_or_empty(&records_root_path()?)
            .await?
            .is_empty())
    }

    async fn list(&self, user_id: &str) -> Result<Vec<SecretRef>, SecretError> {
        let mut refs = Vec::new();
        for entry in self
            .list_dir_or_empty(&user_records_path(user_id)?)
            .await?
            .into_iter()
            .filter(|entry| entry.file_type == FileType::File)
        {
            let Some(versioned) = self
                .get_optional_entry(&scoped_from_virtual(&entry.path)?)
                .await?
            else {
                continue;
            };
            let secret: StoredSecret = parse_entry(versioned)?;
            ensure_stored_secret_not_expired(&secret)?;
            refs.push(SecretRef {
                name: secret.name,
                provider: secret.provider,
            });
        }
        refs.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(refs)
    }

    async fn delete(&self, user_id: &str, name: &str) -> Result<bool, SecretError> {
        match self.filesystem.delete(&record_path(user_id, name)?).await {
            Ok(()) => Ok(true),
            Err(FilesystemError::NotFound { .. }) => Ok(false),
            Err(error) => Err(secret_filesystem_error(error)),
        }
    }

    async fn record_usage(&self, secret_id: Uuid) -> Result<(), SecretError> {
        let mut matches = self
            .filesystem
            .query(
                &records_root_path()?,
                &Filter::Eq {
                    key: secret_id_index_key()?,
                    value: IndexValue::Text(secret_id.to_string()),
                },
                Page::first(2),
            )
            .await
            .map_err(secret_filesystem_error)?;
        let Some(mut versioned) = matches.pop() else {
            return Err(SecretError::NotFound(secret_id.to_string()));
        };

        loop {
            let mut secret: StoredSecret = parse_entry(versioned.clone())?;
            if secret.id != secret_id {
                return Err(SecretError::NotFound(secret_id.to_string()));
            }
            secret.last_used_at = Some(Utc::now());
            secret.usage_count += 1;
            secret.updated_at = Utc::now();
            let path = scoped_from_virtual(&versioned.path)?;
            let entry = secret_record_entry(&secret)?;
            match self
                .filesystem
                .put(&path, entry, CasExpectation::Version(versioned.version))
                .await
            {
                Ok(_) => return Ok(()),
                Err(FilesystemError::VersionMismatch { .. }) => {
                    let Some(fresh) = self.get_optional_entry(&path).await? else {
                        return Err(SecretError::NotFound(secret_id.to_string()));
                    };
                    versioned = fresh;
                }
                Err(error) => return Err(secret_filesystem_error(error)),
            }
        }
    }

    async fn is_accessible(
        &self,
        user_id: &str,
        secret_name: &str,
        allowed_secrets: &[String],
    ) -> Result<bool, SecretError> {
        let secret_name_lower = normalize_secret_name(secret_name);
        if !self.exists(user_id, &secret_name_lower).await? {
            return Ok(false);
        }
        for pattern in allowed_secrets {
            let pattern_lower = pattern.to_lowercase();
            if pattern_lower == secret_name_lower {
                return Ok(true);
            }
            if let Some(prefix) = pattern_lower.strip_suffix('*')
                && secret_name_lower.starts_with(prefix)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct StoredSecret {
    id: Uuid,
    user_id: String,
    name: String,
    value: String,
    provider: Option<String>,
    expires_at: Option<chrono::DateTime<Utc>>,
    last_used_at: Option<chrono::DateTime<Utc>>,
    usage_count: i64,
    created_at: chrono::DateTime<Utc>,
    updated_at: chrono::DateTime<Utc>,
}

impl From<StoredSecret> for Secret {
    fn from(record: StoredSecret) -> Self {
        Self {
            id: record.id,
            user_id: record.user_id,
            name: record.name,
            encrypted_value: Vec::new(),
            key_salt: Vec::new(),
            provider: record.provider,
            expires_at: record.expires_at,
            last_used_at: record.last_used_at,
            usage_count: record.usage_count,
            created_at: record.created_at,
            updated_at: record.updated_at,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct StoredKeyCheck {
    value: String,
}

fn build_stored_secret(
    user_id: &str,
    name: &str,
    value: &str,
    provider: Option<String>,
    expires_at: Option<chrono::DateTime<Utc>>,
    existing: Option<&StoredSecret>,
) -> StoredSecret {
    let now = Utc::now();
    StoredSecret {
        id: existing
            .map(|secret| secret.id)
            .unwrap_or_else(Uuid::new_v4),
        user_id: user_id.to_string(),
        name: name.to_string(),
        value: value.to_string(),
        provider,
        expires_at,
        last_used_at: existing.and_then(|secret| secret.last_used_at),
        usage_count: existing.map(|secret| secret.usage_count).unwrap_or(0),
        created_at: existing.map(|secret| secret.created_at).unwrap_or(now),
        updated_at: now,
    }
}

fn build_key_check_record() -> StoredKeyCheck {
    StoredKeyCheck {
        value: SECRET_STORE_KEY_CHECK_PLAINTEXT.to_string(),
    }
}

fn verify_secret_store_key_check(record: &StoredKeyCheck) -> Result<(), SecretError> {
    if record.value != SECRET_STORE_KEY_CHECK_PLAINTEXT {
        return Err(SecretError::DecryptionFailed(
            "secret store key check mismatch".to_string(),
        ));
    }
    Ok(())
}

fn parse_entry<T: serde::de::DeserializeOwned>(entry: VersionedEntry) -> Result<T, SecretError> {
    entry.entry.parse_json().map_err(|error| {
        SecretError::Database(format!("invalid filesystem secret record: {error}"))
    })
}

fn record_entry<T: Serialize>(kind: &str, value: &T) -> Result<Entry, SecretError> {
    let value = serde_json::to_value(value)
        .map_err(|error| SecretError::Database(format!("serialize secret record: {error}")))?;
    Entry::record(
        RecordKind::new(kind).map_err(secret_filesystem_error)?,
        &value,
    )
    .map_err(|error| SecretError::Database(format!("serialize secret record: {error}")))
}

fn secret_record_entry(secret: &StoredSecret) -> Result<Entry, SecretError> {
    Ok(record_entry(SECRET_RECORD_KIND, secret)?.with_indexed(
        secret_id_index_key()?,
        IndexValue::Text(secret.id.to_string()),
    ))
}

fn normalize_secret_name(name: &str) -> String {
    name.to_lowercase()
}

fn ensure_stored_secret_not_expired(secret: &StoredSecret) -> Result<(), SecretError> {
    if let Some(expires_at) = secret.expires_at
        && expires_at < Utc::now()
    {
        return Err(SecretError::Expired);
    }
    Ok(())
}

fn secret_id_index_key() -> Result<IndexKey, SecretError> {
    IndexKey::new(SECRET_ID_INDEX_KEY).map_err(secret_filesystem_error)
}

fn records_root_path() -> Result<ScopedPath, SecretError> {
    ScopedPath::new("/secrets/records").map_err(secret_filesystem_error)
}

fn user_records_path(user_id: &str) -> Result<ScopedPath, SecretError> {
    ScopedPath::new(format!("/secrets/records/{}", encode_path_segment(user_id)))
        .map_err(secret_filesystem_error)
}

fn record_path(user_id: &str, name: &str) -> Result<ScopedPath, SecretError> {
    ScopedPath::new(format!(
        "/secrets/records/{}/{}.json",
        encode_path_segment(user_id),
        encode_path_segment(&normalize_secret_name(name))
    ))
    .map_err(secret_filesystem_error)
}

fn key_check_path() -> Result<ScopedPath, SecretError> {
    ScopedPath::new(format!(
        "/secrets/key-check/{SECRET_STORE_KEY_CHECK_ID}.json"
    ))
    .map_err(secret_filesystem_error)
}

fn scoped_from_virtual(path: &VirtualPath) -> Result<ScopedPath, SecretError> {
    ScopedPath::new(path.as_str()).map_err(secret_filesystem_error)
}

fn encode_path_segment(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(value.len() * 2);
    for byte in value.as_bytes() {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn secret_filesystem_error(error: impl std::fmt::Display) -> SecretError {
    SecretError::Database(format!("filesystem secret store error: {error}"))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::Duration;
    use ironclaw_filesystem::{EncryptedBackend, InMemoryBackend, RootFilesystem};
    use ironclaw_host_api::VirtualPath;
    use secrecy::SecretString;

    use super::*;
    use crate::SecretsCrypto;

    #[tokio::test]
    async fn filesystem_secret_store_persists_records_through_root_filesystem() {
        let root = Arc::new(InMemoryBackend::new());
        let store = filesystem_store(Arc::clone(&root), "01234567890123456789012345678901");
        store.verify_can_decrypt_existing_secrets().await.unwrap();

        store
            .create(
                "tenant-user",
                CreateSecretParams::new("openai_key", "sk-test-filesystem"),
            )
            .await
            .unwrap();

        let reopened = filesystem_store(Arc::clone(&root), "01234567890123456789012345678901");
        reopened
            .verify_can_decrypt_existing_secrets()
            .await
            .unwrap();
        let decrypted = reopened
            .get_decrypted("tenant-user", "openai_key")
            .await
            .unwrap();
        assert_eq!(decrypted.expose(), "sk-test-filesystem");

        let refs = reopened.list("tenant-user").await.unwrap();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].name, "openai_key");
        assert!(reopened.any_exist().await.unwrap());

        let raw_path =
            VirtualPath::new(record_path("tenant-user", "openai_key").unwrap().as_str()).unwrap();
        let raw = root.get(&raw_path).await.unwrap().unwrap();
        assert!(
            !String::from_utf8_lossy(&raw.entry.body).contains("sk-test-filesystem"),
            "raw filesystem body must be encrypted by the backend decorator"
        );
    }

    #[tokio::test]
    async fn filesystem_secret_store_key_check_rejects_wrong_key() {
        let root = Arc::new(InMemoryBackend::new());
        let store = filesystem_store(Arc::clone(&root), "01234567890123456789012345678901");
        store.verify_can_decrypt_existing_secrets().await.unwrap();
        store
            .create(
                "tenant-user",
                CreateSecretParams::new("openai_key", "sk-test-filesystem"),
            )
            .await
            .unwrap();

        let wrong_key_store = filesystem_store(root, "abcdefghijklmnopqrstuvwxyzABCDEF");
        let error = wrong_key_store
            .verify_can_decrypt_existing_secrets()
            .await
            .expect_err("wrong key must fail filesystem secret readiness");
        assert!(!format!("{error:?}").contains("sk-test-filesystem"));
    }

    #[tokio::test]
    async fn filesystem_secret_store_key_check_bootstrap_scans_all_existing_records() {
        let root = Arc::new(InMemoryBackend::new());
        let store = filesystem_store(Arc::clone(&root), "01234567890123456789012345678901");
        store
            .create(
                "tenant-user",
                CreateSecretParams::new("openai_key", "sk-test-filesystem"),
            )
            .await
            .unwrap();
        assert!(
            root.get(&VirtualPath::new(key_check_path().unwrap().as_str()).unwrap())
                .await
                .unwrap()
                .is_none(),
            "test setup should exercise the pre-sentinel scan path"
        );

        let wrong_key_store = filesystem_store(root, "abcdefghijklmnopqrstuvwxyzABCDEF");
        let error = wrong_key_store
            .verify_can_decrypt_existing_secrets()
            .await
            .expect_err("wrong key must fail while scanning pre-sentinel records");
        assert!(!format!("{error:?}").contains("sk-test-filesystem"));
    }

    #[tokio::test]
    async fn filesystem_secret_store_preserves_metadata_on_overwrite() {
        let root = Arc::new(InMemoryBackend::new());
        let store = filesystem_store(root, "01234567890123456789012345678901");
        let first = store
            .create("tenant-user", CreateSecretParams::new("api_key", "first"))
            .await
            .unwrap();
        store.record_usage(first.id).await.unwrap();

        let second = store
            .create("tenant-user", CreateSecretParams::new("api_key", "second"))
            .await
            .unwrap();

        assert_eq!(second.id, first.id);
        assert_eq!(second.created_at, first.created_at);
        assert_eq!(second.usage_count, 1);
        assert!(second.last_used_at.is_some());
        assert_eq!(
            store
                .get_decrypted("tenant-user", "api_key")
                .await
                .unwrap()
                .expose(),
            "second"
        );
    }

    #[tokio::test]
    async fn filesystem_secret_store_consume_if_matches_is_one_shot_and_preserves_mismatch_and_expired()
     {
        let root = Arc::new(InMemoryBackend::new());
        let store = filesystem_store(root, "01234567890123456789012345678901");

        assert_eq!(
            store
                .consume_if_matches("tenant-user", "missing", "expected")
                .await
                .unwrap(),
            SecretConsumeResult::NotFound
        );
        store
            .create(
                "tenant-user",
                CreateSecretParams::new("api_key", "expected"),
            )
            .await
            .unwrap();
        assert_eq!(
            store
                .consume_if_matches("tenant-user", "api_key", "wrong")
                .await
                .unwrap(),
            SecretConsumeResult::Mismatched
        );
        assert!(store.exists("tenant-user", "api_key").await.unwrap());
        assert_eq!(
            store
                .consume_if_matches("tenant-user", "api_key", "expected")
                .await
                .unwrap(),
            SecretConsumeResult::Matched
        );
        assert!(!store.exists("tenant-user", "api_key").await.unwrap());

        let mut expired = CreateSecretParams::new("expired", "expected");
        expired.expires_at = Some(Utc::now() - Duration::seconds(1));
        store.create("tenant-user", expired).await.unwrap();
        assert!(matches!(
            store
                .consume_if_matches("tenant-user", "expired", "expected")
                .await,
            Err(SecretError::Expired)
        ));
    }

    #[tokio::test]
    async fn filesystem_secret_store_tracks_usage_delete_and_access_edges() {
        let root = Arc::new(InMemoryBackend::new());
        let store = filesystem_store(root, "01234567890123456789012345678901");
        let secret = store
            .create(
                "tenant-user",
                CreateSecretParams::new("OpenAI_Key", "secret"),
            )
            .await
            .unwrap();

        store.record_usage(secret.id).await.unwrap();
        let used = store.get("tenant-user", "openai_key").await.unwrap();
        assert_eq!(used.usage_count, 1);
        assert!(used.last_used_at.is_some());
        assert!(matches!(
            store.record_usage(Uuid::new_v4()).await,
            Err(SecretError::NotFound(_))
        ));

        assert!(
            store
                .is_accessible("tenant-user", "openai_key", &["openai_key".to_string()])
                .await
                .unwrap()
        );
        assert!(
            store
                .is_accessible("tenant-user", "openai_key", &["openai_*".to_string()])
                .await
                .unwrap()
        );
        assert!(
            !store
                .is_accessible("tenant-user", "openai_key", &["anthropic_*".to_string()])
                .await
                .unwrap()
        );
        assert!(
            !store
                .is_accessible("tenant-user", "missing", &["*".to_string()])
                .await
                .unwrap()
        );

        assert!(store.delete("tenant-user", "openai_key").await.unwrap());
        assert!(!store.exists("tenant-user", "openai_key").await.unwrap());
        assert!(!store.delete("tenant-user", "openai_key").await.unwrap());
    }

    fn filesystem_store(
        root: Arc<InMemoryBackend>,
        key: &str,
    ) -> FilesystemSecretsStore<EncryptedBackend<InMemoryBackend, SecretsCrypto>> {
        let crypto = Arc::new(SecretsCrypto::new(SecretString::from(key)).unwrap());
        let root = Arc::new(EncryptedBackend::new(root, crypto));
        FilesystemSecretsStore::over_root(root).unwrap()
    }
}
