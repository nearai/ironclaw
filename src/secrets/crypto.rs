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

    /// Derive a per-secret key using HKDF-SHA256.
    fn derive_key(&self, salt: &[u8]) -> Result<[u8; KEY_SIZE], SecretError> {
        let master_bytes = self.master_key.expose_secret().as_bytes();

        // HKDF extract + expand
        let hk = Hkdf::<Sha256>::new(Some(salt), master_bytes);

        let mut derived = [0u8; KEY_SIZE];
        hk.expand(b"near-agent-secrets-v1", &mut derived)
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

    fn test_crypto() -> SecretsCrypto {
        // 32-byte test key
        let key = "0123456789abcdef0123456789abcdef";
        SecretsCrypto::new(SecretString::from(key.to_string())).unwrap()
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
}
