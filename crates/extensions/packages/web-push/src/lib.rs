//! Web Push channel package: the `ChannelAdapter` that fans one delivery out
//! to a user's enrolled browsers, the preference-target codec for the
//! `web-push/v1/<tenant>/<user>` binding grammar, and the owner-scoped
//! outbound target provider that puts "Web app" in the delivery-target
//! catalog.
//!
//! Protocol mechanics (records, encryption, request planning) live in
//! `ironclaw_web_push`; the `Authorization: vapid` header is computed
//! host-side at the egress credential boundary. This crate is linked only by
//! the binary's binding table, like every concrete channel package.

mod channel;
mod preference_targets;
mod targets;

/// The package manifest, embedded crate-locally so the binary's bundle table
/// can ship it without a cross-crate include reach-in (§11.2.7).
pub const MANIFEST: &str = include_str!("../manifest.toml");

pub use channel::WebPushChannelAdapter;
pub use preference_targets::WebPushPreferenceTargetCodec;
pub use targets::{WEB_PUSH_TARGET_PROVIDER_KEY, WebPushOutboundTargetProvider};
