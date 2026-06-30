//! Projection read/subscription contracts.

use async_trait::async_trait;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_turns::{TurnActor, TurnScope};
use serde::{Deserialize, Serialize};

use crate::auth::VerifiedAuthClaim;
use crate::error::ProductAdapterError;
use crate::external::{ExternalActorRef, ExternalConversationRef, ExternalEventId};
use crate::identity::{AdapterInstallationId, ProductAdapterId};
use crate::inbound::{ProductInboundEnvelope, ProductInboundPayload};
use crate::outbound::{ProductOutboundEnvelope, ProjectionCursor};
use crate::redaction::RedactedString;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductProjectionReadInput {
    pub subject: ProductProjectionSubject,
    pub thread_id_hint: Option<String>,
    pub after_cursor: Option<ProjectionCursor>,
    pub limit: Option<u16>,
}

impl ProductProjectionReadInput {
    pub fn new(
        subject: ProductProjectionSubject,
        thread_id_hint: Option<String>,
        after_cursor: Option<ProjectionCursor>,
        limit: Option<u16>,
    ) -> Self {
        Self {
            subject,
            thread_id_hint,
            after_cursor,
            limit,
        }
    }

    pub fn from_inbound_envelope(
        envelope: &ProductInboundEnvelope,
    ) -> Result<Self, ProductAdapterError> {
        let ProductInboundPayload::ProjectionRead(payload) = envelope.payload() else {
            return Err(ProductAdapterError::MalformedInboundPayload {
                reason: RedactedString::new(
                    "projection read resolution requires projection_read payload",
                ),
            });
        };
        Ok(Self {
            subject: ProductProjectionSubject::from_inbound_envelope(envelope),
            thread_id_hint: payload.thread_id_hint.clone(),
            after_cursor: payload.after_cursor.clone(),
            limit: payload.limit,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductProjectionSubscribeInput {
    pub subject: ProductProjectionSubject,
    pub thread_id_hint: Option<String>,
    pub after_cursor: Option<ProjectionCursor>,
}

impl ProductProjectionSubscribeInput {
    pub fn new(
        subject: ProductProjectionSubject,
        thread_id_hint: Option<String>,
        after_cursor: Option<ProjectionCursor>,
    ) -> Self {
        Self {
            subject,
            thread_id_hint,
            after_cursor,
        }
    }

    pub fn from_inbound_envelope(
        envelope: &ProductInboundEnvelope,
    ) -> Result<Self, ProductAdapterError> {
        let ProductInboundPayload::SubscriptionRequest(payload) = envelope.payload() else {
            return Err(ProductAdapterError::MalformedInboundPayload {
                reason: RedactedString::new(
                    "projection subscription resolution requires subscription_request payload",
                ),
            });
        };
        Ok(Self {
            subject: ProductProjectionSubject::from_inbound_envelope(envelope),
            thread_id_hint: payload.thread_id_hint.clone(),
            after_cursor: payload.after_cursor.clone(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProductProjectionSubject {
    /// Adapter/channel style request: ProductWorkflow resolves canonical
    /// actor/scope/thread from verified installation + external refs.
    AdapterExternalRefs {
        adapter_id: ProductAdapterId,
        installation_id: AdapterInstallationId,
        external_event_id: ExternalEventId,
        external_actor_ref: ExternalActorRef,
        external_conversation_ref: ExternalConversationRef,
        auth_claim: VerifiedAuthClaim,
    },

    /// API/projection style request: caller has already resolved an opaque
    /// product/API id such as OpenAI `resp_*` into canonical Reborn metadata.
    /// ProductWorkflow remains the facade door, but does not own that mapping.
    CanonicalProjection { actor: TurnActor, scope: TurnScope },
}

impl ProductProjectionSubject {
    pub fn from_inbound_envelope(envelope: &ProductInboundEnvelope) -> Self {
        Self::AdapterExternalRefs {
            adapter_id: envelope.adapter_id().clone(),
            installation_id: envelope.installation_id().clone(),
            external_event_id: envelope.external_event_id().clone(),
            external_actor_ref: envelope.external_actor_ref().clone(),
            external_conversation_ref: envelope.external_conversation_ref().clone(),
            auth_claim: envelope.auth_claim().clone(),
        }
    }

    pub fn canonical(actor: TurnActor, scope: TurnScope) -> Self {
        Self::CanonicalProjection { actor, scope }
    }

    pub fn canonical_thread_scope(
        actor_user_id: UserId,
        tenant_id: TenantId,
        agent_id: Option<AgentId>,
        project_id: Option<ProjectId>,
        thread_id: ThreadId,
        owner_user_id: Option<UserId>,
    ) -> Self {
        Self::CanonicalProjection {
            actor: TurnActor::new(actor_user_id),
            scope: TurnScope::new_with_owner(
                tenant_id,
                agent_id,
                project_id,
                thread_id,
                owner_user_id,
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProjectionReadRequest {
    pub actor: TurnActor,
    pub scope: TurnScope,
    pub after_cursor: Option<ProjectionCursor>,
    pub limit: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProjectionSubscriptionRequest {
    pub actor: TurnActor,
    pub scope: TurnScope,
    pub after_cursor: Option<ProjectionCursor>,
}

#[async_trait]
pub trait ProjectionStream: Send + Sync {
    async fn drain(
        &self,
        request: ProjectionSubscriptionRequest,
    ) -> Result<Vec<ProductOutboundEnvelope>, ProductAdapterError>;
}
