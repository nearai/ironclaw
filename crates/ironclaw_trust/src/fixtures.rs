//! Test-only fixture constructors. **Not for production use.**
//!
//! This module is gated behind the `test-fixtures` Cargo feature so it is
//! invisible to normal production builds. The crate's own unit tests
//! (`#[cfg(test)]`) and integration tests opt in via the feature; nothing
//! else can.
//!
//! Even with the feature on, naming intentionally signals the boundary
//! (`*_for_test`). PR review should reject any non-test caller.

use ironclaw_host_api::{EffectKind, PackageId, PackageSource, ResourceCeiling};

use crate::decision::EffectiveTrustClass;
use crate::sources::{AdminEntry, BundledEntry, admin_entry_with_trust, bundled_entry_with_trust};

/// Test fixture: privileged effective trust at the `FirstParty` ceiling.
pub fn effective_first_party_for_test() -> EffectiveTrustClass {
    EffectiveTrustClass::first_party()
}

/// Test fixture: privileged effective trust at the `System` ceiling.
pub fn effective_system_for_test() -> EffectiveTrustClass {
    EffectiveTrustClass::system()
}

/// Test fixture: a [`BundledEntry`] at the given effective trust ceiling.
pub fn bundled_entry_for_test(
    package_id: PackageId,
    digest: Option<String>,
    effective_trust: EffectiveTrustClass,
    allowed_effects: Vec<EffectKind>,
) -> BundledEntry {
    bundled_entry_with_trust(package_id, digest, effective_trust, allowed_effects, None)
}

/// Test fixture: a [`BundledEntry`] with an explicit resource ceiling.
pub fn bundled_entry_with_resource_ceiling_for_test(
    package_id: PackageId,
    digest: Option<String>,
    effective_trust: EffectiveTrustClass,
    allowed_effects: Vec<EffectKind>,
    max_resource_ceiling: ResourceCeiling,
) -> BundledEntry {
    bundled_entry_with_trust(
        package_id,
        digest,
        effective_trust,
        allowed_effects,
        Some(max_resource_ceiling),
    )
}

/// Test fixture: an [`AdminEntry`] bound to a specific [`PackageSource`].
///
/// Tests must spell the source explicitly so that the source-pin invariant
/// in `AdminConfig::evaluate` is exercised end-to-end. The fixture exists
/// to keep test bodies short, not to hide the source binding.
pub fn admin_entry_for_test(
    package_id: PackageId,
    source: PackageSource,
    effective_trust: EffectiveTrustClass,
    allowed_effects: Vec<EffectKind>,
) -> AdminEntry {
    admin_entry_with_trust(
        package_id,
        source,
        None,
        effective_trust,
        allowed_effects,
        None,
    )
}

/// Test fixture: an [`AdminEntry`] with an explicit digest pin.
pub fn admin_entry_with_digest_for_test(
    package_id: PackageId,
    source: PackageSource,
    digest: String,
    effective_trust: EffectiveTrustClass,
    allowed_effects: Vec<EffectKind>,
) -> AdminEntry {
    admin_entry_with_trust(
        package_id,
        source,
        Some(digest),
        effective_trust,
        allowed_effects,
        None,
    )
}
