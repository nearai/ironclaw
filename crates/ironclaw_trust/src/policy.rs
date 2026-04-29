//! Trust policy evaluation surface.
//!
//! [`TrustPolicy`] turns an untrusted [`TrustPolicyInput`] (manifest identity +
//! requested trust + requested authority) into a host-controlled
//! [`TrustDecision`]. [`HostTrustPolicy`] is the default implementation: it
//! consults a list of [`PolicySource`]s in order; the first source that
//! recognizes the package identity assigns the effective trust. If no source
//! matches, the policy falls through to a non-privileged default.

use std::collections::BTreeSet;

use ironclaw_host_api::{
    CapabilityId, EffectKind, PackageId, PackageIdentity, RequestedTrustClass, ResourceCeiling,
};

use crate::clock::{Clock, SystemClock};
use crate::decision::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};
use crate::error::TrustError;
use crate::invalidation::{InvalidationBus, TrustChange};
use crate::sources::{
    AdminConfig, AdminEntry, BundledEntry, BundledRegistry, PolicySource, SignedRegistry,
    SignerEntry,
};

/// Untrusted input to the policy engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustPolicyInput {
    pub identity: PackageIdentity,
    pub requested_trust: RequestedTrustClass,
    /// Set of capabilities the package is requesting authority over.
    /// Typed as `BTreeSet` (not `Vec`) so the policy engine sees a
    /// canonicalized set — capability authority is conceptually a set,
    /// not a multiset, and `[a, a, b]` should never differ from `[a, b]`.
    pub requested_authority: BTreeSet<CapabilityId>,
}

/// The host trust policy contract.
pub trait TrustPolicy: Send + Sync {
    fn evaluate(&self, input: &TrustPolicyInput) -> Result<TrustDecision, TrustError>;
}

/// What a [`PolicySource`] says about a package.
///
/// `None` means "this source does not recognize the package" — the policy
/// engine moves on to the next source. `Some` is binding for that source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceMatch {
    pub effective_trust: EffectiveTrustClass,
    pub provenance: TrustProvenance,
    pub allowed_effects: Vec<EffectKind>,
    /// Optional resource ceiling forwarded from the matching source's entry
    /// onto the resulting `AuthorityCeiling`. `None` means the source
    /// imposes no extra resource cap.
    pub max_resource_ceiling: Option<ResourceCeiling>,
}

/// Default host-controlled policy. Composes layered sources in priority order;
/// the first source returning `Some` wins. No source ⇒ non-privileged default.
///
/// The clock is injectable so policy evaluation is deterministic in tests
/// and audit-replay harnesses; production wiring uses [`SystemClock`].
pub struct HostTrustPolicy {
    sources: Vec<Box<dyn PolicySource>>,
    clock: Box<dyn Clock>,
}

impl HostTrustPolicy {
    /// Construct with a default `SystemClock`. Most production callers use
    /// this.
    pub fn new(sources: Vec<Box<dyn PolicySource>>) -> Self {
        Self {
            sources,
            clock: Box::new(SystemClock),
        }
    }

    pub fn empty() -> Self {
        Self::new(Vec::new())
    }

    /// Construct with an explicit clock. Tests inject `FixedClock` here so
    /// `evaluated_at` is reproducible across runs.
    pub fn with_clock(sources: Vec<Box<dyn PolicySource>>, clock: Box<dyn Clock>) -> Self {
        Self { sources, clock }
    }

    pub fn add_source(&mut self, source: Box<dyn PolicySource>) {
        self.sources.push(source);
    }
}

impl TrustPolicy for HostTrustPolicy {
    fn evaluate(&self, input: &TrustPolicyInput) -> Result<TrustDecision, TrustError> {
        let evaluated_at = self.clock.now();
        for source in &self.sources {
            if let Some(matched) = source.evaluate(input)? {
                return Ok(TrustDecision {
                    effective_trust: matched.effective_trust,
                    authority_ceiling: AuthorityCeiling {
                        allowed_effects: matched.allowed_effects,
                        max_resource_ceiling: matched.max_resource_ceiling,
                    },
                    provenance: matched.provenance,
                    evaluated_at,
                });
            }
        }

        Ok(default_decision(input, evaluated_at))
    }
}

impl HostTrustPolicy {
    /// Mutate one or more policy sources atomically with respect to the
    /// trust-change invalidation contract (AC #6).
    ///
    /// The orchestration:
    ///
    /// 1. Evaluate `affected_identity` against the current chain to capture
    ///    the *previous* effective trust.
    /// 2. Run the closure with [`SourceMutators`] handles. Inside the closure
    ///    the caller can `bundled_upsert` / `admin_remove` / etc. — these are
    ///    the only public path to the per-source `pub(crate)` mutators.
    /// 3. Re-evaluate `affected_identity`.
    /// 4. If the effective trust class changed, publish a [`TrustChange`] on
    ///    `bus` synchronously so listeners observe it before any subsequent
    ///    `evaluate()` returns the new decision.
    ///
    /// Closures that don't actually change `affected_identity`'s effective
    /// trust produce no publish — the bus is only notified on real
    /// downgrades/upgrades. Closures that *do* change it cannot bypass the
    /// publish, because the orchestration is hard-wired into this method.
    /// That's the whole point: AC #6 becomes a compile-time guarantee.
    ///
    /// `previous_authority` is forwarded onto the published `TrustChange`
    /// so listeners know which grant set to invalidate. `requested_trust` is
    /// the same axis the caller would use for an ordinary `evaluate` — kept
    /// stable across the pre/post evaluations so we measure only the
    /// mutation's effect.
    ///
    /// Returns the closure's result.
    ///
    /// Error semantics:
    /// - **Pre-mutation evaluate failure**: returned before any source is
    ///   touched. No mutation, no publish.
    /// - **Closure error**: returned via `?` from the closure short-circuits
    ///   the orchestration. Any partial mutation the closure performed
    ///   before erroring stays in place (the registry mutators are
    ///   in-place inserts/removes), but no `TrustChange` is published —
    ///   callers needing rollback must handle it inside the closure.
    /// - **Post-mutation evaluate failure**: surfaced to the caller after
    ///   the mutation has already happened. No publish — the caller is
    ///   responsible for any recovery.
    pub fn mutate_with<F, R>(
        &self,
        bus: &InvalidationBus,
        affected_identity: PackageIdentity,
        previous_authority: BTreeSet<CapabilityId>,
        requested_trust: RequestedTrustClass,
        f: F,
    ) -> Result<R, TrustError>
    where
        F: FnOnce(&SourceMutators<'_>) -> Result<R, TrustError>,
    {
        let probe = TrustPolicyInput {
            identity: affected_identity.clone(),
            requested_trust,
            requested_authority: previous_authority.clone(),
        };
        let prev = self.evaluate(&probe)?;

        let mutators = SourceMutators {
            sources: &self.sources,
        };
        let result = f(&mutators)?;

        let curr = self.evaluate(&probe)?;

        // `TrustChange::new` returns `None` for no-ops (prev == curr) — that
        // is the canonical filter, so a closure that mutates without
        // changing this identity's effective trust class produces no
        // publish.
        if let Some(change) = TrustChange::new(
            affected_identity,
            prev.effective_trust,
            curr.effective_trust,
            previous_authority,
            curr.evaluated_at,
        ) {
            bus.publish(change);
        }
        Ok(result)
    }
}

/// Typed mutator handles handed to the [`HostTrustPolicy::mutate_with`]
/// closure.
///
/// The per-source `upsert` / `remove` methods on `BundledRegistry`,
/// `AdminConfig`, and `SignedRegistry` are `pub(crate)`; the only public
/// way to reach them at runtime is through this struct, which itself is
/// only constructible inside `mutate_with`. That construction-by-position
/// means runtime mutation cannot happen without the surrounding
/// pre-evaluate/post-evaluate/publish dance.
///
/// If the policy chain doesn't contain a source of the requested kind,
/// the helper returns [`TrustError::InvariantViolation`] with the missing
/// type spelled out — wiring a `mutate_with` closure that mutates a
/// source the chain doesn't have is a configuration bug, not a silent
/// no-op.
pub struct SourceMutators<'a> {
    sources: &'a [Box<dyn PolicySource>],
}

impl<'a> SourceMutators<'a> {
    fn find<T: PolicySource + 'static>(&self) -> Result<&'a T, TrustError> {
        self.sources
            .iter()
            .find_map(|s| s.as_any().downcast_ref::<T>())
            .ok_or_else(|| TrustError::InvariantViolation {
                reason: format!(
                    "policy chain does not contain a source of type `{}`",
                    std::any::type_name::<T>()
                ),
            })
    }

    /// Insert or replace a [`BundledEntry`] in the chain's
    /// `BundledRegistry`.
    pub fn bundled_upsert(&self, entry: BundledEntry) -> Result<(), TrustError> {
        self.find::<BundledRegistry>()?.upsert(entry);
        Ok(())
    }

    /// Remove a [`BundledEntry`] from the chain's `BundledRegistry`,
    /// returning the previous value if any.
    pub fn bundled_remove(
        &self,
        package_id: &PackageId,
    ) -> Result<Option<BundledEntry>, TrustError> {
        Ok(self.find::<BundledRegistry>()?.remove(package_id))
    }

    /// Insert or replace an [`AdminEntry`] in the chain's `AdminConfig`.
    pub fn admin_upsert(&self, entry: AdminEntry) -> Result<(), TrustError> {
        self.find::<AdminConfig>()?.upsert(entry);
        Ok(())
    }

    /// Remove an [`AdminEntry`] from the chain's `AdminConfig`, returning
    /// the previous value if any.
    pub fn admin_remove(&self, package_id: &PackageId) -> Result<Option<AdminEntry>, TrustError> {
        Ok(self.find::<AdminConfig>()?.remove(package_id))
    }

    /// Insert or replace a [`SignerEntry`] in the chain's
    /// `SignedRegistry`. Note: the source itself is currently inert —
    /// this is the staging path future signature-verification work will
    /// consume.
    pub fn signed_upsert(&self, entry: SignerEntry) -> Result<(), TrustError> {
        self.find::<SignedRegistry>()?.upsert(entry);
        Ok(())
    }

    /// Remove a trusted signer from the chain's `SignedRegistry`,
    /// returning the previous entry if any.
    pub fn signed_remove(&self, signer: &str) -> Result<Option<SignerEntry>, TrustError> {
        Ok(self.find::<SignedRegistry>()?.remove(signer))
    }
}

/// Fallback decision when no policy source recognizes the package.
///
/// **All unmatched origins fall to `Sandbox`.** The earlier shape of this
/// function granted `UserTrusted` to unmatched `Bundled`, `Registry`, and
/// `Admin` packages on the theory that those origins were "capable of
/// being host-blessed" — but that's fail-open in two specific ways:
///
/// - `Registry { url }` is a remote source. Until signature verification
///   ships in [`crate::SignedRegistry`] (currently inert), nothing
///   authenticates the `url` value or the bytes it claims to identify.
///   Granting `UserTrusted` on the basis of an unverified self-declared
///   origin string is the textbook fail-open shape in a security-critical
///   surface.
/// - `Bundled` "compiled into the host binary" reaching this path means
///   the package didn't make it into [`crate::BundledRegistry`]. That's a
///   host-config bug (the catalog is out of sync with the binary), not a
///   runtime case warranting silent third-party authority.
/// - `Admin` reaching this path means an operator declared the package
///   without a matching `AdminConfig` entry — a similar misconfiguration.
///
/// Loud detection of "Bundled package missing from registry" belongs in a
/// startup audit that compares the registry against the compiled-in
/// package list (out of scope here). At policy evaluation time, the right
/// answer for "no source vouched for this" is uniform: no authority.
fn default_decision(
    _input: &TrustPolicyInput,
    evaluated_at: ironclaw_host_api::Timestamp,
) -> TrustDecision {
    TrustDecision {
        effective_trust: EffectiveTrustClass::sandbox(),
        authority_ceiling: AuthorityCeiling::empty(),
        provenance: TrustProvenance::Default,
        evaluated_at,
    }
}
