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
    },

    /// An LLM response was received.
    LlmResponse {
        provider: String,
        model: String,
        duration: Duration,
        success: bool,
        error_message: Option<String>,
    },

    /// A tool call is about to start.
    ToolCallStart { tool: String },

    /// A tool call finished.
    ToolCallEnd {
        tool: String,
        duration: Duration,
        success: bool,
    },

    /// One reasoning turn completed.
    TurnComplete,

    /// A message was sent or received on a channel.
    ChannelMessage { channel: String, direction: String },

    /// The heartbeat system ran a tick.
    HeartbeatTick,

    /// Agent finished processing.
    AgentEnd {
        duration: Duration,
        tokens_used: Option<u64>,
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
        };
        let _ = ObserverEvent::LlmResponse {
            provider: "nearai".into(),
            model: "test".into(),
            duration: Duration::from_millis(100),
            success: true,
            error_message: None,
        };
        let _ = ObserverEvent::ToolCallStart {
            tool: "echo".into(),
        };
        let _ = ObserverEvent::ToolCallEnd {
            tool: "echo".into(),
            duration: Duration::from_millis(5),
            success: true,
        };
        let _ = ObserverEvent::TurnComplete;
        let _ = ObserverEvent::ChannelMessage {
            channel: "tui".into(),
            direction: "inbound".into(),
        };
        let _ = ObserverEvent::HeartbeatTick;
        let _ = ObserverEvent::AgentEnd {
            duration: Duration::from_secs(10),
            tokens_used: Some(1500),
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
