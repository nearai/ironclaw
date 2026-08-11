//! `set_notification_channels` / `get_notification_channels` — split out of the
//! parent `outbound_preferences` module to keep both files under the repo's
//! file-size budget (`.claude/rules/architecture.md` §5: a `.rs` file over
//! 1,500 lines needs either an owning tracking issue or a decomposition; this
//! is the decomposition).
//!
//! A child module of `outbound_preferences`, so it reaches that module's
//! private helpers (`target_scope`, `reborn_summary_from_outbound`,
//! `reborn_capabilities_from_outbound`, `reborn_target_id_from_outbound`,
//! `map_outbound_repository_error`) and `RebornOutboundPreferencesService`'s
//! private fields/methods through Rust's ancestor-visibility rule — a private
//! item is visible in its defining module AND every descendant, so nothing
//! here needed a `pub(super)` widening on the parent side. `use super::*;`
//! mirrors exactly what the parent's own `#[cfg(test)] mod tests` already
//! relies on for the same reason.

use super::*;

pub(super) async fn set_notification_channels(
    service: &RebornOutboundPreferencesService,
    caller: ProductSurfaceCaller,
    request: RebornSetNotificationChannelsRequest,
) -> Result<RebornNotificationChannelsResponse, ProductSurfaceError> {
    // Dedup (preserving order) and cap BEFORE any registry/backend I/O: a
    // caller-supplied list longer than the cap is rejected deterministically
    // without spending a resolve call per id, and without persisting
    // anything (§7: "excess -> validation error").
    let deduped_ids = dedup_preserving_order(request.target_ids);
    if deduped_ids.len() > NOTIFICATION_TARGETS_CAP {
        return Err(too_many_notification_targets());
    }

    let scope = target_scope(&caller);
    let mut resolved_entries = Vec::with_capacity(deduped_ids.len());
    let mut stored_ids = Vec::with_capacity(deduped_ids.len());
    for reborn_id in &deduped_ids {
        let outbound_id = notification_channel_target_id_from_reborn(reborn_id)?;
        let entry = service
            .targets
            .resolve_notification_target(&scope, &outbound_id)
            .await
            .map_err(map_outbound_repository_error)?
            .ok_or_else(notification_channel_not_found)?;
        stored_ids.push(outbound_id);
        resolved_entries.push(entry);
    }

    let key = RebornOutboundPreferencesService::key(&caller);
    let existing = service
        .preferences
        .load_communication_preference(key.clone())
        .await
        .map_err(map_outbound_repository_error)?;
    let updated_at = Utc::now();
    service
        .preferences
        .write_communication_preference(WriteCommunicationPreferenceRequest {
            expected_version: existing.as_ref().map(|existing| existing.version),
            record: CommunicationPreferenceRecord {
                scope: key.scope,
                // Spec §7 migration write-back: an explicit notification-channel
                // set retires the legacy single-slot model for THIS record, so
                // the legacy target is cleared unconditionally — see
                // `legacy_row_then_notification_channels_set_clears_legacy_slot`.
                legacy_notification_target: None,
                default_modality: existing
                    .as_ref()
                    .and_then(|record| record.record.default_modality),
                notification_targets: stored_ids,
                updated_at,
                updated_by: caller.user_id.clone(),
            },
        })
        .await
        .map_err(map_outbound_repository_error)?;

    let channels = deduped_ids
        .iter()
        .zip(resolved_entries.iter())
        .map(|(target_id, entry)| {
            Ok(RebornNotificationChannel {
                target_id: target_id.clone(),
                status: RebornOutboundDeliveryTargetStatus::Available,
                option: Some(RebornOutboundDeliveryTargetOption {
                    target: reborn_summary_from_outbound(&entry.summary)?,
                    capabilities: reborn_capabilities_from_outbound(&entry.capabilities),
                }),
            })
        })
        .collect::<Result<Vec<_>, ProductSurfaceError>>()?;
    Ok(RebornNotificationChannelsResponse { channels })
}

pub(super) async fn get_notification_channels(
    service: &RebornOutboundPreferencesService,
    caller: ProductSurfaceCaller,
) -> Result<RebornNotificationChannelsResponse, ProductSurfaceError> {
    let key = RebornOutboundPreferencesService::key(&caller);
    let scope = target_scope(&caller);
    let resolution =
        crate::notification_channel_resolution::resolve_effective_notification_channels_arc(
            &service.preferences,
            &service.targets,
            &scope,
            key,
            crate::notification_channel_resolution::LookupErrorPolicy::Fail,
        )
        .await
        .map_err(map_outbound_repository_error)?;

    let mut channels = Vec::with_capacity(resolution.channels.len());
    for channel in resolution.channels {
        channels.push(match channel {
            crate::notification_channel_resolution::EffectiveNotificationChannel::Resolved(
                entry,
            ) => RebornNotificationChannel {
                target_id: reborn_target_id_from_outbound(&entry.summary.target_id)?,
                status: RebornOutboundDeliveryTargetStatus::Available,
                option: Some(RebornOutboundDeliveryTargetOption {
                    target: reborn_summary_from_outbound(&entry.summary)?,
                    capabilities: reborn_capabilities_from_outbound(&entry.capabilities),
                }),
            },
            // A stored id that no longer resolves is still represented — as
            // `Unavailable` with no resolved option — rather than silently
            // dropped, so a caller can tell "2 channels" from "3, one broken".
            crate::notification_channel_resolution::EffectiveNotificationChannel::Missing {
                target_id,
            } => RebornNotificationChannel {
                target_id: reborn_target_id_from_outbound(&target_id)?,
                status: RebornOutboundDeliveryTargetStatus::Unavailable,
                option: None,
            },
            // The legacy single slot's ref no longer resolves: mirror the
            // stored-set path (one broken channel), never an empty list. The
            // slot predates target ids, so the binding ref is the only stable
            // identity to show.
            crate::notification_channel_resolution::EffectiveNotificationChannel::LegacyUnresolvable {
                reply_ref,
            } => RebornNotificationChannel {
                target_id: RebornOutboundDeliveryTargetId::new(reply_ref.as_str().to_string())
                    .map_err(ProductSurfaceError::internal_from)?,
                status: RebornOutboundDeliveryTargetStatus::Unavailable,
                option: None,
            },
        });
    }
    Ok(RebornNotificationChannelsResponse { channels })
}

/// Dedup a caller-supplied target id list while preserving first-seen order
/// (spec §7).
fn dedup_preserving_order(
    ids: Vec<RebornOutboundDeliveryTargetId>,
) -> Vec<RebornOutboundDeliveryTargetId> {
    let mut seen = std::collections::HashSet::new();
    ids.into_iter()
        .filter(|id| seen.insert(id.clone()))
        .collect()
}

/// Same shape conversion as `outbound_target_id_from_reborn` (in the parent
/// module), but scoped to the plural `target_ids` field
/// `set_notification_channels` validates.
fn notification_channel_target_id_from_reborn(
    target_id: &RebornOutboundDeliveryTargetId,
) -> Result<OutboundDeliveryTargetId, ProductSurfaceError> {
    OutboundDeliveryTargetId::new(target_id.as_str()).map_err(|error| {
        tracing::debug!(%error, "notification channel target id failed validation");
        ProductSurfaceError {
            code: ProductSurfaceErrorCode::InvalidRequest,
            kind: ProductSurfaceErrorKind::Validation,
            status_code: 400,
            retryable: false,
            field: Some("target_ids".to_string()),
            validation_code: Some(ProductSurfaceValidationCode::InvalidId),
        }
    })
}

/// A `target_ids` entry that does not resolve through the caller-scoped
/// target registry (unknown id, or one owned by a different caller — the
/// registry drops foreign-owned entries structurally, see
/// `ironclaw_outbound::delivery_targets`'s owner-scoping tests).
///
/// Deliberately `InvalidRequest`/`Validation` (400), not the single-target
/// sibling's `NotFound` (404, see `outbound_target_not_found`): the brief
/// frames both plural rejections (this one and the cap-exceeded one below) as
/// "validation error", and the two should read consistently to the caller.
/// Both collapse to the same model-visible `Failed(InvalidInput)` outcome via
/// `outbound_delivery_outcome` regardless of which code is chosen.
fn notification_channel_not_found() -> ProductSurfaceError {
    ProductSurfaceError {
        code: ProductSurfaceErrorCode::InvalidRequest,
        kind: ProductSurfaceErrorKind::Validation,
        status_code: 400,
        retryable: false,
        field: Some("target_ids".to_string()),
        validation_code: Some(ProductSurfaceValidationCode::InvalidId),
    }
}

/// `target_ids` (after dedup) exceeds `NOTIFICATION_TARGETS_CAP` (spec §7).
/// Distinct from the pre-existing, UNRELATED `MAX_NOTIFICATION_TARGETS = 32`
/// in `ironclaw_outbound::validation` (which guards `ThreadNotificationPolicy`,
/// not this record) — do not conflate the two ceilings or their errors.
fn too_many_notification_targets() -> ProductSurfaceError {
    ProductSurfaceError {
        code: ProductSurfaceErrorCode::InvalidRequest,
        kind: ProductSurfaceErrorKind::Validation,
        status_code: 400,
        retryable: false,
        field: Some("target_ids".to_string()),
        validation_code: Some(ProductSurfaceValidationCode::TooLong),
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_outbound::DeliveryDefaultScope;

    use super::super::support_tests::*;
    use super::*;

    #[tokio::test]
    async fn set_notification_channels_dedups_preserving_first_seen_order() {
        let store =
            Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
        let provider = Arc::new(FakeTargetProvider::default());
        provider.insert(
            "user-alpha",
            target_entry("slack-alpha", "reply:slack-alpha", true),
        );
        provider.insert(
            "user-alpha",
            target_entry("slack-bravo", "reply:slack-bravo", true),
        );
        let service = RebornOutboundPreferencesService::new(store.clone(), provider);

        let response = service
            .set_notification_channels(
                caller("tenant-alpha", "user-alpha"),
                RebornSetNotificationChannelsRequest {
                    target_ids: vec![
                        target_id("slack-alpha"),
                        target_id("slack-bravo"),
                        target_id("slack-alpha"),
                    ],
                },
            )
            .await
            .expect("dedup preserving order should succeed");

        assert_eq!(
            response
                .channels
                .iter()
                .map(|channel| channel.target_id.as_str())
                .collect::<Vec<_>>(),
            vec!["slack-alpha", "slack-bravo"],
            "the repeated leading id must not duplicate or reorder the set"
        );
        assert!(
            response.channels.iter().all(|channel| channel.status
                == RebornOutboundDeliveryTargetStatus::Available
                && channel.option.is_some()),
            "every applied channel must be Available with a resolved option"
        );
        let stored = store
            .load_communication_preference(CommunicationPreferenceKey::new(
                tenant("tenant-alpha"),
                user("user-alpha"),
            ))
            .await
            .expect("load stored record")
            .expect("stored record");
        assert_eq!(
            stored
                .record
                .notification_targets
                .iter()
                .map(|id| id.as_str())
                .collect::<Vec<_>>(),
            vec!["slack-alpha", "slack-bravo"]
        );
    }

    #[tokio::test]
    async fn set_notification_channels_with_empty_list_clears_stored_set() {
        let store =
            Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
        let provider = Arc::new(FakeTargetProvider::default());
        provider.insert(
            "user-alpha",
            target_entry("slack-alpha", "reply:slack-alpha", true),
        );
        let service = RebornOutboundPreferencesService::new(store.clone(), provider);
        service
            .set_notification_channels(
                caller("tenant-alpha", "user-alpha"),
                RebornSetNotificationChannelsRequest {
                    target_ids: vec![target_id("slack-alpha")],
                },
            )
            .await
            .expect("initial set succeeds");

        let response = service
            .set_notification_channels(
                caller("tenant-alpha", "user-alpha"),
                RebornSetNotificationChannelsRequest {
                    target_ids: Vec::new(),
                },
            )
            .await
            .expect("clearing with an empty list succeeds");

        assert!(response.channels.is_empty());
        let stored = store
            .load_communication_preference(CommunicationPreferenceKey::new(
                tenant("tenant-alpha"),
                user("user-alpha"),
            ))
            .await
            .expect("load stored record")
            .expect("stored record");
        assert!(stored.record.notification_targets.is_empty());
    }

    #[tokio::test]
    async fn set_notification_channels_rejects_more_than_cap_targets() {
        let store =
            Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
        let provider = Arc::new(FakeTargetProvider::default());
        // Every id is individually RESOLVABLE (seeded into the provider below):
        // the rejection must come from the cap alone, not from any id failing
        // to resolve. `NOTIFICATION_TARGETS_CAP` of these would each succeed;
        // the `+1`th is what must push the set over the cap.
        let overflow_ids: Vec<_> = (0..=NOTIFICATION_TARGETS_CAP)
            .map(|index| {
                let target_id_value = format!("slack-{index}");
                provider.insert(
                    "user-alpha",
                    target_entry(&target_id_value, &format!("reply:{target_id_value}"), true),
                );
                target_id(&target_id_value)
            })
            .collect();
        assert_eq!(overflow_ids.len(), NOTIFICATION_TARGETS_CAP + 1);
        let service = RebornOutboundPreferencesService::new(store.clone(), provider);

        let error = service
            .set_notification_channels(
                caller("tenant-alpha", "user-alpha"),
                RebornSetNotificationChannelsRequest {
                    target_ids: overflow_ids,
                },
            )
            .await
            .expect_err("more than the cap must be rejected even when every id resolves");

        assert_eq!(error.code, ProductSurfaceErrorCode::InvalidRequest);
        assert_eq!(error.kind, ProductSurfaceErrorKind::Validation);
        assert_eq!(error.field.as_deref(), Some("target_ids"));
        // `TooLong` (not `InvalidId`) is what proves this is the cap check and
        // not a coincidental not-found rejection — `notification_channel_not_found`
        // produces byte-identical code/kind/field but a different
        // `validation_code`, so a deleted cap check (which would then reject on
        // the first unresolvable id it hit) could otherwise still pass this
        // assertion block. Here every id resolves, so only the cap check itself
        // can produce this error at all.
        assert_eq!(
            error.validation_code,
            Some(ProductSurfaceValidationCode::TooLong),
            "cap-exceeded must be distinguishable from a not-found id (InvalidId)"
        );
        assert!(
            store
                .load_communication_preference(CommunicationPreferenceKey::new(
                    tenant("tenant-alpha"),
                    user("user-alpha"),
                ))
                .await
                .expect("load preference")
                .is_none(),
            "an overflow set must not persist anything"
        );
    }

    #[tokio::test]
    async fn set_notification_channels_rejects_id_outside_callers_registry() {
        let store =
            Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
        let provider = Arc::new(FakeTargetProvider::default());
        provider.insert(
            "user-alpha",
            target_entry("slack-alpha", "reply:slack-alpha", true),
        );
        let service = RebornOutboundPreferencesService::new(store.clone(), provider);

        let error = service
            .set_notification_channels(
                caller("tenant-alpha", "user-alpha"),
                RebornSetNotificationChannelsRequest {
                    target_ids: vec![target_id("slack-alpha"), target_id("slack-unknown")],
                },
            )
            .await
            .expect_err("an id outside the caller's registry must be rejected");

        assert_eq!(error.field.as_deref(), Some("target_ids"));
        assert_eq!(
            error.validation_code,
            Some(ProductSurfaceValidationCode::InvalidId),
            "a not-found id must be distinguishable from cap-exceeded (TooLong)"
        );
        assert!(
            store
                .load_communication_preference(CommunicationPreferenceKey::new(
                    tenant("tenant-alpha"),
                    user("user-alpha"),
                ))
                .await
                .expect("load preference")
                .is_none(),
            "a rejected set must not persist a partial list"
        );
    }

    #[tokio::test]
    async fn legacy_row_then_notification_channels_set_clears_legacy_slot() {
        // Migration write-back: a row still carrying the retired single slot,
        // then an explicit `set_notification_channels` — the legacy slot must
        // be cleared and the new set must be the one stored.
        let store =
            Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
        let provider = Arc::new(FakeTargetProvider::default());
        provider.insert(
            "user-alpha",
            target_entry("slack-alpha", "reply:slack-alpha", true),
        );
        seed_record(
            store.as_ref(),
            "tenant-alpha",
            "user-alpha",
            Some(reply_ref("reply:slack-alpha")),
        )
        .await;
        let service = RebornOutboundPreferencesService::new(store.clone(), provider);

        service
            .set_notification_channels(
                caller("tenant-alpha", "user-alpha"),
                RebornSetNotificationChannelsRequest {
                    target_ids: vec![target_id("slack-alpha")],
                },
            )
            .await
            .expect("notification channels set succeeds");

        let stored = store
            .load_communication_preference(CommunicationPreferenceKey::new(
                tenant("tenant-alpha"),
                user("user-alpha"),
            ))
            .await
            .expect("load stored record")
            .expect("stored record");
        assert!(
            stored.record.legacy_notification_target.is_none(),
            "the legacy single slot must be cleared by the new-path set"
        );
        assert_eq!(
            stored
                .record
                .notification_targets
                .iter()
                .map(|id| id.as_str())
                .collect::<Vec<_>>(),
            vec!["slack-alpha"]
        );
    }

    #[tokio::test]
    async fn get_notification_channels_marks_unresolvable_stored_id_unavailable() {
        // CRITICAL regression pin: a stored id that no longer resolves must be
        // represented as `Unavailable` (with no resolved `option`), NOT
        // silently dropped from the list — otherwise a caller can't
        // distinguish "2 channels" from "3, one broken".
        let store =
            Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
        let provider = Arc::new(FakeTargetProvider::default());
        provider.insert(
            "user-alpha",
            target_entry("slack-alpha", "reply:slack-alpha", true),
        );
        store
            .put_communication_preference(CommunicationPreferenceRecord {
                scope: DeliveryDefaultScope::personal(tenant("tenant-alpha"), user("user-alpha")),
                legacy_notification_target: None,
                default_modality: None,
                notification_targets: vec![
                    outbound_target_id("slack-alpha"),
                    outbound_target_id("slack-gone"),
                ],
                updated_at: Utc::now(),
                updated_by: user("user-alpha"),
            })
            .await
            .expect("seed record with a stale id");
        let service = RebornOutboundPreferencesService::new(store, provider);

        let response = service
            .get_notification_channels(caller("tenant-alpha", "user-alpha"))
            .await
            .expect("get should succeed even with an unresolvable stored id");

        assert_eq!(
            response
                .channels
                .iter()
                .map(|channel| channel.target_id.as_str())
                .collect::<Vec<_>>(),
            vec!["slack-alpha", "slack-gone"],
            "both stored ids must be represented, including the unresolvable one"
        );
        let resolvable = &response.channels[0];
        assert_eq!(resolvable.target_id.as_str(), "slack-alpha");
        assert_eq!(
            resolvable.status,
            RebornOutboundDeliveryTargetStatus::Available
        );
        assert!(
            resolvable.option.is_some(),
            "an available channel must carry its resolved option"
        );
        let unresolvable = &response.channels[1];
        assert_eq!(unresolvable.target_id.as_str(), "slack-gone");
        assert_eq!(
            unresolvable.status,
            RebornOutboundDeliveryTargetStatus::Unavailable
        );
        assert!(
            unresolvable.option.is_none(),
            "an unavailable channel must carry no resolved option"
        );
    }

    /// The legacy single slot's ref no longer resolves: the read must mirror
    /// the stored-set path — ONE `Unavailable` row carrying the binding ref as
    /// its only stable identity — never an empty "no channels" response.
    #[tokio::test]
    async fn get_notification_channels_marks_unresolvable_legacy_slot_unavailable() {
        let store =
            Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
        // Provider knows NO targets: the legacy ref cannot resolve.
        let provider = Arc::new(FakeTargetProvider::default());
        seed_record(
            store.as_ref(),
            "tenant-alpha",
            "user-alpha",
            Some(reply_ref("reply:slack-gone")),
        )
        .await;
        let service = RebornOutboundPreferencesService::new(store, provider);

        let response = service
            .get_notification_channels(caller("tenant-alpha", "user-alpha"))
            .await
            .expect("an unresolvable legacy slot is a broken channel, not an error");

        assert_eq!(
            response
                .channels
                .iter()
                .map(|channel| (channel.target_id.as_str(), channel.status))
                .collect::<Vec<_>>(),
            vec![(
                "reply:slack-gone",
                RebornOutboundDeliveryTargetStatus::Unavailable
            )],
            "one broken channel must never read as 'no channels'"
        );
        assert!(response.channels[0].option.is_none());
    }

    #[tokio::test]
    async fn get_notification_channels_falls_back_to_legacy_final_reply_target() {
        let store =
            Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
        let provider = Arc::new(FakeTargetProvider::default());
        provider.insert(
            "user-alpha",
            target_entry("slack-alpha", "reply:slack-alpha", true),
        );
        seed_record(
            store.as_ref(),
            "tenant-alpha",
            "user-alpha",
            Some(reply_ref("reply:slack-alpha")),
        )
        .await;
        let service = RebornOutboundPreferencesService::new(store, provider);

        let response = service
            .get_notification_channels(caller("tenant-alpha", "user-alpha"))
            .await
            .expect("get should fall back to the legacy slot");

        assert_eq!(
            response
                .channels
                .iter()
                .map(|channel| channel.target_id.as_str())
                .collect::<Vec<_>>(),
            vec!["slack-alpha"]
        );
        assert_eq!(
            response.channels[0].status,
            RebornOutboundDeliveryTargetStatus::Available
        );

        // Caller-level isolation: the preference row is keyed off the
        // authenticated caller, so a second user in the same tenant sees
        // nothing of user-alpha's stored/legacy channels.
        let other_user = service
            .get_notification_channels(caller("tenant-alpha", "user-bravo"))
            .await
            .expect("second caller reads its own (absent) preference row");
        assert!(
            other_user.channels.is_empty(),
            "a different caller must never read another user's notification channels"
        );
    }
}
