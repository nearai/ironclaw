//! Conversation binding resolution — the port and the DTOs that cross it.
//!
//! Maps external adapter references (external actor, external conversation) to
//! canonical Reborn identifiers (tenant, user, thread, agent/project scope).
//!
//! **Why this is a contract and not product's own** (§12.11 D-A): the channel
//! host's per-extension workflow factory returns a live binding service to its
//! caller, and the caller sits *below* product. The port therefore has to be
//! declared at the boundary, which is only possible now that its request and
//! response DTOs name nothing but `ironclaw_host_api` and
//! `ironclaw_extension_contracts` vocabulary. The error is the boundary error
//! ([`ProductOperationFailure`]) for the same reason; `ironclaw_assistant`
//! absorbs it into its workflow error with a total, discriminant-preserving
//! `From`, so a product call site keeps every distinction it had.

use async_trait::async_trait;
use ironclaw_extension_contracts::channel_adapter::ProductTriggerReason;
use ironclaw_extension_contracts::external::{
    ExternalActorRef, ExternalConversationRef, ExternalEventId,
};
use ironclaw_host_api::ids::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_host_api::product_adapter::{
    AdapterInstallationId, ProductAdapterId, VerifiedAuthClaim,
};
use serde::{Deserialize, Serialize};

use crate::error::ProductOperationFailure;
use crate::inbound::{ProductInboundEnvelope, ProductInboundPayload};

/// Resolved canonical binding for a product inbound action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedBinding {
    pub tenant_id: TenantId,
    /// Real paired human actor who sent or authorized the external action.
    ///
    /// The `user_id` alias is a sanctioned one-time wire-fold for persisted
    /// binding rows written before the actor/subject split — a durable-data
    /// migration concern, not a runtime compatibility path (new
    /// serializations emit `actor_user_id` only).
    #[serde(alias = "user_id")]
    pub actor_user_id: UserId,
    /// User scope whose agent/context/tools/memory execute the turn.
    ///
    /// Direct/personal routes set this to the actor. Shared routes set this to
    /// the configured team/agent subject; routes without an explicit subject
    /// are rejected before turn submission.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_user_id: Option<UserId>,
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
    pub external_event_id: ExternalEventId,
    pub route_kind: ProductConversationRouteKind,
    pub auth_claim: VerifiedAuthClaim,
}

/// Atomically rotate an existing external conversation route to a fresh
/// canonical thread. The expected id fences stale concurrent reset attempts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResetBindingRequest {
    pub resolve_request: ResolveBindingRequest,
    pub expected_thread_id: ThreadId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResetBindingOutcome {
    pub previous_thread_id: ThreadId,
    pub binding: ResolvedBinding,
}

/// Stable route-access shape for product bindings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductConversationRouteKind {
    /// One external actor owns the external conversation route.
    Direct,
    /// A shared channel/group route where allowed participants may post.
    Shared,
}

/// Whether an inbound user message may create a new product conversation
/// binding, or must target a conversation that the product has already linked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProductConversationBindingCreationPolicy {
    CreateAllowed,
    ExistingOnly,
}

impl ResolveBindingRequest {
    pub fn from_envelope(envelope: &ProductInboundEnvelope) -> Self {
        Self {
            adapter_id: envelope.adapter_id().clone(),
            installation_id: envelope.installation_id().clone(),
            external_actor_ref: envelope.external_actor_ref().clone(),
            external_conversation_ref: envelope.external_conversation_ref().clone(),
            external_event_id: envelope.external_event_id().clone(),
            route_kind: route_kind_for_inbound_payload(envelope.payload()),
            auth_claim: envelope.auth_claim().clone(),
        }
    }
}

pub fn binding_profile_for_trigger(
    trigger: ProductTriggerReason,
) -> (
    ProductConversationRouteKind,
    ProductConversationBindingCreationPolicy,
) {
    match trigger {
        ProductTriggerReason::DirectChat => (
            ProductConversationRouteKind::Direct,
            ProductConversationBindingCreationPolicy::CreateAllowed,
        ),
        ProductTriggerReason::BotMention | ProductTriggerReason::BotCommand => (
            ProductConversationRouteKind::Shared,
            ProductConversationBindingCreationPolicy::CreateAllowed,
        ),
        // Reply/action callbacks refer to a prior bot turn by definition, so
        // they are shared routes that must already have a conversation binding.
        ProductTriggerReason::ReplyToBot | ProductTriggerReason::LinkedThreadAction => (
            ProductConversationRouteKind::Shared,
            ProductConversationBindingCreationPolicy::ExistingOnly,
        ),
    }
}

pub fn route_kind_for_inbound_payload(
    payload: &ProductInboundPayload,
) -> ProductConversationRouteKind {
    match payload {
        ProductInboundPayload::UserMessage(message) => route_kind_for_trigger(message.trigger),
        ProductInboundPayload::Command(command) => route_kind_for_trigger(command.trigger),
        ProductInboundPayload::ApprovalResolution(resolution) => resolution
            .source_trigger
            .map(route_kind_for_trigger)
            .unwrap_or(ProductConversationRouteKind::Direct),
        ProductInboundPayload::ScopedApprovalResolution(resolution) => resolution
            .source_trigger
            .map(route_kind_for_trigger)
            .unwrap_or(ProductConversationRouteKind::Direct),
        ProductInboundPayload::AuthResolution(resolution) => resolution
            .source_trigger
            .map(route_kind_for_trigger)
            .unwrap_or(ProductConversationRouteKind::Direct),
        ProductInboundPayload::ProjectionRead(_)
        | ProductInboundPayload::SubscriptionRequest(_)
        | ProductInboundPayload::ControlAction(_)
        | ProductInboundPayload::LinkedThreadAction(_)
        | ProductInboundPayload::NoOp => ProductConversationRouteKind::Direct,
    }
}

pub fn route_kind_for_trigger(trigger: ProductTriggerReason) -> ProductConversationRouteKind {
    binding_profile_for_trigger(trigger).0
}

/// Conversation binding resolution contract. Host implementations wire this to
/// the tenant registry, user directory, and thread management services.
#[async_trait]
pub trait ProductBindingResolver: Send + Sync {
    /// Resolve external adapter references to canonical Reborn identifiers.
    /// Implementations must create or look up the thread as needed.
    async fn resolve_binding(
        &self,
        request: ResolveBindingRequest,
    ) -> Result<ResolvedBinding, ProductOperationFailure>;

    /// Look up an existing binding without creating conversation/thread state.
    async fn lookup_binding(
        &self,
        request: ResolveBindingRequest,
    ) -> Result<ResolvedBinding, ProductOperationFailure>;

    /// Reset is fail-closed by default so test doubles and custom hosts cannot
    /// silently claim a route was rotated without durable binding support.
    async fn reset_binding(
        &self,
        _request: ResetBindingRequest,
    ) -> Result<ResetBindingOutcome, ProductOperationFailure> {
        Err(ProductOperationFailure::BindingResolutionFailed {
            reason: "conversation binding reset is not supported".to_string(),
        })
    }
}

#[async_trait]
impl<T> ProductBindingResolver for std::sync::Arc<T>
where
    T: ProductBindingResolver + ?Sized,
{
    async fn resolve_binding(
        &self,
        request: ResolveBindingRequest,
    ) -> Result<ResolvedBinding, ProductOperationFailure> {
        self.as_ref().resolve_binding(request).await
    }

    async fn lookup_binding(
        &self,
        request: ResolveBindingRequest,
    ) -> Result<ResolvedBinding, ProductOperationFailure> {
        self.as_ref().lookup_binding(request).await
    }

    async fn reset_binding(
        &self,
        request: ResetBindingRequest,
    ) -> Result<ResetBindingOutcome, ProductOperationFailure> {
        self.as_ref().reset_binding(request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolved_binding_accepts_legacy_user_id_actor_field() {
        let binding: ResolvedBinding = serde_json::from_value(serde_json::json!({
            "tenant_id": "tenant:legacy",
            "user_id": "user:legacy-actor",
            "subject_user_id": "user:legacy-subject",
            "thread_id": "thread:legacy",
            "agent_id": "agent:legacy",
            "project_id": "project:legacy"
        }))
        .expect("legacy binding should deserialize");

        assert_eq!(binding.actor_user_id.as_str(), "user:legacy-actor");
        assert_eq!(
            binding.subject_user_id.as_ref().map(UserId::as_str),
            Some("user:legacy-subject")
        );
    }
}
