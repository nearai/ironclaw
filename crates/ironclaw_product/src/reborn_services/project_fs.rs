//! Generic project-filesystem read port for the WebUI v2 service.
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
//! filesystem authority. The service verifies the caller owns the thread before
//! calling the port and hands it a [`ThreadScope`] derived from the
//! authenticated caller; the port never sees raw request identity. Paths in and
//! out are scoped paths (`/workspace/...`) — never host or virtual paths.

use async_trait::async_trait;
use ironclaw_host_api::attachment::WorkspaceFile;

use ironclaw_threads::ThreadScope;

pub use ironclaw_product_contracts::workspace_views::{
    ProjectFsEntry, ProjectFsEntryKind, ProjectFsError, ProjectFsFile, ProjectFsStat,
    RebornProjectFsListRequest, RebornProjectFsListResponse, RebornProjectFsReadRequest,
    RebornProjectFsStatRequest, RebornProjectFsStatResponse,
};

/// Read-only access to a thread's project workspace filesystem.
///
/// Every method takes a [`ThreadScope`] the service has already authorized and a
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
    ) -> Result<WorkspaceFile, ProjectFsError>;

    /// Return metadata for `path` without reading its bytes.
    async fn stat(
        &self,
        thread_scope: &ThreadScope,
        path: &str,
    ) -> Result<ProjectFsStat, ProjectFsError>;
}
