//! Descriptor-backed outbound delivery read projections.

use super::{
    IronClawOutboundDeliveryTargetListResponse, IronClawOutboundPreferencesResponse,
    IronClawServices, IronClawServicesError, IronClawViewProvider, ProductCapabilityInvoker,
    ProductView, WebUiAuthenticatedCaller,
};

pub const OUTBOUND_PREFERENCES_VIEW: ProductView<
    serde_json::Value,
    IronClawOutboundPreferencesResponse,
> = ProductView::unpaginated("outbound_preferences");

pub const OUTBOUND_DELIVERY_TARGETS_VIEW: ProductView<
    serde_json::Value,
    IronClawOutboundDeliveryTargetListResponse,
> = ProductView::unpaginated("outbound_delivery_targets");

impl<I, V> IronClawServices<I, V>
where
    I: ProductCapabilityInvoker + Clone + 'static,
    V: IronClawViewProvider + Clone + 'static,
{
    pub(super) async fn build_outbound_preferences_view(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<IronClawOutboundPreferencesResponse, IronClawServicesError> {
        self.outbound_preferences_facade
            .get_outbound_preferences(caller)
            .await
    }

    pub(super) async fn build_outbound_delivery_targets_view(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<IronClawOutboundDeliveryTargetListResponse, IronClawServicesError> {
        self.outbound_preferences_facade
            .list_outbound_delivery_targets(caller)
            .await
    }
}
