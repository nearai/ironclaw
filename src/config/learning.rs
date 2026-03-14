use crate::config::helpers::{parse_bool_env, parse_optional_env};
use crate::error::ConfigError;

/// Configuration for the adaptive learning subsystem.
#[derive(Debug, Clone)]
pub struct LearningConfig {
    /// Whether the learning system is enabled.
    pub enabled: bool,
    /// Minimum tool calls for a "complex tool chain" detection.
    pub min_tool_calls: usize,
    /// Minimum unique tools for a "novel combination" detection.
    pub min_unique_tools: usize,
    /// Minimum quality score for automatic detection (0-100).
    pub min_quality_score: u32,
    /// Minimum turns for any detection (except user-requested).
    pub min_turns: usize,
    /// Maximum synthesized skills per user.
    pub max_skills_per_user: usize,
    /// Maximum synthesized skill size in bytes.
    pub max_skill_size: usize,
}

impl Default for LearningConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            min_tool_calls: 3,
            min_unique_tools: 2,
            min_quality_score: 75,
            min_turns: 2,
            max_skills_per_user: 50,
            max_skill_size: 16 * 1024,
        }
    }
}

impl LearningConfig {
    pub(crate) fn resolve() -> Result<Self, ConfigError> {
        Ok(Self {
            enabled: parse_bool_env("LEARNING_ENABLED", false)?,
            min_tool_calls: parse_optional_env("LEARNING_MIN_TOOL_CALLS", 3)?,
            min_unique_tools: parse_optional_env("LEARNING_MIN_UNIQUE_TOOLS", 2)?,
            min_quality_score: parse_optional_env("LEARNING_MIN_QUALITY_SCORE", 75)?,
            min_turns: parse_optional_env("LEARNING_MIN_TURNS", 2)?,
            max_skills_per_user: parse_optional_env("LEARNING_MAX_SKILLS_PER_USER", 50)?,
            max_skill_size: parse_optional_env("LEARNING_MAX_SKILL_SIZE", 16 * 1024)?,
        })
    }
}
