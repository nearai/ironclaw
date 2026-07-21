//! Composition adapter for the Telegram-owned host builder.

use std::num::NonZeroUsize;
use std::sync::Arc;

use ironclaw_conversations::RebornFilesystemConversationServices;
use ironclaw_host_api::ExtensionId;
use ironclaw_product_workflow::{
    IdempotencyLedger, LifecyclePackageKind, LifecyclePackageRef, LifecyclePhase,
    RebornFilesystemIdempotencyLedger,
};

use crate::RebornRuntime;
use crate::extension_host::extension_lifecycle::{
    ExtensionActivationMode, RebornLocalExtensionManagementPort,
};
use crate::outbound::OutboundDeliveryTargetRegistrationOutcome;
use crate::webui::route_mounts::{ProtectedRouteMount, PublicRouteDrain, PublicRouteMount};
use ironclaw_telegram_extension::channel_routes::{
    TelegramChannelSetupActivation, TelegramChannelSetupActivationError,
    telegram_channel_route_parts,
};
use ironclaw_telegram_extension::host::{
    TelegramDeliveryServicePorts, TelegramHostInput, TelegramHostParts, build_telegram_host,
    telegram_host_scope_template, telegram_outbound_delivery_target_provider_key,
};
use ironclaw_telegram_extension::ingress::{
    TelegramUpdatesRouteState, telegram_updates_route_parts,
};
use ironclaw_telegram_extension::state::FilesystemTelegramHostState;

pub use ironclaw_telegram_extension::TelegramHostBuildError;
pub use ironclaw_telegram_extension::host::TelegramHostConfig as TelegramHostRuntimeConfig;

const TELEGRAM_IDEMPOTENCY_LEDGER_SETTLED_LIMIT: usize = 10_000;
const TELEGRAM_IDEMPOTENCY_LEDGER_PRUNE_INTERVAL: usize = 1_000;

/// Route mounts and WebUI facades produced by one Telegram host assembly.
#[non_exhaustive]
pub struct TelegramHostMounts {
    pub events: PublicRouteMount,
    protected: ProtectedRouteMount,
    connectable: Arc<dyn ironclaw_product_workflow::ConnectableChannelsProductFacade>,
    channel_connection: Arc<dyn ironclaw_product_workflow::ChannelConnectionFacade>,
}

impl TelegramHostMounts {
    pub fn protected_routes(&self) -> ProtectedRouteMount {
        self.protected.clone()
    }

    pub(crate) fn connectable_channels(
        &self,
    ) -> Arc<dyn ironclaw_product_workflow::ConnectableChannelsProductFacade> {
        Arc::clone(&self.connectable)
    }

    pub(crate) fn channel_connection(
        &self,
    ) -> Arc<dyn ironclaw_product_workflow::ChannelConnectionFacade> {
        Arc::clone(&self.channel_connection)
    }
}

struct TelegramUpdatesRouteDrain(TelegramUpdatesRouteState);

impl PublicRouteDrain for TelegramUpdatesRouteDrain {
    fn drain<'a>(&'a self) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
        Box::pin(self.0.drain_immediate_ack_tasks())
    }
}

struct DynamicTelegramChannelSetupActivation {
    extension_management: Arc<RebornLocalExtensionManagementPort>,
}

#[async_trait::async_trait]
impl TelegramChannelSetupActivation for DynamicTelegramChannelSetupActivation {
    async fn activate_telegram_channel_after_setup_save(
        &self,
    ) -> Result<(), TelegramChannelSetupActivationError> {
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "telegram")
            .map_err(telegram_setup_activation_error)?;
        let caller = self.extension_management.tenant_operator_user_id().clone();
        let projection = self
            .extension_management
            .project(package_ref.clone(), &caller)
            .await
            .map_err(telegram_setup_activation_error)?;
        if projection.phase == LifecyclePhase::Discovered {
            return Ok(());
        }
        self.extension_management
            .activate(package_ref, ExtensionActivationMode::Static, &caller)
            .await
            .map_err(telegram_setup_activation_error)?;
        Ok(())
    }
}

fn telegram_setup_activation_error(
    error: impl std::fmt::Display,
) -> TelegramChannelSetupActivationError {
    TelegramChannelSetupActivationError::new(error.to_string())
}

/// Extract runtime-owned ports, delegate Telegram behavior to the extension
/// crate, then mount and register the returned parts.
pub async fn build_telegram_host_runtime_mounts(
    runtime: &RebornRuntime,
    config: TelegramHostRuntimeConfig,
) -> Result<TelegramHostMounts, TelegramHostBuildError> {
    let local_runtime = runtime
        .services()
        .local_runtime
        .as_ref()
        .ok_or(TelegramHostBuildError::DurableHostStateUnavailable)?;
    let host_state_filesystem = Arc::clone(&local_runtime.telegram_host_state_filesystem);
    let state = Arc::new(FilesystemTelegramHostState::new(
        Arc::clone(&host_state_filesystem),
        config.tenant_id.clone(),
        config.operator_user_id.clone(),
        config.agent_id.clone(),
        config.project_id.clone(),
    ));
    let host_egress = local_runtime
        .host_runtime_http_egress
        .clone()
        .ok_or(TelegramHostBuildError::RuntimeHttpEgressUnavailable)?;
    let continuation = runtime
        .services()
        .product_auth
        .as_ref()
        .ok_or(TelegramHostBuildError::ProductAuthUnavailable)?
        .continuation_dispatcher();
    let conversation_services = Arc::new(
        RebornFilesystemConversationServices::new(Arc::clone(&host_state_filesystem))
            .await
            .map_err(
                |error| TelegramHostBuildError::ConversationStoreUnavailable {
                    reason: error.to_string(),
                },
            )?,
    );
    let idempotency_ledger: Arc<dyn IdempotencyLedger> = Arc::new(
        RebornFilesystemIdempotencyLedger::new(
            host_state_filesystem,
            telegram_host_scope_template(&config),
        )
        .with_settled_entry_limit(nonzero_idempotency_value(
            TELEGRAM_IDEMPOTENCY_LEDGER_SETTLED_LIMIT,
            "settled_entry_limit",
        )?)
        .with_settled_prune_interval(nonzero_idempotency_value(
            TELEGRAM_IDEMPOTENCY_LEDGER_PRUNE_INTERVAL,
            "settled_prune_interval",
        )?),
    );
    let approval_interactions = Arc::new(
        crate::delivered_gate_routing::DeliveredGateRoutingApprovalService::new(
            runtime.webui_approval_interaction_service(),
            Arc::clone(&local_runtime.delivered_gate_routes),
        ),
    );
    let setup_activation = local_runtime
        .extension_management
        .as_ref()
        .map(|management| {
            Arc::new(DynamicTelegramChannelSetupActivation {
                extension_management: Arc::clone(management),
            }) as Arc<dyn TelegramChannelSetupActivation>
        });
    let provider_key = telegram_outbound_delivery_target_provider_key(&config);
    let TelegramHostParts {
        updates,
        channel_routes,
        connectable,
        channel_connection,
        outbound_targets,
        trigger_hook,
        account_status,
    } = build_telegram_host(TelegramHostInput {
        config,
        state,
        secret_store: runtime.services().secret_store(),
        host_egress,
        continuation,
        conversation_bindings: conversation_services.clone(),
        actor_pairings: conversation_services,
        idempotency_ledger,
        thread_service: runtime.webui_thread_service(),
        turn_coordinator: runtime.webui_turn_coordinator(),
        approval_interactions,
        auth_interactions: runtime.webui_auth_interaction_service(),
        delivery_services: TelegramDeliveryServicePorts {
            outbound_store: Arc::clone(&local_runtime.outbound_state),
            delivered_gate_routes: Arc::clone(&local_runtime.delivered_gate_routes),
            communication_preferences: Arc::clone(&local_runtime.outbound_preferences),
            triggered_run_delivery: Arc::clone(&local_runtime.triggered_run_delivery),
            approval_requests: local_runtime.approval_requests.clone(),
            auth_challenges: runtime.auth_challenge_provider(),
            auth_flow_canceller: runtime.blocked_auth_flow_canceller(),
        },
        setup_activation,
    })
    .await?;

    let (protected_router, protected_descriptors) = telegram_channel_route_parts(channel_routes)
        .map_err(|error| invalid_config("channel_routes", error.to_string()))?;
    let protected = ProtectedRouteMount::new(protected_router, protected_descriptors);
    let (updates_router, updates_descriptors) = telegram_updates_route_parts(updates.clone());
    let events = PublicRouteMount::new(updates_router, updates_descriptors)
        .with_drain(Arc::new(TelegramUpdatesRouteDrain(updates)));
    let provider_already_registered = runtime
        .outbound_delivery_target_provider_key_registered(&provider_key)
        .map_err(outbound_registration_error)?;
    if !provider_already_registered {
        match runtime
            .register_outbound_delivery_target_provider(provider_key, outbound_targets)
            .map_err(outbound_registration_error)?
        {
            OutboundDeliveryTargetRegistrationOutcome::Registered => {}
            OutboundDeliveryTargetRegistrationOutcome::Replaced => {
                return Err(outbound_registration_error(
                    "Telegram outbound delivery target provider was concurrently registered",
                ));
            }
        }
    }
    let hook_added = runtime.add_trigger_post_submit_hook(
        crate::runtime::TELEGRAM_TRIGGER_POST_SUBMIT_HOOK_KEY,
        trigger_hook,
    );
    if !hook_added && runtime.trigger_post_submit_hook_is_set() && !provider_already_registered {
        return Err(outbound_registration_error(
            "Telegram triggered-run delivery hook is already wired for a different Telegram host config",
        ));
    }
    connect_account_status(local_runtime, account_status)?;

    Ok(TelegramHostMounts {
        events,
        protected,
        connectable,
        channel_connection,
    })
}

fn connect_account_status(
    local_runtime: &crate::factory::RebornRuntimeSubstrate,
    account_status: Arc<dyn ironclaw_product_workflow::AccountConnectionStatusSource>,
) -> Result<(), TelegramHostBuildError> {
    let Some(extension_management) = &local_runtime.extension_management else {
        return Ok(());
    };
    let extension_id = ExtensionId::new(ironclaw_telegram_extension::TELEGRAM_EXTENSION_ID)
        .map_err(|error| invalid_config("account_setup", error.to_string()))?;
    let account_setups = extension_management.account_setup_registry();
    if account_setups.descriptor(&extension_id).is_none() {
        return Err(invalid_config(
            "account_setup",
            "Telegram account setup was not declared".to_string(),
        ));
    }
    let _ = account_setups.connect(&extension_id, account_status);
    Ok(())
}

fn nonzero_idempotency_value(
    value: usize,
    field: &'static str,
) -> Result<NonZeroUsize, TelegramHostBuildError> {
    NonZeroUsize::new(value).ok_or_else(|| invalid_config(field, "must be non-zero".to_string()))
}

fn outbound_registration_error(error: impl std::fmt::Display) -> TelegramHostBuildError {
    TelegramHostBuildError::OutboundDeliveryTargetRegistration {
        reason: error.to_string(),
    }
}

fn invalid_config(field: &'static str, reason: String) -> TelegramHostBuildError {
    TelegramHostBuildError::InvalidConfig { field, reason }
}

#[cfg(test)]
#[path = "telegram_host_beta_tests.rs"]
mod tests;
