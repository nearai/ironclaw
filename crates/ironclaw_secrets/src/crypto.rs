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
use secrecy::zeroize::Zeroizing;
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

    /// Mint a fresh, cryptographically-random in-process master key.
    ///
    /// Intended for ephemeral / in-memory keystores (local-dev, tests) where no
    /// secret must be persisted in source and no stable key is needed across
    /// restarts. The key is a hex-encoded 32-byte `OsRng` draw, which always
    /// satisfies the length + distinct-byte validation in [`Self::new`].
    /// Durable deployments must still source the master key from the OS
    /// keychain / KMS, never from this generator.
    pub fn generate() -> Self {
        let mut key = [0u8; KEY_SIZE];
        rand::RngCore::fill_bytes(&mut OsRng, &mut key);
        let mut hex = String::with_capacity(KEY_SIZE * 2);
        for b in key {
            use std::fmt::Write as _;
            // Infallible: writing to a String never errors.
            let _ = write!(hex, "{b:02x}");
        }
        Self::from_valid_master_key(hex)
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
        let cipher = Aes256Gcm::new_from_slice(derived_key.as_slice())
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
        let cipher = Aes256Gcm::new_from_slice(derived_key.as_slice())
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

    /// Derive the per-secret AES key via HKDF, returned in a [`Zeroizing`]
    /// wrapper so the derived key material is wiped from the stack as soon as
    /// the cipher has consumed it (rather than lingering in freed memory).
    fn derive_key(&self, salt: &[u8]) -> Result<Zeroizing<[u8; KEY_SIZE]>, SecretError> {
        let hk = Hkdf::<Sha256>::new(Some(salt), self.master_key.expose_secret().as_bytes());
        let mut derived = Zeroizing::new([0u8; KEY_SIZE]);
        hk.expand(b"near-agent-secrets-v1", derived.as_mut())
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
pub(crate) const AAD_DOMAIN_FILESYSTEM_SECRET: &[u8] = b"reborn/v1/fs_secret_record";
pub(crate) const AAD_DOMAIN_CHAIN_KEY: &[u8] = b"reborn/v1/chain_key";

/// AAD for a custodial chain-signing key payload, binding ciphertext to
/// `(owner scope, chain)`.
///
/// The attested-signing substrate (PR6, `ironclaw_chain_signing`) stores each
/// per-`(user, chain)` custodial private key as a secret encrypted under this
/// AAD. Binding the chain identity into the AAD is the crypto half of the
/// wrong-chain-confusion defense: a key sealed for chain A authenticates only
/// under chain A's AAD, so an attacker who swaps a chain-A ciphertext into a
/// chain-B keystore row sees the AES-GCM tag check fail
/// (`SecretError::DecryptionFailed`) and never recovers usable key bytes. The
/// owner scope (`tenant/user/agent/project`) matches the account-scope binding
/// used by the credential and filesystem stores so a key cannot be replayed
/// across owners either.
///
/// This is a **pure crypto/AAD-domain** helper: it performs no authorization,
/// no chain validation, and no key handling — it only computes the
/// authenticated-data byte string. Authorization lives in
/// `ironclaw_chain_signing` (grant claim + sign-time hash re-check); this crate
/// stays the secret-material boundary.
pub fn chain_key_aad(scope: &ResourceScope, chain: &str) -> Vec<u8> {
    let key = ScopeKey::from_account_scope(scope);
    build_aad(
        AAD_DOMAIN_CHAIN_KEY,
        &[
            key.tenant_id.as_bytes(),
            key.user_id.as_bytes(),
            key.agent_id.as_bytes(),
            key.project_id.as_bytes(),
            chain.as_bytes(),
        ],
    )
}

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
    // The filesystem secret store keys by *owner scope*
    // (`tenant/user/agent/project`) — see `secret_path` and
    // `same_scope_owner` in `filesystem_store.rs`. The AAD must match the
    // storage scope: previously this bound `mission_id`/`thread_id`/
    // `invocation_id` too, so a secret written by one invocation could be
    // *read* by another invocation under the same owner (the path layer
    // allowed it) but `consume` failed with a confusing decryption error.
    // Bind AAD to the owner scope so cross-invocation reads within the
    // same owner succeed and cross-owner reads still fail closed (both at
    // the path layer and via AAD).
    let key = ScopeKey::from_account_scope(scope);
    build_aad(
        AAD_DOMAIN_FILESYSTEM_SECRET,
        &[
            key.tenant_id.as_bytes(),
            key.user_id.as_bytes(),
            key.agent_id.as_bytes(),
            key.project_id.as_bytes(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{InvocationId, ProjectId, TenantId, UserId};
    use secrecy::SecretString;

    fn scope(tenant: &str, user: &str, project: Option<&str>) -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new(tenant).unwrap(),
            user_id: UserId::new(user).unwrap(),
            agent_id: None,
            project_id: project.map(|p| ProjectId::new(p).unwrap()),
            // Account-scope AAD ignores mission/thread/invocation, so these
            // never affect the chain-key AAD — verified below.
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }

    fn crypto() -> SecretsCrypto {
        // 32 distinct bytes => passes the entropy floor.
        SecretsCrypto::new(SecretString::from(
            "0123456789abcdef0123456789ABCDEF".to_string(),
        ))
        .expect("valid master key")
    }

    #[test]
    fn chain_key_aad_is_deterministic_and_owner_scope_only() {
        let a = scope("tenant-a", "user-a", Some("proj"));
        // Differs only in invocation/mission/thread — owner scope is identical.
        let mut b = a.clone();
        b.invocation_id = InvocationId::new();
        assert_eq!(
            chain_key_aad(&a, "eip155:1"),
            chain_key_aad(&b, "eip155:1"),
            "chain-key AAD binds the owner scope only, not the invocation"
        );
    }

    #[test]
    fn chain_key_aad_differs_per_chain() {
        let s = scope("tenant-a", "user-a", Some("proj"));
        assert_ne!(
            chain_key_aad(&s, "eip155:1"),
            chain_key_aad(&s, "eip155:10"),
            "different chains must produce different AAD"
        );
        assert_ne!(
            chain_key_aad(&s, "eip155:1"),
            chain_key_aad(&s, "solana:mainnet-beta"),
        );
    }

    #[test]
    fn chain_key_aad_differs_per_owner() {
        let chain = "eip155:1";
        assert_ne!(
            chain_key_aad(&scope("tenant-a", "user-a", Some("proj")), chain),
            chain_key_aad(&scope("tenant-a", "user-b", Some("proj")), chain),
        );
        assert_ne!(
            chain_key_aad(&scope("tenant-a", "user-a", Some("proj")), chain),
            chain_key_aad(&scope("tenant-a", "user-a", None), chain),
        );
    }

    #[test]
    fn chain_key_aad_domain_is_distinct_from_other_aads() {
        // A chain-key ciphertext must not decrypt under any other AAD domain
        // even with an identical scope: the domain separator prevents
        // cross-shape replay.
        let s = scope("tenant-a", "user-a", Some("proj"));
        let chain_aad = chain_key_aad(&s, "eip155:1");
        assert_ne!(chain_aad, secret_record_aad("tenant-a", "user-a"));
        assert!(chain_aad.starts_with(AAD_DOMAIN_CHAIN_KEY));
    }

    #[test]
    fn chain_key_ciphertext_fails_under_wrong_chain_aad() {
        // The end-to-end crypto property the keystore relies on: a key sealed
        // for chain A cannot be decrypted under chain B's AAD.
        let crypto = crypto();
        let s = scope("tenant-a", "user-a", Some("proj"));
        let key_material = [7u8; 32];
        let (ct, salt) = crypto
            .encrypt(&key_material, &chain_key_aad(&s, "eip155:1"))
            .expect("encrypt");

        // Right chain decrypts.
        let ok = crypto.decrypt(&ct, &salt, &chain_key_aad(&s, "eip155:1"));
        assert!(ok.is_ok(), "correct chain AAD must decrypt");

        // Wrong chain fails closed.
        let wrong = crypto.decrypt(&ct, &salt, &chain_key_aad(&s, "eip155:10"));
        assert!(
            matches!(wrong, Err(SecretError::DecryptionFailed(_))),
            "wrong-chain AAD must fail decryption, got {wrong:?}"
        );

        // Wrong owner also fails closed.
        let wrong_owner = crypto.decrypt(
            &ct,
            &salt,
            &chain_key_aad(&scope("tenant-a", "user-b", Some("proj")), "eip155:1"),
        );
        assert!(matches!(wrong_owner, Err(SecretError::DecryptionFailed(_))));
    }
}
