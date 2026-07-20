//! The outbound delivery-target provider port channel hosts implement.
//!
//! Each channel host registers a provider that lists the caller's delivery
//! targets (e.g. a paired Telegram DM, a Slack personal DM) so WebUI delivery
//! defaults and triggered-run delivery can address proactive sends. The
//! registries that aggregate providers stay in composition; only the port and
//! its entry shape live here so channel host crates can implement them.

use async_trait::async_trait;
use ironclaw_product_workflow::{
    RebornOutboundDeliveryTargetCapabilities, RebornOutboundDeliveryTargetId,
    RebornOutboundDeliveryTargetSummary, RebornServicesError, WebUiAuthenticatedCaller,
};
use ironclaw_turns::ReplyTargetBindingRef;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundDeliveryTargetEntry {
    pub summary: RebornOutboundDeliveryTargetSummary,
    pub capabilities: RebornOutboundDeliveryTargetCapabilities,
    pub reply_target_binding_ref: ReplyTargetBindingRef,
}

#[async_trait]
pub trait OutboundDeliveryTargetProvider: Send + Sync {
    async fn list_outbound_delivery_targets(
        &self,
        caller: &WebUiAuthenticatedCaller,
    ) -> Result<Vec<OutboundDeliveryTargetEntry>, RebornServicesError>;

    async fn resolve_outbound_delivery_target(
        &self,
        caller: &WebUiAuthenticatedCaller,
        target_id: &RebornOutboundDeliveryTargetId,
    ) -> Result<Option<OutboundDeliveryTargetEntry>, RebornServicesError> {
        Ok(self
            .list_outbound_delivery_targets(caller)
            .await?
            .into_iter()
            .find(|entry| {
                entry.capabilities.final_replies
                    && entry.summary.target_id.as_str() == target_id.as_str()
            }))
    }

    async fn resolve_reply_target_binding(
        &self,
        caller: &WebUiAuthenticatedCaller,
        target: &ReplyTargetBindingRef,
    ) -> Result<Option<OutboundDeliveryTargetEntry>, RebornServicesError> {
        Ok(self
            .list_outbound_delivery_targets(caller)
            .await?
            .into_iter()
            .find(|entry| {
                entry.capabilities.final_replies
                    && entry.reply_target_binding_ref.as_str() == target.as_str()
            }))
    }
}

/// Outcome of registering a provider under a host key: `Replaced` signals a
/// concurrent registration the host treats as a wiring conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboundDeliveryTargetRegistrationOutcome {
    Registered,
    Replaced,
}
