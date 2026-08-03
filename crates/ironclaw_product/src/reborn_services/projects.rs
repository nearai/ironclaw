//! Project management port for the WebUI v2 service.
//!
//! Surfaces first-class projects — create, list, read, update, delete — plus
//! their membership grants (the ACL surface). The port is injected by host
//! composition, which owns the durable [`ProjectRepository`] and performs
//! access-control gating (owner/role checks via the repository's
//! `resolve_access`) before any mutation.
//!
//! Identity is authority-bearing: the service derives [`ProjectCaller`] from the
//! authenticated caller (tenant + user), never from the request body. Roles and
//! states are product-level enums here so this boundary stays free of the
//! `ironclaw_projects` substrate types — the composition adapter maps between
//! the two (mirrors how [`ProjectFsEntryKind`](super::ProjectFsEntryKind) shadows
//! `ironclaw_filesystem::FileType`).

use async_trait::async_trait;

pub use ironclaw_product_contracts::workspace_views::{
    ProjectCaller, RebornAddMemberRequest, RebornCreateProjectRequest, RebornDeleteProjectRequest,
    RebornGetProjectRequest, RebornListMembersRequest, RebornListMembersResponse,
    RebornListProjectsRequest, RebornListProjectsResponse, RebornProjectInfo,
    RebornProjectMemberInfo, RebornProjectMemberStatus, RebornProjectResponse, RebornProjectRole,
    RebornProjectState, RebornRemoveMemberRequest, RebornUpdateMemberRoleRequest,
    RebornUpdateProjectRequest,
};

/// Errors a project operation may produce.
///
/// Deliberately coarse and free of host paths / backend strings: the service
/// maps each variant to a sanitized [`ProductSurfaceError`](crate::ProductSurfaceError)
/// at the boundary. Implementations outside this crate construct these instead
/// of reaching for the service error's `pub(super)` constructors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProjectServiceError {
    #[error("project not found")]
    NotFound,
    #[error("caller is not permitted to perform this project operation")]
    Denied,
    #[error("invalid project input: {field}")]
    InvalidInput { field: String },
    #[error("project already exists")]
    Conflict,
    #[error("project service temporarily unavailable")]
    Unavailable,
    #[error("internal project service error")]
    Internal,
}

/// Project management + membership (ACL) port.
///
/// Every method takes a [`ProjectCaller`] the service derived from the
/// authenticated caller. Implementations are responsible for access-control
/// gating (owner/role checks) before mutating writes; reads return only
/// projects the caller can access.
#[async_trait]
pub trait ProjectService: Send + Sync {
    /// List projects the caller can access, most recently created first.
    async fn list_projects(
        &self,
        caller: ProjectCaller,
        request: RebornListProjectsRequest,
    ) -> Result<RebornListProjectsResponse, ProjectServiceError>;

    /// Create a project owned by the caller.
    async fn create_project(
        &self,
        caller: ProjectCaller,
        request: RebornCreateProjectRequest,
    ) -> Result<RebornProjectResponse, ProjectServiceError>;

    /// Fetch a single project the caller can access.
    async fn get_project(
        &self,
        caller: ProjectCaller,
        request: RebornGetProjectRequest,
    ) -> Result<RebornProjectResponse, ProjectServiceError>;

    /// Update a project. Requires editor or owner access.
    async fn update_project(
        &self,
        caller: ProjectCaller,
        request: RebornUpdateProjectRequest,
    ) -> Result<RebornProjectResponse, ProjectServiceError>;

    /// Delete a project. Requires owner access.
    async fn delete_project(
        &self,
        caller: ProjectCaller,
        request: RebornDeleteProjectRequest,
    ) -> Result<(), ProjectServiceError>;

    /// List a project's membership grants. Requires viewer access.
    async fn list_members(
        &self,
        caller: ProjectCaller,
        request: RebornListMembersRequest,
    ) -> Result<RebornListMembersResponse, ProjectServiceError>;

    /// Grant a user a role on a project. Requires owner access.
    async fn add_member(
        &self,
        caller: ProjectCaller,
        request: RebornAddMemberRequest,
    ) -> Result<RebornProjectMemberInfo, ProjectServiceError>;

    /// Change a member's role. Requires owner access.
    async fn update_member_role(
        &self,
        caller: ProjectCaller,
        request: RebornUpdateMemberRoleRequest,
    ) -> Result<RebornProjectMemberInfo, ProjectServiceError>;

    /// Revoke a member. Requires owner access.
    async fn remove_member(
        &self,
        caller: ProjectCaller,
        request: RebornRemoveMemberRequest,
    ) -> Result<(), ProjectServiceError>;
}
