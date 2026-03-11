//! Memory fact extraction configuration.

use crate::config::helpers::{optional_env, parse_bool_env, parse_optional_env};
use crate::error::ConfigError;
use crate::settings::Settings;

/// Configuration for automatic fact extraction from conversations.
#[derive(Debug, Clone)]
pub struct MemoryConfig {
    /// Whether automatic fact extraction is enabled.
    pub extraction_enabled: bool,
    /// Model to use for extraction (e.g., "claude-3-5-haiku-20241022").
    /// If None, uses the default model.
    pub extraction_model: Option<String>,
    /// Maximum number of facts to inject into the system prompt.
    pub max_facts_in_context: usize,
    /// TTL for facts in days (0 = no expiry).
    pub fact_ttl_days: u64,
    /// Minimum number of messages in a session to trigger extraction.
    pub extraction_min_messages: usize,
    /// Maximum facts to extract per session.
    pub max_facts_per_session: usize,
    /// Cosine similarity threshold for dedup (0.0-1.0).
    pub dedup_threshold: f32,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            extraction_enabled: false,
            extraction_model: None,
            max_facts_in_context: 15,
            fact_ttl_days: 0,
            extraction_min_messages: 5,
            max_facts_per_session: 15,
            dedup_threshold: 0.85,
        }
    }
}

impl MemoryConfig {
    pub(crate) fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
        Ok(Self {
            extraction_enabled: parse_bool_env(
                "MEMORY_EXTRACTION_ENABLED",
                settings.memory.extraction_enabled,
            )?,
            extraction_model: optional_env("MEMORY_EXTRACTION_MODEL")?
                .or_else(|| settings.memory.extraction_model.clone()),
            max_facts_in_context: parse_optional_env(
                "MEMORY_MAX_FACTS_IN_CONTEXT",
                settings.memory.max_facts_in_context,
            )?,
            fact_ttl_days: parse_optional_env(
                "MEMORY_FACT_TTL_DAYS",
                settings.memory.fact_ttl_days,
            )?,
            extraction_min_messages: parse_optional_env(
                "MEMORY_EXTRACTION_MIN_MESSAGES",
                settings.memory.extraction_min_messages,
            )?,
            max_facts_per_session: parse_optional_env(
                "MEMORY_MAX_FACTS_PER_SESSION",
                settings.memory.max_facts_per_session,
            )?,
            dedup_threshold: {
                let val: f32 = parse_optional_env(
                    "MEMORY_DEDUP_THRESHOLD",
                    settings.memory.dedup_threshold,
                )?;
                val.clamp(0.0, 1.0)
            },
        })
    }
}
