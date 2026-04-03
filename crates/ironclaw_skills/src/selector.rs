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

use crate::types::LoadedSkill;

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
    scored.sort_by_key(|b| std::cmp::Reverse(b.score));

    // Apply candidate limit and context budget
    let mut result = Vec::new();
    let mut budget_remaining = max_context_tokens;

    for entry in scored {
        if result.len() >= max_candidates {
            break;
        }
        let declared_tokens = entry.skill.manifest.activation.max_context_tokens;
        // Rough token estimate: ~0.25 tokens per byte (~4 bytes per token for English prose)
        let approx_tokens = (entry.skill.prompt_content.len() as f64 * 0.25) as usize;
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
    // Exclusion veto: if any exclude_keyword is present in the message, score 0
    if skill
        .lowercased_exclude_keywords
        .iter()
        .any(|excl| message_lower.contains(excl.as_str()))
    {
        return 0;
    }

    let mut score: u32 = 0;

    // Keyword scoring with cap to prevent gaming via keyword stuffing
    let mut keyword_score: u32 = 0;
    for kw_lower in &skill.lowercased_keywords {
        // Exact word match (surrounded by word boundaries)
        if message_lower
            .split_whitespace()
            .any(|word| word.trim_matches(|c: char| !c.is_alphanumeric()) == kw_lower.as_str())
        {
            keyword_score += 10;
        } else if message_lower.contains(kw_lower.as_str()) {
            // Substring match
            keyword_score += 5;
        }
    }
    score += keyword_score.min(MAX_KEYWORD_SCORE);

    // Tag scoring from activation.tags
    let mut tag_score: u32 = 0;
    for tag_lower in &skill.lowercased_tags {
        if message_lower.contains(tag_lower.as_str()) {
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

/// Extract explicit `/skill-name` mentions from a message.
///
/// Users can write `/github` or `/file-issues` anywhere in their message to
/// force-activate a skill. Returns the matched skills and a rewritten message
/// where each `/skill-name` is replaced with the skill's description (so the
/// sentence still reads naturally for the LLM).
///
/// Example: `"fetch issues from /github"` with a skill named `github`
/// (description "GitHub API") → rewritten to `"fetch issues from GitHub API"`,
/// and the github skill is force-included.
pub fn extract_skill_mentions<'a>(
    message: &str,
    available_skills: &'a [LoadedSkill],
) -> (Vec<&'a LoadedSkill>, String) {
    let mut matched = Vec::new();
    let mut rewritten = message.to_string();

    // Build a name→skill lookup (case-insensitive)
    let skill_map: std::collections::HashMap<String, &'a LoadedSkill> = available_skills
        .iter()
        .map(|s| (s.manifest.name.to_lowercase(), s))
        .collect();

    // Find /word patterns that match skill names. Scan from end to avoid
    // index shifts when replacing.
    let mut replacements: Vec<(usize, usize, String)> = Vec::new();
    let bytes = message.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'/' {
            // Check that / is at start or preceded by whitespace/punctuation
            let is_boundary = i == 0
                || bytes[i - 1] == b' '
                || bytes[i - 1] == b'\n'
                || bytes[i - 1] == b'\t'
                || bytes[i - 1] == b'"'
                || bytes[i - 1] == b'(';

            if is_boundary {
                // Extract the name using the same character class accepted by
                // skill validation: [a-zA-Z0-9._-]+
                let start = i + 1;
                let mut end = start;
                while end < bytes.len()
                    && (bytes[end].is_ascii_lowercase()
                        || bytes[end].is_ascii_uppercase()
                        || bytes[end].is_ascii_digit()
                        || bytes[end] == b'-'
                        || bytes[end] == b'_'
                        || bytes[end] == b'.')
                {
                    end += 1;
                }
                if end > start {
                    let name = &message[start..end];
                    let lookup = name.to_lowercase();
                    if let Some(skill) = skill_map.get(&lookup) {
                        let replacement = if skill.manifest.description.is_empty() {
                            // No description — just remove the slash
                            name.replace('-', " ")
                        } else {
                            skill.manifest.description.clone()
                        };
                        replacements.push((i, end, replacement));
                        if !matched
                            .iter()
                            .any(|s: &&LoadedSkill| s.manifest.name == skill.manifest.name)
                        {
                            matched.push(*skill);
                        }
                    }
                }
            }
        }
        i += 1;
    }

    // Apply replacements in reverse order to preserve indices
    for (start, end, replacement) in replacements.into_iter().rev() {
        rewritten.replace_range(start..end, &replacement);
    }

    (matched, rewritten)
}

/// Apply confidence factor to a base score.
///
/// Authored skills always get factor 1.0 (no adjustment).
/// Extracted skills get `0.5 + 0.5 * confidence`, so a skill with 0% confidence
/// gets its score halved (not zeroed — it can still be selected when strongly
/// keyword-matched).
pub fn apply_confidence_factor(base_score: u32, confidence: f64, is_authored: bool) -> u32 {
    if is_authored {
        return base_score;
    }
    let factor = 0.5 + 0.5 * confidence.clamp(0.0, 1.0);
    (base_score as f64 * factor) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ActivationCriteria, LoadedSkill, SkillManifest, SkillSource, SkillTrust};
    use std::path::PathBuf;

    fn make_skill(name: &str, keywords: &[&str], tags: &[&str], patterns: &[&str]) -> LoadedSkill {
        let pattern_strings: Vec<String> = patterns.iter().map(|s| s.to_string()).collect();
        let compiled = LoadedSkill::compile_patterns(&pattern_strings);
        let kw_vec: Vec<String> = keywords.iter().map(|s| s.to_string()).collect();
        let tag_vec: Vec<String> = tags.iter().map(|s| s.to_string()).collect();
        let lowercased_keywords = kw_vec.iter().map(|k| k.to_lowercase()).collect();
        let lowercased_tags = tag_vec.iter().map(|t| t.to_lowercase()).collect();
        LoadedSkill {
            manifest: SkillManifest {
                name: name.to_string(),
                version: "1.0.0".to_string(),
                description: format!("{} skill", name),
                activation: ActivationCriteria {
                    keywords: kw_vec,
                    exclude_keywords: vec![],
                    patterns: pattern_strings,
                    tags: tag_vec,
                    max_context_tokens: 1000,
                },
                credentials: vec![],
                metadata: None,
            },
            prompt_content: "Test prompt".to_string(),
            trust: SkillTrust::Trusted,
            source: SkillSource::User(PathBuf::from("/tmp/test")),
            content_hash: "sha256:000".to_string(),
            compiled_patterns: compiled,
            lowercased_keywords,
            lowercased_exclude_keywords: vec![],
            lowercased_tags,
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
        let result = prefilter_skills("test", &skills, 5, 4000);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_invalid_regex_handled_gracefully() {
        let skills = vec![make_skill("bad", &["test"], &[], &["[invalid regex"])];
        let result = prefilter_skills("test", &skills, 3, MAX_SKILL_CONTEXT_TOKENS);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_keyword_score_capped() {
        let many_keywords: Vec<&str> = vec![
            "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p",
        ];
        let skill = make_skill("spammer", &many_keywords, &[], &[]);
        let skills = vec![skill];
        let result = prefilter_skills(
            "a b c d e f g h i j k l m n o p",
            &skills,
            3,
            MAX_SKILL_CONTEXT_TOKENS,
        );
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_tag_score_capped() {
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
    }

    #[test]
    fn test_regex_score_capped() {
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
    }

    #[test]
    fn test_zero_context_tokens_still_costs_budget() {
        let mut skill = make_skill("free", &["test"], &[], &[]);
        skill.manifest.activation.max_context_tokens = 0;
        skill.prompt_content = String::new();
        let mut skill2 = make_skill("also_free", &["test"], &[], &[]);
        skill2.manifest.activation.max_context_tokens = 0;
        skill2.prompt_content = String::new();

        let skills = vec![skill, skill2];
        let result = prefilter_skills("test", &skills, 5, 1);
        assert_eq!(result.len(), 1);
    }

    fn make_skill_with_excludes(
        name: &str,
        keywords: &[&str],
        exclude_keywords: &[&str],
        tags: &[&str],
        patterns: &[&str],
    ) -> LoadedSkill {
        let mut skill = make_skill(name, keywords, tags, patterns);
        let excl_vec: Vec<String> = exclude_keywords.iter().map(|s| s.to_string()).collect();
        skill.lowercased_exclude_keywords = excl_vec.iter().map(|k| k.to_lowercase()).collect();
        skill.manifest.activation.exclude_keywords = excl_vec;
        skill
    }

    #[test]
    fn test_exclude_keyword_vetos_match() {
        let skills = vec![make_skill_with_excludes(
            "writer",
            &["write"],
            &["route"],
            &[],
            &[],
        )];
        let result = prefilter_skills(
            "route this write request to another agent",
            &skills,
            3,
            MAX_SKILL_CONTEXT_TOKENS,
        );
        assert!(
            result.is_empty(),
            "skill with matching exclude_keyword should score 0"
        );
    }

    #[test]
    fn test_exclude_keyword_absent_does_not_block() {
        let skills = vec![make_skill_with_excludes(
            "writer",
            &["write"],
            &["route"],
            &[],
            &[],
        )];
        let result = prefilter_skills(
            "help me write an email",
            &skills,
            3,
            MAX_SKILL_CONTEXT_TOKENS,
        );
        assert_eq!(
            result.len(),
            1,
            "skill should activate when no exclude_keyword is present"
        );
    }

    #[test]
    fn test_exclude_keyword_veto_wins_over_positive_match() {
        let skills = vec![make_skill_with_excludes(
            "writer",
            &["write", "draft", "compose"],
            &["redirect"],
            &[],
            &[],
        )];
        let result = prefilter_skills(
            "write and draft and compose — but redirect this somewhere else",
            &skills,
            3,
            MAX_SKILL_CONTEXT_TOKENS,
        );
        assert!(
            result.is_empty(),
            "exclude_keyword veto must win even when multiple positive keywords match"
        );
    }

    #[test]
    fn test_exclude_keyword_case_insensitive() {
        let skills = vec![make_skill_with_excludes(
            "writer",
            &["write"],
            &["Route"],
            &[],
            &[],
        )];
        let result = prefilter_skills(
            "please ROUTE this write request",
            &skills,
            3,
            MAX_SKILL_CONTEXT_TOKENS,
        );
        assert!(
            result.is_empty(),
            "exclude_keyword veto should be case-insensitive"
        );
    }

    #[test]
    fn test_apply_confidence_factor_authored() {
        assert_eq!(apply_confidence_factor(100, 0.0, true), 100);
        assert_eq!(apply_confidence_factor(100, 0.5, true), 100);
        assert_eq!(apply_confidence_factor(100, 1.0, true), 100);
    }

    #[test]
    fn test_apply_confidence_factor_extracted() {
        // 0% confidence → factor 0.5 → score halved
        assert_eq!(apply_confidence_factor(100, 0.0, false), 50);
        // 50% confidence → factor 0.75 → score * 0.75
        assert_eq!(apply_confidence_factor(100, 0.5, false), 75);
        // 100% confidence → factor 1.0 → unchanged
        assert_eq!(apply_confidence_factor(100, 1.0, false), 100);
    }

    #[test]
    fn test_apply_confidence_factor_clamps() {
        // Negative confidence clamped to 0
        assert_eq!(apply_confidence_factor(100, -0.5, false), 50);
        // Over 1.0 clamped to 1.0
        assert_eq!(apply_confidence_factor(100, 1.5, false), 100);
    }

    // ── extract_skill_mentions tests ──────────────────────────

    #[test]
    fn test_extract_no_mentions() {
        let skills = vec![make_skill("github", &["github"], &[], &[])];
        let (matched, rewritten) = extract_skill_mentions("fetch issues from github", &skills);
        assert!(matched.is_empty());
        assert_eq!(rewritten, "fetch issues from github");
    }

    #[test]
    fn test_extract_slash_mention() {
        let skills = vec![make_skill("github", &["github"], &[], &[])];
        let (matched, rewritten) = extract_skill_mentions("fetch issues from /github", &skills);
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].manifest.name, "github");
        assert_eq!(rewritten, "fetch issues from github skill");
    }

    #[test]
    fn test_extract_slash_mention_with_description() {
        let mut skill = make_skill("github", &["github"], &[], &[]);
        skill.manifest.description = "GitHub API".to_string();
        let skills = vec![skill];
        let (matched, rewritten) = extract_skill_mentions("fetch issues from /github", &skills);
        assert_eq!(matched.len(), 1);
        assert_eq!(rewritten, "fetch issues from GitHub API");
    }

    #[test]
    fn test_extract_hyphenated_skill_name() {
        let mut skill = make_skill("file-issues", &["file", "issues"], &[], &[]);
        skill.manifest.description = "file detailed GitHub issues".to_string();
        let skills = vec![skill];
        let (matched, rewritten) =
            extract_skill_mentions("please /file-issues for all found bugs", &skills);
        assert_eq!(matched.len(), 1);
        assert_eq!(
            rewritten,
            "please file detailed GitHub issues for all found bugs"
        );
    }

    #[test]
    fn test_extract_underscored_skill_name() {
        let mut skill = make_skill("my_skill", &["skill"], &[], &[]);
        skill.manifest.description = "custom workflow".to_string();
        let skills = vec![skill];
        let (matched, rewritten) = extract_skill_mentions("run /my_skill on this task", &skills);
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].manifest.name, "my_skill");
        assert_eq!(rewritten, "run custom workflow on this task");
    }

    #[test]
    fn test_extract_dotted_skill_name() {
        let mut skill = make_skill("skill.v2", &["skill"], &[], &[]);
        skill.manifest.description = "second generation skill".to_string();
        let skills = vec![skill];
        let (matched, rewritten) = extract_skill_mentions("please use /skill.v2 here", &skills);
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].manifest.name, "skill.v2");
        assert_eq!(rewritten, "please use second generation skill here");
    }

    #[test]
    fn test_extract_multiple_mentions() {
        let mut gh = make_skill("github", &["github"], &[], &[]);
        gh.manifest.description = "GitHub API".to_string();
        let mut linear = make_skill("linear", &["linear"], &[], &[]);
        linear.manifest.description = "Linear project management".to_string();
        let skills = vec![gh, linear];
        let (matched, rewritten) =
            extract_skill_mentions("sync /github issues to /linear", &skills);
        assert_eq!(matched.len(), 2);
        assert_eq!(
            rewritten,
            "sync GitHub API issues to Linear project management"
        );
    }

    #[test]
    fn test_extract_unknown_slash_not_replaced() {
        let skills = vec![make_skill("github", &["github"], &[], &[])];
        let (matched, rewritten) = extract_skill_mentions("run /unknown-thing now", &skills);
        assert!(matched.is_empty());
        assert_eq!(rewritten, "run /unknown-thing now");
    }

    #[test]
    fn test_extract_slash_at_start_of_message() {
        let mut skill = make_skill("github", &["github"], &[], &[]);
        skill.manifest.description = "GitHub API".to_string();
        let skills = vec![skill];
        let (matched, rewritten) = extract_skill_mentions("/github list my repos", &skills);
        assert_eq!(matched.len(), 1);
        assert_eq!(rewritten, "GitHub API list my repos");
    }

    #[test]
    fn test_extract_url_not_matched() {
        let skills = vec![make_skill("github", &["github"], &[], &[])];
        let (matched, rewritten) = extract_skill_mentions("open https://github.com/repo", &skills);
        // The /github.com won't match because '.' breaks the name pattern
        assert!(matched.is_empty());
        assert_eq!(rewritten, "open https://github.com/repo");
    }
}
