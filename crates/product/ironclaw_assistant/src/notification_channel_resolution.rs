//! One shared resolver for a user's *effective* notification channels.
//!
//! Two consumers read the same stored state — the WebUI read surface
//! (`reborn_services::outbound_preferences::notification_channels`) and the
//! background-run notifier (`run_delivery::triggered`) — and previously each
//! hand-maintained the same five-step flow: load the creator's preference
//! record, resolve the legacy single slot only when the stored set is empty,
//! fold through `effective_notification_target_ids`, then re-resolve every id
//! through the owner-scoped registry. This module is the single copy; the
//! per-consumer mapping (wire DTO rows vs. delivery targets) and the
//! per-consumer *failure* policy stay at each call site.

use std::sync::Arc;

use ironclaw_outbound::{
    CommunicationPreferenceKey, CommunicationPreferenceRepository, OutboundDeliveryTargetEntry,
    OutboundDeliveryTargetId, OutboundDeliveryTargetProvider, OutboundDeliveryTargetScope,
    OutboundError,
};

use ironclaw_host_api::turn::ReplyTargetBindingRef;

/// One effective notification channel, in resolution order.
pub(crate) enum EffectiveNotificationChannel {
    /// The stored id resolved to a live, owner-scoped catalog entry.
    Resolved(OutboundDeliveryTargetEntry),
    /// A stored id that no longer resolves for its owner. Represented rather
    /// than dropped so a consumer can tell "2 channels" from "3, one broken".
    Missing { target_id: OutboundDeliveryTargetId },
    /// The legacy single slot's binding ref no longer resolves to any catalog
    /// entry. There is no target id to name — the slot predates ids — so the
    /// binding ref itself is the only stable identity the consumer can show.
    LegacyUnresolvable { reply_ref: ReplyTargetBindingRef },
}

/// What to do when one stored id's catalog lookup FAILS (backend error, not
/// "not found" — missing entries are always represented).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum LookupErrorPolicy {
    /// Surface the error to the caller (the WebUI read surface: a broken
    /// lookup is a failed request, not a silently shorter list).
    Fail,
    /// Record the error in [`NotificationChannelResolution::skipped`] and
    /// continue (the background notifier: one unreachable catalog entry must
    /// not suppress the notification on every other channel).
    SkipEntry,
}

/// The resolver's output: the effective channels plus, under
/// [`LookupErrorPolicy::SkipEntry`], the per-entry errors that were skipped
/// (for the caller to log — this module never logs).
pub(crate) struct NotificationChannelResolution {
    pub(crate) channels: Vec<EffectiveNotificationChannel>,
    pub(crate) skipped: Vec<(OutboundDeliveryTargetId, OutboundError)>,
}

/// Resolve the caller's effective notification channels.
///
/// Errors: the preference-record read and (when the stored set is empty) the
/// legacy-slot lookup always propagate — nothing is known without them. Per-id
/// lookup failures follow `lookup_errors`.
pub(crate) async fn resolve_effective_notification_channels(
    preferences: &dyn CommunicationPreferenceRepository,
    targets: &dyn OutboundDeliveryTargetProvider,
    scope: &OutboundDeliveryTargetScope,
    key: CommunicationPreferenceKey,
    lookup_errors: LookupErrorPolicy,
) -> Result<NotificationChannelResolution, OutboundError> {
    let Some(record) = preferences.load_communication_preference(key).await? else {
        return Ok(NotificationChannelResolution {
            channels: Vec::new(),
            skipped: Vec::new(),
        });
    };
    let record = record.record;

    // Only resolve the legacy single slot when the stored set is empty —
    // `effective_notification_target_ids` ignores the closure otherwise, and a
    // migrated caller should not pay a catalog round-trip.
    let mut legacy_unresolvable = None;
    let legacy_target_id = if record.notification_targets.is_empty() {
        match record.legacy_notification_target.as_ref() {
            Some(reply_ref) => {
                let entry = targets
                    .resolve_reply_target_binding(scope, reply_ref)
                    .await?;
                if entry.is_none() {
                    legacy_unresolvable = Some(reply_ref.clone());
                }
                entry.map(|entry| entry.summary.target_id.clone())
            }
            None => None,
        }
    } else {
        None
    };

    let effective_ids = record.effective_notification_target_ids(move |_| legacy_target_id);
    let mut channels = Vec::with_capacity(effective_ids.len());
    let mut skipped = Vec::new();
    // An unresolvable legacy slot yields an empty effective set; represent the
    // broken slot instead of an empty list (mirroring the stored-set path's
    // `Missing` rows) so "one broken channel" never reads as "no channels".
    if let Some(reply_ref) = legacy_unresolvable {
        channels.push(EffectiveNotificationChannel::LegacyUnresolvable { reply_ref });
    }
    for target_id in effective_ids {
        match targets.resolve_notification_target(scope, &target_id).await {
            Ok(Some(entry)) => channels.push(EffectiveNotificationChannel::Resolved(entry)),
            Ok(None) => channels.push(EffectiveNotificationChannel::Missing { target_id }),
            Err(error) => match lookup_errors {
                LookupErrorPolicy::Fail => return Err(error),
                LookupErrorPolicy::SkipEntry => skipped.push((target_id, error)),
            },
        }
    }
    Ok(NotificationChannelResolution { channels, skipped })
}

/// `dyn`-friendly adapter: both consumers hold `Arc<dyn …>` handles.
pub(crate) async fn resolve_effective_notification_channels_arc(
    preferences: &Arc<dyn CommunicationPreferenceRepository>,
    targets: &Arc<dyn OutboundDeliveryTargetProvider>,
    scope: &OutboundDeliveryTargetScope,
    key: CommunicationPreferenceKey,
    lookup_errors: LookupErrorPolicy,
) -> Result<NotificationChannelResolution, OutboundError> {
    resolve_effective_notification_channels(
        preferences.as_ref(),
        targets.as_ref(),
        scope,
        key,
        lookup_errors,
    )
    .await
}
