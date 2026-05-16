//! Identity inputs for the assembled Reborn runtime.
//!
//! `RebornIdentityConfig` carries the tenant / agent / owner-user scope the
//! runtime acts under by default. Per-conversation overrides are supported
//! via `RebornRuntime::new_conversation_for(ConversationIdentity)`.
//!
//! Today the composition root reads these values directly because no
//! tenant repo / identity service exists yet. When epic #3036
//! ("Configuration-as-Code") lands its tenant/blueprint substrate, the
//! same DTO will be sourced from a `RebornTenantRepo` lookup keyed by
//! deployment, and the CLI's `RebornIdentityConfig::cli_default` becomes
//! a single special case rather than the default path.

use ironclaw_host_api::{AgentId, HostApiError, ProjectId, TenantId, UserId};

/// Default identity used for newly-created conversations on the runtime.
///
/// Stable identifier values let repeated boots produce comparable
/// `TurnScope`/`ThreadScope` values, which keeps audit traces and turn
/// idempotency keys consistent across restarts of the same CLI session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebornIdentityConfig {
    pub tenant: TenantId,
    pub default_agent: AgentId,
    pub default_owner: UserId,
    /// Optional project scope applied to every conversation that doesn't
    /// override it. `None` means "no project binding".
    pub default_project: Option<ProjectId>,
}

impl RebornIdentityConfig {
    pub fn new(
        tenant: TenantId,
        default_agent: AgentId,
        default_owner: UserId,
    ) -> Self {
        Self {
            tenant,
            default_agent,
            default_owner,
            default_project: None,
        }
    }

    pub fn with_default_project(mut self, project: ProjectId) -> Self {
        self.default_project = Some(project);
        self
    }

    /// Stable single-tenant local-dev identity used by the standalone CLI
    /// when no operator-supplied identity is configured. Once the tenant
    /// repo (epic #3036) is wired, the CLI binds to a real tenant on
    /// first-run and this default becomes the "no tenant configured"
    /// fail-closed message instead.
    pub fn cli_default() -> Result<Self, HostApiError> {
        Ok(Self::new(
            TenantId::new("reborn-cli")?,
            AgentId::new("reborn-cli-agent")?,
            UserId::new("reborn-cli")?,
        ))
    }
}

/// Per-conversation identity override. Any field left `None` falls back
/// to the corresponding `RebornIdentityConfig` default.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConversationIdentity {
    pub tenant: Option<TenantId>,
    pub agent: Option<AgentId>,
    pub owner: Option<UserId>,
    pub project: Option<ProjectId>,
}

impl ConversationIdentity {
    pub fn for_tenant(tenant: TenantId) -> Self {
        Self {
            tenant: Some(tenant),
            ..Self::default()
        }
    }

    pub fn with_agent(mut self, agent: AgentId) -> Self {
        self.agent = Some(agent);
        self
    }

    pub fn with_owner(mut self, owner: UserId) -> Self {
        self.owner = Some(owner);
        self
    }

    pub fn with_project(mut self, project: ProjectId) -> Self {
        self.project = Some(project);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_default_constructs() {
        let identity =
            RebornIdentityConfig::cli_default().expect("CLI default identity must construct");
        assert_eq!(identity.tenant.as_str(), "reborn-cli");
        assert_eq!(identity.default_agent.as_str(), "reborn-cli-agent");
        assert_eq!(identity.default_owner.as_str(), "reborn-cli");
        assert!(identity.default_project.is_none());
    }

    #[test]
    fn conversation_identity_builders_compose() {
        let tenant = TenantId::new("acme").expect("tenant id");
        let agent = AgentId::new("acme-bot").expect("agent id");
        let project = ProjectId::new("acme-platform").expect("project id");
        let id = ConversationIdentity::for_tenant(tenant.clone())
            .with_agent(agent.clone())
            .with_project(project.clone());
        assert_eq!(id.tenant.as_ref(), Some(&tenant));
        assert_eq!(id.agent.as_ref(), Some(&agent));
        assert_eq!(id.project.as_ref(), Some(&project));
        assert!(id.owner.is_none());
    }
}
