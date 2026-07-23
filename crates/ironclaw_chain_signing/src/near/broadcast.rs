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

/// Live NEAR broadcaster: submits the borsh-serialized signed transaction via
/// the `broadcast_tx_async` JSON-RPC method (base64-encoded parameter) to a
/// configured RPC URL, returning the base58 transaction hash.
///
/// One-shot submitter of an already-signed transaction: it never bumps the
/// access-key nonce or re-signs. A fresh nonce requires a fresh approval (new
/// gate_ref + grant), enforced by the signing-ledger guard. We use the `_async`
/// variant deliberately — it returns the tx hash on submission without the node
/// retrying or resubmitting on our behalf. The RPC URL comes from config
/// (network-allowlisted), never hard-coded.
#[cfg(feature = "broadcast-http")]
pub struct JsonRpcNearBroadcaster {
    client: reqwest::Client,
    rpc_url: String,
}

#[cfg(feature = "broadcast-http")]
impl JsonRpcNearBroadcaster {
    /// Build a broadcaster against a raw URL string, validated through
    /// [`crate::RpcEndpoint`] (rustls-backed HTTP client).
    pub fn new(rpc_url: impl AsRef<str>) -> Result<Self, ChainSigningError> {
        Self::with_endpoint(crate::RpcEndpoint::parse(rpc_url)?)
    }

    /// Build against a pre-validated [`crate::RpcEndpoint`].
    pub fn with_endpoint(endpoint: crate::RpcEndpoint) -> Result<Self, ChainSigningError> {
        let client = crate::broadcast_http::build_broadcast_client("near")?;
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
impl NearBroadcaster for JsonRpcNearBroadcaster {
    async fn broadcast_tx(
        &self,
        signed_tx: &[u8],
    ) -> Result<NearBroadcastOutcome, ChainSigningError> {
        use base64::Engine as _;

        let broadcast = |reason: String| ChainSigningError::Broadcast {
            chain: "near",
            reason,
        };
        let encoded = base64::engine::general_purpose::STANDARD.encode(signed_tx);
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": crate::broadcast_http::RPC_REQUEST_ID,
            "method": "broadcast_tx_async",
            "params": [encoded],
        });
        let response = self
            .client
            .post(&self.rpc_url)
            .json(&request)
            .send()
            .await
            .map_err(|error| broadcast(format!("request failed: {error}")))?;
        let result = crate::broadcast_http::read_jsonrpc_result("near", response).await?;
        // `broadcast_tx_async` returns the tx hash directly as the result string.
        // Validate its SHAPE (base58 decoding to 32 bytes) — a hostile node must
        // not be able to return an arbitrary string we'd accept as a tx hash.
        let result = result
            .as_str()
            .ok_or_else(|| broadcast("JSON-RPC result was not a string tx hash".to_string()))?;
        let tx_hash = crate::broadcast_http::validate_near_tx_hash("near", result)?;
        Ok(NearBroadcastOutcome { tx_hash })
    }
}
