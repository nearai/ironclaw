//! One-release fixed-path channel ingress aliases (extension-runtime MIG-5).
//!
//! This is the deliberately isolated compatibility table. Every entry forwards
//! into the same generic ingress router and carries an explicit removal note;
//! channel behavior and routing policy remain owned by the generic path.

use crate::extension_host::extension_ingress::{
    ExtensionIngressParts, extension_ingress_alias_route_mount,
};
use crate::webui::route_mounts::PublicRouteMount;

struct LegacyChannelIngressAlias {
    legacy_path: &'static str,
    route_id: &'static str,
    extension_id: &'static str,
    route_suffix: &'static str,
}

/// REMOVAL NOTE (MIG-5): delete this entry and this module in the first
/// release after the generic Slack ingress cutover ships and operators have
/// moved their Events API URL to `/webhooks/extensions/slack/events`.
const LEGACY_CHANNEL_INGRESS_ALIASES: &[LegacyChannelIngressAlias] = &[LegacyChannelIngressAlias {
    legacy_path: "/webhooks/slack/events",
    route_id: "slack.events.compat",
    extension_id: "slack",
    route_suffix: "events",
}];

pub fn legacy_extension_ingress_alias_mounts(
    parts: &ExtensionIngressParts,
) -> Result<Vec<PublicRouteMount>, crate::RebornBuildError> {
    LEGACY_CHANNEL_INGRESS_ALIASES
        .iter()
        .map(|alias| {
            extension_ingress_alias_route_mount(
                parts,
                alias.route_id,
                alias.legacy_path,
                alias.extension_id,
                alias.route_suffix,
            )
        })
        .collect()
}
