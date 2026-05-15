//! Filesystem-backed implementation of [`IdentityStore`].
//!
//! Routes linked external identity persistence (OAuth, social login) through
//! the unified [`RootFilesystem`] surface. Mirrors the canonical migration
//! shape from `crates/ironclaw_secrets/src/filesystem_store.rs` and
//! `crates/ironclaw_authorization/src/lib.rs`.
//!
//! Path layout:
//!
//! - `/identities/<provider>/<provider_user_id>` — one [`Entry`] per identity.
//!
//! The indexed projection carries `provider`, `provider_user_id`, `user_id`,
//! and a normalized `email` so `query` can satisfy the lookup-by-email and
//! lookup-by-user paths without scanning every record. The `email_verified`
//! and `identity_kind` projections support the `find_identity_by_verified_email`
//! path.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_filesystem::{
    CasExpectation, ContentType, DirEntry, Entry, FilesystemError, IndexKey, IndexValue,
    RecordKind, RootFilesystem,
};
use ironclaw_host_api::VirtualPath;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db::{IdentityStore, UserIdentityRecord, UserRecord, UserStore};
use crate::error::DatabaseError;

use super::filesystem_users::FilesystemUserStore;

const RECORD_KIND: &str = "user_identity";

mod fs_keys {
    pub const PROVIDER: &str = "provider";
    pub const PROVIDER_USER_ID: &str = "provider_user_id";
    pub const USER_ID: &str = "user_id";
    pub const EMAIL: &str = "email";
    pub const EMAIL_VERIFIED: &str = "email_verified";
    pub const IDENTITY_KIND: &str = "identity_kind";
}

/// Filesystem-backed [`IdentityStore`].
pub struct FilesystemIdentityStore<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<F>,
    /// Reused for the atomic [`IdentityStore::create_user_with_identity`] path.
    /// Sharing the same root filesystem keeps the two records in the same
    /// namespace; multi-mount deployments must route both stores to the same
    /// composite filesystem.
    users: FilesystemUserStore<F>,
}

impl<F> FilesystemIdentityStore<F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: Arc<F>) -> Self {
        let users = FilesystemUserStore::new(Arc::clone(&filesystem));
        Self { filesystem, users }
    }

    fn identity_path(provider: &str, provider_user_id: &str) -> Result<VirtualPath, DatabaseError> {
        let provider_seg = encode_segment(provider);
        let pid_seg = encode_segment(provider_user_id);
        VirtualPath::new(format!("/identities/{provider_seg}/{pid_seg}"))
            .map_err(|e| DatabaseError::Query(format!("invalid identity path: {e}")))
    }

    fn identities_root() -> Result<VirtualPath, DatabaseError> {
        VirtualPath::new("/identities".to_string())
            .map_err(|e| DatabaseError::Query(format!("invalid identities root: {e}")))
    }

    fn build_entry(record: &UserIdentityRecord) -> Result<Entry, DatabaseError> {
        let body = serde_json::to_vec(&SerializableIdentity::from_record(record))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        let kind = RecordKind::new(RECORD_KIND).map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut entry = Entry::bytes(body).with_content_type(ContentType::json());
        entry.kind = Some(kind);

        let key_provider =
            IndexKey::new(fs_keys::PROVIDER).map_err(|e| DatabaseError::Query(e.to_string()))?;
        let key_pid = IndexKey::new(fs_keys::PROVIDER_USER_ID)
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let key_user =
            IndexKey::new(fs_keys::USER_ID).map_err(|e| DatabaseError::Query(e.to_string()))?;
        let key_verified = IndexKey::new(fs_keys::EMAIL_VERIFIED)
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let key_kind = IndexKey::new(fs_keys::IDENTITY_KIND)
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        entry = entry
            .with_indexed(key_provider, IndexValue::Text(record.provider.clone()))
            .with_indexed(key_pid, IndexValue::Text(record.provider_user_id.clone()))
            .with_indexed(key_user, IndexValue::Text(record.user_id.clone()))
            .with_indexed(key_verified, IndexValue::Bool(record.email_verified))
            .with_indexed(key_kind, IndexValue::Text("oauth".to_string()));
        if let Some(email) = &record.email {
            let key_email =
                IndexKey::new(fs_keys::EMAIL).map_err(|e| DatabaseError::Query(e.to_string()))?;
            entry = entry.with_indexed(key_email, IndexValue::Text(email.to_lowercase()));
        }
        Ok(entry)
    }

    async fn read_identity(
        &self,
        path: &VirtualPath,
    ) -> Result<Option<UserIdentityRecord>, DatabaseError> {
        let Some(versioned) = self.filesystem.get(path).await.map_err(fs_to_db_error)? else {
            return Ok(None);
        };
        let serializable: SerializableIdentity = serde_json::from_slice(&versioned.entry.body)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        Ok(Some(serializable.into_record()))
    }

    async fn list_provider_entries(
        &self,
        path: &VirtualPath,
    ) -> Result<Vec<DirEntry>, DatabaseError> {
        match self.filesystem.list_dir(path).await {
            Ok(entries) => Ok(entries),
            Err(FilesystemError::NotFound { .. }) => Ok(Vec::new()),
            Err(error) => Err(fs_to_db_error(error)),
        }
    }
}

#[async_trait]
impl<F> IdentityStore for FilesystemIdentityStore<F>
where
    F: RootFilesystem,
{
    async fn get_identity_by_provider(
        &self,
        provider: &str,
        provider_user_id: &str,
    ) -> Result<Option<UserIdentityRecord>, DatabaseError> {
        let path = Self::identity_path(provider, provider_user_id)?;
        self.read_identity(&path).await
    }

    async fn list_identities_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<UserIdentityRecord>, DatabaseError> {
        // The trait surface doesn't expose paths inside a `query` result, so
        // we scan provider roots. This matches the legacy SQL semantics and
        // is bounded by the count of providers a user has registered.
        let root = Self::identities_root()?;
        let providers = match self.filesystem.list_dir(&root).await {
            Ok(entries) => entries,
            Err(FilesystemError::NotFound { .. }) => return Ok(Vec::new()),
            Err(error) => return Err(fs_to_db_error(error)),
        };
        let mut out = Vec::new();
        for provider_dir in providers {
            let entries = self.list_provider_entries(&provider_dir.path).await?;
            for entry in entries {
                if let Some(record) = self.read_identity(&entry.path).await?
                    && record.user_id == user_id
                {
                    out.push(record);
                }
            }
        }
        out.sort_by_key(|r| r.created_at);
        Ok(out)
    }

    async fn create_identity(&self, identity: &UserIdentityRecord) -> Result<(), DatabaseError> {
        let path = Self::identity_path(&identity.provider, &identity.provider_user_id)?;
        let entry = Self::build_entry(identity)?;
        self.filesystem
            .put(&path, entry, CasExpectation::Absent)
            .await
            .map(|_| ())
            .map_err(fs_to_db_error)
    }

    async fn update_identity_profile(
        &self,
        provider: &str,
        provider_user_id: &str,
        display_name: Option<&str>,
        avatar_url: Option<&str>,
    ) -> Result<(), DatabaseError> {
        let path = Self::identity_path(provider, provider_user_id)?;
        let Some(versioned) = self.filesystem.get(&path).await.map_err(fs_to_db_error)? else {
            return Ok(());
        };
        let serializable: SerializableIdentity = serde_json::from_slice(&versioned.entry.body)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        let mut record = serializable.into_record();
        if let Some(name) = display_name {
            record.display_name = Some(name.to_string());
        }
        if let Some(url) = avatar_url {
            record.avatar_url = Some(url.to_string());
        }
        record.updated_at = Utc::now();
        let entry = Self::build_entry(&record)?;
        // CAS on the version we just read keeps concurrent updates from
        // overwriting each other. On a conflict the filesystem returns
        // VersionMismatch, surfaced as a DatabaseError::Query to the caller.
        self.filesystem
            .put(&path, entry, CasExpectation::Version(versioned.version))
            .await
            .map(|_| ())
            .map_err(fs_to_db_error)
    }

    async fn find_identity_by_verified_email(
        &self,
        email: &str,
    ) -> Result<Option<UserIdentityRecord>, DatabaseError> {
        // Trait surface doesn't return paths from `query` yet, so we scan
        // every provider tree. Matches the legacy SQL semantics where this
        // lookup runs against a single (potentially indexed) column scan.
        let root = Self::identities_root()?;
        let providers = match self.filesystem.list_dir(&root).await {
            Ok(entries) => entries,
            Err(FilesystemError::NotFound { .. }) => return Ok(None),
            Err(error) => return Err(fs_to_db_error(error)),
        };
        let needle = email.to_lowercase();
        for provider_dir in providers {
            let entries = self.list_provider_entries(&provider_dir.path).await?;
            for entry in entries {
                if let Some(record) = self.read_identity(&entry.path).await?
                    && record.email_verified
                    && record
                        .email
                        .as_deref()
                        .is_some_and(|e| e.eq_ignore_ascii_case(&needle))
                {
                    return Ok(Some(record));
                }
            }
        }
        Ok(None)
    }

    async fn create_user_with_identity(
        &self,
        user: &UserRecord,
        identity: &UserIdentityRecord,
    ) -> Result<(), DatabaseError> {
        // Two records on the unified surface lack a single-mount CAS that
        // covers both keys. We write the user first; if the identity insert
        // fails, the user write is rolled back via a best-effort delete to
        // approximate the legacy BEGIN/ROLLBACK semantics from
        // `libsql/identities.rs::create_user_with_identity`.
        self.users.create_user(user).await?;
        if let Err(error) = self.create_identity(identity).await {
            let _ = self.users.delete_user(&user.id).await;
            return Err(error);
        }

        // Match the libSQL semantics: if this is the only user in the
        // system, promote them to admin. The promotion runs against the
        // already-written user record and reuses the users facade.
        let user_count = self.users.list_users(None).await?.len();
        if user_count == 1 {
            self.users.update_user_role(&user.id, "admin").await?;
        }

        Ok(())
    }
}

fn encode_segment(value: &str) -> String {
    value
        .chars()
        .map(|c| match c {
            '/' | '\\' | '\0' | '\n' | '\r' | '\t' | ' ' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect()
}

fn fs_to_db_error(error: FilesystemError) -> DatabaseError {
    DatabaseError::Query(format!("filesystem error: {error}"))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SerializableIdentity {
    id: Uuid,
    user_id: String,
    provider: String,
    provider_user_id: String,
    email: Option<String>,
    email_verified: bool,
    display_name: Option<String>,
    avatar_url: Option<String>,
    raw_profile: serde_json::Value,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl SerializableIdentity {
    fn from_record(record: &UserIdentityRecord) -> Self {
        Self {
            id: record.id,
            user_id: record.user_id.clone(),
            provider: record.provider.clone(),
            provider_user_id: record.provider_user_id.clone(),
            email: record.email.clone(),
            email_verified: record.email_verified,
            display_name: record.display_name.clone(),
            avatar_url: record.avatar_url.clone(),
            raw_profile: record.raw_profile.clone(),
            created_at: record.created_at,
            updated_at: record.updated_at,
        }
    }

    fn into_record(self) -> UserIdentityRecord {
        UserIdentityRecord {
            id: self.id,
            user_id: self.user_id,
            provider: self.provider,
            provider_user_id: self.provider_user_id,
            email: self.email,
            email_verified: self.email_verified,
            display_name: self.display_name,
            avatar_url: self.avatar_url,
            raw_profile: self.raw_profile,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_filesystem::InMemoryBackend;
    use uuid::Uuid;

    fn make_user(id: &str) -> UserRecord {
        let now = Utc::now();
        UserRecord {
            id: id.to_string(),
            email: Some(format!("{id}@example.com")),
            display_name: id.to_string(),
            status: "active".to_string(),
            role: "member".to_string(),
            created_at: now,
            updated_at: now,
            last_login_at: None,
            created_by: None,
            metadata: serde_json::json!({}),
        }
    }

    fn make_identity(
        user_id: &str,
        provider: &str,
        pid: &str,
        verified: bool,
    ) -> UserIdentityRecord {
        let now = Utc::now();
        UserIdentityRecord {
            id: Uuid::new_v4(),
            user_id: user_id.to_string(),
            provider: provider.to_string(),
            provider_user_id: pid.to_string(),
            email: Some(format!("{user_id}@example.com")),
            email_verified: verified,
            display_name: Some(user_id.to_string()),
            avatar_url: None,
            raw_profile: serde_json::json!({}),
            created_at: now,
            updated_at: now,
        }
    }

    fn store() -> FilesystemIdentityStore<InMemoryBackend> {
        FilesystemIdentityStore::new(Arc::new(InMemoryBackend::new()))
    }

    #[tokio::test]
    async fn create_then_get_identity_round_trips() {
        let s = store();
        let identity = make_identity("alice", "google", "sub-1", true);
        s.create_identity(&identity).await.unwrap();
        let fetched = s.get_identity_by_provider("google", "sub-1").await.unwrap();
        assert!(fetched.is_some());
        assert_eq!(fetched.unwrap().user_id, "alice");
    }

    #[tokio::test]
    async fn get_identity_returns_none_for_wrong_provider() {
        let s = store();
        let identity = make_identity("alice", "google", "sub-1", true);
        s.create_identity(&identity).await.unwrap();
        assert!(
            s.get_identity_by_provider("github", "sub-1")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn list_identities_returns_only_caller_user() {
        let s = store();
        s.create_identity(&make_identity("alice", "google", "g1", true))
            .await
            .unwrap();
        s.create_identity(&make_identity("alice", "github", "h1", true))
            .await
            .unwrap();
        s.create_identity(&make_identity("bob", "google", "g2", true))
            .await
            .unwrap();
        let list = s.list_identities_for_user("alice").await.unwrap();
        assert_eq!(list.len(), 2);
        assert!(list.iter().all(|r| r.user_id == "alice"));
    }

    #[tokio::test]
    async fn update_identity_profile_overwrites_display_name() {
        let s = store();
        let identity = make_identity("alice", "google", "sub-1", true);
        s.create_identity(&identity).await.unwrap();
        s.update_identity_profile("google", "sub-1", Some("Alice G"), None)
            .await
            .unwrap();
        let fetched = s
            .get_identity_by_provider("google", "sub-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(fetched.display_name.as_deref(), Some("Alice G"));
    }

    #[tokio::test]
    async fn find_by_verified_email_ignores_unverified_records() {
        let s = store();
        s.create_identity(&make_identity("alice", "google", "g1", false))
            .await
            .unwrap();
        s.create_identity(&make_identity("bob", "github", "h1", true))
            .await
            .unwrap();
        let alice = s
            .find_identity_by_verified_email("alice@example.com")
            .await
            .unwrap();
        assert!(alice.is_none());
        let bob = s
            .find_identity_by_verified_email("bob@example.com")
            .await
            .unwrap();
        assert!(bob.is_some());
    }

    #[tokio::test]
    async fn create_user_with_identity_writes_both_records() {
        let s = store();
        let user = make_user("alice");
        let identity = make_identity("alice", "google", "sub-1", true);
        s.create_user_with_identity(&user, &identity).await.unwrap();
        // identity must be queryable
        let fetched_identity = s.get_identity_by_provider("google", "sub-1").await.unwrap();
        assert!(fetched_identity.is_some());
        // user must be queryable
        let fetched_user = s.users.get_user("alice").await.unwrap();
        assert!(fetched_user.is_some());
        // first user gets promoted to admin
        assert!(fetched_user.unwrap().is_admin());
    }
}
