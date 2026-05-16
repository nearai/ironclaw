//! Model discovery and fetching for multiple LLM providers.
//!
//! External callers should use [`fetch_models_for`] — a single verb-based
//! facade that dispatches on a provider ID string. The per-provider
//! fetcher functions below are `pub(crate)` and not part of the public
//! surface of `ironclaw_llm`.

/// Options for [`fetch_models_for`].
#[derive(Debug, Default)]
pub struct ModelFetchOptions<'a> {
    /// API key for backends that authenticate per request (anthropic,
    /// openai, openai-compatible). Optional — fetchers fall back to env
    /// vars and then to static defaults if no key is available.
    pub api_key: Option<&'a str>,
    /// Base URL for self-hosted or proxied backends (ollama,
    /// openai-compatible). `None` uses the per-backend default
    /// (e.g. `http://localhost:11434` for ollama).
    pub base_url: Option<&'a str>,
}

/// Fetch the model catalog for a given backend.
///
/// Dispatches on `provider_id`:
/// - `"anthropic"` → Anthropic `/v1/models`
/// - `"openai"` → OpenAI `/v1/models` (filtered to chat-capable models)
/// - `"ollama"` → local Ollama `/api/tags`
/// - any other ID → generic OpenAI-compatible `/v1/models` against
///   `options.base_url`. Used by openrouter, deepseek, custom endpoints,
///   etc.
///
/// For `anthropic` / `openai` / `ollama`, the per-backend fetcher falls
/// back to its own static default list on network or auth failure so
/// the setup wizard can still progress offline.
///
/// The generic openai-compatible branch has **no static fallback** — it
/// returns an empty list if `options.base_url` is missing/empty or the
/// `/v1/models` call fails. Callers must handle the empty case (e.g.
/// fall back to the registry's default model).
pub async fn fetch_models_for(
    provider_id: &str,
    options: &ModelFetchOptions<'_>,
) -> Vec<(String, String)> {
    match provider_id {
        "anthropic" => fetch_anthropic_models(options.api_key).await,
        "openai" => fetch_openai_models(options.api_key).await,
        "ollama" => {
            let base_url = options.base_url.unwrap_or("http://localhost:11434");
            fetch_ollama_models(base_url).await
        }
        _ => {
            let base_url = options.base_url.unwrap_or("");
            fetch_openai_compatible_models(base_url, options.api_key).await
        }
    }
}

/// Fetch models from the Anthropic API.
///
/// Returns `(model_id, display_label)` pairs. Falls back to static defaults on error.
pub(crate) async fn fetch_anthropic_models(cached_key: Option<&str>) -> Vec<(String, String)> {
    let static_defaults = vec![
        (
            "claude-opus-4-6".into(),
            "Claude Opus 4.6 (latest flagship)".into(),
        ),
        ("claude-sonnet-4-6".into(), "Claude Sonnet 4.6".into()),
        ("claude-opus-4-5".into(), "Claude Opus 4.5".into()),
        ("claude-sonnet-4-5".into(), "Claude Sonnet 4.5".into()),
        ("claude-haiku-4-5".into(), "Claude Haiku 4.5 (fast)".into()),
    ];

    let api_key = cached_key
        .map(String::from)
        .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
        .filter(|k| !k.is_empty() && k != crate::config::OAUTH_PLACEHOLDER);

    // Fall back to OAuth token if no API key
    let oauth_token = if api_key.is_none() {
        ironclaw_common::env_helpers::env_or_override("ANTHROPIC_OAUTH_TOKEN")
            .filter(|t| !t.is_empty())
    } else {
        None
    };

    let (key_or_token, is_oauth) = match (api_key, oauth_token) {
        (Some(k), _) => (k, false),
        (None, Some(t)) => (t, true),
        (None, None) => return static_defaults,
    };

    let client = reqwest::Client::new();
    let mut request = client
        .get("https://api.anthropic.com/v1/models")
        .header("anthropic-version", "2023-06-01")
        .timeout(std::time::Duration::from_secs(5));

    if is_oauth {
        request = request
            .bearer_auth(&key_or_token)
            .header("anthropic-beta", "oauth-2025-04-20");
    } else {
        request = request.header("x-api-key", &key_or_token);
    }

    let resp = match request.send().await {
        Ok(r) if r.status().is_success() => r,
        _ => return static_defaults,
    };

    #[derive(serde::Deserialize)]
    struct ModelEntry {
        id: String,
    }
    #[derive(serde::Deserialize)]
    struct ModelsResponse {
        data: Vec<ModelEntry>,
    }

    match resp.json::<ModelsResponse>().await {
        Ok(body) => {
            let mut models: Vec<(String, String)> = body
                .data
                .into_iter()
                .filter(|m| !m.id.contains("embedding") && !m.id.contains("audio"))
                .map(|m| {
                    let label = m.id.clone();
                    (m.id, label)
                })
                .collect();
            if models.is_empty() {
                return static_defaults;
            }
            models.sort_by(|a, b| a.0.cmp(&b.0));
            models
        }
        Err(_) => static_defaults,
    }
}

/// Fetch models from the OpenAI API.
///
/// Returns `(model_id, display_label)` pairs. Falls back to static defaults on error.
pub(crate) async fn fetch_openai_models(cached_key: Option<&str>) -> Vec<(String, String)> {
    let static_defaults = vec![
        (
            "gpt-5.3-codex".into(),
            "GPT-5.3 Codex (latest flagship)".into(),
        ),
        ("gpt-5.2-codex".into(), "GPT-5.2 Codex".into()),
        ("gpt-5.2".into(), "GPT-5.2".into()),
        (
            "gpt-5.1-codex-mini".into(),
            "GPT-5.1 Codex Mini (fast)".into(),
        ),
        ("gpt-5".into(), "GPT-5".into()),
        ("gpt-5-mini".into(), "GPT-5 Mini".into()),
        ("gpt-4.1".into(), "GPT-4.1".into()),
        ("gpt-4.1-mini".into(), "GPT-4.1 Mini".into()),
        ("o4-mini".into(), "o4-mini (fast reasoning)".into()),
        ("o3".into(), "o3 (reasoning)".into()),
    ];

    let api_key = cached_key
        .map(String::from)
        .or_else(|| std::env::var("OPENAI_API_KEY").ok())
        .filter(|k| !k.is_empty());

    let api_key = match api_key {
        Some(k) => k,
        None => return static_defaults,
    };

    let client = reqwest::Client::new();
    let resp = match client
        .get("https://api.openai.com/v1/models")
        .bearer_auth(&api_key)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r,
        _ => return static_defaults,
    };

    #[derive(serde::Deserialize)]
    struct ModelEntry {
        id: String,
    }
    #[derive(serde::Deserialize)]
    struct ModelsResponse {
        data: Vec<ModelEntry>,
    }

    match resp.json::<ModelsResponse>().await {
        Ok(body) => {
            let mut models: Vec<(String, String)> = body
                .data
                .into_iter()
                .filter(|m| is_openai_chat_model(&m.id))
                .map(|m| {
                    let label = m.id.clone();
                    (m.id, label)
                })
                .collect();
            if models.is_empty() {
                return static_defaults;
            }
            sort_openai_models(&mut models);
            models
        }
        Err(_) => static_defaults,
    }
}

pub(crate) fn is_openai_chat_model(model_id: &str) -> bool {
    let id = model_id.to_ascii_lowercase();

    let is_chat_family = id.starts_with("gpt-")
        || id.starts_with("chatgpt-")
        || id.starts_with("o1")
        || id.starts_with("o3")
        || id.starts_with("o4")
        || id.starts_with("o5");

    let is_non_chat_variant = id.contains("realtime")
        || id.contains("audio")
        || id.contains("transcribe")
        || id.contains("tts")
        || id.contains("embedding")
        || id.contains("moderation")
        || id.contains("image");

    is_chat_family && !is_non_chat_variant
}

pub(crate) fn openai_model_priority(model_id: &str) -> usize {
    let id = model_id.to_ascii_lowercase();

    const EXACT_PRIORITY: &[&str] = &[
        "gpt-5.3-codex",
        "gpt-5.2-codex",
        "gpt-5.2",
        "gpt-5.1-codex-mini",
        "gpt-5",
        "gpt-5-mini",
        "gpt-5-nano",
        "o4-mini",
        "o3",
        "o1",
        "gpt-4.1",
        "gpt-4.1-mini",
        "gpt-4o",
        "gpt-4o-mini",
    ];
    if let Some(pos) = EXACT_PRIORITY.iter().position(|m| id == *m) {
        return pos;
    }

    const PREFIX_PRIORITY: &[&str] = &[
        "gpt-5.", "gpt-5-", "o3-", "o4-", "o1-", "gpt-4.1-", "gpt-4o-", "gpt-3.5-", "chatgpt-",
    ];
    if let Some(pos) = PREFIX_PRIORITY
        .iter()
        .position(|prefix| id.starts_with(prefix))
    {
        return EXACT_PRIORITY.len() + pos;
    }

    EXACT_PRIORITY.len() + PREFIX_PRIORITY.len() + 1
}

pub(crate) fn sort_openai_models(models: &mut [(String, String)]) {
    models.sort_by(|a, b| {
        openai_model_priority(&a.0)
            .cmp(&openai_model_priority(&b.0))
            .then_with(|| a.0.cmp(&b.0))
    });
}

/// Fetch installed models from a local Ollama instance.
///
/// Returns `(model_name, display_label)` pairs. Falls back to static defaults on error.
pub(crate) async fn fetch_ollama_models(base_url: &str) -> Vec<(String, String)> {
    let static_defaults = vec![
        ("llama3".into(), "llama3".into()),
        ("mistral".into(), "mistral".into()),
        ("codellama".into(), "codellama".into()),
    ];

    let url = format!("{}/api/tags", base_url.trim_end_matches('/'));
    let client = reqwest::Client::new();

    let resp = match client
        .get(&url)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r,
        Ok(_) => return static_defaults,
        Err(_) => {
            tracing::warn!(
                "Could not connect to Ollama at {base_url}. Is it running? Using static defaults."
            );
            return static_defaults;
        }
    };

    #[derive(serde::Deserialize)]
    struct ModelEntry {
        name: String,
    }
    #[derive(serde::Deserialize)]
    struct TagsResponse {
        models: Vec<ModelEntry>,
    }

    match resp.json::<TagsResponse>().await {
        Ok(body) => {
            let models: Vec<(String, String)> = body
                .models
                .into_iter()
                .map(|m| {
                    let label = m.name.clone();
                    (m.name, label)
                })
                .collect();
            if models.is_empty() {
                return static_defaults;
            }
            models
        }
        Err(_) => static_defaults,
    }
}

/// Fetch models from a generic OpenAI-compatible /v1/models endpoint.
///
/// Used for registry providers like Groq, NVIDIA NIM, etc.
pub(crate) async fn fetch_openai_compatible_models(
    base_url: &str,
    cached_key: Option<&str>,
) -> Vec<(String, String)> {
    if base_url.is_empty() {
        return vec![];
    }

    let url = format!("{}/models", base_url.trim_end_matches('/'));
    let client = reqwest::Client::new();
    let mut req = client.get(&url).timeout(std::time::Duration::from_secs(5));
    if let Some(key) = cached_key {
        req = req.bearer_auth(key);
    }

    let resp = match req.send().await {
        Ok(r) if r.status().is_success() => r,
        _ => return vec![],
    };

    #[derive(serde::Deserialize)]
    struct Model {
        id: String,
    }
    #[derive(serde::Deserialize)]
    struct ModelsResponse {
        data: Vec<Model>,
    }

    match resp.json::<ModelsResponse>().await {
        Ok(body) => body
            .data
            .into_iter()
            .map(|m| {
                let label = m.id.clone();
                (m.id, label)
            })
            .collect(),
        Err(_) => vec![],
    }
}

/// Build the `LlmConfig` used by `fetch_nearai_models` to list available models.
///
/// Uses [`NearAiConfig::for_model_discovery()`] to construct a minimal NEAR AI
/// config, then wraps it in an `LlmConfig` with session config for auth.
pub fn build_nearai_model_fetch_config() -> crate::config::LlmConfig {
    let auth_base_url = ironclaw_common::env_helpers::env_or_override("NEARAI_AUTH_URL")
        .unwrap_or_else(|| "https://private.near.ai".to_string());

    crate::config::LlmConfig {
        backend: "nearai".to_string(),
        session: crate::session::SessionConfig {
            auth_base_url,
            session_path: ironclaw_common::paths::ironclaw_base_dir().join("session.json"),
        },
        nearai: crate::config::NearAiConfig::for_model_discovery(),
        provider: None,
        bedrock: None,
        gemini_oauth: None,
        request_timeout_secs: 120,
        cheap_model: None,
        smart_routing_cascade: false,
        openai_codex: None,
        max_retries: 3,
        circuit_breaker_threshold: None,
        circuit_breaker_recovery_secs: 30,
        response_cache_enabled: false,
        response_cache_ttl_secs: 3600,
        response_cache_max_entries: 1000,
    }
}

#[cfg(test)]
mod classifier_tests {
    use super::*;

    #[test]
    fn is_openai_chat_model_includes_gpt5_and_filters_non_chat_variants() {
        assert!(is_openai_chat_model("gpt-5"));
        assert!(is_openai_chat_model("gpt-5-mini-2026-01-01"));
        assert!(is_openai_chat_model("o3-2025-04-16"));
        assert!(!is_openai_chat_model("chatgpt-image-latest"));
        assert!(!is_openai_chat_model("gpt-4o-realtime-preview"));
        assert!(!is_openai_chat_model("gpt-4o-mini-transcribe"));
        assert!(!is_openai_chat_model("text-embedding-3-large"));
    }

    #[test]
    fn sort_openai_models_prioritizes_best_models_first() {
        let mut models = vec![
            ("gpt-4o-mini".to_string(), "gpt-4o-mini".to_string()),
            ("gpt-5-mini".to_string(), "gpt-5-mini".to_string()),
            ("o3".to_string(), "o3".to_string()),
            ("gpt-4.1".to_string(), "gpt-4.1".to_string()),
            ("gpt-5".to_string(), "gpt-5".to_string()),
        ];

        sort_openai_models(&mut models);

        let ordered: Vec<String> = models.into_iter().map(|(id, _)| id).collect();
        assert_eq!(
            ordered,
            vec![
                "gpt-5".to_string(),
                "gpt-5-mini".to_string(),
                "o3".to_string(),
                "gpt-4.1".to_string(),
                "gpt-4o-mini".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn fetch_ollama_models_unreachable_fallback() {
        // Point at a port nothing listens on.
        let models = fetch_ollama_models("http://127.0.0.1:1").await;
        assert!(!models.is_empty(), "should fall back to static defaults");
    }
}
