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

use crate::pairing::{TelegramDmTarget, TelegramPairingError};
use crate::setup::{TelegramSetupError, TelegramSetupService};
use crate::state::FilesystemTelegramHostState;
use ironclaw_channel_host::outbound_targets::OutboundDeliveryTargetEntry;
use ironclaw_channel_host::outbound_targets::OutboundDeliveryTargetOwner;
use ironclaw_channel_host::outbound_targets::OutboundDeliveryTargetProvider;

/// Outbound delivery targets for the Telegram channel host: exactly one
/// personal-DM entry for the authenticated caller when the bot is configured
/// and the caller is paired; empty otherwise.
pub struct TelegramOutboundTargetProvider {
    tenant_id: TenantId,
    setup_service: Arc<TelegramSetupService>,
    state: Arc<FilesystemTelegramHostState>,
}

impl TelegramOutboundTargetProvider {
    pub fn new(
        tenant_id: TenantId,
        setup_service: Arc<TelegramSetupService>,
        state: Arc<FilesystemTelegramHostState>,
    ) -> Self {
        Self {
            tenant_id,
            setup_service,
            state,
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
        .map_err(|error| {
            tracing::debug!(%error, "telegram outbound target id/label construction failed");
            telegram_target_backend_error()
        })?;
        Ok(OutboundDeliveryTargetEntry {
            summary: RebornOutboundDeliveryTargetSummary::new(
                target_id,
                "telegram",
                "Telegram DM".to_string(),
                Some(format!("Telegram DM via @{bot_username}")),
            )
            .map_err(|error| {
                tracing::debug!(%error, "telegram outbound target id/label construction failed");
                telegram_target_backend_error()
            })?,
            capabilities: RebornOutboundDeliveryTargetCapabilities {
                final_replies: true,
                gate_prompts: true,
                auth_prompts: true,
            },
            // Canonical `tg:<chat_id>:_:_` encoding (no topic, no reply
            // threading for proactive DM delivery), built by the adapter crate
            // so it always round-trips through its render-time parser.
            reply_target_binding_ref: build_reply_target_binding(target.chat_id, None, None),
            // Owner is the paired DM target's user in this provider's tenant,
            // taken from the stored target rather than the caller, so the
            // registry drops the entry if a lookup ever crosses users.
            owner: OutboundDeliveryTargetOwner::new(self.tenant_id.clone(), target.user_id.clone()),
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
            .state
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
    use crate::ingress::dispatch::test_fixtures::{
        FIXTURE_BOT_USERNAME, RecordingBotApi, fixture_installation_id,
        unconfigured_setup_service_with_state,
    };
    use crate::setup::TelegramInstallationSetupUpdate;
    use crate::test_support::telegram_state;
    use secrecy::SecretString;

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

    async fn paired_state() -> Arc<FilesystemTelegramHostState> {
        let state = telegram_state();
        state
            .upsert_dm_target(
                &fixture_installation_id(),
                TelegramDmTarget {
                    user_id: UserId::new(USER).expect("user"),
                    chat_id: CHAT_ID,
                },
            )
            .await
            .expect("dm target stores");
        state
    }

    async fn configured_provider(
        state: Arc<FilesystemTelegramHostState>,
    ) -> TelegramOutboundTargetProvider {
        let setup = unconfigured_setup_service_with_state(
            Arc::new(RecordingBotApi::default()),
            Arc::clone(&state),
        );
        setup
            .save_with_previous(TelegramInstallationSetupUpdate {
                bot_token: Some(SecretString::from("123:abc".to_string())),
                webhook_url_override: None,
            })
            .await
            .expect("test setup saves");
        TelegramOutboundTargetProvider::new(TenantId::new(TENANT).expect("tenant"), setup, state)
    }

    #[tokio::test]
    async fn list_is_empty_when_unconfigured() {
        let state = paired_state().await;
        let provider = TelegramOutboundTargetProvider::new(
            TenantId::new(TENANT).expect("tenant"),
            unconfigured_setup_service_with_state(
                Arc::new(RecordingBotApi::default()),
                Arc::clone(&state),
            ),
            state,
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
        let provider = configured_provider(telegram_state()).await;

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
        let provider = configured_provider(paired_state().await).await;
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
        let provider = configured_provider(paired_state().await).await;

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
        let provider = configured_provider(paired_state().await).await;
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
        let provider = configured_provider(paired_state().await).await;
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
