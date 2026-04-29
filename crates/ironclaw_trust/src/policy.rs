//! Trust policy evaluation surface.
//!
//! [`TrustPolicy`] turns an untrusted [`TrustPolicyInput`] (manifest identity +
//! requested trust + requested authority) into a host-controlled
//! [`TrustDecision`]. [`HostTrustPolicy`] is the default implementation: it
//! consults a list of [`PolicySource`]s in order; the first source that
//! recognizes the package identity assigns the effective trust. If no source
//! matches, the policy falls through to a non-privileged default.

use std::any::TypeId;
use std::cell::RefCell;
use std::collections::{BTreeSet, HashSet};
use std::sync::RwLock;

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

thread_local! {
    static PUBLISHING_POLICIES: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
}

struct PublishReentryGuard {
    key: usize,
}

impl PublishReentryGuard {
    fn enter(policy: &HostTrustPolicy) -> Self {
        let key = policy.reentry_key();
        PUBLISHING_POLICIES.with(|policies| policies.borrow_mut().push(key));
        Self { key }
    }
}

impl Drop for PublishReentryGuard {
    fn drop(&mut self) {
        PUBLISHING_POLICIES.with(|policies| {
            let mut policies = policies.borrow_mut();
            if let Some(pos) = policies.iter().rposition(|key| *key == self.key) {
                policies.remove(pos);
            }
        });
    }
}

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
    mutation_gate: RwLock<()>,
}

impl HostTrustPolicy {
    /// Construct with a default `SystemClock`. Most production callers use
    /// this. Duplicate source types are rejected because mutation routing is
    /// type-directed.
    pub fn new(sources: Vec<Box<dyn PolicySource>>) -> Result<Self, TrustError> {
        Self::with_clock(sources, Box::new(SystemClock))
    }

    pub fn empty() -> Self {
        Self::from_parts_unchecked(Vec::new(), Box::new(SystemClock))
    }

    /// Construct with an explicit clock. Tests inject `FixedClock` here so
    /// `evaluated_at` is reproducible across runs.
    pub fn with_clock(
        sources: Vec<Box<dyn PolicySource>>,
        clock: Box<dyn Clock>,
    ) -> Result<Self, TrustError> {
        ensure_unique_source_types(&sources)?;
        Ok(Self::from_parts_unchecked(sources, clock))
    }

    fn from_parts_unchecked(sources: Vec<Box<dyn PolicySource>>, clock: Box<dyn Clock>) -> Self {
        Self {
            sources,
            clock,
            mutation_gate: RwLock::new(()),
        }
    }

    pub fn add_source(&mut self, source: Box<dyn PolicySource>) -> Result<(), TrustError> {
        ensure_source_type_absent(&self.sources, source.as_ref())?;
        self.sources.push(source);
        Ok(())
    }

    fn reentry_key(&self) -> usize {
        self as *const Self as usize
    }

    fn is_publish_reentrant(&self) -> bool {
        let key = self.reentry_key();
        PUBLISHING_POLICIES.with(|policies| policies.borrow().contains(&key))
    }
}

fn ensure_unique_source_types(sources: &[Box<dyn PolicySource>]) -> Result<(), TrustError> {
    let mut seen = HashSet::<TypeId>::new();
    for source in sources {
        let type_id = source.as_any().type_id();
        if !seen.insert(type_id) {
            return Err(TrustError::InvariantViolation {
                reason: format!(
                    "policy chain contains duplicate source type `{}`",
                    source.name()
                ),
            });
        }
    }
    Ok(())
}

fn ensure_source_type_absent(
    sources: &[Box<dyn PolicySource>],
    candidate: &dyn PolicySource,
) -> Result<(), TrustError> {
    let candidate_type = candidate.as_any().type_id();
    if sources
        .iter()
        .any(|source| source.as_any().type_id() == candidate_type)
    {
        return Err(TrustError::InvariantViolation {
            reason: format!(
                "policy chain already contains source type `{}`",
                candidate.name()
            ),
        });
    }
    Ok(())
}

impl TrustPolicy for HostTrustPolicy {
    fn evaluate(&self, input: &TrustPolicyInput) -> Result<TrustDecision, TrustError> {
        if self.is_publish_reentrant() {
            return self.evaluate_unlocked(input);
        }
        let _guard = self
            .mutation_gate
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.evaluate_unlocked(input)
    }
}

impl HostTrustPolicy {
    fn evaluate_unlocked(&self, input: &TrustPolicyInput) -> Result<TrustDecision, TrustError> {
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
    /// The orchestration runs under a policy-level write gate:
    ///
    /// 1. Evaluate `affected_identity` against the current chain to capture
    ///    the *previous* decision.
    /// 2. Run the closure with [`SourceMutators`] handles. The handles stage
    ///    `bundled_upsert` / `admin_remove` / etc.; they do **not** mutate
    ///    live source state while the closure is still fallible.
    /// 3. If the closure returns an error, drop the staged mutations and
    ///    return that error. No lower decision became visible, so no publish
    ///    is needed.
    /// 4. Commit the staged mutations, re-evaluate `affected_identity`, and
    ///    publish a [`TrustChange`] if the effective trust class changed or
    ///    the authority ceiling shrank.
    /// 5. Release the policy gate only after synchronous bus publication
    ///    completes, so no concurrent `evaluate()` can observe the new lower
    ///    decision before listeners invalidate stale grants.
    ///
    /// Closures that don't actually change `affected_identity`'s effective
    /// trust or reduce its authority ceiling produce no publish. Closures
    /// that *do* change the invalidation-relevant decision cannot bypass the
    /// publish, because the orchestration is hard-wired into this method.
    /// That's the whole point: AC #6 becomes a compile-time guarantee.
    ///
    /// `requested_authority` is the same authority set the caller would use
    /// for an ordinary `evaluate` — kept stable across the pre/post
    /// evaluations so we measure only the mutation's effect. It is not
    /// forwarded to `TrustChange`; invalidation listeners must derive the
    /// grant-revocation scope from their own grant store and the decision
    /// delta, not from caller-supplied hints.
    ///
    /// Returns the closure's result.
    ///
    /// Error semantics:
    /// - **Pre-mutation evaluate failure**: returned before any source is
    ///   touched. No mutation, no publish.
    /// - **Closure error**: staged mutations are discarded, then the closure
    ///   error is returned. No mutation, no publish.
    /// - **Post-commit evaluate failure**: surfaced to the caller after the
    ///   mutation has already happened. This indicates corrupt policy state;
    ///   the policy gate stays held until the error is observed so another
    ///   evaluation cannot race ahead of the failed orchestration.
    pub fn mutate_with<F, R>(
        &self,
        bus: &InvalidationBus,
        affected_identity: PackageIdentity,
        requested_authority: BTreeSet<CapabilityId>,
        requested_trust: RequestedTrustClass,
        f: F,
    ) -> Result<R, TrustError>
    where
        F: FnOnce(&SourceMutators<'_>) -> Result<R, TrustError>,
    {
        let _gate = self
            .mutation_gate
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let probe = TrustPolicyInput {
            identity: affected_identity.clone(),
            requested_trust,
            requested_authority: requested_authority.clone(),
        };
        let prev = self.evaluate_unlocked(&probe)?;

        let mutators = SourceMutators {
            sources: &self.sources,
            staged: RefCell::new(Vec::new()),
        };
        let result = f(&mutators)?;
        mutators.commit()?;

        let curr = self.evaluate_unlocked(&probe)?;

        // `TrustChange::new` returns `None` for no-ops and for benign
        // authority-ceiling expansions; it returns `Some` when the trust
        // class changes or the authority ceiling shrinks.
        if let Some(change) = TrustChange::new(affected_identity, &prev, &curr) {
            let _reentry_guard = PublishReentryGuard::enter(self);
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
enum SourceMutation {
    BundledUpsert(BundledEntry),
    BundledRemove(PackageId),
    AdminUpsert(AdminEntry),
    AdminRemove {
        package_id: PackageId,
        source: ironclaw_host_api::PackageSource,
    },
    SignedUpsert(SignerEntry),
    SignedRemove(String),
}

pub struct SourceMutators<'a> {
    sources: &'a [Box<dyn PolicySource>],
    staged: RefCell<Vec<SourceMutation>>,
}

impl<'a> SourceMutators<'a> {
    fn find<T: PolicySource + 'static>(&self) -> Result<&'a T, TrustError> {
        let mut matches = self
            .sources
            .iter()
            .filter_map(|s| s.as_any().downcast_ref::<T>());
        let Some(first) = matches.next() else {
            return Err(TrustError::InvariantViolation {
                reason: format!(
                    "policy chain does not contain a source of type `{}`",
                    std::any::type_name::<T>()
                ),
            });
        };
        if matches.next().is_some() {
            return Err(TrustError::InvariantViolation {
                reason: format!(
                    "policy chain contains duplicate source type `{}`",
                    std::any::type_name::<T>()
                ),
            });
        }
        Ok(first)
    }

    /// Insert or replace a [`BundledEntry`] in the chain's
    /// `BundledRegistry`.
    pub fn bundled_upsert(&self, entry: BundledEntry) -> Result<(), TrustError> {
        self.find::<BundledRegistry>()?;
        self.staged
            .borrow_mut()
            .push(SourceMutation::BundledUpsert(entry));
        Ok(())
    }

    /// Remove a [`BundledEntry`] from the chain's `BundledRegistry`,
    /// returning the currently committed value if any. The removal itself
    /// is staged and is only committed if the enclosing `mutate_with`
    /// closure returns `Ok`.
    pub fn bundled_remove(
        &self,
        package_id: &PackageId,
    ) -> Result<Option<BundledEntry>, TrustError> {
        let registry = self.find::<BundledRegistry>()?;
        let previous = registry.get(package_id);
        self.staged
            .borrow_mut()
            .push(SourceMutation::BundledRemove(package_id.clone()));
        Ok(previous)
    }

    /// Insert or replace an [`AdminEntry`] in the chain's `AdminConfig`.
    pub fn admin_upsert(&self, entry: AdminEntry) -> Result<(), TrustError> {
        self.find::<AdminConfig>()?;
        self.staged
            .borrow_mut()
            .push(SourceMutation::AdminUpsert(entry));
        Ok(())
    }

    /// Remove an [`AdminEntry`] from the chain's `AdminConfig`, returning
    /// the currently committed value if any. The key is the full trust
    /// subject `(package_id, source)`; same-id entries from other sources
    /// are unaffected.
    pub fn admin_remove(
        &self,
        package_id: &PackageId,
        source: &ironclaw_host_api::PackageSource,
    ) -> Result<Option<AdminEntry>, TrustError> {
        let admin = self.find::<AdminConfig>()?;
        let previous = admin.get(package_id, source);
        self.staged.borrow_mut().push(SourceMutation::AdminRemove {
            package_id: package_id.clone(),
            source: source.clone(),
        });
        Ok(previous)
    }

    /// Insert or replace a [`SignerEntry`] in the chain's
    /// `SignedRegistry`. Note: the source itself is currently inert —
    /// this is the staging path future signature-verification work will
    /// consume.
    pub fn signed_upsert(&self, entry: SignerEntry) -> Result<(), TrustError> {
        self.find::<SignedRegistry>()?;
        self.staged
            .borrow_mut()
            .push(SourceMutation::SignedUpsert(entry));
        Ok(())
    }

    /// Remove a trusted signer from the chain's `SignedRegistry`,
    /// returning the currently committed entry if any.
    pub fn signed_remove(&self, signer: &str) -> Result<Option<SignerEntry>, TrustError> {
        let registry = self.find::<SignedRegistry>()?;
        let previous = registry.get(signer);
        self.staged
            .borrow_mut()
            .push(SourceMutation::SignedRemove(signer.to_string()));
        Ok(previous)
    }

    fn commit(&self) -> Result<(), TrustError> {
        for mutation in self.staged.borrow_mut().drain(..) {
            match mutation {
                SourceMutation::BundledUpsert(entry) => {
                    self.find::<BundledRegistry>()?.upsert(entry);
                }
                SourceMutation::BundledRemove(package_id) => {
                    self.find::<BundledRegistry>()?.remove(&package_id);
                }
                SourceMutation::AdminUpsert(entry) => {
                    self.find::<AdminConfig>()?.upsert(entry);
                }
                SourceMutation::AdminRemove { package_id, source } => {
                    self.find::<AdminConfig>()?.remove(&package_id, &source);
                }
                SourceMutation::SignedUpsert(entry) => {
                    self.find::<SignedRegistry>()?.upsert(entry);
                }
                SourceMutation::SignedRemove(signer) => {
                    self.find::<SignedRegistry>()?.remove(&signer);
                }
            }
        }
        Ok(())
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
