//! The owner-scoped "Web app" delivery-target catalog provider.
//!
//! One constant target per user — their enrolled browsers. The entry is
//! offered whether or not any browser is currently enrolled: the settings
//! panel needs the row to exist so users can discover and enable the route,
//! and delivery to an empty enrollment set fails closed with a clear reason
//! at send time.

use async_trait::async_trait;
use ironclaw_outbound::{
    CommunicationModality, DeliveryTargetCapabilities, OutboundDeliveryTargetEntry,
    OutboundDeliveryTargetId, OutboundDeliveryTargetOwner, OutboundDeliveryTargetProvider,
    OutboundDeliveryTargetScope, OutboundDeliveryTargetSummary, OutboundError,
};
use ironclaw_web_app::{WEB_APP_CHANNEL_NAME, WEB_APP_TARGET_ID, encode_web_app_target_ref};

/// Stateless: the entry is constant per owner (enrollment counts surface
/// through the web-app status view, not the catalog row).
pub struct WebAppOutboundTargetProvider;

impl WebAppOutboundTargetProvider {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self
    }
}

fn web_app_entry(
    scope: &OutboundDeliveryTargetScope,
) -> Result<OutboundDeliveryTargetEntry, OutboundError> {
    let target_id = OutboundDeliveryTargetId::new(WEB_APP_TARGET_ID).map_err(|error| {
        tracing::debug!(
            target: "ironclaw::web_app",
            error = %error,
            "web-app target id rejected"
        );
        OutboundError::Backend
    })?;
    let summary = OutboundDeliveryTargetSummary::new(
        target_id,
        WEB_APP_CHANNEL_NAME,
        "Web app",
        Some("Browser push notifications to your enrolled devices".to_string()),
    )
    .map_err(|error| {
        tracing::debug!(
            target: "ironclaw::web_app",
            error = %error,
            "web-app target summary rejected"
        );
        OutboundError::Backend
    })?;
    let destination =
        encode_web_app_target_ref(&scope.tenant_id, &scope.user_id).map_err(|error| {
            tracing::debug!(
                target: "ironclaw::web_app",
                error = %error,
                "web-app destination encoding failed"
            );
            OutboundError::Backend
        })?;
    Ok(OutboundDeliveryTargetEntry {
        summary,
        capabilities: DeliveryTargetCapabilities {
            // The web app is a NOTIFICATION target (blocked-automation notices),
            // not a final-reply/model-delivery target — a run's reply already
            // lands in the web app; browser push is only for notices. Outbound
            // thread creation is a later capability.
            final_replies: false,
            progress: false,
            gate_prompts: true,
            auth_prompts: true,
            notifications: true,
            modalities: vec![CommunicationModality::Text],
        },
        destination,
        owner: OutboundDeliveryTargetOwner::for_scope(scope),
    })
}

#[async_trait]
impl OutboundDeliveryTargetProvider for WebAppOutboundTargetProvider {
    async fn list_outbound_delivery_targets(
        &self,
        scope: &OutboundDeliveryTargetScope,
    ) -> Result<Vec<OutboundDeliveryTargetEntry>, OutboundError> {
        Ok(vec![web_app_entry(scope)?])
    }
}

/// Registry key composition registers the provider under — the extension id,
/// so the provider key and the manifest identity cannot drift.
pub const WEB_APP_TARGET_PROVIDER_KEY: &str = ironclaw_web_app::WEB_APP_EXTENSION_ID;

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::ids::{TenantId, UserId};
    use ironclaw_web_app::decode_web_app_target_ref;

    fn scope() -> OutboundDeliveryTargetScope {
        OutboundDeliveryTargetScope::new(
            TenantId::new("tenant1").expect("tenant"),
            UserId::new("user1").expect("user"),
        )
    }

    #[tokio::test]
    async fn the_entry_is_owner_scoped_and_decodes_back_to_the_owner() {
        let provider = WebAppOutboundTargetProvider::new();
        let entries = provider
            .list_outbound_delivery_targets(&scope())
            .await
            .expect("list");
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.summary.target_id.as_str(), WEB_APP_TARGET_ID);
        assert_eq!(entry.summary.channel.as_str(), WEB_APP_CHANNEL_NAME);
        assert!(
            !entry.capabilities.final_replies,
            "the web app is a notification target, not a final-reply/model-delivery target"
        );
        assert!(entry.capabilities.notifications);
        assert!(entry.capabilities.gate_prompts);
        assert!(entry.capabilities.auth_prompts);
        assert!(!entry.capabilities.progress);
        let (tenant, user) =
            decode_web_app_target_ref(entry.destination.as_str()).expect("decodes");
        assert_eq!(tenant.to_string(), "tenant1");
        assert_eq!(user.to_string(), "user1");
        assert!(entry.owner.matches_scope(&scope()));
    }

    #[tokio::test]
    async fn resolves_as_a_notification_target_but_not_a_final_reply_target() {
        let provider = WebAppOutboundTargetProvider::new();
        let id = OutboundDeliveryTargetId::new(WEB_APP_TARGET_ID).expect("id");
        // Notification path (blocked-automation notices) resolves it.
        let notification = provider
            .resolve_notification_target(&scope(), &id)
            .await
            .expect("resolve");
        assert!(
            notification.is_some(),
            "notifications=true keeps it resolvable as a notification target"
        );
        // Model/final-reply path must NOT resolve it: browser push is not where
        // a final reply or a model-chosen delivery lands.
        let final_reply = provider
            .resolve_outbound_delivery_target(&scope(), &id)
            .await
            .expect("resolve");
        assert!(
            final_reply.is_none(),
            "final_replies=false keeps it out of model/final-reply delivery"
        );
    }
}
