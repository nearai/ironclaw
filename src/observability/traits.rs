//! Core observer trait and event/metric types.

use std::time::Duration;

/// Provider-agnostic observer for agent lifecycle events and metrics.
///
/// Implementations can log to tracing, export to OpenTelemetry, write to
/// Prometheus, or do nothing at all. The agent records events at key
/// lifecycle points and the observer decides what to do with them.
///
/// Thread-safe and cheaply cloneable behind `Arc<dyn Observer>`.
pub trait Observer: Send + Sync {
    /// Record a discrete lifecycle event.
    fn record_event(&self, event: &ObserverEvent);

    /// Record a numeric metric sample.
    fn record_metric(&self, metric: &ObserverMetric);

    /// Flush any buffered data (e.g. OTLP batch exporter). No-op by default.
    fn flush(&self) {}

    /// Shut down the observer backend, flushing remaining data and releasing
    /// resources. After shutdown, further calls to `record_event` /
    /// `record_metric` may silently no-op. Default implementation calls
    /// `flush()`.
    fn shutdown(&self) {
        self.flush();
    }

    /// Human-readable backend name (e.g. "noop", "log", "otel").
    fn name(&self) -> &str;
}

/// Discrete lifecycle events the agent can emit.
#[derive(Debug, Clone)]
pub enum ObserverEvent {
    /// Agent started processing.
    AgentStart { provider: String, model: String },

    /// An LLM request was sent.
    LlmRequest {
        provider: String,
        model: String,
        message_count: usize,
        /// gen_ai.request.temperature
        temperature: Option<f32>,
        /// gen_ai.request.max_tokens
        max_tokens: Option<u32>,
        /// Thread/conversation identifier.
        thread_id: Option<String>,
    },

    /// An LLM response was received.
    LlmResponse {
        provider: String,
        model: String,
        duration: Duration,
        success: bool,
        error_message: Option<String>,
        /// gen_ai.usage.input_tokens
        input_tokens: Option<u32>,
        /// gen_ai.usage.output_tokens
        output_tokens: Option<u32>,
        /// gen_ai.response.finish_reasons (array per spec)
        finish_reasons: Option<Vec<String>>,
        /// Estimated cost of this call in USD.
        cost_usd: Option<f64>,
        /// Whether this response was served from the response cache.
        /// When `true`, duration is near-zero and no real LLM call was made.
        cached: bool,
    },

    /// A tool call is about to start.
    ToolCallStart {
        tool: String,
        /// Unique call ID for distinguishing concurrent calls to the same tool.
        call_id: Option<String>,
        /// Thread/conversation identifier.
        thread_id: Option<String>,
    },

    /// A tool call finished.
    ToolCallEnd {
        tool: String,
        /// Must match the `call_id` from `ToolCallStart`.
        call_id: Option<String>,
        duration: Duration,
        success: bool,
        /// Error description when `success` is false.
        error_message: Option<String>,
    },

    /// One reasoning turn completed.
    TurnComplete {
        /// Thread/conversation identifier.
        thread_id: Option<String>,
        /// Iteration number within the agentic loop.
        iteration: u32,
        /// Number of tool calls executed in this turn.
        tool_calls_in_turn: u32,
    },

    /// A message was sent or received on a channel.
    ChannelMessage { channel: String, direction: String },

    /// The heartbeat system ran a tick.
    HeartbeatTick,

    /// Agent finished processing.
    AgentEnd {
        duration: Duration,
        tokens_used: Option<u64>,
        /// Total estimated cost in USD for the entire agent invocation.
        total_cost_usd: Option<f64>,
    },

    /// An error occurred in a component.
    Error { component: String, message: String },
}

/// Numeric metric samples.
#[derive(Debug, Clone)]
pub enum ObserverMetric {
    /// Latency of a single request (histogram-style).
    RequestLatency(Duration),
    /// Cumulative tokens consumed.
    TokensUsed(u64),
    /// Current number of active jobs (gauge).
    ActiveJobs(u64),
    /// Current message queue depth (gauge).
    QueueDepth(u64),
}

#[cfg(test)]
mod tests {
    use crate::observability::traits::*;

    #[test]
    fn event_variants_are_constructible() {
        let _ = ObserverEvent::AgentStart {
            provider: "nearai".into(),
            model: "test".into(),
        };
        let _ = ObserverEvent::LlmRequest {
            provider: "nearai".into(),
            model: "test".into(),
            message_count: 3,
            temperature: Some(0.7),
            max_tokens: Some(4096),
            thread_id: Some("thread-1".into()),
        };
        let _ = ObserverEvent::LlmResponse {
            provider: "nearai".into(),
            model: "test".into(),
            duration: Duration::from_millis(100),
            success: true,
            error_message: None,
            input_tokens: Some(150),
            output_tokens: Some(50),
            finish_reasons: Some(vec!["stop".into()]),
            cost_usd: Some(0.001),
            cached: false,
        };
        let _ = ObserverEvent::ToolCallStart {
            tool: "echo".into(),
            call_id: None,
            thread_id: Some("thread-1".into()),
        };
        let _ = ObserverEvent::ToolCallEnd {
            tool: "echo".into(),
            call_id: None,
            duration: Duration::from_millis(5),
            success: true,
            error_message: None,
        };
        let _ = ObserverEvent::TurnComplete {
            thread_id: Some("thread-1".into()),
            iteration: 1,
            tool_calls_in_turn: 2,
        };
        let _ = ObserverEvent::ChannelMessage {
            channel: "tui".into(),
            direction: "inbound".into(),
        };
        let _ = ObserverEvent::HeartbeatTick;
        let _ = ObserverEvent::AgentEnd {
            duration: Duration::from_secs(10),
            tokens_used: Some(1500),
            total_cost_usd: Some(0.05),
        };
        let _ = ObserverEvent::Error {
            component: "llm".into(),
            message: "timeout".into(),
        };
    }

    #[test]
    fn metric_variants_are_constructible() {
        let _ = ObserverMetric::RequestLatency(Duration::from_millis(200));
        let _ = ObserverMetric::TokensUsed(500);
        let _ = ObserverMetric::ActiveJobs(3);
        let _ = ObserverMetric::QueueDepth(10);
    }
}
