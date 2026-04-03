//! Smart routing provider that routes requests to cheap or primary models based on task complexity.
//!
//! Uses a 13-dimension complexity scorer (from PR #208 by @onlyamicrowave) to analyze prompts
//! across reasoning, code, multi-step, domain-specific, creativity, precision, safety, and other
//! dimensions. Pattern overrides provide fast-path routing for obvious cases (greetings → cheap,
//! security audits → primary).
//!
//! This is a decorator that wraps two `LlmProvider`s and implements `LlmProvider` itself,
//! following the same pattern as `RetryProvider`, `CachedProvider`, and `CircuitBreakerProvider`.
//!
//! # Complexity Tiers
//!
//! The scorer produces a 0-100 score mapped to four tiers:
//! - **Flash** (0-15): Greetings, quick lookups → cheap model
//! - **Standard** (16-40): Writing, comparisons → cheap model
//! - **Pro** (41-65): Multi-step analysis, code review → cheap with cascade, or primary
//! - **Frontier** (66+): Security audits, critical decisions → primary model

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use regex::Regex;
use rust_decimal::Decimal;

use crate::llm::error::LlmError;
use crate::llm::provider::{
    CompletionRequest, CompletionResponse, LlmProvider, ModelMetadata, Role, ToolCompletionRequest,
    ToolCompletionResponse,
};

// ---------------------------------------------------------------------------
// Complexity tiers & scoring
// ---------------------------------------------------------------------------

/// Complexity tier produced by the 13-dimension scorer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tier {
    /// Simple requests: greetings, quick lookups (score 0-15).
    Flash,
    /// Standard tasks: writing, comparisons (score 16-40).
    Standard,
    /// Complex work: multi-step analysis, code review (score 41-65).
    Pro,
    /// Critical tasks: security audits, high-stakes decisions (score 66+).
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

    /// Get a representative score for this tier (used when score is not computed).
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

/// Weights for each of the 13 scoring dimensions.
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

/// Default domain-specific keywords for complexity scoring.
pub const DEFAULT_DOMAIN_KEYWORDS: &[&str] = &[
    // Infrastructure
    "kubernetes",
    "k8s",
    "docker",
    "terraform",
    "nginx",
    "apache",
    "linux",
    "unix",
    "bash",
    "shell",
    // Languages & frameworks
    "solidity",
    "rust",
    "typescript",
    "react",
    "nextjs",
    "vue",
    "angular",
    "svelte",
    // Databases
    "postgresql",
    "postgres",
    "mysql",
    "mongodb",
    "redis",
    // APIs & protocols
    "graphql",
    "grpc",
    "protobuf",
    "websocket",
    "oauth",
    "jwt",
    "cors",
    "csrf",
    "xss",
    "sql.?injection",
    "api",
    "rest",
    "http",
    "https",
    "tcp",
    "udp",
    "dns",
    "cdn",
    // Cloud & deployment
    "aws",
    "gcp",
    "azure",
    "vercel",
    "netlify",
    "cloudflare",
    "ci/cd",
    "devops",
    // Version control
    "git",
    "github",
    "gitlab",
    // Web3 general
    "blockchain",
    "web3",
    "defi",
    "nft",
    "smart.?contract",
    // Ethereum
    "ethereum",
    "evm",
    "anchor",
    // NEAR ecosystem
    "near",
    "near.?sdk",
    "near.?api",
    "testnet",
    "mainnet",
    "meteor",
    "ledger",
    "cold.?wallet",
    "rpc",
    "indexer",
    "relayer",
    "cross.?chain",
    "intents",
    // Fogo/SVM
    "fogo",
    "svm",
    "firedancer",
    "paymaster",
    "gasless",
    "sessions.?sdk",
    // Rust/NEAR tooling
    "cargo.?near",
    "workspaces",
    "sandbox",
    // Project-specific
    "lobo",
    "trezu",
    "multisig",
    "treasury",
    "openclaw",
    "ironclaw",
];

/// Configuration for the complexity scorer.
#[derive(Debug, Clone, Default)]
pub struct ScorerConfig {
    /// Weights for each scoring dimension.
    pub weights: ScorerWeights,
    /// Custom domain-specific keywords (overrides defaults if provided).
    /// Each entry is a word or regex pattern fragment.
    pub domain_keywords: Option<Vec<String>>,
}

/// Build a domain regex from a keyword list, with fallback on invalid patterns.
///
/// An empty keyword list falls back to the default keywords so scoring
/// doesn't break when `domain_keywords: Some(vec![])` is configured.
fn build_domain_regex(keywords: &[&str]) -> Regex {
    if keywords.is_empty() {
        return RE_DOMAIN_DEFAULT.clone();
    }
    let pattern = format!(r"(?i)\b({})\b", keywords.join("|"));
    Regex::new(&pattern).unwrap_or_else(|e| {
        tracing::warn!(error = %e, "Invalid domain keywords pattern, using minimal fallback");
        Regex::new(r"(?i)\b(api|code|deploy)\b").expect("fallback regex is valid") // safety: hardcoded literal
    })
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

// ---------------------------------------------------------------------------
// Static regex patterns (compiled once via LazyLock)
// ---------------------------------------------------------------------------

use std::sync::LazyLock;

static RE_REASONING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(why|how|explain|analyze|analyse|compare|contrast|evaluate|assess|reason|think|consider|implications?|consequences?|trade-?offs?|pros?\s*(and|&)\s*cons?|advantages?|disadvantages?|benefits?|drawbacks?|differs?|difference|versus|vs\.?|better|worse|optimal|best|worst)\b"
    ).expect("RE_REASONING is a valid regex") // safety: hardcoded literal
});

static RE_MULTI_STEP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(first|then|next|after|before|finally|step|steps|phase|stages?|process|workflow|sequence|procedure|pipeline|chain|series|order|followed by)\b"
    ).expect("RE_MULTI_STEP is a valid regex") // safety: hardcoded literal
});

static RE_CREATIVITY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(write|create|generate|compose|design|imagine|brainstorm|ideate|draft|invent|story|poem|essay|article|blog|content|narrative|script|summarize|summarise|rewrite|paraphrase|translate|adapt|tweet|post|thread|outline|structure|format|style|tone|voice)\b"
    ).expect("RE_CREATIVITY is a valid regex") // safety: hardcoded literal
});

static RE_PRECISION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(\d{4}|\d+\.\d+|exactly|precisely|specific|accurate|correct|verify|confirm|date|time|number|calculate|compute|measure|count)\b"
    ).expect("RE_PRECISION is a valid regex") // safety: hardcoded literal
});

static RE_CODE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(`{1,3}|```|function|const|let|var|import|export|class|def |async|await|=>|\.ts|\.js|\.py|\.rs|\.go|\.sol|\(\)|\[\]|\{\}|<[A-Z][a-z]+>|useState|useEffect|npm|yarn|pnpm|cargo|pip|implement|rebase|merge|commit|branch|PR|pull.?request|columns?|migrations?|module|refactor|debug|fix|bug|error|schema|database|query)"
    ).expect("RE_CODE is a valid regex") // safety: hardcoded literal
});

static RE_TOOL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(file|read|write|search|fetch|run|execute|check|look up|find|open|save|send|post|get|download|upload|install|deploy|build|compile|test|add|update|remove|delete|modify|change|edit|create|resolve|push|pull|clone)\b"
    ).expect("RE_TOOL is a valid regex") // safety: hardcoded literal
});

static RE_SAFETY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(password|secret|private|confidential|medical|legal|financial|personal|sensitive|ssn|credit.?card|auth|token|key|encrypt|decrypt|hash|vulnerability|exploit|attack|breach)\b"
    ).expect("RE_SAFETY is a valid regex") // safety: hardcoded literal
});

static RE_CONTEXT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(previous|earlier|above|before|last|that|those|it|they|we discussed|you said|mentioned|remember|recall|as I said|like I mentioned)\b"
    ).expect("RE_CONTEXT is a valid regex") // safety: hardcoded literal
});

static RE_VAGUE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(it|this|that|something|stuff|thing|things)\b")
        .expect("RE_VAGUE is a valid regex") // safety: hardcoded literal
});

static RE_OPEN_ENDED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(why|how|what if|explain|describe|elaborate|discuss)\b")
        .expect("RE_OPEN_ENDED is a valid regex") // safety: hardcoded literal
});

static RE_CONJUNCTIONS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(and|but|or|however|therefore|because|although|while|whereas|moreover|furthermore)\b",
    )
    .expect("RE_CONJUNCTIONS is a valid regex") // safety: hardcoded literal
});

static RE_TIER_HINT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\[tier:(flash|standard|pro|frontier)\]")
        .expect("RE_TIER_HINT is a valid regex") // safety: hardcoded literal
});

/// Default domain regex, compiled once from `DEFAULT_DOMAIN_KEYWORDS`.
static RE_DOMAIN_DEFAULT: LazyLock<Regex> =
    LazyLock::new(|| build_domain_regex(DEFAULT_DOMAIN_KEYWORDS));

// ---------------------------------------------------------------------------
// Pattern overrides (fast-path before scoring)
// ---------------------------------------------------------------------------

/// A compiled pattern override entry.
struct PatternOverride {
    regex: Regex,
    tier: Tier,
}

/// Default pattern overrides, compiled once.
static DEFAULT_OVERRIDES: LazyLock<Vec<PatternOverride>> = LazyLock::new(|| {
    vec![
        // Flash tier: greetings and acknowledgments
        PatternOverride {
            regex: Regex::new(
                r"(?i)^(hi|hello|hey|thanks|ok|sure|yes|no|yep|nope|cool|nice|great|got it)$",
            )
            .expect("greeting pattern is valid"), // safety: hardcoded literal
            tier: Tier::Flash,
        },
        // Flash tier: quick lookups (end-anchored to avoid matching complex questions
        // like "What time complexity is merge sort?")
        PatternOverride {
            regex: Regex::new(
                r"(?i)^what(?:'s|\s+is)?\s+(?:the\s+)?(time|date|day|weather)\b(?:\s+(?:is\s+it|today|now|in\s+\S+))?[?.!]*$",
            )
            .expect("lookup pattern is valid"), // safety: hardcoded literal
            tier: Tier::Flash,
        },
        // Frontier tier: security audits
        PatternOverride {
            regex: Regex::new(r"(?i)security.*(audit|review|scan)")
                .expect("security audit pattern is valid"), // safety: hardcoded literal
            tier: Tier::Frontier,
        },
        PatternOverride {
            regex: Regex::new(r"(?i)vulnerabilit(y|ies).*(review|scan|check|audit)")
                .expect("vulnerability pattern is valid"), // safety: hardcoded literal
            tier: Tier::Frontier,
        },
        // Pro tier: production deployments
        PatternOverride {
            regex: Regex::new(r"(?i)deploy.*(mainnet|production)")
                .expect("deploy pattern is valid"), // safety: hardcoded literal
            tier: Tier::Pro,
        },
        PatternOverride {
            regex: Regex::new(r"(?i)production.*(deploy|release|push)")
                .expect("production pattern is valid"), // safety: hardcoded literal
            tier: Tier::Pro,
        },
    ]
});

// ---------------------------------------------------------------------------
// Scoring functions
// ---------------------------------------------------------------------------

/// Count regex matches in text.
fn count_matches(re: &Regex, text: &str) -> usize {
    re.find_iter(text).count()
}

/// Score a prompt's complexity across 13 dimensions.
///
/// Returns a `ScoreBreakdown` with a total score (0-100) and per-dimension breakdown.
pub fn score_complexity(prompt: &str) -> ScoreBreakdown {
    score_complexity_with_config(prompt, &ScorerConfig::default())
}

/// Score with custom configuration (weights + domain keywords).
///
/// If you will call this repeatedly with the same config, prefer
/// [`score_complexity_with_regex`] and pre-build the domain regex once.
pub fn score_complexity_with_config(prompt: &str, config: &ScorerConfig) -> ScoreBreakdown {
    let domain_regex = match &config.domain_keywords {
        Some(custom) => {
            let refs: Vec<&str> = custom.iter().map(|s| s.as_str()).collect();
            build_domain_regex(&refs)
        }
        None => RE_DOMAIN_DEFAULT.clone(),
    };
    score_complexity_internal(prompt, &config.weights, &domain_regex)
}

/// Score with a pre-compiled domain regex (avoids rebuilding per call).
pub fn score_complexity_with_regex(
    prompt: &str,
    weights: &ScorerWeights,
    domain_regex: &Regex,
) -> ScoreBreakdown {
    score_complexity_internal(prompt, weights, domain_regex)
}

/// Internal scoring implementation.
fn score_complexity_internal(
    prompt: &str,
    weights: &ScorerWeights,
    domain_regex: &Regex,
) -> ScoreBreakdown {
    let mut hints = Vec::new();
    let mut components = HashMap::new();

    // Check for explicit tier hint (e.g. "[tier:flash]")
    if let Some(caps) = RE_TIER_HINT.captures(prompt) {
        let tier_str = caps.get(1).expect("capture group 1 exists").as_str(); // safety: RE_TIER_HINT has group 1
        let tier = match tier_str.to_lowercase().as_str() {
            "flash" => Tier::Flash,
            "standard" => Tier::Standard,
            "pro" => Tier::Pro,
            "frontier" => Tier::Frontier,
            // The regex only captures valid tiers, so this is defensive.
            other => {
                tracing::error!(tier = %other, "Unexpected tier in hint despite regex constraint");
                Tier::Standard
            }
        };
        hints.push(format!("Explicit tier hint: {tier}"));
        return ScoreBreakdown {
            total: tier.to_score(),
            tier,
            components,
            hints,
        };
    }

    // Token estimate (based on char count): <20 chars = 0, >=520 chars = 100
    let char_count = prompt.len();
    let token_score = ((char_count as i32 - 20).max(0) as f32 / 5.0).min(100.0) as u32;
    components.insert("token_estimate".to_string(), token_score);
    if char_count > 200 {
        hints.push(format!("Long prompt ({char_count} chars)"));
    }

    // Reasoning words
    let reasoning_count = count_matches(&RE_REASONING, prompt);
    let reasoning_score = (reasoning_count * 50).min(100) as u32;
    components.insert("reasoning_words".to_string(), reasoning_score);
    if reasoning_count >= 2 {
        hints.push(format!("reasoning_words: {reasoning_count} matches"));
    }

    // Multi-step
    let multi_step_count = count_matches(&RE_MULTI_STEP, prompt);
    let multi_step_score = (multi_step_count * 50).min(100) as u32;
    components.insert("multi_step".to_string(), multi_step_score);
    if multi_step_count >= 2 {
        hints.push(format!("multi_step: {multi_step_count} matches"));
    }

    // Creativity
    let creativity_count = count_matches(&RE_CREATIVITY, prompt);
    let creativity_score = (creativity_count * 50).min(100) as u32;
    components.insert("creativity".to_string(), creativity_score);
    if creativity_count >= 2 {
        hints.push(format!("creativity: {creativity_count} matches"));
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
        hints.push(format!("code_indicators: {code_count} matches"));
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
        hints.push(format!("safety_sensitivity: {safety_count} matches"));
    }

    // Context dependency
    let context_count = count_matches(&RE_CONTEXT, prompt);
    let context_score = (context_count * 50).min(100) as u32;
    components.insert("context_dependency".to_string(), context_score);

    // Domain specific
    let domain_count = count_matches(domain_regex, prompt);
    let domain_score = (domain_count * 50).min(100) as u32;
    components.insert("domain_specific".to_string(), domain_score);
    if domain_count >= 2 {
        hints.push(format!("domain_specific: {domain_count} matches"));
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
        hints.push(format!("Multiple questions: {question_marks}"));
    }

    // Sentence complexity (commas, semicolons, conjunctions)
    let commas = prompt.matches(',').count();
    let semicolons = prompt.matches(';').count();
    let conjunctions = count_matches(&RE_CONJUNCTIONS, prompt);
    let clauses = commas + (semicolons * 2) + conjunctions;
    let sentence_score = (clauses * 12).min(100) as u32;
    components.insert("sentence_complexity".to_string(), sentence_score);
    if clauses >= 5 {
        hints.push(format!("Complex structure: {clauses} clauses"));
    }

    // Calculate weighted total using data-driven iteration
    let total: f32 = [
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
    let total = if triggered_dimensions >= 3 {
        hints.push(format!(
            "Multi-dimensional ({triggered_dimensions} triggers)"
        ));
        total * 1.3
    } else if triggered_dimensions >= 2 {
        total * 1.15
    } else {
        total
    };

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

// ---------------------------------------------------------------------------
// TaskComplexity (provider-level classification)
// ---------------------------------------------------------------------------

/// Classification of a request's complexity, determining which model handles it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskComplexity {
    /// Short, simple queries -> cheap model (Flash + Standard tiers)
    Simple,
    /// Ambiguous complexity -> cheap model first, cascade to primary if uncertain (Pro tier)
    Moderate,
    /// Code generation, analysis, multi-step reasoning -> primary model (Frontier tier)
    Complex,
}

impl From<Tier> for TaskComplexity {
    fn from(tier: Tier) -> Self {
        match tier {
            Tier::Flash | Tier::Standard => TaskComplexity::Simple,
            Tier::Pro => TaskComplexity::Moderate,
            Tier::Frontier => TaskComplexity::Complex,
        }
    }
}

// ---------------------------------------------------------------------------
// SmartRoutingConfig & Provider
// ---------------------------------------------------------------------------

/// Configuration for the smart routing provider.
#[derive(Debug, Clone)]
pub struct SmartRoutingConfig {
    /// Enable cascade mode: retry with primary if cheap model response seems uncertain.
    pub cascade_enabled: bool,
    /// Custom domain keywords for the scorer (None uses defaults).
    pub domain_keywords: Option<Vec<String>>,
}

impl Default for SmartRoutingConfig {
    fn default() -> Self {
        Self {
            cascade_enabled: true,
            domain_keywords: None,
        }
    }
}

/// Atomic counters for routing observability.
struct SmartRoutingStats {
    total_requests: AtomicU64,
    cheap_requests: AtomicU64,
    primary_requests: AtomicU64,
    cascade_escalations: AtomicU64,
}

impl SmartRoutingStats {
    fn new() -> Self {
        Self {
            total_requests: AtomicU64::new(0),
            cheap_requests: AtomicU64::new(0),
            primary_requests: AtomicU64::new(0),
            cascade_escalations: AtomicU64::new(0),
        }
    }
}

/// Snapshot of routing statistics for external consumption.
#[derive(Debug, Clone)]
pub struct SmartRoutingSnapshot {
    pub total_requests: u64,
    pub cheap_requests: u64,
    pub primary_requests: u64,
    pub cascade_escalations: u64,
}

/// Smart routing provider that classifies task complexity and routes to the appropriate model.
///
/// - `complete()` — scores complexity across 13 dimensions, checks pattern overrides, then
///   routes to cheap or primary model. Moderate tasks use cascade (try cheap, escalate if uncertain).
/// - `complete_with_tools()` — always routes to primary (tool use requires reliable structured output)
pub struct SmartRoutingProvider {
    primary: Arc<dyn LlmProvider>,
    cheap: Arc<dyn LlmProvider>,
    config: SmartRoutingConfig,
    scorer_config: ScorerConfig,
    /// Pre-compiled domain regex (built once at construction time).
    domain_regex: Regex,
    stats: SmartRoutingStats,
}

impl SmartRoutingProvider {
    /// Create a new smart routing provider wrapping a primary and cheap provider.
    pub fn new(
        primary: Arc<dyn LlmProvider>,
        cheap: Arc<dyn LlmProvider>,
        config: SmartRoutingConfig,
    ) -> Self {
        let scorer_config = ScorerConfig {
            weights: ScorerWeights::default(),
            domain_keywords: config.domain_keywords.clone(),
        };
        let domain_regex = match &scorer_config.domain_keywords {
            Some(custom) => {
                let refs: Vec<&str> = custom.iter().map(|s| s.as_str()).collect();
                build_domain_regex(&refs)
            }
            None => RE_DOMAIN_DEFAULT.clone(),
        };
        Self {
            primary,
            cheap,
            config,
            scorer_config,
            domain_regex,
            stats: SmartRoutingStats::new(),
        }
    }

    /// Get a snapshot of routing statistics.
    pub fn stats(&self) -> SmartRoutingSnapshot {
        SmartRoutingSnapshot {
            total_requests: self.stats.total_requests.load(Ordering::Relaxed),
            cheap_requests: self.stats.cheap_requests.load(Ordering::Relaxed),
            primary_requests: self.stats.primary_requests.load(Ordering::Relaxed),
            cascade_escalations: self.stats.cascade_escalations.load(Ordering::Relaxed),
        }
    }

    /// Classify the complexity of a request based on its last user message.
    ///
    /// Priority: explicit tier hints > pattern overrides > 13-dimension scorer.
    fn classify(&self, request: &CompletionRequest) -> TaskComplexity {
        let last_user_msg = request
            .messages
            .iter()
            .rev()
            .find(|m| m.role == Role::User)
            .map(|m| m.content.as_str())
            .unwrap_or("");

        // Normalize: trim whitespace so anchored regexes and token scoring are consistent.
        let last_user_msg = last_user_msg.trim();

        // Highest priority: explicit tier hints (e.g. "[tier:flash]")
        if let Some(caps) = RE_TIER_HINT.captures(last_user_msg) {
            // SAFETY: RE_TIER_HINT has exactly one capture group; get(1) is guaranteed Some after match.
            let tier_str = caps.get(1).expect("capture group 1 exists").as_str(); // safety: RE_TIER_HINT has group 1
            let tier = match tier_str.to_lowercase().as_str() {
                "flash" => Tier::Flash,
                "standard" => Tier::Standard,
                "pro" => Tier::Pro,
                "frontier" => Tier::Frontier,
                other => {
                    tracing::error!(tier = %other, "Unexpected tier in hint despite regex constraint");
                    Tier::Standard
                }
            };
            let complexity = TaskComplexity::from(tier);
            tracing::trace!(
                %tier,
                ?complexity,
                "Smart routing: explicit tier hint"
            );
            return complexity;
        }

        // Fast-path: check pattern overrides
        for po in DEFAULT_OVERRIDES.iter() {
            if po.regex.is_match(last_user_msg) {
                let complexity = TaskComplexity::from(po.tier);
                tracing::trace!(
                    tier = %po.tier,
                    ?complexity,
                    "Smart routing: pattern override matched"
                );
                return complexity;
            }
        }

        // Full 13-dimension scoring (uses pre-compiled domain regex)
        let breakdown = score_complexity_with_regex(
            last_user_msg,
            &self.scorer_config.weights,
            &self.domain_regex,
        );
        let complexity = TaskComplexity::from(breakdown.tier);
        tracing::trace!(
            score = breakdown.total,
            tier = %breakdown.tier,
            ?complexity,
            hints = ?breakdown.hints,
            "Smart routing: scored complexity"
        );
        complexity
    }

    /// Check if a response from the cheap model shows uncertainty, warranting escalation.
    fn response_is_uncertain(response: &CompletionResponse) -> bool {
        let content = response.content.trim();

        // Empty response is always uncertain
        if content.is_empty() {
            return true;
        }

        let lower = content.to_lowercase();

        // Uncertainty signals
        let uncertainty_patterns = [
            "i'm not sure",
            "i am not sure",
            "i don't know",
            "i do not know",
            "i'm unable to",
            "i am unable to",
            "i cannot",
            "i can't",
            "beyond my capabilities",
            "beyond my ability",
            "i'm not able to",
            "i am not able to",
            "i don't have enough",
            "i do not have enough",
            "i need more context",
            "i need more information",
            "could you clarify",
            "could you provide more",
            "i'm not confident",
            "i am not confident",
        ];

        uncertainty_patterns.iter().any(|p| lower.contains(p))
    }
}

#[async_trait]
impl LlmProvider for SmartRoutingProvider {
    fn model_name(&self) -> &str {
        self.primary.model_name()
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        self.primary.cost_per_token()
    }

    fn cache_write_multiplier(&self) -> Decimal {
        self.primary.cache_write_multiplier()
    }

    fn cache_read_discount(&self) -> Decimal {
        self.primary.cache_read_discount()
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        self.stats.total_requests.fetch_add(1, Ordering::Relaxed);

        let complexity = self.classify(&request);

        match complexity {
            TaskComplexity::Simple => {
                tracing::trace!(
                    model = %self.cheap.model_name(),
                    "Smart routing: Simple task -> cheap model"
                );
                self.stats.cheap_requests.fetch_add(1, Ordering::Relaxed);
                self.cheap.complete(request).await
            }
            TaskComplexity::Complex => {
                tracing::trace!(
                    model = %self.primary.model_name(),
                    "Smart routing: Complex task -> primary model"
                );
                self.stats.primary_requests.fetch_add(1, Ordering::Relaxed);
                self.primary.complete(request).await
            }
            TaskComplexity::Moderate => {
                if self.config.cascade_enabled {
                    tracing::trace!(
                        model = %self.cheap.model_name(),
                        "Smart routing: Moderate task -> cheap model (cascade enabled)"
                    );
                    self.stats.cheap_requests.fetch_add(1, Ordering::Relaxed);

                    let response = self.cheap.complete(request.clone()).await?;

                    if Self::response_is_uncertain(&response) {
                        tracing::info!(
                            cheap_model = %self.cheap.model_name(),
                            primary_model = %self.primary.model_name(),
                            "Smart routing: Escalating to primary (cheap model response uncertain)"
                        );
                        self.stats
                            .cascade_escalations
                            .fetch_add(1, Ordering::Relaxed);
                        self.stats.primary_requests.fetch_add(1, Ordering::Relaxed);
                        self.primary.complete(request).await
                    } else {
                        Ok(response)
                    }
                } else {
                    // Without cascade, moderate tasks go to cheap model
                    tracing::trace!(
                        model = %self.cheap.model_name(),
                        "Smart routing: Moderate task -> cheap model (cascade disabled)"
                    );
                    self.stats.cheap_requests.fetch_add(1, Ordering::Relaxed);
                    self.cheap.complete(request).await
                }
            }
        }
    }

    /// Tool use always goes to the primary model for reliable structured output.
    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        self.stats.total_requests.fetch_add(1, Ordering::Relaxed);
        self.stats.primary_requests.fetch_add(1, Ordering::Relaxed);
        tracing::trace!(
            model = %self.primary.model_name(),
            "Smart routing: Tool use -> primary model (always)"
        );
        self.primary.complete_with_tools(request).await
    }

    async fn complete_streaming(
        &self,
        request: CompletionRequest,
        on_token: &(dyn Fn(String) + Send + Sync),
    ) -> Result<CompletionResponse, LlmError> {
        self.primary.complete_streaming(request, on_token).await
    }

    async fn complete_with_tools_streaming(
        &self,
        request: ToolCompletionRequest,
        on_token: &(dyn Fn(String) + Send + Sync),
    ) -> Result<ToolCompletionResponse, LlmError> {
        self.primary
            .complete_with_tools_streaming(request, on_token)
            .await
    }

    async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        self.primary.list_models().await
    }

    async fn model_metadata(&self) -> Result<ModelMetadata, LlmError> {
        self.primary.model_metadata().await
    }

    fn effective_model_name(&self, requested_model: Option<&str>) -> String {
        self.primary.effective_model_name(requested_model)
    }

    fn active_model_name(&self) -> String {
        self.primary.active_model_name()
    }

    fn set_model(&self, model: &str) -> Result<(), LlmError> {
        self.primary.set_model(model)
    }

    fn calculate_cost(&self, input_tokens: u32, output_tokens: u32) -> Decimal {
        self.primary.calculate_cost(input_tokens, output_tokens)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::ChatMessage;
    use crate::testing::StubLlm;

    fn default_config() -> SmartRoutingConfig {
        SmartRoutingConfig::default()
    }

    // -----------------------------------------------------------------------
    // Score complexity: tier boundaries
    // -----------------------------------------------------------------------

    #[test]
    fn score_empty_prompt_is_flash() {
        let result = score_complexity("");
        assert_eq!(result.tier, Tier::Flash);
        assert!(result.total <= 15);
    }

    #[test]
    fn score_simple_greeting_is_flash() {
        let result = score_complexity("Hi");
        assert_eq!(result.tier, Tier::Flash);
        assert!(result.total <= 15);
    }

    #[test]
    fn score_quick_question_is_flash_or_standard() {
        let result = score_complexity("What time is it?");
        assert!(
            result.tier == Tier::Flash || result.tier == Tier::Standard,
            "Expected Flash or Standard, got {:?} (score {})",
            result.tier,
            result.total
        );
    }

    #[test]
    fn score_code_task_is_standard_or_higher() {
        let result = score_complexity("Implement a function to sort an array in TypeScript");
        assert!(
            result.tier == Tier::Standard || result.tier == Tier::Pro,
            "Expected Standard or Pro, got {:?} (score {})",
            result.tier,
            result.total
        );
    }

    #[test]
    fn score_complex_analysis_is_at_least_standard() {
        let result = score_complexity(
            "Explain why React uses a virtual DOM and compare it to Svelte's approach. \
             Consider the trade-offs for performance and developer experience.",
        );
        assert!(
            result.total >= 20,
            "Expected score >= 20, got {}",
            result.total
        );
        assert!(
            result.tier == Tier::Standard || result.tier == Tier::Pro,
            "Expected Standard or Pro, got {:?}",
            result.tier
        );
    }

    #[test]
    fn score_security_audit_prompt_is_at_least_standard() {
        let result = score_complexity(
            "Analyze this Solidity contract for reentrancy vulnerabilities, \
             check for authentication bypass, and provide a security audit report.",
        );
        assert!(
            result.total >= 16,
            "Expected score >= 16, got {}",
            result.total
        );
    }

    // -----------------------------------------------------------------------
    // Score complexity: individual dimensions
    // -----------------------------------------------------------------------

    #[test]
    fn score_reasoning_dimension() {
        let result = score_complexity("Why is this better? Explain the trade-offs and compare");
        let reasoning = result
            .components
            .get("reasoning_words")
            .copied()
            .unwrap_or(0);
        assert!(
            reasoning >= 100,
            "Expected reasoning >= 100, got {reasoning}"
        );
    }

    #[test]
    fn score_multi_step_dimension() {
        let result = score_complexity(
            "First, read the file at src/auth.ts. Then analyze it for security issues. \
             After that, write a detailed report.",
        );
        let multi_step = result.components.get("multi_step").copied().unwrap_or(0);
        assert!(
            multi_step >= 100,
            "Expected multi_step >= 100, got {multi_step}"
        );
        assert!(result.hints.iter().any(|h| h.contains("multi_step")));
    }

    #[test]
    fn score_code_dimension() {
        let result = score_complexity("Fix the bug in the async function, refactor the module");
        let code = result
            .components
            .get("code_indicators")
            .copied()
            .unwrap_or(0);
        assert!(code >= 50, "Expected code_indicators >= 50, got {code}");
    }

    #[test]
    fn score_safety_dimension() {
        let result = score_complexity("Store the password and encrypt the auth token");
        let safety = result
            .components
            .get("safety_sensitivity")
            .copied()
            .unwrap_or(0);
        assert!(safety >= 100, "Expected safety >= 100, got {safety}");
    }

    #[test]
    fn score_domain_dimension() {
        let result = score_complexity("Deploy the kubernetes cluster on aws with terraform");
        let domain = result
            .components
            .get("domain_specific")
            .copied()
            .unwrap_or(0);
        assert!(
            domain >= 100,
            "Expected domain_specific >= 100, got {domain}"
        );
    }

    #[test]
    fn score_creativity_dimension() {
        let result = score_complexity("Write a blog post about design patterns, then summarize");
        let creativity = result.components.get("creativity").copied().unwrap_or(0);
        assert!(
            creativity >= 100,
            "Expected creativity >= 100, got {creativity}"
        );
    }

    #[test]
    fn score_question_complexity_dimension() {
        let result = score_complexity("Why does this fail? How can I fix it? What if I try X?");
        let qc = result
            .components
            .get("question_complexity")
            .copied()
            .unwrap_or(0);
        assert!(qc >= 60, "Expected question_complexity >= 60, got {qc}");
        assert!(
            result
                .hints
                .iter()
                .any(|h| h.contains("Multiple questions"))
        );
    }

    #[test]
    fn score_sentence_complexity_dimension() {
        let result = score_complexity(
            "This is complex, because it has commas, and conjunctions, \
             however it also has semicolons; moreover, it keeps going, and going",
        );
        let sc = result
            .components
            .get("sentence_complexity")
            .copied()
            .unwrap_or(0);
        assert!(sc >= 60, "Expected sentence_complexity >= 60, got {sc}");
    }

    #[test]
    fn score_token_estimate_for_long_prompt() {
        let long_prompt = "a ".repeat(300); // 600 chars
        let result = score_complexity(&long_prompt);
        let token = result
            .components
            .get("token_estimate")
            .copied()
            .unwrap_or(0);
        assert!(token >= 80, "Expected token_estimate >= 80, got {token}");
    }

    #[test]
    fn score_token_estimate_for_short_prompt() {
        let result = score_complexity("hi");
        let token = result
            .components
            .get("token_estimate")
            .copied()
            .unwrap_or(0);
        assert_eq!(token, 0, "Expected token_estimate == 0, got {token}");
    }

    // -----------------------------------------------------------------------
    // Score complexity: multi-dimensional boost
    // -----------------------------------------------------------------------

    #[test]
    fn score_multi_dimensional_boost() {
        // This triggers reasoning, multi-step, code, domain, creativity, safety
        let result = score_complexity(
            "First, explain why the kubernetes deployment fails. \
             Then refactor the auth module to fix the vulnerability. \
             After that, write a security report comparing the approaches.",
        );
        assert!(
            result.hints.iter().any(|h| h.contains("Multi-dimensional")),
            "Expected multi-dimensional boost, hints: {:?}",
            result.hints
        );
    }

    // -----------------------------------------------------------------------
    // Score complexity: explicit tier hint
    // -----------------------------------------------------------------------

    #[test]
    fn score_explicit_tier_hint_flash() {
        let result = score_complexity("[tier:flash] This looks complex but override to flash");
        assert_eq!(result.tier, Tier::Flash);
        assert!(
            result
                .hints
                .iter()
                .any(|h| h.contains("Explicit tier hint"))
        );
    }

    #[test]
    fn score_explicit_tier_hint_frontier() {
        let result = score_complexity("[tier:frontier] Simple question but I want the best");
        assert_eq!(result.tier, Tier::Frontier);
    }

    #[test]
    fn score_explicit_tier_hint_case_insensitive() {
        let result = score_complexity("[tier:PRO] some message");
        assert_eq!(result.tier, Tier::Pro);
    }

    // -----------------------------------------------------------------------
    // Score complexity: custom domain keywords
    // -----------------------------------------------------------------------

    #[test]
    fn score_custom_domain_keywords_override_defaults() {
        // Default keywords should match "kubernetes"
        let default_result = score_complexity("How do I deploy kubernetes?");
        let default_domain = default_result
            .components
            .get("domain_specific")
            .copied()
            .unwrap_or(0);
        assert!(
            default_domain > 0,
            "Default keywords should match 'kubernetes'"
        );

        // Custom keywords that DON'T include kubernetes
        let config = ScorerConfig {
            weights: ScorerWeights::default(),
            domain_keywords: Some(vec!["mycompany".to_string(), "myproduct".to_string()]),
        };
        let custom_result = score_complexity_with_config("How do I deploy kubernetes?", &config);
        let custom_domain = custom_result
            .components
            .get("domain_specific")
            .copied()
            .unwrap_or(0);
        assert_eq!(
            custom_domain, 0,
            "Custom keywords shouldn't match 'kubernetes'"
        );

        // Custom keywords should match their own terms
        let custom_result2 =
            score_complexity_with_config("Tell me about myproduct features", &config);
        let custom_domain2 = custom_result2
            .components
            .get("domain_specific")
            .copied()
            .unwrap_or(0);
        assert!(
            custom_domain2 > 0,
            "Custom keywords should match 'myproduct'"
        );
    }

    // -----------------------------------------------------------------------
    // Score complexity: edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn score_whitespace_only_is_flash() {
        let result = score_complexity("   \n\t  ");
        assert_eq!(result.tier, Tier::Flash);
    }

    #[test]
    fn score_single_word_no_keywords() {
        let result = score_complexity("banana");
        assert!(
            result.tier == Tier::Flash || result.tier == Tier::Standard,
            "Single non-keyword word should be Flash or Standard, got {:?}",
            result.tier
        );
    }

    #[test]
    fn score_very_long_prompt_is_at_least_standard() {
        let long = "Tell me about ".to_string() + &"things ".repeat(200);
        let result = score_complexity(&long);
        assert!(
            result.total >= 16,
            "Very long prompt should score at least Standard, got {}",
            result.total
        );
    }

    #[test]
    fn score_all_dimensions_have_entries() {
        let result = score_complexity(
            "First, explain why the function fails. Then write a fix and deploy it.",
        );
        let expected_keys = [
            "reasoning_words",
            "token_estimate",
            "code_indicators",
            "multi_step",
            "domain_specific",
            "ambiguity",
            "creativity",
            "precision",
            "context_dependency",
            "tool_likelihood",
            "safety_sensitivity",
            "question_complexity",
            "sentence_complexity",
        ];
        for key in &expected_keys {
            assert!(
                result.components.contains_key(*key),
                "Missing component: {key}"
            );
        }
    }

    #[test]
    fn score_is_clamped_to_100() {
        // Trigger every dimension hard
        let prompt = "First, explain why the kubernetes docker terraform deployment on aws fails. \
             Then analyze the security vulnerability and compare the trade-offs. \
             After that, write a detailed blog post report with code examples: \
             ```rust\nfn main() {}\n``` \
             Calculate exactly how many steps are needed? Why? How? \
             Deploy to production mainnet. Review the authentication token password.";
        let result = score_complexity(prompt);
        assert!(
            result.total <= 100,
            "Score should be clamped to 100, got {}",
            result.total
        );
    }

    // -----------------------------------------------------------------------
    // Pattern overrides
    // -----------------------------------------------------------------------

    #[test]
    fn pattern_override_greeting_is_simple() {
        let primary = Arc::new(StubLlm::new("p").with_model_name("primary"));
        let cheap = Arc::new(StubLlm::new("c").with_model_name("cheap"));
        let provider = SmartRoutingProvider::new(primary, cheap, default_config());

        let req = CompletionRequest::new(vec![ChatMessage::user("Hi")]);
        let complexity = provider.classify(&req);
        assert_eq!(complexity, TaskComplexity::Simple);
    }

    #[test]
    fn pattern_override_security_audit_is_complex() {
        let primary = Arc::new(StubLlm::new("p").with_model_name("primary"));
        let cheap = Arc::new(StubLlm::new("c").with_model_name("cheap"));
        let provider = SmartRoutingProvider::new(primary, cheap, default_config());

        let req = CompletionRequest::new(vec![ChatMessage::user(
            "Please do a security audit of this contract",
        )]);
        let complexity = provider.classify(&req);
        assert_eq!(complexity, TaskComplexity::Complex);
    }

    #[test]
    fn pattern_override_production_deploy_is_moderate() {
        let primary = Arc::new(StubLlm::new("p").with_model_name("primary"));
        let cheap = Arc::new(StubLlm::new("c").with_model_name("cheap"));
        let provider = SmartRoutingProvider::new(primary, cheap, default_config());

        let req = CompletionRequest::new(vec![ChatMessage::user("Deploy this to production")]);
        let complexity = provider.classify(&req);
        assert_eq!(complexity, TaskComplexity::Moderate);
    }

    #[test]
    fn pattern_override_time_question_is_simple() {
        let primary = Arc::new(StubLlm::new("p").with_model_name("primary"));
        let cheap = Arc::new(StubLlm::new("c").with_model_name("cheap"));
        let provider = SmartRoutingProvider::new(primary, cheap, default_config());

        let req = CompletionRequest::new(vec![ChatMessage::user("What time is it?")]);
        let complexity = provider.classify(&req);
        assert_eq!(complexity, TaskComplexity::Simple);
    }

    #[test]
    fn pattern_override_time_does_not_match_complex_questions() {
        // The quick-lookup override regex should NOT match "What time complexity..."
        // because it's end-anchored. Verify the regex itself doesn't fire.
        let overrides = &*DEFAULT_OVERRIDES;
        let lookup_override = overrides
            .iter()
            .find(|po| po.tier == Tier::Flash && po.regex.as_str().contains("time"))
            .expect("time lookup override exists");

        assert!(
            !lookup_override
                .regex
                .is_match("What time complexity is merge sort?"),
            "Time override should not match 'What time complexity is merge sort?'"
        );
        // But it should still match actual time lookups
        assert!(lookup_override.regex.is_match("What time is it?"));
        assert!(lookup_override.regex.is_match("what's the date today?"));
    }

    #[test]
    fn empty_domain_keywords_uses_defaults() {
        // An empty custom keywords list should fall back to defaults, not produce
        // a broken regex that matches empty strings everywhere.
        let config = ScorerConfig {
            domain_keywords: Some(vec![]),
            ..ScorerConfig::default()
        };
        let result = score_complexity_with_config("deploy kubernetes to mainnet", &config);
        // Should still detect domain keywords via the default fallback
        assert!(
            result
                .components
                .get("domain_specific")
                .copied()
                .unwrap_or(0)
                > 0,
            "Empty custom keywords should fall back to defaults"
        );
    }

    // -----------------------------------------------------------------------
    // Tier → TaskComplexity mapping
    // -----------------------------------------------------------------------

    #[test]
    fn tier_to_task_complexity_mapping() {
        assert_eq!(TaskComplexity::from(Tier::Flash), TaskComplexity::Simple);
        assert_eq!(TaskComplexity::from(Tier::Standard), TaskComplexity::Simple);
        assert_eq!(TaskComplexity::from(Tier::Pro), TaskComplexity::Moderate);
        assert_eq!(
            TaskComplexity::from(Tier::Frontier),
            TaskComplexity::Complex
        );
    }

    #[test]
    fn tier_from_score_boundaries() {
        assert_eq!(Tier::from_score(0), Tier::Flash);
        assert_eq!(Tier::from_score(15), Tier::Flash);
        assert_eq!(Tier::from_score(16), Tier::Standard);
        assert_eq!(Tier::from_score(40), Tier::Standard);
        assert_eq!(Tier::from_score(41), Tier::Pro);
        assert_eq!(Tier::from_score(65), Tier::Pro);
        assert_eq!(Tier::from_score(66), Tier::Frontier);
        assert_eq!(Tier::from_score(100), Tier::Frontier);
    }

    #[test]
    fn tier_display() {
        assert_eq!(Tier::Flash.as_str(), "flash");
        assert_eq!(Tier::Frontier.to_string(), "frontier");
    }

    // -----------------------------------------------------------------------
    // Uncertainty detection
    // -----------------------------------------------------------------------

    #[test]
    fn detects_uncertain_short_response() {
        let response = CompletionResponse {
            content: "I'm not sure.".to_string(),
            input_tokens: 10,
            output_tokens: 5,
            finish_reason: crate::llm::FinishReason::Stop,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
        };
        assert!(SmartRoutingProvider::response_is_uncertain(&response));
    }

    #[test]
    fn detects_empty_response_as_uncertain() {
        let response = CompletionResponse {
            content: "".to_string(),
            input_tokens: 10,
            output_tokens: 0,
            finish_reason: crate::llm::FinishReason::Stop,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
        };
        assert!(SmartRoutingProvider::response_is_uncertain(&response));
    }

    #[test]
    fn short_confident_response_is_not_uncertain() {
        let response = CompletionResponse {
            content: "Yes.".to_string(),
            input_tokens: 10,
            output_tokens: 1,
            finish_reason: crate::llm::FinishReason::Stop,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
        };
        assert!(!SmartRoutingProvider::response_is_uncertain(&response));
    }

    #[test]
    fn confident_response_is_not_uncertain() {
        let response = CompletionResponse {
            content: "The answer is 42. This is a well-known constant from the Hitchhiker's Guide."
                .to_string(),
            input_tokens: 10,
            output_tokens: 20,
            finish_reason: crate::llm::FinishReason::Stop,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
        };
        assert!(!SmartRoutingProvider::response_is_uncertain(&response));
    }

    // -----------------------------------------------------------------------
    // Provider routing tests
    // -----------------------------------------------------------------------

    fn make_request(content: &str) -> CompletionRequest {
        CompletionRequest::new(vec![ChatMessage::user(content)])
    }

    fn make_tool_request() -> ToolCompletionRequest {
        ToolCompletionRequest::new(vec![ChatMessage::user("implement a search")], vec![])
    }

    #[tokio::test]
    async fn simple_task_routes_to_cheap() {
        let primary = Arc::new(StubLlm::new("primary-response").with_model_name("primary"));
        let cheap = Arc::new(StubLlm::new("cheap-response").with_model_name("cheap"));

        let router = SmartRoutingProvider::new(
            primary.clone(),
            cheap.clone(),
            SmartRoutingConfig {
                cascade_enabled: false,
                ..default_config()
            },
        );

        let resp = router.complete(make_request("hello")).await.unwrap();
        assert_eq!(resp.content, "cheap-response");
        assert_eq!(cheap.calls(), 1);
        assert_eq!(primary.calls(), 0);
    }

    #[tokio::test]
    async fn complex_task_routes_to_primary() {
        let primary = Arc::new(StubLlm::new("primary-response").with_model_name("primary"));
        let cheap = Arc::new(StubLlm::new("cheap-response").with_model_name("cheap"));

        let router = SmartRoutingProvider::new(primary.clone(), cheap.clone(), default_config());

        // Security audit triggers Frontier via pattern override → Complex → primary
        let resp = router
            .complete(make_request(
                "Please do a security audit of this smart contract",
            ))
            .await
            .unwrap();
        assert_eq!(resp.content, "primary-response");
        assert_eq!(primary.calls(), 1);
        assert_eq!(cheap.calls(), 0);
    }

    #[tokio::test]
    async fn tool_use_always_routes_to_primary() {
        let primary = Arc::new(StubLlm::new("primary-response").with_model_name("primary"));
        let cheap = Arc::new(StubLlm::new("cheap-response").with_model_name("cheap"));

        let router = SmartRoutingProvider::new(primary.clone(), cheap.clone(), default_config());

        let resp = router
            .complete_with_tools(make_tool_request())
            .await
            .unwrap();
        assert_eq!(resp.content, Some("primary-response".to_string()));
        assert_eq!(primary.calls(), 1);
        assert_eq!(cheap.calls(), 0);
    }

    #[tokio::test]
    async fn stats_increment_correctly() {
        let primary = Arc::new(StubLlm::new("primary").with_model_name("primary"));
        let cheap = Arc::new(StubLlm::new("cheap").with_model_name("cheap"));

        let router = SmartRoutingProvider::new(
            primary,
            cheap,
            SmartRoutingConfig {
                cascade_enabled: false,
                ..default_config()
            },
        );

        // Simple → cheap (greeting pattern override)
        router.complete(make_request("hello")).await.unwrap();
        // Complex → primary (security audit pattern override → Frontier)
        router
            .complete(make_request("security audit review"))
            .await
            .unwrap();
        // Tool use → primary
        router
            .complete_with_tools(make_tool_request())
            .await
            .unwrap();

        let stats = router.stats();
        assert_eq!(stats.total_requests, 3);
        assert_eq!(stats.cheap_requests, 1);
        assert_eq!(stats.primary_requests, 2);
        assert_eq!(stats.cascade_escalations, 0);
    }

    #[tokio::test]
    async fn cascade_escalates_on_uncertain_response() {
        let primary = Arc::new(StubLlm::new("primary-response").with_model_name("primary"));
        let cheap = Arc::new(StubLlm::new("I'm not sure about that.").with_model_name("cheap"));

        let router = SmartRoutingProvider::new(
            primary.clone(),
            cheap.clone(),
            SmartRoutingConfig {
                cascade_enabled: true,
                ..default_config()
            },
        );

        // A Pro-tier task (triggers Moderate → cascade)
        let resp = router
            .complete(make_request("Deploy this to production"))
            .await
            .unwrap();

        // Should have escalated to primary
        assert_eq!(resp.content, "primary-response");
        assert_eq!(cheap.calls(), 1);
        assert_eq!(primary.calls(), 1);

        let stats = router.stats();
        assert_eq!(stats.cascade_escalations, 1);
    }

    #[tokio::test]
    async fn cascade_does_not_escalate_on_confident_response() {
        let primary = Arc::new(StubLlm::new("primary-response").with_model_name("primary"));
        let cheap = Arc::new(
            StubLlm::new("Deployed successfully to production mainnet.").with_model_name("cheap"),
        );

        let router = SmartRoutingProvider::new(
            primary.clone(),
            cheap.clone(),
            SmartRoutingConfig {
                cascade_enabled: true,
                ..default_config()
            },
        );

        let resp = router
            .complete(make_request("Deploy this to production"))
            .await
            .unwrap();

        // Should NOT have escalated
        assert!(resp.content.contains("Deployed successfully"));
        assert_eq!(cheap.calls(), 1);
        assert_eq!(primary.calls(), 0);

        let stats = router.stats();
        assert_eq!(stats.cascade_escalations, 0);
    }

    #[tokio::test]
    async fn model_name_returns_primary() {
        let primary = Arc::new(StubLlm::new("ok").with_model_name("sonnet"));
        let cheap = Arc::new(StubLlm::new("ok").with_model_name("haiku"));

        let router = SmartRoutingProvider::new(primary, cheap, default_config());
        assert_eq!(router.model_name(), "sonnet");
        assert_eq!(router.active_model_name(), "sonnet");
    }

    #[tokio::test]
    async fn tier_hint_overrides_pattern_override() {
        // "[tier:flash] security audit review" has both a Flash tier hint and
        // a Frontier pattern override. Tier hints should win.
        let primary = Arc::new(StubLlm::new("primary").with_model_name("primary"));
        let cheap = Arc::new(StubLlm::new("cheap").with_model_name("cheap"));

        let router = SmartRoutingProvider::new(
            primary.clone(),
            cheap.clone(),
            SmartRoutingConfig {
                cascade_enabled: false,
                ..default_config()
            },
        );

        router
            .complete(make_request("[tier:flash] security audit review"))
            .await
            .unwrap();

        // Tier hint → Flash → Simple → cheap model
        assert_eq!(cheap.calls(), 1);
        assert_eq!(primary.calls(), 0);
    }

    #[tokio::test]
    async fn trimmed_greeting_matches_override() {
        // Trailing whitespace should not prevent the greeting override from matching.
        let primary = Arc::new(StubLlm::new("primary").with_model_name("primary"));
        let cheap = Arc::new(StubLlm::new("cheap").with_model_name("cheap"));

        let router = SmartRoutingProvider::new(
            primary.clone(),
            cheap.clone(),
            SmartRoutingConfig {
                cascade_enabled: false,
                ..default_config()
            },
        );

        router.complete(make_request("  hello  \n")).await.unwrap();

        // Should match greeting override → Flash → Simple → cheap model
        assert_eq!(cheap.calls(), 1);
        assert_eq!(primary.calls(), 0);
    }
}
