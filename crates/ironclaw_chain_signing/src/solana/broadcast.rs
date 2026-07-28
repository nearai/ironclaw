//! Solana broadcast + confirmation tracking.
//!
//! ## No silent fresh-blockhash retry (broadcast idempotency)
//!
//! Re-broadcasting a Solana transaction with a fresh `recent_blockhash` after
//! the original expires creates a NEW transaction the user never approved.
//! This module submits an already-signed transaction one-shot and exposes no
//! API that re-signs or refreshes the blockhash; a new blockhash requires a new
//! approval (new gate_ref + grant), enforced by the signing-ledger guard.

use async_trait::async_trait;

use crate::error::ChainSigningError;

/// Outcome of submitting a signed Solana transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SolanaBroadcastOutcome {
    /// The transaction signature (base58 in practice; raw bytes here).
    pub signature: [u8; 64],
}

/// Submits an already-signed Solana transaction.
#[async_trait]
pub trait SolanaBroadcaster: Send + Sync {
    /// Submit the serialized signed transaction. MUST NOT refresh the blockhash
    /// or re-sign.
    async fn send_transaction(
        &self,
        signed_tx: &[u8],
    ) -> Result<SolanaBroadcastOutcome, ChainSigningError>;
}
