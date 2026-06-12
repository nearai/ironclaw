//! NormalizingProvider — Layer 3 shape-invariant decorator.
//!
//! Sits between any LlmProvider and downstream consumers (RetryProvider,
//! SmartRoutingProvider, etc.). Enforces Class A invariants on the decoded
//! ToolCompletionResponse — invariants that hold for every provider
//! regardless of wire format. Class B quirks (wire-decode dialects,
//! reasoning-field names, arg-parse policy) stay in each provider file.
//!
//! Current invariants:
//! - If response.tool_calls is non-empty and finish_reason is one of the
//!   ambiguous-but-tool-using variants (`Unknown` or `Stop`), upgrade
//!   finish_reason to `ToolUse`. `Length` (truncation) and `ContentFilter`
//!   (policy stop) are deliberately preserved — they carry meaningful error
//!   classification that downstream callers rely on. Closes audit RC1/M1.

use std::sync::Arc;

use async_trait::async_trait;
use rust_decimal::Decimal;

use crate::error::LlmError;
use crate::provider::{
    CompletionRequest, CompletionResponse, FinishReason, LlmProvider, ModelMetadata,
    ToolCompletionRequest, ToolCompletionResponse,
};

/// Decorator that enforces Class A shape invariants on every
/// `ToolCompletionResponse` returned by the wrapped provider.
///
/// Construct via `NormalizingProvider::new(inner)` where `inner` is any
/// `Arc<dyn LlmProvider>`. All methods delegate to the inner provider;
/// only `complete_with_tools` applies normalization before returning.
pub struct NormalizingProvider {
    inner: Arc<dyn LlmProvider>,
}

impl NormalizingProvider {
    /// Wrap an existing provider with shape-invariant normalization.
    pub fn new(inner: Arc<dyn LlmProvider>) -> Self {
        Self { inner }
    }
}

/// Enforce Class A shape invariants on a decoded `ToolCompletionResponse`.
///
/// Invariant RC1/M1: if `tool_calls` is non-empty AND `finish_reason` is one
/// of the ambiguous-but-tool-using variants (`Unknown` or `Stop`), upgrade
/// `finish_reason` to `ToolUse`. `Length` (truncation) and `ContentFilter`
/// (policy stop) are deliberately preserved — they carry meaningful error
/// classification that downstream callers (e.g. `agentic_loop`,
/// `model_gateway`) rely on to discard malformed truncated tool args or
/// surface policy-denied calls.
fn normalize_shape(resp: &mut ToolCompletionResponse) {
    if !resp.tool_calls.is_empty()
        && matches!(
            resp.finish_reason,
            FinishReason::Unknown | FinishReason::Stop
        )
    {
        tracing::debug!(
            tool_call_count = resp.tool_calls.len(),
            "NormalizingProvider rewrote finish_reason to ToolUse (was {:?})",
            resp.finish_reason,
        );
        resp.finish_reason = FinishReason::ToolUse;
    }
}

#[async_trait]
impl LlmProvider for NormalizingProvider {
    fn model_name(&self) -> &str {
        self.inner.model_name()
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        self.inner.cost_per_token()
    }

    fn cache_write_multiplier(&self) -> Decimal {
        self.inner.cache_write_multiplier()
    }

    fn cache_read_discount(&self) -> Decimal {
        self.inner.cache_read_discount()
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        self.inner.complete(request).await
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        let mut resp = self.inner.complete_with_tools(request).await?;
        normalize_shape(&mut resp);
        Ok(resp)
    }

    async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        self.inner.list_models().await
    }

    async fn model_metadata(&self) -> Result<ModelMetadata, LlmError> {
        self.inner.model_metadata().await
    }

    fn effective_model_name(&self, requested_model: Option<&str>) -> String {
        self.inner.effective_model_name(requested_model)
    }

    fn active_model_name(&self) -> String {
        self.inner.active_model_name()
    }

    fn set_model(&self, model: &str) -> Result<(), LlmError> {
        self.inner.set_model(model)
    }

    fn calculate_cost(&self, input_tokens: u32, output_tokens: u32) -> Decimal {
        self.inner.calculate_cost(input_tokens, output_tokens)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

    use async_trait::async_trait;
    use rust_decimal::Decimal;

    use super::NormalizingProvider;
    use crate::error::LlmError;
    use crate::provider::{
        CompletionRequest, CompletionResponse, FinishReason, LlmProvider, ToolCall,
        ToolCompletionRequest, ToolCompletionResponse,
    };

    /// Minimal stub that returns a pre-baked `ToolCompletionResponse`.
    struct StubProvider {
        tool_response: ToolCompletionResponse,
        complete_called: AtomicBool,
        tool_call_count: AtomicU32,
    }

    impl StubProvider {
        fn new(finish_reason: FinishReason, tool_calls: Vec<ToolCall>) -> Self {
            Self {
                tool_response: ToolCompletionResponse {
                    content: None,
                    tool_calls,
                    input_tokens: 0,
                    output_tokens: 0,
                    finish_reason,
                    cache_read_input_tokens: 0,
                    cache_creation_input_tokens: 0,
                    reasoning: None,
                },
                complete_called: AtomicBool::new(false),
                tool_call_count: AtomicU32::new(0),
            }
        }

        fn complete_was_called(&self) -> bool {
            self.complete_called.load(Ordering::Relaxed)
        }

        fn tool_calls_made(&self) -> u32 {
            self.tool_call_count.load(Ordering::Relaxed)
        }
    }

    #[async_trait]
    impl LlmProvider for StubProvider {
        fn model_name(&self) -> &str {
            "stub"
        }

        fn cost_per_token(&self) -> (Decimal, Decimal) {
            (Decimal::ZERO, Decimal::ZERO)
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            self.complete_called.store(true, Ordering::Relaxed);
            Ok(CompletionResponse {
                content: "delegated".to_string(),
                input_tokens: 0,
                output_tokens: 0,
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
            self.tool_call_count.fetch_add(1, Ordering::Relaxed);
            Ok(self.tool_response.clone())
        }
    }

    fn make_tool_call() -> ToolCall {
        ToolCall {
            name: "x".into(),
            id: "1".into(),
            ..Default::default()
        }
    }

    fn make_tool_request() -> ToolCompletionRequest {
        ToolCompletionRequest::new(vec![crate::ChatMessage::user("hi")], vec![])
    }

    fn make_completion_request() -> CompletionRequest {
        CompletionRequest::new(vec![crate::ChatMessage::user("hi")])
    }

    /// RC1/M1: Bedrock (and similar providers) can return FinishReason::Unknown
    /// even when tool_calls are present. The normalizer must upgrade it to ToolUse.
    #[tokio::test]
    async fn normalizes_unknown_finish_with_calls_to_tool_use() {
        let stub = Arc::new(StubProvider::new(
            FinishReason::Unknown,
            vec![make_tool_call()],
        ));
        let provider = NormalizingProvider::new(stub);

        let resp = provider
            .complete_with_tools(make_tool_request())
            .await
            .unwrap();

        assert_eq!(resp.finish_reason, FinishReason::ToolUse);
        assert_eq!(resp.tool_calls.len(), 1);
    }

    /// Stop finish with non-empty calls must also be upgraded to ToolUse.
    #[tokio::test]
    async fn normalizes_stop_finish_with_calls_to_tool_use() {
        let stub = Arc::new(StubProvider::new(
            FinishReason::Stop,
            vec![make_tool_call()],
        ));
        let provider = NormalizingProvider::new(stub);

        let resp = provider
            .complete_with_tools(make_tool_request())
            .await
            .unwrap();

        assert_eq!(resp.finish_reason, FinishReason::ToolUse);
    }

    /// When finish_reason is already ToolUse and calls are present,
    /// the normalizer must not change it (idempotent).
    #[tokio::test]
    async fn does_not_touch_tool_use_already_set() {
        let stub = Arc::new(StubProvider::new(
            FinishReason::ToolUse,
            vec![make_tool_call()],
        ));
        let provider = NormalizingProvider::new(stub);

        let resp = provider
            .complete_with_tools(make_tool_request())
            .await
            .unwrap();

        assert_eq!(resp.finish_reason, FinishReason::ToolUse);
    }

    /// When tool_calls is empty, the normalizer must leave finish_reason alone
    /// even if it is Unknown (only non-empty calls trigger the upgrade).
    #[tokio::test]
    async fn does_not_touch_when_no_calls() {
        let stub = Arc::new(StubProvider::new(FinishReason::Unknown, vec![]));
        let provider = NormalizingProvider::new(stub);

        let resp = provider
            .complete_with_tools(make_tool_request())
            .await
            .unwrap();

        assert_eq!(resp.finish_reason, FinishReason::Unknown);
        assert!(resp.tool_calls.is_empty());
    }

    /// complete() (not complete_with_tools) must delegate unchanged —
    /// the normalizer only operates on the tool path.
    #[tokio::test]
    async fn passes_through_complete_unchanged() {
        let stub = Arc::new(StubProvider::new(FinishReason::Unknown, vec![]));
        let provider = NormalizingProvider::new(stub.clone());

        assert!(!stub.complete_was_called());
        assert_eq!(stub.tool_calls_made(), 0);

        let resp = provider.complete(make_completion_request()).await.unwrap();

        assert!(
            stub.complete_was_called(),
            "complete() must delegate to inner"
        );
        assert_eq!(
            stub.tool_calls_made(),
            0,
            "complete_with_tools must not be called"
        );
        assert_eq!(resp.content, "delegated");
    }

    /// FinishReason::Length carries token-cap-truncation semantics. The
    /// normalizer must preserve it even with non-empty tool_calls, because
    /// truncated args are likely incomplete and downstream callers
    /// (agentic_loop.rs:317, model_gateway.rs:1033) need the original finish
    /// to discard the call or surface BudgetExceeded.
    #[tokio::test]
    async fn does_not_rewrite_length_finish_with_calls() {
        let stub = Arc::new(StubProvider::new(
            FinishReason::Length,
            vec![make_tool_call()],
        ));
        let provider = NormalizingProvider::new(stub);

        let resp = provider
            .complete_with_tools(make_tool_request())
            .await
            .unwrap();

        assert_eq!(resp.finish_reason, FinishReason::Length);
        assert_eq!(resp.tool_calls.len(), 1);
    }

    /// FinishReason::ContentFilter means a safety stop. The normalizer must
    /// preserve it so policy-denied tool calls surface through the right
    /// downstream branch instead of being executed.
    #[tokio::test]
    async fn does_not_rewrite_content_filter_finish_with_calls() {
        let stub = Arc::new(StubProvider::new(
            FinishReason::ContentFilter,
            vec![make_tool_call()],
        ));
        let provider = NormalizingProvider::new(stub);

        let resp = provider
            .complete_with_tools(make_tool_request())
            .await
            .unwrap();

        assert_eq!(resp.finish_reason, FinishReason::ContentFilter);
        assert_eq!(resp.tool_calls.len(), 1);
    }

    /// `?` on inner.complete_with_tools must forward Err unchanged — the
    /// normalizer is a shape-fixup, not an error interceptor.
    #[tokio::test]
    async fn propagates_inner_error_on_complete_with_tools() {
        struct ErrorStub;

        #[async_trait]
        impl LlmProvider for ErrorStub {
            fn model_name(&self) -> &str {
                "error-stub"
            }

            fn cost_per_token(&self) -> (Decimal, Decimal) {
                (Decimal::ZERO, Decimal::ZERO)
            }

            async fn complete(&self, _: CompletionRequest) -> Result<CompletionResponse, LlmError> {
                unimplemented!()
            }

            async fn complete_with_tools(
                &self,
                _: ToolCompletionRequest,
            ) -> Result<ToolCompletionResponse, LlmError> {
                Err(LlmError::RequestFailed {
                    provider: "error-stub".to_string(),
                    reason: "synthetic test failure".to_string(),
                })
            }
        }

        let provider = NormalizingProvider::new(Arc::new(ErrorStub));
        let err = provider
            .complete_with_tools(make_tool_request())
            .await
            .expect_err("expected synthetic RequestFailed");
        assert!(
            matches!(err, LlmError::RequestFailed { .. }),
            "expected RequestFailed, got {err:?}"
        );
    }
}
