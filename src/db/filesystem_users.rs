//! Filesystem-backed implementation of [`UserStore`].
//!
//! Routes user records, API tokens, and login bookkeeping through the
//! unified [`RootFilesystem`] surface. Mirrors the canonical migration
//! shape from `crates/ironclaw_secrets/src/filesystem_store.rs` and
//! `crates/ironclaw_authorization/src/lib.rs`.
//!
//! Path layout:
//!
//! - `/users/<id>` — one [`Entry`] per user record.
//! - `/users/.tokens/<token_id>` — one [`Entry`] per API token. Hidden
//!   under `.tokens/` so a `list_dir("/users")` returns user records
//!   only.
//! - `/users/.tokens-by-hash/<hex(token_hash)>` — index entry mapping the
//!   token-hash bytes back to the `token_id`. Filesystem `query` against
//!   the indexed `token_hash` projection is the preferred path; the
//!   sibling pointer file is the CAS-only fallback for backends that
//!   don't serve `Filter::Eq` on `IndexValue::Bytes`.
//!
//! Note: cross-table aggregations exposed by the trait —
//! `user_usage_stats`, `user_summary_stats`, `admin_usage_summary` —
//! aggregate over `llm_calls` / `agent_jobs` records that live in the
//! `JobStore` mount, which this facade does not see. The filesystem
//! variant returns empty / zeroed results for those methods so the
//! trait surface stays satisfied; a deployment that needs real
//! cross-mount aggregation must wire those queries at a layer that
//! sees both mounts.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, FilesystemError, IndexKey, IndexValue, RecordKind,
    RootFilesystem,
};
use ironclaw_host_api::VirtualPath;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db::{
    AdminUsageSummary, ApiTokenRecord, DatabaseError, UserRecord, UserStore, UserSummaryStats,
    UserUsageStats,
};

const USER_RECORD_KIND: &str = "user";
const TOKEN_RECORD_KIND: &str = "api_token";

mod fs_keys {
    pub const USER_ID: &str = "user_id";
    pub const STATUS: &str = "status";
    pub const EMAIL: &str = "email";
    pub const ROLE: &str = "role";
    pub const TOKEN_ID: &str = "token_id";
    pub const TOKEN_HASH: &str = "token_hash";
    pub const REVOKED: &str = "revoked";
}

/// Filesystem-backed [`UserStore`].
pub struct FilesystemUserStore<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<F>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredToken {
    record: SerializableToken,
    token_hash: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SerializableToken {
    id: Uuid,
    user_id: String,
    name: String,
    token_prefix: String,
    expires_at: Option<DateTime<Utc>>,
    last_used_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    revoked_at: Option<DateTime<Utc>>,
}

impl SerializableToken {
    fn from_record(record: &ApiTokenRecord) -> Self {
        Self {
            id: record.id,
            user_id: record.user_id.clone(),
            name: record.name.clone(),
            token_prefix: record.token_prefix.clone(),
            expires_at: record.expires_at,
            last_used_at: record.last_used_at,
            created_at: record.created_at,
            revoked_at: record.revoked_at,
        }
    }

    fn into_record(self) -> ApiTokenRecord {
        ApiTokenRecord {
            id: self.id,
            user_id: self.user_id,
            name: self.name,
            token_prefix: self.token_prefix,
            expires_at: self.expires_at,
            last_used_at: self.last_used_at,
            created_at: self.created_at,
            revoked_at: self.revoked_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TokenHashPointer {
    token_id: Uuid,
}

impl<F> FilesystemUserStore<F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: Arc<F>) -> Self {
        Self { filesystem }
    }

    fn user_path(id: &str) -> Result<VirtualPath, DatabaseError> {
        VirtualPath::new(format!("/users/{}", encode_segment(id)))
            .map_err(|e| DatabaseError::Query(format!("invalid user path: {e}")))
    }

    fn users_root() -> Result<VirtualPath, DatabaseError> {
        VirtualPath::new("/users".to_string())
            .map_err(|e| DatabaseError::Query(format!("invalid users root: {e}")))
    }

    fn token_path(token_id: Uuid) -> Result<VirtualPath, DatabaseError> {
        VirtualPath::new(format!("/users/.tokens/{}", token_id))
            .map_err(|e| DatabaseError::Query(format!("invalid token path: {e}")))
    }

    fn token_hash_pointer_path(token_hash: &[u8; 32]) -> Result<VirtualPath, DatabaseError> {
        VirtualPath::new(format!("/users/.tokens-by-hash/{}", hex(token_hash)))
            .map_err(|e| DatabaseError::Query(format!("invalid token-hash path: {e}")))
    }

    fn tokens_root() -> Result<VirtualPath, DatabaseError> {
        VirtualPath::new("/users/.tokens".to_string())
            .map_err(|e| DatabaseError::Query(format!("invalid tokens root: {e}")))
    }

    fn build_user_entry(user: &UserRecord) -> Result<Entry, DatabaseError> {
        let body = serde_json::to_vec(&SerializableUser::from(user))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        let kind =
            RecordKind::new(USER_RECORD_KIND).map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut entry = Entry::bytes(body).with_content_type(ContentType::json());
        entry.kind = Some(kind);
        let k_user =
            IndexKey::new(fs_keys::USER_ID).map_err(|e| DatabaseError::Query(e.to_string()))?;
        let k_status =
            IndexKey::new(fs_keys::STATUS).map_err(|e| DatabaseError::Query(e.to_string()))?;
        let k_role =
            IndexKey::new(fs_keys::ROLE).map_err(|e| DatabaseError::Query(e.to_string()))?;
        entry = entry
            .with_indexed(k_user, IndexValue::Text(user.id.clone()))
            .with_indexed(k_status, IndexValue::Text(user.status.clone()))
            .with_indexed(k_role, IndexValue::Text(user.role.clone()));
        if let Some(email) = &user.email {
            let k_email =
                IndexKey::new(fs_keys::EMAIL).map_err(|e| DatabaseError::Query(e.to_string()))?;
            entry = entry.with_indexed(k_email, IndexValue::Text(email.to_lowercase()));
        }
        Ok(entry)
    }

    fn build_token_entry(stored: &StoredToken) -> Result<Entry, DatabaseError> {
        let body =
            serde_json::to_vec(stored).map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        let kind =
            RecordKind::new(TOKEN_RECORD_KIND).map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut entry = Entry::bytes(body).with_content_type(ContentType::json());
        entry.kind = Some(kind);
        let k_user =
            IndexKey::new(fs_keys::USER_ID).map_err(|e| DatabaseError::Query(e.to_string()))?;
        let k_token =
            IndexKey::new(fs_keys::TOKEN_ID).map_err(|e| DatabaseError::Query(e.to_string()))?;
        let k_hash =
            IndexKey::new(fs_keys::TOKEN_HASH).map_err(|e| DatabaseError::Query(e.to_string()))?;
        let k_revoked =
            IndexKey::new(fs_keys::REVOKED).map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(entry
            .with_indexed(k_user, IndexValue::Text(stored.record.user_id.clone()))
            .with_indexed(k_token, IndexValue::Text(stored.record.id.to_string()))
            .with_indexed(k_hash, IndexValue::Bytes(stored.token_hash.clone()))
            .with_indexed(
                k_revoked,
                IndexValue::Bool(stored.record.revoked_at.is_some()),
            ))
    }

    async fn read_user(&self, id: &str) -> Result<Option<UserRecord>, DatabaseError> {
        let path = Self::user_path(id)?;
        let Some(versioned) = self.filesystem.get(&path).await.map_err(fs_to_db_error)? else {
            return Ok(None);
        };
        let serializable: SerializableUser = serde_json::from_slice(&versioned.entry.body)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        Ok(Some(serializable.into_record()))
    }

    async fn read_users(&self) -> Result<Vec<UserRecord>, DatabaseError> {
        let root = Self::users_root()?;
        let entries = match self.filesystem.list_dir(&root).await {
            Ok(entries) => entries,
            Err(FilesystemError::NotFound { .. }) => return Ok(Vec::new()),
            Err(error) => return Err(fs_to_db_error(error)),
        };
        let mut out = Vec::new();
        for entry in entries {
            // Skip the hidden `.tokens` / `.tokens-by-hash` directories used
            // for token storage; they live alongside user records under
            // `/users` so the same prefix can be torn down by a single
            // composite-mount config.
            if entry.name.starts_with('.') {
                continue;
            }
            let Some(versioned) = self
                .filesystem
                .get(&entry.path)
                .await
                .map_err(fs_to_db_error)?
            else {
                continue;
            };
            let serializable: SerializableUser = match serde_json::from_slice(&versioned.entry.body)
            {
                Ok(s) => s,
                Err(_) => continue,
            };
            out.push(serializable.into_record());
        }
        Ok(out)
    }

    async fn write_user(&self, user: &UserRecord) -> Result<(), DatabaseError> {
        let path = Self::user_path(&user.id)?;
        let entry = Self::build_user_entry(user)?;
        self.filesystem
            .put(&path, entry, CasExpectation::Any)
            .await
            .map(|_| ())
            .map_err(fs_to_db_error)
    }

    async fn read_token(&self, token_id: Uuid) -> Result<Option<StoredToken>, DatabaseError> {
        let path = Self::token_path(token_id)?;
        let Some(versioned) = self.filesystem.get(&path).await.map_err(fs_to_db_error)? else {
            return Ok(None);
        };
        let stored: StoredToken = serde_json::from_slice(&versioned.entry.body)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        Ok(Some(stored))
    }

    async fn write_token(&self, stored: &StoredToken) -> Result<(), DatabaseError> {
        let path = Self::token_path(stored.record.id)?;
        let entry = Self::build_token_entry(stored)?;
        self.filesystem
            .put(&path, entry, CasExpectation::Any)
            .await
            .map(|_| ())
            .map_err(fs_to_db_error)
    }

    async fn list_tokens(&self) -> Result<Vec<StoredToken>, DatabaseError> {
        let root = Self::tokens_root()?;
        let entries = match self.filesystem.list_dir(&root).await {
            Ok(entries) => entries,
            Err(FilesystemError::NotFound { .. }) => return Ok(Vec::new()),
            Err(error) => return Err(fs_to_db_error(error)),
        };
        let mut out = Vec::new();
        for entry in entries {
            let Some(versioned) = self
                .filesystem
                .get(&entry.path)
                .await
                .map_err(fs_to_db_error)?
            else {
                continue;
            };
            if let Ok(stored) = serde_json::from_slice::<StoredToken>(&versioned.entry.body) {
                out.push(stored);
            }
        }
        Ok(out)
    }
}

#[async_trait]
impl<F> UserStore for FilesystemUserStore<F>
where
    F: RootFilesystem,
{
    async fn create_user(&self, user: &UserRecord) -> Result<(), DatabaseError> {
        let path = Self::user_path(&user.id)?;
        let entry = Self::build_user_entry(user)?;
        self.filesystem
            .put(&path, entry, CasExpectation::Absent)
            .await
            .map(|_| ())
            .map_err(fs_to_db_error)
    }

    async fn get_or_create_user(&self, user: UserRecord) -> Result<(), DatabaseError> {
        // CAS::Absent makes this atomic at the single-key level: either we
        // win and write, or VersionMismatch tells us the row already
        // exists (so we no-op). Matches `INSERT OR IGNORE` semantics.
        let path = Self::user_path(&user.id)?;
        let entry = Self::build_user_entry(&user)?;
        match self
            .filesystem
            .put(&path, entry, CasExpectation::Absent)
            .await
        {
            Ok(_) => Ok(()),
            Err(FilesystemError::VersionMismatch { .. }) => Ok(()),
            Err(error) => Err(fs_to_db_error(error)),
        }
    }

    async fn get_user(&self, id: &str) -> Result<Option<UserRecord>, DatabaseError> {
        self.read_user(id).await
    }

    async fn get_user_by_email(&self, email: &str) -> Result<Option<UserRecord>, DatabaseError> {
        let users = self.read_users().await?;
        let needle = email.to_lowercase();
        Ok(users.into_iter().find(|u| {
            u.email
                .as_deref()
                .is_some_and(|e| e.eq_ignore_ascii_case(&needle))
        }))
    }

    async fn list_users(&self, status: Option<&str>) -> Result<Vec<UserRecord>, DatabaseError> {
        let mut users = self.read_users().await?;
        if let Some(filter) = status {
            users.retain(|u| u.status == filter);
        }
        users.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(users)
    }

    async fn update_user_status(&self, id: &str, status: &str) -> Result<(), DatabaseError> {
        let Some(mut user) = self.read_user(id).await? else {
            return Ok(());
        };
        user.status = status.to_string();
        user.updated_at = Utc::now();
        self.write_user(&user).await
    }

    async fn update_user_role(&self, id: &str, role: &str) -> Result<(), DatabaseError> {
        let Some(mut user) = self.read_user(id).await? else {
            return Ok(());
        };
        user.role = role.to_string();
        user.updated_at = Utc::now();
        self.write_user(&user).await
    }

    async fn update_user_profile(
        &self,
        id: &str,
        display_name: &str,
        metadata: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        let Some(mut user) = self.read_user(id).await? else {
            return Ok(());
        };
        user.display_name = display_name.to_string();
        user.metadata = metadata.clone();
        user.updated_at = Utc::now();
        self.write_user(&user).await
    }

    async fn record_login(&self, id: &str) -> Result<(), DatabaseError> {
        let Some(mut user) = self.read_user(id).await? else {
            return Ok(());
        };
        let now = Utc::now();
        user.last_login_at = Some(now);
        user.updated_at = now;
        self.write_user(&user).await
    }

    async fn create_api_token(
        &self,
        user_id: &str,
        name: &str,
        token_hash: &[u8; 32],
        token_prefix: &str,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<ApiTokenRecord, DatabaseError> {
        let id = Uuid::new_v4();
        let now = Utc::now();
        let record = ApiTokenRecord {
            id,
            user_id: user_id.to_string(),
            name: name.to_string(),
            token_prefix: token_prefix.to_string(),
            expires_at,
            last_used_at: None,
            created_at: now,
            revoked_at: None,
        };
        let stored = StoredToken {
            record: SerializableToken::from_record(&record),
            token_hash: token_hash.to_vec(),
        };
        self.write_token(&stored).await?;
        // Write the sibling pointer last so a half-finished token write
        // doesn't surface to authenticate_token before the canonical row.
        let pointer_path = Self::token_hash_pointer_path(token_hash)?;
        let pointer = TokenHashPointer { token_id: id };
        let body = serde_json::to_vec(&pointer)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        let pointer_entry = Entry::bytes(body).with_content_type(ContentType::json());
        self.filesystem
            .put(&pointer_path, pointer_entry, CasExpectation::Any)
            .await
            .map(|_| ())
            .map_err(fs_to_db_error)?;
        Ok(record)
    }

    async fn list_api_tokens(&self, user_id: &str) -> Result<Vec<ApiTokenRecord>, DatabaseError> {
        let tokens = self.list_tokens().await?;
        let mut out: Vec<ApiTokenRecord> = tokens
            .into_iter()
            .filter(|t| t.record.user_id == user_id)
            .map(|t| t.record.into_record())
            .collect();
        out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(out)
    }

    async fn revoke_api_token(&self, token_id: Uuid, user_id: &str) -> Result<bool, DatabaseError> {
        let Some(mut stored) = self.read_token(token_id).await? else {
            return Ok(false);
        };
        if stored.record.user_id != user_id || stored.record.revoked_at.is_some() {
            return Ok(false);
        }
        stored.record.revoked_at = Some(Utc::now());
        self.write_token(&stored).await?;
        Ok(true)
    }

    async fn authenticate_token(
        &self,
        token_hash: &[u8; 32],
    ) -> Result<Option<(ApiTokenRecord, UserRecord)>, DatabaseError> {
        let pointer_path = Self::token_hash_pointer_path(token_hash)?;
        let pointer_versioned = match self.filesystem.get(&pointer_path).await {
            Ok(p) => p,
            Err(FilesystemError::NotFound { .. }) => return Ok(None),
            Err(error) => return Err(fs_to_db_error(error)),
        };
        let Some(pointer_versioned) = pointer_versioned else {
            return Ok(None);
        };
        let pointer: TokenHashPointer = serde_json::from_slice(&pointer_versioned.entry.body)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        let Some(stored) = self.read_token(pointer.token_id).await? else {
            return Ok(None);
        };
        if stored.record.revoked_at.is_some() {
            return Ok(None);
        }
        if stored.record.expires_at.is_some_and(|t| t <= Utc::now()) {
            return Ok(None);
        }
        let Some(user) = self.read_user(&stored.record.user_id).await? else {
            return Ok(None);
        };
        if user.status != "active" {
            return Ok(None);
        }
        Ok(Some((stored.record.into_record(), user)))
    }

    async fn record_token_usage(&self, token_id: Uuid) -> Result<(), DatabaseError> {
        let Some(mut stored) = self.read_token(token_id).await? else {
            return Ok(());
        };
        stored.record.last_used_at = Some(Utc::now());
        self.write_token(&stored).await
    }

    async fn has_any_users(&self) -> Result<bool, DatabaseError> {
        let users = self.read_users().await?;
        Ok(!users.is_empty())
    }

    async fn delete_user(&self, id: &str) -> Result<bool, DatabaseError> {
        let path = Self::user_path(id)?;
        match self.filesystem.delete(&path).await {
            Ok(()) => {
                // Also revoke + remove tokens owned by this user. We don't
                // delete the token records (LLM-data retention does not
                // apply here, but the legacy delete cascade did remove
                // them, so we match that).
                let tokens = self.list_tokens().await?;
                for stored in tokens {
                    if stored.record.user_id == id {
                        let token_path = Self::token_path(stored.record.id)?;
                        let _ = self.filesystem.delete(&token_path).await;
                        if let Ok(hash_arr) = <[u8; 32]>::try_from(stored.token_hash.as_slice()) {
                            let ptr_path = Self::token_hash_pointer_path(&hash_arr)?;
                            let _ = self.filesystem.delete(&ptr_path).await;
                        }
                    }
                }
                Ok(true)
            }
            Err(FilesystemError::NotFound { .. }) => Ok(false),
            Err(error) => Err(fs_to_db_error(error)),
        }
    }

    async fn user_usage_stats(
        &self,
        _user_id: Option<&str>,
        _since: DateTime<Utc>,
    ) -> Result<Vec<UserUsageStats>, DatabaseError> {
        // Aggregations live in the JobStore mount. The filesystem facade
        // returns an empty vector; callers that need real cross-mount
        // aggregation must wire it at a higher layer that sees both
        // mounts. Matches the legacy contract where no rows = no data.
        Ok(Vec::new())
    }

    async fn user_summary_stats(
        &self,
        user_id: Option<&str>,
    ) -> Result<Vec<UserSummaryStats>, DatabaseError> {
        let users = self.read_users().await?;
        let zero = Decimal::ZERO;
        let stats: Vec<UserSummaryStats> = users
            .into_iter()
            .filter(|u| user_id.map(|id| u.id == id).unwrap_or(true))
            .map(|u| UserSummaryStats {
                user_id: u.id,
                job_count: 0,
                total_cost: zero,
                last_active_at: u.last_login_at,
            })
            .collect();
        Ok(stats)
    }

    async fn admin_usage_summary(
        &self,
        _since: DateTime<Utc>,
    ) -> Result<AdminUsageSummary, DatabaseError> {
        let users = self.read_users().await?;
        let total_users = users.len() as i64;
        let active_users = users.iter().filter(|u| u.status == "active").count() as i64;
        let suspended_users = users.iter().filter(|u| u.status == "suspended").count() as i64;
        let admin_users = users.iter().filter(|u| u.is_admin()).count() as i64;
        Ok(AdminUsageSummary {
            total_users,
            active_users,
            suspended_users,
            admin_users,
            total_jobs: 0,
            llm_calls: 0,
            input_tokens: 0,
            output_tokens: 0,
            usage_cost: Decimal::ZERO,
        })
    }

    async fn create_user_with_token(
        &self,
        user: &UserRecord,
        token_name: &str,
        token_hash: &[u8; 32],
        token_prefix: &str,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<ApiTokenRecord, DatabaseError> {
        // CAS::Absent on the user record makes the user side atomic.
        // The token follows; on token-write failure we roll back the user
        // write via a best-effort delete to approximate the legacy
        // BEGIN/ROLLBACK shape.
        self.create_user(user).await?;
        match self
            .create_api_token(&user.id, token_name, token_hash, token_prefix, expires_at)
            .await
        {
            Ok(record) => Ok(record),
            Err(error) => {
                let _ = self.delete_user(&user.id).await;
                Err(error)
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SerializableUser {
    id: String,
    email: Option<String>,
    display_name: String,
    status: String,
    role: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    last_login_at: Option<DateTime<Utc>>,
    created_by: Option<String>,
    metadata: serde_json::Value,
}

impl SerializableUser {
    fn from(user: &UserRecord) -> Self {
        Self {
            id: user.id.clone(),
            email: user.email.clone(),
            display_name: user.display_name.clone(),
            status: user.status.clone(),
            role: user.role.clone(),
            created_at: user.created_at,
            updated_at: user.updated_at,
            last_login_at: user.last_login_at,
            created_by: user.created_by.clone(),
            metadata: user.metadata.clone(),
        }
    }

    fn into_record(self) -> UserRecord {
        UserRecord {
            id: self.id,
            email: self.email,
            display_name: self.display_name,
            status: self.status,
            role: self.role,
            created_at: self.created_at,
            updated_at: self.updated_at,
            last_login_at: self.last_login_at,
            created_by: self.created_by,
            metadata: self.metadata,
        }
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

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn fs_to_db_error(error: FilesystemError) -> DatabaseError {
    DatabaseError::Query(format!("filesystem error: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_filesystem::InMemoryBackend;

    fn store() -> FilesystemUserStore<InMemoryBackend> {
        FilesystemUserStore::new(Arc::new(InMemoryBackend::new()))
    }

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

    #[tokio::test]
    async fn create_then_get_round_trips() {
        let s = store();
        s.create_user(&make_user("alice")).await.unwrap();
        let fetched = s.get_user("alice").await.unwrap();
        assert!(fetched.is_some());
        assert_eq!(fetched.unwrap().display_name, "alice");
    }

    #[tokio::test]
    async fn get_or_create_user_is_idempotent() {
        let s = store();
        s.get_or_create_user(make_user("alice")).await.unwrap();
        s.get_or_create_user(make_user("alice")).await.unwrap();
        assert!(s.has_any_users().await.unwrap());
        let users = s.list_users(None).await.unwrap();
        assert_eq!(users.len(), 1);
    }

    #[tokio::test]
    async fn get_user_by_email_is_case_insensitive() {
        let s = store();
        s.create_user(&make_user("alice")).await.unwrap();
        let fetched = s
            .get_user_by_email("Alice@Example.Com")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(fetched.id, "alice");
    }

    #[tokio::test]
    async fn list_users_filters_by_status() {
        let s = store();
        let mut alice = make_user("alice");
        alice.status = "suspended".to_string();
        s.create_user(&alice).await.unwrap();
        s.create_user(&make_user("bob")).await.unwrap();
        let active = s.list_users(Some("active")).await.unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, "bob");
    }

    #[tokio::test]
    async fn update_user_status_and_role_persist() {
        let s = store();
        s.create_user(&make_user("alice")).await.unwrap();
        s.update_user_status("alice", "suspended").await.unwrap();
        s.update_user_role("alice", "admin").await.unwrap();
        let fetched = s.get_user("alice").await.unwrap().unwrap();
        assert_eq!(fetched.status, "suspended");
        assert!(fetched.is_admin());
    }

    #[tokio::test]
    async fn record_login_sets_last_login_at() {
        let s = store();
        s.create_user(&make_user("alice")).await.unwrap();
        s.record_login("alice").await.unwrap();
        let fetched = s.get_user("alice").await.unwrap().unwrap();
        assert!(fetched.last_login_at.is_some());
    }

    #[tokio::test]
    async fn api_token_lifecycle_round_trips() {
        let s = store();
        s.create_user(&make_user("alice")).await.unwrap();
        let hash = [7u8; 32];
        let token = s
            .create_api_token("alice", "laptop", &hash, "abcd1234", None)
            .await
            .unwrap();

        let listed = s.list_api_tokens("alice").await.unwrap();
        assert_eq!(listed.len(), 1);

        let authed = s.authenticate_token(&hash).await.unwrap();
        assert!(authed.is_some());
        let (returned_token, user) = authed.unwrap();
        assert_eq!(returned_token.id, token.id);
        assert_eq!(user.id, "alice");

        s.record_token_usage(token.id).await.unwrap();

        assert!(s.revoke_api_token(token.id, "alice").await.unwrap());
        // After revocation, authenticate_token must refuse.
        assert!(s.authenticate_token(&hash).await.unwrap().is_none());
        // Revoking again returns false (already revoked).
        assert!(!s.revoke_api_token(token.id, "alice").await.unwrap());
    }

    #[tokio::test]
    async fn authenticate_token_rejects_inactive_user() {
        let s = store();
        s.create_user(&make_user("alice")).await.unwrap();
        let hash = [3u8; 32];
        s.create_api_token("alice", "k", &hash, "p", None)
            .await
            .unwrap();
        s.update_user_status("alice", "suspended").await.unwrap();
        assert!(s.authenticate_token(&hash).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn delete_user_returns_false_when_absent() {
        let s = store();
        assert!(!s.delete_user("nobody").await.unwrap());
    }

    #[tokio::test]
    async fn delete_user_removes_record_and_tokens() {
        let s = store();
        s.create_user(&make_user("alice")).await.unwrap();
        let hash = [5u8; 32];
        s.create_api_token("alice", "k", &hash, "p", None)
            .await
            .unwrap();
        assert!(s.delete_user("alice").await.unwrap());
        assert!(s.get_user("alice").await.unwrap().is_none());
        assert!(s.authenticate_token(&hash).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn create_user_with_token_writes_both_atomically() {
        let s = store();
        let user = make_user("alice");
        let hash = [1u8; 32];
        let record = s
            .create_user_with_token(&user, "k", &hash, "p", None)
            .await
            .unwrap();
        assert_eq!(record.user_id, "alice");
        assert!(s.get_user("alice").await.unwrap().is_some());
        let listed = s.list_api_tokens("alice").await.unwrap();
        assert_eq!(listed.len(), 1);
    }

    #[tokio::test]
    async fn admin_usage_summary_aggregates_user_counts() {
        let s = store();
        s.create_user(&make_user("alice")).await.unwrap();
        let mut bob = make_user("bob");
        bob.status = "suspended".to_string();
        s.create_user(&bob).await.unwrap();
        s.update_user_role("alice", "admin").await.unwrap();
        let summary = s.admin_usage_summary(Utc::now()).await.unwrap();
        assert_eq!(summary.total_users, 2);
        assert_eq!(summary.active_users, 1);
        assert_eq!(summary.suspended_users, 1);
        assert_eq!(summary.admin_users, 1);
    }
}
