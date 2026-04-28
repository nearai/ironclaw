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
//! See `ironclaw_trust` for the policy engine that bridges the two and
//! `docs/reborn/contracts/host-api.md` for the broader trust contract.

use serde::{Deserialize, Serialize};

/// Trust class declared by an untrusted package manifest or registry entry.
///
/// Free deserialization is intentional: any source — bundled, registry,
/// user-installed manifest, admin config — can produce one of these. It is not
/// authority. The privileged-sounding `FirstPartyRequested` and
/// `SystemRequested` variants only express *intent*; they grant nothing on
/// their own and must be matched against host policy in `ironclaw_trust`
/// before any privileged effect can take place.
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
/// (bundled-only registry vs. signed remote vs. operator override). Only
/// host-controlled origins (`Bundled`, `Admin`, signed `Registry`) can produce
/// privileged effective trust; `LocalManifest` always caps at user-trusted.
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
