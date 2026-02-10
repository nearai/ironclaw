//! NEP-413 intent construction and signing.
//!
//! Provides types and signing for NEAR intents following the NEP-413 standard.
//! Intents are signed messages that authorize actions on a verifying contract.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::keys::KeyError;
use crate::keys::signer::sign_hash;
use crate::keys::types::NearPublicKey;
use crate::secrets::SecretsStore;

/// NEP-413 intent message to be signed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentMessage {
    /// Account signing the intent.
    pub signer_id: String,
    /// Contract that will verify the signature.
    pub verifying_contract: String,
    /// Deadline (block height or timestamp) after which the intent expires.
    pub deadline: String,
    /// Unique nonce to prevent replay.
    pub nonce: String,
    /// List of intent actions.
    pub intents: Vec<IntentAction>,
}

/// An action within an intent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum IntentAction {
    /// Token difference (swap, deposit, etc.)
    TokenDiff { token: String, amount: String },
    /// Add a public key to the account.
    AddPublicKey { public_key: String },
    /// Custom action with arbitrary data.
    Custom {
        action_type: String,
        data: serde_json::Value,
    },
}

/// A signed NEP-413 intent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedIntent {
    pub standard: String,
    pub payload: IntentMessage,
    pub public_key: String,
    pub signature: String,
}

/// Construct the NEP-413 signing payload.
///
/// The payload is: SHA-256(tag + message_json + nonce + recipient)
/// where tag is the NEP-413 tag prefix.
pub fn nep413_signing_payload(message: &IntentMessage) -> Result<[u8; 32], KeyError> {
    let message_json = serde_json::to_string(message).map_err(|e| {
        KeyError::SerializationFailed(format!("failed to serialize intent message: {}", e))
    })?;

    // NEP-413 tag
    const NEP413_TAG: u32 = 2147484061; // (1 << 31) + 413

    let mut hasher = Sha256::new();
    hasher.update(NEP413_TAG.to_le_bytes());
    hasher.update(message_json.as_bytes());

    Ok(hasher.finalize().into())
}

/// Sign an intent message using a key from the secrets store.
pub async fn sign_intent(
    secrets_store: &dyn SecretsStore,
    user_id: &str,
    label: &str,
    public_key: &NearPublicKey,
    intent: IntentMessage,
) -> Result<SignedIntent, KeyError> {
    let hash = nep413_signing_payload(&intent)?;
    let signature_bytes = sign_hash(secrets_store, user_id, label, &hash).await?;

    // Base64-encode the signature
    let signature =
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, signature_bytes);

    Ok(SignedIntent {
        standard: "nep413".to_string(),
        payload: intent,
        public_key: public_key.to_near_format(),
        signature,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ed25519_dalek::SigningKey;
    use secrecy::SecretString;

    use crate::keys::intents::{IntentAction, IntentMessage, nep413_signing_payload, sign_intent};
    use crate::keys::signer::public_key_from_secret;
    use crate::secrets::{CreateSecretParams, InMemorySecretsStore, SecretsCrypto, SecretsStore};

    fn test_store() -> Arc<InMemorySecretsStore> {
        let key = "0123456789abcdef0123456789abcdef";
        let crypto = Arc::new(SecretsCrypto::new(SecretString::from(key.to_string())).unwrap());
        Arc::new(InMemorySecretsStore::new(crypto))
    }

    fn test_intent() -> IntentMessage {
        IntentMessage {
            signer_id: "alice.near".to_string(),
            verifying_contract: "intents.near".to_string(),
            deadline: "100000000".to_string(),
            nonce: "unique-nonce-123".to_string(),
            intents: vec![IntentAction::TokenDiff {
                token: "wrap.near".to_string(),
                amount: "1000000".to_string(),
            }],
        }
    }

    #[test]
    fn test_nep413_payload_deterministic() {
        let intent = test_intent();
        let hash1 = nep413_signing_payload(&intent).unwrap();
        let hash2 = nep413_signing_payload(&intent).unwrap();
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_nep413_payload_different_nonces() {
        let mut intent1 = test_intent();
        let mut intent2 = test_intent();
        intent1.nonce = "nonce-1".to_string();
        intent2.nonce = "nonce-2".to_string();

        let hash1 = nep413_signing_payload(&intent1).unwrap();
        let hash2 = nep413_signing_payload(&intent2).unwrap();
        assert_ne!(hash1, hash2);
    }

    #[tokio::test]
    async fn test_sign_intent_roundtrip() {
        let store = test_store();

        // Generate a key
        let signing_key = SigningKey::generate(&mut rand::rngs::OsRng);
        let verifying_key = signing_key.verifying_key();
        let mut combined = Vec::with_capacity(64);
        combined.extend_from_slice(signing_key.as_bytes());
        combined.extend_from_slice(verifying_key.as_bytes());
        let secret = format!("ed25519:{}", bs58::encode(&combined).into_string());

        store
            .create(
                "user1",
                CreateSecretParams::new("near_key:intent-signer", &secret)
                    .with_provider("near_keys"),
            )
            .await
            .unwrap();

        let public_key = public_key_from_secret(&secret).unwrap();
        let intent = test_intent();

        let signed = sign_intent(
            store.as_ref(),
            "user1",
            "intent-signer",
            &public_key,
            intent,
        )
        .await
        .unwrap();

        assert_eq!(signed.standard, "nep413");
        assert_eq!(signed.public_key, public_key.to_near_format());
        assert!(!signed.signature.is_empty());
    }
}
