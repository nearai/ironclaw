//! Deterministic skill prefilter for two-phase selection.
//!
//! The first phase of skill selection is entirely deterministic -- no LLM involvement,
//! no skill content in context. This prevents circular manipulation where a loaded
//! skill could influence which skills get loaded.
//!
//! Scoring:
//! - Keyword exact match: 10 points (capped at 30 total)
//! - Keyword substring match: 5 points (capped at 30 total)
//! - Tag match: 3 points (capped at 15 total)
//! - Regex pattern match: 20 points (capped at 40 total)

use crate::skills::LoadedSkill;

/// Default maximum context tokens allocated to skills.
pub const MAX_SKILL_CONTEXT_TOKENS: usize = 4000;

/// Maximum keyword score cap per skill to prevent gaming via keyword stuffing.
/// Even if a skill has 20 keywords, it can earn at most this many keyword points.
const MAX_KEYWORD_SCORE: u32 = 30;

/// Maximum tag score cap per skill (parallel to keyword cap).
const MAX_TAG_SCORE: u32 = 15;

/// Maximum regex pattern score cap per skill. Without a cap, 5 patterns at
/// 20 points each could yield 100 points, dominating keyword+tag scores.
const MAX_REGEX_SCORE: u32 = 40;

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
        let declared_tokens = entry.skill.manifest.activation.max_context_tokens;
        // Rough token estimate: ~0.75 tokens per byte for English prose
        let approx_tokens = (entry.skill.prompt_content.len() as f64 * 0.75) as usize;
        let raw_cost = if approx_tokens > declared_tokens * 2 {
            tracing::warn!(
                "Skill '{}' declares max_context_tokens={} but prompt is ~{} tokens; using actual estimate",
                entry.skill.name(),
                declared_tokens,
                approx_tokens,
            );
            approx_tokens
        } else {
            declared_tokens
        };
        // Enforce a minimum token cost so max_context_tokens=0 can't bypass budgeting
        let token_cost = raw_cost.max(1);
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

    // Keyword scoring with cap to prevent gaming via keyword stuffing
    let mut keyword_score: u32 = 0;
    for keyword in &criteria.keywords {
        let kw_lower = keyword.to_lowercase();
        // Exact word match (surrounded by word boundaries)
        if message_lower
            .split_whitespace()
            .any(|word| word.trim_matches(|c: char| !c.is_alphanumeric()) == kw_lower)
        {
            keyword_score += 10;
        } else if message_lower.contains(&kw_lower) {
            // Substring match
            keyword_score += 5;
        }
    }
    score += keyword_score.min(MAX_KEYWORD_SCORE);

    // Tag scoring (from manifest.skill.tags merged with activation.tags), with cap
    let all_tags: Vec<&str> = criteria
        .tags
        .iter()
        .chain(skill.manifest.skill.tags.iter())
        .map(|s| s.as_str())
        .collect();

    let mut tag_score: u32 = 0;
    for tag in &all_tags {
        let tag_lower = tag.to_lowercase();
        if message_lower.contains(&tag_lower) {
            tag_score += 3;
        }
    }
    score += tag_score.min(MAX_TAG_SCORE);

    // Regex pattern scoring using pre-compiled patterns (cached at load time), with cap
    let mut regex_score: u32 = 0;
    for re in &skill.compiled_patterns {
        if re.is_match(message_original) {
            regex_score += 20;
        }
    }
    score += regex_score.min(MAX_REGEX_SCORE);

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
        let pattern_strings: Vec<String> = patterns.iter().map(|s| s.to_string()).collect();
        let compiled = LoadedSkill::compile_patterns(&pattern_strings);
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
                    patterns: pattern_strings,
                    tags: vec![],
                    max_context_tokens: 1000,
                },
                permissions: Default::default(),
                integrity: IntegrityInfo::default(),
                http: None,
            },
            prompt_content: "Test prompt".to_string(),
            trust: SkillTrust::Local,
            source: SkillSource::Local(PathBuf::from("/tmp/test")),
            content_hash: "sha256:000".to_string(),
            scan_warnings: vec![],
            compiled_patterns: compiled,
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
        // Invalid patterns won't compile, so compiled_patterns will be empty.
        // But keyword still matches, so the skill is still selected.
        let skills = vec![make_skill("bad", &["test"], &[], &["[invalid regex"])];
        let result = prefilter_skills("test", &skills, 3, MAX_SKILL_CONTEXT_TOKENS);
        assert_eq!(result.len(), 1); // Still matches on keyword
    }

    #[test]
    fn test_keyword_score_capped() {
        // A skill with many keywords should not get an unbounded score
        let many_keywords: Vec<&str> = vec![
            "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p",
        ];
        let skill = make_skill("spammer", &many_keywords, &[], &[]);
        // Message that matches all keywords
        let skills = vec![skill];
        let result = prefilter_skills(
            "a b c d e f g h i j k l m n o p",
            &skills,
            3,
            MAX_SKILL_CONTEXT_TOKENS,
        );
        assert_eq!(result.len(), 1);
        // Score should be capped at MAX_KEYWORD_SCORE (30), not 16 * 10 = 160
    }

    #[test]
    fn test_tag_score_capped() {
        // A skill with many tags should not get an unbounded score
        let many_tags: Vec<&str> = vec![
            "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel",
        ];
        let skill = make_skill("tag-spammer", &[], &many_tags, &[]);
        let skills = vec![skill];
        let result = prefilter_skills(
            "alpha bravo charlie delta echo foxtrot golf hotel",
            &skills,
            3,
            MAX_SKILL_CONTEXT_TOKENS,
        );
        assert_eq!(result.len(), 1);
        // Tag score should be capped at MAX_TAG_SCORE (15), not 8 * 3 = 24
    }

    #[test]
    fn test_regex_score_capped() {
        // A skill with many regex patterns should have its score capped
        let skill = make_skill(
            "regex-spammer",
            &[],
            &[],
            &[
                r"(?i)\bwrite\b",
                r"(?i)\bdraft\b",
                r"(?i)\bedit\b",
                r"(?i)\bcompose\b",
                r"(?i)\bauthor\b",
            ],
        );
        let skills = vec![skill];
        let result = prefilter_skills(
            "write draft edit compose author",
            &skills,
            3,
            MAX_SKILL_CONTEXT_TOKENS,
        );
        assert_eq!(result.len(), 1);
        // Regex score should be capped at MAX_REGEX_SCORE (40), not 5 * 20 = 100
    }

    #[test]
    fn test_zero_context_tokens_still_costs_budget() {
        // Edge case: empty prompt + max_context_tokens=0 should still cost
        // at least 1 token so it can't bypass budget limits entirely.
        let mut skill = make_skill("free", &["test"], &[], &[]);
        skill.manifest.activation.max_context_tokens = 0;
        skill.prompt_content = String::new();
        let mut skill2 = make_skill("also_free", &["test"], &[], &[]);
        skill2.manifest.activation.max_context_tokens = 0;
        skill2.prompt_content = String::new();

        let skills = vec![skill, skill2];
        // Budget of 1 should only fit one skill (each costs at least 1)
        let result = prefilter_skills("test", &skills, 5, 1);
        assert_eq!(result.len(), 1);
    }
}
