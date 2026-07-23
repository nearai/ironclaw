use std::sync::Arc;

use super::{
    OutboundPreferencesProductFacade, RebornOutboundDeliveryModality,
    RebornOutboundDeliveryTargetCapabilities, RebornOutboundDeliveryTargetId,
    RebornOutboundDeliveryTargetListResponse, RebornOutboundDeliveryTargetOption,
    RebornOutboundDeliveryTargetStatus, RebornOutboundDeliveryTargetSummary,
    RebornOutboundPreferencesResponse, RebornServicesError, RebornServicesErrorCode,
    RebornServicesErrorKind, RebornSetOutboundPreferencesRequest, WebUiAuthenticatedCaller,
};
use async_trait::async_trait;
use chrono::Utc;
use ironclaw_outbound::{
    CommunicationPreferenceKey, CommunicationPreferenceRecord, CommunicationPreferenceRepository,
    OutboundDeliveryTargetEntry, OutboundDeliveryTargetId, OutboundDeliveryTargetProvider,
    OutboundDeliveryTargetScope, OutboundDeliveryTargetSummary, OutboundError,
    WriteCommunicationPreferenceRequest,
};
use ironclaw_turns::ReplyTargetBindingRef;

pub struct RebornOutboundPreferencesFacade {
    preferences: Arc<dyn CommunicationPreferenceRepository>,
    targets: Arc<dyn OutboundDeliveryTargetProvider>,
}

impl std::fmt::Debug for RebornOutboundPreferencesFacade {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RebornOutboundPreferencesFacade")
            .field("preferences", &"Arc<dyn CommunicationPreferenceRepository>")
            .field("targets", &"Arc<dyn OutboundDeliveryTargetProvider>")
            .finish()
    }
}

impl RebornOutboundPreferencesFacade {
    pub fn new(
        preferences: Arc<dyn CommunicationPreferenceRepository>,
        targets: Arc<dyn OutboundDeliveryTargetProvider>,
    ) -> Self {
        Self {
            preferences,
            targets,
        }
    }

    async fn response_for_record(
        &self,
        caller: &WebUiAuthenticatedCaller,
        record: Option<&CommunicationPreferenceRecord>,
    ) -> Result<RebornOutboundPreferencesResponse, RebornServicesError> {
        let (final_reply_target, final_reply_target_status) =
            match record.and_then(|record| record.final_reply_target.as_ref()) {
                Some(target) => match self.summary_for_reply_target(caller, target).await? {
                    Some(target) => (Some(target), RebornOutboundDeliveryTargetStatus::Available),
                    None => (None, RebornOutboundDeliveryTargetStatus::Unavailable),
                },
                None => (None, RebornOutboundDeliveryTargetStatus::NoneConfigured),
            };
        Ok(RebornOutboundPreferencesResponse {
            final_reply_target,
            final_reply_target_status,
            default_modality: RebornOutboundDeliveryModality::Text,
        })
    }

    fn response_for_resolved_final_reply_target(
        resolved_final_reply_target: Option<&OutboundDeliveryTargetEntry>,
    ) -> Result<RebornOutboundPreferencesResponse, RebornServicesError> {
        let final_reply_target = resolved_final_reply_target
            .map(|entry| reborn_summary_from_outbound(&entry.summary))
            .transpose()?;
        let final_reply_target_status = if final_reply_target.is_some() {
            RebornOutboundDeliveryTargetStatus::Available
        } else {
            RebornOutboundDeliveryTargetStatus::NoneConfigured
        };
        Ok(RebornOutboundPreferencesResponse {
            final_reply_target,
            final_reply_target_status,
            default_modality: RebornOutboundDeliveryModality::Text,
        })
    }

    async fn summary_for_reply_target(
        &self,
        caller: &WebUiAuthenticatedCaller,
        target: &ReplyTargetBindingRef,
    ) -> Result<Option<RebornOutboundDeliveryTargetSummary>, RebornServicesError> {
        self.targets
            .resolve_reply_target_binding(&target_scope(caller), target)
            .await
            .map_err(map_outbound_repository_error)?
            .map(|entry| reborn_summary_from_outbound(&entry.summary))
            .transpose()
    }

    async fn resolve_final_reply_target(
        &self,
        caller: &WebUiAuthenticatedCaller,
        target_id: &RebornOutboundDeliveryTargetId,
    ) -> Result<OutboundDeliveryTargetEntry, RebornServicesError> {
        let target_id = outbound_target_id_from_reborn(target_id)?;
        self.targets
            .resolve_outbound_delivery_target(&target_scope(caller), &target_id)
            .await
            .map_err(map_outbound_repository_error)?
            .ok_or_else(outbound_target_not_found)
    }

    /// Invariant: `WebUiAuthenticatedCaller` must come from the authenticated
    /// product/session boundary, never from request-body tenant/user fields.
    /// This key and target-provider scope intentionally share the same
    /// verified caller identity.
    fn key(caller: &WebUiAuthenticatedCaller) -> CommunicationPreferenceKey {
        CommunicationPreferenceKey::personal(caller.tenant_id.clone(), caller.user_id.clone())
    }
}

#[async_trait]
impl OutboundPreferencesProductFacade for RebornOutboundPreferencesFacade {
    async fn get_outbound_preferences(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornOutboundPreferencesResponse, RebornServicesError> {
        let record = self
            .preferences
            .load_communication_preference(Self::key(&caller))
            .await
            .map_err(map_outbound_repository_error)?;
        self.response_for_record(&caller, record.as_ref().map(|record| &record.record))
            .await
    }

    async fn set_outbound_preferences(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornSetOutboundPreferencesRequest,
    ) -> Result<RebornOutboundPreferencesResponse, RebornServicesError> {
        let key = Self::key(&caller);
        let scope = key.scope.clone();
        let resolved_final_reply_target = match request.final_reply_target_id.as_ref() {
            Some(target_id) => Some(self.resolve_final_reply_target(&caller, target_id).await?),
            None => None,
        };
        let final_reply_target = resolved_final_reply_target
            .as_ref()
            .map(|entry| entry.reply_target_binding_ref.clone());
        let existing = self
            .preferences
            .load_communication_preference(key)
            .await
            .map_err(map_outbound_repository_error)?;
        let user_id = caller.user_id.clone();
        let updated_at = Utc::now();
        self.preferences
            .write_communication_preference(WriteCommunicationPreferenceRequest {
                expected_version: existing.as_ref().map(|existing| existing.version),
                record: CommunicationPreferenceRecord {
                    scope,
                    final_reply_target,
                    progress_target: existing
                        .as_ref()
                        .and_then(|record| record.record.progress_target.clone()),
                    approval_prompt_target: existing
                        .as_ref()
                        .and_then(|record| record.record.approval_prompt_target.clone()),
                    auth_prompt_target: existing
                        .as_ref()
                        .and_then(|record| record.record.auth_prompt_target.clone()),
                    default_modality: existing
                        .as_ref()
                        .and_then(|record| record.record.default_modality),
                    updated_at,
                    updated_by: user_id.clone(),
                },
            })
            .await
            .map_err(map_outbound_repository_error)?;
        Self::response_for_resolved_final_reply_target(resolved_final_reply_target.as_ref())
    }

    async fn list_outbound_delivery_targets(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornOutboundDeliveryTargetListResponse, RebornServicesError> {
        let targets = self
            .targets
            .list_outbound_delivery_targets(&target_scope(&caller))
            .await
            .map_err(map_outbound_repository_error)?
            .into_iter()
            .filter(|entry| entry.capabilities.final_replies)
            .map(|entry| {
                Ok(RebornOutboundDeliveryTargetOption {
                    target: reborn_summary_from_outbound(&entry.summary)?,
                    capabilities: reborn_capabilities_from_outbound(&entry.capabilities),
                })
            })
            .collect::<Result<Vec<_>, RebornServicesError>>()?;
        Ok(RebornOutboundDeliveryTargetListResponse {
            targets,
            next_cursor: None,
        })
    }
}

fn target_scope(caller: &WebUiAuthenticatedCaller) -> OutboundDeliveryTargetScope {
    OutboundDeliveryTargetScope::new(caller.tenant_id.clone(), caller.user_id.clone())
}

fn outbound_target_id_from_reborn(
    target_id: &RebornOutboundDeliveryTargetId,
) -> Result<OutboundDeliveryTargetId, RebornServicesError> {
    OutboundDeliveryTargetId::new(target_id.as_str()).map_err(|_| RebornServicesError {
        code: RebornServicesErrorCode::InvalidRequest,
        kind: RebornServicesErrorKind::Validation,
        status_code: 400,
        retryable: false,
        field: Some("final_reply_target_id".to_string()),
        validation_code: None,
    })
}

fn reborn_summary_from_outbound(
    summary: &OutboundDeliveryTargetSummary,
) -> Result<RebornOutboundDeliveryTargetSummary, RebornServicesError> {
    let target_id = RebornOutboundDeliveryTargetId::new(summary.target_id.as_str())
        .map_err(|_| outbound_target_projection_error())?;
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
    }
}

fn outbound_target_projection_error() -> RebornServicesError {
    RebornServicesError {
        code: RebornServicesErrorCode::Internal,
        kind: RebornServicesErrorKind::Internal,
        status_code: 500,
        retryable: false,
        field: None,
        validation_code: None,
    }
}

fn outbound_target_not_found() -> RebornServicesError {
    RebornServicesError {
        code: RebornServicesErrorCode::NotFound,
        kind: RebornServicesErrorKind::NotFound,
        status_code: 404,
        retryable: false,
        field: Some("final_reply_target_id".to_string()),
        validation_code: None,
    }
}

fn map_outbound_repository_error(error: OutboundError) -> RebornServicesError {
    match error {
        OutboundError::InvalidRequest { .. }
        | OutboundError::PreferenceTargetMissing { .. }
        | OutboundError::SubscriptionScopeMismatch
        | OutboundError::DeliveryNotFound => RebornServicesError {
            code: RebornServicesErrorCode::InvalidRequest,
            kind: RebornServicesErrorKind::Validation,
            status_code: 400,
            retryable: false,
            field: None,
            validation_code: None,
        },
        OutboundError::AccessDenied => RebornServicesError {
            code: RebornServicesErrorCode::Forbidden,
            kind: RebornServicesErrorKind::ParticipantDenied,
            status_code: 403,
            retryable: false,
            field: None,
            validation_code: None,
        },
        OutboundError::CasConflict => RebornServicesError {
            code: RebornServicesErrorCode::Conflict,
            kind: RebornServicesErrorKind::Conflict,
            status_code: 409,
            retryable: false,
            field: None,
            validation_code: None,
        },
        OutboundError::Backend | OutboundError::Serialization => RebornServicesError {
            code: RebornServicesErrorCode::Unavailable,
            kind: RebornServicesErrorKind::ServiceUnavailable,
            status_code: 503,
            retryable: true,
            field: None,
            validation_code: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Mutex};

    use ironclaw_host_api::{TenantId, UserId};
    use ironclaw_outbound::{
        CommunicationModality, CommunicationPreferenceRepository, CommunicationPreferenceVersion,
        DeliveryDefaultScope, DeliveryTargetCapabilities, MutableOutboundDeliveryTargetRegistry,
        OutboundDeliveryTargetOwner, OutboundDeliveryTargetRegistry,
        VersionedCommunicationPreferenceRecord,
    };

    use super::*;

    #[derive(Default)]
    struct FakeTargetProvider {
        by_user: Mutex<HashMap<String, Vec<OutboundDeliveryTargetEntry>>>,
    }

    impl FakeTargetProvider {
        fn insert(&self, user_id: &str, entry: OutboundDeliveryTargetEntry) {
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
            Ok(
                (self.entry.reply_target_binding_ref.as_str() == target.as_str())
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
                    final_reply_target: None,
                    progress_target: None,
                    approval_prompt_target: None,
                    auth_prompt_target: None,
                    default_modality: None,
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

    #[tokio::test]
    async fn get_preferences_projects_stored_final_target_for_authenticated_user() {
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
        let facade = RebornOutboundPreferencesFacade::new(store, provider);

        let response = facade
            .get_outbound_preferences(caller("tenant-alpha", "user-alpha"))
            .await
            .expect("preferences response");

        assert_eq!(
            response
                .final_reply_target
                .as_ref()
                .map(|target| target.target_id.as_str()),
            Some("slack-alpha")
        );
        assert_eq!(
            response.final_reply_target_status,
            RebornOutboundDeliveryTargetStatus::Available
        );

        let other_user = facade
            .get_outbound_preferences(caller("tenant-alpha", "user-bravo"))
            .await
            .expect("other user preferences");
        assert!(other_user.final_reply_target.is_none());
        assert_eq!(
            other_user.final_reply_target_status,
            RebornOutboundDeliveryTargetStatus::NoneConfigured
        );
    }

    #[tokio::test]
    async fn get_preferences_returns_none_when_stored_target_not_in_provider() {
        let store =
            Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
        let provider = Arc::new(FakeTargetProvider::default());
        seed_record(
            store.as_ref(),
            "tenant-alpha",
            "user-alpha",
            Some(reply_ref("reply:slack-alpha")),
        )
        .await;
        let facade = RebornOutboundPreferencesFacade::new(store, provider);

        let response = facade
            .get_outbound_preferences(caller("tenant-alpha", "user-alpha"))
            .await
            .expect("preferences response");

        assert!(response.final_reply_target.is_none());
        assert_eq!(
            response.final_reply_target_status,
            RebornOutboundDeliveryTargetStatus::Unavailable
        );
    }

    #[tokio::test]
    async fn get_preferences_maps_backend_read_error_to_unavailable() {
        let facade = RebornOutboundPreferencesFacade::new(
            Arc::new(LoadFailingPreferenceRepository),
            Arc::new(FakeTargetProvider::default()),
        );

        let error = facade
            .get_outbound_preferences(caller("tenant-alpha", "user-alpha"))
            .await
            .expect_err("backend read failure");

        assert_unavailable_backend_error(error);
    }

    #[tokio::test]
    async fn set_preferences_validates_target_id_before_writing_reply_target() {
        let store =
            Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
        let provider = Arc::new(FakeTargetProvider::default());
        provider.insert(
            "user-alpha",
            target_entry("slack-alpha", "reply:slack-alpha", true),
        );
        let facade = RebornOutboundPreferencesFacade::new(store.clone(), provider);

        let response = facade
            .set_outbound_preferences(
                caller("tenant-alpha", "user-alpha"),
                RebornSetOutboundPreferencesRequest {
                    final_reply_target_id: Some(target_id("slack-alpha")),
                },
            )
            .await
            .expect("set valid target");

        assert_eq!(
            response
                .final_reply_target
                .as_ref()
                .map(|target| target.target_id.as_str()),
            Some("slack-alpha")
        );
        assert_eq!(
            response.final_reply_target_status,
            RebornOutboundDeliveryTargetStatus::Available
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
                .final_reply_target
                .as_ref()
                .map(|target| target.as_str()),
            Some("reply:slack-alpha")
        );
        assert!(stored.record.default_modality.is_none());

        let error = facade
            .set_outbound_preferences(
                caller("tenant-alpha", "user-alpha"),
                RebornSetOutboundPreferencesRequest {
                    final_reply_target_id: Some(target_id("slack-missing")),
                },
            )
            .await
            .expect_err("reject unknown target");
        assert_eq!(error.code, RebornServicesErrorCode::NotFound);
        assert_eq!(error.field.as_deref(), Some("final_reply_target_id"));
    }

    #[tokio::test]
    async fn set_preferences_maps_backend_write_error_to_unavailable() {
        let provider = Arc::new(FakeTargetProvider::default());
        provider.insert(
            "user-alpha",
            target_entry("slack-alpha", "reply:slack-alpha", true),
        );
        let facade = RebornOutboundPreferencesFacade::new(
            Arc::new(PutFailingPreferenceRepository),
            provider,
        );

        let error = facade
            .set_outbound_preferences(
                caller("tenant-alpha", "user-alpha"),
                RebornSetOutboundPreferencesRequest {
                    final_reply_target_id: Some(target_id("slack-alpha")),
                },
            )
            .await
            .expect_err("backend write failure");

        assert_unavailable_backend_error(error);
    }

    #[tokio::test]
    async fn set_preferences_maps_backend_read_error_before_resolving_target() {
        let provider = Arc::new(FakeTargetProvider::default());
        provider.insert(
            "user-alpha",
            target_entry("slack-alpha", "reply:slack-alpha", true),
        );
        let facade = RebornOutboundPreferencesFacade::new(
            Arc::new(LoadFailingPreferenceRepository),
            provider,
        );

        let error = facade
            .set_outbound_preferences(
                caller("tenant-alpha", "user-alpha"),
                RebornSetOutboundPreferencesRequest {
                    final_reply_target_id: Some(target_id("slack-alpha")),
                },
            )
            .await
            .expect_err("backend read failure");

        assert_unavailable_backend_error(error);
    }

    #[tokio::test]
    async fn set_preferences_maps_write_cas_conflict_to_conflict() {
        let provider = Arc::new(FakeTargetProvider::default());
        provider.insert(
            "user-alpha",
            target_entry("slack-alpha", "reply:slack-alpha", true),
        );
        let facade = RebornOutboundPreferencesFacade::new(
            Arc::new(CasConflictingPreferenceRepository),
            provider,
        );

        let error = facade
            .set_outbound_preferences(
                caller("tenant-alpha", "user-alpha"),
                RebornSetOutboundPreferencesRequest {
                    final_reply_target_id: Some(target_id("slack-alpha")),
                },
            )
            .await
            .expect_err("conflicting preference write");

        assert_eq!(error.code, RebornServicesErrorCode::Conflict);
        assert_eq!(error.kind, RebornServicesErrorKind::Conflict);
        assert_eq!(error.status_code, 409);
        assert!(!error.retryable);
    }

    #[tokio::test]
    async fn set_preferences_with_none_target_on_new_user_creates_empty_record() {
        let store =
            Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
        let provider = Arc::new(FakeTargetProvider::default());
        let facade = RebornOutboundPreferencesFacade::new(store.clone(), provider);

        let response = facade
            .set_outbound_preferences(
                caller("tenant-alpha", "user-new"),
                RebornSetOutboundPreferencesRequest {
                    final_reply_target_id: None,
                },
            )
            .await
            .expect("new-user clear");

        assert!(response.final_reply_target.is_none());
        assert_eq!(
            response.final_reply_target_status,
            RebornOutboundDeliveryTargetStatus::NoneConfigured
        );
        let stored = store
            .load_communication_preference(CommunicationPreferenceKey::new(
                tenant("tenant-alpha"),
                user("user-new"),
            ))
            .await
            .expect("load stored record")
            .expect("stored record");
        assert!(stored.record.final_reply_target.is_none());
        assert!(stored.record.progress_target.is_none());
        assert!(stored.record.approval_prompt_target.is_none());
        assert!(stored.record.auth_prompt_target.is_none());
        assert!(stored.record.default_modality.is_none());
    }

    #[tokio::test]
    async fn target_provider_errors_are_propagated_by_get_set_and_list() {
        let store =
            Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
        seed_record(
            store.as_ref(),
            "tenant-alpha",
            "user-alpha",
            Some(reply_ref("reply:slack-alpha")),
        )
        .await;
        let facade = RebornOutboundPreferencesFacade::new(store, Arc::new(FailingTargetProvider));

        let get_error = facade
            .get_outbound_preferences(caller("tenant-alpha", "user-alpha"))
            .await
            .expect_err("get target provider failure");
        assert_unavailable_backend_error(get_error);

        let set_error = facade
            .set_outbound_preferences(
                caller("tenant-alpha", "user-alpha"),
                RebornSetOutboundPreferencesRequest {
                    final_reply_target_id: Some(target_id("slack-alpha")),
                },
            )
            .await
            .expect_err("set target provider failure");
        assert_unavailable_backend_error(set_error);

        let list_error = facade
            .list_outbound_delivery_targets(caller("tenant-alpha", "user-alpha"))
            .await
            .expect_err("list target provider failure");
        assert_unavailable_backend_error(list_error);
    }

    #[tokio::test]
    async fn clear_preferences_preserves_non_final_slots() {
        let store =
            Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
        let provider = Arc::new(FakeTargetProvider::default());
        seed_record(
            store.as_ref(),
            "tenant-alpha",
            "user-alpha",
            Some(reply_ref("reply:slack-alpha")),
        )
        .await;
        let facade = RebornOutboundPreferencesFacade::new(store.clone(), provider);

        let response = facade
            .set_outbound_preferences(
                caller("tenant-alpha", "user-alpha"),
                RebornSetOutboundPreferencesRequest {
                    final_reply_target_id: None,
                },
            )
            .await
            .expect("clear target");

        assert!(response.final_reply_target.is_none());
        assert_eq!(
            response.final_reply_target_status,
            RebornOutboundDeliveryTargetStatus::NoneConfigured
        );
        let stored = store
            .load_communication_preference(CommunicationPreferenceKey::new(
                tenant("tenant-alpha"),
                user("user-alpha"),
            ))
            .await
            .expect("load stored record")
            .expect("stored record");
        assert!(stored.record.final_reply_target.is_none());
        assert_eq!(
            stored
                .record
                .progress_target
                .as_ref()
                .map(|target| target.as_str()),
            Some("reply:progress")
        );
        assert_eq!(
            stored
                .record
                .approval_prompt_target
                .as_ref()
                .map(|target| target.as_str()),
            Some("reply:approval")
        );
        assert_eq!(
            stored
                .record
                .auth_prompt_target
                .as_ref()
                .map(|target| target.as_str()),
            Some("reply:auth")
        );
        assert_eq!(
            stored.record.default_modality,
            Some(CommunicationModality::Voice)
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
        let facade = RebornOutboundPreferencesFacade::new(store, provider);

        let response = facade
            .list_outbound_delivery_targets(caller("tenant-alpha", "user-alpha"))
            .await
            .expect("target list");

        assert_eq!(response.targets.len(), 1);
        assert_eq!(response.targets[0].target.target_id.as_str(), "slack-alpha");
        assert!(response.next_cursor.is_none());
    }

    #[tokio::test]
    async fn preference_facade_uses_authority_resolver_not_public_target_list_for_write_and_read() {
        let store =
            Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
        let provider = Arc::new(ResolvingOnlyTargetProvider {
            entry: target_entry("slack-alpha", "reply:slack-alpha", true),
        });
        let facade = RebornOutboundPreferencesFacade::new(store.clone(), provider);

        let listed = facade
            .list_outbound_delivery_targets(caller("tenant-alpha", "user-alpha"))
            .await
            .expect("list targets");
        assert!(listed.targets.is_empty());

        let set_response = facade
            .set_outbound_preferences(
                caller("tenant-alpha", "user-alpha"),
                RebornSetOutboundPreferencesRequest {
                    final_reply_target_id: Some(target_id("slack-alpha")),
                },
            )
            .await
            .expect("set target through resolver");
        assert_eq!(
            set_response.final_reply_target_status,
            RebornOutboundDeliveryTargetStatus::Available
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
                .final_reply_target
                .as_ref()
                .map(|target| target.as_str()),
            Some("reply:slack-alpha")
        );

        let get_response = facade
            .get_outbound_preferences(caller("tenant-alpha", "user-alpha"))
            .await
            .expect("get target through resolver");
        assert_eq!(
            get_response.final_reply_target_status,
            RebornOutboundDeliveryTargetStatus::Available
        );
        assert_eq!(
            get_response
                .final_reply_target
                .as_ref()
                .map(|target| target.target_id.as_str()),
            Some("slack-alpha")
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
        let facade = RebornOutboundPreferencesFacade::new(store.clone(), registry);

        let listed = facade
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

        facade
            .set_outbound_preferences(
                caller("tenant-alpha", "user-alpha"),
                RebornSetOutboundPreferencesRequest {
                    final_reply_target_id: Some(target_id("telegram-alpha")),
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
                .final_reply_target
                .as_ref()
                .map(|target| target.as_str()),
            Some("reply:telegram-alpha")
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
        let facade = RebornOutboundPreferencesFacade::new(store.clone(), registry);

        let error = facade
            .set_outbound_preferences(
                caller("tenant-alpha", "user-alpha"),
                RebornSetOutboundPreferencesRequest {
                    final_reply_target_id: Some(target_id("slack-progress")),
                },
            )
            .await
            .expect_err("non-final target resolver result is rejected");
        assert_eq!(error.code, RebornServicesErrorCode::NotFound);
        assert_eq!(error.field.as_deref(), Some("final_reply_target_id"));
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

    #[tokio::test]
    async fn target_registry_propagates_resolver_failure_for_get_and_set() {
        let store =
            Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
        seed_record(
            store.as_ref(),
            "tenant-alpha",
            "user-alpha",
            Some(reply_ref("reply:slack-alpha")),
        )
        .await;
        let facade =
            RebornOutboundPreferencesFacade::new(store, Arc::new(ResolveFailingTargetProvider));

        let get_error = facade
            .get_outbound_preferences(caller("tenant-alpha", "user-alpha"))
            .await
            .expect_err("get target resolver failure");
        assert_unavailable_backend_error(get_error);

        let set_error = facade
            .set_outbound_preferences(
                caller("tenant-alpha", "user-alpha"),
                RebornSetOutboundPreferencesRequest {
                    final_reply_target_id: Some(target_id("slack-alpha")),
                },
            )
            .await
            .expect_err("set target resolver failure");
        assert_unavailable_backend_error(set_error);
    }

    #[test]
    fn repository_error_mapping_distinguishes_authority_conflict_and_backend_errors() {
        for invalid_request_error in [
            OutboundError::InvalidRequest { reason: "bad" },
            OutboundError::SubscriptionScopeMismatch,
            OutboundError::DeliveryNotFound,
        ] {
            let mapped = map_outbound_repository_error(invalid_request_error);
            assert_eq!(mapped.code, RebornServicesErrorCode::InvalidRequest);
            assert_eq!(mapped.kind, RebornServicesErrorKind::Validation);
            assert_eq!(mapped.status_code, 400);
            assert!(!mapped.retryable);
        }

        let access_denied = map_outbound_repository_error(OutboundError::AccessDenied);
        assert_eq!(access_denied.code, RebornServicesErrorCode::Forbidden);
        assert_eq!(
            access_denied.kind,
            RebornServicesErrorKind::ParticipantDenied
        );
        assert_eq!(access_denied.status_code, 403);
        assert!(!access_denied.retryable);

        let cas_conflict = map_outbound_repository_error(OutboundError::CasConflict);
        assert_eq!(cas_conflict.code, RebornServicesErrorCode::Conflict);
        assert_eq!(cas_conflict.kind, RebornServicesErrorKind::Conflict);
        assert_eq!(cas_conflict.status_code, 409);
        assert!(!cas_conflict.retryable);

        let serialization = map_outbound_repository_error(OutboundError::Serialization);
        assert_unavailable_backend_error(serialization);
    }

    fn assert_unavailable_backend_error(error: RebornServicesError) {
        assert_eq!(error.code, RebornServicesErrorCode::Unavailable);
        assert_eq!(error.kind, RebornServicesErrorKind::ServiceUnavailable);
        assert_eq!(error.status_code, 503);
        assert!(error.retryable);
    }

    fn target_entry(
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

    fn target_entry_for_channel(
        target_id_value: &str,
        channel: &str,
        display_name: &str,
        reply_target: &str,
        final_replies: bool,
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
                modalities: Vec::new(),
            },
            reply_target_binding_ref: reply_ref(reply_target),
            // Existing registry-path tests query as tenant-alpha/user-alpha, so
            // fixtures claim that owner and survive the caller-scoping filter.
            owner: OutboundDeliveryTargetOwner::new(tenant("tenant-alpha"), user("user-alpha")),
        }
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

    async fn seed_record(
        store: &dyn CommunicationPreferenceRepository,
        tenant_id: &str,
        user_id: &str,
        final_reply_target: Option<ReplyTargetBindingRef>,
    ) {
        store
            .put_communication_preference(CommunicationPreferenceRecord {
                scope: DeliveryDefaultScope::personal(tenant(tenant_id), user(user_id)),
                final_reply_target,
                progress_target: Some(reply_ref("reply:progress")),
                approval_prompt_target: Some(reply_ref("reply:approval")),
                auth_prompt_target: Some(reply_ref("reply:auth")),
                default_modality: Some(CommunicationModality::Voice),
                updated_at: Utc::now(),
                updated_by: user(user_id),
            })
            .await
            .expect("seed communication preference");
    }

    fn caller(tenant_id: &str, user_id: &str) -> WebUiAuthenticatedCaller {
        WebUiAuthenticatedCaller::new(tenant(tenant_id), user(user_id), None, None)
    }

    fn outbound_scope(tenant_id: &str, user_id: &str) -> OutboundDeliveryTargetScope {
        OutboundDeliveryTargetScope::new(tenant(tenant_id), user(user_id))
    }

    fn tenant(value: &str) -> TenantId {
        TenantId::new(value).expect("valid tenant")
    }

    fn user(value: &str) -> UserId {
        UserId::new(value).expect("valid user")
    }

    fn reply_ref(value: &str) -> ReplyTargetBindingRef {
        ReplyTargetBindingRef::new(value).expect("valid reply target")
    }

    fn target_id(value: &str) -> RebornOutboundDeliveryTargetId {
        RebornOutboundDeliveryTargetId::new(value).expect("valid target id")
    }

    fn outbound_target_id(value: &str) -> OutboundDeliveryTargetId {
        OutboundDeliveryTargetId::new(value).expect("valid target id")
    }
}
