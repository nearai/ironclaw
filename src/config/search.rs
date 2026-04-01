use crate::config::helpers::{db_first_option, db_first_or_default, parse_optional_env};
use crate::error::ConfigError;
use crate::settings::Settings;
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
    /// [`Default`] uses 0.5. When the configuration is resolved, per-strategy
    /// defaults are applied: 0.5 (RRF) or 0.3 (weighted).
    pub fts_weight: f32,
    /// Vector weight for fusion.
    ///
    /// [`Default`] uses 0.5. When the configuration is resolved, per-strategy
    /// defaults are applied: 0.5 (RRF) or 0.7 (weighted).
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
    pub(crate) fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
        let defaults = crate::settings::SearchSettings::default();
        let ss = &settings.search;

        // Resolve fusion_strategy string via DB-first, then parse into enum.
        let strategy_str = db_first_or_default(
            &ss.fusion_strategy,
            &defaults.fusion_strategy,
            "SEARCH_FUSION_STRATEGY",
        )?;
        let fusion_strategy = match strategy_str.to_lowercase().as_str() {
            "rrf" => FusionStrategy::Rrf,
            "weighted" => FusionStrategy::WeightedScore,
            other => {
                return Err(ConfigError::InvalidValue {
                    key: "SEARCH_FUSION_STRATEGY".to_string(),
                    message: format!("must be 'rrf' or 'weighted', got '{other}'"),
                });
            }
        };

        let rrf_k = db_first_or_default(&ss.rrf_k, &defaults.rrf_k, "SEARCH_RRF_K")?;

        // Per-strategy weight defaults: RRF uses 0.5/0.5, weighted uses 0.3/0.7 (vector-biased).
        let (default_fts, default_vec) = match fusion_strategy {
            FusionStrategy::Rrf => (0.5f32, 0.5f32),
            FusionStrategy::WeightedScore => (0.3f32, 0.7f32),
        };

        // Weights: DB (Some) > env > per-strategy default.
        // Uses db_first_option for shadow warnings when DB overrides env.
        let fts_weight = db_first_option(&ss.fts_weight, "SEARCH_FTS_WEIGHT")?
            .unwrap_or(parse_optional_env("SEARCH_FTS_WEIGHT", default_fts)?);
        let vector_weight = db_first_option(&ss.vector_weight, "SEARCH_VECTOR_WEIGHT")?
            .unwrap_or(parse_optional_env("SEARCH_VECTOR_WEIGHT", default_vec)?);

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
    use crate::config::helpers::lock_env;

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
        let _guard = lock_env();
        clear_search_env();

        let settings = Settings::default();
        let config = WorkspaceSearchConfig::resolve(&settings).expect("should resolve");
        assert_eq!(config.fusion_strategy, FusionStrategy::Rrf);
        assert_eq!(config.rrf_k, 60);
        assert!((config.fts_weight - 0.5).abs() < 0.001);
        assert!((config.vector_weight - 0.5).abs() < 0.001);
    }

    #[test]
    fn db_settings_override_env() {
        let _guard = lock_env();
        clear_search_env();

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::set_var("SEARCH_FUSION_STRATEGY", "rrf");
            std::env::set_var("SEARCH_RRF_K", "30");
            std::env::set_var("SEARCH_FTS_WEIGHT", "0.9");
            std::env::set_var("SEARCH_VECTOR_WEIGHT", "0.1");
        }

        let mut settings = Settings::default();
        settings.search.fusion_strategy = "weighted".to_string();
        settings.search.rrf_k = 42;
        settings.search.fts_weight = Some(0.4);
        settings.search.vector_weight = Some(0.6);

        let config = WorkspaceSearchConfig::resolve(&settings).expect("should resolve");
        assert_eq!(config.fusion_strategy, FusionStrategy::WeightedScore);
        assert_eq!(config.rrf_k, 42);
        assert!((config.fts_weight - 0.4).abs() < 0.001);
        assert!((config.vector_weight - 0.6).abs() < 0.001);

        clear_search_env();
    }

    #[test]
    fn env_fallback_when_settings_at_default() {
        let _guard = lock_env();
        clear_search_env();

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::set_var("SEARCH_FUSION_STRATEGY", "weighted");
            std::env::set_var("SEARCH_RRF_K", "30");
            std::env::set_var("SEARCH_FTS_WEIGHT", "0.9");
            std::env::set_var("SEARCH_VECTOR_WEIGHT", "0.1");
        }

        let settings = Settings::default();
        let config = WorkspaceSearchConfig::resolve(&settings).expect("should resolve");
        assert_eq!(config.fusion_strategy, FusionStrategy::WeightedScore);
        assert_eq!(config.rrf_k, 30);
        assert!((config.fts_weight - 0.9).abs() < 0.001);
        assert!((config.vector_weight - 0.1).abs() < 0.001);

        clear_search_env();
    }

    #[test]
    fn invalid_strategy_rejected() {
        let _guard = lock_env();
        clear_search_env();

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::set_var("SEARCH_FUSION_STRATEGY", "bm25");
        }

        let settings = Settings::default();
        let result = WorkspaceSearchConfig::resolve(&settings);
        assert!(result.is_err());

        clear_search_env();
    }

    #[test]
    fn weighted_strategy_defaults() {
        let _guard = lock_env();
        clear_search_env();

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::set_var("SEARCH_FUSION_STRATEGY", "weighted");
        }

        let settings = Settings::default();
        let config = WorkspaceSearchConfig::resolve(&settings).expect("should resolve");
        assert_eq!(config.fusion_strategy, FusionStrategy::WeightedScore);
        // Weighted mode should default to 0.3 FTS / 0.7 vector
        assert!((config.fts_weight - 0.3).abs() < 0.001);
        assert!((config.vector_weight - 0.7).abs() < 0.001);

        clear_search_env();
    }

    #[test]
    fn weighted_both_zero_rejected() {
        let _guard = lock_env();
        clear_search_env();

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::set_var("SEARCH_FUSION_STRATEGY", "weighted");
            std::env::set_var("SEARCH_FTS_WEIGHT", "0.0");
            std::env::set_var("SEARCH_VECTOR_WEIGHT", "0.0");
        }

        let settings = Settings::default();
        let result = WorkspaceSearchConfig::resolve(&settings);
        assert!(result.is_err());

        clear_search_env();
    }

    #[test]
    fn rrf_both_zero_allowed() {
        let _guard = lock_env();
        clear_search_env();

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::set_var("SEARCH_FTS_WEIGHT", "0.0");
            std::env::set_var("SEARCH_VECTOR_WEIGHT", "0.0");
        }

        // RRF ignores weights, so both=0 is fine
        let settings = Settings::default();
        let config = WorkspaceSearchConfig::resolve(&settings).expect("should resolve");
        assert_eq!(config.fusion_strategy, FusionStrategy::Rrf);

        clear_search_env();
    }
}
