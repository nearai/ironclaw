use ironclaw_product_contracts::lifecycle_service::{
    LifecycleProductContext, LifecycleProductService, LifecycleProductSurfaceContext,
};
use std::{sync::Arc, time::Duration};

use crate::{
    LifecycleProductAction, LifecycleProductPayload, OutboundPreferencesProductService,
    RebornOutboundDeliveryTargetStatus,
};
use ironclaw_extension_contracts::{state::InstallationState, surface::CapabilitySurfaceKind};
use ironclaw_host_api::turn::{TurnActor, TurnScope};
use ironclaw_loop_contracts::{
    CommunicationContextFetch, CommunicationContextProvider, CommunicationRuntimeContext,
    ConnectedChannelSummary, ConnectedChannelsState, NotificationChannelsState,
};
use ironclaw_product_contracts::surface::ProductSurfaceCaller;
use tokio::join;
use tokio::time::timeout;

/// Shared timeout budget for the whole communication-context fetch (notification
/// channels + lifecycle/channels). Both futures run concurrently under this
/// single budget; expiry degrades both notification-channels and connected-channels
/// to `Unknown`.
const COMMUNICATION_CONTEXT_FETCH_TIMEOUT: Duration = Duration::from_millis(500);

pub struct RuntimeCommunicationContextProvider {
    outbound_preferences: Arc<dyn OutboundPreferencesProductService>,
    /// Optional lifecycle service used to populate connected channels.
    /// When None the slice always renders `Connected channels: unknown.`
    lifecycle_service: Option<Arc<dyn LifecycleProductService>>,
}

impl RuntimeCommunicationContextProvider {
    pub fn new(outbound_preferences: Arc<dyn OutboundPreferencesProductService>) -> Self {
        Self {
            outbound_preferences,
            lifecycle_service: None,
        }
    }

    pub fn with_lifecycle_service(
        mut self,
        lifecycle_service: Arc<dyn LifecycleProductService>,
    ) -> Self {
        self.lifecycle_service = Some(lifecycle_service);
        self
    }
}

impl CommunicationContextProvider for RuntimeCommunicationContextProvider {
    fn begin_communication_context(
        &self,
        scope: TurnScope,
        actor: Option<TurnActor>,
    ) -> CommunicationContextFetch {
        // Clone the service handles into the spawned task so the backend lookups
        // run concurrently with loop-start work; the caller joins the result
        // later via `resolve`. Dropping the returned fetch before resolve aborts
        // the task via `CommunicationContextFetch`'s `Drop` impl, preventing
        // wasted backend work on the run-start hot path.
        let outbound_preferences = Arc::clone(&self.outbound_preferences);
        let lifecycle_service = self.lifecycle_service.clone();
        let actor_present = actor.is_some();
        let handle = tokio::spawn(async move {
            fetch_communication_context(outbound_preferences, lifecycle_service, scope, actor).await
        });
        // Pass `actor_present` so that `resolve` can degrade a `JoinError`
        // (task panic) to `Some(Unknown)` rather than `None` when an actor is
        // present — preserving the actor-present / no-actor distinction.
        CommunicationContextFetch::from_handle(handle, actor_present)
    }
}

/// Resolve the advisory communication slice from backend services under a single
/// shared timeout budget. The returned context's `delivery_tools_visible` is a
/// placeholder (`false`); the real, surface-derived value is stamped by
/// `CommunicationContextFetch::resolve`.
async fn fetch_communication_context(
    outbound_preferences: Arc<dyn OutboundPreferencesProductService>,
    lifecycle_service: Option<Arc<dyn LifecycleProductService>>,
    scope: TurnScope,
    actor: Option<TurnActor>,
) -> Option<CommunicationRuntimeContext> {
    let actor = actor?;
    // A run's notification-channel state is its user's (the actor).
    let acting_user_id = actor.user_id.clone();
    let caller = ProductSurfaceCaller::new(
        scope.tenant_id.clone(),
        acting_user_id,
        scope.agent_id.clone(),
        scope.project_id.clone(),
    );

    // There is no stored "default reply target" anymore (route_current/
    // web_app-pseudo-target/target_set were retired; see `delivery.md`) — the
    // notification-channel set is the only per-user outbound preference this
    // slice still surfaces, driving the "Background-run notifications: ..."
    // one-liner.
    let notifications_fut = outbound_preferences.get_notification_channels(caller.clone());

    // Fetch the installed-extension list to classify channel surfaces. Skipped
    // only when no lifecycle service is wired (the slice then renders channels as
    // `unknown`). Runs concurrently with the notifications fetch under the shared
    // budget below.
    let lifecycle_fut = async {
        match lifecycle_service.as_deref() {
            Some(service) => {
                let ctx = LifecycleProductContext::Surface(LifecycleProductSurfaceContext {
                    tenant_id: caller.tenant_id.clone(),
                    user_id: caller.user_id.clone(),
                    agent_id: caller.agent_id.clone(),
                    project_id: caller.project_id.clone(),
                });
                Some(
                    service
                        .execute(ctx, LifecycleProductAction::ExtensionList)
                        .await,
                )
            }
            None => None,
        }
    };

    // Both futures share a single 500 ms budget and run concurrently.
    let combined_result = timeout(COMMUNICATION_CONTEXT_FETCH_TIMEOUT, async {
        join!(notifications_fut, lifecycle_fut)
    })
    .await;

    let (notifications_result, lifecycle_result) = match combined_result {
        Ok(pair) => pair,
        Err(_) => {
            tracing::debug!("communication context budget expired; degrading to unknown");
            // Budget expired — both are unknown.
            return Some(CommunicationRuntimeContext {
                connected_channels: ConnectedChannelsState::Unknown,
                notification_channels: NotificationChannelsState::Unknown,
                delivery_tools_visible: false,
            });
        }
    };

    let notification_channels = match notifications_result {
        Ok(response) => NotificationChannelsState::Known(
            response
                .channels
                .iter()
                .filter(|channel| channel.status == RebornOutboundDeliveryTargetStatus::Available)
                .count(),
        ),
        Err(error) => {
            tracing::debug!(
                error = %error,
                "notification channels fetch failed; degrading notification channels to unknown"
            );
            NotificationChannelsState::Unknown
        }
    };

    let connected_channels = match lifecycle_result {
        // A present response means a lifecycle service was wired and returned the
        // installed-extension list; classify each by its projected surface kind.
        Some(Ok(response)) => {
            let extensions = match response.payload {
                Some(LifecycleProductPayload::ExtensionList { extensions, .. }) => extensions,
                _ => Vec::new(),
            };
            let channels: Vec<ConnectedChannelSummary> = extensions
                .into_iter()
                .filter(|ext| {
                    extension_is_channel_surface(ext) && ext.phase == InstallationState::Active
                })
                .map(|ext| ConnectedChannelSummary {
                    name: ext.summary.name.clone(),
                    authenticated: true,
                    active: true,
                    presentation: ext.summary.channel_presentation.clone(),
                })
                .collect();
            ConnectedChannelsState::Known(channels)
        }
        Some(Err(error)) => {
            tracing::debug!(
                error = %error,
                "lifecycle extension list fetch failed; degrading connected channels to unknown"
            );
            ConnectedChannelsState::Unknown
        }
        // None means lifecycle service was skipped or not wired — not an error.
        None => ConnectedChannelsState::Unknown,
    };

    Some(CommunicationRuntimeContext {
        connected_channels,
        notification_channels,
        delivery_tools_visible: false,
    })
}

/// Whether a lifecycle extension exposes a channel surface (e.g. Slack).
///
/// Checks the projected `surface_kinds` for `ExternalChannel`, the surface kind
/// that maps to a connected chat channel.
fn extension_is_channel_surface(extension: &crate::LifecycleInstalledExtensionSummary) -> bool {
    extension
        .summary
        .surface_kinds
        .contains(&CapabilitySurfaceKind::Channel)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::{
        LifecycleExtensionRuntimeKind, LifecycleExtensionSource, LifecycleExtensionSummary,
        LifecycleInstalledExtensionSummary, LifecyclePackageKind, LifecyclePackageRef,
        LifecycleProductAction, LifecycleProductPayload, LifecycleProductResponse,
        OutboundPreferencesProductService, RebornNotificationChannel,
        RebornNotificationChannelsResponse, RebornOutboundDeliveryTargetId,
        RebornOutboundDeliveryTargetListResponse, RebornOutboundDeliveryTargetStatus,
    };
    use async_trait::async_trait;
    use ironclaw_extension_contracts::{state::InstallationState, surface::CapabilitySurfaceKind};
    use ironclaw_host_api::ids::{AgentId, ProjectId, TenantId, UserId};
    use ironclaw_host_api::turn::{TurnActor, TurnScope};
    use ironclaw_loop_contracts::{
        CommunicationContextProvider, ConnectedChannelsState, NotificationChannelsState,
    };
    use ironclaw_product_contracts::lifecycle_service::{
        LifecycleProductContext, LifecycleProductService,
    };
    use ironclaw_product_contracts::surface::{
        ProductSurfaceCaller, ProductSurfaceError, ProductSurfaceErrorCode, ProductSurfaceErrorKind,
    };

    use super::RuntimeCommunicationContextProvider;

    fn scope() -> TurnScope {
        TurnScope {
            tenant_id: TenantId::new("tenant-test").unwrap(),
            agent_id: Some(AgentId::new("agent-test").unwrap()),
            project_id: Some(ProjectId::new("project-test").unwrap()),
            thread_id: ironclaw_host_api::ids::ThreadId::new("thread-test").unwrap(),
            thread_owner: Default::default(),
        }
    }

    fn actor() -> TurnActor {
        TurnActor::new(UserId::new("user-test").unwrap())
    }

    // --- OutboundPreferencesProductService fakes ---

    fn test_service_error() -> ProductSurfaceError {
        ProductSurfaceError {
            code: ProductSurfaceErrorCode::Unavailable,
            kind: ProductSurfaceErrorKind::ServiceUnavailable,
            status_code: 503,
            retryable: false,
            field: None,
            validation_code: None,
        }
    }

    // `list_outbound_delivery_targets` is required by the trait but not read by
    // `fetch_communication_context` (there is no stored "default reply target"
    // anymore — see `delivery.md`); every fake below stubs it with a trivial
    // default and drives `get_notification_channels` instead, which is what the
    // fetch now calls.
    macro_rules! fake_notification_channels_service {
        ($name:ident, $get_notifications:expr) => {
            struct $name;

            #[async_trait]
            impl OutboundPreferencesProductService for $name {
                async fn list_outbound_delivery_targets(
                    &self,
                    _caller: ProductSurfaceCaller,
                ) -> Result<RebornOutboundDeliveryTargetListResponse, ProductSurfaceError> {
                    Ok(RebornOutboundDeliveryTargetListResponse {
                        targets: Vec::new(),
                        next_cursor: None,
                    })
                }

                async fn get_notification_channels(
                    &self,
                    _caller: ProductSurfaceCaller,
                ) -> Result<RebornNotificationChannelsResponse, ProductSurfaceError> {
                    $get_notifications
                }
            }
        };
    }

    fake_notification_channels_service!(
        EmptyNotificationChannelsService,
        Ok(RebornNotificationChannelsResponse::default())
    );

    fake_notification_channels_service!(
        PopulatedNotificationChannelsService,
        Ok(RebornNotificationChannelsResponse {
            channels: vec![
                RebornNotificationChannel {
                    target_id: RebornOutboundDeliveryTargetId::new("target-1").unwrap(),
                    status: RebornOutboundDeliveryTargetStatus::Available,
                    option: None,
                },
                RebornNotificationChannel {
                    target_id: RebornOutboundDeliveryTargetId::new("target-2").unwrap(),
                    status: RebornOutboundDeliveryTargetStatus::Available,
                    option: None,
                },
            ],
        })
    );

    fake_notification_channels_service!(
        MixedNotificationChannelsService,
        Ok(RebornNotificationChannelsResponse {
            channels: vec![
                RebornNotificationChannel {
                    target_id: RebornOutboundDeliveryTargetId::new("target-live").unwrap(),
                    status: RebornOutboundDeliveryTargetStatus::Available,
                    option: None,
                },
                RebornNotificationChannel {
                    target_id: RebornOutboundDeliveryTargetId::new("target-stale").unwrap(),
                    status: RebornOutboundDeliveryTargetStatus::Unavailable,
                    option: None,
                },
            ],
        })
    );

    fake_notification_channels_service!(
        ErrorNotificationChannelsService,
        Err(test_service_error())
    );

    // --- LifecycleProductService fakes ---

    struct EmptyLifecycleService;

    #[async_trait]
    impl LifecycleProductService for EmptyLifecycleService {
        async fn execute(
            &self,
            _context: LifecycleProductContext,
            _action: LifecycleProductAction,
        ) -> Result<LifecycleProductResponse, ProductSurfaceError> {
            Ok(LifecycleProductResponse {
                phase: InstallationState::Active,
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
        ) -> Result<LifecycleProductResponse, ProductSurfaceError> {
            Err(test_service_error())
        }
    }

    struct ChannelListLifecycleService {
        extensions: Vec<LifecycleInstalledExtensionSummary>,
    }

    #[async_trait]
    impl LifecycleProductService for ChannelListLifecycleService {
        async fn execute(
            &self,
            _context: LifecycleProductContext,
            _action: LifecycleProductAction,
        ) -> Result<LifecycleProductResponse, ProductSurfaceError> {
            let count = self.extensions.len();
            Ok(LifecycleProductResponse {
                phase: InstallationState::Active,
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
        ) -> Result<LifecycleProductResponse, ProductSurfaceError> {
            Err(test_service_error())
        }
    }

    struct ErrorLifecycleService;

    #[async_trait]
    impl LifecycleProductService for ErrorLifecycleService {
        async fn execute(
            &self,
            _context: LifecycleProductContext,
            _action: LifecycleProductAction,
        ) -> Result<LifecycleProductResponse, ProductSurfaceError> {
            Err(test_service_error())
        }

        async fn project_package(
            &self,
            _context: LifecycleProductContext,
            _package_ref: LifecyclePackageRef,
        ) -> Result<LifecycleProductResponse, ProductSurfaceError> {
            Err(test_service_error())
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
                surface_kinds: vec![CapabilitySurfaceKind::Channel],
                channel_directions: None,
                channel_connection: None,
                channel_presentation: None,
                visible_capability_ids: Vec::new(),
                visible_read_only_capability_ids: Vec::new(),
                credential_requirements: Vec::new(),
                onboarding: None,
            },
            phase: InstallationState::Active,
            install_scope: None,
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
                surface_kinds: Vec::new(),
                channel_directions: None,
                channel_connection: None,
                channel_presentation: None,
                visible_capability_ids: Vec::new(),
                visible_read_only_capability_ids: Vec::new(),
                credential_requirements: Vec::new(),
                onboarding: None,
            },
            phase: InstallationState::Active,
            install_scope: None,
        }
    }

    fn inactive_channel_extension(name: &str) -> LifecycleInstalledExtensionSummary {
        let mut ext = channel_extension(name);
        ext.phase = InstallationState::Installed;
        ext
    }

    // --- Tests: actor None ---

    #[tokio::test]
    async fn actor_none_returns_none() {
        let provider =
            RuntimeCommunicationContextProvider::new(Arc::new(EmptyNotificationChannelsService));
        let result = provider
            .begin_communication_context(scope(), None)
            .resolve(false)
            .await;
        assert!(result.is_none(), "actor None must return None");
    }

    // --- Tests: notification-channel lookup is keyed by the run owner, not the actor ---

    /// Notification-channels service that records the `user_id` of the caller it
    /// received, so tests can assert the provider keys the lookup by the run
    /// owner rather than the acting principal.
    struct CaptureCallerPreferencesService {
        seen_user_id: Arc<std::sync::Mutex<Option<String>>>,
    }

    #[async_trait]
    impl OutboundPreferencesProductService for CaptureCallerPreferencesService {
        async fn list_outbound_delivery_targets(
            &self,
            _caller: ProductSurfaceCaller,
        ) -> Result<RebornOutboundDeliveryTargetListResponse, ProductSurfaceError> {
            Ok(RebornOutboundDeliveryTargetListResponse {
                targets: Vec::new(),
                next_cursor: None,
            })
        }

        async fn get_notification_channels(
            &self,
            caller: ProductSurfaceCaller,
        ) -> Result<RebornNotificationChannelsResponse, ProductSurfaceError> {
            *self.seen_user_id.lock().expect("lock") = Some(caller.user_id.as_str().to_string());
            Ok(RebornNotificationChannelsResponse::default())
        }
    }

    /// A run's notification-channel state is its own user's (#7377): the
    /// communication-context provider keys the preference lookup by the run's
    /// user (its actor). Owner == actor since the ephemeral-per-ping remodel.
    #[tokio::test]
    async fn preferences_keyed_by_the_run_user() {
        let seen_user_id = Arc::new(std::sync::Mutex::new(None));
        let service = CaptureCallerPreferencesService {
            seen_user_id: Arc::clone(&seen_user_id),
        };
        let provider = RuntimeCommunicationContextProvider::new(Arc::new(service));

        // A normal run: owner == actor ("user-test").
        let owned_scope = TurnScope::new_with_owner(
            TenantId::new("tenant-test").unwrap(),
            Some(AgentId::new("agent-test").unwrap()),
            Some(ProjectId::new("project-test").unwrap()),
            ironclaw_host_api::ids::ThreadId::new("thread-test").unwrap(),
            Some(UserId::new("user-test").unwrap()),
        );

        provider
            .begin_communication_context(owned_scope, Some(actor()))
            .resolve(false)
            .await
            .expect("context");

        assert_eq!(
            seen_user_id.lock().expect("lock").as_deref(),
            Some("user-test"),
            "preference lookup must be keyed by the run's user",
        );
    }

    #[tokio::test]
    async fn preferences_fall_back_to_actor_without_explicit_owner() {
        let seen_user_id = Arc::new(std::sync::Mutex::new(None));
        let service = CaptureCallerPreferencesService {
            seen_user_id: Arc::clone(&seen_user_id),
        };
        let provider = RuntimeCommunicationContextProvider::new(Arc::new(service));

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

    // --- Tests: notification-channel state branches ---

    #[tokio::test]
    async fn no_notification_channels_configured_maps_to_known_zero() {
        let provider =
            RuntimeCommunicationContextProvider::new(Arc::new(EmptyNotificationChannelsService));
        let ctx = provider
            .begin_communication_context(scope(), Some(actor()))
            .resolve(false)
            .await
            .expect("context");
        assert_eq!(
            ctx.notification_channels,
            NotificationChannelsState::Known(0)
        );
    }

    #[tokio::test]
    async fn notification_channels_populated_maps_to_known_count() {
        let provider = RuntimeCommunicationContextProvider::new(Arc::new(
            PopulatedNotificationChannelsService,
        ));
        let ctx = provider
            .begin_communication_context(scope(), Some(actor()))
            .resolve(false)
            .await
            .expect("context");
        assert_eq!(
            ctx.notification_channels,
            NotificationChannelsState::Known(2),
            "notification-channel count must reflect the resolved channel list length"
        );
    }

    #[tokio::test]
    async fn unavailable_notification_channels_do_not_count_as_deliverable() {
        let provider =
            RuntimeCommunicationContextProvider::new(Arc::new(MixedNotificationChannelsService));
        let ctx = provider
            .begin_communication_context(scope(), Some(actor()))
            .resolve(false)
            .await
            .expect("context");
        assert_eq!(
            ctx.notification_channels,
            NotificationChannelsState::Known(1),
            "model guidance must count only channels that can currently receive a notification"
        );
    }

    #[tokio::test]
    async fn notification_channels_error_maps_to_unknown() {
        let provider =
            RuntimeCommunicationContextProvider::new(Arc::new(ErrorNotificationChannelsService));
        let ctx = provider
            .begin_communication_context(scope(), Some(actor()))
            .resolve(false)
            .await
            .expect("context");
        assert_eq!(
            ctx.notification_channels,
            NotificationChannelsState::Unknown
        );
    }

    // --- Tests: connected channels ---

    #[tokio::test]
    async fn no_lifecycle_service_returns_unknown_channels() {
        let provider =
            RuntimeCommunicationContextProvider::new(Arc::new(EmptyNotificationChannelsService));
        let ctx = provider
            .begin_communication_context(scope(), Some(actor()))
            .resolve(false)
            .await
            .expect("context");
        assert_eq!(ctx.connected_channels, ConnectedChannelsState::Unknown);
    }

    #[tokio::test]
    async fn empty_extension_list_returns_known_no_channels() {
        // Classification is available, so an empty extension list is genuine
        // certainty: no channels connected → Known([]), not Unknown.
        let provider =
            RuntimeCommunicationContextProvider::new(Arc::new(EmptyNotificationChannelsService))
                .with_lifecycle_service(Arc::new(EmptyLifecycleService));
        let ctx = provider
            .begin_communication_context(scope(), Some(actor()))
            .resolve(false)
            .await
            .expect("context");
        assert_eq!(
            ctx.connected_channels,
            ConnectedChannelsState::Known(Vec::new()),
            "classification available + empty list → Known([])"
        );
    }

    #[tokio::test]
    async fn channel_extensions_are_classified_as_connected_channels() {
        // Only active channel-surface extensions count: telegram (active channel)
        // is included; github (non-channel) and slack (inactive channel) are not.
        // The telegram summary also carries a declared presentation (OUT-11).
        let mut telegram = channel_extension("telegram");
        telegram.summary.channel_presentation =
            Some(ironclaw_extension_contracts::channel::ChannelPresentation {
                supports_markdown: true,
                supports_threads: false,
                can_reply_in_threads: false,
                max_message_chars: Some(4096),
                command_prefix: None,
            });
        let provider =
            RuntimeCommunicationContextProvider::new(Arc::new(EmptyNotificationChannelsService))
                .with_lifecycle_service(Arc::new(ChannelListLifecycleService {
                    extensions: vec![
                        telegram,
                        non_channel_extension("github"),
                        inactive_channel_extension("slack"),
                    ],
                }));
        let ctx = provider
            .begin_communication_context(scope(), Some(actor()))
            .resolve(false)
            .await
            .expect("context");
        let channels = match ctx.connected_channels {
            ConnectedChannelsState::Known(channels) => channels,
            other => panic!("expected Known channels, got {other:?}"),
        };
        let names: Vec<String> = channels.iter().map(|c| c.name.clone()).collect();
        assert_eq!(
            names,
            vec!["telegram".to_string()],
            "only active channel-surface extensions are reported as connected"
        );
        // OUT-11: the channel's declared presentation flows through the provider
        // onto the connected-channel summary that prompt construction renders.
        assert_eq!(
            channels[0].presentation,
            Some(ironclaw_extension_contracts::channel::ChannelPresentation {
                supports_markdown: true,
                supports_threads: false,
                can_reply_in_threads: false,
                max_message_chars: Some(4096),
                command_prefix: None,
            }),
            "the channel's declared presentation reaches the connected-channel summary"
        );
    }

    #[tokio::test]
    async fn lifecycle_service_error_returns_unknown_channels() {
        let provider =
            RuntimeCommunicationContextProvider::new(Arc::new(EmptyNotificationChannelsService))
                .with_lifecycle_service(Arc::new(ErrorLifecycleService));
        let ctx = provider
            .begin_communication_context(scope(), Some(actor()))
            .resolve(false)
            .await
            .expect("context");
        assert_eq!(ctx.connected_channels, ConnectedChannelsState::Unknown);
    }

    // --- Tests: timeout path ---

    /// A preferences service whose `get_notification_channels` never resolves.
    /// Used to exercise the shared-timeout Unknown path.
    ///
    /// Note: `tokio/test-util` is not in this crate's feature set, so
    /// `start_paused` / `tokio::time::advance` are unavailable. The test relies
    /// on the real 500 ms wall-clock timeout firing against a `pending()` future.
    struct HangingPreferencesService;

    #[async_trait]
    impl OutboundPreferencesProductService for HangingPreferencesService {
        async fn list_outbound_delivery_targets(
            &self,
            _caller: ProductSurfaceCaller,
        ) -> Result<RebornOutboundDeliveryTargetListResponse, ProductSurfaceError> {
            Ok(RebornOutboundDeliveryTargetListResponse {
                targets: Vec::new(),
                next_cursor: None,
            })
        }

        async fn get_notification_channels(
            &self,
            _caller: ProductSurfaceCaller,
        ) -> Result<RebornNotificationChannelsResponse, ProductSurfaceError> {
            std::future::pending().await
        }
    }

    /// A preferences service whose `get_notification_channels` panics immediately.
    /// This causes the spawned `fetch_communication_context` task to abort with a
    /// `JoinError`, exercising the actor-present degrade-to-unknown path in
    /// `begin_communication_context`.
    struct PanickingPreferencesService;

    #[async_trait]
    impl OutboundPreferencesProductService for PanickingPreferencesService {
        async fn list_outbound_delivery_targets(
            &self,
            _caller: ProductSurfaceCaller,
        ) -> Result<RebornOutboundDeliveryTargetListResponse, ProductSurfaceError> {
            Ok(RebornOutboundDeliveryTargetListResponse {
                targets: Vec::new(),
                next_cursor: None,
            })
        }

        async fn get_notification_channels(
            &self,
            _caller: ProductSurfaceCaller,
        ) -> Result<RebornNotificationChannelsResponse, ProductSurfaceError> {
            panic!("induced panic for JoinError test")
        }
    }

    #[tokio::test]
    async fn actor_present_join_failure_degrades_to_unknown() {
        // When the spawned fetch task panics (JoinError) and an actor IS present,
        // the resolved context must be Some with Unknown states — not None.
        // None would be ambiguous with the "no actor" path and would suppress
        // `delivery_tools_visible` stamping for a run that genuinely has an actor.
        let provider =
            RuntimeCommunicationContextProvider::new(Arc::new(PanickingPreferencesService));

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
            ctx.notification_channels,
            NotificationChannelsState::Unknown,
            "join failure with actor present must degrade notification_channels to Unknown"
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
            None::<ironclaw_loop_contracts::CommunicationRuntimeContext>
        });

        // Ensure the task is actually running and parked before we drop.
        task_started.notified().await;

        let fetch = ironclaw_loop_contracts::CommunicationContextFetch::from_handle(handle, false);
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
    async fn shared_timeout_yields_unknown_for_both_notifications_and_channels() {
        // The notification-channels future never resolves; the 500 ms outer
        // timeout fires. Both notification_channels and connected_channels must
        // be Unknown — never fabricated definitive states. Uses real wall-clock
        // time (500 ms) since tokio/test-util is not in this crate's features.
        let provider =
            RuntimeCommunicationContextProvider::new(Arc::new(HangingPreferencesService));

        let ctx = provider
            .begin_communication_context(scope(), Some(actor()))
            .resolve(false)
            .await
            .expect("communication_context must return Some even on timeout");

        assert_eq!(
            ctx.notification_channels,
            NotificationChannelsState::Unknown,
            "timed-out notification-channels fetch must map to Unknown"
        );
        assert_eq!(
            ctx.connected_channels,
            ConnectedChannelsState::Unknown,
            "timed-out budget must leave connected_channels Unknown"
        );
    }
}
