use ironclaw_event_projections::ProjectionScope;

use crate::types::ProjectionTarget;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ScopeAdmissionKey {
    pub(crate) scope: ProjectionScopeKey,
    pub(crate) target: ProjectionTargetKey,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ProjectionScopeKey {
    tenant_id: String,
    user_id: String,
    agent_id: Option<String>,
    project_id: Option<String>,
    mission_id: Option<String>,
    thread_id: Option<String>,
    process_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum ProjectionTargetKey {
    Thread(String),
    Mission(String),
    Run(String),
    Process(String),
    DeliveryStatus(String),
}

pub(crate) fn scope_key(scope: &ProjectionScope, target: &ProjectionTarget) -> ScopeAdmissionKey {
    ScopeAdmissionKey {
        scope: projection_scope_key(scope),
        target: target_key(target),
    }
}

pub(crate) fn projection_scope_key(scope: &ProjectionScope) -> ProjectionScopeKey {
    ProjectionScopeKey {
        tenant_id: scope.stream.tenant_id.to_string(),
        user_id: scope.stream.user_id.to_string(),
        agent_id: scope.stream.agent_id.as_ref().map(ToString::to_string),
        project_id: scope
            .read_scope
            .project_id
            .as_ref()
            .map(ToString::to_string),
        mission_id: scope
            .read_scope
            .mission_id
            .as_ref()
            .map(ToString::to_string),
        thread_id: scope.read_scope.thread_id.as_ref().map(ToString::to_string),
        process_id: scope
            .read_scope
            .process_id
            .as_ref()
            .map(ToString::to_string),
    }
}

fn target_key(target: &ProjectionTarget) -> ProjectionTargetKey {
    match target {
        ProjectionTarget::Thread { thread_id } => {
            ProjectionTargetKey::Thread(thread_id.to_string())
        }
        ProjectionTarget::Mission { mission_id } => {
            ProjectionTargetKey::Mission(mission_id.to_string())
        }
        ProjectionTarget::Run { invocation_id } => {
            ProjectionTargetKey::Run(invocation_id.to_string())
        }
        ProjectionTarget::Process { process_id } => {
            ProjectionTargetKey::Process(process_id.to_string())
        }
        ProjectionTarget::DeliveryStatus { thread_id } => {
            ProjectionTargetKey::DeliveryStatus(thread_id.to_string())
        }
    }
}
