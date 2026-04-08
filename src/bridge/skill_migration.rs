//! V1 → V2 skill migration.
//!
//! Converts v1 `LoadedSkill` instances (from filesystem SKILL.md files) into
//! v2 `MemoryDoc` with `DocType::Skill` and structured `V2SkillMetadata`.
//! The migration is idempotent: skills with unchanged content_hash are skipped.
//!
//! **Ownership model:**
//! - `Bundled` / `Installed` skills are admin-installed and go into the global
//!   `system_project_id()` under `shared_owner_id()`. Every tenant sees them.
//! - `User` / `Workspace` skills belong to the owner and go into their own
//!   project under `owner_id`. Other tenants do not see them.
//!
//! **Remove after v1 migration is complete.** Once all users are on ENGINE_V2
//! and SKILL.md files are authored directly as v2 MemoryDocs (or via the
//! skill-extraction mission), this one-time migration code is unnecessary.
//! The `migrate_v1_skills` / `migrate_v1_skill_list` functions and the call
//! site in `bridge/router.rs:init_engine()` can all be deleted.

use std::sync::Arc;

use ironclaw_engine::system_project_id;
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
    owner_id: &str,
    tenant_project_id: ProjectId,
) -> Result<usize, EngineError> {
    migrate_v1_skill_list(v1_registry.skills(), store, owner_id, tenant_project_id).await
}

/// Migrate a snapshot of v1 skills to v2 MemoryDocs.
///
/// Takes a pre-cloned slice of skills (to avoid holding a lock across await).
///
/// - Admin skills (`Bundled`/`Installed`) → `system_project_id()`, `user_id = "__shared__"`
/// - Tenant skills (`User`/`Workspace`)   → `tenant_project_id`, `user_id = owner_id`
pub async fn migrate_v1_skill_list(
    v1_skills: &[LoadedSkill],
    store: &Arc<dyn Store>,
    owner_id: &str,
    tenant_project_id: ProjectId,
) -> Result<usize, EngineError> {
    if v1_skills.is_empty() {
        return Ok(0);
    }

    // Load existing docs from both locations to check for duplicates by content_hash.
    let sys = system_project_id();
    let mut existing_docs = store.list_shared_memory_docs(sys).await?;
    existing_docs.extend(
        store
            .list_memory_docs(tenant_project_id, owner_id)
            .await?,
    );
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
        // Skip if content hasn't changed (idempotent).
        if existing_hashes.contains(&skill.content_hash) {
            tracing::debug!(
                skill = %skill.name(),
                "skipping v1 skill migration: content unchanged"
            );
            continue;
        }

        let doc = v1_skill_to_memory_doc(skill, owner_id, tenant_project_id);
        store.save_memory_doc(&doc).await?;
        migrated += 1;

        tracing::debug!(
            skill = %skill.name(),
            doc_id = %doc.id.0,
            project_id = %doc.project_id.0,
            user_id = %doc.user_id,
            "migrated v1 skill to v2 MemoryDoc"
        );
    }

    if migrated > 0 {
        tracing::debug!("migrated {migrated} v1 skill(s) to v2 engine");
    }

    Ok(migrated)
}

/// Convert a single v1 `LoadedSkill` to a v2 `MemoryDoc`.
///
/// Routing:
/// - `Bundled` / `Installed` → admin skill → system project, shared owner
/// - `User` / `Workspace`    → tenant skill → owner's project, owner's user_id
fn v1_skill_to_memory_doc(
    skill: &LoadedSkill,
    owner_id: &str,
    tenant_project_id: ProjectId,
) -> MemoryDoc {
    let (project_id, user_id) = match &skill.source {
        SkillSource::Bundled(_) | SkillSource::Installed(_) => {
            (system_project_id(), shared_owner_id().to_string())
        }
        SkillSource::User(_) | SkillSource::Workspace(_) => {
            (tenant_project_id, owner_id.to_string())
        }
    };

    let meta = V2SkillMetadata {
        name: skill.manifest.name.clone(),
        version: 1,
        description: skill.manifest.description.clone(),
        activation: skill.manifest.activation.clone(),
        source: V2SkillSource::Migrated,
        trust: skill.trust,
        code_snippets: vec![],
        metrics: SkillMetrics::default(),
        parent_version: None,
        revisions: vec![],
        repairs: vec![],
        content_hash: skill.content_hash.clone(),
    };

    let mut doc = MemoryDoc::new(
        project_id,
        user_id,
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

    fn make_skill(name: &str, content: &str, source: SkillSource) -> LoadedSkill {
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
                metadata: None,
            },
            prompt_content: content.to_string(),
            trust: SkillTrust::Trusted,
            source,
            content_hash: ironclaw_skills::compute_hash(content),
            compiled_patterns: vec![],
            lowercased_keywords: vec!["test".to_string()],
            lowercased_exclude_keywords: vec![],
            lowercased_tags: vec![],
        }
    }

    #[test]
    fn bundled_skill_goes_to_system_project() {
        let skill = make_skill(
            "admin-skill",
            "Admin prompt",
            SkillSource::Bundled(PathBuf::from("/bundled")),
        );
        let tenant_project = ProjectId::new();
        let doc = v1_skill_to_memory_doc(&skill, "alice", tenant_project);

        assert_eq!(doc.project_id, system_project_id());
        assert_eq!(doc.user_id, shared_owner_id());
        assert_eq!(doc.doc_type, DocType::Skill);
    }

    #[test]
    fn installed_skill_goes_to_system_project() {
        let skill = make_skill(
            "installed-skill",
            "Installed prompt",
            SkillSource::Installed(PathBuf::from("/installed")),
        );
        let tenant_project = ProjectId::new();
        let doc = v1_skill_to_memory_doc(&skill, "alice", tenant_project);

        assert_eq!(doc.project_id, system_project_id());
        assert_eq!(doc.user_id, shared_owner_id());
    }

    #[test]
    fn user_skill_goes_to_tenant_project() {
        let skill = make_skill(
            "my-skill",
            "Personal prompt",
            SkillSource::User(PathBuf::from("/home/alice/.ironclaw/skills/my-skill")),
        );
        let tenant_project = ProjectId::new();
        let doc = v1_skill_to_memory_doc(&skill, "alice", tenant_project);

        assert_eq!(doc.project_id, tenant_project);
        assert_eq!(doc.user_id, "alice");
    }

    #[test]
    fn workspace_skill_goes_to_tenant_project() {
        let skill = make_skill(
            "ws-skill",
            "Workspace prompt",
            SkillSource::Workspace(PathBuf::from("/workspace/skills/ws-skill")),
        );
        let tenant_project = ProjectId::new();
        let doc = v1_skill_to_memory_doc(&skill, "bob", tenant_project);

        assert_eq!(doc.project_id, tenant_project);
        assert_eq!(doc.user_id, "bob");
    }
}
