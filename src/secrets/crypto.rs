//! Cryptographic operations for secret storage.
//!
//! Uses AES-256-GCM for authenticated encryption with per-secret key derivation.
//!
//! # Key Derivation
//!
//! ```text
//! master_key (from env) ─┬─► HKDF-SHA256 ─► derived_key (per secret)
//!                        │
//! per-secret salt ───────┘
//! ```
//!
//! Each secret has its own randomly-generated salt, so even if two secrets
//! have the same plaintext, they'll have different ciphertexts.

use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, AeadCore, OsRng},
};
use hkdf::Hkdf;
use secrecy::{ExposeSecret, SecretString};
use sha2::Sha256;

use crate::secrets::types::{DecryptedSecret, SecretError};

/// Size of the AES-256 key in bytes.
const KEY_SIZE: usize = 32;

/// Size of the GCM nonce in bytes.
const NONCE_SIZE: usize = 12;

/// Size of the per-secret salt for key derivation.
const SALT_SIZE: usize = 32;

/// Size of the GCM authentication tag.
const TAG_SIZE: usize = 16;

/// Cryptographic operations for secrets.
///
/// Holds the master key and provides encrypt/decrypt operations.
/// The master key is kept in secure memory and zeroed on drop.
pub struct SecretsCrypto {
    master_key: SecretString,
}

impl SecretsCrypto {
    /// Create a new crypto instance from a master key.
    ///
    /// The master key should be at least 32 bytes of high-entropy data,
    /// typically loaded from an environment variable or secure vault.
    pub fn new(master_key: SecretString) -> Result<Self, SecretError> {
        // Validate master key length
        if master_key.expose_secret().len() < KEY_SIZE {
            return Err(SecretError::InvalidMasterKey);
        }
        Ok(Self { master_key })
    }

    /// Generate a random salt for a new secret.
    pub fn generate_salt() -> Vec<u8> {
        let mut salt = vec![0u8; SALT_SIZE];
        rand::RngCore::fill_bytes(&mut OsRng, &mut salt);
        salt
    }

    /// Encrypt a secret value.
    ///
    /// Returns (encrypted_value, salt) where:
    /// - encrypted_value = nonce || ciphertext || tag
    /// - salt = random bytes used for key derivation
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<(Vec<u8>, Vec<u8>), SecretError> {
        let salt = Self::generate_salt();
        let derived_key = self.derive_key(&salt)?;

        let cipher = Aes256Gcm::new_from_slice(&derived_key).map_err(|e| {
            SecretError::EncryptionFailed(format!("Failed to create cipher: {}", e))
        })?;

        // Generate random nonce
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);

        // Encrypt
        let ciphertext = cipher
            .encrypt(&nonce, plaintext)
            .map_err(|e| SecretError::EncryptionFailed(format!("Encryption failed: {}", e)))?;

        // Combine: nonce || ciphertext (which includes tag)
        let mut encrypted = Vec::with_capacity(NONCE_SIZE + ciphertext.len());
        encrypted.extend_from_slice(&nonce);
        encrypted.extend_from_slice(&ciphertext);

        Ok((encrypted, salt))
    }

    /// Decrypt a secret value.
    ///
    /// Takes the encrypted_value (nonce || ciphertext || tag) and the salt
    /// that was used during encryption.
    pub fn decrypt(
        &self,
        encrypted_value: &[u8],
        salt: &[u8],
    ) -> Result<DecryptedSecret, SecretError> {
        if encrypted_value.len() < NONCE_SIZE + TAG_SIZE {
            return Err(SecretError::DecryptionFailed(
                "Encrypted value too short".to_string(),
            ));
        }

        let derived_key = self.derive_key(salt)?;

        let cipher = Aes256Gcm::new_from_slice(&derived_key).map_err(|e| {
            SecretError::DecryptionFailed(format!("Failed to create cipher: {}", e))
        })?;

        // Split: nonce || ciphertext
        let (nonce_bytes, ciphertext) = encrypted_value.split_at(NONCE_SIZE);
        let nonce = Nonce::from_slice(nonce_bytes);

        // Decrypt
        let plaintext = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| SecretError::DecryptionFailed(format!("Decryption failed: {}", e)))?;

        DecryptedSecret::from_bytes(plaintext)
    }

    /// Encrypt with domain-specific HKDF info for key domain separation.
    ///
    /// Unlike `encrypt()` which uses a fixed info string, this allows callers
    /// to bind the derived key to a specific context (e.g. user_id, agent_id)
    /// so that identical salts for different contexts produce different keys.
    pub fn encrypt_with_info(
        &self,
        plaintext: &[u8],
        info: &[u8],
    ) -> Result<(Vec<u8>, Vec<u8>), SecretError> {
        let salt = Self::generate_salt();
        let derived_key = self.derive_key_with_info(&salt, info)?;

        let cipher = Aes256Gcm::new_from_slice(&derived_key).map_err(|e| {
            SecretError::EncryptionFailed(format!("Failed to create cipher: {}", e))
        })?;

        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);

        let ciphertext = cipher
            .encrypt(&nonce, plaintext)
            .map_err(|e| SecretError::EncryptionFailed(format!("Encryption failed: {}", e)))?;

        let mut encrypted = Vec::with_capacity(NONCE_SIZE + ciphertext.len());
        encrypted.extend_from_slice(&nonce);
        encrypted.extend_from_slice(&ciphertext);

        Ok((encrypted, salt))
    }

    /// Decrypt with domain-specific HKDF info (must match the info used during encryption).
    pub fn decrypt_with_info(
        &self,
        encrypted_value: &[u8],
        salt: &[u8],
        info: &[u8],
    ) -> Result<DecryptedSecret, SecretError> {
        if encrypted_value.len() < NONCE_SIZE + TAG_SIZE {
            return Err(SecretError::DecryptionFailed(
                "Encrypted value too short".to_string(),
            ));
        }

        let derived_key = self.derive_key_with_info(salt, info)?;

        let cipher = Aes256Gcm::new_from_slice(&derived_key).map_err(|e| {
            SecretError::DecryptionFailed(format!("Failed to create cipher: {}", e))
        })?;

        let (nonce_bytes, ciphertext) = encrypted_value.split_at(NONCE_SIZE);
        let nonce = Nonce::from_slice(nonce_bytes);

        let plaintext = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| SecretError::DecryptionFailed(format!("Decryption failed: {}", e)))?;

        DecryptedSecret::from_bytes(plaintext)
    }

    /// Derive a per-secret key using HKDF-SHA256.
    fn derive_key(&self, salt: &[u8]) -> Result<[u8; KEY_SIZE], SecretError> {
        self.derive_key_with_info(salt, b"near-agent-secrets-v1")
    }

    /// Derive a key with caller-specified HKDF info for domain separation.
    fn derive_key_with_info(
        &self,
        salt: &[u8],
        info: &[u8],
    ) -> Result<[u8; KEY_SIZE], SecretError> {
        let master_bytes = self.master_key.expose_secret().as_bytes();

        let hk = Hkdf::<Sha256>::new(Some(salt), master_bytes);

        let mut derived = [0u8; KEY_SIZE];
        hk.expand(info, &mut derived)
            .map_err(|_| SecretError::EncryptionFailed("HKDF expansion failed".to_string()))?;

        Ok(derived)
    }
}

impl std::fmt::Debug for SecretsCrypto {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretsCrypto")
            .field("master_key", &"[REDACTED]")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use secrecy::SecretString;

    use crate::secrets::crypto::SecretsCrypto;
    use crate::testing::credentials::TEST_CRYPTO_KEY;

    fn test_crypto() -> SecretsCrypto {
        // 32-byte test key
        SecretsCrypto::new(SecretString::from(TEST_CRYPTO_KEY.to_string())).unwrap()
    }

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let crypto = test_crypto();
        let plaintext = b"my_super_secret_api_key_12345";

        let (encrypted, salt) = crypto.encrypt(plaintext).unwrap();

        // Encrypted should be larger than plaintext (nonce + tag)
        assert!(encrypted.len() > plaintext.len());

        let decrypted = crypto.decrypt(&encrypted, &salt).unwrap();
        assert_eq!(decrypted.expose().as_bytes(), plaintext);
    }

    #[test]
    fn test_different_salts_different_ciphertext() {
        let crypto = test_crypto();
        let plaintext = b"same_secret";

        let (encrypted1, salt1) = crypto.encrypt(plaintext).unwrap();
        let (encrypted2, salt2) = crypto.encrypt(plaintext).unwrap();

        // Same plaintext, different salts = different ciphertext
        assert_ne!(salt1, salt2);
        assert_ne!(encrypted1, encrypted2);

        // But both decrypt to the same value
        let decrypted1 = crypto.decrypt(&encrypted1, &salt1).unwrap();
        let decrypted2 = crypto.decrypt(&encrypted2, &salt2).unwrap();
        assert_eq!(decrypted1.expose(), decrypted2.expose());
    }

    #[test]
    fn test_wrong_salt_fails() {
        let crypto = test_crypto();
        let plaintext = b"secret";

        let (encrypted, _salt) = crypto.encrypt(plaintext).unwrap();
        let wrong_salt = SecretsCrypto::generate_salt();

        let result = crypto.decrypt(&encrypted, &wrong_salt);
        assert!(result.is_err());
    }

    #[test]
    fn test_tampered_ciphertext_fails() {
        let crypto = test_crypto();
        let plaintext = b"secret";

        let (mut encrypted, salt) = crypto.encrypt(plaintext).unwrap();

        // Tamper with the ciphertext
        if let Some(byte) = encrypted.last_mut() {
            *byte ^= 0xFF;
        }

        let result = crypto.decrypt(&encrypted, &salt);
        assert!(result.is_err());
    }

    #[test]
    fn test_master_key_too_short() {
        let short_key = "tooshort";
        let result = SecretsCrypto::new(SecretString::from(short_key.to_string()));
        assert!(result.is_err());
    }

    #[test]
    fn test_empty_plaintext() {
        let crypto = test_crypto();
        let plaintext = b"";

        let (encrypted, salt) = crypto.encrypt(plaintext).unwrap();
        let decrypted = crypto.decrypt(&encrypted, &salt).unwrap();
        assert!(decrypted.is_empty());
    }

    #[test]
    fn test_large_plaintext() {
        let crypto = test_crypto();
        // 1 MB of data
        let plaintext = vec![0x42u8; 1024 * 1024];

        let (encrypted, salt) = crypto.encrypt(&plaintext).unwrap();
        let decrypted = crypto.decrypt(&encrypted, &salt).unwrap();
        assert_eq!(decrypted.expose().as_bytes(), plaintext.as_slice());
    }

    #[test]
    fn test_generate_salt_correct_length() {
        let salt = SecretsCrypto::generate_salt();
        assert_eq!(salt.len(), super::SALT_SIZE);
    }

    #[test]
    fn test_generate_salt_nonzero() {
        let salt = SecretsCrypto::generate_salt();
        assert!(salt.iter().any(|&b| b != 0), "salt should not be all zeros");
    }

    #[test]
    fn test_generate_salt_unique() {
        let s1 = SecretsCrypto::generate_salt();
        let s2 = SecretsCrypto::generate_salt();
        assert_ne!(s1, s2, "two generated salts should not be identical");
    }

    #[test]
    fn test_decrypt_truncated_ciphertext() {
        let crypto = test_crypto();
        // Too short: less than NONCE_SIZE + TAG_SIZE (12 + 16 = 28)
        let short = vec![0u8; 10];
        let salt = SecretsCrypto::generate_salt();
        let result = crypto.decrypt(&short, &salt);
        assert!(result.is_err());
        match result.unwrap_err() {
            crate::secrets::types::SecretError::DecryptionFailed(msg) => {
                assert!(msg.contains("too short"));
            }
            other => panic!("expected DecryptionFailed, got {:?}", other),
        }
    }

    #[test]
    fn test_different_master_keys_different_ciphertext() {
        let key_a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let key_b = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let crypto_a = SecretsCrypto::new(SecretString::from(key_a.to_string())).unwrap();
        let crypto_b = SecretsCrypto::new(SecretString::from(key_b.to_string())).unwrap();

        let plaintext = b"shared_secret";
        let (enc_a, salt_a) = crypto_a.encrypt(plaintext).unwrap();
        let (enc_b, salt_b) = crypto_b.encrypt(plaintext).unwrap();

        // Each decrypts its own ciphertext
        let dec_a = crypto_a.decrypt(&enc_a, &salt_a).unwrap();
        let dec_b = crypto_b.decrypt(&enc_b, &salt_b).unwrap();
        assert_eq!(dec_a.expose(), "shared_secret");
        assert_eq!(dec_b.expose(), "shared_secret");

        // Cross-decryption fails
        assert!(crypto_a.decrypt(&enc_b, &salt_b).is_err());
        assert!(crypto_b.decrypt(&enc_a, &salt_a).is_err());
    }

    #[test]
    fn test_exact_minimum_key_length() {
        // Exactly 32 bytes should work
        let key = "a".repeat(super::KEY_SIZE);
        assert!(SecretsCrypto::new(SecretString::from(key)).is_ok());

        // 31 bytes should fail
        let short = "a".repeat(super::KEY_SIZE - 1);
        assert!(SecretsCrypto::new(SecretString::from(short)).is_err());
    }

    #[test]
    fn test_longer_master_key_works() {
        // Keys longer than 32 bytes are fine (HKDF handles it)
        let long_key = "x".repeat(128);
        let crypto = SecretsCrypto::new(SecretString::from(long_key)).unwrap();
        let plaintext = b"works with long key";
        let (encrypted, salt) = crypto.encrypt(plaintext).unwrap();
        let decrypted = crypto.decrypt(&encrypted, &salt).unwrap();
        assert_eq!(decrypted.expose(), "works with long key");
    }

    #[test]
    fn test_debug_redacts_master_key() {
        let crypto = test_crypto();
        let debug = format!("{:?}", crypto);
        assert!(debug.contains("REDACTED"));
        assert!(!debug.contains("0123456789abcdef"));
    }

    #[test]
    fn test_encrypted_output_structure() {
        let crypto = test_crypto();
        let plaintext = b"hello";
        let (encrypted, salt) = crypto.encrypt(plaintext).unwrap();

        // encrypted = nonce (12) + ciphertext (plaintext_len) + tag (16)
        assert_eq!(
            encrypted.len(),
            super::NONCE_SIZE + plaintext.len() + super::TAG_SIZE
        );
        assert_eq!(salt.len(), super::SALT_SIZE);
    }

    #[test]
    fn test_tampered_nonce_fails() {
        let crypto = test_crypto();
        let plaintext = b"sensitive";
        let (mut encrypted, salt) = crypto.encrypt(plaintext).unwrap();

        // Flip a bit in the nonce region (first 12 bytes)
        encrypted[0] ^= 0x01;

        let result = crypto.decrypt(&encrypted, &salt);
        assert!(result.is_err());
    }

    #[test]
    fn test_unicode_plaintext_roundtrip() {
        let crypto = test_crypto();
        let plaintext = "password: p@$$w0rd! 你好 🔑".as_bytes();
        let (encrypted, salt) = crypto.encrypt(plaintext).unwrap();
        let decrypted = crypto.decrypt(&encrypted, &salt).unwrap();
        assert_eq!(decrypted.expose(), "password: p@$$w0rd! 你好 🔑");
    }

    #[test]
    fn test_encrypt_with_info_roundtrip() {
        let crypto = test_crypto();
        let plaintext = b"user profile fact";
        let info = b"ironclaw-profile-v1:user123:default";

        let (encrypted, salt) = crypto.encrypt_with_info(plaintext, info).unwrap();
        let decrypted = crypto.decrypt_with_info(&encrypted, &salt, info).unwrap();
        assert_eq!(decrypted.expose(), "user profile fact");
    }

    #[test]
    fn test_encrypt_with_info_domain_isolation() {
        let crypto = test_crypto();
        let plaintext = b"shared value";
        let info_a = b"ironclaw-profile-v1:alice:default";
        let info_b = b"ironclaw-profile-v1:bob:default";

        let (encrypted, salt) = crypto.encrypt_with_info(plaintext, info_a).unwrap();

        // Decrypting with different info must fail (domain isolation)
        assert!(crypto.decrypt_with_info(&encrypted, &salt, info_b).is_err());

        // Decrypting with original info succeeds
        let decrypted = crypto.decrypt_with_info(&encrypted, &salt, info_a).unwrap();
        assert_eq!(decrypted.expose(), "shared value");
    }

    #[test]
    fn test_encrypt_with_info_vs_plain_encrypt_isolated() {
        let crypto = test_crypto();
        let plaintext = b"same data";

        let (enc_plain, salt_plain) = crypto.encrypt(plaintext).unwrap();
        let (enc_info, salt_info) = crypto
            .encrypt_with_info(plaintext, b"custom-domain")
            .unwrap();

        // Cross-decryption must fail: plain encrypt uses fixed info, with_info uses custom
        assert!(
            crypto
                .decrypt_with_info(&enc_plain, &salt_plain, b"custom-domain")
                .is_err()
        );
        assert!(crypto.decrypt(&enc_info, &salt_info).is_err());
    }
}
