//! Per-chain control-of-account verifiers for trust registration.
//!
//! Each verifier proves the registrant controls the *claimed account* by
//! checking a signature over the [`TrustChallenge`] digest. The crypto itself
//! is **reused** from `ironclaw_wallet_external` — this module never
//! reimplements ecrecover or ed25519 verification, it only frames the
//! chain-specific binding:
//!
//! * **EVM** — EIP-191 `personal_sign` over the challenge digest; the address
//!   is recovered (k256 ecrecover) and must equal the claimed address.
//! * **Solana** — ed25519 `signMessage` over the challenge digest; verified
//!   against the claimed pubkey.
//! * **NEAR** — ed25519 over the challenge digest proving control of a
//!   *specific access key*; the proven `(account_id, public_key)` is recorded.
//!   Whether that key is actually an on-chain access key for the account
//!   requires tenant RPC — that check is delegated to a pluggable
//!   [`NearAccessKeyVerifier`] (gap-D follow-up; see below).

use sha3::{Digest, Keccak256};

use ironclaw_signing_provider::SigningProviderError;
use ironclaw_wallet_external::{verify_ed25519_signer_over_digest, verify_evm_signer_over_digest};

use super::challenge::TrustChallenge;

/// EIP-191 personal-message prefix for a 32-byte message (mirrors the injected
/// EVM provider so a wallet's `personal_sign` over the challenge verifies).
const PERSONAL_PREFIX_32: &[u8] = b"\x19Ethereum Signed Message:\n32";

/// The control-of-account evidence a completed registration proves, used to key
/// the resulting [`super::TrustedSignerBinding`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifiedControl {
    /// EVM: the recovered 20-byte address (lowercase `0x` hex) equals the claim.
    Evm { address: String },
    /// Solana: the claimed ed25519 pubkey produced a valid signature.
    Solana { pubkey: String },
    /// NEAR: a specific access key signed the challenge for the account.
    Near {
        /// The NEAR `account_id` controlled.
        account_id: String,
        /// The ed25519 access-key public key (lowercase hex) that signed.
        public_key: String,
    },
}

impl VerifiedControl {
    /// The canonical account-or-key string the binding is keyed by.
    pub fn account_or_key(&self) -> String {
        match self {
            VerifiedControl::Evm { address } => address.clone(),
            VerifiedControl::Solana { pubkey } => pubkey.clone(),
            // NEAR binds to the (account, key) pair; the account is the primary
            // identity, the key is recorded alongside it on the binding.
            VerifiedControl::Near { account_id, .. } => account_id.clone(),
        }
    }
}

/// Verify an EVM `personal_sign` (EIP-191) signature over the challenge and
/// require the recovered address to equal `challenge.claimed_account`.
pub(super) fn verify_evm(
    challenge: &TrustChallenge,
    signature: &[u8],
) -> Result<VerifiedControl, SigningProviderError> {
    let mut hasher = Keccak256::new();
    hasher.update(PERSONAL_PREFIX_32);
    hasher.update(challenge.digest());
    let eip191: [u8; 32] = hasher.finalize().into();

    verify_evm_signer_over_digest(&eip191, signature, &challenge.claimed_account)?;
    Ok(VerifiedControl::Evm {
        address: normalize_evm(&challenge.claimed_account),
    })
}

/// Verify a Solana ed25519 `signMessage` signature over the challenge digest
/// against the claimed pubkey.
pub(super) fn verify_solana(
    challenge: &TrustChallenge,
    signature: &[u8],
    public_key: &[u8],
) -> Result<VerifiedControl, SigningProviderError> {
    let digest = challenge.digest();
    verify_ed25519_signer_over_digest(&digest, signature, public_key, &challenge.claimed_account)?;
    Ok(VerifiedControl::Solana {
        pubkey: challenge
            .claimed_account
            .trim_start_matches("0x")
            .to_string(),
    })
}

/// Verify a NEAR ed25519 access-key signature over the challenge digest.
///
/// `public_key_hex` is the lowercase-hex 32-byte ed25519 access-key public key
/// the wallet signed with. The signature must verify against it; the claimed
/// `account_id` is recorded with the proven key. The on-chain check that this
/// key is *actually* a registered access key for the account is delegated to
/// `access_key_verifier` (fail-closed if it rejects).
pub(super) fn verify_near(
    challenge: &TrustChallenge,
    signature: &[u8],
    public_key_hex: &str,
    access_key_verifier: &dyn NearAccessKeyVerifier,
) -> Result<VerifiedControl, SigningProviderError> {
    let pk_bytes = decode_hex32(public_key_hex)?;
    let digest = challenge.digest();
    // The "bound account" for the ed25519 kernel is the access key itself: the
    // signature must verify against the key the registrant claims to control.
    verify_ed25519_signer_over_digest(&digest, signature, &pk_bytes, public_key_hex)?;

    let account_id = &challenge.claimed_account;
    // gap-D follow-up: the on-chain check that `public_key_hex` is a registered
    // access key for `account_id` requires tenant RPC. Delegated to the
    // pluggable verifier; FAIL CLOSED if a configured verifier rejects.
    access_key_verifier.verify_access_key(account_id, public_key_hex)?;

    Ok(VerifiedControl::Near {
        account_id: account_id.clone(),
        public_key: public_key_hex.to_string(),
    })
}

/// Pluggable on-chain NEAR access-key check.
///
/// Proving an ed25519 signature only shows the registrant holds *a* key; it
/// does not prove that key is a registered access key for the NEAR account.
/// That requires querying the tenant's NEAR RPC (gap-D). This trait abstracts
/// that check so the durable RPC implementation can be wired later; the
/// in-tree [`AlwaysTrustNearAccessKeyVerifier`] stub accepts any key (suitable
/// for tests / single-user-trusted deployments only).
///
/// **Fail-closed contract:** if a verifier is configured and returns `Err`, the
/// registration fails — the binding is never created.
pub trait NearAccessKeyVerifier: Send + Sync {
    /// Confirm `public_key_hex` is an on-chain access key for `account_id`.
    fn verify_access_key(
        &self,
        account_id: &str,
        public_key_hex: &str,
    ) -> Result<(), SigningProviderError>;
}

/// Stub [`NearAccessKeyVerifier`] that accepts any key.
///
/// For tests and single-user-trusted deployments where the on-chain access-key
/// lookup (gap-D) is not yet wired. An ed25519 signature only proves
/// *possession* of a key, not that it is a registered access key for the
/// account, so this stub is a silent privilege-escalation vector if wired into
/// a multi-tenant production binary: a user could register an arbitrary key as
/// trusted for an account they do not control.
///
/// It is therefore compiled **only** under `cfg(test)` or behind the explicit
/// opt-in `unsafe-always-trust-near` feature — a production build cannot wire it
/// by accident — and it emits a `warn!` on every use so any deployment that
/// does opt in is unmistakable in the logs. Production multi-tenant deployments
/// MUST replace it with an RPC-backed verifier.
#[cfg(any(test, feature = "unsafe-always-trust-near"))]
#[derive(Debug, Default, Clone, Copy)]
pub struct AlwaysTrustNearAccessKeyVerifier;

#[cfg(any(test, feature = "unsafe-always-trust-near"))]
impl NearAccessKeyVerifier for AlwaysTrustNearAccessKeyVerifier {
    fn verify_access_key(
        &self,
        account_id: &str,
        public_key_hex: &str,
    ) -> Result<(), SigningProviderError> {
        tracing::warn!(
            account_id,
            public_key_hex,
            "AlwaysTrustNearAccessKeyVerifier in use: on-chain NEAR access-key check BYPASSED — \
             any ed25519 key signs as trusted for this account. Never use in multi-tenant production."
        );
        Ok(())
    }
}

/// Lowercase + `0x`-normalize an EVM address for canonical binding storage.
pub(super) fn normalize_evm(addr: &str) -> String {
    let stripped = addr.strip_prefix("0x").unwrap_or(addr);
    format!("0x{}", stripped.to_ascii_lowercase())
}

/// Canonicalize a Solana account to lowercase hex of its 32-byte ed25519
/// pubkey.
///
/// Real Solana wallets (Phantom, Solflare, the wallet-adapter standard) present
/// their public key as **base58** (~44 chars), while the verification kernel
/// ([`verify_ed25519_signer_over_digest`]) and the rest of this ceremony key on
/// lowercase hex. Accept either surface form so a real wallet's registration
/// does not silently no-op, and converge both to the canonical hex form:
///
/// * 64-char hex (optionally `0x`-prefixed) → lowercased.
/// * Otherwise → base58-decoded; valid only if it yields exactly 32 bytes.
///
/// Fails closed with a clear error on anything that is neither a 32-byte hex nor
/// a 32-byte base58 key.
pub(super) fn normalize_solana_pubkey(account: &str) -> Result<String, SigningProviderError> {
    let stripped = account.strip_prefix("0x").unwrap_or(account);
    // Hex form: 64 lowercase/uppercase hex chars over the 32-byte key.
    if stripped.len() == 64 && stripped.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Ok(stripped.to_ascii_lowercase());
    }
    // Base58 form (the common real-wallet case): must decode to 32 bytes.
    let decoded =
        bs58::decode(account)
            .into_vec()
            .map_err(|e| SigningProviderError::ProofInvalid {
                reason: format!("solana account is neither 32-byte hex nor valid base58: {e}"),
            })?;
    let bytes: [u8; 32] =
        decoded
            .as_slice()
            .try_into()
            .map_err(|_| SigningProviderError::ProofInvalid {
                reason: format!(
                    "solana account base58 must decode to 32 bytes, got {}",
                    decoded.len()
                ),
            })?;
    // One byte->hex implementation for the subsystem: reuse the allocation-free
    // nibble-push helper rather than per-byte `format!`.
    Ok(super::hex_encode(&bytes))
}

/// Decode a 32-byte ed25519 public key from (optionally `0x`-prefixed) hex.
fn decode_hex32(s: &str) -> Result<[u8; 32], SigningProviderError> {
    // Decode over raw bytes: `s` is the attacker-controlled `public_key_hex`
    // (not committed in the challenge digest), so `&str` byte-range slicing
    // would panic on a non-char boundary for even-byte-length multi-byte UTF-8.
    let stripped = s.strip_prefix("0x").unwrap_or(s).as_bytes();
    if stripped.len() != 64 {
        return Err(SigningProviderError::ProofInvalid {
            reason: format!("near access-key public key must be 32-byte hex, got {s}"),
        });
    }
    let mut out = [0u8; 32];
    for (byte, pair) in out.iter_mut().zip(stripped.chunks_exact(2)) {
        *byte = (super::hex_digit(pair[0])? << 4) | super::hex_digit(pair[1])?;
    }
    Ok(out)
}
