//! WebUI as an outbound delivery channel.
//!
//! Exposes the per-user "default WebUI thread" as a selectable outbound
//! delivery target so the delivery-defaults surface (Automations panel, the
//! `builtin__outbound_delivery_target_set` agent tool) can route routine /
//! trigger results to the WebUI instead of an external product like Slack.
//!
//! The target is channel-neutral inventory only: actual delivery into the
//! default WebUI thread is owned by `webui_agent_messages`, which recognises
//! the binding ref exposed here. Nothing in this module sends anything.

use async_trait::async_trait;
use ironclaw_product_workflow::{
    RebornOutboundDeliveryTargetCapabilities, RebornOutboundDeliveryTargetId,
    RebornOutboundDeliveryTargetSummary, RebornServicesError, RebornServicesErrorCode,
    RebornServicesErrorKind, WebUiAuthenticatedCaller,
};
use ironclaw_turns::ReplyTargetBindingRef;

use crate::outbound_preferences::{OutboundDeliveryTargetEntry, OutboundDeliveryTargetProvider};

/// Stable target id clients submit to select the default WebUI thread.
pub(crate) const WEBUI_DEFAULT_THREAD_TARGET_ID: &str = "webui:default-thread";

/// Channel label shown next to the target in inventory listings.
pub(crate) const WEBUI_OUTBOUND_CHANNEL: &str = "webui";

/// Sealed-format reply-target binding ref stored in communication preferences
/// when the default WebUI thread is selected. `webui_agent_messages` matches
/// resolved candidates against this ref to decide whether it owns delivery.
pub(crate) const WEBUI_DEFAULT_THREAD_REPLY_TARGET_BINDING_REF: &str = "reply:webui:default-thread";

pub(crate) fn webui_default_thread_reply_target_binding_ref() -> ReplyTargetBindingRef {
    // safety: the constant is a short, non-empty, control-character-free
    // literal — `ReplyTargetBindingRef::new` cannot fail on it.
    ReplyTargetBindingRef::new(WEBUI_DEFAULT_THREAD_REPLY_TARGET_BINDING_REF)
        .expect("webui default thread binding ref literal is valid")
}

/// Offers the caller's default WebUI thread as an always-available
/// final-reply delivery target.
///
/// The WebUI thread is an internal surface (no external egress), so the
/// target exists for every authenticated caller without pairing or
/// provisioning. Gate and auth prompts stay on their existing delivery paths.
#[derive(Debug, Default)]
pub(crate) struct WebUiOutboundDeliveryTargetProvider;

#[async_trait]
impl OutboundDeliveryTargetProvider for WebUiOutboundDeliveryTargetProvider {
    async fn list_outbound_delivery_targets(
        &self,
        _caller: &WebUiAuthenticatedCaller,
    ) -> Result<Vec<OutboundDeliveryTargetEntry>, RebornServicesError> {
        Ok(vec![webui_default_thread_target_entry()?])
    }
}

fn webui_default_thread_target_entry() -> Result<OutboundDeliveryTargetEntry, RebornServicesError> {
    let target_id = RebornOutboundDeliveryTargetId::new(WEBUI_DEFAULT_THREAD_TARGET_ID)
        .map_err(|_| webui_target_internal_error())?;
    let summary = RebornOutboundDeliveryTargetSummary::new(
        target_id,
        WEBUI_OUTBOUND_CHANNEL,
        "WebUI default thread",
        Some("Agent messages thread in the web app".to_string()),
    )
    .map_err(|_| webui_target_internal_error())?;
    Ok(OutboundDeliveryTargetEntry {
        summary,
        capabilities: RebornOutboundDeliveryTargetCapabilities {
            final_replies: true,
            gate_prompts: false,
            auth_prompts: false,
        },
        reply_target_binding_ref: webui_default_thread_reply_target_binding_ref(),
    })
}

fn webui_target_internal_error() -> RebornServicesError {
    RebornServicesError {
        code: RebornServicesErrorCode::Internal,
        kind: RebornServicesErrorKind::Internal,
        status_code: 500,
        retryable: false,
        field: None,
        validation_code: None,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ironclaw_host_api::{TenantId, UserId};
    use ironclaw_outbound::{
        CommunicationPreferenceKey, CommunicationPreferenceRepository, InMemoryOutboundStateStore,
    };
    use ironclaw_product_workflow::{
        OutboundPreferencesProductFacade, RebornSetOutboundPreferencesRequest,
    };

    use super::*;
    use crate::outbound_preferences::RebornOutboundPreferencesFacade;

    fn caller() -> WebUiAuthenticatedCaller {
        WebUiAuthenticatedCaller::new(
            TenantId::new("tenant-webui-target").expect("tenant"),
            UserId::new("user-webui-target").expect("user"),
            None,
            None,
        )
    }

    #[tokio::test]
    async fn webui_provider_lists_the_default_thread_target_for_every_caller() {
        let provider = WebUiOutboundDeliveryTargetProvider;

        let targets = provider
            .list_outbound_delivery_targets(&caller())
            .await
            .expect("list targets");

        assert_eq!(targets.len(), 1);
        let entry = &targets[0];
        assert_eq!(
            entry.summary.target_id.as_str(),
            WEBUI_DEFAULT_THREAD_TARGET_ID
        );
        assert_eq!(entry.summary.channel.as_str(), WEBUI_OUTBOUND_CHANNEL);
        assert!(entry.capabilities.final_replies);
        assert!(!entry.capabilities.gate_prompts);
        assert!(!entry.capabilities.auth_prompts);
        assert_eq!(
            entry.reply_target_binding_ref.as_str(),
            WEBUI_DEFAULT_THREAD_REPLY_TARGET_BINDING_REF
        );
    }

    #[tokio::test]
    async fn webui_target_is_selectable_as_the_default_outbound_channel() {
        let store = Arc::new(InMemoryOutboundStateStore::default());
        let facade = RebornOutboundPreferencesFacade::new(
            store.clone(),
            Arc::new(WebUiOutboundDeliveryTargetProvider),
        );

        let response = facade
            .set_outbound_preferences(
                caller(),
                RebornSetOutboundPreferencesRequest {
                    final_reply_target_id: Some(
                        RebornOutboundDeliveryTargetId::new(WEBUI_DEFAULT_THREAD_TARGET_ID)
                            .expect("target id"),
                    ),
                },
            )
            .await
            .expect("select webui target");

        assert_eq!(
            response
                .final_reply_target
                .as_ref()
                .map(|target| target.target_id.as_str()),
            Some(WEBUI_DEFAULT_THREAD_TARGET_ID)
        );
        let stored = store
            .load_communication_preference(CommunicationPreferenceKey::new(
                TenantId::new("tenant-webui-target").expect("tenant"),
                UserId::new("user-webui-target").expect("user"),
            ))
            .await
            .expect("load stored record")
            .expect("stored record");
        assert_eq!(
            stored
                .record
                .final_reply_target
                .as_ref()
                .map(|target| target.as_str()),
            Some(WEBUI_DEFAULT_THREAD_REPLY_TARGET_BINDING_REF)
        );
    }
}
