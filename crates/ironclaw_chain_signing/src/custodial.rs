//! The custodial signer: the orchestration that turns a resolved attestation +
//! a persisted decoded transaction into a signed, broadcast transaction, behind
//! two independent enforcement points and the broadcast-idempotency guard.
//!
//! ## Enforcement points
//!
//! 1. **Grant claim (authorization).** The signer refuses to do anything
//!    without successfully claiming the sealed one-shot `AttestedSigningGrant`
//!    (PR3) for this `(context, approved_tx_hash)`. The claim is a one-shot CAS:
//!    a replayed approval cannot be turned into a second signature.
//!
//! 2. **Sign-time approved-tx-hash re-check (integrity).** The signer
//!    re-derives the canonical signing bytes and recomputes the
//!    `ApprovedTxHash` *from the persisted decoded transaction* and asserts it
//!    equals the approved hash the grant was sealed against. If the persisted
//!    decoded tx was mutated after approval, the recomputed hash diverges and
//!    signing fails closed — **before any key is consumed**.
//!
//! On top of these, the [`SigningLedger`] (PR3) enforces broadcast idempotency:
//! the signer advances `Approved -> Signing -> Signed -> BroadcastSubmitted ->
//! (Finalized | Unknown | ManualReview)` and the ledger refuses to re-enter
//! signing for a gate_ref already past `BroadcastSubmitted`, surviving a
//! `Stuck -> InProgress` job recovery.

use std::sync::Arc;

use ironclaw_attestation::{
    DecodedTransaction, GrantKey, RenderingSchemaVersion, SealedGrantStore, SigningLedger,
    SigningLedgerState, approved_tx_hash_for, canonical_signing_bytes,
};
use ironclaw_host_api::ResourceScope;
use ironclaw_signing_provider::{ApprovedTxHash, SigningContext};

use crate::chain::{ChainFamily, ChainKeyId};
use crate::error::ChainSigningError;
use crate::keystore::{ChainKeyBinding, ConsumedChainKey, KeyStore};
use crate::kms::{KmsSigner, ShipGate, SignatureAlg, SigningPath};
use crate::policy::{CustodyDecision, KeyCustodyPolicy};

/// Inputs to a custodial signing operation. Every value is already persisted /
/// resolved by the higher layers; the signer re-derives the binding hash from
/// `decoded` rather than trusting any caller-supplied hash beyond the approved
/// one it must match.
pub struct CustodialSignRequest {
    /// The signing context (who/what/where/which gate).
    pub context: SigningContext,
    /// Owner scope used to address the keystore + chain AAD.
    pub scope: ResourceScope,
    /// Chain the key is bound to.
    pub chain: ChainKeyId,
    /// The persisted decoded transaction (PR2 model). Enforcement point #2
    /// recomputes the hash from THIS.
    pub decoded: DecodedTransaction,
    /// The `ApprovedTxHash` recorded at approval time (what the grant was sealed
    /// against). The signer recomputes from `decoded` and asserts equality.
    pub approved_tx_hash: ApprovedTxHash,
    /// Schema version the approval was rendered under.
    pub schema_version: RenderingSchemaVersion,
}

/// What a successful signing produced. The chain-native signature bytes are
/// returned for the per-chain broadcast path; the ledger has already been
/// advanced to `Signed`.
#[derive(Debug)]
pub struct CustodialSignOutcome {
    /// Raw chain-native signature bytes.
    pub signature: Vec<u8>,
    /// The signer/account the signature recovered to (public).
    pub signer: String,
}

/// The signing path + the public binding decided by authorization.
struct Authorized {
    path: SigningPath,
    binding: ChainKeyBinding,
}

/// The custodial signer, wired with a keystore, grant store, ledger, ship-gate,
/// an optional sign-only KMS backend, and an injectable custody policy.
pub struct CustodialSigner<K, G, L> {
    keystore: Arc<K>,
    grants: Arc<G>,
    ledger: Arc<L>,
    ship_gate: ShipGate,
    /// Sign-only KMS/HSM backend. REQUIRED for mainnet signing (no private-key
    /// bytes in process); absent means only testnet/dev hot-key signing works.
    kms: Option<Arc<dyn KmsSigner>>,
    custody_policy: Arc<dyn KeyCustodyPolicy>,
}

impl<K, G, L> CustodialSigner<K, G, L>
where
    K: KeyStore,
    G: SealedGrantStore,
    L: SigningLedger,
{
    /// Construct a hot-key-only custodial signer (testnet/dev). Mainnet signing
    /// will be refused by the ship-gate because no KMS backend is wired.
    pub fn new(
        keystore: Arc<K>,
        grants: Arc<G>,
        ledger: Arc<L>,
        ship_gate: ShipGate,
        custody_policy: Arc<dyn KeyCustodyPolicy>,
    ) -> Self {
        Self {
            keystore,
            grants,
            ledger,
            ship_gate,
            kms: None,
            custody_policy,
        }
    }

    /// Construct a custodial signer wired with a sign-only KMS backend, enabling
    /// the mainnet KMS signing path (no private-key bytes in process).
    pub fn with_kms(
        keystore: Arc<K>,
        grants: Arc<G>,
        ledger: Arc<L>,
        ship_gate: ShipGate,
        kms: Arc<dyn KmsSigner>,
        custody_policy: Arc<dyn KeyCustodyPolicy>,
    ) -> Self {
        Self {
            keystore,
            grants,
            ledger,
            ship_gate,
            kms: Some(kms),
            custody_policy,
        }
    }

    /// Run authorization: ship-gate (deciding hot-key vs KMS path), custody
    /// policy, EXACT chain binding, the one-shot grant claim, and the sign-time
    /// hash re-check — all BEFORE any key access. Returns the chosen signing
    /// path and the public keystore binding.
    ///
    /// Every early return here happens before any private-key consumption or
    /// KMS sign call, preserving the "no key access on failure" property.
    async fn authorize(
        &self,
        req: &CustodialSignRequest,
        requested_family: ChainFamily,
    ) -> Result<Authorized, ChainSigningError> {
        // --- Ship-gate (threat #18): mainnet => KMS path; testnet => hot key. ---
        let path = self.ship_gate.authorize_chain(req.chain.as_str())?;

        // --- Injectable custody policy (deny-first defaults). ---
        if let CustodyDecision::Deny { reason } =
            self.custody_policy.authorize_sign(&req.context, &req.chain)
        {
            return Err(ChainSigningError::PolicyDenied { reason });
        }

        // --- EXACT chain binding (review finding #2): family alone is not
        //     enough (an eip155:1 key must not sign an eip155:10 tx). Require
        //     full equality among the context chain id, the bound key chain,
        //     the decoded tx's chain/network, and the typed family. ---
        let tx_family = ChainFamily::of_transaction(&req.decoded);
        let bound_chain = req.chain.as_str();
        let tx_network = req.decoded.chain_network(); // e.g. "eip155:1"
        if tx_family != requested_family
            || requested_family != req.chain.family()
            || bound_chain != req.context.chain_id.as_str()
            || bound_chain != tx_network
        {
            return Err(ChainSigningError::ChainMismatch {
                bound: req.chain.to_string(),
                requested: tx_network,
            });
        }

        // --- Read the public binding (no key access) and require its chain to
        //     match exactly too, so a key row mis-filed under the wrong chain
        //     cannot be used. ---
        let binding = self
            .keystore
            .binding(&req.scope, &req.chain)
            .await
            .map_err(|e| ChainSigningError::KeyStore {
                reason: e.to_string(),
            })?;
        if binding.chain.as_str() != bound_chain {
            return Err(ChainSigningError::ChainMismatch {
                bound: binding.chain.to_string(),
                requested: bound_chain.to_string(),
            });
        }

        // --- Enforcement point #1: claim the sealed one-shot grant. ---
        // Refuse to sign without a successfully-claimed grant. A second claim
        // of the same grant fails (one-shot), so a replayed approval cannot
        // produce a second signature.
        let grant_key = GrantKey::from_context(&req.context, req.approved_tx_hash);
        self.grants.claim(&grant_key).await?; // GrantError -> ChainSigningError

        // --- Enforcement point #2: sign-time approved-tx-hash re-check. ---
        // Recompute the binding hash FROM THE PERSISTED decoded tx and compare
        // to the approved hash. Any post-approval mutation of `decoded` diverges
        // the hash and fails closed BEFORE any key access.
        // The signer bound to the gate/approval is carried in the signing
        // context (`key_or_account_id`), NOT derived from the decoded tx body.
        // This preserves the WYSIWYS binding: the same account the approval was
        // sealed against is folded into the recomputed hash.
        let recomputed = recompute_approved_hash(
            &req.decoded,
            req.context.key_or_account_id.as_str(),
            req.schema_version,
        )?;
        if recomputed != req.approved_tx_hash {
            return Err(ChainSigningError::ApprovedHashMismatch);
        }

        Ok(Authorized { path, binding })
    }

    /// Consume the hot key (decrypt) for the request's chain. Only used on the
    /// hot-key (testnet) path.
    async fn consume_hot_key(
        &self,
        req: &CustodialSignRequest,
        requested_family: ChainFamily,
    ) -> Result<ConsumedChainKey, ChainSigningError> {
        self.keystore
            .consume(&req.scope, &req.chain, requested_family)
            .await
            .map_err(|e| ChainSigningError::KeyStore {
                reason: e.to_string(),
            })
    }

    /// Drive the full custodial signing flow for an EVM transaction.
    ///
    /// The signable transaction is RECONSTRUCTED from `req.decoded` (the same
    /// decoded tx the approved hash was computed over), so there is no separate
    /// caller-supplied transaction that could drift from the approved one
    /// (review finding #1). Flow: authorize (all enforcement points, before any
    /// key access) -> advance ledger `Approved -> Signing` -> sign the rebuilt
    /// digest via the gated path (hot key for testnet, KMS for mainnet) with the
    /// ecrecover binding check -> advance `Signing -> Signed`.
    pub async fn sign_evm(
        &self,
        req: &CustodialSignRequest,
    ) -> Result<CustodialSignOutcome, ChainSigningError> {
        let authorized = self.authorize(req, ChainFamily::Evm).await?;

        // Reconstruct the signable tx FROM the decoded projection and compute
        // the signing digest. This is byte-identical to the wallet/HSM digest
        // (see decode::rebuild_signable tests) and derives from the exact
        // decoded tx the approved hash was computed over.
        let DecodedTransaction::Evm(evm) = &req.decoded else {
            return Err(ChainSigningError::ChainMismatch {
                bound: req.chain.to_string(),
                requested: req.decoded.chain_tag().to_string(),
            });
        };
        let rebuilt = crate::evm::decode::rebuild_signable(evm)?;
        let digest = rebuilt.signature_hash();
        let bound = bound_evm_address(&authorized.binding)?;

        // Advance the ledger into Signing only after authorization succeeds, so
        // a rejected request never moves the ledger. This happens BEFORE any key
        // consumption so a stale ledger row fails before private-key access.
        self.ledger
            .advance(&req.context.gate_ref, SigningLedgerState::Signing)
            .await?;

        let signed = match authorized.path {
            SigningPath::HotKey => {
                let consumed = self.consume_hot_key(req, ChainFamily::Evm).await?;
                let key = crate::evm::sign::signing_key_from_bytes(consumed.expose_private_key())?;
                // `consumed` (and the decrypted key) drops at the end of this arm.
                crate::evm::sign::sign_prehash_hot(digest, &key, bound)?
            }
            SigningPath::Kms => {
                let key_ref = self.require_kms_ref(&authorized)?;
                let raw = self
                    .require_kms()?
                    .sign_digest(key_ref, &digest.0, SignatureAlg::Secp256k1)
                    .await?;
                crate::evm::sign::bind_kms_signature(digest, &raw, bound)?
            }
        };

        self.ledger
            .advance(&req.context.gate_ref, SigningLedgerState::Signed)
            .await?;

        Ok(CustodialSignOutcome {
            signature: signed.signature.as_bytes().to_vec(),
            signer: format!("{:#x}", signed.recovered),
        })
    }

    /// Drive the full custodial signing flow for a Solana transaction.
    ///
    /// Like [`Self::sign_evm`], the bytes signed are the SHARED
    /// [`canonical_signing_bytes`] of `req.decoded` — the exact bytes the
    /// approved hash binds (review finding #4) — so the signed bytes cannot
    /// drift from the approved ones. Enforces all the same gates (ship-gate
    /// path, custody policy, exact chain binding, one-shot grant, sign-time
    /// hash re-check) before any key access (review finding #5).
    pub async fn sign_solana(
        &self,
        req: &CustodialSignRequest,
    ) -> Result<CustodialSignOutcome, ChainSigningError> {
        let authorized = self.authorize(req, ChainFamily::Solana).await?;

        let DecodedTransaction::Solana(sol) = &req.decoded else {
            return Err(ChainSigningError::ChainMismatch {
                bound: req.chain.to_string(),
                requested: req.decoded.chain_tag().to_string(),
            });
        };
        // The ed25519 signing digest is sha256 over the SHARED canonical bytes
        // (review finding #4): both the hot-key and the digest-oriented KMS path
        // sign the exact same 32 bytes, derived solely from `req.decoded`.
        let signing_bytes = canonical_signing_bytes(&req.decoded, req.schema_version)?;
        let digest = crate::sha256(&signing_bytes);
        let fee_payer = crate::solana::sign::fee_payer_of(sol)?;

        self.ledger
            .advance(&req.context.gate_ref, SigningLedgerState::Signing)
            .await?;

        let signed = match authorized.path {
            SigningPath::HotKey => {
                let consumed = self.consume_hot_key(req, ChainFamily::Solana).await?;
                let key =
                    crate::solana::sign::signing_key_from_bytes(consumed.expose_private_key())?;
                crate::solana::sign::sign_canonical_hot(&digest, fee_payer, &key)?
            }
            SigningPath::Kms => {
                let key_ref = self.require_kms_ref(&authorized)?;
                let raw = self
                    .require_kms()?
                    .sign_digest(key_ref, &digest, SignatureAlg::Ed25519)
                    .await?;
                crate::solana::sign::bind_kms_signature(&digest, &raw, fee_payer)?
            }
        };

        self.ledger
            .advance(&req.context.gate_ref, SigningLedgerState::Signed)
            .await?;

        Ok(CustodialSignOutcome {
            signature: signed.signature.to_vec(),
            signer: alloy_primitives::hex::encode(signed.public_key),
        })
    }

    /// Drive the full custodial signing flow for a NEAR transaction. See
    /// [`Self::sign_solana`] for the shared properties.
    pub async fn sign_near(
        &self,
        req: &CustodialSignRequest,
    ) -> Result<CustodialSignOutcome, ChainSigningError> {
        let authorized = self.authorize(req, ChainFamily::Near).await?;

        if !matches!(&req.decoded, DecodedTransaction::Near(_)) {
            return Err(ChainSigningError::ChainMismatch {
                bound: req.chain.to_string(),
                requested: req.decoded.chain_tag().to_string(),
            });
        }
        let signing_bytes = canonical_signing_bytes(&req.decoded, req.schema_version)?;
        let digest = crate::sha256(&signing_bytes);
        let expected_pubkey = ed25519_pubkey_from_binding(&authorized.binding)?;

        self.ledger
            .advance(&req.context.gate_ref, SigningLedgerState::Signing)
            .await?;

        let signed = match authorized.path {
            SigningPath::HotKey => {
                let consumed = self.consume_hot_key(req, ChainFamily::Near).await?;
                let key = crate::near::sign::signing_key_from_bytes(consumed.expose_private_key())?;
                crate::near::sign::sign_canonical_hot(&digest, &key, expected_pubkey)?
            }
            SigningPath::Kms => {
                let key_ref = self.require_kms_ref(&authorized)?;
                let raw = self
                    .require_kms()?
                    .sign_digest(key_ref, &digest, SignatureAlg::Ed25519)
                    .await?;
                crate::near::sign::bind_kms_signature(&digest, &raw, expected_pubkey)?
            }
        };

        self.ledger
            .advance(&req.context.gate_ref, SigningLedgerState::Signed)
            .await?;

        Ok(CustodialSignOutcome {
            signature: signed.signature.to_vec(),
            signer: alloy_primitives::hex::encode(signed.public_key),
        })
    }

    /// Borrow the wired KMS backend or fail closed.
    fn require_kms(&self) -> Result<&Arc<dyn KmsSigner>, ChainSigningError> {
        self.kms
            .as_ref()
            .ok_or_else(|| ChainSigningError::ShipGateRefused {
                reason: "mainnet KMS path required but no KMS backend wired".to_string(),
            })
    }

    /// Borrow the binding's KMS key reference or fail closed.
    fn require_kms_ref<'a>(
        &self,
        authorized: &'a Authorized,
    ) -> Result<&'a str, ChainSigningError> {
        authorized
            .binding
            .kms_key_ref
            .as_deref()
            .ok_or_else(|| ChainSigningError::KeyStore {
                reason: "mainnet signing requires a KMS key_ref binding".to_string(),
            })
    }

    /// Advance the ledger to `BroadcastSubmitted`. Call this immediately after
    /// the network accepts the signed transaction. The ledger refuses this for
    /// any gate_ref not currently at `Signed`, and refuses re-entry to signing
    /// afterwards (broadcast idempotency).
    pub async fn mark_broadcast_submitted(
        &self,
        ctx: &SigningContext,
    ) -> Result<(), ChainSigningError> {
        self.ledger
            .advance(&ctx.gate_ref, SigningLedgerState::BroadcastSubmitted)
            .await
            .map_err(Into::into)
    }

    /// Advance the ledger to a terminal state after broadcast.
    pub async fn finalize(
        &self,
        ctx: &SigningContext,
        terminal: SigningLedgerState,
    ) -> Result<(), ChainSigningError> {
        if !terminal.is_terminal() {
            return Err(ChainSigningError::Ledger(
                ironclaw_attestation::LedgerError::InvalidTransition {
                    from: SigningLedgerState::BroadcastSubmitted,
                    to: terminal,
                },
            ));
        }
        self.ledger
            .advance(&ctx.gate_ref, terminal)
            .await
            .map_err(Into::into)
    }
}

/// Recompute the binding [`ApprovedTxHash`] from a decoded transaction, exactly
/// as PR2 computed it at approval time (render ∥ canonical ∥ signer ∥ network ∥
/// type ∥ schema). Used by enforcement point #2.
///
/// `signer_account` is the gate-bound signer carried in the
/// [`SigningContext::key_or_account_id`] — it is NOT derived from the decoded
/// transaction body (which could be mutated post-approval). Folding the
/// explicit gate-bound signer is what preserves the WYSIWYS binding.
///
/// Fallible: render / canonicalization can fail (e.g. an unprojectable field).
/// This is a security path, so the error is propagated and signing fails closed
/// rather than proceeding against an under-described transaction.
pub fn recompute_approved_hash(
    tx: &DecodedTransaction,
    signer_account: &str,
    schema_version: RenderingSchemaVersion,
) -> Result<ApprovedTxHash, ChainSigningError> {
    Ok(approved_tx_hash_for(tx, signer_account, schema_version)?)
}

/// Parse the bound EVM address (hex, no `0x`) from a key binding.
fn bound_evm_address(
    binding: &ChainKeyBinding,
) -> Result<alloy_primitives::Address, ChainSigningError> {
    binding
        .public_address_hex
        .parse::<alloy_primitives::Address>()
        .map_err(|e| ChainSigningError::KeyStore {
            reason: format!("invalid bound EVM address: {e}"),
        })
}

/// Parse the bound ed25519 public key (32-byte hex, optional `0x`) from a
/// binding. Used as the NEAR signer-key binding (the Solana fee-payer
/// cross-check is the message's own first account key, not the binding).
fn ed25519_pubkey_from_binding(binding: &ChainKeyBinding) -> Result<[u8; 32], ChainSigningError> {
    let bytes = alloy_primitives::hex::decode(&binding.public_address_hex).map_err(|e| {
        ChainSigningError::KeyStore {
            reason: format!("bound ed25519 public key is not valid hex: {e}"),
        }
    })?;
    bytes.try_into().map_err(|_| ChainSigningError::KeyStore {
        reason: "bound ed25519 public key is not 32 bytes".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chain::ChainKeyId;

    fn binding(addr: &str) -> ChainKeyBinding {
        ChainKeyBinding {
            chain: ChainKeyId::new("eip155:1").unwrap(),
            public_address_hex: addr.to_string(),
            evm_chain_id: Some(1),
            derivation_path: "m/44'/60'/0'/0/0".to_string(),
            kms_key_ref: None,
        }
    }

    #[test]
    fn bound_evm_address_rejects_invalid_hex() {
        // A 20-byte valid address parses.
        let ok = binding("0x52908400098527886E0F7030069857D2E4169EE7");
        assert!(bound_evm_address(&ok).is_ok());
        // Non-hex / wrong-length input fails closed with a KeyStore error.
        let bad = binding("not-an-address");
        let err = bound_evm_address(&bad).unwrap_err();
        assert!(matches!(err, ChainSigningError::KeyStore { .. }));
    }

    #[test]
    fn ed25519_pubkey_from_binding_rejects_wrong_length() {
        // Exactly 32 bytes of hex parses to the pubkey.
        let ok = binding(&"11".repeat(32));
        assert_eq!(ed25519_pubkey_from_binding(&ok).unwrap(), [0x11u8; 32]);
        // 31 bytes is valid hex but the wrong length: fail closed.
        let short = binding(&"11".repeat(31));
        let err = ed25519_pubkey_from_binding(&short).unwrap_err();
        assert!(matches!(err, ChainSigningError::KeyStore { .. }));
        // Non-hex also fails closed.
        let nonhex = binding("zz");
        assert!(matches!(
            ed25519_pubkey_from_binding(&nonhex).unwrap_err(),
            ChainSigningError::KeyStore { .. }
        ));
    }
}
