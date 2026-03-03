# Plan: OpenTelemetry Observability for IronClaw

## Context

PR #334 added an ad-hoc tool audit trail (tool summaries persisted to DB). Reviewer (ilblackdragon) requested a full audit trail covering LLM calls, tools, and agent turns. Rather than expanding the ad-hoc approach, we replace it with proper OpenTelemetry observability using the `gen_ai.*` semantic conventions.

IronClaw already has a dormant `Observer` trait infrastructure in `src/observability/` with event types for LLM, tools, and agent lifecycle — defined but never wired in. We activate it, enrich it with `gen_ai.*` attributes, add an OTEL backend, and wire it into the agent.

## Approach: Two-Layer

1. **`OtelObserver`** — New `Observer` impl creating OTEL spans with `gen_ai.*` attributes at LLM/tool/turn boundaries
2. **`tracing-opentelemetry` layer** — Bridges all existing `tracing::info!`/`warn!` calls into the OTEL pipeline (ambient telemetry for free)

Both feature-gated behind `otel` cargo feature. Zero cost when disabled.

---

## Phase 1: Dependencies and Feature Flag

**`Cargo.toml`** — Add `otel` feature + 4 optional deps:

```toml
[features]
otel = ["dep:opentelemetry", "dep:opentelemetry_sdk", "dep:opentelemetry-otlp", "dep:tracing-opentelemetry"]

[dependencies]
opentelemetry = { version = "0.31", optional = true }
opentelemetry_sdk = { version = "0.31", features = ["rt-tokio"], optional = true }
opentelemetry-otlp = { version = "0.31", features = ["tonic"], optional = true }
tracing-opentelemetry = { version = "0.29", optional = true }
```

## Phase 2: Enrich `ObserverEvent` Variants

**`src/observability/traits.rs`** — Add optional `gen_ai.*` fields to existing variants:

- `LlmRequest` += `temperature: Option<f32>`, `max_tokens: Option<u32>`, `thread_id: Option<String>`
- `LlmResponse` += `input_tokens: Option<u32>`, `output_tokens: Option<u32>`, `finish_reason: Option<String>`, `cost_usd: Option<f64>`
- `ToolCallStart` += `thread_id: Option<String>`
- `ToolCallEnd` += `error_message: Option<String>`
- `TurnComplete` → struct variant with `thread_id`, `iteration`, `tool_calls_in_turn`
- `AgentEnd` += `total_cost_usd: Option<f64>`

**`src/observability/log.rs`** — Update `LogObserver` to log new fields when present.

## Phase 3: Create `OtelObserver`

**New file: `src/observability/otel.rs`** (`#[cfg(feature = "otel")]`)

- Implements `Observer` trait
- Uses `Mutex<HashMap<String, BoxedSpan>>` for start/end span pairing (`ToolCallStart`→`ToolCallEnd`, etc.)
- Maps events to `gen_ai.*` OTEL attributes:

| OTEL Attribute | Source |
|----------------|--------|
| `gen_ai.operation.name` | `"chat"` / `"execute_tool"` / `"invoke_agent"` |
| `gen_ai.system` | provider name from event |
| `gen_ai.request.model` | model name from event |
| `gen_ai.request.temperature` | from `LlmRequest` |
| `gen_ai.request.max_tokens` | from `LlmRequest` |
| `gen_ai.response.finish_reasons` | from `LlmResponse` |
| `gen_ai.usage.input_tokens` | from `LlmResponse` |
| `gen_ai.usage.output_tokens` | from `LlmResponse` |

- Includes `init_otel_pipeline()`: OTLP batch exporter, resource with `service.name` + `service.version`

## Phase 4: Expand Config and Factory

**`src/observability/mod.rs`**:
- Add to `ObservabilityConfig`: `otel_endpoint`, `otel_protocol`, `otel_service_name`
- Add `"otel"` and `"log+otel"` to `create_observer()` factory (latter uses existing `MultiObserver`)

**`src/config/mod.rs`** (line ~201):
- Read standard env vars: `OTEL_EXPORTER_OTLP_ENDPOINT` (default `http://localhost:4317`), `OTEL_EXPORTER_OTLP_PROTOCOL` (default `grpc`), `OTEL_SERVICE_NAME` (default `ironclaw`)

## Phase 5: Add `tracing-opentelemetry` Layer

**`src/channels/web/log_layer.rs`** — In `init_tracing()`, conditionally add `tracing_opentelemetry::layer()` to the subscriber stack when `otel` feature is active. Uses `Option` layer wrapping (zero-cost when `None`).

**`src/main.rs`** — Pass tracer provider reference to `init_tracing()`.

## Phase 6: Wire Observer into Agent

**Add `observer: Arc<dyn Observer>` to:**
- `AgentDeps` (`src/agent/agent_loop.rs:60`)
- `AppComponents` (`src/app.rs:31`)
- Test harness defaults (`src/testing.rs`) — uses `NoopObserver`

**`src/main.rs`** — Create observer from config, store in `AppComponents`, flows into `AgentDeps`.

**`src/agent/dispatcher.rs`** — Emit events at boundaries:
- Before/after `reasoning.respond_with_tools()` (~line 215) → `LlmRequest`/`LlmResponse`
- Before/after tool execution (~line 768) → `ToolCallStart`/`ToolCallEnd`
- End of loop iteration → `TurnComplete`
- Add `observer: &dyn Observer` parameter to `execute_chat_tool_standalone()` (it's `pub(super)`, only called within the agent module)

## Phase 7: Remove PR #334 Ad-Hoc Audit Trail

**`src/agent/dispatcher.rs`** — Remove:
- `tool_summaries: Vec<String>` and all `.push()` calls
- DB persistence block (`add_conversation_message(thread_id, "system", &summary)`)
- `sanitize_audit_field()`, `extract_params_preview()`, `AUDIT_FIELD_MAX_LEN`
- Related tests (`test_tool_summary_*`, `test_sanitize_audit_field_*`)

**Keep** (not observability):
- Source attribution prompt section in `src/llm/reasoning.rs`
- Leak detector threshold in `src/safety/leak_detector.rs`
- Diagnostic `tracing::warn!` in `src/llm/rig_adapter.rs` (picked up by tracing-otel bridge)

## Phase 8: Shutdown

**`src/main.rs`** — `observer.flush()` + `shutdown_tracer_provider()` during graceful shutdown.

## Phase 9: Testing — Three-Tier Strategy

Every event type, attribute, and span relationship must be tested. Three tiers ensure correctness at different levels.

### Tier 1: `RecordingObserver` — Event Wiring Tests

**New file: `src/observability/recording.rs`** (`#[cfg(test)]`)

A test-only `Observer` that captures all events into `Arc<Mutex<Vec<ObserverEvent>>>`. This tests that the dispatcher emits the right events at the right boundaries — **independent of OTEL**.

```rust
pub struct RecordingObserver {
    events: Arc<Mutex<Vec<ObserverEvent>>>,
}
impl RecordingObserver {
    pub fn new() -> (Self, Arc<Mutex<Vec<ObserverEvent>>>) { ... }
}
impl Observer for RecordingObserver {
    fn record_event(&self, event: &ObserverEvent) {
        self.events.lock().unwrap().push(event.clone());
    }
    ...
}
```

**Tests in `src/agent/dispatcher.rs`** (integration-style, using `TestHarnessBuilder`):

| Test | What it verifies |
|------|-----------------|
| `test_observer_llm_request_response` | `LlmRequest` emitted before LLM call, `LlmResponse` after, with correct provider/model/token counts |
| `test_observer_tool_call_lifecycle` | `ToolCallStart` before tool exec, `ToolCallEnd` after, with tool name, duration, success/failure |
| `test_observer_tool_call_error` | `ToolCallEnd` with `success: false` and `error_message` when tool fails |
| `test_observer_turn_complete` | `TurnComplete` at end of each agentic loop iteration with iteration count |
| `test_observer_agent_lifecycle` | `AgentStart` at beginning, `AgentEnd` at end with total duration and cost |
| `test_observer_full_turn_sequence` | Simulate full agent turn (LLM → 2 tools → LLM → done). Assert exact event sequence: `AgentStart, LlmRequest, LlmResponse, ToolCallStart, ToolCallEnd, ToolCallStart, ToolCallEnd, LlmRequest, LlmResponse, TurnComplete, TurnComplete, AgentEnd` |
| `test_observer_error_event` | Verify `Error` event emitted when LLM call fails |

**Completeness guard** — A test that constructs every `ObserverEvent` variant using `strum` or manual enum count, and verifies `RecordingObserver` captures all of them. If a new variant is added without a corresponding emission point, this test will flag it via a mismatch between "variants defined" and "variants observed in integration tests".

### Tier 2: `InMemorySpanExporter` — OTEL Attribute Tests

**Tests in `src/observability/otel.rs`** (`#[cfg(all(test, feature = "otel"))]`)

Uses `opentelemetry_sdk`'s `InMemorySpanExporter` + `SimpleSpanProcessor` (synchronous — no flush/deadlock issues). Tests that `OtelObserver` translates events into correct OTEL spans with `gen_ai.*` attributes.

**Setup helper:**
```rust
fn setup_test_otel() -> (OtelObserver, InMemorySpanExporter) {
    let exporter = InMemorySpanExporterBuilder::new().build();
    let provider = SdkTracerProvider::builder()
        .with_span_processor(SimpleSpanProcessor::new(exporter.clone()))
        .build();
    let observer = OtelObserver::new(provider);
    (observer, exporter)
}
```

| Test | Span assertions |
|------|----------------|
| `test_otel_llm_request_span` | Span name = `"chat"`, attrs: `gen_ai.operation.name = "chat"`, `gen_ai.system`, `gen_ai.request.model`, `gen_ai.request.temperature`, `gen_ai.request.max_tokens` |
| `test_otel_llm_response_attrs` | After `LlmResponse`: `gen_ai.usage.input_tokens`, `gen_ai.usage.output_tokens`, `gen_ai.response.finish_reasons`, span status ok/error |
| `test_otel_tool_span` | Span name = tool name, attrs: `gen_ai.operation.name = "execute_tool"`, `gen_ai.tool.name`, duration recorded |
| `test_otel_tool_error_span` | Failed tool: span status = Error, `error_message` attribute set |
| `test_otel_agent_span` | Span name = `"agent_turn"`, attrs: `gen_ai.operation.name = "invoke_agent"`, `thread_id` |
| `test_otel_span_hierarchy` | LLM and tool spans are children of the agent turn span (`parent_span_id` matches agent span's `span_id`) |
| `test_otel_cost_attribute` | `AgentEnd` with `total_cost_usd` → span attribute `gen_ai.usage.cost` |
| `test_otel_all_event_types_produce_spans` | Fire every `ObserverEvent` variant → assert no span is silently dropped (span count >= expected) |

**Attribute helper** (reused across tests):
```rust
fn find_attr<'a>(span: &'a SpanData, key: &str) -> Option<&'a Value> {
    span.attributes.iter().find(|kv| kv.key.as_str() == key).map(|kv| &kv.value)
}
```

**Dev-dependency addition to `Cargo.toml`:**
```toml
[dev-dependencies]
opentelemetry_sdk = { version = "0.31", features = ["testing"] }
```
This ensures `InMemorySpanExporter` is available in test builds even without the `otel` runtime feature.

### Tier 3: Docker E2E — Full Pipeline Test

**`docker-compose.otel-test.yml`** — New compose file extending the existing `docker-compose.yml`:

```yaml
services:
  jaeger:
    image: jaegertracing/jaeger:2
    ports:
      - "4317:4317"    # OTLP gRPC
      - "16686:16686"  # Jaeger UI
      - "16685:16685"  # Jaeger gRPC query API
    environment:
      COLLECTOR_OTLP_ENABLED: "true"
```

**`tests/otel_e2e.rs`** — Integration test (`#[cfg(feature = "otel")]`, `#[ignore]` by default for CI opt-in):

1. Assumes Jaeger is running (started by `docker compose -f docker-compose.otel-test.yml up -d`)
2. Creates `OtelObserver` pointing at `localhost:4317`
3. Fires a complete agent turn sequence (AgentStart → LlmRequest → LlmResponse → ToolCallStart → ToolCallEnd → TurnComplete → AgentEnd)
4. Calls `observer.flush()` to drain the OTLP batch exporter
5. Queries Jaeger's gRPC query API (`jaeger.api_v3.QueryService/FindTraces`) or HTTP API (`http://localhost:16686/api/traces?service=ironclaw`) to verify:
   - Trace exists with service name `ironclaw`
   - Expected span count (7 spans for the sequence above)
   - `gen_ai.*` attributes present on LLM spans
   - Span hierarchy: tool/LLM spans are children of agent span

**CI integration** — Add to `.github/workflows/` (or document):
```bash
# Start Jaeger
docker compose -f docker-compose.otel-test.yml up -d
# Wait for health
sleep 5
# Run E2E tests
cargo test --features otel --test otel_e2e -- --ignored
# Cleanup
docker compose -f docker-compose.otel-test.yml down
```

### Testing Matrix Summary

| Tier | Scope | External deps | Feature gate | Run by default |
|------|-------|--------------|-------------|---------------|
| Tier 1: RecordingObserver | Event emission correctness | None | None (always available) | Yes (`cargo test`) |
| Tier 2: InMemorySpanExporter | OTEL span/attribute correctness | None (in-process) | `otel` | Yes (`cargo test --features otel`) |
| Tier 3: Docker E2E | Full OTLP pipeline to Jaeger | Jaeger container | `otel` + `#[ignore]` | No (CI opt-in) |

---

## Files Summary

| File | Action |
|------|--------|
| `Cargo.toml` | Modify — `otel` feature + 4 deps + `opentelemetry_sdk` dev-dep with `testing` |
| `src/observability/traits.rs` | Modify — enrich event variants |
| `src/observability/otel.rs` | **Create** — `OtelObserver` + pipeline init + Tier 2 tests |
| `src/observability/recording.rs` | **Create** — `RecordingObserver` test helper |
| `src/observability/mod.rs` | Modify — config expansion, factory update, expose recording |
| `src/observability/log.rs` | Modify — log new optional fields |
| `src/config/mod.rs` | Modify — read OTEL env vars |
| `src/channels/web/log_layer.rs` | Modify — add tracing-otel layer |
| `src/agent/agent_loop.rs` | Modify — `observer` in `AgentDeps` |
| `src/app.rs` | Modify — `observer` in `AppComponents` |
| `src/main.rs` | Modify — create observer, thread through, shutdown |
| `src/agent/dispatcher.rs` | Modify — emit events, remove ad-hoc audit trail, Tier 1 tests |
| `src/testing.rs` | Modify — `NoopObserver` default, `with_observer()` builder method |
| `docker-compose.otel-test.yml` | **Create** — Jaeger service for E2E tests |
| `tests/otel_e2e.rs` | **Create** — Tier 3 Docker E2E test |

## Verification

```bash
# Default features (no otel) — must compile and pass
cargo check
cargo test

# Otel feature — Tier 1 + Tier 2 tests
cargo check --features otel
cargo test --features otel

# Feature isolation
cargo check --no-default-features --features libsql
cargo check --no-default-features --features "libsql,otel"

# CI gate
cargo fmt --check
cargo clippy --all --benches --tests --examples --all-features -- -D warnings

# Tier 3: Docker E2E (opt-in, requires Docker)
docker compose -f docker-compose.otel-test.yml up -d
sleep 5
cargo test --features otel --test otel_e2e -- --ignored
docker compose -f docker-compose.otel-test.yml down
```

### What each tier proves

- **Tier 1** (`cargo test`): Every agent boundary (LLM, tool, turn, lifecycle) emits the correct `ObserverEvent` with the right fields. No OTEL dependency needed.
- **Tier 2** (`cargo test --features otel`): `OtelObserver` translates every event into correct OTEL spans with `gen_ai.*` attributes, proper parent-child hierarchy, and error status propagation. Uses in-process `InMemorySpanExporter` — no external services.
- **Tier 3** (`--ignored` + Docker): Full OTLP gRPC pipeline works end-to-end. Spans arrive in Jaeger with correct service name, hierarchy, and attributes. Catches serialization bugs, network issues, and batch exporter edge cases that in-memory testing cannot.
