//! Requested-trust vocabulary for IronClaw Reborn.
//!
//! This module is the *input* side of the host trust policy boundary. Manifests,
//! registry entries, and admin configuration deserialize into [`PackageIdentity`]
//! and [`RequestedTrustClass`]; the host policy engine in `ironclaw_trust`
//! consumes them and produces an effective trust decision.
//!
//! The split is deliberate: [`crate::TrustClass`] (in `runtime`) is the
//! *effective* ceiling that downstream authorization consumes, and its
//! privileged variants (`FirstParty`, `System`) reject `serde` deserialization.
//! [`RequestedTrustClass`] is the *declared* counterpart — it can be safely
//! deserialized from any source, including untrusted user manifests, because
//! it cannot be confused with effective trust at the type level.
//!
//! ## Cross-crate vocabulary
//!
//! - The manifest field `trust = "..."` parses into [`RequestedTrustClass`]
//!   (snake_case mapping: `untrusted`, `third_party`, `first_party_requested`,
//!   `system_requested`). It is metadata — never authority.
//! - [`PackageIdentity`] is the trust-policy-side identity for any
//!   manifest-bearing package: installed extensions (WASM / Script / MCP),
//!   bundled extensions / loops / skills, operator-declared packages, and
//!   eventual built-in tools (see `crates/ironclaw_trust/CONTRACT.md` §9 for
//!   the migration path).
//! - The `package_id: PackageId` field on [`PackageIdentity`] is the same
//!   value as `ExtensionId` at other layers — `ExtensionId` when the
//!   identity reaches the extension registry / `CapabilityDescriptor.provider`,
//!   `PackageId` when it reaches the trust policy. The two names describe
//!   the same value at different layers.
//! - [`crate::CapabilityDescriptor::trust_ceiling`] mirrors the manifest's
//!   declared trust as `TrustClass` — it is *declarative metadata*, not the
//!   policy-validated effective ceiling. The privileged variants of
//!   `TrustClass` reject deserialization, so this field can only carry
//!   `Sandbox` / `UserTrusted` from manifest input. Effective trust comes
//!   from `ironclaw_trust::TrustPolicy::evaluate` and is attached to
//!   [`crate::ExecutionContext::trust`] at dispatch time.
//!
//! See `ironclaw_trust::TrustPolicy` for the engine that bridges request to
//! effective trust, `crates/ironclaw_trust/CONTRACT.md` for the full
//! evaluation matrix, and `docs/reborn/contracts/host-api.md` (in the
//! staging-track docs) for the broader Reborn vocabulary.

use serde::{Deserialize, Serialize};

/// Trust class declared by an untrusted package manifest or registry entry.
///
/// Free deserialization is intentional: any source — bundled, registry,
/// user-installed manifest, admin config — can produce one of these. It is not
/// authority. The privileged-sounding `FirstPartyRequested` and
/// `SystemRequested` variants only express *intent*; they grant nothing on
/// their own and must be matched against host policy in `ironclaw_trust`
/// before any privileged effect can take place.
///
/// ## Manifest mapping
///
/// The manifest field `trust = "..."` (see `docs/reborn/contracts/extensions.md`
/// §4 in the staging-track docs) parses into this type via snake_case serde:
///
/// | Manifest value | Variant |
/// |---|---|
/// | `"untrusted"` (or absent) | [`Self::Untrusted`] |
/// | `"third_party"` | [`Self::ThirdParty`] |
/// | `"first_party_requested"` | [`Self::FirstPartyRequested`] |
/// | `"system_requested"` | [`Self::SystemRequested`] |
///
/// A manifest may freely declare any variant; whether the host honors a
/// privileged request is decided by `ironclaw_trust::TrustPolicy::evaluate`,
/// not by parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestedTrustClass {
    /// No trust requested. Treated as fully sandboxed.
    Untrusted,
    /// Third-party extension requesting normal user-trusted operation.
    ThirdParty,
    /// Manifest requests first-party privileges. Only effective if host policy
    /// matches the package identity.
    FirstPartyRequested,
    /// Manifest requests system-level privileges. Only effective if host policy
    /// matches the package identity. Reserved for host-owned services in
    /// production; ordinary manifests should never carry this.
    SystemRequested,
}

/// Origin of a package definition.
///
/// The variant tells the trust policy engine which evaluation rule applies
/// (bundled-only registry vs. signed remote vs. operator override). Local
/// manifests are untrusted by default; privileged effective trust requires an
/// explicit, source-pinned host policy entry (for example an admin override
/// for that exact manifest path), never the manifest's own assertion.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum PackageSource {
    /// Compiled into the host binary or bundled with a signed release.
    Bundled,
    /// User-installed package read from a local manifest file. Untrusted by
    /// default — privileged trust requires a separate host-policy match.
    LocalManifest { path: String },
    /// Fetched from a remote registry. Trust requires signature verification
    /// and a host-policy entry; PR1b only validates the source tag.
    Registry { url: String },
    /// Operator/admin configuration assertion (e.g., trusted-package list set
    /// outside any user-controlled file).
    Admin,
}

/// Stable identity for a package as seen by the host trust policy.
///
/// `package_id` is the canonical name; `source` records where the definition
/// came from; `digest` and `signer` are optional verification anchors. The
/// trust policy engine matches on the combination — drift in any of these
/// fields invalidates retained grants per the issue acceptance criteria.
///
/// ## Scope
///
/// `PackageIdentity` is the trust-policy identity for **any
/// manifest-bearing package** that flows through the host trust pipeline:
///
/// | Scope | Typical [`PackageSource`] | Notes |
/// |---|---|---|
/// | Installed extensions (WASM / Script / MCP) | `LocalManifest` or `Registry` | Manifest declares `trust = "..."` |
/// | Bundled extensions / loops / skills | `Bundled` | Compiled with the host; matched by `BundledRegistry` |
/// | Operator declarations | `Admin` | `AdminConfig` out-of-band trust assertion |
/// | Built-in tools (eventual) | `Bundled` | See `crates/ironclaw_trust/CONTRACT.md` §9 for migration |
///
/// `package_id: PackageId` is the same value as `ExtensionId` at the extension
/// registry / `CapabilityDescriptor.provider` layer. The two names describe
/// the same identifier observed by different consumers.
///
/// `package_id` collisions across [`PackageSource`] variants are **never**
/// treated as the same package — the `ironclaw_trust` policy binds trust to
/// the `(package_id, source)` pair, so a `LocalManifest` and a `Bundled`
/// package with the same id are distinct trust subjects.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PackageIdentity {
    pub package_id: crate::PackageId,
    pub source: PackageSource,
    /// Hex-encoded sha256 of the artifact bytes when the source supplies one.
    pub digest: Option<String>,
    /// Signing key or signer identity when the source supplies a verified
    /// signature.
    pub signer: Option<String>,
}

impl PackageIdentity {
    pub fn new(
        package_id: crate::PackageId,
        source: PackageSource,
        digest: Option<String>,
        signer: Option<String>,
    ) -> Self {
        Self {
            package_id,
            source,
            digest,
            signer,
        }
    }
}
