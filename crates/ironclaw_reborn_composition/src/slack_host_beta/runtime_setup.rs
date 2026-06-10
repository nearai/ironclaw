use std::sync::Arc;

use ironclaw_host_api::{AgentId, ProjectId, TenantId};
use ironclaw_product_adapters::{
    AdapterInstallationId, EgressCredentialHandle, EgressRequest, EgressResponse,
    ProtocolHttpEgress, ProtocolHttpEgressError, RedactedString,
};
use ironclaw_product_workflow::{
    ProductConversationSubjectRouteResolver, RebornOutboundDeliveryTargetId, RebornServicesError,
    RebornServicesErrorCode, RebornServicesErrorKind, WebUiAuthenticatedCaller,
};
use ironclaw_turns::ReplyTargetBindingRef;

use crate::RebornRuntime;
use crate::outbound_preferences::{OutboundDeliveryTargetEntry, OutboundDeliveryTargetProvider};
use crate::slack_actor_identity::SlackUserIdentityActorResolver;
use crate::slack_channel_routes::{
    SlackChannelRouteAdminRouteConfig, SlackChannelRouteStore, SlackChannelRouteSubjectResolver,
};
use crate::slack_host_state::FilesystemSlackHostState;
use crate::slack_outbound_targets::{
    SlackHostBetaOutboundTargetProvider, SlackOutboundTargetProviderConfig,
    SlackPersonalDmTargetStore,
};
use crate::slack_pairing_notifier::SlackPairingChallengeHttpNotifier;
use crate::slack_personal_binding::{
    RebornUserIdentityBinding, RebornUserIdentityBindingError, RebornUserIdentityBindingStore,
    SlackPersonalBindingInstallation, SlackPersonalBindingPrincipal, SlackPersonalUserBindingError,
    SlackPersonalUserBindingService,
};
use crate::slack_personal_binding_pairing::{
    SlackPairingActorResolver, SlackPersonalBindingPairingChallengeStore,
    SlackPersonalBindingPairingNotifier, SlackPersonalBindingPairingService,
    SlackPersonalUserBinder,
};
use crate::slack_personal_binding_pairing_serve::SlackPersonalBindingPairingRouteConfig;
use crate::slack_serve::{
    ResolvedSlackIngress, SlackEventsRouteState, SlackIngressError, SlackInstallationResolver,
    SlackInstallationSelector, SlackTeamId, SlackUserId, StaticSlackInstallationResolver,
    slack_events_route_mount,
};
use crate::slack_setup::{SlackInstallationSetup, SlackInstallationSetupStore, SlackSetupService};

use super::{
    SlackHostBetaActorUserResolver, SlackHostBetaBuildError, SlackHostBetaConfig,
    SlackHostBetaConfigInput, SlackHostBetaMounts, SlackHostBetaRuntimeConfig,
    SlackHostBetaRuntimeParts, build_slack_installation_record_with_resolvers,
    slack_bot_token_handle, slack_protocol_egress_from_parts,
};

pub(super) fn build_runtime_mounts(
    runtime: &RebornRuntime,
    config: SlackHostBetaRuntimeConfig,
) -> Result<SlackHostBetaMounts, SlackHostBetaBuildError> {
    let parts = Arc::new(SlackHostBetaRuntimeParts::from_runtime(runtime)?);
    let state = Arc::new(FilesystemSlackHostState::new(
        Arc::clone(&parts.local_runtime.host_state_filesystem),
        config.tenant_id.clone(),
        config.operator_user_id.clone(),
        config.agent_id.clone(),
        config.project_id.clone(),
    ));
    let setup_store: Arc<dyn SlackInstallationSetupStore> = state.clone();
    let setup_service = Arc::new(SlackSetupService::new(
        config.tenant_id.clone(),
        config.agent_id.clone(),
        config.project_id.clone(),
        config.operator_user_id.clone(),
        setup_store,
        runtime.services().secret_store(),
    ));
    let binding_store: Arc<dyn RebornUserIdentityBindingStore> = state.clone();
    let dynamic_binding_service: Arc<dyn SlackPersonalUserBinder> = Arc::new(
        DynamicSlackPersonalUserBinder::new(Arc::clone(&setup_service), binding_store),
    );
    let token_handle = slack_bot_token_handle()?;
    let notifier: Arc<dyn SlackPersonalBindingPairingNotifier> =
        Arc::new(SlackPairingChallengeHttpNotifier::new(
            Arc::new(DynamicSlackProtocolHttpEgress::new(
                Arc::clone(&parts),
                Arc::clone(&setup_service),
                token_handle.clone(),
            )),
            token_handle,
        ));
    let challenge_store: Arc<dyn SlackPersonalBindingPairingChallengeStore> = state.clone();
    let pairing = SlackPersonalBindingPairingService::new_with_binder(
        dynamic_binding_service,
        challenge_store,
        notifier,
    );
    let channel_route_store: Arc<dyn SlackChannelRouteStore> = state.clone();
    let personal_dm_target_store: Arc<dyn SlackPersonalDmTargetStore> = state.clone();
    let resolver = DynamicSlackInstallationResolver::new(
        Arc::clone(&parts),
        Arc::clone(&setup_service),
        state,
        pairing.clone(),
        Arc::clone(&channel_route_store),
    );
    let channel_routes = SlackChannelRouteAdminRouteConfig::dynamic(
        config.tenant_id.clone(),
        config.operator_user_id.clone(),
        Arc::clone(&channel_route_store),
        Arc::clone(&setup_service),
    );

    Ok(SlackHostBetaMounts {
        events: slack_events_route_mount(SlackEventsRouteState::from_resolver(Arc::new(resolver))),
        personal_binding_pairing: SlackPersonalBindingPairingRouteConfig::new(pairing),
        channel_routes,
        outbound_delivery_target_provider: Arc::new(SlackDynamicOutboundTargetProvider::new(
            SlackDynamicOutboundTargetProviderConfig {
                tenant_id: config.tenant_id,
                agent_id: config.agent_id,
                project_id: config.project_id,
            },
            setup_service,
            channel_route_store,
            personal_dm_target_store,
        )),
    })
}

#[derive(Clone)]
struct DynamicSlackProtocolHttpEgress {
    parts: Arc<SlackHostBetaRuntimeParts>,
    setup_service: Arc<SlackSetupService>,
    token_handle: EgressCredentialHandle,
}

impl DynamicSlackProtocolHttpEgress {
    fn new(
        parts: Arc<SlackHostBetaRuntimeParts>,
        setup_service: Arc<SlackSetupService>,
        token_handle: EgressCredentialHandle,
    ) -> Self {
        Self {
            parts,
            setup_service,
            token_handle,
        }
    }

    async fn configured_egress(
        &self,
    ) -> Result<Arc<dyn ProtocolHttpEgress>, ProtocolHttpEgressError> {
        let setup = self
            .setup_service
            .current_setup()
            .await
            .map_err(map_setup_error_to_egress)?
            .ok_or_else(|| ProtocolHttpEgressError::PolicyDenied {
                reason: RedactedString::new("Slack setup is not configured"),
            })?;
        let config = slack_host_beta_config_from_setup(&self.setup_service, setup)
            .await
            .map_err(map_setup_error_to_egress)
            .and_then(|config| {
                config.map_err(|error| ProtocolHttpEgressError::PolicyDenied {
                    reason: RedactedString::new(error.to_string()),
                })
            })?;
        slack_protocol_egress_from_parts(&self.parts, &config, self.token_handle.clone()).map_err(
            |error| ProtocolHttpEgressError::PolicyDenied {
                reason: RedactedString::new(error.to_string()),
            },
        )
    }
}

#[async_trait::async_trait]
impl ProtocolHttpEgress for DynamicSlackProtocolHttpEgress {
    async fn send(
        &self,
        request: EgressRequest,
    ) -> Result<EgressResponse, ProtocolHttpEgressError> {
        self.configured_egress().await?.send(request).await
    }
}

#[derive(Clone)]
struct DynamicSlackInstallationResolver {
    parts: Arc<SlackHostBetaRuntimeParts>,
    setup_service: Arc<SlackSetupService>,
    state: Arc<FilesystemSlackHostState<crate::factory::LocalDevRootFilesystem>>,
    pairing: SlackPersonalBindingPairingService,
    channel_route_store: Arc<dyn SlackChannelRouteStore>,
}

impl DynamicSlackInstallationResolver {
    fn new(
        parts: Arc<SlackHostBetaRuntimeParts>,
        setup_service: Arc<SlackSetupService>,
        state: Arc<FilesystemSlackHostState<crate::factory::LocalDevRootFilesystem>>,
        pairing: SlackPersonalBindingPairingService,
        channel_route_store: Arc<dyn SlackChannelRouteStore>,
    ) -> Self {
        Self {
            parts,
            setup_service,
            state,
            pairing,
            channel_route_store,
        }
    }

    async fn resolver(&self) -> Result<StaticSlackInstallationResolver, SlackIngressError> {
        let setup = self
            .setup_service
            .current_setup()
            .await
            .map_err(|_| SlackIngressError::InstallationNotFound)?
            .ok_or(SlackIngressError::InstallationNotFound)?;
        let config = slack_host_beta_config_from_setup(&self.setup_service, setup)
            .await
            .map_err(|_| SlackIngressError::InstallationNotFound)?
            .map_err(|_| SlackIngressError::InstallationNotFound)?;
        let identity_lookup: Arc<dyn crate::slack_actor_identity::RebornUserIdentityLookup> =
            self.state.clone();
        let actor_user_resolver = Arc::new(SlackHostBetaActorUserResolver::new(
            config.installation_id.clone(),
            config.slack_actor.clone(),
            config.user_id.clone(),
            Arc::new(SlackUserIdentityActorResolver::new(Arc::clone(
                &identity_lookup,
            ))),
            Arc::new(SlackPairingActorResolver::new(
                identity_lookup,
                self.pairing.clone(),
            )),
        ));
        let subject_route_resolver: Arc<dyn ProductConversationSubjectRouteResolver> =
            Arc::new(SlackChannelRouteSubjectResolver::new(
                config.tenant_id.clone(),
                config.installation_id.clone(),
                Arc::clone(&self.channel_route_store),
            ));
        let record = build_slack_installation_record_with_resolvers(
            &self.parts,
            config,
            actor_user_resolver,
            Some(subject_route_resolver),
        )
        .map_err(|_| SlackIngressError::InstallationNotFound)?;
        Ok(StaticSlackInstallationResolver::new([record]))
    }
}

impl SlackInstallationResolver for DynamicSlackInstallationResolver {
    fn resolve_ingress<'a>(
        &'a self,
        headers: &'a axum::http::HeaderMap,
        body: &'a [u8],
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<ResolvedSlackIngress, SlackIngressError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move { self.resolver().await?.resolve_ingress(headers, body).await })
    }

    fn drain_installations<'a>(
        &'a self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            if let Ok(resolver) = self.resolver().await {
                resolver.drain_installations().await;
            }
        })
    }
}

#[derive(Clone)]
struct DynamicSlackPersonalUserBinder {
    setup_service: Arc<SlackSetupService>,
    store: Arc<dyn RebornUserIdentityBindingStore>,
}

impl DynamicSlackPersonalUserBinder {
    fn new(
        setup_service: Arc<SlackSetupService>,
        store: Arc<dyn RebornUserIdentityBindingStore>,
    ) -> Self {
        Self {
            setup_service,
            store,
        }
    }

    async fn binding_service(
        &self,
    ) -> Result<SlackPersonalUserBindingService, SlackPersonalUserBindingError> {
        let setup = self
            .setup_service
            .current_setup()
            .await
            .map_err(|error| {
                SlackPersonalUserBindingError::BindingStore(
                    RebornUserIdentityBindingError::Backend(error.to_string()),
                )
            })?
            .ok_or_else(|| SlackPersonalUserBindingError::UnknownInstallation {
                tenant_id: self.setup_service.tenant_id().clone(),
                installation_id: AdapterInstallationId::new("slack_setup_missing")
                    .expect("missing Slack setup sentinel installation id must be valid"), // safety: literal is non-empty and contains no control characters.
            })?;
        let installation = SlackPersonalBindingInstallation {
            tenant_id: self.setup_service.tenant_id().clone(),
            installation_id: setup.installation_id().map_err(|error| {
                SlackPersonalUserBindingError::BindingStore(
                    RebornUserIdentityBindingError::Backend(error.to_string()),
                )
            })?,
            selector: SlackInstallationSelector::app_team(setup.api_app_id, setup.team_id),
        };
        Ok(SlackPersonalUserBindingService::new(
            [installation],
            Arc::clone(&self.store),
        ))
    }
}

impl std::fmt::Debug for DynamicSlackPersonalUserBinder {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DynamicSlackPersonalUserBinder")
            .field("tenant_id", &self.setup_service.tenant_id())
            .finish_non_exhaustive()
    }
}

#[async_trait::async_trait]
impl SlackPersonalUserBinder for DynamicSlackPersonalUserBinder {
    async fn validate_installation_actor(
        &self,
        principal: &SlackPersonalBindingPrincipal,
        installation_id: &AdapterInstallationId,
        slack_user_id: &SlackUserId,
    ) -> Result<(), SlackPersonalUserBindingError> {
        self.binding_service().await?.validate_installation_actor(
            principal,
            installation_id,
            slack_user_id,
        )
    }

    async fn bind_installation_actor(
        &self,
        principal: SlackPersonalBindingPrincipal,
        installation_id: AdapterInstallationId,
        slack_user_id: SlackUserId,
    ) -> Result<RebornUserIdentityBinding, SlackPersonalUserBindingError> {
        self.binding_service()
            .await?
            .bind_installation_actor(principal, installation_id, slack_user_id)
            .await
    }
}

#[derive(Clone)]
struct SlackDynamicOutboundTargetProviderConfig {
    tenant_id: TenantId,
    agent_id: AgentId,
    project_id: Option<ProjectId>,
}

#[derive(Clone)]
struct SlackDynamicOutboundTargetProvider {
    config: SlackDynamicOutboundTargetProviderConfig,
    setup_service: Arc<SlackSetupService>,
    channel_route_store: Arc<dyn SlackChannelRouteStore>,
    personal_dm_target_store: Arc<dyn SlackPersonalDmTargetStore>,
}

impl SlackDynamicOutboundTargetProvider {
    fn new(
        config: SlackDynamicOutboundTargetProviderConfig,
        setup_service: Arc<SlackSetupService>,
        channel_route_store: Arc<dyn SlackChannelRouteStore>,
        personal_dm_target_store: Arc<dyn SlackPersonalDmTargetStore>,
    ) -> Self {
        Self {
            config,
            setup_service,
            channel_route_store,
            personal_dm_target_store,
        }
    }

    async fn configured_provider(
        &self,
    ) -> Result<Option<SlackHostBetaOutboundTargetProvider>, RebornServicesError> {
        let Some(setup) = self
            .setup_service
            .current_setup()
            .await
            .map_err(|_| slack_dynamic_target_unavailable())?
        else {
            return Ok(None);
        };
        let installation_id = setup
            .installation_id()
            .map_err(|_| slack_dynamic_target_unavailable())?;
        let team_id = setup.team_id();
        Ok(Some(SlackHostBetaOutboundTargetProvider::new(
            SlackOutboundTargetProviderConfig {
                tenant_id: self.config.tenant_id.clone(),
                agent_id: self.config.agent_id.clone(),
                project_id: self.config.project_id.clone(),
                installation_id,
                team_id,
                configured_channel_routes: Vec::new(),
            },
            Arc::clone(&self.channel_route_store),
            Arc::clone(&self.personal_dm_target_store),
        )))
    }
}

impl std::fmt::Debug for SlackDynamicOutboundTargetProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SlackDynamicOutboundTargetProvider")
            .field("tenant_id", &self.config.tenant_id)
            .field("agent_id", &self.config.agent_id)
            .field("project_id", &self.config.project_id)
            .finish_non_exhaustive()
    }
}

#[async_trait::async_trait]
impl OutboundDeliveryTargetProvider for SlackDynamicOutboundTargetProvider {
    async fn list_outbound_delivery_targets(
        &self,
        caller: &WebUiAuthenticatedCaller,
    ) -> Result<Vec<OutboundDeliveryTargetEntry>, RebornServicesError> {
        let Some(provider) = self.configured_provider().await? else {
            return Ok(Vec::new());
        };
        provider.list_outbound_delivery_targets(caller).await
    }

    async fn resolve_outbound_delivery_target(
        &self,
        caller: &WebUiAuthenticatedCaller,
        target_id: &RebornOutboundDeliveryTargetId,
    ) -> Result<Option<OutboundDeliveryTargetEntry>, RebornServicesError> {
        let Some(provider) = self.configured_provider().await? else {
            return Ok(None);
        };
        provider
            .resolve_outbound_delivery_target(caller, target_id)
            .await
    }

    async fn resolve_reply_target_binding(
        &self,
        caller: &WebUiAuthenticatedCaller,
        target: &ReplyTargetBindingRef,
    ) -> Result<Option<OutboundDeliveryTargetEntry>, RebornServicesError> {
        let Some(provider) = self.configured_provider().await? else {
            return Ok(None);
        };
        provider.resolve_reply_target_binding(caller, target).await
    }
}

fn slack_dynamic_target_unavailable() -> RebornServicesError {
    RebornServicesError {
        code: RebornServicesErrorCode::Unavailable,
        kind: RebornServicesErrorKind::ServiceUnavailable,
        status_code: 503,
        retryable: true,
        field: None,
        validation_code: None,
    }
}

async fn slack_host_beta_config_from_setup(
    setup_service: &SlackSetupService,
    setup: SlackInstallationSetup,
) -> Result<Result<SlackHostBetaConfig, SlackHostBetaBuildError>, crate::slack_setup::SlackSetupError>
{
    let user_id = setup.user_id()?;
    let shared_subject_user_id = setup.shared_subject_user_id()?;
    let signing_secret = setup_service.signing_secret(&setup).await?;
    let bot_token = setup_service.bot_token(&setup).await?;
    let tenant_id = setup_service.tenant_id().clone();
    let agent_id = setup_service.agent_id().clone();
    let project_id = setup_service.project_id().cloned();
    Ok(SlackHostBetaConfig::new(SlackHostBetaConfigInput {
        tenant_id,
        agent_id,
        project_id,
        installation_id: setup.installation_id.clone(),
        team_id: SlackTeamId::new(setup.team_id.clone()),
        api_app_id: Some(setup.api_app_id.clone()),
        slack_user_id: None,
        user_id,
        shared_subject_user_id,
        channel_routes: Vec::new(),
        signing_secret,
        bot_token,
    }))
}

fn map_setup_error_to_egress(
    error: crate::slack_setup::SlackSetupError,
) -> ProtocolHttpEgressError {
    ProtocolHttpEgressError::PolicyDenied {
        reason: RedactedString::new(error.to_string()),
    }
}
