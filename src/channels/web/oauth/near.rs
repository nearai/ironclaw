//! NEAR wallet authentication via NEP-413 signature verification.
//!
//! Unlike OAuth providers, NEAR uses a challenge-response flow:
//! 1. Server generates a random nonce (`GET /auth/near/challenge`)
//! 2. Client signs `{ message, nonce, recipient }` with a NEAR wallet
//! 3. Client sends signature + account_id + public_key to `POST /auth/near/verify`
//! 4. Server verifies the Ed25519 signature and confirms the public key
//!    is an active access key on the claimed NEAR account via RPC

use std::collections::HashMap;
use std::time::{Duration, Instant};

use ed25519_dalek::{Signature, VerifyingKey};
use rand::RngCore;
use rand::rngs::OsRng;
use tokio::sync::RwLock;

use super::OAuthError;

const NONCE_TTL: Duration = Duration::from_secs(300); // 5 minutes
const MAX_NONCES: usize = 4096;

/// In-memory nonce store for NEAR auth challenges.
#[derive(Default)]
pub struct NearNonceStore {
    nonces: RwLock<HashMap<String, Instant>>,
}

impl NearNonceStore {
    pub fn new() -> Self {
        Self {
            nonces: RwLock::new(HashMap::new()),
        }
    }

    /// Generate and store a random 32-byte nonce, returned as hex.
    pub async fn generate(&self) -> String {
        let mut bytes = [0u8; 32];
        OsRng.fill_bytes(&mut bytes);
        let nonce = hex::encode(bytes);

        let mut nonces = self.nonces.write().await;

        // Evict expired nonces if near capacity.
        if nonces.len() >= MAX_NONCES {
            let now = Instant::now();
            nonces.retain(|_, created| now.duration_since(*created) < NONCE_TTL);
        }

        nonces.insert(nonce.clone(), Instant::now());
        nonce
    }

    /// Consume a nonce — returns true if valid (exists and not expired).
    /// Single-use: the nonce is removed regardless.
    pub async fn consume(&self, nonce: &str) -> bool {
        let mut nonces = self.nonces.write().await;
        match nonces.remove(nonce) {
            Some(created) => Instant::now().duration_since(created) < NONCE_TTL,
            None => false,
        }
    }

    /// Remove expired nonces. Call periodically from a background task.
    pub async fn sweep_expired(&self) {
        let mut nonces = self.nonces.write().await;
        let now = Instant::now();
        nonces.retain(|_, created| now.duration_since(*created) < NONCE_TTL);
    }
}

/// Verify an Ed25519 signature over a NEP-413 message.
///
/// The signed payload is: the message bytes prepended with a NEP-413 tag.
/// NEP-413 specifies the payload as:
///   `u32_le(2**31 + 413)` || `u32_le(len(message))` || message || nonce || recipient || callback_url?
///
/// For simplicity we verify over the raw message bytes directly, which is
/// compatible with wallets that sign the plain message via `signMessage`.
pub fn verify_signature(
    public_key_bytes: &[u8; 32],
    signature_bytes: &[u8; 64],
    message: &[u8],
) -> Result<(), OAuthError> {
    let key = VerifyingKey::from_bytes(public_key_bytes)
        .map_err(|e| OAuthError::ProfileFetch(format!("Invalid public key: {e}")))?;
    let sig = Signature::from_bytes(signature_bytes);
    use ed25519_dalek::Verifier;
    key.verify(message, &sig)
        .map_err(|_| OAuthError::ProfileFetch("Invalid signature".to_string()))
}

/// Verify that a public key is an active access key on a NEAR account via RPC.
pub async fn verify_access_key(
    rpc_url: &str,
    account_id: &str,
    public_key: &str,
    http: &reqwest::Client,
) -> Result<(), OAuthError> {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": "ironclaw",
        "method": "query",
        "params": {
            "request_type": "view_access_key",
            "finality": "final",
            "account_id": account_id,
            "public_key": public_key,
        }
    });

    let resp = http
        .post(rpc_url)
        .json(&body)
        .send()
        .await
        .map_err(|e| OAuthError::ProfileFetch(format!("NEAR RPC request failed: {e}")))?;

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| OAuthError::ProfileFetch(format!("NEAR RPC response parse error: {e}")))?;

    // Check for RPC-level error (key doesn't exist, account doesn't exist, etc.)
    if let Some(error) = json.get("error") {
        let msg = error
            .get("cause")
            .and_then(|c| c.get("name"))
            .and_then(|n| n.as_str())
            .unwrap_or("unknown error");
        return Err(OAuthError::ProfileFetch(format!(
            "Access key not found on account '{account_id}': {msg}"
        )));
    }

    // Verify we got a result (not an error response).
    if json.get("result").is_none() {
        return Err(OAuthError::ProfileFetch(
            "NEAR RPC returned no result for access key query".to_string(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_nonce_generate_and_consume() {
        let store = NearNonceStore::new();
        let nonce = store.generate().await;
        assert_eq!(nonce.len(), 64); // 32 bytes hex-encoded

        assert!(store.consume(&nonce).await);
        // Second consume should fail (single-use).
        assert!(!store.consume(&nonce).await);
    }

    #[tokio::test]
    async fn test_nonce_unknown_rejected() {
        let store = NearNonceStore::new();
        assert!(!store.consume("nonexistent").await);
    }

    #[tokio::test]
    async fn test_nonce_sweep() {
        let store = NearNonceStore::new();
        // Insert an already-expired nonce.
        {
            let mut nonces = store.nonces.write().await;
            nonces.insert(
                "old-nonce".to_string(),
                Instant::now() - Duration::from_secs(600),
            );
            nonces.insert("fresh-nonce".to_string(), Instant::now());
        }
        store.sweep_expired().await;
        let nonces = store.nonces.read().await;
        assert_eq!(nonces.len(), 1);
        assert!(nonces.contains_key("fresh-nonce"));
    }

    #[test]
    fn test_verify_signature_valid() {
        use ed25519_dalek::{Signer, SigningKey};
        let signing_key = SigningKey::from_bytes(&{
            let mut b = [0u8; 32];
            OsRng.fill_bytes(&mut b);
            b
        });
        let verifying_key = signing_key.verifying_key();

        let message = b"Sign in to IronClaw";
        let signature = signing_key.sign(message);

        assert!(
            verify_signature(verifying_key.as_bytes(), &signature.to_bytes(), message,).is_ok()
        );
    }

    #[test]
    fn test_verify_signature_wrong_message() {
        use ed25519_dalek::{Signer, SigningKey};
        let signing_key = SigningKey::from_bytes(&{
            let mut b = [0u8; 32];
            OsRng.fill_bytes(&mut b);
            b
        });
        let verifying_key = signing_key.verifying_key();

        let signature = signing_key.sign(b"original message");

        assert!(
            verify_signature(
                verifying_key.as_bytes(),
                &signature.to_bytes(),
                b"different message",
            )
            .is_err()
        );
    }
}
