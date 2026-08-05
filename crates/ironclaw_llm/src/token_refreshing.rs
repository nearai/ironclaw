//! Token-refreshing LlmProvider decorator for OpenAI Codex.
//!
//! Wraps an `OpenAiCodexProvider` and:
//! - Pre-emptively refreshes the OAuth access token before each call if near expiry
//! - Updates the inner provider's token after refresh (no client rebuild needed)
//! - Retries once on `AuthFailed` / `SessionExpired` after refreshing
//! - Overrides `cost_per_token()` to return (0, 0) since billing is through subscription

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use rust_decimal::Decimal;
use secrecy::ExposeSecret;

use crate::error::LlmError;
use crate::openai_codex_provider::OpenAiCodexProvider;
use crate::openai_codex_session::OpenAiCodexSessionManager;
use crate::provider::{
    CompletionRequest, CompletionResponse, CompletionStreamSink, LlmProvider, ModelMetadata,
    ToolCompletionRequest, ToolCompletionResponse,
};

/// Decorator that refreshes OAuth tokens before API calls and reports zero cost.
///
/// The inner `OpenAiCodexProvider` manages its own token state, so after a
/// refresh we just call `update_token()` -- no client rebuild is needed.
pub struct TokenRefreshingProvider {
    inner: Arc<dyn TokenRefreshingInner>,
    session: Arc<OpenAiCodexSessionManager>,
}

#[async_trait]
trait TokenRefreshingInner: LlmProvider {
    async fn update_token(&self, token: &str) -> Result<(), LlmError>;
}

#[async_trait]
impl TokenRefreshingInner for OpenAiCodexProvider {
    async fn update_token(&self, token: &str) -> Result<(), LlmError> {
        OpenAiCodexProvider::update_token(self, token).await
    }
}

struct AuthRetryStreamSink {
    inner: Arc<dyn CompletionStreamSink>,
    emitted_text: AtomicBool,
}

impl AuthRetryStreamSink {
    fn new(inner: Arc<dyn CompletionStreamSink>) -> Self {
        Self {
            inner,
            emitted_text: AtomicBool::new(false),
        }
    }

    fn emitted_text(&self) -> bool {
        self.emitted_text.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl CompletionStreamSink for AuthRetryStreamSink {
    async fn text_delta(&self, delta: String) {
        if !delta.is_empty() {
            self.emitted_text.store(true, Ordering::Relaxed);
        }
        self.inner.text_delta(delta).await;
    }

    fn supports_text_replacement(&self) -> bool {
        self.inner.supports_text_replacement()
    }

    async fn replace_on_next_text_delta(&self) {
        self.inner.replace_on_next_text_delta().await;
    }

    async fn finish_text_replacement(&self) {
        self.inner.finish_text_replacement().await;
    }
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
    fn provider_id(&self) -> String {
        self.inner.provider_id()
    }

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

    async fn complete_streaming(
        &self,
        request: CompletionRequest,
        sink: Arc<dyn CompletionStreamSink>,
    ) -> Result<CompletionResponse, LlmError> {
        self.ensure_fresh_token().await;

        let attempt_sink = Arc::new(AuthRetryStreamSink::new(Arc::clone(&sink)));

        match self
            .inner
            .complete_streaming(request.clone(), attempt_sink.clone())
            .await
        {
            Err(error @ (LlmError::AuthFailed { .. } | LlmError::SessionExpired { .. }))
                if attempt_sink.emitted_text() =>
            {
                tracing::warn!(
                    error = %error,
                    "Streaming auth failure occurred after emitting text; not retrying"
                );
                Err(error)
            }
            Err(LlmError::AuthFailed { .. } | LlmError::SessionExpired { .. }) => {
                self.session.handle_auth_failure().await?;
                self.update_inner_token().await?;
                self.inner.complete_streaming(request, sink).await
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

    async fn complete_with_tools_streaming(
        &self,
        request: ToolCompletionRequest,
        sink: Arc<dyn CompletionStreamSink>,
    ) -> Result<ToolCompletionResponse, LlmError> {
        self.ensure_fresh_token().await;

        let attempt_sink = Arc::new(AuthRetryStreamSink::new(Arc::clone(&sink)));

        match self
            .inner
            .complete_with_tools_streaming(request.clone(), attempt_sink.clone())
            .await
        {
            Err(error @ (LlmError::AuthFailed { .. } | LlmError::SessionExpired { .. }))
                if attempt_sink.emitted_text() =>
            {
                tracing::warn!(
                    error = %error,
                    "Streaming auth failure occurred after emitting text; not retrying tool completion"
                );
                Err(error)
            }
            Err(LlmError::AuthFailed { .. } | LlmError::SessionExpired { .. }) => {
                self.session.handle_auth_failure().await?;
                self.update_inner_token().await?;
                self.inner
                    .complete_with_tools_streaming(request, sink)
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

    fn fallback_route(
        &self,
        fallback_index: u32,
        requested_model: Option<&str>,
    ) -> Result<crate::ModelFallbackRoute, LlmError> {
        self.inner.fallback_route(fallback_index, requested_model)
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
    use std::sync::Mutex;
    use std::sync::atomic::AtomicUsize;

    use crate::codex_test_helpers::{make_test_jwt, test_codex_config};
    use crate::openai_codex_session::{OpenAiCodexSession, OpenAiCodexSessionManager};
    use crate::provider::{FinishReason, ToolCompletionRequest};
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

    struct StreamingAuthProvider {
        emit_before_auth_failure: bool,
        text_calls: AtomicUsize,
        tool_calls: AtomicUsize,
        token_updates: AtomicUsize,
    }

    impl StreamingAuthProvider {
        fn new(emit_before_auth_failure: bool) -> Self {
            Self {
                emit_before_auth_failure,
                text_calls: AtomicUsize::new(0),
                tool_calls: AtomicUsize::new(0),
                token_updates: AtomicUsize::new(0),
            }
        }

        async fn first_attempt(
            &self,
            calls: &AtomicUsize,
            sink: &Arc<dyn CompletionStreamSink>,
        ) -> Result<(), LlmError> {
            let attempt = calls.fetch_add(1, Ordering::Relaxed);
            if attempt == 0 {
                let delta = if self.emit_before_auth_failure {
                    "partial"
                } else {
                    ""
                };
                sink.text_delta(delta.to_string()).await;
                return Err(LlmError::AuthFailed {
                    provider: "test".to_string(),
                });
            }
            sink.text_delta("replacement".to_string()).await;
            Ok(())
        }
    }

    #[async_trait]
    impl LlmProvider for StreamingAuthProvider {
        fn model_name(&self) -> &str {
            "streaming-auth-test"
        }

        fn cost_per_token(&self) -> (Decimal, Decimal) {
            (Decimal::ZERO, Decimal::ZERO)
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            panic!("streaming test must not use complete()")
        }

        async fn complete_streaming(
            &self,
            _request: CompletionRequest,
            sink: Arc<dyn CompletionStreamSink>,
        ) -> Result<CompletionResponse, LlmError> {
            self.first_attempt(&self.text_calls, &sink).await?;
            Ok(CompletionResponse {
                content: "replacement".to_string(),
                input_tokens: 1,
                output_tokens: 1,
                finish_reason: FinishReason::Stop,
                reasoning: None,
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
            })
        }

        async fn complete_with_tools(
            &self,
            _request: ToolCompletionRequest,
        ) -> Result<ToolCompletionResponse, LlmError> {
            panic!("streaming test must not use complete_with_tools()")
        }

        async fn complete_with_tools_streaming(
            &self,
            _request: ToolCompletionRequest,
            sink: Arc<dyn CompletionStreamSink>,
        ) -> Result<ToolCompletionResponse, LlmError> {
            self.first_attempt(&self.tool_calls, &sink).await?;
            Ok(ToolCompletionResponse {
                content: Some("replacement".to_string()),
                tool_calls: Vec::new(),
                input_tokens: 1,
                output_tokens: 1,
                finish_reason: FinishReason::Stop,
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
                reasoning: None,
                reasoning_details: None,
            })
        }
    }

    #[async_trait]
    impl TokenRefreshingInner for StreamingAuthProvider {
        async fn update_token(&self, _token: &str) -> Result<(), LlmError> {
            self.token_updates.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    #[derive(Default)]
    struct RecordingSink {
        deltas: Mutex<Vec<String>>,
        replacements: AtomicUsize,
        finishes: AtomicUsize,
    }

    #[async_trait]
    impl CompletionStreamSink for RecordingSink {
        async fn text_delta(&self, delta: String) {
            self.deltas.lock().unwrap().push(delta);
        }

        fn supports_text_replacement(&self) -> bool {
            true
        }

        async fn replace_on_next_text_delta(&self) {
            self.replacements.fetch_add(1, Ordering::Relaxed);
        }

        async fn finish_text_replacement(&self) {
            self.finishes.fetch_add(1, Ordering::Relaxed);
        }
    }

    async fn make_streaming_auth_provider(
        emit_before_auth_failure: bool,
    ) -> (
        TokenRefreshingProvider,
        Arc<StreamingAuthProvider>,
        tempfile::TempDir,
    ) {
        let dir = tempdir().unwrap();
        let config = test_codex_config(dir.path().join("session.json"));
        let session = Arc::new(OpenAiCodexSessionManager::new(config).unwrap());
        session
            .set_session(OpenAiCodexSession {
                access_token: make_test_jwt("acct_test"),
                refresh_token: "refresh-token".to_string(),
                expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
                created_at: chrono::Utc::now(),
            })
            .await;
        let inner = Arc::new(StreamingAuthProvider::new(emit_before_auth_failure));
        let provider = TokenRefreshingProvider {
            inner: inner.clone(),
            session,
        };
        (provider, inner, dir)
    }

    #[tokio::test]
    async fn text_stream_retries_auth_failure_before_visible_delta() {
        let (provider, inner, _dir) = make_streaming_auth_provider(false).await;
        let sink = Arc::new(RecordingSink::default());

        let response = provider
            .complete_streaming(CompletionRequest::new(Vec::new()), sink.clone())
            .await
            .expect("auth failure before visible text should retry");

        assert_eq!(response.content, "replacement");
        assert_eq!(sink.deltas.lock().unwrap().as_slice(), ["", "replacement"]);
        assert_eq!(inner.text_calls.load(Ordering::Relaxed), 2);
        assert_eq!(inner.token_updates.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn text_stream_does_not_retry_auth_failure_after_visible_delta() {
        let (provider, inner, _dir) = make_streaming_auth_provider(true).await;
        let sink = Arc::new(RecordingSink::default());

        let result = provider
            .complete_streaming(CompletionRequest::new(Vec::new()), sink.clone())
            .await;

        assert!(matches!(result, Err(LlmError::AuthFailed { .. })));
        assert_eq!(sink.deltas.lock().unwrap().as_slice(), ["partial"]);
        assert_eq!(inner.text_calls.load(Ordering::Relaxed), 1);
        assert_eq!(inner.token_updates.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn tool_stream_retries_auth_failure_before_visible_delta() {
        let (provider, inner, _dir) = make_streaming_auth_provider(false).await;
        let sink = Arc::new(RecordingSink::default());

        let response = provider
            .complete_with_tools_streaming(
                ToolCompletionRequest::new(Vec::new(), Vec::new()),
                sink.clone(),
            )
            .await
            .expect("auth failure before visible text should retry");

        assert_eq!(response.content.as_deref(), Some("replacement"));
        assert_eq!(sink.deltas.lock().unwrap().as_slice(), ["", "replacement"]);
        assert_eq!(inner.tool_calls.load(Ordering::Relaxed), 2);
        assert_eq!(inner.token_updates.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn tool_stream_does_not_retry_auth_failure_after_visible_delta() {
        let (provider, inner, _dir) = make_streaming_auth_provider(true).await;
        let sink = Arc::new(RecordingSink::default());

        let result = provider
            .complete_with_tools_streaming(
                ToolCompletionRequest::new(Vec::new(), Vec::new()),
                sink.clone(),
            )
            .await;

        assert!(matches!(result, Err(LlmError::AuthFailed { .. })));
        assert_eq!(sink.deltas.lock().unwrap().as_slice(), ["partial"]);
        assert_eq!(inner.tool_calls.load(Ordering::Relaxed), 1);
        assert_eq!(inner.token_updates.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn auth_retry_sink_delegates_replacement_capabilities() {
        let inner = Arc::new(RecordingSink::default());
        let sink = AuthRetryStreamSink::new(inner.clone());

        assert!(sink.supports_text_replacement());
        sink.replace_on_next_text_delta().await;
        sink.finish_text_replacement().await;

        assert_eq!(inner.replacements.load(Ordering::Relaxed), 1);
        assert_eq!(inner.finishes.load(Ordering::Relaxed), 1);
    }
}
