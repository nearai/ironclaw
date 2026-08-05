//! Generic multi-mount filesystem-browse read port for the WebUI v2 service.
//!
//! Where [`ProjectFilesystemReader`](super::project_fs::ProjectFilesystemReader)
//! surfaces a single thread's project workspace, this port surfaces the agent's
//! internal filesystem as a standalone, caller-scoped, **read-only** explorer:
//! the persistent memory store, the project working directory (which also holds
//! agent-produced attachments), and the skills tree. It is the backend the
//! WebUI "Workspace / Files" page navigates.
//!
//! Design notes:
//!
//! - **Mount, not thread.** The browse scope is derived by the service from the
//!   authenticated caller (tenant/user/agent/project). An optional typed
//!   project selector is authorized through the project service before it is
//!   adopted; it never directly supplies a filesystem scope. A [`FsMount`] selects *which* virtual mount to read;
//!   paths are mount-relative (`""`/`"/"` is the mount root) so a host or
//!   virtual path is never serialized across the boundary.
//! - **Substrate-free.** Like the project-fs port, this re-uses the coarse
//!   [`ProjectFsEntry`]/[`ProjectFsStat`]/[`ProjectFsFile`]/[`ProjectFsError`]
//!   shapes and knows nothing about `ironclaw_filesystem`. The alias→target
//!   mapping, path confinement, and sensitive-name filtering live in the host
//!   composition impl.
//! - **Read-only.** No `put`/`write` — this is a navigation + preview/download
//!   surface only. The agent's own tools and the memory write tools remain the
//!   sole mutation path.

use async_trait::async_trait;
use ironclaw_host_api::resource::ResourceScope;

use ironclaw_product_contracts::workspace_views::{
    ProjectFsEntry, ProjectFsError, ProjectFsFile, ProjectFsStat,
};

pub use ironclaw_product_contracts::workspace_views::{
    FsMount, RebornFsListRequest, RebornFsListResponse, RebornFsMountInfo, RebornFsMountsRequest,
    RebornFsMountsResponse, RebornFsReadRequest, RebornFsStatRequest, RebornFsStatResponse,
};

/// Read-only navigation + download access to the agent's internal filesystem
/// across multiple logical mounts.
///
/// Every method takes a [`ResourceScope`] the service has already derived from
/// the authenticated caller and authorized; mutations are intentionally absent.
/// Entry/stat paths are mount-relative — the same value passes back to
/// [`Self::read_file`]/[`Self::stat`].
#[async_trait]
pub trait FilesystemBrowseReader: Send + Sync {
    /// The mounts this composition can actually serve. The service filters
    /// requests against this set so an unwired mount yields a clean
    /// "not found" rather than a backend error.
    fn available_mounts(&self) -> Vec<FsMount>;

    /// List the entries directly under `path` (mount-relative) on `mount`.
    async fn list_dir(
        &self,
        scope: &ResourceScope,
        mount: FsMount,
        path: &str,
    ) -> Result<Vec<ProjectFsEntry>, ProjectFsError>;

    /// Read the bytes of the regular file at `path` on `mount`, with metadata.
    async fn read_file(
        &self,
        scope: &ResourceScope,
        mount: FsMount,
        path: &str,
    ) -> Result<ProjectFsFile, ProjectFsError>;

    /// Return metadata for `path` on `mount` without reading its bytes.
    async fn stat(
        &self,
        scope: &ResourceScope,
        mount: FsMount,
        path: &str,
    ) -> Result<ProjectFsStat, ProjectFsError>;
}
