//! Filesystem-backed implementation of [`ChannelPairingStore`].
//!
//! Routes pairing requests and channel-identity records through the unified
//! [`RootFilesystem`] surface. Mirrors the canonical migration shape from
//! `crates/ironclaw_secrets/src/filesystem_store.rs` and
//! `crates/ironclaw_authorization/src/lib.rs`.
//!
//! Path layout:
//!
//! - `/pairing/requests/<channel>/<request_id>` — pending pairing requests.
//! - `/pairing/identities/<channel>/<binding_id>` — approved channel
//!   identities. `binding_id` is the lowercased, encoded `external_id`.
//! - `/pairing/code-index/<channel>/<UPPER(code)>` — pointer to the
//!   `request_id` for a still-pending code, so `approve_pairing` can find
//!   the request without scanning the whole channel tree.
//!
//! `resolve_channel_identity` needs the resolved user's role to construct a
//! [`UserId`]. The facade composes a [`FilesystemUserStore`] over the same
//! `RootFilesystem` so the lookup stays inside one mount; for multi-mount
//! deployments callers must point both facades at the same composite
//! filesystem.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{Duration, Utc};
use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, FilesystemError, IndexKey, IndexValue, RecordKind,
    RootFilesystem,
};
use ironclaw_host_api::VirtualPath;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db::{
    ChannelPairingStore, DatabaseError, PairingApprovalRecord, PairingRequestRecord, UserStore,
    generate_pairing_code,
};
use crate::ownership::{UserId, UserRole};

use super::filesystem_users::FilesystemUserStore;

const REQUEST_KIND: &str = "pairing_request";
const IDENTITY_KIND: &str = "channel_identity";

mod fs_keys {
    pub const CHANNEL: &str = "channel";
    pub const STATUS: &str = "status";
    pub const OWNER_ID: &str = "owner_id";
    pub const HAS_OWNER_BINDING: &str = "has_owner_binding";
    pub const PAIRED: &str = "paired";
    pub const EXTERNAL_ID: &str = "external_id";
}

const PENDING: &str = "pending";
const APPROVED: &str = "approved";

const DEFAULT_TTL_MINUTES: i64 = 15;

/// Filesystem-backed [`ChannelPairingStore`].
pub struct FilesystemChannelPairingStore<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<F>,
    users: FilesystemUserStore<F>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredPairingRequest {
    id: Uuid,
    channel: String,
    external_id: String,
    code: String,
    meta: Option<serde_json::Value>,
    created_at: chrono::DateTime<Utc>,
    expires_at: chrono::DateTime<Utc>,
    owner_id: Option<String>,
    approved_at: Option<chrono::DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredChannelIdentity {
    id: Uuid,
    channel: String,
    external_id: String,
    owner_id: String,
    created_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CodePointer {
    request_id: Uuid,
}

impl<F> FilesystemChannelPairingStore<F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: Arc<F>) -> Self {
        let users = FilesystemUserStore::new(Arc::clone(&filesystem));
        Self { filesystem, users }
    }

    fn normalize_channel(channel: &str) -> String {
        crate::pairing::normalize_channel_name(channel)
    }

    fn requests_root(channel: &str) -> Result<VirtualPath, DatabaseError> {
        VirtualPath::new(format!("/pairing/requests/{}", encode_segment(channel)))
            .map_err(|e| DatabaseError::Query(format!("invalid pairing requests root: {e}")))
    }

    fn request_path(channel: &str, id: Uuid) -> Result<VirtualPath, DatabaseError> {
        VirtualPath::new(format!(
            "/pairing/requests/{}/{}",
            encode_segment(channel),
            id
        ))
        .map_err(|e| DatabaseError::Query(format!("invalid pairing request path: {e}")))
    }

    fn identities_root(channel: &str) -> Result<VirtualPath, DatabaseError> {
        VirtualPath::new(format!("/pairing/identities/{}", encode_segment(channel)))
            .map_err(|e| DatabaseError::Query(format!("invalid identities root: {e}")))
    }

    fn identity_path(channel: &str, external_id: &str) -> Result<VirtualPath, DatabaseError> {
        VirtualPath::new(format!(
            "/pairing/identities/{}/{}",
            encode_segment(channel),
            encode_segment(external_id)
        ))
        .map_err(|e| DatabaseError::Query(format!("invalid identity path: {e}")))
    }

    fn code_pointer_path(channel: &str, code: &str) -> Result<VirtualPath, DatabaseError> {
        VirtualPath::new(format!(
            "/pairing/code-index/{}/{}",
            encode_segment(channel),
            code.to_ascii_uppercase()
        ))
        .map_err(|e| DatabaseError::Query(format!("invalid code-index path: {e}")))
    }

    fn build_request_entry(req: &StoredPairingRequest) -> Result<Entry, DatabaseError> {
        let body =
            serde_json::to_vec(req).map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        let kind =
            RecordKind::new(REQUEST_KIND).map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut entry = Entry::bytes(body).with_content_type(ContentType::json());
        entry.kind = Some(kind);
        let k_channel =
            IndexKey::new(fs_keys::CHANNEL).map_err(|e| DatabaseError::Query(e.to_string()))?;
        let k_status =
            IndexKey::new(fs_keys::STATUS).map_err(|e| DatabaseError::Query(e.to_string()))?;
        let k_external =
            IndexKey::new(fs_keys::EXTERNAL_ID).map_err(|e| DatabaseError::Query(e.to_string()))?;
        let status = if req.approved_at.is_some() {
            APPROVED
        } else {
            PENDING
        };
        Ok(entry
            .with_indexed(k_channel, IndexValue::Text(req.channel.clone()))
            .with_indexed(k_status, IndexValue::Text(status.to_string()))
            .with_indexed(k_external, IndexValue::Text(req.external_id.clone())))
    }

    fn build_identity_entry(ident: &StoredChannelIdentity) -> Result<Entry, DatabaseError> {
        let body =
            serde_json::to_vec(ident).map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        let kind =
            RecordKind::new(IDENTITY_KIND).map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut entry = Entry::bytes(body).with_content_type(ContentType::json());
        entry.kind = Some(kind);
        let k_channel =
            IndexKey::new(fs_keys::CHANNEL).map_err(|e| DatabaseError::Query(e.to_string()))?;
        let k_owner =
            IndexKey::new(fs_keys::OWNER_ID).map_err(|e| DatabaseError::Query(e.to_string()))?;
        let k_paired =
            IndexKey::new(fs_keys::PAIRED).map_err(|e| DatabaseError::Query(e.to_string()))?;
        let k_has_owner = IndexKey::new(fs_keys::HAS_OWNER_BINDING)
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(entry
            .with_indexed(k_channel, IndexValue::Text(ident.channel.clone()))
            .with_indexed(k_owner, IndexValue::Text(ident.owner_id.clone()))
            .with_indexed(k_paired, IndexValue::Bool(true))
            .with_indexed(k_has_owner, IndexValue::Bool(true)))
    }

    async fn read_request(
        &self,
        channel: &str,
        id: Uuid,
    ) -> Result<Option<StoredPairingRequest>, DatabaseError> {
        let path = Self::request_path(channel, id)?;
        let Some(versioned) = self.filesystem.get(&path).await.map_err(fs_to_db_error)? else {
            return Ok(None);
        };
        let stored: StoredPairingRequest = serde_json::from_slice(&versioned.entry.body)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        Ok(Some(stored))
    }

    async fn read_identity(
        &self,
        channel: &str,
        external_id: &str,
    ) -> Result<Option<StoredChannelIdentity>, DatabaseError> {
        let path = Self::identity_path(channel, external_id)?;
        let Some(versioned) = self.filesystem.get(&path).await.map_err(fs_to_db_error)? else {
            return Ok(None);
        };
        let stored: StoredChannelIdentity = serde_json::from_slice(&versioned.entry.body)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        Ok(Some(stored))
    }

    async fn write_request(&self, req: &StoredPairingRequest) -> Result<(), DatabaseError> {
        let path = Self::request_path(&req.channel, req.id)?;
        let entry = Self::build_request_entry(req)?;
        self.filesystem
            .put(&path, entry, CasExpectation::Any)
            .await
            .map(|_| ())
            .map_err(fs_to_db_error)
    }

    async fn write_identity(&self, ident: &StoredChannelIdentity) -> Result<(), DatabaseError> {
        let path = Self::identity_path(&ident.channel, &ident.external_id)?;
        let entry = Self::build_identity_entry(ident)?;
        self.filesystem
            .put(&path, entry, CasExpectation::Any)
            .await
            .map(|_| ())
            .map_err(fs_to_db_error)
    }

    async fn delete_identity(&self, channel: &str, external_id: &str) -> Result<(), DatabaseError> {
        let path = Self::identity_path(channel, external_id)?;
        match self.filesystem.delete(&path).await {
            Ok(()) => Ok(()),
            Err(FilesystemError::NotFound { .. }) => Ok(()),
            Err(error) => Err(fs_to_db_error(error)),
        }
    }

    async fn write_code_pointer(
        &self,
        channel: &str,
        code: &str,
        request_id: Uuid,
    ) -> Result<(), DatabaseError> {
        let path = Self::code_pointer_path(channel, code)?;
        let body = serde_json::to_vec(&CodePointer { request_id })
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        let entry = Entry::bytes(body).with_content_type(ContentType::json());
        self.filesystem
            .put(&path, entry, CasExpectation::Any)
            .await
            .map(|_| ())
            .map_err(fs_to_db_error)
    }

    async fn read_code_pointer(
        &self,
        channel: &str,
        code: &str,
    ) -> Result<Option<Uuid>, DatabaseError> {
        let path = Self::code_pointer_path(channel, code)?;
        let Some(versioned) = self.filesystem.get(&path).await.map_err(fs_to_db_error)? else {
            return Ok(None);
        };
        let pointer: CodePointer = serde_json::from_slice(&versioned.entry.body)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        Ok(Some(pointer.request_id))
    }

    async fn delete_code_pointer(&self, channel: &str, code: &str) -> Result<(), DatabaseError> {
        let path = Self::code_pointer_path(channel, code)?;
        match self.filesystem.delete(&path).await {
            Ok(()) => Ok(()),
            Err(FilesystemError::NotFound { .. }) => Ok(()),
            Err(error) => Err(fs_to_db_error(error)),
        }
    }

    async fn list_requests_for_channel(
        &self,
        channel: &str,
    ) -> Result<Vec<StoredPairingRequest>, DatabaseError> {
        let root = Self::requests_root(channel)?;
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
            if let Ok(req) = serde_json::from_slice::<StoredPairingRequest>(&versioned.entry.body) {
                out.push(req);
            }
        }
        Ok(out)
    }
}

#[async_trait]
impl<F> ChannelPairingStore for FilesystemChannelPairingStore<F>
where
    F: RootFilesystem,
{
    async fn resolve_channel_identity(
        &self,
        channel: &str,
        external_id: &str,
    ) -> Result<Option<UserId>, DatabaseError> {
        let channel = Self::normalize_channel(channel);
        let Some(ident) = self.read_identity(&channel, external_id).await? else {
            return Ok(None);
        };
        let Some(user) = self.users.get_user(&ident.owner_id).await? else {
            return Ok(None);
        };
        if user.status != "active" {
            return Ok(None);
        }
        let role = UserRole::from_db_role(&user.role);
        Ok(Some(UserId::from_trusted(user.id, role)))
    }

    async fn read_allow_from(&self, channel: &str) -> Result<Vec<String>, DatabaseError> {
        let channel = Self::normalize_channel(channel);
        let root = Self::identities_root(&channel)?;
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
            let stored: StoredChannelIdentity = match serde_json::from_slice(&versioned.entry.body)
            {
                Ok(s) => s,
                Err(_) => continue,
            };
            // Join with users — only include identities whose owning user is
            // active. Matches the SQL JOIN ... WHERE u.status = 'active'.
            if let Some(user) = self.users.get_user(&stored.owner_id).await?
                && user.status == "active"
            {
                out.push(stored.external_id);
            }
        }
        out.sort();
        Ok(out)
    }

    async fn resolve_channel_external_id_for_owner(
        &self,
        channel: &str,
        owner_id: &str,
    ) -> Result<Option<String>, DatabaseError> {
        let channel = Self::normalize_channel(channel);
        let root = Self::identities_root(&channel)?;
        let entries = match self.filesystem.list_dir(&root).await {
            Ok(entries) => entries,
            Err(FilesystemError::NotFound { .. }) => return Ok(None),
            Err(error) => return Err(fs_to_db_error(error)),
        };
        let mut matches: Vec<String> = Vec::new();
        for entry in entries {
            let Some(versioned) = self
                .filesystem
                .get(&entry.path)
                .await
                .map_err(fs_to_db_error)?
            else {
                continue;
            };
            let stored: StoredChannelIdentity = match serde_json::from_slice(&versioned.entry.body)
            {
                Ok(s) => s,
                Err(_) => continue,
            };
            if stored.owner_id != owner_id {
                continue;
            }
            // Mirror the legacy SQL: include rows where the user is active OR
            // missing (LEFT JOIN: u.id IS NULL OR u.status = 'active').
            match self.users.get_user(owner_id).await? {
                Some(user) if user.status == "active" => matches.push(stored.external_id),
                None => matches.push(stored.external_id),
                _ => {}
            }
        }
        matches.sort();
        Ok(matches.into_iter().next())
    }

    async fn upsert_pairing_request(
        &self,
        channel: &str,
        external_id: &str,
        meta: Option<serde_json::Value>,
    ) -> Result<PairingRequestRecord, DatabaseError> {
        let channel = Self::normalize_channel(channel);
        let now = Utc::now();
        // Retire any active pending request for this sender by shrinking its
        // expires_at so future approvals reject. This matches the legacy
        // SQL `UPDATE ... SET expires_at = now WHERE ... AND expires_at > now`.
        let existing = self.list_requests_for_channel(&channel).await?;
        for mut req in existing {
            if req.external_id == external_id && req.approved_at.is_none() && req.expires_at > now {
                req.expires_at = now;
                // Best-effort: also clean up the code pointer so a racing
                // approver doesn't pick up the retired request.
                let _ = self.delete_code_pointer(&channel, &req.code).await;
                self.write_request(&req).await?;
            }
        }

        // Try up to 3 codes (matching the legacy retry loop on UNIQUE
        // constraint).
        for _ in 0..3 {
            let id = Uuid::new_v4();
            let code = generate_pairing_code();
            // CAS::Absent on the code pointer makes the code-uniqueness
            // check atomic. If the pointer already exists, retry.
            let pointer_path = Self::code_pointer_path(&channel, &code)?;
            let pointer_body = serde_json::to_vec(&CodePointer { request_id: id })
                .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
            let pointer_entry = Entry::bytes(pointer_body).with_content_type(ContentType::json());
            match self
                .filesystem
                .put(&pointer_path, pointer_entry, CasExpectation::Absent)
                .await
            {
                Ok(_) => {}
                Err(FilesystemError::VersionMismatch { .. }) => continue,
                Err(error) => return Err(fs_to_db_error(error)),
            };
            let req = StoredPairingRequest {
                id,
                channel: channel.clone(),
                external_id: external_id.to_string(),
                code: code.clone(),
                meta: meta.clone(),
                created_at: now,
                expires_at: now + Duration::minutes(DEFAULT_TTL_MINUTES),
                owner_id: None,
                approved_at: None,
            };
            self.write_request(&req).await?;
            return Ok(PairingRequestRecord {
                id,
                channel: req.channel,
                external_id: req.external_id,
                code: req.code,
                created: true,
                created_at: req.created_at,
                expires_at: req.expires_at,
            });
        }
        Err(DatabaseError::Query(
            "failed to generate unique pairing code after 3 attempts".to_string(),
        ))
    }

    async fn approve_pairing(
        &self,
        channel: &str,
        code: &str,
        owner_id: &str,
    ) -> Result<PairingApprovalRecord, DatabaseError> {
        let channel = Self::normalize_channel(channel);
        let request_id = self
            .read_code_pointer(&channel, code)
            .await?
            .ok_or_else(|| DatabaseError::NotFound {
                entity: "pairing_request".into(),
                id: code.to_string(),
            })?;
        let mut req = self
            .read_request(&channel, request_id)
            .await?
            .ok_or_else(|| DatabaseError::NotFound {
                entity: "pairing_request".into(),
                id: code.to_string(),
            })?;
        let now = Utc::now();
        if req.approved_at.is_some() {
            return Err(DatabaseError::NotFound {
                entity: "pairing_request".into(),
                id: code.to_string(),
            });
        }
        if req.expires_at <= now {
            return Err(DatabaseError::NotFound {
                entity: "pairing_request".into(),
                id: code.to_string(),
            });
        }
        if req.channel != channel {
            return Err(DatabaseError::NotFound {
                entity: "pairing_request".into(),
                id: code.to_string(),
            });
        }
        let previous_owner_id = self
            .read_identity(&channel, &req.external_id)
            .await?
            .map(|ident| ident.owner_id);

        // Persist request as approved (CAS-free; the legacy SQL also commits
        // unconditionally inside the transaction).
        req.owner_id = Some(owner_id.to_string());
        req.approved_at = Some(now);
        self.write_request(&req).await?;

        // Insert/upsert channel identity. We replace any prior owner.
        let identity = StoredChannelIdentity {
            id: Uuid::new_v4(),
            channel: channel.clone(),
            external_id: req.external_id.clone(),
            owner_id: owner_id.to_string(),
            created_at: now,
        };
        self.write_identity(&identity).await?;
        // The code pointer can be cleared — the code has been consumed.
        let _ = self.delete_code_pointer(&channel, &req.code).await;

        Ok(PairingApprovalRecord {
            request_id: req.id,
            channel: req.channel.clone(),
            external_id: req.external_id.clone(),
            owner_id: owner_id.to_string(),
            previous_owner_id,
        })
    }

    async fn revert_pairing_approval(
        &self,
        approval: &PairingApprovalRecord,
    ) -> Result<(), DatabaseError> {
        let channel = Self::normalize_channel(&approval.channel);
        let Some(mut req) = self.read_request(&channel, approval.request_id).await? else {
            return Err(DatabaseError::NotFound {
                entity: "pairing_approval".into(),
                id: approval.request_id.to_string(),
            });
        };
        if req.approved_at.is_none() || req.owner_id.as_deref() != Some(approval.owner_id.as_str())
        {
            return Err(DatabaseError::NotFound {
                entity: "pairing_approval".into(),
                id: approval.request_id.to_string(),
            });
        }
        req.approved_at = None;
        req.owner_id = None;
        self.write_request(&req).await?;

        if let Some(previous) = &approval.previous_owner_id {
            let identity = StoredChannelIdentity {
                id: Uuid::new_v4(),
                channel: channel.clone(),
                external_id: approval.external_id.clone(),
                owner_id: previous.clone(),
                created_at: Utc::now(),
            };
            self.write_identity(&identity).await?;
        } else if let Some(ident) = self.read_identity(&channel, &approval.external_id).await?
            && ident.owner_id == approval.owner_id
        {
            self.delete_identity(&channel, &approval.external_id)
                .await?;
        }
        Ok(())
    }

    async fn list_pending_pairings(
        &self,
        channel: &str,
    ) -> Result<Vec<PairingRequestRecord>, DatabaseError> {
        let channel = Self::normalize_channel(channel);
        let now = Utc::now();
        let mut out: Vec<PairingRequestRecord> = self
            .list_requests_for_channel(&channel)
            .await?
            .into_iter()
            .filter(|r| r.approved_at.is_none() && r.expires_at > now)
            .map(|r| PairingRequestRecord {
                id: r.id,
                channel: r.channel,
                external_id: r.external_id,
                code: r.code,
                created: false,
                created_at: r.created_at,
                expires_at: r.expires_at,
            })
            .collect();
        out.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        Ok(out)
    }

    async fn remove_channel_identity(
        &self,
        channel: &str,
        external_id: &str,
    ) -> Result<(), DatabaseError> {
        let channel = Self::normalize_channel(channel);
        self.delete_identity(&channel, external_id).await
    }

    async fn create_channel_identity(
        &self,
        channel: &str,
        external_id: &str,
        owner_id: &str,
    ) -> Result<(), DatabaseError> {
        let channel = Self::normalize_channel(channel);
        let identity = StoredChannelIdentity {
            id: Uuid::new_v4(),
            channel,
            external_id: external_id.to_string(),
            owner_id: owner_id.to_string(),
            created_at: Utc::now(),
        };
        self.write_identity(&identity).await
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

// Re-export `write_code_pointer` to silence unused-method warnings; kept as
// an explicit helper because callers may want to reseat a code pointer
// after a manual repair.
#[allow(dead_code)]
fn _keep_write_code_pointer_used<F: RootFilesystem>(
    s: &FilesystemChannelPairingStore<F>,
) -> &FilesystemChannelPairingStore<F> {
    let _ = FilesystemChannelPairingStore::<F>::write_code_pointer;
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::UserRecord;
    use ironclaw_filesystem::InMemoryBackend;

    async fn make_user_in(s: &FilesystemChannelPairingStore<InMemoryBackend>, id: &str) {
        let user = UserRecord {
            id: id.to_string(),
            email: None,
            display_name: id.to_string(),
            status: "active".to_string(),
            role: "member".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_login_at: None,
            created_by: None,
            metadata: serde_json::json!({}),
        };
        s.users.create_user(&user).await.unwrap();
    }

    fn store() -> FilesystemChannelPairingStore<InMemoryBackend> {
        FilesystemChannelPairingStore::new(Arc::new(InMemoryBackend::new()))
    }

    #[tokio::test]
    async fn resolve_unknown_external_id_returns_none() {
        let s = store();
        assert!(
            s.resolve_channel_identity("telegram", "x")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn upsert_pairing_request_returns_fresh_code() {
        let s = store();
        let req = s
            .upsert_pairing_request("telegram", "tg-1", None)
            .await
            .unwrap();
        assert_eq!(req.channel, "telegram");
        assert_eq!(req.code.chars().count(), 8);
        assert!(req.created);
    }

    #[tokio::test]
    async fn upsert_rotates_code_on_retry() {
        let s = store();
        let r1 = s
            .upsert_pairing_request("telegram", "tg-1", None)
            .await
            .unwrap();
        let r2 = s
            .upsert_pairing_request("telegram", "tg-1", None)
            .await
            .unwrap();
        assert_ne!(r1.code, r2.code);
    }

    #[tokio::test]
    async fn approve_pairing_creates_channel_identity() {
        let s = store();
        make_user_in(&s, "alice").await;
        let req = s
            .upsert_pairing_request("telegram", "tg-alice", None)
            .await
            .unwrap();
        s.approve_pairing("telegram", &req.code, "alice")
            .await
            .unwrap();
        let resolved = s
            .resolve_channel_identity("telegram", "tg-alice")
            .await
            .unwrap();
        assert!(resolved.is_some());
        assert_eq!(resolved.unwrap().as_str(), "alice");
    }

    #[tokio::test]
    async fn approve_pairing_after_rotation_rejects_old_code() {
        let s = store();
        make_user_in(&s, "alice").await;
        let r1 = s
            .upsert_pairing_request("telegram", "tg-1", None)
            .await
            .unwrap();
        let _r2 = s
            .upsert_pairing_request("telegram", "tg-1", None)
            .await
            .unwrap();
        let err = s.approve_pairing("telegram", &r1.code, "alice").await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn revert_pairing_approval_restores_previous_owner() {
        let s = store();
        make_user_in(&s, "alice").await;
        make_user_in(&s, "bob").await;
        s.create_channel_identity("telegram", "tg-1", "alice")
            .await
            .unwrap();
        let req = s
            .upsert_pairing_request("telegram", "tg-1", None)
            .await
            .unwrap();
        let approval = s
            .approve_pairing("telegram", &req.code, "bob")
            .await
            .unwrap();
        assert_eq!(approval.previous_owner_id.as_deref(), Some("alice"));
        s.revert_pairing_approval(&approval).await.unwrap();
        let resolved = s
            .resolve_channel_identity("telegram", "tg-1")
            .await
            .unwrap();
        assert_eq!(resolved.unwrap().as_str(), "alice");
    }

    #[tokio::test]
    async fn revert_pairing_without_previous_owner_removes_identity() {
        let s = store();
        make_user_in(&s, "alice").await;
        let req = s
            .upsert_pairing_request("telegram", "tg-1", None)
            .await
            .unwrap();
        let approval = s
            .approve_pairing("telegram", &req.code, "alice")
            .await
            .unwrap();
        s.revert_pairing_approval(&approval).await.unwrap();
        let resolved = s
            .resolve_channel_identity("telegram", "tg-1")
            .await
            .unwrap();
        assert!(resolved.is_none());
    }

    #[tokio::test]
    async fn list_pending_pairings_skips_approved_and_expired() {
        let s = store();
        make_user_in(&s, "alice").await;
        let r1 = s
            .upsert_pairing_request("telegram", "tg-1", None)
            .await
            .unwrap();
        let r2 = s
            .upsert_pairing_request("telegram", "tg-2", None)
            .await
            .unwrap();
        s.approve_pairing("telegram", &r2.code, "alice")
            .await
            .unwrap();
        let pending = s.list_pending_pairings("telegram").await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, r1.id);
    }

    #[tokio::test]
    async fn read_allow_from_returns_active_user_external_ids() {
        let s = store();
        make_user_in(&s, "alice").await;
        s.create_channel_identity("telegram", "tg-alice", "alice")
            .await
            .unwrap();
        let allow = s.read_allow_from("telegram").await.unwrap();
        assert_eq!(allow, vec!["tg-alice".to_string()]);
    }

    #[tokio::test]
    async fn remove_channel_identity_clears_resolve() {
        let s = store();
        make_user_in(&s, "alice").await;
        s.create_channel_identity("telegram", "tg-1", "alice")
            .await
            .unwrap();
        s.remove_channel_identity("telegram", "tg-1").await.unwrap();
        let resolved = s
            .resolve_channel_identity("telegram", "tg-1")
            .await
            .unwrap();
        assert!(resolved.is_none());
    }
}
