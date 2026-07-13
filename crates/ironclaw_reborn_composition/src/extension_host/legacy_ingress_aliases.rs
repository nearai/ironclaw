//! One-release legacy channel ingress path aliases (extension-runtime §11,
//! migration MIG-5).
//!
//! Sanctioned specificity: like `extension_host/channel_state_folds.rs`,
//! this compatibility module names the concrete legacy surface it forwards —
//! nothing outside this module carries a vendor-named webhook path once the
//! per-vendor lanes are deleted.
//!
//! Each alias mounts the retired fixed webhook path and forwards the request
//! internally to the SAME generic ingress router the canonical
//! `/webhooks/extensions/{extension_id}/{route_suffix}` route drives (an
//! internal forward, not an HTTP redirect — vendors do not follow redirects
//! for event delivery). REMOVAL NOTE (MIG-5): delete each alias entry — and
//! this module when the table empties — in the first release after its
//! channel's cutover ships; vendors reconfigure their event URL to the
//! canonical path.

use std::pin::Pin;
use std::sync::Arc;

use axum::{
    Router, body::Bytes, extract::State, http::HeaderMap, response::Response, routing::post,
};
use ironclaw_extension_host::ingress::ExtensionIngressRouter;

use crate::extension_host::extension_ingress::{ChannelIngressDrain, forward_alias_request};
use crate::webui::webui_serve::{PublicRouteDrain, PublicRouteMount};

/// One legacy fixed-path alias onto the canonical generic ingress route.
struct LegacyChannelIngressAlias {
    /// The retired fixed path vendors still post to.
    legacy_path: &'static str,
    /// Ingress descriptor route id for the alias mount.
    route_id: &'static str,
    /// The canonical route's extension id.
    extension_id: &'static str,
    /// The canonical route's manifest-declared route suffix.
    route_suffix: &'static str,
    /// The bundled manifest the alias projects its ingress policy from.
    manifest_toml: fn() -> &'static str,
}

/// The alias table. REMOVAL NOTE (MIG-5): the slack entry — the legacy
/// `/webhooks/slack/events` Events API path — is deleted in the first
/// release after the P4 cutover ships.
const LEGACY_CHANNEL_INGRESS_ALIASES: &[LegacyChannelIngressAlias] = &[LegacyChannelIngressAlias {
    legacy_path: "/webhooks/slack/events",
    route_id: "slack.events",
    extension_id: "slack",
    route_suffix: "events",
    manifest_toml: crate::extension_host::available_extensions::slack_manifest_toml,
}];

/// Build every legacy alias mount over the generic ingress router. The
/// optional drain rides every alias (route-owned in-flight work must settle
/// before the runtime tears down, exactly like the canonical mount).
pub(crate) fn legacy_channel_ingress_alias_mounts(
    router: &Arc<ExtensionIngressRouter>,
    drain: Option<Arc<dyn ChannelIngressDrain>>,
) -> Result<Vec<PublicRouteMount>, crate::RebornBuildError> {
    LEGACY_CHANNEL_INGRESS_ALIASES
        .iter()
        .map(|alias| legacy_alias_mount(alias, Arc::clone(router), drain.clone()))
        .collect()
}

fn legacy_alias_mount(
    alias: &LegacyChannelIngressAlias,
    router: Arc<ExtensionIngressRouter>,
    drain: Option<Arc<dyn ChannelIngressDrain>>,
) -> Result<PublicRouteMount, crate::RebornBuildError> {
    let descriptor = crate::host_ingress::bundled_channel_ingress_descriptor(
        (alias.manifest_toml)(),
        alias.route_id,
        alias.legacy_path,
    )
    .map_err(|error| crate::RebornBuildError::InvalidConfig {
        reason: format!("legacy channel events alias descriptor invalid: {error}"),
    })?;
    let axum_router = Router::new()
        .route(alias.legacy_path, post(alias_handler))
        .with_state(AliasState {
            router,
            extension_id: alias.extension_id,
            route_suffix: alias.route_suffix,
        });
    let mut mount = PublicRouteMount::new(axum_router, vec![descriptor]);
    if let Some(drain) = drain {
        mount = mount.with_drain(Arc::new(AliasDrain(drain)));
    }
    Ok(mount)
}

#[derive(Clone)]
struct AliasState {
    router: Arc<ExtensionIngressRouter>,
    extension_id: &'static str,
    route_suffix: &'static str,
}

async fn alias_handler(
    State(state): State<AliasState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    forward_alias_request(
        &state.router,
        state.extension_id,
        state.route_suffix,
        &headers,
        body,
    )
    .await
}

struct AliasDrain(Arc<dyn ChannelIngressDrain>);

impl PublicRouteDrain for AliasDrain {
    fn drain<'a>(&'a self) -> Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
        Box::pin(self.0.drain())
    }
}

/// Serve-layer wrapper over the alias table: one mount per retired vendor
/// webhook path, forwarding into the SAME generic ingress router the
/// canonical `/webhooks/extensions/{extension_id}/{route_suffix}` mount
/// drives. In-flight work drains through the shared ingress registry (the
/// canonical mount's drain), so the aliases carry no drain of their own.
pub fn legacy_extension_ingress_alias_mounts(
    parts: &crate::extension_host::extension_ingress::ExtensionIngressParts,
) -> Result<Vec<PublicRouteMount>, crate::RebornBuildError> {
    legacy_channel_ingress_alias_mounts(&parts.router, None)
}
