//! In-process `ProjectService` fault-injecting decorator for the C-SYNTH
//! `project_create` seam: wraps the real inner `ProjectService`; a sentinel
//! `create_project` name triggers a scripted fault, any other call passes
//! straight through.
//!
//! Two sentinels: `FAULT_INJECT_DENIED_PROJECT_NAME` forces a permanent
//! `Denied` (recoverable, non-retrying `Failed` on the first attempt).
//! `FAULT_INJECT_UNAVAILABLE_ONCE_PROJECT_NAME` forces `Unavailable` on the
//! first call only, then delegates to the inner service — exercising the
//! capability-retry recovery path (`DefaultRecoveryStrategy` retries
//! `Unavailable`/`Internal`).

#![allow(dead_code)]

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use async_trait::async_trait;
use ironclaw_product_workflow::{
    ProjectCaller, ProjectService, ProjectServiceError, RebornAddMemberRequest,
    RebornCreateProjectRequest, RebornDeleteProjectRequest, RebornGetProjectRequest,
    RebornListMembersRequest, RebornListMembersResponse, RebornListProjectsRequest,
    RebornListProjectsResponse, RebornProjectMemberInfo, RebornProjectResponse,
    RebornRemoveMemberRequest, RebornUpdateMemberRoleRequest, RebornUpdateProjectRequest,
};

/// Sentinel `create_project` name that triggers the injected fault; distinct
/// from ordinary test project names so it never intercepts a real create.
pub(crate) const FAULT_INJECT_DENIED_PROJECT_NAME: &str = "FAULT_INJECT_DENIED_PROJECT";

/// Sentinel `create_project` name that triggers a one-shot `Unavailable`
/// fault: the FIRST call for this name fails `Unavailable`; every subsequent
/// call (including a capability retry) delegates to the inner service.
pub(crate) const FAULT_INJECT_UNAVAILABLE_ONCE_PROJECT_NAME: &str =
    "FAULT_INJECT_UNAVAILABLE_ONCE_PROJECT";

/// Decorator around a real `Arc<dyn ProjectService>`: forces `Denied` on the
/// `FAULT_INJECT_DENIED_PROJECT_NAME` sentinel and `Unavailable` on the FIRST
/// call naming `FAULT_INJECT_UNAVAILABLE_ONCE_PROJECT_NAME`, delegating
/// everything else (including retries of the latter) to the inner service.
pub(crate) struct FaultInjectingProjectService {
    inner: Arc<dyn ProjectService>,
    unavailable_once_calls: AtomicUsize,
}

impl FaultInjectingProjectService {
    pub(crate) fn wrapping(inner: Arc<dyn ProjectService>) -> Arc<Self> {
        Arc::new(Self {
            inner,
            unavailable_once_calls: AtomicUsize::new(0),
        })
    }
}

#[async_trait]
impl ProjectService for FaultInjectingProjectService {
    async fn list_projects(
        &self,
        caller: ProjectCaller,
        request: RebornListProjectsRequest,
    ) -> Result<RebornListProjectsResponse, ProjectServiceError> {
        self.inner.list_projects(caller, request).await
    }

    async fn create_project(
        &self,
        caller: ProjectCaller,
        request: RebornCreateProjectRequest,
    ) -> Result<RebornProjectResponse, ProjectServiceError> {
        if request.name == FAULT_INJECT_DENIED_PROJECT_NAME {
            return Err(ProjectServiceError::Denied);
        }
        if request.name == FAULT_INJECT_UNAVAILABLE_ONCE_PROJECT_NAME
            && self.unavailable_once_calls.fetch_add(1, Ordering::SeqCst) == 0
        {
            return Err(ProjectServiceError::Unavailable);
        }
        self.inner.create_project(caller, request).await
    }

    async fn get_project(
        &self,
        caller: ProjectCaller,
        request: RebornGetProjectRequest,
    ) -> Result<RebornProjectResponse, ProjectServiceError> {
        self.inner.get_project(caller, request).await
    }

    async fn update_project(
        &self,
        caller: ProjectCaller,
        request: RebornUpdateProjectRequest,
    ) -> Result<RebornProjectResponse, ProjectServiceError> {
        self.inner.update_project(caller, request).await
    }

    async fn delete_project(
        &self,
        caller: ProjectCaller,
        request: RebornDeleteProjectRequest,
    ) -> Result<(), ProjectServiceError> {
        self.inner.delete_project(caller, request).await
    }

    async fn list_members(
        &self,
        caller: ProjectCaller,
        request: RebornListMembersRequest,
    ) -> Result<RebornListMembersResponse, ProjectServiceError> {
        self.inner.list_members(caller, request).await
    }

    async fn add_member(
        &self,
        caller: ProjectCaller,
        request: RebornAddMemberRequest,
    ) -> Result<RebornProjectMemberInfo, ProjectServiceError> {
        self.inner.add_member(caller, request).await
    }

    async fn update_member_role(
        &self,
        caller: ProjectCaller,
        request: RebornUpdateMemberRoleRequest,
    ) -> Result<RebornProjectMemberInfo, ProjectServiceError> {
        self.inner.update_member_role(caller, request).await
    }

    async fn remove_member(
        &self,
        caller: ProjectCaller,
        request: RebornRemoveMemberRequest,
    ) -> Result<(), ProjectServiceError> {
        self.inner.remove_member(caller, request).await
    }
}
