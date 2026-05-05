use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TurnScope {
    pub tenant_id: TenantId,
    pub agent_id: AgentId,
    pub project_id: Option<ProjectId>,
    pub thread_id: ThreadId,
}

impl TurnScope {
    pub fn new(
        tenant_id: TenantId,
        agent_id: AgentId,
        project_id: Option<ProjectId>,
        thread_id: ThreadId,
    ) -> Self {
        Self {
            tenant_id,
            agent_id,
            project_id,
            thread_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TurnActor {
    pub user_id: UserId,
}

impl TurnActor {
    pub fn new(user_id: UserId) -> Self {
        Self { user_id }
    }
}
