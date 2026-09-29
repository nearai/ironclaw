//! Build the configured [`EmbeddingProvider`] from its provider id, failing
//! closed.
//!
//! The composition root owns *when* a provider is built and who receives it;
//! this module owns the id catalog and the construction rules, so the
//! composition root never names a concrete embeddings client.
//!
//! Fail-closed rule: an unset id, an unknown id, a missing model, a missing
//! required credential, an unparseable override, or a base URL the SSRF guard
//! rejects all yield **no provider**. An unknown id is never routed to a
//! default vendor.

use std::sync::Arc;
use std::time::Duration;

use secrecy::SecretString;

use super::{
    EmbeddingError, EmbeddingProvider, OPENAI_DEFAULT_BASE_URL, OpenAiCompatibleEmbeddingConfig,
    OpenAiCompatibleEmbeddings,
};

/// Provider id for OpenAI itself. The base URL defaults to
/// [`OPENAI_DEFAULT_BASE_URL`] and an API key is required.
pub const OPENAI_EMBEDDING_PROVIDER_ID: &str = "openai";

/// Provider id for any other OpenAI-compatible endpoint. A base URL is
/// required and the API key is optional.
pub const OPENAI_COMPATIBLE_EMBEDDING_PROVIDER_ID: &str = "openai_compatible";

/// Environment variable read for the API key when no `api_key_env` is set.
pub const DEFAULT_EMBEDDING_API_KEY_ENV: &str = "EMBEDDING_API_KEY";

/// Environment overrides, applied over the config-file values (env wins).
pub const EMBEDDING_PROVIDER_ENV: &str = "EMBEDDING_PROVIDER";
pub const EMBEDDING_BASE_URL_ENV: &str = "EMBEDDING_BASE_URL";
pub const EMBEDDING_MODEL_ENV: &str = "EMBEDDING_MODEL";
pub const EMBEDDING_API_KEY_ENV_ENV: &str = "EMBEDDING_API_KEY_ENV";
pub const EMBEDDING_DIMENSION_ENV: &str = "EMBEDDING_DIMENSION";
pub const EMBEDDING_MAX_BATCH_SIZE_ENV: &str = "EMBEDDING_MAX_BATCH_SIZE";
pub const EMBEDDING_REQUEST_TIMEOUT_SECS_ENV: &str = "EMBEDDING_REQUEST_TIMEOUT_SECS";

/// Unresolved embeddings settings, as written in the operator config file.
///
/// Pure data with no secrets: the API key is referenced only by the NAME of
/// the environment variable holding it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EmbeddingProviderSettings {
    /// Provider id; `None` means no provider.
    pub provider_id: Option<String>,
    pub base_url: Option<String>,
    pub model: Option<String>,
    /// Name of the env var holding the API key; `None` reads
    /// [`DEFAULT_EMBEDDING_API_KEY_ENV`].
    pub api_key_env: Option<String>,
    pub dimension: Option<usize>,
    pub max_batch_size: Option<usize>,
    pub request_timeout_secs: Option<u64>,
}

/// Which concrete client an id selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EmbeddingProviderKind {
    OpenAi,
    OpenAiCompatible,
}

impl EmbeddingProviderKind {
    fn from_id(id: &str) -> Option<Self> {
        match id {
            OPENAI_EMBEDDING_PROVIDER_ID => Some(Self::OpenAi),
            OPENAI_COMPATIBLE_EMBEDDING_PROVIDER_ID => Some(Self::OpenAiCompatible),
            _ => None,
        }
    }
}

impl EmbeddingProviderSettings {
    /// Apply the `EMBEDDING_*` environment overrides. `env` returns a
    /// variable's value, or `None` when it is unset or blank.
    pub fn with_env_overrides(
        mut self,
        env: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Self, EmbeddingError> {
        let string = |name: &str, current: Option<String>| env(name).or(current);
        self.provider_id = string(EMBEDDING_PROVIDER_ENV, self.provider_id);
        self.base_url = string(EMBEDDING_BASE_URL_ENV, self.base_url);
        self.model = string(EMBEDDING_MODEL_ENV, self.model);
        self.api_key_env = string(EMBEDDING_API_KEY_ENV_ENV, self.api_key_env);
        self.dimension = parse_override(env, EMBEDDING_DIMENSION_ENV)?.or(self.dimension);
        self.max_batch_size =
            parse_override(env, EMBEDDING_MAX_BATCH_SIZE_ENV)?.or(self.max_batch_size);
        self.request_timeout_secs =
            parse_override(env, EMBEDDING_REQUEST_TIMEOUT_SECS_ENV)?.or(self.request_timeout_secs);
        Ok(self)
    }

    /// Turn the settings into a client config, or explain why there is none.
    /// `Ok(None)` means no provider was asked for.
    fn resolve(
        &self,
        env: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Option<OpenAiCompatibleEmbeddingConfig>, EmbeddingError> {
        let Some(provider_id) = self.provider_id.as_deref() else {
            return Ok(None);
        };
        let invalid = |reason: String| EmbeddingError::InvalidConfig { reason };
        let kind = EmbeddingProviderKind::from_id(provider_id).ok_or_else(|| {
            invalid(format!(
                "unknown embeddings provider id '{provider_id}' (known: \
                 '{OPENAI_EMBEDDING_PROVIDER_ID}', '{OPENAI_COMPATIBLE_EMBEDDING_PROVIDER_ID}')"
            ))
        })?;
        let model = self
            .model
            .clone()
            .ok_or_else(|| invalid(format!("embeddings provider '{provider_id}' needs a model")))?;
        let base_url = match (kind, self.base_url.clone()) {
            (_, Some(url)) => url,
            (EmbeddingProviderKind::OpenAi, None) => OPENAI_DEFAULT_BASE_URL.to_string(),
            (EmbeddingProviderKind::OpenAiCompatible, None) => {
                return Err(invalid(format!(
                    "embeddings provider '{provider_id}' needs a base URL"
                )));
            }
        };
        let key_env = self
            .api_key_env
            .as_deref()
            .unwrap_or(DEFAULT_EMBEDDING_API_KEY_ENV);
        let api_key = env(key_env).map(SecretString::from);
        if kind == EmbeddingProviderKind::OpenAi && api_key.is_none() {
            return Err(invalid(format!(
                "embeddings provider '{provider_id}' needs an API key in ${key_env}"
            )));
        }

        let mut config = OpenAiCompatibleEmbeddingConfig::new(provider_id, base_url, model);
        config.api_key = api_key;
        config.dimension = self.dimension;
        if let Some(max_batch_size) = self.max_batch_size {
            config.max_batch_size = max_batch_size;
        }
        if let Some(secs) = self.request_timeout_secs {
            config.request_timeout = Duration::from_secs(secs);
        }
        Ok(Some(config))
    }
}

fn parse_override<T: std::str::FromStr>(
    env: &dyn Fn(&str) -> Option<String>,
    name: &str,
) -> Result<Option<T>, EmbeddingError> {
    env(name)
        .map(|raw| {
            raw.trim()
                .parse::<T>()
                .map_err(|_| EmbeddingError::InvalidConfig {
                    reason: format!("${name} must be a non-negative integer, got '{raw}'"),
                })
        })
        .transpose()
}

/// Build the configured embeddings provider, or `None` (fail closed).
///
/// Applies the environment overrides, selects the client by provider id, and
/// constructs it (which runs the base-URL SSRF guard). Unset id ⇒ `None`
/// quietly; every other reason for `None` is logged.
pub async fn create_embedding_provider(
    settings: EmbeddingProviderSettings,
    env: &dyn Fn(&str) -> Option<String>,
) -> Option<Arc<dyn EmbeddingProvider>> {
    let built = async {
        let Some(config) = settings.with_env_overrides(env)?.resolve(env)? else {
            return Ok(None);
        };
        OpenAiCompatibleEmbeddings::new(config).await.map(Some)
    };
    match built.await {
        Ok(Some(provider)) => Some(Arc::new(provider)),
        Ok(None) => {
            tracing::debug!("no embeddings provider configured");
            None
        }
        Err(error) => {
            tracing::warn!(%error, "embeddings provider not built; failing closed");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |name| map.get(name).cloned()
    }

    fn settings(provider: &str) -> EmbeddingProviderSettings {
        EmbeddingProviderSettings {
            provider_id: Some(provider.to_string()),
            base_url: Some("http://127.0.0.1:9/v1".to_string()),
            model: Some("embed-model".to_string()),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn unset_settings_give_no_provider() {
        let env = env_of(&[]);
        assert!(
            create_embedding_provider(EmbeddingProviderSettings::default(), &env)
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn an_unknown_id_gives_no_provider_even_with_openai_credentials() {
        // The regression this guards: an unrecognised id must not fall through
        // to the OpenAI client just because an OpenAI key is present.
        let env = env_of(&[(DEFAULT_EMBEDDING_API_KEY_ENV, "sk-test")]);
        for id in ["opanai", "ollama", "OpenAI", ""] {
            let mut s = settings(id);
            s.base_url = None;
            assert!(
                create_embedding_provider(s.clone(), &env).await.is_none(),
                "id {id:?} must fail closed"
            );
            assert!(matches!(
                s.resolve(&env),
                Err(EmbeddingError::InvalidConfig { .. })
            ));
        }
    }

    #[tokio::test]
    async fn a_configured_openai_compatible_provider_is_built() {
        let env = env_of(&[]);
        let provider = create_embedding_provider(settings("openai_compatible"), &env)
            .await
            .expect("provider built");
        assert_eq!(provider.model_name(), "embed-model");
        assert_eq!(provider.dimension(), None);
    }

    #[test]
    fn openai_defaults_its_base_url_and_requires_a_key() {
        let mut s = settings("openai");
        s.base_url = None;
        assert!(matches!(
            s.resolve(&env_of(&[])),
            Err(EmbeddingError::InvalidConfig { .. })
        ));

        let config = s
            .resolve(&env_of(&[(DEFAULT_EMBEDDING_API_KEY_ENV, "sk-test")]))
            .expect("resolves")
            .expect("some config");
        assert_eq!(config.base_url, OPENAI_DEFAULT_BASE_URL);
        assert!(config.api_key.is_some());
    }

    #[test]
    fn openai_compatible_requires_a_base_url_and_every_provider_a_model() {
        let mut no_url = settings("openai_compatible");
        no_url.base_url = None;
        assert!(no_url.resolve(&env_of(&[])).is_err());

        let mut no_model = settings("openai_compatible");
        no_model.model = None;
        assert!(no_model.resolve(&env_of(&[])).is_err());
    }

    #[test]
    fn the_api_key_is_read_from_the_named_env_var() {
        let mut s = settings("openai_compatible");
        s.api_key_env = Some("MY_EMBED_KEY".to_string());
        let config = s
            .resolve(&env_of(&[
                ("MY_EMBED_KEY", "k-named"),
                (DEFAULT_EMBEDDING_API_KEY_ENV, "k-default"),
            ]))
            .expect("resolves")
            .expect("some config");
        use secrecy::ExposeSecret;
        assert_eq!(
            config.api_key.as_ref().map(|k| k.expose_secret()),
            Some("k-named")
        );
    }

    #[test]
    fn env_overrides_win_over_the_config_file() {
        let file = EmbeddingProviderSettings {
            provider_id: Some("openai".to_string()),
            base_url: Some("https://file.example.com".to_string()),
            model: Some("file-model".to_string()),
            api_key_env: Some("FILE_KEY".to_string()),
            dimension: Some(3),
            max_batch_size: Some(4),
            request_timeout_secs: Some(5),
        };
        let env = env_of(&[
            (EMBEDDING_PROVIDER_ENV, "openai_compatible"),
            (EMBEDDING_BASE_URL_ENV, "http://localhost:8080"),
            (EMBEDDING_MODEL_ENV, "env-model"),
            (EMBEDDING_API_KEY_ENV_ENV, "ENV_KEY"),
            (EMBEDDING_DIMENSION_ENV, "768"),
            (EMBEDDING_MAX_BATCH_SIZE_ENV, "16"),
            (EMBEDDING_REQUEST_TIMEOUT_SECS_ENV, " 9 "),
        ]);
        let merged = file.clone().with_env_overrides(&env).expect("overrides");
        assert_eq!(
            merged,
            EmbeddingProviderSettings {
                provider_id: Some("openai_compatible".to_string()),
                base_url: Some("http://localhost:8080".to_string()),
                model: Some("env-model".to_string()),
                api_key_env: Some("ENV_KEY".to_string()),
                dimension: Some(768),
                max_batch_size: Some(16),
                request_timeout_secs: Some(9),
            }
        );

        // No env: the file values stand.
        assert_eq!(
            file.clone().with_env_overrides(&env_of(&[])).expect("ok"),
            file
        );
    }

    #[tokio::test]
    async fn an_unparseable_numeric_override_fails_closed() {
        let env = env_of(&[(EMBEDDING_DIMENSION_ENV, "lots")]);
        assert!(matches!(
            settings("openai_compatible").with_env_overrides(&env),
            Err(EmbeddingError::InvalidConfig { .. })
        ));
        assert!(
            create_embedding_provider(settings("openai_compatible"), &env)
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn a_blocked_base_url_fails_closed() {
        let mut s = settings("openai_compatible");
        s.base_url = Some("http://169.254.169.254".to_string());
        assert!(create_embedding_provider(s, &env_of(&[])).await.is_none());
    }
}
