//! V2 engine skill types.
//!
//! These types extend the v1 skill model with capabilities needed by the v2
//! engine: executable code snippets, usage/confidence metrics, and versioning.
//! They are serialized into `MemoryDoc.metadata` JSON in the engine crate.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::types::{ActivationCriteria, SkillTrust};

/// How a v2 skill was created.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum V2SkillSource {
    /// User-authored SKILL.md (migrated from v1 or hand-written).
    #[default]
    Authored,
    /// Auto-extracted by the skill-extraction learning mission.
    Extracted,
    /// One-time v1 → v2 migration.
    Migrated,
}

/// A Python code snippet carried by a v2 skill.
///
/// Registered as a callable function in the CodeAct/Monty runtime so the LLM
/// can call it directly without reconstructing the logic from scratch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeSnippet {
    /// Function name (e.g., "fetch_issues"). Must be a valid Python identifier.
    pub name: String,
    /// Python function body (e.g., `def fetch_issues(owner, repo): ...`).
    pub code: String,
    /// Short description for the LLM context / docstring.
    #[serde(default)]
    pub description: String,
}

/// Usage and confidence metrics for auto-extracted skills.
///
/// Tracks how often a skill is used and whether it contributes to successful
/// thread outcomes. Skills with low confidence get demoted in scoring.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillMetrics {
    /// Total number of times this skill was activated in a thread.
    #[serde(default)]
    pub usage_count: u64,
    /// Number of times the skill was active in a successfully completed thread.
    #[serde(default)]
    pub success_count: u64,
    /// Number of times the skill was active in a failed thread.
    #[serde(default)]
    pub failure_count: u64,
    /// When this skill was last activated.
    #[serde(default)]
    pub last_used: Option<DateTime<Utc>>,
}

impl SkillMetrics {
    /// Compute confidence as success ratio.
    ///
    /// Returns 1.0 if there are no recorded outcomes (benefit of the doubt).
    pub fn confidence(&self) -> f64 {
        let total = self.success_count + self.failure_count;
        if total == 0 {
            return 1.0;
        }
        self.success_count as f64 / total as f64
    }
}

/// Full metadata for a v2 skill.
///
/// Serialized to/from the `metadata` JSON field of a `MemoryDoc` with
/// `DocType::Skill`. All fields use `#[serde(default)]` for forward
/// compatibility — old skills missing new fields deserialize gracefully.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct V2SkillMetadata {
    /// Skill name (matches the MemoryDoc title minus the "skill:" prefix).
    #[serde(default)]
    pub name: String,
    /// Skill version (incremented by extraction/update missions).
    #[serde(default = "default_version")]
    pub version: u32,
    /// Short description.
    #[serde(default)]
    pub description: String,
    /// Activation criteria for deterministic selection.
    #[serde(default)]
    pub activation: ActivationCriteria,
    /// How this skill was created.
    #[serde(default)]
    pub source: V2SkillSource,
    /// Trust level.
    #[serde(default = "default_trust")]
    pub trust: SkillTrust,
    /// Executable Python code snippets for CodeAct injection.
    #[serde(default)]
    pub code_snippets: Vec<CodeSnippet>,
    /// Usage and confidence metrics.
    #[serde(default)]
    pub metrics: SkillMetrics,
    /// Previous version number (for rollback).
    #[serde(default)]
    pub parent_version: Option<u32>,
    /// SHA-256 hash of the prompt content.
    #[serde(default)]
    pub content_hash: String,
}

fn default_version() -> u32 {
    1
}

fn default_trust() -> SkillTrust {
    SkillTrust::Trusted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_confidence_no_data() {
        let m = SkillMetrics::default();
        assert!((m.confidence() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_confidence_all_success() {
        let m = SkillMetrics {
            success_count: 10,
            failure_count: 0,
            ..Default::default()
        };
        assert!((m.confidence() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_confidence_mixed() {
        let m = SkillMetrics {
            success_count: 3,
            failure_count: 7,
            ..Default::default()
        };
        assert!((m.confidence() - 0.3).abs() < f64::EPSILON);
    }

    #[test]
    fn test_confidence_all_failure() {
        let m = SkillMetrics {
            success_count: 0,
            failure_count: 5,
            ..Default::default()
        };
        assert!((m.confidence() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_v2_metadata_serde_roundtrip() {
        let meta = V2SkillMetadata {
            name: "test-skill".to_string(),
            version: 3,
            description: "A test".to_string(),
            activation: ActivationCriteria {
                keywords: vec!["test".to_string()],
                ..Default::default()
            },
            source: V2SkillSource::Extracted,
            trust: SkillTrust::Trusted,
            code_snippets: vec![CodeSnippet {
                name: "do_thing".to_string(),
                code: "def do_thing(): pass".to_string(),
                description: "Does a thing".to_string(),
            }],
            metrics: SkillMetrics {
                usage_count: 5,
                success_count: 4,
                failure_count: 1,
                last_used: None,
            },
            parent_version: Some(2),
            content_hash: "sha256:abc".to_string(),
        };

        let json = serde_json::to_string(&meta).expect("serialize");
        let parsed: V2SkillMetadata = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(parsed.name, "test-skill");
        assert_eq!(parsed.version, 3);
        assert_eq!(parsed.source, V2SkillSource::Extracted);
        assert_eq!(parsed.code_snippets.len(), 1);
        assert_eq!(parsed.metrics.success_count, 4);
        assert_eq!(parsed.parent_version, Some(2));
    }

    #[test]
    fn test_v2_metadata_default_fields() {
        // Deserializing an empty JSON object should produce valid defaults
        let parsed: V2SkillMetadata = serde_json::from_str("{}").expect("deserialize empty");
        assert_eq!(parsed.name, "");
        assert_eq!(parsed.version, 1);
        assert_eq!(parsed.source, V2SkillSource::Authored);
        assert_eq!(parsed.trust, SkillTrust::Trusted);
        assert!(parsed.code_snippets.is_empty());
        assert!((parsed.metrics.confidence() - 1.0).abs() < f64::EPSILON);
    }
}
