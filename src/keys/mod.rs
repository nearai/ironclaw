//! NEAR key management for IronClaw.
//!
//! Manages NEAR Protocol blockchain keys so the agent can sign transactions,
//! intents, and cross-chain signature requests.
//!
//! # Security Model
//!
//! Hybrid custody: the agent holds scoped function-call keys for routine
//! operations. High-value operations require explicit user approval.
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────────┐
//! │                          Key Management                                │
//! │                                                                        │
//! │  KeyManager ──► SecretsStore (AES-256-GCM encrypted private keys)     │
//! │       │                                                                │
//! │       ├──► Signer (ed25519 sign, Zeroize on drop)                     │
//! │       ├──► Policy (analyze transaction, evaluate rules, approve/deny) │
//! │       ├──► SpendTracker (daily cumulative spend)                      │
//! │       └──► RPC Client (nonce, submit, status)                         │
//! │                                                                        │
//! │  INVARIANT: Private keys NEVER reach the LLM or WASM boundary.        │
//! └─────────────────────────────────────────────────────────────────────────┘
//! ```

pub mod chain_signatures;
mod error;
pub mod intents;
pub mod policy;
pub mod rpc;
pub mod signer;
pub mod spending;
pub mod transaction;
pub mod types;

pub use error::KeyError;

use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use ed25519_dalek::SigningKey;
use tokio::fs;
use zeroize::Zeroize;

use crate::keys::policy::{
    ChainSigAnalysis, PolicyConfig, PolicyDecision, SignatureDomain, analyze_transaction,
    infer_target_chain,
};
use crate::keys::rpc::NearRpcClient;
use crate::keys::signer::{public_key_from_secret, sign_hash};
use crate::keys::spending::SpendTracker;
use crate::keys::transaction::{BlockHash, Signature, SignedTransaction, Transaction};
use crate::keys::types::{
    AccessKeyPermission, KeyMetadata, KeyStore, KeyType, NearAccountId, NearNetwork, NearPublicKey,
};
use crate::secrets::{CreateSecretParams, SecretsStore};

/// Result of a signing operation.
#[derive(Debug)]
pub enum SignResult {
    /// Transaction was signed (policy auto-approved).
    Signed {
        transaction: SignedTransaction,
        analysis: policy::TransactionAnalysis,
    },
    /// User must approve before signing can proceed.
    ApprovalRequired {
        analysis: policy::TransactionAnalysis,
        reasons: Vec<String>,
    },
}

/// Central key management struct.
pub struct KeyManager {
    secrets_store: Arc<dyn SecretsStore + Send + Sync>,
    metadata_path: PathBuf,
    policy: PolicyConfig,
    spend_tracker: SpendTracker,
    user_id: String,
}

impl KeyManager {
    /// Create a new KeyManager.
    pub fn new(secrets_store: Arc<dyn SecretsStore + Send + Sync>, user_id: String) -> Self {
        Self {
            secrets_store,
            metadata_path: default_keys_path(),
            policy: PolicyConfig::default(),
            spend_tracker: SpendTracker::new(SpendTracker::default_path()),
            user_id,
        }
    }

    /// Set a custom metadata path (for testing).
    pub fn with_metadata_path(mut self, path: PathBuf) -> Self {
        self.metadata_path = path;
        self
    }

    /// Set the policy config.
    pub fn with_policy(mut self, policy: PolicyConfig) -> Self {
        self.policy = policy;
        self
    }

    /// Set a custom spend tracker (for testing).
    pub fn with_spend_tracker(mut self, tracker: SpendTracker) -> Self {
        self.spend_tracker = tracker;
        self
    }

    /// Get a reference to the current policy config.
    pub fn policy(&self) -> &PolicyConfig {
        &self.policy
    }

    /// Get a mutable reference to the policy config.
    pub fn policy_mut(&mut self) -> &mut PolicyConfig {
        &mut self.policy
    }

    // -- Key lifecycle --

    /// Generate a new ed25519 keypair and store it.
    pub async fn generate_key(
        &self,
        label: &str,
        account_id: &NearAccountId,
        permission: AccessKeyPermission,
        network: NearNetwork,
    ) -> Result<KeyMetadata, KeyError> {
        // Check for duplicates
        let store = self.load_store().await?;
        if store.keys.contains_key(label) {
            return Err(KeyError::AlreadyExists {
                label: label.to_string(),
            });
        }

        // Generate keypair
        let signing_key = SigningKey::generate(&mut rand::rngs::OsRng);
        let verifying_key = signing_key.verifying_key();

        // Build NEAR-format secret: ed25519:<base58(seed + pubkey)>
        let mut combined = Vec::with_capacity(64);
        combined.extend_from_slice(signing_key.as_bytes());
        combined.extend_from_slice(verifying_key.as_bytes());
        let secret_key = format!("ed25519:{}", bs58::encode(&combined).into_string());
        combined.zeroize();
        // signing_key drops here (Zeroize on drop)

        let public_key = NearPublicKey {
            key_type: KeyType::Ed25519,
            data: verifying_key.to_bytes(),
        };

        // Store private key in secrets store
        let secret_name = format!("near_key:{}", label);
        self.secrets_store
            .create(
                &self.user_id,
                CreateSecretParams::new(&secret_name, &secret_key).with_provider("near_keys"),
            )
            .await?;

        // Build metadata
        let metadata = KeyMetadata {
            label: label.to_string(),
            account_id: account_id.to_string(),
            public_key: public_key.to_near_format(),
            permission,
            network,
            created_at: Utc::now(),
            cached_nonce: None,
        };

        // Save metadata
        let mut store = self.load_store().await?;
        store.keys.insert(label.to_string(), metadata.clone());
        self.save_store(&store).await?;

        Ok(metadata)
    }

    /// Import an existing key from a NEAR-format secret key string.
    pub async fn import_key(
        &self,
        label: &str,
        account_id: &NearAccountId,
        secret_key: &str,
        permission: AccessKeyPermission,
        network: NearNetwork,
    ) -> Result<KeyMetadata, KeyError> {
        // Check for duplicates
        let store = self.load_store().await?;
        if store.keys.contains_key(label) {
            return Err(KeyError::AlreadyExists {
                label: label.to_string(),
            });
        }

        // Validate and derive public key
        let public_key = public_key_from_secret(secret_key)?;

        // Store private key in secrets store
        let secret_name = format!("near_key:{}", label);
        self.secrets_store
            .create(
                &self.user_id,
                CreateSecretParams::new(&secret_name, secret_key).with_provider("near_keys"),
            )
            .await?;

        // Build metadata
        let metadata = KeyMetadata {
            label: label.to_string(),
            account_id: account_id.to_string(),
            public_key: public_key.to_near_format(),
            permission,
            network,
            created_at: Utc::now(),
            cached_nonce: None,
        };

        // Save metadata
        let mut store = self.load_store().await?;
        store.keys.insert(label.to_string(), metadata.clone());
        self.save_store(&store).await?;

        Ok(metadata)
    }

    /// List all stored keys (metadata only).
    pub async fn list_keys(&self) -> Result<Vec<KeyMetadata>, KeyError> {
        let store = self.load_store().await?;
        let mut keys: Vec<KeyMetadata> = store.keys.values().cloned().collect();
        keys.sort_by(|a, b| a.label.cmp(&b.label));
        Ok(keys)
    }

    /// Get metadata for a specific key.
    pub async fn get_key(&self, label: &str) -> Result<KeyMetadata, KeyError> {
        let store = self.load_store().await?;
        store
            .keys
            .get(label)
            .cloned()
            .ok_or_else(|| KeyError::NotFound {
                label: label.to_string(),
            })
    }

    /// Remove a key (deletes from secrets store and metadata).
    pub async fn remove_key(&self, label: &str) -> Result<(), KeyError> {
        let mut store = self.load_store().await?;
        if store.keys.remove(label).is_none() {
            return Err(KeyError::NotFound {
                label: label.to_string(),
            });
        }

        // Delete from secrets store
        let secret_name = format!("near_key:{}", label);
        let _ = self.secrets_store.delete(&self.user_id, &secret_name).await;

        self.save_store(&store).await?;
        Ok(())
    }

    /// Export the public key (NEVER the private key).
    pub async fn export_public_key(&self, label: &str) -> Result<NearPublicKey, KeyError> {
        let metadata = self.get_key(label).await?;
        NearPublicKey::from_near_format(&metadata.public_key)
    }

    // -- Transaction signing --

    /// Sign a transaction with policy enforcement.
    pub async fn sign_transaction(
        &self,
        label: &str,
        receiver_id: &NearAccountId,
        actions: Vec<transaction::Action>,
    ) -> Result<SignResult, KeyError> {
        let metadata = self.get_key(label).await?;

        // Analyze
        let analysis = analyze_transaction(
            receiver_id.as_str(),
            &actions,
            &metadata.permission,
            &self.policy,
        );

        // Check spend
        let daily_spend = self.spend_tracker.get_daily_spend().await?;

        // Evaluate policy
        let decision = self
            .policy
            .evaluate(&analysis, &metadata.permission, daily_spend);

        match decision {
            PolicyDecision::Deny { reason } => Err(KeyError::PolicyDenied { reason }),
            PolicyDecision::RequireApproval { reasons } => {
                Ok(SignResult::ApprovalRequired { analysis, reasons })
            }
            PolicyDecision::AutoApprove => {
                let signed = self
                    .build_and_sign(label, &metadata, receiver_id, actions)
                    .await?;

                // Record spend
                if analysis.total_value_yocto > 0 {
                    let _ = self
                        .spend_tracker
                        .record_spend(analysis.total_value_yocto, analysis.summary.clone(), None)
                        .await;
                }

                Ok(SignResult::Signed {
                    transaction: signed,
                    analysis,
                })
            }
        }
    }

    /// Request a chain signature via MPC.
    pub async fn request_chain_signature(
        &self,
        label: &str,
        payload: &[u8],
        derivation_path: &str,
        domain: SignatureDomain,
    ) -> Result<SignResult, KeyError> {
        let metadata = self.get_key(label).await?;

        // Build chain sig analysis
        let chain_sig = ChainSigAnalysis {
            derivation_path: derivation_path.to_string(),
            domain,
            target_chain: infer_target_chain(derivation_path),
            payload_size: payload.len(),
            risk_level: policy::RiskLevel::Medium,
        };

        let daily_spend = self.spend_tracker.get_daily_spend().await?;
        let decision = self.policy.evaluate_chain_sig(&chain_sig, daily_spend);

        // Build the function call action
        let action =
            chain_signatures::build_chain_signature_action(payload, derivation_path, domain)?;

        let contract = chain_signatures::chain_sig_contract(&metadata.network);
        let contract_id = NearAccountId::new(contract)?;

        // Analyze the underlying transaction too
        let analysis = analyze_transaction(
            contract,
            &[action.clone()],
            &metadata.permission,
            &self.policy,
        );

        match decision {
            PolicyDecision::Deny { reason } => Err(KeyError::PolicyDenied { reason }),
            PolicyDecision::RequireApproval { reasons } => {
                Ok(SignResult::ApprovalRequired { analysis, reasons })
            }
            PolicyDecision::AutoApprove => {
                let signed = self
                    .build_and_sign(label, &metadata, &contract_id, vec![action])
                    .await?;
                Ok(SignResult::Signed {
                    transaction: signed,
                    analysis,
                })
            }
        }
    }

    /// Build and sign a transaction (internal, after policy check passes).
    async fn build_and_sign(
        &self,
        label: &str,
        metadata: &KeyMetadata,
        receiver_id: &NearAccountId,
        actions: Vec<transaction::Action>,
    ) -> Result<SignedTransaction, KeyError> {
        let public_key = NearPublicKey::from_near_format(&metadata.public_key)?;

        // Get nonce and block hash from RPC
        let rpc = NearRpcClient::new(&metadata.network);
        let access_key = rpc
            .view_access_key(&metadata.account_id, &metadata.public_key)
            .await?;

        let nonce = access_key.nonce + 1;
        let block_hash = BlockHash::from_base58(&access_key.block_hash)?;

        let signer_id = NearAccountId::new(&metadata.account_id)?;

        let tx = Transaction {
            signer_id,
            public_key,
            nonce,
            receiver_id: receiver_id.clone(),
            block_hash,
            actions,
        };

        // Hash and sign
        let hash = tx.hash_for_signing()?;
        let sig_bytes = sign_hash(self.secrets_store.as_ref(), &self.user_id, label, &hash).await?;

        Ok(SignedTransaction {
            transaction: tx,
            signature: Signature {
                key_type: KeyType::Ed25519,
                data: sig_bytes,
            },
        })
    }

    // -- Backup / Restore --

    /// Create an encrypted backup of all keys.
    pub async fn create_backup(&self, passphrase: &str) -> Result<Vec<u8>, KeyError> {
        let store = self.load_store().await?;

        let mut entries = Vec::new();
        for (label, metadata) in &store.keys {
            let secret_name = format!("near_key:{}", label);
            let decrypted = self
                .secrets_store
                .get_decrypted(&self.user_id, &secret_name)
                .await
                .map_err(|e| KeyError::BackupError {
                    reason: format!("failed to decrypt key '{}': {}", label, e),
                })?;

            entries.push(KeyBackupEntry {
                label: label.clone(),
                account_id: metadata.account_id.clone(),
                secret_key_near_format: decrypted.expose().to_string(),
                permission: metadata.permission.clone(),
                network: metadata.network.clone(),
            });
        }

        let backup = KeyBackup {
            version: 1,
            created_at: Utc::now(),
            keys: entries,
        };

        let plaintext = serde_json::to_vec(&backup).map_err(|e| KeyError::BackupError {
            reason: format!("failed to serialize backup: {}", e),
        })?;

        encrypt_backup(passphrase, &plaintext)
    }

    /// Restore keys from an encrypted backup.
    pub async fn restore_backup(
        &self,
        backup_data: &[u8],
        passphrase: &str,
    ) -> Result<Vec<String>, KeyError> {
        let plaintext = decrypt_backup(passphrase, backup_data)?;

        let backup: KeyBackup =
            serde_json::from_slice(&plaintext).map_err(|e| KeyError::BackupError {
                reason: format!("failed to parse backup: {}", e),
            })?;

        let mut restored = Vec::new();
        for entry in backup.keys {
            // Validate the key
            let _ = public_key_from_secret(&entry.secret_key_near_format)?;
            let account_id = NearAccountId::new(&entry.account_id)?;

            // Import (skip if already exists)
            match self
                .import_key(
                    &entry.label,
                    &account_id,
                    &entry.secret_key_near_format,
                    entry.permission,
                    entry.network,
                )
                .await
            {
                Ok(_) => restored.push(entry.label),
                Err(KeyError::AlreadyExists { .. }) => {
                    // Skip existing keys
                }
                Err(e) => return Err(e),
            }
        }

        // Update backup timestamp
        let mut store = self.load_store().await?;
        store.last_backup_at = Some(Utc::now());
        self.save_store(&store).await?;

        Ok(restored)
    }

    // -- Internal helpers --

    async fn load_store(&self) -> Result<KeyStore, KeyError> {
        if !self.metadata_path.exists() {
            return Ok(KeyStore::default());
        }

        let content = fs::read_to_string(&self.metadata_path).await?;
        serde_json::from_str(&content).map_err(|e| {
            KeyError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("corrupt keys.json: {}", e),
            ))
        })
    }

    async fn save_store(&self, store: &KeyStore) -> Result<(), KeyError> {
        if let Some(parent) = self.metadata_path.parent() {
            fs::create_dir_all(parent).await?;
        }

        let content = serde_json::to_string_pretty(store).map_err(|e| {
            KeyError::SerializationFailed(format!("failed to serialize key store: {}", e))
        })?;

        fs::write(&self.metadata_path, content).await?;
        Ok(())
    }
}

/// Default path for keys metadata.
fn default_keys_path() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".ironclaw").join("keys.json"))
        .unwrap_or_else(|| PathBuf::from(".ironclaw/keys.json"))
}

// -- Backup encryption --

/// Backup file magic bytes.
const BACKUP_MAGIC: &[u8; 4] = b"ICLK";
const BACKUP_VERSION: u32 = 1;
const ARGON2_SALT_LEN: usize = 32;
const AES_NONCE_LEN: usize = 12;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct KeyBackup {
    version: u32,
    created_at: chrono::DateTime<Utc>,
    keys: Vec<KeyBackupEntry>,
}

#[derive(Serialize, Deserialize)]
struct KeyBackupEntry {
    label: String,
    account_id: String,
    secret_key_near_format: String,
    permission: AccessKeyPermission,
    network: NearNetwork,
}

fn encrypt_backup(passphrase: &str, plaintext: &[u8]) -> Result<Vec<u8>, KeyError> {
    use aes_gcm::{Aes256Gcm, KeyInit, Nonce, aead::Aead};
    use argon2::Argon2;

    // Generate salt
    let mut salt = [0u8; ARGON2_SALT_LEN];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut salt);

    // Derive key with Argon2id
    let mut derived_key = [0u8; 32];
    Argon2::default()
        .hash_password_into(passphrase.as_bytes(), &salt, &mut derived_key)
        .map_err(|e| KeyError::BackupError {
            reason: format!("Argon2 key derivation failed: {}", e),
        })?;

    // Encrypt with AES-256-GCM
    let cipher = Aes256Gcm::new_from_slice(&derived_key).map_err(|e| KeyError::BackupError {
        reason: format!("failed to create cipher: {}", e),
    })?;

    let mut nonce_bytes = [0u8; AES_NONCE_LEN];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| KeyError::BackupError {
            reason: format!("encryption failed: {}", e),
        })?;

    // Assemble: magic + version + salt + nonce + ciphertext
    let mut output = Vec::new();
    output.extend_from_slice(BACKUP_MAGIC);
    output.extend_from_slice(&BACKUP_VERSION.to_le_bytes());
    output.extend_from_slice(&salt);
    output.extend_from_slice(&nonce_bytes);
    output.extend_from_slice(&ciphertext);

    derived_key.zeroize();

    Ok(output)
}

pub(crate) fn decrypt_backup(passphrase: &str, data: &[u8]) -> Result<Vec<u8>, KeyError> {
    use aes_gcm::{Aes256Gcm, KeyInit, Nonce, aead::Aead};
    use argon2::Argon2;

    let header_len = 4 + 4 + ARGON2_SALT_LEN + AES_NONCE_LEN;
    if data.len() < header_len {
        return Err(KeyError::BackupError {
            reason: "backup file too short".to_string(),
        });
    }

    // Check magic
    if &data[..4] != BACKUP_MAGIC {
        return Err(KeyError::BackupError {
            reason: "not a valid IronClaw backup file".to_string(),
        });
    }

    // Check version
    let version = u32::from_le_bytes(data[4..8].try_into().unwrap());
    if version != BACKUP_VERSION {
        return Err(KeyError::BackupError {
            reason: format!("unsupported backup version: {}", version),
        });
    }

    let salt = &data[8..8 + ARGON2_SALT_LEN];
    let nonce_bytes = &data[8 + ARGON2_SALT_LEN..header_len];
    let ciphertext = &data[header_len..];

    // Derive key
    let mut derived_key = [0u8; 32];
    Argon2::default()
        .hash_password_into(passphrase.as_bytes(), salt, &mut derived_key)
        .map_err(|e| KeyError::BackupError {
            reason: format!("Argon2 key derivation failed: {}", e),
        })?;

    // Decrypt
    let cipher = Aes256Gcm::new_from_slice(&derived_key).map_err(|e| KeyError::BackupError {
        reason: format!("failed to create cipher: {}", e),
    })?;

    let nonce = Nonce::from_slice(nonce_bytes);
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| KeyError::BackupError {
            reason: "decryption failed (wrong passphrase?)".to_string(),
        })?;

    derived_key.zeroize();

    Ok(plaintext)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use secrecy::SecretString;
    use tempfile::TempDir;

    use crate::keys::spending::SpendTracker;
    use crate::keys::transaction::{Action, ONE_NEAR, Transfer};
    use crate::keys::types::{AccessKeyPermission, NearAccountId, NearNetwork};
    use crate::keys::{KeyManager, SignResult};
    use crate::secrets::{InMemorySecretsStore, SecretsCrypto};

    fn test_manager(dir: &TempDir) -> KeyManager {
        let key = "0123456789abcdef0123456789abcdef";
        let crypto = Arc::new(SecretsCrypto::new(SecretString::from(key.to_string())).unwrap());
        let store: Arc<dyn crate::secrets::SecretsStore + Send + Sync> =
            Arc::new(InMemorySecretsStore::new(crypto));

        KeyManager::new(store, "test_user".to_string())
            .with_metadata_path(dir.path().join("keys.json"))
            .with_spend_tracker(SpendTracker::new(dir.path().join("spend.json")))
    }

    #[tokio::test]
    async fn test_generate_key() {
        let dir = TempDir::new().unwrap();
        let manager = test_manager(&dir);

        let account = NearAccountId::new("alice.testnet").unwrap();
        let metadata = manager
            .generate_key(
                "test-key",
                &account,
                AccessKeyPermission::FunctionCall {
                    allowance: None,
                    receiver_id: "intents.near".to_string(),
                    method_names: vec![],
                },
                NearNetwork::Testnet,
            )
            .await
            .unwrap();

        assert_eq!(metadata.label, "test-key");
        assert_eq!(metadata.account_id, "alice.testnet");
        assert!(metadata.public_key.starts_with("ed25519:"));
    }

    #[tokio::test]
    async fn test_generate_duplicate_key_fails() {
        let dir = TempDir::new().unwrap();
        let manager = test_manager(&dir);
        let account = NearAccountId::new("alice.testnet").unwrap();

        manager
            .generate_key(
                "dup",
                &account,
                AccessKeyPermission::FullAccess,
                NearNetwork::Testnet,
            )
            .await
            .unwrap();

        let result = manager
            .generate_key(
                "dup",
                &account,
                AccessKeyPermission::FullAccess,
                NearNetwork::Testnet,
            )
            .await;

        assert!(matches!(
            result,
            Err(crate::keys::KeyError::AlreadyExists { .. })
        ));
    }

    #[tokio::test]
    async fn test_list_keys() {
        let dir = TempDir::new().unwrap();
        let manager = test_manager(&dir);
        let account = NearAccountId::new("alice.testnet").unwrap();

        assert_eq!(manager.list_keys().await.unwrap().len(), 0);

        manager
            .generate_key(
                "key-1",
                &account,
                AccessKeyPermission::FullAccess,
                NearNetwork::Testnet,
            )
            .await
            .unwrap();

        manager
            .generate_key(
                "key-2",
                &account,
                AccessKeyPermission::FullAccess,
                NearNetwork::Testnet,
            )
            .await
            .unwrap();

        let keys = manager.list_keys().await.unwrap();
        assert_eq!(keys.len(), 2);
    }

    #[tokio::test]
    async fn test_remove_key() {
        let dir = TempDir::new().unwrap();
        let manager = test_manager(&dir);
        let account = NearAccountId::new("alice.testnet").unwrap();

        manager
            .generate_key(
                "to-remove",
                &account,
                AccessKeyPermission::FullAccess,
                NearNetwork::Testnet,
            )
            .await
            .unwrap();

        manager.remove_key("to-remove").await.unwrap();
        assert!(manager.get_key("to-remove").await.is_err());
    }

    #[tokio::test]
    async fn test_export_public_key() {
        let dir = TempDir::new().unwrap();
        let manager = test_manager(&dir);
        let account = NearAccountId::new("alice.testnet").unwrap();

        let metadata = manager
            .generate_key(
                "export-test",
                &account,
                AccessKeyPermission::FullAccess,
                NearNetwork::Testnet,
            )
            .await
            .unwrap();

        let pubkey = manager.export_public_key("export-test").await.unwrap();
        assert_eq!(pubkey.to_near_format(), metadata.public_key);
    }

    #[tokio::test]
    async fn test_import_key() {
        let dir = TempDir::new().unwrap();
        let manager = test_manager(&dir);
        let account = NearAccountId::new("bob.testnet").unwrap();

        // Generate a test secret key
        let signing_key = ed25519_dalek::SigningKey::generate(&mut rand::rngs::OsRng);
        let verifying_key = signing_key.verifying_key();
        let mut combined = Vec::with_capacity(64);
        combined.extend_from_slice(signing_key.as_bytes());
        combined.extend_from_slice(verifying_key.as_bytes());
        let secret = format!("ed25519:{}", bs58::encode(&combined).into_string());

        let metadata = manager
            .import_key(
                "imported",
                &account,
                &secret,
                AccessKeyPermission::FullAccess,
                NearNetwork::Testnet,
            )
            .await
            .unwrap();

        assert_eq!(metadata.label, "imported");
        assert!(metadata.public_key.starts_with("ed25519:"));
    }

    #[tokio::test]
    async fn test_backup_and_restore_roundtrip() {
        let dir = TempDir::new().unwrap();
        let manager = test_manager(&dir);
        let account = NearAccountId::new("alice.testnet").unwrap();

        // Generate a key
        manager
            .generate_key(
                "backup-test",
                &account,
                AccessKeyPermission::FunctionCall {
                    allowance: None,
                    receiver_id: "contract.near".to_string(),
                    method_names: vec!["deposit".to_string()],
                },
                NearNetwork::Testnet,
            )
            .await
            .unwrap();

        // Create backup
        let backup_data = manager.create_backup("test-passphrase").await.unwrap();
        assert!(!backup_data.is_empty());

        // Restore into a fresh manager
        let dir2 = TempDir::new().unwrap();
        let manager2 = test_manager(&dir2);

        let restored = manager2
            .restore_backup(&backup_data, "test-passphrase")
            .await
            .unwrap();

        assert_eq!(restored, vec!["backup-test"]);

        // Verify the restored key
        let keys = manager2.list_keys().await.unwrap();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].label, "backup-test");
    }

    #[tokio::test]
    async fn test_backup_wrong_passphrase() {
        let dir = TempDir::new().unwrap();
        let manager = test_manager(&dir);
        let account = NearAccountId::new("alice.testnet").unwrap();

        manager
            .generate_key(
                "test",
                &account,
                AccessKeyPermission::FullAccess,
                NearNetwork::Testnet,
            )
            .await
            .unwrap();

        let backup_data = manager.create_backup("correct").await.unwrap();

        let dir2 = TempDir::new().unwrap();
        let manager2 = test_manager(&dir2);

        let result = manager2.restore_backup(&backup_data, "wrong").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_sign_transaction_policy_deny() {
        let dir = TempDir::new().unwrap();
        let key = "0123456789abcdef0123456789abcdef";
        let crypto = Arc::new(SecretsCrypto::new(SecretString::from(key.to_string())).unwrap());
        let store: Arc<dyn crate::secrets::SecretsStore + Send + Sync> =
            Arc::new(InMemorySecretsStore::new(crypto));

        let mut manager = KeyManager::new(store, "test_user".to_string())
            .with_metadata_path(dir.path().join("keys.json"))
            .with_spend_tracker(SpendTracker::new(dir.path().join("spend.json")));

        // Deny full access operations
        manager.policy_mut().deny_full_access_operations = true;

        let account = NearAccountId::new("alice.testnet").unwrap();
        manager
            .generate_key(
                "denied",
                &account,
                AccessKeyPermission::FullAccess,
                NearNetwork::Testnet,
            )
            .await
            .unwrap();

        let receiver = NearAccountId::new("bob.testnet").unwrap();
        let result = manager
            .sign_transaction(
                "denied",
                &receiver,
                vec![Action::Transfer(Transfer { deposit: 0 })],
            )
            .await;

        assert!(matches!(
            result,
            Err(crate::keys::KeyError::PolicyDenied { .. })
        ));
    }

    #[tokio::test]
    async fn test_sign_transaction_requires_approval() {
        let dir = TempDir::new().unwrap();
        let manager = test_manager(&dir);

        let account = NearAccountId::new("alice.testnet").unwrap();
        manager
            .generate_key(
                "signer",
                &account,
                AccessKeyPermission::FullAccess,
                NearNetwork::Testnet,
            )
            .await
            .unwrap();

        let receiver = NearAccountId::new("unknown.testnet").unwrap();
        let result = manager
            .sign_transaction(
                "signer",
                &receiver,
                vec![Action::Transfer(Transfer {
                    deposit: 100 * ONE_NEAR,
                })],
            )
            .await
            .unwrap();

        // Default policy requires approval for any transfer
        assert!(matches!(result, SignResult::ApprovalRequired { .. }));
    }
}
