//! The typed, domain-separated, single-use trust-registration challenge.
//!
//! When a user wants to register a connected wallet as a *trusted signer* for
//! their account, the server issues a challenge that the wallet must sign to
//! prove control of the claimed account. The challenge is:
//!
//! * **Domain-separated** — a distinct tag (`ironclaw/trust/register/v1`) so a
//!   trust-registration signature can never be replayed as a per-gate signing
//!   attestation (which uses `ironclaw/walletconnect/attest/v1` or the EIP-191
//!   injected digest) and vice-versa.
//! * **Tenant / user / chain / network / account bound** — the challenge
//!   commits to exactly who is registering what, on which chain, so a signature
//!   minted for one `(tenant, user, chain)` cannot be replayed for another.
//! * **Nonced** — a per-challenge random nonce makes each ceremony unique.
//! * **Expiring** — a server-set expiry (`expires_at_unix_ms`) bounds the
//!   window; an expired challenge is rejected fail-closed.
//!
//! The wallet signs the 32-byte [`TrustChallenge::digest`]; per-chain verifiers
//! recover/verify the signer over exactly that digest. The challenge carries no
//! secret material — only public binding fields and a nonce.

use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};

use ironclaw_signing_provider::{ChainId, TenantId, UserId};

/// Domain-separation tag for the trust-registration challenge digest.
///
/// Deliberately distinct from any per-gate signing-attestation domain so a
/// trust-registration proof can never be cross-replayed against a gate
/// resolve (and vice-versa).
pub(crate) const TRUST_CHALLENGE_DOMAIN: &[u8] = b"ironclaw/trust/register/v1";

/// A typed, domain-separated, single-use, expiring challenge a connected
/// wallet signs to prove control of the claimed account.
///
/// Serde-serializable so it can be handed to a frontend/wallet and echoed back
/// with the signature. The signed message is the 32-byte [`Self::digest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustChallenge {
    /// Tenant boundary the registration belongs to.
    pub tenant_id: TenantId,
    /// End user registering the trusted signer.
    pub user_id: UserId,
    /// Target chain (e.g. `eip155:1`, `solana:mainnet`, `near:mainnet`).
    pub chain_id: ChainId,
    /// Network label within the chain family (e.g. `mainnet`, `testnet`).
    pub network: String,
    /// The account/key the user claims to control (EVM address, Solana pubkey
    /// hex, or NEAR `account_id`). Normalized per chain at verification time.
    pub claimed_account: String,
    /// Per-challenge random nonce (lowercase hex), supplied by a
    /// [`super::NonceSource`].
    pub nonce_hex: String,
    /// Server-set expiry as unix milliseconds. A challenge whose `now` is at or
    /// past this value is rejected.
    pub expires_at_unix_ms: u64,
}

impl TrustChallenge {
    /// Compute the 32-byte domain-separated digest the wallet signs.
    ///
    /// `keccak256(domain ∥ Σ len-prefixed(field))` over every binding field.
    /// Length-prefixing each variable-length field makes the commitment
    /// unambiguous (no concatenation collisions between distinct field tuples).
    pub fn digest(&self) -> [u8; 32] {
        let mut hasher = Keccak256::new();
        hasher.update(TRUST_CHALLENGE_DOMAIN);
        for field in [
            self.tenant_id.as_str(),
            self.user_id.as_str(),
            self.chain_id.as_str(),
            self.network.as_str(),
            self.claimed_account.as_str(),
            self.nonce_hex.as_str(),
        ] {
            hasher.update((field.len() as u64).to_be_bytes());
            hasher.update(field.as_bytes());
        }
        hasher.update(self.expires_at_unix_ms.to_be_bytes());
        hasher.finalize().into()
    }

    /// True iff `now_unix_ms` is at or past the challenge expiry.
    pub fn is_expired(&self, now_unix_ms: u64) -> bool {
        now_unix_ms >= self.expires_at_unix_ms
    }
}
