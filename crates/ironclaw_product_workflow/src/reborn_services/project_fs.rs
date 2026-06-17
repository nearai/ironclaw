//! Generic project-filesystem read port for the WebUI v2 facade.
//!
//! Surfaces a thread's project workspace (the same `/workspace` mount the
//! agent's file tools and inbound-attachment landing resolve through) as a
//! read-only navigation + download API: list a directory, stat a path, and
//! read a file's bytes. The download side is what makes agent-produced
//! attachments retrievable — an [`AttachmentRef`](crate::AttachmentRef)'s
//! `storage_key` is exactly the scoped path these methods accept — but the port
//! itself knows nothing about attachments and is reusable for a future file
//! browser.
//!
//! The port is injected by host composition, which owns the project-scoped
//! filesystem authority. The facade verifies the caller owns the thread before
//! calling the port and hands it a [`ThreadScope`] derived from the
//! authenticated caller; the port never sees raw request identity. Paths in and
//! out are scoped paths (`/workspace/...`) — never host or virtual paths.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use ironclaw_threads::ThreadScope;

/// Coarse filesystem entry kind exposed to product/WebUI consumers.
///
/// Mirrors `ironclaw_filesystem::FileType` without depending on that crate so
/// the product boundary stays free of substrate types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectFsEntryKind {
    File,
    Directory,
    Symlink,
    Other,
}

/// A single entry in a project directory listing.
///
/// `path` is the scoped path (`/workspace/...`) the consumer passes back to
/// [`ProjectFilesystemReader::read_file`] / [`ProjectFilesystemReader::stat`] —
/// reconstructed by the implementation from the request directory plus the
/// entry name so a host or virtual path is never serialized.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectFsEntry {
    pub name: String,
    pub path: String,
    pub kind: ProjectFsEntryKind,
}

/// Metadata for a single scoped project path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectFsStat {
    pub path: String,
    pub kind: ProjectFsEntryKind,
    pub size_bytes: u64,
    /// Best-effort MIME type derived from the path extension — mirrors the
    /// download `Content-Type`. Lets the WebUI choose a preview representation
    /// (image/pdf/text/…) before fetching the bytes. `application/octet-stream`
    /// when the extension is unknown.
    pub mime_type: String,
}

/// Materialized file bytes plus the metadata a download response needs.
///
/// Not `Serialize`: the bytes are streamed as the HTTP body, never embedded in
/// a JSON envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectFsFile {
    pub path: String,
    pub filename: Option<String>,
    pub mime_type: String,
    pub size_bytes: u64,
    pub bytes: Vec<u8>,
}

/// Errors a project-filesystem read may produce.
///
/// Deliberately coarse and free of host paths / backend strings: the facade
/// maps each variant to a sanitized [`RebornServicesError`](crate::RebornServicesError)
/// at the boundary. Implementations outside this crate construct these instead
/// of reaching for the facade error's `pub(super)` constructors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProjectFsError {
    #[error("path not found")]
    NotFound,
    #[error("path is not a regular file")]
    NotAFile,
    #[error("path is not a directory")]
    NotADirectory,
    #[error("path is not permitted")]
    Denied,
    #[error("invalid path")]
    InvalidPath,
    #[error("file exceeds the maximum readable size")]
    TooLarge { size: u64, max: u64 },
    #[error("project filesystem temporarily unavailable")]
    Unavailable,
    #[error("internal project filesystem error")]
    Internal,
}

/// Request to list a directory under a thread's project workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornProjectFsListRequest {
    pub thread_id: String,
    pub path: String,
}

/// Directory listing response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornProjectFsListResponse {
    pub entries: Vec<ProjectFsEntry>,
}

/// Request to stat a path under a thread's project workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornProjectFsStatRequest {
    pub thread_id: String,
    pub path: String,
}

/// Path metadata response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornProjectFsStatResponse {
    pub stat: ProjectFsStat,
}

/// Request to read (download) a file under a thread's project workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornProjectFsReadRequest {
    pub thread_id: String,
    pub path: String,
}

/// Read-only access to a thread's project workspace filesystem.
///
/// Every method takes a [`ThreadScope`] the facade has already authorized and a
/// scoped path; mutations are intentionally absent (this is a navigation +
/// download surface, not a write surface).
#[async_trait]
pub trait ProjectFilesystemReader: Send + Sync {
    /// List the entries directly under `path` (a directory).
    async fn list_dir(
        &self,
        thread_scope: &ThreadScope,
        path: &str,
    ) -> Result<Vec<ProjectFsEntry>, ProjectFsError>;

    /// Read the bytes of the regular file at `path`, with its metadata.
    async fn read_file(
        &self,
        thread_scope: &ThreadScope,
        path: &str,
    ) -> Result<ProjectFsFile, ProjectFsError>;

    /// Return metadata for `path` without reading its bytes.
    async fn stat(
        &self,
        thread_scope: &ThreadScope,
        path: &str,
    ) -> Result<ProjectFsStat, ProjectFsError>;
}
