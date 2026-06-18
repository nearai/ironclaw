use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TurnScope {
    pub tenant_id: TenantId,
    pub agent_id: Option<AgentId>,
    pub project_id: Option<ProjectId>,
    pub thread_id: ThreadId,
    #[serde(default, skip_serializing_if = "TurnThreadOwner::is_actor_fallback")]
    pub thread_owner: TurnThreadOwner,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "mode")]
pub enum TurnThreadOwner {
    #[default]
    ActorFallback,
    #[serde(alias = "explicit")]
    ExplicitUser {
        owner_user_id: UserId,
    },
    Ownerless,
}

impl TurnThreadOwner {
    pub fn explicit(owner_user_id: Option<UserId>) -> Self {
        match owner_user_id {
            Some(owner_user_id) => Self::ExplicitUser { owner_user_id },
            None => Self::Ownerless,
        }
    }

    fn is_actor_fallback(&self) -> bool {
        matches!(self, Self::ActorFallback)
    }

    pub fn explicit_owner_user_id(&self) -> Option<&UserId> {
        match self {
            Self::ExplicitUser { owner_user_id } => Some(owner_user_id),
            Self::ActorFallback | Self::Ownerless => None,
        }
    }

    pub fn is_explicit(&self) -> bool {
        !self.is_actor_fallback()
    }
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
            thread_owner: TurnThreadOwner::ActorFallback,
        }
    }

    pub fn new_with_owner(
        tenant_id: TenantId,
        agent_id: Option<AgentId>,
        project_id: Option<ProjectId>,
        thread_id: ThreadId,
        owner_user_id: Option<UserId>,
    ) -> Self {
        Self {
            tenant_id,
            agent_id,
            project_id,
            thread_id,
            thread_owner: TurnThreadOwner::explicit(owner_user_id),
        }
    }

    pub fn explicit_owner_user_id(&self) -> Option<&UserId> {
        self.thread_owner.explicit_owner_user_id()
    }

    pub fn has_explicit_thread_owner(&self) -> bool {
        self.thread_owner.is_explicit()
    }

    /// Owner for product-context: explicit thread owner → Personal, else agent-scoped, else actor.
    pub fn product_owner(&self, actor: &TurnActor) -> crate::TurnOwner {
        if let Some(user) = self.explicit_owner_user_id() {
            crate::TurnOwner::Personal { user: user.clone() }
        } else if let Some(agent) = &self.agent_id {
            crate::TurnOwner::SharedAgent {
                agent: agent.clone(),
                project: self.project_id.clone(),
            }
        } else {
            crate::TurnOwner::Personal {
                user: actor.user_id.clone(),
            }
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
        scope.user_id = self.explicit_owner_user_id().cloned().unwrap_or_else(|| {
            UserId::from_trusted(ironclaw_host_api::SYSTEM_RESERVED_ID.to_string())
        });
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_actor(user: &str) -> TurnActor {
        TurnActor::new(UserId::from_trusted(user.to_string()))
    }

    fn make_scope_with_explicit_owner(owner: Option<UserId>) -> TurnScope {
        TurnScope::new_with_owner(
            ironclaw_host_api::TenantId::from_trusted("tenant:t".to_string()),
            None,
            None,
            ironclaw_host_api::ThreadId::from_trusted("thread:t".to_string()),
            owner,
        )
    }

    fn make_scope_with_agent(
        agent_id: ironclaw_host_api::AgentId,
        project_id: Option<ironclaw_host_api::ProjectId>,
    ) -> TurnScope {
        TurnScope::new(
            ironclaw_host_api::TenantId::from_trusted("tenant:t".to_string()),
            Some(agent_id),
            project_id,
            ironclaw_host_api::ThreadId::from_trusted("thread:t".to_string()),
        )
    }

    #[test]
    fn product_owner_prefers_explicit_then_agent_then_actor() {
        let actor = make_actor("user:actor");

        // Branch 1: explicit owner user wins.
        let explicit_owner = UserId::from_trusted("user:explicit".to_string());
        let scope = make_scope_with_explicit_owner(Some(explicit_owner.clone()));
        assert_eq!(
            scope.product_owner(&actor),
            crate::TurnOwner::Personal {
                user: explicit_owner
            }
        );

        // Branch 2: no explicit owner but agent_id → SharedAgent.
        let agent = ironclaw_host_api::AgentId::from_trusted("agent:bot".to_string());
        let project = ironclaw_host_api::ProjectId::from_trusted("project:p".to_string());
        let scope = make_scope_with_agent(agent.clone(), Some(project.clone()));
        assert_eq!(
            scope.product_owner(&actor),
            crate::TurnOwner::SharedAgent {
                agent,
                project: Some(project)
            }
        );

        // Branch 3: no explicit owner, no agent → fallback to actor.
        let scope = make_scope_with_explicit_owner(None);
        assert_eq!(
            scope.product_owner(&actor),
            crate::TurnOwner::Personal {
                user: actor.user_id.clone()
            }
        );
    }

    #[test]
    fn turn_scope_accepts_legacy_explicit_thread_owner_mode() {
        let scope: TurnScope = serde_json::from_value(serde_json::json!({
            "tenant_id": "tenant:slack",
            "agent_id": "agent:slack",
            "project_id": "project:slack",
            "thread_id": "thread:slack",
            "thread_owner": {
                "mode": "explicit",
                "owner_user_id": "user:slack-subject"
            }
        }))
        .expect("legacy explicit owner mode should deserialize");

        assert_eq!(
            scope.explicit_owner_user_id().map(UserId::as_str),
            Some("user:slack-subject")
        );
        assert_eq!(
            serde_json::to_value(&scope.thread_owner).expect("serialize owner"),
            serde_json::json!({
                "mode": "explicit_user",
                "owner_user_id": "user:slack-subject"
            })
        );
    }
}
