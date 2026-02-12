//! Deterministic skill prefilter for two-phase selection.
//!
//! The first phase of skill selection is entirely deterministic -- no LLM involvement,
//! no skill content in context. This prevents circular manipulation where a loaded
//! skill could influence which skills get loaded.
//!
//! Scoring:
//! - Keyword exact match: 10 points
//! - Keyword substring match: 5 points
//! - Tag match: 3 points
//! - Regex pattern match: 20 points

use regex::Regex;

use crate::skills::LoadedSkill;

/// Default maximum context tokens allocated to skills.
pub const MAX_SKILL_CONTEXT_TOKENS: usize = 4000;

/// Result of prefiltering with score information.
#[derive(Debug)]
pub struct ScoredSkill<'a> {
    pub skill: &'a LoadedSkill,
    pub score: u32,
}

/// Select candidate skills for a given message using deterministic scoring.
///
/// Returns skills sorted by score (highest first), limited by `max_candidates`
/// and total context budget. No LLM is involved in this selection.
pub fn prefilter_skills<'a>(
    message: &str,
    available_skills: &'a [LoadedSkill],
    max_candidates: usize,
    max_context_tokens: usize,
) -> Vec<&'a LoadedSkill> {
    if available_skills.is_empty() || message.is_empty() {
        return vec![];
    }

    let message_lower = message.to_lowercase();

    let mut scored: Vec<ScoredSkill<'a>> = available_skills
        .iter()
        .filter_map(|skill| {
            let score = score_skill(skill, &message_lower, message);
            if score > 0 {
                Some(ScoredSkill { skill, score })
            } else {
                None
            }
        })
        .collect();

    // Sort by score descending
    scored.sort_by(|a, b| b.score.cmp(&a.score));

    // Apply candidate limit and context budget
    let mut result = Vec::new();
    let mut budget_remaining = max_context_tokens;

    for entry in scored {
        if result.len() >= max_candidates {
            break;
        }
        let token_cost = entry.skill.manifest.activation.max_context_tokens;
        if token_cost <= budget_remaining {
            budget_remaining -= token_cost;
            result.push(entry.skill);
        }
    }

    result
}

/// Score a skill against a user message.
fn score_skill(skill: &LoadedSkill, message_lower: &str, message_original: &str) -> u32 {
    let mut score: u32 = 0;
    let criteria = &skill.manifest.activation;

    // Keyword scoring
    for keyword in &criteria.keywords {
        let kw_lower = keyword.to_lowercase();
        // Exact word match (surrounded by word boundaries)
        if message_lower
            .split_whitespace()
            .any(|word| word.trim_matches(|c: char| !c.is_alphanumeric()) == kw_lower)
        {
            score += 10;
        } else if message_lower.contains(&kw_lower) {
            // Substring match
            score += 5;
        }
    }

    // Tag scoring (from manifest.skill.tags merged with activation.tags)
    let all_tags: Vec<&str> = criteria
        .tags
        .iter()
        .chain(skill.manifest.skill.tags.iter())
        .map(|s| s.as_str())
        .collect();

    for tag in &all_tags {
        let tag_lower = tag.to_lowercase();
        if message_lower.contains(&tag_lower) {
            score += 3;
        }
    }

    // Regex pattern scoring
    for pattern_str in &criteria.patterns {
        match Regex::new(pattern_str) {
            Ok(re) => {
                if re.is_match(message_original) {
                    score += 20;
                }
            }
            Err(e) => {
                tracing::warn!(
                    "Invalid regex pattern in skill '{}': {} ({})",
                    skill.name(),
                    pattern_str,
                    e
                );
            }
        }
    }

    score
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::{
        ActivationCriteria, IntegrityInfo, LoadedSkill, SkillManifest, SkillMeta, SkillSource,
        SkillTrust,
    };
    use std::path::PathBuf;

    fn make_skill(name: &str, keywords: &[&str], tags: &[&str], patterns: &[&str]) -> LoadedSkill {
        LoadedSkill {
            manifest: SkillManifest {
                skill: SkillMeta {
                    name: name.to_string(),
                    version: "1.0.0".to_string(),
                    description: format!("{} skill", name),
                    author: "test".to_string(),
                    tags: tags.iter().map(|s| s.to_string()).collect(),
                },
                activation: ActivationCriteria {
                    keywords: keywords.iter().map(|s| s.to_string()).collect(),
                    patterns: patterns.iter().map(|s| s.to_string()).collect(),
                    tags: vec![],
                    max_context_tokens: 1000,
                },
                permissions: Default::default(),
                integrity: IntegrityInfo::default(),
            },
            prompt_content: "Test prompt".to_string(),
            trust: SkillTrust::Local,
            source: SkillSource::Local(PathBuf::from("/tmp/test")),
            content_hash: "sha256:000".to_string(),
            scan_warnings: vec![],
        }
    }

    #[test]
    fn test_empty_message_returns_nothing() {
        let skills = vec![make_skill("test", &["write"], &[], &[])];
        let result = prefilter_skills("", &skills, 3, MAX_SKILL_CONTEXT_TOKENS);
        assert!(result.is_empty());
    }

    #[test]
    fn test_no_matching_skills() {
        let skills = vec![make_skill("cooking", &["recipe", "cook", "bake"], &[], &[])];
        let result = prefilter_skills(
            "Help me write an email",
            &skills,
            3,
            MAX_SKILL_CONTEXT_TOKENS,
        );
        assert!(result.is_empty());
    }

    #[test]
    fn test_keyword_exact_match() {
        let skills = vec![make_skill("writing", &["write", "edit"], &[], &[])];
        let result = prefilter_skills(
            "Please write an email",
            &skills,
            3,
            MAX_SKILL_CONTEXT_TOKENS,
        );
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name(), "writing");
    }

    #[test]
    fn test_keyword_substring_match() {
        let skills = vec![make_skill("writing", &["writing"], &[], &[])];
        let result = prefilter_skills(
            "I need help with rewriting this text",
            &skills,
            3,
            MAX_SKILL_CONTEXT_TOKENS,
        );
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_tag_match() {
        let skills = vec![make_skill("writing", &[], &["prose", "email"], &[])];
        let result = prefilter_skills(
            "Draft an email for me",
            &skills,
            3,
            MAX_SKILL_CONTEXT_TOKENS,
        );
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_regex_pattern_match() {
        let skills = vec![make_skill(
            "writing",
            &[],
            &[],
            &[r"(?i)\b(write|draft)\b.*\b(email|letter)\b"],
        )];
        let result = prefilter_skills(
            "Please draft an email to my boss",
            &skills,
            3,
            MAX_SKILL_CONTEXT_TOKENS,
        );
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_scoring_priority() {
        let skills = vec![
            make_skill("cooking", &["cook"], &[], &[]),
            make_skill(
                "writing",
                &["write", "draft"],
                &["email"],
                &[r"(?i)\b(write|draft)\b.*\bemail\b"],
            ),
        ];
        let result = prefilter_skills(
            "Write and draft an email",
            &skills,
            3,
            MAX_SKILL_CONTEXT_TOKENS,
        );
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name(), "writing");
    }

    #[test]
    fn test_max_candidates_limit() {
        let skills = vec![
            make_skill("a", &["test"], &[], &[]),
            make_skill("b", &["test"], &[], &[]),
            make_skill("c", &["test"], &[], &[]),
        ];
        let result = prefilter_skills("test", &skills, 2, MAX_SKILL_CONTEXT_TOKENS);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_context_budget_limit() {
        let mut skill = make_skill("big", &["test"], &[], &[]);
        skill.manifest.activation.max_context_tokens = 3000;
        let mut skill2 = make_skill("also_big", &["test"], &[], &[]);
        skill2.manifest.activation.max_context_tokens = 3000;

        let skills = vec![skill, skill2];
        // Budget of 4000 can only fit one 3000-token skill
        let result = prefilter_skills("test", &skills, 5, 4000);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_invalid_regex_handled_gracefully() {
        let skills = vec![make_skill("bad", &["test"], &[], &["[invalid regex"])];
        // Should not panic, just log a warning
        let result = prefilter_skills("test", &skills, 3, MAX_SKILL_CONTEXT_TOKENS);
        assert_eq!(result.len(), 1); // Still matches on keyword
    }
}
