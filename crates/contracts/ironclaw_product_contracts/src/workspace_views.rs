//! Project and filesystem-browse wire vocabulary (PROPOSAL §6.1.3, "product
//! wire DTO homes").
//!
//! The WebUI Projects page and the read-only Workspace/Files explorer both
//! serialize these. Of the three ports that serve them, one is now here and two
//! are still in `ironclaw_assistant`, for reasons worth keeping straight:
//!
//! - `ProjectService` **moved here** — [`project_service`](crate::project_service),
//!   hoisted 2026-08-05 by PROPOSAL §12.13 D-P. Its implementing adapter sits
//!   *below* product (`ironclaw_identity::projects`), which a `products`-tier
//!   declaration made unplaceable; that is the dependency-edge removal the
//!   earlier text here said the hoist did not buy.
//! - `ProjectFilesystemReader` **cannot** move — every method takes an
//!   `ironclaw_threads::ThreadScope`, outside this crate's allowlist.
//! - `FilesystemBrowseReader` merely **has not**: §6.1.3's port list does not
//!   name it, and its implementor (`ironclaw_composition`) stays put, so
//!   hoisting it removes no edge. D-P deliberately did not take it — see the
//!   §6.10.1 measurement of what the browse reader would drag with it.
//!
//! Never here: a repository, a path resolver, or any mount alias.

use chrono::{DateTime, Utc};
use ironclaw_host_api::ids::{ProjectId, TenantId, UserId};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

/// Trusted caller identity for project operations.
///
/// Built by the service from the authenticated caller. Never reconstructed from
/// the request body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectCaller {
    pub tenant_id: TenantId,
    pub user_id: UserId,
}

/// Access role a user holds on a project. Privilege order, highest first:
/// `Owner > Editor > Viewer` (matches the variant declaration order).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RebornProjectRole {
    Owner,
    Editor,
    Viewer,
}

/// Lifecycle state of a project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RebornProjectState {
    Active,
    Archived,
}

/// Membership grant status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RebornProjectMemberStatus {
    Active,
    Revoked,
}

/// Sanitized project view returned to the WebUI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornProjectInfo {
    pub project_id: String,
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Extensible bag (goals, GitHub links, …).
    pub metadata: JsonValue,
    pub state: RebornProjectState,
    /// The calling user's effective role on this project.
    pub role: RebornProjectRole,
    /// RFC3339 on the wire (serde-serialized `DateTime<Utc>`); typed here to
    /// match the other WebUI service DTOs rather than an ambiguous `String`.
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Sanitized membership grant view returned to the WebUI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornProjectMemberInfo {
    pub user_id: String,
    pub role: RebornProjectRole,
    pub status: RebornProjectMemberStatus,
    pub granted_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Browser body for listing the caller's projects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RebornListProjectsRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// List response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornListProjectsResponse {
    pub projects: Vec<RebornProjectInfo>,
    #[serde(default)]
    pub total_projects: usize,
    #[serde(default)]
    pub active_projects: usize,
    #[serde(default)]
    pub archived_projects: usize,
}

impl RebornListProjectsResponse {
    pub fn new(
        projects: Vec<RebornProjectInfo>,
        total_projects: usize,
        active_projects: usize,
        archived_projects: usize,
    ) -> Self {
        Self {
            projects,
            total_projects,
            active_projects,
            archived_projects,
        }
    }
}

/// Single-project response (create / get / update).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornProjectResponse {
    pub project: RebornProjectInfo,
}

/// Browser body for creating a project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornCreateProjectRequest {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<JsonValue>,
}

/// Path/body for fetching a single project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornGetProjectRequest {
    pub project_id: String,
}

/// Browser body for updating a project. Absent fields are left unchanged.
///
/// `project_id` is supplied by the route path; the handler overrides any body
/// value, so it carries `#[serde(default)]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornUpdateProjectRequest {
    #[serde(default)]
    pub project_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<JsonValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<RebornProjectState>,
}

/// Path/body for deleting a project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornDeleteProjectRequest {
    pub project_id: String,
}

/// Path/body for listing a project's members.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornListMembersRequest {
    pub project_id: String,
}

/// Members list response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornListMembersResponse {
    pub members: Vec<RebornProjectMemberInfo>,
}

/// Browser body for granting a project member a role.
///
/// `project_id` comes from the route path (handler-overridden).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornAddMemberRequest {
    #[serde(default)]
    pub project_id: String,
    pub user_id: String,
    pub role: RebornProjectRole,
}

/// Browser body for changing a member's role.
///
/// `project_id` and `user_id` come from the route path (handler-overridden).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornUpdateMemberRoleRequest {
    #[serde(default)]
    pub project_id: String,
    #[serde(default)]
    pub user_id: String,
    pub role: RebornProjectRole,
}

/// Browser body for revoking a member.
///
/// `project_id` and `user_id` come from the route path (handler-overridden), so
/// both carry `#[serde(default)]` like [`RebornUpdateMemberRoleRequest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornRemoveMemberRequest {
    #[serde(default)]
    pub project_id: String,
    #[serde(default)]
    pub user_id: String,
}

/// Coarse filesystem entry kind exposed to product/WebUI consumers.
///
/// A wire projection of `ironclaw_filesystem::FileType`: this one is
/// serialized into product/WebUI responses and must stay stable independently
/// of the substrate enum, which is free to grow variants.
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
/// `ProjectFilesystemReader::read_file` / `::stat` — the port stayed in
/// `ironclaw_assistant::reborn_services::project_fs` because every method takes
/// an `ironclaw_threads::ThreadScope`, so these are code spans, not links. The
/// path is reconstructed by the implementation from the request directory plus
/// the entry name so a host or virtual path is never serialized.
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
/// Product-surface commands carry this through the host JSON envelope before
/// WebUI streams the bytes as the HTTP body.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectFsFile {
    pub path: String,
    pub filename: Option<String>,
    pub mime_type: String,
    pub size_bytes: u64,
    pub bytes: Vec<u8>,
}

/// Renders the byte length, never the bytes. This is the wire type crossing
/// the product-surface command envelope, so a derived `Debug` would put whole
/// user files into any diagnostic that formats a command.
impl std::fmt::Debug for ProjectFsFile {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProjectFsFile")
            .field("path", &self.path)
            .field("filename", &self.filename)
            .field("mime_type", &self.mime_type)
            .field("size_bytes", &self.size_bytes)
            .finish()
    }
}

/// Errors a project-filesystem read may produce.
///
/// Deliberately coarse and free of host paths / backend strings: the service
/// maps each variant to a sanitized [`ProductSurfaceError`](crate::surface::ProductSurfaceError)
/// at the boundary. Implementations outside this crate construct these instead
/// of reaching for the service error's `pub(super)` constructors.
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
    /// `FilesystemBrowseReader::available_mounts` (the port stayed in
    /// `ironclaw_assistant::reborn_services::fs_browse`) — a mount may be known
    /// here but unwired in a particular composition.
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

/// Request to list browsable mounts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornFsMountsRequest {}

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
    /// Optional project selector. When absent, the authenticated caller's
    /// default project scope is preserved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<ProjectId>,
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
    /// Optional project selector. The service authorizes it before resolving
    /// the browse scope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<ProjectId>,
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
    /// Optional project selector. The service authorizes it before resolving
    /// the browse scope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<ProjectId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `ProjectFsFile` hand-writes `Debug` for one reason: it crosses the
    /// product-surface command envelope carrying whole user files, and a
    /// derived `Debug` would put those bytes into any diagnostic that formats
    /// a command. Adding `#[derive(Debug)]` back compiles fine and silently
    /// re-opens that leak, so the redaction is pinned here.
    #[test]
    fn project_fs_file_debug_reports_the_length_and_never_the_bytes() {
        let file = ProjectFsFile {
            path: "/workspace/report.txt".to_string(),
            filename: Some("report.txt".to_string()),
            mime_type: "text/plain".to_string(),
            size_bytes: 6,
            bytes: b"SECRET".to_vec(),
        };

        let rendered = format!("{file:?}");
        assert!(
            !rendered.contains("SECRET"),
            "file contents must never reach a diagnostic: {rendered}"
        );
        assert!(
            !rendered.contains(" bytes:"),
            "the `bytes` field itself must be absent; only `size_bytes` may \
             appear: {rendered}"
        );
        assert!(
            rendered.contains("size_bytes: 6"),
            "length kept: {rendered}"
        );
        assert!(
            rendered.contains("/workspace/report.txt") && rendered.contains("text/plain"),
            "the diagnostic must stay useful: {rendered}"
        );
    }

    /// `FsMount::ALL` drives the WebUI mount picker and is a hand-maintained
    /// const slice, so the compiler cannot tell us when a new variant is added
    /// without one. A missing entry makes a real mount invisible in the file
    /// explorer with no error anywhere.
    #[test]
    fn every_mount_is_listed_once_in_all_and_carries_a_distinct_label() {
        for mount in [FsMount::Memory, FsMount::Workspace, FsMount::Skills] {
            assert!(
                FsMount::ALL.contains(&mount),
                "{mount:?} is missing from FsMount::ALL"
            );
        }
        assert_eq!(FsMount::ALL.len(), 3, "ALL must not carry duplicates");

        let labels: Vec<&str> = FsMount::ALL.iter().map(|mount| mount.label()).collect();
        let mut unique = labels.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), labels.len(), "labels collide: {labels:?}");
        assert!(
            labels.iter().all(|label| !label.is_empty()),
            "a blank label renders an unselectable row: {labels:?}"
        );
    }

    /// These discriminants are the browser's wire vocabulary; a rename is a
    /// silent break for every deployed client, so the encodings are pinned
    /// where they are defined rather than only where they are produced.
    #[test]
    fn mount_role_state_and_entry_kind_keep_their_wire_forms() {
        assert_eq!(
            serde_json::to_value(FsMount::Workspace).expect("serialize"),
            serde_json::json!("workspace")
        );
        assert_eq!(
            serde_json::to_value(RebornProjectRole::Owner).expect("serialize"),
            serde_json::json!("owner")
        );
        assert_eq!(
            serde_json::to_value(RebornProjectState::Active).expect("serialize"),
            serde_json::json!("active")
        );
        assert_eq!(
            serde_json::to_value(RebornProjectMemberStatus::Active).expect("serialize"),
            serde_json::json!("active")
        );
        assert_eq!(
            serde_json::to_value(ProjectFsEntryKind::Directory).expect("serialize"),
            serde_json::json!("directory")
        );
    }

    /// The error taxonomy is the whole contract an out-of-crate implementor
    /// has: it must describe a failure without leaking a host path or a
    /// backend string, and `TooLarge` must keep the two numbers a caller needs
    /// to explain the refusal.
    #[test]
    fn project_fs_errors_render_coarse_reasons_and_keep_the_size_ceiling() {
        assert_eq!(ProjectFsError::NotFound.to_string(), "path not found");
        assert_eq!(ProjectFsError::Denied.to_string(), "path is not permitted");
        let too_large = ProjectFsError::TooLarge { size: 42, max: 16 };
        assert_eq!(
            too_large.to_string(),
            "file exceeds the maximum readable size"
        );
        assert!(
            matches!(too_large, ProjectFsError::TooLarge { size: 42, max: 16 }),
            "the ceiling and the actual size must both survive the boundary"
        );
    }
}
