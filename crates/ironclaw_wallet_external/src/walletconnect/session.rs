//! Per-gate WalletConnect session binding.
//!
//! When [`initiate`](super::WalletConnectSigningProvider) establishes a session
//! it records, keyed by the gate, the **expected** binding the eventual proof
//! must match: the WalletConnect session topic, the account the session settled
//! on (within the pinned scope), and a freshly-minted per-request nonce. At
//! [`verify_resume`](super::WalletConnectSigningProvider) time the returned
//! proof must carry exactly this `(session_topic, account, nonce)` triple, its
//! `signed_payload` must equal the recorded `expected_signing_payload`, and the
//! wallet's REAL chain signature must verify over those bytes (see
//! [`super::signer::verify_chain_signature`]).
//!
//! Binding the proof to the session + nonce defeats **T18** (a proof minted
//! under a *different* WC session / relay key, or replayed with a stale nonce,
//! is rejected) and complements the one-shot grant CAS (T20).
//!
//! The in-memory store here is the PR9 testable surface. Persisting the binding
//! durably across the initiate→resume gap (so it survives process restarts and
//! is consumed exactly once at the storage layer) is composition wiring owned by
//! PR10.
//!
//! PR10/PR11 must additionally derive [`SessionBinding::expected_signing_payload`]
//! from the decoded transaction at `initiate` using a real chain encoder (the
//! EVM EIP-2718/RLP secp256k1 sighash, the Solana message bytes) — the exact
//! bytes `eth_signTransaction` / `solana_signTransaction` will sign. That
//! encoder is not present in this crate (no alloy / solana-sdk; openssl-free),
//! so PR9 records the expected payload directly (in tests) and the verifier is
//! exercised against a real signed-payload fixture. Until the encoder exists,
//! `initiate` cannot populate a real expected payload, and `verify_resume` fails
//! closed for any gate without a recorded binding.

use std::collections::HashMap;
use std::sync::Mutex;

use ironclaw_signing_provider::{ApprovedTxHash, GateRef};

use super::namespace::PinnedScope;

/// The expected binding a WalletConnect proof must satisfy for a given gate.
///
/// Recorded at `initiate` from the **same decoded transaction** that produced
/// the approved render + [`ApprovedTxHash`]. The binding — not the proof — is
/// the authority: the proof echoes these values, but the verifier compares the
/// echoed values against this recorded expectation. In particular
/// [`Self::expected_signing_payload`] is the bridge from the approved hash to
/// the real chain signing bytes the wallet will sign (#1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionBinding {
    /// The WalletConnect v2 session topic the proof must belong to.
    pub session_topic: String,
    /// The account the session settled on (must lie within the pinned scope and
    /// equal the gate's bound account).
    pub account: String,
    /// Per-request nonce the wallet must commit to in its signature.
    pub nonce: Vec<u8>,
    /// The pinned single-chain / single-method scope for this gate.
    pub pinned: PinnedScope,
    /// The approved-tx hash this binding was recorded for. The verifier requires
    /// it to equal the gate's bound hash — a binding recorded for a different
    /// approval can never authorize this gate (#1, T20).
    pub approved_tx_hash: ApprovedTxHash,
    /// The **exact bytes the wallet's chain signature must cover** — the EVM
    /// secp256k1 sighash / the Solana ed25519 message, derived at `initiate`
    /// from the same decoded transaction that produced
    /// [`Self::approved_tx_hash`]. The verifier requires the proof's
    /// `signed_payload` to equal this and the chain signature to verify over it.
    /// This is the sound substitute for recomputing the approved hash (which
    /// `verify_resume` cannot do — it lacks the decode/render inputs): both the
    /// hash and this payload are derived from the one decoded tx the human
    /// approved. See `// PR10/PR11:` below.
    pub expected_signing_payload: Vec<u8>,
}

/// In-memory store of per-gate [`SessionBinding`]s.
///
/// `record` inserts the expectation at `initiate`; `take` removes and returns it
/// at `verify_resume` so a binding is consumed at most once in-process. (Durable
/// one-shot consumption is layered by the sealed-grant CAS at verify time and by
/// PR10's persistence.)
#[derive(Debug, Default)]
pub struct SessionBindingStore {
    bindings: Mutex<HashMap<String, SessionBinding>>,
}

impl SessionBindingStore {
    /// Construct an empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record the expected binding for `gate`. Overwrites any prior binding for
    /// the same gate (a re-initiation supersedes the stale expectation).
    ///
    /// A poisoned mutex (a thread panicked while holding the lock) is an
    /// unrecoverable invariant violation for this security-critical store:
    /// silently dropping the binding would make `verify_resume` fail closed for
    /// *every* gate with no diagnostic, so we propagate the poison by panicking
    /// rather than masking it with a no-op.
    pub fn record(&self, gate: &GateRef, binding: SessionBinding) {
        let mut map = self
            .bindings
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        map.insert(gate.as_str().to_string(), binding);
    }

    /// Return a clone of the expected binding for `gate`, if any, WITHOUT
    /// consuming it.
    ///
    /// Used by the verifier to run all hash/signature validation against the
    /// recorded expectation before the binding is consumed: a malformed relay
    /// response must not burn the binding (the recommendation). Consumption
    /// happens only on the success path via [`Self::take`]. Recovers from a
    /// poisoned mutex (see [`Self::record`]) instead of silently returning
    /// `None`, which would spuriously fail verification closed.
    pub fn peek(&self, gate: &GateRef) -> Option<SessionBinding> {
        let map = self
            .bindings
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        map.get(gate.as_str()).cloned()
    }

    /// Remove and return the expected binding for `gate`, if any. Recovers from
    /// a poisoned mutex (see [`Self::record`]) rather than silently dropping the
    /// consume.
    pub fn take(&self, gate: &GateRef) -> Option<SessionBinding> {
        let mut map = self
            .bindings
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        map.remove(gate.as_str())
    }
}
