//! EVM secp256k1 signing over EIP-1559 / legacy / EIP-2930 with a mandatory
//! ecrecover signer-binding check (threat #5).
//!
//! The signing digest is computed by alloy's
//! [`SignableTransaction::signature_hash`] (the correct keccak256 over the
//! RLP-encoded unsigned payload, including the EIP-2718 type byte). We sign that
//! prehash with `k256` and then **recover the signer from the produced
//! signature and assert it equals the bound keystore account**. If recovery
//! does not match, the signature is discarded and signing fails closed
//! ([`ChainSigningError::SignerMismatch`]).

use alloy_primitives::{Address, B256, Signature};
use k256::ecdsa::SigningKey;

use crate::error::ChainSigningError;

/// The address recovered from a freshly produced signature, plus the signature
/// itself (alloy form). Returned by [`sign_with_binding_check`] only when the
/// recovered address equals the bound account.
#[derive(Debug, Clone)]
pub struct EvmSignature {
    /// The 65-byte (r ∥ s ∥ v) signature.
    pub signature: Signature,
    /// The recovered signer address (== bound account, by construction).
    pub recovered: Address,
}

/// Parse a 32-byte secp256k1 private key into a `k256` signing key.
///
/// `pub(crate)`: raw key consumption is only reachable inside the guarded
/// custodial flow (review finding #5), never from outside the crate.
pub(crate) fn signing_key_from_bytes(bytes: &[u8]) -> Result<SigningKey, ChainSigningError> {
    SigningKey::from_slice(bytes).map_err(|e| ChainSigningError::Sign {
        chain: "evm",
        // The error type from k256 does not include key bytes; still, keep the
        // message generic.
        reason: format!("invalid secp256k1 private key: {e}"),
    })
}

/// Derive the EVM address bound to a private key.
///
/// This is a PUBLIC-key derivation (no secret leaves), used during key
/// bootstrap/binding to record the public address. It is kept `pub` so callers
/// can compute the binding address; it never exposes private material.
pub fn address_of(key: &SigningKey) -> Address {
    Address::from_public_key(key.verifying_key())
}

/// Recover the EVM signer from a 65-byte (r∥s∥v) signature over `digest` and
/// enforce that it equals `bound_account`.
///
/// This is the binding half of threat #5, factored so BOTH the hot-key path
/// and the KMS path enforce it identically: whichever backend produced the
/// signature, we independently ecrecover the signer over the exact digest that
/// was signed and reject (fail closed) on any mismatch — a corrupt key, a wrong
/// keystore binding, or a malleable/foreign signature can never pass.
pub(crate) fn bind_recovered_signer(
    digest: B256,
    signature: Signature,
    bound_account: Address,
) -> Result<EvmSignature, ChainSigningError> {
    let recovered = signature
        .recover_address_from_prehash(&digest)
        .map_err(|e| ChainSigningError::Sign {
            chain: "evm",
            reason: format!("signer recovery failed: {e}"),
        })?;

    if recovered != bound_account {
        return Err(ChainSigningError::SignerMismatch);
    }

    Ok(EvmSignature {
        signature,
        recovered,
    })
}

/// Sign a precomputed signing `digest` with a hot (in-process) secp256k1 key
/// and enforce the recovered signer equals `bound_account`.
///
/// The caller derives `digest` from the SAME decoded transaction the approved
/// hash was computed over (see [`crate::evm::decode::rebuild_signable`]); there
/// is no separate caller-supplied signable transaction (review finding #1).
pub(crate) fn sign_prehash_hot(
    digest: B256,
    key: &SigningKey,
    bound_account: Address,
) -> Result<EvmSignature, ChainSigningError> {
    let (sig, recid) = key
        .sign_prehash_recoverable(digest.as_slice())
        .map_err(|e| ChainSigningError::Sign {
            chain: "evm",
            reason: format!("prehash signing failed: {e}"),
        })?;
    let signature = Signature::from((sig, recid));
    bind_recovered_signer(digest, signature, bound_account)
}

/// Reconstruct an alloy [`Signature`] from KMS-returned raw signature bytes and
/// enforce the recovered signer equals `bound_account`.
///
/// A sign-only KMS/HSM returns either a 64-byte (r∥s) or 65-byte (r∥s∥v)
/// signature; some backends do not return the recovery id. When `v` is absent
/// we brute-force the two candidate recovery ids and keep the one that recovers
/// to the bound account — still fully bound, since a signature that recovers to
/// neither candidate is rejected. This is the path used when no private key
/// bytes ever entered the process (mainnet / secure custody, finding #3).
pub(crate) fn bind_kms_signature(
    digest: B256,
    raw: &[u8],
    bound_account: Address,
) -> Result<EvmSignature, ChainSigningError> {
    let (r_s, explicit_v): (&[u8], Option<u8>) = match raw.len() {
        64 => (raw, None),
        65 => (&raw[..64], Some(raw[64])),
        other => {
            return Err(ChainSigningError::Sign {
                chain: "evm",
                reason: format!("KMS signature must be 64 or 65 bytes, got {other}"),
            });
        }
    };

    let try_v = |v_parity: bool| -> Option<EvmSignature> {
        let signature = Signature::from_bytes_and_parity(r_s, v_parity);
        bind_recovered_signer(digest, signature, bound_account).ok()
    };

    if let Some(v) = explicit_v {
        // Normalize EIP-155 / legacy v to a y-parity bit.
        let parity = match v {
            0 | 27 => false,
            1 | 28 => true,
            _ => v % 2 == 0,
        };
        if let Some(sig) = try_v(parity) {
            return Ok(sig);
        }
    }
    // Fall back to trying both parities (KMS without recid).
    try_v(false)
        .or_else(|| try_v(true))
        .ok_or(ChainSigningError::SignerMismatch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_consensus::{SignableTransaction, TxEip1559};
    use alloy_primitives::{Bytes, TxKind, U256, address};
    use k256::ecdsa::SigningKey;

    fn sample_key() -> SigningKey {
        // Deterministic non-zero scalar for reproducible tests.
        SigningKey::from_slice(&[0x11u8; 32]).expect("valid key")
    }

    fn sample_digest() -> B256 {
        TxEip1559 {
            chain_id: 1,
            nonce: 1,
            gas_limit: 21000,
            max_fee_per_gas: 100,
            max_priority_fee_per_gas: 1,
            to: TxKind::Call(address!("00000000000000000000000000000000000000aa")),
            value: U256::from(1u64),
            access_list: Default::default(),
            input: Bytes::new(),
        }
        .signature_hash()
    }

    #[test]
    fn sign_recovers_to_bound_account() {
        let key = sample_key();
        let bound = address_of(&key);
        let sig = sign_prehash_hot(sample_digest(), &key, bound).expect("sign");
        assert_eq!(sig.recovered, bound);
    }

    #[test]
    fn sign_rejects_when_bound_account_is_wrong() {
        let key = sample_key();
        let wrong = address!("00000000000000000000000000000000000000bb");
        let err = sign_prehash_hot(sample_digest(), &key, wrong).unwrap_err();
        assert!(matches!(err, ChainSigningError::SignerMismatch));
    }

    #[test]
    fn kms_signature_binds_without_recovery_id() {
        // Produce a real signature, strip the recovery id, and prove the KMS
        // binding path recovers it by trying both parities.
        let key = sample_key();
        let bound = address_of(&key);
        let digest = sample_digest();
        let hot = sign_prehash_hot(digest, &key, bound).expect("sign");
        let r_s = &hot.signature.as_bytes()[..64];
        let sig = bind_kms_signature(digest, r_s, bound).expect("kms bind");
        assert_eq!(sig.recovered, bound);
    }

    #[test]
    fn kms_signature_rejects_foreign_signer() {
        let key = sample_key();
        let bound = address_of(&key);
        let digest = sample_digest();
        let hot = sign_prehash_hot(digest, &key, bound).expect("sign");
        let r_s = &hot.signature.as_bytes()[..64];
        let wrong = address!("00000000000000000000000000000000000000bb");
        let err = bind_kms_signature(digest, r_s, wrong).unwrap_err();
        assert!(matches!(err, ChainSigningError::SignerMismatch));
    }
}
