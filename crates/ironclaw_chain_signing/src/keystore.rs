//! Custodial chain-key / account-binding store.
//!
//! Per `(user, chain)` there is one custodial key whose **private bytes are a
//! secret** encrypted with [`ironclaw_secrets::SecretsCrypto`] under the
//! [`ironclaw_secrets::chain_key_aad`] domain. Alongside the encrypted key the
//! store keeps a public **binding**: the chain id, the bound public address /
//! pubkey, and the derivation path. The binding is what the custodial signer
//! checks against the recovered signer at sign time (EVM ecrecover) and what
//! the AAD pins the ciphertext to.
//!
//! ## Security
//!
//! * Private key bytes are returned only inside [`secrecy::SecretBox`] and are
//!   never logged, audited, or placed in an error. [`ConsumedChainKey`] zeroizes
//!   on drop.
//! * `consume` decrypts under the chain AAD; a ciphertext sealed for a
//!   different chain or owner fails the AES-GCM tag check (wrong-chain
//!   confusion defense, crypto half).
//! * Durable PG / libSQL backends are a stacked follow-up; the in-memory and
//!   SecretsCrypto-backed impls here both go through the same [`KeyStore`]
//!   trait so the custodial signer is backend-agnostic.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use ironclaw_host_api::ResourceScope;
use ironclaw_secrets::{SecretsCrypto, chain_key_aad};
use secrecy::zeroize::Zeroizing;
use secrecy::{ExposeSecret, SecretBox};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::chain::{ChainFamily, ChainKeyId};

/// Public, non-secret binding metadata for a custodial key.
///
/// Everything here is public (addresses and paths are not secret) and is safe
/// to surface in logs / audit. The private key is stored separately, encrypted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChainKeyBinding {
    /// Chain / network the key is bound to.
    pub chain: ChainKeyId,
    /// Bound public address / pubkey, lowercase hex (no `0x`), as the chain
    /// presents it. For EVM this is the 20-byte address; for ed25519 chains the
    /// 32-byte public key.
    pub public_address_hex: String,
    /// Numeric chain id for EVM (`eip155:<id>`), echoed for convenience; `None`
    /// for non-EVM chains.
    pub evm_chain_id: Option<u64>,
    /// BIP-32 / SLIP-0010 derivation path the key was derived at (display only).
    pub derivation_path: String,
    /// Opaque KMS/HSM key reference, when this key is custodied by a sign-only
    /// backend ([`crate::kms::KmsSigner`]) rather than held as a hot key. `Some`
    /// is REQUIRED for mainnet signing (the ship-gate routes mainnet through the
    /// KMS path); `None` means hot-key custody, which the ship-gate permits for
    /// testnet/dev only. Never key material — just a handle.
    pub kms_key_ref: Option<String>,
}

/// A decrypted custodial private key, held only transiently.
///
/// Wraps the bytes in [`SecretBox`] so they are zeroized on drop and never
/// appear in `Debug`. The custodial signer consumes one of these, signs, and
/// drops it immediately.
pub struct ConsumedChainKey {
    binding: ChainKeyBinding,
    private_key: SecretBox<[u8]>,
}

impl ConsumedChainKey {
    /// Construct from a binding and raw private-key bytes.
    pub fn new(binding: ChainKeyBinding, private_key: Vec<u8>) -> Self {
        Self {
            binding,
            private_key: SecretBox::new(private_key.into_boxed_slice()),
        }
    }

    /// Borrow the binding metadata (public).
    pub fn binding(&self) -> &ChainKeyBinding {
        &self.binding
    }

    /// Expose the private key bytes. Callers MUST NOT log, store, or copy these
    /// beyond the immediate signing call.
    pub fn expose_private_key(&self) -> &[u8] {
        self.private_key.expose_secret()
    }
}

impl std::fmt::Debug for ConsumedChainKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConsumedChainKey")
            .field("binding", &self.binding)
            .field("private_key", &"[REDACTED]")
            .finish()
    }
}

/// Keystore failures. Never carries key material.
#[derive(Debug, Error)]
pub enum KeyStoreError {
    /// No key is bound for the requested `(scope, chain)`.
    #[error("no custodial key bound for this scope/chain")]
    NotFound,

    /// A key already exists for `(scope, chain)` and bootstrap would overwrite
    /// it (one-shot create).
    #[error("a custodial key already exists for this scope/chain")]
    AlreadyExists,

    /// The requested chain family does not match the key's bound family.
    #[error("chain family mismatch: key is {bound:?}, request is {requested:?}")]
    ChainFamilyMismatch {
        /// Bound family.
        bound: ChainFamily,
        /// Requested family.
        requested: ChainFamily,
    },

    /// Decryption failed (wrong AAD / corrupt ciphertext). Opaque — never key
    /// bytes.
    #[error("key decryption failed: {reason}")]
    Decryption {
        /// Opaque description.
        reason: String,
    },

    /// A backend-internal failure.
    #[error("keystore backend error: {reason}")]
    Backend {
        /// Opaque description.
        reason: String,
    },
}

/// Custodial chain-key store.
#[async_trait]
pub trait KeyStore: Send + Sync {
    /// Bind a fresh custodial key for `(scope, chain)`. One-shot per key:
    /// a second bind fails with [`KeyStoreError::AlreadyExists`]. The
    /// `private_key` is encrypted under the chain AAD before storage.
    async fn bind(
        &self,
        scope: &ResourceScope,
        binding: ChainKeyBinding,
        private_key: Vec<u8>,
    ) -> Result<(), KeyStoreError>;

    /// Read the public binding for `(scope, chain)` without touching the key.
    async fn binding(
        &self,
        scope: &ResourceScope,
        chain: &ChainKeyId,
    ) -> Result<ChainKeyBinding, KeyStoreError>;

    /// Decrypt and return the custodial key for `(scope, chain)`.
    ///
    /// The `requested_family` is checked against the bound family BEFORE any
    /// decryption: a key bound to chain family A cannot be consumed for a
    /// chain-family-B request (wrong-chain confusion defense, typed half).
    async fn consume(
        &self,
        scope: &ResourceScope,
        chain: &ChainKeyId,
        requested_family: ChainFamily,
    ) -> Result<ConsumedChainKey, KeyStoreError>;
}

/// The `(owner scope, chain)` map key. Bound to the account-scope owner only,
/// matching the chain-key AAD.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct KeyStoreKey {
    tenant: String,
    user: String,
    agent: String,
    project: String,
    chain: String,
}

impl KeyStoreKey {
    fn new(scope: &ResourceScope, chain: &ChainKeyId) -> Self {
        Self {
            tenant: scope.tenant_id.to_string(),
            user: scope.user_id.to_string(),
            agent: scope
                .agent_id
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
            project: scope
                .project_id
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
            chain: chain.as_str().to_string(),
        }
    }
}

struct StoredKey {
    binding: ChainKeyBinding,
    /// nonce ∥ ciphertext as produced by `SecretsCrypto::encrypt`.
    encrypted: Vec<u8>,
    /// HKDF salt produced alongside the ciphertext.
    salt: Vec<u8>,
}

/// A [`KeyStore`] that encrypts private keys with
/// [`ironclaw_secrets::SecretsCrypto`] under the chain-key AAD and holds the
/// resulting ciphertext in memory.
///
/// Persisting the ciphertext to PG / libSQL is a stacked follow-up; the
/// encryption path (the security-critical part) is identical regardless of
/// where the ciphertext bytes live.
pub struct SecretsKeyStore {
    crypto: SecretsCrypto,
    keys: Mutex<HashMap<KeyStoreKey, StoredKey>>,
}

impl SecretsKeyStore {
    /// Build a keystore over the given crypto context.
    pub fn new(crypto: SecretsCrypto) -> Self {
        Self {
            crypto,
            keys: Mutex::new(HashMap::new()),
        }
    }

    fn lock(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, HashMap<KeyStoreKey, StoredKey>>, KeyStoreError> {
        self.keys.lock().map_err(|e| KeyStoreError::Backend {
            reason: e.to_string(),
        })
    }
}

#[async_trait]
impl KeyStore for SecretsKeyStore {
    async fn bind(
        &self,
        scope: &ResourceScope,
        binding: ChainKeyBinding,
        private_key: Vec<u8>,
    ) -> Result<(), KeyStoreError> {
        let key = KeyStoreKey::new(scope, &binding.chain);
        let aad = chain_key_aad(scope, binding.chain.as_str());
        // `SecretsCrypto` round-trips UTF-8 string secrets (its `DecryptedSecret`
        // is UTF-8-only). Private key bytes are arbitrary binary, so we hex-encode
        // before encrypting and decode after decrypting — the AES-GCM tag still
        // covers the (hex) plaintext and the chain AAD unchanged.
        //
        // Both the caller-owned `private_key` Vec and the transient hex `String`
        // hold raw key material; wrap them in `Zeroizing` so they are wiped from
        // memory as soon as encryption consumes them (review finding #6).
        let private_key = Zeroizing::new(private_key);
        let hex_key = Zeroizing::new(alloy_primitives::hex::encode(&private_key));
        let (encrypted, salt) =
            self.crypto
                .encrypt(hex_key.as_bytes(), &aad)
                .map_err(|e| KeyStoreError::Backend {
                    reason: e.to_string(),
                })?;
        let mut keys = self.lock()?;
        if keys.contains_key(&key) {
            return Err(KeyStoreError::AlreadyExists);
        }
        keys.insert(
            key,
            StoredKey {
                binding,
                encrypted,
                salt,
            },
        );
        Ok(())
    }

    async fn binding(
        &self,
        scope: &ResourceScope,
        chain: &ChainKeyId,
    ) -> Result<ChainKeyBinding, KeyStoreError> {
        let keys = self.lock()?;
        keys.get(&KeyStoreKey::new(scope, chain))
            .map(|stored| stored.binding.clone())
            .ok_or(KeyStoreError::NotFound)
    }

    async fn consume(
        &self,
        scope: &ResourceScope,
        chain: &ChainKeyId,
        requested_family: ChainFamily,
    ) -> Result<ConsumedChainKey, KeyStoreError> {
        // Typed wrong-chain defense: reject before any decryption.
        let bound_family = chain.family();
        if bound_family != requested_family {
            return Err(KeyStoreError::ChainFamilyMismatch {
                bound: bound_family,
                requested: requested_family,
            });
        }

        let (binding, encrypted, salt) = {
            let keys = self.lock()?;
            let stored = keys
                .get(&KeyStoreKey::new(scope, chain))
                .ok_or(KeyStoreError::NotFound)?;
            (
                stored.binding.clone(),
                stored.encrypted.clone(),
                stored.salt.clone(),
            )
        };

        // Crypto wrong-chain defense: AAD pins ciphertext to (owner, chain).
        let aad = chain_key_aad(scope, chain.as_str());
        let decrypted = self.crypto.decrypt(&encrypted, &salt, &aad).map_err(|e| {
            KeyStoreError::Decryption {
                reason: e.to_string(),
            }
        })?;
        let bytes = alloy_primitives::hex::decode(decrypted.expose()).map_err(|e| {
            KeyStoreError::Decryption {
                reason: e.to_string(),
            }
        })?;
        Ok(ConsumedChainKey::new(binding, bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{InvocationId, ProjectId, TenantId, UserId};
    use secrecy::SecretString;

    fn scope(user: &str) -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new("default").unwrap(),
            user_id: UserId::new(user).unwrap(),
            agent_id: None,
            project_id: Some(ProjectId::new("bootstrap").unwrap()),
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }

    fn crypto() -> SecretsCrypto {
        SecretsCrypto::new(SecretString::from(
            "0123456789abcdef0123456789ABCDEF".to_string(),
        ))
        .unwrap()
    }

    fn binding(chain: &str, addr: &str) -> ChainKeyBinding {
        ChainKeyBinding {
            chain: ChainKeyId::new(chain).expect("valid chain id in test"),
            public_address_hex: addr.to_string(),
            evm_chain_id: chain.strip_prefix("eip155:").and_then(|s| s.parse().ok()),
            derivation_path: "m/44'/60'/0'/0/0".to_string(),
            kms_key_ref: None,
        }
    }

    #[tokio::test]
    async fn bind_then_consume_round_trips_key_bytes() {
        let store = SecretsKeyStore::new(crypto());
        let s = scope("alice");
        let priv_bytes = vec![9u8; 32];
        store
            .bind(&s, binding("eip155:1", "abc"), priv_bytes.clone())
            .await
            .unwrap();
        let consumed = store
            .consume(&s, &ChainKeyId::new("eip155:1").unwrap(), ChainFamily::Evm)
            .await
            .unwrap();
        assert_eq!(consumed.expose_private_key(), priv_bytes.as_slice());
        assert_eq!(consumed.binding().public_address_hex, "abc");
    }

    #[tokio::test]
    async fn double_bind_is_already_exists() {
        let store = SecretsKeyStore::new(crypto());
        let s = scope("alice");
        store
            .bind(&s, binding("eip155:1", "abc"), vec![1u8; 32])
            .await
            .unwrap();
        let err = store
            .bind(&s, binding("eip155:1", "abc"), vec![2u8; 32])
            .await
            .unwrap_err();
        assert!(matches!(err, KeyStoreError::AlreadyExists));
    }

    #[tokio::test]
    async fn wrong_family_rejected_before_decryption() {
        let store = SecretsKeyStore::new(crypto());
        let s = scope("alice");
        store
            .bind(&s, binding("eip155:1", "abc"), vec![1u8; 32])
            .await
            .unwrap();
        // Ask for the EVM-bound key as a Solana key.
        let err = store
            .consume(
                &s,
                &ChainKeyId::new("eip155:1").unwrap(),
                ChainFamily::Solana,
            )
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            KeyStoreError::ChainFamilyMismatch {
                bound: ChainFamily::Evm,
                requested: ChainFamily::Solana
            }
        ));
    }

    #[tokio::test]
    async fn consume_missing_is_not_found() {
        let store = SecretsKeyStore::new(crypto());
        let err = store
            .consume(
                &scope("bob"),
                &ChainKeyId::new("eip155:1").unwrap(),
                ChainFamily::Evm,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, KeyStoreError::NotFound));
    }

    #[tokio::test]
    async fn binding_missing_is_not_found() {
        let store = SecretsKeyStore::new(crypto());
        let err = store
            .binding(&scope("bob"), &ChainKeyId::new("eip155:1").unwrap())
            .await
            .unwrap_err();
        assert!(matches!(err, KeyStoreError::NotFound));
    }

    #[test]
    fn consumed_key_debug_redacts_private_bytes() {
        let consumed = ConsumedChainKey::new(binding("eip155:1", "abc"), vec![7u8; 32]);
        let dbg = format!("{consumed:?}");
        assert!(dbg.contains("[REDACTED]"));
        assert!(!dbg.contains("7, 7, 7"));
    }
}
