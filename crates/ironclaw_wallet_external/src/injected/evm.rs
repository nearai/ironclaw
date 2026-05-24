//! EVM (`window.ethereum`) injected-proof signer recovery.
//!
//! An injected EVM wallet attests to the bound [`ApprovedTxHash`] via
//! `personal_sign` over the raw 32 hash bytes. `personal_sign` applies the
//! EIP-191 personal-message prefix and keccak256, so the recoverable digest is
//! `keccak256("\x19Ethereum Signed Message:\n32" ∥ hash)`. We recover the
//! signer from the 65-byte (r ∥ s ∥ v) signature over that digest with `k256`
//! ecrecover, derive its 20-byte address, and require it to equal the bound
//! account (threat #5). No alloy wire stack is pulled — k256 directly.

use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};
use sha3::{Digest, Keccak256};

use ironclaw_signing_provider::SigningProviderError;

/// EIP-191 personal-message prefix for a 32-byte message.
const PERSONAL_PREFIX_32: &[u8] = b"\x19Ethereum Signed Message:\n32";

/// Recover the signer from `signature` over the EIP-191 personal-sign digest of
/// `hash_bytes`, and require it to equal `bound_account`.
///
/// `bound_account` is a `0x`-prefixed (case-insensitive) hex EVM address.
pub(super) fn verify_signer_over_hash(
    hash_bytes: &[u8; 32],
    signature: &[u8],
    bound_account: &str,
) -> Result<(), SigningProviderError> {
    if signature.len() != 65 {
        return Err(SigningProviderError::ProofInvalid {
            reason: format!("evm signature must be 65 bytes, got {}", signature.len()),
        });
    }

    // EIP-191 personal-sign digest over the 32-byte approved hash.
    let mut hasher = Keccak256::new();
    hasher.update(PERSONAL_PREFIX_32);
    hasher.update(hash_bytes);
    let digest = hasher.finalize();

    // Split r ∥ s ∥ v and normalize v to a 0/1 recovery id (accepts 27/28 and
    // EIP-155-style large v by taking the low bit's parity).
    let sig = Signature::from_slice(&signature[..64]).map_err(|e| {
        SigningProviderError::ProofInvalid {
            reason: format!("invalid evm signature scalars: {e}"),
        }
    })?;
    let v = signature[64];
    let rec_id = recovery_id_from_v(v)?;

    let recovered_key = VerifyingKey::recover_from_prehash(digest.as_slice(), &sig, rec_id)
        .map_err(|e| SigningProviderError::ProofInvalid {
            reason: format!("evm signer recovery failed: {e}"),
        })?;

    let recovered_address = address_from_verifying_key(&recovered_key);
    let bound = parse_evm_address(bound_account)?;

    if recovered_address != bound {
        return Err(SigningProviderError::SignerMismatch);
    }
    Ok(())
}

/// Normalize the signature `v` byte to a `k256` [`RecoveryId`].
///
/// Accepts the raw 0/1 form, the legacy 27/28 form, and EIP-155 `v`
/// (`35 + 2*chain_id + parity`) by reducing to parity.
fn recovery_id_from_v(v: u8) -> Result<RecoveryId, SigningProviderError> {
    let parity = match v {
        0 | 1 => v,
        27 | 28 => v - 27,
        v if v >= 35 => (v - 35) & 1,
        other => {
            return Err(SigningProviderError::ProofInvalid {
                reason: format!("invalid evm recovery id v={other}"),
            });
        }
    };
    RecoveryId::from_byte(parity).ok_or(SigningProviderError::ProofInvalid {
        reason: "invalid evm recovery id parity".to_string(),
    })
}

/// Derive the 20-byte EVM address from a recovered secp256k1 public key:
/// `keccak256(uncompressed_pubkey[1..])[12..]`.
fn address_from_verifying_key(key: &VerifyingKey) -> [u8; 20] {
    let encoded = key.to_encoded_point(false);
    // Skip the 0x04 prefix byte of the uncompressed SEC1 point.
    let pubkey_bytes = &encoded.as_bytes()[1..];
    let hash = Keccak256::digest(pubkey_bytes);
    let mut addr = [0u8; 20];
    addr.copy_from_slice(&hash[12..]);
    addr
}

/// Parse a `0x`-prefixed (case-insensitive) hex EVM address into 20 bytes.
fn parse_evm_address(s: &str) -> Result<[u8; 20], SigningProviderError> {
    let stripped = s.strip_prefix("0x").unwrap_or(s);
    if stripped.len() != 40 {
        return Err(SigningProviderError::ProofInvalid {
            reason: format!("bound account is not a 20-byte evm address: {s}"),
        });
    }
    let mut out = [0u8; 20];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&stripped[i * 2..i * 2 + 2], 16).map_err(|e| {
            SigningProviderError::ProofInvalid {
                reason: format!("bound account hex invalid: {e}"),
            }
        })?;
    }
    Ok(out)
}
