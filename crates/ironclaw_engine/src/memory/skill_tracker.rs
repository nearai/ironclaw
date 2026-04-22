//! Skill confidence tracking.
//!
//! Tracks usage and success/failure metrics for auto-extracted skills.
//! After each thread completes, the active skills' metrics are updated
//! based on whether the thread succeeded or failed.

use std::sync::Arc;

use ironclaw_skills::v2::{SkillRevision, V2SkillMetadata};
use sha2::{Digest, Sha256};

use crate::traits::store::Store;
use crate::types::error::EngineError;
use crate::types::memory::{DocId, DocType, MemoryDoc};

/// Tracks skill usage and updates confidence metrics.
pub struct SkillTracker {
    store: Arc<dyn Store>,
}

fn compute_content_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!(
        "sha256:{}",
        hasher
            .finalize()
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect::<String>()
    )
}

impl SkillTracker {
    pub fn new(store: Arc<dyn Store>) -> Self {
        Self { store }
    }

    /// Record that a skill was used in a completed thread.
    ///
    /// Loads the skill's MemoryDoc, updates metrics in the metadata JSON,
    /// and saves it back. Returns `Err(EngineError::Skill)` if the doc is
    /// missing, not a Skill, or has invalid metadata — callers decide whether
    /// to propagate or log-and-swallow.
    pub async fn record_usage(&self, doc_id: DocId, success: bool) -> Result<(), EngineError> {
        let doc = self
            .store
            .load_memory_doc(doc_id)
            .await?
            .ok_or_else(|| EngineError::Skill {
                reason: format!("skill doc not found: {}", doc_id.0),
            })?;

        if doc.doc_type != DocType::Skill {
            return Err(EngineError::Skill {
                reason: format!("doc {} is not a skill (type: {:?})", doc_id.0, doc.doc_type),
            });
        }

        let mut meta: V2SkillMetadata =
            serde_json::from_value(doc.metadata.clone()).map_err(|e| EngineError::Skill {
                reason: format!("invalid skill metadata for {}: {e}", doc_id.0),
            })?;

        meta.metrics.usage_count += 1;
        if success {
            meta.metrics.success_count += 1;
        } else {
            meta.metrics.failure_count += 1;
        }
        meta.metrics.last_used = Some(chrono::Utc::now());

        let updated_doc = MemoryDoc {
            metadata: serde_json::to_value(&meta).map_err(|e| EngineError::Skill {
                reason: format!("failed to serialize skill metadata: {e}"),
            })?,
            updated_at: chrono::Utc::now(),
            ..doc
        };

        self.store.save_memory_doc(&updated_doc).await
    }

    /// Update a skill's content and increment its version.
    ///
    /// Sets `parent_version` to the current version before incrementing,
    /// enabling rollback if the update causes issues.
    pub async fn update_skill(
        &self,
        doc_id: DocId,
        new_content: String,
        expected_version: Option<u32>,
        updater: impl FnOnce(&mut V2SkillMetadata),
    ) -> Result<(), EngineError> {
        let doc = self
            .store
            .load_memory_doc(doc_id)
            .await?
            .ok_or_else(|| EngineError::Skill {
                reason: format!("skill doc not found: {}", doc_id.0),
            })?;

        let mut meta: V2SkillMetadata =
            serde_json::from_value(doc.metadata.clone()).map_err(|e| EngineError::Skill {
                reason: format!("invalid skill metadata: {e}"),
            })?;

        if let Some(expected) = expected_version
            && meta.version != expected
        {
            return Err(EngineError::Skill {
                reason: format!(
                    "skill {} version conflict: expected {expected}, found {}",
                    doc_id.0, meta.version
                ),
            });
        }

        // Always recompute from actual content — meta.content_hash may have
        // drifted if the doc was updated outside this tracker (e.g. direct
        // memory_write).
        let archived_hash = compute_content_hash(&doc.content);
        meta.revisions.push(SkillRevision {
            version: meta.version,
            content: doc.content.clone(),
            description: meta.description.clone(),
            activation: meta.activation.clone(),
            code_snippets: meta.code_snippets.clone(),
            content_hash: archived_hash,
            archived_at: Some(chrono::Utc::now()),
        });
        // Cap in-memory revisions at 10 to bound metadata size on every
        // load_memory_doc.  This is a pragmatic trade-off: full prompt
        // snapshots embedded in the skill JSON can grow to many KB per
        // revision.  Older revisions are dropped; if long-term retention is
        // needed, they should be externalized to separate MemoryDocs.
        if meta.revisions.len() > 10 {
            let keep_from = meta.revisions.len() - 10;
            meta.revisions.drain(0..keep_from);
        }
        meta.parent_version = Some(meta.version);
        meta.version += 1;
        updater(&mut meta);
        meta.content_hash = compute_content_hash(&new_content);

        let updated_doc = MemoryDoc {
            content: new_content,
            metadata: serde_json::to_value(&meta).map_err(|e| EngineError::Skill {
                reason: format!("failed to serialize skill metadata: {e}"),
            })?,
            updated_at: chrono::Utc::now(),
            ..doc
        };

        self.store.save_memory_doc(&updated_doc).await
    }

    /// Rollback a skill to its previous version.
    ///
    /// If an archived revision exists for `parent_version`, restores the full
    /// content and metadata snapshot. Otherwise falls back to a simple version
    /// decrement without content restoration for older skills.
    pub async fn rollback_skill(&self, doc_id: DocId) -> Result<(), EngineError> {
        let doc = self
            .store
            .load_memory_doc(doc_id)
            .await?
            .ok_or_else(|| EngineError::Skill {
                reason: format!("skill doc not found: {}", doc_id.0),
            })?;

        let mut meta: V2SkillMetadata =
            serde_json::from_value(doc.metadata.clone()).map_err(|e| EngineError::Skill {
                reason: format!("invalid skill metadata: {e}"),
            })?;

        let parent = meta.parent_version.ok_or_else(|| EngineError::Skill {
            reason: format!("skill {} has no parent version to rollback to", doc_id.0),
        })?;

        let revision_opt = meta
            .revisions
            .iter()
            .position(|revision| revision.version == parent);

        let rolled_content = if let Some(revision_index) = revision_opt {
            let revision = meta.revisions[revision_index].clone();
            meta.version = revision.version;
            meta.description = revision.description;
            meta.activation = revision.activation;
            meta.code_snippets = revision.code_snippets;
            meta.content_hash = revision.content_hash;
            meta.revisions
                .retain(|archived| archived.version < revision.version);
            meta.repairs
                .retain(|repair| repair.to_version <= revision.version);
            meta.parent_version = meta.revisions.iter().map(|archived| archived.version).max();
            revision.content
        } else {
            meta.version = parent;
            meta.parent_version = None;
            doc.content.clone()
        };

        let updated_doc = MemoryDoc {
            content: rolled_content,
            metadata: serde_json::to_value(&meta).map_err(|e| EngineError::Skill {
                reason: format!("failed to serialize skill metadata: {e}"),
            })?,
            updated_at: chrono::Utc::now(),
            ..doc
        };

        self.store.save_memory_doc(&updated_doc).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::project::ProjectId;
    use ironclaw_skills::SkillTrust;
    use ironclaw_skills::v2::{SkillMetrics, V2SkillSource};

    fn make_skill_doc(project_id: ProjectId) -> MemoryDoc {
        let meta = V2SkillMetadata {
            name: "test-skill".to_string(),
            version: 1,
            description: "test".to_string(),
            activation: Default::default(),
            source: V2SkillSource::Extracted,
            trust: SkillTrust::Trusted,
            requires: Default::default(),
            code_snippets: vec![],
            metrics: SkillMetrics {
                usage_count: 5,
                success_count: 3,
                failure_count: 2,
                last_used: None,
            },
            parent_version: None,
            revisions: vec![],
            repairs: vec![],
            content_hash: String::new(),
            bundle_path: None,
            source_url: None,
        };

        let mut doc = MemoryDoc::new(
            project_id,
            "test-user",
            DocType::Skill,
            "skill:test",
            "Test skill prompt",
        );
        doc.metadata = serde_json::to_value(&meta).unwrap();
        doc
    }

    #[tokio::test]
    async fn test_record_usage_success() {
        let project_id = ProjectId::new();
        let doc = make_skill_doc(project_id);
        let doc_id = doc.id;

        let store = Arc::new(crate::tests::InMemoryStore::with_docs(vec![doc]));
        let tracker = SkillTracker::new(store.clone());

        tracker.record_usage(doc_id, true).await.unwrap();

        let updated = store.load_memory_doc(doc_id).await.unwrap().unwrap();
        let meta: V2SkillMetadata = serde_json::from_value(updated.metadata).unwrap();
        assert_eq!(meta.metrics.usage_count, 6);
        assert_eq!(meta.metrics.success_count, 4);
        assert_eq!(meta.metrics.failure_count, 2);
        assert!(meta.metrics.last_used.is_some());
    }

    #[tokio::test]
    async fn test_record_usage_failure() {
        let project_id = ProjectId::new();
        let doc = make_skill_doc(project_id);
        let doc_id = doc.id;

        let store = Arc::new(crate::tests::InMemoryStore::with_docs(vec![doc]));
        let tracker = SkillTracker::new(store.clone());

        tracker.record_usage(doc_id, false).await.unwrap();

        let updated = store.load_memory_doc(doc_id).await.unwrap().unwrap();
        let meta: V2SkillMetadata = serde_json::from_value(updated.metadata).unwrap();
        assert_eq!(meta.metrics.usage_count, 6);
        assert_eq!(meta.metrics.success_count, 3);
        assert_eq!(meta.metrics.failure_count, 3);
    }

    #[tokio::test]
    async fn test_update_skill_increments_version() {
        let project_id = ProjectId::new();
        let doc = make_skill_doc(project_id);
        let doc_id = doc.id;

        let store = Arc::new(crate::tests::InMemoryStore::with_docs(vec![doc]));
        let tracker = SkillTracker::new(store.clone());

        tracker
            .update_skill(doc_id, "Updated content".to_string(), None, |meta| {
                meta.description = "Updated description".to_string();
            })
            .await
            .unwrap();

        let updated = store.load_memory_doc(doc_id).await.unwrap().unwrap();
        assert_eq!(updated.content, "Updated content");

        let meta: V2SkillMetadata = serde_json::from_value(updated.metadata).unwrap();
        assert_eq!(meta.version, 2);
        assert_eq!(meta.parent_version, Some(1));
        assert_eq!(meta.description, "Updated description");
        assert_eq!(meta.revisions.len(), 1);
        assert_eq!(meta.revisions[0].version, 1);
    }

    #[tokio::test]
    async fn test_rollback_restores_parent_version() {
        let project_id = ProjectId::new();
        let doc = make_skill_doc(project_id);
        let doc_id = doc.id;

        let store = Arc::new(crate::tests::InMemoryStore::with_docs(vec![doc]));
        let tracker = SkillTracker::new(store.clone());

        // First update to version 2
        tracker
            .update_skill(doc_id, "v2 content".to_string(), None, |_| {})
            .await
            .unwrap();

        // Now rollback
        tracker.rollback_skill(doc_id).await.unwrap();

        let rolled = store.load_memory_doc(doc_id).await.unwrap().unwrap();
        let meta: V2SkillMetadata = serde_json::from_value(rolled.metadata).unwrap();
        assert_eq!(meta.version, 1);
        assert_eq!(meta.parent_version, None);
        assert_eq!(rolled.content, "Test skill prompt");
        assert!(meta.revisions.is_empty());
    }

    #[tokio::test]
    async fn test_rollback_without_parent_fails() {
        let project_id = ProjectId::new();
        let doc = make_skill_doc(project_id);
        let doc_id = doc.id;

        let store = Arc::new(crate::tests::InMemoryStore::with_docs(vec![doc]));
        let tracker = SkillTracker::new(store);

        let result = tracker.rollback_skill(doc_id).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_record_usage_missing_doc() {
        let store = Arc::new(crate::tests::InMemoryStore::with_docs(vec![]));
        let tracker = SkillTracker::new(store);

        let result = tracker.record_usage(DocId::new(), true).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_update_skill_version_conflict() {
        let project_id = ProjectId::new();
        let doc = make_skill_doc(project_id);
        let doc_id = doc.id;

        let store = Arc::new(crate::tests::InMemoryStore::with_docs(vec![doc]));
        let tracker = SkillTracker::new(store);

        let result = tracker
            .update_skill(doc_id, "Updated content".to_string(), Some(2), |_| {})
            .await;

        assert!(result.is_err());
        let error = result.unwrap_err().to_string();
        assert!(
            error.contains("version conflict"),
            "expected version conflict error, got: {error}"
        );
    }
}
