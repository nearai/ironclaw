//! Error type for the attestation core.
//!
//! Canonicalization is **fail-closed**: any decoded transaction that cannot be
//! reproduced byte-for-byte as the exact wallet/HSM-signed payload is rejected
//! rather than silently truncated. Silent truncation of a wire length prefix
//! would desynchronize the length from the payload and let an attacker smuggle
//! extra instructions/accounts/value past the what-you-see-is-what-you-sign
//! (WYSIWYS) binding (approve-A / sign-B). Returning an error instead of
//! truncating keeps the canonical bytes and the bound [`crate::ApprovedTxHash`]
//! an exact, injective commitment to the signed wire form.

use thiserror::Error;

/// An error produced while projecting / canonicalizing a decoded transaction.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AttestationError {
    /// A Solana compact-u16 ("shortvec") length would exceed `u16::MAX`. The
    /// Solana wire format cannot represent it, so faithfully reproducing the
    /// signed bytes is impossible — reject rather than truncate.
    #[error(
        "solana {what} length {len} exceeds the compact-u16 maximum {max}; \
         cannot reproduce the signed wire bytes"
    )]
    SolanaShortVecOverflow {
        /// Which vector overflowed (e.g. `account_indices`, `instructions`).
        what: &'static str,
        /// The offending length.
        len: usize,
        /// The compact-u16 maximum (`u16::MAX`).
        max: usize,
    },

    /// A NEAR borsh `u32` length prefix would exceed `u32::MAX`. Borsh encodes
    /// `String`/`Vec` lengths as `u32`; a longer slice cannot be faithfully
    /// serialized, so reject rather than truncate.
    #[error(
        "near borsh {what} length {len} exceeds the u32 maximum {max}; \
         cannot reproduce the signed wire bytes"
    )]
    NearBorshLengthOverflow {
        /// Which field overflowed (e.g. `string`, `bytes`, `actions`).
        what: &'static str,
        /// The offending length.
        len: usize,
        /// The borsh `u32` maximum (`u32::MAX`).
        max: usize,
    },

    /// A NEAR yoctoNEAR amount (`deposit` / `stake` / `allowance`) carried more
    /// than 16 big-endian bytes and so does not fit a borsh `u128`. The
    /// human-rendered value would not match the signed (wrapped/truncated)
    /// value — an approve-vs-sign divergence — so reject rather than wrap.
    #[error(
        "near u128 amount has {len} big-endian bytes, exceeding the 16-byte \
         u128 maximum; the rendered value would diverge from the signed value"
    )]
    NearU128Overflow {
        /// The number of big-endian bytes supplied.
        len: usize,
    },
}
