use crate::config::helpers::{optional_env, parse_optional_env};
use crate::error::ConfigError;
use crate::workspace::FusionStrategy;

/// Workspace search configuration resolved from environment variables.
#[derive(Debug, Clone)]
pub struct WorkspaceSearchConfig {
    /// Fusion strategy: "rrf" or "weighted".
    pub fusion_strategy: FusionStrategy,
    /// RRF constant k (default 60).
    pub rrf_k: u32,
    /// FTS weight for weighted fusion (default 0.3).
    pub fts_weight: f32,
    /// Vector weight for weighted fusion (default 0.7).
    pub vector_weight: f32,
}

impl Default for WorkspaceSearchConfig {
    fn default() -> Self {
        Self {
            fusion_strategy: FusionStrategy::default(),
            rrf_k: 60,
            fts_weight: 0.3,
            vector_weight: 0.7,
        }
    }
}

impl WorkspaceSearchConfig {
    pub(crate) fn resolve() -> Result<Self, ConfigError> {
        let fusion_strategy = match optional_env("SEARCH_FUSION_STRATEGY")? {
            Some(s) => match s.to_lowercase().as_str() {
                "rrf" => FusionStrategy::Rrf,
                "weighted" => FusionStrategy::WeightedScore,
                other => {
                    return Err(ConfigError::InvalidValue {
                        key: "SEARCH_FUSION_STRATEGY".to_string(),
                        message: format!("must be 'rrf' or 'weighted', got '{other}'"),
                    });
                }
            },
            None => FusionStrategy::default(),
        };

        let rrf_k = parse_optional_env("SEARCH_RRF_K", 60u32)?;
        let fts_weight = parse_optional_env("SEARCH_FTS_WEIGHT", 0.3f32)?;
        let vector_weight = parse_optional_env("SEARCH_VECTOR_WEIGHT", 0.7f32)?;

        if !fts_weight.is_finite() || fts_weight < 0.0 {
            return Err(ConfigError::InvalidValue {
                key: "SEARCH_FTS_WEIGHT".to_string(),
                message: "must be a finite, non-negative float".to_string(),
            });
        }
        if !vector_weight.is_finite() || vector_weight < 0.0 {
            return Err(ConfigError::InvalidValue {
                key: "SEARCH_VECTOR_WEIGHT".to_string(),
                message: "must be a finite, non-negative float".to_string(),
            });
        }

        Ok(Self {
            fusion_strategy,
            rrf_k,
            fts_weight,
            vector_weight,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::helpers::ENV_MUTEX;

    fn clear_search_env() {
        // SAFETY: Only called under ENV_MUTEX in tests.
        unsafe {
            std::env::remove_var("SEARCH_FUSION_STRATEGY");
            std::env::remove_var("SEARCH_RRF_K");
            std::env::remove_var("SEARCH_FTS_WEIGHT");
            std::env::remove_var("SEARCH_VECTOR_WEIGHT");
        }
    }

    #[test]
    fn defaults_when_no_env() {
        let _guard = ENV_MUTEX.lock().expect("env mutex poisoned");
        clear_search_env();

        let config = WorkspaceSearchConfig::resolve().expect("should resolve");
        assert_eq!(config.fusion_strategy, FusionStrategy::Rrf);
        assert_eq!(config.rrf_k, 60);
        assert!((config.fts_weight - 0.3).abs() < 0.001);
        assert!((config.vector_weight - 0.7).abs() < 0.001);
    }

    #[test]
    fn env_overrides() {
        let _guard = ENV_MUTEX.lock().expect("env mutex poisoned");
        clear_search_env();

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::set_var("SEARCH_FUSION_STRATEGY", "weighted");
            std::env::set_var("SEARCH_RRF_K", "30");
            std::env::set_var("SEARCH_FTS_WEIGHT", "0.9");
            std::env::set_var("SEARCH_VECTOR_WEIGHT", "0.1");
        }

        let config = WorkspaceSearchConfig::resolve().expect("should resolve");
        assert_eq!(config.fusion_strategy, FusionStrategy::WeightedScore);
        assert_eq!(config.rrf_k, 30);
        assert!((config.fts_weight - 0.9).abs() < 0.001);
        assert!((config.vector_weight - 0.1).abs() < 0.001);

        clear_search_env();
    }

    #[test]
    fn invalid_strategy_rejected() {
        let _guard = ENV_MUTEX.lock().expect("env mutex poisoned");
        clear_search_env();

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::set_var("SEARCH_FUSION_STRATEGY", "bm25");
        }

        let result = WorkspaceSearchConfig::resolve();
        assert!(result.is_err());

        clear_search_env();
    }
}
