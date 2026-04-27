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
    MountView, ProcessId, ProjectId, ResourceScope, RuntimeKind, TenantId, ThreadId, TrustClass,
    UserId,
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
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionContext {
    pub invocation_id: InvocationId,
    pub correlation_id: CorrelationId,
    pub process_id: Option<ProcessId>,
    pub parent_process_id: Option<ProcessId>,

    pub tenant_id: TenantId,
    pub user_id: UserId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
    pub project_id: Option<ProjectId>,
    pub mission_id: Option<MissionId>,
    pub thread_id: Option<ThreadId>,

    pub extension_id: ExtensionId,
    pub runtime: RuntimeKind,
    pub trust: TrustClass,

    pub grants: CapabilitySet,
    pub mounts: MountView,
    pub resource_scope: ResourceScope,
}

impl ExecutionContext {
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
