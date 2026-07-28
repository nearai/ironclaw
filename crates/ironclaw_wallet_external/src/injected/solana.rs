//! Solana (`window.solana`) injected-proof signature verification.
//!
//! An injected Solana wallet attests to the bound [`ApprovedTxHash`] via
//! `signMessage` over the raw 32 hash bytes, producing a 64-byte ed25519
//! signature. We verify the signature against the connected wallet's ed25519
//! public key with the vendored `ed25519-dalek` (no `solana-sdk`), and require
//! that public key to equal the bound account (threat #5).

use ed25519_dalek::{Signature, Verifier, VerifyingKey};

use ironclaw_signing_provider::SigningProviderError;

/// Verify `signature` over `hash_bytes` against `public_key`, and require
/// `public_key` to equal `bound_account`.
///
/// `bound_account` is the lowercase hex of the 32-byte ed25519 public key.
pub(super) fn verify_signer_over_hash(
    hash_bytes: &[u8; 32],
    signature: &[u8],
    public_key: &[u8],
    bound_account: &str,
) -> Result<(), SigningProviderError> {
    if signature.len() != 64 {
        return Err(SigningProviderError::ProofInvalid {
            reason: format!("solana signature must be 64 bytes, got {}", signature.len()),
        });
    }
    let pk_bytes: [u8; 32] =
        public_key
            .try_into()
            .map_err(|_| SigningProviderError::ProofInvalid {
                reason: format!(
                    "solana public key must be 32 bytes, got {}",
                    public_key.len()
                ),
            })?;

    // Signer binding (threat #5): the verifying key must match the bound
    // account before we trust any signature it produced.
    let bound = parse_solana_pubkey(bound_account)?;
    if pk_bytes != bound {
        return Err(SigningProviderError::SignerMismatch);
    }

    let verifying_key =
        VerifyingKey::from_bytes(&pk_bytes).map_err(|e| SigningProviderError::ProofInvalid {
            reason: format!("invalid solana ed25519 public key: {e}"),
        })?;
    let sig_bytes: [u8; 64] =
        signature
            .try_into()
            .map_err(|_| SigningProviderError::ProofInvalid {
                reason: "solana signature length mismatch".to_string(),
            })?;
    let sig = Signature::from_bytes(&sig_bytes);

    verifying_key
        .verify(hash_bytes, &sig)
        .map_err(|e| SigningProviderError::ProofInvalid {
            reason: format!("solana ed25519 verification failed: {e}"),
        })?;
    Ok(())
}

/// Parse the lowercase-hex 32-byte ed25519 public key bound account.
fn parse_solana_pubkey(s: &str) -> Result<[u8; 32], SigningProviderError> {
    // Decode over raw bytes: the bound account is untrusted input and may carry
    // multi-byte UTF-8 of even byte length, so `&str` byte-range slicing would
    // panic on a non-char-boundary. `&[u8]` indexing is panic-free and any
    // non-ASCII byte is rejected cleanly below.
    let stripped = s.strip_prefix("0x").unwrap_or(s).as_bytes();
    if stripped.len() != 64 {
        return Err(SigningProviderError::ProofInvalid {
            reason: format!("bound account is not a 32-byte ed25519 key: {s}"),
        });
    }
    let mut out = [0u8; 32];
    for (byte, pair) in out.iter_mut().zip(stripped.chunks_exact(2)) {
        let hi = hex_digit(pair[0])?;
        let lo = hex_digit(pair[1])?;
        *byte = (hi << 4) | lo;
    }
    Ok(out)
}

/// Decode a single ASCII hex digit byte to its 0–15 value, rejecting any
/// non-hex (including non-ASCII) byte without panicking.
fn hex_digit(b: u8) -> Result<u8, SigningProviderError> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        other => Err(SigningProviderError::ProofInvalid {
            reason: format!("bound account hex invalid digit: {other:#04x}"),
        }),
    }
}
