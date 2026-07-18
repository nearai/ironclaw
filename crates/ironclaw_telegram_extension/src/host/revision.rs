//! Per-setup-revision workflow and delivery-observer construction.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_channel_delivery::{
    FinalReplyDeliveryObserver, FinalReplyDeliveryServices, FinalReplyDeliverySettings,
};
use ironclaw_conversations::{ConversationActorPairingService, ConversationBindingService};
use ironclaw_outbound::DeliveredGateRouteStore;
use ironclaw_product_adapters::{
    DeliveryStatus, EgressCredentialHandle, OutboundDeliverySink, ProductAdapter, ProductWorkflow,
    ProtocolHttpEgress,
};
use ironclaw_product_workflow::{
    ApprovalInteractionService, AuthInteractionService,
    ConversationBindingService as ProductBindingService, DefaultInboundTurnService,
    DefaultProductWorkflow, IdempotencyLedger, ProductActorUserResolver,
    ProductConversationBindingService, ProductInstallationKey, ProductInstallationScope,
    StaticProductInstallationResolver,
};
use ironclaw_threads::SessionThreadService;
use ironclaw_turns::TurnCoordinator;

use crate::TelegramHostBuildError;
use crate::delivery::TelegramDeliveryProtocol;
use crate::ingress::{
    TelegramRevisionWorkflow, TelegramRevisionWorkflowBuildError, TelegramRevisionWorkflowBuilder,
};
use crate::setup::TelegramInstallationSetup;
use crate::telegram_actor_identity::TELEGRAM_V2_ADAPTER_ID;
use crate::telegram_adapter::telegram_adapter_for_setup;

use super::{TelegramDeliveryServicePorts, TelegramHostConfig};

/// Revision-independent ports reused by inbound and triggered delivery.
pub(crate) struct TelegramRevisionWorkflowParts {
    config: TelegramHostConfig,
    conversation_bindings: Arc<dyn ConversationBindingService>,
    actor_pairings: Arc<dyn ConversationActorPairingService>,
    actor_user_resolver: Arc<dyn ProductActorUserResolver>,
    idempotency_ledger: Arc<dyn IdempotencyLedger>,
    thread_service: Arc<dyn SessionThreadService>,
    turn_coordinator: Arc<dyn TurnCoordinator>,
    approval_interactions: Arc<dyn ApprovalInteractionService>,
    auth_interactions: Arc<dyn AuthInteractionService>,
    delivery_services: TelegramDeliveryServicePorts,
    egress: Arc<dyn ProtocolHttpEgress>,
    token_handle: EgressCredentialHandle,
}

impl TelegramRevisionWorkflowParts {
    // arch-exempt: too_many_args, revision construction joins typed host config with retained cross-owner runtime ports at the Telegram owner boundary, plan #6159
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        config: TelegramHostConfig,
        conversation_bindings: Arc<dyn ConversationBindingService>,
        actor_pairings: Arc<dyn ConversationActorPairingService>,
        actor_user_resolver: Arc<dyn ProductActorUserResolver>,
        idempotency_ledger: Arc<dyn IdempotencyLedger>,
        thread_service: Arc<dyn SessionThreadService>,
        turn_coordinator: Arc<dyn TurnCoordinator>,
        approval_interactions: Arc<dyn ApprovalInteractionService>,
        auth_interactions: Arc<dyn AuthInteractionService>,
        delivery_services: TelegramDeliveryServicePorts,
        egress: Arc<dyn ProtocolHttpEgress>,
        token_handle: EgressCredentialHandle,
    ) -> Self {
        Self {
            config,
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
        }
    }

    pub(crate) fn config(&self) -> &TelegramHostConfig {
        &self.config
    }

    pub(crate) fn delivered_gate_routes(&self) -> Arc<dyn DeliveredGateRouteStore> {
        Arc::clone(&self.delivery_services.delivered_gate_routes)
    }

    pub(crate) fn adapter_for_setup(
        &self,
        setup: &TelegramInstallationSetup,
        installation_id: ironclaw_product_adapters::AdapterInstallationId,
    ) -> Result<Arc<dyn ProductAdapter>, TelegramHostBuildError> {
        telegram_adapter_for_setup(setup, installation_id, self.token_handle.clone())
            .map_err(|error| invalid_config(error.field, error.reason))
    }

    pub(crate) fn final_reply_delivery_services(
        &self,
        binding_service: Arc<dyn ProductBindingService>,
        adapter: Arc<dyn ProductAdapter>,
    ) -> FinalReplyDeliveryServices {
        FinalReplyDeliveryServices {
            channel_protocol: Arc::new(TelegramDeliveryProtocol),
            binding_service,
            thread_service: Arc::clone(&self.thread_service),
            turn_coordinator: Arc::clone(&self.turn_coordinator),
            outbound_store: Arc::clone(&self.delivery_services.outbound_store),
            route_store: Arc::clone(&self.delivery_services.delivered_gate_routes),
            communication_preferences: Arc::clone(
                &self.delivery_services.communication_preferences,
            ),
            adapter,
            egress: Arc::clone(&self.egress),
            delivery_sink: Arc::new(NoopTelegramDeliverySink),
            auth_challenges: self.delivery_services.auth_challenges.clone(),
            auth_flow_canceller: self.delivery_services.auth_flow_canceller.clone(),
            approval_requests: Some(Arc::clone(&self.delivery_services.approval_requests)),
        }
    }
}

impl TelegramRevisionWorkflowBuilder for TelegramRevisionWorkflowParts {
    fn build_revision_workflow(
        &self,
        setup: &TelegramInstallationSetup,
    ) -> Result<TelegramRevisionWorkflow, TelegramRevisionWorkflowBuildError> {
        let installation_id = setup
            .installation_id()
            .map_err(revision_workflow_build_error)?;
        let scope = ProductInstallationScope::with_default_scope(
            self.config.tenant_id.clone(),
            self.config.agent_id.clone(),
            self.config.project_id.clone(),
        )
        .with_default_subject_user_id(self.config.operator_user_id.clone())
        .with_actor_user_resolver(
            Arc::clone(&self.actor_user_resolver),
            Arc::clone(&self.actor_pairings),
        );
        let adapter_id = ironclaw_product_adapters::ProductAdapterId::new(TELEGRAM_V2_ADAPTER_ID)
            .map_err(revision_workflow_build_error)?;
        let installation_resolver = StaticProductInstallationResolver::new([(
            ProductInstallationKey::new(adapter_id, installation_id.clone()),
            scope,
        )]);
        let binding = ProductConversationBindingService::new(
            Arc::clone(&self.conversation_bindings),
            installation_resolver,
        );
        let inbound = Arc::new(DefaultInboundTurnService::new(
            binding.clone(),
            Arc::clone(&self.thread_service),
            Arc::clone(&self.turn_coordinator),
        ));
        let workflow: Arc<dyn ProductWorkflow> = Arc::new(
            DefaultProductWorkflow::new(
                inbound,
                Arc::clone(&self.idempotency_ledger),
                Arc::new(binding.clone()),
            )
            .with_approval_interaction_service(Arc::clone(&self.approval_interactions))
            .with_auth_interaction_service(Arc::clone(&self.auth_interactions))
            .with_delivered_gate_routes(Arc::clone(&self.delivery_services.delivered_gate_routes)),
        );
        let adapter = self
            .adapter_for_setup(setup, installation_id)
            .map_err(revision_workflow_build_error)?;
        let observer = Arc::new(FinalReplyDeliveryObserver::with_settings(
            self.final_reply_delivery_services(Arc::new(binding), adapter),
            FinalReplyDeliverySettings::default(),
        ));

        Ok(TelegramRevisionWorkflow {
            workflow,
            workflow_observer: Some(observer),
        })
    }
}

struct NoopTelegramDeliverySink;

#[async_trait]
impl OutboundDeliverySink for NoopTelegramDeliverySink {
    async fn record(&self, _status: DeliveryStatus) {}
}

fn revision_workflow_build_error(
    error: impl std::fmt::Display,
) -> TelegramRevisionWorkflowBuildError {
    TelegramRevisionWorkflowBuildError::new(error.to_string())
}

fn invalid_config(field: &'static str, reason: String) -> TelegramHostBuildError {
    TelegramHostBuildError::InvalidConfig { field, reason }
}
