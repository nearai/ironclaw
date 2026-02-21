//! Model auto-discovery for LLM backends.
//!
//! rig-core's `CompletionModel` trait has no model listing method, so we make
//! direct HTTP calls to the well-known discovery endpoints.

use std::time::Duration;

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;

use crate::error::LlmError;
use crate::llm::provider::ModelMetadata;

/// Discovery requests timeout (seconds). Matches the wizard's timeout for
/// the same endpoints, with extra margin for slow networks.
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(10);

/// Build a reqwest client with the discovery timeout.
fn build_discovery_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(DISCOVERY_TIMEOUT)
        .build()
        .unwrap_or_else(|e| {
            tracing::warn!(
                "Failed to build discovery HTTP client with timeout: {e}; using default"
            );
            reqwest::Client::new()
        })
}

/// Strip a trailing `/v1` segment from a base URL so callers that configure
/// `base_url = "http://host:port/v1"` don't produce `.../v1/v1/models`.
fn normalize_base_url(mut base: String) -> String {
    while base.ends_with('/') {
        base.pop();
    }
    if base.ends_with("/v1") {
        base.truncate(base.len() - 3);
    }
    base
}

/// Fetches available model information from a provider's API.
#[async_trait]
pub(crate) trait ModelListFetcher: Send + Sync {
    /// List all available model IDs.
    async fn fetch_model_ids(&self) -> Result<Vec<String>, LlmError>;

    /// Fetch metadata for a specific model.
    async fn fetch_model_entry(&self, model_id: &str) -> Result<Option<ModelMetadata>, LlmError>;
}

// ---------------------------------------------------------------------------
// OpenAI-compatible /v1/models
// ---------------------------------------------------------------------------

/// Fetches models from endpoints that implement the OpenAI `/v1/models` list
/// API. Per-model metadata uses `GET /v1/models/{model_id}` and gracefully
/// degrades to `None` when the detail route is not supported (404 or 405).
///
/// Covers: OpenAI direct, OpenAI-compatible providers.
pub(crate) struct OpenAiModelFetcher {
    base_url: String,
    api_key: Option<SecretString>,
    provider_name: String,
    client: reqwest::Client,
}

impl OpenAiModelFetcher {
    pub(crate) fn new(
        base_url: impl Into<String>,
        api_key: Option<SecretString>,
        provider_name: impl Into<String>,
    ) -> Self {
        Self {
            base_url: normalize_base_url(base_url.into()),
            api_key,
            provider_name: provider_name.into(),
            client: build_discovery_client(),
        }
    }

    fn authed_get(&self, url: &str) -> reqwest::RequestBuilder {
        let mut req = self.client.get(url);
        if let Some(ref key) = self.api_key {
            req = req.bearer_auth(key.expose_secret());
        }
        req
    }
}

#[derive(Deserialize)]
struct OpenAiModelsResponse {
    data: Vec<OpenAiModelEntry>,
}

#[derive(Deserialize)]
struct OpenAiModelEntry {
    id: String,
}

#[async_trait]
impl ModelListFetcher for OpenAiModelFetcher {
    async fn fetch_model_ids(&self) -> Result<Vec<String>, LlmError> {
        let url = format!("{}/v1/models", self.base_url);

        let resp = self
            .authed_get(&url)
            .send()
            .await
            .map_err(|e| LlmError::RequestFailed {
                provider: self.provider_name.clone(),
                reason: format!("Model discovery request failed: {}", e),
            })?;

        if !resp.status().is_success() {
            return Err(LlmError::RequestFailed {
                provider: self.provider_name.clone(),
                reason: format!("Model discovery returned HTTP {}", resp.status()),
            });
        }

        let body: OpenAiModelsResponse =
            resp.json().await.map_err(|e| LlmError::RequestFailed {
                provider: self.provider_name.clone(),
                reason: format!("Failed to parse models response: {}", e),
            })?;

        Ok(body.data.into_iter().map(|m| m.id).collect())
    }

    /// Check for a single model via `GET /v1/models/{model_id}`.
    async fn fetch_model_entry(&self, model_id: &str) -> Result<Option<ModelMetadata>, LlmError> {
        let url = format!(
            "{}/v1/models/{}",
            self.base_url,
            urlencoding::encode(model_id)
        );

        let resp = self
            .authed_get(&url)
            .send()
            .await
            .map_err(|e| LlmError::RequestFailed {
                provider: self.provider_name.clone(),
                reason: format!("Model metadata request failed: {}", e),
            })?;

        let status = resp.status();
        // Drain body so the connection can be reused (keep-alive).
        let _ = resp.bytes().await;

        if status.is_success() {
            Ok(Some(ModelMetadata {
                id: model_id.to_string(),
                context_length: None,
            }))
        } else if status == reqwest::StatusCode::NOT_FOUND
            || status == reqwest::StatusCode::METHOD_NOT_ALLOWED
        {
            Ok(None)
        } else {
            Err(LlmError::RequestFailed {
                provider: self.provider_name.clone(),
                reason: format!("Model metadata request returned HTTP {}", status),
            })
        }
    }
}

// ---------------------------------------------------------------------------
// Ollama /api/tags and /api/show
// ---------------------------------------------------------------------------

/// Fetches models from an Ollama instance via `/api/tags`.
pub(crate) struct OllamaModelFetcher {
    base_url: String,
    client: reqwest::Client,
}

impl OllamaModelFetcher {
    pub(crate) fn new(base_url: impl Into<String>) -> Self {
        let mut base = base_url.into();
        while base.ends_with('/') {
            base.pop();
        }
        Self {
            base_url: base,
            client: build_discovery_client(),
        }
    }
}

#[derive(Deserialize)]
struct OllamaTagsResponse {
    models: Vec<OllamaModelEntry>,
}

#[derive(Deserialize)]
struct OllamaModelEntry {
    name: String,
}

#[async_trait]
impl ModelListFetcher for OllamaModelFetcher {
    async fn fetch_model_ids(&self) -> Result<Vec<String>, LlmError> {
        let url = format!("{}/api/tags", self.base_url);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| LlmError::RequestFailed {
                provider: "ollama".to_string(),
                reason: format!("Model discovery request failed: {}", e),
            })?;

        if !resp.status().is_success() {
            return Err(LlmError::RequestFailed {
                provider: "ollama".to_string(),
                reason: format!("Model discovery returned HTTP {}", resp.status()),
            });
        }

        let body: OllamaTagsResponse = resp.json().await.map_err(|e| LlmError::RequestFailed {
            provider: "ollama".to_string(),
            reason: format!("Failed to parse tags response: {}", e),
        })?;

        Ok(body.models.into_iter().map(|m| m.name).collect())
    }

    /// Check for a single model via `POST /api/show`.
    async fn fetch_model_entry(&self, model_id: &str) -> Result<Option<ModelMetadata>, LlmError> {
        let url = format!("{}/api/show", self.base_url);
        let body = serde_json::json!({ "name": model_id });

        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::RequestFailed {
                provider: "ollama".to_string(),
                reason: format!("Model metadata request failed: {}", e),
            })?;

        let status = resp.status();
        // Drain body so the connection can be reused (keep-alive).
        let _ = resp.bytes().await;

        if status.is_success() {
            Ok(Some(ModelMetadata {
                id: model_id.to_string(),
                context_length: None,
            }))
        } else if status == reqwest::StatusCode::NOT_FOUND {
            Ok(None)
        } else {
            Err(LlmError::RequestFailed {
                provider: "ollama".to_string(),
                reason: format!("Model metadata request returned HTTP {}", status),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_parse_models_response() {
        let json = r#"{
            "object": "list",
            "data": [
                {"id": "gpt-4o", "object": "model"},
                {"id": "gpt-4o-mini", "object": "model"},
                {"id": "gpt-3.5-turbo", "object": "model"}
            ]
        }"#;

        let parsed: OpenAiModelsResponse = serde_json::from_str(json).unwrap();
        let ids: Vec<String> = parsed.data.into_iter().map(|m| m.id).collect();
        assert_eq!(ids, vec!["gpt-4o", "gpt-4o-mini", "gpt-3.5-turbo"]);
    }

    #[test]
    fn ollama_parse_tags_response() {
        let json = r#"{
            "models": [
                {"name": "llama3:latest", "size": 4000000000},
                {"name": "mistral:7b", "size": 3800000000}
            ]
        }"#;

        let parsed: OllamaTagsResponse = serde_json::from_str(json).unwrap();
        let names: Vec<String> = parsed.models.into_iter().map(|m| m.name).collect();
        assert_eq!(names, vec!["llama3:latest", "mistral:7b"]);
    }

    #[test]
    fn openai_parse_empty_response() {
        let json = r#"{"object": "list", "data": []}"#;

        let parsed: OpenAiModelsResponse = serde_json::from_str(json).unwrap();
        assert!(parsed.data.is_empty());
    }

    #[test]
    fn ollama_parse_empty_response() {
        let json = r#"{"models": []}"#;

        let parsed: OllamaTagsResponse = serde_json::from_str(json).unwrap();
        assert!(parsed.models.is_empty());
    }

    #[test]
    fn normalize_strips_trailing_v1() {
        assert_eq!(
            normalize_base_url("http://localhost:8000/v1".into()),
            "http://localhost:8000"
        );
        assert_eq!(
            normalize_base_url("http://localhost:8000/v1/".into()),
            "http://localhost:8000"
        );
        assert_eq!(
            normalize_base_url("https://api.openai.com".into()),
            "https://api.openai.com"
        );
    }
}
