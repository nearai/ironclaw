//! Construction of Telegram-owned host parts from explicit ports.

use std::sync::Arc;

use ironclaw_channel_delivery::PostSubmitDeliveryHook;
use ironclaw_channel_host::identity::RebornUserIdentityLookup;
use ironclaw_channel_host::outbound_targets::OutboundDeliveryTargetProvider;
use ironclaw_host_api::{ProjectId, ResourceScope};
use ironclaw_product_adapters::ProtocolHttpEgress;
use ironclaw_product_workflow::{
    AccountConnectionStatusSource, ChannelConnectionFacade, ConnectableChannelsProductFacade,
    ProductActorUserResolver,
};
use ironclaw_safety::{SafetyConfig, SafetyLayer};
use ironclaw_wasm_product_adapters::EgressPolicy;
use sha2::{Digest, Sha256};

use crate::TelegramHostBuildError;
use crate::bot_api::HostEgressTelegramBotApi;
use crate::channel_routes::TelegramChannelRouteConfig;
use crate::delivery::{DynamicTelegramTriggeredRunDeliveryHook, TelegramOutboundTargetProvider};
use crate::egress::TelegramProtocolHttpEgress;
use crate::ingress::{DynamicTelegramInstallationResolver, TelegramUpdatesRouteState};
use crate::pairing::TelegramPairingService;
use crate::setup::TelegramSetupService;
use crate::telegram_actor_identity::TelegramUserIdentityActorResolver;
use crate::telegram_adapter::{telegram_bot_token_handle, telegram_declared_egress_targets};
use crate::telegram_connectable_channel::{
    TelegramChannelConnectionFacade, TelegramConnectableChannelsProductFacade,
};

use super::{
    TelegramHostConfig, TelegramHostInput, TelegramHostParts, TelegramRevisionWorkflowParts,
};

const TELEGRAM_OUTBOUND_PROVIDER_KEY_PREFIX: &str = "telegram-host-runtime-setup";

pub async fn build_telegram_host(
    input: TelegramHostInput,
) -> Result<TelegramHostParts, TelegramHostBuildError> {
    let TelegramHostInput {
        config,
        state,
        secret_store,
        host_egress,
        continuation,
        conversation_bindings,
        actor_pairings,
        idempotency_ledger,
        thread_service,
        turn_coordinator,
        approval_interactions,
        auth_interactions,
        delivery_services,
        setup_activation,
    } = input;
    let identity_lookup: Arc<dyn RebornUserIdentityLookup> = state.clone();
    let bot_api =
        HostEgressTelegramBotApi::arced(host_egress.clone(), telegram_host_scope_template(&config));
    let setup_service = Arc::new(TelegramSetupService::new(
        config.tenant_id.clone(),
        config.agent_id.clone(),
        config.project_id.clone(),
        config.operator_user_id.clone(),
        Arc::clone(&state),
        secret_store,
        bot_api,
        config.public_base_url.clone(),
    ));
    let pairing = Arc::new(TelegramPairingService::new(
        config.tenant_id.clone(),
        config.agent_id.clone(),
        config.project_id.clone(),
        Arc::clone(&setup_service),
        Arc::clone(&state),
        continuation,
        Arc::clone(&actor_pairings),
    ));
    let actor_user_resolver: Arc<dyn ProductActorUserResolver> = Arc::new(
        TelegramUserIdentityActorResolver::new(Arc::clone(&identity_lookup)),
    );
    let token_handle =
        telegram_bot_token_handle().map_err(|error| invalid_config(error.field, error.reason))?;
    let egress: Arc<dyn ProtocolHttpEgress> = Arc::new(TelegramProtocolHttpEgress::new(
        host_egress,
        Arc::clone(&setup_service),
        EgressPolicy::new(telegram_declared_egress_targets(token_handle.clone())),
        telegram_host_scope_template(&config),
    ));
    let triggered_run_delivery = Arc::clone(&delivery_services.triggered_run_delivery);
    let revision_parts = Arc::new(TelegramRevisionWorkflowParts::new(
        config.clone(),
        conversation_bindings,
        actor_pairings,
        actor_user_resolver,
        idempotency_ledger,
        thread_service,
        turn_coordinator,
        approval_interactions,
        auth_interactions,
        delivery_services,
        egress,
        token_handle,
    ));
    let resolver = Arc::new(DynamicTelegramInstallationResolver::new(
        Arc::clone(&setup_service),
        Arc::clone(&pairing),
        identity_lookup,
        Arc::clone(&revision_parts) as Arc<dyn crate::ingress::TelegramRevisionWorkflowBuilder>,
    ));
    let updates = TelegramUpdatesRouteState::from_resolver(resolver);
    let outbound_targets: Arc<dyn OutboundDeliveryTargetProvider> =
        Arc::new(TelegramOutboundTargetProvider::new(
            config.tenant_id.clone(),
            Arc::clone(&setup_service),
            state,
        ));
    let trigger_hook: Arc<dyn PostSubmitDeliveryHook> =
        Arc::new(DynamicTelegramTriggeredRunDeliveryHook::new(
            revision_parts,
            Arc::clone(&setup_service),
            triggered_run_delivery,
            Arc::clone(&outbound_targets),
        ));
    let mut channel_routes = TelegramChannelRouteConfig::new(
        Arc::clone(&setup_service),
        Arc::clone(&pairing),
        Arc::new(SafetyLayer::new(&SafetyConfig {
            max_output_length: 16 * 1024,
            injection_check_enabled: true,
        })),
    );
    if let Some(setup_activation) = setup_activation {
        channel_routes = channel_routes.with_setup_activation(setup_activation);
    }
    let connectable: Arc<dyn ConnectableChannelsProductFacade> = Arc::new(
        TelegramConnectableChannelsProductFacade::new(Arc::clone(&setup_service), true),
    );
    let channel_connection: Arc<dyn ChannelConnectionFacade> = Arc::new(
        TelegramChannelConnectionFacade::new(Arc::clone(&pairing), setup_service),
    );
    let account_status: Arc<dyn AccountConnectionStatusSource> = pairing;

    Ok(TelegramHostParts {
        updates,
        channel_routes,
        connectable,
        channel_connection,
        outbound_targets,
        trigger_hook,
        account_status,
    })
}

/// Deterministic per-host-config registry key for the outbound target
/// provider. Length-prefixing prevents tuple-field ambiguity.
pub fn telegram_outbound_delivery_target_provider_key(config: &TelegramHostConfig) -> String {
    let mut hasher = Sha256::new();
    hash_provider_key_field(&mut hasher, config.tenant_id.as_str());
    hash_provider_key_field(&mut hasher, config.agent_id.as_str());
    hash_provider_key_field(
        &mut hasher,
        config.project_id.as_ref().map_or("", ProjectId::as_str),
    );
    hash_provider_key_field(&mut hasher, config.operator_user_id.as_str());

    let digest = hasher.finalize();
    let suffix = digest
        .into_iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("{TELEGRAM_OUTBOUND_PROVIDER_KEY_PREFIX}:{suffix}")
}

fn hash_provider_key_field(hasher: &mut Sha256, value: &str) {
    hasher.update(value.len().to_be_bytes());
    hasher.update(value.as_bytes());
}

pub fn telegram_host_scope_template(config: &TelegramHostConfig) -> ResourceScope {
    ResourceScope {
        tenant_id: config.tenant_id.clone(),
        user_id: config.operator_user_id.clone(),
        agent_id: Some(config.agent_id.clone()),
        project_id: config.project_id.clone(),
        mission_id: None,
        thread_id: None,
        invocation_id: ironclaw_host_api::InvocationId::new(),
    }
}

fn invalid_config(field: &'static str, reason: String) -> TelegramHostBuildError {
    TelegramHostBuildError::InvalidConfig { field, reason }
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::{AgentId, TenantId, UserId};

    use super::*;

    fn config(project_id: Option<ProjectId>) -> TelegramHostConfig {
        TelegramHostConfig::new(
            TenantId::new("tenant-a").expect("tenant"),
            AgentId::new("agent-a").expect("agent"),
            project_id,
            UserId::new("operator-a").expect("operator"),
            Some("https://ironclaw.example".to_string()),
        )
    }

    #[test]
    fn outbound_provider_key_is_stable_and_scope_sensitive() {
        let first = telegram_outbound_delivery_target_provider_key(&config(None));
        let repeated = telegram_outbound_delivery_target_provider_key(&config(None));
        let project = telegram_outbound_delivery_target_provider_key(&config(Some(
            ProjectId::new("project-a").expect("project"),
        )));

        assert_eq!(first, repeated);
        assert_ne!(first, project);
        assert!(first.starts_with("telegram-host-runtime-setup:"));
    }
}
