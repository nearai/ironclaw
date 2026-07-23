//! The production [`AttestedResumePort`] — the synchronous binding re-check the
//! turn store runs while resolving a `BlockedAttested` gate.
//!
//! ## Why this is synchronous and lightweight
//!
//! `ironclaw_turns::InMemoryTurnStateStore` calls
//! [`AttestedResumePort::verify_attested_resume`] **inside its resume mutex**
//! (see `crates/ironclaw_turns/src/memory.rs`: `resume_turn` holds
//! `lock_inner()` across `resume_turn_once`). Blocking on a tokio runtime there
//! would deadlock the store. So the port does only non-blocking work:
//!
//! 1. **Binding re-check (threats #2 / #3 / #4):** look up the authoritative
//!    [`AttestedGateBinding`] persisted when the gate was raised and confirm
//!    its `ApprovedTxHash` equals the `expected_tx_hash` the store recorded on
//!    the run. The caller never supplies the hash; it only attests to it.
//! 2. **One-shot resume guard (threats #1 / #16):** atomically claim a
//!    synchronous per-`gate_ref` guard so a replayed resume of an
//!    already-resolved gate fails closed at the boundary, and the LLM loop is
//!    never re-entered.
//!
//! The heavyweight verification (provider `verify_resume`, the authoritative
//! sealed-grant CAS) and the sign + broadcast happen *after* the store
//! transitions to `AttestedResolved`, in
//! [`crate::AttestedSignerContinuationDriver`]. Two independent one-shot guards
//! (this resume guard and the sealed grant claimed in the driver) is defense in
//! depth, not redundancy: either alone fails a replay closed.

use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Mutex};

use ironclaw_signing_provider::GateRef as SigningGateRef;
use ironclaw_turns::{AttestedResumePort, AttestedResumeRejection, AttestedResumeRequest};

use crate::binding::InMemoryAttestedGateBindingStore;

/// A synchronous one-shot guard claimed at resume time, keyed by `gate_ref`.
///
/// `claim` is an atomic compare-and-set: the first claim of a gate wins; every
/// later claim fails. This guarantees a `BlockedAttested` gate can be resolved
/// at most once even before the async sealed-grant claim runs in the driver.
pub trait ResumeGuard: Send + Sync {
    /// Atomically claim the one-shot guard for `gate_ref`. Returns `true` if
    /// this caller won (first claim), `false` if it was already claimed.
    fn claim(&self, gate_ref: &str) -> bool;
}

/// Maximum number of claimed gate refs retained by [`InMemoryResumeGuard`].
///
/// Bounded so a long-lived local runtime cannot grow this set without limit
/// (one entry per attested gate, never released). Eviction is safe here and
/// does NOT restore replayability: as the module doc states, the resume guard
/// and the sealed-grant CAS are two *independent* one-shot controls and either
/// alone fails a replay closed. An evicted gate that is replayed still hits the
/// authoritative sealed-grant CAS in the driver and is refused there — the
/// guard is defense in depth, not the primary control.
const MAX_CLAIMED_GATES: usize = 8192;

/// In-memory [`ResumeGuard`]. A single mutex makes claim atomic.
///
/// Bounded by [`MAX_CLAIMED_GATES`] with FIFO eviction of the oldest claims.
#[derive(Debug, Default)]
pub struct InMemoryResumeGuard {
    claimed: Mutex<(HashSet<String>, VecDeque<String>)>,
}

impl InMemoryResumeGuard {
    /// Construct an empty guard.
    pub fn new() -> Self {
        Self::default()
    }
}

impl ResumeGuard for InMemoryResumeGuard {
    fn claim(&self, gate_ref: &str) -> bool {
        match self.claimed.lock() {
            Ok(mut guard) => {
                let (set, order) = &mut *guard;
                // `insert` returns true iff the value was newly inserted, i.e.
                // this caller won the one-shot race.
                if !set.insert(gate_ref.to_string()) {
                    return false;
                }
                order.push_back(gate_ref.to_string());
                // FIFO-evict the oldest claims past the cap. Safe: see
                // `MAX_CLAIMED_GATES` — the sealed-grant CAS remains the
                // authoritative one-shot control for an evicted gate.
                while order.len() > MAX_CLAIMED_GATES {
                    if let Some(oldest) = order.pop_front() {
                        set.remove(&oldest);
                    }
                }
                true
            }
            // A poisoned lock fails closed: refuse the claim.
            Err(_) => false,
        }
    }
}

/// The production [`AttestedResumePort`] for the reborn composition.
pub struct RuntimeAttestedResumePort {
    bindings: Arc<InMemoryAttestedGateBindingStore>,
    resume_guard: Arc<dyn ResumeGuard>,
}

impl RuntimeAttestedResumePort {
    /// Build the port over the authoritative gate-binding store and the
    /// one-shot resume guard. Both are shared with the driver (which reads the
    /// same binding to verify + broadcast).
    pub fn new(
        bindings: Arc<InMemoryAttestedGateBindingStore>,
        resume_guard: Arc<dyn ResumeGuard>,
    ) -> Self {
        Self {
            bindings,
            resume_guard,
        }
    }
}

impl AttestedResumePort for RuntimeAttestedResumePort {
    fn verify_attested_resume(
        &self,
        request: AttestedResumeRequest<'_>,
    ) -> Result<(), AttestedResumeRejection> {
        let gate_ref_str = request.gate_ref.as_str();

        // 1. Authoritative binding re-check. The binding was persisted from the
        //    server-decoded transaction when the gate was raised; the caller's
        //    resume never gets to define the hash, only attest to it.
        let signing_gate_ref = SigningGateRef::new(gate_ref_str);
        let binding = self
            .bindings
            .get_sync(&signing_gate_ref)
            .ok_or(AttestedResumeRejection::InvalidClaim)?;

        // The store-recorded `expected_tx_hash` (an opaque ref) must match the
        // authoritative bound hash. We compare the lowercase-hex of the bound
        // hash against the recorded ref string. A mismatch is a caller-supplied
        // hash attempt (threat #3) and fails closed.
        let bound_hex = hex_lower(binding.approved_tx_hash.as_bytes());
        if request.expected_tx_hash.as_str() != bound_hex {
            return Err(AttestedResumeRejection::BindingMismatch);
        }

        // The attestation claim carried on the wire must itself attest to the
        // bound hash. The claim ref is the lowercase-hex of the attested hash;
        // it must equal the bound hash. (The cryptographic proof verification —
        // signer recovery, grant CAS — runs in the driver against this same
        // bound hash; here we only reject an attestation that does not even
        // claim the right hash.)
        if request.attestation.as_str() != bound_hex {
            return Err(AttestedResumeRejection::BindingMismatch);
        }

        // 2. One-shot resume guard (threats #1 / #16): a replayed resume of an
        //    already-resolved gate fails closed before the loop could ever be
        //    re-entered.
        if !self.resume_guard.claim(gate_ref_str) {
            return Err(AttestedResumeRejection::EvidenceRejected);
        }

        Ok(())
    }
}

/// Lowercase-hex encode (inline; no dependency).
fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        // Lookup table rather than `from_digit(..).unwrap_or('0')`: the fallback
        // was unreachable (both nibbles are < 16) but it is the pattern that
        // silently emits WRONG data if the surrounding arithmetic ever changes,
        // and this rendering is compared against a binding — a silent '0' would
        // corrupt a comparison rather than fail it.
        const HEX: &[u8; 16] = b"0123456789abcdef";
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

/// Public helper so the driver / binding writers produce the exact ref strings
/// the port expects for `expected_tx_hash` and `attestation`.
pub fn approved_tx_hash_ref_hex(bytes: &[u8]) -> String {
    hex_lower(bytes)
}

#[cfg(test)]
mod bounded_guard_tests {
    use super::*;

    #[test]
    fn resume_guard_is_one_shot_and_bounded() {
        let guard = InMemoryResumeGuard::default();

        // One-shot: the first claim wins, the replay loses.
        assert!(guard.claim("gate-a"), "first claim must win");
        assert!(!guard.claim("gate-a"), "replay must be refused");

        // Bounded: claiming past the cap evicts oldest entries rather than
        // growing without limit (#3994 review).
        for i in 0..MAX_CLAIMED_GATES + 16 {
            guard.claim(&format!("gate-fill-{i}"));
        }
        let (set, order) = &*guard.claimed.lock().expect("lock");
        assert!(
            set.len() <= MAX_CLAIMED_GATES && order.len() <= MAX_CLAIMED_GATES,
            "guard must stay bounded, got set={} order={}",
            set.len(),
            order.len()
        );
    }
}
