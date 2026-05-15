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

use ironclaw_host_api::{ResourceScope, SecretHandle};

use crate::SecretError;
use crate::legacy_store::DecryptedSecret;
use crate::{CredentialAccountId, CredentialSessionId};

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
        DecryptedSecret::from_bytes(plaintext)
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
pub(crate) const AAD_DOMAIN_CREDENTIAL_ACCOUNT: &[u8] = b"reborn/v1/credential_account";
pub(crate) const AAD_DOMAIN_CREDENTIAL_SESSION: &[u8] = b"reborn/v1/credential_session";
pub(crate) const AAD_DOMAIN_SECRET_STORE_KEY_CHECK: &[u8] = b"reborn/v1/secret_store_key_check";
pub(crate) const AAD_DOMAIN_FILESYSTEM_SECRET: &[u8] = b"reborn/v1/fs_secret_record";

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

/// Scope-derived key bytes used by credential AAD helpers.
///
/// Centralises the `ResourceScope` → AAD-component mapping so the libSQL,
/// Postgres, and filesystem credential stores all bind ciphertext to the same
/// identity tuple. Two flavours: account-scope clears the
/// mission/thread/invocation slots (accounts are pinned to the owner prefix
/// only), session-scope fills every slot (sessions carry the full execution
/// identity).
pub(crate) struct ScopeKey {
    pub tenant_id: String,
    pub user_id: String,
    pub agent_id: String,
    pub project_id: String,
    pub mission_id: String,
    pub thread_id: String,
    pub invocation_id: String,
}

impl ScopeKey {
    pub(crate) fn from_account_scope(scope: &ResourceScope) -> Self {
        Self {
            tenant_id: scope.tenant_id.to_string(),
            user_id: scope.user_id.to_string(),
            agent_id: scope
                .agent_id
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
            project_id: scope
                .project_id
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
            mission_id: String::new(),
            thread_id: String::new(),
            invocation_id: String::new(),
        }
    }

    pub(crate) fn from_full_scope(scope: &ResourceScope) -> Self {
        Self {
            tenant_id: scope.tenant_id.to_string(),
            user_id: scope.user_id.to_string(),
            agent_id: scope
                .agent_id
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
            project_id: scope
                .project_id
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
            mission_id: scope
                .mission_id
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
            thread_id: scope
                .thread_id
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
            invocation_id: scope.invocation_id.to_string(),
        }
    }
}

/// AAD for the credential-account payload, binding ciphertext to
/// `(scope, account_id)`.
///
/// Used by every credential-account store (libSQL, Postgres, filesystem) so
/// the same payload-id binding holds across backends. Production storage code
/// reaches this through the credential store API and never needs to call it
/// directly. It is `pub` so contract tests and integration fixtures that
/// bypass the store and write directly to the underlying table or path can
/// construct ciphertext the production code will accept — and cross-domain
/// replay tests can prove that swapping AAD between domains fails decryption.
pub fn credential_account_aad(scope: &ResourceScope, account_id: &CredentialAccountId) -> Vec<u8> {
    let key = ScopeKey::from_account_scope(scope);
    build_aad(
        AAD_DOMAIN_CREDENTIAL_ACCOUNT,
        &[
            key.tenant_id.as_bytes(),
            key.user_id.as_bytes(),
            key.agent_id.as_bytes(),
            key.project_id.as_bytes(),
            account_id.as_str().as_bytes(),
        ],
    )
}

/// AAD for the credential-session payload, binding ciphertext to
/// `(scope, session_id)`.
///
/// Same fixture-only motivation as [`credential_account_aad`].
pub fn credential_session_aad(scope: &ResourceScope, session_id: CredentialSessionId) -> Vec<u8> {
    let key = ScopeKey::from_full_scope(scope);
    let session_id_string = session_id.to_private_storage_string();
    build_aad(
        AAD_DOMAIN_CREDENTIAL_SESSION,
        &[
            key.tenant_id.as_bytes(),
            key.user_id.as_bytes(),
            key.agent_id.as_bytes(),
            key.project_id.as_bytes(),
            key.mission_id.as_bytes(),
            key.thread_id.as_bytes(),
            key.invocation_id.as_bytes(),
            session_id_string.as_bytes(),
        ],
    )
}

/// AAD for the filesystem secret-material payload, binding ciphertext to
/// `(scope, handle)`.
///
/// Distinct domain from the DB-backed `secret_record_aad` because the
/// filesystem store keys secrets by `(scope, SecretHandle)` rather than
/// `(user_id, name)` — a swap between the two encodings must fail decryption
/// even with an identical scope/user, which the domain separator enforces.
pub fn filesystem_secret_aad(scope: &ResourceScope, handle: &SecretHandle) -> Vec<u8> {
    let key = ScopeKey::from_full_scope(scope);
    build_aad(
        AAD_DOMAIN_FILESYSTEM_SECRET,
        &[
            key.tenant_id.as_bytes(),
            key.user_id.as_bytes(),
            key.agent_id.as_bytes(),
            key.project_id.as_bytes(),
            key.mission_id.as_bytes(),
            key.thread_id.as_bytes(),
            key.invocation_id.as_bytes(),
            handle.as_str().as_bytes(),
        ],
    )
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
