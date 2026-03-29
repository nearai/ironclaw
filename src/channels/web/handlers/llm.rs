//! LLM utility handlers: test connection, list models, env defaults.

use std::sync::Arc;

use axum::{Json, extract::State};

use crate::channels::web::auth::AuthenticatedUser;
use crate::channels::web::server::GatewayState;
use crate::config::helpers::validate_base_url;

// ---------------------------------------------------------------------------
// Test connection
// ---------------------------------------------------------------------------

/// Fields shared by `test_connection` and `list_models` requests.
///
/// When `api_key` is absent the handler falls back to the encrypted secrets
/// store, using `provider_id` + `provider_type` to locate the vaulted key.
#[derive(serde::Deserialize)]
pub struct TestConnectionRequest {
    adapter: String,
    base_url: String,
    /// Model to use for the test chat completion request.
    model: String,
    #[serde(default)]
    api_key: Option<String>,
    /// Provider identifier used to look up the vaulted API key when `api_key`
    /// is not supplied by the frontend (key already stored in secrets).
    #[serde(default)]
    provider_id: Option<String>,
    /// `"builtin"` or `"custom"` — determines the secret name prefix.
    #[serde(default)]
    provider_type: Option<String>,
}

#[derive(serde::Serialize)]
pub struct TestConnectionResponse {
    ok: bool,
    message: String,
}

pub async fn llm_test_connection_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(mut body): Json<TestConnectionRequest>,
) -> Json<TestConnectionResponse> {
    resolve_api_key_from_secrets(
        &state,
        &user.user_id,
        &mut body.api_key,
        &body.provider_id,
        &body.provider_type,
    )
    .await;
    Json(test_provider_connection(body).await)
}

async fn test_provider_connection(req: TestConnectionRequest) -> TestConnectionResponse {
    if let Err(e) = validate_base_url(&req.base_url, "base_url") {
        return TestConnectionResponse {
            ok: false,
            message: format!("Invalid base URL: {e}"),
        };
    }

    if req.model.trim().is_empty() {
        return TestConnectionResponse {
            ok: false,
            message: "Model is required for connection test".to_string(),
        };
    }

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return TestConnectionResponse {
                ok: false,
                message: format!("Failed to build HTTP client: {e}"),
            };
        }
    };

    let base = req.base_url.trim_end_matches('/');

    match req.adapter.as_str() {
        "anthropic" => {
            let anthropic_base = if base.ends_with("/v1") || base.contains("/v1/") {
                base.to_string()
            } else {
                format!("{base}/v1")
            };
            let url = format!("{anthropic_base}/messages");
            let body = serde_json::json!({
                "model": req.model,
                "max_tokens": 16,
                "messages": [{"role": "user", "content": "hi"}]
            });
            let mut builder = client
                .post(&url)
                .header("anthropic-version", "2023-06-01")
                .json(&body);
            if let Some(key) = req.api_key.as_deref().filter(|k| !k.is_empty()) {
                builder = builder.header("x-api-key", key);
            }
            interpret_chat_response(builder.send().await)
        }
        "ollama" => {
            let url = format!("{base}/api/chat");
            let body = serde_json::json!({
                "model": req.model,
                "messages": [{"role": "user", "content": "hi"}],
                "stream": false
            });
            let builder = client.post(&url).json(&body);
            interpret_chat_response(builder.send().await)
        }
        _ => {
            // OpenAI-compatible (including nearai): POST /v1/chat/completions
            // If base already ends with /v1, append directly; otherwise insert /v1.
            let chat_url = if base.ends_with("/v1") {
                format!("{base}/chat/completions")
            } else {
                format!("{base}/v1/chat/completions")
            };
            let body = serde_json::json!({
                "model": req.model,
                "max_tokens": 16,
                "messages": [{"role": "user", "content": "hi"}]
            });
            let mut builder = client.post(&chat_url).json(&body);
            if let Some(key) = req.api_key.as_deref().filter(|k| !k.is_empty()) {
                builder = builder.header("Authorization", format!("Bearer {key}"));
            }
            interpret_chat_response(builder.send().await)
        }
    }
}

fn interpret_chat_response(
    result: Result<reqwest::Response, reqwest::Error>,
) -> TestConnectionResponse {
    match result {
        Ok(r) => {
            let status = r.status();
            if status.is_success() {
                TestConnectionResponse {
                    ok: true,
                    message: format!("Connected ({})", status),
                }
            } else if status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN
            {
                TestConnectionResponse {
                    ok: false,
                    message: format!("Authentication failed ({})", status),
                }
            } else if status == reqwest::StatusCode::BAD_REQUEST
                || status == reqwest::StatusCode::UNPROCESSABLE_ENTITY
            {
                // 400/422 = server reachable, likely wrong endpoint variant — connectivity OK
                TestConnectionResponse {
                    ok: true,
                    message: format!("Server reachable ({})", status),
                }
            } else if status == reqwest::StatusCode::NOT_FOUND {
                // 404 = /models endpoint not found — server reachable but not OpenAI-compatible
                TestConnectionResponse {
                    ok: false,
                    message: format!(
                        "Server reachable but /models endpoint not found ({}). \
                         Check the base URL and adapter type.",
                        status
                    ),
                }
            } else if status.is_client_error() {
                TestConnectionResponse {
                    ok: false,
                    message: format!("Client error ({})", status),
                }
            } else {
                TestConnectionResponse {
                    ok: false,
                    message: format!("Server error ({})", status),
                }
            }
        }
        Err(e) => TestConnectionResponse {
            ok: false,
            message: format!("Connection failed: {e}"),
        },
    }
}

// ---------------------------------------------------------------------------
// List models
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
pub struct ListModelsRequest {
    adapter: String,
    base_url: String,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    provider_id: Option<String>,
    #[serde(default)]
    provider_type: Option<String>,
}

#[derive(serde::Serialize)]
pub struct ListModelsResponse {
    ok: bool,
    models: Vec<String>,
    message: String,
}

pub async fn llm_list_models_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(mut body): Json<ListModelsRequest>,
) -> Json<ListModelsResponse> {
    resolve_api_key_from_secrets(
        &state,
        &user.user_id,
        &mut body.api_key,
        &body.provider_id,
        &body.provider_type,
    )
    .await;
    Json(fetch_provider_models(body).await)
}

async fn fetch_provider_models(req: ListModelsRequest) -> ListModelsResponse {
    if let Err(e) = validate_base_url(&req.base_url, "base_url") {
        return ListModelsResponse {
            ok: false,
            models: vec![],
            message: format!("Invalid base URL: {e}"),
        };
    }

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return ListModelsResponse {
                ok: false,
                models: vec![],
                message: format!("Failed to build HTTP client: {e}"),
            };
        }
    };

    let base = req.base_url.trim_end_matches('/');
    let auth = req.api_key.as_deref().filter(|k| !k.is_empty());

    match req.adapter.as_str() {
        "ollama" => {
            let url = format!("{base}/api/tags");
            match client.get(&url).send().await {
                Ok(r) if r.status().is_success() => {
                    let body: serde_json::Value = r.json().await.unwrap_or_default();
                    let models: Vec<String> = body["models"]
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|m| m["name"].as_str().map(|s| s.to_string()))
                                .collect()
                        })
                        .unwrap_or_default();
                    if models.is_empty() {
                        ListModelsResponse {
                            ok: false,
                            models: vec![],
                            message: "No models found".to_string(),
                        }
                    } else {
                        ListModelsResponse {
                            ok: true,
                            message: format!("{} model(s) found", models.len()),
                            models,
                        }
                    }
                }
                Ok(r) => ListModelsResponse {
                    ok: false,
                    models: vec![],
                    message: format!("Server returned {}", r.status()),
                },
                Err(e) => ListModelsResponse {
                    ok: false,
                    models: vec![],
                    message: format!("Connection failed: {e}"),
                },
            }
        }
        _ => {
            // OpenAI-compatible, Anthropic, and NEAR AI all support GET /models.
            // NEAR AI private endpoints and Anthropic need a /v1 prefix.
            let effective_base = if (req.adapter == "nearai" && is_nearai_private_endpoint(base))
                || (req.adapter == "anthropic" && !base.ends_with("/v1") && !base.contains("/v1/"))
            {
                format!("{base}/v1")
            } else {
                base.to_string()
            };
            let url = format!("{effective_base}/models");
            let mut builder = client.get(&url);
            if req.adapter == "anthropic" {
                // Anthropic requires a version header and uses x-api-key for authentication
                builder = builder.header("anthropic-version", "2023-06-01");
                if let Some(key) = auth {
                    builder = builder.header("x-api-key", key);
                }
            } else if let Some(key) = auth {
                builder = builder.header("Authorization", format!("Bearer {key}"));
            }
            match builder.send().await {
                Ok(r) if r.status().is_success() => {
                    let body: serde_json::Value = r.json().await.unwrap_or_default();
                    // OpenAI: {"data": [{"id": "..."}]}
                    // Anthropic: {"data": [{"id": "..."}]}
                    let models: Vec<String> = body["data"]
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|m| m["id"].as_str().map(|s| s.to_string()))
                                .collect()
                        })
                        .unwrap_or_default();
                    if models.is_empty() {
                        ListModelsResponse {
                            ok: false,
                            models: vec![],
                            message: "No models found in response".to_string(),
                        }
                    } else {
                        ListModelsResponse {
                            ok: true,
                            message: format!("{} model(s) found", models.len()),
                            models,
                        }
                    }
                }
                Ok(r) => ListModelsResponse {
                    ok: false,
                    models: vec![],
                    message: format!("Server returned {} — list models not supported", r.status()),
                },
                Err(e) => ListModelsResponse {
                    ok: false,
                    models: vec![],
                    message: format!("Connection failed: {e}"),
                },
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Env defaults
// ---------------------------------------------------------------------------

/// Returns env-var-based defaults for each builtin LLM provider.
///
/// The frontend uses these as fallback values when the DB has no overrides.
/// API keys are never returned — only a boolean `has_api_key`.
pub async fn llm_env_defaults_handler(
    AuthenticatedUser(_user): AuthenticatedUser,
) -> Json<serde_json::Value> {
    Json(build_llm_env_defaults())
}

fn build_llm_env_defaults() -> serde_json::Value {
    use crate::config::helpers::optional_env;
    use crate::llm::registry::ProviderRegistry;

    let registry = ProviderRegistry::load();
    let mut defaults = serde_json::Map::new();

    // Helper: read env var via optional_env (checks real env + injected overlay).
    let read_env = |key: &str| -> Option<String> { optional_env(key).ok().flatten() };

    // NEAR AI is a special case (not in the registry)
    {
        let mut entry = serde_json::Map::new();
        // Only expose presence of API key, never the value itself.
        entry.insert(
            "has_api_key".to_string(),
            serde_json::Value::Bool(read_env("NEARAI_API_KEY").is_some()),
        );
        if let Some(model) = read_env("NEARAI_MODEL") {
            entry.insert("model".to_string(), serde_json::Value::String(model));
        }
        if let Some(url) = read_env("NEARAI_BASE_URL") {
            entry.insert("base_url".to_string(), serde_json::Value::String(url));
        }
        defaults.insert("nearai".to_string(), serde_json::Value::Object(entry));
    }

    // Registry-based providers
    for def in registry.all() {
        let mut entry = serde_json::Map::new();

        if let Some(ref api_key_env) = def.api_key_env {
            entry.insert(
                "has_api_key".to_string(),
                serde_json::Value::Bool(read_env(api_key_env).is_some()),
            );
        }

        if let Some(model) = read_env(&def.model_env) {
            entry.insert("model".to_string(), serde_json::Value::String(model));
        }

        if let Some(ref base_url_env) = def.base_url_env
            && let Some(url) = read_env(base_url_env)
        {
            entry.insert("base_url".to_string(), serde_json::Value::String(url));
        }

        defaults.insert(def.id.clone(), serde_json::Value::Object(entry));
    }

    serde_json::Value::Object(defaults)
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// When the frontend doesn't supply an `api_key` (because it was already vaulted),
/// look it up from the encrypted secrets store using `provider_id` + `provider_type`.
async fn resolve_api_key_from_secrets(
    state: &GatewayState,
    user_id: &str,
    api_key: &mut Option<String>,
    provider_id: &Option<String>,
    provider_type: &Option<String>,
) {
    // Already have a key from the request — nothing to resolve.
    if api_key.as_ref().is_some_and(|k| !k.is_empty()) {
        return;
    }
    let pid = match provider_id.as_deref().filter(|s| !s.is_empty()) {
        Some(id) => id,
        None => return,
    };
    let secrets = match state.secrets_store.as_ref() {
        Some(s) => s,
        None => return,
    };
    let secret_name = match provider_type.as_deref() {
        Some("custom") => format!("llm_custom_{}_api_key", pid),
        _ => format!("llm_builtin_{}_api_key", pid),
    };
    if let Ok(decrypted) = secrets.get_decrypted(user_id, &secret_name).await {
        *api_key = Some(decrypted.expose().to_string());
    }
}

/// Check if a base URL belongs to a NEAR AI private endpoint.
///
/// Matches `private.near.ai` exactly or any subdomain of it
/// (e.g. `us.private.near.ai`). Rejects lookalikes like
/// `private-evil.near.ai` or `myprivate.near.ai`.
fn is_nearai_private_endpoint(base_url: &str) -> bool {
    url::Url::parse(base_url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_lowercase()))
        .is_some_and(|host| host == "private.near.ai" || host.ends_with(".private.near.ai"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- LLM env defaults handler tests ---

    #[tokio::test]
    async fn test_llm_env_defaults_returns_nearai_env_vars() {
        // SAFETY: test-only; tokio::test runs single-threaded by default.
        unsafe {
            std::env::set_var("NEARAI_API_KEY", "test-key-123");
            std::env::set_var("NEARAI_MODEL", "test-model");
            std::env::set_var("NEARAI_BASE_URL", "https://test.near.ai/v1");
        }

        let result = build_llm_env_defaults();
        let map = result.as_object().expect("should be an object");

        // Check NEAR AI entry
        let nearai = map
            .get("nearai")
            .and_then(|v| v.as_object())
            .expect("nearai entry");
        // API key should NOT be exposed — only has_api_key presence flag.
        assert_eq!(
            nearai.get("has_api_key").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert!(
            nearai.get("api_key").is_none(),
            "raw api_key must never be returned"
        );
        assert_eq!(
            nearai.get("model").and_then(|v| v.as_str()),
            Some("test-model")
        );
        assert_eq!(
            nearai.get("base_url").and_then(|v| v.as_str()),
            Some("https://test.near.ai/v1")
        );

        // Clean up
        unsafe {
            std::env::remove_var("NEARAI_API_KEY");
            std::env::remove_var("NEARAI_MODEL");
            std::env::remove_var("NEARAI_BASE_URL");
        }
    }

    #[tokio::test]
    async fn test_llm_env_defaults_includes_registry_providers() {
        let result = build_llm_env_defaults();
        let map = result.as_object().expect("should be an object");

        // Registry providers should be present (openai, anthropic, ollama, etc.)
        assert!(map.contains_key("openai"), "should contain openai");
        assert!(map.contains_key("anthropic"), "should contain anthropic");
        assert!(map.contains_key("ollama"), "should contain ollama");

        // Each entry should have has_api_key field
        for (id, val) in map {
            assert!(val.is_object(), "{id} should be an object");
        }
    }

    // --- is_nearai_private_endpoint tests ---

    #[test]
    fn test_nearai_private_exact_match() {
        assert!(is_nearai_private_endpoint("https://private.near.ai/v1"));
    }

    #[test]
    fn test_nearai_private_subdomain() {
        assert!(is_nearai_private_endpoint("https://us.private.near.ai/v1"));
    }

    #[test]
    fn test_nearai_public_endpoint_not_private() {
        assert!(!is_nearai_private_endpoint("https://cloud-api.near.ai/v1"));
    }

    #[test]
    fn test_nearai_private_lookalike_rejected() {
        // "private" appears in the hostname but not as the correct domain
        assert!(!is_nearai_private_endpoint(
            "https://private-evil.near.ai/v1"
        ));
        assert!(!is_nearai_private_endpoint("https://myprivate.near.ai/v1"));
    }

    #[test]
    fn test_nearai_private_non_near_ai_rejected() {
        assert!(!is_nearai_private_endpoint("https://private.evil.com/v1"));
    }
}
