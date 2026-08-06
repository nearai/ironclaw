//! Descriptor-backed outbound delivery read projections, plus the
//! notification-channels product-command write (co-located here rather than
//! in a same-named "views" file split, since both sides of the WebUI-facing
//! notification-channels surface delegate directly to
//! `OutboundPreferencesProductService` with no transformation of their own —
//! see `crates/ironclaw_webui/src/webui_v2/handlers.rs`'s
//! `get_notification_channels` / `set_notification_channels`).

use ironclaw_product_contracts::views::RebornViewProvider;

use super::{
    ProductCapabilityInvoker, ProductSurfaceCaller, ProductSurfaceError, ProductView,
    RebornNotificationChannelsResponse, RebornOutboundDeliveryTargetListResponse, RebornServices,
    RebornSetNotificationChannelsRequest,
};

pub const OUTBOUND_DELIVERY_TARGETS_VIEW: ProductView<
    serde_json::Value,
    RebornOutboundDeliveryTargetListResponse,
> = ProductView::unpaginated("outbound_delivery_targets");

/// WebUI-facing read of the caller's stored notification-channel set (Task 8's
/// `get_notification_channels`, spec §7). Distinct from
/// `builtin.notification_channels_set`'s model-callable write surface in
/// `outbound_delivery_capability_surface.rs` — this view has no model-visible
/// counterpart.
pub const NOTIFICATION_CHANNELS_VIEW: ProductView<
    serde_json::Value,
    RebornNotificationChannelsResponse,
> = ProductView::unpaginated("notification_channels");

impl<I, V> RebornServices<I, V>
where
    I: ProductCapabilityInvoker + Clone + 'static,
    V: RebornViewProvider + Clone + 'static,
{
    pub(super) async fn build_outbound_delivery_targets_view(
        &self,
        caller: ProductSurfaceCaller,
    ) -> Result<RebornOutboundDeliveryTargetListResponse, ProductSurfaceError> {
        self.outbound_preferences_service
            .list_outbound_delivery_targets(caller)
            .await
    }

    pub(super) async fn build_notification_channels_view(
        &self,
        caller: ProductSurfaceCaller,
    ) -> Result<RebornNotificationChannelsResponse, ProductSurfaceError> {
        self.outbound_preferences_service
            .get_notification_channels(caller)
            .await
    }

    /// WebUI-facing full-replace write (Task 8's `set_notification_channels`),
    /// dispatched through [`super::product_capability_handlers::ProductCommandHandler`]
    /// rather than a composition-registered capability: the mutation is a
    /// direct authenticated-user settings write with no approval-gate need —
    /// see the composed `OutboundPreferencesProductService`.
    /// The model-callable `builtin.notification_channels_set` tool is a
    /// separate, approval-gated capability that reaches the same underlying
    /// service method through its own dispatch path
    /// (`set_notification_channels_for_model`).
    pub(super) async fn set_notification_channels(
        &self,
        caller: ProductSurfaceCaller,
        request: RebornSetNotificationChannelsRequest,
    ) -> Result<RebornNotificationChannelsResponse, ProductSurfaceError> {
        self.outbound_preferences_service
            .set_notification_channels(caller, request)
            .await
    }
}
