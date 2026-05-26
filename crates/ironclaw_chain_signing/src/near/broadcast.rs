//! NEAR broadcast + finalization tracking.
//!
//! ## No silent nonce bump (broadcast idempotency)
//!
//! NEAR access keys carry a monotonic nonce; re-broadcasting with a fresh nonce
//! after a stuck submission creates a new transaction the user never approved.
//! This module submits an already-signed transaction one-shot and exposes no
//! API that re-signs or bumps the nonce; a fresh nonce requires a new approval
//! (new gate_ref + grant), enforced by the signing-ledger guard.

use async_trait::async_trait;

use crate::error::ChainSigningError;

/// Outcome of submitting a signed NEAR transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NearBroadcastOutcome {
    /// The final transaction hash, base58 in practice.
    pub tx_hash: String,
}

/// Submits an already-signed NEAR transaction.
#[async_trait]
pub trait NearBroadcaster: Send + Sync {
    /// Submit the borsh-serialized signed transaction. MUST NOT bump the nonce
    /// or re-sign.
    async fn broadcast_tx(
        &self,
        signed_tx: &[u8],
    ) -> Result<NearBroadcastOutcome, ChainSigningError>;
}
