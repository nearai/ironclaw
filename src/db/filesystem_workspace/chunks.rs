//! Chunk-level operations for the filesystem-backed workspace store.

use chrono::Utc;
use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, FilesystemError, IndexKey, IndexValue, RecordKind,
    RootFilesystem,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::WorkspaceError;
use crate::workspace::{ChunkWrite, MemoryChunk};

use super::paths;
use super::{FilesystemWorkspaceStore, fs_to_workspace_error};

const CHUNK_KIND: &str = "memory_chunk";

pub(super) mod fs_keys {
    pub const DOCUMENT_ID: &str = "document_id";
    pub const USER_ID: &str = "user_id";
    pub const AGENT_ID: &str = "agent_id";
    pub const CHUNK_INDEX: &str = "chunk_index";
    pub const CONTENT: &str = "content";
    pub const EMBEDDING: &str = "embedding";
    pub const HAS_EMBEDDING: &str = "has_embedding";
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct StoredChunk {
    pub id: Uuid,
    pub document_id: Uuid,
    pub user_id: String,
    pub agent_id: Option<Uuid>,
    pub chunk_index: i32,
    pub content: String,
    pub embedding: Option<Vec<f32>>,
    pub created_at: chrono::DateTime<Utc>,
}

impl StoredChunk {
    pub(super) fn to_memory_chunk(&self) -> MemoryChunk {
        MemoryChunk {
            id: self.id,
            document_id: self.document_id,
            chunk_index: self.chunk_index,
            content: self.content.clone(),
            embedding: self.embedding.clone(),
            created_at: self.created_at,
        }
    }
}

pub(super) fn build_chunk_entry(chunk: &StoredChunk) -> Result<Entry, WorkspaceError> {
    let body = serde_json::to_vec(chunk).map_err(serialization_error)?;
    let kind = RecordKind::new(CHUNK_KIND).map_err(|e| WorkspaceError::SearchFailed {
        reason: e.to_string(),
    })?;
    let mut entry = Entry::bytes(body).with_content_type(ContentType::json());
    entry.kind = Some(kind);
    let k_doc = IndexKey::new(fs_keys::DOCUMENT_ID).map_err(|e| WorkspaceError::SearchFailed {
        reason: e.to_string(),
    })?;
    let k_user = IndexKey::new(fs_keys::USER_ID).map_err(|e| WorkspaceError::SearchFailed {
        reason: e.to_string(),
    })?;
    let k_agent = IndexKey::new(fs_keys::AGENT_ID).map_err(|e| WorkspaceError::SearchFailed {
        reason: e.to_string(),
    })?;
    let k_idx = IndexKey::new(fs_keys::CHUNK_INDEX).map_err(|e| WorkspaceError::SearchFailed {
        reason: e.to_string(),
    })?;
    let k_content = IndexKey::new(fs_keys::CONTENT).map_err(|e| WorkspaceError::SearchFailed {
        reason: e.to_string(),
    })?;
    let k_has_emb =
        IndexKey::new(fs_keys::HAS_EMBEDDING).map_err(|e| WorkspaceError::SearchFailed {
            reason: e.to_string(),
        })?;
    entry = entry
        .with_indexed(k_doc, IndexValue::Text(chunk.document_id.to_string()))
        .with_indexed(k_user, IndexValue::Text(chunk.user_id.clone()))
        .with_indexed(
            k_agent,
            IndexValue::Text(paths::agent_id_segment(chunk.agent_id)),
        )
        .with_indexed(k_idx, IndexValue::I64(chunk.chunk_index as i64))
        .with_indexed(k_content, IndexValue::Text(chunk.content.clone()))
        .with_indexed(k_has_emb, IndexValue::Bool(chunk.embedding.is_some()));
    if let Some(emb) = &chunk.embedding {
        let k_emb =
            IndexKey::new(fs_keys::EMBEDDING).map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let bytes = bytemuck_pod_cast(emb);
        entry = entry.with_indexed(k_emb, IndexValue::Bytes(bytes));
    }
    Ok(entry)
}

fn bytemuck_pod_cast(values: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * 4);
    for value in values {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

pub(super) async fn read_chunks_for_doc<F>(
    store: &FilesystemWorkspaceStore<F>,
    document_id: Uuid,
) -> Result<Vec<StoredChunk>, WorkspaceError>
where
    F: RootFilesystem,
{
    let root = paths::chunks_root_for_doc(document_id)?;
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
        if let Ok(chunk) = serde_json::from_slice::<StoredChunk>(&versioned.entry.body) {
            out.push(chunk);
        }
    }
    out.sort_by_key(|c| c.chunk_index);
    Ok(out)
}

pub(super) async fn delete_all<F>(
    store: &FilesystemWorkspaceStore<F>,
    document_id: Uuid,
) -> Result<(), WorkspaceError>
where
    F: RootFilesystem,
{
    let root = paths::chunks_root_for_doc(document_id)?;
    match store.filesystem.delete(&root).await {
        Ok(()) => Ok(()),
        Err(FilesystemError::NotFound { .. }) => Ok(()),
        Err(error) => Err(fs_to_workspace_error(error)),
    }
}

pub(super) async fn insert<F>(
    store: &FilesystemWorkspaceStore<F>,
    document_id: Uuid,
    chunk_index: i32,
    content: &str,
    embedding: Option<&[f32]>,
) -> Result<Uuid, WorkspaceError>
where
    F: RootFilesystem,
{
    // Resolve document so we can copy user_id/agent_id into the chunk for
    // user-scoped queries. This costs one extra `get`; in the SQL backends
    // the chunk row carried a FK to memory_documents and the scope was
    // recovered via join.
    let doc = super::documents::get_by_id(store, document_id).await?;
    let id = Uuid::new_v4();
    let chunk = super::chunks::StoredChunk {
        id,
        document_id,
        user_id: doc.user_id,
        agent_id: doc.agent_id,
        chunk_index,
        content: content.to_string(),
        embedding: embedding.map(|e| e.to_vec()),
        created_at: Utc::now(),
    };
    let path = paths::chunk_path(document_id, chunk_index)?;
    let entry = build_chunk_entry(&chunk)?;
    store
        .filesystem
        .put(&path, entry, CasExpectation::Any)
        .await
        .map(|_| ())
        .map_err(fs_to_workspace_error)?;
    Ok(id)
}

pub(super) async fn replace_all<F>(
    store: &FilesystemWorkspaceStore<F>,
    document_id: Uuid,
    chunks_in: &[ChunkWrite],
) -> Result<(), WorkspaceError>
where
    F: RootFilesystem,
{
    delete_all(store, document_id).await?;
    if chunks_in.is_empty() {
        return Ok(());
    }
    for (idx, chunk) in chunks_in.iter().enumerate() {
        insert(
            store,
            document_id,
            idx as i32,
            &chunk.content,
            chunk.embedding.as_deref(),
        )
        .await?;
    }
    Ok(())
}

pub(super) async fn update_embedding<F>(
    store: &FilesystemWorkspaceStore<F>,
    chunk_id: Uuid,
    embedding: &[f32],
) -> Result<(), WorkspaceError>
where
    F: RootFilesystem,
{
    // We don't index chunks by chunk_id directly; we have to find the
    // chunk. The legacy SQL backed by `id` did one-row lookup; here we
    // accept O(n) over all chunks in the matching document path tree.
    // For deployments with many chunks, the FS backends translate the
    // `Filter::Eq` on the `chunk_id` indexed projection into a native
    // single-key lookup — but this trait method takes only the chunk_id,
    // not the document_id, so we fall back to scanning every chunk dir
    // under `/workspace/chunks`.
    let chunks_root = ironclaw_host_api::VirtualPath::new("/workspace/chunks".to_string())
        .map_err(|e| WorkspaceError::InvalidPath {
            path: "/workspace/chunks".to_string(),
            reason: e.to_string(),
        })?;
    let doc_dirs = match store.filesystem.list_dir(&chunks_root).await {
        Ok(entries) => entries,
        Err(FilesystemError::NotFound { .. }) => return Ok(()),
        Err(error) => return Err(fs_to_workspace_error(error)),
    };
    for doc_dir in doc_dirs {
        let entries = match store.filesystem.list_dir(&doc_dir.path).await {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries {
            let Some(versioned) = store
                .filesystem
                .get(&entry.path)
                .await
                .map_err(fs_to_workspace_error)?
            else {
                continue;
            };
            let mut chunk: StoredChunk = match serde_json::from_slice(&versioned.entry.body) {
                Ok(c) => c,
                Err(_) => continue,
            };
            if chunk.id != chunk_id {
                continue;
            }
            chunk.embedding = Some(embedding.to_vec());
            let new_entry = build_chunk_entry(&chunk)?;
            store
                .filesystem
                .put(&entry.path, new_entry, CasExpectation::Any)
                .await
                .map(|_| ())
                .map_err(fs_to_workspace_error)?;
            return Ok(());
        }
    }
    Ok(())
}

pub(super) async fn list_without_embeddings<F>(
    store: &FilesystemWorkspaceStore<F>,
    user_id: &str,
    agent_id: Option<Uuid>,
    limit: usize,
) -> Result<Vec<MemoryChunk>, WorkspaceError>
where
    F: RootFilesystem,
{
    let chunks_root = ironclaw_host_api::VirtualPath::new("/workspace/chunks".to_string())
        .map_err(|e| WorkspaceError::InvalidPath {
            path: "/workspace/chunks".to_string(),
            reason: e.to_string(),
        })?;
    let doc_dirs = match store.filesystem.list_dir(&chunks_root).await {
        Ok(entries) => entries,
        Err(FilesystemError::NotFound { .. }) => return Ok(Vec::new()),
        Err(error) => return Err(fs_to_workspace_error(error)),
    };
    let mut out = Vec::new();
    'outer: for doc_dir in doc_dirs {
        let entries = match store.filesystem.list_dir(&doc_dir.path).await {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries {
            if out.len() >= limit {
                break 'outer;
            }
            let Some(versioned) = store
                .filesystem
                .get(&entry.path)
                .await
                .map_err(fs_to_workspace_error)?
            else {
                continue;
            };
            let chunk: StoredChunk = match serde_json::from_slice(&versioned.entry.body) {
                Ok(c) => c,
                Err(_) => continue,
            };
            if chunk.user_id != user_id || chunk.agent_id != agent_id {
                continue;
            }
            if chunk.embedding.is_some() {
                continue;
            }
            out.push(chunk.to_memory_chunk());
        }
    }
    Ok(out)
}

fn serialization_error(error: serde_json::Error) -> WorkspaceError {
    WorkspaceError::SearchFailed {
        reason: format!("workspace chunk serialization: {error}"),
    }
}
