//! Solana ed25519 signing over the canonical signing bytes, with a fee-payer
//! pubkey binding check.
//!
//! ## What is signed (review finding #4)
//!
//! Rather than re-deriving a SEPARATE synthetic projection of the message here
//! (which could drift from the bytes the approved hash was computed over), the
//! custodial signer hands this function the EXACT
//! [`ironclaw_attestation::canonical_signing_bytes`] of the decoded
//! transaction — the single source of truth the [`ironclaw_signing_provider::ApprovedTxHash`]
//! binds. The bytes signed are therefore byte-identical to the approved bytes by
//! construction.
//!
//! The current canonical encoding is a deterministic, domain-separated field
//! projection (PR2), not the `solana-sdk` `Message::serialize` wire layout —
//! producing the on-wire bytes (so the signature is directly broadcastable)
//! requires the heavy `solana-sdk` crate and is the deferred next slice, flagged
//! in the crate docs / PR body. The equality-with-the-approved-hash property and
//! the ed25519 + fee-payer-binding security checks are fully exercised here.

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};

use ironclaw_attestation::SolanaTransaction;

use crate::error::ChainSigningError;

/// A produced Solana signature plus the signer pubkey (== fee payer).
#[derive(Debug, Clone)]
pub struct SolanaSignature {
    /// The 64-byte ed25519 signature.
    pub signature: [u8; 64],
    /// The signer's 32-byte public key (the message's first account key).
    pub public_key: [u8; 32],
}

/// Parse a 32-byte ed25519 secret seed into a signing key.
///
/// `pub(crate)`: raw key consumption stays inside the guarded custodial flow
/// (review finding #5).
pub(crate) fn signing_key_from_bytes(bytes: &[u8]) -> Result<SigningKey, ChainSigningError> {
    let arr: [u8; 32] = bytes.try_into().map_err(|_| ChainSigningError::Sign {
        chain: "solana",
        reason: "ed25519 secret key must be 32 bytes".to_string(),
    })?;
    Ok(SigningKey::from_bytes(&arr))
}

/// The 32-byte public key for a signing key. `pub(crate)`: keystore-binding and
/// tests only.
pub(crate) fn public_key_of(key: &SigningKey) -> [u8; 32] {
    key.verifying_key().to_bytes()
}

/// The fee payer (first required signer) pubkey for a Solana message.
pub(crate) fn fee_payer_of(tx: &SolanaTransaction) -> Result<[u8; 32], ChainSigningError> {
    Ok(tx
        .static_account_keys
        .first()
        .ok_or(ChainSigningError::Sign {
            chain: "solana",
            reason: "message has no fee payer account key".to_string(),
        })?
        .0)
}

/// Sign the 32-byte `digest` (the sha256 commitment of the canonical signing
/// bytes, review finding #4) with a hot ed25519 key and enforce that the signer
/// pubkey equals `fee_payer` (the message's first/required-signer account key).
///
/// Mismatch fails closed: a key that is not the declared fee payer cannot
/// produce a usable signature.
pub(crate) fn sign_canonical_hot(
    digest: &[u8; 32],
    fee_payer: [u8; 32],
    key: &SigningKey,
) -> Result<SolanaSignature, ChainSigningError> {
    let pubkey = public_key_of(key);
    if pubkey != fee_payer {
        return Err(ChainSigningError::SignerMismatch);
    }
    let sig: Signature = key.sign(digest);
    verify_and_wrap(digest, sig.to_bytes(), pubkey)
}

/// Bind a KMS-produced ed25519 signature: verify it against the fee-payer
/// pubkey and `digest`, failing closed on any mismatch. No private-key bytes
/// were in process.
pub(crate) fn bind_kms_signature(
    digest: &[u8; 32],
    raw: &[u8],
    fee_payer: [u8; 32],
) -> Result<SolanaSignature, ChainSigningError> {
    let sig: [u8; 64] = raw.try_into().map_err(|_| ChainSigningError::Sign {
        chain: "solana",
        reason: "ed25519 signature must be 64 bytes".to_string(),
    })?;
    verify_and_wrap(digest, sig, fee_payer)
}

/// Verify an ed25519 signature against `pubkey` and `msg`, returning the wrapped
/// signature on success.
fn verify_and_wrap(
    msg: &[u8],
    sig_bytes: [u8; 64],
    pubkey: [u8; 32],
) -> Result<SolanaSignature, ChainSigningError> {
    let vk = VerifyingKey::from_bytes(&pubkey).map_err(|e| ChainSigningError::Sign {
        chain: "solana",
        reason: format!("invalid verifying key: {e}"),
    })?;
    let sig = Signature::from_bytes(&sig_bytes);
    vk.verify_strict(msg, &sig)
        .map_err(|_| ChainSigningError::SignerMismatch)?;
    Ok(SolanaSignature {
        signature: sig_bytes,
        public_key: pubkey,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_attestation::{
        Bytes32, RenderingSchemaVersion, SolanaCompiledInstruction, SolanaMessageHeader,
        SolanaMessageVersion, canonical_signing_bytes,
    };

    fn key() -> SigningKey {
        SigningKey::from_bytes(&[0x22u8; 32])
    }

    fn tx(fee_payer: [u8; 32]) -> SolanaTransaction {
        let program = Bytes32([9u8; 32]);
        SolanaTransaction {
            cluster: "mainnet-beta".into(),
            version: SolanaMessageVersion::Legacy,
            header: SolanaMessageHeader {
                num_required_signatures: 1,
                num_readonly_signed_accounts: 0,
                num_readonly_unsigned_accounts: 1,
            },
            // Index 0 is the fee payer (the bound signer); index 1 is the program.
            static_account_keys: vec![Bytes32(fee_payer), program],
            recent_blockhash: Bytes32([2u8; 32]),
            instructions: vec![SolanaCompiledInstruction {
                program_id_index: 1,
                account_indices: vec![0],
                data: vec![1],
            }],
            address_table_lookups: vec![],
        }
    }

    fn digest(t: &SolanaTransaction) -> [u8; 32] {
        let canonical = canonical_signing_bytes(
            &ironclaw_attestation::DecodedTransaction::Solana(t.clone()),
            RenderingSchemaVersion::CURRENT,
        )
        .unwrap();
        crate::sha256(&canonical)
    }

    #[test]
    fn signs_when_key_is_fee_payer() {
        let k = key();
        let t = tx(public_key_of(&k));
        let fp = fee_payer_of(&t).unwrap();
        let sig = sign_canonical_hot(&digest(&t), fp, &k).expect("sign");
        assert_eq!(sig.public_key, public_key_of(&k));
    }

    #[test]
    fn rejects_when_key_is_not_fee_payer() {
        let k = key();
        let t = tx([0xff; 32]);
        let fp = fee_payer_of(&t).unwrap();
        let err = sign_canonical_hot(&digest(&t), fp, &k).unwrap_err();
        assert!(matches!(err, ChainSigningError::SignerMismatch));
    }
}
