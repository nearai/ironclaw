//! Telegram-owned host assembly below the composition boundary.

mod builder;
mod revision;

use std::sync::Arc;

use ironclaw_channel_delivery::PostSubmitDeliveryHook;
use ironclaw_channel_host::auth_continuation::RebornAuthContinuationDispatcher;
use ironclaw_channel_host::outbound_targets::OutboundDeliveryTargetProvider;
use ironclaw_conversations::{
    ConversationActorPairingService,
    ConversationBindingService as DurableConversationBindingService,
};
use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};
use ironclaw_host_runtime::HostRuntimeHttpEgressPort;
use ironclaw_outbound::{
    CommunicationPreferenceRepository, DeliveredGateRouteStore, OutboundStateStore,
    TriggeredRunDeliveryStore,
};
use ironclaw_product_workflow::{
    AccountConnectionStatusSource, ApprovalInteractionService, AuthChallengeProvider,
    AuthInteractionService, BlockedAuthFlowCanceller, ChannelConnectionFacade,
    ConnectableChannelsProductFacade, IdempotencyLedger,
};
use ironclaw_run_state::ApprovalRequestStore;
use ironclaw_secrets::SecretStore;
use ironclaw_threads::SessionThreadService;
use ironclaw_turns::TurnCoordinator;

use crate::channel_routes::{TelegramChannelRouteConfig, TelegramChannelSetupActivation};
use crate::ingress::TelegramUpdatesRouteState;
use crate::state::FilesystemTelegramHostState;

pub use builder::{
    build_telegram_host, telegram_host_scope_template,
    telegram_outbound_delivery_target_provider_key,
};
pub(crate) use revision::TelegramRevisionWorkflowParts;

/// Identity and public-origin configuration for one Telegram host.
#[derive(Debug, Clone)]
pub struct TelegramHostConfig {
    pub tenant_id: TenantId,
    pub agent_id: AgentId,
    pub project_id: Option<ProjectId>,
    pub operator_user_id: UserId,
    pub public_base_url: Option<String>,
}

impl TelegramHostConfig {
    pub fn new(
        tenant_id: TenantId,
        agent_id: AgentId,
        project_id: Option<ProjectId>,
        operator_user_id: UserId,
        public_base_url: Option<String>,
    ) -> Self {
        Self {
            tenant_id,
            agent_id,
            project_id,
            operator_user_id,
            public_base_url,
        }
    }
}

/// Generic delivery ports consumed by Telegram-owned revision and triggered
/// delivery behavior.
#[derive(Clone)]
pub struct TelegramDeliveryServicePorts {
    pub outbound_store: Arc<dyn OutboundStateStore>,
    pub delivered_gate_routes: Arc<dyn DeliveredGateRouteStore>,
    pub communication_preferences: Arc<dyn CommunicationPreferenceRepository>,
    pub triggered_run_delivery: Arc<dyn TriggeredRunDeliveryStore>,
    pub approval_requests: Arc<dyn ApprovalRequestStore>,
    pub auth_challenges: Option<Arc<dyn AuthChallengeProvider>>,
    pub auth_flow_canceller: Option<Arc<dyn BlockedAuthFlowCanceller>>,
}

/// Explicit inputs required to build the Telegram-owned runtime parts.
pub struct TelegramHostInput {
    pub config: TelegramHostConfig,
    pub state: Arc<FilesystemTelegramHostState>,
    pub secret_store: Arc<dyn SecretStore>,
    pub host_egress: HostRuntimeHttpEgressPort,
    pub continuation: Arc<dyn RebornAuthContinuationDispatcher>,
    pub conversation_bindings: Arc<dyn DurableConversationBindingService>,
    pub actor_pairings: Arc<dyn ConversationActorPairingService>,
    pub idempotency_ledger: Arc<dyn IdempotencyLedger>,
    pub thread_service: Arc<dyn SessionThreadService>,
    pub turn_coordinator: Arc<dyn TurnCoordinator>,
    pub approval_interactions: Arc<dyn ApprovalInteractionService>,
    pub auth_interactions: Arc<dyn AuthInteractionService>,
    pub delivery_services: TelegramDeliveryServicePorts,
    pub setup_activation: Option<Arc<dyn TelegramChannelSetupActivation>>,
}

/// Telegram-owned parts composition mounts and registers in host-global
/// registries.
pub struct TelegramHostParts {
    pub updates: TelegramUpdatesRouteState,
    pub channel_routes: TelegramChannelRouteConfig,
    pub connectable: Arc<dyn ConnectableChannelsProductFacade>,
    pub channel_connection: Arc<dyn ChannelConnectionFacade>,
    pub outbound_targets: Arc<dyn OutboundDeliveryTargetProvider>,
    pub trigger_hook: Arc<dyn PostSubmitDeliveryHook>,
    pub account_status: Arc<dyn AccountConnectionStatusSource>,
}
