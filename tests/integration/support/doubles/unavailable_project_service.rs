//! No-op `ProjectService` for harness-port-seam P1 Change 2: production's
//! `RefreshingLocalDevCapabilityPortConfig::project_service` is a plain
//! required `Arc<dyn ProjectService>` (the synthetic `project_create`
//! capability is always assembled by `build_inner`, independent of whether
//! `PROJECT_CREATE_CAPABILITY_ID` is in a harness's `capability_ids`), so
//! every harness needs SOME implementation even when it has no opinion on
//! project storage. Every method returns `Unavailable` -- a harness that
//! genuinely dispatches `project_create` supplies its own real
//! `Arc<dyn ProjectService>` via `project_service_for_test` instead.
use async_trait::async_trait;
use ironclaw_product_workflow::{
    ProjectCaller, ProjectService, ProjectServiceError, RebornAddMemberRequest,
    RebornCreateProjectRequest, RebornDeleteProjectRequest, RebornGetProjectRequest,
    RebornListMembersRequest, RebornListMembersResponse, RebornListProjectsRequest,
    RebornListProjectsResponse, RebornProjectMemberInfo, RebornProjectResponse,
    RebornRemoveMemberRequest, RebornUpdateMemberRoleRequest, RebornUpdateProjectRequest,
};

pub(crate) struct UnavailableProjectService;

#[async_trait]
impl ProjectService for UnavailableProjectService {
    async fn list_projects(
        &self,
        _caller: ProjectCaller,
        _request: RebornListProjectsRequest,
    ) -> Result<RebornListProjectsResponse, ProjectServiceError> {
        Err(ProjectServiceError::Unavailable)
    }

    async fn create_project(
        &self,
        _caller: ProjectCaller,
        _request: RebornCreateProjectRequest,
    ) -> Result<RebornProjectResponse, ProjectServiceError> {
        Err(ProjectServiceError::Unavailable)
    }

    async fn get_project(
        &self,
        _caller: ProjectCaller,
        _request: RebornGetProjectRequest,
    ) -> Result<RebornProjectResponse, ProjectServiceError> {
        Err(ProjectServiceError::Unavailable)
    }

    async fn update_project(
        &self,
        _caller: ProjectCaller,
        _request: RebornUpdateProjectRequest,
    ) -> Result<RebornProjectResponse, ProjectServiceError> {
        Err(ProjectServiceError::Unavailable)
    }

    async fn delete_project(
        &self,
        _caller: ProjectCaller,
        _request: RebornDeleteProjectRequest,
    ) -> Result<(), ProjectServiceError> {
        Err(ProjectServiceError::Unavailable)
    }

    async fn list_members(
        &self,
        _caller: ProjectCaller,
        _request: RebornListMembersRequest,
    ) -> Result<RebornListMembersResponse, ProjectServiceError> {
        Err(ProjectServiceError::Unavailable)
    }

    async fn add_member(
        &self,
        _caller: ProjectCaller,
        _request: RebornAddMemberRequest,
    ) -> Result<RebornProjectMemberInfo, ProjectServiceError> {
        Err(ProjectServiceError::Unavailable)
    }

    async fn update_member_role(
        &self,
        _caller: ProjectCaller,
        _request: RebornUpdateMemberRoleRequest,
    ) -> Result<RebornProjectMemberInfo, ProjectServiceError> {
        Err(ProjectServiceError::Unavailable)
    }

    async fn remove_member(
        &self,
        _caller: ProjectCaller,
        _request: RebornRemoveMemberRequest,
    ) -> Result<(), ProjectServiceError> {
        Err(ProjectServiceError::Unavailable)
    }
}
