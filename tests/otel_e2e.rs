//! Tier 3: Docker E2E test for the OTEL pipeline.
//!
//! Requires a running Jaeger instance (see `docker-compose.otel-test.yml`).
//! Ignored by default — run with:
//!
//! ```bash
//! docker compose -f docker-compose.otel-test.yml up -d
//! sleep 5
//! cargo test --features otel --test otel_e2e -- --ignored
//! docker compose -f docker-compose.otel-test.yml down
//! ```

#![cfg(feature = "otel")]

use std::time::Duration;

use ironclaw::observability::ObservabilityConfig;
use ironclaw::observability::otel::OtelObserver;
use ironclaw::observability::traits::{Observer, ObserverEvent};

/// Fire a complete agent lifecycle and verify spans arrive in Jaeger.
#[tokio::test]
#[ignore]
async fn test_otel_e2e_spans_arrive_in_jaeger() {
    let config = ObservabilityConfig {
        backend: "otel".into(),
        otel_endpoint: Some("http://localhost:4317".into()),
        otel_protocol: Some("grpc".into()),
        otel_service_name: Some("ironclaw-e2e-test".into()),
    };

    let observer = OtelObserver::new(&config).expect("should init OTEL observer");

    // Fire a complete agent turn sequence
    observer.record_event(&ObserverEvent::AgentStart {
        provider: "test".into(),
        model: "e2e-model".into(),
    });

    observer.record_event(&ObserverEvent::LlmRequest {
        provider: "test".into(),
        model: "e2e-model".into(),
        message_count: 2,
        temperature: Some(0.5),
        max_tokens: Some(1024),
        thread_id: Some("e2e-thread".into()),
    });

    observer.record_event(&ObserverEvent::LlmResponse {
        provider: "test".into(),
        model: "e2e-model".into(),
        duration: Duration::from_millis(250),
        success: true,
        error_message: None,
        input_tokens: Some(100),
        output_tokens: Some(50),
        finish_reasons: Some(vec!["stop".into()]),
        cost_usd: Some(0.001),
        cached: false,
    });

    observer.record_event(&ObserverEvent::ToolCallStart {
        tool: "echo".into(),
        call_id: None,
        thread_id: Some("e2e-thread".into()),
    });

    observer.record_event(&ObserverEvent::ToolCallEnd {
        tool: "echo".into(),
        call_id: None,
        duration: Duration::from_millis(5),
        success: true,
        error_message: None,
    });

    observer.record_event(&ObserverEvent::TurnComplete {
        thread_id: Some("e2e-thread".into()),
        iteration: 1,
        tool_calls_in_turn: 1,
    });

    observer.record_event(&ObserverEvent::AgentEnd {
        duration: Duration::from_secs(1),
        tokens_used: Some(150),
        total_cost_usd: Some(0.001),
    });

    observer.flush();

    // Wait for the batch exporter to deliver
    // Wait for the batch exporter to deliver (default batch interval is 5s)
    tokio::time::sleep(Duration::from_secs(6)).await;

    // Query Jaeger for the trace
    let resp = reqwest::get("http://localhost:16686/api/traces?service=ironclaw-e2e-test&limit=1")
        .await
        .expect("Jaeger query should succeed (is Jaeger running?)");

    assert!(resp.status().is_success(), "Jaeger returned non-200");

    let body: serde_json::Value = resp.json().await.expect("Jaeger response should be JSON");
    let traces = body["data"].as_array().expect("data should be array");
    assert!(
        !traces.is_empty(),
        "Expected at least 1 trace in Jaeger for service ironclaw-e2e-test"
    );

    // M6: Assert on span names and attributes
    let trace = &traces[0];
    let spans = trace["spans"].as_array().expect("spans should be array");

    // We fired 7 events producing 7 spans:
    // AgentStart+AgentEnd → invoke_agent, LlmRequest+LlmResponse → chat,
    // ToolCallStart+ToolCallEnd → tool:echo, TurnComplete, ChannelMessage, HeartbeatTick, Error
    // (but E2E didn't fire ChannelMessage/HeartbeatTick/Error — only 4 spans)
    assert_eq!(
        spans.len(),
        4,
        "Expected 4 spans (agent + llm + tool + turn_complete), got {}",
        spans.len()
    );

    // Collect span names
    let span_names: Vec<&str> = spans
        .iter()
        .filter_map(|s| s["operationName"].as_str())
        .collect();
    assert!(
        span_names.contains(&"invoke_agent"),
        "Missing invoke_agent span; got: {:?}",
        span_names
    );
    assert!(
        span_names.contains(&"chat"),
        "Missing chat span; got: {:?}",
        span_names
    );
    assert!(
        span_names.contains(&"tool:echo"),
        "Missing tool:echo span; got: {:?}",
        span_names
    );
    assert!(
        span_names.contains(&"turn_complete"),
        "Missing turn_complete span; got: {:?}",
        span_names
    );

    // Check that the chat span has gen_ai.provider.name attribute
    let chat_span = spans
        .iter()
        .find(|s| s["operationName"].as_str() == Some("chat"))
        .expect("chat span must exist");
    let tags = chat_span["tags"].as_array().expect("tags should be array");
    let has_provider = tags.iter().any(|t| {
        t["key"].as_str() == Some("gen_ai.provider.name") && t["value"].as_str() == Some("test")
    });
    assert!(
        has_provider,
        "chat span should have gen_ai.provider.name=test attribute"
    );
}
