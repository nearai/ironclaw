//! InstrumentedLlm -- an LLM provider wrapper that captures per-call metrics.
//!
//! Wraps any `Arc<dyn LlmProvider>` and transparently intercepts `complete()`
//! and `complete_with_tools()` to record timing, token counts, and call metadata.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use async_trait::async_trait;
use rust_decimal::Decimal;
use tokio::sync::Mutex;

use ironclaw::error::LlmError;
use ironclaw::llm::{
    CompletionRequest, CompletionResponse, LlmProvider, ModelMetadata, ToolCompletionRequest,
    ToolCompletionResponse,
};

/// Metrics captured for a single LLM call.
#[derive(Debug, Clone)]
pub struct LlmCallRecord {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub duration_ms: u64,
    pub had_tool_calls: bool,
}

/// A transparent wrapper around any `LlmProvider` that records per-call metrics.
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

    pub fn call_count(&self) -> u32 {
        self.call_count.load(Ordering::Relaxed)
    }

    pub fn total_input_tokens(&self) -> u32 {
        self.total_input_tokens.load(Ordering::Relaxed)
    }

    pub fn total_output_tokens(&self) -> u32 {
        self.total_output_tokens.load(Ordering::Relaxed)
    }

    pub fn estimated_cost_usd(&self) -> f64 {
        let (input_cost, output_cost) = self.inner.cost_per_token();
        let input_total = Decimal::from(self.total_input_tokens());
        let output_total = Decimal::from(self.total_output_tokens());
        let cost = input_cost * input_total + output_cost * output_total;
        use std::str::FromStr;
        f64::from_str(&cost.to_string()).unwrap_or(0.0)
    }

    pub async fn records(&self) -> Vec<LlmCallRecord> {
        self.records.lock().await.clone()
    }

    async fn record_call(
        &self,
        input_tokens: u32,
        output_tokens: u32,
        duration_ms: u64,
        had_tool_calls: bool,
    ) {
        self.call_count.fetch_add(1, Ordering::Relaxed);
        self.total_input_tokens
            .fetch_add(input_tokens, Ordering::Relaxed);
        self.total_output_tokens
            .fetch_add(output_tokens, Ordering::Relaxed);

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
        let result = self.inner.complete(request).await;
        let elapsed = start.elapsed().as_millis() as u64;

        if let Ok(ref resp) = result {
            self.record_call(resp.input_tokens, resp.output_tokens, elapsed, false)
                .await;
        }

        result
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        let start = Instant::now();
        let result = self.inner.complete_with_tools(request).await;
        let elapsed = start.elapsed().as_millis() as u64;

        if let Ok(ref resp) = result {
            let had_tool_calls = !resp.tool_calls.is_empty();
            self.record_call(
                resp.input_tokens,
                resp.output_tokens,
                elapsed,
                had_tool_calls,
            )
            .await;
        }

        result
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
