use ironclaw_event_projections::ProjectionScope;
use ironclaw_host_api::{
    AgentId, InvocationId, MissionId, ProcessId, ProjectId, TenantId, ThreadId, UserId,
};

use crate::types::ProjectionTarget;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ScopeAdmissionKey {
    pub(crate) scope: ProjectionScopeKey,
    pub(crate) target: ProjectionTargetKey,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ProjectionScopeKey {
    tenant_id: TenantId,
    user_id: UserId,
    agent_id: Option<AgentId>,
    project_id: Option<ProjectId>,
    mission_id: Option<MissionId>,
    thread_id: Option<ThreadId>,
    process_id: Option<ProcessId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum ProjectionTargetKey {
    Thread(ThreadId),
    Mission(MissionId),
    Run(InvocationId),
    Process(ProcessId),
    DeliveryStatus(ThreadId),
}

pub(crate) fn scope_key(scope: &ProjectionScope, target: &ProjectionTarget) -> ScopeAdmissionKey {
    ScopeAdmissionKey {
        scope: projection_scope_key(scope),
        target: target_key(target),
    }
}

pub(crate) fn projection_scope_key(scope: &ProjectionScope) -> ProjectionScopeKey {
    ProjectionScopeKey {
        tenant_id: scope.stream.tenant_id.clone(),
        user_id: scope.stream.user_id.clone(),
        agent_id: scope.stream.agent_id.clone(),
        project_id: scope.read_scope.project_id.clone(),
        mission_id: scope.read_scope.mission_id.clone(),
        thread_id: scope.read_scope.thread_id.clone(),
        process_id: scope.read_scope.process_id,
    }
}

fn target_key(target: &ProjectionTarget) -> ProjectionTargetKey {
    match target {
        ProjectionTarget::Thread { thread_id } => ProjectionTargetKey::Thread(thread_id.clone()),
        ProjectionTarget::Mission { mission_id } => {
            ProjectionTargetKey::Mission(mission_id.clone())
        }
        ProjectionTarget::Run { invocation_id } => ProjectionTargetKey::Run(*invocation_id),
        ProjectionTarget::Process { process_id } => ProjectionTargetKey::Process(*process_id),
        ProjectionTarget::DeliveryStatus { thread_id } => {
            ProjectionTargetKey::DeliveryStatus(thread_id.clone())
        }
    }
}
