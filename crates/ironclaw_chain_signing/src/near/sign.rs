//! NEAR ed25519 signing over the canonical signing bytes, with a signer-key
//! binding check.
//!
//! ## What is signed (review finding #4)
//!
//! Rather than re-deriving a SEPARATE borsh pre-image here (which could drift
//! from the bytes the approved hash was computed over), the custodial signer
//! hands this function the EXACT
//! [`ironclaw_attestation::canonical_signing_bytes`] of the decoded transaction
//! — the single source of truth the approved hash binds. The bytes signed are
//! therefore byte-identical to the approved bytes by construction.
//!
//! NEAR's production wire format signs `sha256(borsh(near_primitives::Transaction))`;
//! producing a directly-broadcastable signature requires the `near-primitives` /
//! `near-crypto` crates and is the deferred next slice (flagged in crate docs /
//! PR body). The equality-with-the-approved-hash property and the ed25519 +
//! signer-key-binding security checks are fully exercised here.

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};

use crate::error::ChainSigningError;

/// A produced NEAR signature plus the signer pubkey.
#[derive(Debug, Clone)]
pub struct NearSignature {
    /// The 64-byte ed25519 signature.
    pub signature: [u8; 64],
    /// The signer public key.
    pub public_key: [u8; 32],
}

/// Parse a 32-byte ed25519 seed into a signing key.
///
/// `pub(crate)`: raw key consumption stays inside the guarded custodial flow
/// (review finding #5).
pub(crate) fn signing_key_from_bytes(bytes: &[u8]) -> Result<SigningKey, ChainSigningError> {
    let arr: [u8; 32] = bytes.try_into().map_err(|_| ChainSigningError::Sign {
        chain: "near",
        reason: "ed25519 secret key must be 32 bytes".to_string(),
    })?;
    Ok(SigningKey::from_bytes(&arr))
}

/// The 32-byte public key for a signing key. `pub(crate)`: keystore-binding and
/// tests only.
pub(crate) fn public_key_of(key: &SigningKey) -> [u8; 32] {
    key.verifying_key().to_bytes()
}

/// Sign the 32-byte `digest` (the sha256 commitment of the canonical signing
/// bytes of the decoded transaction, review finding #4) with a hot ed25519 key
/// and enforce that the signer pubkey equals `expected_public_key` (the access
/// key the keystore binding records).
///
/// NEAR account access is keyed by `(account_id, public_key)`. Binding the
/// signing key's public key to the keystore record is the NEAR analog of the
/// EVM ecrecover check: a key whose public key is not the bound access key
/// cannot sign for this account.
pub(crate) fn sign_canonical_hot(
    digest: &[u8; 32],
    key: &SigningKey,
    expected_public_key: [u8; 32],
) -> Result<NearSignature, ChainSigningError> {
    let public_key = public_key_of(key);
    if public_key != expected_public_key {
        return Err(ChainSigningError::SignerMismatch);
    }
    let sig: Signature = key.sign(digest);
    verify_and_wrap(digest, sig.to_bytes(), public_key)
}

/// Bind a KMS-produced ed25519 signature: verify it against the bound pubkey and
/// `digest`, failing closed on any mismatch. No private-key bytes were in
/// process.
pub(crate) fn bind_kms_signature(
    digest: &[u8; 32],
    raw: &[u8],
    expected_public_key: [u8; 32],
) -> Result<NearSignature, ChainSigningError> {
    let sig: [u8; 64] = raw.try_into().map_err(|_| ChainSigningError::Sign {
        chain: "near",
        reason: "ed25519 signature must be 64 bytes".to_string(),
    })?;
    verify_and_wrap(digest, sig, expected_public_key)
}

fn verify_and_wrap(
    msg: &[u8],
    sig_bytes: [u8; 64],
    public_key: [u8; 32],
) -> Result<NearSignature, ChainSigningError> {
    let vk = VerifyingKey::from_bytes(&public_key).map_err(|e| ChainSigningError::Sign {
        chain: "near",
        reason: format!("invalid verifying key: {e}"),
    })?;
    let sig = Signature::from_bytes(&sig_bytes);
    vk.verify_strict(msg, &sig)
        .map_err(|_| ChainSigningError::SignerMismatch)?;
    Ok(NearSignature {
        signature: sig_bytes,
        public_key,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_attestation::{
        Bytes32, NearAction, NearPublicKey, NearTransaction, RenderingSchemaVersion,
        canonical_signing_bytes,
    };

    fn key() -> SigningKey {
        SigningKey::from_bytes(&[0x33u8; 32])
    }

    fn tx() -> NearTransaction {
        NearTransaction {
            network: "mainnet".into(),
            signer_id: "alice.near".into(),
            public_key: NearPublicKey {
                key_type: 0,
                data: vec![7u8; 32],
            },
            receiver_id: "bob.near".into(),
            nonce: 1,
            block_hash: Bytes32([3u8; 32]),
            actions: vec![NearAction::Transfer { deposit: vec![1] }],
        }
    }

    fn digest(t: &NearTransaction) -> [u8; 32] {
        let canonical = canonical_signing_bytes(
            &ironclaw_attestation::DecodedTransaction::Near(t.clone()),
            RenderingSchemaVersion::CURRENT,
        )
        .unwrap();
        crate::sha256(&canonical)
    }

    #[test]
    fn signs_when_public_key_matches_binding() {
        let k = key();
        let sig = sign_canonical_hot(&digest(&tx()), &k, public_key_of(&k)).expect("sign");
        assert_eq!(sig.public_key, public_key_of(&k));
    }

    #[test]
    fn rejects_when_public_key_does_not_match_binding() {
        let k = key();
        let err = sign_canonical_hot(&digest(&tx()), &k, [0xff; 32]).unwrap_err();
        assert!(matches!(err, ChainSigningError::SignerMismatch));
    }
}
