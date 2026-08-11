//! Web-push enrollment operation descriptors: the status view and the
//! subscribe/unsubscribe product commands the WebUI settings panel drives.
//!
//! Declared here (not in the product crate) per the transport/product
//! boundary: transports consume the product boundary, and new feature
//! descriptors join the contracts tier rather than growing the frozen
//! webui→assistant symbol residue (same placement as
//! [`crate::ironhub`]'s command descriptor). The *behavior* — enrollment
//! validation, storage, VAPID custody — stays in `ironclaw_assistant` and
//! the web-push domain/channel crates.

use crate::descriptors::{ProductSurfaceCommandDescriptor, ProductView};
use crate::product_wire::{
    RebornWebPushStatusResponse, RebornWebPushSubscribeRequest, RebornWebPushSubscribeResponse,
    RebornWebPushUnsubscribeRequest, RebornWebPushUnsubscribeResponse,
};

/// WebUI-facing read of the deployment VAPID public key and the caller's
/// enrolled browsers.
pub const WEB_PUSH_STATUS_VIEW: ProductView<serde_json::Value, RebornWebPushStatusResponse> =
    ProductView::unpaginated("web_push_status");

/// Enroll/refresh the caller's current browser for web push. A webui-only
/// authenticated settings write (same class as the notification-channels
/// command); the browser mints the subscription and the service validates it
/// against the deployment's declared push-service hosts.
pub const WEB_PUSH_SUBSCRIBE_COMMAND_ID: &str = "web_push.subscribe";
pub const WEB_PUSH_SUBSCRIBE_COMMAND: ProductSurfaceCommandDescriptor<
    RebornWebPushSubscribeRequest,
    RebornWebPushSubscribeResponse,
> = ProductSurfaceCommandDescriptor::new(WEB_PUSH_SUBSCRIBE_COMMAND_ID);

/// Remove one of the caller's browser enrollments by endpoint.
pub const WEB_PUSH_UNSUBSCRIBE_COMMAND_ID: &str = "web_push.unsubscribe";
pub const WEB_PUSH_UNSUBSCRIBE_COMMAND: ProductSurfaceCommandDescriptor<
    RebornWebPushUnsubscribeRequest,
    RebornWebPushUnsubscribeResponse,
> = ProductSurfaceCommandDescriptor::new(WEB_PUSH_UNSUBSCRIBE_COMMAND_ID);
