//! The WalletConnect v2 [`SigningProvider`] backend (attested-signing PR9).
//!
//! [`WalletConnectSigningProvider`] bridges a WalletConnect v2 session — driven
//! over the openssl-free `relay_client` fork — to the attested-signing gate. It
//! reports [`ProviderId::WalletConnect`] and [`TrustModel::ExternalWallet`] and
//! holds **no key material**: the user's wallet renders + signs natively (true
//! WYSIWYS). It is configured with a WalletConnect Cloud [`ProjectId`]
//! (publishable, API-key class — injected, never hardcoded).
//!
//! ## Two security cores
//!
//! 1. **Namespace pinning** ([`namespace`]). The gate has decided exactly one
//!    chain + one signing operation; a WC session can negotiate far broader
//!    scope. [`PinnedScope`] derives the single CAIP-2 chain + single signing
//!    method the gate authorizes, and every proposed/settled session scope is
//!    checked equal to it — any superset is a [`SigningProviderError::ScopeViolation`]
//!    (threats T17/T19).
//! 2. **Proof verification** ([`Self::verify_resume`]). Fail-closed, in order:
//!    pinned-scope match, hash binding (T20, both proof and recorded binding),
//!    session + nonce binding (T18), account binding, **real signed-transaction
//!    binding** (the proof's `signed_payload` — the exact bytes the wallet's
//!    chain signature covers — must equal the `expected_signing_payload`
//!    recorded at initiate from the same decoded tx), signer binding over that
//!    real signature (T17, [`SignerMismatch`](SigningProviderError::SignerMismatch)),
//!    then the one-shot sealed-grant CAS (T20,
//!    [`GrantClaimFailed`](SigningProviderError::GrantClaimFailed)). The recorded
//!    binding is consumed only on full success. Only then is a [`VerifiedProof`]
//!    returned. A signature over a synthetic digest is never accepted.
//!
//! ## Scope boundary (PR9 vs PR10)
//!
//! PR9 implements the provider, the namespace pinning, and the full proof
//! verification against a recorded session binding. The encrypted CAIP-25 Sign
//! envelope round-trip over the relay (publishing the pairing proposal,
//! subscribing for the wallet's response, decrypting the signed payload) and the
//! verified-proof → gate handoff + broadcast are composition owned by PR10 and
//! are marked `// PR10:`. The fork's `relay_client` provides relay transport but
//! not the higher-level v2 Sign session layer, so the live round-trip is stubbed
//! at the verified-proof boundary rather than reimplemented here.

mod namespace;
mod proof;
mod session;
mod signer;

use std::sync::Arc;

use async_trait::async_trait;

use ironclaw_attestation::{GrantError, GrantKey, SealedGrantStore};
use ironclaw_signing_provider::{
    ApprovedTxHash, DecodedTransaction, InitiationOutcome, ProviderId, RenderedTx, SigningContext,
    SigningProof, SigningProvider, SigningProviderError, TrustModel, VerifiedProof,
};

// Re-export the publishable relay-side ProjectId newtype from the fork so
// callers configure the provider with the SDK's own type (no parallel newtype).
pub use relay_rpc::domain::ProjectId;

pub use namespace::{
    Caip2ChainId, Caip10Account, ChainFamily, PinnedScope, ProposedScope, enforce_pinned_scope,
};
pub use proof::{
    WalletConnectProofPayload, decode_walletconnect_proof, encode_walletconnect_proof,
};
pub use session::{SessionBinding, SessionBindingStore};

/// The WalletConnect v2 signing backend.
///
/// Holds the injected [`ProjectId`], a sealed-grant store (for the one-shot CAS
/// at verify time), and the per-gate [`SessionBindingStore`]. Holds **no key
/// material**.
pub struct WalletConnectSigningProvider {
    project_id: ProjectId,
    grants: Arc<dyn SealedGrantStore>,
    bindings: Arc<SessionBindingStore>,
}

impl WalletConnectSigningProvider {
    /// Construct over an injected [`ProjectId`] and a sealed-grant store.
    ///
    /// The `ProjectId` is a publishable WalletConnect Cloud API key sourced from
    /// configuration / `ironclaw_secrets` by the composition layer (PR10) — it
    /// is never hardcoded. Different tenants may inject different project ids;
    /// this provider treats it as opaque per-instance config.
    pub fn new(project_id: ProjectId, grants: Arc<dyn SealedGrantStore>) -> Self {
        Self {
            project_id,
            grants,
            bindings: Arc::new(SessionBindingStore::new()),
        }
    }

    /// The WalletConnect Cloud project id this provider is configured with.
    pub fn project_id(&self) -> &ProjectId {
        &self.project_id
    }

    /// Record an expected session binding for a gate.
    ///
    /// In production this is populated by `initiate` once the WC session settles
    /// over the relay (PR10 wires the live settlement). Exposed so the
    /// composition layer — and tests — can install the binding the eventual
    /// proof must match.
    pub fn record_session_binding(
        &self,
        gate: &ironclaw_signing_provider::GateRef,
        binding: SessionBinding,
    ) {
        self.bindings.record(gate, binding);
    }
}

#[async_trait]
impl SigningProvider for WalletConnectSigningProvider {
    fn provider_id(&self) -> ProviderId {
        ProviderId::WalletConnect
    }

    fn trust_model(&self) -> TrustModel {
        TrustModel::ExternalWallet
    }

    async fn initiate(
        &self,
        context: &SigningContext,
        _decoded: &DecodedTransaction,
        _rendered: &RenderedTx,
        _approved_tx_hash: &ApprovedTxHash,
    ) -> Result<InitiationOutcome, SigningProviderError> {
        // Pin the single chain + single signing method the gate authorizes.
        // Resolving this here — before any relay traffic — means a session can
        // only ever be proposed at the pinned scope (T17/T19). An unsupported or
        // malformed chain id fails closed as a ScopeViolation.
        let pinned = PinnedScope::from_chain_id(&context.chain_id)?;

        // PR10: establish the WC v2 session over the forked `relay_client`:
        // build a pairing URI for the configured ProjectId, publish the CAIP-25
        // session proposal pinned to `pinned.caip2_chain` + `pinned.method`,
        // subscribe for the wallet's settlement, enforce `enforce_pinned_scope`
        // against the SETTLED namespaces + accounts, mint a per-request nonce,
        // derive `expected_signing_payload` from the decoded tx via the real
        // chain encoder (the EVM EIP-2718/RLP secp256k1 sighash / the Solana
        // message bytes — PR10/PR11; not present in this crate), and
        // `record_session_binding(gate, SessionBinding { session_topic, account,
        // nonce, pinned, approved_tx_hash, expected_signing_payload })`. The
        // encrypted Sign envelope layer is not exposed by the relay_client fork,
        // so the live round-trip is deferred to PR10. The pinned method that
        // would be requested is `pinned.method` (e.g. eth_signTransaction /
        // solana_signTransaction) — sign-only; broadcast is PR10.
        let _ = (&self.project_id, pinned);

        // Until PR10 wires the live pairing, signal that the ceremony proceeds
        // out of band (the directive bytes a real session would carry — the
        // pairing URI — are a PR10 concern).
        Ok(InitiationOutcome::AwaitingUserAction {
            // PR10: replace with the real WalletConnect pairing URI bytes.
            directive: Vec::new(),
        })
    }

    async fn verify_resume(
        &self,
        context: &SigningContext,
        approved_tx_hash: &ApprovedTxHash,
        proof: &SigningProof,
    ) -> Result<VerifiedProof, SigningProviderError> {
        // Only WalletConnect proofs are accepted; anything else is a routing
        // error and fails closed.
        let SigningProof::WalletConnectProof(bytes) = proof else {
            return Err(SigningProviderError::ProofInvalid {
                reason: "walletconnect provider received a non-walletconnect proof".to_string(),
            });
        };
        let payload = decode_walletconnect_proof(bytes)?;

        // Re-derive the pinned scope from the gate's bound chain id. A malformed
        // / unsupported chain id fails closed (ScopeViolation) before any crypto.
        let pinned = PinnedScope::from_chain_id(&context.chain_id)?;

        // The session binding recorded at initiate. PEEKED (not yet consumed):
        // all hash/signature validation runs against this recorded expectation
        // first, so a malformed relay/wallet response cannot burn the binding.
        // It is consumed (taken) only on the success path, after the one-shot
        // grant is claimed.
        let binding =
            self.bindings
                .peek(&context.gate_ref)
                .ok_or(SigningProviderError::ProofInvalid {
                    reason: "no recorded walletconnect session binding for this gate".to_string(),
                })?;

        // The recorded binding's pinned scope must match the gate's bound scope
        // (defense in depth against a binding recorded under a different chain).
        if binding.pinned != pinned {
            return Err(SigningProviderError::ScopeViolation {
                reason: "recorded session binding scope does not match the gate's pinned scope"
                    .to_string(),
            });
        }

        // 1. Hash binding (T20). The wallet's proof must carry the exact bound
        //    hash, AND the binding recorded at initiate must have been recorded
        //    for that same hash — neither the proof nor the binding may smuggle
        //    a different approval.
        if &payload.approved_tx_hash != approved_tx_hash {
            return Err(SigningProviderError::ProofInvalid {
                reason: "proof approved-tx hash does not match the bound hash".to_string(),
            });
        }
        if &binding.approved_tx_hash != approved_tx_hash {
            return Err(SigningProviderError::ProofInvalid {
                reason: "recorded session binding hash does not match the bound hash".to_string(),
            });
        }

        // 2. Session + nonce binding (T18): the proof must belong to the recorded
        //    session and carry the recorded nonce. A proof minted under a
        //    different WC session / relay key, or replayed with a stale/forged
        //    nonce, is rejected here. This is an ADDITIONAL anti-replay layer on
        //    top of the real chain-signature check below, not a replacement.
        if payload.session_topic != binding.session_topic {
            return Err(SigningProviderError::ProofInvalid {
                reason: "proof session topic does not match the recorded session binding"
                    .to_string(),
            });
        }
        if payload.nonce != binding.nonce {
            return Err(SigningProviderError::ProofInvalid {
                reason: "proof nonce does not match the recorded session binding".to_string(),
            });
        }

        // The session-bound account must equal the gate's bound account — the WC
        // session settled on the same account the grant is bound to. Compared
        // case-insensitively: EVM addresses are case-insensitive hex (and may be
        // EIP-55 mixed-case as returned by the wallet/relay) and the ed25519 key
        // accounts are hex; the authoritative signer binding is the byte-exact
        // chain-signature check in `verify_chain_signature` below, which this
        // string compare only guards as defense in depth.
        let bound_account = context.key_or_account_id.as_str();
        if !binding.account.eq_ignore_ascii_case(bound_account) {
            return Err(SigningProviderError::SignerMismatch);
        }

        // 3. Real signed-transaction binding (#1, T20). The proof carries the
        //    EXACT bytes the wallet's chain signature covers (`signed_payload` —
        //    the EVM secp256k1 sighash / the Solana ed25519 message returned by
        //    eth_signTransaction / solana_signTransaction). Bind those bytes back
        //    to what the human approved by requiring them to equal the
        //    `expected_signing_payload` recorded at initiate from the SAME
        //    decoded transaction that produced `approved_tx_hash`. A wallet that
        //    signed *different* bytes than the approved transaction is rejected
        //    here — the synthetic-digest acceptance is gone.
        if payload.signed_payload != binding.expected_signing_payload {
            return Err(SigningProviderError::ProofInvalid {
                reason: "proof signed payload does not match the approved transaction bytes"
                    .to_string(),
            });
        }

        // 4. Signer binding (T17): verify the wallet's REAL chain signature over
        //    `signed_payload` and require the recovered/verified signer to equal
        //    the bound account.
        signer::verify_chain_signature(
            pinned.family,
            &payload.signed_payload,
            &payload.signature,
            payload.public_key.as_deref(),
            bound_account,
        )?;

        // 5. One-shot grant (T20): claim the sealed grant atomically. A replay of
        //    an already-claimed grant fails closed here.
        let key = GrantKey::from_context(context, *approved_tx_hash);
        self.grants.claim(&key).await.map_err(map_grant_error)?;

        // Consume the binding only now that every check (incl. the durable grant
        // CAS) has passed — a malformed earlier response never burned it.
        let _ = self.bindings.take(&context.gate_ref);

        // PR10: hand the verified proof back to the gate / runner for the
        // deterministic post-approval continuation (broadcast via
        // ironclaw_chain_signing). PR9 stops at the verified-proof boundary.
        Ok(VerifiedProof::new(
            ProviderId::WalletConnect,
            *approved_tx_hash,
            proof.clone(),
        ))
    }
}

/// Map a [`GrantError`] onto the provider error taxonomy, fail-closed.
fn map_grant_error(err: GrantError) -> SigningProviderError {
    match err {
        GrantError::AlreadyClaimed | GrantError::NotFound | GrantError::AlreadySealed => {
            SigningProviderError::GrantClaimFailed
        }
        GrantError::Backend { reason } => SigningProviderError::Provider { reason },
    }
}

/// Hex (de)serialization helpers shared by the proof payload fields.
pub(crate) mod hex_bytes {
    use serde::{Deserialize, Deserializer, Serializer};

    pub(crate) fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex_encode(bytes))
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(d)?;
        hex_decode(&s).map_err(serde::de::Error::custom)
    }

    pub(crate) fn hex_encode(bytes: &[u8]) -> String {
        let mut out = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            out.push(char::from_digit((b >> 4) as u32, 16).unwrap_or('0'));
            out.push(char::from_digit((b & 0x0f) as u32, 16).unwrap_or('0'));
        }
        out
    }

    /// Decode `0x`-prefixed hex into bytes, panic-free on any input.
    ///
    /// Operates on **bytes**, never on `&str` byte-offset slices: a non-ASCII
    /// even-byte input would make byte-offset slicing land on an invalid UTF-8
    /// boundary and panic. Non-ASCII / non-hex bytes are rejected as
    /// `ProofInvalid`-grade errors so relay/wallet-supplied hex fails closed
    /// (#3).
    pub(crate) fn hex_decode(s: &str) -> Result<Vec<u8>, String> {
        let s = s.strip_prefix("0x").unwrap_or(s);
        let bytes = s.as_bytes();
        if !bytes.len().is_multiple_of(2) {
            return Err("odd-length hex".to_string());
        }
        let mut out = Vec::with_capacity(bytes.len() / 2);
        for chunk in bytes.chunks_exact(2) {
            let hi = nibble(chunk[0])?;
            let lo = nibble(chunk[1])?;
            out.push((hi << 4) | lo);
        }
        Ok(out)
    }

    /// Map a single ASCII hex digit byte to its nibble value.
    fn nibble(b: u8) -> Result<u8, String> {
        match b {
            b'0'..=b'9' => Ok(b - b'0'),
            b'a'..=b'f' => Ok(b - b'a' + 10),
            b'A'..=b'F' => Ok(b - b'A' + 10),
            other => Err(format!("invalid hex byte 0x{other:02x}")),
        }
    }
}
