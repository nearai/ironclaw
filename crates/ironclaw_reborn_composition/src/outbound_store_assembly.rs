use std::sync::Arc;

use ironclaw_filesystem::CompositeRootFilesystem;
use ironclaw_outbound::{
    CommunicationPreferenceRepository, DeliveredGateRouteStore, OutboundStateStore,
    OutboundStateStorePort, ReplyAttachmentIntentPort, TriggeredRunDeliveryStore,
};

/// All outbound persistence roles backed by one shared store allocation.
pub(crate) struct OutboundStoreAssembly {
    pub(crate) outbound_preferences: Arc<dyn CommunicationPreferenceRepository>,
    pub(crate) outbound_state: Arc<dyn OutboundStateStorePort>,
    pub(crate) reply_attachment_intents: Arc<dyn ReplyAttachmentIntentPort>,
    pub(crate) delivered_gate_routes: Arc<dyn DeliveredGateRouteStore>,
    pub(crate) triggered_run_delivery: Arc<dyn TriggeredRunDeliveryStore>,
}

/// Builds the outbound persistence graph over the composition-owned filesystem.
pub(crate) fn build_outbound_stores(
    filesystem: Arc<CompositeRootFilesystem>,
) -> OutboundStoreAssembly {
    // Every outbound role is an Arc clone of this allocation, so preferences
    // and delivery state cannot drift across backing trees.
    #[allow(clippy::disallowed_methods)]
    let store: Arc<OutboundStateStore<CompositeRootFilesystem>> =
        Arc::new(OutboundStateStore::new(crate::wrap_scoped(filesystem)));
    OutboundStoreAssembly {
        outbound_preferences: Arc::clone(&store) as Arc<dyn CommunicationPreferenceRepository>,
        outbound_state: Arc::clone(&store) as Arc<dyn OutboundStateStorePort>,
        reply_attachment_intents: Arc::clone(&store) as Arc<dyn ReplyAttachmentIntentPort>,
        delivered_gate_routes: Arc::clone(&store) as Arc<dyn DeliveredGateRouteStore>,
        triggered_run_delivery: store as Arc<dyn TriggeredRunDeliveryStore>,
    }
}
