use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TurnScope {
    pub tenant_id: TenantId,
    pub agent_id: Option<AgentId>,
    pub project_id: Option<ProjectId>,
    pub thread_id: ThreadId,
}

impl TurnScope {
    pub fn new(
        tenant_id: TenantId,
        agent_id: Option<AgentId>,
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

    /// Convert into a [`ironclaw_host_api::ResourceScope`] for filesystem
    /// resolver lookup. `user_id` falls back to the system sentinel when
    /// the turn scope is anchored at tenant level without a specific owner.
    /// The invocation id is filled with a fresh value from
    /// `ResourceScope::system()` — storage dispatch only needs the tenant /
    /// agent / project / thread axes, and the architecture boundary forbids
    /// this crate from naming lower runtime identifiers directly.
    pub fn to_resource_scope(&self) -> ironclaw_host_api::ResourceScope {
        let mut scope = ironclaw_host_api::ResourceScope::system();
        scope.tenant_id = self.tenant_id.clone();
        scope.user_id = UserId::from_trusted(ironclaw_host_api::SYSTEM_RESERVED_ID.to_string());
        scope.agent_id = self.agent_id.clone();
        scope.project_id = self.project_id.clone();
        scope.mission_id = None;
        scope.thread_id = Some(self.thread_id.clone());
        scope
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
