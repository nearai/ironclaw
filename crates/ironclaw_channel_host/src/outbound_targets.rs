//! The outbound delivery-target provider port channel hosts implement.
//!
//! Each channel host registers a provider that lists the caller's delivery
//! targets (e.g. a paired Telegram DM, a Slack personal DM) so WebUI delivery
//! defaults and triggered-run delivery can address proactive sends. The
//! registries that aggregate providers stay in composition; only the port and
//! its entry shape live here so channel host crates can implement them.

use async_trait::async_trait;
use ironclaw_host_api::{TenantId, UserId};
use ironclaw_product_workflow::{
    RebornOutboundDeliveryTargetCapabilities, RebornOutboundDeliveryTargetId,
    RebornOutboundDeliveryTargetSummary, RebornServicesError, WebUiAuthenticatedCaller,
};
use ironclaw_turns::ReplyTargetBindingRef;

/// The `(tenant, user)` an outbound delivery-target entry belongs to.
///
/// Providers stamp this with the identity of the resource they resolved — the
/// route's subject user, the DM target's paired user — so the aggregating
/// registry can drop any entry that does not belong to the querying caller.
/// Populating it from the resolved resource (not merely echoing the caller) is
/// what makes registry scoping a genuine defense-in-depth layer: a provider
/// that fails to filter by caller yields an owner that no longer matches the
/// caller, so the registry drops the leaked entry regardless.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundDeliveryTargetOwner {
    pub tenant_id: TenantId,
    pub user_id: UserId,
}

impl OutboundDeliveryTargetOwner {
    pub fn new(tenant_id: TenantId, user_id: UserId) -> Self {
        Self { tenant_id, user_id }
    }

    /// The owner scope for the authenticated caller. Static test fixtures that
    /// intentionally answer whichever caller asks claim ownership this way;
    /// real providers derive the owner from the resolved resource instead.
    pub fn for_caller(caller: &WebUiAuthenticatedCaller) -> Self {
        Self {
            tenant_id: caller.tenant_id.clone(),
            user_id: caller.user_id.clone(),
        }
    }

    /// Whether this owner is the querying caller's `(tenant, user)`.
    pub fn matches_caller(&self, caller: &WebUiAuthenticatedCaller) -> bool {
        self.tenant_id == caller.tenant_id && self.user_id == caller.user_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundDeliveryTargetEntry {
    pub summary: RebornOutboundDeliveryTargetSummary,
    pub capabilities: RebornOutboundDeliveryTargetCapabilities,
    pub reply_target_binding_ref: ReplyTargetBindingRef,
    /// The `(tenant, user)` this entry belongs to. The aggregating registry
    /// drops any fanned-out entry whose owner does not match the querying
    /// caller, so cross-caller isolation is structural rather than a
    /// per-provider convention.
    pub owner: OutboundDeliveryTargetOwner,
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
