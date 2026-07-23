//! Descriptor-backed outbound delivery read projections.

use super::{
    ProductCapabilityInvoker, ProductSurfaceError, ProductView,
    RebornOutboundDeliveryTargetListResponse, RebornOutboundPreferencesResponse, RebornServices,
    RebornViewProvider, WebUiAuthenticatedCaller,
};

pub const OUTBOUND_PREFERENCES_VIEW: ProductView<
    serde_json::Value,
    RebornOutboundPreferencesResponse,
> = ProductView::unpaginated("outbound_preferences");

pub const OUTBOUND_DELIVERY_TARGETS_VIEW: ProductView<
    serde_json::Value,
    RebornOutboundDeliveryTargetListResponse,
> = ProductView::unpaginated("outbound_delivery_targets");

impl<I, V> RebornServices<I, V>
where
    I: ProductCapabilityInvoker + Clone + 'static,
    V: RebornViewProvider + Clone + 'static,
{
    pub(super) async fn build_outbound_preferences_view(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornOutboundPreferencesResponse, ProductSurfaceError> {
        self.outbound_preferences_facade
            .get_outbound_preferences(caller)
            .await
    }

    pub(super) async fn build_outbound_delivery_targets_view(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornOutboundDeliveryTargetListResponse, ProductSurfaceError> {
        self.outbound_preferences_facade
            .list_outbound_delivery_targets(caller)
            .await
    }
}
