//! EVM broadcast + finalization tracking.
//!
//! ## No silent fresh-nonce retry (threat #8 / broadcast idempotency)
//!
//! The single most dangerous EVM broadcast mistake is to "retry" a stuck
//! transaction by re-signing with a fresh nonce — that can produce a SECOND
//! valid transaction the user never approved. This module therefore models
//! broadcast as a one-shot submission of an already-signed payload and
//! exposes NO API that re-signs or bumps the nonce. Retrying requires a new
//! approval (a new gate_ref + grant), which the custodial signer enforces via
//! the [`ironclaw_attestation::SigningLedger`] broadcast-idempotency guard.
//!
//! The concrete RPC client (eth_sendRawTransaction + receipt polling) is wired
//! through the injectable [`EvmBroadcaster`] trait; a live JSON-RPC
//! implementation is a deferred follow-up.

use async_trait::async_trait;

use crate::error::ChainSigningError;

/// Outcome of submitting a signed EVM transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmBroadcastOutcome {
    /// The transaction hash assigned by the network.
    pub tx_hash: [u8; 32],
}

/// Submits an already-signed EVM transaction.
///
/// Implementations MUST NOT alter the signed payload (no nonce bump, no
/// re-sign). They submit the exact bytes and report the resulting hash.
#[async_trait]
pub trait EvmBroadcaster: Send + Sync {
    /// Submit the RLP-encoded signed transaction. Returns the tx hash on
    /// acceptance.
    async fn send_raw(&self, signed_rlp: &[u8]) -> Result<EvmBroadcastOutcome, ChainSigningError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// A broadcaster that records submissions and returns a canned hash, proving
    /// the trait submits the exact signed bytes with no mutation.
    struct RecordingBroadcaster {
        submissions: Mutex<Vec<Vec<u8>>>,
        hash: [u8; 32],
    }

    #[async_trait]
    impl EvmBroadcaster for RecordingBroadcaster {
        async fn send_raw(
            &self,
            signed_rlp: &[u8],
        ) -> Result<EvmBroadcastOutcome, ChainSigningError> {
            self.submissions
                .lock()
                .expect("lock")
                .push(signed_rlp.to_vec());
            Ok(EvmBroadcastOutcome { tx_hash: self.hash })
        }
    }

    #[tokio::test]
    async fn broadcaster_submits_exact_bytes() {
        let b = RecordingBroadcaster {
            submissions: Mutex::new(Vec::new()),
            hash: [7u8; 32],
        };
        let out = b.send_raw(&[1, 2, 3]).await.expect("send");
        assert_eq!(out.tx_hash, [7u8; 32]);
        assert_eq!(b.submissions.lock().unwrap().as_slice(), &[vec![1, 2, 3]]);
    }
}
