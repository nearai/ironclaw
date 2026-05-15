//! Document-version operations for the filesystem-backed workspace store.
//!
//! **Trust boundary:** mirrors the trait docstring in `src/db/mod.rs`. These
//! helpers accept bare `document_id` UUIDs and trust the caller to have
//! resolved the ownership first.

use chrono::Utc;
use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, FilesystemError, IndexKey, IndexValue, RecordKind,
    RootFilesystem,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::WorkspaceError;
use crate::workspace::{DocumentVersion, VersionSummary};

use super::paths;
use super::{FilesystemWorkspaceStore, fs_to_workspace_error};

const VERSION_KIND: &str = "memory_document_version";

mod fs_keys {
    pub const DOCUMENT_ID: &str = "document_id";
    pub const VERSION: &str = "version";
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredVersion {
    id: Uuid,
    document_id: Uuid,
    version: i32,
    content: String,
    content_hash: String,
    created_at: chrono::DateTime<Utc>,
    changed_by: Option<String>,
}

impl StoredVersion {
    fn into_full(self) -> DocumentVersion {
        DocumentVersion {
            id: self.id,
            document_id: self.document_id,
            version: self.version,
            content: self.content,
            content_hash: self.content_hash,
            created_at: self.created_at,
            changed_by: self.changed_by,
        }
    }

    fn to_summary(&self) -> VersionSummary {
        VersionSummary {
            version: self.version,
            content_hash: self.content_hash.clone(),
            created_at: self.created_at,
            changed_by: self.changed_by.clone(),
        }
    }
}

fn build_entry(v: &StoredVersion) -> Result<Entry, WorkspaceError> {
    let body = serde_json::to_vec(v).map_err(serialization_error)?;
    let kind = RecordKind::new(VERSION_KIND).map_err(|e| WorkspaceError::SearchFailed {
        reason: e.to_string(),
    })?;
    let mut entry = Entry::bytes(body).with_content_type(ContentType::json());
    entry.kind = Some(kind);
    let k_doc = IndexKey::new(fs_keys::DOCUMENT_ID).map_err(|e| WorkspaceError::SearchFailed {
        reason: e.to_string(),
    })?;
    let k_v = IndexKey::new(fs_keys::VERSION).map_err(|e| WorkspaceError::SearchFailed {
        reason: e.to_string(),
    })?;
    Ok(entry
        .with_indexed(k_doc, IndexValue::Text(v.document_id.to_string()))
        .with_indexed(k_v, IndexValue::I64(v.version as i64)))
}

async fn read_versions<F>(
    store: &FilesystemWorkspaceStore<F>,
    document_id: Uuid,
) -> Result<Vec<StoredVersion>, WorkspaceError>
where
    F: RootFilesystem,
{
    let root = paths::versions_root_for_doc(document_id)?;
    let entries = match store.filesystem.list_dir(&root).await {
        Ok(entries) => entries,
        Err(FilesystemError::NotFound { .. }) => return Ok(Vec::new()),
        Err(error) => return Err(fs_to_workspace_error(error)),
    };
    let mut out = Vec::new();
    for entry in entries {
        let Some(versioned) = store
            .filesystem
            .get(&entry.path)
            .await
            .map_err(fs_to_workspace_error)?
        else {
            continue;
        };
        if let Ok(v) = serde_json::from_slice::<StoredVersion>(&versioned.entry.body) {
            out.push(v);
        }
    }
    out.sort_by_key(|v| v.version);
    Ok(out)
}

pub(super) async fn save<F>(
    store: &FilesystemWorkspaceStore<F>,
    document_id: Uuid,
    content: &str,
    content_hash: &str,
    changed_by: Option<&str>,
) -> Result<i32, WorkspaceError>
where
    F: RootFilesystem,
{
    let existing = read_versions(store, document_id).await?;
    let next_version = existing.last().map(|v| v.version + 1).unwrap_or(1);
    let template = StoredVersion {
        id: Uuid::new_v4(),
        document_id,
        version: next_version,
        content: content.to_string(),
        content_hash: content_hash.to_string(),
        created_at: Utc::now(),
        changed_by: changed_by.map(|s| s.to_string()),
    };
    // CAS::Absent makes the new-version slot atomic for the common case.
    // On a `VersionMismatch` we retry with the next slot.
    let mut attempt_version = next_version;
    loop {
        let attempt_path = paths::version_path(document_id, attempt_version)?;
        let mut attempt = template.clone();
        attempt.version = attempt_version;
        let attempt_entry = build_entry(&attempt)?;
        match store
            .filesystem
            .put(&attempt_path, attempt_entry, CasExpectation::Absent)
            .await
        {
            Ok(_) => return Ok(attempt_version),
            Err(FilesystemError::VersionMismatch { .. }) => {
                attempt_version += 1;
                if attempt_version > next_version + 16 {
                    return Err(WorkspaceError::SearchFailed {
                        reason: "version slot contention".to_string(),
                    });
                }
                continue;
            }
            Err(error) => return Err(fs_to_workspace_error(error)),
        }
    }
}

pub(super) async fn get<F>(
    store: &FilesystemWorkspaceStore<F>,
    document_id: Uuid,
    version: i32,
) -> Result<DocumentVersion, WorkspaceError>
where
    F: RootFilesystem,
{
    let path = paths::version_path(document_id, version)?;
    let Some(versioned) = store
        .filesystem
        .get(&path)
        .await
        .map_err(fs_to_workspace_error)?
    else {
        return Err(WorkspaceError::VersionNotFound {
            document_id,
            version,
        });
    };
    let v: StoredVersion =
        serde_json::from_slice(&versioned.entry.body).map_err(serialization_error)?;
    Ok(v.into_full())
}

pub(super) async fn list<F>(
    store: &FilesystemWorkspaceStore<F>,
    document_id: Uuid,
    limit: i64,
) -> Result<Vec<VersionSummary>, WorkspaceError>
where
    F: RootFilesystem,
{
    let mut versions = read_versions(store, document_id).await?;
    versions.sort_by_key(|b| std::cmp::Reverse(b.version));
    if limit > 0 {
        versions.truncate(limit as usize);
    }
    Ok(versions.iter().map(|v| v.to_summary()).collect())
}

pub(super) async fn get_latest_number<F>(
    store: &FilesystemWorkspaceStore<F>,
    document_id: Uuid,
) -> Result<Option<i32>, WorkspaceError>
where
    F: RootFilesystem,
{
    let versions = read_versions(store, document_id).await?;
    Ok(versions.last().map(|v| v.version))
}

pub(super) async fn prune<F>(
    store: &FilesystemWorkspaceStore<F>,
    document_id: Uuid,
    keep_count: i32,
) -> Result<u64, WorkspaceError>
where
    F: RootFilesystem,
{
    if keep_count < 0 {
        return Ok(0);
    }
    let versions = read_versions(store, document_id).await?;
    if versions.len() <= keep_count as usize {
        return Ok(0);
    }
    let to_delete = versions.len() - keep_count as usize;
    let mut deleted = 0u64;
    for v in versions.iter().take(to_delete) {
        let path = paths::version_path(document_id, v.version)?;
        if store.filesystem.delete(&path).await.is_ok() {
            deleted += 1;
        }
    }
    Ok(deleted)
}

fn serialization_error(error: serde_json::Error) -> WorkspaceError {
    WorkspaceError::SearchFailed {
        reason: format!("workspace version serialization: {error}"),
    }
}
