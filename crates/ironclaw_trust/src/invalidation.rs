//! Trust-change invalidation contract.
//!
//! When the host policy revokes or downgrades a package's effective trust,
//! or shrinks its authority ceiling, affected grants and leases must be
//! invalidated *before* any subsequent dispatch can produce a side effect
//! under the stale ceiling. PR3's
//! authorization layer registers a [`TrustChangeListener`] on the
//! [`InvalidationBus`] and revokes matching grants synchronously when
//! [`InvalidationBus::publish`] runs.
//!
//! The bus is fail-closed by design: listeners are run synchronously on the
//! publishing thread. If any listener panics, the panic propagates — the
//! caller of `publish` must observe a failure rather than continue believing
//! invalidation succeeded.

use std::collections::BTreeSet;
use std::sync::{Arc, RwLock};

use ironclaw_host_api::{CapabilityId, PackageIdentity, Timestamp};

use crate::decision::{AuthorityCeiling, EffectiveTrustClass, TrustDecision};

/// Listener notified when trust or authority ceiling for a package changes.
pub trait TrustChangeListener: Send + Sync {
    fn on_trust_changed(&self, change: &TrustChange);
}

/// One invalidation-relevant trust/authority change event.
///
/// Use [`TrustChange::new`] to construct — it compares the full previous
/// and current [`TrustDecision`] and filters no-ops / benign authority
/// expansions by returning `None`. Direct struct-literal construction is
/// still possible for tests but [`InvalidationBus::publish`] applies a
/// defense-in-depth filter that drops events with neither a trust-class
/// change nor an authority-ceiling reduction.
///
/// Listeners that only care about *downgrades* (the AC #6 case — revoke
/// grants whose authority no longer fits the lower trust ceiling) should
/// gate on [`TrustChange::is_downgrade`]. Listeners must also treat
/// [`TrustChange::authority_ceiling_reduced`] as invalidating: a same-class
/// removal of `WriteFilesystem`, or a stricter resource ceiling, can make
/// retained grants exceed the new cap even though `effective_trust` did
/// not change. Listeners that scope grants to a specific privilege *kind*
/// (e.g., FirstParty vs System) should also react to
/// [`TrustChange::is_kind_change`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustChange {
    pub identity: PackageIdentity,
    pub previous: EffectiveTrustClass,
    pub current: EffectiveTrustClass,
    /// Authority ceiling before the policy mutation.
    pub previous_authority_ceiling: AuthorityCeiling,
    /// Authority ceiling after the policy mutation.
    pub current_authority_ceiling: AuthorityCeiling,
    pub effective_at: Timestamp,
}

impl TrustChange {
    /// Construct a `TrustChange` from full policy decisions. Returns `None`
    /// when neither the trust class changed nor the authority ceiling shrank.
    ///
    /// Same-class authority reductions are publishable because existing
    /// grants may now exceed the new `AuthorityCeiling`; benign expansions
    /// are not publishable because they do not require fail-closed grant
    /// invalidation.
    pub fn new(
        identity: PackageIdentity,
        previous_decision: &TrustDecision,
        current_decision: &TrustDecision,
    ) -> Option<Self> {
        let authority_ceiling_reduced = current_decision
            .authority_ceiling
            .is_reduction_from(&previous_decision.authority_ceiling);
        if previous_decision.effective_trust == current_decision.effective_trust
            && !authority_ceiling_reduced
        {
            return None;
        }
        Some(Self {
            identity,
            previous: previous_decision.effective_trust,
            current: current_decision.effective_trust,
            previous_authority_ceiling: previous_decision.authority_ceiling.clone(),
            current_authority_ceiling: current_decision.authority_ceiling.clone(),
            effective_at: current_decision.evaluated_at,
        })
    }

    /// True when `current`'s authority level is strictly less than
    /// `previous`'s. This is the AC #6 case: grants issued under the
    /// previous ceiling may exceed what the new ceiling allows and must
    /// be revoked or scoped down.
    pub fn is_downgrade(&self) -> bool {
        self.current.authority_level() < self.previous.authority_level()
    }

    /// True when `current`'s authority level is strictly greater than
    /// `previous`'s. Existing grants stay valid (more authority is a
    /// superset); listeners reacting to upgrades typically *grow* the
    /// available surface rather than revoking anything.
    pub fn is_upgrade(&self) -> bool {
        self.current.authority_level() > self.previous.authority_level()
    }

    /// True when `previous != current` but the authority levels are
    /// equal — the only case is a sideways move between `FirstParty` and
    /// `System`. The two are different *kinds* of privilege; grants
    /// scoped to one kind do not transfer to the other, so listeners
    /// must treat this as invalidating even though it's not a downgrade.
    pub fn is_kind_change(&self) -> bool {
        self.previous != self.current
            && self.current.authority_level() == self.previous.authority_level()
    }

    /// True when the trust class stayed the same but the grant ceiling got
    /// stricter, or when a trust-class change also carried a stricter
    /// ceiling. This is invalidation-relevant because retained grants may
    /// include effects or resource budgets no longer allowed.
    pub fn authority_ceiling_reduced(&self) -> bool {
        self.current_authority_ceiling
            .is_reduction_from(&self.previous_authority_ceiling)
    }
}

/// Synchronous fan-out of [`TrustChange`] events.
///
/// Listeners are run in registration order on the publishing thread. The
/// bus does not buffer or deduplicate events — semantics are intentionally
/// simple so PR3's grant store can rely on strict happens-before ordering.
pub struct InvalidationBus {
    listeners: RwLock<Vec<Arc<dyn TrustChangeListener>>>,
}

impl InvalidationBus {
    pub fn new() -> Self {
        Self {
            listeners: RwLock::new(Vec::new()),
        }
    }

    pub fn register(&self, listener: Arc<dyn TrustChangeListener>) {
        // Recover from poisoning rather than panic: the listeners Vec itself
        // never enters an inconsistent state (we only push), so the previous
        // panic that poisoned the lock has already failed closed for its own
        // publish — subsequent registrations are safe.
        let mut guard = self
            .listeners
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.push(listener);
    }

    /// Fan out the change synchronously. All listeners run before this
    /// returns; downstream code observing the post-publish state can rely on
    /// every listener having processed the change. A panicking listener
    /// propagates on the publishing thread — fail-closed.
    ///
    /// Events with no trust-class change and no authority-ceiling reduction
    /// are dropped without invoking any listener. The recommended
    /// construction path ([`TrustChange::new`]) prevents these at the
    /// source; this filter is defense-in-depth for callers that built a
    /// `TrustChange` via struct literal. In debug builds the no-op trips an
    /// assertion so the offending caller is surfaced loudly.
    pub fn publish(&self, change: TrustChange) {
        if change.previous == change.current && !change.authority_ceiling_reduced() {
            debug_assert!(
                false,
                "TrustChange without trust-class change or authority-ceiling reduction \
                 reached InvalidationBus::publish — use TrustChange::new(...)"
            );
            return;
        }
        let listeners = self
            .listeners
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for listener in listeners.iter() {
            listener.on_trust_changed(&change);
        }
    }

    pub fn listener_count(&self) -> usize {
        self.listeners
            .read()
            .map(|g| g.len())
            .unwrap_or_else(|p| p.into_inner().len())
    }
}

impl Default for InvalidationBus {
    fn default() -> Self {
        Self::new()
    }
}

/// Returns true when two package identities differ in any field that should
/// invalidate retained grants. Used by PR3's grant store to decide whether
/// an existing grant survives a re-evaluation.
pub fn identity_changed(prev: &PackageIdentity, curr: &PackageIdentity) -> bool {
    prev.package_id != curr.package_id
        || prev.source != curr.source
        || prev.digest != curr.digest
        || prev.signer != curr.signer
}

/// Returns true when the requested-authority set differs between two
/// re-evaluations of the same package.
///
/// The check fires on any content difference — additions, removals, or
/// reorderings into a different set. This is deliberately stricter than
/// a literal reading of AC #5 ("Expanded authority requires renewed
/// approval"): over-firing on removal is safe (a smaller authority set
/// going through reissue is at worst a redundant approval), while
/// *under*-firing on additions would silently retain a grant whose
/// authority surface changed. We choose the safer side.
///
/// Authority is typed as `BTreeSet<CapabilityId>` because it is
/// conceptually a set, not a multiset. Earlier slice-based shapes had
/// to length-guard against `[a, a, b]` vs `[a, b]` to avoid false
/// matches, but that meant two callers with the *same effective
/// authority* but different list-canonicalization fired this check
/// unnecessarily. Set typing closes that gap at the type level — the
/// duplicates literally cannot exist.
pub fn authority_changed(prev: &BTreeSet<CapabilityId>, curr: &BTreeSet<CapabilityId>) -> bool {
    prev != curr
}

/// Returns true when an existing grant may be retained across a
/// re-evaluation: identity stable, effective trust unchanged, and requested
/// authority unchanged. Any drift forces grant reissue per AC #7.
pub fn grant_retention_eligible(
    prev_identity: &PackageIdentity,
    curr_identity: &PackageIdentity,
    prev_trust: EffectiveTrustClass,
    curr_trust: EffectiveTrustClass,
    prev_authority: &BTreeSet<CapabilityId>,
    curr_authority: &BTreeSet<CapabilityId>,
) -> bool {
    !identity_changed(prev_identity, curr_identity)
        && prev_trust == curr_trust
        && !authority_changed(prev_authority, curr_authority)
}
