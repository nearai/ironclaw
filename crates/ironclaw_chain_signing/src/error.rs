//! Error types for custodial chain signing.
//!
//! Every variant is deliberately **key-material-free**: no private-key bytes,
//! no decrypted secret, and no recoverable seed ever appears in an error
//! `Display` or `Debug`. Signing/broadcast failures carry only opaque
//! descriptions, chain tags, and addresses (which are public).

use thiserror::Error;

use ironclaw_attestation::{AttestationError, GrantError, LedgerError};

/// Failures surfaced by the custodial signing path.
#[derive(Debug, Error)]
pub enum ChainSigningError {
    /// No grant could be claimed for this request — signing is refused. Wraps
    /// the underlying one-shot store error (e.g. `AlreadyClaimed`, `NotFound`).
    #[error("signing grant could not be claimed: {0}")]
    Grant(#[from] GrantError),

    /// The signing ledger rejected a transition (e.g. the row is past
    /// `BroadcastSubmitted`, or no row exists). Carries the ledger error.
    #[error("signing ledger rejected the operation: {0}")]
    Ledger(#[from] LedgerError),

    /// Rendering / canonicalization / approved-hash derivation in the
    /// attestation crate failed (e.g. a field could not be projected). This is a
    /// security path: fail closed rather than sign against an under-described
    /// transaction.
    #[error("attestation derivation failed: {0}")]
    Attestation(#[from] AttestationError),

    /// Sign-time enforcement point #2 failed: the `ApprovedTxHash` recomputed
    /// from the persisted decoded transaction does not equal the approved hash
    /// the grant was sealed against. Fail closed — no key is consumed.
    #[error(
        "sign-time approved-tx-hash re-check failed: persisted transaction does not match the approved hash"
    )]
    ApprovedHashMismatch,

    /// A key bound to one chain was asked to sign a transaction for a different
    /// chain. The typed `DecodedTransaction` variant and the keystore binding
    /// disagree. Fail closed.
    #[error("chain mismatch: key bound to {bound} cannot sign a {requested} transaction")]
    ChainMismatch {
        /// Chain the key is bound to.
        bound: String,
        /// Chain of the transaction presented for signing.
        requested: String,
    },

    /// The recovered EVM signer (ecrecover) does not equal the bound keystore
    /// account. Fail closed — the produced signature is discarded.
    #[error("signer mismatch: recovered signer does not equal the bound account")]
    SignerMismatch,

    /// The HSM/KMS ship-gate refused a real-value / mainnet custodial signing
    /// because no KMS backend is wired. Hot-key custodial is testnet/dev only.
    #[error("ship-gate refused custodial mainnet signing: {reason}")]
    ShipGateRefused {
        /// Human-readable refusal reason (never includes key material).
        reason: String,
    },

    /// The injectable key-custody / bootstrap policy denied the operation.
    #[error("custody policy denied the operation: {reason}")]
    PolicyDenied {
        /// Human-readable denial reason.
        reason: String,
    },

    /// The keystore could not produce usable key material for the request.
    #[error("keystore error: {reason}")]
    KeyStore {
        /// Opaque description (never key bytes).
        reason: String,
    },

    /// A chain-native decode failure.
    #[error("decode error ({chain}): {reason}")]
    Decode {
        /// Chain tag.
        chain: &'static str,
        /// Opaque description.
        reason: String,
    },

    /// A chain-native signing-primitive failure.
    #[error("signing error ({chain}): {reason}")]
    Sign {
        /// Chain tag.
        chain: &'static str,
        /// Opaque description.
        reason: String,
    },

    /// A broadcast / submission failure.
    #[error("broadcast error ({chain}): {reason}")]
    Broadcast {
        /// Chain tag.
        chain: &'static str,
        /// Opaque description.
        reason: String,
    },

    /// Untrusted RPC / token metadata failed a policy check (wrong chainId,
    /// genesis, network, etc.).
    #[error("metadata policy violation ({chain}): {reason}")]
    MetadataPolicy {
        /// Chain tag.
        chain: &'static str,
        /// Opaque description.
        reason: String,
    },
}

/// Convenience result alias for the crate.
pub type Result<T> = std::result::Result<T, ChainSigningError>;
