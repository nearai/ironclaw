//! Tracing-based observer that emits structured log events.
//!
//! Uses the existing `tracing` infrastructure so events appear alongside
//! normal application logs, with no extra dependencies. Good for local
//! development and debugging.

use crate::observability::traits::{Observer, ObserverEvent, ObserverMetric};

/// Observer that logs events and metrics via `tracing`.
pub struct LogObserver;

impl Observer for LogObserver {
    #[allow(clippy::cognitive_complexity, clippy::too_many_lines)] // Exhaustive match over 11 event variants
    fn record_event(&self, event: &ObserverEvent) {
        match event {
            ObserverEvent::AgentStart { provider, model } => {
                tracing::info!(provider, model, "observer: agent.start");
            }
            ObserverEvent::LlmRequest {
                provider,
                model,
                message_count,
                temperature,
                max_tokens,
                thread_id,
            } => {
                tracing::info!(
                    provider,
                    model,
                    message_count,
                    temperature = temperature.unwrap_or(0.0),
                    max_tokens = max_tokens.unwrap_or(0),
                    thread_id = thread_id.as_deref().unwrap_or(""),
                    "observer: llm.request"
                );
            }
            ObserverEvent::LlmResponse {
                provider,
                model,
                duration,
                success,
                error_message,
                input_tokens,
                output_tokens,
                finish_reasons,
                cost_usd,
                cached,
            } => {
                tracing::info!(
                    provider,
                    model,
                    duration_ms = duration.as_millis() as u64,
                    success,
                    cached,
                    error = error_message.as_deref().unwrap_or(""),
                    input_tokens = input_tokens.unwrap_or(0),
                    output_tokens = output_tokens.unwrap_or(0),
                    finish_reasons = ?finish_reasons,
                    cost_usd = cost_usd.unwrap_or(0.0),
                    "observer: llm.response"
                );
            }
            ObserverEvent::ToolCallStart {
                tool,
                call_id,
                thread_id,
            } => {
                tracing::info!(
                    tool,
                    call_id = call_id.as_deref().unwrap_or(""),
                    thread_id = thread_id.as_deref().unwrap_or(""),
                    "observer: tool.start"
                );
            }
            ObserverEvent::ToolCallEnd {
                tool,
                call_id,
                duration,
                success,
                error_message,
            } => {
                tracing::info!(
                    tool,
                    call_id = call_id.as_deref().unwrap_or(""),
                    duration_ms = duration.as_millis() as u64,
                    success,
                    error = error_message.as_deref().unwrap_or(""),
                    "observer: tool.end"
                );
            }
            ObserverEvent::TurnComplete {
                thread_id,
                iteration,
                tool_calls_in_turn,
            } => {
                tracing::info!(
                    thread_id = thread_id.as_deref().unwrap_or(""),
                    iteration,
                    tool_calls_in_turn,
                    "observer: turn.complete"
                );
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
                total_cost_usd,
            } => {
                tracing::info!(
                    duration_secs = duration.as_secs_f64(),
                    tokens_used = tokens_used.unwrap_or(0),
                    total_cost_usd = total_cost_usd.unwrap_or(0.0),
                    "observer: agent.end"
                );
            }
            ObserverEvent::Error { component, message } => {
                tracing::warn!(component, error = message.as_str(), "observer: error");
            }
        }
    }

    #[allow(clippy::cognitive_complexity)] // tracing macros inflate complexity
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
            temperature: Some(0.7),
            max_tokens: Some(4096),
            thread_id: Some("t-1".into()),
        });
        obs.record_event(&ObserverEvent::LlmResponse {
            provider: "nearai".into(),
            model: "test".into(),
            duration: Duration::from_millis(150),
            success: true,
            error_message: None,
            input_tokens: Some(100),
            output_tokens: Some(50),
            finish_reasons: Some(vec!["stop".into()]),
            cost_usd: Some(0.001),
            cached: false,
        });
        obs.record_event(&ObserverEvent::LlmResponse {
            provider: "nearai".into(),
            model: "test".into(),
            duration: Duration::from_millis(1500),
            success: false,
            error_message: Some("timeout".into()),
            input_tokens: None,
            output_tokens: None,
            finish_reasons: None,
            cost_usd: None,
            cached: false,
        });
        obs.record_event(&ObserverEvent::ToolCallStart {
            tool: "shell".into(),
            call_id: None,
            thread_id: None,
        });
        obs.record_event(&ObserverEvent::ToolCallEnd {
            tool: "shell".into(),
            call_id: None,
            duration: Duration::from_millis(20),
            success: true,
            error_message: None,
        });
        obs.record_event(&ObserverEvent::TurnComplete {
            thread_id: Some("t-1".into()),
            iteration: 1,
            tool_calls_in_turn: 2,
        });
        obs.record_event(&ObserverEvent::ChannelMessage {
            channel: "tui".into(),
            direction: "inbound".into(),
        });
        obs.record_event(&ObserverEvent::HeartbeatTick);
        obs.record_event(&ObserverEvent::AgentEnd {
            duration: Duration::from_secs(30),
            tokens_used: Some(2500),
            total_cost_usd: Some(0.05),
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
