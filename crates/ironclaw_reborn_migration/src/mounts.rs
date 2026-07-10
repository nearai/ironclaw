//! Production-owned mount resolution for migration writers.
//!
//! Migration must never reproduce Reborn's virtual-path layout. Every scoped
//! writer delegates to composition's canonical resolver so a cold production
//! runtime reopens the exact tenant/user paths written here.

use ironclaw_host_api::{HostApiError, MountView, ResourceScope};

pub(crate) fn production_mount_view(scope: &ResourceScope) -> Result<MountView, HostApiError> {
    ironclaw_reborn_composition::invocation_mount_view(scope)
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::{AgentId, InvocationId, ResourceScope, TenantId, UserId};

    use super::production_mount_view;

    #[test]
    fn migration_mount_view_is_the_production_mount_view() {
        let scope = ResourceScope {
            tenant_id: TenantId::new("tenant-a").unwrap(),
            user_id: UserId::new("user-a").unwrap(),
            agent_id: Some(AgentId::new("agent-a").unwrap()),
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        };

        assert_eq!(
            production_mount_view(&scope).unwrap(),
            ironclaw_reborn_composition::invocation_mount_view(&scope).unwrap()
        );
    }
}
