//! User profile persistence engine with at-rest encryption.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::db::UserProfileStore;
use crate::user_profile::error::UserProfileError;
use crate::user_profile::types::{FactCategory, FactSource, ProfileFact, UserProfile};

/// Trait for user profile persistence and retrieval.
#[async_trait]
pub trait UserProfileEngine: Send + Sync {
    /// Load the full profile for a user.
    async fn load_profile(
        &self,
        user_id: &str,
        agent_id: &str,
    ) -> Result<UserProfile, UserProfileError>;

    /// Store or update a single fact (encrypted at rest).
    async fn store_fact(
        &self,
        user_id: &str,
        agent_id: &str,
        fact: &ProfileFact,
    ) -> Result<(), UserProfileError>;

    /// Remove a fact.
    async fn remove_fact(
        &self,
        user_id: &str,
        agent_id: &str,
        category: &FactCategory,
        key: &str,
    ) -> Result<bool, UserProfileError>;

    /// Remove all facts in a category (batch operation).
    async fn clear_facts_by_category(
        &self,
        user_id: &str,
        agent_id: &str,
        category: &FactCategory,
    ) -> Result<u64, UserProfileError>;

    /// Remove all facts for a user+agent (GDPR "forget me").
    async fn clear_profile(&self, user_id: &str, agent_id: &str) -> Result<u64, UserProfileError>;
}

/// Profile engine that encrypts fact values using the existing `SecretsCrypto`.
///
/// Reuses the same HKDF + AES-256-GCM path as credential storage (`src/secrets/crypto.rs`).
/// Each fact gets a unique HKDF-derived key via a random 32-byte salt — no key reuse.
pub struct EncryptedProfileEngine {
    db: Arc<dyn UserProfileStore>,
    crypto: Arc<crate::secrets::SecretsCrypto>,
    max_facts: usize,
    /// Per-user locks to prevent TOCTOU races on max_facts limit.
    /// Key is `"{user_id}:{agent_id}"`. The outer std::sync::Mutex is held
    /// only briefly to get/insert the per-user tokio::sync::Mutex.
    ///
    /// NOTE: This map grows unbounded (one entry per distinct user+agent pair).
    /// Acceptable for single-user personal assistant; add LRU eviction before
    /// supporting multi-user deployments.
    user_locks: std::sync::Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl EncryptedProfileEngine {
    pub fn new(db: Arc<dyn UserProfileStore>, crypto: Arc<crate::secrets::SecretsCrypto>) -> Self {
        Self {
            db,
            crypto,
            max_facts: 100,
            user_locks: std::sync::Mutex::new(HashMap::new()),
        }
    }

    pub fn with_max_facts(mut self, max_facts: usize) -> Self {
        self.max_facts = max_facts;
        self
    }

    fn encrypt_value(&self, plaintext: &str) -> Result<(Vec<u8>, Vec<u8>), UserProfileError> {
        self.crypto
            .encrypt(plaintext.as_bytes())
            .map_err(|e| UserProfileError::EncryptionError {
                reason: e.to_string(),
            })
    }

    fn decrypt_value(&self, encrypted: &[u8], salt: &[u8]) -> Result<String, UserProfileError> {
        let decrypted = self.crypto.decrypt(encrypted, salt).map_err(|e| {
            UserProfileError::DecryptionError {
                reason: e.to_string(),
            }
        })?;
        Ok(decrypted.expose().to_string())
    }

    /// Get or create the per-user lock for atomic check-and-write operations.
    fn user_lock(&self, user_id: &str, agent_id: &str) -> Arc<Mutex<()>> {
        let key = format!("{user_id}:{agent_id}");
        let mut locks = self.user_locks.lock().expect("user_locks poisoned"); // safety: held briefly, no .await
        Arc::clone(locks.entry(key).or_insert_with(|| Arc::new(Mutex::new(()))))
    }
}

#[async_trait]
impl UserProfileEngine for EncryptedProfileEngine {
    async fn load_profile(
        &self,
        user_id: &str,
        agent_id: &str,
    ) -> Result<UserProfile, UserProfileError> {
        let rows = self.db.get_profile_facts(user_id, agent_id).await?;

        let mut facts = Vec::with_capacity(rows.len());
        for row in rows {
            let value = self.decrypt_value(&row.fact_value_encrypted, &row.key_salt)?;
            let category = FactCategory::from_str_opt(&row.category).unwrap_or_else(|| {
                tracing::warn!(
                    "Unknown profile category '{}', defaulting to Context",
                    row.category
                );
                FactCategory::Context
            });

            facts.push(ProfileFact {
                category,
                key: row.fact_key,
                value,
                confidence: row.confidence,
                source: match row.source.as_str() {
                    "explicit" => FactSource::Explicit,
                    "corrected" => FactSource::Corrected,
                    other => {
                        if other != "inferred" {
                            tracing::warn!(
                                "Unknown fact source '{}', defaulting to Inferred",
                                other
                            );
                        }
                        FactSource::Inferred
                    }
                },
                updated_at: row.updated_at,
            });
        }

        Ok(UserProfile { facts })
    }

    async fn store_fact(
        &self,
        user_id: &str,
        agent_id: &str,
        fact: &ProfileFact,
    ) -> Result<(), UserProfileError> {
        // Safety scan before storage (outside the lock — no DB access needed)
        if let Some(threat) = ironclaw_safety::scan_content_for_threats(&fact.value) {
            return Err(UserProfileError::SafetyRejected {
                reason: format!("Fact value matches threat pattern: {threat}"),
            });
        }
        if let Some(threat) = ironclaw_safety::scan_content_for_threats(&fact.key) {
            return Err(UserProfileError::SafetyRejected {
                reason: format!("Fact key matches threat pattern: {threat}"),
            });
        }

        // Acquire per-user lock to prevent TOCTOU race on max_facts limit.
        // Multiple concurrent distillation tasks for the same user are serialized here.
        let lock = self.user_lock(user_id, agent_id);
        let _guard = lock.lock().await;

        // Check fact count limit (inside the lock — atomic with the write below)
        let existing = self.db.get_profile_facts(user_id, agent_id).await?;
        let is_update = existing
            .iter()
            .any(|r| r.category == fact.category.as_str() && r.fact_key == fact.key);
        if !is_update && existing.len() >= self.max_facts {
            return Err(UserProfileError::SafetyRejected {
                reason: format!(
                    "Profile fact limit reached ({}/{})",
                    existing.len(),
                    self.max_facts
                ),
            });
        }

        let (encrypted, salt) = self.encrypt_value(&fact.value)?;

        self.db
            .upsert_profile_fact(
                user_id,
                agent_id,
                fact.category.as_str(),
                &fact.key,
                &encrypted,
                &salt,
                fact.confidence,
                fact.source.as_str(),
            )
            .await?;

        Ok(())
    }

    async fn remove_fact(
        &self,
        user_id: &str,
        agent_id: &str,
        category: &FactCategory,
        key: &str,
    ) -> Result<bool, UserProfileError> {
        Ok(self
            .db
            .delete_profile_fact(user_id, agent_id, category.as_str(), key)
            .await?)
    }

    async fn clear_facts_by_category(
        &self,
        user_id: &str,
        agent_id: &str,
        category: &FactCategory,
    ) -> Result<u64, UserProfileError> {
        Ok(self
            .db
            .delete_profile_facts_by_category(user_id, agent_id, category.as_str())
            .await?)
    }

    async fn clear_profile(&self, user_id: &str, agent_id: &str) -> Result<u64, UserProfileError> {
        Ok(self.db.clear_profile(user_id, agent_id).await?)
    }
}
