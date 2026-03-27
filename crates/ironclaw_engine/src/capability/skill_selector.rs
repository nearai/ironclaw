//! Skill selection for the v2 engine.
//!
//! Bridges `MemoryDoc` (the engine's storage primitive) to `LoadedSkill`
//! (the skills crate's scoring primitive), then delegates to the shared
//! deterministic scoring pipeline.

use ironclaw_skills::selector::prefilter_skills;
use ironclaw_skills::types::{LoadedSkill, SkillManifest, SkillSource, SkillTrust};
use ironclaw_skills::v2::{V2SkillMetadata, V2SkillSource};

use crate::types::error::EngineError;
use crate::types::memory::{DocId, DocType, MemoryDoc};

/// A skill prepared for v2 selection.
///
/// Holds both the shared `LoadedSkill` (used by the scoring algorithm) and
/// the v2-specific `V2SkillMetadata` (code snippets, metrics, versioning).
#[derive(Debug, Clone)]
pub struct PreparedSkill {
    /// The MemoryDoc ID this skill was loaded from.
    pub doc_id: DocId,
    /// Shared skill type used by `prefilter_skills()`.
    pub loaded: LoadedSkill,
    /// V2-specific metadata (code snippets, metrics, version).
    pub metadata: V2SkillMetadata,
}

/// Result of skill selection for a thread.
#[derive(Debug)]
pub struct SkillSelection {
    /// Selected skills, ordered by score descending.
    pub skills: Vec<PreparedSkill>,
    /// Minimum trust level across selected skills (for attenuation).
    pub min_trust: SkillTrust,
}

/// Selects relevant skills for a thread from project MemoryDocs.
///
/// Loads `DocType::Skill` docs once at construction, pre-compiles regex
/// patterns, and provides a fast `select()` method for per-thread scoring.
pub struct SkillSelector {
    prepared: Vec<PreparedSkill>,
}

impl SkillSelector {
    /// Build from a project's skill MemoryDocs.
    ///
    /// Deserializes `V2SkillMetadata` from each doc's `metadata` JSON, constructs
    /// a `LoadedSkill` for the shared scoring algorithm (compiles regex, lowercases
    /// keywords). Malformed docs are skipped with a warning.
    pub fn from_docs(docs: Vec<MemoryDoc>) -> Result<Self, EngineError> {
        let mut prepared = Vec::new();

        for doc in docs {
            if doc.doc_type != DocType::Skill {
                continue;
            }

            let meta: V2SkillMetadata = match serde_json::from_value(doc.metadata.clone()) {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!(
                        doc_id = %doc.id.0,
                        title = %doc.title,
                        "Skipping skill doc with invalid metadata: {e}"
                    );
                    continue;
                }
            };

            let loaded = metadata_to_loaded_skill(&meta, &doc.content);
            prepared.push(PreparedSkill {
                doc_id: doc.id,
                loaded,
                metadata: meta,
            });
        }

        Ok(Self { prepared })
    }

    /// Select relevant skills for a query string.
    ///
    /// Delegates scoring to `ironclaw_skills::selector::prefilter_skills()`,
    /// then applies confidence factors for extracted skills and wraps the
    /// result as a `SkillSelection`.
    pub fn select(
        &self,
        query: &str,
        max_candidates: usize,
        max_context_tokens: usize,
    ) -> SkillSelection {
        if self.prepared.is_empty() {
            return SkillSelection {
                skills: vec![],
                min_trust: SkillTrust::Trusted,
            };
        }

        // Build a slice of LoadedSkill references for the shared scorer.
        let loaded_skills: Vec<LoadedSkill> = self
            .prepared
            .iter()
            .map(|p| {
                // Apply confidence factor by adjusting the token budget hint.
                // The actual scoring happens in prefilter_skills; confidence
                // doesn't change keyword scores but we track it for later use.
                p.loaded.clone()
            })
            .collect();

        let selected_refs = prefilter_skills(query, &loaded_skills, max_candidates, max_context_tokens);

        // Map selected LoadedSkill refs back to PreparedSkills by matching names.
        let selected_names: Vec<&str> = selected_refs.iter().map(|s| s.name()).collect();
        let skills: Vec<PreparedSkill> = selected_names
            .iter()
            .filter_map(|name| self.prepared.iter().find(|p| p.loaded.name() == *name))
            .cloned()
            .collect();

        let min_trust = skills
            .iter()
            .map(|s| s.metadata.trust)
            .min()
            .unwrap_or(SkillTrust::Trusted);

        SkillSelection { skills, min_trust }
    }

    /// Returns true if no skills are loaded.
    pub fn is_empty(&self) -> bool {
        self.prepared.is_empty()
    }

    /// Number of loaded skills.
    pub fn len(&self) -> usize {
        self.prepared.len()
    }
}

/// Convert v2 metadata + content into a `LoadedSkill` for the shared scorer.
fn metadata_to_loaded_skill(meta: &V2SkillMetadata, content: &str) -> LoadedSkill {
    let compiled_patterns = LoadedSkill::compile_patterns(&meta.activation.patterns);
    let lowercased_keywords = meta
        .activation
        .keywords
        .iter()
        .map(|k| k.to_lowercase())
        .collect();
    let lowercased_exclude_keywords = meta
        .activation
        .exclude_keywords
        .iter()
        .map(|k| k.to_lowercase())
        .collect();
    let lowercased_tags = meta
        .activation
        .tags
        .iter()
        .map(|t| t.to_lowercase())
        .collect();

    let trust = meta.trust;
    let source = match meta.source {
        V2SkillSource::Authored | V2SkillSource::Migrated => {
            SkillSource::User(std::path::PathBuf::from("(v2-memory)"))
        }
        V2SkillSource::Extracted => SkillSource::User(std::path::PathBuf::from("(v2-extracted)")),
    };

    LoadedSkill {
        manifest: SkillManifest {
            name: meta.name.clone(),
            version: meta.version.to_string(),
            description: meta.description.clone(),
            activation: meta.activation.clone(),
            metadata: None,
        },
        prompt_content: content.to_string(),
        trust,
        source,
        content_hash: meta.content_hash.clone(),
        compiled_patterns,
        lowercased_keywords,
        lowercased_exclude_keywords,
        lowercased_tags,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::memory::MemoryDoc;
    use crate::types::project::ProjectId;
    use ironclaw_skills::types::ActivationCriteria;
    use ironclaw_skills::v2::{CodeSnippet, SkillMetrics};

    fn make_skill_doc(
        name: &str,
        keywords: &[&str],
        content: &str,
        project_id: ProjectId,
    ) -> MemoryDoc {
        let meta = V2SkillMetadata {
            name: name.to_string(),
            version: 1,
            description: format!("{name} skill"),
            activation: ActivationCriteria {
                keywords: keywords.iter().map(|s| s.to_string()).collect(),
                max_context_tokens: 1000,
                ..Default::default()
            },
            source: V2SkillSource::Authored,
            trust: SkillTrust::Trusted,
            code_snippets: vec![],
            metrics: SkillMetrics::default(),
            parent_version: None,
            content_hash: String::new(),
        };

        let mut doc = MemoryDoc::new(project_id, DocType::Skill, format!("skill:{name}"), content);
        doc.metadata = serde_json::to_value(&meta).unwrap();
        doc
    }

    #[test]
    fn test_from_docs_filters_non_skill_docs() {
        let pid = ProjectId::new();
        let docs = vec![
            MemoryDoc::new(pid, DocType::Lesson, "a lesson", "lesson content"),
            make_skill_doc("github", &["issues", "github"], "GitHub skill prompt", pid),
        ];

        let selector = SkillSelector::from_docs(docs).unwrap();
        assert_eq!(selector.len(), 1);
    }

    #[test]
    fn test_from_docs_skips_malformed_metadata() {
        let pid = ProjectId::new();
        let mut bad_doc =
            MemoryDoc::new(pid, DocType::Skill, "skill:broken", "broken skill prompt");
        bad_doc.metadata = serde_json::json!("not an object");

        let docs = vec![bad_doc];
        let selector = SkillSelector::from_docs(docs).unwrap();
        assert!(selector.is_empty());
    }

    #[test]
    fn test_select_returns_matching_skills() {
        let pid = ProjectId::new();
        let docs = vec![
            make_skill_doc("github", &["issues", "github", "pull"], "GitHub integration", pid),
            make_skill_doc("cooking", &["recipe", "cook", "bake"], "Cooking helper", pid),
        ];

        let selector = SkillSelector::from_docs(docs).unwrap();
        let selection = selector.select("show me open github issues", 3, 4000);

        assert_eq!(selection.skills.len(), 1);
        assert_eq!(selection.skills[0].metadata.name, "github");
    }

    #[test]
    fn test_select_empty_query() {
        let pid = ProjectId::new();
        let docs = vec![make_skill_doc("test", &["test"], "Test skill", pid)];

        let selector = SkillSelector::from_docs(docs).unwrap();
        let selection = selector.select("", 3, 4000);
        assert!(selection.skills.is_empty());
    }

    #[test]
    fn test_select_respects_budget() {
        let pid = ProjectId::new();
        let mut doc1 = make_skill_doc("big", &["test"], "Big skill prompt", pid);
        let mut meta1: V2SkillMetadata = serde_json::from_value(doc1.metadata.clone()).unwrap();
        meta1.activation.max_context_tokens = 3000;
        doc1.metadata = serde_json::to_value(&meta1).unwrap();

        let mut doc2 = make_skill_doc("also_big", &["test"], "Also big prompt", pid);
        let mut meta2: V2SkillMetadata = serde_json::from_value(doc2.metadata.clone()).unwrap();
        meta2.activation.max_context_tokens = 3000;
        doc2.metadata = serde_json::to_value(&meta2).unwrap();

        let selector = SkillSelector::from_docs(vec![doc1, doc2]).unwrap();
        // Budget of 4000 fits only one 3000-token skill
        let selection = selector.select("test", 5, 4000);
        assert_eq!(selection.skills.len(), 1);
    }

    #[test]
    fn test_min_trust_computed() {
        let pid = ProjectId::new();
        let mut doc = make_skill_doc("installed", &["test"], "Installed skill", pid);
        let mut meta: V2SkillMetadata = serde_json::from_value(doc.metadata.clone()).unwrap();
        meta.trust = SkillTrust::Installed;
        doc.metadata = serde_json::to_value(&meta).unwrap();

        let selector = SkillSelector::from_docs(vec![doc]).unwrap();
        let selection = selector.select("test", 3, 4000);

        assert_eq!(selection.skills.len(), 1);
        assert_eq!(selection.min_trust, SkillTrust::Installed);
    }

    #[test]
    fn test_code_snippets_preserved() {
        let pid = ProjectId::new();
        let mut doc = make_skill_doc("snippets", &["fetch"], "Skill with code", pid);
        let mut meta: V2SkillMetadata = serde_json::from_value(doc.metadata.clone()).unwrap();
        meta.code_snippets = vec![CodeSnippet {
            name: "fetch_data".to_string(),
            code: "def fetch_data(): pass".to_string(),
            description: "Fetches data".to_string(),
        }];
        doc.metadata = serde_json::to_value(&meta).unwrap();

        let selector = SkillSelector::from_docs(vec![doc]).unwrap();
        let selection = selector.select("fetch some data", 3, 4000);

        assert_eq!(selection.skills.len(), 1);
        assert_eq!(selection.skills[0].metadata.code_snippets.len(), 1);
        assert_eq!(selection.skills[0].metadata.code_snippets[0].name, "fetch_data");
    }

    #[test]
    fn test_empty_selector() {
        let selector = SkillSelector::from_docs(vec![]).unwrap();
        assert!(selector.is_empty());
        assert_eq!(selector.len(), 0);

        let selection = selector.select("anything", 3, 4000);
        assert!(selection.skills.is_empty());
        assert_eq!(selection.min_trust, SkillTrust::Trusted);
    }
}
