//! Descriptor-backed outbound delivery read projections.

use super::{
    ProductCapabilityInvoker, RebornOutboundDeliveryTargetListResponse,
    RebornOutboundPreferencesResponse, RebornServices, RebornServicesError, RebornViewDescriptor,
    RebornViewProvider, WebUiAuthenticatedCaller,
};

pub const OUTBOUND_PREFERENCES_VIEW: RebornViewDescriptor = RebornViewDescriptor {
    id: "outbound_preferences",
    paginated: false,
};

pub const OUTBOUND_DELIVERY_TARGETS_VIEW: RebornViewDescriptor = RebornViewDescriptor {
    id: "outbound_delivery_targets",
    paginated: false,
};

impl<I, V> RebornServices<I, V>
where
    I: ProductCapabilityInvoker + Clone + 'static,
    V: RebornViewProvider + Clone + 'static,
{
    pub(super) async fn build_outbound_preferences_view(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornOutboundPreferencesResponse, RebornServicesError> {
        self.outbound_preferences_facade
            .get_outbound_preferences(caller)
            .await
    }

    pub(super) async fn build_outbound_delivery_targets_view(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornOutboundDeliveryTargetListResponse, RebornServicesError> {
        self.outbound_preferences_facade
            .list_outbound_delivery_targets(caller)
            .await
    }
}
