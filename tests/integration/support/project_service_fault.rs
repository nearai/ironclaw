//! In-process `ProjectService` fault-injecting decorator for the C-SYNTH
//! `project_create` seam: wraps the real inner `ProjectService`; a sentinel
//! `create_project` name triggers a scripted fault, any other call passes
//! straight through.
//!
//! Deliberately forces `ProjectServiceError::Denied`, not
//! `Unavailable`/`Internal`: those retry via `DefaultRecoveryStrategy` and hit
//! a real `StagedCapabilityIo` input-ref restaging bug (issue #5608),
//! collapsing the retry contract into an immediate `driver_unavailable`.
//! `Denied` surfaces to the model on the first attempt, avoiding the bug.

#![allow(dead_code)]

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_product_workflow::{
    IronClawAddMemberRequest, IronClawCreateProjectRequest, IronClawDeleteProjectRequest,
    IronClawGetProjectRequest, IronClawListMembersRequest, IronClawListMembersResponse,
    IronClawListProjectsRequest, IronClawListProjectsResponse, IronClawProjectMemberInfo,
    IronClawProjectResponse, IronClawRemoveMemberRequest, IronClawUpdateMemberRoleRequest,
    IronClawUpdateProjectRequest, ProjectCaller, ProjectService, ProjectServiceError,
};

/// Sentinel `create_project` name that triggers the injected fault; distinct
/// from ordinary test project names so it never intercepts a real create.
pub(crate) const FAULT_INJECT_DENIED_PROJECT_NAME: &str = "FAULT_INJECT_DENIED_PROJECT";

/// Decorator around a real `Arc<dyn ProjectService>`: forces `Denied` on the
/// sentinel `create_project` name, delegates everything else to the inner service.
pub(crate) struct FaultInjectingProjectService {
    inner: Arc<dyn ProjectService>,
}

impl FaultInjectingProjectService {
    pub(crate) fn wrapping(inner: Arc<dyn ProjectService>) -> Arc<Self> {
        Arc::new(Self { inner })
    }
}

#[async_trait]
impl ProjectService for FaultInjectingProjectService {
    async fn list_projects(
        &self,
        caller: ProjectCaller,
        request: IronClawListProjectsRequest,
    ) -> Result<IronClawListProjectsResponse, ProjectServiceError> {
        self.inner.list_projects(caller, request).await
    }

    async fn create_project(
        &self,
        caller: ProjectCaller,
        request: IronClawCreateProjectRequest,
    ) -> Result<IronClawProjectResponse, ProjectServiceError> {
        if request.name == FAULT_INJECT_DENIED_PROJECT_NAME {
            return Err(ProjectServiceError::Denied);
        }
        self.inner.create_project(caller, request).await
    }

    async fn get_project(
        &self,
        caller: ProjectCaller,
        request: IronClawGetProjectRequest,
    ) -> Result<IronClawProjectResponse, ProjectServiceError> {
        self.inner.get_project(caller, request).await
    }

    async fn update_project(
        &self,
        caller: ProjectCaller,
        request: IronClawUpdateProjectRequest,
    ) -> Result<IronClawProjectResponse, ProjectServiceError> {
        self.inner.update_project(caller, request).await
    }

    async fn delete_project(
        &self,
        caller: ProjectCaller,
        request: IronClawDeleteProjectRequest,
    ) -> Result<(), ProjectServiceError> {
        self.inner.delete_project(caller, request).await
    }

    async fn list_members(
        &self,
        caller: ProjectCaller,
        request: IronClawListMembersRequest,
    ) -> Result<IronClawListMembersResponse, ProjectServiceError> {
        self.inner.list_members(caller, request).await
    }

    async fn add_member(
        &self,
        caller: ProjectCaller,
        request: IronClawAddMemberRequest,
    ) -> Result<IronClawProjectMemberInfo, ProjectServiceError> {
        self.inner.add_member(caller, request).await
    }

    async fn update_member_role(
        &self,
        caller: ProjectCaller,
        request: IronClawUpdateMemberRoleRequest,
    ) -> Result<IronClawProjectMemberInfo, ProjectServiceError> {
        self.inner.update_member_role(caller, request).await
    }

    async fn remove_member(
        &self,
        caller: ProjectCaller,
        request: IronClawRemoveMemberRequest,
    ) -> Result<(), ProjectServiceError> {
        self.inner.remove_member(caller, request).await
    }
}
