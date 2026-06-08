use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_outbound::{
    CommunicationPreferenceKey, CommunicationPreferenceRecord, CommunicationPreferenceRepository,
    OutboundError,
};
use ironclaw_product_workflow::{
    OutboundPreferencesProductFacade, RebornOutboundDeliveryModality,
    RebornOutboundDeliveryTargetCapabilities, RebornOutboundDeliveryTargetId,
    RebornOutboundDeliveryTargetListResponse, RebornOutboundDeliveryTargetOption,
    RebornOutboundDeliveryTargetSummary, RebornOutboundPreferencesResponse, RebornServicesError,
    RebornServicesErrorCode, RebornServicesErrorKind, RebornSetOutboundPreferencesRequest,
    WebUiAuthenticatedCaller,
};
use ironclaw_turns::ReplyTargetBindingRef;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OutboundDeliveryTargetEntry {
    pub(crate) summary: RebornOutboundDeliveryTargetSummary,
    pub(crate) capabilities: RebornOutboundDeliveryTargetCapabilities,
    pub(crate) reply_target_binding_ref: ReplyTargetBindingRef,
}

#[async_trait]
pub(crate) trait OutboundDeliveryTargetInventory: Send + Sync {
    async fn list_outbound_delivery_targets(
        &self,
        caller: &WebUiAuthenticatedCaller,
    ) -> Result<Vec<OutboundDeliveryTargetEntry>, RebornServicesError>;
}

#[derive(Debug, Default)]
pub(crate) struct EmptyOutboundDeliveryTargetInventory;

#[async_trait]
impl OutboundDeliveryTargetInventory for EmptyOutboundDeliveryTargetInventory {
    async fn list_outbound_delivery_targets(
        &self,
        _caller: &WebUiAuthenticatedCaller,
    ) -> Result<Vec<OutboundDeliveryTargetEntry>, RebornServicesError> {
        Ok(Vec::new())
    }
}

pub(crate) struct RebornOutboundPreferencesFacade {
    preferences: Arc<dyn CommunicationPreferenceRepository>,
    targets: Arc<dyn OutboundDeliveryTargetInventory>,
}

impl std::fmt::Debug for RebornOutboundPreferencesFacade {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RebornOutboundPreferencesFacade")
            .field("preferences", &"Arc<dyn CommunicationPreferenceRepository>")
            .field("targets", &"Arc<dyn OutboundDeliveryTargetInventory>")
            .finish()
    }
}

impl RebornOutboundPreferencesFacade {
    pub(crate) fn new(
        preferences: Arc<dyn CommunicationPreferenceRepository>,
        targets: Arc<dyn OutboundDeliveryTargetInventory>,
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
        let final_reply_target = match record.and_then(|record| record.final_reply_target.as_ref())
        {
            Some(target) => self.summary_for_reply_target(caller, target).await?,
            None => None,
        };
        Ok(RebornOutboundPreferencesResponse {
            final_reply_target,
            default_modality: RebornOutboundDeliveryModality::Text,
        })
    }

    async fn summary_for_reply_target(
        &self,
        caller: &WebUiAuthenticatedCaller,
        target: &ReplyTargetBindingRef,
    ) -> Result<Option<RebornOutboundDeliveryTargetSummary>, RebornServicesError> {
        Ok(self
            .find_target_entry(caller, |entry| {
                entry.reply_target_binding_ref.as_str() == target.as_str()
            })
            .await?
            .map(|entry| entry.summary))
    }

    async fn resolve_final_reply_target(
        &self,
        caller: &WebUiAuthenticatedCaller,
        target_id: &RebornOutboundDeliveryTargetId,
    ) -> Result<OutboundDeliveryTargetEntry, RebornServicesError> {
        self.find_target_entry(caller, |entry| {
            entry.summary.target_id.as_str() == target_id.as_str()
        })
        .await?
        .ok_or_else(outbound_target_not_found)
    }

    async fn find_target_entry(
        &self,
        caller: &WebUiAuthenticatedCaller,
        predicate: impl Fn(&OutboundDeliveryTargetEntry) -> bool,
    ) -> Result<Option<OutboundDeliveryTargetEntry>, RebornServicesError> {
        let targets = self.targets.list_outbound_delivery_targets(caller).await?;
        Ok(targets
            .into_iter()
            .find(|entry| entry.capabilities.final_replies && predicate(entry)))
    }

    /// Invariant: `WebUiAuthenticatedCaller` must come from the authenticated
    /// product/session boundary, never from request-body tenant/user fields.
    /// This key and target-inventory scope intentionally share the same
    /// verified caller identity.
    fn key(caller: &WebUiAuthenticatedCaller) -> CommunicationPreferenceKey {
        CommunicationPreferenceKey::new(caller.tenant_id.clone(), caller.user_id.clone())
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
        self.response_for_record(&caller, record.as_ref()).await
    }

    async fn set_outbound_preferences(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornSetOutboundPreferencesRequest,
    ) -> Result<RebornOutboundPreferencesResponse, RebornServicesError> {
        let key = Self::key(&caller);
        let existing = self
            .preferences
            .load_communication_preference(key)
            .await
            .map_err(map_outbound_repository_error)?;
        let resolved_final_reply_target = match request.final_reply_target_id.as_ref() {
            Some(target_id) => Some(self.resolve_final_reply_target(&caller, target_id).await?),
            None => None,
        };
        let now = Utc::now();
        let record = CommunicationPreferenceRecord {
            tenant_id: caller.tenant_id.clone(),
            user_id: caller.user_id.clone(),
            final_reply_target: resolved_final_reply_target
                .as_ref()
                .map(|entry| entry.reply_target_binding_ref.clone()),
            progress_target: existing
                .as_ref()
                .and_then(|record| record.progress_target.clone()),
            approval_prompt_target: existing
                .as_ref()
                .and_then(|record| record.approval_prompt_target.clone()),
            auth_prompt_target: existing
                .as_ref()
                .and_then(|record| record.auth_prompt_target.clone()),
            default_modality: existing.as_ref().and_then(|record| record.default_modality),
            updated_at: now,
            updated_by: caller.user_id.clone(),
        };
        self.preferences
            .put_communication_preference(record)
            .await
            .map_err(map_outbound_repository_error)?;
        Ok(RebornOutboundPreferencesResponse {
            final_reply_target: resolved_final_reply_target.map(|entry| entry.summary),
            default_modality: RebornOutboundDeliveryModality::Text,
        })
    }

    async fn list_outbound_delivery_targets(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornOutboundDeliveryTargetListResponse, RebornServicesError> {
        let targets = self
            .targets
            .list_outbound_delivery_targets(&caller)
            .await?
            .into_iter()
            .filter(|entry| entry.capabilities.final_replies)
            .map(|entry| RebornOutboundDeliveryTargetOption {
                target: entry.summary,
                capabilities: entry.capabilities,
            })
            .collect();
        Ok(RebornOutboundDeliveryTargetListResponse {
            targets,
            next_cursor: None,
        })
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
        CommunicationModality, CommunicationPreferenceRepository, InMemoryOutboundStateStore,
    };

    use super::*;

    #[derive(Default)]
    struct FakeTargetInventory {
        by_user: Mutex<HashMap<String, Vec<OutboundDeliveryTargetEntry>>>,
    }

    impl FakeTargetInventory {
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
    impl OutboundDeliveryTargetInventory for FakeTargetInventory {
        async fn list_outbound_delivery_targets(
            &self,
            caller: &WebUiAuthenticatedCaller,
        ) -> Result<Vec<OutboundDeliveryTargetEntry>, RebornServicesError> {
            Ok(self
                .by_user
                .lock()
                .expect("lock")
                .get(caller.user_id.as_str())
                .cloned()
                .unwrap_or_default())
        }
    }

    struct FailingTargetInventory;

    #[async_trait]
    impl OutboundDeliveryTargetInventory for FailingTargetInventory {
        async fn list_outbound_delivery_targets(
            &self,
            _caller: &WebUiAuthenticatedCaller,
        ) -> Result<Vec<OutboundDeliveryTargetEntry>, RebornServicesError> {
            Err(RebornServicesError {
                code: RebornServicesErrorCode::Unavailable,
                kind: RebornServicesErrorKind::ServiceUnavailable,
                status_code: 503,
                retryable: true,
                field: None,
                validation_code: None,
            })
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
        ) -> Result<Option<CommunicationPreferenceRecord>, OutboundError> {
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
        ) -> Result<Option<CommunicationPreferenceRecord>, OutboundError> {
            Ok(None)
        }
    }

    #[tokio::test]
    async fn get_preferences_projects_stored_final_target_for_authenticated_user() {
        let store = Arc::new(InMemoryOutboundStateStore::default());
        let inventory = Arc::new(FakeTargetInventory::default());
        inventory.insert(
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
        let facade = RebornOutboundPreferencesFacade::new(store, inventory);

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

        let other_user = facade
            .get_outbound_preferences(caller("tenant-alpha", "user-bravo"))
            .await
            .expect("other user preferences");
        assert!(other_user.final_reply_target.is_none());
    }

    #[tokio::test]
    async fn get_preferences_returns_none_when_stored_target_not_in_inventory() {
        let store = Arc::new(InMemoryOutboundStateStore::default());
        let inventory = Arc::new(FakeTargetInventory::default());
        seed_record(
            store.as_ref(),
            "tenant-alpha",
            "user-alpha",
            Some(reply_ref("reply:slack-alpha")),
        )
        .await;
        let facade = RebornOutboundPreferencesFacade::new(store, inventory);

        let response = facade
            .get_outbound_preferences(caller("tenant-alpha", "user-alpha"))
            .await
            .expect("preferences response");

        assert!(response.final_reply_target.is_none());
    }

    #[tokio::test]
    async fn get_preferences_maps_backend_read_error_to_unavailable() {
        let facade = RebornOutboundPreferencesFacade::new(
            Arc::new(LoadFailingPreferenceRepository),
            Arc::new(FakeTargetInventory::default()),
        );

        let error = facade
            .get_outbound_preferences(caller("tenant-alpha", "user-alpha"))
            .await
            .expect_err("backend read failure");

        assert_unavailable_backend_error(error);
    }

    #[tokio::test]
    async fn set_preferences_validates_target_id_before_writing_reply_target() {
        let store = Arc::new(InMemoryOutboundStateStore::default());
        let inventory = Arc::new(FakeTargetInventory::default());
        inventory.insert(
            "user-alpha",
            target_entry("slack-alpha", "reply:slack-alpha", true),
        );
        let facade = RebornOutboundPreferencesFacade::new(store.clone(), inventory);

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
                .final_reply_target
                .as_ref()
                .map(|target| target.as_str()),
            Some("reply:slack-alpha")
        );
        assert!(stored.default_modality.is_none());

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
        let inventory = Arc::new(FakeTargetInventory::default());
        inventory.insert(
            "user-alpha",
            target_entry("slack-alpha", "reply:slack-alpha", true),
        );
        let facade = RebornOutboundPreferencesFacade::new(
            Arc::new(PutFailingPreferenceRepository),
            inventory,
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
        let inventory = Arc::new(FakeTargetInventory::default());
        inventory.insert(
            "user-alpha",
            target_entry("slack-alpha", "reply:slack-alpha", true),
        );
        let facade = RebornOutboundPreferencesFacade::new(
            Arc::new(LoadFailingPreferenceRepository),
            inventory,
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
    async fn set_preferences_with_none_target_on_new_user_creates_empty_record() {
        let store = Arc::new(InMemoryOutboundStateStore::default());
        let inventory = Arc::new(FakeTargetInventory::default());
        let facade = RebornOutboundPreferencesFacade::new(store.clone(), inventory);

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
        let stored = store
            .load_communication_preference(CommunicationPreferenceKey::new(
                tenant("tenant-alpha"),
                user("user-new"),
            ))
            .await
            .expect("load stored record")
            .expect("stored record");
        assert!(stored.final_reply_target.is_none());
        assert!(stored.progress_target.is_none());
        assert!(stored.approval_prompt_target.is_none());
        assert!(stored.auth_prompt_target.is_none());
        assert!(stored.default_modality.is_none());
    }

    #[tokio::test]
    async fn target_inventory_errors_are_propagated_by_get_set_and_list() {
        let store = Arc::new(InMemoryOutboundStateStore::default());
        seed_record(
            store.as_ref(),
            "tenant-alpha",
            "user-alpha",
            Some(reply_ref("reply:slack-alpha")),
        )
        .await;
        let facade = RebornOutboundPreferencesFacade::new(store, Arc::new(FailingTargetInventory));

        let get_error = facade
            .get_outbound_preferences(caller("tenant-alpha", "user-alpha"))
            .await
            .expect_err("get target inventory failure");
        assert_unavailable_backend_error(get_error);

        let set_error = facade
            .set_outbound_preferences(
                caller("tenant-alpha", "user-alpha"),
                RebornSetOutboundPreferencesRequest {
                    final_reply_target_id: Some(target_id("slack-alpha")),
                },
            )
            .await
            .expect_err("set target inventory failure");
        assert_unavailable_backend_error(set_error);

        let list_error = facade
            .list_outbound_delivery_targets(caller("tenant-alpha", "user-alpha"))
            .await
            .expect_err("list target inventory failure");
        assert_unavailable_backend_error(list_error);
    }

    #[tokio::test]
    async fn clear_preferences_preserves_non_final_slots() {
        let store = Arc::new(InMemoryOutboundStateStore::default());
        let inventory = Arc::new(FakeTargetInventory::default());
        seed_record(
            store.as_ref(),
            "tenant-alpha",
            "user-alpha",
            Some(reply_ref("reply:slack-alpha")),
        )
        .await;
        let facade = RebornOutboundPreferencesFacade::new(store.clone(), inventory);

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
        let stored = store
            .load_communication_preference(CommunicationPreferenceKey::new(
                tenant("tenant-alpha"),
                user("user-alpha"),
            ))
            .await
            .expect("load stored record")
            .expect("stored record");
        assert!(stored.final_reply_target.is_none());
        assert_eq!(
            stored
                .progress_target
                .as_ref()
                .map(|target| target.as_str()),
            Some("reply:progress")
        );
        assert_eq!(
            stored
                .approval_prompt_target
                .as_ref()
                .map(|target| target.as_str()),
            Some("reply:approval")
        );
        assert_eq!(
            stored
                .auth_prompt_target
                .as_ref()
                .map(|target| target.as_str()),
            Some("reply:auth")
        );
        assert_eq!(stored.default_modality, Some(CommunicationModality::Voice));
    }

    #[tokio::test]
    async fn list_targets_is_scoped_to_caller_and_final_reply_capability() {
        let store = Arc::new(InMemoryOutboundStateStore::default());
        let inventory = Arc::new(FakeTargetInventory::default());
        inventory.insert(
            "user-alpha",
            target_entry("slack-alpha", "reply:slack-alpha", true),
        );
        inventory.insert(
            "user-alpha",
            target_entry("slack-progress", "reply:slack-progress", false),
        );
        inventory.insert(
            "user-bravo",
            target_entry("slack-bravo", "reply:slack-bravo", true),
        );
        let facade = RebornOutboundPreferencesFacade::new(store, inventory);

        let response = facade
            .list_outbound_delivery_targets(caller("tenant-alpha", "user-alpha"))
            .await
            .expect("target list");

        assert_eq!(response.targets.len(), 1);
        assert_eq!(response.targets[0].target.target_id.as_str(), "slack-alpha");
        assert!(response.next_cursor.is_none());
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
        OutboundDeliveryTargetEntry {
            summary: RebornOutboundDeliveryTargetSummary::new(
                target_id(target_id_value),
                "slack",
                "Slack DM",
                Some("Slack direct message".to_string()),
            )
            .expect("valid target summary"),
            capabilities: RebornOutboundDeliveryTargetCapabilities {
                final_replies,
                gate_prompts: true,
                auth_prompts: true,
            },
            reply_target_binding_ref: reply_ref(reply_target),
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
                tenant_id: tenant(tenant_id),
                user_id: user(user_id),
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
}
