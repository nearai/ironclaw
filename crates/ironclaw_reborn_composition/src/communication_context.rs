use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use ironclaw_product_workflow::{
    LifecyclePhase, LifecycleProductAction, LifecycleProductContext, LifecycleProductFacade,
    LifecycleProductPayload, LifecycleProductSurfaceContext, OutboundPreferencesProductFacade,
    RebornOutboundDeliveryTargetStatus, WebUiAuthenticatedCaller,
};
use ironclaw_turns::{
    run_profile::{
        CommunicationContextProvider, CommunicationRuntimeContext, ConnectedChannelSummary,
        ConnectedChannelsState, DeliveryTargetState, DeliveryTargetSummary,
    },
    scope::{TurnActor, TurnScope},
};
use tokio::time::timeout;

/// Shared timeout budget for both outbound-preferences and lifecycle fetches.
const OUTBOUND_PREFERENCES_TIMEOUT: Duration = Duration::from_millis(500);

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

#[async_trait]
impl CommunicationContextProvider for RuntimeCommunicationContextProvider {
    async fn communication_context(
        &self,
        scope: &TurnScope,
        actor: Option<&TurnActor>,
        delivery_tools_visible: bool,
        run_origin: Option<ironclaw_turns::TurnRunOrigin>,
    ) -> Option<CommunicationRuntimeContext> {
        let actor = actor?;
        let caller = WebUiAuthenticatedCaller::new(
            scope.tenant_id.clone(),
            actor.user_id.clone(),
            scope.agent_id.clone(),
            scope.project_id.clone(),
        );

        // Fetch outbound delivery preferences.
        let delivery_target = match timeout(
            OUTBOUND_PREFERENCES_TIMEOUT,
            self.outbound_preferences
                .get_outbound_preferences(caller.clone()),
        )
        .await
        {
            Ok(Ok(response)) => match (
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
            Ok(Err(_)) | Err(_) => DeliveryTargetState::Unknown,
        };

        // Fetch connected channels from the lifecycle facade when available.
        let connected_channels = match &self.lifecycle_facade {
            Some(facade) => {
                let context = LifecycleProductContext::Surface(LifecycleProductSurfaceContext {
                    tenant_id: caller.tenant_id,
                    user_id: caller.user_id,
                    agent_id: caller.agent_id,
                    project_id: caller.project_id,
                });
                match timeout(
                    OUTBOUND_PREFERENCES_TIMEOUT,
                    facade.execute(context, LifecycleProductAction::ExtensionList),
                )
                .await
                {
                    Ok(Ok(response)) => {
                        let extensions = match response.payload {
                            Some(LifecycleProductPayload::ExtensionList { extensions, .. }) => {
                                extensions
                            }
                            _ => Vec::new(),
                        };
                        let channels: Vec<ConnectedChannelSummary> = extensions
                            .into_iter()
                            .filter(|ext| {
                                // Only channel-surface extensions count as connected
                                // channels. Lifecycle summaries carry no channel
                                // discriminator yet — #4778 adds the ProductAdapter
                                // surface-kind projection; switch this predicate to it
                                // when that lands. Until then nothing matches and the
                                // slice truthfully renders "Connected channels: none."
                                extension_is_channel_surface(ext)
                                    && ext.phase == LifecyclePhase::Active
                            })
                            .map(|ext| ConnectedChannelSummary {
                                name: ext.summary.name.clone(),
                                // An Active channel extension passed through activation;
                                // treat it as authenticated. Credential readiness would
                                // require ExtensionCredentialSetupService which is not
                                // wired in this provider.
                                authenticated: true,
                                active: true,
                            })
                            .collect();
                        ConnectedChannelsState::Known(channels)
                    }
                    Ok(Err(_)) | Err(_) => ConnectedChannelsState::Unknown,
                }
            }
            None => ConnectedChannelsState::Unknown,
        };

        Some(CommunicationRuntimeContext {
            connected_channels,
            delivery_target,
            delivery_tools_visible,
            run_origin,
        })
    }
}

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
        TurnRunOrigin,
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
            .communication_context(&scope(), None, false, None)
            .await;
        assert!(result.is_none(), "actor None must return None");
    }

    // --- Tests: delivery target state branches ---

    #[tokio::test]
    async fn none_configured_maps_to_none_set() {
        let provider = RuntimeCommunicationContextProvider::new(Arc::new(NoneSetPreferencesFacade));
        let ctx = provider
            .communication_context(&scope(), Some(&actor()), false, None)
            .await
            .expect("context");
        assert_eq!(ctx.delivery_target, DeliveryTargetState::NoneSet);
    }

    #[tokio::test]
    async fn unavailable_status_maps_to_set_unresolved() {
        let provider =
            RuntimeCommunicationContextProvider::new(Arc::new(UnavailablePreferencesFacade));
        let ctx = provider
            .communication_context(&scope(), Some(&actor()), false, None)
            .await
            .expect("context");
        assert_eq!(ctx.delivery_target, DeliveryTargetState::SetUnresolved);
    }

    #[tokio::test]
    async fn target_set_maps_to_set_with_summary() {
        let provider =
            RuntimeCommunicationContextProvider::new(Arc::new(TargetSetPreferencesFacade));
        let ctx = provider
            .communication_context(&scope(), Some(&actor()), false, None)
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
            .communication_context(&scope(), Some(&actor()), false, None)
            .await
            .expect("context");
        assert_eq!(ctx.delivery_target, DeliveryTargetState::Unknown);
    }

    // --- Tests: connected channels ---

    #[tokio::test]
    async fn no_lifecycle_facade_returns_unknown_channels() {
        let provider = RuntimeCommunicationContextProvider::new(Arc::new(NoneSetPreferencesFacade));
        let ctx = provider
            .communication_context(&scope(), Some(&actor()), false, None)
            .await
            .expect("context");
        assert_eq!(ctx.connected_channels, ConnectedChannelsState::Unknown);
    }

    #[tokio::test]
    async fn empty_extension_list_returns_known_empty() {
        let provider = RuntimeCommunicationContextProvider::new(Arc::new(NoneSetPreferencesFacade))
            .with_lifecycle_facade(Arc::new(EmptyLifecycleFacade));
        let ctx = provider
            .communication_context(&scope(), Some(&actor()), false, None)
            .await
            .expect("context");
        assert_eq!(
            ctx.connected_channels,
            ConnectedChannelsState::Known(vec![]),
            "no channel-kind extensions → Known([])"
        );
    }

    #[tokio::test]
    async fn non_channel_extensions_never_appear_as_connected() {
        // Lifecycle summaries carry no channel discriminator until #4778's
        // ProductAdapter surface projection: no extension qualifies, so even
        // active first-party extensions must NOT be presented as connected
        // channels (a web-access tool extension is not a channel). When #4778
        // merges, extension_is_channel_surface switches to the projected
        // surface kind and this test should grow a positive case.
        let provider = RuntimeCommunicationContextProvider::new(Arc::new(NoneSetPreferencesFacade))
            .with_lifecycle_facade(Arc::new(ChannelListLifecycleFacade {
                extensions: vec![
                    channel_extension("telegram"),
                    non_channel_extension("github"),
                    inactive_channel_extension("slack"),
                ],
            }));
        let ctx = provider
            .communication_context(&scope(), Some(&actor()), false, None)
            .await
            .expect("context");
        assert_eq!(
            ctx.connected_channels,
            ConnectedChannelsState::Known(vec![]),
            "no channel discriminator pre-#4778 → Known([]) renders 'none'"
        );
    }

    #[tokio::test]
    async fn lifecycle_facade_error_returns_unknown_channels() {
        let provider = RuntimeCommunicationContextProvider::new(Arc::new(NoneSetPreferencesFacade))
            .with_lifecycle_facade(Arc::new(ErrorLifecycleFacade));
        let ctx = provider
            .communication_context(&scope(), Some(&actor()), false, None)
            .await
            .expect("context");
        assert_eq!(ctx.connected_channels, ConnectedChannelsState::Unknown);
    }

    // --- Tests: run_origin propagation ---

    #[tokio::test]
    async fn run_origin_is_passed_through() {
        let provider = RuntimeCommunicationContextProvider::new(Arc::new(NoneSetPreferencesFacade));
        let ctx = provider
            .communication_context(
                &scope(),
                Some(&actor()),
                false,
                Some(TurnRunOrigin::ScheduledTrigger),
            )
            .await
            .expect("context");
        assert_eq!(ctx.run_origin, Some(TurnRunOrigin::ScheduledTrigger));
    }
}
