//! Execution scope contracts.
//!
//! [`ExecutionContext`] is the authority envelope for one invocation. It ties
//! together identity, tenancy, optional process/thread/mission/project context,
//! runtime/trust class, capability grants, mount view, resource scope, and
//! correlation ID. Every filesystem, resource, secret, network, dispatch, spawn,
//! and audit decision should be traceable back to this context.

use serde::{Deserialize, Serialize};

use crate::{
    AgentId, CapabilitySet, CorrelationId, ExtensionId, HostApiError, InvocationId, MissionId,
    MountView, ProcessId, ProjectId, ResourceScope, RunId, RuntimeKind, SystemServiceId, TenantId,
    ThreadId, TrustClass, UserId,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "id")]
pub enum Principal {
    Tenant(TenantId),
    User(UserId),
    Agent(AgentId),
    Project(ProjectId),
    Mission(MissionId),
    Thread(ThreadId),
    Extension(ExtensionId),
    /// Host runtime internals acting on their own behalf. Never match this as a grantable userland principal.
    HostRuntime,
    /// Named trusted system service, such as heartbeat, routine engine, or migration runner.
    System(SystemServiceId),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionContext {
    pub invocation_id: InvocationId,
    pub correlation_id: CorrelationId,
    pub process_id: Option<ProcessId>,
    pub parent_process_id: Option<ProcessId>,

    pub tenant_id: TenantId,
    pub user_id: UserId,
    /// Authenticated human actor sealed by trusted ingress/loop orchestration.
    ///
    /// This is intentionally distinct from `user_id`, which identifies the
    /// resource subject. Untrusted and system-created contexts leave it unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authenticated_actor_user_id: Option<UserId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
    pub project_id: Option<ProjectId>,
    pub mission_id: Option<MissionId>,
    pub thread_id: Option<ThreadId>,
    /// Prompt-visible run identity for the loop turn-run this invocation
    /// belongs to, sitting between `thread_id` (spans many runs) and
    /// `invocation_id` (one tool call) in the scope cascade.
    ///
    /// Stamped host-side by loop orchestration when it builds the invocation
    /// context (see `invocation_context_from_visible` in
    /// `ironclaw_loop_support`); tool calls within the same run share it.
    /// `None` for non-loop callers (system services, one-shot product
    /// invocations). Policy layers that require "within the current run"
    /// continuity (e.g. coding read-before-edit) key on it; consumers must
    /// treat `None` as its own bucket, never as a wildcard.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<RunId>,

    pub extension_id: ExtensionId,
    pub runtime: RuntimeKind,
    pub trust: TrustClass,

    pub grants: CapabilitySet,
    pub mounts: MountView,
    pub resource_scope: ResourceScope,
}

impl ExecutionContext {
    /// Build a local/single-user execution context using the canonical default
    /// tenant, agent, and bootstrap project.
    ///
    /// Callers still supply extension/runtime/trust/grants/mounts because those
    /// are product-workflow decisions; this helper only normalizes local scope.
    pub fn local_default(
        user_id: UserId,
        extension_id: ExtensionId,
        runtime: RuntimeKind,
        trust: TrustClass,
        grants: CapabilitySet,
        mounts: MountView,
    ) -> Result<Self, HostApiError> {
        let invocation_id = InvocationId::new();
        let resource_scope = ResourceScope::local_default(user_id.clone(), invocation_id)?;
        let context = Self {
            invocation_id,
            correlation_id: CorrelationId::new(),
            process_id: None,
            parent_process_id: None,
            tenant_id: resource_scope.tenant_id.clone(),
            user_id,
            authenticated_actor_user_id: None,
            agent_id: resource_scope.agent_id.clone(),
            project_id: resource_scope.project_id.clone(),
            mission_id: None,
            thread_id: None,
            run_id: None,
            extension_id,
            runtime,
            trust,
            grants,
            mounts,
            resource_scope,
        };
        context.validate()?;
        Ok(context)
    }

    pub fn validate(&self) -> Result<(), HostApiError> {
        if self.resource_scope.invocation_id != self.invocation_id {
            return Err(HostApiError::invariant(
                "resource_scope.invocation_id must match execution context invocation_id",
            ));
        }
        if self.resource_scope.tenant_id != self.tenant_id {
            return Err(HostApiError::invariant(
                "resource_scope.tenant_id must match execution context tenant_id",
            ));
        }
        if self.resource_scope.user_id != self.user_id {
            return Err(HostApiError::invariant(
                "resource_scope.user_id must match execution context user_id",
            ));
        }
        if self.resource_scope.agent_id != self.agent_id {
            return Err(HostApiError::invariant(
                "resource_scope.agent_id must match execution context agent_id",
            ));
        }
        if self.resource_scope.project_id != self.project_id {
            return Err(HostApiError::invariant(
                "resource_scope.project_id must match execution context project_id",
            ));
        }
        if self.resource_scope.mission_id != self.mission_id {
            return Err(HostApiError::invariant(
                "resource_scope.mission_id must match execution context mission_id",
            ));
        }
        if self.resource_scope.thread_id != self.thread_id {
            return Err(HostApiError::invariant(
                "resource_scope.thread_id must match execution context thread_id",
            ));
        }
        self.mounts.validate()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_execution_context_without_optional_identity_fields_deserializes() {
        let mut context = ExecutionContext::local_default(
            UserId::new("subject").unwrap(),
            ExtensionId::new("demo").unwrap(),
            RuntimeKind::Script,
            TrustClass::Sandbox,
            CapabilitySet::default(),
            MountView::default(),
        )
        .unwrap();
        context.authenticated_actor_user_id = Some(UserId::new("slack-alice").unwrap());
        context.run_id = Some(RunId::new());
        let mut legacy = serde_json::to_value(context).unwrap();
        let fields = legacy.as_object_mut().unwrap();
        fields.remove("authenticated_actor_user_id");
        fields.remove("run_id");

        let decoded: ExecutionContext = serde_json::from_value(legacy).unwrap();

        assert_eq!(decoded.authenticated_actor_user_id, None);
        assert_eq!(decoded.run_id, None);
    }
}
