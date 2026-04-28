//! Test-only fixture constructors. **Not for production use.**
//!
//! This module is `#[doc(hidden)]` because it is the deliberate escape
//! hatch that integration tests use to stage privileged
//! [`EffectiveTrustClass`] entries. Production code MUST construct
//! `EffectiveTrustClass` only via [`crate::TrustPolicy::evaluate`]; using
//! these helpers anywhere outside `tests/` is a code-review failure.
//!
//! Naming intentionally signals the boundary (`*_for_test`). PR review
//! should reject any non-test caller.

use ironclaw_host_api::{EffectKind, PackageId};

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
    bundled_entry_with_trust(package_id, digest, effective_trust, allowed_effects)
}

/// Test fixture: an [`AdminEntry`] at the given effective trust ceiling.
pub fn admin_entry_for_test(
    package_id: PackageId,
    effective_trust: EffectiveTrustClass,
    allowed_effects: Vec<EffectKind>,
) -> AdminEntry {
    admin_entry_with_trust(package_id, effective_trust, allowed_effects)
}
