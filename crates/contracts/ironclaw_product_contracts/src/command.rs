//! The authority-bearing command context and the actor-role admission port
//! (PROPOSAL §6.1.3).
//!
//! [`ProductCommandContext`] is what a channel host hands the product surface
//! when an inbound message turns out to be a command: the verified claim, the
//! external refs it arrived on, and the action identity it is deduplicated by.
//! It crosses the boundary in both directions — product builds it from an
//! envelope, and `ironclaw_extension_host` reads it to resolve the bound
//! user's admin role through [`CommandActorRoleResolver`].
//!
//! Never here: the command grammar (`ProductCommand` and the declared command
//! inventory stay with product's frozen surface), admission policy, or any
//! resolver implementation.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_extension_contracts::channel_adapter::ProductTriggerReason;
use ironclaw_extension_contracts::external::{ExternalActorRef, ExternalConversationRef};
use ironclaw_host_api::product_adapter::{
    AdapterInstallationId, ProductAdapterId, VerifiedAuthClaim,
};
use serde::Serialize;

use crate::action::{ActionFingerprintKey, ProductActionId};
use crate::admin_users::AdminUserRole;
use crate::inbound::{ProductInboundEnvelope, ProductInboundPayload};
use crate::surface::{ProductSurfaceError, ProductSurfaceErrorCode};

/// Authority-bearing command dispatch context built by the workflow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProductCommandContext {
    pub action_id: ProductActionId,
    pub fingerprint: ActionFingerprintKey,
    /// Exact raw inbound command token, verbatim from the payload.
    pub requested_command: String,
    pub adapter_id: ProductAdapterId,
    pub installation_id: AdapterInstallationId,
    pub external_actor_ref: ExternalActorRef,
    pub external_conversation_ref: ExternalConversationRef,
    pub auth_claim: VerifiedAuthClaim,
    pub trigger: ProductTriggerReason,
    pub received_at: DateTime<Utc>,
}

impl ProductCommandContext {
    pub fn from_envelope(
        envelope: &ProductInboundEnvelope,
        action_id: ProductActionId,
        fingerprint: ActionFingerprintKey,
    ) -> Result<Self, ProductSurfaceError> {
        let ProductInboundPayload::Command(command) = envelope.payload() else {
            return Err(ProductSurfaceError::from_status(
                ProductSurfaceErrorCode::InvalidRequest,
                400,
                false,
            ));
        };
        // Channel commands are a webhook-ingress concern; a session envelope
        // carries no verified claim and must not reach command dispatch.
        let Some(auth_claim) = envelope.auth_claim().cloned() else {
            return Err(ProductSurfaceError::from_status(
                ProductSurfaceErrorCode::InvalidRequest,
                400,
                false,
            ));
        };
        Ok(Self {
            action_id,
            fingerprint,
            requested_command: command.command.clone(),
            adapter_id: envelope.adapter_id().clone(),
            installation_id: envelope.installation_id().clone(),
            external_actor_ref: envelope.external_actor_ref().clone(),
            external_conversation_ref: envelope.external_conversation_ref().clone(),
            auth_claim,
            trigger: command.trigger,
            received_at: envelope.received_at(),
        })
    }
}

/// Resolves the admin-boundary role of the ACTIVE bound user behind an
/// inbound channel actor. `Ok(None)` means unbound actor, missing record, or
/// suspended account — all treated as not-admin (fail closed). `Err` means
/// transient resolution failure; the command fails retryable rather than
/// silently degrading to member or admin treatment.
#[async_trait]
pub trait CommandActorRoleResolver: Send + Sync {
    async fn actor_role(
        &self,
        context: &ProductCommandContext,
    ) -> Result<Option<AdminUserRole>, ProductSurfaceError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use ironclaw_extension_contracts::external::{
        ExternalActorRef, ExternalConversationRef, ExternalEventId,
    };
    use ironclaw_host_api::product_adapter::ProtocolAuthEvidence;
    use ironclaw_host_api::product_adapter::auth::AuthRequirement;

    use crate::action::{ActionFingerprintKey, ProductActionId, SourceBindingKey};
    use crate::inbound::{
        InboundCommandPayload, ParsedProductInbound, TrustedInboundContext, UserMessagePayload,
    };

    fn envelope(payload: ProductInboundPayload) -> ProductInboundEnvelope {
        let evidence = ProtocolAuthEvidence::test_verified(
            AuthRequirement::SharedSecretHeader {
                header_name: "X-Slack-Signature".into(),
            },
            "install_alpha",
        );
        let context = TrustedInboundContext::from_verified_evidence(
            ProductAdapterId::new("slack").expect("valid adapter"),
            AdapterInstallationId::new("install_alpha").expect("valid installation"),
            Utc::now(),
            &evidence,
        )
        .expect("verified evidence");
        let parsed = ParsedProductInbound::new(
            ExternalEventId::new("evt:1").expect("valid event"),
            ExternalActorRef::new("slack_user", "U1", Option::<String>::None).expect("valid actor"),
            ExternalConversationRef::new(None, "C1", None, None).expect("valid conversation"),
            payload,
        )
        .expect("parsed");
        ProductInboundEnvelope::from_trusted_parse(context, parsed).expect("envelope")
    }

    fn fingerprint() -> ActionFingerprintKey {
        ActionFingerprintKey::new(
            ProductAdapterId::new("slack").expect("valid adapter"),
            AdapterInstallationId::new("install_alpha").expect("valid installation"),
            ExternalActorRef::new("slack_user", "U1", Option::<String>::None).expect("valid actor"),
            SourceBindingKey::new("space:0:;conversation:2:C1;topic:0:;").expect("valid binding"),
            ExternalEventId::new("evt:1").expect("valid event"),
        )
    }

    #[test]
    fn command_context_is_built_from_a_command_envelope_verbatim() {
        let envelope = envelope(ProductInboundPayload::Command(
            InboundCommandPayload::new("status", "--json", ProductTriggerReason::DirectChat)
                .expect("payload"),
        ));
        let action_id = ProductActionId::new();
        let context = ProductCommandContext::from_envelope(&envelope, action_id, fingerprint())
            .expect("built");

        assert_eq!(context.requested_command, "status");
        assert_eq!(context.action_id, action_id);
        assert_eq!(context.adapter_id.as_str(), "slack");
        assert_eq!(context.installation_id.as_str(), "install_alpha");
        assert_eq!(context.trigger, ProductTriggerReason::DirectChat);
        assert_eq!(context.received_at, envelope.received_at());
    }

    #[test]
    fn non_command_envelope_is_rejected_as_an_invalid_request_not_an_internal_error() {
        let envelope = envelope(ProductInboundPayload::UserMessage(
            UserMessagePayload::new("hello", vec![], ProductTriggerReason::DirectChat)
                .expect("payload"),
        ));
        let error =
            ProductCommandContext::from_envelope(&envelope, ProductActionId::new(), fingerprint())
                .expect_err("a user message is not a command");
        assert_eq!(error.code, ProductSurfaceErrorCode::InvalidRequest);
    }
}
