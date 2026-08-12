//! Generic notification-setup operation descriptors (§7b of the unified
//! channel model): one status view and one enable/disable command pair,
//! parameterized by `extension_id` and dispatched to the channel's adapter.
//!
//! This surface replaces the per-channel enrollment routes the web app used
//! to carry (the retired per-channel enrollment route family): generic
//! code never mentions a channel name, a
//! push endpoint, or a VAPID key — the enable/disable payloads and the
//! status detail are channel-opaque documents only the channel's own client
//! interprets. Declared here (not in the product crate) per the
//! transport/product boundary, the same placement as [`crate::ironhub`]'s
//! command descriptor; the behavior stays behind the adapter.

use crate::descriptors::{ProductSurfaceCommandDescriptor, ProductView};
use crate::product_wire::{
    RebornNotificationSetupMutationRequest, RebornNotificationSetupRequest,
    RebornNotificationSetupStatusResponse,
};

/// WebUI-facing read of one channel's per-user notification-setup state.
pub const NOTIFICATION_SETUP_STATUS_VIEW: ProductView<
    RebornNotificationSetupRequest,
    RebornNotificationSetupStatusResponse,
> = ProductView::unpaginated("notification_setup_status");

/// Perform one channel's per-user notification enrollment with a
/// channel-opaque payload. An authenticated settings write (same class as
/// the notification-channels command); the adapter validates the payload.
pub const NOTIFICATION_SETUP_ENABLE_COMMAND_ID: &str = "notification_setup.enable";
pub const NOTIFICATION_SETUP_ENABLE_COMMAND: ProductSurfaceCommandDescriptor<
    RebornNotificationSetupMutationRequest,
    RebornNotificationSetupStatusResponse,
> = ProductSurfaceCommandDescriptor::new(NOTIFICATION_SETUP_ENABLE_COMMAND_ID);

/// Tear down one channel's per-user notification enrollment.
pub const NOTIFICATION_SETUP_DISABLE_COMMAND_ID: &str = "notification_setup.disable";
pub const NOTIFICATION_SETUP_DISABLE_COMMAND: ProductSurfaceCommandDescriptor<
    RebornNotificationSetupMutationRequest,
    RebornNotificationSetupStatusResponse,
> = ProductSurfaceCommandDescriptor::new(NOTIFICATION_SETUP_DISABLE_COMMAND_ID);
