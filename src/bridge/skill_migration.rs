//! V1 → V2 skill migration.
//!
//! Converts v1 `LoadedSkill` instances (from filesystem SKILL.md files) into
//! v2 `MemoryDoc` with `DocType::Skill` and structured `V2SkillMetadata`.
//! The migration is idempotent: skills with unchanged content_hash are skipped.
//!
//! **Remove after v1 migration is complete.** Once all users are on ENGINE_V2
//! and SKILL.md files are authored directly as v2 MemoryDocs (or via the
//! skill-extraction mission), this one-time migration code is unnecessary.
//! The `migrate_v1_skills` / `migrate_v1_skill_list` functions and the call
//! site in `bridge/router.rs:init_engine()` can all be deleted.

use std::sync::Arc;

use ironclaw_engine::traits::store::Store;
use ironclaw_engine::types::error::EngineError;
use ironclaw_engine::types::memory::{DocType, MemoryDoc};
use ironclaw_engine::types::project::ProjectId;
use ironclaw_engine::types::shared_owner_id;

use ironclaw_skills::SkillRegistry;
use ironclaw_skills::types::{LoadedSkill, SkillSource};
use ironclaw_skills::v2::{SkillMetrics, V2SkillMetadata, V2SkillSource};

/// Migrate v1 skills to v2 MemoryDocs.
///
/// Reads all skills from the v1 `SkillRegistry`, converts each to a `MemoryDoc`
/// with `DocType::Skill` and `V2SkillMetadata`, and saves to the Store.
///
/// Returns the number of skills migrated or updated.
pub async fn migrate_v1_skills(
    v1_registry: &SkillRegistry,
    store: &Arc<dyn Store>,
    project_id: ProjectId,
) -> Result<usize, EngineError> {
    migrate_v1_skill_list(v1_registry.skills(), store, project_id).await
}

/// Migrate a snapshot of v1 skills to v2 MemoryDocs.
///
/// Takes a pre-cloned slice of skills (to avoid holding a lock across await).
pub async fn migrate_v1_skill_list(
    v1_skills: &[LoadedSkill],
    store: &Arc<dyn Store>,
    project_id: ProjectId,
) -> Result<usize, EngineError> {
    if v1_skills.is_empty() {
        return Ok(0);
    }

    // Load existing skill docs to check for duplicates by content_hash
    let existing_docs = store.list_shared_memory_docs(project_id).await?;
    let existing_hashes: std::collections::HashSet<String> = existing_docs
        .iter()
        .filter(|d| d.doc_type == DocType::Skill)
        .filter_map(|d| {
            serde_json::from_value::<V2SkillMetadata>(d.metadata.clone())
                .ok()
                .map(|m| m.content_hash)
        })
        .filter(|h| !h.is_empty())
        .collect();

    let mut migrated = 0;

    for skill in v1_skills {
        // Skip if content hasn't changed (idempotent)
        if existing_hashes.contains(&skill.content_hash) {
            tracing::debug!(
                skill = %skill.name(),
                "skipping v1 skill migration: content unchanged"
            );
            continue;
        }

        let doc = v1_skill_to_memory_doc(skill, project_id).await;
        store.save_memory_doc(&doc).await?;
        migrated += 1;

        tracing::debug!(
            skill = %skill.name(),
            doc_id = %doc.id.0,
            "migrated v1 skill to v2 MemoryDoc"
        );
    }

    if migrated > 0 {
        tracing::debug!("migrated {migrated} v1 skill(s) to v2 engine");
    }

    Ok(migrated)
}

/// Sync a single v1 skill into the v2 store, updating an existing `skill:<name>`
/// doc in place when present.
pub async fn sync_v1_skill_to_store(
    skill: &LoadedSkill,
    store: &Arc<dyn Store>,
    project_id: ProjectId,
) -> Result<MemoryDoc, EngineError> {
    let title = format!("skill:{}", skill.manifest.name);
    let existing = store
        .list_shared_memory_docs(project_id)
        .await?
        .into_iter()
        .find(|doc| doc.doc_type == DocType::Skill && doc.title == title);

    if let Some(existing) = existing.as_ref()
        && existing.content == skill.prompt_content
        && serde_json::from_value::<V2SkillMetadata>(existing.metadata.clone())
            .ok()
            .is_some_and(|meta| meta.content_hash == skill.content_hash)
    {
        return Ok(existing.clone());
    }

    let mut doc = v1_skill_to_memory_doc(skill, project_id).await;
    if let Some(existing) = existing {
        doc.id = existing.id;
        doc.created_at = existing.created_at;
    }
    store.save_memory_doc(&doc).await?;
    Ok(doc)
}

/// Convert a single v1 `LoadedSkill` to a v2 `MemoryDoc`.
async fn v1_skill_to_memory_doc(skill: &LoadedSkill, project_id: ProjectId) -> MemoryDoc {
    let v2_source = match &skill.source {
        SkillSource::Workspace(_) | SkillSource::User(_) | SkillSource::Installed(_) => {
            V2SkillSource::Migrated
        }
        SkillSource::Bundled(_) => V2SkillSource::Migrated,
    };
    let (bundle_path, source_url) = match &skill.source {
        SkillSource::Workspace(path)
        | SkillSource::User(path)
        | SkillSource::Installed(path)
        | SkillSource::Bundled(path) => (
            Some(path.display().to_string()),
            ironclaw_skills::registry::SkillRegistry::read_install_metadata(path)
                .await
                .and_then(|meta| meta.source_url),
        ),
    };

    let meta = V2SkillMetadata {
        name: skill.manifest.name.clone(),
        version: 1,
        description: skill.manifest.description.clone(),
        activation: skill.manifest.activation.clone(),
        source: v2_source,
        trust: skill.trust,
        // Preserve companion list so the v2 orchestrator's chain-loading
        // pass can see which operational skills each persona bundle
        // expects to pull in. Without this, `requires.skills` was
        // silently dropped at migration time and chain-loading in v2
        // was dead code.
        requires: skill.manifest.requires.clone(),
        code_snippets: vec![], // v1 skills are prompt-only
        metrics: SkillMetrics::default(),
        parent_version: None,
        revisions: vec![],
        repairs: vec![],
        content_hash: skill.content_hash.clone(),
        bundle_path,
        source_url,
    };

    let mut doc = MemoryDoc::new(
        project_id,
        shared_owner_id(),
        DocType::Skill,
        format!("skill:{}", skill.manifest.name),
        &skill.prompt_content,
    );
    doc.metadata = serde_json::to_value(&meta).unwrap_or_default();
    doc.tags = vec!["migrated_from_v1".to_string()];
    doc
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_skills::types::{ActivationCriteria, SkillManifest, SkillTrust};
    use std::path::PathBuf;

    fn make_v1_skill(name: &str, content: &str) -> LoadedSkill {
        LoadedSkill {
            manifest: SkillManifest {
                name: name.to_string(),
                version: "1.0.0".to_string(),
                description: format!("{name} skill"),
                activation: ActivationCriteria {
                    keywords: vec!["test".to_string()],
                    ..Default::default()
                },
                credentials: vec![],
                requires: ironclaw_skills::GatingRequirements::default(),
            },
            prompt_content: content.to_string(),
            trust: SkillTrust::Trusted,
            source: SkillSource::User(PathBuf::from("/tmp/test")), // safety: dummy path in test, not used for I/O
            content_hash: ironclaw_skills::compute_hash(content),
            compiled_patterns: vec![],
            lowercased_keywords: vec!["test".to_string()],
            lowercased_exclude_keywords: vec![],
            lowercased_tags: vec![],
        }
    }

    #[tokio::test]
    async fn test_v1_skill_converts_to_memory_doc() {
        let skill = make_v1_skill("test-skill", "Test prompt content");
        let project_id = ProjectId::new();
        let doc = v1_skill_to_memory_doc(&skill, project_id).await;

        assert_eq!(doc.doc_type, DocType::Skill);
        assert_eq!(doc.title, "skill:test-skill");
        assert_eq!(doc.content, "Test prompt content");
        assert_eq!(doc.project_id, project_id);
        assert!(doc.tags.contains(&"migrated_from_v1".to_string()));

        let meta: V2SkillMetadata = serde_json::from_value(doc.metadata).unwrap();
        assert_eq!(meta.name, "test-skill");
        assert_eq!(meta.version, 1);
        assert_eq!(meta.source, V2SkillSource::Migrated);
        assert_eq!(meta.trust, SkillTrust::Trusted);
        assert!(meta.code_snippets.is_empty());
        assert!(!meta.content_hash.is_empty());
        assert_eq!(meta.bundle_path.as_deref(), Some("/tmp/test"));
        assert_eq!(meta.source_url, None);
    }
}
