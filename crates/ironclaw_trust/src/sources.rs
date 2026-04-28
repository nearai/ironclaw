//! Layered policy sources.
//!
//! [`PolicySource`] is the extension point for host-controlled trust
//! assignment. PR1b ships in-memory [`BundledRegistry`] and [`AdminConfig`]
//! sources; signed-registry verification is intentionally a stub
//! ([`SignedRegistry`]) — the interface is here so PR3 can plug in real
//! signature verification without changing call sites.

use std::collections::HashMap;
use std::sync::RwLock;

use ironclaw_host_api::{EffectKind, PackageId, PackageSource};

use crate::decision::{EffectiveTrustClass, TrustProvenance};
use crate::error::TrustError;
use crate::policy::{SourceMatch, TrustPolicyInput};

/// Contract for a single policy source.
///
/// Returning `Ok(None)` means "this source does not recognize the package"
/// — the policy engine continues to the next source. `Ok(Some)` is binding.
/// `Err` is reserved for real evaluation failures (corrupt config, signature
/// verification error); a "this source did not match" outcome must always be
/// `Ok(None)`.
pub trait PolicySource: Send + Sync {
    fn name(&self) -> &'static str;
    fn evaluate(&self, input: &TrustPolicyInput) -> Result<Option<SourceMatch>, TrustError>;
}

/// One entry in the bundled trust registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundledEntry {
    pub package_id: PackageId,
    /// Optional digest pin. When set, the entry only matches packages whose
    /// `PackageIdentity::digest` is `Some` and equals this value — digest
    /// drift forces grant reissue per AC #7.
    pub digest: Option<String>,
    /// Effective trust this entry grants. The constructor accepts an
    /// `EffectiveTrustClass` so only crate-internal callers (or test
    /// fixtures) can stage privileged entries.
    pub effective_trust: EffectiveTrustClass,
    /// Effects the entry permits to be granted. Trust class alone grants
    /// nothing; downstream authorization must intersect this with each
    /// proposed `CapabilityGrant`'s effect list.
    pub allowed_effects: Vec<EffectKind>,
}

/// Compiled-in / signed-bundled package registry.
///
/// Only `PackageSource::Bundled` packages are evaluated by this source — a
/// `LocalManifest` package matching by ID alone gets `Ok(None)` so it falls
/// through to the next source (or the default downgrade).
pub struct BundledRegistry {
    entries: RwLock<HashMap<PackageId, BundledEntry>>,
}

impl BundledRegistry {
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
        }
    }

    pub fn with_entries<I: IntoIterator<Item = BundledEntry>>(entries: I) -> Self {
        let map = entries
            .into_iter()
            .map(|entry| (entry.package_id.clone(), entry))
            .collect();
        Self {
            entries: RwLock::new(map),
        }
    }

    pub fn upsert(&self, entry: BundledEntry) {
        let mut entries = self
            .entries
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        entries.insert(entry.package_id.clone(), entry);
    }

    pub fn remove(&self, package_id: &PackageId) -> Option<BundledEntry> {
        let mut entries = self
            .entries
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        entries.remove(package_id)
    }
}

impl Default for BundledRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl PolicySource for BundledRegistry {
    fn name(&self) -> &'static str {
        "bundled"
    }

    fn evaluate(&self, input: &TrustPolicyInput) -> Result<Option<SourceMatch>, TrustError> {
        if !matches!(input.identity.source, PackageSource::Bundled) {
            return Ok(None);
        }
        let entries = self
            .entries
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(entry) = entries.get(&input.identity.package_id) else {
            return Ok(None);
        };
        // Digest pin: when the registry entry pins a digest, the package must
        // match it exactly. Drift fails the source match (returns None) so
        // the package falls through to default downgrade, which is exactly
        // the AC #7 grant-reissue trigger.
        if let Some(pinned) = entry.digest.as_deref() {
            match input.identity.digest.as_deref() {
                Some(actual) if actual == pinned => {}
                _ => return Ok(None),
            }
        }
        Ok(Some(SourceMatch {
            effective_trust: entry.effective_trust,
            provenance: TrustProvenance::Bundled,
            allowed_effects: entry.allowed_effects.clone(),
        }))
    }
}

/// Operator/admin trust configuration. Same shape as bundled but distinct
/// provenance and intended for sources outside any user-controllable file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminEntry {
    pub package_id: PackageId,
    pub effective_trust: EffectiveTrustClass,
    pub allowed_effects: Vec<EffectKind>,
}

pub struct AdminConfig {
    entries: RwLock<HashMap<PackageId, AdminEntry>>,
}

impl AdminConfig {
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
        }
    }

    pub fn with_entries<I: IntoIterator<Item = AdminEntry>>(entries: I) -> Self {
        let map = entries
            .into_iter()
            .map(|entry| (entry.package_id.clone(), entry))
            .collect();
        Self {
            entries: RwLock::new(map),
        }
    }

    pub fn upsert(&self, entry: AdminEntry) {
        let mut entries = self
            .entries
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        entries.insert(entry.package_id.clone(), entry);
    }
}

impl Default for AdminConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl PolicySource for AdminConfig {
    fn name(&self) -> &'static str {
        "admin_config"
    }

    fn evaluate(&self, input: &TrustPolicyInput) -> Result<Option<SourceMatch>, TrustError> {
        // AdminConfig matches any source — operators may elevate a package
        // installed from any origin.
        let entries = self
            .entries
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(entry) = entries.get(&input.identity.package_id) else {
            return Ok(None);
        };
        Ok(Some(SourceMatch {
            effective_trust: entry.effective_trust,
            provenance: TrustProvenance::AdminConfig,
            allowed_effects: entry.allowed_effects.clone(),
        }))
    }
}

/// Stub source for signed remote registry entries.
///
/// PR1b only encodes the interface — real signature verification belongs to
/// a later PR. This stub is intentionally non-matching so that wiring it
/// into a `HostTrustPolicy` does not silently grant trust.
pub struct SignedRegistry;

impl PolicySource for SignedRegistry {
    fn name(&self) -> &'static str {
        "signed_registry"
    }

    fn evaluate(&self, _input: &TrustPolicyInput) -> Result<Option<SourceMatch>, TrustError> {
        Ok(None)
    }
}

/// Constructors for fixture-style privileged entries used by tests. See
/// [`crate::fixtures`] for the public, hidden-from-docs surface that
/// integration tests use; this internal helper takes the
/// [`EffectiveTrustClass`] directly.
pub(crate) fn bundled_entry_with_trust(
    package_id: PackageId,
    digest: Option<String>,
    effective_trust: EffectiveTrustClass,
    allowed_effects: Vec<EffectKind>,
) -> BundledEntry {
    BundledEntry {
        package_id,
        digest,
        effective_trust,
        allowed_effects,
    }
}

pub(crate) fn admin_entry_with_trust(
    package_id: PackageId,
    effective_trust: EffectiveTrustClass,
    allowed_effects: Vec<EffectKind>,
) -> AdminEntry {
    AdminEntry {
        package_id,
        effective_trust,
        allowed_effects,
    }
}
