//! The first-party package inventory.
//!
//! One small module per package (`packages/<id>.rs`) owns that package's
//! embeds (manifest + WASM), asset descriptors, digest, and any bespoke display
//! or onboarding copy, beside its `assets/<id>/` directory. [`bundled_packages`]
//! concatenates the per-module bundles; composition and the CLI consume them as
//! opaque [`PackageBundle`]s and never name a package. See
//! `docs/reborn/extension-runtime/overview.md` §3 (package self-containment).
//!
//! This crate is the sanctioned home for concrete package names — the
//! extension-specificity gate excludes it — so the names live here, next to the
//! assets they describe, and nowhere in generic code.

use std::borrow::Cow;

use ironclaw_host_api::VirtualPath;

mod github;
mod telegram;

/// Byte or filesystem content of one asset shipped inside a package, addressed
/// by its in-package `path` (manifest, input schema, prompt doc, or WASM
/// module).
pub struct PackageAsset {
    pub path: String,
    pub content: PackageAssetContent,
}

pub enum PackageAssetContent {
    Bytes(Vec<u8>),
    Filesystem(VirtualPath),
}

/// An opaque, cleanly-built first-party package: identity + display copy +
/// manifest source + assets. Host code consumes this without naming the
/// package; the concrete identity lives only in the owning package module.
pub struct PackageBundle {
    pub id: &'static str,
    pub display_name: &'static str,
    pub manifest_toml: Cow<'static, str>,
    pub assets: Vec<PackageAsset>,
}

/// A byte-content asset addressed by `path`.
pub(crate) fn bytes_asset(path: &str, bytes: &[u8]) -> PackageAsset {
    PackageAsset {
        path: path.to_string(),
        content: PackageAssetContent::Bytes(bytes.to_vec()),
    }
}

/// The bundled first-party package inventory — one entry per package module.
/// Composition and the CLI iterate these opaquely; adding an integration is a
/// new `assets/<id>/` directory plus its `packages/<id>.rs` module and a line
/// here.
pub fn bundled_packages() -> Vec<PackageBundle> {
    vec![github::bundle(), telegram::bundle()]
}
