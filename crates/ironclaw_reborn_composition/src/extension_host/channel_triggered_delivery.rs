//! Assembly adapter between trigger settlement and product-owned delivery.
//!
//! Composition supplies current channel candidates and runtime ports. Target
//! resolution, preference fallback, ambiguity, failure classification, and
//! driver selection live in `ironclaw_product`.

use std::sync::Arc;

use ironclaw_outbound::{CommunicationPreferenceRepository, TriggeredRunDeliveryStore};
use ironclaw_product::{
    CurrentDeliveryTargetResolver, RunDeliveryEventRouter, TriggeredRunDeliveryRouter,
};
use ironclaw_triggers::TriggerFire;
use ironclaw_turns::{TurnRunId, TurnScope};

use crate::automation::trigger_poller::PostSubmitDeliveryHook;
use crate::extension_host::channel_host::GenericChannelHostAssembly;

/// Thin composition adapter for the generic product-owned trigger router.
pub(crate) struct GenericTriggeredRunDeliveryHook {
    assembly: Arc<GenericChannelHostAssembly>,
    router: TriggeredRunDeliveryRouter,
}

impl GenericTriggeredRunDeliveryHook {
    pub(crate) fn new(
        assembly: Arc<GenericChannelHostAssembly>,
        delivery_store: Arc<dyn TriggeredRunDeliveryStore>,
        preferences: Arc<dyn CommunicationPreferenceRepository>,
        current_targets: Arc<dyn CurrentDeliveryTargetResolver>,
        event_router: Arc<RunDeliveryEventRouter>,
    ) -> Self {
        let router = TriggeredRunDeliveryRouter::new(
            delivery_store,
            preferences,
            current_targets,
            assembly.identity().agent_id.clone(),
            event_router,
        );
        Self { assembly, router }
    }
}

#[async_trait::async_trait]
impl PostSubmitDeliveryHook for GenericTriggeredRunDeliveryHook {
    async fn on_trigger_submitted(&self, fire: TriggerFire, run_id: TurnRunId, scope: TurnScope) {
        self.router
            .on_trigger_submitted(
                fire,
                run_id,
                scope,
                self.assembly.active_triggered_delivery_channels(),
            )
            .await;
    }
}
