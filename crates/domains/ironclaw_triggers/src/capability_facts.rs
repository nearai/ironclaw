use async_trait::async_trait;
use ironclaw_host_api::{
    ids::{AgentId, CapabilityId, InvocationId, ProjectId, TenantId, UserId},
    resource::ResourceScope,
    turn::TurnRunId,
};
use serde::{Deserialize, Serialize};

/// Metadata-only runtime fact for one capability invocation in a trigger run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerCapabilityCallFact {
    pub invocation_id: InvocationId,
    pub run_id: TurnRunId,
    pub capability_id: CapabilityId,
    pub status: TriggerCapabilityCallStatus,
    pub error_kind: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerCapabilityCallStatus {
    Started,
    Running,
    Completed,
    Failed,
    Killed,
}

/// Owner scope for reading facts from a trigger run's thread.
///
/// The asking conversation's thread is deliberately excluded: a scheduled
/// run owns a different thread, while tenant/user/agent/project remain the
/// authorization boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerCapabilityCallFactsScope {
    pub tenant_id: TenantId,
    pub user_id: UserId,
    pub agent_id: Option<AgentId>,
    pub project_id: Option<ProjectId>,
}

impl TriggerCapabilityCallFactsScope {
    pub fn from_resource_scope(scope: &ResourceScope) -> Self {
        Self {
            tenant_id: scope.tenant_id.clone(),
            user_id: scope.user_id.clone(),
            agent_id: scope.agent_id.clone(),
            project_id: scope.project_id.clone(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TriggerCapabilityCallFactsError {
    #[error("trigger capability-call facts are unavailable")]
    Unavailable,
}

#[async_trait]
pub trait TriggerCapabilityCallFactsSource: Send + Sync {
    /// Return bounded, metadata-only facts for exactly `run_id`.
    async fn capability_calls_for_run(
        &self,
        scope: &TriggerCapabilityCallFactsScope,
        run_id: TurnRunId,
    ) -> Result<Vec<TriggerCapabilityCallFact>, TriggerCapabilityCallFactsError>;
}

#[derive(Debug, Default)]
pub struct MissingTriggerCapabilityCallFactsSource;

#[async_trait]
impl TriggerCapabilityCallFactsSource for MissingTriggerCapabilityCallFactsSource {
    async fn capability_calls_for_run(
        &self,
        _scope: &TriggerCapabilityCallFactsScope,
        _run_id: TurnRunId,
    ) -> Result<Vec<TriggerCapabilityCallFact>, TriggerCapabilityCallFactsError> {
        Err(TriggerCapabilityCallFactsError::Unavailable)
    }
}
