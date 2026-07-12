//! Generic multi-mount filesystem-browse read port for the WebUI v2 facade.
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
//! - **Mount, not thread.** The browse scope is derived by the facade from the
//!   authenticated caller (tenant/user/agent/project), never from a thread id or
//!   the request body. A [`FsMount`] selects *which* virtual mount to read;
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
use ironclaw_host_api::ResourceScope;
use serde::{Deserialize, Serialize};

use super::project_fs::{ProjectFsEntry, ProjectFsError, ProjectFsFile, ProjectFsStat};

/// A logical, browsable filesystem mount exposed by the read-only file viewer.
///
/// Deliberately a small logical enum: the concrete alias (`/memory`,
/// `/workspace`, …) and physical target are composition concerns and never
/// cross this product boundary. New mounts (e.g. a future engine-internals or
/// secrets-metadata surface) extend this enum; the wire form is the stable
/// snake_case discriminant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FsMount {
    /// Persistent memory store (identity files, daily logs, curated memory).
    Memory,
    /// Project working directory the agent's file tools read/write, including
    /// agent-produced and landed attachment files.
    Workspace,
    /// Installed and user-placed skills.
    Skills,
}

impl FsMount {
    /// All mounts known to the product layer, in display order. Which of these
    /// a given deployment actually serves is reported by
    /// [`FilesystemBrowseReader::available_mounts`] — a mount may be known here
    /// but unwired in a particular composition.
    pub const ALL: &'static [FsMount] = &[FsMount::Memory, FsMount::Workspace, FsMount::Skills];

    /// Stable, human-facing default label. The frontend may localize via its
    /// own i18n; this is the server-side fallback.
    pub fn label(self) -> &'static str {
        match self {
            FsMount::Memory => "Memory",
            FsMount::Workspace => "Workspace files",
            FsMount::Skills => "Skills",
        }
    }
}

/// Metadata describing one browsable mount for the WebUI mount picker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornFsMountInfo {
    pub mount: FsMount,
    pub label: String,
}

/// Response listing the mounts this deployment can browse.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornFsMountsResponse {
    pub mounts: Vec<RebornFsMountInfo>,
}

/// Request to list a directory under a browsable mount.
///
/// `path` is mount-relative (`""` or `"/"` for the mount root). The
/// implementation composes the concrete scoped path from the mount alias plus
/// this value; the browser never supplies an alias or host path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornFsListRequest {
    pub mount: FsMount,
    #[serde(default)]
    pub path: String,
}

/// Directory listing response. Echoes the requested `mount`/`path` so the
/// browser can reconcile out-of-order responses, and carries mount-relative
/// entry paths.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornFsListResponse {
    pub mount: FsMount,
    pub path: String,
    pub entries: Vec<ProjectFsEntry>,
}

/// Request to stat a path under a browsable mount.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornFsStatRequest {
    pub mount: FsMount,
    #[serde(default)]
    pub path: String,
}

/// Path metadata response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornFsStatResponse {
    pub stat: ProjectFsStat,
}

/// Request to read (preview/download) a file under a browsable mount.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornFsReadRequest {
    pub mount: FsMount,
    #[serde(default)]
    pub path: String,
}

/// Read-only navigation + download access to the agent's internal filesystem
/// across multiple logical mounts.
///
/// Every method takes a [`ResourceScope`] the facade has already derived from
/// the authenticated caller and authorized; mutations are intentionally absent.
/// Entry/stat paths are mount-relative — the same value passes back to
/// [`Self::read_file`]/[`Self::stat`].
#[async_trait]
pub trait FilesystemBrowseReader: Send + Sync {
    /// The mounts this composition can actually serve. The facade filters
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
