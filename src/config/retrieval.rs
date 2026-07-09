use crate::config::helpers::{optional_env, parse_optional_env};
use crate::error::ConfigError;
use crate::settings::Settings;

/// Per-turn tool-retrieval configuration (env + default only).
#[derive(Debug, Clone)]
pub struct RetrievalConfig {
    /// Master flag; false = inject all visible tools (current behavior).
    pub enabled: bool,
    /// Max retrieved tools per turn (in addition to the core set).
    pub top_k: usize,
    /// Cosine similarity floor for a tool to be eligible.
    pub min_score: f32,
    /// Always-injected tool names, regardless of score.
    pub core_set: Vec<String>,
}

fn default_core_set() -> Vec<String> {
    [
        // Discovery escape hatch: always advertised so the model can find and
        // then call any tool that per-turn narrowing did not surface. Without
        // these, retrieval can silently hide a needed capability.
        "find_tools",
        "tool_info",
        // Core memory + messaging surface.
        "memory_search",
        "memory_write",
        "memory_tree",
        "memory_read",
        "message",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

impl Default for RetrievalConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            top_k: 10,
            min_score: 0.2,
            core_set: default_core_set(),
        }
    }
}

/// Split a comma-separated list into trimmed, non-empty names.
pub fn parse_core_set(s: &str) -> Vec<String> {
    s.split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect()
}

impl RetrievalConfig {
    pub(crate) fn resolve(_settings: &Settings) -> Result<Self, ConfigError> {
        let defaults = Self::default();
        let core_set = match optional_env("TOOL_CORE_SET")? {
            Some(raw) => parse_core_set(&raw),
            None => defaults.core_set,
        };
        Ok(Self {
            enabled: parse_optional_env("TOOL_RETRIEVAL_ENABLED", defaults.enabled)?,
            top_k: parse_optional_env("TOOL_RETRIEVAL_TOP_K", defaults.top_k)?,
            min_score: parse_optional_env("TOOL_RETRIEVAL_MIN_SCORE", defaults.min_score)?,
            core_set,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retrieval_config_defaults() {
        let c = RetrievalConfig::default();
        assert!(c.enabled);
        assert_eq!(c.top_k, 10);
        assert!((c.min_score - 0.2).abs() < 1e-6);
        assert!(c.core_set.contains(&"memory_tree".to_string()));
        // Discovery escape hatch must always be in core so retrieval can never
        // hide a needed tool beyond recovery.
        assert!(c.core_set.contains(&"find_tools".to_string()));
        assert!(c.core_set.contains(&"tool_info".to_string()));
        assert_eq!(c.core_set.len(), 7);
    }

    #[test]
    fn parse_core_set_trims_and_drops_empties() {
        assert_eq!(
            parse_core_set("a, b ,c"),
            vec!["a".to_string(), "b".to_string(), "c".to_string()]
        );
        assert_eq!(
            parse_core_set(" x ,, y ,"),
            vec!["x".to_string(), "y".to_string()]
        );
        assert!(parse_core_set("").is_empty());
    }
}
