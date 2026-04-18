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

/// Estimate the token cost of loading a skill's prompt into the LLM
/// context. Prefers the declared `max_context_tokens` but falls back
/// to the actual length-based estimate (and warns) if the declaration
/// is implausibly low relative to the prompt content. Enforces a
/// minimum of 1 token so a `max_context_tokens: 0` declaration can't
/// bypass budgeting.
fn skill_token_cost(skill: &LoadedSkill) -> usize {
    let declared_tokens = skill.manifest.activation.max_context_tokens;
    // Rough token estimate: ~0.25 tokens per byte (~4 bytes per token for English prose)
    let approx_tokens = (skill.prompt_content.len() as f64 * 0.25) as usize;
    let raw_cost = if approx_tokens > declared_tokens * 2 {
        tracing::warn!(
            "Skill '{}' declares max_context_tokens={} but prompt is ~{} tokens; using actual estimate",
            skill.name(),
            declared_tokens,
            approx_tokens,
        );
        approx_tokens
    } else {
        declared_tokens
    };
    raw_cost.max(1)
}

/// Try to add a skill to the selected set. Returns `true` if the skill
/// was added, `false` if it was already present, excluded by marker,
/// over the candidate limit, or didn't fit in the remaining budget.
///
/// Shared between the scored-selection loop and the chain-loading loop.
fn try_select<'a>(
    skill: &'a LoadedSkill,
    result: &mut Vec<&'a LoadedSkill>,
    selected_names: &mut std::collections::HashSet<&'a str>,
    budget_remaining: &mut usize,
    max_candidates: usize,
    satisfied_setup_markers: &std::collections::HashSet<String>,
) -> bool {
    if result.len() >= max_candidates {
        return false;
    }
    let name = skill.manifest.name.as_str();
    if selected_names.contains(name) {
        return false;
    }
    // Respect marker exclusion even for chain-loaded companions: if a
    // companion's setup is already done, there's nothing for it to
    // contribute to the current turn.
    if let Some(marker) = &skill.manifest.activation.setup_marker
        && satisfied_setup_markers.contains(marker)
    {
        return false;
    }
    let cost = skill_token_cost(skill);
    if cost > *budget_remaining {
        return false;
    }
    *budget_remaining -= cost;
    selected_names.insert(name);
    result.push(skill);
    true
}

/// Select candidate skills for a given message using deterministic scoring.
///
/// Returns skills sorted by score (highest first), limited by `max_candidates`
/// and total context budget. No LLM is involved in this selection.
///
/// ## Chain-loading via `requires.skills`
///
/// When a skill is selected by score, its `requires.skills` companions
/// are also pulled in (if available), **bypassing the scoring filter** —
/// they ride on the parent's selection. This makes persona/bundle
/// skills like `developer-setup` work as designed: the orchestrator
/// declares which operational skills it delegates to, and selecting
/// the orchestrator automatically loads them. Chain-loading is
/// non-transitive (depth 1); a chain-loaded companion does not load
/// its own companions, to keep the behavior predictable.
///
/// Chain-loaded companions still consume from the same budget and
/// respect `max_candidates`. If the remaining budget can't fit a
/// companion, it is silently skipped with a debug log — the parent is
/// still selected. Companions with a satisfied `setup_marker` are
/// also skipped (their work is already done).
///
/// ## Setup-marker exclusion
///
/// `satisfied_setup_markers` is the set of workspace paths that already
/// exist for one-time setup skills. Any skill whose
/// `activation.setup_marker` is in this set is excluded from candidates
/// regardless of score — its setup has already been completed and there's
/// nothing for it to do. The caller (`agent_loop::select_active_skills`)
/// is responsible for computing this set by checking the workspace for
/// each distinct marker referenced by loaded skills.
///
/// Pass an empty set to disable marker filtering (the legacy behavior
/// where every skill competes regardless of workspace state).
pub fn prefilter_skills<'a>(
    message: &str,
    available_skills: &'a [LoadedSkill],
    max_candidates: usize,
    max_context_tokens: usize,
    satisfied_setup_markers: &std::collections::HashSet<String>,
) -> Vec<&'a LoadedSkill> {
    if available_skills.is_empty() || message.is_empty() {
        return vec![];
    }

    let message_lower = message.to_lowercase();

    // Build name → skill lookup for chain-loading companion resolution.
    let by_name: std::collections::HashMap<&str, &'a LoadedSkill> = available_skills
        .iter()
        .map(|s| (s.manifest.name.as_str(), s))
        .collect();

    let mut scored: Vec<ScoredSkill<'a>> = available_skills
        .iter()
        .filter_map(|skill| {
            // Setup-marker exclusion: a one-time setup skill whose
            // marker file already exists in the workspace has finished
            // its job. Skip scoring entirely so it can't burn budget.
            if let Some(marker) = &skill.manifest.activation.setup_marker
                && satisfied_setup_markers.contains(marker)
            {
                return None;
            }
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

    // Apply candidate limit and context budget.
    let mut result: Vec<&'a LoadedSkill> = Vec::new();
    let mut selected_names: std::collections::HashSet<&'a str> = std::collections::HashSet::new();
    let mut budget_remaining = max_context_tokens;

    for entry in scored {
        // Try to select the parent first.
        if !try_select(
            entry.skill,
            &mut result,
            &mut selected_names,
            &mut budget_remaining,
            max_candidates,
            satisfied_setup_markers,
        ) {
            // Parent didn't fit or was already selected — don't try to
            // chain-load companions for a parent that isn't in the set.
            continue;
        }

        // Chain-load companions declared in requires.skills.
        // Non-transitive: companions don't load their own companions.
        for companion_name in &entry.skill.manifest.requires.skills {
            let Some(companion) = by_name.get(companion_name.as_str()) else {
                // Listed but not loaded — ignore silently. Persona
                // bundles declare optional companions.
                continue;
            };
            if !try_select(
                companion,
                &mut result,
                &mut selected_names,
                &mut budget_remaining,
                max_candidates,
                satisfied_setup_markers,
            ) {
                tracing::debug!(
                    parent = %entry.skill.name(),
                    companion = %companion_name,
                    budget_remaining,
                    "chain-load skipped (already selected, budget full, or marker satisfied)"
                );
            }
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
    use crate::types::{
        ActivationCriteria, GatingRequirements, LoadedSkill, SkillManifest, SkillSource, SkillTrust,
    };
    use std::collections::HashSet;
    use std::path::PathBuf;

    /// Test wrapper around `prefilter_skills` that defaults the
    /// satisfied-marker set to empty (legacy behavior — no setup-marker
    /// filtering). Most existing tests don't care about marker
    /// semantics; the dedicated marker tests below construct their own
    /// HashSet.
    fn prefilter_no_markers<'a>(
        message: &str,
        available: &'a [LoadedSkill],
        max_candidates: usize,
        max_context_tokens: usize,
    ) -> Vec<&'a LoadedSkill> {
        super::prefilter_skills(
            message,
            available,
            max_candidates,
            max_context_tokens,
            &HashSet::new(),
        )
    }

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
                    setup_marker: None,
                },
                credentials: vec![],
                requires: GatingRequirements::default(),
            },
            prompt_content: "Test prompt".to_string(),
            trust: SkillTrust::Trusted,
            source: SkillSource::User(PathBuf::from("/tmp/test")), // safety: dummy path in test, not used for I/O
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
        let result = prefilter_no_markers("", &skills, 3, MAX_SKILL_CONTEXT_TOKENS);
        assert!(result.is_empty());
    }

    #[test]
    fn test_no_matching_skills() {
        let skills = vec![make_skill("cooking", &["recipe", "cook", "bake"], &[], &[])];
        let result = prefilter_no_markers(
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
        let result = prefilter_no_markers(
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
        let result = prefilter_no_markers(
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
        let result = prefilter_no_markers(
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
        let result = prefilter_no_markers(
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
        let result = prefilter_no_markers(
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
        let result = prefilter_no_markers("test", &skills, 2, MAX_SKILL_CONTEXT_TOKENS);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_context_budget_limit() {
        let mut skill = make_skill("big", &["test"], &[], &[]);
        skill.manifest.activation.max_context_tokens = 3000;
        let mut skill2 = make_skill("also_big", &["test"], &[], &[]);
        skill2.manifest.activation.max_context_tokens = 3000;

        let skills = vec![skill, skill2];
        let result = prefilter_no_markers("test", &skills, 5, 4000);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_invalid_regex_handled_gracefully() {
        let skills = vec![make_skill("bad", &["test"], &[], &["[invalid regex"])];
        let result = prefilter_no_markers("test", &skills, 3, MAX_SKILL_CONTEXT_TOKENS);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_keyword_score_capped() {
        let many_keywords: Vec<&str> = vec![
            "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p",
        ];
        let skill = make_skill("spammer", &many_keywords, &[], &[]);
        let skills = vec![skill];
        let result = prefilter_no_markers(
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
        let result = prefilter_no_markers(
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
        let result = prefilter_no_markers(
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
        let result = prefilter_no_markers("test", &skills, 5, 1);
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
        let result = prefilter_no_markers(
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
        let result = prefilter_no_markers(
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
        let result = prefilter_no_markers(
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
        let result = prefilter_no_markers(
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

    // ───────────────────────────────────────────────────────────────────
    // setup_marker filtering — one-time setup skills excluded after run
    // ───────────────────────────────────────────────────────────────────

    fn make_setup_skill(name: &str, marker: &str) -> LoadedSkill {
        let mut skill = make_skill(name, &["setup"], &[], &[]);
        skill.manifest.activation.setup_marker = Some(marker.to_string());
        skill
    }

    #[test]
    fn test_setup_marker_excludes_skill_when_marker_present() {
        let skills = vec![
            make_setup_skill("developer-setup", "commitments/README.md"),
            make_skill("github-workflow", &["workflow"], &[], &[]),
        ];
        let mut markers = HashSet::new();
        markers.insert("commitments/README.md".to_string());

        let result = prefilter_skills(
            "setup the workflow",
            &skills,
            5,
            MAX_SKILL_CONTEXT_TOKENS,
            &markers,
        );
        // developer-setup should be filtered out — its marker exists.
        // github-workflow has no marker so it's still selected.
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name(), "github-workflow");
    }

    #[test]
    fn test_setup_marker_includes_skill_when_marker_absent() {
        let skills = vec![
            make_setup_skill("developer-setup", "commitments/README.md"),
            make_skill("github-workflow", &["workflow"], &[], &[]),
        ];
        // Marker is NOT in the satisfied set — setup hasn't run yet.
        let result = prefilter_skills(
            "setup the workflow",
            &skills,
            5,
            MAX_SKILL_CONTEXT_TOKENS,
            &HashSet::new(),
        );
        // Both should be selected — both match keywords and neither
        // has a satisfied marker.
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_setup_marker_other_marker_does_not_exclude() {
        // Sanity check: a satisfied marker for a DIFFERENT path must not
        // exclude this skill. Marker matching is exact-string.
        let skills = vec![make_setup_skill("developer-setup", "commitments/README.md")];
        let mut markers = HashSet::new();
        markers.insert("projects/foo/project.md".to_string());
        markers.insert("commitments/calibration.md".to_string());

        let result = prefilter_skills("setup", &skills, 5, MAX_SKILL_CONTEXT_TOKENS, &markers);
        assert_eq!(result.len(), 1, "marker mismatch should not exclude");
    }

    #[test]
    fn test_setup_marker_skill_with_no_marker_unaffected() {
        // Skills WITHOUT a setup_marker must not be filtered regardless
        // of what's in the satisfied set.
        let skills = vec![make_skill("reactive", &["test"], &[], &[])];
        let mut markers = HashSet::new();
        markers.insert("anything".to_string());

        let result = prefilter_skills("test", &skills, 5, MAX_SKILL_CONTEXT_TOKENS, &markers);
        assert_eq!(result.len(), 1);
    }

    // ───────────────────────────────────────────────────────────────────
    // Chain-loading via requires.skills — companions ride on parent
    // selection, bypassing their own score filter.
    // ───────────────────────────────────────────────────────────────────

    fn make_skill_with_requires(name: &str, keywords: &[&str], required: &[&str]) -> LoadedSkill {
        let mut skill = make_skill(name, keywords, &[], &[]);
        skill.manifest.requires.skills = required.iter().map(|s| s.to_string()).collect();
        skill
    }

    #[test]
    fn test_chain_load_pulls_in_required_companions() {
        // Parent is scored normally; companions bypass scoring.
        // The companion has NO matching keywords — it would score 0
        // and be filtered out on its own. Chain-loading should still
        // bring it in because it's in the parent's requires.skills.
        let parent = make_skill_with_requires(
            "developer-setup",
            &["setup"],
            &["commitment-triage", "tech-debt-tracker"],
        );
        let companion1 = make_skill(
            "commitment-triage",
            &["unrelated-keyword-that-wont-match"],
            &[],
            &[],
        );
        let companion2 = make_skill("tech-debt-tracker", &["another-unrelated"], &[], &[]);
        let bystander = make_skill("unrelated-skill", &["nope"], &[], &[]);

        let skills = vec![parent, companion1, companion2, bystander];

        let result = prefilter_skills(
            "setup my dev workflow",
            &skills,
            10,
            MAX_SKILL_CONTEXT_TOKENS,
            &HashSet::new(),
        );

        let names: Vec<&str> = result.iter().map(|s| s.name()).collect();
        assert!(
            names.contains(&"developer-setup"),
            "parent must be selected (it scored), got: {names:?}"
        );
        assert!(
            names.contains(&"commitment-triage"),
            "companion must be chain-loaded even though it scored 0, got: {names:?}"
        );
        assert!(
            names.contains(&"tech-debt-tracker"),
            "second companion must also be chain-loaded, got: {names:?}"
        );
        assert!(
            !names.contains(&"unrelated-skill"),
            "unrelated skill must not be pulled in, got: {names:?}"
        );
    }

    #[test]
    fn test_chain_load_skipped_when_parent_not_selected() {
        // Parent doesn't match the message, so it's not scored. Its
        // companions should NOT be chain-loaded either.
        let parent = make_skill_with_requires(
            "developer-setup",
            &["dev-onboarding-keyword"],
            &["commitment-triage"],
        );
        let companion = make_skill("commitment-triage", &["random-kw"], &[], &[]);

        let skills = vec![parent, companion];

        let result = prefilter_skills(
            "completely unrelated message",
            &skills,
            10,
            MAX_SKILL_CONTEXT_TOKENS,
            &HashSet::new(),
        );

        assert!(
            result.is_empty(),
            "neither parent nor chain-loaded companion should activate \
             on an unrelated message; got: {:?}",
            result.iter().map(|s| s.name()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_chain_load_respects_budget() {
        // Parent (3000 tok) plus companion (3000 tok) exceeds a 4000
        // budget. Parent selected; companion skipped.
        let mut parent = make_skill_with_requires("big-setup", &["setup"], &["heavy-companion"]);
        parent.manifest.activation.max_context_tokens = 3000;
        let mut companion = make_skill("heavy-companion", &["x"], &[], &[]);
        companion.manifest.activation.max_context_tokens = 3000;

        let skills = vec![parent, companion];
        let result = prefilter_skills("setup", &skills, 10, 4000, &HashSet::new());
        let names: Vec<&str> = result.iter().map(|s| s.name()).collect();
        assert!(
            names.contains(&"big-setup"),
            "parent must still be selected"
        );
        assert!(
            !names.contains(&"heavy-companion"),
            "companion must be budget-skipped when it doesn't fit"
        );
    }

    #[test]
    fn test_chain_load_skips_companion_with_satisfied_marker() {
        // Companion has a setup_marker that's in the satisfied set.
        // Even though the parent requires it, chain-loading must
        // respect the marker exclusion — nothing for it to do.
        let parent = make_skill_with_requires("parent-setup", &["setup"], &["nested-setup"]);
        let mut companion = make_skill("nested-setup", &["nothing"], &[], &[]);
        companion.manifest.activation.setup_marker = Some("marker/already-done".to_string());

        let skills = vec![parent, companion];
        let mut markers = HashSet::new();
        markers.insert("marker/already-done".to_string());

        let result = prefilter_skills("setup", &skills, 10, MAX_SKILL_CONTEXT_TOKENS, &markers);
        let names: Vec<&str> = result.iter().map(|s| s.name()).collect();
        assert!(names.contains(&"parent-setup"), "parent must be selected");
        assert!(
            !names.contains(&"nested-setup"),
            "companion with satisfied marker must be skipped even via chain-load"
        );
    }

    #[test]
    fn test_chain_load_is_non_transitive() {
        // A -> B (B is in A's requires.skills)
        // B -> C (C is in B's requires.skills)
        // Selecting A should pull in B but NOT C. This keeps the
        // behavior predictable — bundles don't transitively explode.
        let a = make_skill_with_requires("top-setup", &["setup"], &["mid-companion"]);
        let b = make_skill_with_requires("mid-companion", &["mid"], &["deep-companion"]);
        let c = make_skill("deep-companion", &["deep"], &[], &[]);

        let skills = vec![a, b, c];
        let result = prefilter_skills(
            "setup",
            &skills,
            10,
            MAX_SKILL_CONTEXT_TOKENS,
            &HashSet::new(),
        );
        let names: Vec<&str> = result.iter().map(|s| s.name()).collect();
        assert!(names.contains(&"top-setup"));
        assert!(
            names.contains(&"mid-companion"),
            "direct companion (depth 1) must be chain-loaded, got: {names:?}"
        );
        assert!(
            !names.contains(&"deep-companion"),
            "transitive companion (depth 2) must NOT be chain-loaded, got: {names:?}"
        );
    }

    #[test]
    fn test_chain_load_missing_companion_is_silent() {
        // Parent lists a required skill that isn't loaded in the
        // registry. Should not error — just skip with a debug log.
        let parent =
            make_skill_with_requires("parent", &["setup"], &["does-not-exist", "also-missing"]);
        let skills = vec![parent];

        let result = prefilter_skills(
            "setup",
            &skills,
            10,
            MAX_SKILL_CONTEXT_TOKENS,
            &HashSet::new(),
        );
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name(), "parent");
    }

    #[test]
    fn test_chain_load_dedups_companion_shared_across_parents() {
        // Both parents declare the same companion. It should appear
        // only once in the result even though it's chain-loaded twice.
        let p1 = make_skill_with_requires("parent-one", &["one"], &["shared"]);
        let p2 = make_skill_with_requires("parent-two", &["two"], &["shared"]);
        let shared = make_skill("shared", &["nomatch"], &[], &[]);

        let skills = vec![p1, p2, shared];
        let result = prefilter_skills(
            "one two",
            &skills,
            10,
            MAX_SKILL_CONTEXT_TOKENS,
            &HashSet::new(),
        );
        let names: Vec<&str> = result.iter().map(|s| s.name()).collect();
        assert_eq!(
            names.iter().filter(|n| **n == "shared").count(),
            1,
            "shared companion must appear only once, got: {names:?}"
        );
        assert!(names.contains(&"parent-one"));
        assert!(names.contains(&"parent-two"));
    }
}
