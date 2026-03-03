//! OpenTelemetry observer backend.
//!
//! Creates OTEL spans with `gen_ai.*` semantic convention attributes at
//! LLM/tool/turn boundaries. Feature-gated behind `otel`.

use std::collections::HashMap;
use std::sync::Mutex;

use opentelemetry::Context;
use opentelemetry::trace::{
    Span as _, SpanKind, Status, TraceContextExt, Tracer, TracerProvider as _,
};
use opentelemetry::{KeyValue, global};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::trace::SdkTracerProvider;

use crate::observability::ObservabilityConfig;
use crate::observability::traits::{Observer, ObserverEvent, ObserverMetric};

/// Generate a HashMap key for a tool span, supporting concurrent same-tool calls.
fn tool_span_key(tool: &str, call_id: Option<&str>) -> String {
    match call_id {
        Some(id) => format!("tool:{}:{}", tool, id),
        None => format!("tool:{}", tool),
    }
}

/// Observer that creates OpenTelemetry spans with `gen_ai.*` attributes.
pub struct OtelObserver {
    tracer: opentelemetry_sdk::trace::SdkTracer,
    /// Active spans keyed by a synthetic ID (e.g. "agent", "llm", "tool:shell:uuid").
    /// Stores the `Context` that owns each span for parent-child propagation.
    /// Use `cx.span()` to access the `SpanRef` for mutations and ending.
    active_spans: Mutex<HashMap<String, Context>>,
    provider: SdkTracerProvider,
}

impl OtelObserver {
    /// Create a new OTEL observer with an OTLP exporter.
    pub fn new(config: &ObservabilityConfig) -> Result<Self, Box<dyn std::error::Error>> {
        let provider = init_otel_pipeline(config)?;
        let tracer = provider.tracer("ironclaw");
        // Set global so tracing-opentelemetry bridge works.
        global::set_tracer_provider(provider.clone());

        Ok(Self {
            tracer,
            active_spans: Mutex::new(HashMap::new()),
            provider,
        })
    }

    /// Create from an existing provider (used in tests with in-memory exporters).
    #[cfg(test)]
    pub fn from_provider(provider: SdkTracerProvider) -> Self {
        let tracer = provider.tracer("ironclaw-test");
        Self {
            tracer,
            active_spans: Mutex::new(HashMap::new()),
            provider,
        }
    }

    /// Acquire the active_spans lock, recovering from poison.
    ///
    /// A poisoned mutex means a previous thread panicked while holding the lock.
    /// For a `HashMap<String, Context>`, the worst-case inconsistency is a
    /// stale or missing span entry — far better than silently losing ALL
    /// telemetry for the rest of the process lifetime.
    fn lock_spans(&self) -> std::sync::MutexGuard<'_, HashMap<String, Context>> {
        self.active_spans.lock().unwrap_or_else(|e| {
            tracing::warn!("active_spans mutex was poisoned, recovering");
            e.into_inner()
        })
    }

    /// Get the agent span's context for parent-child propagation.
    ///
    /// Takes a reference to the already-locked span map to avoid a second
    /// lock acquisition (TOCTOU fix — see D5).
    ///
    /// Falls back to `Context::current()` if no agent span is active.
    fn agent_context(spans: &HashMap<String, Context>) -> Context {
        if let Some(cx) = spans.get("agent") {
            cx.clone()
        } else {
            Context::current()
        }
    }
}

impl Observer for OtelObserver {
    #[allow(clippy::cognitive_complexity, clippy::too_many_lines)] // Exhaustive match over 11 event variants
    fn record_event(&self, event: &ObserverEvent) {
        match event {
            ObserverEvent::AgentStart { provider, model } => {
                let span = self
                    .tracer
                    .span_builder("invoke_agent")
                    .with_kind(SpanKind::Client)
                    .with_attributes(vec![
                        KeyValue::new("gen_ai.operation.name", "invoke_agent"),
                        KeyValue::new("gen_ai.provider.name", provider.clone()),
                        KeyValue::new("gen_ai.request.model", model.clone()),
                    ])
                    .start(&self.tracer);
                // Capture context with the active span for parent-child.
                let cx = Context::current().with_span(span);
                self.lock_spans().insert("agent".to_string(), cx);
            }

            ObserverEvent::LlmRequest {
                provider,
                model,
                message_count,
                temperature,
                max_tokens,
                thread_id,
            } => {
                let mut attrs = vec![
                    KeyValue::new("gen_ai.operation.name", "chat"),
                    KeyValue::new("gen_ai.provider.name", provider.clone()),
                    KeyValue::new("gen_ai.request.model", model.clone()),
                    KeyValue::new("ironclaw.request.message_count", *message_count as i64),
                ];
                if let Some(t) = temperature {
                    attrs.push(KeyValue::new("gen_ai.request.temperature", *t as f64));
                }
                if let Some(m) = max_tokens {
                    attrs.push(KeyValue::new("gen_ai.request.max_tokens", *m as i64));
                }
                if let Some(tid) = thread_id {
                    attrs.push(KeyValue::new("gen_ai.conversation.id", tid.clone()));
                }

                // D5 fix: single lock acquisition for context lookup + insert.
                let mut spans = self.lock_spans();
                let parent_cx = Self::agent_context(&spans);
                let span = self
                    .tracer
                    .span_builder("chat")
                    .with_kind(SpanKind::Client)
                    .with_attributes(attrs)
                    .start_with_context(&self.tracer, &parent_cx);
                let cx = parent_cx.with_span(span);
                spans.insert("llm".to_string(), cx);
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
                if let Some(cx) = self.lock_spans().remove("llm") {
                    let span = cx.span();
                    span.set_attribute(KeyValue::new("gen_ai.provider.name", provider.clone()));
                    span.set_attribute(KeyValue::new("gen_ai.response.model", model.clone()));
                    span.set_attribute(KeyValue::new(
                        "ironclaw.response.duration_ms",
                        duration.as_millis() as i64,
                    ));
                    if let Some(it) = input_tokens {
                        span.set_attribute(KeyValue::new("gen_ai.usage.input_tokens", *it as i64));
                    }
                    if let Some(ot) = output_tokens {
                        span.set_attribute(KeyValue::new("gen_ai.usage.output_tokens", *ot as i64));
                    }
                    if let Some(reasons) = finish_reasons {
                        let arr: Vec<opentelemetry::StringValue> =
                            reasons.iter().map(|r| r.clone().into()).collect();
                        span.set_attribute(KeyValue::new(
                            "gen_ai.response.finish_reasons",
                            opentelemetry::Value::Array(arr.into()),
                        ));
                    }
                    if let Some(cost) = cost_usd {
                        span.set_attribute(KeyValue::new("ironclaw.usage.cost_usd", *cost));
                    }
                    if *cached {
                        span.set_attribute(KeyValue::new("ironclaw.response.cached", true));
                    }
                    if !success {
                        let msg = error_message.as_deref().unwrap_or("unknown error");
                        span.set_status(Status::error(msg.to_string()));
                    }
                    span.end();
                }
            }

            ObserverEvent::ToolCallStart {
                tool,
                call_id,
                thread_id,
            } => {
                let mut attrs = vec![
                    KeyValue::new("gen_ai.operation.name", "execute_tool"),
                    KeyValue::new("ironclaw.tool.name", tool.clone()),
                ];
                if let Some(tid) = thread_id {
                    attrs.push(KeyValue::new("gen_ai.conversation.id", tid.clone()));
                }

                // D5 fix: single lock acquisition for context lookup + insert.
                let mut spans = self.lock_spans();
                let parent_cx = Self::agent_context(&spans);
                let span = self
                    .tracer
                    .span_builder(format!("tool:{}", tool))
                    .with_kind(SpanKind::Internal)
                    .with_attributes(attrs)
                    .start_with_context(&self.tracer, &parent_cx);
                // Key by call_id if provided (supports concurrent same-tool calls).
                let key = tool_span_key(tool, call_id.as_deref());
                let cx = parent_cx.with_span(span);
                spans.insert(key, cx);
            }

            ObserverEvent::ToolCallEnd {
                tool,
                call_id,
                duration,
                success,
                error_message,
            } => {
                let key = tool_span_key(tool, call_id.as_deref());
                if let Some(cx) = self.lock_spans().remove(&key) {
                    let span = cx.span();
                    span.set_attribute(KeyValue::new(
                        "ironclaw.tool.duration_ms",
                        duration.as_millis() as i64,
                    ));
                    span.set_attribute(KeyValue::new("ironclaw.tool.success", *success));
                    if !success {
                        let msg = error_message.as_deref().unwrap_or("tool execution failed");
                        span.set_status(Status::error(msg.to_string()));
                        span.set_attribute(KeyValue::new("error.message", msg.to_string()));
                    }
                    span.end();
                }
            }

            ObserverEvent::TurnComplete {
                thread_id,
                iteration,
                tool_calls_in_turn,
            } => {
                let mut attrs = vec![
                    KeyValue::new("gen_ai.operation.name", "turn_complete"),
                    KeyValue::new("ironclaw.turn.iteration", *iteration as i64),
                    KeyValue::new("ironclaw.turn.tool_calls", *tool_calls_in_turn as i64),
                ];
                if let Some(tid) = thread_id {
                    attrs.push(KeyValue::new("gen_ai.conversation.id", tid.clone()));
                }
                let parent_cx = Self::agent_context(&self.lock_spans());
                let mut span = self
                    .tracer
                    .span_builder("turn_complete")
                    .with_kind(SpanKind::Internal)
                    .with_attributes(attrs)
                    .start_with_context(&self.tracer, &parent_cx);
                span.end();
            }

            ObserverEvent::ChannelMessage { channel, direction } => {
                let parent_cx = Self::agent_context(&self.lock_spans());
                let mut span = self
                    .tracer
                    .span_builder("channel_message")
                    .with_kind(SpanKind::Internal)
                    .with_attributes(vec![
                        KeyValue::new("ironclaw.channel", channel.clone()),
                        KeyValue::new("ironclaw.direction", direction.clone()),
                    ])
                    .start_with_context(&self.tracer, &parent_cx);
                span.end();
            }

            ObserverEvent::HeartbeatTick => {
                let parent_cx = Self::agent_context(&self.lock_spans());
                let mut span = self
                    .tracer
                    .span_builder("heartbeat_tick")
                    .with_kind(SpanKind::Internal)
                    .start_with_context(&self.tracer, &parent_cx);
                span.end();
            }

            ObserverEvent::AgentEnd {
                duration,
                tokens_used,
                total_cost_usd,
            } => {
                // I2 fix: drain ALL remaining child spans (llm, tool:*) before
                // ending the agent span. Early-exit paths (iteration limit,
                // interrupt, cost guardrail) may emit AgentEnd without closing
                // child spans first. Without this cleanup, orphaned Context/Span
                // objects stay in the map forever and are never properly exported.
                let mut spans = self.lock_spans();
                let agent_cx = spans.remove("agent");

                // Drain remaining entries — these are orphaned child spans.
                let orphans: Vec<(String, Context)> = spans.drain().collect();
                drop(spans); // Release lock before span operations

                for (key, cx) in orphans {
                    let span = cx.span();
                    span.set_status(Status::error(format!(
                        "span '{}' orphaned at agent shutdown",
                        key
                    )));
                    span.end();
                }

                if let Some(cx) = agent_cx {
                    let span = cx.span();
                    span.set_attribute(KeyValue::new(
                        "ironclaw.agent.duration_secs",
                        duration.as_secs_f64(),
                    ));
                    if let Some(tokens) = tokens_used {
                        span.set_attribute(KeyValue::new(
                            "ironclaw.usage.total_tokens",
                            *tokens as i64,
                        ));
                    }
                    if let Some(cost) = total_cost_usd {
                        span.set_attribute(KeyValue::new("ironclaw.usage.total_cost_usd", *cost));
                    }
                    span.end();
                }
            }

            ObserverEvent::Error { component, message } => {
                let parent_cx = Self::agent_context(&self.lock_spans());
                let mut span = self
                    .tracer
                    .span_builder("error")
                    .with_kind(SpanKind::Internal)
                    .with_attributes(vec![
                        KeyValue::new("ironclaw.component", component.clone()),
                        KeyValue::new("error.message", message.clone()),
                    ])
                    .start_with_context(&self.tracer, &parent_cx);
                span.set_status(Status::error(message.clone()));
                span.end();
            }
        }
    }

    fn record_metric(&self, _metric: &ObserverMetric) {
        // Metrics are deferred to a future phase (OTEL metrics API).
    }

    fn flush(&self) {
        if let Err(e) = self.provider.force_flush() {
            tracing::warn!("Failed to flush OTEL provider: {:?}", e);
        }
    }

    fn shutdown(&self) {
        if let Err(e) = self.provider.shutdown() {
            tracing::warn!("Failed to shutdown OTEL provider: {:?}", e);
        }
    }

    fn name(&self) -> &str {
        "otel"
    }
}

/// Initialize the OTEL tracing pipeline with an OTLP exporter.
fn init_otel_pipeline(
    config: &ObservabilityConfig,
) -> Result<SdkTracerProvider, Box<dyn std::error::Error>> {
    let endpoint = config
        .otel_endpoint
        .as_deref()
        .unwrap_or("http://localhost:4317");
    let service_name = config
        .otel_service_name
        .clone()
        .unwrap_or_else(|| "ironclaw".to_string());

    let exporter = match config.otel_protocol.as_deref() {
        Some("http") => opentelemetry_otlp::SpanExporter::builder()
            .with_http()
            .with_endpoint(endpoint)
            .build()?,
        _ => opentelemetry_otlp::SpanExporter::builder()
            .with_tonic()
            .with_endpoint(endpoint)
            .build()?,
    };

    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(Resource::builder().with_service_name(service_name).build())
        .build();

    Ok(provider)
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};

    use super::*;
    use crate::observability::traits::ObserverEvent;

    fn test_observer() -> (OtelObserver, InMemorySpanExporter) {
        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        let obs = OtelObserver::from_provider(provider);
        (obs, exporter)
    }

    #[test]
    fn name_is_otel() {
        let (obs, _) = test_observer();
        assert_eq!(obs.name(), "otel");
    }

    #[test]
    fn test_otel_llm_request_response_spans() {
        let (obs, exporter) = test_observer();

        obs.record_event(&ObserverEvent::LlmRequest {
            provider: "openai".into(),
            model: "gpt-4".into(),
            message_count: 3,
            temperature: Some(0.7),
            max_tokens: Some(4096),
            thread_id: Some("t-1".into()),
        });

        obs.record_event(&ObserverEvent::LlmResponse {
            provider: "openai".into(),
            model: "gpt-4".into(),
            duration: Duration::from_millis(500),
            success: true,
            error_message: None,
            input_tokens: Some(100),
            output_tokens: Some(50),
            finish_reasons: Some(vec!["stop".into()]),
            cost_usd: Some(0.003),
            cached: false,
        });

        // Force flush to get spans
        obs.flush();

        let spans = exporter.get_finished_spans().unwrap();
        assert_eq!(
            spans.len(),
            1,
            "LlmRequest + LlmResponse should produce one span"
        );

        let span = &spans[0];
        assert_eq!(span.name, "chat");

        // Check gen_ai attributes
        let attrs: HashMap<_, _> = span
            .attributes
            .iter()
            .map(|kv| (kv.key.as_str().to_string(), kv.value.clone()))
            .collect();

        assert_eq!(attrs.get("gen_ai.operation.name").unwrap().as_str(), "chat");
        assert_eq!(
            attrs.get("gen_ai.provider.name").unwrap().as_str(),
            "openai"
        );
        assert!(attrs.contains_key("gen_ai.usage.input_tokens"));
        assert!(attrs.contains_key("gen_ai.usage.output_tokens"));
        assert!(attrs.contains_key("gen_ai.response.finish_reasons"));
    }

    #[test]
    fn test_otel_tool_span() {
        let (obs, exporter) = test_observer();

        obs.record_event(&ObserverEvent::ToolCallStart {
            tool: "shell".into(),
            call_id: None,
            thread_id: None,
        });
        obs.record_event(&ObserverEvent::ToolCallEnd {
            tool: "shell".into(),
            call_id: None,
            duration: Duration::from_millis(100),
            success: true,
            error_message: None,
        });

        obs.flush();
        let spans = exporter.get_finished_spans().unwrap();
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].name, "tool:shell");

        let attrs: HashMap<_, _> = spans[0]
            .attributes
            .iter()
            .map(|kv| (kv.key.as_str().to_string(), kv.value.clone()))
            .collect();
        assert_eq!(
            attrs.get("gen_ai.operation.name").unwrap().as_str(),
            "execute_tool"
        );
    }

    #[test]
    fn test_otel_tool_error_span() {
        let (obs, exporter) = test_observer();

        obs.record_event(&ObserverEvent::ToolCallStart {
            tool: "http".into(),
            call_id: None,
            thread_id: None,
        });
        obs.record_event(&ObserverEvent::ToolCallEnd {
            tool: "http".into(),
            call_id: None,
            duration: Duration::from_millis(50),
            success: false,
            error_message: Some("connection refused".into()),
        });

        obs.flush();
        let spans = exporter.get_finished_spans().unwrap();
        assert_eq!(spans.len(), 1);

        let span = &spans[0];
        assert!(
            matches!(span.status, Status::Error { .. }),
            "Failed tool should produce error status"
        );
    }

    #[test]
    fn test_otel_all_event_types_produce_spans() {
        let (obs, exporter) = test_observer();

        // Fire every variant
        obs.record_event(&ObserverEvent::AgentStart {
            provider: "test".into(),
            model: "m".into(),
        });
        obs.record_event(&ObserverEvent::LlmRequest {
            provider: "test".into(),
            model: "m".into(),
            message_count: 1,
            temperature: None,
            max_tokens: None,
            thread_id: None,
        });
        obs.record_event(&ObserverEvent::LlmResponse {
            provider: "test".into(),
            model: "m".into(),
            duration: Duration::from_millis(10),
            success: true,
            error_message: None,
            input_tokens: None,
            output_tokens: None,
            finish_reasons: None,
            cost_usd: None,
            cached: false,
        });
        obs.record_event(&ObserverEvent::ToolCallStart {
            tool: "echo".into(),
            call_id: None,
            thread_id: None,
        });
        obs.record_event(&ObserverEvent::ToolCallEnd {
            tool: "echo".into(),
            call_id: None,
            duration: Duration::from_millis(1),
            success: true,
            error_message: None,
        });
        obs.record_event(&ObserverEvent::TurnComplete {
            thread_id: None,
            iteration: 1,
            tool_calls_in_turn: 1,
        });
        obs.record_event(&ObserverEvent::ChannelMessage {
            channel: "tui".into(),
            direction: "in".into(),
        });
        obs.record_event(&ObserverEvent::HeartbeatTick);
        obs.record_event(&ObserverEvent::AgentEnd {
            duration: Duration::from_secs(1),
            tokens_used: Some(100),
            total_cost_usd: None,
        });
        obs.record_event(&ObserverEvent::Error {
            component: "test".into(),
            message: "oops".into(),
        });

        obs.flush();
        let spans = exporter.get_finished_spans().unwrap();
        // AgentStart creates an open span, ended by AgentEnd → 1 span
        // LlmRequest+Response → 1 span
        // ToolCallStart+End → 1 span
        // TurnComplete → 1 span
        // ChannelMessage → 1 span
        // HeartbeatTick → 1 span
        // Error → 1 span
        // Total: 7
        assert_eq!(
            spans.len(),
            7,
            "Expected exactly 7 spans, got {}",
            spans.len()
        );
    }

    #[test]
    fn test_otel_agent_lifecycle_span() {
        let (obs, exporter) = test_observer();

        obs.record_event(&ObserverEvent::AgentStart {
            provider: "nearai".into(),
            model: "claude".into(),
        });
        obs.record_event(&ObserverEvent::AgentEnd {
            duration: Duration::from_secs(5),
            tokens_used: Some(2000),
            total_cost_usd: Some(0.10),
        });

        obs.flush();
        let spans = exporter.get_finished_spans().unwrap();
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].name, "invoke_agent");

        let attrs: HashMap<_, _> = spans[0]
            .attributes
            .iter()
            .map(|kv| (kv.key.as_str().to_string(), kv.value.clone()))
            .collect();
        assert!(attrs.contains_key("ironclaw.usage.total_tokens"));
        assert!(attrs.contains_key("ironclaw.usage.total_cost_usd"));
    }

    #[test]
    fn test_otel_span_hierarchy() {
        // H2: LLM and tool spans should be children of the agent span.
        let (obs, exporter) = test_observer();

        obs.record_event(&ObserverEvent::AgentStart {
            provider: "test".into(),
            model: "m".into(),
        });
        obs.record_event(&ObserverEvent::LlmRequest {
            provider: "test".into(),
            model: "m".into(),
            message_count: 1,
            temperature: None,
            max_tokens: None,
            thread_id: None,
        });
        obs.record_event(&ObserverEvent::LlmResponse {
            provider: "test".into(),
            model: "m".into(),
            duration: Duration::from_millis(10),
            success: true,
            error_message: None,
            input_tokens: None,
            output_tokens: None,
            finish_reasons: None,
            cost_usd: None,
            cached: false,
        });
        obs.record_event(&ObserverEvent::ToolCallStart {
            tool: "echo".into(),
            call_id: Some("c1".into()),
            thread_id: None,
        });
        obs.record_event(&ObserverEvent::ToolCallEnd {
            tool: "echo".into(),
            call_id: Some("c1".into()),
            duration: Duration::from_millis(1),
            success: true,
            error_message: None,
        });
        obs.record_event(&ObserverEvent::AgentEnd {
            duration: Duration::from_secs(1),
            tokens_used: None,
            total_cost_usd: None,
        });

        obs.flush();
        let spans = exporter.get_finished_spans().unwrap();
        assert_eq!(spans.len(), 3, "agent + llm + tool = 3 spans");

        // Find the agent span (root)
        let agent_span = spans.iter().find(|s| s.name == "invoke_agent").unwrap();
        let agent_span_id = agent_span.span_context.span_id();

        // LLM and tool spans should reference the agent span as parent
        let llm_span = spans.iter().find(|s| s.name == "chat").unwrap();
        assert_eq!(
            llm_span.parent_span_id, agent_span_id,
            "LLM span should be a child of the agent span"
        );

        let tool_span = spans.iter().find(|s| s.name == "tool:echo").unwrap();
        assert_eq!(
            tool_span.parent_span_id, agent_span_id,
            "Tool span should be a child of the agent span"
        );
    }

    /// Regression test for I9: ChannelMessage and HeartbeatTick spans should
    /// be children of the agent span, not orphaned roots.
    ///
    /// Before the fix, both used `.start(&self.tracer)` without parent context,
    /// creating disconnected root spans in Jaeger. Other event types (LlmRequest,
    /// ToolCallStart, TurnComplete) correctly used `start_with_context`.
    #[test]
    fn channel_and_heartbeat_spans_are_children_of_agent() {
        let (obs, exporter) = test_observer();

        obs.record_event(&ObserverEvent::AgentStart {
            provider: "test".into(),
            model: "m".into(),
        });
        obs.record_event(&ObserverEvent::ChannelMessage {
            channel: "web".into(),
            direction: "inbound".into(),
        });
        obs.record_event(&ObserverEvent::HeartbeatTick);
        obs.record_event(&ObserverEvent::AgentEnd {
            duration: Duration::from_secs(1),
            tokens_used: None,
            total_cost_usd: None,
        });

        obs.flush();
        let spans = exporter.get_finished_spans().unwrap();
        assert_eq!(
            spans.len(),
            3,
            "agent + channel_message + heartbeat = 3 spans"
        );

        let agent_span = spans.iter().find(|s| s.name == "invoke_agent").unwrap();
        let agent_span_id = agent_span.span_context.span_id();

        let channel_span = spans.iter().find(|s| s.name == "channel_message").unwrap();
        assert_eq!(
            channel_span.parent_span_id, agent_span_id,
            "ChannelMessage span should be a child of the agent span, not an orphaned root"
        );

        let heartbeat_span = spans.iter().find(|s| s.name == "heartbeat_tick").unwrap();
        assert_eq!(
            heartbeat_span.parent_span_id, agent_span_id,
            "HeartbeatTick span should be a child of the agent span, not an orphaned root"
        );
    }

    /// Regression test for f-1 (PR #334 review): Error spans should be
    /// children of the agent span, not orphaned roots.
    ///
    /// Before the fix, the `ObserverEvent::Error` handler used
    /// `.start(&self.tracer)` (implicitly `Context::current()` as parent),
    /// while every other event handler correctly used `agent_context()` +
    /// `start_with_context()`. This made Error spans appear as disconnected
    /// root spans in Jaeger, breaking the trace hierarchy.
    #[test]
    fn error_span_is_child_of_agent() {
        let (obs, exporter) = test_observer();

        obs.record_event(&ObserverEvent::AgentStart {
            provider: "test".into(),
            model: "m".into(),
        });
        obs.record_event(&ObserverEvent::Error {
            component: "llm".into(),
            message: "connection refused".into(),
        });
        obs.record_event(&ObserverEvent::AgentEnd {
            duration: Duration::from_secs(1),
            tokens_used: None,
            total_cost_usd: None,
        });

        obs.flush();
        let spans = exporter.get_finished_spans().unwrap();
        assert_eq!(spans.len(), 2, "agent + error = 2 spans");

        let agent_span = spans.iter().find(|s| s.name == "invoke_agent").unwrap();
        let agent_span_id = agent_span.span_context.span_id();

        let error_span = spans.iter().find(|s| s.name == "error").unwrap();
        assert_eq!(
            error_span.parent_span_id, agent_span_id,
            "Error span should be a child of the agent span, not an orphaned root"
        );
    }

    #[test]
    fn test_otel_concurrent_tool_spans() {
        // H3: Two concurrent calls to the same tool should produce distinct spans.
        let (obs, exporter) = test_observer();

        obs.record_event(&ObserverEvent::ToolCallStart {
            tool: "shell".into(),
            call_id: Some("id-a".into()),
            thread_id: None,
        });
        obs.record_event(&ObserverEvent::ToolCallStart {
            tool: "shell".into(),
            call_id: Some("id-b".into()),
            thread_id: None,
        });

        // End in reverse order to prove they're independent
        obs.record_event(&ObserverEvent::ToolCallEnd {
            tool: "shell".into(),
            call_id: Some("id-b".into()),
            duration: Duration::from_millis(10),
            success: true,
            error_message: None,
        });
        obs.record_event(&ObserverEvent::ToolCallEnd {
            tool: "shell".into(),
            call_id: Some("id-a".into()),
            duration: Duration::from_millis(20),
            success: false,
            error_message: Some("failed".into()),
        });

        obs.flush();
        let spans = exporter.get_finished_spans().unwrap();
        assert_eq!(spans.len(), 2, "Two concurrent tool calls = 2 spans");

        // Both should be named tool:shell
        assert!(spans.iter().all(|s| s.name == "tool:shell"));

        // They should have different span IDs
        assert_ne!(
            spans[0].span_context.span_id(),
            spans[1].span_context.span_id(),
            "Concurrent tool spans should have distinct IDs"
        );
    }

    #[test]
    fn test_otel_unmatched_end_does_not_panic() {
        // M10: End events without matching starts should not panic.
        let (obs, _exporter) = test_observer();

        obs.record_event(&ObserverEvent::LlmResponse {
            provider: "test".into(),
            model: "m".into(),
            duration: Duration::from_millis(10),
            success: true,
            error_message: None,
            input_tokens: None,
            output_tokens: None,
            finish_reasons: None,
            cost_usd: None,
            cached: false,
        });
        obs.record_event(&ObserverEvent::ToolCallEnd {
            tool: "echo".into(),
            call_id: None,
            duration: Duration::from_millis(1),
            success: true,
            error_message: None,
        });
        obs.record_event(&ObserverEvent::AgentEnd {
            duration: Duration::from_secs(1),
            tokens_used: None,
            total_cost_usd: None,
        });
        // No panic = test passes
    }

    /// Regression test for D5: agent_context lookup and span insertion must
    /// share a single lock acquisition to prevent TOCTOU races.
    ///
    /// Before the fix, `agent_context()` acquired `lock_spans()`, read the
    /// "agent" key, and released the lock. Then the caller acquired the lock
    /// again to insert the new span. Under concurrent use, another thread
    /// could fire `AgentEnd` between the two acquisitions, draining the map
    /// and ending the agent span — leaving the newly inserted span orphaned
    /// (inserted after AgentEnd already cleaned up).
    ///
    /// After the fix, `agent_context()` no longer acquires its own lock.
    /// Instead, callers hold a single `MutexGuard` across both the context
    /// lookup and the insert, making the race structurally impossible.
    #[test]
    fn concurrent_tool_starts_have_correct_parent() {
        use std::sync::Arc;

        let (obs, exporter) = test_observer();
        let obs = Arc::new(obs);

        obs.record_event(&ObserverEvent::AgentStart {
            provider: "test".into(),
            model: "m".into(),
        });

        // Spawn threads that concurrently start and end tool calls.
        // With the old double-lock code, a concurrent AgentEnd between
        // agent_context() and insert() could drain the map, causing the
        // tool span to be inserted into a post-cleanup map.
        let handles: Vec<_> = (0..10)
            .map(|i| {
                let obs = Arc::clone(&obs);
                std::thread::spawn(move || {
                    let tool = format!("tool-{}", i);
                    let id = format!("id-{}", i);
                    obs.record_event(&ObserverEvent::ToolCallStart {
                        tool: tool.clone(),
                        call_id: Some(id.clone()),
                        thread_id: None,
                    });
                    obs.record_event(&ObserverEvent::ToolCallEnd {
                        tool,
                        call_id: Some(id),
                        duration: Duration::from_millis(1),
                        success: true,
                        error_message: None,
                    });
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }

        obs.record_event(&ObserverEvent::AgentEnd {
            duration: Duration::from_secs(1),
            tokens_used: None,
            total_cost_usd: None,
        });

        obs.flush();
        let spans = exporter.get_finished_spans().unwrap();

        // All tool spans should be children of the agent span.
        let agent_span = spans.iter().find(|s| s.name == "invoke_agent").unwrap();
        let agent_id = agent_span.span_context.span_id();

        for span in spans.iter().filter(|s| s.name.starts_with("tool:")) {
            assert_eq!(
                span.parent_span_id, agent_id,
                "Tool span {} should be child of agent span under concurrent access",
                span.name
            );
        }

        assert_eq!(spans.len(), 11, "agent + 10 tools = 11 spans");

        // active_spans must be empty after AgentEnd
        let remaining = obs.lock_spans().len();
        assert_eq!(remaining, 0, "active_spans should be empty after AgentEnd");
    }

    #[test]
    fn test_otel_double_init_does_not_panic() {
        // H9: Calling from_provider twice should not panic.
        let (obs1, _) = test_observer();
        let (obs2, _) = test_observer();
        obs1.record_event(&ObserverEvent::HeartbeatTick);
        obs2.record_event(&ObserverEvent::HeartbeatTick);
        // No panic = test passes
    }

    /// Regression test for I1: observer must recover from mutex poison.
    ///
    /// Before the fix, all `active_spans.lock()` uses `if let Ok(...)`,
    /// silently dropping telemetry data after a panic poisons the mutex.
    /// AgentStart can't insert the span context, so AgentEnd can't find it
    /// and never sets duration/tokens/cost attributes. The span still exports
    /// (via OTEL drop behavior) but is missing all end-event data.
    ///
    /// After the fix, poison is recovered via `unwrap_or_else(|e| e.into_inner())`
    /// and a warning is logged.
    #[test]
    fn continues_after_mutex_poison() {
        let (obs, exporter) = test_observer();

        // Poison the mutex by panicking while holding the lock
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = obs.active_spans.lock().unwrap();
            panic!("intentional panic to poison mutex");
        }));

        // Verify mutex is actually poisoned
        assert!(
            obs.active_spans.lock().is_err(),
            "Mutex should be poisoned after panic"
        );

        // Record an agent lifecycle with distinctive values
        obs.record_event(&ObserverEvent::AgentStart {
            provider: "test".into(),
            model: "m".into(),
        });
        obs.record_event(&ObserverEvent::AgentEnd {
            duration: Duration::from_secs(42),
            tokens_used: Some(999),
            total_cost_usd: None,
        });

        obs.flush();
        let spans = exporter.get_finished_spans().unwrap();
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].name, "invoke_agent");

        // The key assertion: AgentEnd attributes must be present.
        // Before fix: AgentStart couldn't insert the context into the poisoned
        // map, so AgentEnd couldn't find the span to set attributes on. The span
        // was ended by OTEL's drop impl but missing all end-event data.
        let attrs: HashMap<_, _> = spans[0]
            .attributes
            .iter()
            .map(|kv| (kv.key.as_str().to_string(), kv.value.clone()))
            .collect();
        assert!(
            attrs.contains_key("ironclaw.agent.duration_secs"),
            "AgentEnd attributes should be set after mutex poison recovery"
        );
    }

    /// Regression test for I2: AgentEnd must drain orphaned child spans.
    ///
    /// When the dispatcher's iteration-limit (or interrupt/cost-guardrail) path
    /// emits AgentEnd without a preceding LlmResponse or ToolCallEnd, any
    /// active child spans ("llm", "tool:*") remain in `active_spans` forever.
    /// The Context/Span objects prevent export until removed.
    ///
    /// After the fix, AgentEnd drains all remaining child spans, sets error
    /// status, and ends them — so no span is permanently leaked.
    #[test]
    fn agent_end_drains_orphaned_child_spans() {
        let (obs, exporter) = test_observer();

        // Simulate: AgentStart → LlmRequest → ToolCallStart → AgentEnd
        // (LlmResponse and ToolCallEnd are intentionally missing)
        obs.record_event(&ObserverEvent::AgentStart {
            provider: "test".into(),
            model: "m".into(),
        });
        obs.record_event(&ObserverEvent::LlmRequest {
            provider: "test".into(),
            model: "m".into(),
            message_count: 5,
            temperature: None,
            max_tokens: None,
            thread_id: None,
        });
        obs.record_event(&ObserverEvent::ToolCallStart {
            tool: "shell".into(),
            call_id: Some("abc".into()),
            thread_id: None,
        });

        // AgentEnd fires without LlmResponse or ToolCallEnd
        obs.record_event(&ObserverEvent::AgentEnd {
            duration: Duration::from_secs(10),
            tokens_used: None,
            total_cost_usd: None,
        });

        obs.flush();
        let spans = exporter.get_finished_spans().unwrap();

        // Before fix: only the agent span is exported (child spans are stuck
        // in active_spans, never ended properly). OTEL's drop impl may export
        // them with no attributes, but they won't have error status set.
        // After fix: all 3 spans are properly ended.
        assert_eq!(
            spans.len(),
            3,
            "AgentEnd should drain orphaned llm + tool spans: got {:?}",
            spans.iter().map(|s| &s.name).collect::<Vec<_>>()
        );

        // The orphaned child spans should have error status
        let llm_span = spans.iter().find(|s| s.name == "chat").unwrap();
        assert!(
            matches!(llm_span.status, Status::Error { .. }),
            "Orphaned llm span should have error status"
        );

        let tool_span = spans.iter().find(|s| s.name == "tool:shell").unwrap();
        assert!(
            matches!(tool_span.status, Status::Error { .. }),
            "Orphaned tool span should have error status"
        );

        // active_spans should be empty after AgentEnd
        let remaining = obs.lock_spans().len();
        assert_eq!(remaining, 0, "active_spans should be empty after AgentEnd");
    }

    /// Regression test for C4: per-instance shutdown via Observer trait.
    ///
    /// Before the fix, Observer had no `shutdown()` method. The module-level
    /// `shutdown_tracer_provider()` used a `OnceLock` static that could only
    /// hold the first-ever provider. If `OtelObserver` was constructed twice
    /// (config reload), the second provider's spans were silently lost on
    /// shutdown.
    ///
    /// After the fix, `Observer::shutdown()` calls `self.provider.shutdown()`
    /// per-instance. Each observer independently shuts down its own provider.
    #[test]
    fn shutdown_is_per_instance() {
        let (obs1, exp1) = test_observer();
        let (obs2, exp2) = test_observer();

        // Record events on each observer independently
        obs1.record_event(&ObserverEvent::HeartbeatTick);
        obs2.record_event(&ObserverEvent::AgentStart {
            provider: "test".into(),
            model: "m".into(),
        });
        obs2.record_event(&ObserverEvent::AgentEnd {
            duration: Duration::from_secs(1),
            tokens_used: None,
            total_cost_usd: None,
        });

        // Per-instance flush: each observer exports its own spans
        obs1.flush();
        obs2.flush();

        // Observer 1 should have exactly 1 span (HeartbeatTick)
        let spans1 = exp1.get_finished_spans().unwrap();
        assert_eq!(
            spans1.len(),
            1,
            "Observer 1 should have 1 span after per-instance flush"
        );
        assert_eq!(spans1[0].name, "heartbeat_tick");

        // Observer 2 should have exactly 1 span (AgentStart + AgentEnd = 1 span)
        let spans2 = exp2.get_finished_spans().unwrap();
        assert_eq!(
            spans2.len(),
            1,
            "Observer 2 should have 1 span after per-instance flush"
        );
        assert_eq!(spans2[0].name, "invoke_agent");

        // Per-instance shutdown — each observer shuts down its own provider.
        // Before C4 fix: only a global OnceLock static could be shutdown,
        // and only the first-ever provider was stored there. A second
        // observer's provider would never be properly shut down.
        obs1.shutdown();
        obs2.shutdown();
        // No crash, no global state confusion = test passes
    }
}
