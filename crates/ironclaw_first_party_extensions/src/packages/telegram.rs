//! Telegram channel package — a pure channel extension: one manifest, no
//! assets beyond it (the manifest itself; DEL-10's addition proof).

use std::borrow::Cow;

use super::{PackageBundle, bytes_asset};

const MANIFEST: &str = include_str!("../../assets/telegram/manifest.toml");

pub(super) fn bundle() -> PackageBundle {
    PackageBundle {
        id: "telegram",
        display_name: "Telegram",
        manifest_toml: Cow::Borrowed(MANIFEST),
        assets: vec![bytes_asset("manifest.toml", MANIFEST.as_bytes())],
    }
}
