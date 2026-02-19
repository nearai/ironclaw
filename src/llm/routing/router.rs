//! Model router for smart model selection.
//!
//! Routes requests to appropriate models based on complexity scoring
//! and pattern-based overrides.

use lazy_static::lazy_static;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::scorer::{score_complexity, ScoreBreakdown, Tier};

/// Configuration for a pattern override.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternOverride {
    /// Regex pattern to match against the prompt.
    pub pattern: String,
    /// Tier to force when pattern matches.
    pub tier: String,
}

/// Configuration for the model router.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouterConfig {
    /// Whether routing is enabled.
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// Mapping from tier name to model identifier.
    /// Example: { "flash": "haiku-latest", "standard": "sonnet-latest" }
    #[serde(default)]
    pub tiers: HashMap<String, String>,

    /// Thinking mode per tier.
    /// Example: { "pro": "low", "frontier": "medium" }
    #[serde(default)]
    pub thinking: HashMap<String, String>,

    /// Pattern overrides that bypass scoring.
    #[serde(default)]
    pub overrides: Vec<PatternOverride>,
}

fn default_enabled() -> bool {
    true
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            tiers: HashMap::new(),
            thinking: HashMap::new(),
            overrides: vec![],
        }
    }
}

/// Result of routing a request.
#[derive(Debug, Clone)]
pub struct RoutingDecision {
    /// Selected model identifier.
    pub model: String,
    /// Selected tier.
    pub tier: Tier,
    /// Thinking mode for this tier (if any).
    pub thinking: Option<String>,
    /// Complexity score breakdown.
    pub score: ScoreBreakdown,
    /// Human-readable reason for this decision.
    pub reason: String,
    /// Fallback models if primary fails.
    pub fallbacks: Vec<String>,
}

lazy_static! {
    // Default pattern overrides (compiled once)
    static ref DEFAULT_OVERRIDES: Vec<(Regex, Tier)> = vec![
        // Flash tier: greetings and acknowledgments
        (Regex::new(r"(?i)^(hi|hello|hey|thanks|ok|sure|yes|no|yep|nope|cool|nice|great|got it)$").unwrap(), Tier::Flash),
        // Flash tier: quick lookups
        (Regex::new(r"(?i)^what.*(time|date|day|weather)").unwrap(), Tier::Flash),
        // Frontier tier: security audits
        (Regex::new(r"(?i)security.*(audit|review|scan)").unwrap(), Tier::Frontier),
        (Regex::new(r"(?i)vulnerabilit(y|ies).*(review|scan|check|audit)").unwrap(), Tier::Frontier),
        // Pro tier: production deployments
        (Regex::new(r"(?i)deploy.*(mainnet|production)").unwrap(), Tier::Pro),
        (Regex::new(r"(?i)production.*(deploy|release|push)").unwrap(), Tier::Pro),
        // Standard tier: vulnerability mentions (without audit)
        (Regex::new(r"(?i)vulnerabilit(y|ies)").unwrap(), Tier::Standard),
        // Standard tier: batch creation tasks
        (Regex::new(r"(?i)create.*(files|posts|documents)").unwrap(), Tier::Standard),
        (Regex::new(r"(?i)rewrite.*(all|multiple|\d+)").unwrap(), Tier::Standard),
    ];
}

/// Model router for smart model selection.
pub struct Router {
    config: RouterConfig,
    /// Compiled user overrides.
    user_overrides: Vec<(Regex, Tier)>,
}

impl Router {
    /// Create a new router with the given configuration.
    pub fn new(config: RouterConfig) -> Self {
        // Compile user-provided pattern overrides
        let user_overrides: Vec<(Regex, Tier)> = config
            .overrides
            .iter()
            .filter_map(|o| {
                let tier = match o.tier.to_lowercase().as_str() {
                    "flash" => Tier::Flash,
                    "standard" => Tier::Standard,
                    "pro" => Tier::Pro,
                    "frontier" => Tier::Frontier,
                    _ => return None,
                };
                Regex::new(&o.pattern).ok().map(|re| (re, tier))
            })
            .collect();

        Self {
            config,
            user_overrides,
        }
    }

    /// Create a router with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(RouterConfig::default())
    }

    /// Check if routing is enabled.
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Route a prompt to determine the appropriate model.
    ///
    /// If routing is disabled, returns Standard tier with default model.
    pub fn route(&self, prompt: &str) -> RoutingDecision {
        // If routing disabled, return standard tier
        if !self.config.enabled {
            return self.make_decision(Tier::Standard, "Routing disabled".to_string());
        }

        // Check user overrides first
        for (re, tier) in &self.user_overrides {
            if re.is_match(prompt) {
                return self.make_decision(*tier, format!("User override: {}", re.as_str()));
            }
        }

        // Check default overrides
        for (re, tier) in DEFAULT_OVERRIDES.iter() {
            if re.is_match(prompt) {
                return self.make_decision(*tier, format!("Pattern override: {}", re.as_str()));
            }
        }

        // Fall back to complexity scoring
        let score = score_complexity(prompt);
        let tier = score.tier;
        let reason = format!("Complexity score: {}/100", score.total);

        self.make_decision_with_score(tier, reason, score)
    }

    /// Get the model for a tier.
    pub fn model_for_tier(&self, tier: Tier) -> String {
        self.config
            .tiers
            .get(tier.as_str())
            .cloned()
            .unwrap_or_else(|| self.default_model_for_tier(tier))
    }

    /// Get thinking mode for a tier.
    pub fn thinking_for_tier(&self, tier: Tier) -> Option<String> {
        self.config.thinking.get(tier.as_str()).cloned()
    }

    /// Default model mapping when not configured.
    fn default_model_for_tier(&self, tier: Tier) -> String {
        match tier {
            Tier::Flash => "haiku-latest".to_string(),
            Tier::Standard => "sonnet-latest".to_string(),
            Tier::Pro => "sonnet-latest".to_string(),
            Tier::Frontier => "opus-latest".to_string(),
        }
    }

    /// Create a routing decision from a tier and reason.
    fn make_decision(&self, tier: Tier, reason: String) -> RoutingDecision {
        let score = ScoreBreakdown {
            total: tier.to_score(),
            tier,
            components: std::collections::HashMap::new(),
            hints: vec![reason.clone()],
        };
        self.make_decision_with_score(tier, reason, score)
    }

    /// Create a routing decision with a pre-computed score.
    fn make_decision_with_score(
        &self,
        tier: Tier,
        reason: String,
        score: ScoreBreakdown,
    ) -> RoutingDecision {
        let model = self.model_for_tier(tier);
        let thinking = self.thinking_for_tier(tier);

        // Gather fallbacks from other models in same or adjacent tiers
        let fallbacks = self.get_fallbacks(tier, &model);

        RoutingDecision {
            model,
            tier,
            thinking,
            score,
            reason,
            fallbacks,
        }
    }

    /// Get fallback models for a tier.
    fn get_fallbacks(&self, tier: Tier, primary: &str) -> Vec<String> {
        // For now, just include the default model if different from selected
        let default = self.default_model_for_tier(tier);
        if default != primary {
            vec![default]
        } else {
            vec![]
        }
    }
}

impl Default for Router {
    fn default() -> Self {
        Self::with_defaults()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_greeting_routes_to_flash() {
        let router = Router::with_defaults();
        let decision = router.route("Hi");
        assert_eq!(decision.tier, Tier::Flash);
        assert!(decision.reason.contains("Pattern override"));
    }

    #[test]
    fn test_time_question_routes_to_flash() {
        let router = Router::with_defaults();
        let decision = router.route("What time is it?");
        assert_eq!(decision.tier, Tier::Flash);
    }

    #[test]
    fn test_security_audit_routes_to_frontier() {
        let router = Router::with_defaults();
        let decision = router.route("Please do a security audit of this contract");
        assert_eq!(decision.tier, Tier::Frontier);
    }

    #[test]
    fn test_production_deploy_routes_to_pro() {
        let router = Router::with_defaults();
        let decision = router.route("Deploy this to production");
        assert_eq!(decision.tier, Tier::Pro);
    }

    #[test]
    fn test_complex_prompt_uses_scoring() {
        let router = Router::with_defaults();
        let decision = router.route(
            "Explain the trade-offs between React and Svelte for a large-scale application",
        );
        // Should use scoring, not pattern override
        assert!(decision.reason.contains("Complexity score"));
    }

    #[test]
    fn test_custom_tier_mapping() {
        let mut config = RouterConfig::default();
        config.tiers.insert("flash".to_string(), "gpt-4o-mini".to_string());
        config.tiers.insert("frontier".to_string(), "o3-high".to_string());

        let router = Router::new(config);

        let flash_decision = router.route("Hi");
        assert_eq!(flash_decision.model, "gpt-4o-mini");

        let frontier_decision = router.route("security audit review");
        assert_eq!(frontier_decision.model, "o3-high");
    }

    #[test]
    fn test_thinking_mode() {
        let mut config = RouterConfig::default();
        config.thinking.insert("pro".to_string(), "low".to_string());
        config.thinking.insert("frontier".to_string(), "high".to_string());

        let router = Router::new(config);

        let frontier_decision = router.route("security audit");
        assert_eq!(frontier_decision.thinking, Some("high".to_string()));

        let flash_decision = router.route("Hi");
        assert_eq!(flash_decision.thinking, None);
    }

    #[test]
    fn test_user_override() {
        let config = RouterConfig {
            enabled: true,
            tiers: HashMap::new(),
            thinking: HashMap::new(),
            overrides: vec![PatternOverride {
                pattern: r"(?i)my-special-pattern".to_string(),
                tier: "frontier".to_string(),
            }],
        };

        let router = Router::new(config);
        let decision = router.route("This contains my-special-pattern somewhere");
        assert_eq!(decision.tier, Tier::Frontier);
        assert!(decision.reason.contains("User override"));
    }

    #[test]
    fn test_disabled_returns_default() {
        let config = RouterConfig {
            enabled: false,
            ..Default::default()
        };
        let router = Router::new(config);
        assert!(!router.is_enabled());
    }
}
