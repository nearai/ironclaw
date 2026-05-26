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

use std::collections::HashSet;
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

/// In-memory [`ResumeGuard`]. A single mutex makes claim atomic.
#[derive(Debug, Default)]
pub struct InMemoryResumeGuard {
    claimed: Mutex<HashSet<String>>,
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
            // `insert` returns true iff the value was newly inserted, i.e. this
            // caller won the one-shot race.
            Ok(mut set) => set.insert(gate_ref.to_string()),
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
        out.push(char::from_digit((b >> 4) as u32, 16).unwrap_or('0'));
        out.push(char::from_digit((b & 0x0f) as u32, 16).unwrap_or('0'));
    }
    out
}

/// Public helper so the driver / binding writers produce the exact ref strings
/// the port expects for `expected_tx_hash` and `attestation`.
pub fn approved_tx_hash_ref_hex(bytes: &[u8]) -> String {
    hex_lower(bytes)
}
