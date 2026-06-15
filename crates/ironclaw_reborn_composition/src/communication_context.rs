use std::{sync::Arc, time::Duration};

use ironclaw_product_workflow::{
    LifecyclePhase, LifecycleProductAction, LifecycleProductContext, LifecycleProductFacade,
    LifecycleProductPayload, LifecycleProductSurfaceContext, OutboundPreferencesProductFacade,
    RebornOutboundDeliveryTargetStatus, WebUiAuthenticatedCaller,
};
use ironclaw_turns::{
    run_profile::{
        CommunicationContextFetch, CommunicationContextProvider, CommunicationRuntimeContext,
        ConnectedChannelSummary, ConnectedChannelsState, DeliveryTargetState,
        DeliveryTargetSummary,
    },
    scope::{TurnActor, TurnScope},
};
use tokio::join;
use tokio::time::timeout;

/// Shared timeout budget for the whole communication-context fetch (outbound
/// preferences + lifecycle/channels). Both futures run concurrently under this
/// single budget; expiry degrades both delivery-target and connected-channels
/// to `Unknown`.
const COMMUNICATION_CONTEXT_FETCH_TIMEOUT: Duration = Duration::from_millis(500);

pub(crate) struct RuntimeCommunicationContextProvider {
    outbound_preferences: Arc<dyn OutboundPreferencesProductFacade>,
    /// Optional lifecycle facade used to populate connected channels.
    /// When None the slice always renders `Connected channels: unknown.`
    lifecycle_facade: Option<Arc<dyn LifecycleProductFacade>>,
}

impl RuntimeCommunicationContextProvider {
    pub(crate) fn new(outbound_preferences: Arc<dyn OutboundPreferencesProductFacade>) -> Self {
        Self {
            outbound_preferences,
            lifecycle_facade: None,
        }
    }

    pub(crate) fn with_lifecycle_facade(
        mut self,
        lifecycle_facade: Arc<dyn LifecycleProductFacade>,
    ) -> Self {
        self.lifecycle_facade = Some(lifecycle_facade);
        self
    }
}

impl CommunicationContextProvider for RuntimeCommunicationContextProvider {
    fn begin_communication_context(
        &self,
        scope: TurnScope,
        actor: Option<TurnActor>,
    ) -> CommunicationContextFetch {
        // Clone the facade handles into the spawned task so the backend lookups
        // run concurrently with loop-start work; the caller joins the result
        // later via `resolve`. Dropping the returned fetch before resolve aborts
        // the task via `CommunicationContextFetch`'s `Drop` impl, preventing
        // wasted backend work on the run-start hot path.
        let outbound_preferences = Arc::clone(&self.outbound_preferences);
        let lifecycle_facade = self.lifecycle_facade.clone();
        let actor_present = actor.is_some();
        let handle = tokio::spawn(async move {
            fetch_communication_context(outbound_preferences, lifecycle_facade, scope, actor).await
        });
        // Pass `actor_present` so that `resolve` can degrade a `JoinError`
        // (task panic) to `Some(Unknown)` rather than `None` when an actor is
        // present — preserving the actor-present / no-actor distinction.
        CommunicationContextFetch::from_handle(handle, actor_present)
    }
}

/// Resolve the advisory communication slice from backend facades under a single
/// shared timeout budget. The returned context's `delivery_tools_visible` is a
/// placeholder (`false`); the real, surface-derived value is stamped by
/// `CommunicationContextFetch::resolve`.
async fn fetch_communication_context(
    outbound_preferences: Arc<dyn OutboundPreferencesProductFacade>,
    lifecycle_facade: Option<Arc<dyn LifecycleProductFacade>>,
    scope: TurnScope,
    actor: Option<TurnActor>,
) -> Option<CommunicationRuntimeContext> {
    let actor = actor?;
    // Key outbound preferences by the run's *owner*, not the acting principal.
    // Product inbound and trusted-trigger runs can carry an explicit thread
    // owner (subject/creator) that differs from `actor`; the owner is who the
    // stored delivery preference belongs to. Fall back to the actor when no
    // explicit owner is set, matching `TurnScope::to_resource_scope`'s owner
    // resolution. Without this, shared/channel inbound and trigger runs would
    // render the actor's delivery target (or "none set") instead of the
    // owner's, producing wrong delivery guidance.
    let owner_user_id = scope
        .explicit_owner_user_id()
        .cloned()
        .unwrap_or_else(|| actor.user_id.clone());
    let caller = WebUiAuthenticatedCaller::new(
        scope.tenant_id.clone(),
        owner_user_id,
        scope.agent_id.clone(),
        scope.project_id.clone(),
    );

    let preferences_fut = outbound_preferences.get_outbound_preferences(caller.clone());

    // Lifecycle fetch is only meaningful when classification is available.
    // Skip the ExtensionList call entirely when the predicate is a stub so
    // the 500 ms timeout budget is not consumed by a discarded result.
    let lifecycle_fut = async {
        if CHANNEL_CLASSIFICATION_AVAILABLE {
            let lifecycle_context = lifecycle_facade.as_deref().map(|_| {
                LifecycleProductContext::Surface(LifecycleProductSurfaceContext {
                    tenant_id: caller.tenant_id.clone(),
                    user_id: caller.user_id.clone(),
                    agent_id: caller.agent_id.clone(),
                    project_id: caller.project_id.clone(),
                })
            });
            match (&lifecycle_facade, lifecycle_context) {
                (Some(facade), Some(ctx)) => Some(
                    facade
                        .execute(ctx, LifecycleProductAction::ExtensionList)
                        .await,
                ),
                _ => None,
            }
        } else {
            None
        }
    };

    // Both futures share a single 500 ms budget.  Lifecycle runs concurrently
    // only when CHANNEL_CLASSIFICATION_AVAILABLE is true.
    let combined_result = timeout(COMMUNICATION_CONTEXT_FETCH_TIMEOUT, async {
        join!(preferences_fut, lifecycle_fut)
    })
    .await;

    let (pref_result, lifecycle_result) = match combined_result {
        Ok(pair) => pair,
        Err(_) => {
            tracing::debug!("communication context budget expired; degrading to unknown");
            // Budget expired — both are unknown.
            return Some(CommunicationRuntimeContext {
                connected_channels: ConnectedChannelsState::Unknown,
                delivery_target: DeliveryTargetState::Unknown,
                delivery_tools_visible: false,
            });
        }
    };

    let delivery_target = match pref_result {
        Ok(response) => match (
            response.final_reply_target,
            response.final_reply_target_status,
        ) {
            (Some(target), _) => DeliveryTargetState::Set(DeliveryTargetSummary {
                display_name: target.display_name.as_str().to_string(),
                channel: target.channel.as_str().to_string(),
            }),
            // A target is stored but the resolving registry in this
            // composition cannot produce its summary (e.g. no delivery
            // target providers wired). Never report "none set" here — a
            // preference exists and triggered delivery will use it.
            (None, RebornOutboundDeliveryTargetStatus::Unavailable) => {
                DeliveryTargetState::SetUnresolved
            }
            (None, _) => DeliveryTargetState::NoneSet,
        },
        Err(error) => {
            tracing::debug!(
                error = %error,
                "outbound preferences fetch failed; degrading delivery target to unknown"
            );
            DeliveryTargetState::Unknown
        }
    };

    let connected_channels = match lifecycle_result {
        Some(Ok(response)) => {
            if !CHANNEL_CLASSIFICATION_AVAILABLE {
                // Channel-surface classification is a stub until #4778's
                // ProductAdapter surface projection lands. Returning Known([])
                // would be false certainty ("none connected") when the predicate
                // cannot yet distinguish channel extensions from tool extensions.
                ConnectedChannelsState::Unknown
            } else {
                let extensions = match response.payload {
                    Some(LifecycleProductPayload::ExtensionList { extensions, .. }) => extensions,
                    _ => Vec::new(),
                };
                let channels: Vec<ConnectedChannelSummary> = extensions
                    .into_iter()
                    .filter(|ext| {
                        extension_is_channel_surface(ext) && ext.phase == LifecyclePhase::Active
                    })
                    .map(|ext| ConnectedChannelSummary {
                        name: ext.summary.name.clone(),
                        authenticated: true,
                        active: true,
                    })
                    .collect();
                ConnectedChannelsState::Known(channels)
            }
        }
        Some(Err(error)) => {
            tracing::debug!(
                error = %error,
                "lifecycle extension list fetch failed; degrading connected channels to unknown"
            );
            ConnectedChannelsState::Unknown
        }
        // None means lifecycle facade was skipped or not wired — not an error.
        None => ConnectedChannelsState::Unknown,
    };

    Some(CommunicationRuntimeContext {
        connected_channels,
        delivery_target,
        delivery_tools_visible: false,
    })
}

/// Whether channel-surface classification is available.
///
/// Flips to `true` when #4778's `ProductAdapter` surface projection merges and
/// `extension_is_channel_surface` becomes a real predicate rather than a stub.
const CHANNEL_CLASSIFICATION_AVAILABLE: bool = false;

/// Whether a lifecycle extension exposes a channel surface (e.g. Slack).
///
/// Pre-#4778 the lifecycle summary has no surface-kind field, so no extension
/// qualifies; once #4778's `ProductAdapter` surface projection merges, this
/// becomes a check on the projected surface kinds.
fn extension_is_channel_surface(
    _extension: &ironclaw_product_workflow::LifecycleInstalledExtensionSummary,
) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};
    use ironclaw_product_workflow::{
        LifecycleExtensionRuntimeKind, LifecycleExtensionSource, LifecycleExtensionSummary,
        LifecycleInstalledExtensionSummary, LifecyclePackageKind, LifecyclePackageRef,
        LifecyclePhase, LifecycleProductAction, LifecycleProductContext, LifecycleProductFacade,
        LifecycleProductPayload, LifecycleProductResponse, OutboundPreferencesProductFacade,
        ProductWorkflowError, RebornOutboundDeliveryTargetId,
        RebornOutboundDeliveryTargetListResponse, RebornOutboundDeliveryTargetStatus,
        RebornOutboundDeliveryTargetSummary, RebornOutboundPreferencesResponse,
        RebornServicesError, RebornServicesErrorCode, RebornServicesErrorKind,
        RebornSetOutboundPreferencesRequest, WebUiAuthenticatedCaller,
    };
    use ironclaw_turns::{
        run_profile::{CommunicationContextProvider, ConnectedChannelsState, DeliveryTargetState},
        scope::{TurnActor, TurnScope},
    };

    use super::RuntimeCommunicationContextProvider;

    fn scope() -> TurnScope {
        TurnScope {
            tenant_id: TenantId::new("tenant-test").unwrap(),
            agent_id: Some(AgentId::new("agent-test").unwrap()),
            project_id: Some(ProjectId::new("project-test").unwrap()),
            thread_id: ironclaw_host_api::ThreadId::new("thread-test").unwrap(),
            thread_owner: Default::default(),
        }
    }

    fn actor() -> TurnActor {
        TurnActor::new(UserId::new("user-test").unwrap())
    }

    // --- OutboundPreferencesProductFacade fakes ---

    fn test_service_error() -> RebornServicesError {
        RebornServicesError {
            code: RebornServicesErrorCode::Unavailable,
            kind: RebornServicesErrorKind::ServiceUnavailable,
            status_code: 503,
            retryable: false,
            field: None,
            validation_code: None,
        }
    }

    macro_rules! fake_preferences_facade {
        ($name:ident, $get:expr) => {
            struct $name;

            #[async_trait]
            impl OutboundPreferencesProductFacade for $name {
                async fn get_outbound_preferences(
                    &self,
                    _caller: WebUiAuthenticatedCaller,
                ) -> Result<RebornOutboundPreferencesResponse, RebornServicesError> {
                    $get
                }

                async fn set_outbound_preferences(
                    &self,
                    _caller: WebUiAuthenticatedCaller,
                    _request: RebornSetOutboundPreferencesRequest,
                ) -> Result<RebornOutboundPreferencesResponse, RebornServicesError> {
                    $get
                }

                async fn list_outbound_delivery_targets(
                    &self,
                    _caller: WebUiAuthenticatedCaller,
                ) -> Result<RebornOutboundDeliveryTargetListResponse, RebornServicesError> {
                    Ok(RebornOutboundDeliveryTargetListResponse {
                        targets: Vec::new(),
                        next_cursor: None,
                    })
                }
            }
        };
    }

    fake_preferences_facade!(
        NoneSetPreferencesFacade,
        Ok(RebornOutboundPreferencesResponse::default())
    );

    fake_preferences_facade!(
        UnavailablePreferencesFacade,
        Ok(RebornOutboundPreferencesResponse {
            final_reply_target: None,
            final_reply_target_status: RebornOutboundDeliveryTargetStatus::Unavailable,
            ..Default::default()
        })
    );

    fake_preferences_facade!(
        TargetSetPreferencesFacade,
        Ok(RebornOutboundPreferencesResponse {
            final_reply_target: Some(
                RebornOutboundDeliveryTargetSummary::new(
                    RebornOutboundDeliveryTargetId::new("target-1").unwrap(),
                    "slack",
                    "#alerts",
                    None,
                )
                .unwrap(),
            ),
            final_reply_target_status: RebornOutboundDeliveryTargetStatus::Available,
            ..Default::default()
        })
    );

    fake_preferences_facade!(ErrorPreferencesFacade, Err(test_service_error()));

    // --- LifecycleProductFacade fakes ---

    struct EmptyLifecycleFacade;

    #[async_trait]
    impl LifecycleProductFacade for EmptyLifecycleFacade {
        async fn execute(
            &self,
            _context: LifecycleProductContext,
            _action: LifecycleProductAction,
        ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
            Ok(LifecycleProductResponse {
                phase: LifecyclePhase::Active,
                package_ref: None,
                blockers: Vec::new(),
                message: None,
                payload: Some(LifecycleProductPayload::ExtensionList {
                    extensions: Vec::new(),
                    count: 0,
                }),
            })
        }

        async fn project_package(
            &self,
            _context: LifecycleProductContext,
            _package_ref: LifecyclePackageRef,
        ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
            Err(ProductWorkflowError::BindingResolutionFailed {
                reason: "not supported".to_string(),
            })
        }
    }

    struct ChannelListLifecycleFacade {
        extensions: Vec<LifecycleInstalledExtensionSummary>,
    }

    #[async_trait]
    impl LifecycleProductFacade for ChannelListLifecycleFacade {
        async fn execute(
            &self,
            _context: LifecycleProductContext,
            _action: LifecycleProductAction,
        ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
            let count = self.extensions.len();
            Ok(LifecycleProductResponse {
                phase: LifecyclePhase::Active,
                package_ref: None,
                blockers: Vec::new(),
                message: None,
                payload: Some(LifecycleProductPayload::ExtensionList {
                    extensions: self.extensions.clone(),
                    count,
                }),
            })
        }

        async fn project_package(
            &self,
            _context: LifecycleProductContext,
            _package_ref: LifecyclePackageRef,
        ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
            Err(ProductWorkflowError::BindingResolutionFailed {
                reason: "not supported".to_string(),
            })
        }
    }

    struct ErrorLifecycleFacade;

    #[async_trait]
    impl LifecycleProductFacade for ErrorLifecycleFacade {
        async fn execute(
            &self,
            _context: LifecycleProductContext,
            _action: LifecycleProductAction,
        ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
            Err(ProductWorkflowError::BindingResolutionFailed {
                reason: "test error".to_string(),
            })
        }

        async fn project_package(
            &self,
            _context: LifecycleProductContext,
            _package_ref: LifecyclePackageRef,
        ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
            Err(ProductWorkflowError::BindingResolutionFailed {
                reason: "not supported".to_string(),
            })
        }
    }

    fn channel_extension(name: &str) -> LifecycleInstalledExtensionSummary {
        LifecycleInstalledExtensionSummary {
            summary: LifecycleExtensionSummary {
                package_ref: LifecyclePackageRef::new(LifecyclePackageKind::Extension, name)
                    .unwrap(),
                name: name.to_string(),
                version: "1.0.0".to_string(),
                description: "channel extension".to_string(),
                source: LifecycleExtensionSource::HostBundled,
                runtime_kind: LifecycleExtensionRuntimeKind::FirstParty,
                visible_capability_ids: Vec::new(),
                visible_read_only_capability_ids: Vec::new(),
                credential_requirements: Vec::new(),
                onboarding: None,
            },
            phase: LifecyclePhase::Active,
        }
    }

    fn non_channel_extension(name: &str) -> LifecycleInstalledExtensionSummary {
        LifecycleInstalledExtensionSummary {
            summary: LifecycleExtensionSummary {
                package_ref: LifecyclePackageRef::new(LifecyclePackageKind::Extension, name)
                    .unwrap(),
                name: name.to_string(),
                version: "1.0.0".to_string(),
                description: "tool extension".to_string(),
                source: LifecycleExtensionSource::HostBundled,
                runtime_kind: LifecycleExtensionRuntimeKind::WasmTool,
                visible_capability_ids: Vec::new(),
                visible_read_only_capability_ids: Vec::new(),
                credential_requirements: Vec::new(),
                onboarding: None,
            },
            phase: LifecyclePhase::Active,
        }
    }

    fn inactive_channel_extension(name: &str) -> LifecycleInstalledExtensionSummary {
        let mut ext = channel_extension(name);
        ext.phase = LifecyclePhase::Installed;
        ext
    }

    // --- Tests: actor None ---

    #[tokio::test]
    async fn actor_none_returns_none() {
        let provider = RuntimeCommunicationContextProvider::new(Arc::new(NoneSetPreferencesFacade));
        let result = provider
            .begin_communication_context(scope(), None)
            .resolve(false)
            .await;
        assert!(result.is_none(), "actor None must return None");
    }

    // --- Tests: preference lookup is keyed by the run owner, not the actor ---

    /// Preferences facade that records the `user_id` of the caller it received,
    /// so tests can assert the provider keys the lookup by the run owner rather
    /// than the acting principal.
    struct CaptureCallerPreferencesFacade {
        seen_user_id: Arc<std::sync::Mutex<Option<String>>>,
    }

    #[async_trait]
    impl OutboundPreferencesProductFacade for CaptureCallerPreferencesFacade {
        async fn get_outbound_preferences(
            &self,
            caller: WebUiAuthenticatedCaller,
        ) -> Result<RebornOutboundPreferencesResponse, RebornServicesError> {
            *self.seen_user_id.lock().expect("lock") = Some(caller.user_id.as_str().to_string());
            Ok(RebornOutboundPreferencesResponse::default())
        }

        async fn set_outbound_preferences(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _request: RebornSetOutboundPreferencesRequest,
        ) -> Result<RebornOutboundPreferencesResponse, RebornServicesError> {
            Ok(RebornOutboundPreferencesResponse::default())
        }

        async fn list_outbound_delivery_targets(
            &self,
            _caller: WebUiAuthenticatedCaller,
        ) -> Result<RebornOutboundDeliveryTargetListResponse, RebornServicesError> {
            Ok(RebornOutboundDeliveryTargetListResponse {
                targets: Vec::new(),
                next_cursor: None,
            })
        }
    }

    #[tokio::test]
    async fn preferences_keyed_by_explicit_owner_not_actor() {
        let seen_user_id = Arc::new(std::sync::Mutex::new(None));
        let facade = CaptureCallerPreferencesFacade {
            seen_user_id: Arc::clone(&seen_user_id),
        };
        let provider = RuntimeCommunicationContextProvider::new(Arc::new(facade));

        // Scope owned by a subject/creator distinct from the acting principal
        // (e.g. a trusted trigger or shared/channel inbound run).
        let owned_scope = TurnScope::new_with_owner(
            TenantId::new("tenant-test").unwrap(),
            Some(AgentId::new("agent-test").unwrap()),
            Some(ProjectId::new("project-test").unwrap()),
            ironclaw_host_api::ThreadId::new("thread-test").unwrap(),
            Some(UserId::new("owner-test").unwrap()),
        );

        provider
            .begin_communication_context(owned_scope, Some(actor()))
            .resolve(false)
            .await
            .expect("context");

        assert_eq!(
            seen_user_id.lock().expect("lock").as_deref(),
            Some("owner-test"),
            "preference lookup must be keyed by the explicit run owner, not the actor",
        );
    }

    #[tokio::test]
    async fn preferences_fall_back_to_actor_without_explicit_owner() {
        let seen_user_id = Arc::new(std::sync::Mutex::new(None));
        let facade = CaptureCallerPreferencesFacade {
            seen_user_id: Arc::clone(&seen_user_id),
        };
        let provider = RuntimeCommunicationContextProvider::new(Arc::new(facade));

        // `scope()` uses `TurnThreadOwner::ActorFallback` (no explicit owner).
        provider
            .begin_communication_context(scope(), Some(actor()))
            .resolve(false)
            .await
            .expect("context");

        assert_eq!(
            seen_user_id.lock().expect("lock").as_deref(),
            Some("user-test"),
            "with no explicit owner the lookup must fall back to the actor",
        );
    }

    // --- Tests: delivery target state branches ---

    #[tokio::test]
    async fn none_configured_maps_to_none_set() {
        let provider = RuntimeCommunicationContextProvider::new(Arc::new(NoneSetPreferencesFacade));
        let ctx = provider
            .begin_communication_context(scope(), Some(actor()))
            .resolve(false)
            .await
            .expect("context");
        assert_eq!(ctx.delivery_target, DeliveryTargetState::NoneSet);
    }

    #[tokio::test]
    async fn unavailable_status_maps_to_set_unresolved() {
        let provider =
            RuntimeCommunicationContextProvider::new(Arc::new(UnavailablePreferencesFacade));
        let ctx = provider
            .begin_communication_context(scope(), Some(actor()))
            .resolve(false)
            .await
            .expect("context");
        assert_eq!(ctx.delivery_target, DeliveryTargetState::SetUnresolved);
    }

    #[tokio::test]
    async fn target_set_maps_to_set_with_summary() {
        let provider =
            RuntimeCommunicationContextProvider::new(Arc::new(TargetSetPreferencesFacade));
        let ctx = provider
            .begin_communication_context(scope(), Some(actor()))
            .resolve(false)
            .await
            .expect("context");
        assert!(
            matches!(ctx.delivery_target, DeliveryTargetState::Set(_)),
            "resolved target must map to Set: {:?}",
            ctx.delivery_target
        );
    }

    #[tokio::test]
    async fn preferences_error_maps_to_unknown() {
        let provider = RuntimeCommunicationContextProvider::new(Arc::new(ErrorPreferencesFacade));
        let ctx = provider
            .begin_communication_context(scope(), Some(actor()))
            .resolve(false)
            .await
            .expect("context");
        assert_eq!(ctx.delivery_target, DeliveryTargetState::Unknown);
    }

    // --- Tests: connected channels ---

    #[tokio::test]
    async fn no_lifecycle_facade_returns_unknown_channels() {
        let provider = RuntimeCommunicationContextProvider::new(Arc::new(NoneSetPreferencesFacade));
        let ctx = provider
            .begin_communication_context(scope(), Some(actor()))
            .resolve(false)
            .await
            .expect("context");
        assert_eq!(ctx.connected_channels, ConnectedChannelsState::Unknown);
    }

    #[tokio::test]
    async fn classification_unavailable_returns_unknown_for_empty_extension_list() {
        // While CHANNEL_CLASSIFICATION_AVAILABLE is false the lifecycle fetch is
        // skipped entirely; connected_channels must be Unknown regardless of what
        // the facade would return (never false-certainty Known([])).
        let provider = RuntimeCommunicationContextProvider::new(Arc::new(NoneSetPreferencesFacade))
            .with_lifecycle_facade(Arc::new(EmptyLifecycleFacade));
        let ctx = provider
            .begin_communication_context(scope(), Some(actor()))
            .resolve(false)
            .await
            .expect("context");
        assert_eq!(
            ctx.connected_channels,
            ConnectedChannelsState::Unknown,
            "classification unavailable → lifecycle skipped → Unknown"
        );
    }

    #[tokio::test]
    async fn classification_unavailable_returns_unknown_for_non_channel_extensions() {
        // While CHANNEL_CLASSIFICATION_AVAILABLE is false the lifecycle fetch is
        // skipped entirely, so connected_channels is Unknown regardless of the
        // extension list the facade would have returned.
        // When #4778 merges, flip CHANNEL_CLASSIFICATION_AVAILABLE to true and
        // grow a positive case here.
        let provider = RuntimeCommunicationContextProvider::new(Arc::new(NoneSetPreferencesFacade))
            .with_lifecycle_facade(Arc::new(ChannelListLifecycleFacade {
                extensions: vec![
                    channel_extension("telegram"),
                    non_channel_extension("github"),
                    inactive_channel_extension("slack"),
                ],
            }));
        let ctx = provider
            .begin_communication_context(scope(), Some(actor()))
            .resolve(false)
            .await
            .expect("context");
        assert_eq!(
            ctx.connected_channels,
            ConnectedChannelsState::Unknown,
            "classification unavailable → Unknown, not Known([])"
        );
    }

    #[tokio::test]
    async fn lifecycle_facade_error_returns_unknown_channels() {
        let provider = RuntimeCommunicationContextProvider::new(Arc::new(NoneSetPreferencesFacade))
            .with_lifecycle_facade(Arc::new(ErrorLifecycleFacade));
        let ctx = provider
            .begin_communication_context(scope(), Some(actor()))
            .resolve(false)
            .await
            .expect("context");
        assert_eq!(ctx.connected_channels, ConnectedChannelsState::Unknown);
    }

    // --- Tests: timeout path ---

    /// A preferences facade whose `get_outbound_preferences` never resolves.
    /// Used to exercise the shared-timeout Unknown path.
    ///
    /// Note: `tokio/test-util` is not in this crate's feature set, so
    /// `start_paused` / `tokio::time::advance` are unavailable. The test relies
    /// on the real 500 ms wall-clock timeout firing against a `pending()` future.
    struct HangingPreferencesFacade;

    #[async_trait]
    impl OutboundPreferencesProductFacade for HangingPreferencesFacade {
        async fn get_outbound_preferences(
            &self,
            _caller: WebUiAuthenticatedCaller,
        ) -> Result<RebornOutboundPreferencesResponse, RebornServicesError> {
            std::future::pending().await
        }

        async fn set_outbound_preferences(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _request: RebornSetOutboundPreferencesRequest,
        ) -> Result<RebornOutboundPreferencesResponse, RebornServicesError> {
            Ok(RebornOutboundPreferencesResponse::default())
        }

        async fn list_outbound_delivery_targets(
            &self,
            _caller: WebUiAuthenticatedCaller,
        ) -> Result<RebornOutboundDeliveryTargetListResponse, RebornServicesError> {
            Ok(RebornOutboundDeliveryTargetListResponse {
                targets: Vec::new(),
                next_cursor: None,
            })
        }
    }

    /// A preferences facade whose `get_outbound_preferences` panics immediately.
    /// This causes the spawned `fetch_communication_context` task to abort with a
    /// `JoinError`, exercising the actor-present degrade-to-unknown path in
    /// `begin_communication_context`.
    struct PanickingPreferencesFacade;

    #[async_trait]
    impl OutboundPreferencesProductFacade for PanickingPreferencesFacade {
        async fn get_outbound_preferences(
            &self,
            _caller: WebUiAuthenticatedCaller,
        ) -> Result<RebornOutboundPreferencesResponse, RebornServicesError> {
            panic!("induced panic for JoinError test")
        }

        async fn set_outbound_preferences(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _request: RebornSetOutboundPreferencesRequest,
        ) -> Result<RebornOutboundPreferencesResponse, RebornServicesError> {
            Ok(RebornOutboundPreferencesResponse::default())
        }

        async fn list_outbound_delivery_targets(
            &self,
            _caller: WebUiAuthenticatedCaller,
        ) -> Result<RebornOutboundDeliveryTargetListResponse, RebornServicesError> {
            Ok(RebornOutboundDeliveryTargetListResponse {
                targets: Vec::new(),
                next_cursor: None,
            })
        }
    }

    #[tokio::test]
    async fn actor_present_join_failure_degrades_to_unknown() {
        // When the spawned fetch task panics (JoinError) and an actor IS present,
        // the resolved context must be Some with Unknown states — not None.
        // None would be ambiguous with the "no actor" path and would suppress
        // `delivery_tools_visible` stamping for a run that genuinely has an actor.
        let provider =
            RuntimeCommunicationContextProvider::new(Arc::new(PanickingPreferencesFacade));

        let ctx = provider
            .begin_communication_context(scope(), Some(actor()))
            .resolve(false)
            .await
            .expect("actor-present join failure must return Some, not None");

        assert_eq!(
            ctx.connected_channels,
            ConnectedChannelsState::Unknown,
            "join failure with actor present must degrade connected_channels to Unknown"
        );
        assert_eq!(
            ctx.delivery_target,
            DeliveryTargetState::Unknown,
            "join failure with actor present must degrade delivery_target to Unknown"
        );
    }

    #[tokio::test]
    async fn drop_before_resolve_aborts_spawned_task() {
        // Regression: dropping a `CommunicationContextFetch` before calling
        // `resolve` must abort the underlying spawned task rather than detaching
        // it. A detached task wastes the ~500 ms timeout budget on failed runs
        // in the hot run-start path.
        //
        // Strategy: the task parks forever on a `Notify` it will never receive
        // (simulating a hanging backend) while holding a drop guard. The guard's
        // `Drop` fires ONLY when the task future is dropped — which happens on
        // abort (or genuine completion, which never occurs here). If the fetch
        // detaches instead of aborting, the task stays parked, the guard never
        // drops, and `aborted` stays `false`. So the assertion fails iff
        // abort-on-drop regresses. (The previous version asserted a "completed"
        // flag stayed false, which was true whether aborted OR merely parked —
        // a false positive flagged in review.)
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        use tokio::sync::Notify;

        struct AbortObserver(Arc<AtomicBool>);
        impl Drop for AbortObserver {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }

        let task_started = Arc::new(Notify::new());
        let task_future_dropped = Arc::new(AtomicBool::new(false));

        let task_started_inner = Arc::clone(&task_started);
        let observer = AbortObserver(Arc::clone(&task_future_dropped));

        let handle = tokio::spawn(async move {
            // Held across the await: dropped iff this future is dropped (abort).
            let _observer = observer;
            task_started_inner.notify_one();
            // Park forever — only an abort interrupts this.
            let never = Notify::new();
            never.notified().await;
            None::<ironclaw_turns::run_profile::CommunicationRuntimeContext>
        });

        // Ensure the task is actually running and parked before we drop.
        task_started.notified().await;

        let fetch =
            ironclaw_turns::run_profile::CommunicationContextFetch::from_handle(handle, false);
        drop(fetch);

        // Give tokio's abort machinery time to drop the task future.
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }

        assert!(
            task_future_dropped.load(Ordering::SeqCst),
            "dropping the fetch must abort the spawned task (its future must be dropped); \
             a detached task would stay parked and never drop the observer"
        );
    }

    #[tokio::test]
    async fn shared_timeout_yields_unknown_for_both_delivery_and_channels() {
        // The preferences future never resolves; the 500 ms outer timeout fires.
        // Both delivery_target and connected_channels must be Unknown — never
        // fabricated definitive states. Uses real wall-clock time (500 ms) since
        // tokio/test-util is not in this crate's features.
        let provider = RuntimeCommunicationContextProvider::new(Arc::new(HangingPreferencesFacade));

        let ctx = provider
            .begin_communication_context(scope(), Some(actor()))
            .resolve(false)
            .await
            .expect("communication_context must return Some even on timeout");

        assert_eq!(
            ctx.delivery_target,
            DeliveryTargetState::Unknown,
            "timed-out preferences must map to Unknown delivery_target"
        );
        assert_eq!(
            ctx.connected_channels,
            ConnectedChannelsState::Unknown,
            "timed-out budget must leave connected_channels Unknown"
        );
    }
}
