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
use tokio::sync::Mutex;

use crate::RebornRuntime;
use crate::outbound_preferences::{OutboundDeliveryTargetEntry, OutboundDeliveryTargetProvider};
use crate::slack_actor_identity::{
    RebornUserIdentityLookup, SLACK_IDENTITY_PROVIDER, SlackUserIdentityActorResolver,
    slack_user_identity_provider_user_id,
};
use crate::slack_channel_routes::{
    SlackChannelRouteAdminRouteConfig, SlackChannelRouteAssignment, SlackChannelRouteError,
    SlackChannelRouteStore, SlackChannelRouteSubjectResolver,
};
use crate::slack_host_state::FilesystemSlackHostState;
use crate::slack_outbound_targets::{
    SlackHostBetaOutboundTargetProvider, SlackOutboundTargetProviderConfig,
    SlackPersonalDmTargetStore,
};
use crate::slack_pairing_notifier::SlackPairingChallengeHttpNotifier;
use crate::slack_personal_binding::{
    RebornIdentityProviderId, RebornIdentityProviderUserId, RebornUserIdentityBinding,
    RebornUserIdentityBindingError, RebornUserIdentityBindingStore,
    SlackPersonalBindingInstallation, SlackPersonalBindingPrincipal, SlackPersonalUserBindingError,
    SlackPersonalUserBindingService,
};
use crate::slack_personal_binding_pairing::{
    IssuedSlackPersonalBindingPairingChallenge, SlackPairingActorResolver,
    SlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingChallengeStore,
    SlackPersonalBindingPairingCode, SlackPersonalBindingPairingError,
    SlackPersonalBindingPairingNotifier, SlackPersonalBindingPairingService,
    SlackPersonalUserBinder,
};
use crate::slack_personal_binding_pairing_serve::SlackPersonalBindingPairingRouteConfig;
use crate::slack_serve::{
    ResolvedSlackIngress, SlackEventsRouteState, SlackIngressError, SlackInstallationResolver,
    SlackInstallationSelector, SlackTeamId, SlackUserId, StaticSlackInstallationResolver,
    slack_events_route_mount,
};
use crate::slack_setup::{
    SlackInstallationSetup, SlackInstallationSetupStore, SlackInstallationSetupUpdate,
    SlackSetupService,
};

use super::{
    SlackHostBetaActorUserResolver, SlackHostBetaBuildError, SlackHostBetaConfig,
    SlackHostBetaConfigInput, SlackHostBetaMounts, SlackHostBetaRuntimeConfig,
    SlackHostBetaRuntimeParts, build_slack_installation_record_with_resolvers,
    slack_bot_token_handle, slack_protocol_egress_from_parts,
};

pub(super) async fn build_runtime_mounts(
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
        DynamicSlackPersonalUserBinder::new(Arc::clone(&setup_service), Arc::clone(&binding_store)),
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
    let challenge_store: Arc<dyn SlackPersonalBindingPairingChallengeStore> = Arc::new(
        DynamicSlackPairingChallengeStore::new(Arc::clone(&setup_service), state.clone()),
    );
    let pairing = SlackPersonalBindingPairingService::new_with_binder(
        dynamic_binding_service,
        challenge_store,
        notifier,
    );
    let channel_route_store: Arc<dyn SlackChannelRouteStore> = state.clone();
    let personal_dm_target_store: Arc<dyn SlackPersonalDmTargetStore> = state.clone();
    if let Some(legacy_setup) = config.legacy_setup.clone() {
        seed_legacy_slack_setup(
            &setup_service,
            Arc::clone(&binding_store),
            Arc::clone(&channel_route_store),
            legacy_setup,
        )
        .await?;
    }
    let resolver = DynamicSlackInstallationResolver::new(
        Arc::clone(&parts),
        Arc::clone(&setup_service),
        state.clone(),
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
        outbound_delivery_target_provider_registered: false,
    })
}

async fn seed_legacy_slack_setup(
    setup_service: &SlackSetupService,
    binding_store: Arc<dyn RebornUserIdentityBindingStore>,
    channel_route_store: Arc<dyn SlackChannelRouteStore>,
    legacy_setup: super::SlackHostBetaLegacySetup,
) -> Result<(), SlackHostBetaBuildError> {
    let setup = setup_service
        .save(SlackInstallationSetupUpdate {
            installation_id: legacy_setup.installation_id,
            team_id: legacy_setup.team_id.clone(),
            api_app_id: legacy_setup.api_app_id,
            user_id: Some(legacy_setup.user_id.to_string()),
            shared_subject_user_id: legacy_setup
                .shared_subject_user_id
                .as_ref()
                .map(ToString::to_string),
            bot_token: Some(legacy_setup.bot_token),
            signing_secret: Some(legacy_setup.signing_secret),
        })
        .await
        .map_err(|error| SlackHostBetaBuildError::InvalidConfig {
            field: "slack.legacy_setup",
            reason: error.to_string(),
        })?;

    let installation_id = setup
        .installation_id()
        .map_err(map_legacy_setup_error("installation_id"))?;
    if !legacy_setup.channel_routes.is_empty() {
        let assignments = legacy_setup
            .channel_routes
            .into_iter()
            .map(|route| SlackChannelRouteAssignment::new(route.channel_id, route.subject_user_id))
            .collect();
        channel_route_store
            .replace_managed_routes(
                setup_service.tenant_id(),
                &installation_id,
                legacy_setup.team_id.as_str(),
                assignments,
            )
            .await
            .map_err(map_legacy_channel_route_error)?;
    }

    if let Some(slack_user_id) = legacy_setup.slack_user_id {
        let provider_user_id =
            slack_user_identity_provider_user_id(&installation_id, slack_user_id.as_str());
        binding_store
            .bind_user_identity(RebornUserIdentityBinding {
                provider: RebornIdentityProviderId::new(SLACK_IDENTITY_PROVIDER)
                    .map_err(map_legacy_binding_error("provider"))?,
                provider_user_id: RebornIdentityProviderUserId::new(provider_user_id)
                    .map_err(map_legacy_binding_error("provider_user_id"))?,
                user_id: legacy_setup.user_id,
            })
            .await
            .map_err(map_legacy_binding_error("slack_user_id"))?;
    }

    Ok(())
}

fn map_legacy_setup_error(
    field: &'static str,
) -> impl FnOnce(crate::slack_setup::SlackSetupError) -> SlackHostBetaBuildError {
    move |error| SlackHostBetaBuildError::InvalidConfig {
        field,
        reason: error.to_string(),
    }
}

fn map_legacy_channel_route_error(error: SlackChannelRouteError) -> SlackHostBetaBuildError {
    SlackHostBetaBuildError::InvalidConfig {
        field: "channel_routes",
        reason: error.to_string(),
    }
}

fn map_legacy_binding_error(
    field: &'static str,
) -> impl FnOnce(RebornUserIdentityBindingError) -> SlackHostBetaBuildError {
    move |error| SlackHostBetaBuildError::InvalidConfig {
        field,
        reason: error.to_string(),
    }
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
    state: Arc<dyn RebornUserIdentityLookup>,
    pairing: SlackPersonalBindingPairingService,
    channel_route_store: Arc<dyn SlackChannelRouteStore>,
    cached_resolver: Arc<Mutex<DynamicSlackInstallationResolverCache>>,
}

impl DynamicSlackInstallationResolver {
    fn new(
        parts: Arc<SlackHostBetaRuntimeParts>,
        setup_service: Arc<SlackSetupService>,
        state: Arc<dyn RebornUserIdentityLookup>,
        pairing: SlackPersonalBindingPairingService,
        channel_route_store: Arc<dyn SlackChannelRouteStore>,
    ) -> Self {
        Self {
            parts,
            setup_service,
            state,
            pairing,
            channel_route_store,
            cached_resolver: Arc::new(Mutex::new(DynamicSlackInstallationResolverCache::default())),
        }
    }

    async fn resolver(&self) -> Result<Arc<StaticSlackInstallationResolver>, SlackIngressError> {
        let setup = self
            .setup_service
            .current_setup()
            .await
            .map_err(|_| SlackIngressError::InstallationNotFound)?
            .ok_or(SlackIngressError::InstallationNotFound)?;
        let revision = setup.revision;
        if let Some(resolver) = self.cached_resolver(revision).await {
            return Ok(resolver);
        }

        let resolver = Arc::new(self.build_resolver(setup).await?);
        let mut cache = self.cached_resolver.lock().await;
        if let Some(current) = &cache.current
            && current.revision == revision
        {
            return Ok(Arc::clone(&current.resolver));
        }
        if let Some(previous) = cache.current.replace(DynamicCachedSlackResolver {
            revision,
            resolver: Arc::clone(&resolver),
        }) {
            cache.retired.push(previous.resolver);
        }
        Ok(resolver)
    }

    async fn cached_resolver(&self, revision: u64) -> Option<Arc<StaticSlackInstallationResolver>> {
        let cache = self.cached_resolver.lock().await;
        cache
            .current
            .as_ref()
            .filter(|current| current.revision == revision)
            .map(|current| Arc::clone(&current.resolver))
    }

    async fn build_resolver(
        &self,
        setup: SlackInstallationSetup,
    ) -> Result<StaticSlackInstallationResolver, SlackIngressError> {
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

    async fn drain_cached_resolvers(&self) {
        let resolvers = {
            let cache = self.cached_resolver.lock().await;
            cache.resolvers()
        };
        for resolver in &resolvers {
            resolver.drain_installations().await;
        }
        let mut cache = self.cached_resolver.lock().await;
        cache.forget_retired(&resolvers);
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
        Box::pin(async move { self.drain_cached_resolvers().await })
    }
}

#[derive(Default)]
struct DynamicSlackInstallationResolverCache {
    current: Option<DynamicCachedSlackResolver>,
    retired: Vec<Arc<StaticSlackInstallationResolver>>,
}

impl DynamicSlackInstallationResolverCache {
    fn resolvers(&self) -> Vec<Arc<StaticSlackInstallationResolver>> {
        self.current
            .iter()
            .map(|current| Arc::clone(&current.resolver))
            .chain(self.retired.iter().map(Arc::clone))
            .collect()
    }

    fn forget_retired(&mut self, drained: &[Arc<StaticSlackInstallationResolver>]) {
        self.retired
            .retain(|resolver| !drained.iter().any(|drained| Arc::ptr_eq(drained, resolver)));
    }
}

struct DynamicCachedSlackResolver {
    revision: u64,
    resolver: Arc<StaticSlackInstallationResolver>,
}

#[derive(Clone)]
struct DynamicSlackPairingChallengeStore {
    setup_service: Arc<SlackSetupService>,
    store: Arc<dyn SlackPersonalBindingPairingChallengeStore>,
}

impl DynamicSlackPairingChallengeStore {
    fn new(
        setup_service: Arc<SlackSetupService>,
        store: Arc<dyn SlackPersonalBindingPairingChallengeStore>,
    ) -> Self {
        Self {
            setup_service,
            store,
        }
    }

    async fn current_setup_revision(
        &self,
        challenge: &SlackPersonalBindingPairingChallenge,
    ) -> Result<u64, SlackPersonalBindingPairingError> {
        let setup = self
            .setup_service
            .current_setup()
            .await
            .map_err(|error| SlackPersonalBindingPairingError::Backend(error.to_string()))?
            .ok_or(SlackPersonalBindingPairingError::ChallengeNotFound)?;
        let installation_id = setup
            .installation_id()
            .map_err(|error| SlackPersonalBindingPairingError::Backend(error.to_string()))?;
        if installation_id != challenge.installation_id {
            return Err(SlackPersonalBindingPairingError::ChallengeNotFound);
        }
        Ok(setup.revision)
    }

    async fn bind_to_current_setup(
        &self,
        mut challenge: SlackPersonalBindingPairingChallenge,
    ) -> Result<SlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError> {
        challenge.setup_revision = Some(self.current_setup_revision(&challenge).await?);
        Ok(challenge)
    }

    async fn require_current_setup(
        &self,
        challenge: SlackPersonalBindingPairingChallenge,
    ) -> Result<SlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError> {
        let current_revision = self.current_setup_revision(&challenge).await?;
        if challenge.setup_revision == Some(current_revision) {
            Ok(challenge)
        } else {
            Err(SlackPersonalBindingPairingError::ChallengeNotFound)
        }
    }
}

#[async_trait::async_trait]
impl SlackPersonalBindingPairingChallengeStore for DynamicSlackPairingChallengeStore {
    async fn issue_challenge(
        &self,
        challenge: SlackPersonalBindingPairingChallenge,
    ) -> Result<IssuedSlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError> {
        let challenge = self.bind_to_current_setup(challenge).await?;
        self.store.issue_challenge(challenge).await
    }

    async fn get_challenge(
        &self,
        code: &SlackPersonalBindingPairingCode,
    ) -> Result<SlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError> {
        let challenge = self.store.get_challenge(code).await?;
        self.require_current_setup(challenge).await
    }

    async fn consume_challenge(
        &self,
        code: &SlackPersonalBindingPairingCode,
    ) -> Result<SlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError> {
        let preview = self.store.get_challenge(code).await?;
        self.require_current_setup(preview).await?;
        let challenge = self.store.consume_challenge(code).await?;
        self.require_current_setup(challenge).await
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

#[cfg(test)]
mod tests {
    use std::sync::Mutex as StdMutex;

    use ironclaw_host_api::{SecretHandle, UserId};
    use ironclaw_secrets::InMemorySecretStore;
    use secrecy::{ExposeSecret, SecretString};
    use tokio::sync::RwLock;

    use super::*;
    use crate::slack_channel_routes::{
        SlackChannelRoute, SlackChannelRouteKey, SlackChannelRouteListPage,
    };
    use crate::{SlackHostBetaChannelRoute, SlackHostBetaLegacySetup};

    #[tokio::test]
    async fn dynamic_pairing_challenge_store_rejects_stale_setup_revision() {
        let setup_store = Arc::new(InMemorySetupStore::new(setup_record(1)));
        let setup_service = Arc::new(SlackSetupService::new(
            TenantId::new("tenant:slack").unwrap(),
            AgentId::new("agent:slack").unwrap(),
            None,
            UserId::new("user:operator").unwrap(),
            setup_store.clone(),
            Arc::new(InMemorySecretStore::default()),
        ));
        let store = DynamicSlackPairingChallengeStore::new(
            setup_service,
            Arc::new(StaticChallengeStore::default()),
        );
        let code = SlackPersonalBindingPairingCode::new("ABC12345").unwrap();
        let challenge = SlackPersonalBindingPairingChallenge {
            installation_id: AdapterInstallationId::new("install-a").unwrap(),
            slack_user_id: SlackUserId::new("U123"),
            setup_revision: None,
        };

        let issued = store
            .issue_challenge(challenge)
            .await
            .expect("challenge issued");
        assert_eq!(issued.challenge.setup_revision, Some(1));
        assert_eq!(
            store
                .get_challenge(&code)
                .await
                .expect("challenge is current")
                .setup_revision,
            Some(1)
        );

        setup_store.put(setup_record(2)).await;

        assert!(matches!(
            store.get_challenge(&code).await,
            Err(SlackPersonalBindingPairingError::ChallengeNotFound)
        ));
        assert!(matches!(
            store.consume_challenge(&code).await,
            Err(SlackPersonalBindingPairingError::ChallengeNotFound)
        ));
    }

    #[tokio::test]
    async fn seed_legacy_slack_setup_persists_setup_routes_and_identity_binding() {
        let setup_store = Arc::new(InMemorySetupStore::empty());
        let secret_store = Arc::new(InMemorySecretStore::default());
        let setup_service = Arc::new(SlackSetupService::new(
            TenantId::new("tenant:slack").unwrap(),
            AgentId::new("agent:slack").unwrap(),
            None,
            UserId::new("user:operator").unwrap(),
            setup_store.clone(),
            secret_store,
        ));
        let binding_store = Arc::new(RecordingBindingStore::default());
        let route_store = Arc::new(RecordingRouteStore::default());

        seed_legacy_slack_setup(
            &setup_service,
            binding_store.clone(),
            route_store.clone(),
            SlackHostBetaLegacySetup {
                installation_id: "install-a".to_string(),
                team_id: "T123".to_string(),
                api_app_id: "A123".to_string(),
                slack_user_id: Some("U123".to_string()),
                user_id: UserId::new("user:operator").unwrap(),
                shared_subject_user_id: Some(UserId::new("user:shared-slack").unwrap()),
                channel_routes: vec![SlackHostBetaChannelRoute::new(
                    "CENG",
                    UserId::new("user:eng-team-agent").unwrap(),
                )],
                signing_secret: SecretString::from("legacy-signing-secret"),
                bot_token: SecretString::from("xoxb-legacy"),
            },
        )
        .await
        .expect("legacy setup seeds");

        let setup = setup_service
            .current_setup()
            .await
            .expect("setup read")
            .expect("setup stored");
        assert_eq!(setup.installation_id, "install-a");
        assert_eq!(setup.team_id, "T123");
        assert_eq!(setup.api_app_id, "A123");
        assert_eq!(setup.user_id, "user:operator");
        assert_eq!(
            setup.shared_subject_user_id.as_deref(),
            Some("user:shared-slack")
        );
        assert_eq!(
            setup_service
                .bot_token(&setup)
                .await
                .expect("bot token")
                .expose_secret(),
            "xoxb-legacy"
        );
        assert_eq!(
            setup_service
                .signing_secret(&setup)
                .await
                .expect("signing secret")
                .expose_secret(),
            "legacy-signing-secret"
        );

        let recorded_routes = route_store.routes.lock().unwrap().clone();
        assert_eq!(recorded_routes.len(), 1);
        assert_eq!(recorded_routes[0].channel_id, "CENG");
        assert_eq!(
            recorded_routes[0].subject_user_id.as_str(),
            "user:eng-team-agent"
        );

        let bindings = binding_store.bindings.lock().unwrap().clone();
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].provider.as_str(), SLACK_IDENTITY_PROVIDER);
        assert_eq!(
            bindings[0].provider_user_id.as_str(),
            slack_user_identity_provider_user_id(
                &AdapterInstallationId::new("install-a").unwrap(),
                "U123"
            )
        );
        assert_eq!(bindings[0].user_id.as_str(), "user:operator");
    }

    fn setup_record(revision: u64) -> SlackInstallationSetup {
        SlackInstallationSetup {
            installation_id: "install-a".to_string(),
            team_id: "T123".to_string(),
            api_app_id: "A123".to_string(),
            user_id: "user:operator".to_string(),
            shared_subject_user_id: None,
            bot_token_handle: SecretHandle::new(format!("bot_{revision}")).unwrap(),
            signing_secret_handle: SecretHandle::new(format!("signing_{revision}")).unwrap(),
            revision,
            updated_at: chrono::Utc::now(),
        }
    }

    #[derive(Debug)]
    struct InMemorySetupStore {
        setup: RwLock<Option<SlackInstallationSetup>>,
    }

    impl InMemorySetupStore {
        fn new(setup: SlackInstallationSetup) -> Self {
            Self {
                setup: RwLock::new(Some(setup)),
            }
        }

        fn empty() -> Self {
            Self {
                setup: RwLock::new(None),
            }
        }

        async fn put(&self, setup: SlackInstallationSetup) {
            *self.setup.write().await = Some(setup);
        }
    }

    #[async_trait::async_trait]
    impl SlackInstallationSetupStore for InMemorySetupStore {
        async fn get_slack_installation_setup(
            &self,
        ) -> Result<Option<SlackInstallationSetup>, crate::slack_setup::SlackSetupError> {
            Ok(self.setup.read().await.clone())
        }

        async fn put_slack_installation_setup(
            &self,
            setup: &SlackInstallationSetup,
        ) -> Result<(), crate::slack_setup::SlackSetupError> {
            self.put(setup.clone()).await;
            Ok(())
        }
    }

    #[derive(Debug, Default)]
    struct StaticChallengeStore {
        challenge: StdMutex<Option<SlackPersonalBindingPairingChallenge>>,
    }

    #[derive(Debug, Default)]
    struct RecordingBindingStore {
        bindings: StdMutex<Vec<RebornUserIdentityBinding>>,
    }

    #[async_trait::async_trait]
    impl RebornUserIdentityBindingStore for RecordingBindingStore {
        async fn bind_user_identity(
            &self,
            binding: RebornUserIdentityBinding,
        ) -> Result<(), RebornUserIdentityBindingError> {
            self.bindings.lock().unwrap().push(binding);
            Ok(())
        }
    }

    #[derive(Debug, Default)]
    struct RecordingRouteStore {
        routes: StdMutex<Vec<SlackChannelRouteAssignment>>,
    }

    #[async_trait::async_trait]
    impl SlackChannelRouteStore for RecordingRouteStore {
        async fn list_routes(
            &self,
            _tenant_id: &TenantId,
            _installation_id: &AdapterInstallationId,
            _team_id: &str,
            _cursor: usize,
            _limit: usize,
        ) -> Result<SlackChannelRouteListPage, SlackChannelRouteError> {
            Ok(SlackChannelRouteListPage {
                routes: Vec::new(),
                next_cursor: None,
            })
        }

        async fn upsert_route(
            &self,
            key: SlackChannelRouteKey,
            subject_user_id: UserId,
        ) -> Result<SlackChannelRoute, SlackChannelRouteError> {
            Ok(SlackChannelRoute::new(key, subject_user_id))
        }

        async fn delete_route(
            &self,
            _key: &SlackChannelRouteKey,
        ) -> Result<bool, SlackChannelRouteError> {
            Ok(false)
        }

        async fn replace_managed_routes(
            &self,
            tenant_id: &TenantId,
            installation_id: &AdapterInstallationId,
            team_id: &str,
            assignments: Vec<SlackChannelRouteAssignment>,
        ) -> Result<Vec<SlackChannelRoute>, SlackChannelRouteError> {
            *self.routes.lock().unwrap() = assignments.clone();
            assignments
                .into_iter()
                .map(|assignment| {
                    Ok(SlackChannelRoute::new(
                        SlackChannelRouteKey::new(
                            tenant_id.clone(),
                            installation_id.clone(),
                            team_id.to_string(),
                            assignment.channel_id,
                        )?,
                        assignment.subject_user_id,
                    ))
                })
                .collect()
        }

        async fn resolve_subject_user_id(
            &self,
            _key: &SlackChannelRouteKey,
        ) -> Result<Option<UserId>, SlackChannelRouteError> {
            Ok(None)
        }
    }

    #[async_trait::async_trait]
    impl SlackPersonalBindingPairingChallengeStore for StaticChallengeStore {
        async fn issue_challenge(
            &self,
            challenge: SlackPersonalBindingPairingChallenge,
        ) -> Result<IssuedSlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError>
        {
            *self.challenge.lock().unwrap() = Some(challenge.clone());
            Ok(IssuedSlackPersonalBindingPairingChallenge {
                code: SlackPersonalBindingPairingCode::new("ABC12345").unwrap(),
                challenge,
            })
        }

        async fn get_challenge(
            &self,
            _code: &SlackPersonalBindingPairingCode,
        ) -> Result<SlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError>
        {
            self.challenge
                .lock()
                .unwrap()
                .clone()
                .ok_or(SlackPersonalBindingPairingError::ChallengeNotFound)
        }

        async fn consume_challenge(
            &self,
            _code: &SlackPersonalBindingPairingCode,
        ) -> Result<SlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError>
        {
            self.challenge
                .lock()
                .unwrap()
                .take()
                .ok_or(SlackPersonalBindingPairingError::ChallengeNotFound)
        }
    }
}
