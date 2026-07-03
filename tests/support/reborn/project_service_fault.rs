//! In-process `ProjectService` fault-injecting decorator for the C-SYNTH
//! `project_create` seam.
//!
//! Substitutes ONLY at the production-wired `Arc<dyn ProjectService>` seam the
//! `builtin.project_create` synthetic capability consumes
//! (`wrap_project_create_capability_for_test` ->
//! `ProjectCreateHandler::project_service`, see
//! `crates/ironclaw_reborn_composition/src/runtime/local_dev/project_create.rs`).
//! Delegates every call to a REAL inner `ProjectService` (the same
//! `local_dev_project_service_for_test()` instance every other project-tools
//! harness uses) except `create_project`, where a caller-chosen sentinel name
//! triggers a scripted `ProjectServiceError` instead of reaching the real
//! store — the same "double at the trait seam production already uses"
//! pattern as `FakeOutboundPreferencesFacade` (`outbound_preferences.rs`) and
//! `ScriptedHttpResponse::egress_error`. Any other name passes straight
//! through to the real service, so the same double can drive both a
//! fault-injection arm and an ordinary happy-path project_create in the same
//! group.
//!
//! Deliberately forces `ProjectServiceError::Denied` (`PolicyDenied`), NOT
//! `Unavailable`/`Internal`: those two `CapabilityFailureKind`s route through
//! `DefaultRecoveryStrategy`'s capability-retry branch
//! (`crates/ironclaw_agent_loop/src/strategies/recovery.rs`), which
//! re-dispatches via `capability_invocation_from_candidate` reusing the
//! ORIGINAL `input_ref`. For a provider-tool-call-originated invocation under
//! local-dev composition, that retry hits a real, independently confirmed
//! production bug — `LocalDevCapabilityIo::resolve_capability_input`
//! (`crates/ironclaw_reborn_composition/src/runtime/local_dev.rs`) rejects the
//! SAME `input_ref` on the retry with `InvalidInvocation`/"capability input
//! ref was not staged for this loop run" (the first attempt's input resolves
//! through a different, staging-bypassing path — see
//! `LocalDevCapabilityIo`'s own doc comment), collapsing the documented
//! "retry twice, then a model-visible `Failed`" contract into an immediate
//! terminal `driver_unavailable`. See issue #5608 for the full
//! repro. `Denied` avoids that retry path entirely
//! (`capability_error_is_model_visible_tool_failure` surfaces `PolicyDenied`
//! straight to the model on the FIRST attempt), so this double still proves a
//! genuine, distinct `project_service_outcome` arm end-to-end without
//! tripping the unrelated retry bug.

#![allow(dead_code)]

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_product_workflow::{
    ProjectCaller, ProjectService, ProjectServiceError, RebornAddMemberRequest,
    RebornCreateProjectRequest, RebornDeleteProjectRequest, RebornGetProjectRequest,
    RebornListMembersRequest, RebornListMembersResponse, RebornListProjectsRequest,
    RebornListProjectsResponse, RebornProjectMemberInfo, RebornProjectResponse,
    RebornRemoveMemberRequest, RebornUpdateMemberRoleRequest, RebornUpdateProjectRequest,
};

/// Sentinel `create_project` name that triggers the injected fault instead of
/// reaching the real project store. Kept distinct from ordinary test project
/// names (`"My Project"` etc.) so the same double never accidentally
/// intercepts an unrelated happy-path create.
pub(crate) const FAULT_INJECT_DENIED_PROJECT_NAME: &str = "FAULT_INJECT_DENIED_PROJECT";

/// Decorator around a real `Arc<dyn ProjectService>` that forces
/// `ProjectServiceError::Denied` on a `create_project` call naming the
/// sentinel, and delegates everything else (including non-sentinel
/// `create_project` calls) to the wrapped real service.
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
