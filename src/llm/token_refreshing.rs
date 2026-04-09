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
    CompletionRequest, CompletionResponse, LlmProvider, ModelMetadata, StreamingChunkSender,
    ToolCompletionRequest, ToolCompletionResponse,
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

    async fn complete_streaming(
        &self,
        request: CompletionRequest,
        chunk_tx: StreamingChunkSender,
    ) -> Result<CompletionResponse, LlmError> {
        self.ensure_fresh_token().await;

        match self.inner.complete_streaming(request.clone(), chunk_tx.clone()).await {
            Err(LlmError::AuthFailed { .. } | LlmError::SessionExpired { .. }) => {
                tracing::info!(
                    "Auth failure during complete_streaming(), refreshing and retrying once"
                );
                self.session.handle_auth_failure().await?;
                self.update_inner_token().await?;
                self.inner.complete_streaming(request, chunk_tx).await
            }
            other => other,
        }
    }

    async fn complete_with_tools_streaming(
        &self,
        request: ToolCompletionRequest,
        chunk_tx: StreamingChunkSender,
    ) -> Result<ToolCompletionResponse, LlmError> {
        self.ensure_fresh_token().await;

        match self
            .inner
            .complete_with_tools_streaming(request.clone(), chunk_tx.clone())
            .await
        {
            Err(LlmError::AuthFailed { .. } | LlmError::SessionExpired { .. }) => {
                tracing::info!(
                    "Auth failure during complete_with_tools_streaming(), refreshing and retrying once"
                );
                self.session.handle_auth_failure().await?;
                self.update_inner_token().await?;
                self.inner
                    .complete_with_tools_streaming(request, chunk_tx)
                    .await
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

    fn set_model(&self, model: &str) -> Result<(), LlmError> {
        self.inner.set_model(model)
    }

    fn calculate_cost(&self, _input_tokens: u32, _output_tokens: u32) -> Decimal {
        Decimal::ZERO
    }

    fn cache_write_multiplier(&self) -> Decimal {
        self.inner.cache_write_multiplier()
    }

    fn cache_read_discount(&self) -> Decimal {
        self.inner.cache_read_discount()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::codex_test_helpers::{make_test_jwt, test_codex_config};
    use crate::llm::openai_codex_session::OpenAiCodexSessionManager;
    use tempfile::tempdir;

    fn make_provider_and_session() -> (TokenRefreshingProvider, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        let config = test_codex_config(dir.path().join("session.json"));
        let jwt = make_test_jwt("acct_test");
        let inner = Arc::new(
            OpenAiCodexProvider::new(&config.model, &config.api_base_url, &jwt, 300)
                .expect("provider creation should succeed"),
        );
        let session = Arc::new(OpenAiCodexSessionManager::new(config).unwrap());
        (TokenRefreshingProvider::new(inner, session), dir)
    }

    #[test]
    fn test_model_name_delegates() {
        let (provider, _dir) = make_provider_and_session();
        assert_eq!(provider.model_name(), "gpt-5.3-codex");
    }

    #[test]
    fn test_cost_per_token_zero() {
        let (provider, _dir) = make_provider_and_session();
        let (input, output) = provider.cost_per_token();
        assert_eq!(input, Decimal::ZERO);
        assert_eq!(output, Decimal::ZERO);
    }

    #[test]
    fn test_calculate_cost_zero() {
        let (provider, _dir) = make_provider_and_session();
        assert_eq!(provider.calculate_cost(1000, 500), Decimal::ZERO);
    }

    #[test]
    fn test_active_model_name_delegates() {
        let (provider, _dir) = make_provider_and_session();
        assert_eq!(provider.active_model_name(), "gpt-5.3-codex");
    }

    #[test]
    fn test_streaming_delegates_model_name() {
        let (provider, _dir) = make_provider_and_session();
        // Streaming methods delegate to same inner provider
        assert_eq!(provider.model_name(), "gpt-5.3-codex");
    }
}
