//! Port of IronClaw's battle-tested secret crypto.
//!
//! Uses AES-256-GCM with per-secret HKDF-SHA256 key derivation, matching the
//! existing `src/secrets/crypto.rs` implementation so Reborn does not introduce
//! a parallel encryption scheme.

use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, AeadCore, OsRng, Payload},
};
use hkdf::Hkdf;
use secrecy::{ExposeSecret, SecretString};
use sha2::Sha256;

use crate::SecretError;
use crate::legacy_store::DecryptedSecret;

const KEY_SIZE: usize = 32;
const NONCE_SIZE: usize = 12;
const SALT_SIZE: usize = 32;
const TAG_SIZE: usize = 16;
/// Minimum distinct-byte count for a master key.
///
/// HKDF accepts any IKM but its security degrades to brute-force when the IKM
/// has trivial entropy. A length-only check accepts 32 bytes of `0`, 32 bytes
/// of `a`, or short alphabet repeats — all of which an operator might paste
/// while bootstrapping. Requiring at least 8 distinct bytes rejects those
/// cases while leaving room for legitimate hex/base64 keys (typical 32-byte
/// hex strings use 16 distinct alphabet characters; random 32-byte keys have
/// ~30 distinct byte values on average).
const KEY_MIN_DISTINCT_BYTES: usize = 8;

pub struct SecretsCrypto {
    master_key: SecretString,
}

impl SecretsCrypto {
    pub fn new(master_key: SecretString) -> Result<Self, SecretError> {
        let bytes = master_key.expose_secret().as_bytes();
        if bytes.len() < KEY_SIZE {
            return Err(SecretError::InvalidMasterKey);
        }
        if distinct_byte_count(bytes) < KEY_MIN_DISTINCT_BYTES {
            return Err(SecretError::InvalidMasterKey);
        }
        Ok(Self { master_key })
    }

    pub(crate) fn from_valid_master_key(master_key: String) -> Self {
        // The caller is limited to crate-owned key generation whose byte length is reviewed.
        // This keeps infallible test/demo store construction out of production panic paths
        // while preserving `new` validation for externally supplied dynamic keys.
        Self {
            master_key: SecretString::from(master_key),
        }
    }

    pub fn generate_salt() -> Vec<u8> {
        let mut salt = vec![0u8; SALT_SIZE];
        rand::RngCore::fill_bytes(&mut OsRng, &mut salt);
        salt
    }

    /// Encrypt `plaintext` and authenticate it against `aad`.
    ///
    /// The `aad` (additional authenticated data) is *not* encrypted but is
    /// covered by the AES-GCM authentication tag. Callers must pass the same
    /// `aad` to [`Self::decrypt`] or the tag check fails. Storage layers use
    /// this to bind ciphertext to the row identity (scope/handle, account id,
    /// session id, etc.) so an attacker with DB write access cannot swap
    /// `(encrypted_value, key_salt)` between rows — the swapped ciphertext
    /// was authenticated under a different `aad` and decryption fails with
    /// `SecretError::DecryptionFailed`.
    pub fn encrypt(&self, plaintext: &[u8], aad: &[u8]) -> Result<(Vec<u8>, Vec<u8>), SecretError> {
        let salt = Self::generate_salt();
        let derived_key = self.derive_key(&salt)?;
        let cipher = Aes256Gcm::new_from_slice(&derived_key)
            .map_err(|error| SecretError::EncryptionFailed(error.to_string()))?;
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ciphertext = cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: plaintext,
                    aad,
                },
            )
            .map_err(|error| SecretError::EncryptionFailed(error.to_string()))?;
        let mut encrypted = Vec::with_capacity(NONCE_SIZE + ciphertext.len());
        encrypted.extend_from_slice(&nonce);
        encrypted.extend_from_slice(&ciphertext);
        Ok((encrypted, salt))
    }

    /// Decrypt `encrypted_value` and verify the AES-GCM tag against `aad`.
    ///
    /// Must pass the same `aad` that was supplied to [`Self::encrypt`]; a
    /// mismatch returns `SecretError::DecryptionFailed`.
    pub fn decrypt(
        &self,
        encrypted_value: &[u8],
        salt: &[u8],
        aad: &[u8],
    ) -> Result<DecryptedSecret, SecretError> {
        DecryptedSecret::from_bytes(self.decrypt_bytes(encrypted_value, salt, aad)?)
    }

    pub fn decrypt_bytes(
        &self,
        encrypted_value: &[u8],
        salt: &[u8],
        aad: &[u8],
    ) -> Result<Vec<u8>, SecretError> {
        if encrypted_value.len() < NONCE_SIZE + TAG_SIZE {
            return Err(SecretError::DecryptionFailed(
                "encrypted value too short".to_string(),
            ));
        }
        let derived_key = self.derive_key(salt)?;
        let cipher = Aes256Gcm::new_from_slice(&derived_key)
            .map_err(|error| SecretError::DecryptionFailed(error.to_string()))?;
        let (nonce_bytes, ciphertext) = encrypted_value.split_at(NONCE_SIZE);
        let nonce = Nonce::from_slice(nonce_bytes);
        let plaintext = cipher
            .decrypt(
                nonce,
                Payload {
                    msg: ciphertext,
                    aad,
                },
            )
            .map_err(|error| SecretError::DecryptionFailed(error.to_string()))?;
        Ok(plaintext)
    }

    fn derive_key(&self, salt: &[u8]) -> Result<[u8; KEY_SIZE], SecretError> {
        let hk = Hkdf::<Sha256>::new(Some(salt), self.master_key.expose_secret().as_bytes());
        let mut derived = [0u8; KEY_SIZE];
        hk.expand(b"near-agent-secrets-v1", &mut derived)
            .map_err(|_| SecretError::EncryptionFailed("HKDF expansion failed".to_string()))?;
        Ok(derived)
    }
}

impl std::fmt::Debug for SecretsCrypto {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SecretsCrypto")
            .field("master_key", &"[REDACTED]")
            .finish()
    }
}

impl ironclaw_filesystem::EntryCipher for SecretsCrypto {
    fn encrypt_entry_bytes(
        &self,
        plaintext: &[u8],
        aad: &[u8],
    ) -> Result<(Vec<u8>, Vec<u8>), ironclaw_filesystem::EntryCipherError> {
        self.encrypt(plaintext, aad)
            .map_err(|error| ironclaw_filesystem::EntryCipherError::new(error.to_string()))
    }

    fn decrypt_entry_bytes(
        &self,
        ciphertext: &[u8],
        salt: &[u8],
        aad: &[u8],
    ) -> Result<Vec<u8>, ironclaw_filesystem::EntryCipherError> {
        self.decrypt_bytes(ciphertext, salt, aad)
            .map_err(|error| ironclaw_filesystem::EntryCipherError::new(error.to_string()))
    }
}

/// Build domain-separated, length-prefixed AAD bytes.
///
/// Each call writes the domain tag followed by every part as
/// `(u64-be length || bytes)`. Length prefixes keep the encoding unambiguous
/// even when parts contain arbitrary bytes (delimiters in part contents
/// cannot be confused with the framing), and the domain tag prevents
/// cross-shape replay (a credential-account ciphertext cannot be replayed as
/// a secret-record ciphertext, etc.). The length is encoded as `u64` so the
/// conversion from `usize` is infallible on all supported platforms (where
/// `usize` is at most 64 bits) and cannot panic on attacker-influenced part
/// lengths such as user-chosen secret names.
pub(crate) fn build_aad(domain: &[u8], parts: &[&[u8]]) -> Vec<u8> {
    const LENGTH_PREFIX_BYTES: usize = size_of::<u64>();
    let capacity = domain.len()
        + parts
            .iter()
            .map(|part| LENGTH_PREFIX_BYTES + part.len())
            .sum::<usize>();
    let mut aad = Vec::with_capacity(capacity);
    aad.extend_from_slice(domain);
    for part in parts {
        let length = part.len() as u64;
        aad.extend_from_slice(&length.to_be_bytes());
        aad.extend_from_slice(part);
    }
    aad
}

pub(crate) const AAD_DOMAIN_SECRET_RECORD: &[u8] = b"reborn/v1/secret_record";
#[cfg(any(feature = "libsql", feature = "postgres"))]
pub(crate) const AAD_DOMAIN_CREDENTIAL_ACCOUNT: &[u8] = b"reborn/v1/credential_account";
#[cfg(any(feature = "libsql", feature = "postgres"))]
pub(crate) const AAD_DOMAIN_CREDENTIAL_SESSION: &[u8] = b"reborn/v1/credential_session";
pub(crate) const AAD_DOMAIN_SECRET_STORE_KEY_CHECK: &[u8] = b"reborn/v1/secret_store_key_check";

/// AAD for the secret-record AES-GCM payload, binding ciphertext to
/// `(user_id, name)`.
///
/// Production storage code reaches this through the higher-level
/// `SecretStore` / `SecretsStore` API and never needs to call it directly.
/// It is `pub` so contract tests and integration fixtures that bypass the
/// store and write directly to `reborn_secret_records` can construct
/// ciphertext the production code will accept.
pub fn secret_record_aad(user_id: &str, name: &str) -> Vec<u8> {
    build_aad(
        AAD_DOMAIN_SECRET_RECORD,
        &[user_id.as_bytes(), name.as_bytes()],
    )
}

/// AAD for the readiness sentinel row in `reborn_secret_store_key_check`.
///
/// Same fixture-only motivation as [`secret_record_aad`].
pub fn secret_store_key_check_aad() -> Vec<u8> {
    build_aad(AAD_DOMAIN_SECRET_STORE_KEY_CHECK, &[])
}

/// Count of distinct byte values in the slice.
///
/// Used as a low-entropy heuristic in [`SecretsCrypto::new`]. A 32-bit bitmap
/// over the 256-byte alphabet (one bit per byte value) keeps this branch
/// constant-time-ish on key length, which matters because the input is a
/// secret.
fn distinct_byte_count(bytes: &[u8]) -> usize {
    let mut seen = [0u64; 4];
    for byte in bytes {
        let slot = (byte >> 6) as usize;
        let bit = byte & 0x3f;
        seen[slot] |= 1u64 << bit;
    }
    seen.iter().map(|word| word.count_ones() as usize).sum()
}
