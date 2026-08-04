//! Nostr event signing for WASM tools.
//!
//! Implements host-side Nostr event signing: the host holds the private key,
//! WASM provides the unsigned event, and the host returns a fully signed event.
//!
//! This follows the NIP-01 event serialization and signing spec:
//! 1. Serialize event fields as `["0", "pubkey", "created_at", "kind", "tags", "content"]`
//! 2. SHA-256 hash the serialized JSON
//! 3. Schnorr sign the hash with the private key
//! 4. Return the complete event with `id` and `sig` fields

use serde_json::{Number, Value};
use sha2::{Digest, Sha256};
use std::str::FromStr;

/// Error type for Nostr signing operations.
#[derive(Debug, thiserror::Error)]
pub enum NostrSignError {
    #[error("Invalid unsigned event: {0}")]
    InvalidEvent(String),

    #[error("Invalid private key: {0}")]
    InvalidKey(String),

    #[error("Signing failed: {0}")]
    SigningFailed(String),
}

/// Sign an unsigned Nostr event with a private key.
///
/// The unsigned event JSON must contain: `kind`, `content`, `tags`, `created_at`, `pubkey`.
/// The `pubkey` in the event must match the public key derived from the private key.
///
/// Returns the complete signed event JSON string with `id` and `sig` added.
pub fn sign_nostr_event(unsigned_json: &str, private_key_hex: &str) -> Result<String, NostrSignError> {
    // Parse the unsigned event
    let mut event: Value =
        serde_json::from_str(unsigned_json).map_err(|e| NostrSignError::InvalidEvent(e.to_string()))?;

    // Validate required fields
    let kind = event
        .get("kind")
        .ok_or_else(|| NostrSignError::InvalidEvent("missing 'kind' field".into()))?
        .as_u64()
        .ok_or_else(|| NostrSignError::InvalidEvent("'kind' must be a number".into()))?;

    let content = event
        .get("content")
        .ok_or_else(|| NostrSignError::InvalidEvent("missing 'content' field".into()))?;

    let tags = event
        .get("tags")
        .ok_or_else(|| NostrSignError::InvalidEvent("missing 'tags' field".into()))?;

    let created_at = event
        .get("created_at")
        .ok_or_else(|| NostrSignError::InvalidEvent("missing 'created_at' field".into()))?
        .as_u64()
        .ok_or_else(|| NostrSignError::InvalidEvent("'created_at' must be a number".into()))?;

    let event_pubkey = event
        .get("pubkey")
        .ok_or_else(|| NostrSignError::InvalidEvent("missing 'pubkey' field".into()))?
        .as_str()
        .ok_or_else(|| NostrSignError::InvalidEvent("'pubkey' must be a string".into()))?;

    // Parse the private key
    let secret_key = secp256k1::SecretKey::from_str(private_key_hex)
        .map_err(|e| NostrSignError::InvalidKey(format!("invalid hex private key: {e}")))?;

    let secp = secp256k1::Secp256k1::new();
    let key_pair = secp256k1::Keypair::from_secret_key(&secp, &secret_key);

    // Verify the event's pubkey matches the keypair
    let (x_only_pubkey, _parity) = key_pair.x_only_public_key();
    let expected_hex = hex::encode(x_only_pubkey.serialize());

    if event_pubkey != expected_hex {
        return Err(NostrSignError::InvalidEvent(format!(
            "event pubkey '{}' does not match keypair pubkey '{}'",
            event_pubkey, expected_hex
        )));
    }

    // NIP-01 canonical serialization: [0, pubkey, created_at, kind, tags, content]
    let serialized = serde_json::to_string(&Value::Array(vec![
        Value::Number(Number::from(0u64)),
        Value::String(event_pubkey.to_string()),
        Value::Number(Number::from(created_at)),
        Value::Number(Number::from(kind)),
        tags.clone(),
        content.clone(),
    ]))
    .map_err(|e| NostrSignError::InvalidEvent(format!("serialization failed: {e}")))?;

    // Compute event ID: sha256 of serialized event
    let mut hasher = Sha256::new();
    hasher.update(serialized.as_bytes());
    let event_id = hasher.finalize();
    let event_id_hex = hex::encode(event_id);

    // Create message from event ID
    let msg = secp256k1::Message::from_digest_slice(&event_id)
        .map_err(|e| NostrSignError::SigningFailed(format!("invalid message: {e}")))?;

    // Schnorr sign
    let aux_rand_bytes = secp256k1::SecretKey::new(&mut secp256k1::rand::thread_rng()).secret_bytes();
    let signature = secp.sign_schnorr_with_aux_rand(&msg, &key_pair, &aux_rand_bytes);

    // Add id and sig to the event
    if let Some(obj) = event.as_object_mut() {
        obj.insert("id".to_string(), Value::String(event_id_hex));
        obj.insert("sig".to_string(), Value::String(signature.to_string()));
    }

    serde_json::to_string(&event)
        .map_err(|e| NostrSignError::SigningFailed(format!("failed to serialize signed event: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Known test vector: deterministic key for testing.
    /// Private key: 0000000000000000000000000000000000000000000000000000000000000001
    /// Public key (x-only): 79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798
    #[test]
    fn test_sign_simple_event() {
        let privkey = "0000000000000000000000000000000000000000000000000000000000000001";
        let pubkey = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";

        let unsigned = r#"{
            "pubkey": "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
            "created_at": 1690000000,
            "kind": 1,
            "tags": [],
            "content": "hello from nostr"
        }"#;

        let result = sign_nostr_event(unsigned, privkey).expect("signing failed");
        let event: Value = serde_json::from_str(&result).unwrap();

        // Must have id and sig
        assert!(event.get("id").unwrap().is_string());
        let sig = event.get("sig").unwrap().as_str().unwrap();
        // Schnorr sig is 128 hex chars (64 bytes)
        assert_eq!(sig.len(), 128);

        // id is sha256 hex (64 chars)
        let id = event.get("id").unwrap().as_str().unwrap();
        assert_eq!(id.len(), 64);
    }

    #[test]
    fn test_reject_wrong_pubkey() {
        let privkey = "0000000000000000000000000000000000000000000000000000000000000001";

        let unsigned = r#"{
            "pubkey": "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "created_at": 1690000000,
            "kind": 1,
            "tags": [],
            "content": "hello"
        }"#;

        let result = sign_nostr_event(unsigned, privkey);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("does not match keypair pubkey"), "err: {err}");
    }

    #[test]
    fn test_reject_missing_fields() {
        let privkey = "0000000000000000000000000000000000000000000000000000000000000001";

        let unsigned = r#"{"kind": 1}"#;

        let result = sign_nostr_event(unsigned, privkey);
        assert!(result.is_err());
    }

    #[test]
    fn test_reject_invalid_private_key() {
        let privkey = "not_hex";

        let unsigned = r#"{
            "pubkey": "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "created_at": 1690000000,
            "kind": 1,
            "tags": [],
            "content": "hello"
        }"#;

        let result = sign_nostr_event(unsigned, privkey);
        assert!(result.is_err());
    }
}
