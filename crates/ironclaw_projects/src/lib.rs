//! First-class Project domain contracts for IronClaw Reborn.
//!
//! This crate owns the Project entity, project membership / access-control
//! records, and the [`ProjectRepository`] persistence contract. The single
//! implementation, [`FilesystemProjectRepository`], persists records over the
//! Reborn `ScopedFilesystem` substrate, so backend selection (Postgres / libSQL
//! / JSONL / in-memory) is the host's `RootFilesystem` concern — this crate
//! contains no SQL.
//!
//! Projects scope threads, automations, and workspace memory. In the Reborn
//! stack a `project_id` already flows through `ThreadScope`,
//! `ProductAgentBoundCaller`, and `TriggerRecord` as a scope identifier; this
//! crate gives that identifier a durable, access-controlled entity.
//!
//! Authorization is **live** — [`ProjectRepository::resolve_access`] is called
//! per request and never cached, so revoking a grant takes effect immediately.

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_host_api::{ProjectId, TenantId, Timestamp, UserId};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use thiserror::Error;
use ulid::Ulid;

mod store;

pub use store::FilesystemProjectRepository;

/// Maximum byte length of a project name.
pub const MAX_PROJECT_NAME_BYTES: usize = 200;
/// Maximum byte length of a project description.
pub const MAX_PROJECT_DESCRIPTION_BYTES: usize = 4_000;
/// Maximum byte length of the serialized `metadata` bag.
pub const MAX_PROJECT_METADATA_BYTES: usize = 64 * 1024;
/// Maximum byte length of the `icon` / `color` presentation hints.
pub const MAX_PROJECT_PRESENTATION_BYTES: usize = 64;

/// Errors returned by project domain operations and repository backends.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProjectError {
    /// A record failed validation before persistence.
    #[error("invalid project record: {reason}")]
    InvalidRecord { reason: String },
    /// A membership record failed validation before persistence.
    #[error("invalid project member record: {reason}")]
    InvalidMember { reason: String },
    /// A create collided with an existing project id.
    #[error("project already exists")]
    AlreadyExists,
    /// The requested project (or member) does not exist.
    #[error("project not found")]
    NotFound,
    /// The persistence backend failed.
    #[error("project repository backend unavailable: {reason}")]
    Backend { reason: String },
}

impl ProjectError {
    pub(crate) fn invalid_record(reason: impl Into<String>) -> Self {
        Self::InvalidRecord {
            reason: reason.into(),
        }
    }

    pub(crate) fn invalid_member(reason: impl Into<String>) -> Self {
        Self::InvalidMember {
            reason: reason.into(),
        }
    }

    pub(crate) fn backend(operation: &str, error: impl std::fmt::Display) -> Self {
        Self::Backend {
            reason: format!("{operation}: {error}"),
        }
    }
}

/// Lifecycle state of a project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectState {
    /// Default, visible state.
    Active,
    /// Hidden from default listings but retained.
    Archived,
}

impl ProjectState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Archived => "archived",
        }
    }

    pub fn parse(value: &str) -> Result<Self, ProjectError> {
        match value {
            "active" => Ok(Self::Active),
            "archived" => Ok(Self::Archived),
            other => Err(ProjectError::invalid_record(format!(
                "unknown project state `{other}`"
            ))),
        }
    }
}

/// Access role a user holds on a project. Ordered: `Viewer < Editor < Owner`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectRole {
    /// Full control, including membership and deletion.
    Owner,
    /// May read and mutate project content.
    Editor,
    /// Read-only access.
    Viewer,
}

impl ProjectRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Editor => "editor",
            Self::Viewer => "viewer",
        }
    }

    pub fn parse(value: &str) -> Result<Self, ProjectError> {
        match value {
            "owner" => Ok(Self::Owner),
            "editor" => Ok(Self::Editor),
            "viewer" => Ok(Self::Viewer),
            other => Err(ProjectError::invalid_member(format!(
                "unknown project role `{other}`"
            ))),
        }
    }

    /// Higher rank == more privilege.
    fn rank(self) -> u8 {
        match self {
            Self::Viewer => 0,
            Self::Editor => 1,
            Self::Owner => 2,
        }
    }

    /// Whether this role satisfies a `required` minimum role.
    pub fn allows(self, required: ProjectRole) -> bool {
        self.rank() >= required.rank()
    }
}

/// Membership grant status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectMemberStatus {
    /// Grant is in effect.
    Active,
    /// Grant has been revoked (retained for audit).
    Revoked,
}

impl ProjectMemberStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Revoked => "revoked",
        }
    }

    pub fn parse(value: &str) -> Result<Self, ProjectError> {
        match value {
            "active" => Ok(Self::Active),
            "revoked" => Ok(Self::Revoked),
            other => Err(ProjectError::invalid_member(format!(
                "unknown project member status `{other}`"
            ))),
        }
    }
}

/// A persisted project entity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectRecord {
    pub project_id: ProjectId,
    pub tenant_id: TenantId,
    pub owner_user_id: UserId,
    pub name: String,
    pub description: String,
    pub icon: Option<String>,
    pub color: Option<String>,
    /// Extensible bag (goals, GitHub links, …). Must be a JSON object or null.
    pub metadata: JsonValue,
    pub state: ProjectState,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl ProjectRecord {
    /// Construct a new active project, minting a fresh [`ProjectId`] and
    /// stamping `created_at` / `updated_at` to now.
    pub fn new(
        tenant_id: TenantId,
        owner_user_id: UserId,
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Result<Self, ProjectError> {
        let project_id = ProjectId::new(Ulid::new().to_string())
            .map_err(|error| ProjectError::invalid_record(error.to_string()))?;
        let now = Utc::now();
        let record = Self {
            project_id,
            tenant_id,
            owner_user_id,
            name: name.into(),
            description: description.into(),
            icon: None,
            color: None,
            metadata: JsonValue::Object(serde_json::Map::new()),
            state: ProjectState::Active,
            created_at: now,
            updated_at: now,
        };
        record.validate()?;
        Ok(record)
    }

    pub fn validate(&self) -> Result<(), ProjectError> {
        if self.name.trim().is_empty() {
            return Err(ProjectError::invalid_record(
                "project name must not be empty",
            ));
        }
        if self.name.len() > MAX_PROJECT_NAME_BYTES {
            return Err(ProjectError::invalid_record(format!(
                "project name must be at most {MAX_PROJECT_NAME_BYTES} bytes"
            )));
        }
        if self.description.len() > MAX_PROJECT_DESCRIPTION_BYTES {
            return Err(ProjectError::invalid_record(format!(
                "project description must be at most {MAX_PROJECT_DESCRIPTION_BYTES} bytes"
            )));
        }
        validate_presentation("icon", self.icon.as_deref())?;
        validate_presentation("color", self.color.as_deref())?;
        if !(self.metadata.is_object() || self.metadata.is_null()) {
            return Err(ProjectError::invalid_record(
                "project metadata must be a JSON object or null",
            ));
        }
        let encoded = serde_json::to_string(&self.metadata)
            .map_err(|error| ProjectError::invalid_record(error.to_string()))?;
        if encoded.len() > MAX_PROJECT_METADATA_BYTES {
            return Err(ProjectError::invalid_record(format!(
                "project metadata must be at most {MAX_PROJECT_METADATA_BYTES} bytes"
            )));
        }
        Ok(())
    }

    /// Returns the owner's standing membership grant (owners are always Owner).
    pub fn owner_membership(&self) -> ProjectMemberRecord {
        ProjectMemberRecord {
            tenant_id: self.tenant_id.clone(),
            project_id: self.project_id.clone(),
            user_id: self.owner_user_id.clone(),
            role: ProjectRole::Owner,
            status: ProjectMemberStatus::Active,
            granted_by: self.owner_user_id.clone(),
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

fn validate_presentation(field: &str, value: Option<&str>) -> Result<(), ProjectError> {
    if let Some(value) = value
        && value.len() > MAX_PROJECT_PRESENTATION_BYTES
    {
        return Err(ProjectError::invalid_record(format!(
            "project {field} must be at most {MAX_PROJECT_PRESENTATION_BYTES} bytes"
        )));
    }
    Ok(())
}

/// A persisted membership grant linking a user to a project at a given role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectMemberRecord {
    pub tenant_id: TenantId,
    pub project_id: ProjectId,
    pub user_id: UserId,
    pub role: ProjectRole,
    pub status: ProjectMemberStatus,
    /// User who created or last modified the grant.
    pub granted_by: UserId,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl ProjectMemberRecord {
    pub fn validate(&self) -> Result<(), ProjectError> {
        // Identity newtypes are already validated; nothing further today.
        let _ = self;
        Ok(())
    }
}

/// Persistence contract for projects and their membership grants.
///
/// The sole implementation ([`FilesystemProjectRepository`]) persists over the
/// Reborn `ScopedFilesystem` substrate, so it is backend-agnostic. Callers are
/// responsible for authorization *before* mutating writes;
/// [`Self::resolve_access`] is the read primitive used to enforce that.
#[async_trait]
pub trait ProjectRepository: Send + Sync {
    /// Insert a new project. Fails with [`ProjectError::AlreadyExists`] if a
    /// project with the same `(tenant_id, project_id)` is already present.
    async fn create_project(&self, record: ProjectRecord) -> Result<(), ProjectError>;

    /// Load a project by id, scoped to the tenant.
    async fn get_project(
        &self,
        tenant_id: &TenantId,
        project_id: &ProjectId,
    ) -> Result<Option<ProjectRecord>, ProjectError>;

    /// Replace an existing project. Fails with [`ProjectError::NotFound`] if the
    /// project does not exist.
    async fn update_project(&self, record: ProjectRecord) -> Result<(), ProjectError>;

    /// Remove a project, returning the removed record if present. Membership
    /// grants for the project are removed as well.
    async fn delete_project(
        &self,
        tenant_id: &TenantId,
        project_id: &ProjectId,
    ) -> Result<Option<ProjectRecord>, ProjectError>;

    /// List projects the user can access (owner or active member), most
    /// recently created first, capped at `limit`.
    async fn list_projects_for_user(
        &self,
        tenant_id: &TenantId,
        user_id: &UserId,
        limit: usize,
    ) -> Result<Vec<ProjectRecord>, ProjectError>;

    /// List all membership grants for a project (any status).
    async fn list_members(
        &self,
        tenant_id: &TenantId,
        project_id: &ProjectId,
    ) -> Result<Vec<ProjectMemberRecord>, ProjectError>;

    /// Insert or replace a membership grant.
    async fn upsert_member(&self, record: ProjectMemberRecord) -> Result<(), ProjectError>;

    /// Remove a membership grant, returning it if present.
    async fn remove_member(
        &self,
        tenant_id: &TenantId,
        project_id: &ProjectId,
        user_id: &UserId,
    ) -> Result<Option<ProjectMemberRecord>, ProjectError>;

    /// Resolve a user's effective role on a project. The project owner always
    /// resolves to [`ProjectRole::Owner`]; otherwise the highest active grant
    /// wins. `None` means no access (including unknown project).
    ///
    /// This is live and must not be cached by callers.
    async fn resolve_access(
        &self,
        tenant_id: &TenantId,
        project_id: &ProjectId,
        user_id: &UserId,
    ) -> Result<Option<ProjectRole>, ProjectError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tenant() -> TenantId {
        TenantId::new("tenant1").unwrap()
    }

    fn user(name: &str) -> UserId {
        UserId::new(name).unwrap()
    }

    fn project(owner: &UserId) -> ProjectRecord {
        ProjectRecord::new(tenant(), owner.clone(), "Research", "AI research project").unwrap()
    }

    #[test]
    fn role_ordering_allows_higher_privilege() {
        assert!(ProjectRole::Owner.allows(ProjectRole::Viewer));
        assert!(ProjectRole::Owner.allows(ProjectRole::Owner));
        assert!(ProjectRole::Editor.allows(ProjectRole::Viewer));
        assert!(!ProjectRole::Viewer.allows(ProjectRole::Editor));
        assert!(!ProjectRole::Editor.allows(ProjectRole::Owner));
    }

    #[test]
    fn record_validation_rejects_empty_name_and_bad_metadata() {
        let owner = user("alice");
        let mut record = project(&owner);
        record.name = "   ".to_string();
        assert!(matches!(
            record.validate(),
            Err(ProjectError::InvalidRecord { .. })
        ));

        let mut record = project(&owner);
        record.metadata = JsonValue::String("not-an-object".to_string());
        assert!(matches!(
            record.validate(),
            Err(ProjectError::InvalidRecord { .. })
        ));
    }
}
