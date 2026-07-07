use std::sync::Arc;

use ironclaw_host_api::{AgentId, ProjectId, TenantId};
use ironclaw_outbound::TriggeredRunDeliveryStore;
use ironclaw_product_adapters::{
    AdapterInstallationId, EgressCredentialHandle, EgressRequest, EgressResponse,
    ProtocolHttpEgress, ProtocolHttpEgressError, RedactedString,
};
use ironclaw_product_workflow::{
    LifecyclePackageKind, LifecyclePackageRef, LifecyclePhase,
    ProductConversationSubjectRouteResolver, RebornOutboundDeliveryTargetId, RebornServicesError,
    RebornServicesErrorCode, RebornServicesErrorKind, WebUiAuthenticatedCaller,
};
use ironclaw_triggers::TriggerFire;
use ironclaw_turns::{ReplyTargetBindingRef, TurnRunId, TurnScope};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

use crate::RebornRuntime;
use crate::extension_host::extension_lifecycle::{
    ExtensionActivationMode, RebornLocalExtensionManagementPort,
};
use crate::outbound::outbound_preferences::OutboundDeliveryTargetEntry;
use crate::outbound::{OutboundDeliveryTargetProvider, OutboundDeliveryTargetRegistrationOutcome};
use crate::slack_actor_identity::{
    RebornUserIdentityLookup, SLACK_IDENTITY_PROVIDER, SlackUserIdentityActorResolver,
    slack_user_identity_provider_user_id,
};
use crate::slack_channel_routes::{
    SlackChannelRouteAdminRouteConfig, SlackChannelRouteAssignment, SlackChannelRouteError,
    SlackChannelRouteStore, SlackChannelRouteSubjectResolver, SlackChannelSetupActivation,
    SlackChannelSetupActivationError,
};
use crate::slack_delivery::{PostSubmitDeliveryHook, TriggeredRunDeliveryDriver};
use crate::slack_host_state::FilesystemSlackHostState;
use crate::slack_outbound_targets::{
    SlackHostBetaOutboundTargetProvider, SlackOutboundTargetProviderConfig, SlackPersonalDmTarget,
    SlackPersonalDmTargetError, SlackPersonalDmTargetProvisioner, SlackPersonalDmTargetStore,
};
use crate::slack_pairing_notifier::SlackPairingChallengeHttpNotifier;
use crate::slack_personal_binding::{
    RebornIdentityProviderId, RebornIdentityProviderUserId, RebornUserIdentityBinding,
    RebornUserIdentityBindingDeleteStore, RebornUserIdentityBindingError,
    RebornUserIdentityBindingStore, SlackPersonalBindingInstallation,
    SlackPersonalBindingPrincipal, SlackPersonalUserBindingError, SlackPersonalUserBindingService,
};
use crate::slack_personal_binding_pairing::{
    IssuedSlackPersonalBindingPairingChallenge, SlackPairingActorResolver,
    SlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingChallengeStore,
    SlackPersonalBindingPairingCode, SlackPersonalBindingPairingError,
    SlackPersonalBindingPairingNotifier, SlackPersonalBindingPairingService,
    SlackPersonalDmTargetProvisioning, SlackPersonalUserBinder,
};
use crate::slack_personal_binding_pairing_serve::SlackPersonalBindingPairingRouteConfig;
use crate::slack_serve::{
    ResolvedSlackCommand, ResolvedSlackIngress, SlackCommandsRouteState, SlackEventsRouteState,
    SlackIngressError, SlackIngressService, SlackInstallationResolver, SlackInstallationSelector,
    SlackTeamId, SlackUserId, StaticSlackInstallationResolver, slack_commands_route_mount,
    slack_events_route_mount,
};
use crate::slack_setup::{
    SlackInstallationSetup, SlackInstallationSetupStore, SlackInstallationSetupUpdate,
    SlackSetupService,
};

use super::{
    SlackHostBetaActorUserResolver, SlackHostBetaBuildError, SlackHostBetaConfig,
    SlackHostBetaConfigInput, SlackHostBetaMounts, SlackHostBetaRuntimeConfig,
    SlackHostBetaRuntimeParts, SlackPersonalConnectionScope, SlackPersonalConnectionScopeResolver,
    build_slack_installation_record_with_resolvers, build_triggered_run_delivery_hook_from_parts,
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
    let token_handle = slack_bot_token_handle()?;
    let binding_store: Arc<dyn RebornUserIdentityBindingStore> = state.clone();
    let user_identity_lookup: Arc<dyn RebornUserIdentityLookup> = state.clone();
    let user_identity_delete_store: Arc<dyn RebornUserIdentityBindingDeleteStore> = state.clone();
    let channel_route_store: Arc<dyn SlackChannelRouteStore> = state.clone();
    let personal_dm_target_store: Arc<dyn SlackPersonalDmTargetStore> = state.clone();
    let dynamic_binding_service: Arc<dyn SlackPersonalUserBinder> = Arc::new(
        DynamicSlackPersonalUserBinder::new(Arc::clone(&setup_service), Arc::clone(&binding_store)),
    );
    let notifier: Arc<dyn SlackPersonalBindingPairingNotifier> =
        Arc::new(SlackPairingChallengeHttpNotifier::new(
            Arc::new(DynamicSlackProtocolHttpEgress::new(
                Arc::clone(&parts),
                Arc::clone(&setup_service),
                token_handle.clone(),
            )),
            token_handle.clone(),
        ));
    let challenge_store: Arc<dyn SlackPersonalBindingPairingChallengeStore> = Arc::new(
        DynamicSlackPairingChallengeStore::new(Arc::clone(&setup_service), state.clone()),
    );
    let dm_provisioner: Arc<dyn SlackPersonalDmTargetProvisioning> =
        Arc::new(DynamicSlackPersonalDmTargetProvisioner::new(
            Arc::clone(&parts),
            Arc::clone(&setup_service),
            token_handle.clone(),
            Arc::clone(&personal_dm_target_store),
        ));
    let pairing = SlackPersonalBindingPairingService::new_with_binder(
        dynamic_binding_service,
        challenge_store,
        notifier,
    )
    .with_dm_provisioner(dm_provisioner);
    if let Some(legacy_setup) = config.legacy_setup.clone() {
        seed_legacy_slack_setup_if_missing(
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
    let mut channel_routes = SlackChannelRouteAdminRouteConfig::dynamic(
        Arc::clone(&channel_route_store),
        Arc::clone(&setup_service),
    );
    if let Some(extension_management) = &parts.local_runtime.extension_management {
        channel_routes = channel_routes.with_setup_activation(Arc::new(
            DynamicSlackChannelSetupActivation::new(Arc::clone(extension_management)),
        ));
    }

    let outbound_delivery_target_provider: Arc<dyn OutboundDeliveryTargetProvider> =
        Arc::new(SlackDynamicOutboundTargetProvider::new(
            SlackDynamicOutboundTargetProviderConfig {
                tenant_id: config.tenant_id.clone(),
                agent_id: config.agent_id.clone(),
                project_id: config.project_id.clone(),
            },
            Arc::clone(&setup_service),
            channel_route_store,
            Arc::clone(&personal_dm_target_store),
        ));
    let provider_key = slack_dynamic_outbound_delivery_target_provider_key(&config);
    let provider_already_registered = runtime
        .outbound_delivery_target_provider_key_registered(&provider_key)
        .map_err(
            |error| SlackHostBetaBuildError::OutboundDeliveryTargetRegistration {
                reason: error.to_string(),
            },
        )?;
    if !provider_already_registered {
        match runtime
            .register_outbound_delivery_target_provider(
                provider_key,
                Arc::clone(&outbound_delivery_target_provider),
            )
            .map_err(
                |error| SlackHostBetaBuildError::OutboundDeliveryTargetRegistration {
                    reason: error.to_string(),
                },
            )? {
            OutboundDeliveryTargetRegistrationOutcome::Registered => {}
            OutboundDeliveryTargetRegistrationOutcome::Replaced => {
                return Err(SlackHostBetaBuildError::OutboundDeliveryTargetRegistration {
                    reason: "Slack dynamic outbound delivery target provider was concurrently registered".to_string(),
                });
            }
        }
    }
    let delivery_store: Arc<dyn TriggeredRunDeliveryStore> =
        Arc::clone(&parts.local_runtime.triggered_run_delivery);
    let trigger_delivery_hook: Arc<dyn PostSubmitDeliveryHook> =
        Arc::new(DynamicSlackTriggeredRunDeliveryHook::new(
            Arc::clone(&parts),
            Arc::clone(&setup_service),
            delivery_store,
        ));
    let hook_set = runtime.set_trigger_post_submit_hook(trigger_delivery_hook);
    if !hook_set && runtime.trigger_post_submit_hook_is_set() && !provider_already_registered {
        return Err(SlackHostBetaBuildError::OutboundDeliveryTargetRegistration {
            reason: "Slack dynamic triggered-run delivery hook is already wired for a different Slack host config".to_string(),
        });
    }

    // Share one installation resolver across the events and `/pair` commands
    // routes: a single source of truth for the Slack signing identity, and the
    // events drain covers the shared resolver.
    let resolver: Arc<dyn SlackInstallationResolver> = Arc::new(resolver);
    let commands = slack_commands_route_mount(SlackCommandsRouteState::new(
        SlackIngressService::new(Arc::clone(&resolver)),
        pairing.clone(),
        state.clone(),
    ));
    let personal_connection_scope_resolver: Arc<dyn SlackPersonalConnectionScopeResolver> =
        Arc::new(DynamicSlackPersonalConnectionScopeResolver {
            setup_service: Arc::clone(&setup_service),
        });

    let channel_connection_resume =
        crate::channel_connection_resume::build_channel_connection_resume_service(
            Arc::clone(&parts.local_runtime.turn_state),
            Arc::clone(&parts.turn_coordinator),
        );

    Ok(SlackHostBetaMounts {
        events: slack_events_route_mount(SlackEventsRouteState::from_resolver(resolver)),
        commands,
        personal_binding_pairing: SlackPersonalBindingPairingRouteConfig::new(
            pairing,
            channel_connection_resume,
        ),
        channel_routes,
        tenant_id: config.tenant_id.clone(),
        personal_connection_scope: None,
        personal_connection_scope_resolver,
        user_identity_lookup,
        user_identity_delete_store,
        personal_dm_target_store,
        outbound_delivery_target_provider,
        outbound_delivery_target_provider_registered: true,
    })
}

struct DynamicSlackChannelSetupActivation {
    extension_management: Arc<RebornLocalExtensionManagementPort>,
}

impl DynamicSlackChannelSetupActivation {
    fn new(extension_management: Arc<RebornLocalExtensionManagementPort>) -> Self {
        Self {
            extension_management,
        }
    }
}

#[async_trait::async_trait]
impl SlackChannelSetupActivation for DynamicSlackChannelSetupActivation {
    async fn activate_slack_channel_after_setup_save(
        &self,
    ) -> Result<(), SlackChannelSetupActivationError> {
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "slack")
            .map_err(slack_setup_activation_error)?;
        let projection = self
            .extension_management
            .project(package_ref.clone())
            .await
            .map_err(slack_setup_activation_error)?;
        if projection.phase == LifecyclePhase::Discovered {
            return Ok(());
        }
        self.extension_management
            .activate(package_ref, ExtensionActivationMode::Static)
            .await
            .map_err(slack_setup_activation_error)?;
        Ok(())
    }
}

fn slack_setup_activation_error(error: impl std::fmt::Display) -> SlackChannelSetupActivationError {
    SlackChannelSetupActivationError::new(error.to_string())
}

struct DynamicSlackPersonalConnectionScopeResolver {
    setup_service: Arc<SlackSetupService>,
}

#[async_trait::async_trait]
impl SlackPersonalConnectionScopeResolver for DynamicSlackPersonalConnectionScopeResolver {
    async fn resolve_personal_connection_scope(
        &self,
    ) -> Result<Option<SlackPersonalConnectionScope>, String> {
        let Some(setup) = self
            .setup_service
            .current_setup()
            .await
            .map_err(|error| error.to_string())?
        else {
            return Ok(None);
        };
        let installation_id =
            AdapterInstallationId::new(setup.installation_id).map_err(|error| error.to_string())?;
        Ok(Some(SlackPersonalConnectionScope {
            installation_id,
            team_id: SlackTeamId::new(setup.team_id),
        }))
    }
}

fn slack_dynamic_outbound_delivery_target_provider_key(
    config: &SlackHostBetaRuntimeConfig,
) -> String {
    let mut hasher = Sha256::new();
    hash_provider_key_field(&mut hasher, config.tenant_id.as_str());
    hash_provider_key_field(&mut hasher, config.agent_id.as_str());
    hash_provider_key_field(
        &mut hasher,
        config.project_id.as_ref().map_or("", ProjectId::as_str),
    );
    hash_provider_key_field(&mut hasher, config.operator_user_id.as_str());

    let digest = hasher.finalize();
    let mut suffix = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut suffix, "{byte:02x}");
    }
    format!("slack-host-beta-runtime-setup:{suffix}")
}

fn hash_provider_key_field(hasher: &mut Sha256, value: &str) {
    hasher.update(value.len().to_be_bytes());
    hasher.update(value.as_bytes());
}

async fn seed_legacy_slack_setup_if_missing(
    setup_service: &SlackSetupService,
    binding_store: Arc<dyn RebornUserIdentityBindingStore>,
    channel_route_store: Arc<dyn SlackChannelRouteStore>,
    legacy_setup: super::SlackHostBetaLegacySetup,
) -> Result<(), SlackHostBetaBuildError> {
    if setup_service
        .current_setup()
        .await
        .map_err(map_legacy_setup_error("slack.legacy_setup"))?
        .is_some()
    {
        return Ok(());
    }

    seed_legacy_slack_setup(
        setup_service,
        binding_store,
        channel_route_store,
        legacy_setup,
    )
    .await
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
struct DynamicSlackPersonalDmTargetProvisioner {
    parts: Arc<SlackHostBetaRuntimeParts>,
    setup_service: Arc<SlackSetupService>,
    token_handle: EgressCredentialHandle,
    store: Arc<dyn SlackPersonalDmTargetStore>,
}

impl DynamicSlackPersonalDmTargetProvisioner {
    fn new(
        parts: Arc<SlackHostBetaRuntimeParts>,
        setup_service: Arc<SlackSetupService>,
        token_handle: EgressCredentialHandle,
        store: Arc<dyn SlackPersonalDmTargetStore>,
    ) -> Self {
        Self {
            parts,
            setup_service,
            token_handle,
            store,
        }
    }

    async fn configured_provisioner(
        &self,
    ) -> Result<SlackPersonalDmTargetProvisioner, SlackPersonalDmTargetError> {
        let setup = self
            .setup_service
            .current_setup()
            .await
            .map_err(|_| SlackPersonalDmTargetError::StoreUnavailable)?
            .ok_or(SlackPersonalDmTargetError::StoreUnavailable)?;
        let config = slack_host_beta_config_from_setup(&self.setup_service, setup)
            .await
            .map_err(|_| SlackPersonalDmTargetError::StoreUnavailable)?
            .map_err(|error| SlackPersonalDmTargetError::ProvisioningFailed(error.to_string()))?;
        let egress =
            slack_protocol_egress_from_parts(&self.parts, &config, self.token_handle.clone())
                .map_err(|error| {
                    SlackPersonalDmTargetError::ProvisioningFailed(error.to_string())
                })?;
        Ok(SlackPersonalDmTargetProvisioner::new(
            config.tenant_id,
            config.installation_id,
            config.team_id,
            egress,
            self.token_handle.clone(),
            Arc::clone(&self.store),
        ))
    }
}

impl std::fmt::Debug for DynamicSlackPersonalDmTargetProvisioner {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DynamicSlackPersonalDmTargetProvisioner")
            .field("tenant_id", &self.setup_service.tenant_id())
            .field("agent_id", &self.setup_service.agent_id())
            .field("project_id", &self.setup_service.project_id())
            .field("store", &self.store)
            .finish_non_exhaustive()
    }
}

#[async_trait::async_trait]
impl SlackPersonalDmTargetProvisioning for DynamicSlackPersonalDmTargetProvisioner {
    async fn provision_for_user(
        &self,
        user_id: ironclaw_host_api::UserId,
        slack_user_id: SlackUserId,
    ) -> Result<SlackPersonalDmTarget, SlackPersonalDmTargetError> {
        self.configured_provisioner()
            .await?
            .provision_for_user(user_id, slack_user_id)
            .await
    }
}

#[derive(Clone)]
struct DynamicSlackTriggeredRunDeliveryHook {
    parts: Arc<SlackHostBetaRuntimeParts>,
    setup_service: Arc<SlackSetupService>,
    delivery_store: Arc<dyn TriggeredRunDeliveryStore>,
    cached_driver: Arc<Mutex<Option<DynamicSlackTriggeredRunDeliveryDriver>>>,
}

#[derive(Clone)]
struct DynamicSlackTriggeredRunDeliveryDriver {
    revision: u64,
    driver: Arc<TriggeredRunDeliveryDriver>,
}

impl DynamicSlackTriggeredRunDeliveryHook {
    fn new(
        parts: Arc<SlackHostBetaRuntimeParts>,
        setup_service: Arc<SlackSetupService>,
        delivery_store: Arc<dyn TriggeredRunDeliveryStore>,
    ) -> Self {
        Self {
            parts,
            setup_service,
            delivery_store,
            cached_driver: Arc::new(Mutex::new(None)),
        }
    }

    async fn current_driver(&self) -> Result<Option<Arc<TriggeredRunDeliveryDriver>>, String> {
        let Some(setup) = self
            .setup_service
            .current_setup()
            .await
            .map_err(|error| error.to_string())?
        else {
            return Ok(None);
        };
        let revision = setup.revision;

        {
            let cached_driver = self.cached_driver.lock().await;
            if let Some(cached) = cached_driver
                .as_ref()
                .filter(|cached| cached.revision == revision)
            {
                return Ok(Some(Arc::clone(&cached.driver)));
            }
        }

        let config = slack_host_beta_config_from_setup(&self.setup_service, setup)
            .await
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())?;
        let driver = build_triggered_run_delivery_hook_from_parts(
            &self.parts,
            &config,
            Arc::clone(&self.delivery_store),
        )
        .map_err(|error| error.to_string())?;

        let mut cached_driver = self.cached_driver.lock().await;
        if let Some(cached) = cached_driver
            .as_ref()
            .filter(|cached| cached.revision >= revision)
        {
            return Ok(Some(Arc::clone(&cached.driver)));
        }
        *cached_driver = Some(DynamicSlackTriggeredRunDeliveryDriver {
            revision,
            driver: Arc::clone(&driver),
        });
        Ok(Some(driver))
    }
}

#[async_trait::async_trait]
impl PostSubmitDeliveryHook for DynamicSlackTriggeredRunDeliveryHook {
    async fn on_trigger_submitted(&self, fire: TriggerFire, run_id: TurnRunId, scope: TurnScope) {
        match self.current_driver().await {
            Ok(Some(driver)) => driver.on_trigger_submitted(fire, run_id, scope).await,
            Ok(None) => {
                tracing::debug!(
                    %run_id,
                    "Slack dynamic triggered-run delivery skipped: Slack setup is not configured"
                );
            }
            Err(error) => {
                tracing::warn!(
                    %run_id,
                    %error,
                    "Slack dynamic triggered-run delivery skipped: delivery hook unavailable"
                );
            }
        }
    }
}

#[derive(Clone)]
struct DynamicSlackInstallationResolver {
    parts: Arc<SlackHostBetaRuntimeParts>,
    setup_service: Arc<SlackSetupService>,
    state: Arc<dyn RebornUserIdentityLookup>,
    pairing: SlackPersonalBindingPairingService,
    channel_route_store: Arc<dyn SlackChannelRouteStore>,
    live_resolvers: Arc<Mutex<DynamicSlackInstallationResolverLifecycle>>,
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
            live_resolvers: Arc::new(Mutex::new(
                DynamicSlackInstallationResolverLifecycle::default(),
            )),
        }
    }

    async fn resolver(&self) -> Result<Arc<StaticSlackInstallationResolver>, SlackIngressError> {
        // Read setup before consulting the live resolver holder so WebUI changes
        // take effect on the next webhook. The holder below is for runner
        // lifecycle/drain ownership, not for hiding setup-store I/O.
        let setup = self
            .setup_service
            .current_setup()
            .await
            .map_err(map_setup_error_to_ingress_not_found("read Slack setup"))?
            .ok_or(SlackIngressError::InstallationNotFound)?;
        let revision = setup.revision;
        if let Some(resolver) = self.live_resolver_for_revision(revision).await {
            return Ok(resolver);
        }

        let resolver = Arc::new(self.build_resolver(setup).await?);
        let mut live_resolvers = self.live_resolvers.lock().await;
        if let Some(current) = &live_resolvers.current
            && current.revision == revision
        {
            return Ok(Arc::clone(&current.resolver));
        }
        if let Some(previous) = live_resolvers.current.replace(DynamicLiveSlackResolver {
            revision,
            resolver: Arc::clone(&resolver),
        }) {
            live_resolvers.retired.push(previous.resolver);
        }
        Ok(resolver)
    }

    async fn live_resolver_for_revision(
        &self,
        revision: u64,
    ) -> Option<Arc<StaticSlackInstallationResolver>> {
        let live_resolvers = self.live_resolvers.lock().await;
        live_resolvers
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
            .map_err(map_setup_error_to_ingress_not_found(
                "resolve Slack setup secrets",
            ))?
            .map_err(map_build_error_to_ingress_not_found(
                "build Slack setup config",
            ))?;
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
        .map_err(map_build_error_to_ingress_not_found(
            "build Slack installation resolver",
        ))?;
        Ok(StaticSlackInstallationResolver::new([record]))
    }

    async fn drain_live_resolvers(&self) {
        let resolvers = {
            let live_resolvers = self.live_resolvers.lock().await;
            live_resolvers.resolvers()
        };
        for resolver in &resolvers {
            resolver.drain_installations().await;
        }
        let mut live_resolvers = self.live_resolvers.lock().await;
        live_resolvers.forget_retired(&resolvers);
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

    fn resolve_command_ingress<'a>(
        &'a self,
        headers: &'a axum::http::HeaderMap,
        body: &'a [u8],
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<ResolvedSlackCommand, SlackIngressError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            self.resolver()
                .await?
                .resolve_command_ingress(headers, body)
                .await
        })
    }

    fn drain_installations<'a>(
        &'a self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
        Box::pin(async move { self.drain_live_resolvers().await })
    }
}

#[derive(Default)]
struct DynamicSlackInstallationResolverLifecycle {
    current: Option<DynamicLiveSlackResolver>,
    retired: Vec<Arc<StaticSlackInstallationResolver>>,
}

impl DynamicSlackInstallationResolverLifecycle {
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

struct DynamicLiveSlackResolver {
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

    async fn reissue_challenge(
        &self,
        challenge: SlackPersonalBindingPairingChallenge,
    ) -> Result<IssuedSlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError> {
        // `/pair` force-mint must reach the inner store's `reissue_challenge`.
        // The trait default delegates to `issue_challenge`, which reuses an
        // active code — that would silently defeat the force-fresh semantic for
        // the production serve path. Bind to the current setup revision exactly
        // like `issue_challenge`, then force-mint through the inner store.
        let challenge = self.bind_to_current_setup(challenge).await?;
        self.store.reissue_challenge(challenge).await
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
        let Some(setup) = self.setup_service.current_setup().await.map_err(
            map_setup_error_to_dynamic_target_unavailable("read Slack setup for outbound targets"),
        )?
        else {
            return Ok(None);
        };
        let installation_id =
            setup
                .installation_id()
                .map_err(map_setup_error_to_dynamic_target_unavailable(
                    "parse Slack setup installation id for outbound targets",
                ))?;
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

fn map_setup_error_to_ingress_not_found(
    context: &'static str,
) -> impl FnOnce(crate::slack_setup::SlackSetupError) -> SlackIngressError {
    move |error| {
        tracing::debug!(%error, context, "Slack setup unavailable for dynamic ingress");
        SlackIngressError::InstallationNotFound
    }
}

fn map_build_error_to_ingress_not_found(
    context: &'static str,
) -> impl FnOnce(SlackHostBetaBuildError) -> SlackIngressError {
    move |error| {
        tracing::debug!(%error, context, "Slack setup config unavailable for dynamic ingress");
        SlackIngressError::InstallationNotFound
    }
}

fn map_setup_error_to_dynamic_target_unavailable(
    context: &'static str,
) -> impl FnOnce(crate::slack_setup::SlackSetupError) -> RebornServicesError {
    move |error| {
        tracing::debug!(
            %error,
            context,
            "Slack setup unavailable for dynamic outbound targets"
        );
        slack_dynamic_target_unavailable()
    }
}

fn map_setup_error_to_egress(
    error: crate::slack_setup::SlackSetupError,
) -> ProtocolHttpEgressError {
    tracing::debug!(%error, "Slack setup unavailable for dynamic Slack egress");
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
    async fn dynamic_pairing_challenge_store_reissue_forwards_to_inner_force_mint() {
        // The `/pair` force-mint must reach the inner store's `reissue_challenge`.
        // Without the wrapper override it would inherit the trait default, which
        // delegates to `issue_challenge` (code reuse) — silently defeating the
        // force-fresh semantic on the production serve path. Drive the wrapper
        // (the caller) and assert the inner force-mint path actually ran.
        let setup_service = Arc::new(SlackSetupService::new(
            TenantId::new("tenant:slack").unwrap(),
            AgentId::new("agent:slack").unwrap(),
            None,
            UserId::new("user:operator").unwrap(),
            Arc::new(InMemorySetupStore::new(setup_record(1))),
            Arc::new(InMemorySecretStore::default()),
        ));
        let inner = Arc::new(StaticChallengeStore::default());
        let store = DynamicSlackPairingChallengeStore::new(setup_service, inner.clone());
        let challenge = SlackPersonalBindingPairingChallenge {
            installation_id: AdapterInstallationId::new("install-a").unwrap(),
            slack_user_id: SlackUserId::new("U123"),
            setup_revision: None,
        };

        let issued = store
            .reissue_challenge(challenge)
            .await
            .expect("reissue mints via inner force-mint");

        // Inner force-mint ran exactly once (not the reuse path).
        assert_eq!(*inner.reissued.lock().unwrap(), 1);
        // The distinct force-mint code came back, not the `issue_challenge` reuse code.
        assert_eq!(issued.code.as_str(), "RAW99999");
        // `bind_to_current_setup` still ran: the challenge carries the live revision.
        assert_eq!(issued.challenge.setup_revision, Some(1));
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

    #[tokio::test]
    async fn seed_legacy_slack_setup_if_missing_preserves_runtime_setup() {
        let existing_setup = setup_record(7);
        let setup_store = Arc::new(InMemorySetupStore::new(existing_setup.clone()));
        let setup_service = Arc::new(SlackSetupService::new(
            TenantId::new("tenant:slack").unwrap(),
            AgentId::new("agent:slack").unwrap(),
            None,
            UserId::new("user:operator").unwrap(),
            setup_store,
            Arc::new(InMemorySecretStore::default()),
        ));
        let binding_store = Arc::new(RecordingBindingStore::default());
        let route_store = Arc::new(RecordingRouteStore::default());

        seed_legacy_slack_setup_if_missing(
            &setup_service,
            binding_store.clone(),
            route_store.clone(),
            SlackHostBetaLegacySetup {
                installation_id: "install-legacy".to_string(),
                team_id: "TLEGACY".to_string(),
                api_app_id: "ALEGACY".to_string(),
                slack_user_id: Some("ULEGACY".to_string()),
                user_id: UserId::new("user:legacy").unwrap(),
                shared_subject_user_id: Some(UserId::new("user:legacy-shared").unwrap()),
                channel_routes: vec![SlackHostBetaChannelRoute::new(
                    "CLEGACY",
                    UserId::new("user:legacy-agent").unwrap(),
                )],
                signing_secret: SecretString::from("legacy-signing-secret"),
                bot_token: SecretString::from("xoxb-legacy"),
            },
        )
        .await
        .expect("existing setup skips legacy seed");

        assert_eq!(
            setup_service
                .current_setup()
                .await
                .expect("setup read")
                .expect("setup remains"),
            existing_setup
        );
        assert!(binding_store.bindings.lock().unwrap().is_empty());
        assert!(route_store.routes.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn dynamic_personal_connection_scope_resolver_reads_current_setup() {
        let setup_service = Arc::new(SlackSetupService::new(
            TenantId::new("tenant:slack").unwrap(),
            AgentId::new("agent:slack").unwrap(),
            None,
            UserId::new("user:operator").unwrap(),
            Arc::new(InMemorySetupStore::new(setup_record(7))),
            Arc::new(InMemorySecretStore::default()),
        ));
        let resolver = DynamicSlackPersonalConnectionScopeResolver { setup_service };

        let scope = resolver
            .resolve_personal_connection_scope()
            .await
            .expect("scope resolves")
            .expect("setup exists");

        assert_eq!(scope.installation_id.as_str(), "install-a");
        assert_eq!(scope.team_id.as_str(), "T123");
    }

    #[tokio::test]
    async fn dynamic_personal_connection_scope_resolver_returns_none_without_setup() {
        let setup_service = Arc::new(SlackSetupService::new(
            TenantId::new("tenant:slack").unwrap(),
            AgentId::new("agent:slack").unwrap(),
            None,
            UserId::new("user:operator").unwrap(),
            Arc::new(InMemorySetupStore::empty()),
            Arc::new(InMemorySecretStore::default()),
        ));
        let resolver = DynamicSlackPersonalConnectionScopeResolver { setup_service };

        assert!(
            resolver
                .resolve_personal_connection_scope()
                .await
                .expect("scope resolves")
                .is_none()
        );
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

        async fn delete_slack_installation_setup(
            &self,
        ) -> Result<(), crate::slack_setup::SlackSetupError> {
            *self.setup.write().await = None;
            Ok(())
        }
    }

    #[derive(Debug, Default)]
    struct StaticChallengeStore {
        challenge: StdMutex<Option<SlackPersonalBindingPairingChallenge>>,
        reissued: StdMutex<usize>,
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

        async fn reissue_challenge(
            &self,
            challenge: SlackPersonalBindingPairingChallenge,
        ) -> Result<IssuedSlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError>
        {
            // Distinct code + counter so a caller test can prove the dynamic
            // wrapper forwarded to this force-mint path rather than falling
            // through to `issue_challenge` (which would reuse "ABC12345").
            *self.reissued.lock().unwrap() += 1;
            *self.challenge.lock().unwrap() = Some(challenge.clone());
            Ok(IssuedSlackPersonalBindingPairingChallenge {
                code: SlackPersonalBindingPairingCode::new("RAW99999").unwrap(),
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
