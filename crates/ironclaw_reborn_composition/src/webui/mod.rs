//! Reborn WebUI cluster — the WebUI services facade plus the shared
//! host-supplied route-mount vocabulary. The middleware/assembly that
//! composes the WebChat v2 gateway (`webui_v2_app` + its per-route
//! body/rate/origin middleware) moved up into
//! `ironclaw_webui`, which owns the host serve lifecycle;
//! only the facade (`RebornWebuiBundle`) and the mount vocabulary that
//! composition's own route builders need stay here.

#[cfg(any(
    test,
    all(feature = "slack-v2-host-beta", feature = "telegram-v2-host-beta")
))]
pub(crate) mod composite_channels;
pub(crate) mod facade;
#[cfg(feature = "webui-v2-beta")]
pub(crate) mod route_mounts;
