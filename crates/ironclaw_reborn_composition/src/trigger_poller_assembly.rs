use std::sync::{Arc, OnceLock};

use ironclaw_filesystem::{CompositeRootFilesystem, ScopedFilesystem};
use ironclaw_host_api::ids::{AgentId, ProjectId, TenantId, UserId};
use ironclaw_threads::SessionThreadService;
use ironclaw_turns::TurnCoordinator;

#[cfg(any(test, feature = "test-support"))]
use crate::automation::trigger_poller::TenantScopedTrustedTriggerFireAuthorizer;
use crate::automation::trigger_poller::{
    AccessCheckerTriggerFireAuthorizer, ConversationContentRefMaterializer, ProcessActiveRunLookup,
};
use crate::factory::filesystem_reborn_identity_store;
use crate::runtime::RebornRuntimeError;
use crate::runtime_input::{
    TriggerFireAccessChecker, TriggerPollerAuthorizerConfig, TriggerPollerSettings,
};
use ironclaw_processes::ProcessLifecycleLookupSource;
use ironclaw_turns::TurnError;

pub(crate) struct TriggerPollerServices {
    pub(crate) materializer: Arc<dyn ironclaw_triggers::TriggerPromptMaterializer>,
    pub(crate) trusted_submitter: Arc<dyn ironclaw_triggers::TrustedTriggerFireSubmitter>,
    pub(crate) post_submit_hook_slot:
        Arc<OnceLock<Arc<dyn crate::automation::trigger_poller::PostSubmitDeliveryHook>>>,
    pub(crate) pairing_service: Arc<dyn ironclaw_conversations::ConversationActorPairingService>,
}

pub(crate) fn build_trigger_poller_services<C>(
    conversation_services: C,
    turn_coordinator: Arc<dyn TurnCoordinator>,
    thread_service: Arc<dyn SessionThreadService>,
    authorizer_config: TriggerPollerAuthorizerConfig,
    access_checker: Option<Arc<dyn TriggerFireAccessChecker>>,
    tenant_id: TenantId,
    default_agent_id: AgentId,
) -> Result<TriggerPollerServices, RebornRuntimeError>
where
    C: ironclaw_conversations::ConversationBindingService
        + ironclaw_conversations::InboundConversationService
        + ironclaw_conversations::ConversationActorPairingService
        + Clone
        + 'static,
{
    let authorizer = build_trigger_fire_authorizer(authorizer_config, access_checker, tenant_id)?;
    let pairing_service: Arc<dyn ironclaw_conversations::ConversationActorPairingService> =
        Arc::new(conversation_services.clone());
    let materializer = Arc::new(ConversationContentRefMaterializer::new(
        conversation_services.clone(),
        Arc::clone(&thread_service),
        default_agent_id,
        authorizer,
    ));
    let trusted_submitter = ironclaw_conversations::trusted_trigger_fire_submitter(
        conversation_services.clone(),
        conversation_services,
        turn_coordinator,
    );
    let services = TriggerPollerServices {
        materializer,
        trusted_submitter,
        post_submit_hook_slot: Arc::new(OnceLock::new()),
        pairing_service,
    };
    #[cfg(not(any(test, feature = "test-support")))]
    let _ = &services.pairing_service;
    Ok(services)
}

fn trigger_poller_authorization_required_error() -> RebornRuntimeError {
    RebornRuntimeError::InvalidArgument {
        reason: "trigger poller cannot be enabled without a fire-time creator access checker"
            .to_string(),
    }
}

pub(crate) fn validate_trigger_poller_authorization(
    trigger_poller: &TriggerPollerSettings,
    access_checker: Option<&Arc<dyn TriggerFireAccessChecker>>,
) -> Result<(), RebornRuntimeError> {
    debug_assert!(trigger_poller.enabled);
    match trigger_poller.authorizer {
        #[cfg(any(test, feature = "test-support"))]
        TriggerPollerAuthorizerConfig::TenantScopedPlaceholderForTest => Ok(()),
        TriggerPollerAuthorizerConfig::CreatorAccessRequired => access_checker
            .map(|_| ())
            .ok_or_else(trigger_poller_authorization_required_error),
    }
}

fn build_trigger_fire_authorizer(
    authorizer_config: TriggerPollerAuthorizerConfig,
    access_checker: Option<Arc<dyn TriggerFireAccessChecker>>,
    tenant_id: TenantId,
) -> Result<
    Arc<dyn crate::automation::trigger_poller_trusted_submit::TriggerFireAuthorizer>,
    RebornRuntimeError,
> {
    #[cfg(not(any(test, feature = "test-support")))]
    let _ = tenant_id;
    match authorizer_config {
        #[cfg(any(test, feature = "test-support"))]
        TriggerPollerAuthorizerConfig::TenantScopedPlaceholderForTest => Ok(Arc::new(
            TenantScopedTrustedTriggerFireAuthorizer::new(tenant_id),
        )),
        TriggerPollerAuthorizerConfig::CreatorAccessRequired => access_checker
            .map(|checker| {
                Arc::new(AccessCheckerTriggerFireAuthorizer::new(checker))
                    as Arc<
                        dyn crate::automation::trigger_poller_trusted_submit::TriggerFireAuthorizer,
                    >
            })
            .ok_or_else(trigger_poller_authorization_required_error),
    }
}

pub(crate) fn build_trigger_active_run_lookup(
    lifecycle_source: Arc<dyn ProcessLifecycleLookupSource<Error = TurnError>>,
) -> Arc<dyn ironclaw_triggers::TriggerActiveRunLookup> {
    Arc::new(ProcessActiveRunLookup::new(lifecycle_source))
}

pub(crate) fn poller_user_directory(
    scoped_filesystem: Arc<ScopedFilesystem<CompositeRootFilesystem>>,
    tenant_id: &TenantId,
    actor_user_id: &UserId,
    agent_id: &AgentId,
    project_id: Option<&ProjectId>,
) -> Arc<dyn ironclaw_reborn_identity::RebornUserDirectory> {
    filesystem_reborn_identity_store(
        scoped_filesystem,
        tenant_id.clone(),
        actor_user_id.clone(),
        agent_id.clone(),
        project_id.cloned(),
    )
}
