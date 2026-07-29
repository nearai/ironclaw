//! Production [`IntentSigner`]: seals per-agent intent keys with
//! `SecretsCrypto` and signs intent pre-images (attested-signing Phase B §B4).
//!
//! This lives here rather than in `ironclaw_attestation` because it needs
//! `ironclaw_secrets`, which the attestation crate is forbidden to depend on
//! (pinned by `attested_signing_boundaries`). Attestation owns the intent type,
//! its pre-image, and verification; this crate owns the private half.
//!
//! ## What the sealing buys
//!
//! Each agent's ed25519 private key is stored AES-256-GCM-sealed under an AAD
//! bound to `(tenant, agent, generation)`
//! ([`ironclaw_secrets::agent_intent_key_aad`]). Table access alone is
//! therefore not enough to move a key: swapping agent A's ciphertext into agent
//! B's row, or replaying a retired generation's ciphertext into the active row,
//! fails the AES-GCM tag check instead of silently producing intents under the
//! wrong attribution.
//!
//! ## Honest threat framing
//!
//! The master key that unseals these lives on the same infrastructure that
//! stores the ciphertext. This is integrity, attribution, and audit across the
//! untrusted chat-channel hop — **not** an independent trust root. An attacker
//! with full backend compromise can sign intents; what they still cannot do is
//! make a Ledger sign a transaction, because the human approves Rust-derived
//! bytes on the device. Nothing here may be treated as authorization.

use std::sync::Arc;

use zeroize::{Zeroize as _, Zeroizing};

use ironclaw_attestation::{
    AGENT_PUBLIC_KEY_LEN, AgentKeyError, AgentKeyId, AgentKeyState, AgentSigningKey,
    AgentSigningKeyStore, INTENT_SIGNATURE_LEN, IntentSigner, IntentSignerError,
};
use ironclaw_secrets::{SecretsCrypto, agent_intent_key_aad};

/// A sealed private key as persisted: ciphertext (nonce-prefixed) plus the
/// per-record salt the KDF used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealedAgentKey {
    /// Nonce-prefixed AES-256-GCM ciphertext of the ed25519 private key.
    pub ciphertext: Vec<u8>,
    /// Per-record KDF salt.
    pub salt: Vec<u8>,
}

/// Storage for the sealed private halves.
///
/// Deliberately separate from
/// [`AgentSigningKeyStore`](ironclaw_attestation::AgentSigningKeyStore), which
/// holds only public keys and lifecycle state: the split is what lets the
/// public registry live in the crypto-free attestation crate while ciphertext
/// stays here.
pub trait SealedAgentKeyStore: Send + Sync {
    /// Persist a sealed key. Insert-only — overwriting would strand every
    /// intent already signed under that generation.
    fn put_sealed(
        &self,
        key_id: &AgentKeyId,
        sealed: SealedAgentKey,
    ) -> Result<(), IntentSignerError>;

    /// Load a sealed key, if one exists.
    fn get_sealed(&self, key_id: &AgentKeyId) -> Result<Option<SealedAgentKey>, IntentSignerError>;
}

/// In-memory [`SealedAgentKeyStore`] for local-dev and tests.
#[derive(Default)]
pub struct InMemorySealedAgentKeyStore {
    keys: std::sync::Mutex<std::collections::HashMap<AgentKeyId, SealedAgentKey>>,
}

impl InMemorySealedAgentKeyStore {
    /// Construct an empty store.
    pub fn new() -> Self {
        Self::default()
    }
}

impl SealedAgentKeyStore for InMemorySealedAgentKeyStore {
    fn put_sealed(
        &self,
        key_id: &AgentKeyId,
        sealed: SealedAgentKey,
    ) -> Result<(), IntentSignerError> {
        let mut keys = self.keys.lock().map_err(|_| IntentSignerError::Backend {
            reason: "sealed agent key store lock poisoned".to_string(),
        })?;
        if keys.contains_key(key_id) {
            return Err(IntentSignerError::Backend {
                reason: "sealed agent key already exists".to_string(),
            });
        }
        keys.insert(key_id.clone(), sealed);
        Ok(())
    }

    fn get_sealed(&self, key_id: &AgentKeyId) -> Result<Option<SealedAgentKey>, IntentSignerError> {
        let keys = self.keys.lock().map_err(|_| IntentSignerError::Backend {
            reason: "sealed agent key store lock poisoned".to_string(),
        })?;
        Ok(keys.get(key_id).cloned())
    }
}

/// The production [`IntentSigner`].
pub struct SecretsIntentSigner {
    crypto: Arc<SecretsCrypto>,
    sealed_keys: Arc<dyn SealedAgentKeyStore>,
    public_keys: Arc<dyn AgentSigningKeyStore>,
}

impl SecretsIntentSigner {
    /// Build the signer over the sealing crypto and the two key stores.
    pub fn new(
        crypto: Arc<SecretsCrypto>,
        sealed_keys: Arc<dyn SealedAgentKeyStore>,
        public_keys: Arc<dyn AgentSigningKeyStore>,
    ) -> Self {
        Self {
            crypto,
            sealed_keys,
            public_keys,
        }
    }

    /// Generate, seal, and register a new key generation for `(tenant, agent)`.
    ///
    /// Ordering matters and is deliberate: the sealed private half is written
    /// FIRST, then the public registry. A crash between the two leaves an
    /// unreferenced ciphertext (harmless, and re-generation picks a new
    /// generation) rather than a public key whose private half does not exist,
    /// which would make every intent signed against it unverifiable.
    pub async fn provision_key(
        &self,
        key_id: &AgentKeyId,
        created_at_ms: i64,
    ) -> Result<[u8; AGENT_PUBLIC_KEY_LEN], IntentSignerError> {
        use ed25519_dalek::SigningKey;

        // Same OS entropy source the trust ceremony's nonce uses (`getrandom`);
        // it errors only when the OS source is unavailable, which is a
        // fail-closed condition, never a silently-weaker key.
        let mut secret = [0u8; 32];
        getrandom::getrandom(&mut secret).map_err(|error| IntentSignerError::Backend {
            reason: format!("entropy unavailable: {error}"),
        })?;
        let signing_key = SigningKey::from_bytes(&secret);
        let public_key = signing_key.verifying_key().to_bytes();

        let aad = agent_intent_key_aad(
            key_id.tenant.as_str(),
            key_id.agent.as_str(),
            key_id.generation,
        );
        // `SecretsCrypto` round-trips UTF-8 (`DecryptedSecret` wraps a String),
        // so the raw 32 bytes are hex-encoded before sealing — the same
        // approach `ironclaw_chain_signing`'s custodial keystore takes for its
        // private keys. `Zeroizing` wipes the encoded copy on drop.
        let hex_secret = Zeroizing::new(hex::encode(secret));
        let (ciphertext, salt) =
            self.crypto
                .encrypt(hex_secret.as_bytes(), &aad)
                .map_err(|error| IntentSignerError::Backend {
                    reason: format!("sealing the agent key failed: {error}"),
                })?;
        // The plaintext secret is not needed past this point.
        secret.zeroize();

        self.sealed_keys
            .put_sealed(key_id, SealedAgentKey { ciphertext, salt })?;
        self.public_keys
            .register(AgentSigningKey {
                key_id: key_id.clone(),
                public_key,
                state: AgentKeyState::Active,
                created_at_ms,
                retiring_since_ms: None,
            })
            .await
            .map_err(map_key_error)?;
        Ok(public_key)
    }
}

#[async_trait::async_trait]
impl IntentSigner for SecretsIntentSigner {
    async fn active_key_id(
        &self,
        tenant: &ironclaw_signing_provider::TenantId,
        agent: &str,
        now_ms: i64,
    ) -> Result<AgentKeyId, IntentSignerError> {
        match self.public_keys.active_key(tenant, agent).await {
            Ok(key) => Ok(key.key_id),
            // First use for this agent: mint generation 1. A key is provisioned
            // lazily rather than at agent creation so an agent that never
            // requests a signature never holds key material.
            Err(AgentKeyError::NotFound) => {
                let key_id = AgentKeyId::new(tenant.clone(), agent, 1);
                self.provision_key(&key_id, now_ms).await?;
                Ok(key_id)
            }
            Err(other) => Err(map_key_error(other)),
        }
    }

    fn sign_intent(
        &self,
        key_id: &AgentKeyId,
        preimage: &[u8],
    ) -> Result<[u8; INTENT_SIGNATURE_LEN], IntentSignerError> {
        use ed25519_dalek::{Signer as _, SigningKey};

        let sealed = self
            .sealed_keys
            .get_sealed(key_id)?
            .ok_or(IntentSignerError::UnknownKey)?;

        // The AAD reconstructs from the REQUESTED key id, so a ciphertext filed
        // under a different agent/generation fails the tag check here rather
        // than signing under the wrong attribution.
        let aad = agent_intent_key_aad(
            key_id.tenant.as_str(),
            key_id.agent.as_str(),
            key_id.generation,
        );
        let decrypted = self
            .crypto
            .decrypt(&sealed.ciphertext, &sealed.salt, &aad)
            .map_err(|_| IntentSignerError::UnknownKey)?;

        let mut plaintext = Zeroizing::new(hex::decode(decrypted.expose()).map_err(|_| {
            IntentSignerError::Backend {
                reason: "sealed agent key is not valid hex".to_string(),
            }
        })?);
        let mut secret = [0u8; 32];
        if plaintext.len() != secret.len() {
            plaintext.zeroize();
            return Err(IntentSignerError::Backend {
                reason: "sealed agent key has the wrong length".to_string(),
            });
        }
        secret.copy_from_slice(&plaintext);
        let signing_key = SigningKey::from_bytes(&secret);
        secret.zeroize();

        Ok(signing_key.sign(preimage).to_bytes())
    }
}

/// Map a public-registry failure onto the signer taxonomy, fail-closed.
fn map_key_error(error: AgentKeyError) -> IntentSignerError {
    match error {
        AgentKeyError::NotFound => IntentSignerError::UnknownKey,
        AgentKeyError::Revoked | AgentKeyError::Retired => IntentSignerError::RevokedKey,
        AgentKeyError::AlreadyExists => IntentSignerError::Backend {
            reason: "agent key generation already registered".to_string(),
        },
        AgentKeyError::Backend { reason } => IntentSignerError::Backend { reason },
        // `AgentKeyError` is `#[non_exhaustive]`: any future variant is treated
        // as a backend failure rather than silently mapped to something
        // permissive.
        other => IntentSignerError::Backend {
            reason: format!("agent key registry error: {other}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    use ironclaw_attestation::{DEFAULT_ROTATION_OVERLAP_MS, InMemoryAgentSigningKeyStore};
    use ironclaw_signing_provider::TenantId;

    const T0: i64 = 1_000_000;

    fn signer() -> (
        SecretsIntentSigner,
        Arc<InMemoryAgentSigningKeyStore>,
        Arc<InMemorySealedAgentKeyStore>,
    ) {
        let crypto = Arc::new(SecretsCrypto::generate());
        let sealed = Arc::new(InMemorySealedAgentKeyStore::new());
        let public = Arc::new(InMemoryAgentSigningKeyStore::new());
        (
            SecretsIntentSigner::new(crypto, sealed.clone(), public.clone()),
            public,
            sealed,
        )
    }

    fn key_id(agent: &str, generation: u32) -> AgentKeyId {
        AgentKeyId::new(TenantId::new("tenant-a"), agent, generation)
    }

    #[tokio::test]
    async fn a_provisioned_key_signs_verifiably_under_its_registered_public_key() {
        let (signer, public, _) = signer();
        let id = key_id("agent-1", 1);
        let pubkey = signer.provision_key(&id, T0).await.expect("provision");

        let signature = signer.sign_intent(&id, b"preimage").expect("sign");

        // The registry hands back the same public key, and it verifies.
        let registered = public
            .verifying_key(&id, T0, DEFAULT_ROTATION_OVERLAP_MS)
            .await
            .expect("registered");
        assert_eq!(registered, pubkey);
        VerifyingKey::from_bytes(&registered)
            .expect("valid key")
            .verify(b"preimage", &Signature::from_bytes(&signature))
            .expect("signature verifies under the registered public key");
    }

    #[tokio::test]
    async fn signing_with_an_unprovisioned_key_fails_closed() {
        let (signer, _, _) = signer();
        assert_eq!(
            signer.sign_intent(&key_id("ghost", 1), b"x"),
            Err(IntentSignerError::UnknownKey)
        );
    }

    /// The AAD binding: a ciphertext filed under another agent's id must not
    /// decrypt, so table access alone cannot move a key between agents.
    #[tokio::test]
    async fn a_key_swapped_into_another_agents_row_fails_the_tag_check() {
        let (signer, _, sealed) = signer();
        let victim = key_id("agent-1", 1);
        signer.provision_key(&victim, T0).await.expect("provision");
        let stolen = sealed.get_sealed(&victim).expect("read").expect("present");

        // Re-file agent-1's ciphertext under agent-2's identity.
        let attacker = key_id("agent-2", 1);
        sealed.put_sealed(&attacker, stolen).expect("planted");

        assert_eq!(
            signer.sign_intent(&attacker, b"x"),
            Err(IntentSignerError::UnknownKey),
            "the AAD binds (tenant, agent, generation); a swapped ciphertext must not decrypt"
        );
    }

    /// The generation is part of the AAD, so a retired generation's ciphertext
    /// cannot be replayed into a newer generation's row.
    #[tokio::test]
    async fn a_ciphertext_replayed_into_another_generation_fails_the_tag_check() {
        let (signer, _, sealed) = signer();
        let gen1 = key_id("agent-1", 1);
        signer.provision_key(&gen1, T0).await.expect("provision");
        let old = sealed.get_sealed(&gen1).expect("read").expect("present");

        let gen2 = key_id("agent-1", 2);
        sealed.put_sealed(&gen2, old).expect("planted");
        assert_eq!(
            signer.sign_intent(&gen2, b"x"),
            Err(IntentSignerError::UnknownKey),
            "rotation must not be reversible by moving ciphertext between generations"
        );
    }

    /// Cross-tenant: the same agent name under another tenant is a different
    /// AAD and must not decrypt.
    #[tokio::test]
    async fn a_key_does_not_decrypt_under_another_tenant() {
        let (signer, _, sealed) = signer();
        let ours = key_id("agent-1", 1);
        signer.provision_key(&ours, T0).await.expect("provision");
        let stolen = sealed.get_sealed(&ours).expect("read").expect("present");

        let foreign = AgentKeyId::new(TenantId::new("tenant-b"), "agent-1", 1);
        sealed.put_sealed(&foreign, stolen).expect("planted");
        assert_eq!(
            signer.sign_intent(&foreign, b"x"),
            Err(IntentSignerError::UnknownKey)
        );
    }

    /// Two generations of the same agent are independent keys — rotation
    /// produces genuinely new material, not a re-seal of the old.
    #[tokio::test]
    async fn rotation_produces_independent_key_material() {
        let (signer, _, _) = signer();
        let gen1 = signer
            .provision_key(&key_id("agent-1", 1), T0)
            .await
            .expect("gen 1");
        let gen2 = signer
            .provision_key(&key_id("agent-1", 2), T0 + 1)
            .await
            .expect("gen 2");
        assert_ne!(gen1, gen2, "a rotation must mint new key material");
    }

    /// Sealed writes are insert-only: re-provisioning a live generation must
    /// not strand intents already signed under it.
    #[tokio::test]
    async fn re_provisioning_a_generation_is_refused() {
        let (signer, _, _) = signer();
        let id = key_id("agent-1", 1);
        signer.provision_key(&id, T0).await.expect("provision");
        assert!(
            signer.provision_key(&id, T0).await.is_err(),
            "re-provisioning would orphan every intent signed under this generation"
        );
    }
}
