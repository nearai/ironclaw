//! Virtual-path helpers shared across the workspace facade sub-modules.

use ironclaw_host_api::VirtualPath;
use uuid::Uuid;

use crate::error::WorkspaceError;

const NONE_AGENT_SENTINEL: &str = "_none";

pub(super) fn encode_segment(value: &str) -> String {
    value
        .chars()
        .map(|c| match c {
            '/' | '\\' | '\0' | '\n' | '\r' | '\t' | ' ' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect()
}

pub(super) fn agent_id_segment(agent_id: Option<Uuid>) -> String {
    agent_id
        .map(|id| id.to_string())
        .unwrap_or_else(|| NONE_AGENT_SENTINEL.to_string())
}

pub(super) fn document_path(user_id: &str, doc_id: Uuid) -> Result<VirtualPath, WorkspaceError> {
    VirtualPath::new(format!(
        "/workspace/documents/{}/{}",
        encode_segment(user_id),
        doc_id
    ))
    .map_err(invalid_path)
}

pub(super) fn documents_root_for_user(user_id: &str) -> Result<VirtualPath, WorkspaceError> {
    VirtualPath::new(format!("/workspace/documents/{}", encode_segment(user_id)))
        .map_err(invalid_path)
}

#[allow(dead_code)]
pub(super) fn documents_root() -> Result<VirtualPath, WorkspaceError> {
    VirtualPath::new("/workspace/documents".to_string()).map_err(invalid_path)
}

pub(super) fn chunks_root_for_doc(doc_id: Uuid) -> Result<VirtualPath, WorkspaceError> {
    VirtualPath::new(format!("/workspace/chunks/{doc_id}")).map_err(invalid_path)
}

pub(super) fn chunks_root() -> Result<VirtualPath, WorkspaceError> {
    VirtualPath::new("/workspace/chunks".to_string()).map_err(invalid_path)
}

pub(super) fn chunk_path(doc_id: Uuid, chunk_index: i32) -> Result<VirtualPath, WorkspaceError> {
    if chunk_index < 0 {
        return Err(WorkspaceError::InvalidPath {
            path: format!("/workspace/chunks/{doc_id}/{chunk_index}"),
            reason: "chunk_index must be non-negative".to_string(),
        });
    }
    VirtualPath::new(format!("/workspace/chunks/{doc_id}/{chunk_index:010}")).map_err(invalid_path)
}

pub(super) fn versions_root_for_doc(doc_id: Uuid) -> Result<VirtualPath, WorkspaceError> {
    VirtualPath::new(format!("/workspace/versions/{doc_id}")).map_err(invalid_path)
}

pub(super) fn version_path(doc_id: Uuid, version: i32) -> Result<VirtualPath, WorkspaceError> {
    if version < 1 {
        return Err(WorkspaceError::VersionNotFound {
            document_id: doc_id,
            version,
        });
    }
    VirtualPath::new(format!("/workspace/versions/{doc_id}/{version:010}")).map_err(invalid_path)
}

#[allow(dead_code)]
pub(super) fn path_index_root_for_user_agent(
    user_id: &str,
    agent_id: Option<Uuid>,
) -> Result<VirtualPath, WorkspaceError> {
    VirtualPath::new(format!(
        "/workspace/path-index/{}/{}",
        encode_segment(user_id),
        agent_id_segment(agent_id)
    ))
    .map_err(invalid_path)
}

pub(super) fn path_index_entry(
    user_id: &str,
    agent_id: Option<Uuid>,
    doc_path: &str,
) -> Result<VirtualPath, WorkspaceError> {
    VirtualPath::new(format!(
        "/workspace/path-index/{}/{}/{}",
        encode_segment(user_id),
        agent_id_segment(agent_id),
        encode_segment(doc_path)
    ))
    .map_err(invalid_path)
}

pub(super) fn id_index_path(doc_id: Uuid) -> Result<VirtualPath, WorkspaceError> {
    VirtualPath::new(format!("/workspace/id-index/{doc_id}")).map_err(invalid_path)
}

fn invalid_path(err: ironclaw_host_api::HostApiError) -> WorkspaceError {
    WorkspaceError::InvalidPath {
        path: "<workspace>".to_string(),
        reason: err.to_string(),
    }
}
