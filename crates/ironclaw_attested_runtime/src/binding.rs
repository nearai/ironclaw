//! The authoritative attested-gate binding the resume path verifies against.
//!
//! When a `BlockedAttested` gate is raised, the composition layer persists the
//! authoritative `(SigningContext, ApprovedTxHash, ProviderId, decoded tx,
//! schema)` for that gate. The resume port and the signer-continuation driver
//! both read this binding back by `gate_ref` rather than trusting any
//! caller-supplied context (threats #2 / #3 / #4): the caller's resume payload
//! only ever *attests* to the bound hash; it can never *redefine* it.
//!
//! In-memory only here (PR10). Durable PG / libSQL backends are PR12 — they
//! must implement [`AttestedGateBindingStore`] with identical semantics and be
//! dual-backend, so no single-backend persistence feature is added in this
//! crate.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;

use ironclaw_attestation::{DecodedTransaction, RenderingSchemaVersion};
use ironclaw_chain_signing::{ChainKeyId, recompute_approved_hash};
use ironclaw_signing_provider::{ApprovedTxHash, GateRef, ProviderId, SigningContext};

/// Errors a binding write can fail closed with. A binding is authoritative —
/// the resume port and driver trust it — so creation is INSERT-ONLY (CAS) and
/// fully validated at write time. None of these are recoverable: a rejected
/// write means the caller tried to mutate or mis-bind an authoritative gate.
#[derive(Debug, PartialEq, Eq)]
pub enum BindingError {
    /// A binding already exists for this `gate_ref`. Bindings are immutable:
    /// the first write wins and a later write can never mutate the binding the
    /// port + driver already trust.
    AlreadyExists,
    /// The store key does not equal `binding.context.gate_ref` — the binding
    /// would be retrievable under a gate_ref it does not describe.
    GateRefMismatch,
    /// The bound `approved_tx_hash` does not equal the hash recomputed from the
    /// binding's own decoded tx + schema (the binding contradicts itself).
    ApprovedHashMismatch,
    /// The bound `chain` does not match the chain/network its own decoded tx
    /// encodes (a testnet `chain` carrying a mainnet tx, or vice versa).
    ChainMismatch,
    /// The custodial signer/account context does not match the binding's
    /// decoded transaction (currently the EVM recipient check is advisory; the
    /// authoritative signer binding is the ecrecover check at sign time, so this
    /// is reserved for chains that carry an explicit signer in the decoded tx).
    SignerMismatch,
    /// The lock was poisoned; fail closed.
    Poisoned,
}

impl std::fmt::Display for BindingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyExists => write!(f, "a binding already exists for this gate_ref"),
            Self::GateRefMismatch => write!(f, "store key does not match binding context gate_ref"),
            Self::ApprovedHashMismatch => {
                write!(f, "approved hash does not match the decoded transaction")
            }
            Self::ChainMismatch => write!(f, "bound chain does not match the decoded transaction"),
            Self::SignerMismatch => {
                write!(f, "bound signer does not match the decoded transaction")
            }
            Self::Poisoned => write!(f, "binding store lock poisoned"),
        }
    }
}

impl std::error::Error for BindingError {}

/// Everything the resume path needs to verify and continue an attested-signing
/// gate, persisted authoritatively when the gate is raised.
#[derive(Debug, Clone)]
pub struct AttestedGateBinding {
    /// Which provider drove the ceremony (selects the verifier on resume).
    pub provider_id: ProviderId,
    /// The authoritative signing context (who/what/where/which gate).
    pub context: SigningContext,
    /// The `ApprovedTxHash` recorded at approval time — the one the resume
    /// `expected_tx_hash` must equal and the wallet/authn must attest to.
    pub approved_tx_hash: ApprovedTxHash,
    /// The server-decoded transaction (PR2 model). The custodial signer
    /// recomputes the hash from THIS; the broadcast path re-signs from it.
    pub decoded: DecodedTransaction,
    /// The chain key id the custodial path would consume (custodial only).
    pub chain: ChainKeyId,
    /// The authoritative keystore/AAD owner scope, persisted when the gate was
    /// raised. Carried directly rather than reconstructed from `context` so the
    /// custodial keystore lookup uses the exact validated scope (custodial
    /// only; ignored on external-wallet paths).
    pub scope: ironclaw_host_api::ResourceScope,
    /// Schema version the approval was rendered under.
    pub schema_version: RenderingSchemaVersion,
}

/// Validate a binding write before it is persisted. Run by every
/// [`AttestedGateBindingStore`] implementation so the authoritative invariants
/// hold regardless of backend (the in-memory store here and the durable PG /
/// libSQL backends in PR12). Does NOT check for an existing key — that is the
/// store's INSERT-ONLY CAS responsibility, which differs per backend.
pub fn validate_binding(key: &GateRef, binding: &AttestedGateBinding) -> Result<(), BindingError> {
    // The binding must be retrievable under exactly the gate_ref it describes.
    if key.as_str() != binding.context.gate_ref.as_str() {
        return Err(BindingError::GateRefMismatch);
    }
    // The bound approved hash must equal the hash recomputed from the binding's
    // own decoded tx — a self-consistency check so a write cannot bind a hash
    // that contradicts the tx the driver will later sign. The signer folded into
    // the recompute is the GATE-BOUND signer from the binding's SigningContext
    // (`context.key_or_account_id`), never the tx body — that is the WYSIWYS
    // binding. Recompute is fallible (render/canonicalization can fail); a
    // failure means the binding cannot be confirmed, so fail closed.
    let recomputed = recompute_approved_hash(
        &binding.decoded,
        binding.context.key_or_account_id.as_str(),
        binding.schema_version,
    )
    .map_err(|_| BindingError::ApprovedHashMismatch)?;
    if recomputed != binding.approved_tx_hash {
        return Err(BindingError::ApprovedHashMismatch);
    }
    // The bound chain must match the chain/network its own decoded tx encodes,
    // so a testnet `chain` can never carry a mainnet tx past the ship-gate.
    if binding.chain.as_str() != binding.decoded.chain_network() {
        return Err(BindingError::ChainMismatch);
    }
    // For chains whose decoded tx carries an explicit signer (NEAR), the signer
    // must match the context's account. EVM recovers the signer at sign time
    // (ecrecover), so its decoded model has no authoritative `from` to bind here.
    if let DecodedTransaction::Near(near) = &binding.decoded
        && near.signer_id != binding.context.key_or_account_id.as_str()
    {
        return Err(BindingError::SignerMismatch);
    }
    Ok(())
}

/// Store of authoritative attested-gate bindings, keyed by `gate_ref`.
///
/// One binding per `gate_ref`, created when the gate is raised and then
/// IMMUTABLE. The resume path and driver read it back and trust it, so writes
/// are INSERT-ONLY (CAS) and fully validated ([`validate_binding`]); durable
/// backends (PR12) carry identical semantics.
#[async_trait]
pub trait AttestedGateBindingStore: Send + Sync {
    /// Persist the authoritative binding for a freshly-raised attested gate.
    ///
    /// INSERT-ONLY: errors [`BindingError::AlreadyExists`] if a binding already
    /// exists for `gate_ref` (no overwrite, ever). Validates the binding
    /// ([`validate_binding`]) before persisting.
    async fn put(
        &self,
        gate_ref: GateRef,
        binding: AttestedGateBinding,
    ) -> Result<(), BindingError>;

    /// Read the authoritative binding for `gate_ref`, if one exists.
    async fn get(&self, gate_ref: &GateRef) -> Option<AttestedGateBinding>;
}

/// In-memory [`AttestedGateBindingStore`].
#[derive(Default)]
pub struct InMemoryAttestedGateBindingStore {
    bindings: Mutex<HashMap<GateRef, AttestedGateBinding>>,
}

impl InMemoryAttestedGateBindingStore {
    /// Construct an empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Synchronous read used by the resume port, which runs inside the turn
    /// store's sync critical section and therefore cannot `.await`. The async
    /// trait method is for the driver / ingress paths.
    pub fn get_sync(&self, gate_ref: &GateRef) -> Option<AttestedGateBinding> {
        self.bindings
            .lock()
            .ok()
            .and_then(|map| map.get(gate_ref).cloned())
    }
}

#[async_trait]
impl AttestedGateBindingStore for InMemoryAttestedGateBindingStore {
    async fn put(
        &self,
        gate_ref: GateRef,
        binding: AttestedGateBinding,
    ) -> Result<(), BindingError> {
        // Validate before taking the lock so a malformed binding never even
        // races for the slot.
        validate_binding(&gate_ref, &binding)?;
        let mut map = self.bindings.lock().map_err(|_| BindingError::Poisoned)?;
        // INSERT-ONLY CAS: a binding is authoritative and immutable. The first
        // write for a gate_ref wins; any later write fails closed so it can
        // never mutate the binding the port + driver already trust.
        if map.contains_key(&gate_ref) {
            return Err(BindingError::AlreadyExists);
        }
        map.insert(gate_ref, binding);
        Ok(())
    }

    async fn get(&self, gate_ref: &GateRef) -> Option<AttestedGateBinding> {
        self.get_sync(gate_ref)
    }
}
