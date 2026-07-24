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

use ironclaw_host_api::EffectKind;

mod github;
mod gmail;
mod gsuite;
mod notion;
mod slack;
mod telegram;
mod web_access;

/// One inventory entry: a package's id paired with its bundle builder.
type PackageEntry = (&'static str, fn() -> PackageBundle);

/// The single source of the bundled inventory: `(id, bundle builder)` per
/// package module. [`bundled_packages`] runs the builders (materializing
/// embeds); [`bundled_package_ids`] reads the ids without touching embeds. Both
/// derive from this one list, so a package cannot appear in one view and not the
/// other. Each `ID` const lives in its owning module beside `bundle()`.
const PACKAGES: &[PackageEntry] = &[
    (github::ID, github::bundle),
    (gmail::ID, gmail::bundle),
    (gsuite::CALENDAR_ID, gsuite::google_calendar_bundle),
    (gsuite::DOCS_ID, gsuite::google_docs_bundle),
    (gsuite::DRIVE_ID, gsuite::google_drive_bundle),
    (gsuite::SHEETS_ID, gsuite::google_sheets_bundle),
    (gsuite::SLIDES_ID, gsuite::google_slides_bundle),
    (notion::ID, notion::bundle),
    (slack::ID, slack::bundle),
    (telegram::ID, telegram::bundle),
    (web_access::ID, web_access::bundle),
];

/// Byte content of one asset shipped inside a package, addressed by its
/// in-package `path` (manifest, input schema, prompt doc, or WASM module).
pub struct PackageAsset {
    pub path: String,
    pub content: PackageAssetContent,
}

pub enum PackageAssetContent {
    Bytes(Vec<u8>),
}

/// A package's user-facing onboarding copy, carried as plain data (no host
/// lifecycle types — this crate sits below `product_workflow`). Composition
/// maps this to its `LifecycleExtensionOnboarding` at summary time. The strings
/// are the exact bespoke copy that used to live in composition's per-id `match`.
pub struct PackageOnboarding {
    pub instructions: String,
    pub credential_instructions: Option<String>,
    pub setup_url: Option<String>,
    pub credential_next_step: String,
}

/// An opaque, cleanly-built first-party package: identity + display copy +
/// manifest source + assets + onboarding. Host code consumes this without
/// naming the package; the concrete identity lives only in the owning package
/// module.
pub struct PackageBundle {
    pub id: &'static str,
    pub display_name: &'static str,
    pub manifest_toml: Cow<'static, str>,
    pub assets: Vec<PackageAsset>,
    /// Bespoke onboarding copy, `None` for packages that need no setup guidance.
    pub onboarding: Option<PackageOnboarding>,
    /// Host authority effects this first-party package is granted in the
    /// built-in trust policy, carried as explicit data (not derived from the
    /// manifest — the trust grant is an independent host assertion, defense in
    /// depth). `None` for packages whose trust comes from the WASM extension
    /// registry rather than an admin local-manifest entry (e.g. the pure WASM
    /// tools and channel-only packages). Composition still owns the trust
    /// *decision* (`HostTrustAssignment::first_party`); the bundle only supplies
    /// the effect list.
    pub trust_effects: Option<Vec<EffectKind>>,
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
/// in [`struct@PACKAGES`].
pub fn bundled_packages() -> Vec<PackageBundle> {
    PACKAGES.iter().map(|(_, build)| build()).collect()
}

/// The ids of every bundled package, cheap (no embed materialization). Host code
/// that only needs to test membership — e.g. "is this a reserved host-bundled
/// id a filesystem extension must not shadow" — uses this instead of building
/// the full bundles.
pub fn bundled_package_ids() -> Vec<&'static str> {
    PACKAGES.iter().map(|(id, _)| *id).collect()
}
