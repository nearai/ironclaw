//! Layered policy sources.
//!
//! [`PolicySource`] is the extension point for host-controlled trust
//! assignment. PR1b ships in-memory [`BundledRegistry`] and [`AdminConfig`]
//! sources; [`SignedRegistry`] and [`LocalDevOverride`] are interface seams
//! that real signature verification / dev-tool overrides will fill in
//! later, but they expose enough shape that downstream wiring can target a
//! stable surface today.
//!
//! ## Mutation and invalidation
//!
//! `BundledRegistry` and `AdminConfig` expose synchronous `upsert` /
//! `remove` methods that mutate in place. Per AC #6 of issue #3012, any
//! mutation that *lowers* a previously-effective trust class must publish a
//! [`crate::TrustChange`] on an [`crate::InvalidationBus`] **before** any
//! subsequent dispatch can run under the stale ceiling. The mutators here
//! intentionally do not own a bus reference — the caller is the only place
//! that knows the previous decision and the current authority list, so the
//! caller is the only place that can build a faithful `TrustChange`.
//!
//! Standard pattern:
//!
//! 1. Call `evaluate` (or otherwise capture the previous `TrustDecision`)
//!    before mutating.
//! 2. Mutate (`upsert` / `remove`).
//! 3. Build a `TrustChange` and call `bus.publish(change)`.
//!
//! Skipping step 3 silently violates AC #6. PR3's grant-store wiring will
//! own this orchestration; for PR1b the contract is documented and exercised
//! by tests T7 / T8.

use std::collections::HashMap;
use std::sync::RwLock;

use ironclaw_host_api::{EffectKind, PackageId, PackageSource, ResourceCeiling};

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
    /// Optional ceiling on resource budgets the entry may unlock. Forwarded
    /// to `AuthorityCeiling::max_resource_ceiling` on match. `None` means
    /// the entry imposes no extra resource cap beyond what the host policy
    /// already enforces elsewhere.
    pub max_resource_ceiling: Option<ResourceCeiling>,
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

    /// Insert or replace an entry. **Caller must publish a `TrustChange` on
    /// the relevant `InvalidationBus` if this mutation lowers an already-
    /// active decision** — see the module docs for the orchestration
    /// pattern.
    pub fn upsert(&self, entry: BundledEntry) {
        let mut entries = self
            .entries
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        entries.insert(entry.package_id.clone(), entry);
    }

    /// Remove an entry by id, returning the previous value if any. **Caller
    /// must publish a `TrustChange` on the relevant `InvalidationBus`
    /// before any further dispatch** — removing an entry typically lowers
    /// effective trust, which is exactly the case AC #6 requires
    /// fail-closed handling for.
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
            max_resource_ceiling: entry.max_resource_ceiling.clone(),
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
    pub max_resource_ceiling: Option<ResourceCeiling>,
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

    /// Insert or replace an entry. **Caller must publish a `TrustChange` on
    /// the relevant `InvalidationBus` if this mutation lowers an already-
    /// active decision** — see the module docs.
    pub fn upsert(&self, entry: AdminEntry) {
        let mut entries = self
            .entries
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        entries.insert(entry.package_id.clone(), entry);
    }

    /// Remove an entry by id, returning the previous value if any. **Caller
    /// must publish a `TrustChange` on the relevant `InvalidationBus`
    /// before any further dispatch** — removing an admin grant typically
    /// downgrades trust, AC #6.
    pub fn remove(&self, package_id: &PackageId) -> Option<AdminEntry> {
        let mut entries = self
            .entries
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        entries.remove(package_id)
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
            max_resource_ceiling: entry.max_resource_ceiling.clone(),
        }))
    }
}

/// Verified-signer entry, keyed by signer identity (e.g., a public-key
/// fingerprint or an SPKI hash). PR1b only declares the shape; real
/// signature verification belongs to a follow-up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignerEntry {
    /// Stable signer identifier. Compared against `PackageIdentity::signer`
    /// when verification logic lands.
    pub signer: String,
    /// Optional human-readable label for audit/logging.
    pub label: Option<String>,
    /// Effective trust to grant matched packages. Privileged values can only
    /// be staged through the test fixtures or future host-controlled
    /// signing infrastructure.
    pub effective_trust: EffectiveTrustClass,
    pub allowed_effects: Vec<EffectKind>,
    pub max_resource_ceiling: Option<ResourceCeiling>,
}

/// Signed-registry source — interface seam for future signature
/// verification.
///
/// Today this is intentionally non-functional: even with `trusted_signers`
/// populated, `evaluate` returns `Ok(None)` because no verification path
/// exists yet. PR1b ships the data shape so callers can stage signers
/// against a stable interface; a follow-up will fill in the actual
/// signature check.
pub struct SignedRegistry {
    trusted_signers: RwLock<HashMap<String, SignerEntry>>,
}

impl SignedRegistry {
    pub fn new() -> Self {
        Self {
            trusted_signers: RwLock::new(HashMap::new()),
        }
    }

    pub fn with_signers<I: IntoIterator<Item = SignerEntry>>(signers: I) -> Self {
        let map = signers
            .into_iter()
            .map(|entry| (entry.signer.clone(), entry))
            .collect();
        Self {
            trusted_signers: RwLock::new(map),
        }
    }

    /// Insert or replace a trusted-signer entry. **Same publish-on-mutation
    /// contract as `BundledRegistry::upsert`.**
    pub fn upsert(&self, entry: SignerEntry) {
        let mut entries = self
            .trusted_signers
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        entries.insert(entry.signer.clone(), entry);
    }

    /// Remove a trusted signer. **Caller must publish a `TrustChange`** —
    /// removing a signer revokes all packages that were trusted via that
    /// signer.
    pub fn remove(&self, signer: &str) -> Option<SignerEntry> {
        let mut entries = self
            .trusted_signers
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        entries.remove(signer)
    }
}

impl Default for SignedRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl PolicySource for SignedRegistry {
    fn name(&self) -> &'static str {
        "signed_registry"
    }

    fn evaluate(&self, _input: &TrustPolicyInput) -> Result<Option<SourceMatch>, TrustError> {
        // Verification path not yet implemented. Returning `Ok(None)` is the
        // safe default — packages flow to the next source / default
        // downgrade rather than being trusted on the basis of a self-
        // declared `signer` field.
        Ok(None)
    }
}

/// Local development override — interface seam for an opt-in,
/// administratively-blessed dev mode that lets a developer mark specific
/// local packages as privileged for testing.
///
/// PR1b ships only the shape. The future implementation will require
/// explicit configuration (e.g., a CLI flag or a config file outside any
/// user-writable location) and audit logging on every match. Without that
/// configuration the source is inert.
pub struct LocalDevOverride {
    // Future shape — the structural seam future PRs will fill in.
    // `#[allow(dead_code)]` is intentional in PR1b: callers can already
    // observe `LocalDevOverride` as a `PolicySource` and stage it in their
    // policy chain, while the interior intentionally has no read path. The
    // alternative (omitting the fields entirely) would force a breaking
    // shape change later.
    /// Packages the operator has explicitly opted in for elevated trust in
    /// development. Empty means the source is fully inert.
    #[allow(dead_code)]
    overrides: RwLock<HashMap<PackageId, AdminEntry>>,
    /// When `false`, even configured overrides are ignored. PR1b
    /// initialises this to `false`; future config wiring must set it to
    /// `true` only when the operator has explicitly opted in *and* an
    /// auditor is recording the activation.
    enabled: bool,
}

impl LocalDevOverride {
    /// Construct an inert `LocalDevOverride`. PR1b has no production opt-in
    /// path — the source is documented as future-compatible and never
    /// matches.
    pub fn inert() -> Self {
        Self {
            overrides: RwLock::new(HashMap::new()),
            enabled: false,
        }
    }
}

impl Default for LocalDevOverride {
    fn default() -> Self {
        Self::inert()
    }
}

impl PolicySource for LocalDevOverride {
    fn name(&self) -> &'static str {
        "local_dev_override"
    }

    fn evaluate(&self, _input: &TrustPolicyInput) -> Result<Option<SourceMatch>, TrustError> {
        // Reserved for future implementation. Even when `enabled` flips to
        // true, the lookup against `overrides` is intentionally absent in
        // PR1b — keeping the inert path explicit avoids accidentally
        // wiring trust through a half-implemented mechanism.
        if !self.enabled {
            return Ok(None);
        }
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
    max_resource_ceiling: Option<ResourceCeiling>,
) -> BundledEntry {
    BundledEntry {
        package_id,
        digest,
        effective_trust,
        allowed_effects,
        max_resource_ceiling,
    }
}

pub(crate) fn admin_entry_with_trust(
    package_id: PackageId,
    effective_trust: EffectiveTrustClass,
    allowed_effects: Vec<EffectKind>,
    max_resource_ceiling: Option<ResourceCeiling>,
) -> AdminEntry {
    AdminEntry {
        package_id,
        effective_trust,
        allowed_effects,
        max_resource_ceiling,
    }
}
