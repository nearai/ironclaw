//! Telegram outbound target authority for default delivery.
//!
//! Mirrors the personal-DM half of the Slack outbound target surface
//! (`slack_outbound_targets`): core outbound preferences only see opaque
//! target ids and validated reply-target bindings, while the
//! Telegram-specific DM authority stays here. Telegram is DM-only — there is
//! no shared-channel target shape.
//!
//! The provider is fully dynamic: every call re-reads the current setup
//! record, so it is registered once at mount time and keeps answering
//! correctly across first-configure and bot swaps without a rebuild.

use std::sync::Arc;

use ironclaw_host_api::TenantId;
use ironclaw_product_adapters::AdapterInstallationId;
use ironclaw_product_workflow::{
    RebornOutboundDeliveryTargetCapabilities, RebornOutboundDeliveryTargetId,
    RebornOutboundDeliveryTargetSummary, RebornServicesError, RebornServicesErrorCode,
    RebornServicesErrorKind, WebUiAuthenticatedCaller,
};
use ironclaw_telegram_v2_adapter::build_reply_target_binding;

use crate::outbound::OutboundDeliveryTargetProvider;
use crate::outbound::outbound_preferences::OutboundDeliveryTargetEntry;
use crate::telegram::telegram_pairing::{
    TelegramDmTarget, TelegramDmTargetStore, TelegramPairingError,
};
use crate::telegram::telegram_setup::{TelegramSetupError, TelegramSetupService};

/// Outbound delivery targets for the Telegram channel host: exactly one
/// personal-DM entry for the authenticated caller when the bot is configured
/// and the caller is paired; empty otherwise.
pub(crate) struct TelegramOutboundTargetProvider {
    tenant_id: TenantId,
    setup_service: Arc<TelegramSetupService>,
    dm_target_store: Arc<dyn TelegramDmTargetStore>,
}

impl TelegramOutboundTargetProvider {
    pub(crate) fn new(
        tenant_id: TenantId,
        setup_service: Arc<TelegramSetupService>,
        dm_target_store: Arc<dyn TelegramDmTargetStore>,
    ) -> Self {
        Self {
            tenant_id,
            setup_service,
            dm_target_store,
        }
    }

    fn entry_for_dm_target(
        &self,
        bot_username: &str,
        installation_id: &AdapterInstallationId,
        target: &TelegramDmTarget,
    ) -> Result<OutboundDeliveryTargetEntry, RebornServicesError> {
        let target_id = RebornOutboundDeliveryTargetId::new(format!(
            "telegram:dm:{}:{}",
            installation_id.as_str(),
            target.user_id.as_str()
        ))
        .map_err(|_| telegram_target_backend_error())?;
        Ok(OutboundDeliveryTargetEntry {
            summary: RebornOutboundDeliveryTargetSummary::new(
                target_id,
                "telegram",
                "Telegram DM".to_string(),
                Some(format!("Telegram DM via @{bot_username}")),
            )
            .map_err(|_| telegram_target_backend_error())?,
            capabilities: RebornOutboundDeliveryTargetCapabilities {
                final_replies: true,
                gate_prompts: true,
                auth_prompts: true,
            },
            // Canonical `tg:<chat_id>:_:_` encoding (no topic, no reply
            // threading for proactive DM delivery), built by the adapter crate
            // so it always round-trips through its render-time parser.
            reply_target_binding_ref: build_reply_target_binding(target.chat_id, None, None),
        })
    }
}

impl std::fmt::Debug for TelegramOutboundTargetProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TelegramOutboundTargetProvider")
            .field("tenant_id", &self.tenant_id)
            .finish_non_exhaustive()
    }
}

#[async_trait::async_trait]
impl OutboundDeliveryTargetProvider for TelegramOutboundTargetProvider {
    async fn list_outbound_delivery_targets(
        &self,
        caller: &WebUiAuthenticatedCaller,
    ) -> Result<Vec<OutboundDeliveryTargetEntry>, RebornServicesError> {
        if caller.tenant_id != self.tenant_id {
            return Ok(Vec::new());
        }
        let Some(setup) = self
            .setup_service
            .current_setup()
            .await
            .map_err(map_telegram_setup_error("read Telegram setup"))?
        else {
            return Ok(Vec::new());
        };
        let installation_id = setup
            .installation_id()
            .map_err(map_telegram_setup_error("derive Telegram installation id"))?;
        let Some(target) = self
            .dm_target_store
            .dm_target_for_user(&installation_id, &caller.user_id)
            .await
            .map_err(map_telegram_pairing_error)?
        else {
            return Ok(Vec::new());
        };
        // Defense in depth: the store lookup is caller-keyed, but never emit a
        // target owned by anyone other than the authenticated caller.
        if target.user_id != caller.user_id {
            return Ok(Vec::new());
        }
        Ok(vec![self.entry_for_dm_target(
            &setup.bot_username,
            &installation_id,
            &target,
        )?])
    }
}

fn map_telegram_setup_error(
    context: &'static str,
) -> impl FnOnce(TelegramSetupError) -> RebornServicesError {
    move |error| {
        tracing::debug!(
            %error,
            context,
            "Telegram setup unavailable for outbound targets"
        );
        telegram_target_backend_error()
    }
}

fn map_telegram_pairing_error(error: TelegramPairingError) -> RebornServicesError {
    tracing::debug!(
        %error,
        "Telegram DM target lookup failed for outbound targets"
    );
    telegram_target_backend_error()
}

fn telegram_target_backend_error() -> RebornServicesError {
    RebornServicesError {
        code: RebornServicesErrorCode::Unavailable,
        kind: RebornServicesErrorKind::ServiceUnavailable,
        status_code: 503,
        retryable: true,
        field: None,
        validation_code: None,
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::UserId;
    use ironclaw_turns::ReplyTargetBindingRef;

    use super::*;
    use crate::telegram::telegram_dispatch::test_fixtures::{
        FIXTURE_BOT_USERNAME, InMemoryDmTargetStore, RecordingBotApi, configured_setup_service,
        fixture_installation_id, unconfigured_setup_service,
    };

    const TENANT: &str = "tenant-a";
    const USER: &str = "ben";
    const CHAT_ID: i64 = 555;

    fn caller() -> WebUiAuthenticatedCaller {
        WebUiAuthenticatedCaller::new(
            TenantId::new(TENANT).expect("tenant"),
            UserId::new(USER).expect("user"),
            None,
            None,
        )
    }

    async fn paired_dm_store() -> Arc<InMemoryDmTargetStore> {
        let store = Arc::new(InMemoryDmTargetStore::default());
        store
            .upsert_dm_target(
                &fixture_installation_id(),
                TelegramDmTarget {
                    user_id: UserId::new(USER).expect("user"),
                    chat_id: CHAT_ID,
                },
            )
            .await
            .expect("dm target stores");
        store
    }

    async fn configured_provider(
        dm_target_store: Arc<InMemoryDmTargetStore>,
    ) -> TelegramOutboundTargetProvider {
        TelegramOutboundTargetProvider::new(
            TenantId::new(TENANT).expect("tenant"),
            configured_setup_service(Arc::new(RecordingBotApi::default())).await,
            dm_target_store,
        )
    }

    #[tokio::test]
    async fn list_is_empty_when_unconfigured() {
        let provider = TelegramOutboundTargetProvider::new(
            TenantId::new(TENANT).expect("tenant"),
            unconfigured_setup_service(Arc::new(RecordingBotApi::default())),
            paired_dm_store().await,
        );

        let targets = provider
            .list_outbound_delivery_targets(&caller())
            .await
            .expect("list");
        assert!(
            targets.is_empty(),
            "no setup record must mean no outbound targets"
        );
    }

    #[tokio::test]
    async fn list_is_empty_when_caller_is_unpaired() {
        let provider = configured_provider(Arc::new(InMemoryDmTargetStore::default())).await;

        let targets = provider
            .list_outbound_delivery_targets(&caller())
            .await
            .expect("list");
        assert!(
            targets.is_empty(),
            "unpaired callers must see no Telegram DM target"
        );
    }

    #[tokio::test]
    async fn list_is_empty_for_cross_tenant_caller() {
        let provider = configured_provider(paired_dm_store().await).await;
        let cross_tenant = WebUiAuthenticatedCaller::new(
            TenantId::new("tenant-other").expect("tenant"),
            UserId::new(USER).expect("user"),
            None,
            None,
        );

        let targets = provider
            .list_outbound_delivery_targets(&cross_tenant)
            .await
            .expect("list");
        assert!(
            targets.is_empty(),
            "cross-tenant callers must see no Telegram targets"
        );
    }

    #[tokio::test]
    async fn paired_caller_gets_dm_entry_with_canonical_binding_ref() {
        let provider = configured_provider(paired_dm_store().await).await;

        let targets = provider
            .list_outbound_delivery_targets(&caller())
            .await
            .expect("list");
        assert_eq!(targets.len(), 1, "exactly the caller's personal DM");
        let entry = &targets[0];
        assert_eq!(
            entry.summary.target_id.as_str(),
            format!("telegram:dm:{}:{USER}", fixture_installation_id().as_str())
        );
        assert_eq!(entry.summary.channel.as_str(), "telegram");
        assert_eq!(entry.summary.display_name.as_str(), "Telegram DM");
        assert_eq!(
            entry
                .summary
                .description
                .as_ref()
                .expect("description present")
                .as_str(),
            format!("Telegram DM via @{FIXTURE_BOT_USERNAME}")
        );
        assert!(entry.capabilities.final_replies);
        assert!(entry.capabilities.gate_prompts);
        assert!(entry.capabilities.auth_prompts);
        assert_eq!(
            entry.reply_target_binding_ref.as_str(),
            format!("tg:{CHAT_ID}:_:_"),
            "binding ref must be the adapter's canonical DM encoding"
        );
    }

    #[tokio::test]
    async fn resolve_outbound_delivery_target_default_impl_matches_own_id_only() {
        let provider = configured_provider(paired_dm_store().await).await;
        let own_id = RebornOutboundDeliveryTargetId::new(format!(
            "telegram:dm:{}:{USER}",
            fixture_installation_id().as_str()
        ))
        .expect("target id");
        let foreign_id = RebornOutboundDeliveryTargetId::new(format!(
            "telegram:dm:{}:someone-else",
            fixture_installation_id().as_str()
        ))
        .expect("target id");

        let resolved = provider
            .resolve_outbound_delivery_target(&caller(), &own_id)
            .await
            .expect("resolve")
            .expect("own target resolves");
        assert_eq!(resolved.summary.target_id, own_id);

        assert!(
            provider
                .resolve_outbound_delivery_target(&caller(), &foreign_id)
                .await
                .expect("resolve")
                .is_none(),
            "a target id owned by another user must not resolve"
        );
    }

    #[tokio::test]
    async fn resolve_reply_target_binding_default_impl_matches_stored_ref() {
        let provider = configured_provider(paired_dm_store().await).await;
        let stored_ref =
            ReplyTargetBindingRef::new(format!("tg:{CHAT_ID}:_:_")).expect("binding ref");
        let other_ref = ReplyTargetBindingRef::new("tg:999999:_:_").expect("binding ref");

        let resolved = provider
            .resolve_reply_target_binding(&caller(), &stored_ref)
            .await
            .expect("resolve")
            .expect("stored binding resolves");
        assert_eq!(resolved.reply_target_binding_ref, stored_ref);

        assert!(
            provider
                .resolve_reply_target_binding(&caller(), &other_ref)
                .await
                .expect("resolve")
                .is_none(),
            "a binding ref for a different chat must not resolve"
        );
    }
}

/// Telegram's [`crate::outbound::channel_delivery::ChannelDeliveryProtocol`]:
/// `tg:` binding-ref decoding and positive-chat-id DM classification. Status
/// messages are deliberately unwired in v1 — the pre-router owns Telegram's
/// static replies, and the delivery machinery's working/notification posts
/// were never delivered on this channel (they previously failed closed at the
/// telegram egress policy); the error here preserves that behavior without a
/// network round-trip.
#[derive(Debug, Default)]
pub(crate) struct TelegramDeliveryProtocol;

#[async_trait::async_trait]
impl crate::outbound::channel_delivery::ChannelDeliveryProtocol for TelegramDeliveryProtocol {
    fn conversation_id_from_reply_target_binding_ref(
        &self,
        target: &ironclaw_turns::ReplyTargetBindingRef,
    ) -> Option<(String, Option<String>)> {
        // The Telegram adapter renders straight from the
        // `tg:<chat_id>:<topic|_>:<reply|_>` binding ref; Telegram has no
        // space/team dimension.
        let parsed = ironclaw_telegram_v2_adapter::parse_reply_target(target).ok()?;
        Some((parsed.chat_id.to_string(), None))
    }

    fn reply_target_is_personal_dm(&self, target: &ironclaw_turns::ReplyTargetBindingRef) -> bool {
        // Telegram private chats have positive chat ids (groups/supergroups/
        // channels are negative), and the host only stores DM targets from
        // private-chat pairing.
        ironclaw_telegram_v2_adapter::parse_reply_target(target)
            .map(|parsed| parsed.chat_id > 0)
            .unwrap_or(false)
    }

    fn posted_message_from_render_response(
        &self,
        _path: &str,
        _body: &[u8],
    ) -> Option<crate::outbound::channel_delivery::PostedChannelMessage> {
        None
    }

    fn connect_nudge_message(&self) -> &'static str {
        // Unreachable in practice (the pairing-aware pre-router intercepts
        // unpaired senders before the workflow), kept consistent with the
        // pre-router's static hint.
        "This bot is IronClaw. Pair your account from IronClaw → Extensions → Telegram, then message me here."
    }

    fn is_direct_message_conversation(&self, conversation_id: &str) -> bool {
        conversation_id
            .parse::<i64>()
            .is_ok_and(|chat_id| chat_id > 0)
    }

    async fn post_status_message(
        &self,
        _egress: &dyn ironclaw_product_adapters::ProtocolHttpEgress,
        _conversation: &ironclaw_product_adapters::ExternalConversationRef,
        _text: &str,
    ) -> Result<
        crate::outbound::channel_delivery::PostedChannelMessage,
        crate::outbound::channel_delivery::FinalReplyDeliveryError,
    > {
        Err(
            crate::outbound::channel_delivery::FinalReplyDeliveryError::StatusMessage {
                reason: "telegram status messages are not wired".to_string(),
            },
        )
    }

    async fn delete_status_message(
        &self,
        _egress: &dyn ironclaw_product_adapters::ProtocolHttpEgress,
        _message: &crate::outbound::channel_delivery::PostedChannelMessage,
    ) -> Result<(), crate::outbound::channel_delivery::FinalReplyDeliveryError> {
        Ok(())
    }
}

#[cfg(test)]
mod telegram_delivery_protocol_tests {
    use ironclaw_product_adapters::{
        EgressRequest, EgressResponse, ExternalConversationRef, ProtocolHttpEgress,
        ProtocolHttpEgressError,
    };

    use super::TelegramDeliveryProtocol;
    use crate::outbound::channel_delivery::{ChannelDeliveryProtocol, FinalReplyDeliveryError};

    /// Egress that panics if the protocol touches the network.
    #[derive(Debug)]
    struct PanicEgress;

    #[async_trait::async_trait]
    impl ProtocolHttpEgress for PanicEgress {
        async fn send(
            &self,
            _request: EgressRequest,
        ) -> Result<EgressResponse, ProtocolHttpEgressError> {
            panic!("telegram status messages must not reach egress");
        }
    }

    /// Behavior-preserving contract for the unwired v1 status messages: the
    /// old shared machinery built Slack-shaped posts that failed closed at
    /// telegram's egress policy; the protocol seam now fails without ANY
    /// network round-trip.
    #[tokio::test]
    async fn post_status_message_fails_without_touching_egress() {
        let protocol = TelegramDeliveryProtocol;
        let conversation =
            ExternalConversationRef::new(None, "555", None, None).expect("conversation");

        let error = protocol
            .post_status_message(&PanicEgress, &conversation, "working…")
            .await
            .expect_err("status messages are unwired in v1");
        assert!(matches!(
            error,
            FinalReplyDeliveryError::StatusMessage { .. }
        ));
        protocol
            .delete_status_message(
                &PanicEgress,
                &crate::outbound::channel_delivery::PostedChannelMessage {
                    conversation_id: "555".to_string(),
                    message_ref: "1".to_string(),
                },
            )
            .await
            .expect("delete is a no-op");
    }

    #[test]
    fn telegram_refs_classify_dm_and_conversation() {
        let protocol = TelegramDeliveryProtocol;
        assert!(protocol.is_direct_message_conversation("555"));
        assert!(!protocol.is_direct_message_conversation("-100123"));
        assert!(!protocol.is_direct_message_conversation("not-a-chat-id"));
    }
}
