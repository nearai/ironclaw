//! Sendblue package assets.
use super::{PackageBundle, bytes_asset};
use std::borrow::Cow;
pub(super) const ID: &str = "sendblue";
const MANIFEST: &str = include_str!("../../../packages/sendblue/manifest.toml");
pub(super) fn bundle() -> PackageBundle {
    PackageBundle {
        id: ID,
        display_name: "Sendblue",
        manifest_toml: Cow::Borrowed(MANIFEST),
        assets: vec![bytes_asset("manifest.toml", MANIFEST.as_bytes())],
        onboarding: None,
        trust_effects: None,
    }
}
