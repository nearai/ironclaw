//! User profile persistence engine with at-rest encryption.

use std::sync::Arc;

use async_trait::async_trait;

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
}

/// Profile engine that encrypts fact values using the existing `SecretsCrypto`.
///
/// Reuses the same HKDF + AES-256-GCM path as credential storage (`src/secrets/crypto.rs`).
/// Each fact gets a unique HKDF-derived key via a random 32-byte salt — no key reuse.
pub struct EncryptedProfileEngine {
    db: Arc<dyn UserProfileStore>,
    crypto: Arc<crate::secrets::SecretsCrypto>,
    max_facts: usize,
}

impl EncryptedProfileEngine {
    pub fn new(db: Arc<dyn UserProfileStore>, crypto: Arc<crate::secrets::SecretsCrypto>) -> Self {
        Self {
            db,
            crypto,
            max_facts: 100,
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
        // Check fact count limit before writing (allow UPDATE of existing facts)
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

        // Safety scan before storage
        if let Some(threat) = ironclaw_safety::scan_content_for_threats(&fact.value) {
            return Err(UserProfileError::SafetyRejected {
                reason: format!("Fact value matches threat pattern: {threat}"),
            });
        }
        // Also scan the key
        if let Some(threat) = ironclaw_safety::scan_content_for_threats(&fact.key) {
            return Err(UserProfileError::SafetyRejected {
                reason: format!("Fact key matches threat pattern: {threat}"),
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
}
