//! Project management + membership (ACL) port (PROPOSAL §6.1.3; hoisted here by
//! §12.13 D-P).
//!
//! Surfaces first-class projects — create, list, read, update, delete — plus
//! their membership grants. The port is injected by host composition, which
//! owns the durable project repository and performs access-control gating
//! (owner/role checks) before any mutation.
//!
//! Identity is authority-bearing: the caller derives [`ProjectCaller`] from the
//! authenticated caller (tenant + user), never from the request body. Roles and
//! states are the coarse product enums in
//! [`workspace_views`](crate::workspace_views) so this boundary stays free of
//! the `ironclaw_identity::projects` substrate types — the implementing adapter
//! maps between the two.
//!
//! The declaration lives here rather than in `ironclaw_assistant` because the
//! implementing adapter sits *below* product (`ironclaw_identity`, `substrates`)
//! while its callers sit at and above it; a `products`-tier declaration made
//! that adapter unplaceable. See §12.13 D-P for the ruling and D-Q for the
//! identity dependency it licenses.
//!
//! Never here: a repository, a role resolver, or any access-control decision —
//! those are the implementor's, beside the records it gates.

use async_trait::async_trait;

use crate::workspace_views::{
    ProjectCaller, RebornAddMemberRequest, RebornCreateProjectRequest, RebornDeleteProjectRequest,
    RebornGetProjectRequest, RebornListMembersRequest, RebornListMembersResponse,
    RebornListProjectsRequest, RebornListProjectsResponse, RebornProjectMemberInfo,
    RebornProjectResponse, RebornRemoveMemberRequest, RebornUpdateMemberRoleRequest,
    RebornUpdateProjectRequest,
};

/// Errors a project operation may produce.
///
/// Deliberately coarse and free of host paths / backend strings: the product
/// surface maps each variant to a sanitized `ProductSurfaceError` at the
/// boundary. Implementations construct these instead of reaching for the
/// service error's crate-private constructors.
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
/// Every method takes a [`ProjectCaller`] the surface derived from the
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
