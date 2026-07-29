//! Driver-side verification of a hardware-device signature (Ledger, §D).
//!
//! ## Why this lives in the driver and not in a provider
//!
//! A `SigningProvider` sees a proof and a context. It does NOT hold the
//! authoritative binding, and it cannot rebuild the signable from
//! `binding.decoded` — only the driver can. That rebuild is the entire point:
//! the question a hardware ceremony has to answer is not "is this a valid
//! signature?" but **"is this a signature, by the account we bound, over the
//! exact transaction we bound?"** Answering it requires the decoded binding and
//! the rebuild, so the check belongs where both live.
//!
//! ## The chain this establishes
//!
//! A Ledger renders a transaction on its own screen and signs a digest. It has
//! no idea what IronClaw approved. So the device's signature is only meaningful
//! once we prove the digest it signed is the digest of the transaction that was
//! approved:
//!
//! 1. The binding's chain must match the network its own decoded tx encodes —
//!    otherwise a testnet-bound gate could carry a mainnet transaction.
//! 2. The approved hash, recomputed from the persisted decoded tx and the
//!    gate-bound signer, must still equal the one recorded at approval — a
//!    binding mutated after approval fails here, before any crypto runs.
//! 3. The signable is rebuilt **from the binding**, never from anything the
//!    caller sent, and its signature hash is the only digest we accept.
//! 4. The signature must recover to the gate-bound account.
//!
//! Break any link and the device's screen stops being evidence about the
//! transaction IronClaw will broadcast. In particular step 3 is what defeats
//! the central attack: showing the user transaction A on the device while the
//! server holds transaction B. The signature would be perfectly valid — over
//! the wrong digest — and is refused.
//!
//! ## What this does NOT establish
//!
//! That the device *displayed* the transaction faithfully. That is the device's
//! own firmware guarantee plus the ERC-7730 descriptor, and no amount of
//! server-side checking substitutes for it. This module makes the server's half
//! sound; clear-signing makes the human's half sound.

use alloy_consensus::SignableTransaction;
use alloy_primitives::Signature;

use ironclaw_chain_signing::recompute_approved_hash;
use ironclaw_wallet_external::verify_evm_signer_over_digest;

use crate::binding::AttestedGateBinding;
use crate::driver::{EvmSignable, RebuildError};

/// Why a device signature was refused.
///
/// Deliberately coarse and non-positional: a caller learns that verification
/// failed, not which link broke in a way that would let them search for one
/// that passes.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum DeviceSignatureError {
    /// The binding's chain disagrees with the network its decoded tx encodes.
    #[error("binding chain does not match the decoded transaction's network")]
    ChainMismatch,

    /// The approved hash no longer recomputes from the persisted binding.
    #[error("binding no longer matches its approved transaction hash")]
    ApprovedHashMismatch,

    /// The decoded transaction could not be rebuilt into a signable.
    #[error("could not rebuild the approved transaction: {reason}")]
    Rebuild {
        /// Sanitized rebuild failure.
        reason: String,
    },

    /// The signature did not recover to the gate-bound account, or was
    /// malformed / malleable.
    #[error("device signature did not verify against the bound signer")]
    SignerMismatch,
}

/// The digest a device must have signed for `signable`.
///
/// Exposed so the ceremony's browser half can be tested against exactly the
/// bytes the server will demand, rather than against its own re-derivation.
pub fn signable_digest(signable: &EvmSignable) -> [u8; 32] {
    let hash = match signable {
        EvmSignable::Eip1559(tx) => SignableTransaction::<Signature>::signature_hash(tx),
        EvmSignable::Legacy(tx) => SignableTransaction::<Signature>::signature_hash(tx),
        EvmSignable::Eip2930(tx) => SignableTransaction::<Signature>::signature_hash(tx),
    };
    hash.0
}

/// Verify a hardware-device signature against the authoritative binding.
///
/// `signature` is the 65-byte `(r ∥ s ∥ v)` the device produced. Nothing else
/// from the caller is consulted — the transaction, the digest, and the expected
/// signer all come from the binding.
pub fn verify_device_signature(
    binding: &AttestedGateBinding,
    signature: &[u8],
) -> Result<(), DeviceSignatureError> {
    // 1. The binding's own chain must match the network its decoded tx encodes.
    //    Checked first and cheaply: it needs no crypto and closes the
    //    testnet-gate / mainnet-transaction smuggle outright.
    if binding.chain.as_str() != binding.decoded.chain_network() {
        return Err(DeviceSignatureError::ChainMismatch);
    }

    // 2. The approved hash must still recompute from the persisted decode, with
    //    the GATE-BOUND signer folded in — never anything from the decoded tx
    //    body, which a post-approval mutation could have moved. A tampered
    //    binding dies here, before we spend any recovery on it.
    let recomputed = recompute_approved_hash(
        &binding.decoded,
        binding.context.key_or_account_id.as_str(),
        binding.schema_version,
    )
    .map_err(|_| DeviceSignatureError::ApprovedHashMismatch)?;
    if recomputed != binding.approved_tx_hash {
        return Err(DeviceSignatureError::ApprovedHashMismatch);
    }

    // 3. Rebuild the signable FROM THE BINDING. This is the step that makes the
    //    device's signature mean something about our transaction: we never
    //    accept a caller-supplied digest, so a device shown a different
    //    transaction produces a signature over a digest we will not test
    //    against.
    let signable = crate::driver::rebuild_evm_signable_for_device(&binding.decoded).map_err(
        |error: RebuildError| DeviceSignatureError::Rebuild {
            reason: error.to_string(),
        },
    )?;
    let digest = signable_digest(&signable);

    // 4. And it must be the bound account that signed it. The shared helper
    //    also rejects malformed and high-S (malleable) signatures.
    verify_evm_signer_over_digest(
        &digest,
        signature,
        binding.context.key_or_account_id.as_str(),
    )
    .map_err(|_| DeviceSignatureError::SignerMismatch)
}

#[cfg(test)]
mod tests {
    use super::*;

    use ironclaw_attestation::{
        DecodedTransaction, EvmAddress, EvmTransaction, RenderingSchemaVersion,
    };
    use ironclaw_chain_signing::ChainKeyId;
    use ironclaw_host_api::{InvocationId, ProjectId, ResourceScope};
    use ironclaw_signing_provider::{
        ActorId, ChainId, GateRef, KeyOrAccountId, ProviderId, RunId, ScopeId, SigningContext,
        TenantId, UserId,
    };
    use k256::ecdsa::{RecoveryId, Signature as EcSignature, SigningKey};
    use sha3::{Digest, Keccak256};

    /// A fixed device key. In production this half lives on the Ledger and
    /// never leaves it; here it stands in so the whole chain is exercised with
    /// no hardware.
    fn device_key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32].into()).expect("valid scalar")
    }

    /// The EVM address for a key, the way the chain derives it.
    fn address_of(key: &SigningKey) -> String {
        let point = key.verifying_key().to_encoded_point(false);
        let hash = Keccak256::digest(&point.as_bytes()[1..]);
        format!("0x{}", hex::encode(&hash[12..]))
    }

    /// Sign `digest` as the device would: 65 bytes, `v` in {27, 28}.
    fn device_sign(key: &SigningKey, digest: &[u8; 32]) -> Vec<u8> {
        let (signature, recovery): (EcSignature, RecoveryId) = key
            .sign_prehash_recoverable(digest)
            .expect("sign the prehash");
        let mut bytes = signature.to_bytes().to_vec();
        bytes.push(27 + recovery.to_byte());
        bytes
    }

    fn evm_tx(nonce: u64) -> EvmTransaction {
        EvmTransaction {
            chain_id: 11155111,
            nonce,
            tx_type: 2,
            to: Some(EvmAddress([0x22; 20])),
            value: vec![],
            data: vec![],
            gas_limit: 21_000,
            gas_price: None,
            max_fee_per_gas: Some(vec![0x09]),
            max_priority_fee_per_gas: Some(vec![0x3b]),
            access_list: vec![],
            max_fee_per_blob_gas: None,
            blob_versioned_hashes: vec![],
        }
    }

    /// A binding whose approved hash is correct for its own decoded tx.
    fn binding_for(key: &SigningKey, tx: EvmTransaction) -> AttestedGateBinding {
        let decoded = DecodedTransaction::Evm(tx);
        let signer = address_of(key);
        let approved_tx_hash =
            recompute_approved_hash(&decoded, &signer, RenderingSchemaVersion::CURRENT)
                .expect("recompute");

        AttestedGateBinding {
            provider_id: ProviderId::Injected,
            context: SigningContext {
                tenant: TenantId::new("tenant-a"),
                user: UserId::new("alice"),
                scope: ScopeId::new("scope-x"),
                actor: ActorId::new("actor-7"),
                run_id: RunId::new("run-1"),
                gate_ref: GateRef::new("gate:device"),
                key_or_account_id: KeyOrAccountId::new(&signer),
                chain_id: ChainId::new("eip155:11155111"),
            },
            approved_tx_hash,
            decoded: decoded.clone(),
            chain: ChainKeyId::new(decoded.chain_network()).expect("valid chain id in test"),
            scope: ResourceScope {
                tenant_id: ironclaw_host_api::TenantId::new("tenant-a").expect("tenant"),
                user_id: ironclaw_host_api::UserId::new("alice").expect("user"),
                agent_id: None,
                project_id: Some(ProjectId::new("bootstrap").expect("project")),
                mission_id: None,
                thread_id: None,
                invocation_id: InvocationId::new(),
            },
            schema_version: RenderingSchemaVersion::CURRENT,
        }
    }

    fn digest_of(binding: &AttestedGateBinding) -> [u8; 32] {
        let signable =
            crate::driver::rebuild_evm_signable_for_device(&binding.decoded).expect("rebuild");
        signable_digest(&signable)
    }

    #[test]
    fn the_bound_device_signing_the_bound_transaction_verifies() {
        let key = device_key(0x11);
        let binding = binding_for(&key, evm_tx(1));
        let signature = device_sign(&key, &digest_of(&binding));

        assert_eq!(verify_device_signature(&binding, &signature), Ok(()));
    }

    /// THE attack this module exists to stop: the user is shown transaction A
    /// on the device while the server holds transaction B. The signature is
    /// cryptographically perfect — over the wrong digest.
    #[test]
    fn a_signature_over_a_different_transaction_is_refused() {
        let key = device_key(0x11);
        let bound = binding_for(&key, evm_tx(1));
        // The device was shown — and faithfully signed — a different nonce.
        let shown = binding_for(&key, evm_tx(999));
        let signature = device_sign(&key, &digest_of(&shown));

        assert_eq!(
            verify_device_signature(&bound, &signature),
            Err(DeviceSignatureError::SignerMismatch),
            "a valid signature over the wrong transaction must not pass"
        );
    }

    /// A different device — an attacker's — signing the right transaction.
    #[test]
    fn a_signature_from_an_unbound_device_is_refused() {
        let bound_key = device_key(0x11);
        let attacker = device_key(0x22);
        let binding = binding_for(&bound_key, evm_tx(1));
        let signature = device_sign(&attacker, &digest_of(&binding));

        assert_eq!(
            verify_device_signature(&binding, &signature),
            Err(DeviceSignatureError::SignerMismatch)
        );
    }

    /// A binding mutated after approval must die before any recovery runs —
    /// the recompute is the tamper-evidence seal on the decoded transaction.
    #[test]
    fn a_binding_mutated_after_approval_is_refused_before_any_crypto() {
        let key = device_key(0x11);
        let mut binding = binding_for(&key, evm_tx(1));
        // Sign what the user actually approved...
        let signature = device_sign(&key, &digest_of(&binding));
        // ...then move the transaction under it.
        binding.decoded = DecodedTransaction::Evm(evm_tx(2));

        assert_eq!(
            verify_device_signature(&binding, &signature),
            Err(DeviceSignatureError::ApprovedHashMismatch)
        );
    }

    /// A testnet-bound gate must not be able to carry a mainnet transaction.
    #[test]
    fn a_chain_that_disagrees_with_its_own_transaction_is_refused() {
        let key = device_key(0x11);
        let mut binding = binding_for(&key, evm_tx(1));
        let signature = device_sign(&key, &digest_of(&binding));
        binding.chain = ChainKeyId::new("eip155:1").expect("valid chain id in test");

        assert_eq!(
            verify_device_signature(&binding, &signature),
            Err(DeviceSignatureError::ChainMismatch),
            "the chain check must run before anything else"
        );
    }

    /// Malleability: the same signature with a flipped S must not also pass,
    /// or one approval yields two broadcastable signatures.
    #[test]
    fn a_high_s_malleable_signature_is_refused() {
        let key = device_key(0x11);
        let binding = binding_for(&key, evm_tx(1));
        let mut signature = device_sign(&key, &digest_of(&binding));

        // Flip S to the high half: s' = n - s, and flip the recovery bit.
        let s = k256::NonZeroScalar::try_from(&signature[32..64]).expect("nonzero s");
        let flipped = (-*s).to_bytes();
        signature[32..64].copy_from_slice(&flipped);
        signature[64] = if signature[64] == 27 { 28 } else { 27 };

        assert_eq!(
            verify_device_signature(&binding, &signature),
            Err(DeviceSignatureError::SignerMismatch)
        );
    }

    #[test]
    fn a_malformed_signature_is_refused_rather_than_panicking() {
        let key = device_key(0x11);
        let binding = binding_for(&key, evm_tx(1));

        for bad in [vec![], vec![0u8; 64], vec![0u8; 66], vec![0xFF; 65]] {
            assert_eq!(
                verify_device_signature(&binding, &bad),
                Err(DeviceSignatureError::SignerMismatch),
                "a {}-byte signature must be refused cleanly",
                bad.len()
            );
        }
    }
}
