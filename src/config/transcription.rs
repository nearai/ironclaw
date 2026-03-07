use secrecy::SecretString;

use crate::config::helpers::{optional_env, parse_bool_env};
use crate::error::ConfigError;
use crate::settings::Settings;

/// Transcription pipeline configuration.
#[derive(Debug, Clone)]
pub struct TranscriptionConfig {
    /// Whether audio transcription is enabled.
    pub enabled: bool,
    /// Provider: "openai" (default).
    pub provider: String,
    /// OpenAI API key (reuses OPENAI_API_KEY).
    pub openai_api_key: Option<SecretString>,
    /// Model to use (default: "whisper-1").
    pub model: String,
    /// Base URL override for the transcription API.
    pub base_url: Option<String>,
}

impl Default for TranscriptionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: "openai".to_string(),
            openai_api_key: None,
            model: "whisper-1".to_string(),
            base_url: None,
        }
    }
}

impl TranscriptionConfig {
    pub(crate) fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
        let enabled = parse_bool_env(
            "TRANSCRIPTION_ENABLED",
            settings.transcription.as_ref().is_some_and(|t| t.enabled),
        )?;

        let provider =
            optional_env("TRANSCRIPTION_PROVIDER")?.unwrap_or_else(|| "openai".to_string());

        let openai_api_key = optional_env("OPENAI_API_KEY")?.map(SecretString::from);

        let model = optional_env("TRANSCRIPTION_MODEL")?.unwrap_or_else(|| "whisper-1".to_string());

        let base_url = optional_env("TRANSCRIPTION_BASE_URL")?;

        Ok(Self {
            enabled,
            provider,
            openai_api_key,
            model,
            base_url,
        })
    }

    /// Create the transcription provider if enabled and configured.
    pub fn create_provider(&self) -> Option<Box<dyn crate::transcription::TranscriptionProvider>> {
        if !self.enabled {
            return None;
        }

        // Currently only OpenAI Whisper is supported; more providers can be
        // added here with a match on self.provider.
        let api_key = self.openai_api_key.as_ref()?;
        tracing::info!(model = %self.model, "Audio transcription enabled via OpenAI Whisper");

        let mut provider = crate::transcription::OpenAiWhisperProvider::new(api_key.clone())
            .with_model(&self.model);

        if let Some(ref base_url) = self.base_url {
            provider = provider.with_base_url(base_url);
        }

        Some(Box::new(provider))
    }
}
