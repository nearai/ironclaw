use ironclaw_product_contracts::lifecycle_service::{
    LifecycleProductContext, LifecycleProductService, LifecycleProductSurfaceContext,
};
use std::{collections::HashMap, sync::Arc, time::Duration};

use crate::{
    ExtensionCredentialSetupService, LifecycleProductAction, LifecycleProductPayload,
    LifecycleProductResponse, OutboundPreferencesProductService,
    RebornOutboundDeliveryTargetStatus,
    reborn_services::{CallerExtensionAuth, caller_extension_auth},
};
use futures::StreamExt;
use ironclaw_auth::{ChannelAuthAccountState, ChannelConnectionService};
use ironclaw_extension_contracts::{state::InstallationState, surface::CapabilitySurfaceKind};
use ironclaw_host_api::ids::ExtensionId;
use ironclaw_host_api::turn::{TurnActor, TurnScope};
use ironclaw_loop_contracts::{
    CommunicationContextFetch, CommunicationContextProvider, CommunicationRuntimeContext,
    ConnectedChannelSummary, ConnectedChannelsState, NotificationChannelsState,
    PendingExtensionAuthState,
};
use ironclaw_product_contracts::surface::{ProductSurfaceCaller, ProductSurfaceError};
use tokio::join;
use tokio::time::timeout;

/// Shared timeout budget for the whole communication-context fetch
/// (notification channels + lifecycle/channels + per-caller extension-auth
/// classification). Everything runs under this single budget; expiry degrades
/// notification-channels, connected-channels, and pending-extension-auth to
/// `Unknown`.
const COMMUNICATION_CONTEXT_FETCH_TIMEOUT: Duration = Duration::from_millis(500);

pub struct RuntimeCommunicationContextProvider {
    outbound_preferences: Arc<dyn OutboundPreferencesProductService>,
    /// Optional lifecycle service used to populate connected channels.
    /// When None the slice always renders `Connected channels: unknown.`
    lifecycle_service: Option<Arc<dyn LifecycleProductService>>,
    /// Optional per-caller credential readiness port (the same scope-gated
    /// `credential_status` the extensions card and the runtime auth gate
    /// resolve through). Without it, extensions that declare required
    /// credentials cannot be claimed authenticated for the caller (#7247).
    extension_credentials: Option<Arc<dyn ExtensionCredentialSetupService>>,
    /// Optional per-caller channel connection/auth-account port. Without it,
    /// channel surfaces that require a personal connection cannot be claimed
    /// authenticated for the caller (#7247).
    channel_connections: Option<Arc<dyn ChannelConnectionService>>,
}

impl RuntimeCommunicationContextProvider {
    pub fn new(outbound_preferences: Arc<dyn OutboundPreferencesProductService>) -> Self {
        Self {
            outbound_preferences,
            lifecycle_service: None,
            extension_credentials: None,
            channel_connections: None,
        }
    }

    pub fn with_lifecycle_service(
        mut self,
        lifecycle_service: Arc<dyn LifecycleProductService>,
    ) -> Self {
        self.lifecycle_service = Some(lifecycle_service);
        self
    }

    pub fn with_extension_credentials(
        mut self,
        extension_credentials: Arc<dyn ExtensionCredentialSetupService>,
    ) -> Self {
        self.extension_credentials = Some(extension_credentials);
        self
    }

    pub fn with_channel_connections(
        mut self,
        channel_connections: Arc<dyn ChannelConnectionService>,
    ) -> Self {
        self.channel_connections = Some(channel_connections);
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
        let extension_credentials = self.extension_credentials.clone();
        let channel_connections = self.channel_connections.clone();
        let actor_present = actor.is_some();
        let handle = tokio::spawn(async move {
            fetch_communication_context(
                outbound_preferences,
                lifecycle_service,
                extension_credentials,
                channel_connections,
                scope,
                actor,
            )
            .await
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
    extension_credentials: Option<Arc<dyn ExtensionCredentialSetupService>>,
    channel_connections: Option<Arc<dyn ChannelConnectionService>>,
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

    // The caller's per-channel connection + auth-account maps, used to prove
    // (or refuse to claim) personal channel connections. `None` means the data
    // is unavailable — the port is not wired or a lookup failed — which is
    // distinct from empty maps ("this caller has connected nothing").
    let connections_fut = async {
        let service = match (&lifecycle_service, &channel_connections) {
            (Some(_), Some(service)) => service,
            _ => return None,
        };
        let (connections, account_states) = join!(
            service.caller_channel_connections(caller.clone()),
            service.caller_channel_account_states(caller.clone()),
        );
        match (connections, account_states) {
            (Ok(connections), Ok(account_states)) => Some((connections, account_states)),
            (Err(error), _) | (_, Err(error)) => {
                tracing::debug!(
                    error = %error,
                    "caller channel connection lookup failed; connection proof unavailable"
                );
                None
            }
        }
    };

    // All fetches AND the per-extension auth classification share a single
    // 500 ms budget.
    let combined_result = timeout(COMMUNICATION_CONTEXT_FETCH_TIMEOUT, async {
        let (notifications_result, lifecycle_result, connection_maps) =
            join!(notifications_fut, lifecycle_fut, connections_fut);

        let notification_channels = match notifications_result {
            Ok(response) => NotificationChannelsState::Known(
                response
                    .channels
                    .iter()
                    .filter(|channel| {
                        channel.status == RebornOutboundDeliveryTargetStatus::Available
                    })
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

        let (connected_channels, pending_extension_auth) = classify_installed_extensions(
            lifecycle_result,
            connection_maps,
            extension_credentials.as_deref(),
            &caller,
        )
        .await;

        (
            notification_channels,
            connected_channels,
            pending_extension_auth,
        )
    })
    .await;

    let (notification_channels, connected_channels, pending_extension_auth) = match combined_result
    {
        Ok(states) => states,
        Err(_) => {
            tracing::debug!("communication context budget expired; degrading to unknown");
            // Budget expired — everything is unknown.
            return Some(CommunicationRuntimeContext {
                connected_channels: ConnectedChannelsState::Unknown,
                notification_channels: NotificationChannelsState::Unknown,
                pending_extension_auth: PendingExtensionAuthState::Unknown,
                delivery_tools_visible: false,
            });
        }
    };

    Some(CommunicationRuntimeContext {
        connected_channels,
        notification_channels,
        pending_extension_auth,
        delivery_tools_visible: false,
    })
}

/// Classify the installed-extension list into the model-facing
/// connected-channels and pending-extension-auth states, using per-caller
/// truth only (#7247).
///
/// Fail-closed: a channel is `authenticated` only when the caller's required
/// credentials are configured and any personal connection/binding the channel
/// declares is proven; a credentialed non-channel extension whose required
/// credential the caller has not configured is named in the pending-auth
/// state. When any needed verdict is unknowable (readiness or connection
/// ports unavailable, or a lookup failed), BOTH states degrade to `Unknown` —
/// the slice claims nothing rather than fabricating either direction.
async fn classify_installed_extensions(
    lifecycle_result: Option<Result<LifecycleProductResponse, ProductSurfaceError>>,
    connection_maps: Option<(
        HashMap<ExtensionId, bool>,
        HashMap<ExtensionId, ChannelAuthAccountState>,
    )>,
    extension_credentials: Option<&dyn ExtensionCredentialSetupService>,
    caller: &ProductSurfaceCaller,
) -> (ConnectedChannelsState, PendingExtensionAuthState) {
    let unknown = (
        ConnectedChannelsState::Unknown,
        PendingExtensionAuthState::Unknown,
    );
    let response = match lifecycle_result {
        // A present response means a lifecycle service was wired and returned
        // the installed-extension list.
        Some(Ok(response)) => response,
        Some(Err(error)) => {
            tracing::debug!(
                error = %error,
                "lifecycle extension list fetch failed; degrading connected channels to unknown"
            );
            return unknown;
        }
        // None means lifecycle service was skipped or not wired — not an error.
        None => return unknown,
    };
    let extensions = match response.payload {
        Some(LifecycleProductPayload::ExtensionList { extensions, .. }) => extensions,
        _ => Vec::new(),
    };
    let maps = connection_maps
        .as_ref()
        .map(|(connections, account_states)| (connections, account_states));
    let candidates: Vec<_> = extensions
        .into_iter()
        .filter(|ext| ext.phase == InstallationState::Active)
        .filter(|ext| {
            // Nothing to claim either way for a credential-free non-channel;
            // skip the readiness lookup.
            extension_is_channel_surface(ext) || !ext.summary.credential_requirements.is_empty()
        })
        .collect();
    // Bounded fan-out (#7474 review): serial awaits divided the single
    // 500 ms context budget by the extension count, starving the whole
    // communication slice on deployments with several credentialed
    // extensions. Same cap as the extensions card's readiness fan-out
    // (`EXTENSION_READINESS_CONCURRENCY` in `reborn_services/extensions.rs`,
    // kept module-private there); `buffered` preserves input order so the
    // rendered lists stay deterministic.
    const EXTENSION_AUTH_FETCH_CONCURRENCY: usize = 8;
    let verdicts: Vec<_> = futures::stream::iter(candidates.into_iter().map(|ext| async move {
        let verdict = caller_extension_auth(extension_credentials, maps, caller, &ext).await;
        (ext, verdict)
    }))
    .buffered(EXTENSION_AUTH_FETCH_CONCURRENCY)
    .collect()
    .await;
    let mut channels: Vec<ConnectedChannelSummary> = Vec::new();
    let mut pending: Vec<String> = Vec::new();
    for (ext, verdict) in verdicts {
        let is_channel = extension_is_channel_surface(&ext);
        match (is_channel, verdict) {
            (_, CallerExtensionAuth::Unknown) => {
                tracing::debug!(
                    extension = %ext.summary.package_ref.id.as_str(),
                    "per-caller extension auth verdict unavailable; degrading \
                     communication slice to unknown"
                );
                return unknown;
            }
            (true, verdict) => channels.push(ConnectedChannelSummary {
                name: ext.summary.name.clone(),
                authenticated: verdict == CallerExtensionAuth::Authenticated,
                active: true,
                presentation: ext.summary.channel_presentation.clone(),
            }),
            (false, CallerExtensionAuth::Unauthenticated) => {
                pending.push(ext.summary.name.clone());
            }
            (false, CallerExtensionAuth::Authenticated) => {}
        }
    }
    (
        ConnectedChannelsState::Known(channels),
        PendingExtensionAuthState::Known(pending),
    )
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

    use ironclaw_auth::ChannelAuthAccountState;

    use std::collections::HashMap;

    use crate::{
        ExtensionCredentialSetupService, ExtensionCredentialStatusRequest,
        ExtensionCredentialSubmitRequest, LifecycleExtensionCredentialRequirement,
        LifecycleExtensionCredentialSetup, LifecycleExtensionRuntimeKind, LifecycleExtensionSource,
        LifecycleExtensionSummary, LifecycleInstalledExtensionSummary, LifecyclePackageKind,
        LifecyclePackageRef, LifecycleProductAction, LifecycleProductPayload,
        LifecycleProductResponse, OutboundPreferencesProductService, RebornNotificationChannel,
        RebornNotificationChannelsResponse, RebornOutboundDeliveryTargetId,
        RebornOutboundDeliveryTargetListResponse, RebornOutboundDeliveryTargetStatus,
    };
    use async_trait::async_trait;
    use ironclaw_auth::{
        ChannelConnectionService, CredentialAccountId, CredentialAccountLabel,
        CredentialAccountProjection, CredentialAccountStatus, CredentialOwnership,
    };
    use ironclaw_extension_contracts::{state::InstallationState, surface::CapabilitySurfaceKind};
    use ironclaw_host_api::ids::{AgentId, ExtensionId, ProjectId, TenantId, UserId};
    use ironclaw_host_api::turn::{TurnActor, TurnScope};
    use ironclaw_loop_contracts::{
        CommunicationContextProvider, ConnectedChannelsState, NotificationChannelsState,
        PendingExtensionAuthState,
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
                command_prefix: None,
            }),
            "the channel's declared presentation reaches the connected-channel summary"
        );
    }

    /// #7247 regression: an Active channel extension whose connect strategy
    /// requires a personal per-caller connection (OAuth) must NOT be reported
    /// `authenticated` to the model when no per-caller connection proof is
    /// available. With no connection/credential ports wired the provider has
    /// no proof, so it must fail closed to `Unknown` rather than claim the
    /// calling user is connected.
    #[tokio::test]
    async fn active_channel_requiring_connection_is_not_claimed_authenticated_without_proof() {
        let mut slack = channel_extension("slack");
        slack.summary.channel_connection = Some(crate::ChannelConnectionRequirement {
            channel: "slack".to_string(),
            display_name: "Slack".to_string(),
            strategy: crate::RebornChannelConnectStrategy::OAuth,
            instructions: String::new(),
            input_placeholder: String::new(),
            submit_label: String::new(),
            error_message: String::new(),
        });
        let provider =
            RuntimeCommunicationContextProvider::new(Arc::new(EmptyNotificationChannelsService))
                .with_lifecycle_service(Arc::new(ChannelListLifecycleService {
                    extensions: vec![slack],
                }));
        let ctx = provider
            .begin_communication_context(scope(), Some(actor()))
            .resolve(false)
            .await
            .expect("context");
        assert_eq!(
            ctx.connected_channels,
            ConnectedChannelsState::Unknown,
            "a personal-connection channel with no per-caller proof must not be \
             claimed authenticated for the caller",
        );
    }

    // --- Per-caller credential/connection fakes (#7247) ---

    /// Credential-status port returning a fixed per-caller account status
    /// (`None` account status = "the caller has no credential account").
    struct StaticCredentialService {
        account_status: Option<CredentialAccountStatus>,
    }

    #[async_trait]
    impl ExtensionCredentialSetupService for StaticCredentialService {
        async fn credential_status(
            &self,
            request: ExtensionCredentialStatusRequest,
        ) -> Result<Option<CredentialAccountProjection>, ProductSurfaceError> {
            Ok(self
                .account_status
                .map(|status| CredentialAccountProjection {
                    id: CredentialAccountId::new(),
                    provider: request.provider.clone(),
                    label: CredentialAccountLabel::new("fixture account")
                        .expect("valid account label"),
                    status,
                    ownership: CredentialOwnership::UserReusable,
                    owner_extension: None,
                    granted_extensions: Vec::new(),
                    secret_handle_count: 1,
                }))
        }

        async fn submit_manual_token(
            &self,
            _request: ExtensionCredentialSubmitRequest,
        ) -> Result<CredentialAccountId, ProductSurfaceError> {
            Ok(CredentialAccountId::new())
        }
    }

    /// Channel-connection port returning a fixed per-caller connections map
    /// and, optionally, per-channel durable auth-account states.
    struct StaticChannelConnections {
        connections: HashMap<ExtensionId, bool>,
        account_states: HashMap<ExtensionId, ChannelAuthAccountState>,
    }

    impl StaticChannelConnections {
        fn none() -> Self {
            Self {
                connections: HashMap::new(),
                account_states: HashMap::new(),
            }
        }

        fn connected(extension_id: &str) -> Self {
            let mut connections = HashMap::new();
            connections.insert(
                ExtensionId::new(extension_id).expect("valid extension id"),
                true,
            );
            Self {
                connections,
                account_states: HashMap::new(),
            }
        }

        /// A caller whose durable account row exists but is not `Configured`
        /// (expired / refresh-failed): the account-backed unconnected path.
        fn with_account_state(extension_id: &str, state: ChannelAuthAccountState) -> Self {
            let mut account_states = HashMap::new();
            account_states.insert(
                ExtensionId::new(extension_id).expect("valid extension id"),
                state,
            );
            Self {
                connections: HashMap::new(),
                account_states,
            }
        }
    }

    #[async_trait]
    impl ChannelConnectionService for StaticChannelConnections {
        async fn caller_channel_connections(
            &self,
            _caller: ProductSurfaceCaller,
        ) -> Result<HashMap<ExtensionId, bool>, ProductSurfaceError> {
            Ok(self.connections.clone())
        }

        async fn caller_channel_account_states(
            &self,
            _caller: ProductSurfaceCaller,
        ) -> Result<HashMap<ExtensionId, ChannelAuthAccountState>, ProductSurfaceError> {
            Ok(self.account_states.clone())
        }
    }

    fn oauth_channel_extension(name: &str) -> LifecycleInstalledExtensionSummary {
        let mut ext = channel_extension(name);
        ext.summary.channel_connection = Some(crate::ChannelConnectionRequirement {
            channel: name.to_string(),
            display_name: name.to_string(),
            strategy: crate::RebornChannelConnectStrategy::OAuth,
            instructions: String::new(),
            input_placeholder: String::new(),
            submit_label: String::new(),
            error_message: String::new(),
        });
        ext
    }

    fn credentialed_tool_extension(name: &str) -> LifecycleInstalledExtensionSummary {
        let mut ext = non_channel_extension(name);
        ext.summary.credential_requirements = vec![LifecycleExtensionCredentialRequirement {
            name: format!("{name}_runtime_token"),
            provider: name.to_string(),
            required: true,
            setup: LifecycleExtensionCredentialSetup::ManualToken,
        }];
        ext
    }

    /// #7247 (the GitHub repro): an Active tools-only extension with a
    /// required credential the caller has NOT configured must be named in the
    /// pending-extension-auth state, so the model is told the caller is not
    /// authenticated instead of inferring "already connected" from the
    /// installed/active catalog state and the visible tools.
    #[tokio::test]
    async fn credentialed_tool_extension_without_caller_credential_is_pending_auth() {
        let provider =
            RuntimeCommunicationContextProvider::new(Arc::new(EmptyNotificationChannelsService))
                .with_lifecycle_service(Arc::new(ChannelListLifecycleService {
                    extensions: vec![credentialed_tool_extension("github")],
                }))
                .with_extension_credentials(Arc::new(StaticCredentialService {
                    account_status: None,
                }));
        let ctx = provider
            .begin_communication_context(scope(), Some(actor()))
            .resolve(false)
            .await
            .expect("context");
        assert_eq!(
            ctx.pending_extension_auth,
            PendingExtensionAuthState::Known(vec!["github".to_string()]),
            "a required credential the caller has not configured must be reported as pending auth",
        );
        assert_eq!(
            ctx.connected_channels,
            ConnectedChannelsState::Known(Vec::new()),
            "a tools-only extension is never a connected channel",
        );
    }

    /// Truthful positive: the same credentialed extension with a Configured
    /// per-caller account is NOT pending auth.
    #[tokio::test]
    async fn credentialed_tool_extension_with_configured_credential_is_not_pending_auth() {
        let provider =
            RuntimeCommunicationContextProvider::new(Arc::new(EmptyNotificationChannelsService))
                .with_lifecycle_service(Arc::new(ChannelListLifecycleService {
                    extensions: vec![credentialed_tool_extension("github")],
                }))
                .with_extension_credentials(Arc::new(StaticCredentialService {
                    account_status: Some(CredentialAccountStatus::Configured),
                }));
        let ctx = provider
            .begin_communication_context(scope(), Some(actor()))
            .resolve(false)
            .await
            .expect("context");
        assert_eq!(
            ctx.pending_extension_auth,
            PendingExtensionAuthState::Known(Vec::new()),
            "a configured per-caller credential must not be reported as pending auth",
        );
    }

    /// An expired credential account is a row, not readiness: it must read as
    /// pending auth, not as authenticated.
    #[tokio::test]
    async fn credentialed_tool_extension_with_expired_credential_is_pending_auth() {
        let provider =
            RuntimeCommunicationContextProvider::new(Arc::new(EmptyNotificationChannelsService))
                .with_lifecycle_service(Arc::new(ChannelListLifecycleService {
                    extensions: vec![credentialed_tool_extension("github")],
                }))
                .with_extension_credentials(Arc::new(StaticCredentialService {
                    account_status: Some(CredentialAccountStatus::Expired),
                }));
        let ctx = provider
            .begin_communication_context(scope(), Some(actor()))
            .resolve(false)
            .await
            .expect("context");
        assert_eq!(
            ctx.pending_extension_auth,
            PendingExtensionAuthState::Known(vec!["github".to_string()]),
            "an expired credential account must read as pending auth, not authenticated",
        );
    }

    /// Truthful positive (#6478 guard for personal-connection channels): an
    /// OAuth channel the caller HAS personally connected still reads
    /// authenticated.
    #[tokio::test]
    async fn oauth_channel_connected_by_caller_reads_authenticated() {
        let provider =
            RuntimeCommunicationContextProvider::new(Arc::new(EmptyNotificationChannelsService))
                .with_lifecycle_service(Arc::new(ChannelListLifecycleService {
                    extensions: vec![oauth_channel_extension("slack")],
                }))
                .with_extension_credentials(Arc::new(StaticCredentialService {
                    account_status: None,
                }))
                .with_channel_connections(Arc::new(StaticChannelConnections::connected("slack")));
        let ctx = provider
            .begin_communication_context(scope(), Some(actor()))
            .resolve(false)
            .await
            .expect("context");
        let channels = match ctx.connected_channels {
            ConnectedChannelsState::Known(channels) => channels,
            other => panic!("expected Known channels, got {other:?}"),
        };
        assert_eq!(channels.len(), 1);
        assert!(
            channels[0].authenticated,
            "a channel the caller personally connected must keep reading authenticated",
        );
    }

    /// #7247: an OAuth channel the caller has NOT connected renders as a
    /// truthful negative (`unauthenticated`), not as a fabricated positive —
    /// connection data is available and proves nothing for this caller.
    #[tokio::test]
    async fn oauth_channel_not_connected_by_caller_reads_unauthenticated() {
        let provider =
            RuntimeCommunicationContextProvider::new(Arc::new(EmptyNotificationChannelsService))
                .with_lifecycle_service(Arc::new(ChannelListLifecycleService {
                    extensions: vec![oauth_channel_extension("slack")],
                }))
                .with_extension_credentials(Arc::new(StaticCredentialService {
                    account_status: None,
                }))
                .with_channel_connections(Arc::new(StaticChannelConnections::none()));
        let ctx = provider
            .begin_communication_context(scope(), Some(actor()))
            .resolve(false)
            .await
            .expect("context");
        let channels = match ctx.connected_channels {
            ConnectedChannelsState::Known(channels) => channels,
            other => panic!("expected Known channels, got {other:?}"),
        };
        assert_eq!(channels.len(), 1);
        assert!(
            !channels[0].authenticated,
            "a personal-connection channel with no proof for this caller must read unauthenticated",
        );
    }

    /// #7474 review: the account-backed unconnected path — a durable account
    /// row EXISTS for the caller but is not `Configured` (expired /
    /// refresh-failed). Row existence is not readiness; the channel must
    /// still read unauthenticated.
    #[tokio::test]
    async fn oauth_channel_with_nonconfigured_account_state_reads_unauthenticated() {
        for status in [
            ironclaw_auth::CredentialAccountStatus::Expired,
            ironclaw_auth::CredentialAccountStatus::RefreshFailed,
        ] {
            let provider = RuntimeCommunicationContextProvider::new(Arc::new(
                EmptyNotificationChannelsService,
            ))
            .with_lifecycle_service(Arc::new(ChannelListLifecycleService {
                extensions: vec![oauth_channel_extension("slack")],
            }))
            .with_extension_credentials(Arc::new(StaticCredentialService {
                account_status: None,
            }))
            .with_channel_connections(Arc::new(
                StaticChannelConnections::with_account_state(
                    "slack",
                    ChannelAuthAccountState {
                        account_status: Some(status),
                        active_flow_status: None,
                    },
                ),
            ));
            let ctx = provider
                .begin_communication_context(scope(), Some(actor()))
                .resolve(false)
                .await
                .expect("context");
            let channels = match ctx.connected_channels {
                ConnectedChannelsState::Known(channels) => channels,
                other => panic!("expected Known channels, got {other:?}"),
            };
            assert_eq!(channels.len(), 1);
            assert!(
                !channels[0].authenticated,
                "a {status:?} account row must not read authenticated — row \
                 existence is not readiness",
            );
        }
    }

    /// #7247 fail-closed: with no credential port wired, a credentialed
    /// extension's per-caller verdict is unknowable — the slice must claim
    /// nothing in either direction.
    #[tokio::test]
    async fn credentialed_extension_without_credential_port_degrades_to_unknown() {
        let provider =
            RuntimeCommunicationContextProvider::new(Arc::new(EmptyNotificationChannelsService))
                .with_lifecycle_service(Arc::new(ChannelListLifecycleService {
                    extensions: vec![credentialed_tool_extension("github")],
                }));
        let ctx = provider
            .begin_communication_context(scope(), Some(actor()))
            .resolve(false)
            .await
            .expect("context");
        assert_eq!(ctx.connected_channels, ConnectedChannelsState::Unknown);
        assert_eq!(
            ctx.pending_extension_auth,
            PendingExtensionAuthState::Unknown
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
