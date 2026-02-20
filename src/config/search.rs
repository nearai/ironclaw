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
    /// FTS weight for fusion.
    ///
    /// [`Default`] uses 0.5. [`resolve()`](Self::resolve) applies per-strategy
    /// defaults: 0.5 (RRF) or 0.3 (weighted).
    pub fts_weight: f32,
    /// Vector weight for fusion.
    ///
    /// [`Default`] uses 0.5. [`resolve()`](Self::resolve) applies per-strategy
    /// defaults: 0.5 (RRF) or 0.7 (weighted).
    pub vector_weight: f32,
}

impl Default for WorkspaceSearchConfig {
    fn default() -> Self {
        Self {
            fusion_strategy: FusionStrategy::default(),
            rrf_k: 60,
            fts_weight: 0.5,
            vector_weight: 0.5,
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

        // Per-strategy weight defaults: RRF uses 0.5/0.5, weighted uses 0.3/0.7 (vector-biased).
        let (default_fts, default_vec) = match fusion_strategy {
            FusionStrategy::Rrf => (0.5f32, 0.5f32),
            FusionStrategy::WeightedScore => (0.3f32, 0.7f32),
        };
        let fts_weight = parse_optional_env("SEARCH_FTS_WEIGHT", default_fts)?;
        let vector_weight = parse_optional_env("SEARCH_VECTOR_WEIGHT", default_vec)?;

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
        if matches!(fusion_strategy, FusionStrategy::WeightedScore)
            && fts_weight == 0.0
            && vector_weight == 0.0
        {
            return Err(ConfigError::InvalidValue {
                key: "SEARCH_FTS_WEIGHT/SEARCH_VECTOR_WEIGHT".to_string(),
                message: "weighted fusion requires at least one non-zero weight".to_string(),
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
        assert!((config.fts_weight - 0.5).abs() < 0.001);
        assert!((config.vector_weight - 0.5).abs() < 0.001);
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

    #[test]
    fn weighted_strategy_defaults() {
        let _guard = ENV_MUTEX.lock().expect("env mutex poisoned");
        clear_search_env();

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::set_var("SEARCH_FUSION_STRATEGY", "weighted");
        }

        let config = WorkspaceSearchConfig::resolve().expect("should resolve");
        assert_eq!(config.fusion_strategy, FusionStrategy::WeightedScore);
        // Weighted mode should default to 0.3 FTS / 0.7 vector
        assert!((config.fts_weight - 0.3).abs() < 0.001);
        assert!((config.vector_weight - 0.7).abs() < 0.001);

        clear_search_env();
    }

    #[test]
    fn weighted_both_zero_rejected() {
        let _guard = ENV_MUTEX.lock().expect("env mutex poisoned");
        clear_search_env();

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::set_var("SEARCH_FUSION_STRATEGY", "weighted");
            std::env::set_var("SEARCH_FTS_WEIGHT", "0.0");
            std::env::set_var("SEARCH_VECTOR_WEIGHT", "0.0");
        }

        let result = WorkspaceSearchConfig::resolve();
        assert!(result.is_err());

        clear_search_env();
    }

    #[test]
    fn rrf_both_zero_allowed() {
        let _guard = ENV_MUTEX.lock().expect("env mutex poisoned");
        clear_search_env();

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::set_var("SEARCH_FTS_WEIGHT", "0.0");
            std::env::set_var("SEARCH_VECTOR_WEIGHT", "0.0");
        }

        // RRF ignores weights, so both=0 is fine
        let config = WorkspaceSearchConfig::resolve().expect("should resolve");
        assert_eq!(config.fusion_strategy, FusionStrategy::Rrf);

        clear_search_env();
    }
}
