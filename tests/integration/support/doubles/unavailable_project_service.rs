//! No-op `ProjectService` for harness-port-seam P1 Change 2: production's
//! `RefreshingCapabilityPortConfig::project_service` is a plain
//! required `Arc<dyn ProjectService>` (the synthetic `project_create`
//! capability is always assembled by `build_inner`, independent of whether
//! `PROJECT_CREATE_CAPABILITY_ID` is in a harness's `capability_ids`), so
//! every harness needs SOME implementation even when it has no opinion on
//! project storage. Every method returns `Unavailable` -- a harness that
//! genuinely dispatches `project_create` supplies its own real
//! `Arc<dyn ProjectService>` via `project_service_for_test` instead.
use async_trait::async_trait;
use ironclaw_product_workflow::{
    IronClawAddMemberRequest, IronClawCreateProjectRequest, IronClawDeleteProjectRequest,
    IronClawGetProjectRequest, IronClawListMembersRequest, IronClawListMembersResponse,
    IronClawListProjectsRequest, IronClawListProjectsResponse, IronClawProjectMemberInfo,
    IronClawProjectResponse, IronClawRemoveMemberRequest, IronClawUpdateMemberRoleRequest,
    IronClawUpdateProjectRequest, ProjectCaller, ProjectService, ProjectServiceError,
};

pub(crate) struct UnavailableProjectService;

#[async_trait]
impl ProjectService for UnavailableProjectService {
    async fn list_projects(
        &self,
        _caller: ProjectCaller,
        _request: IronClawListProjectsRequest,
    ) -> Result<IronClawListProjectsResponse, ProjectServiceError> {
        Err(ProjectServiceError::Unavailable)
    }

    async fn create_project(
        &self,
        _caller: ProjectCaller,
        _request: IronClawCreateProjectRequest,
    ) -> Result<IronClawProjectResponse, ProjectServiceError> {
        Err(ProjectServiceError::Unavailable)
    }

    async fn get_project(
        &self,
        _caller: ProjectCaller,
        _request: IronClawGetProjectRequest,
    ) -> Result<IronClawProjectResponse, ProjectServiceError> {
        Err(ProjectServiceError::Unavailable)
    }

    async fn update_project(
        &self,
        _caller: ProjectCaller,
        _request: IronClawUpdateProjectRequest,
    ) -> Result<IronClawProjectResponse, ProjectServiceError> {
        Err(ProjectServiceError::Unavailable)
    }

    async fn delete_project(
        &self,
        _caller: ProjectCaller,
        _request: IronClawDeleteProjectRequest,
    ) -> Result<(), ProjectServiceError> {
        Err(ProjectServiceError::Unavailable)
    }

    async fn list_members(
        &self,
        _caller: ProjectCaller,
        _request: IronClawListMembersRequest,
    ) -> Result<IronClawListMembersResponse, ProjectServiceError> {
        Err(ProjectServiceError::Unavailable)
    }

    async fn add_member(
        &self,
        _caller: ProjectCaller,
        _request: IronClawAddMemberRequest,
    ) -> Result<IronClawProjectMemberInfo, ProjectServiceError> {
        Err(ProjectServiceError::Unavailable)
    }

    async fn update_member_role(
        &self,
        _caller: ProjectCaller,
        _request: IronClawUpdateMemberRoleRequest,
    ) -> Result<IronClawProjectMemberInfo, ProjectServiceError> {
        Err(ProjectServiceError::Unavailable)
    }

    async fn remove_member(
        &self,
        _caller: ProjectCaller,
        _request: IronClawRemoveMemberRequest,
    ) -> Result<(), ProjectServiceError> {
        Err(ProjectServiceError::Unavailable)
    }
}
