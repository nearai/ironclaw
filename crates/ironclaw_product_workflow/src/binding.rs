//! Conversation binding resolution service contract.
//!
//! Maps external adapter references (external actor, external conversation) to
//! canonical Reborn identifiers (tenant, user, thread, agent/project scope).
//! This replaces the ad-hoc session/thread resolution scattered across v1
//! `Agent::handle_message` and the engine-v2 bridge.

use async_trait::async_trait;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_product_adapters::{
    AdapterInstallationId, ExternalActorRef, ExternalConversationRef, ProductAdapterId,
    VerifiedAuthClaim,
};
use serde::{Deserialize, Serialize};

use crate::error::ProductWorkflowError;

/// Resolved canonical binding for a product inbound action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedBinding {
    pub tenant_id: TenantId,
    pub user_id: UserId,
    pub thread_id: ThreadId,
    /// Required for user-message turn submission because Reborn `ThreadScope`
    /// and `TurnScope` are agent-scoped. Product bindings that are only
    /// user-scoped must be completed before entering `InboundTurnService`.
    pub agent_id: Option<AgentId>,
    pub project_id: Option<ProjectId>,
}

/// Request to resolve external adapter refs into canonical Reborn bindings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveBindingRequest {
    pub adapter_id: ProductAdapterId,
    pub installation_id: AdapterInstallationId,
    pub external_actor_ref: ExternalActorRef,
    pub external_conversation_ref: ExternalConversationRef,
    pub auth_claim: VerifiedAuthClaim,
}

/// Conversation binding resolution contract. Host implementations wire this to
/// the tenant registry, user directory, and thread management services.
#[async_trait]
pub trait ConversationBindingService: Send + Sync {
    /// Resolve external adapter references to canonical Reborn identifiers.
    /// Implementations must create or look up the thread as needed.
    async fn resolve_binding(
        &self,
        request: ResolveBindingRequest,
    ) -> Result<ResolvedBinding, ProductWorkflowError>;
}
