//! Presence-based shared-conversation admission (extension-runtime §5.3).
//!
//! A channel event only reaches admission after the channel's verified
//! ingress accepted it, and a provider only delivers a conversation's events
//! to this deployment because the bot is a member of that conversation.
//! Delivery through the channel's verified ingress IS the membership
//! evidence, so [`PresenceSharedAdmission`] admits exactly the shared
//! conversations delivered for its own adapter installation — no operator
//! allowlist, no config reads.
//!
//! The `SharedConversationAdmission` port survives as the seam: the product
//! workflow still fails closed when no resolver is installed (a channel
//! whose actor identity is not per-user gets `None` and never admits shared
//! conversations), a channel-extras override can still replace the default
//! resolver, and future admission policy plugs in here without touching the
//! workflow.

use async_trait::async_trait;
use ironclaw_host_api::product_adapter::{AdapterInstallationId, ProductAdapterId};
use ironclaw_product_contracts::error::ProductOperationFailure;
use ironclaw_product_contracts::shared_admission::{
    SharedConversationAdmission, SharedConversationAdmissionRequest,
};

/// Handle-suffix convention for manifest-declared `[channel.config]` fields:
/// `{name}` or `*_{name}` declares the field. Consumed by the
/// connection-scoping claims (e.g. the `*_team_id` workspace claim).
pub fn handle_declares_field(handle: &str, name: &str) -> bool {
    handle == name
        || handle
            .strip_suffix(name)
            .is_some_and(|prefix| prefix.ends_with('_'))
}

/// The presence-based admission resolver: a shared conversation is admitted
/// iff its request arrived for this resolver's own adapter and installation —
/// the channel's verified ingress delivered it, so the bot is present in the
/// conversation. A foreign adapter or installation resolves to not-admitted,
/// which the product workflow fails closed. Infallible in practice: there is
/// nothing to read and nothing to parse.
#[derive(Debug)]
pub struct PresenceSharedAdmission {
    adapter_id: ProductAdapterId,
    installation_id: AdapterInstallationId,
}

impl PresenceSharedAdmission {
    pub fn new(adapter_id: ProductAdapterId, installation_id: AdapterInstallationId) -> Self {
        Self {
            adapter_id,
            installation_id,
        }
    }
}

#[async_trait]
impl SharedConversationAdmission for PresenceSharedAdmission {
    async fn shared_conversation_admitted(
        &self,
        request: SharedConversationAdmissionRequest,
    ) -> Result<bool, ProductOperationFailure> {
        Ok(
            request.adapter_id == self.adapter_id
                && request.installation_id == self.installation_id,
        )
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_product_contracts::shared_admission::ProductConversationRouteKey;

    use super::*;

    const INSTALLATION: &str = "vendorx-install-1";

    fn resolver() -> PresenceSharedAdmission {
        PresenceSharedAdmission::new(
            ProductAdapterId::new("vendorx").expect("adapter id"),
            AdapterInstallationId::new(INSTALLATION).expect("installation id"),
        )
    }

    fn request(
        adapter: &str,
        installation: &str,
        space: Option<&str>,
        conversation: &str,
    ) -> SharedConversationAdmissionRequest {
        SharedConversationAdmissionRequest {
            adapter_id: ProductAdapterId::new(adapter).expect("adapter id"),
            installation_id: AdapterInstallationId::new(installation).expect("installation id"),
            route_key: ProductConversationRouteKey::new(
                space.map(str::to_string),
                conversation.to_string(),
            )
            .expect("route key"),
        }
    }

    /// Presence admits: any conversation delivered through this
    /// installation's verified ingress is admitted, with zero configuration.
    #[tokio::test]
    async fn own_installation_conversations_are_admitted_by_presence() {
        for conversation in ["C777", "C888", "-1008675309"] {
            let admitted = resolver()
                .shared_conversation_admitted(request(
                    "vendorx",
                    INSTALLATION,
                    Some("S1"),
                    conversation,
                ))
                .await
                .expect("admission resolves");
            assert!(
                admitted,
                "conversation {conversation} is admitted by presence"
            );
        }
    }

    #[tokio::test]
    async fn foreign_adapter_is_denied() {
        let admitted = resolver()
            .shared_conversation_admitted(request("vendory", INSTALLATION, Some("S1"), "C777"))
            .await
            .expect("admission resolves");
        assert!(
            !admitted,
            "a foreign adapter's conversation is never admitted"
        );
    }

    #[tokio::test]
    async fn foreign_installation_is_denied() {
        let admitted = resolver()
            .shared_conversation_admitted(request(
                "vendorx",
                "vendorx-install-2",
                Some("S1"),
                "C777",
            ))
            .await
            .expect("admission resolves");
        assert!(
            !admitted,
            "another installation's conversation is never admitted"
        );
    }

    #[test]
    fn handle_declares_field_follows_the_suffix_convention() {
        assert!(handle_declares_field("team_id", "team_id"));
        assert!(handle_declares_field("slack_team_id", "team_id"));
        assert!(
            !handle_declares_field("slackteam_id", "team_id"),
            "a prefix must be underscore-separated"
        );
        assert!(
            !handle_declares_field("team_id_extra", "team_id"),
            "the name is a suffix, never an infix"
        );
    }
}
