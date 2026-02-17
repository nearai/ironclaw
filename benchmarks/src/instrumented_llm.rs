use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use async_trait::async_trait;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use tokio::sync::Mutex;

use ironclaw::error::LlmError;
use ironclaw::llm::{
    CompletionRequest, CompletionResponse, LlmProvider, ToolCompletionRequest,
    ToolCompletionResponse,
};

/// Recorded metrics from a single LLM call.
#[derive(Debug, Clone)]
pub struct LlmCallRecord {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub duration_ms: u64,
    pub had_tool_calls: bool,
}

/// Wraps an `LlmProvider` to record per-call metrics.
///
/// The wrapper is transparent to the agent: it delegates every call
/// to the inner provider and captures token counts and timings.
pub struct InstrumentedLlm {
    inner: Arc<dyn LlmProvider>,
    records: Mutex<Vec<LlmCallRecord>>,
    total_input_tokens: AtomicU32,
    total_output_tokens: AtomicU32,
    call_count: AtomicU32,
}

impl InstrumentedLlm {
    pub fn new(inner: Arc<dyn LlmProvider>) -> Self {
        Self {
            inner,
            records: Mutex::new(Vec::new()),
            total_input_tokens: AtomicU32::new(0),
            total_output_tokens: AtomicU32::new(0),
            call_count: AtomicU32::new(0),
        }
    }

    /// Take all recorded call metrics, clearing the internal buffer.
    pub async fn take_records(&self) -> Vec<LlmCallRecord> {
        let mut records = self.records.lock().await;
        std::mem::take(&mut *records)
    }

    /// Snapshot of total tokens without clearing.
    pub fn total_input_tokens(&self) -> u32 {
        self.total_input_tokens.load(Ordering::Relaxed)
    }

    pub fn total_output_tokens(&self) -> u32 {
        self.total_output_tokens.load(Ordering::Relaxed)
    }

    pub fn call_count(&self) -> u32 {
        self.call_count.load(Ordering::Relaxed)
    }

    /// Estimated cost using the inner provider's cost-per-token rates.
    pub fn estimated_cost(&self) -> f64 {
        let (input_rate, output_rate) = self.inner.cost_per_token();
        let input_cost =
            input_rate * Decimal::from(self.total_input_tokens.load(Ordering::Relaxed));
        let output_cost =
            output_rate * Decimal::from(self.total_output_tokens.load(Ordering::Relaxed));
        let total = input_cost + output_cost;
        total.to_f64().unwrap_or(0.0)
    }

    /// Reset all counters and records.
    pub async fn reset(&self) {
        self.records.lock().await.clear();
        self.total_input_tokens.store(0, Ordering::Relaxed);
        self.total_output_tokens.store(0, Ordering::Relaxed);
        self.call_count.store(0, Ordering::Relaxed);
    }

    async fn record(
        &self,
        input_tokens: u32,
        output_tokens: u32,
        duration_ms: u64,
        had_tool_calls: bool,
    ) {
        self.total_input_tokens
            .fetch_add(input_tokens, Ordering::Relaxed);
        self.total_output_tokens
            .fetch_add(output_tokens, Ordering::Relaxed);
        self.call_count.fetch_add(1, Ordering::Relaxed);
        self.records.lock().await.push(LlmCallRecord {
            input_tokens,
            output_tokens,
            duration_ms,
            had_tool_calls,
        });
    }
}

#[async_trait]
impl LlmProvider for InstrumentedLlm {
    fn model_name(&self) -> &str {
        self.inner.model_name()
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        self.inner.cost_per_token()
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let start = Instant::now();
        let response = self.inner.complete(request).await?;
        let elapsed = start.elapsed().as_millis() as u64;
        self.record(
            response.input_tokens,
            response.output_tokens,
            elapsed,
            false,
        )
        .await;
        Ok(response)
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        let start = Instant::now();
        let response = self.inner.complete_with_tools(request).await?;
        let elapsed = start.elapsed().as_millis() as u64;
        let had_tool_calls = !response.tool_calls.is_empty();
        self.record(
            response.input_tokens,
            response.output_tokens,
            elapsed,
            had_tool_calls,
        )
        .await;
        Ok(response)
    }

    async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        self.inner.list_models().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw::llm::{ChatMessage, CompletionRequest, CompletionResponse, FinishReason};

    /// Fake LLM that returns a canned response with known token counts.
    struct FakeLlm;

    #[async_trait]
    impl LlmProvider for FakeLlm {
        fn model_name(&self) -> &str {
            "fake-model"
        }

        fn cost_per_token(&self) -> (Decimal, Decimal) {
            (
                Decimal::new(3, 6),  // $0.000003 per input token
                Decimal::new(15, 6), // $0.000015 per output token
            )
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            Ok(CompletionResponse {
                content: "test response".to_string(),
                input_tokens: 100,
                output_tokens: 50,
                finish_reason: FinishReason::Stop,
                response_id: None,
            })
        }

        async fn complete_with_tools(
            &self,
            _request: ToolCompletionRequest,
        ) -> Result<ToolCompletionResponse, LlmError> {
            Ok(ToolCompletionResponse {
                content: Some("tool response".to_string()),
                tool_calls: vec![],
                input_tokens: 200,
                output_tokens: 100,
                finish_reason: FinishReason::Stop,
                response_id: None,
            })
        }
    }

    #[tokio::test]
    async fn test_instrumented_records_metrics() {
        let inner = Arc::new(FakeLlm);
        let instrumented = InstrumentedLlm::new(inner);

        let request = CompletionRequest::new(vec![ChatMessage::user("hello")]);
        let _ = instrumented.complete(request).await.unwrap();

        assert_eq!(instrumented.call_count(), 1);
        assert_eq!(instrumented.total_input_tokens(), 100);
        assert_eq!(instrumented.total_output_tokens(), 50);

        let records = instrumented.take_records().await;
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].input_tokens, 100);
        assert!(!records[0].had_tool_calls);
    }

    #[tokio::test]
    async fn test_instrumented_cost_calculation() {
        let inner = Arc::new(FakeLlm);
        let instrumented = InstrumentedLlm::new(inner);

        let request = CompletionRequest::new(vec![ChatMessage::user("hello")]);
        let _ = instrumented.complete(request).await.unwrap();

        // 100 * 0.000003 + 50 * 0.000015 = 0.0003 + 0.00075 = 0.00105
        let cost = instrumented.estimated_cost();
        assert!((cost - 0.00105).abs() < 0.0001);
    }

    #[tokio::test]
    async fn test_instrumented_reset() {
        let inner = Arc::new(FakeLlm);
        let instrumented = InstrumentedLlm::new(inner);

        let request = CompletionRequest::new(vec![ChatMessage::user("hello")]);
        let _ = instrumented.complete(request).await.unwrap();
        assert_eq!(instrumented.call_count(), 1);

        instrumented.reset().await;
        assert_eq!(instrumented.call_count(), 0);
        assert_eq!(instrumented.total_input_tokens(), 0);

        let records = instrumented.take_records().await;
        assert!(records.is_empty());
    }
}
