//! Tracing-based observer that emits structured log events.
//!
//! Uses the existing `tracing` infrastructure so events appear alongside
//! normal application logs, with no extra dependencies. Good for local
//! development and debugging.

use crate::observability::traits::{Observer, ObserverEvent, ObserverMetric};

/// Observer that logs events and metrics via `tracing`.
pub struct LogObserver;

impl Observer for LogObserver {
    fn record_event(&self, event: &ObserverEvent) {
        match event {
            ObserverEvent::AgentStart { provider, model } => {
                tracing::info!(provider, model, "observer: agent.start");
            }
            ObserverEvent::LlmRequest {
                provider,
                model,
                message_count,
            } => {
                tracing::info!(provider, model, message_count, "observer: llm.request");
            }
            ObserverEvent::LlmResponse {
                provider,
                model,
                duration,
                success,
                error_message,
            } => {
                tracing::info!(
                    provider,
                    model,
                    duration_ms = duration.as_millis() as u64,
                    success,
                    error = error_message.as_deref().unwrap_or(""),
                    "observer: llm.response"
                );
            }
            ObserverEvent::ToolCallStart { tool } => {
                tracing::info!(tool, "observer: tool.start");
            }
            ObserverEvent::ToolCallEnd {
                tool,
                duration,
                success,
            } => {
                tracing::info!(
                    tool,
                    duration_ms = duration.as_millis() as u64,
                    success,
                    "observer: tool.end"
                );
            }
            ObserverEvent::TurnComplete => {
                tracing::info!("observer: turn.complete");
            }
            ObserverEvent::ChannelMessage { channel, direction } => {
                tracing::info!(channel, direction, "observer: channel.message");
            }
            ObserverEvent::HeartbeatTick => {
                tracing::debug!("observer: heartbeat.tick");
            }
            ObserverEvent::AgentEnd {
                duration,
                tokens_used,
            } => {
                tracing::info!(
                    duration_secs = duration.as_secs_f64(),
                    tokens_used = tokens_used.unwrap_or(0),
                    "observer: agent.end"
                );
            }
            ObserverEvent::Error { component, message } => {
                tracing::warn!(component, error = message.as_str(), "observer: error");
            }
        }
    }

    fn record_metric(&self, metric: &ObserverMetric) {
        match metric {
            ObserverMetric::RequestLatency(d) => {
                tracing::debug!(
                    latency_ms = d.as_millis() as u64,
                    "observer: metric.request_latency"
                );
            }
            ObserverMetric::TokensUsed(n) => {
                tracing::debug!(tokens = n, "observer: metric.tokens_used");
            }
            ObserverMetric::ActiveJobs(n) => {
                tracing::debug!(active_jobs = n, "observer: metric.active_jobs");
            }
            ObserverMetric::QueueDepth(n) => {
                tracing::debug!(queue_depth = n, "observer: metric.queue_depth");
            }
        }
    }

    fn name(&self) -> &str {
        "log"
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::observability::log::LogObserver;
    use crate::observability::traits::*;

    #[test]
    fn name_is_log() {
        assert_eq!(LogObserver.name(), "log");
    }

    #[test]
    fn record_event_does_not_panic() {
        let obs = LogObserver;
        obs.record_event(&ObserverEvent::AgentStart {
            provider: "nearai".into(),
            model: "test".into(),
        });
        obs.record_event(&ObserverEvent::LlmRequest {
            provider: "nearai".into(),
            model: "test".into(),
            message_count: 5,
        });
        obs.record_event(&ObserverEvent::LlmResponse {
            provider: "nearai".into(),
            model: "test".into(),
            duration: Duration::from_millis(150),
            success: true,
            error_message: None,
        });
        obs.record_event(&ObserverEvent::LlmResponse {
            provider: "nearai".into(),
            model: "test".into(),
            duration: Duration::from_millis(1500),
            success: false,
            error_message: Some("timeout".into()),
        });
        obs.record_event(&ObserverEvent::ToolCallStart {
            tool: "shell".into(),
        });
        obs.record_event(&ObserverEvent::ToolCallEnd {
            tool: "shell".into(),
            duration: Duration::from_millis(20),
            success: true,
        });
        obs.record_event(&ObserverEvent::TurnComplete);
        obs.record_event(&ObserverEvent::ChannelMessage {
            channel: "tui".into(),
            direction: "inbound".into(),
        });
        obs.record_event(&ObserverEvent::HeartbeatTick);
        obs.record_event(&ObserverEvent::AgentEnd {
            duration: Duration::from_secs(30),
            tokens_used: Some(2500),
        });
        obs.record_event(&ObserverEvent::Error {
            component: "llm".into(),
            message: "connection refused".into(),
        });
    }

    #[test]
    fn record_metric_does_not_panic() {
        let obs = LogObserver;
        obs.record_metric(&ObserverMetric::RequestLatency(Duration::from_millis(200)));
        obs.record_metric(&ObserverMetric::TokensUsed(1000));
        obs.record_metric(&ObserverMetric::ActiveJobs(5));
        obs.record_metric(&ObserverMetric::QueueDepth(12));
    }

    #[test]
    fn flush_does_not_panic() {
        LogObserver.flush();
    }
}
