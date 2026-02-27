# Plan: Systematically Fix OTEL Implementation Issues

## Context

The OTEL observability implementation (Phases 1-9) is complete and passing all tests. A 6-angle review identified 32 issues documented in `.claude/docs/corrections.md` (4 Critical, 10 High, 14 Medium, 4 Low). The user wants to fix these systematically using a test-first approach: enable quality gates and write tests that fail with current code, then fix the code to make them pass.

Reference: `.claude/docs/corrections.md` for full issue list.

---

## Phase 0: Quality Gate Infrastructure

Strengthen `scripts/quality-gate.sh` and `clippy.toml` to catch issues automatically.

### 0a: Grep-based mutex unwrap check in quality-gate.sh

Add a step that greps for `.lock().unwrap()` in non-test production code. This catches **C1** and prevents regressions.

```bash
# After clippy step in quality-gate.sh
step "mutex unwrap check"
MUTEX_UNWRAPS=$(grep -rn '\.lock()\.unwrap()' src/ --include='*.rs' | grep -v '#\[cfg(test)\]' | grep -v 'mod tests' | grep -v 'recording.rs' || true)
if [[ -z "$MUTEX_UNWRAPS" ]]; then
    pass "no .lock().unwrap() in production code"
else
    echo "$MUTEX_UNWRAPS"
    fail "found .lock().unwrap() in production code — use if let Ok() or .unwrap_or_else()"
fi
```

**Expected**: FAILS now (6 sites in otel.rs). Passes after Phase 1.

### 0b: Feature isolation compilation checks

Add `cargo check` with isolated features per CLAUDE.md. Catches **M8**.

```bash
step "feature isolation"
cargo check --no-default-features --features libsql 2>&1 && pass "libsql-only" || fail "libsql-only"
cargo check --no-default-features --features "libsql,otel" 2>&1 && pass "libsql+otel" || fail "libsql+otel"
```

**Expected**: Should pass now (already works), prevents regressions.

### 0c: Docker cleanup trap in quality-gate.sh

Fix **M4** — add `trap` to clean up Jaeger container if the script exits between `up` and `down`.

```bash
# Before docker compose up
trap 'docker compose -f "$COMPOSE_FILE" down 2>/dev/null' EXIT
```

### 0d: Replace `sleep 5` with readiness poll

Fix **M5** — poll Jaeger health endpoint instead of blind sleep.

```bash
for i in $(seq 1 30); do
    if curl -sf http://localhost:16686/ >/dev/null 2>&1; then break; fi
    sleep 1
done
```

### 0e: Docker healthcheck in compose file

Fix **M12** — add healthcheck to `docker-compose.otel-test.yml`:

```yaml
healthcheck:
  test: ["CMD", "curl", "-f", "http://localhost:16686/"]
  interval: 2s
  timeout: 5s
  retries: 15
```

**Files**: `scripts/quality-gate.sh`, `docker-compose.otel-test.yml`

---

## Phase 1: Fix Mutex Unwraps and OnceLock (C1, H9)

### C1: Replace `.lock().unwrap()` with safe patterns (6 sites)

**File**: `src/observability/otel.rs` — lines 76, 112, 127, 175, 186, 251

Replace all `self.active_spans.lock().unwrap()` with `if let Ok(mut spans) = self.active_spans.lock()`. For insert operations, wrap in the `if let`. For remove+use operations, same pattern.

Test: The quality gate grep check from Phase 0a will pass after this.

### H9: Handle `OnceLock::set()` result

**File**: `src/observability/otel.rs` — line 38

Change `let _ = PROVIDER.set(provider.clone());` to log a warning on double-init:

```rust
if PROVIDER.set(provider.clone()).is_err() {
    tracing::warn!("OTEL provider already initialized; second init ignored");
}
```

**Tier 2 test**: Add `test_otel_double_init_does_not_panic` — call `OtelObserver::new()` twice, verify no panic.

**Files**: `src/observability/otel.rs`

---

## Phase 2: Add `provider_name()` to LlmProvider (prerequisite for H1)

**File**: `src/llm/provider.rs` — Add to `LlmProvider` trait:

```rust
/// Provider name (e.g. "openai", "anthropic", "nearai").
/// Used for `gen_ai.provider.name` OTEL attribute.
fn provider_name(&self) -> &str;
```

Implement in each backend:
- `src/llm/nearai_chat.rs` → `"nearai"`
- `src/llm/rig_adapter.rs` → map from `LlmBackend` enum
- Any other `LlmProvider` impls

**File**: `src/agent/dispatcher.rs` — Change emission sites to use `self.llm.provider_name()` for the `provider` field and `self.llm.active_model_name()` for `model`.

This fixes **H1** (provider field set to model name).

**Files**: `src/llm/provider.rs`, all `LlmProvider` impls, `src/agent/dispatcher.rs`

---

## Phase 3: Fix OTEL Attribute Names and Types (H4, H5, H10, M1, M2, M3)

### Tier 2 test: OTEL attribute allowlist

**File**: `src/observability/otel.rs` (tests section)

Add a test that fires all event types, collects all attribute keys starting with `gen_ai.`, and validates each against the OTEL GenAI semconv spec allowlist. This catches **H4, H5, M3** and prevents future non-standard attributes.

```rust
const ALLOWED_GEN_AI_ATTRS: &[&str] = &[
    "gen_ai.operation.name",
    "gen_ai.provider.name",      // was gen_ai.system (H4)
    "gen_ai.request.model",
    "gen_ai.request.temperature",
    "gen_ai.request.max_tokens",
    "gen_ai.response.model",
    "gen_ai.response.finish_reasons",
    "gen_ai.usage.input_tokens",
    "gen_ai.usage.output_tokens",
    "gen_ai.conversation.id",    // was gen_ai.thread.id (H5)
    // ironclaw.* namespace for custom attrs
];
```

**Expected**: FAILS now (uses `gen_ai.system`, `gen_ai.thread.id`, and 12 custom `gen_ai.*` attrs).

### Fixes in `src/observability/otel.rs`:

- **H4**: Rename `gen_ai.system` → `gen_ai.provider.name` (lines 70, 89, 129)
- **H5**: Rename `gen_ai.thread.id` → `gen_ai.conversation.id` (lines 101, 164)
- **H10**: Change `finish_reason` handling — store as array value:
  ```rust
  // In traits.rs: finish_reason: Option<String> → finish_reasons: Option<Vec<String>>
  // In otel.rs: use opentelemetry::Value::Array for the attribute
  ```
- **M1**: Rename span names to follow `"{operation} {model}"` pattern:
  - `"chat"` → `"chat {model}"`
  - `"invoke_agent"` → `"invoke_agent {model}"`
- **M2**: Change `invoke_agent` SpanKind from `Internal` to `Client` (line 67)
- **M3**: Move custom attributes from `gen_ai.*` to `ironclaw.*` namespace:
  - `gen_ai.response.duration_ms` → `ironclaw.response.duration_ms`
  - `gen_ai.tool.duration_ms` → `ironclaw.tool.duration_ms`
  - `gen_ai.tool.success` → `ironclaw.tool.success`
  - `gen_ai.tool.name` → `ironclaw.tool.name`
  - `gen_ai.turn.iteration` → `ironclaw.turn.iteration`
  - `gen_ai.turn.tool_calls` → `ironclaw.turn.tool_calls`
  - `gen_ai.agent.duration_secs` → `ironclaw.agent.duration_secs`
  - `gen_ai.usage.total_tokens` → `ironclaw.usage.total_tokens`
  - `gen_ai.usage.total_cost_usd` → `ironclaw.usage.total_cost_usd`
  - `gen_ai.usage.cost_usd` → `ironclaw.usage.cost_usd`
  - `gen_ai.request.message_count` → `ironclaw.request.message_count`
  - `error.message` stays (it's standard OTEL, not `gen_ai.*`)

### Update existing tests

All existing Tier 2 tests that assert on attribute names must be updated to match the new names.

**Files**: `src/observability/otel.rs`, `src/observability/traits.rs`, `src/observability/log.rs`, `tests/otel_e2e.rs`

---

## Phase 4: Fix Span Architecture (H2, H3)

### H2: Parent-child span hierarchy

**File**: `src/observability/otel.rs`

Store `opentelemetry::Context` alongside spans in `active_spans`. When starting LLM/tool spans, derive them from the agent span's context so they appear as children in Jaeger.

Change `active_spans` type:
```rust
active_spans: Mutex<HashMap<String, (opentelemetry_sdk::trace::Span, opentelemetry::Context)>>,
```

When `AgentStart` creates the root span, capture its context. When `LlmRequest` / `ToolCallStart` create spans, use `cx.with_span(parent)` to set parent context.

**Tier 2 test**: `test_otel_span_hierarchy` — fire Agent→LLM→Tool→AgentEnd, verify LLM and tool `parent_span_id` match the agent span.

### H3: Concurrent tool calls use unique keys

**File**: `src/observability/otel.rs`

Add `call_id: Option<String>` field to `ToolCallStart` and `ToolCallEnd` in `traits.rs`. Key `active_spans` by `call_id` (or `tool_name` as fallback).

In `dispatcher.rs`, generate a unique ID per tool call (e.g., `format!("{}:{}", tool_name, uuid)`) and pass it through both events.

**Tier 2 test**: `test_otel_concurrent_tool_spans` — fire two `ToolCallStart{tool:"shell"}` with different `call_id`, end both, verify 2 distinct spans.

**Files**: `src/observability/traits.rs`, `src/observability/otel.rs`, `src/observability/log.rs`, `src/agent/dispatcher.rs`

---

## Phase 5: Fix Missing Event Emissions (C2, C3, C4, H7, H8)

### C2: Emit AgentStart/AgentEnd in agent loop

**File**: `src/agent/dispatcher.rs` — In `run_agentic_loop()`:
- Emit `AgentStart` at the top (before the loop)
- Emit `AgentEnd` at every return point (success and error paths)

Use a guard pattern or `defer!` to ensure `AgentEnd` is always emitted.

### C3: Emit observer events in JoinSet parallel path

**File**: `src/agent/dispatcher.rs` — lines 528-611

Pass `Arc<dyn Observer>` into the spawned JoinSet tasks. Emit `ToolCallStart`/`ToolCallEnd` inside each task, around the `execute_chat_tool_standalone` call.

### C4: Emit LlmResponse on error path

**File**: `src/agent/dispatcher.rs` — In the error handling after `respond_with_tools()` fails, emit:
```rust
observer.record_event(&ObserverEvent::LlmResponse {
    provider: ..., model: ..., duration: llm_start.elapsed(),
    success: false, error_message: Some(e.to_string()),
    input_tokens: None, output_tokens: None,
    finish_reason: None, cost_usd: None,
});
```

### H7: Emit TurnComplete for text-only responses

**File**: `src/agent/dispatcher.rs` — In the `RespondResult::Text` branch (around line 342), emit:
```rust
observer.record_event(&ObserverEvent::TurnComplete {
    thread_id: Some(thread_id_str.clone()),
    iteration: iteration as u32,
    tool_calls_in_turn: 0,
});
```

### H8: Emit ChannelMessage and HeartbeatTick

**Files**:
- `src/channels/manager.rs` — Emit `ChannelMessage` when a message is received/sent
- `src/agent/heartbeat.rs` — Emit `HeartbeatTick` at each tick

### Tier 1 tests (RecordingObserver)

Add tests using `RecordingObserver` that verify the complete event sequence for:
- Full tool turn: `AgentStart → LlmRequest → LlmResponse → ToolCallStart → ToolCallEnd → TurnComplete → AgentEnd`
- Text-only turn: `AgentStart → LlmRequest → LlmResponse → TurnComplete → AgentEnd`
- Error turn: `AgentStart → LlmRequest → LlmResponse{success:false} → AgentEnd`

**Variant coverage test**: Enumerate all `ObserverEvent` variant names, grep `src/` (excluding tests) for each, assert all have at least one emission site.

**Files**: `src/agent/dispatcher.rs`, `src/channels/manager.rs`, `src/agent/heartbeat.rs`

---

## Phase 6: Fix Config Protocol Handling (H6)

**File**: `src/observability/otel.rs` — `init_otel_pipeline()`

Branch on `config.otel_protocol` to select gRPC vs HTTP:

```rust
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
```

This requires `http-proto` feature in `opentelemetry-otlp` Cargo.toml dep.

**Test**: Unit test that creates an observer with `otel_protocol: Some("http")` and verifies no error.

**Files**: `Cargo.toml`, `src/observability/otel.rs`

---

## Phase 7: Enhance E2E Tests and Coverage (M6, M9, M10, M13, M14)

### M6: E2E test asserts on span names and attributes

**File**: `tests/otel_e2e.rs` — After querying Jaeger, assert:
- Span count (exact, not `>=`)
- Span names include `"invoke_agent"`, `"chat"`, `"tool:echo"`, `"turn_complete"`
- At least one span has `gen_ai.provider.name` attribute

### M9: Test factory paths `"otel"` and `"log+otel"`

**File**: `src/observability/mod.rs` tests — Add `#[cfg(feature = "otel")]` tests:
```rust
#[cfg(feature = "otel")]
#[test]
fn factory_returns_otel_for_otel() { ... }

#[cfg(feature = "otel")]
#[test]
fn factory_returns_multi_for_log_plus_otel() { ... }
```

### M10: Test unmatched span pairs

**File**: `src/observability/otel.rs` tests — Add tests for:
- `LlmResponse` without prior `LlmRequest` (should not panic)
- `ToolCallEnd` without prior `ToolCallStart` (should not panic)
- `AgentEnd` without prior `AgentStart` (should not panic)

### M13: Fix sleep timing

**File**: `tests/otel_e2e.rs` — Increase from 3s to 6s (or use retry loop) to exceed default OTEL batch delay of 5s.

### M14: Exact span count assertion

**File**: `src/observability/otel.rs` — Change `spans.len() >= 7` to `spans.len() == 7`.

**Files**: `tests/otel_e2e.rs`, `src/observability/otel.rs`, `src/observability/mod.rs`

---

## Execution Order

Phases are ordered so each builds on the previous:

1. **Phase 0** — Quality gates first (tests that FAIL with current code)
2. **Phase 1** — Fix C1/H9 (unblocks Phase 0a passing)
3. **Phase 2** — Add provider_name() (prerequisite for Phase 3 and Phase 5)
4. **Phase 3** — Fix attribute names/types (spec compliance)
5. **Phase 4** — Fix span architecture (parent-child, concurrency)
6. **Phase 5** — Fix missing emissions (biggest behavioral change)
7. **Phase 6** — Fix protocol config
8. **Phase 7** — Harden tests

## Issues NOT in Scope

These are low priority or require broader architectural discussion:
- **L1-L4**: Missing recommended attributes, non-standard error.message, turn_complete operation name, pub use re-exports
- **M7**: RecordingObserver wired into dispatcher tests (partially addressed by Phase 5 Tier 1 tests)
- **M8**: Feature isolation in quality gate (addressed in Phase 0b)
- **M11**: OTEL E2E in GitHub Actions CI (CI configuration, separate PR)

## Verification

After all phases:

```bash
# Quality gate (all checks pass)
./scripts/quality-gate.sh --quick

# Full test suite
cargo test --all-features

# Feature isolation
cargo check --no-default-features --features libsql
cargo check --no-default-features --features "libsql,otel"

# Clippy zero warnings
cargo clippy --all --all-features -- -D warnings

# Docker E2E
docker compose -f docker-compose.otel-test.yml up -d
# (wait for healthcheck)
cargo test --features otel --test otel_e2e -- --ignored
docker compose -f docker-compose.otel-test.yml down
```
