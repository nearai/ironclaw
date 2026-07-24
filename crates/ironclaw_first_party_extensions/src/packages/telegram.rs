//! Telegram channel package — a pure channel extension: one manifest, no
//! assets beyond it (the manifest itself; DEL-10's addition proof).

use std::borrow::Cow;

use super::{PackageBundle, bytes_asset};

pub(super) const ID: &str = "telegram";

const MANIFEST: &str = include_str!("../../assets/telegram/manifest.toml");

pub(super) fn bundle() -> PackageBundle {
    PackageBundle {
        id: ID,
        display_name: "Telegram",
        manifest_toml: Cow::Borrowed(MANIFEST),
        assets: vec![bytes_asset("manifest.toml", MANIFEST.as_bytes())],
        // Telegram is bot-token setup handled by the channel host, not an
        // extension-card onboarding flow: no bespoke copy.
        onboarding: None,
        // Channel-only package: trust comes from the extension registry, not an
        // admin local-manifest effect grant.
        trust_effects: None,
    }
}
