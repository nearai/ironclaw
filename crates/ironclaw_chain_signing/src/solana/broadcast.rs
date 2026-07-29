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

/// Live Solana broadcaster: submits the serialized signed transaction via the
/// `sendTransaction` JSON-RPC method (base64 encoding) to a configured RPC URL.
///
/// One-shot submitter of an already-signed transaction: it never refreshes the
/// `recent_blockhash` or re-signs. An expired blockhash requires a fresh
/// approval (new gate_ref + grant), enforced by the signing-ledger guard. The
/// RPC URL comes from config (network-allowlisted), never hard-coded.
#[cfg(feature = "broadcast-http")]
pub struct JsonRpcSolanaBroadcaster {
    client: reqwest::Client,
    rpc_url: String,
}

#[cfg(feature = "broadcast-http")]
impl JsonRpcSolanaBroadcaster {
    /// Build a broadcaster against a raw URL string, validated through
    /// [`crate::RpcEndpoint`] (rustls-backed HTTP client).
    pub fn new(rpc_url: impl AsRef<str>) -> Result<Self, ChainSigningError> {
        Self::with_endpoint(crate::RpcEndpoint::parse(rpc_url)?)
    }

    /// Build against a pre-validated [`crate::RpcEndpoint`].
    pub fn with_endpoint(endpoint: crate::RpcEndpoint) -> Result<Self, ChainSigningError> {
        let client = crate::broadcast_http::build_broadcast_client("solana")?;
        Ok(Self {
            client,
            rpc_url: endpoint.as_str().to_string(),
        })
    }

    /// Build over a pre-configured client and pre-validated endpoint (injected
    /// timeouts / proxy / policy).
    pub fn with_client(client: reqwest::Client, endpoint: crate::RpcEndpoint) -> Self {
        Self {
            client,
            rpc_url: endpoint.as_str().to_string(),
        }
    }
}

#[cfg(feature = "broadcast-http")]
#[async_trait]
impl SolanaBroadcaster for JsonRpcSolanaBroadcaster {
    async fn send_transaction(
        &self,
        signed_tx: &[u8],
    ) -> Result<SolanaBroadcastOutcome, ChainSigningError> {
        use base64::Engine as _;

        let broadcast = |reason: String| ChainSigningError::Broadcast {
            chain: "solana",
            reason,
        };
        let encoded = base64::engine::general_purpose::STANDARD.encode(signed_tx);
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": crate::broadcast_http::RPC_REQUEST_ID,
            "method": "sendTransaction",
            // skipPreflight=false keeps the node's sanity checks; encoding must
            // match the payload. `maxRetries: 0` disables the RPC node's default
            // rebroadcast loop so the attested payload is submitted exactly once
            // — re-broadcast beyond the single submission would violate the
            // one-shot/idempotency guarantee the ledger guard relies on.
            "params": [encoded, { "encoding": "base64", "maxRetries": 0 }],
        });
        let response = self
            .client
            .post(&self.rpc_url)
            .json(&request)
            .send()
            .await
            .map_err(|error| broadcast(format!("request failed: {error}")))?;
        let result = crate::broadcast_http::read_jsonrpc_result("solana", response).await?;
        let signature_b58 = result
            .as_str()
            .ok_or_else(|| broadcast("JSON-RPC result was not a string signature".to_string()))?;
        let bytes = bs58::decode(signature_b58)
            .into_vec()
            .map_err(|error| broadcast(format!("invalid base58 signature: {error}")))?;
        let signature: [u8; 64] = bytes
            .try_into()
            .map_err(|_| broadcast("signature was not 64 bytes".to_string()))?;
        Ok(SolanaBroadcastOutcome { signature })
    }
}
