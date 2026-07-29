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

/// Live EVM broadcaster: submits the signed RLP via `eth_sendRawTransaction`
/// over JSON-RPC to a configured endpoint.
///
/// It is a one-shot submitter of an *already-signed* payload: it never bumps
/// the nonce, re-signs, or refreshes any field. A stuck transaction must be
/// re-approved (new gate_ref + grant), which the [`crate::SigningLedger`]
/// broadcast-idempotency guard enforces upstream. The RPC URL is supplied by
/// the composition layer from config (subject to the network allowlist), never
/// hard-coded.
#[cfg(feature = "broadcast-http")]
pub struct JsonRpcEvmBroadcaster {
    client: reqwest::Client,
    rpc_url: String,
}

#[cfg(feature = "broadcast-http")]
impl JsonRpcEvmBroadcaster {
    /// Build a broadcaster against a raw URL string, validating it through
    /// [`crate::RpcEndpoint`] (rejects metadata/loopback/private/link-local
    /// hosts and non-http schemes). The HTTP client is rustls-backed.
    pub fn new(rpc_url: impl AsRef<str>) -> Result<Self, ChainSigningError> {
        Self::with_endpoint(crate::RpcEndpoint::parse(rpc_url)?)
    }

    /// Build against a pre-validated [`crate::RpcEndpoint`].
    pub fn with_endpoint(endpoint: crate::RpcEndpoint) -> Result<Self, ChainSigningError> {
        let client = crate::broadcast_http::build_broadcast_client("evm")?;
        Ok(Self {
            client,
            rpc_url: endpoint.as_str().to_string(),
        })
    }

    /// Build over a pre-configured client and pre-validated endpoint (so callers
    /// can inject timeouts / proxy / allowlist policy).
    pub fn with_client(client: reqwest::Client, endpoint: crate::RpcEndpoint) -> Self {
        Self {
            client,
            rpc_url: endpoint.as_str().to_string(),
        }
    }
}

#[cfg(feature = "broadcast-http")]
#[async_trait]
impl EvmBroadcaster for JsonRpcEvmBroadcaster {
    async fn send_raw(&self, signed_rlp: &[u8]) -> Result<EvmBroadcastOutcome, ChainSigningError> {
        let broadcast = |reason: String| ChainSigningError::Broadcast {
            chain: "evm",
            reason,
        };
        let raw_hex = format!("0x{}", crate::broadcast_http::hex_encode(signed_rlp));
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": crate::broadcast_http::RPC_REQUEST_ID,
            "method": "eth_sendRawTransaction",
            "params": [raw_hex],
        });
        let response = self
            .client
            .post(&self.rpc_url)
            .json(&request)
            .send()
            .await
            .map_err(|error| broadcast(format!("request failed: {error}")))?;
        let result = crate::broadcast_http::read_jsonrpc_result("evm", response).await?;
        let result = result
            .as_str()
            .ok_or_else(|| broadcast("JSON-RPC result was not a string tx hash".to_string()))?;
        let bytes = crate::broadcast_http::decode_hex(result.trim_start_matches("0x"))
            .map_err(|reason| broadcast(format!("invalid tx hash in response: {reason}")))?;
        let tx_hash: [u8; 32] = bytes
            .try_into()
            .map_err(|_| broadcast("tx hash was not 32 bytes".to_string()))?;
        // An EVM tx hash is deterministically keccak256(signed_rlp) and is fully
        // verifiable from the bytes in hand. A malicious, buggy, or
        // proxy/load-balanced node can return a hash that does not correspond to
        // the submitted payload; recording it would corrupt finalization
        // tracking and user-facing tx links. Reject the response and record only
        // the locally-derived hash as canonical.
        let expected = alloy_primitives::keccak256(signed_rlp);
        if tx_hash != expected.0 {
            return Err(broadcast(
                "node-returned tx hash does not match keccak256(signed_rlp)".to_string(),
            ));
        }
        Ok(EvmBroadcastOutcome { tx_hash })
    }
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

    /// A one-shot mock JSON-RPC node that returns the `result` hex string it is
    /// configured with, so we can exercise the live `send_raw` hash check
    /// against both an honest and a lying node. Uses `std::net` only (no extra
    /// dev-dep) and serves exactly one request before shutting down.
    #[cfg(feature = "broadcast-http")]
    fn spawn_mock_node(result_hex: String) -> (String, std::thread::JoinHandle<()>) {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock node");
        let addr = listener.local_addr().expect("local addr");
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buf = [0u8; 4096];
            // Drain the request; we don't assert on it here (the
            // RecordingBroadcaster test already proves exact-bytes submission).
            let _ = stream.read(&mut buf);
            let body = format!(r#"{{"jsonrpc":"2.0","id":1,"result":"{result_hex}"}}"#);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        });
        (format!("http://127.0.0.1:{}", addr.port()), handle)
    }

    #[cfg(feature = "broadcast-http")]
    #[tokio::test(flavor = "multi_thread")]
    async fn send_raw_rejects_node_hash_mismatch() {
        let signed_rlp = [0x11u8, 0x22, 0x33, 0x44];
        // A hash that is NOT keccak256(signed_rlp): the hostile/buggy node lies.
        let wrong_hash = format!("0x{}", crate::broadcast_http::hex_encode(&[0xaau8; 32]));
        let (url, handle) = spawn_mock_node(wrong_hash);

        let endpoint = crate::RpcEndpoint::parse_with_allowlist(&url, &["127.0.0.1"])
            .expect("loopback allowlisted endpoint");
        let client = crate::broadcast_http::build_broadcast_client("evm").expect("client");
        let broadcaster = JsonRpcEvmBroadcaster::with_client(client, endpoint);

        let err = broadcaster
            .send_raw(&signed_rlp)
            .await
            .expect_err("mismatched node hash must be rejected");
        assert!(
            format!("{err}").contains("does not match keccak256(signed_rlp)"),
            "{err}"
        );
        handle.join().expect("mock node thread");
    }

    #[cfg(feature = "broadcast-http")]
    #[tokio::test(flavor = "multi_thread")]
    async fn send_raw_accepts_node_hash_matching_keccak256() {
        let signed_rlp = [0x11u8, 0x22, 0x33, 0x44];
        let expected = alloy_primitives::keccak256(signed_rlp);
        let honest_hash = format!("0x{}", crate::broadcast_http::hex_encode(&expected.0));
        let (url, handle) = spawn_mock_node(honest_hash);

        let endpoint = crate::RpcEndpoint::parse_with_allowlist(&url, &["127.0.0.1"])
            .expect("loopback allowlisted endpoint");
        let client = crate::broadcast_http::build_broadcast_client("evm").expect("client");
        let broadcaster = JsonRpcEvmBroadcaster::with_client(client, endpoint);

        let out = broadcaster
            .send_raw(&signed_rlp)
            .await
            .expect("honest node hash accepted");
        // The canonical recorded hash is the locally-derived keccak256.
        assert_eq!(out.tx_hash, expected.0);
        handle.join().expect("mock node thread");
    }
}
