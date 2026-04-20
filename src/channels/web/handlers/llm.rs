//! LLM utility handlers: test connection, list models, env defaults.

use std::sync::Arc;

use axum::{Json, extract::State};

use crate::channels::web::auth::{AdminUser, AuthenticatedUser};
use crate::channels::web::server::GatewayState;
use crate::config::helpers::validate_operator_base_url;

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
    AdminUser(user): AdminUser,
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
    if let Err(e) = validate_operator_base_url(&req.base_url, "base_url") {
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
        Ok(r) => interpret_chat_status(r.status()),
        Err(e) => TestConnectionResponse {
            ok: false,
            message: format!("Connection failed: {e}"),
        },
    }
}

/// Pure status-code interpretation, extracted for testability.
fn interpret_chat_status(status: reqwest::StatusCode) -> TestConnectionResponse {
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
        // 400/422 = server reachable but the request was rejected, likely a
        // wrong model name or endpoint variant.  Report as not-ok so the UI
        // doesn't mislead the user with a green badge.
        TestConnectionResponse {
            ok: false,
            message: format!(
                "Server reachable but returned an error ({}). \
                 Check the model name and adapter type.",
                status
            ),
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
    AdminUser(user): AdminUser,
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
    if let Err(e) = validate_operator_base_url(&req.base_url, "base_url") {
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
// Provider list + env defaults (replaces static providers.js)
// ---------------------------------------------------------------------------

/// Returns all builtin LLM provider definitions plus env-var defaults.
///
/// Each entry contains the provider definition (id, name, adapter, base_url,
/// default_model, api_key_required, can_list_models) and env-var overrides
/// (has_api_key presence flag, model override, base_url override).
/// API keys are never returned — only a boolean `has_api_key`.
pub async fn llm_providers_handler(
    AuthenticatedUser(_user): AuthenticatedUser,
) -> Json<serde_json::Value> {
    Json(build_llm_providers())
}

fn build_llm_providers() -> serde_json::Value {
    use crate::config::helpers::optional_env;
    use crate::llm::registry::ProviderRegistry;

    let registry = ProviderRegistry::load();

    // Helper: read env var via optional_env (checks real env + injected overlay).
    // Intentionally swallows ConfigError — this is a best-effort informational
    // endpoint, not a config resolver.
    let read_env = |key: &str| -> Option<String> { optional_env(key).ok().flatten() };

    let mut providers = Vec::new();

    // NEAR AI is not in the registry — add it as a special case.
    {
        let mut entry = serde_json::Map::new();
        entry.insert("id".into(), "nearai".into());
        entry.insert("name".into(), "NEAR AI".into());
        entry.insert("adapter".into(), "nearai".into());
        entry.insert("base_url".into(), "https://cloud-api.near.ai/v1".into());
        entry.insert("builtin".into(), true.into());
        entry.insert(
            "default_model".into(),
            serde_json::Value::String(crate::llm::DEFAULT_MODEL.to_string()),
        );
        entry.insert("api_key_required".into(), true.into());
        entry.insert("base_url_required".into(), false.into());
        entry.insert("can_list_models".into(), true.into());
        // Env defaults
        entry.insert(
            "has_api_key".into(),
            read_env("NEARAI_API_KEY").is_some().into(),
        );
        if let Some(model) = read_env("NEARAI_MODEL") {
            entry.insert("env_model".into(), serde_json::Value::String(model));
        }
        if let Some(url) = read_env("NEARAI_BASE_URL") {
            entry.insert("env_base_url".into(), serde_json::Value::String(url));
        }
        providers.push(serde_json::Value::Object(entry));
    }

    // Registry-based providers
    for def in registry.all() {
        let mut entry = serde_json::Map::new();
        entry.insert("id".into(), serde_json::Value::String(def.id.clone()));
        // Use display_name from setup hint, falling back to titlecased id.
        let name = def
            .setup
            .as_ref()
            .map(|s| s.display_name().to_string())
            .unwrap_or_else(|| def.id.clone());
        entry.insert("name".into(), serde_json::Value::String(name));
        // Serialize protocol as the adapter name the frontend expects.
        let adapter = serde_json::to_value(def.protocol)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| "open_ai_completions".to_string());
        entry.insert("adapter".into(), serde_json::Value::String(adapter));
        entry.insert(
            "base_url".into(),
            serde_json::Value::String(def.default_base_url.clone().unwrap_or_default()),
        );
        entry.insert("builtin".into(), true.into());
        entry.insert(
            "default_model".into(),
            serde_json::Value::String(def.default_model.clone()),
        );
        entry.insert("api_key_required".into(), def.api_key_required.into());
        entry.insert("base_url_required".into(), def.base_url_required.into());
        let can_list = def.setup.as_ref().is_some_and(|s| s.can_list_models());
        entry.insert("can_list_models".into(), can_list.into());
        // Env defaults
        if let Some(ref api_key_env) = def.api_key_env {
            entry.insert("has_api_key".into(), read_env(api_key_env).is_some().into());
        }
        if let Some(model) = read_env(&def.model_env) {
            entry.insert("env_model".into(), serde_json::Value::String(model));
        }
        if let Some(ref base_url_env) = def.base_url_env
            && let Some(url) = read_env(base_url_env)
        {
            entry.insert("env_base_url".into(), serde_json::Value::String(url));
        }
        providers.push(serde_json::Value::Object(entry));
    }

    // Bedrock is not in the registry — add it as a special case.
    {
        let mut entry = serde_json::Map::new();
        entry.insert("id".into(), "bedrock".into());
        entry.insert("name".into(), "AWS Bedrock".into());
        entry.insert("adapter".into(), "bedrock".into());
        entry.insert("base_url".into(), "".into());
        entry.insert("builtin".into(), true.into());
        entry.insert(
            "default_model".into(),
            "anthropic.claude-3-sonnet-20240229-v1:0".into(),
        );
        entry.insert("api_key_required".into(), false.into());
        entry.insert("base_url_required".into(), false.into());
        entry.insert("can_list_models".into(), false.into());
        providers.push(serde_json::Value::Object(entry));
    }

    serde_json::Value::Array(providers)
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
        Some("custom") => crate::settings::custom_secret_name(pid),
        _ => crate::settings::builtin_secret_name(pid),
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

    // --- LLM providers handler tests ---

    fn find_provider<'a>(
        providers: &'a [serde_json::Value],
        id: &str,
    ) -> Option<&'a serde_json::Value> {
        providers
            .iter()
            .find(|p| p.get("id").and_then(|v| v.as_str()) == Some(id))
    }

    #[tokio::test]
    async fn test_llm_providers_returns_nearai_with_env_vars() {
        // SAFETY: test-only; tokio::test runs single-threaded by default.
        unsafe {
            std::env::set_var("NEARAI_API_KEY", "test-key-123");
            std::env::set_var("NEARAI_MODEL", "test-model");
            std::env::set_var("NEARAI_BASE_URL", "https://test.near.ai/v1");
        }

        let result = build_llm_providers();
        let arr = result.as_array().expect("should be an array");

        let nearai = find_provider(arr, "nearai").expect("nearai entry");
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
            nearai.get("env_model").and_then(|v| v.as_str()),
            Some("test-model")
        );
        assert_eq!(
            nearai.get("env_base_url").and_then(|v| v.as_str()),
            Some("https://test.near.ai/v1")
        );
        // Check definition fields are present
        assert_eq!(
            nearai.get("adapter").and_then(|v| v.as_str()),
            Some("nearai")
        );
        assert_eq!(nearai.get("builtin").and_then(|v| v.as_bool()), Some(true));

        // Clean up
        unsafe {
            std::env::remove_var("NEARAI_API_KEY");
            std::env::remove_var("NEARAI_MODEL");
            std::env::remove_var("NEARAI_BASE_URL");
        }
    }

    #[tokio::test]
    async fn test_llm_providers_includes_registry_and_special_providers() {
        let result = build_llm_providers();
        let arr = result.as_array().expect("should be an array");

        // Registry providers should be present
        assert!(
            find_provider(arr, "openai").is_some(),
            "should contain openai"
        );
        assert!(
            find_provider(arr, "anthropic").is_some(),
            "should contain anthropic"
        );
        assert!(
            find_provider(arr, "ollama").is_some(),
            "should contain ollama"
        );

        // Special providers should be present
        assert!(
            find_provider(arr, "nearai").is_some(),
            "should contain nearai"
        );
        assert!(
            find_provider(arr, "bedrock").is_some(),
            "should contain bedrock"
        );

        // Each entry should have required fields
        for p in arr {
            let id = p.get("id").and_then(|v| v.as_str()).unwrap_or("<missing>");
            assert!(p.get("name").is_some(), "{id} missing name");
            assert!(p.get("adapter").is_some(), "{id} missing adapter");
            assert!(p.get("builtin").is_some(), "{id} missing builtin");
            assert!(
                p.get("default_model").is_some(),
                "{id} missing default_model"
            );
            // api_key_required and base_url_required gate frontend activation —
            // both must be present so isProviderConfigured() can reason about them.
            assert!(
                p.get("api_key_required").is_some(),
                "{id} missing api_key_required"
            );
            assert!(
                p.get("base_url_required").is_some(),
                "{id} missing base_url_required"
            );
        }
    }

    #[tokio::test]
    async fn test_openai_compatible_exposes_base_url_required_true() {
        // Regression: openai_compatible has base_url_required=true (no default).
        // The frontend needs this flag to gate activation on a configured URL.
        let result = build_llm_providers();
        let arr = result.as_array().expect("should be an array");
        let oc =
            find_provider(arr, "openai_compatible").expect("openai_compatible should be present");
        assert_eq!(
            oc.get("base_url_required").and_then(|v| v.as_bool()),
            Some(true),
            "openai_compatible must advertise base_url_required=true so the UI gates activation"
        );
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

    // --- interpret_chat_status tests ---

    #[test]
    fn test_interpret_chat_status_400_reports_not_ok() {
        // Regression: 400 was previously reported as ok:true ("Server reachable"),
        // which misled the UI into showing a green "connected" badge when the
        // model name or endpoint was actually wrong.
        let result = interpret_chat_status(reqwest::StatusCode::BAD_REQUEST);
        assert!(!result.ok, "400 must not be reported as ok");
        assert!(
            result.message.contains("400"),
            "message should include status code"
        );
        assert!(
            result.message.contains("model name") || result.message.contains("adapter"),
            "message should hint at model/adapter mismatch, got: {}",
            result.message
        );
    }

    #[test]
    fn test_interpret_chat_status_422_reports_not_ok() {
        let result = interpret_chat_status(reqwest::StatusCode::UNPROCESSABLE_ENTITY);
        assert!(!result.ok, "422 must not be reported as ok");
        assert!(result.message.contains("422"));
    }

    #[test]
    fn test_interpret_chat_status_200_reports_ok() {
        let result = interpret_chat_status(reqwest::StatusCode::OK);
        assert!(result.ok, "200 should be reported as ok");
    }

    #[test]
    fn test_interpret_chat_status_401_reports_auth_failed() {
        let result = interpret_chat_status(reqwest::StatusCode::UNAUTHORIZED);
        assert!(!result.ok);
        assert!(result.message.contains("Authentication"));
    }
}
