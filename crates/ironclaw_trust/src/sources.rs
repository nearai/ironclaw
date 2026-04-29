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
//! `BundledRegistry`, `AdminConfig`, and `SignedRegistry` mutate in place
//! via `upsert` / `remove`. Per AC #6 of issue #3012, any mutation that
//! lowers effective trust or shrinks the authority ceiling must publish a
//! [`crate::TrustChange`] on an [`crate::InvalidationBus`] **before** any
//! subsequent dispatch can run under the stale ceiling.
//!
//! Computing "previous" effective trust requires evaluating the *whole*
//! policy chain — not just the source being mutated — so the orchestration
//! cannot live on a single source. Instead it lives on
//! [`crate::HostTrustPolicy::mutate_with`], which is the **only public
//! runtime-mutation path**: the per-source `upsert` / `remove` methods are
//! `pub(crate)` and reachable only through
//! [`crate::SourceMutators`] inside a `mutate_with` closure. That call
//! pre-evaluates, stages fallible mutations, commits only after the closure
//! succeeds, post-evaluates, and publishes a `TrustChange` synchronously if
//! trust changed or the authority ceiling shrank — making AC #6 a
//! compile-time guarantee rather than a doc-comment convention.
//!
//! Construction-time population (the `with_entries` / `with_signers`
//! constructors below) remains `pub` because no policy state exists for an
//! invalidation to be meaningful against — those constructors are for
//! seeding the chain *before* it is wired up to a bus.

use std::any::Any;
use std::collections::HashMap;
use std::sync::RwLock;

use ironclaw_host_api::{EffectKind, PackageId, PackageSource, ResourceCeiling};

use crate::decision::{EffectiveTrustClass, HostTrustAssignment, TrustProvenance};
use crate::error::TrustError;
use crate::policy::{SourceMatch, TrustPolicyInput};

/// Contract for a single policy source.
///
/// Returning `Ok(None)` means "this source does not recognize the package"
/// — the policy engine continues to the next source. `Ok(Some)` is binding.
/// `Err` is reserved for real evaluation failures (corrupt config, signature
/// verification error); a "this source did not match" outcome must always be
/// `Ok(None)`.
///
/// The `as_any` hook lets [`crate::SourceMutators`] downcast a
/// `&dyn PolicySource` back to its concrete type (`BundledRegistry`,
/// `AdminConfig`, etc.) so a `mutate_with` closure can call the
/// crate-private mutators on the right source. Implementations should
/// return `self`.
pub trait PolicySource: Send + Sync + Any {
    fn name(&self) -> &'static str;
    fn evaluate(&self, input: &TrustPolicyInput) -> Result<Option<SourceMatch>, TrustError>;
    fn as_any(&self) -> &dyn Any;
}

/// One entry in the bundled trust registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundledEntry {
    pub package_id: PackageId,
    /// Optional digest pin. When set, the entry only matches packages whose
    /// `PackageIdentity::digest` is `Some` and equals this value — digest
    /// drift forces grant reissue per AC #7.
    pub digest: Option<String>,
    /// Effective trust this entry grants. Public constructors accept
    /// [`HostTrustAssignment`] so host-controlled bundle/admin loaders can
    /// stage privileged entries without exposing raw effective-trust
    /// constructors.
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

impl BundledEntry {
    /// Construct a bundled policy entry from host-controlled bundle metadata.
    pub fn new(
        package_id: PackageId,
        digest: Option<String>,
        trust: HostTrustAssignment,
        allowed_effects: Vec<EffectKind>,
        max_resource_ceiling: Option<ResourceCeiling>,
    ) -> Self {
        Self {
            package_id,
            digest,
            effective_trust: trust.into_effective(),
            allowed_effects,
            max_resource_ceiling,
        }
    }
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

    /// Insert or replace an entry.
    ///
    /// `pub(crate)` because runtime mutation is only correct when wrapped
    /// in a [`crate::HostTrustPolicy::mutate_with`] call that publishes
    /// the consequent `TrustChange` on an `InvalidationBus`. Use
    /// [`Self::with_entries`] for construction-time population.
    pub(crate) fn upsert(&self, entry: BundledEntry) {
        let mut entries = self
            .entries
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        entries.insert(entry.package_id.clone(), entry);
    }

    /// Return an entry by id, without mutating it.
    pub(crate) fn get(&self, package_id: &PackageId) -> Option<BundledEntry> {
        let entries = self
            .entries
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        entries.get(package_id).cloned()
    }

    /// Remove an entry by id, returning the previous value if any.
    ///
    /// `pub(crate)` for the same reason as [`Self::upsert`]: runtime
    /// removal must go through `HostTrustPolicy::mutate_with` so the
    /// invalidation contract (AC #6) is honored automatically.
    pub(crate) fn remove(&self, package_id: &PackageId) -> Option<BundledEntry> {
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

    fn as_any(&self) -> &dyn Any {
        self
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

/// Operator/admin trust configuration.
///
/// Each entry binds an elevation to a *specific* `(package_id, source)` pair
/// — and optionally a digest — so a same-`package_id` package from a
/// less-trusted origin cannot inherit the elevation. The structural fix
/// closes the shadowing footgun documented in the PR3043 review: without
/// the source pin, an `AdminEntry` for a `Bundled` package would have also
/// matched a `LocalManifest` package with the same id, letting an
/// unprivileged user shadow an admin-blessed identifier into `FirstParty`.
///
/// Construct entries through the `AdminEntry::for_*` constructors. The
/// `for_local_manifest` constructor is named separately so that *every*
/// site that elevates a user-writable package is greppable for review.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminEntry {
    pub package_id: PackageId,
    /// Origin this entry binds to. The entry only matches packages whose
    /// `PackageIdentity::source` equals this value. Use the matching
    /// `for_bundled` / `for_registry` / `for_admin` / `for_local_manifest`
    /// constructor rather than building the variant by hand.
    pub source: PackageSource,
    /// Optional digest pin. When set, the entry only matches packages whose
    /// `PackageIdentity::digest` is `Some` and equals this value — drift
    /// falls through to the default downgrade per AC #7.
    pub digest: Option<String>,
    pub effective_trust: EffectiveTrustClass,
    pub allowed_effects: Vec<EffectKind>,
    pub max_resource_ceiling: Option<ResourceCeiling>,
}

impl AdminEntry {
    /// Bind an elevation to a `Bundled` package.
    pub fn for_bundled(
        package_id: PackageId,
        digest: Option<String>,
        trust: HostTrustAssignment,
        allowed_effects: Vec<EffectKind>,
        max_resource_ceiling: Option<ResourceCeiling>,
    ) -> Self {
        Self {
            package_id,
            source: PackageSource::Bundled,
            digest,
            effective_trust: trust.into_effective(),
            allowed_effects,
            max_resource_ceiling,
        }
    }

    /// Bind an elevation to a package fetched from a specific registry URL.
    /// The URL is part of the source-equality match — operators must spell
    /// out which registry they're trusting.
    pub fn for_registry(
        package_id: PackageId,
        registry_url: String,
        digest: Option<String>,
        trust: HostTrustAssignment,
        allowed_effects: Vec<EffectKind>,
        max_resource_ceiling: Option<ResourceCeiling>,
    ) -> Self {
        Self {
            package_id,
            source: PackageSource::Registry { url: registry_url },
            digest,
            effective_trust: trust.into_effective(),
            allowed_effects,
            max_resource_ceiling,
        }
    }

    /// Bind an elevation to a `PackageSource::Admin` declaration. No digest
    /// — the operator is the source.
    pub fn for_admin(
        package_id: PackageId,
        trust: HostTrustAssignment,
        allowed_effects: Vec<EffectKind>,
        max_resource_ceiling: Option<ResourceCeiling>,
    ) -> Self {
        Self {
            package_id,
            source: PackageSource::Admin,
            digest: None,
            effective_trust: trust.into_effective(),
            allowed_effects,
            max_resource_ceiling,
        }
    }

    /// Bind an elevation to a `LocalManifest` package at a specific path.
    ///
    /// **This is the highest-risk AdminEntry constructor.** A
    /// `LocalManifest` is a user-writable file; elevating one to `FirstParty`
    /// or `System` trusts whatever bytes that file contains at evaluation
    /// time. The constructor exists separately so every elevation of a
    /// user-writable package is `rg for_local_manifest` away. Pair with a
    /// digest pin when the elevation is meant to bind a specific known
    /// artifact rather than "whatever this manifest currently says".
    pub fn for_local_manifest(
        package_id: PackageId,
        manifest_path: String,
        digest: Option<String>,
        trust: HostTrustAssignment,
        allowed_effects: Vec<EffectKind>,
        max_resource_ceiling: Option<ResourceCeiling>,
    ) -> Self {
        Self {
            package_id,
            source: PackageSource::LocalManifest {
                path: manifest_path,
            },
            digest,
            effective_trust: trust.into_effective(),
            allowed_effects,
            max_resource_ceiling,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct AdminSubject {
    package_id: PackageId,
    source: PackageSource,
}

impl AdminSubject {
    pub(crate) fn new(package_id: PackageId, source: PackageSource) -> Self {
        Self { package_id, source }
    }

    fn from_entry(entry: &AdminEntry) -> Self {
        Self {
            package_id: entry.package_id.clone(),
            source: entry.source.clone(),
        }
    }
}

pub struct AdminConfig {
    entries: RwLock<HashMap<AdminSubject, AdminEntry>>,
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
            .map(|entry| (AdminSubject::from_entry(&entry), entry))
            .collect();
        Self {
            entries: RwLock::new(map),
        }
    }

    /// Insert or replace an entry. `pub(crate)` — runtime mutation must go
    /// through [`crate::HostTrustPolicy::mutate_with`].
    pub(crate) fn upsert(&self, entry: AdminEntry) {
        let mut entries = self
            .entries
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        entries.insert(AdminSubject::from_entry(&entry), entry);
    }

    /// Return an entry by full trust subject, without mutating it.
    pub(crate) fn get(&self, package_id: &PackageId, source: &PackageSource) -> Option<AdminEntry> {
        let entries = self
            .entries
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        entries
            .get(&AdminSubject::new(package_id.clone(), source.clone()))
            .cloned()
    }

    /// Remove an entry by full trust subject, returning the previous value if any.
    /// `pub(crate)` — runtime mutation must go through `mutate_with`.
    pub(crate) fn remove(
        &self,
        package_id: &PackageId,
        source: &PackageSource,
    ) -> Option<AdminEntry> {
        let mut entries = self
            .entries
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        entries.remove(&AdminSubject::new(package_id.clone(), source.clone()))
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

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn evaluate(&self, input: &TrustPolicyInput) -> Result<Option<SourceMatch>, TrustError> {
        let entries = self
            .entries
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(entry) = entries.get(&AdminSubject::new(
            input.identity.package_id.clone(),
            input.identity.source.clone(),
        )) else {
            return Ok(None);
        };
        // Digest pin: same drift semantics as `BundledRegistry` — when set,
        // the package's digest must match exactly. Drift falls through to
        // the default downgrade, which is the AC #7 grant-reissue trigger.
        if let Some(pinned) = entry.digest.as_deref() {
            match input.identity.digest.as_deref() {
                Some(actual) if actual == pinned => {}
                _ => return Ok(None),
            }
        }
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
    /// Effective trust to grant matched packages. Privileged values should
    /// be staged through crate-owned host-assignment constructors or future
    /// host-controlled signing infrastructure.
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
///
/// **Threat note for the future implementation.** The current `SignerEntry`
/// shape keys an `effective_trust` ceiling on `signer` alone — i.e., a
/// verified signature from signer X would, naively, vouch for *any*
/// package signed by X with whatever ceiling the entry declares. A
/// compromised signer key, or a signer that signs packages outside its
/// intended scope, would then escalate every such package. When this
/// source is wired up:
///
/// 1. The verification path must bind `(signer, package_id, digest)` —
///    a verified signature is evidence about *that artifact*, not about
///    the signer's general authority.
/// 2. `effective_trust` should be capped by what the host policy decides
///    is acceptable for the *package*, not what the entry asserts about
///    the *signer*.
/// 3. Reuse the `AdminEntry::for_*` constructor pattern: separate
///    constructors per `PackageSource` so the call sites that elevate
///    user-writable origins are greppable.
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

    /// Insert or replace a trusted-signer entry. `pub(crate)` — runtime
    /// mutation must go through [`crate::HostTrustPolicy::mutate_with`].
    pub(crate) fn upsert(&self, entry: SignerEntry) {
        let mut entries = self
            .trusted_signers
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        entries.insert(entry.signer.clone(), entry);
    }

    /// Return a trusted signer without mutating it.
    pub(crate) fn get(&self, signer: &str) -> Option<SignerEntry> {
        let entries = self
            .trusted_signers
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        entries.get(signer).cloned()
    }

    /// Remove a trusted signer. `pub(crate)` — runtime mutation must go
    /// through `mutate_with`.
    pub(crate) fn remove(&self, signer: &str) -> Option<SignerEntry> {
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

    fn as_any(&self) -> &dyn Any {
        self
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
    /// Packages the operator has explicitly opted in for elevated trust in
    /// development. Empty means the source has nothing to evaluate even
    /// once the implementation lands.
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

    /// True when the source has been opted in. Always `false` in PR1b
    /// because no production opt-in path exists yet — the accessor is
    /// here so tests can pin the inert contract and so future
    /// implementations have a stable read surface.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Number of staged override entries.
    pub fn override_count(&self) -> usize {
        self.overrides
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    /// Test-only: construct a `LocalDevOverride` that has been "enabled"
    /// and pre-staged with override entries.
    ///
    /// Even with this, [`PolicySource::evaluate`] returns `Ok(None)` —
    /// PR1b's contract is that the source is inert until the future
    /// dev-override implementation lands. The fixture exists so tests
    /// can pin that inert contract: enabling + staging overrides must
    /// not produce trust under any input. It is compiled only for this
    /// crate's `#[cfg(test)]` targets and is not exposed by any Cargo
    /// feature.
    #[cfg(test)]
    pub fn enabled_for_test(entries: Vec<(PackageId, AdminEntry)>) -> Self {
        let map: HashMap<_, _> = entries.into_iter().collect();
        Self {
            overrides: RwLock::new(map),
            enabled: true,
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

    fn as_any(&self) -> &dyn Any {
        self
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

/// Constructors for crate-internal privileged test entries. These helpers
/// take [`EffectiveTrustClass`] directly but are compiled only for this
/// crate's `#[cfg(test)]` targets.
#[cfg(test)]
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

#[cfg(test)]
pub(crate) fn admin_entry_with_trust(
    package_id: PackageId,
    source: PackageSource,
    digest: Option<String>,
    effective_trust: EffectiveTrustClass,
    allowed_effects: Vec<EffectKind>,
    max_resource_ceiling: Option<ResourceCeiling>,
) -> AdminEntry {
    AdminEntry {
        package_id,
        source,
        digest,
        effective_trust,
        allowed_effects,
        max_resource_ceiling,
    }
}
