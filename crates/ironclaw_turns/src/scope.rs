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
