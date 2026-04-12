//! Helpers for `did:key` instance identities.

use ed25519_dalek::SigningKey;
use rand::RngCore;
use rand::rngs::OsRng;

/// `did:key` method prefix.
pub const DID_KEY_METHOD: &str = "did:key";

/// Multicodec varint prefix for `ed25519-pub`.
const ED25519_PUB_MULTICODEC_PREFIX: [u8; 2] = [0xed, 0x01];

/// Generate a fresh Ed25519 secret key.
pub fn generate_secret_key() -> [u8; 32] {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    bytes
}

fn verifying_key_bytes(secret_key: &[u8; 32]) -> [u8; 32] {
    SigningKey::from_bytes(secret_key)
        .verifying_key()
        .to_bytes()
}

/// Encode an Ed25519 public key as a multibase string suitable for `did:key`.
pub fn public_key_multibase(secret_key: &[u8; 32]) -> String {
    let public_key = verifying_key_bytes(secret_key);
    let mut payload = Vec::with_capacity(ED25519_PUB_MULTICODEC_PREFIX.len() + public_key.len());
    payload.extend_from_slice(&ED25519_PUB_MULTICODEC_PREFIX);
    payload.extend_from_slice(&public_key);
    format!("z{}", bs58::encode(payload).into_string())
}

/// Build the DID string from the public key multibase value.
pub fn did_from_public_key_multibase(public_key_multibase: &str) -> String {
    format!("{DID_KEY_METHOD}:{public_key_multibase}")
}

/// Build the verification method ID for the current key.
pub fn key_id(did: &str, public_key_multibase: &str) -> String {
    format!("{did}#{public_key_multibase}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_key_multibase_uses_base58btc_prefix() {
        let secret = [7u8; 32];
        let multibase = public_key_multibase(&secret);
        assert!(multibase.starts_with('z'));
        assert!(multibase.len() > 10);
    }

    #[test]
    fn did_and_key_id_are_stable_for_same_key() {
        let secret = [42u8; 32];
        let multibase = public_key_multibase(&secret);
        let did = did_from_public_key_multibase(&multibase);
        let key_id = key_id(&did, &multibase);

        assert_eq!(did, format!("did:key:{multibase}"));
        assert_eq!(key_id, format!("{did}#{multibase}"));
    }
}
