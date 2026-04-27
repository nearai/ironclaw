use secrecy::SecretString;

use crate::config::helpers::{
    db_first_bool, db_first_optional_string, optional_env, validate_operator_base_url,
};
use crate::error::ConfigError;
use crate::llm::config::LlmConfig;
use crate::llm::registry::ProviderProtocol;
use crate::settings::Settings;

/// Configuration for built-in image generation, editing, and analysis tools.
///
/// Image tools speak OpenAI-compatible endpoints directly, so their endpoint
/// configuration is kept separate from the conversational LLM provider. When
/// the operator only supplies an image model, we may safely inherit the
/// current LLM endpoint and key. If the operator supplies a distinct
/// `IMAGE_BASE_URL`, they must also supply `IMAGE_API_KEY`; this avoids
/// forwarding the chat provider's bearer token to an arbitrary endpoint.
#[derive(Debug, Clone)]
pub struct ImageConfig {
    /// Whether image tools may be registered when enough endpoint details exist.
    pub enabled: bool,
    /// OpenAI-compatible base URL for image and vision tools.
    pub api_base_url: Option<String>,
    /// Bearer token for the configured image endpoint.
    pub api_key: Option<SecretString>,
    /// Model used by `image_generate` and `image_edit`.
    pub model: Option<String>,
    /// Model used by `image_analyze`.
    pub vision_model: Option<String>,
}

impl Default for ImageConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            api_base_url: None,
            api_key: None,
            model: None,
            vision_model: None,
        }
    }
}

#[derive(Debug, Clone)]
struct InheritedEndpoint {
    base_url: String,
    api_key: SecretString,
}

impl ImageConfig {
    pub(crate) fn resolve(settings: &Settings, llm: &LlmConfig) -> Result<Self, ConfigError> {
        let defaults = crate::settings::ImageSettings::default();
        let enabled = db_first_bool(
            settings.image.enabled,
            defaults.enabled,
            "IMAGE_TOOLS_ENABLED",
        )?;
        if !enabled {
            return Ok(Self {
                enabled,
                ..Self::default()
            });
        }

        let explicit_base_url =
            db_first_optional_string(&settings.image.base_url, "IMAGE_BASE_URL")?;
        if let Some(ref base_url) = explicit_base_url {
            validate_operator_base_url(base_url, "IMAGE_BASE_URL")?;
        }

        let explicit_api_key = optional_env("IMAGE_API_KEY")?.map(SecretString::from);
        let inherited = inherited_endpoint(llm);

        let api_base_url = explicit_base_url
            .clone()
            .or_else(|| inherited.as_ref().map(|endpoint| endpoint.base_url.clone()));

        let api_key = explicit_api_key.or_else(|| {
            if explicit_base_url.is_none() {
                inherited.map(|endpoint| endpoint.api_key)
            } else {
                None
            }
        });

        let active_model = llm.active_model_name();
        let explicit_model = db_first_optional_string(&settings.image.model, "IMAGE_MODEL")?;
        let model = explicit_model.or_else(|| {
            let models = vec![active_model.clone()];
            crate::llm::image_models::suggest_image_model(&models).map(str::to_string)
        });

        let explicit_vision_model =
            db_first_optional_string(&settings.image.vision_model, "IMAGE_VISION_MODEL")?;
        let vision_model = explicit_vision_model.or_else(|| {
            let models = vec![active_model];
            crate::llm::vision_models::suggest_vision_model(&models).map(str::to_string)
        });

        Ok(Self {
            enabled,
            api_base_url,
            api_key,
            model,
            vision_model,
        })
    }

    pub fn endpoint_credentials(&self) -> Option<(&str, &SecretString)> {
        if !self.enabled {
            return None;
        }

        Some((self.api_base_url.as_deref()?, self.api_key.as_ref()?))
    }
}

fn inherited_endpoint(llm: &LlmConfig) -> Option<InheritedEndpoint> {
    match llm.backend.as_str() {
        "nearai" | "near_ai" | "near" => {
            llm.nearai
                .api_key
                .as_ref()
                .cloned()
                .map(|api_key| InheritedEndpoint {
                    base_url: llm.nearai.base_url.clone(),
                    api_key,
                })
        }
        _ => {
            let provider = llm.provider.as_ref()?;
            if !matches!(provider.protocol, ProviderProtocol::OpenAiCompletions) {
                return None;
            }

            let base_url = if !provider.base_url.trim().is_empty() {
                provider.base_url.clone()
            } else if provider.provider_id == "openai" {
                "https://api.openai.com/v1".to_string()
            } else {
                return None;
            };

            provider
                .api_key
                .as_ref()
                .cloned()
                .map(|api_key| InheritedEndpoint { base_url, api_key })
        }
    }
}

#[cfg(test)]
mod tests {
    use secrecy::ExposeSecret;

    use super::*;
    use crate::config::helpers::lock_env;
    use crate::llm::config::{CacheRetention, NearAiConfig, RegistryProviderConfig};
    use crate::llm::session::SessionConfig;

    fn nearai_llm(api_key: Option<&str>, model: &str) -> LlmConfig {
        LlmConfig {
            backend: "nearai".to_string(),
            session: SessionConfig {
                auth_base_url: "http://localhost:0".to_string(),
                session_path: std::env::temp_dir().join("ironclaw-image-test-session.json"),
            },
            nearai: NearAiConfig {
                model: model.to_string(),
                cheap_model: None,
                base_url: "http://localhost:3000".to_string(),
                api_key: api_key.map(|key| SecretString::from(key.to_string())),
                fallback_model: None,
                max_retries: 0,
                circuit_breaker_threshold: None,
                circuit_breaker_recovery_secs: 30,
                response_cache_enabled: false,
                response_cache_ttl_secs: 3600,
                response_cache_max_entries: 100,
                failover_cooldown_secs: 300,
                failover_cooldown_threshold: 3,
                smart_routing_cascade: false,
            },
            provider: None,
            bedrock: None,
            gemini_oauth: None,
            openai_codex: None,
            request_timeout_secs: 120,
            cheap_model: None,
            smart_routing_cascade: false,
            max_retries: 0,
            circuit_breaker_threshold: None,
            circuit_breaker_recovery_secs: 30,
            response_cache_enabled: false,
            response_cache_ttl_secs: 3600,
            response_cache_max_entries: 100,
        }
    }

    fn registry_llm(
        provider_id: &str,
        base_url: &str,
        api_key: Option<&str>,
        model: &str,
    ) -> LlmConfig {
        let mut llm = nearai_llm(None, "nearai-chat");
        llm.backend = provider_id.to_string();
        llm.provider = Some(RegistryProviderConfig {
            protocol: ProviderProtocol::OpenAiCompletions,
            provider_id: provider_id.to_string(),
            api_key: api_key.map(|key| SecretString::from(key.to_string())),
            base_url: base_url.to_string(),
            model: model.to_string(),
            extra_headers: Vec::new(),
            oauth_token: None,
            is_codex_chatgpt: false,
            refresh_token: None,
            auth_path: None,
            cache_retention: CacheRetention::default(),
            unsupported_params: Vec::new(),
        });
        llm
    }

    fn clear_image_env() {
        unsafe {
            std::env::remove_var("IMAGE_TOOLS_ENABLED");
            std::env::remove_var("IMAGE_BASE_URL");
            std::env::remove_var("IMAGE_API_KEY");
            std::env::remove_var("IMAGE_MODEL");
            std::env::remove_var("IMAGE_VISION_MODEL");
        }
    }

    #[test]
    fn explicit_image_config_resolves_endpoint_and_models() {
        let _guard = lock_env();
        clear_image_env();
        unsafe {
            std::env::set_var("IMAGE_BASE_URL", "http://localhost:8080/v1");
            std::env::set_var("IMAGE_API_KEY", "image-key");
            std::env::set_var("IMAGE_MODEL", "gpt-image-1");
            std::env::set_var("IMAGE_VISION_MODEL", "gpt-4o");
        }

        let cfg = ImageConfig::resolve(&Settings::default(), &nearai_llm(Some("near-key"), "qwen"))
            .expect("image config should resolve");

        assert!(cfg.enabled);
        assert_eq!(
            cfg.api_base_url.as_deref(),
            Some("http://localhost:8080/v1")
        );
        assert_eq!(
            cfg.api_key.as_ref().map(|k| k.expose_secret()),
            Some("image-key")
        );
        assert_eq!(cfg.model.as_deref(), Some("gpt-image-1"));
        assert_eq!(cfg.vision_model.as_deref(), Some("gpt-4o"));

        clear_image_env();
    }

    #[test]
    fn explicit_base_url_does_not_inherit_chat_key() {
        let _guard = lock_env();
        clear_image_env();
        unsafe {
            std::env::set_var("IMAGE_BASE_URL", "http://localhost:8080/v1");
            std::env::set_var("IMAGE_MODEL", "dall-e-3");
        }

        let cfg = ImageConfig::resolve(&Settings::default(), &nearai_llm(Some("near-key"), "qwen"))
            .expect("image config should resolve");

        assert_eq!(
            cfg.api_base_url.as_deref(),
            Some("http://localhost:8080/v1")
        );
        assert!(cfg.api_key.is_none());
        assert!(cfg.endpoint_credentials().is_none());

        clear_image_env();
    }

    #[test]
    fn disabled_image_tools_skip_other_image_config() {
        let _guard = lock_env();
        clear_image_env();
        unsafe {
            std::env::set_var("IMAGE_TOOLS_ENABLED", "false");
            std::env::set_var("IMAGE_BASE_URL", "not a url");
            std::env::set_var("IMAGE_MODEL", "gpt-image-1");
            std::env::set_var("IMAGE_VISION_MODEL", "openai/gpt-5.2");
        }

        let cfg = ImageConfig::resolve(&Settings::default(), &nearai_llm(Some("near-key"), "qwen"))
            .expect("disabled image config should not validate unused fields");

        assert!(!cfg.enabled);
        assert!(cfg.api_base_url.is_none());
        assert!(cfg.api_key.is_none());
        assert!(cfg.model.is_none());
        assert!(cfg.vision_model.is_none());
        assert!(cfg.endpoint_credentials().is_none());

        clear_image_env();
    }

    #[test]
    fn image_model_can_reuse_openai_endpoint_when_no_image_base_is_set() {
        let _guard = lock_env();
        clear_image_env();
        unsafe {
            std::env::set_var("IMAGE_MODEL", "gpt-image-1");
        }

        let cfg = ImageConfig::resolve(
            &Settings::default(),
            &registry_llm("openai", "", Some("openai-key"), "gpt-4o"),
        )
        .expect("image config should resolve");

        assert_eq!(
            cfg.api_base_url.as_deref(),
            Some("https://api.openai.com/v1")
        );
        assert_eq!(
            cfg.api_key.as_ref().map(|k| k.expose_secret()),
            Some("openai-key")
        );
        assert_eq!(cfg.model.as_deref(), Some("gpt-image-1"));
        assert_eq!(cfg.vision_model.as_deref(), Some("gpt-4o"));

        clear_image_env();
    }

    #[test]
    fn chat_model_no_longer_gets_hardcoded_generation_fallback() {
        let _guard = lock_env();
        clear_image_env();

        let cfg = ImageConfig::resolve(&Settings::default(), &nearai_llm(Some("near-key"), "qwen"))
            .expect("image config should resolve");

        assert_eq!(cfg.api_base_url.as_deref(), Some("http://localhost:3000"));
        assert!(cfg.api_key.is_some());
        assert!(cfg.model.is_none());
    }
}
