//! Reborn WebUI cluster — the WebUI services service plus the shared
//! host-supplied route-mount vocabulary. The middleware/assembly that
//! composes the WebChat v2 gateway (`webui_v2_app` + its per-route
//! body/rate/origin middleware) moved up into
//! `ironclaw_webui`, which owns the host serve lifecycle;
//! only the service (`RebornWebuiBundle`) and the mount vocabulary that
//! composition's own route builders need stay here.

mod product_capability;
pub(crate) mod route_mounts;
pub(crate) mod service;
