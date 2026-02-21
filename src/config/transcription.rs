use secrecy::SecretString;

use crate::config::helpers::optional_env;
use crate::error::ConfigError;
use crate::settings::Settings;

/// Transcription provider configuration.
#[derive(Debug, Clone)]
pub struct TranscriptionConfig {
    /// Whether transcription is enabled.
    pub enabled: bool,
    /// Provider to use: "openai".
    pub provider: String,
    /// OpenAI API key (reused from embeddings/LLM config).
    pub openai_api_key: Option<SecretString>,
    /// Model to use for transcription.
    pub model: String,
    /// Optional language hint (ISO-639-1, e.g., "en").
    pub language: Option<String>,
}

impl Default for TranscriptionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: "openai".to_string(),
            openai_api_key: None,
            model: "whisper-1".to_string(),
            language: None,
        }
    }
}

impl TranscriptionConfig {
    pub(crate) fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
        let openai_api_key = optional_env("OPENAI_API_KEY")?.map(SecretString::from);

        let provider = optional_env("TRANSCRIPTION_PROVIDER")?
            .unwrap_or_else(|| settings.transcription.provider.clone());

        let model = optional_env("TRANSCRIPTION_MODEL")?
            .unwrap_or_else(|| settings.transcription.model.clone());

        let language = optional_env("TRANSCRIPTION_LANGUAGE")?
            .or_else(|| settings.transcription.language.clone());

        let enabled = optional_env("TRANSCRIPTION_ENABLED")?
            .map(|s| s.parse())
            .transpose()
            .map_err(|e| ConfigError::InvalidValue {
                key: "TRANSCRIPTION_ENABLED".to_string(),
                message: format!("must be 'true' or 'false': {e}"),
            })?
            .unwrap_or(settings.transcription.enabled);

        // Only "openai" is currently supported
        if enabled && provider != "openai" {
            return Err(ConfigError::InvalidValue {
                key: "TRANSCRIPTION_PROVIDER".to_string(),
                message: format!(
                    "unsupported provider '{}', only 'openai' is currently supported",
                    provider
                ),
            });
        }

        Ok(Self {
            enabled,
            provider,
            openai_api_key,
            model,
            language,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::helpers::ENV_MUTEX;
    use crate::settings::{Settings, TranscriptionSettings};

    fn clear_transcription_env() {
        unsafe {
            std::env::remove_var("TRANSCRIPTION_ENABLED");
            std::env::remove_var("TRANSCRIPTION_PROVIDER");
            std::env::remove_var("TRANSCRIPTION_MODEL");
            std::env::remove_var("TRANSCRIPTION_LANGUAGE");
            std::env::remove_var("OPENAI_API_KEY");
        }
    }

    #[test]
    fn transcription_defaults_from_settings() {
        let _guard = ENV_MUTEX.lock().expect("env mutex poisoned");
        clear_transcription_env();

        let settings = Settings::default();
        let config = TranscriptionConfig::resolve(&settings).expect("resolve should succeed");

        assert!(!config.enabled);
        assert_eq!(config.provider, "openai");
        assert_eq!(config.model, "whisper-1");
        assert!(config.language.is_none());
    }

    #[test]
    fn transcription_env_overrides_settings() {
        let _guard = ENV_MUTEX.lock().expect("env mutex poisoned");
        clear_transcription_env();

        unsafe {
            std::env::set_var("TRANSCRIPTION_ENABLED", "true");
            std::env::set_var("TRANSCRIPTION_MODEL", "whisper-large-v3");
            std::env::set_var("TRANSCRIPTION_LANGUAGE", "en");
        }

        let settings = Settings::default();
        let config = TranscriptionConfig::resolve(&settings).expect("resolve should succeed");

        assert!(config.enabled);
        assert_eq!(config.model, "whisper-large-v3");
        assert_eq!(config.language, Some("en".to_string()));

        unsafe {
            std::env::remove_var("TRANSCRIPTION_ENABLED");
            std::env::remove_var("TRANSCRIPTION_MODEL");
            std::env::remove_var("TRANSCRIPTION_LANGUAGE");
        }
    }

    #[test]
    fn transcription_settings_with_custom_values() {
        let _guard = ENV_MUTEX.lock().expect("env mutex poisoned");
        clear_transcription_env();

        let settings = Settings {
            transcription: TranscriptionSettings {
                enabled: true,
                model: "whisper-large-v3".to_string(),
                language: Some("fr".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };

        let config = TranscriptionConfig::resolve(&settings).expect("resolve should succeed");

        assert!(config.enabled);
        assert_eq!(config.model, "whisper-large-v3");
        assert_eq!(config.language, Some("fr".to_string()));
    }

    #[test]
    fn transcription_rejects_unsupported_provider() {
        let _guard = ENV_MUTEX.lock().expect("env mutex poisoned");
        clear_transcription_env();

        let settings = Settings {
            transcription: TranscriptionSettings {
                enabled: true,
                provider: "deepgram".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };

        let result = TranscriptionConfig::resolve(&settings);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("unsupported provider"),
            "Error should mention unsupported provider, got: {err_msg}"
        );
    }

    #[test]
    fn transcription_disabled_skips_provider_validation() {
        let _guard = ENV_MUTEX.lock().expect("env mutex poisoned");
        clear_transcription_env();

        // When disabled, any provider string is accepted (never used)
        let settings = Settings {
            transcription: TranscriptionSettings {
                enabled: false,
                provider: "nonexistent".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };

        let config = TranscriptionConfig::resolve(&settings).expect("should succeed when disabled");
        assert!(!config.enabled);
    }
}
