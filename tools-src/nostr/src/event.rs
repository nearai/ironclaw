//! Nostr event construction, ID computation, and Schnorr signing (NIP-01).
//!
//! Event ID = SHA-256 of the serialized event array:
//!   [0, pubkey_hex, created_at, kind, tags, content]
//! Signature = Schnorr (BIP-340) over the event ID using the private key.
//!
//! Uses `k256` (pure Rust) -- no C dependencies, compiles cleanly to WASM.

use k256::schnorr::{signature::Signer, SigningKey};
use k256::sha2::{Digest, Sha256};

/// A Nostr event (NIP-01).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Event {
    pub id: String,
    pub pubkey: String,
    pub created_at: u64,
    pub kind: u64,
    #[serde(default)]
    pub tags: Vec<Vec<String>>,
    pub content: String,
    pub sig: String,
}

/// Compute the event ID (SHA-256 of the serialized event).
/// The serialization is: JSON array [0, pubkey, created_at, kind, tags, content]
pub fn compute_event_id(
    pubkey_hex: &str,
    created_at: u64,
    kind: u64,
    tags: &[Vec<String>],
    content: &str,
) -> String {
    let serialized = serde_json::json!([
        0,
        pubkey_hex,
        created_at,
        kind,
        tags,
        content
    ]);
    let serialized_str = serde_json::to_string(&serialized).unwrap_or_default();

    let mut hasher = Sha256::new();
    hasher.update(serialized_str.as_bytes());
    let hash = hasher.finalize();
    hex::encode(hash)
}

/// Sign an event ID with a Schnorr (BIP-340) signature.
/// Returns the signature as a hex string (64 bytes = 128 hex chars).
pub fn sign_event_id(event_id_hex: &str, secret_key_bytes: &[u8; 32]) -> Result<String, String> {
    let signing_key = SigningKey::from_bytes(secret_key_bytes)
        .map_err(|e| format!("invalid secret key: {e}"))?;

    let event_id_bytes =
        hex::decode(event_id_hex).map_err(|e| format!("invalid event id hex: {e}"))?;

    let mut msg_bytes = [0u8; 32];
    msg_bytes.copy_from_slice(&event_id_bytes);

    let signature: k256::schnorr::Signature = signing_key.sign(&msg_bytes);

    Ok(hex::encode(signature.to_bytes()))
}

/// Derive the x-only public key from a secret key (32 bytes hex).
pub fn derive_pubkey(secret_key_bytes: &[u8; 32]) -> Result<String, String> {
    let signing_key = SigningKey::from_bytes(secret_key_bytes)
        .map_err(|e| format!("invalid secret key: {e}"))?;

    let verifying_key = signing_key.verifying_key();
    let pubkey_bytes = verifying_key.to_bytes();

    Ok(hex::encode(pubkey_bytes))
}

/// Build a complete signed event.
pub fn build_signed_event(
    secret_key_bytes: &[u8; 32],
    kind: u64,
    tags: Vec<Vec<String>>,
    content: String,
    created_at: u64,
) -> Result<Event, String> {
    let pubkey_hex = derive_pubkey(secret_key_bytes)?;

    let id = compute_event_id(&pubkey_hex, created_at, kind, &tags, &content);
    let sig = sign_event_id(&id, secret_key_bytes)?;

    Ok(Event {
        id,
        pubkey: pubkey_hex,
        created_at,
        kind,
        tags,
        content,
        sig,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_event_id_deterministic() {
        let id1 = compute_event_id(
            "0000000000000000000000000000000000000000000000000000000000000001",
            1700000000,
            1,
            &[],
            "hello nostr",
        );
        let id2 = compute_event_id(
            "0000000000000000000000000000000000000000000000000000000000000001",
            1700000000,
            1,
            &[],
            "hello nostr",
        );
        assert_eq!(id1, id2);
        assert_eq!(id1.len(), 64);
    }

    #[test]
    fn test_derive_pubkey() {
        let sk = [1u8; 32];
        let pk = derive_pubkey(&sk).unwrap();
        assert_eq!(pk.len(), 64);
    }

    #[test]
    fn test_build_signed_event() {
        let sk = [0x42u8; 32];
        let event = build_signed_event(
            &sk,
            1,
            vec![vec!["p".into(), "00".repeat(32).into()]],
            "Hello Nostr!".into(),
            1700000000,
        )
        .unwrap();

        assert_eq!(event.kind, 1);
        assert_eq!(event.content, "Hello Nostr!");
        assert_eq!(event.id.len(), 64);
        assert_eq!(event.sig.len(), 128);
        assert_eq!(event.pubkey.len(), 64);
    }
}
