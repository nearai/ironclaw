//! The bundled Telegram channel manifest (main schema,
//! `[[product_adapter.inbound.host_ingress]]`).
//!
//! One compile-time constant shared by serve-time ingress projection
//! ([`crate::ingress`]) and composition's bundled-extension catalog —
//! the asset itself lives with the other first-party extension manifests so
//! the #6116 fold (which ships its v3 manifest at the same path) replaces the
//! contents without moving the file.

/// Raw manifest TOML for the single `telegram` extension.
pub const TELEGRAM_MANIFEST: &str =
    include_str!("../../ironclaw_first_party_extensions/assets/telegram/manifest.toml");

/// The Telegram channel manifest — [`crate::ingress`] projects the
/// `telegram.updates` host-ingress route descriptor from here.
pub fn telegram_manifest_toml() -> &'static str {
    TELEGRAM_MANIFEST
}
