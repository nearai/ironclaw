//! Web App channel package: the `ChannelDelivery` implementation that fans
//! one host-owned registration set out to a user's enrolled browsers, the preference-target codec for the
//! `web-app/v1/<tenant>/<user>` binding grammar, and the owner-scoped
//! outbound target provider that puts "Web app" in the delivery-target
//! catalog.
//!
//! Protocol mechanics (registration parsing, encryption, request planning) live in
//! `ironclaw_web_app`; the `Authorization: vapid` header is computed
//! host-side at the egress credential boundary. This crate is linked only by
//! the binary's binding table, like every concrete channel package.

mod channel;
mod preference_targets;
mod targets;

/// The package manifest, embedded crate-locally so the binary's bundle table
/// can ship it without a cross-crate include reach-in (§11.2.7).
pub const MANIFEST: &str = include_str!("../manifest.toml");

pub use channel::WebAppChannelAdapter;
pub use preference_targets::WebAppPreferenceTargetCodec;
pub use targets::{WEB_APP_TARGET_PROVIDER_KEY, WebAppOutboundTargetProvider};
