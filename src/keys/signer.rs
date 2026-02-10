//! Ed25519 signing for NEAR transactions.
//!
//! SECURITY: Private keys are held in memory for the absolute minimum time.
//! The flow is: decrypt -> construct SigningKey -> sign -> drop (Zeroize).
//! The `ed25519_dalek::SigningKey` implements Zeroize, so memory is zeroed on drop.

use ed25519_dalek::Signer;
use sha2::{Digest, Sha256};
use zeroize::Zeroize;

use crate::keys::KeyError;
use crate::keys::types::NearPublicKey;
use crate::secrets::SecretsStore;

/// Parse a NEAR-format secret key and extract the 32-byte ed25519 seed.
///
/// NEAR secret keys are formatted as `ed25519:<base58-encoded-64-bytes>`.
/// The 64 bytes are the seed (32) + public key (32) concatenated.
/// Some wallets store only the 32-byte seed with the same prefix.
fn parse_near_secret_key(near_format: &str) -> Result<[u8; 32], KeyError> {
    let data_str =
        near_format
            .strip_prefix("ed25519:")
            .ok_or_else(|| KeyError::InvalidKeyFormat {
                reason: "secret key must start with 'ed25519:'".to_string(),
            })?;

    let mut bytes = bs58::decode(data_str)
        .into_vec()
        .map_err(|e| KeyError::InvalidKeyFormat {
            reason: format!("invalid base58 in secret key: {}", e),
        })?;

    let seed = match bytes.len() {
        64 => {
            // Standard NEAR format: seed (32) + public key (32)
            let mut seed = [0u8; 32];
            seed.copy_from_slice(&bytes[..32]);
            bytes.zeroize();
            seed
        }
        32 => {
            // Some wallets export just the seed
            let mut seed = [0u8; 32];
            seed.copy_from_slice(&bytes);
            bytes.zeroize();
            seed
        }
        other => {
            bytes.zeroize();
            return Err(KeyError::InvalidKeyFormat {
                reason: format!("ed25519 secret key must be 32 or 64 bytes, got {}", other),
            });
        }
    };

    Ok(seed)
}

/// Derive the public key from a NEAR-format secret key string.
pub fn public_key_from_secret(near_format_secret: &str) -> Result<NearPublicKey, KeyError> {
    let seed = parse_near_secret_key(near_format_secret)?;
    let signing_key = ed25519_dalek::SigningKey::from_bytes(&seed);
    let verifying_key = signing_key.verifying_key();
    // signing_key implements Zeroize on drop

    Ok(NearPublicKey {
        key_type: crate::keys::types::KeyType::Ed25519,
        data: verifying_key.to_bytes(),
    })
}

/// Sign a 32-byte SHA-256 hash using a key from the secrets store.
///
/// This is the core signing function. It:
/// 1. Decrypts the private key from the secrets store
/// 2. Parses the NEAR-format key to extract the ed25519 seed
/// 3. Constructs a SigningKey (implements Zeroize on drop)
/// 4. Signs the hash
/// 5. Drops the SigningKey (memory zeroed)
///
/// The plaintext key exists in memory for microseconds.
pub async fn sign_hash(
    secrets_store: &dyn SecretsStore,
    user_id: &str,
    label: &str,
    hash: &[u8; 32],
) -> Result<[u8; 64], KeyError> {
    let secret_name = format!("near_key:{}", label);
    let decrypted = secrets_store
        .get_decrypted(user_id, &secret_name)
        .await
        .map_err(|e| KeyError::SigningFailed {
            reason: format!("failed to decrypt key '{}': {}", label, e),
        })?;

    let mut seed = parse_near_secret_key(decrypted.expose())?;
    let signing_key = ed25519_dalek::SigningKey::from_bytes(&seed);
    seed.zeroize();

    let signature = signing_key.sign(hash);
    // signing_key drops here, Zeroize zeroes the key material

    Ok(signature.to_bytes())
}

/// SHA-256 hash of data (used for transaction signing).
pub fn sha256_hash(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
    use secrecy::SecretString;

    use crate::keys::signer::{
        parse_near_secret_key, public_key_from_secret, sha256_hash, sign_hash,
    };
    use crate::keys::types::KeyType;
    use crate::secrets::{CreateSecretParams, InMemorySecretsStore, SecretsCrypto, SecretsStore};

    fn test_store() -> Arc<InMemorySecretsStore> {
        let key = "0123456789abcdef0123456789abcdef";
        let crypto = Arc::new(SecretsCrypto::new(SecretString::from(key.to_string())).unwrap());
        Arc::new(InMemorySecretsStore::new(crypto))
    }

    /// Generate a test keypair and return (near_format_secret, near_format_public).
    fn generate_test_keypair() -> (String, String) {
        let signing_key = SigningKey::generate(&mut rand::rngs::OsRng);
        let verifying_key = signing_key.verifying_key();

        // NEAR format: ed25519:<base58(seed + pubkey)>
        let mut combined = Vec::with_capacity(64);
        combined.extend_from_slice(signing_key.as_bytes());
        combined.extend_from_slice(verifying_key.as_bytes());

        let secret = format!("ed25519:{}", bs58::encode(&combined).into_string());
        let public = format!(
            "ed25519:{}",
            bs58::encode(verifying_key.as_bytes()).into_string()
        );

        (secret, public)
    }

    #[test]
    fn test_parse_near_secret_key_64_bytes() {
        let (secret, _) = generate_test_keypair();
        let seed = parse_near_secret_key(&secret).unwrap();
        assert_eq!(seed.len(), 32);
    }

    #[test]
    fn test_parse_near_secret_key_32_bytes() {
        // Some wallets export just the 32-byte seed
        let signing_key = SigningKey::generate(&mut rand::rngs::OsRng);
        let secret = format!(
            "ed25519:{}",
            bs58::encode(signing_key.as_bytes()).into_string()
        );
        let seed = parse_near_secret_key(&secret).unwrap();
        assert_eq!(seed, *signing_key.as_bytes());
    }

    #[test]
    fn test_parse_invalid_prefix() {
        assert!(parse_near_secret_key("secp256k1:abc").is_err());
    }

    #[test]
    fn test_public_key_from_secret() {
        let (secret, expected_public) = generate_test_keypair();
        let pubkey = public_key_from_secret(&secret).unwrap();
        assert_eq!(pubkey.key_type, KeyType::Ed25519);
        assert_eq!(pubkey.to_near_format(), expected_public);
    }

    #[test]
    fn test_sign_and_verify_roundtrip() {
        let signing_key = SigningKey::generate(&mut rand::rngs::OsRng);
        let verifying_key = signing_key.verifying_key();

        let message = b"test message for signing";
        let hash = sha256_hash(message);

        let signature = signing_key.sign(&hash);

        // Verify
        assert!(verifying_key.verify(&hash, &signature).is_ok());
    }

    #[tokio::test]
    async fn test_sign_hash_from_store() {
        let store = test_store();
        let (secret, _public) = generate_test_keypair();

        // Store the key
        store
            .create(
                "user1",
                CreateSecretParams::new("near_key:test-signer", &secret).with_provider("near_keys"),
            )
            .await
            .unwrap();

        // Sign
        let hash = sha256_hash(b"test transaction data");
        let sig_bytes = sign_hash(store.as_ref(), "user1", "test-signer", &hash)
            .await
            .unwrap();

        // Verify using the public key derived from the secret
        let pubkey = public_key_from_secret(&secret).unwrap();
        let verifying_key = VerifyingKey::from_bytes(pubkey.as_bytes()).unwrap();
        let signature = ed25519_dalek::Signature::from_bytes(&sig_bytes);
        assert!(verifying_key.verify(&hash, &signature).is_ok());
    }

    #[tokio::test]
    async fn test_sign_hash_key_not_found() {
        let store = test_store();
        let hash = [0u8; 32];
        let result = sign_hash(store.as_ref(), "user1", "nonexistent", &hash).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_sha256_hash() {
        let hash = sha256_hash(b"hello");
        let hex: String = hash.iter().map(|b| format!("{:02x}", b)).collect();
        // Known SHA-256 of "hello"
        assert_eq!(
            hex,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }
}
