//! Trust-change invalidation contract.
//!
//! When the host policy revokes or downgrades a package's effective trust,
//! affected grants and leases must be invalidated *before* any subsequent
//! dispatch can produce a side effect under the stale ceiling. PR3's
//! authorization layer registers a [`TrustChangeListener`] on the
//! [`InvalidationBus`] and revokes matching grants synchronously when
//! [`InvalidationBus::publish`] runs.
//!
//! The bus is fail-closed by design: listeners are run synchronously on the
//! publishing thread. If any listener panics, the panic propagates — the
//! caller of `publish` must observe a failure rather than continue believing
//! invalidation succeeded.

use std::sync::{Arc, RwLock};

use ironclaw_host_api::{CapabilityId, PackageIdentity, Timestamp};

use crate::decision::EffectiveTrustClass;

/// Listener notified when effective trust for a package changes.
pub trait TrustChangeListener: Send + Sync {
    fn on_trust_changed(&self, change: &TrustChange);
}

/// One trust-change event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustChange {
    pub identity: PackageIdentity,
    pub previous: EffectiveTrustClass,
    pub current: EffectiveTrustClass,
    /// Authority that was active before the change, if known. Listeners use
    /// this to scope the grant set they invalidate.
    pub previous_authority: Vec<CapabilityId>,
    pub effective_at: Timestamp,
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
    pub fn publish(&self, change: TrustChange) {
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
/// The check is set-equality with a length guard, so it fires on **any**
/// content difference: additions, removals, or reorderings into a different
/// set. This is deliberately stricter than a literal reading of AC #5
/// ("Expanded authority requires renewed approval") — over-firing on
/// removal is safe (a smaller authority set going through reissue is at
/// worst a redundant approval), while *under*-firing on additions would
/// silently retain a grant whose authority surface changed. We choose the
/// safer side.
///
/// Implementation: a bidirectional `iter().all(contains)` runs in O(n²) on
/// the slice contents but allocates nothing — a measurable win on WASM and
/// no slower than `sort + compare` for the small authority lists (a
/// handful of capability ids per package) we see in practice. The length
/// guard is required: without it, `[a, a, b]` would falsely match `[a, b]`
/// because every entry of the longer slice is contained in the shorter.
pub fn authority_changed(prev: &[CapabilityId], curr: &[CapabilityId]) -> bool {
    if prev.len() != curr.len() {
        return true;
    }
    !prev.iter().all(|p| curr.contains(p)) || !curr.iter().all(|c| prev.contains(c))
}

/// Returns true when an existing grant may be retained across a
/// re-evaluation: identity stable, effective trust unchanged, and requested
/// authority unchanged. Any drift forces grant reissue per AC #7.
pub fn grant_retention_eligible(
    prev_identity: &PackageIdentity,
    curr_identity: &PackageIdentity,
    prev_trust: EffectiveTrustClass,
    curr_trust: EffectiveTrustClass,
    prev_authority: &[CapabilityId],
    curr_authority: &[CapabilityId],
) -> bool {
    !identity_changed(prev_identity, curr_identity)
        && prev_trust == curr_trust
        && !authority_changed(prev_authority, curr_authority)
}
