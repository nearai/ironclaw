//! Composition adapter implementing the product-workflow [`ProjectService`]
//! port over the durable [`ProjectRepository`].
//!
//! This is where access-control gating lives: every read/mutation resolves the
//! caller's effective role through `resolve_access` and enforces a minimum role
//! before touching the repository. The product boundary's coarse enums are
//! mapped to/from the `ironclaw_projects` domain types here so neither side
//! depends on the other's vocabulary.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{ProjectId, TenantId, UserId};
use ironclaw_product_workflow::{
    ProjectCaller, ProjectService, ProjectServiceError, RebornAddMemberRequest,
    RebornCreateProjectRequest, RebornDeleteProjectRequest, RebornGetProjectRequest,
    RebornListMembersRequest, RebornListMembersResponse, RebornListProjectsRequest,
    RebornListProjectsResponse, RebornProjectInfo, RebornProjectMemberInfo,
    RebornProjectMemberStatus, RebornProjectResponse, RebornProjectRole, RebornProjectState,
    RebornRemoveMemberRequest, RebornUpdateMemberRoleRequest, RebornUpdateProjectRequest,
};
use ironclaw_projects::{
    ProjectError, ProjectMemberRecord, ProjectMemberStatus, ProjectRecord, ProjectRepository,
    ProjectRole, ProjectState,
};

/// Default cap on `list_projects` when the request omits a limit.
const DEFAULT_PROJECT_LIST_LIMIT: usize = 200;
/// Hard cap on `list_projects` regardless of requested limit.
const MAX_PROJECT_LIST_LIMIT: usize = 500;

/// Access-controlled [`ProjectService`] backed by a [`ProjectRepository`].
pub(crate) struct RebornProjectService {
    repository: Arc<dyn ProjectRepository>,
}

impl RebornProjectService {
    pub(crate) fn new(repository: Arc<dyn ProjectRepository>) -> Self {
        Self { repository }
    }

    /// Resolve the caller's effective role and require at least `minimum`.
    ///
    /// No access at all collapses to [`ProjectServiceError::NotFound`] (the
    /// project's existence is not revealed); access below `minimum` is
    /// [`ProjectServiceError::Denied`].
    async fn require_role(
        &self,
        tenant_id: &TenantId,
        project_id: &ProjectId,
        user_id: &UserId,
        minimum: ProjectRole,
    ) -> Result<ProjectRole, ProjectServiceError> {
        match self
            .repository
            .resolve_access(tenant_id, project_id, user_id)
            .await
            .map_err(map_repo_error)?
        {
            None => Err(ProjectServiceError::NotFound),
            Some(role) if role.allows(minimum) => Ok(role),
            Some(_) => Err(ProjectServiceError::Denied),
        }
    }
}

#[async_trait]
impl ProjectService for RebornProjectService {
    async fn list_projects(
        &self,
        caller: ProjectCaller,
        request: RebornListProjectsRequest,
    ) -> Result<RebornListProjectsResponse, ProjectServiceError> {
        let limit = request
            .limit
            .map(|value| (value as usize).min(MAX_PROJECT_LIST_LIMIT))
            .unwrap_or(DEFAULT_PROJECT_LIST_LIMIT);
        let records = self
            .repository
            .list_projects_for_user(&caller.tenant_id, &caller.user_id, limit)
            .await
            .map_err(map_repo_error)?;
        let mut projects = Vec::with_capacity(records.len());
        for record in records {
            // Effective role for the caller on each listed project. If access was
            // revoked between the list and this resolve, drop the project rather
            // than fabricate a role (authorization is live; never show a project
            // the caller can no longer access).
            let Some(role) = self
                .repository
                .resolve_access(&caller.tenant_id, &record.project_id, &caller.user_id)
                .await
                .map_err(map_repo_error)?
            else {
                continue;
            };
            projects.push(project_info(record, role));
        }
        Ok(RebornListProjectsResponse { projects })
    }

    async fn create_project(
        &self,
        caller: ProjectCaller,
        request: RebornCreateProjectRequest,
    ) -> Result<RebornProjectResponse, ProjectServiceError> {
        let mut record = ProjectRecord::new(
            caller.tenant_id.clone(),
            caller.user_id.clone(),
            request.name,
            request.description,
        )
        .map_err(map_repo_error)?;
        record.icon = request.icon;
        record.color = request.color;
        if let Some(metadata) = request.metadata {
            record.metadata = metadata;
        }
        record.validate().map_err(map_repo_error)?;
        self.repository
            .create_project(record.clone())
            .await
            .map_err(map_repo_error)?;
        Ok(RebornProjectResponse {
            project: project_info(record, ProjectRole::Owner),
        })
    }

    async fn get_project(
        &self,
        caller: ProjectCaller,
        request: RebornGetProjectRequest,
    ) -> Result<RebornProjectResponse, ProjectServiceError> {
        let project_id = parse_project_id(&request.project_id)?;
        let role = self
            .require_role(
                &caller.tenant_id,
                &project_id,
                &caller.user_id,
                ProjectRole::Viewer,
            )
            .await?;
        let record = self
            .repository
            .get_project(&caller.tenant_id, &project_id)
            .await
            .map_err(map_repo_error)?
            .ok_or(ProjectServiceError::NotFound)?;
        Ok(RebornProjectResponse {
            project: project_info(record, role),
        })
    }

    async fn update_project(
        &self,
        caller: ProjectCaller,
        request: RebornUpdateProjectRequest,
    ) -> Result<RebornProjectResponse, ProjectServiceError> {
        let project_id = parse_project_id(&request.project_id)?;
        let role = self
            .require_role(
                &caller.tenant_id,
                &project_id,
                &caller.user_id,
                ProjectRole::Editor,
            )
            .await?;
        let mut record = self
            .repository
            .get_project(&caller.tenant_id, &project_id)
            .await
            .map_err(map_repo_error)?
            .ok_or(ProjectServiceError::NotFound)?;
        if let Some(name) = request.name {
            record.name = name;
        }
        if let Some(description) = request.description {
            record.description = description;
        }
        if request.icon.is_some() {
            record.icon = request.icon;
        }
        if request.color.is_some() {
            record.color = request.color;
        }
        if let Some(metadata) = request.metadata {
            record.metadata = metadata;
        }
        if let Some(state) = request.state {
            record.state = project_state_from_product(state);
        }
        record.updated_at = ironclaw_projects_now();
        record.validate().map_err(map_repo_error)?;
        self.repository
            .update_project(record.clone())
            .await
            .map_err(map_repo_error)?;
        Ok(RebornProjectResponse {
            project: project_info(record, role),
        })
    }

    async fn delete_project(
        &self,
        caller: ProjectCaller,
        request: RebornDeleteProjectRequest,
    ) -> Result<(), ProjectServiceError> {
        let project_id = parse_project_id(&request.project_id)?;
        self.require_role(
            &caller.tenant_id,
            &project_id,
            &caller.user_id,
            ProjectRole::Owner,
        )
        .await?;
        self.repository
            .delete_project(&caller.tenant_id, &project_id)
            .await
            .map_err(map_repo_error)?
            .ok_or(ProjectServiceError::NotFound)?;
        Ok(())
    }

    async fn list_members(
        &self,
        caller: ProjectCaller,
        request: RebornListMembersRequest,
    ) -> Result<RebornListMembersResponse, ProjectServiceError> {
        let project_id = parse_project_id(&request.project_id)?;
        self.require_role(
            &caller.tenant_id,
            &project_id,
            &caller.user_id,
            ProjectRole::Viewer,
        )
        .await?;
        let members = self
            .repository
            .list_members(&caller.tenant_id, &project_id)
            .await
            .map_err(map_repo_error)?
            .into_iter()
            .map(member_info)
            .collect();
        Ok(RebornListMembersResponse { members })
    }

    async fn add_member(
        &self,
        caller: ProjectCaller,
        request: RebornAddMemberRequest,
    ) -> Result<RebornProjectMemberInfo, ProjectServiceError> {
        let project_id = parse_project_id(&request.project_id)?;
        self.require_role(
            &caller.tenant_id,
            &project_id,
            &caller.user_id,
            ProjectRole::Owner,
        )
        .await?;
        let member_user = parse_user_id(&request.user_id)?;
        let now = ironclaw_projects_now();
        let record = ProjectMemberRecord {
            tenant_id: caller.tenant_id.clone(),
            project_id,
            user_id: member_user,
            role: project_role_from_product(request.role),
            status: ProjectMemberStatus::Active,
            granted_by: caller.user_id.clone(),
            created_at: now,
            updated_at: now,
        };
        self.repository
            .upsert_member(record.clone())
            .await
            .map_err(map_repo_error)?;
        Ok(member_info(record))
    }

    async fn update_member_role(
        &self,
        caller: ProjectCaller,
        request: RebornUpdateMemberRoleRequest,
    ) -> Result<RebornProjectMemberInfo, ProjectServiceError> {
        let project_id = parse_project_id(&request.project_id)?;
        self.require_role(
            &caller.tenant_id,
            &project_id,
            &caller.user_id,
            ProjectRole::Owner,
        )
        .await?;
        let member_user = parse_user_id(&request.user_id)?;
        // Only an active grant can be updated; a revoked member must be re-added
        // via add_member rather than silently resurrected by a role change.
        let existing = self
            .repository
            .list_members(&caller.tenant_id, &project_id)
            .await
            .map_err(map_repo_error)?
            .into_iter()
            .find(|member| {
                member.user_id == member_user && member.status == ProjectMemberStatus::Active
            })
            .ok_or(ProjectServiceError::NotFound)?;
        let record = ProjectMemberRecord {
            role: project_role_from_product(request.role),
            status: ProjectMemberStatus::Active,
            granted_by: caller.user_id.clone(),
            updated_at: ironclaw_projects_now(),
            ..existing
        };
        self.repository
            .upsert_member(record.clone())
            .await
            .map_err(map_repo_error)?;
        Ok(member_info(record))
    }

    async fn remove_member(
        &self,
        caller: ProjectCaller,
        request: RebornRemoveMemberRequest,
    ) -> Result<(), ProjectServiceError> {
        let project_id = parse_project_id(&request.project_id)?;
        self.require_role(
            &caller.tenant_id,
            &project_id,
            &caller.user_id,
            ProjectRole::Owner,
        )
        .await?;
        let member_user = parse_user_id(&request.user_id)?;
        self.repository
            .remove_member(&caller.tenant_id, &project_id, &member_user)
            .await
            .map_err(map_repo_error)?
            .ok_or(ProjectServiceError::NotFound)?;
        Ok(())
    }
}

fn ironclaw_projects_now() -> ironclaw_host_api::Timestamp {
    chrono::Utc::now()
}

fn parse_project_id(value: &str) -> Result<ProjectId, ProjectServiceError> {
    ProjectId::new(value).map_err(|error| {
        tracing::debug!(error = %error, "invalid project_id");
        ProjectServiceError::InvalidInput {
            field: "project_id".to_string(),
        }
    })
}

fn parse_user_id(value: &str) -> Result<UserId, ProjectServiceError> {
    UserId::new(value).map_err(|error| {
        tracing::debug!(error = %error, "invalid user_id");
        ProjectServiceError::InvalidInput {
            field: "user_id".to_string(),
        }
    })
}

fn project_info(record: ProjectRecord, role: ProjectRole) -> RebornProjectInfo {
    RebornProjectInfo {
        project_id: record.project_id.into_string(),
        name: record.name,
        description: record.description,
        icon: record.icon,
        color: record.color,
        metadata: record.metadata,
        state: project_state_to_product(record.state),
        role: project_role_to_product(role),
        created_at: record.created_at.to_rfc3339(),
        updated_at: record.updated_at.to_rfc3339(),
    }
}

fn member_info(record: ProjectMemberRecord) -> RebornProjectMemberInfo {
    RebornProjectMemberInfo {
        user_id: record.user_id.into_string(),
        role: project_role_to_product(record.role),
        status: member_status_to_product(record.status),
        granted_by: record.granted_by.into_string(),
        created_at: record.created_at.to_rfc3339(),
        updated_at: record.updated_at.to_rfc3339(),
    }
}

fn project_role_to_product(role: ProjectRole) -> RebornProjectRole {
    match role {
        ProjectRole::Owner => RebornProjectRole::Owner,
        ProjectRole::Editor => RebornProjectRole::Editor,
        ProjectRole::Viewer => RebornProjectRole::Viewer,
    }
}

fn project_role_from_product(role: RebornProjectRole) -> ProjectRole {
    match role {
        RebornProjectRole::Owner => ProjectRole::Owner,
        RebornProjectRole::Editor => ProjectRole::Editor,
        RebornProjectRole::Viewer => ProjectRole::Viewer,
    }
}

fn project_state_to_product(state: ProjectState) -> RebornProjectState {
    match state {
        ProjectState::Active => RebornProjectState::Active,
        ProjectState::Archived => RebornProjectState::Archived,
    }
}

fn project_state_from_product(state: RebornProjectState) -> ProjectState {
    match state {
        RebornProjectState::Active => ProjectState::Active,
        RebornProjectState::Archived => ProjectState::Archived,
    }
}

fn member_status_to_product(status: ProjectMemberStatus) -> RebornProjectMemberStatus {
    match status {
        ProjectMemberStatus::Active => RebornProjectMemberStatus::Active,
        ProjectMemberStatus::Revoked => RebornProjectMemberStatus::Revoked,
    }
}

/// Map a repository error to the sanitized product error, logging backend
/// causes so a 5xx is never undiagnosable (per `.claude/rules/error-handling.md`).
fn map_repo_error(error: ProjectError) -> ProjectServiceError {
    match error {
        ProjectError::NotFound => ProjectServiceError::NotFound,
        ProjectError::AlreadyExists => ProjectServiceError::Conflict,
        ProjectError::InvalidRecord { reason } => {
            tracing::debug!(error = %reason, "invalid project record");
            ProjectServiceError::InvalidInput {
                field: "project".to_string(),
            }
        }
        ProjectError::InvalidMember { reason } => {
            tracing::debug!(error = %reason, "invalid project member record");
            ProjectServiceError::InvalidInput {
                field: "member".to_string(),
            }
        }
        ProjectError::Backend { reason } => {
            tracing::error!(error = %reason, "project repository backend error");
            ProjectServiceError::Unavailable
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Repository fake with canned responses, so tests drive the real
    /// access-control path (`require_role` → `resolve_access`) of the service.
    #[derive(Default)]
    struct FakeRepo {
        access: Option<ProjectRole>,
        project: Option<ProjectRecord>,
        list: Vec<ProjectRecord>,
        members: Vec<ProjectMemberRecord>,
    }

    #[async_trait]
    impl ProjectRepository for FakeRepo {
        async fn create_project(&self, _: ProjectRecord) -> Result<(), ProjectError> {
            Ok(())
        }
        async fn get_project(
            &self,
            _: &TenantId,
            _: &ProjectId,
        ) -> Result<Option<ProjectRecord>, ProjectError> {
            Ok(self.project.clone())
        }
        async fn update_project(&self, _: ProjectRecord) -> Result<(), ProjectError> {
            Ok(())
        }
        async fn delete_project(
            &self,
            _: &TenantId,
            _: &ProjectId,
        ) -> Result<Option<ProjectRecord>, ProjectError> {
            Ok(self.project.clone())
        }
        async fn list_projects_for_user(
            &self,
            _: &TenantId,
            _: &UserId,
            _: usize,
        ) -> Result<Vec<ProjectRecord>, ProjectError> {
            Ok(self.list.clone())
        }
        async fn list_members(
            &self,
            _: &TenantId,
            _: &ProjectId,
        ) -> Result<Vec<ProjectMemberRecord>, ProjectError> {
            Ok(self.members.clone())
        }
        async fn upsert_member(&self, _: ProjectMemberRecord) -> Result<(), ProjectError> {
            Ok(())
        }
        async fn remove_member(
            &self,
            _: &TenantId,
            _: &ProjectId,
            _: &UserId,
        ) -> Result<Option<ProjectMemberRecord>, ProjectError> {
            Ok(self.members.first().cloned())
        }
        async fn resolve_access(
            &self,
            _: &TenantId,
            _: &ProjectId,
            _: &UserId,
        ) -> Result<Option<ProjectRole>, ProjectError> {
            Ok(self.access)
        }
    }

    fn caller() -> ProjectCaller {
        ProjectCaller {
            tenant_id: TenantId::new("t1").unwrap(),
            user_id: UserId::new("alice").unwrap(),
        }
    }

    fn a_record() -> ProjectRecord {
        ProjectRecord::new(
            TenantId::new("t1").unwrap(),
            UserId::new("alice").unwrap(),
            "P",
            "",
        )
        .unwrap()
    }

    fn svc(repo: FakeRepo) -> RebornProjectService {
        RebornProjectService::new(Arc::new(repo))
    }

    /// No grant at all collapses to NotFound — the project's existence is not
    /// revealed to a caller without access.
    #[tokio::test]
    async fn no_access_reads_as_not_found() {
        let record = a_record();
        let service = svc(FakeRepo {
            access: None,
            project: Some(record.clone()),
            ..Default::default()
        });
        let err = service
            .get_project(
                caller(),
                RebornGetProjectRequest {
                    project_id: record.project_id.into_string(),
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ProjectServiceError::NotFound));
    }

    /// A Viewer is denied a mutation that requires Editor.
    #[tokio::test]
    async fn viewer_cannot_update_project() {
        let record = a_record();
        let service = svc(FakeRepo {
            access: Some(ProjectRole::Viewer),
            project: Some(record.clone()),
            ..Default::default()
        });
        let err = service
            .update_project(
                caller(),
                RebornUpdateProjectRequest {
                    project_id: record.project_id.into_string(),
                    name: Some("new".to_string()),
                    description: None,
                    icon: None,
                    color: None,
                    metadata: None,
                    state: None,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ProjectServiceError::Denied));
    }

    /// `list_projects` drops a project whose access was revoked between the list
    /// and the per-project resolve (live authorization, never a stale role).
    #[tokio::test]
    async fn list_drops_project_revoked_after_listing() {
        let service = svc(FakeRepo {
            access: None,
            list: vec![a_record()],
            ..Default::default()
        });
        let response = service
            .list_projects(caller(), RebornListProjectsRequest { limit: None })
            .await
            .unwrap();
        assert!(response.projects.is_empty());
    }

    /// A revoked member cannot be resurrected by a role change — it reads as
    /// NotFound rather than silently re-activating the grant.
    #[tokio::test]
    async fn update_member_role_rejects_revoked_member() {
        let record = a_record();
        let now = chrono::Utc::now();
        let revoked = ProjectMemberRecord {
            tenant_id: TenantId::new("t1").unwrap(),
            project_id: record.project_id.clone(),
            user_id: UserId::new("bob").unwrap(),
            role: ProjectRole::Editor,
            status: ProjectMemberStatus::Revoked,
            granted_by: UserId::new("alice").unwrap(),
            created_at: now,
            updated_at: now,
        };
        let service = svc(FakeRepo {
            access: Some(ProjectRole::Owner),
            members: vec![revoked],
            ..Default::default()
        });
        let err = service
            .update_member_role(
                caller(),
                RebornUpdateMemberRoleRequest {
                    project_id: record.project_id.into_string(),
                    user_id: "bob".to_string(),
                    role: RebornProjectRole::Viewer,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ProjectServiceError::NotFound));
    }

    /// The creating caller is reported as the project Owner.
    #[tokio::test]
    async fn create_reports_owner_role() {
        let service = svc(FakeRepo::default());
        let response = service
            .create_project(
                caller(),
                RebornCreateProjectRequest {
                    name: "P".to_string(),
                    description: String::new(),
                    icon: None,
                    color: None,
                    metadata: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(response.project.role, RebornProjectRole::Owner);
    }
}
