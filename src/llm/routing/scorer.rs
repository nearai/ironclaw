//! Complexity scorer for smart model routing.
//!
//! Analyzes user prompts across 13 dimensions to determine complexity,
//! then maps to a tier (flash/standard/pro/frontier).

use lazy_static::lazy_static;
use regex::Regex;
use std::collections::HashMap;

/// Complexity tier for model selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tier {
    /// Simple requests: greetings, quick lookups (0-15)
    Flash,
    /// Standard tasks: writing, comparisons (16-40)
    Standard,
    /// Complex work: multi-step analysis, code review (41-65)
    Pro,
    /// Critical tasks: security audits, high-stakes decisions (66+)
    Frontier,
}

impl Tier {
    /// Convert a complexity score to a tier.
    pub fn from_score(score: u32) -> Self {
        match score {
            0..=15 => Tier::Flash,
            16..=40 => Tier::Standard,
            41..=65 => Tier::Pro,
            _ => Tier::Frontier,
        }
    }

    /// Get a representative score for this tier.
    pub fn to_score(self) -> u32 {
        match self {
            Tier::Flash => 8,
            Tier::Standard => 28,
            Tier::Pro => 52,
            Tier::Frontier => 80,
        }
    }

    /// Tier name as string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Tier::Flash => "flash",
            Tier::Standard => "standard",
            Tier::Pro => "pro",
            Tier::Frontier => "frontier",
        }
    }
}

impl std::fmt::Display for Tier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Weights for each scoring dimension.
#[derive(Debug, Clone)]
pub struct ScorerWeights {
    pub reasoning_words: f32,
    pub token_estimate: f32,
    pub code_indicators: f32,
    pub multi_step: f32,
    pub domain_specific: f32,
    pub ambiguity: f32,
    pub creativity: f32,
    pub precision: f32,
    pub context_dependency: f32,
    pub tool_likelihood: f32,
    pub safety_sensitivity: f32,
    pub question_complexity: f32,
    pub sentence_complexity: f32,
}

impl Default for ScorerWeights {
    fn default() -> Self {
        Self {
            reasoning_words: 0.14,
            token_estimate: 0.12,
            code_indicators: 0.10,
            multi_step: 0.10,
            domain_specific: 0.10,
            ambiguity: 0.05,
            creativity: 0.07,
            precision: 0.06,
            context_dependency: 0.05,
            tool_likelihood: 0.05,
            safety_sensitivity: 0.04,
            question_complexity: 0.07,
            sentence_complexity: 0.05,
        }
    }
}

/// Breakdown of complexity score by dimension.
#[derive(Debug, Clone)]
pub struct ScoreBreakdown {
    /// Total complexity score (0-100).
    pub total: u32,
    /// Computed tier.
    pub tier: Tier,
    /// Per-dimension scores (0-100 each).
    pub components: HashMap<String, u32>,
    /// Human-readable hints about why this score.
    pub hints: Vec<String>,
}

lazy_static! {
    // Reasoning indicators
    static ref RE_REASONING: Regex = Regex::new(
        r"(?i)\b(why|how|explain|analyze|analyse|compare|contrast|evaluate|assess|reason|think|consider|implications?|consequences?|trade-?offs?|pros?\s*(and|&)\s*cons?|advantages?|disadvantages?|benefits?|drawbacks?|differs?|difference|versus|vs\.?|better|worse|optimal|best|worst)\b"
    ).unwrap();

    // Multi-step indicators
    static ref RE_MULTI_STEP: Regex = Regex::new(
        r"(?i)\b(first|then|next|after|before|finally|step|steps|phase|stages?|process|workflow|sequence|procedure|pipeline|chain|series|order|followed by)\b"
    ).unwrap();

    // Creativity indicators
    static ref RE_CREATIVITY: Regex = Regex::new(
        r"(?i)\b(write|create|generate|compose|design|imagine|brainstorm|ideate|draft|invent|story|poem|essay|article|blog|content|narrative|script|summarize|summarise|rewrite|paraphrase|translate|adapt|tweet|post|thread|outline|structure|format|style|tone|voice)\b"
    ).unwrap();

    // Precision indicators
    static ref RE_PRECISION: Regex = Regex::new(
        r"(?i)\b(\d{4}|\d+\.\d+|exactly|precisely|specific|accurate|correct|verify|confirm|date|time|number|calculate|compute|measure|count)\b"
    ).unwrap();

    // Code indicators
    static ref RE_CODE: Regex = Regex::new(
        r"(?i)(`{1,3}|```|function|const|let|var|import|export|class|def |async|await|=>|\.ts|\.js|\.py|\.rs|\.go|\.sol|\(\)|\[\]|\{\}|<[A-Z][a-z]+>|useState|useEffect|npm|yarn|pnpm|cargo|pip|implement|rebase|merge|commit|branch|PR|pull.?request|columns?|migrations?|module|refactor|debug|fix|bug|error|schema|database|query)"
    ).unwrap();

    // Tool usage indicators
    static ref RE_TOOL: Regex = Regex::new(
        r"(?i)\b(file|read|write|search|fetch|run|execute|check|look up|find|open|save|send|post|get|download|upload|install|deploy|build|compile|test|add|update|remove|delete|modify|change|edit|create|resolve|push|pull|clone)\b"
    ).unwrap();

    // Safety-sensitive indicators
    static ref RE_SAFETY: Regex = Regex::new(
        r"(?i)\b(password|secret|private|confidential|medical|legal|financial|personal|sensitive|ssn|credit.?card|auth|token|key|encrypt|decrypt|hash|vulnerability|exploit|attack|breach)\b"
    ).unwrap();

    // Context dependency indicators
    static ref RE_CONTEXT: Regex = Regex::new(
        r"(?i)\b(previous|earlier|above|before|last|that|those|it|they|we discussed|you said|mentioned|remember|recall|as I said|like I mentioned)\b"
    ).unwrap();

    // Domain-specific terms
    // TODO: Make configurable via ScorerConfig for project-specific keywords.
    // Current list covers common web3/infra terms as sensible defaults.
    static ref RE_DOMAIN: Regex = Regex::new(
        r"(?i)\b(kubernetes|k8s|docker|terraform|solidity|rust|typescript|react|nextjs|vue|angular|svelte|postgresql|postgres|mysql|mongodb|redis|graphql|grpc|protobuf|websocket|oauth|jwt|cors|csrf|xss|sql.?injection|api|rest|http|https|tcp|udp|dns|cdn|aws|gcp|azure|vercel|netlify|cloudflare|nginx|apache|linux|unix|bash|shell|git|github|gitlab|ci/cd|devops|blockchain|web3|ethereum|near|solana|defi|nft|smart.?contract|near.?sdk|near.?api|testnet|mainnet|fogo|lobo|trezu|multisig|treasury|openclaw|ironclaw|substack|anchor|svm|firedancer|paymaster|gasless|sessions.?sdk|cargo.?near|workspaces|sandbox|rpc|indexer|relayer|cross.?chain|intents|meteor|ledger|cold.?wallet)\b"
    ).unwrap();

    // Vague pronouns for ambiguity
    static ref RE_VAGUE: Regex = Regex::new(
        r"(?i)\b(it|this|that|something|stuff|thing|things)\b"
    ).unwrap();

    // Open-ended question starters
    static ref RE_OPEN_ENDED: Regex = Regex::new(
        r"(?i)\b(why|how|what if|explain|describe|elaborate|discuss)\b"
    ).unwrap();

    // Conjunctions for sentence complexity
    static ref RE_CONJUNCTIONS: Regex = Regex::new(
        r"(?i)\b(and|but|or|however|therefore|because|although|while|whereas|moreover|furthermore)\b"
    ).unwrap();

    // Explicit tier hint pattern
    static ref RE_TIER_HINT: Regex = Regex::new(
        r"(?i)\[tier:(flash|standard|pro|frontier)\]"
    ).unwrap();
}

/// Count regex matches in text.
fn count_matches(re: &Regex, text: &str) -> usize {
    re.find_iter(text).count()
}

/// Score a prompt for complexity.
///
/// Returns a breakdown with total score (0-100), tier, and per-dimension scores.
pub fn score_complexity(prompt: &str) -> ScoreBreakdown {
    score_complexity_with_weights(prompt, &ScorerWeights::default())
}

/// Score with custom weights.
pub fn score_complexity_with_weights(prompt: &str, weights: &ScorerWeights) -> ScoreBreakdown {
    let mut hints = Vec::new();
    let mut components = HashMap::new();

    // Check for explicit tier hint
    if let Some(caps) = RE_TIER_HINT.captures(prompt) {
        let tier_str = caps.get(1).unwrap().as_str().to_lowercase();
        let tier = match tier_str.as_str() {
            "flash" => Tier::Flash,
            "standard" => Tier::Standard,
            "pro" => Tier::Pro,
            "frontier" => Tier::Frontier,
            _ => unreachable!("RE_TIER_HINT regex should only capture valid tiers"),
        };
        hints.push(format!("Explicit tier hint: {}", tier));
        return ScoreBreakdown {
            total: tier.to_score(),
            tier,
            components,
            hints,
        };
    }

    // Token estimate (based on char count)
    // <20 chars = 0, >500 chars = 100
    let char_count = prompt.len();
    let token_score = ((char_count as i32 - 20).max(0) as f32 / 5.0).min(100.0) as u32;
    components.insert("token_estimate".to_string(), token_score);
    if char_count > 200 {
        hints.push(format!("Long prompt ({} chars)", char_count));
    }

    // Reasoning words
    let reasoning_count = count_matches(&RE_REASONING, prompt);
    let reasoning_score = (reasoning_count * 50).min(100) as u32;
    components.insert("reasoning_words".to_string(), reasoning_score);
    if reasoning_count >= 2 {
        hints.push(format!("reasoning_words: {} matches", reasoning_count));
    }

    // Multi-step
    let multi_step_count = count_matches(&RE_MULTI_STEP, prompt);
    let multi_step_score = (multi_step_count * 50).min(100) as u32;
    components.insert("multi_step".to_string(), multi_step_score);
    if multi_step_count >= 2 {
        hints.push(format!("multi_step: {} matches", multi_step_count));
    }

    // Creativity
    let creativity_count = count_matches(&RE_CREATIVITY, prompt);
    let creativity_score = (creativity_count * 50).min(100) as u32;
    components.insert("creativity".to_string(), creativity_score);
    if creativity_count >= 2 {
        hints.push(format!("creativity: {} matches", creativity_count));
    }

    // Precision
    let precision_count = count_matches(&RE_PRECISION, prompt);
    let precision_score = (precision_count * 50).min(100) as u32;
    components.insert("precision".to_string(), precision_score);

    // Code indicators
    let code_count = count_matches(&RE_CODE, prompt);
    let code_score = (code_count * 50).min(100) as u32;
    components.insert("code_indicators".to_string(), code_score);
    if code_count >= 2 {
        hints.push(format!("code_indicators: {} matches", code_count));
    }

    // Tool likelihood
    let tool_count = count_matches(&RE_TOOL, prompt);
    let tool_score = (tool_count * 50).min(100) as u32;
    components.insert("tool_likelihood".to_string(), tool_score);

    // Safety sensitivity
    let safety_count = count_matches(&RE_SAFETY, prompt);
    let safety_score = (safety_count * 50).min(100) as u32;
    components.insert("safety_sensitivity".to_string(), safety_score);
    if safety_count >= 1 {
        hints.push(format!("safety_sensitivity: {} matches", safety_count));
    }

    // Context dependency
    let context_count = count_matches(&RE_CONTEXT, prompt);
    let context_score = (context_count * 50).min(100) as u32;
    components.insert("context_dependency".to_string(), context_score);

    // Domain specific
    let domain_count = count_matches(&RE_DOMAIN, prompt);
    let domain_score = (domain_count * 50).min(100) as u32;
    components.insert("domain_specific".to_string(), domain_score);
    if domain_count >= 2 {
        hints.push(format!("domain_specific: {} matches", domain_count));
    }

    // Ambiguity (vague pronouns)
    let vague_count = count_matches(&RE_VAGUE, prompt);
    let ambiguity_score = (vague_count * 25).min(100) as u32;
    components.insert("ambiguity".to_string(), ambiguity_score);

    // Question complexity
    let question_marks = prompt.matches('?').count();
    let open_ended_count = count_matches(&RE_OPEN_ENDED, prompt);
    let question_score = ((question_marks * 20) + (open_ended_count * 25)).min(100) as u32;
    components.insert("question_complexity".to_string(), question_score);
    if question_marks >= 2 {
        hints.push(format!("Multiple questions: {}", question_marks));
    }

    // Sentence complexity (commas, semicolons, conjunctions)
    let commas = prompt.matches(',').count();
    let semicolons = prompt.matches(';').count();
    let conjunctions = count_matches(&RE_CONJUNCTIONS, prompt);
    let clauses = commas + (semicolons * 2) + conjunctions;
    let sentence_score = (clauses * 12).min(100) as u32;
    components.insert("sentence_complexity".to_string(), sentence_score);
    if clauses >= 5 {
        hints.push(format!("Complex structure: {} clauses", clauses));
    }

    // Calculate weighted total
    let mut total: f32 = [
        ("reasoning_words", weights.reasoning_words),
        ("token_estimate", weights.token_estimate),
        ("code_indicators", weights.code_indicators),
        ("multi_step", weights.multi_step),
        ("domain_specific", weights.domain_specific),
        ("ambiguity", weights.ambiguity),
        ("creativity", weights.creativity),
        ("precision", weights.precision),
        ("context_dependency", weights.context_dependency),
        ("tool_likelihood", weights.tool_likelihood),
        ("safety_sensitivity", weights.safety_sensitivity),
        ("question_complexity", weights.question_complexity),
        ("sentence_complexity", weights.sentence_complexity),
    ]
    .iter()
    .map(|(name, weight)| components.get(*name).copied().unwrap_or(0) as f32 * weight)
    .sum();

    // Multi-dimensional boost: +30% when 3+ dimensions fire above threshold
    let triggered_dimensions = components.values().filter(|&&v| v > 20).count();
    if triggered_dimensions >= 3 {
        total *= 1.3;
        hints.push(format!("Multi-dimensional ({} triggers)", triggered_dimensions));
    } else if triggered_dimensions >= 2 {
        total *= 1.15;
    }

    // Clamp to 0-100
    let total = (total as u32).clamp(0, 100);
    let tier = Tier::from_score(total);

    ScoreBreakdown {
        total,
        tier,
        components,
        hints,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_greeting() {
        let result = score_complexity("Hi");
        assert_eq!(result.tier, Tier::Flash);
        assert!(result.total <= 15);
    }

    #[test]
    fn test_quick_question() {
        let result = score_complexity("What time is it?");
        assert!(result.tier == Tier::Flash || result.tier == Tier::Standard);
    }

    #[test]
    fn test_code_task() {
        let result = score_complexity("Implement a function to sort an array in TypeScript");
        assert!(result.tier == Tier::Standard || result.tier == Tier::Pro);
    }

    #[test]
    fn test_complex_analysis() {
        let result = score_complexity(
            "Explain why React uses a virtual DOM and compare it to Svelte's approach. \
             Consider the trade-offs for performance and developer experience."
        );
        assert!(result.tier == Tier::Standard || result.tier == Tier::Pro);
        assert!(result.total >= 20);
    }

    #[test]
    fn test_security_audit() {
        // Note: Security audits are caught by pattern override in Router, not scorer.
        // The scorer should still rate this relatively high due to multi-step + safety terms.
        let result = score_complexity(
            "Analyze this Solidity contract for reentrancy vulnerabilities, \
             check for authentication bypass, and provide a security audit report."
        );
        // Should score at least Standard (16+) due to complexity
        assert!(result.tier == Tier::Standard || result.tier == Tier::Pro || result.tier == Tier::Frontier);
        assert!(result.total >= 16, "Expected score >= 16, got {}", result.total);
    }

    #[test]
    fn test_explicit_tier_hint() {
        let result = score_complexity("[tier:flash] This is a complex-looking message but should be fast");
        assert_eq!(result.tier, Tier::Flash);
        assert!(result.hints.iter().any(|h| h.contains("Explicit tier hint")));
    }

    #[test]
    fn test_frontier_override() {
        let result = score_complexity("[tier:frontier] Simple question but I want the best");
        assert_eq!(result.tier, Tier::Frontier);
    }

    #[test]
    fn test_multi_step() {
        let result = score_complexity(
            "First, read the file at src/auth.ts. Then analyze it for security issues. \
             After that, write a detailed report."
        );
        assert!(result.total >= 30);
        assert!(result.hints.iter().any(|h| h.contains("multi_step")));
    }

    #[test]
    fn test_tier_display() {
        assert_eq!(Tier::Flash.as_str(), "flash");
        assert_eq!(Tier::Frontier.to_string(), "frontier");
    }
}
