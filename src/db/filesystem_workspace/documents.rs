//! Document-level operations for the filesystem-backed workspace store.

use std::collections::{HashMap, HashSet};

use chrono::Utc;
use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, FilesystemError, IndexKey, IndexValue, RecordKind,
    RootFilesystem,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::WorkspaceError;
use crate::workspace::{MemoryDocument, WorkspaceEntry};

use super::paths;
use super::{FilesystemWorkspaceStore, fs_to_workspace_error};

const DOCUMENT_KIND: &str = "memory_document";

pub(super) mod fs_keys {
    pub const USER_ID: &str = "user_id";
    pub const AGENT_ID: &str = "agent_id";
    pub const KIND: &str = "kind";
    pub const PATH: &str = "path";
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct StoredDocument {
    pub id: Uuid,
    pub user_id: String,
    pub agent_id: Option<Uuid>,
    pub path: String,
    pub content: String,
    pub created_at: chrono::DateTime<Utc>,
    pub updated_at: chrono::DateTime<Utc>,
    pub metadata: serde_json::Value,
}

impl StoredDocument {
    fn to_document(&self) -> MemoryDocument {
        MemoryDocument {
            id: self.id,
            user_id: self.user_id.clone(),
            agent_id: self.agent_id,
            path: self.path.clone(),
            content: self.content.clone(),
            created_at: self.created_at,
            updated_at: self.updated_at,
            metadata: self.metadata.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PathIndexEntry {
    doc_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct IdIndexEntry {
    user_id: String,
}

pub(super) fn build_document_entry(doc: &StoredDocument) -> Result<Entry, WorkspaceError> {
    let body = serde_json::to_vec(doc).map_err(serialization_error)?;
    let kind = RecordKind::new(DOCUMENT_KIND).map_err(|e| WorkspaceError::SearchFailed {
        reason: e.to_string(),
    })?;
    let mut entry = Entry::bytes(body).with_content_type(ContentType::json());
    entry.kind = Some(kind);
    let k_user = IndexKey::new(fs_keys::USER_ID).map_err(|e| WorkspaceError::SearchFailed {
        reason: e.to_string(),
    })?;
    let k_agent = IndexKey::new(fs_keys::AGENT_ID).map_err(|e| WorkspaceError::SearchFailed {
        reason: e.to_string(),
    })?;
    let k_kind = IndexKey::new(fs_keys::KIND).map_err(|e| WorkspaceError::SearchFailed {
        reason: e.to_string(),
    })?;
    let k_path = IndexKey::new(fs_keys::PATH).map_err(|e| WorkspaceError::SearchFailed {
        reason: e.to_string(),
    })?;
    Ok(entry
        .with_indexed(k_user, IndexValue::Text(doc.user_id.clone()))
        .with_indexed(
            k_agent,
            IndexValue::Text(paths::agent_id_segment(doc.agent_id)),
        )
        .with_indexed(k_kind, IndexValue::Text(DOCUMENT_KIND.to_string()))
        .with_indexed(k_path, IndexValue::Text(doc.path.clone())))
}

pub(super) async fn read_stored<F>(
    store: &FilesystemWorkspaceStore<F>,
    user_id: &str,
    doc_id: Uuid,
) -> Result<Option<StoredDocument>, WorkspaceError>
where
    F: RootFilesystem,
{
    let path = paths::document_path(user_id, doc_id)?;
    let Some(versioned) = store
        .filesystem
        .get(&path)
        .await
        .map_err(fs_to_workspace_error)?
    else {
        return Ok(None);
    };
    let stored: StoredDocument =
        serde_json::from_slice(&versioned.entry.body).map_err(serialization_error)?;
    if stored.user_id != user_id {
        return Ok(None);
    }
    Ok(Some(stored))
}

pub(super) async fn write_stored<F>(
    store: &FilesystemWorkspaceStore<F>,
    doc: &StoredDocument,
) -> Result<(), WorkspaceError>
where
    F: RootFilesystem,
{
    let path = paths::document_path(&doc.user_id, doc.id)?;
    let entry = build_document_entry(doc)?;
    store
        .filesystem
        .put(&path, entry, CasExpectation::Any)
        .await
        .map(|_| ())
        .map_err(fs_to_workspace_error)
}

async fn write_path_index<F>(
    store: &FilesystemWorkspaceStore<F>,
    user_id: &str,
    agent_id: Option<Uuid>,
    doc_path: &str,
    doc_id: Uuid,
) -> Result<(), WorkspaceError>
where
    F: RootFilesystem,
{
    let path = paths::path_index_entry(user_id, agent_id, doc_path)?;
    let body = serde_json::to_vec(&PathIndexEntry { doc_id }).map_err(serialization_error)?;
    let entry = Entry::bytes(body).with_content_type(ContentType::json());
    store
        .filesystem
        .put(&path, entry, CasExpectation::Any)
        .await
        .map(|_| ())
        .map_err(fs_to_workspace_error)
}

async fn write_id_index<F>(
    store: &FilesystemWorkspaceStore<F>,
    doc_id: Uuid,
    user_id: &str,
) -> Result<(), WorkspaceError>
where
    F: RootFilesystem,
{
    let path = paths::id_index_path(doc_id)?;
    let body = serde_json::to_vec(&IdIndexEntry {
        user_id: user_id.to_string(),
    })
    .map_err(serialization_error)?;
    let entry = Entry::bytes(body).with_content_type(ContentType::json());
    store
        .filesystem
        .put(&path, entry, CasExpectation::Any)
        .await
        .map(|_| ())
        .map_err(fs_to_workspace_error)
}

async fn lookup_user_id_for_doc<F>(
    store: &FilesystemWorkspaceStore<F>,
    doc_id: Uuid,
) -> Result<Option<String>, WorkspaceError>
where
    F: RootFilesystem,
{
    let path = paths::id_index_path(doc_id)?;
    let Some(versioned) = store
        .filesystem
        .get(&path)
        .await
        .map_err(fs_to_workspace_error)?
    else {
        return Ok(None);
    };
    let entry: IdIndexEntry =
        serde_json::from_slice(&versioned.entry.body).map_err(serialization_error)?;
    Ok(Some(entry.user_id))
}

async fn lookup_doc_id_by_path<F>(
    store: &FilesystemWorkspaceStore<F>,
    user_id: &str,
    agent_id: Option<Uuid>,
    doc_path: &str,
) -> Result<Option<Uuid>, WorkspaceError>
where
    F: RootFilesystem,
{
    let path = paths::path_index_entry(user_id, agent_id, doc_path)?;
    let Some(versioned) = store
        .filesystem
        .get(&path)
        .await
        .map_err(fs_to_workspace_error)?
    else {
        return Ok(None);
    };
    let entry: PathIndexEntry =
        serde_json::from_slice(&versioned.entry.body).map_err(serialization_error)?;
    Ok(Some(entry.doc_id))
}

pub(super) async fn get_by_path<F>(
    store: &FilesystemWorkspaceStore<F>,
    user_id: &str,
    agent_id: Option<Uuid>,
    path: &str,
) -> Result<MemoryDocument, WorkspaceError>
where
    F: RootFilesystem,
{
    if let Some(doc_id) = lookup_doc_id_by_path(store, user_id, agent_id, path).await?
        && let Some(stored) = read_stored(store, user_id, doc_id).await?
        && stored.path == path
        && stored.agent_id == agent_id
    {
        return Ok(stored.to_document());
    }
    Err(WorkspaceError::DocumentNotFound {
        doc_type: path.to_string(),
        user_id: user_id.to_string(),
    })
}

pub(super) async fn get_by_id<F>(
    store: &FilesystemWorkspaceStore<F>,
    id: Uuid,
) -> Result<MemoryDocument, WorkspaceError>
where
    F: RootFilesystem,
{
    let Some(user_id) = lookup_user_id_for_doc(store, id).await? else {
        return Err(WorkspaceError::DocumentNotFound {
            doc_type: id.to_string(),
            user_id: "<unknown>".to_string(),
        });
    };
    let Some(stored) = read_stored(store, &user_id, id).await? else {
        return Err(WorkspaceError::DocumentNotFound {
            doc_type: id.to_string(),
            user_id,
        });
    };
    Ok(stored.to_document())
}

pub(super) async fn get_or_create<F>(
    store: &FilesystemWorkspaceStore<F>,
    user_id: &str,
    agent_id: Option<Uuid>,
    path: &str,
) -> Result<MemoryDocument, WorkspaceError>
where
    F: RootFilesystem,
{
    if let Some(doc_id) = lookup_doc_id_by_path(store, user_id, agent_id, path).await?
        && let Some(stored) = read_stored(store, user_id, doc_id).await?
    {
        return Ok(stored.to_document());
    }
    let now = Utc::now();
    let id = Uuid::new_v4();
    let doc = StoredDocument {
        id,
        user_id: user_id.to_string(),
        agent_id,
        path: path.to_string(),
        content: String::new(),
        created_at: now,
        updated_at: now,
        metadata: serde_json::Value::Object(serde_json::Map::new()),
    };
    write_stored(store, &doc).await?;
    write_path_index(store, user_id, agent_id, path, id).await?;
    write_id_index(store, id, user_id).await?;
    Ok(doc.to_document())
}

pub(super) async fn update_content<F>(
    store: &FilesystemWorkspaceStore<F>,
    id: Uuid,
    content: &str,
) -> Result<(), WorkspaceError>
where
    F: RootFilesystem,
{
    let Some(user_id) = lookup_user_id_for_doc(store, id).await? else {
        return Err(WorkspaceError::DocumentNotFound {
            doc_type: id.to_string(),
            user_id: "<unknown>".to_string(),
        });
    };
    let Some(mut stored) = read_stored(store, &user_id, id).await? else {
        return Err(WorkspaceError::DocumentNotFound {
            doc_type: id.to_string(),
            user_id,
        });
    };
    stored.content = content.to_string();
    stored.updated_at = Utc::now();
    write_stored(store, &stored).await
}

pub(super) async fn delete_by_path<F>(
    store: &FilesystemWorkspaceStore<F>,
    user_id: &str,
    agent_id: Option<Uuid>,
    path: &str,
) -> Result<(), WorkspaceError>
where
    F: RootFilesystem,
{
    let Some(doc_id) = lookup_doc_id_by_path(store, user_id, agent_id, path).await? else {
        return Ok(());
    };
    let doc_path = paths::document_path(user_id, doc_id)?;
    let _ = store.filesystem.delete(&doc_path).await;
    let path_idx = paths::path_index_entry(user_id, agent_id, path)?;
    let _ = store.filesystem.delete(&path_idx).await;
    let id_idx = paths::id_index_path(doc_id)?;
    let _ = store.filesystem.delete(&id_idx).await;
    let chunks_root = paths::chunks_root_for_doc(doc_id)?;
    let _ = store.filesystem.delete(&chunks_root).await;
    let versions_root = paths::versions_root_for_doc(doc_id)?;
    let _ = store.filesystem.delete(&versions_root).await;
    Ok(())
}

pub(super) async fn list_documents<F>(
    store: &FilesystemWorkspaceStore<F>,
    user_id: &str,
    agent_id: Option<Uuid>,
) -> Result<Vec<MemoryDocument>, WorkspaceError>
where
    F: RootFilesystem,
{
    let root = paths::documents_root_for_user(user_id)?;
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
        let stored: StoredDocument = match serde_json::from_slice(&versioned.entry.body) {
            Ok(s) => s,
            Err(_) => continue,
        };
        if stored.user_id != user_id || stored.agent_id != agent_id {
            continue;
        }
        out.push(stored.to_document());
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

pub(super) async fn list_all_paths<F>(
    store: &FilesystemWorkspaceStore<F>,
    user_id: &str,
    agent_id: Option<Uuid>,
) -> Result<Vec<String>, WorkspaceError>
where
    F: RootFilesystem,
{
    let docs = list_documents(store, user_id, agent_id).await?;
    let mut out: Vec<String> = docs.into_iter().map(|d| d.path).collect();
    out.sort();
    out.dedup();
    Ok(out)
}

pub(super) async fn list_directory<F>(
    store: &FilesystemWorkspaceStore<F>,
    user_id: &str,
    agent_id: Option<Uuid>,
    directory: &str,
) -> Result<Vec<WorkspaceEntry>, WorkspaceError>
where
    F: RootFilesystem,
{
    let docs = list_documents(store, user_id, agent_id).await?;
    let prefix = if directory.is_empty() || directory == "/" {
        "".to_string()
    } else {
        format!("{}/", directory.trim_end_matches('/'))
    };
    // Group children of `directory` by their first segment.
    let mut direct_files: Vec<MemoryDocument> = Vec::new();
    let mut subdirs: HashMap<String, Option<chrono::DateTime<Utc>>> = HashMap::new();
    let mut seen_dirs: HashSet<String> = HashSet::new();
    for doc in docs {
        let rel = if prefix.is_empty() {
            doc.path.clone()
        } else if let Some(stripped) = doc.path.strip_prefix(&prefix) {
            stripped.to_string()
        } else {
            continue;
        };
        if rel.is_empty() {
            continue;
        }
        if let Some(idx) = rel.find('/') {
            let dir_name = rel[..idx].to_string();
            let dir_path = if prefix.is_empty() {
                dir_name.clone()
            } else {
                format!("{}{}", prefix, dir_name)
            };
            seen_dirs.insert(dir_path.clone());
            let entry_ts = subdirs.entry(dir_path).or_insert(None);
            *entry_ts = match (*entry_ts, Some(doc.updated_at)) {
                (Some(a), Some(b)) if b > a => Some(b),
                (None, b) => b,
                (a, _) => a,
            };
        } else {
            direct_files.push(doc);
        }
    }
    let mut out: Vec<WorkspaceEntry> = Vec::new();
    for (dir_path, updated_at) in subdirs {
        out.push(WorkspaceEntry {
            path: dir_path,
            is_directory: true,
            updated_at,
            content_preview: None,
        });
    }
    for doc in direct_files {
        let preview = if doc.content.is_empty() {
            None
        } else {
            Some(doc.content.chars().take(200).collect::<String>())
        };
        out.push(WorkspaceEntry {
            path: doc.path,
            is_directory: false,
            updated_at: Some(doc.updated_at),
            content_preview: preview,
        });
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    let _ = seen_dirs;
    Ok(out)
}

pub(super) async fn update_metadata<F>(
    store: &FilesystemWorkspaceStore<F>,
    id: Uuid,
    metadata: &serde_json::Value,
) -> Result<(), WorkspaceError>
where
    F: RootFilesystem,
{
    let Some(user_id) = lookup_user_id_for_doc(store, id).await? else {
        return Err(WorkspaceError::DocumentNotFound {
            doc_type: id.to_string(),
            user_id: "<unknown>".to_string(),
        });
    };
    let Some(mut stored) = read_stored(store, &user_id, id).await? else {
        return Err(WorkspaceError::DocumentNotFound {
            doc_type: id.to_string(),
            user_id,
        });
    };
    stored.metadata = metadata.clone();
    stored.updated_at = Utc::now();
    write_stored(store, &stored).await
}

pub(super) async fn find_config_documents<F>(
    store: &FilesystemWorkspaceStore<F>,
    user_id: &str,
    agent_id: Option<Uuid>,
) -> Result<Vec<MemoryDocument>, WorkspaceError>
where
    F: RootFilesystem,
{
    let docs = list_documents(store, user_id, agent_id).await?;
    Ok(docs
        .into_iter()
        .filter(|d| crate::workspace::is_config_path(&d.path))
        .collect())
}

fn serialization_error(error: serde_json::Error) -> WorkspaceError {
    WorkspaceError::SearchFailed {
        reason: format!("workspace document serialization: {error}"),
    }
}
