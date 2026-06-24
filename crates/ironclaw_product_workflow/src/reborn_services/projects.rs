//! Project management port for the WebUI v2 facade.
//!
//! Surfaces first-class projects — create, list, read, update, delete — plus
//! their membership grants (the ACL surface). The port is injected by host
//! composition, which owns the durable [`ProjectRepository`] and performs
//! access-control gating (owner/role checks via the repository's
//! `resolve_access`) before any mutation.
//!
//! Identity is authority-bearing: the facade derives [`ProjectCaller`] from the
//! authenticated caller (tenant + user), never from the request body. Roles and
//! states are product-level enums here so this boundary stays free of the
//! `ironclaw_projects` substrate types — the composition adapter maps between
//! the two (mirrors how [`ProjectFsEntryKind`](super::ProjectFsEntryKind) shadows
//! `ironclaw_filesystem::FileType`).

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_host_api::{TenantId, UserId};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

/// Trusted caller identity for project operations.
///
/// Built by the facade from the authenticated caller. Never reconstructed from
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
    /// match the other WebUI facade DTOs rather than an ambiguous `String`.
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

/// Errors a project operation may produce.
///
/// Deliberately coarse and free of host paths / backend strings: the facade
/// maps each variant to a sanitized [`RebornServicesError`](crate::RebornServicesError)
/// at the boundary. Implementations outside this crate construct these instead
/// of reaching for the facade error's `pub(super)` constructors.
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
/// Every method takes a [`ProjectCaller`] the facade derived from the
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
