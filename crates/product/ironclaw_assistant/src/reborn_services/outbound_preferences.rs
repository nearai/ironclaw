mod notification_channels;

use std::sync::Arc;

use super::{
    OutboundPreferencesProductService, ProductSurfaceCaller, ProductSurfaceError,
    ProductSurfaceErrorCode, ProductSurfaceErrorKind, ProductSurfaceValidationCode,
    RebornNotificationChannel, RebornNotificationChannelsResponse,
    RebornOutboundDeliveryTargetCapabilities, RebornOutboundDeliveryTargetId,
    RebornOutboundDeliveryTargetListResponse, RebornOutboundDeliveryTargetOption,
    RebornOutboundDeliveryTargetStatus, RebornOutboundDeliveryTargetSummary,
    RebornSetNotificationChannelsRequest,
};
use async_trait::async_trait;
use chrono::Utc;
use ironclaw_outbound::{
    CommunicationPreferenceKey, CommunicationPreferenceRecord, CommunicationPreferenceRepository,
    NOTIFICATION_TARGETS_CAP, OutboundDeliveryTargetId, OutboundDeliveryTargetProvider,
    OutboundDeliveryTargetScope, OutboundDeliveryTargetSummary, OutboundError,
    WriteCommunicationPreferenceRequest,
};

pub struct RebornOutboundPreferencesService {
    preferences: Arc<dyn CommunicationPreferenceRepository>,
    targets: Arc<dyn OutboundDeliveryTargetProvider>,
}

impl std::fmt::Debug for RebornOutboundPreferencesService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RebornOutboundPreferencesService")
            .field("preferences", &"Arc<dyn CommunicationPreferenceRepository>")
            .field("targets", &"Arc<dyn OutboundDeliveryTargetProvider>")
            .finish()
    }
}

impl RebornOutboundPreferencesService {
    pub fn new(
        preferences: Arc<dyn CommunicationPreferenceRepository>,
        targets: Arc<dyn OutboundDeliveryTargetProvider>,
    ) -> Self {
        Self {
            preferences,
            targets,
        }
    }

    /// Invariant: `ProductSurfaceCaller` must come from the authenticated
    /// product/session boundary, never from request-body tenant/user fields.
    /// This key and target-provider scope intentionally share the same
    /// verified caller identity.
    fn key(caller: &ProductSurfaceCaller) -> CommunicationPreferenceKey {
        CommunicationPreferenceKey::personal(caller.tenant_id.clone(), caller.user_id.clone())
    }
}

#[async_trait]
impl OutboundPreferencesProductService for RebornOutboundPreferencesService {
    async fn list_outbound_delivery_targets(
        &self,
        caller: ProductSurfaceCaller,
    ) -> Result<RebornOutboundDeliveryTargetListResponse, ProductSurfaceError> {
        let targets = self
            .targets
            .list_outbound_delivery_targets(&target_scope(&caller))
            .await
            .map_err(map_outbound_repository_error)?
            .into_iter()
            // Base list is the UNION of both capabilities; each consumer narrows
            // to the one it needs (picker → notifications, model → final_replies),
            // keeping the two capabilities independent.
            .filter(|entry| entry.capabilities.final_replies || entry.capabilities.notifications)
            .map(|entry| {
                Ok(RebornOutboundDeliveryTargetOption {
                    target: reborn_summary_from_outbound(&entry.summary)?,
                    capabilities: reborn_capabilities_from_outbound(&entry.capabilities),
                })
            })
            .collect::<Result<Vec<_>, ProductSurfaceError>>()?;
        Ok(RebornOutboundDeliveryTargetListResponse {
            targets,
            next_cursor: None,
        })
    }

    async fn set_notification_channels(
        &self,
        caller: ProductSurfaceCaller,
        request: RebornSetNotificationChannelsRequest,
    ) -> Result<RebornNotificationChannelsResponse, ProductSurfaceError> {
        notification_channels::set_notification_channels(self, caller, request).await
    }

    async fn get_notification_channels(
        &self,
        caller: ProductSurfaceCaller,
    ) -> Result<RebornNotificationChannelsResponse, ProductSurfaceError> {
        notification_channels::get_notification_channels(self, caller).await
    }
}

fn target_scope(caller: &ProductSurfaceCaller) -> OutboundDeliveryTargetScope {
    OutboundDeliveryTargetScope::new(caller.tenant_id.clone(), caller.user_id.clone())
}

/// Convert the internal (`ironclaw_outbound`) target id to the wire-facing
/// newtype. Both share the same 512-byte/character-safety validation, so this
/// only fails on a genuine internal bug (see `outbound_target_projection_error`).
fn reborn_target_id_from_outbound(
    target_id: &OutboundDeliveryTargetId,
) -> Result<RebornOutboundDeliveryTargetId, ProductSurfaceError> {
    RebornOutboundDeliveryTargetId::new(target_id.as_str()).map_err(|error| {
        tracing::error!(
            target_id = %target_id,
            %error,
            "outbound target id failed wire projection"
        );
        outbound_target_projection_error()
    })
}

fn reborn_summary_from_outbound(
    summary: &OutboundDeliveryTargetSummary,
) -> Result<RebornOutboundDeliveryTargetSummary, ProductSurfaceError> {
    let target_id = reborn_target_id_from_outbound(&summary.target_id)?;
    RebornOutboundDeliveryTargetSummary::new(
        target_id,
        summary.channel.as_str(),
        summary.display_name.as_str(),
        summary
            .description
            .as_ref()
            .map(|description| description.as_str().to_string()),
    )
    .map_err(|_| outbound_target_projection_error())
}

fn reborn_capabilities_from_outbound(
    capabilities: &ironclaw_outbound::DeliveryTargetCapabilities,
) -> RebornOutboundDeliveryTargetCapabilities {
    RebornOutboundDeliveryTargetCapabilities {
        final_replies: capabilities.final_replies,
        gate_prompts: capabilities.gate_prompts,
        auth_prompts: capabilities.auth_prompts,
        notifications: capabilities.notifications,
    }
}

fn outbound_target_projection_error() -> ProductSurfaceError {
    ProductSurfaceError {
        code: ProductSurfaceErrorCode::Internal,
        kind: ProductSurfaceErrorKind::Internal,
        status_code: 500,
        retryable: false,
        field: None,
        validation_code: None,
    }
}

fn map_outbound_repository_error(error: OutboundError) -> ProductSurfaceError {
    match error {
        OutboundError::InvalidRequest { .. }
        | OutboundError::PreferenceTargetMissing { .. }
        | OutboundError::SubscriptionScopeMismatch
        | OutboundError::DeliveryNotFound
        | OutboundError::ReplyAttachmentIntentLimitExceeded => ProductSurfaceError {
            code: ProductSurfaceErrorCode::InvalidRequest,
            kind: ProductSurfaceErrorKind::Validation,
            status_code: 400,
            retryable: false,
            field: None,
            validation_code: None,
        },
        OutboundError::AccessDenied => ProductSurfaceError {
            code: ProductSurfaceErrorCode::Forbidden,
            kind: ProductSurfaceErrorKind::ParticipantDenied,
            status_code: 403,
            retryable: false,
            field: None,
            validation_code: None,
        },
        OutboundError::CasConflict
        | OutboundError::ReplyAttachmentIntentsSealed
        | OutboundError::ReplyAttachmentIntentConflict => ProductSurfaceError {
            code: ProductSurfaceErrorCode::Conflict,
            kind: ProductSurfaceErrorKind::Conflict,
            status_code: 409,
            retryable: false,
            field: None,
            validation_code: None,
        },
        OutboundError::Backend | OutboundError::Serialization => ProductSurfaceError {
            code: ProductSurfaceErrorCode::Unavailable,
            kind: ProductSurfaceErrorKind::ServiceUnavailable,
            status_code: 503,
            retryable: true,
            field: None,
            validation_code: None,
        },
    }
}

#[cfg(test)]
pub(super) mod support_tests;

#[cfg(test)]
mod tests {
    use ironclaw_outbound::{
        CommunicationPreferenceRepository, CommunicationPreferenceVersion, DeliveryDefaultScope,
        MutableOutboundDeliveryTargetRegistry, OutboundDeliveryTargetEntry,
        OutboundDeliveryTargetOwner, OutboundDeliveryTargetRegistry,
        VersionedCommunicationPreferenceRecord,
    };
    use ironclaw_turns::ReplyTargetBindingRef;

    use super::support_tests::*;
    use super::*;

    struct FailingTargetProvider;

    #[async_trait]
    impl OutboundDeliveryTargetProvider for FailingTargetProvider {
        async fn list_outbound_delivery_targets(
            &self,
            _caller: &OutboundDeliveryTargetScope,
        ) -> Result<Vec<OutboundDeliveryTargetEntry>, OutboundError> {
            Err(OutboundError::Backend)
        }
    }

    struct ResolvingOnlyTargetProvider {
        entry: OutboundDeliveryTargetEntry,
    }

    #[async_trait]
    impl OutboundDeliveryTargetProvider for ResolvingOnlyTargetProvider {
        async fn list_outbound_delivery_targets(
            &self,
            _caller: &OutboundDeliveryTargetScope,
        ) -> Result<Vec<OutboundDeliveryTargetEntry>, OutboundError> {
            Ok(Vec::new())
        }

        async fn resolve_outbound_delivery_target(
            &self,
            _caller: &OutboundDeliveryTargetScope,
            target_id: &OutboundDeliveryTargetId,
        ) -> Result<Option<OutboundDeliveryTargetEntry>, OutboundError> {
            Ok(
                (self.entry.summary.target_id.as_str() == target_id.as_str())
                    .then(|| self.entry.clone()),
            )
        }

        async fn resolve_reply_target_binding(
            &self,
            _caller: &OutboundDeliveryTargetScope,
            target: &ReplyTargetBindingRef,
        ) -> Result<Option<OutboundDeliveryTargetEntry>, OutboundError> {
            Ok((self.entry.destination.as_str() == target.as_str()).then(|| self.entry.clone()))
        }

        async fn resolve_notification_target(
            &self,
            _caller: &OutboundDeliveryTargetScope,
            target_id: &OutboundDeliveryTargetId,
        ) -> Result<Option<OutboundDeliveryTargetEntry>, OutboundError> {
            Ok(
                (self.entry.summary.target_id.as_str() == target_id.as_str())
                    .then(|| self.entry.clone()),
            )
        }
    }

    struct ResolveFailingTargetProvider;

    #[async_trait]
    impl OutboundDeliveryTargetProvider for ResolveFailingTargetProvider {
        async fn list_outbound_delivery_targets(
            &self,
            _caller: &OutboundDeliveryTargetScope,
        ) -> Result<Vec<OutboundDeliveryTargetEntry>, OutboundError> {
            Ok(Vec::new())
        }

        async fn resolve_outbound_delivery_target(
            &self,
            _caller: &OutboundDeliveryTargetScope,
            _target_id: &OutboundDeliveryTargetId,
        ) -> Result<Option<OutboundDeliveryTargetEntry>, OutboundError> {
            Err(OutboundError::Backend)
        }

        async fn resolve_reply_target_binding(
            &self,
            _caller: &OutboundDeliveryTargetScope,
            _target: &ReplyTargetBindingRef,
        ) -> Result<Option<OutboundDeliveryTargetEntry>, OutboundError> {
            Err(OutboundError::Backend)
        }

        async fn resolve_notification_target(
            &self,
            _caller: &OutboundDeliveryTargetScope,
            _target_id: &OutboundDeliveryTargetId,
        ) -> Result<Option<OutboundDeliveryTargetEntry>, OutboundError> {
            Err(OutboundError::Backend)
        }
    }

    struct NullResolvingTargetProvider;

    #[async_trait]
    impl OutboundDeliveryTargetProvider for NullResolvingTargetProvider {
        async fn list_outbound_delivery_targets(
            &self,
            _caller: &OutboundDeliveryTargetScope,
        ) -> Result<Vec<OutboundDeliveryTargetEntry>, OutboundError> {
            Ok(Vec::new())
        }

        async fn resolve_outbound_delivery_target(
            &self,
            _caller: &OutboundDeliveryTargetScope,
            _target_id: &OutboundDeliveryTargetId,
        ) -> Result<Option<OutboundDeliveryTargetEntry>, OutboundError> {
            Ok(None)
        }

        async fn resolve_reply_target_binding(
            &self,
            _caller: &OutboundDeliveryTargetScope,
            _target: &ReplyTargetBindingRef,
        ) -> Result<Option<OutboundDeliveryTargetEntry>, OutboundError> {
            Ok(None)
        }

        async fn resolve_notification_target(
            &self,
            _caller: &OutboundDeliveryTargetScope,
            _target_id: &OutboundDeliveryTargetId,
        ) -> Result<Option<OutboundDeliveryTargetEntry>, OutboundError> {
            Ok(None)
        }
    }

    struct LoadFailingPreferenceRepository;

    #[async_trait]
    impl CommunicationPreferenceRepository for LoadFailingPreferenceRepository {
        async fn put_communication_preference(
            &self,
            _record: CommunicationPreferenceRecord,
        ) -> Result<(), OutboundError> {
            Ok(())
        }

        async fn load_communication_preference(
            &self,
            _key: CommunicationPreferenceKey,
        ) -> Result<Option<VersionedCommunicationPreferenceRecord>, OutboundError> {
            Err(OutboundError::Backend)
        }

        async fn write_communication_preference(
            &self,
            _request: WriteCommunicationPreferenceRequest,
        ) -> Result<VersionedCommunicationPreferenceRecord, OutboundError> {
            Err(OutboundError::Backend)
        }
    }

    struct PutFailingPreferenceRepository;

    #[async_trait]
    impl CommunicationPreferenceRepository for PutFailingPreferenceRepository {
        async fn put_communication_preference(
            &self,
            _record: CommunicationPreferenceRecord,
        ) -> Result<(), OutboundError> {
            Err(OutboundError::Backend)
        }

        async fn load_communication_preference(
            &self,
            _key: CommunicationPreferenceKey,
        ) -> Result<Option<VersionedCommunicationPreferenceRecord>, OutboundError> {
            Ok(None)
        }

        async fn write_communication_preference(
            &self,
            _request: WriteCommunicationPreferenceRequest,
        ) -> Result<VersionedCommunicationPreferenceRecord, OutboundError> {
            Err(OutboundError::Backend)
        }
    }

    struct CasConflictingPreferenceRepository;

    #[async_trait]
    impl CommunicationPreferenceRepository for CasConflictingPreferenceRepository {
        async fn put_communication_preference(
            &self,
            _record: CommunicationPreferenceRecord,
        ) -> Result<(), OutboundError> {
            Err(OutboundError::CasConflict)
        }

        async fn load_communication_preference(
            &self,
            _key: CommunicationPreferenceKey,
        ) -> Result<Option<VersionedCommunicationPreferenceRecord>, OutboundError> {
            Ok(Some(VersionedCommunicationPreferenceRecord {
                record: CommunicationPreferenceRecord {
                    scope: DeliveryDefaultScope::personal(
                        tenant("tenant-alpha"),
                        user("user-alpha"),
                    ),
                    legacy_notification_target: None,
                    default_modality: None,
                    notification_targets: Vec::new(),
                    updated_at: Utc::now(),
                    updated_by: user("user-alpha"),
                },
                version: CommunicationPreferenceVersion::from_raw(1),
            }))
        }

        async fn write_communication_preference(
            &self,
            _request: WriteCommunicationPreferenceRequest,
        ) -> Result<VersionedCommunicationPreferenceRecord, OutboundError> {
            Err(OutboundError::CasConflict)
        }
    }

    /// Every backend failure the notification-channel surface can hit, mapped
    /// at the one seam that survives the retired preference setter: a
    /// preference-store read/write failure and a CAS conflict on `set`, and a
    /// target-provider list/resolve failure on `set`, `get`, and `list`.
    /// Consolidated from the six per-arm tests the retired
    /// `get/set_outbound_preferences` pair used to own.
    #[tokio::test]
    async fn notification_channel_backend_failures_map_to_unavailable_and_conflict() {
        let channels_request = || RebornSetNotificationChannelsRequest {
            target_ids: vec![target_id("slack-alpha")],
        };
        let seeded_provider = || {
            let provider = Arc::new(FakeTargetProvider::default());
            provider.insert(
                "user-alpha",
                target_entry("slack-alpha", "reply:slack-alpha", true),
            );
            provider
        };

        // Preference-store read failure: both directions report unavailable.
        let service = RebornOutboundPreferencesService::new(
            Arc::new(LoadFailingPreferenceRepository),
            seeded_provider(),
        );
        assert_unavailable_backend_error(
            service
                .get_notification_channels(caller("tenant-alpha", "user-alpha"))
                .await
                .expect_err("backend read failure"),
        );
        assert_unavailable_backend_error(
            service
                .set_notification_channels(caller("tenant-alpha", "user-alpha"), channels_request())
                .await
                .expect_err("backend read failure before write"),
        );

        // Preference-store write failure.
        let service = RebornOutboundPreferencesService::new(
            Arc::new(PutFailingPreferenceRepository),
            seeded_provider(),
        );
        assert_unavailable_backend_error(
            service
                .set_notification_channels(caller("tenant-alpha", "user-alpha"), channels_request())
                .await
                .expect_err("backend write failure"),
        );

        // Losing the optimistic-lock race is a conflict, not an outage.
        let service = RebornOutboundPreferencesService::new(
            Arc::new(CasConflictingPreferenceRepository),
            seeded_provider(),
        );
        let error = service
            .set_notification_channels(caller("tenant-alpha", "user-alpha"), channels_request())
            .await
            .expect_err("conflicting preference write");
        assert_eq!(error.code, ProductSurfaceErrorCode::Conflict);
        assert_eq!(error.kind, ProductSurfaceErrorKind::Conflict);
        assert_eq!(error.status_code, 409);
        assert!(!error.retryable);

        // Target-provider listing failure, on every read that fans out to it.
        let store =
            Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
        seed_record(
            store.as_ref(),
            "tenant-alpha",
            "user-alpha",
            Some(reply_ref("reply:slack-alpha")),
        )
        .await;
        let service = RebornOutboundPreferencesService::new(store, Arc::new(FailingTargetProvider));
        assert_unavailable_backend_error(
            service
                .list_outbound_delivery_targets(caller("tenant-alpha", "user-alpha"))
                .await
                .expect_err("list target provider failure"),
        );
        assert_unavailable_backend_error(
            service
                .get_notification_channels(caller("tenant-alpha", "user-alpha"))
                .await
                .expect_err("get target provider failure"),
        );
        assert_unavailable_backend_error(
            service
                .set_notification_channels(caller("tenant-alpha", "user-alpha"), channels_request())
                .await
                .expect_err("set target provider failure"),
        );

        // Target-provider RESOLVE failure (distinct from listing: the write and
        // the legacy read-migration both go through the resolver).
        let store =
            Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
        seed_record(
            store.as_ref(),
            "tenant-alpha",
            "user-alpha",
            Some(reply_ref("reply:slack-alpha")),
        )
        .await;
        let service =
            RebornOutboundPreferencesService::new(store, Arc::new(ResolveFailingTargetProvider));
        assert_unavailable_backend_error(
            service
                .get_notification_channels(caller("tenant-alpha", "user-alpha"))
                .await
                .expect_err("get target resolver failure"),
        );
        assert_unavailable_backend_error(
            service
                .set_notification_channels(caller("tenant-alpha", "user-alpha"), channels_request())
                .await
                .expect_err("set target resolver failure"),
        );
    }

    #[tokio::test]
    async fn list_targets_is_scoped_to_caller_and_final_reply_capability() {
        let store =
            Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
        let provider = Arc::new(FakeTargetProvider::default());
        provider.insert(
            "user-alpha",
            target_entry("slack-alpha", "reply:slack-alpha", true),
        );
        provider.insert(
            "user-alpha",
            target_entry("slack-progress", "reply:slack-progress", false),
        );
        provider.insert(
            "user-bravo",
            target_entry("slack-bravo", "reply:slack-bravo", true),
        );
        let service = RebornOutboundPreferencesService::new(store, provider);

        let response = service
            .list_outbound_delivery_targets(caller("tenant-alpha", "user-alpha"))
            .await
            .expect("target list");

        assert_eq!(response.targets.len(), 1);
        assert_eq!(response.targets[0].target.target_id.as_str(), "slack-alpha");
        assert!(response.next_cursor.is_none());
    }

    /// The base list is the UNION of `final_replies` and `notifications`, and the
    /// two are INDEPENDENT: a final-reply-only target (a model-delivery target,
    /// not a notification channel) and a notification-only target (a notification
    /// channel, not a model target — like web-push) both survive the base;
    /// a target with neither capability is excluded. The picker
    /// (`build_outbound_delivery_targets_view`) and the model list
    /// (`list_outbound_delivery_targets_for_model`) then narrow this base to
    /// their own capability.
    #[tokio::test]
    async fn list_targets_returns_the_union_of_final_reply_and_notification_capabilities() {
        let store =
            Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
        let provider = Arc::new(FakeTargetProvider::default());
        provider.insert(
            "user-alpha",
            target_entry_with_caps(
                "final-only",
                "slack",
                "Final only",
                "reply:final-only",
                true,
                false,
            ),
        );
        provider.insert(
            "user-alpha",
            target_entry_with_caps(
                "notif-only",
                "web-push",
                "Notif only",
                "reply:notif-only",
                false,
                true,
            ),
        );
        provider.insert(
            "user-alpha",
            target_entry_with_caps("neither", "slack", "Neither", "reply:neither", false, false),
        );
        let service = RebornOutboundPreferencesService::new(store, provider);

        let response = service
            .list_outbound_delivery_targets(caller("tenant-alpha", "user-alpha"))
            .await
            .expect("target list");

        let ids: Vec<&str> = response
            .targets
            .iter()
            .map(|entry| entry.target.target_id.as_str())
            .collect();
        assert!(
            ids.contains(&"final-only"),
            "a final-reply-only target must survive the union base: {ids:?}"
        );
        assert!(
            ids.contains(&"notif-only"),
            "a notification-only target must survive the union base: {ids:?}"
        );
        assert!(
            !ids.contains(&"neither"),
            "a target with neither capability must be excluded: {ids:?}"
        );
        assert_eq!(response.targets.len(), 2);
    }

    /// A notification-channel write resolves ids through the authority
    /// resolver, never through the public target list: the provider below
    /// lists nothing yet resolves the id, and the set still succeeds and
    /// reads back.
    #[tokio::test]
    async fn notification_channels_use_the_authority_resolver_not_the_public_target_list() {
        let store =
            Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
        let provider = Arc::new(ResolvingOnlyTargetProvider {
            entry: target_entry("slack-alpha", "reply:slack-alpha", true),
        });
        let service = RebornOutboundPreferencesService::new(store.clone(), provider);

        let listed = service
            .list_outbound_delivery_targets(caller("tenant-alpha", "user-alpha"))
            .await
            .expect("list targets");
        assert!(listed.targets.is_empty());

        let set_response = service
            .set_notification_channels(
                caller("tenant-alpha", "user-alpha"),
                RebornSetNotificationChannelsRequest {
                    target_ids: vec![target_id("slack-alpha")],
                },
            )
            .await
            .expect("set channel through resolver");
        assert_eq!(
            set_response
                .channels
                .iter()
                .map(|channel| channel.target_id.as_str())
                .collect::<Vec<_>>(),
            vec!["slack-alpha"]
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
            vec!["slack-alpha"]
        );

        let get_response = service
            .get_notification_channels(caller("tenant-alpha", "user-alpha"))
            .await
            .expect("get channels through resolver");
        assert_eq!(
            get_response
                .channels
                .iter()
                .map(|channel| channel.status)
                .collect::<Vec<_>>(),
            vec![RebornOutboundDeliveryTargetStatus::Available]
        );
    }

    #[tokio::test]
    async fn target_registry_aggregates_channel_neutral_providers_for_default_selection() {
        let store =
            Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
        let slack_provider = Arc::new(FakeTargetProvider::default());
        slack_provider.insert(
            "user-alpha",
            target_entry_for_channel(
                "slack-alpha",
                "slack",
                "Slack DM",
                "reply:slack-alpha",
                true,
            ),
        );
        let telegram_provider = Arc::new(FakeTargetProvider::default());
        telegram_provider.insert(
            "user-alpha",
            target_entry_for_channel(
                "telegram-alpha",
                "telegram",
                "Telegram chat",
                "reply:telegram-alpha",
                true,
            ),
        );
        let registry = Arc::new(OutboundDeliveryTargetRegistry::new(vec![
            slack_provider,
            telegram_provider,
        ]));
        let service = RebornOutboundPreferencesService::new(store.clone(), registry);

        let listed = service
            .list_outbound_delivery_targets(caller("tenant-alpha", "user-alpha"))
            .await
            .expect("target list");
        assert_eq!(
            listed
                .targets
                .iter()
                .map(|entry| entry.target.channel.as_str())
                .collect::<Vec<_>>(),
            vec!["slack", "telegram"]
        );

        service
            .set_notification_channels(
                caller("tenant-alpha", "user-alpha"),
                RebornSetNotificationChannelsRequest {
                    target_ids: vec![target_id("telegram-alpha")],
                },
            )
            .await
            .expect("set target from second provider");
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
            vec!["telegram-alpha"]
        );
    }

    #[tokio::test]
    async fn target_registry_filters_non_final_reply_resolver_results() {
        let store =
            Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
        let registry = Arc::new(OutboundDeliveryTargetRegistry::new(vec![Arc::new(
            ResolvingOnlyTargetProvider {
                entry: target_entry("slack-progress", "reply:slack-progress", false),
            },
        )]));
        let service = RebornOutboundPreferencesService::new(store.clone(), registry);

        let error = service
            .set_notification_channels(
                caller("tenant-alpha", "user-alpha"),
                RebornSetNotificationChannelsRequest {
                    target_ids: vec![target_id("slack-progress")],
                },
            )
            .await
            .expect_err("non-final target resolver result is rejected");
        assert_eq!(error.code, ProductSurfaceErrorCode::InvalidRequest);
        assert_eq!(error.field.as_deref(), Some("target_ids"));
        assert_eq!(
            error.validation_code,
            Some(ProductSurfaceValidationCode::InvalidId)
        );
        assert!(
            store
                .load_communication_preference(CommunicationPreferenceKey::new(
                    tenant("tenant-alpha"),
                    user("user-alpha"),
                ))
                .await
                .expect("load preference")
                .is_none()
        );
    }

    #[tokio::test]
    async fn target_registry_returns_second_provider_match_after_first_provider_none() {
        let registry = OutboundDeliveryTargetRegistry::new(vec![
            Arc::new(NullResolvingTargetProvider),
            Arc::new(ResolvingOnlyTargetProvider {
                entry: target_entry("slack-alpha", "reply:slack-alpha", true),
            }),
        ]);

        let resolved_target = registry
            .resolve_outbound_delivery_target(
                &outbound_scope("tenant-alpha", "user-alpha"),
                &outbound_target_id("slack-alpha"),
            )
            .await
            .expect("resolve target");
        assert_eq!(
            resolved_target
                .as_ref()
                .map(|entry| entry.summary.target_id.as_str()),
            Some("slack-alpha")
        );

        let resolved_reply_target = registry
            .resolve_reply_target_binding(
                &outbound_scope("tenant-alpha", "user-alpha"),
                &reply_ref("reply:slack-alpha"),
            )
            .await
            .expect("resolve reply target");
        assert_eq!(
            resolved_reply_target
                .as_ref()
                .map(|entry| entry.summary.target_id.as_str()),
            Some("slack-alpha")
        );
    }

    /// A provider that ignores caller scoping: it returns one entry owned by a
    /// different `(tenant, user)` and one owned by the querying caller, for
    /// every caller. Models a provider whose own filtering is buggy or absent.
    struct MisbehavingUnscopedProvider {
        foreign: OutboundDeliveryTargetEntry,
        owned: OutboundDeliveryTargetEntry,
    }

    #[async_trait]
    impl OutboundDeliveryTargetProvider for MisbehavingUnscopedProvider {
        async fn list_outbound_delivery_targets(
            &self,
            _caller: &OutboundDeliveryTargetScope,
        ) -> Result<Vec<OutboundDeliveryTargetEntry>, OutboundError> {
            Ok(vec![self.foreign.clone(), self.owned.clone()])
        }
        // resolve_* deliberately use the trait defaults (list-then-find), which
        // filter only on capability/id — NOT on owner. This is the leak the
        // registry must close on top of the provider.
    }

    /// The registry must drop any fanned-out entry that does not belong to the
    /// querying caller, regardless of provider behavior — cross-caller
    /// isolation is structural at the registry, not a per-provider convention.
    /// (#6389 bucket-2 multi-tenant-safety hardening.)
    ///
    /// Red→green check: commenting out the `entry_owned_by_caller` guard in the
    /// two registry impls surfaces the tenant-B/user-B `foreign` entry through
    /// `list`/`resolve_*`, failing this test.
    #[tokio::test]
    async fn target_registry_drops_entries_not_owned_by_querying_caller() {
        let foreign = target_entry_owned_by(
            "slack-foreign",
            "reply:slack-foreign",
            "tenant-bravo",
            "user-bravo",
        );
        let owned = target_entry_owned_by(
            "slack-owned",
            "reply:slack-owned",
            "tenant-alpha",
            "user-alpha",
        );

        // Cover both registry implementations behind the shared provider trait.
        let mutable = MutableOutboundDeliveryTargetRegistry::default();
        mutable
            .register_provider(
                "misbehaving",
                Arc::new(MisbehavingUnscopedProvider {
                    foreign: foreign.clone(),
                    owned: owned.clone(),
                }),
            )
            .expect("register misbehaving provider");
        let immutable =
            OutboundDeliveryTargetRegistry::new(vec![Arc::new(MisbehavingUnscopedProvider {
                foreign: foreign.clone(),
                owned: owned.clone(),
            })]);

        let caller = outbound_scope("tenant-alpha", "user-alpha");
        let registries: [&dyn OutboundDeliveryTargetProvider; 2] = [&mutable, &immutable];
        for registry in registries {
            // list: only the caller-owned entry survives; the B-owned entry is
            // dropped by the registry even though the provider returned it.
            let listed = registry
                .list_outbound_delivery_targets(&caller)
                .await
                .expect("list");
            assert_eq!(
                listed
                    .iter()
                    .map(|entry| entry.summary.target_id.as_str())
                    .collect::<Vec<_>>(),
                vec!["slack-owned"],
                "only the querying caller's entry may surface from list"
            );

            // resolve by target id: the caller-owned id resolves; the foreign id
            // does not, even though the provider would return it.
            assert_eq!(
                registry
                    .resolve_outbound_delivery_target(&caller, &outbound_target_id("slack-owned"))
                    .await
                    .expect("resolve owned")
                    .map(|entry| entry.summary.target_id.as_str().to_string()),
                Some("slack-owned".to_string())
            );
            assert!(
                registry
                    .resolve_outbound_delivery_target(
                        &caller,
                        &outbound_target_id("slack-foreign"),
                    )
                    .await
                    .expect("resolve foreign")
                    .is_none(),
                "a target owned by another (tenant, user) must not resolve"
            );

            // resolve by reply-target binding: same asymmetry.
            assert_eq!(
                registry
                    .resolve_reply_target_binding(&caller, &reply_ref("reply:slack-owned"))
                    .await
                    .expect("resolve owned binding")
                    .map(|entry| entry.summary.target_id.as_str().to_string()),
                Some("slack-owned".to_string())
            );
            assert!(
                registry
                    .resolve_reply_target_binding(&caller, &reply_ref("reply:slack-foreign"))
                    .await
                    .expect("resolve foreign binding")
                    .is_none(),
                "a binding owned by another (tenant, user) must not resolve"
            );
        }
    }

    #[tokio::test]
    async fn target_registry_propagates_provider_failure() {
        let registry = OutboundDeliveryTargetRegistry::new(vec![Arc::new(FailingTargetProvider)]);

        let error = registry
            .list_outbound_delivery_targets(&outbound_scope("tenant-alpha", "user-alpha"))
            .await
            .expect_err("provider failure");

        assert!(matches!(error, OutboundError::Backend));
    }

    #[test]
    fn repository_error_mapping_distinguishes_authority_conflict_and_backend_errors() {
        for invalid_request_error in [
            OutboundError::InvalidRequest { reason: "bad" },
            OutboundError::SubscriptionScopeMismatch,
            OutboundError::DeliveryNotFound,
        ] {
            let mapped = map_outbound_repository_error(invalid_request_error);
            assert_eq!(mapped.code, ProductSurfaceErrorCode::InvalidRequest);
            assert_eq!(mapped.kind, ProductSurfaceErrorKind::Validation);
            assert_eq!(mapped.status_code, 400);
            assert!(!mapped.retryable);
        }

        let access_denied = map_outbound_repository_error(OutboundError::AccessDenied);
        assert_eq!(access_denied.code, ProductSurfaceErrorCode::Forbidden);
        assert_eq!(
            access_denied.kind,
            ProductSurfaceErrorKind::ParticipantDenied
        );
        assert_eq!(access_denied.status_code, 403);
        assert!(!access_denied.retryable);

        let cas_conflict = map_outbound_repository_error(OutboundError::CasConflict);
        assert_eq!(cas_conflict.code, ProductSurfaceErrorCode::Conflict);
        assert_eq!(cas_conflict.kind, ProductSurfaceErrorKind::Conflict);
        assert_eq!(cas_conflict.status_code, 409);
        assert!(!cas_conflict.retryable);

        let serialization = map_outbound_repository_error(OutboundError::Serialization);
        assert_unavailable_backend_error(serialization);
    }

    fn assert_unavailable_backend_error(error: ProductSurfaceError) {
        assert_eq!(error.code, ProductSurfaceErrorCode::Unavailable);
        assert_eq!(error.kind, ProductSurfaceErrorKind::ServiceUnavailable);
        assert_eq!(error.status_code, 503);
        assert!(error.retryable);
    }

    fn target_entry_owned_by(
        target_id_value: &str,
        reply_target: &str,
        owner_tenant: &str,
        owner_user: &str,
    ) -> OutboundDeliveryTargetEntry {
        OutboundDeliveryTargetEntry {
            owner: OutboundDeliveryTargetOwner::new(tenant(owner_tenant), user(owner_user)),
            ..target_entry(target_id_value, reply_target, true)
        }
    }

    fn outbound_scope(tenant_id: &str, user_id: &str) -> OutboundDeliveryTargetScope {
        OutboundDeliveryTargetScope::new(tenant(tenant_id), user(user_id))
    }
}
