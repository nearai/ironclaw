//! Resolver that builds an `EmbeddingsConfig` from binary-side `Settings`.
//!
//! The `EmbeddingsConfig` data shape, factory, cache, and providers all live
//! in the `ironclaw_embeddings` crate. Only the bit that reads
//! `crate::settings::Settings` (a binary-internal type) and validates
//! env-driven base URLs against the SSRF blocklist (also binary-internal)
//! stays here.

use secrecy::SecretString;

use crate::config::helpers::{
    db_first_bool, db_first_or_default, optional_env, parse_optional_env,
    validate_operator_base_url,
};
use crate::error::ConfigError;
use crate::settings::Settings;
use ironclaw_embeddings::{
    DEFAULT_EMBEDDING_CACHE_SIZE, EmbeddingsConfig, default_dimension_for_model,
};

/// Resolve embeddings configuration from settings, env vars, and defaults.
///
/// Precedence: DB/TOML settings > env > default.
pub(crate) fn resolve_embeddings_config(
    settings: &Settings,
) -> Result<EmbeddingsConfig, ConfigError> {
    let defaults = crate::settings::EmbeddingsSettings::default();

    let openai_api_key = optional_env("OPENAI_API_KEY")?.map(SecretString::from);

    let provider = db_first_or_default(
        &settings.embeddings.provider,
        &defaults.provider,
        "EMBEDDING_PROVIDER",
    )?;

    let model = if provider == "bedrock" {
        optional_env("EMBEDDING_MODEL")?
            .unwrap_or_else(|| "amazon.titan-embed-text-v2:0".to_string())
    } else {
        db_first_or_default(
            &settings.embeddings.model,
            &defaults.model,
            "EMBEDDING_MODEL",
        )?
    };

    // ollama_base_url lives on the top-level Settings, not the embeddings
    // sub-struct. Use a manual DB > env > default chain.
    let default_ollama_url = "http://localhost:11434".to_string();
    let ollama_base_url = match settings
        .ollama_base_url
        .as_ref()
        .filter(|s| !s.is_empty())
        .cloned()
    {
        Some(url) => url,
        None => optional_env("OLLAMA_BASE_URL")?.unwrap_or(default_ollama_url),
    };

    // Dimension depends on the resolved model, not on a DB setting — env-only.
    let dimension = parse_optional_env("EMBEDDING_DIMENSION", default_dimension_for_model(&model))?;
    if provider == "bedrock" && !matches!(dimension, 256 | 512 | 1024) {
        return Err(ConfigError::InvalidValue {
            key: "EMBEDDING_DIMENSION".to_string(),
            message: "Bedrock Titan v2 embeddings support only 256, 512, or 1024 dimensions"
                .to_string(),
        });
    }

    let enabled = db_first_bool(
        settings.embeddings.enabled,
        defaults.enabled,
        "EMBEDDING_ENABLED",
    )?;

    let openai_base_url = optional_env("EMBEDDING_BASE_URL")?;

    // Validate base URLs to prevent SSRF attacks (#1103).
    validate_operator_base_url(&ollama_base_url, "OLLAMA_BASE_URL")?;
    if let Some(ref url) = openai_base_url {
        validate_operator_base_url(url, "EMBEDDING_BASE_URL")?;
    }

    let cache_size = parse_optional_env("EMBEDDING_CACHE_SIZE", DEFAULT_EMBEDDING_CACHE_SIZE)?;

    if cache_size == 0 {
        return Err(ConfigError::InvalidValue {
            key: "EMBEDDING_CACHE_SIZE".to_string(),
            message: "must be at least 1".to_string(),
        });
    }

    Ok(EmbeddingsConfig {
        enabled,
        provider,
        openai_api_key,
        model,
        ollama_base_url,
        dimension,
        openai_base_url,
        cache_size,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::helpers::lock_env;
    use crate::settings::{EmbeddingsSettings, Settings};
    use crate::testing::credentials::*;

    /// Clear all embedding-related env vars.
    fn clear_embedding_env() {
        // SAFETY: Only called under ENV_MUTEX in tests.
        unsafe {
            std::env::remove_var("EMBEDDING_ENABLED");
            std::env::remove_var("EMBEDDING_PROVIDER");
            std::env::remove_var("EMBEDDING_MODEL");
            std::env::remove_var("EMBEDDING_DIMENSION");
            std::env::remove_var("OPENAI_API_KEY");
            std::env::remove_var("EMBEDDING_BASE_URL");
            std::env::remove_var("EMBEDDING_CACHE_SIZE");
            std::env::remove_var("OLLAMA_BASE_URL");
        }
    }

    #[test]
    fn embeddings_disabled_not_overridden_by_openai_key() {
        let _guard = lock_env();
        clear_embedding_env();
        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe {
            std::env::set_var("OPENAI_API_KEY", TEST_OPENAI_API_KEY_ISSUE_129);
        }

        let settings = Settings {
            embeddings: EmbeddingsSettings {
                enabled: false,
                ..Default::default()
            },
            ..Default::default()
        };

        let config = resolve_embeddings_config(&settings).expect("resolve should succeed");
        assert!(
            !config.enabled,
            "embeddings should remain disabled when settings.embeddings.enabled=false, \
             even when OPENAI_API_KEY is set (issue #129)"
        );

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("OPENAI_API_KEY");
        }
    }

    #[test]
    fn embeddings_enabled_from_settings() {
        let _guard = lock_env();
        clear_embedding_env();

        let settings = Settings {
            embeddings: EmbeddingsSettings {
                enabled: true,
                ..Default::default()
            },
            ..Default::default()
        };

        let config = resolve_embeddings_config(&settings).expect("resolve should succeed");
        assert!(
            config.enabled,
            "embeddings should be enabled when settings say so"
        );
    }

    #[test]
    fn db_settings_override_env() {
        let _guard = lock_env();
        clear_embedding_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::set_var("EMBEDDING_ENABLED", "false");
            std::env::set_var("EMBEDDING_PROVIDER", "ollama");
            std::env::set_var("EMBEDDING_MODEL", "all-minilm");
        }

        let settings = Settings {
            embeddings: EmbeddingsSettings {
                enabled: true,
                provider: "openai".to_string(),
                model: "text-embedding-3-large".to_string(),
            },
            ..Default::default()
        };

        let config = resolve_embeddings_config(&settings).expect("resolve should succeed");
        assert!(
            config.enabled,
            "DB enabled=true should win over env EMBEDDING_ENABLED=false"
        );
        assert_eq!(config.provider, "openai", "DB provider should win over env");
        assert_eq!(
            config.model, "text-embedding-3-large",
            "DB model should win over env"
        );

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("EMBEDDING_ENABLED");
            std::env::remove_var("EMBEDDING_PROVIDER");
            std::env::remove_var("EMBEDDING_MODEL");
        }
    }

    #[test]
    fn env_used_when_no_db_setting() {
        let _guard = lock_env();
        clear_embedding_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::set_var("EMBEDDING_ENABLED", "true");
            std::env::set_var("EMBEDDING_PROVIDER", "ollama");
            std::env::set_var("EMBEDDING_MODEL", "nomic-embed-text");
        }

        // Settings left at defaults — no explicit DB/TOML override
        let settings = Settings::default();

        let config = resolve_embeddings_config(&settings).expect("resolve should succeed");
        assert!(
            config.enabled,
            "env EMBEDDING_ENABLED should be used when settings at default"
        );
        assert_eq!(
            config.provider, "ollama",
            "env EMBEDDING_PROVIDER should be used when settings at default"
        );
        assert_eq!(
            config.model, "nomic-embed-text",
            "env EMBEDDING_MODEL should be used when settings at default"
        );

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("EMBEDDING_ENABLED");
            std::env::remove_var("EMBEDDING_PROVIDER");
            std::env::remove_var("EMBEDDING_MODEL");
        }
    }

    #[test]
    fn embedding_base_url_parsed_from_env() {
        let _guard = lock_env();
        clear_embedding_env();

        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe {
            std::env::set_var("EMBEDDING_BASE_URL", "https://8.8.8.8");
        }

        let settings = Settings::default();
        let config = resolve_embeddings_config(&settings).expect("resolve should succeed");
        assert_eq!(config.openai_base_url.as_deref(), Some("https://8.8.8.8"));
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("EMBEDDING_BASE_URL");
        }
    }

    #[test]
    fn embedding_base_url_defaults_to_none() {
        let _guard = lock_env();
        clear_embedding_env();

        let settings = Settings::default();
        let config = resolve_embeddings_config(&settings).expect("resolve should succeed");
        assert!(
            config.openai_base_url.is_none(),
            "openai_base_url should be None when EMBEDDING_BASE_URL is not set"
        );
    }

    #[test]
    fn cache_size_zero_rejected() {
        let _guard = lock_env();
        clear_embedding_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::set_var("EMBEDDING_CACHE_SIZE", "0");
        }

        let settings = Settings::default();
        let result = resolve_embeddings_config(&settings);
        assert!(result.is_err(), "cache_size=0 should be rejected");
        let err = result.unwrap_err().to_string();
        assert!(err.contains("at least 1"), "should mention minimum: {err}");
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("EMBEDDING_CACHE_SIZE");
        }
    }

    #[cfg(feature = "bedrock")]
    #[test]
    fn bedrock_provider_defaults_to_titan_v2() {
        let _guard = lock_env();
        clear_embedding_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::set_var("EMBEDDING_ENABLED", "true");
            std::env::set_var("EMBEDDING_PROVIDER", "bedrock");
        }

        let settings = Settings::default();
        let config = resolve_embeddings_config(&settings).expect("resolve should succeed");
        assert_eq!(config.provider, "bedrock");
        assert_eq!(config.model, "amazon.titan-embed-text-v2:0");
        assert_eq!(config.dimension, 1024);

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("EMBEDDING_ENABLED");
            std::env::remove_var("EMBEDDING_PROVIDER");
        }
    }

    #[cfg(feature = "bedrock")]
    #[test]
    fn bedrock_dimension_validation_rejects_unsupported_values() {
        let _guard = lock_env();
        clear_embedding_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::set_var("EMBEDDING_ENABLED", "true");
            std::env::set_var("EMBEDDING_PROVIDER", "bedrock");
            std::env::set_var("EMBEDDING_DIMENSION", "1536");
        }

        let settings = Settings::default();
        let result = resolve_embeddings_config(&settings);
        assert!(
            result.is_err(),
            "unsupported bedrock dimensions should fail"
        );
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("256, 512, or 1024"),
            "error should mention valid dimensions: {err}"
        );

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("EMBEDDING_ENABLED");
            std::env::remove_var("EMBEDDING_PROVIDER");
            std::env::remove_var("EMBEDDING_DIMENSION");
        }
    }
}
