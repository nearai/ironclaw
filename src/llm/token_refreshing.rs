//! Token-refreshing LlmProvider decorator for OpenAI Codex.
//!
//! Wraps an `OpenAiCodexProvider` and:
//! - Pre-emptively refreshes the OAuth access token before each call if near expiry
//! - Updates the inner provider's token after refresh (no client rebuild needed)
//! - Retries once on `AuthFailed` / `SessionExpired` after refreshing
//! - Overrides `cost_per_token()` to return (0, 0) since billing is through subscription

use std::sync::Arc;

use async_trait::async_trait;
use rust_decimal::Decimal;
use secrecy::ExposeSecret;

use crate::error::LlmError;
use crate::llm::openai_codex_provider::OpenAiCodexProvider;
use crate::llm::openai_codex_session::OpenAiCodexSessionManager;
use crate::llm::provider::{
    CompletionRequest, CompletionResponse, LlmProvider, ModelMetadata, ToolCompletionRequest,
    ToolCompletionResponse,
};

/// Decorator that refreshes OAuth tokens before API calls and reports zero cost.
///
/// The inner `OpenAiCodexProvider` manages its own token state, so after a
/// refresh we just call `update_token()` -- no client rebuild is needed.
pub struct TokenRefreshingProvider {
    inner: Arc<OpenAiCodexProvider>,
    session: Arc<OpenAiCodexSessionManager>,
}

impl TokenRefreshingProvider {
    pub fn new(inner: Arc<OpenAiCodexProvider>, session: Arc<OpenAiCodexSessionManager>) -> Self {
        Self { inner, session }
    }

    /// Push a fresh token from the session manager into the inner provider.
    async fn update_inner_token(&self) -> Result<(), LlmError> {
        let token = self.session.get_access_token().await?;
        self.inner.update_token(token.expose_secret()).await?;
        tracing::debug!("Updated inner provider token after refresh");
        Ok(())
    }

    /// Best-effort pre-emptive token refresh before an API call.
    ///
    /// If refresh fails (e.g., no refresh token), we log and continue so the
    /// actual request still fires and the retry-on-auth-failure path can kick in.
    async fn ensure_fresh_token(&self) {
        if self.session.needs_refresh().await {
            match self.session.refresh_tokens().await {
                Ok(()) => {
                    if let Err(e) = self.update_inner_token().await {
                        tracing::warn!(
                            "Pre-emptive token update failed: {e}, will retry on auth failure"
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "Pre-emptive token refresh failed: {e}, will retry on auth failure"
                    );
                }
            }
        }
    }
}

#[async_trait]
impl LlmProvider for TokenRefreshingProvider {
    fn model_name(&self) -> &str {
        self.inner.model_name()
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        (Decimal::ZERO, Decimal::ZERO)
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        self.ensure_fresh_token().await;

        match self.inner.complete(request.clone()).await {
            Err(LlmError::AuthFailed { .. } | LlmError::SessionExpired { .. }) => {
                tracing::info!("Auth failure during complete(), refreshing and retrying once");
                self.session.handle_auth_failure().await?;
                self.update_inner_token().await?;
                self.inner.complete(request).await
            }
            other => other,
        }
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        self.ensure_fresh_token().await;

        match self.inner.complete_with_tools(request.clone()).await {
            Err(LlmError::AuthFailed { .. } | LlmError::SessionExpired { .. }) => {
                tracing::info!(
                    "Auth failure during complete_with_tools(), refreshing and retrying once"
                );
                self.session.handle_auth_failure().await?;
                self.update_inner_token().await?;
                self.inner.complete_with_tools(request).await
            }
            other => other,
        }
    }

    async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        self.ensure_fresh_token().await;
        self.inner.list_models().await
    }

    async fn model_metadata(&self) -> Result<ModelMetadata, LlmError> {
        self.ensure_fresh_token().await;
        self.inner.model_metadata().await
    }

    fn active_model_name(&self) -> String {
        self.inner.model_name().to_string()
    }

    fn effective_model_name(&self, requested_model: Option<&str>) -> String {
        self.inner.effective_model_name(requested_model)
    }

    fn set_model(&self, _model: &str) -> Result<(), LlmError> {
        Err(LlmError::RequestFailed {
            provider: "openai_codex".to_string(),
            reason: "Cannot change model on Codex provider at runtime".to_string(),
        })
    }

    fn calculate_cost(&self, _input_tokens: u32, _output_tokens: u32) -> Decimal {
        Decimal::ZERO
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::OpenAiCodexConfig;
    use crate::llm::openai_codex_session::OpenAiCodexSessionManager;

    fn test_codex_config() -> OpenAiCodexConfig {
        OpenAiCodexConfig {
            model: "gpt-5.3-codex".to_string(),
            auth_endpoint: "https://auth.openai.com".to_string(),
            api_base_url: "https://chatgpt.com/backend-api/codex".to_string(),
            client_id: "test_client_id".to_string(),
            session_path: std::path::PathBuf::from("/tmp/test-codex-session.json"),
            token_refresh_margin_secs: 300,
        }
    }

    /// Build a minimal JWT for testing.
    fn make_test_jwt(account_id: &str) -> String {
        use base64::Engine;
        let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let header = engine.encode(b"{\"alg\":\"RS256\",\"typ\":\"JWT\"}");
        let payload_json = serde_json::json!({
            "sub": "user123",
            "https://api.openai.com/auth": {
                "chatgpt_account_id": account_id,
            },
        });
        let payload = engine.encode(payload_json.to_string().as_bytes());
        let sig = engine.encode(b"fake-signature");
        format!("{header}.{payload}.{sig}")
    }

    fn make_provider_and_session() -> TokenRefreshingProvider {
        let config = test_codex_config();
        let jwt = make_test_jwt("acct_test");
        let inner = Arc::new(
            OpenAiCodexProvider::new(&config.model, &config.api_base_url, &jwt)
                .expect("provider creation should succeed"),
        );
        let session = Arc::new(OpenAiCodexSessionManager::new(config));
        TokenRefreshingProvider::new(inner, session)
    }

    #[test]
    fn test_model_name_delegates() {
        let provider = make_provider_and_session();
        assert_eq!(provider.model_name(), "gpt-5.3-codex");
    }

    #[test]
    fn test_cost_per_token_zero() {
        let provider = make_provider_and_session();
        let (input, output) = provider.cost_per_token();
        assert_eq!(input, Decimal::ZERO);
        assert_eq!(output, Decimal::ZERO);
    }

    #[test]
    fn test_calculate_cost_zero() {
        let provider = make_provider_and_session();
        assert_eq!(provider.calculate_cost(1000, 500), Decimal::ZERO);
    }

    #[test]
    fn test_active_model_name_delegates() {
        let provider = make_provider_and_session();
        assert_eq!(provider.active_model_name(), "gpt-5.3-codex");
    }
}
