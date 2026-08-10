//! Test fixtures shared by `outbound_preferences`'s own `mod tests` and
//! the `notification_channels` submodule's test module. A sibling module of
//! both under `outbound_preferences` (their common ancestor), reached via
//! Rust's ancestor-visibility rule with no `pub(super)` widening needed on
//! the parent's own items.

use std::{collections::HashMap, sync::Mutex};

use ironclaw_host_api::ids::{TenantId, UserId};
use ironclaw_outbound::{
    CommunicationModality, CommunicationPreferenceRepository, DeliveryDefaultScope,
    DeliveryTargetCapabilities, OutboundDeliveryTargetEntry, OutboundDeliveryTargetOwner,
};
use ironclaw_turns::ReplyTargetBindingRef;

use super::*;

#[derive(Default)]
pub(super) struct FakeTargetProvider {
    by_user: Mutex<HashMap<String, Vec<OutboundDeliveryTargetEntry>>>,
}

impl FakeTargetProvider {
    pub(super) fn insert(&self, user_id: &str, entry: OutboundDeliveryTargetEntry) {
        self.by_user
            .lock()
            .expect("lock")
            .entry(user_id.to_string())
            .or_default()
            .push(entry);
    }
}

#[async_trait]
impl OutboundDeliveryTargetProvider for FakeTargetProvider {
    async fn list_outbound_delivery_targets(
        &self,
        caller: &OutboundDeliveryTargetScope,
    ) -> Result<Vec<OutboundDeliveryTargetEntry>, OutboundError> {
        Ok(self
            .by_user
            .lock()
            .expect("lock")
            .get(caller.user_id.as_str())
            .cloned()
            .unwrap_or_default())
    }
}

pub(super) fn target_entry(
    target_id_value: &str,
    reply_target: &str,
    final_replies: bool,
) -> OutboundDeliveryTargetEntry {
    target_entry_for_channel(
        target_id_value,
        "slack",
        "Slack DM",
        reply_target,
        final_replies,
    )
}

pub(super) fn target_entry_for_channel(
    target_id_value: &str,
    channel: &str,
    display_name: &str,
    reply_target: &str,
    final_replies: bool,
) -> OutboundDeliveryTargetEntry {
    // The coincident case (a full channel target is both a final-reply and a
    // notification target); divergent combinations use `target_entry_with_caps`.
    target_entry_with_caps(
        target_id_value,
        channel,
        display_name,
        reply_target,
        final_replies,
        final_replies,
    )
}

/// Build a target with INDEPENDENT `final_replies` and `notifications`
/// capabilities. The two are separate concerns: the model-delivery list filters
/// `final_replies`, the notification picker filters `notifications`.
pub(super) fn target_entry_with_caps(
    target_id_value: &str,
    channel: &str,
    display_name: &str,
    reply_target: &str,
    final_replies: bool,
    notifications: bool,
) -> OutboundDeliveryTargetEntry {
    OutboundDeliveryTargetEntry {
        summary: OutboundDeliveryTargetSummary::new(
            outbound_target_id(target_id_value),
            channel,
            display_name,
            Some(display_name.to_string()),
        )
        .expect("valid target summary"),
        capabilities: DeliveryTargetCapabilities {
            final_replies,
            progress: false,
            gate_prompts: true,
            auth_prompts: true,
            notifications,
            modalities: Vec::new(),
        },
        destination: reply_ref(reply_target),
        // Existing registry-path tests query as tenant-alpha/user-alpha, so
        // fixtures claim that owner and survive the caller-scoping filter.
        owner: OutboundDeliveryTargetOwner::new(tenant("tenant-alpha"), user("user-alpha")),
    }
}

/// Seed a row in the shape written before notification channels existed: a
/// single legacy target and no notification set.
pub(super) async fn seed_record(
    store: &dyn CommunicationPreferenceRepository,
    tenant_id: &str,
    user_id: &str,
    legacy_notification_target: Option<ReplyTargetBindingRef>,
) {
    store
        .put_communication_preference(CommunicationPreferenceRecord {
            scope: DeliveryDefaultScope::personal(tenant(tenant_id), user(user_id)),
            legacy_notification_target,
            default_modality: Some(CommunicationModality::Voice),
            notification_targets: Vec::new(),
            updated_at: Utc::now(),
            updated_by: user(user_id),
        })
        .await
        .expect("seed communication preference");
}

pub(super) fn caller(tenant_id: &str, user_id: &str) -> ProductSurfaceCaller {
    ProductSurfaceCaller::new(tenant(tenant_id), user(user_id), None, None)
}

pub(super) fn tenant(value: &str) -> TenantId {
    TenantId::new(value).expect("valid tenant")
}

pub(super) fn user(value: &str) -> UserId {
    UserId::new(value).expect("valid user")
}

pub(super) fn reply_ref(value: &str) -> ReplyTargetBindingRef {
    ReplyTargetBindingRef::new(value).expect("valid reply target")
}

pub(super) fn target_id(value: &str) -> RebornOutboundDeliveryTargetId {
    RebornOutboundDeliveryTargetId::new(value).expect("valid target id")
}

pub(super) fn outbound_target_id(value: &str) -> OutboundDeliveryTargetId {
    OutboundDeliveryTargetId::new(value).expect("valid target id")
}
