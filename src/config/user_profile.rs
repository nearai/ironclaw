use crate::config::helpers::{parse_bool_env, parse_optional_env};
use crate::error::ConfigError;

/// Configuration for the user profile engine.
#[derive(Debug, Clone)]
pub struct UserProfileConfig {
    /// Whether user profile learning is enabled.
    pub enabled: bool,
    /// Maximum characters for profile injection into system prompt.
    pub max_prompt_chars: usize,
    /// Minimum turns between profile distillation runs.
    pub distill_interval_turns: usize,
    /// Maximum profile facts per user.
    pub max_facts_per_user: usize,
}

impl Default for UserProfileConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_prompt_chars: 2000,
            distill_interval_turns: 5,
            max_facts_per_user: 100,
        }
    }
}

impl UserProfileConfig {
    pub(crate) fn resolve() -> Result<Self, ConfigError> {
        Ok(Self {
            enabled: parse_bool_env("USER_PROFILE_ENABLED", false)?,
            max_prompt_chars: parse_optional_env("USER_PROFILE_MAX_PROMPT_CHARS", 2000)?,
            distill_interval_turns: parse_optional_env("USER_PROFILE_DISTILL_INTERVAL", 5)?,
            max_facts_per_user: parse_optional_env("USER_PROFILE_MAX_FACTS", 100)?,
        })
    }
}
