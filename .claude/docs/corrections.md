# OTEL Implementation Corrections

Consolidated findings from 6-angle review of the OpenTelemetry observability implementation.

## CRITICAL

### C1: `.unwrap()` on `Mutex::lock()` in production code (6 sites)
- **Files:** `otel.rs:76,112,127,175,186,251`
- **Problem:** Violates CLAUDE.md. A single panic poisons the mutex and cascades to crash all observer calls across all threads.
- **Fix:** Replace with `if let Ok(mut spans) = self.active_spans.lock() { ... }` or `.unwrap_or_else(|e| e.into_inner())`.
- **Clippy-checkable:** YES -- custom lint or `disallowed-methods` for `Mutex::lock().unwrap()`

### C2: `AgentStart` and `AgentEnd` are never emitted
- **Files:** `dispatcher.rs`, `agent_loop.rs`
- **Problem:** The root span wrapping the entire agent invocation is never created or closed. All other spans are parentless orphans in Jaeger.
- **Fix:** Emit `AgentStart` at the top of `run_agentic_loop`, `AgentEnd` at every return point (including error paths).
- **Clippy-checkable:** NO -- semantic/business logic gap, not a syntactic issue

### C3: Parallel tool execution (JoinSet path) emits zero observer events
- **Files:** `dispatcher.rs:528-611`
- **Problem:** When the LLM returns 2+ tool calls, none get `ToolCallStart`/`ToolCallEnd`. Only the single-tool inline path is instrumented.
- **Fix:** Pass `Arc<dyn Observer>` into the spawned async block, emit tool start/end inside each task.
- **Clippy-checkable:** NO -- asymmetric code paths, requires semantic analysis

### C4: LLM error path emits nothing
- **Files:** `dispatcher.rs:272-308`
- **Problem:** When `respond_with_tools()` fails, no `LlmResponse{success:false}` is emitted. The open "llm" span in `active_spans` leaks forever.
- **Fix:** Emit `LlmResponse { success: false, error_message: Some(e.to_string()), ... }` before each `return Err(...)`.
- **Clippy-checkable:** NO -- requires understanding of start/end pairing semantics

## HIGH

### H1: `provider` field set to model name
- **Files:** `dispatcher.rs:262-268,329-332`
- **Problem:** Both `LlmRequest` and `LlmResponse` pass `active_model_name()` for both `provider` and `model`. Makes `gen_ai.system`/`gen_ai.provider.name` useless.
- **Fix:** Plumb a separate provider name from config or LlmProvider trait.
- **Clippy-checkable:** NO -- same-type args passed, no syntactic signal

### H2: All spans are flat -- no parent-child hierarchy
- **Files:** `otel.rs:63-282`
- **Problem:** LLM and tool spans are root spans, not children of the agent span. Trace waterfall views in Jaeger/Tempo are meaningless.
- **Fix:** Store `Context` alongside spans in `active_spans`, propagate parent context when starting child spans.
- **Clippy-checkable:** NO -- architectural decision, not a lint target

### H3: Concurrent same-tool calls corrupt each other
- **Files:** `otel.rs:158-199`
- **Problem:** `active_spans` keys by tool name only. Two parallel `shell` calls overwrite each other; the first span is leaked.
- **Fix:** Key by `(tool_name, thread_id)` or use a unique span ID.
- **Clippy-checkable:** NO -- requires understanding of concurrency semantics

### H4: `gen_ai.system` is deprecated (semconv v1.37+)
- **Files:** `otel.rs:70,89,129`
- **Problem:** Renamed to `gen_ai.provider.name`.
- **Fix:** Rename all three occurrences.
- **Clippy-checkable:** PARTIALLY -- a custom lint checking string literals against a known deprecation list could catch this, but no built-in clippy lint exists

### H5: `gen_ai.thread.id` doesn't exist in the spec
- **Files:** `otel.rs:101,164`
- **Problem:** The correct attribute is `gen_ai.conversation.id`.
- **Fix:** Rename.
- **Clippy-checkable:** PARTIALLY -- same as H4, requires domain-specific string validation

### H6: `otel_protocol` config field is silently ignored
- **Files:** `otel.rs:302-325`
- **Problem:** Always uses gRPC regardless of `OTEL_EXPORTER_OTLP_PROTOCOL=http` setting.
- **Fix:** Branch on `config.otel_protocol` to select `.with_tonic()` vs `.with_http()`.
- **Clippy-checkable:** YES -- `clippy::unused_self` won't catch it but a custom "unused struct field" lint could. The field is used nowhere in the function that receives the config.

### H7: `TurnComplete` not emitted for text-only responses
- **Files:** `dispatcher.rs:342-344`
- **Problem:** Simple Q&A (no tool calls) produces no turn-complete event.
- **Fix:** Emit `TurnComplete { tool_calls_in_turn: 0, ... }` on the `RespondResult::Text` branch.
- **Clippy-checkable:** NO -- business logic gap

### H8: `ChannelMessage` and `HeartbeatTick` never emitted
- **Files:** entire `src/agent/`
- **Problem:** Event variants exist in the trait but have zero call sites.
- **Fix:** Emit in channel manager and heartbeat.rs respectively.
- **Clippy-checkable:** PARTIALLY -- a test (not clippy) that greps for each enum variant in non-test code would catch this. Similar to an exhaustiveness check.

### H9: `OnceLock::set()` result silently discarded
- **Files:** `otel.rs:19,38`
- **Problem:** Double `new()` loses shutdown coverage for the second provider.
- **Fix:** Return an error or log a warning on double-init.
- **Clippy-checkable:** YES -- `clippy::let_underscore_must_use` or `#[must_use]` on `OnceLock::set()` (already returns `Result` which `let _ =` discards)

### H10: `gen_ai.response.finish_reasons` is a string, spec requires `string[]`
- **Files:** `otel.rs:141-145`, `traits.rs`
- **Problem:** Type mismatch with OTEL spec; breaks strict schema enforcement.
- **Fix:** Change `finish_reason: Option<String>` to `finish_reasons: Option<Vec<String>>`, use OTEL array value.
- **Clippy-checkable:** NO -- domain-specific type requirement

## MEDIUM

### M1: Span names don't follow spec pattern `"{operation} {model}"`
- **Files:** `otel.rs`
- **Clippy-checkable:** NO

### M2: `invoke_agent` SpanKind should be Client, not Internal
- **Files:** `otel.rs:67`
- **Clippy-checkable:** NO

### M3: 12 custom attributes use `gen_ai.*` namespace instead of `ironclaw.*`
- **Files:** `otel.rs` (various)
- **Clippy-checkable:** PARTIALLY -- a custom lint matching `gen_ai.` prefix against an allowlist could catch non-standard attrs

### M4: No cleanup trap in quality-gate.sh
- **Files:** `quality-gate.sh:63-70`
- **Problem:** Docker container leaks if the script exits unexpectedly between `up` and `down`.
- **Fix:** Add `trap 'docker compose -f "$COMPOSE_FILE" down 2>&1' EXIT` before bringing up the container.
- **Clippy-checkable:** N/A -- shell script, not Rust

### M5: `sleep 5` instead of readiness check in quality gate
- **Files:** `quality-gate.sh:64`
- **Fix:** Poll Jaeger health endpoint in a loop, or add a `healthcheck` to docker-compose + use `--wait`.
- **Clippy-checkable:** N/A -- shell script

### M6: E2E test only checks trace count, not span names or attributes
- **Files:** `otel_e2e.rs:99`
- **Fix:** Assert on span count, span names, and at least one key attribute per span type.
- **Clippy-checkable:** NO

### M7: RecordingObserver never wired into dispatcher tests
- **Files:** `recording.rs`, `dispatcher.rs` tests
- **Fix:** Add `make_test_agent_with_observer()` and integration tests that drive the loop with a RecordingObserver.
- **Clippy-checkable:** NO

### M8: Quality gate missing `cargo check --no-default-features --features libsql`
- **Files:** `quality-gate.sh`
- **Fix:** Add feature-isolation compilation checks per CLAUDE.md.
- **Clippy-checkable:** N/A -- CI/script issue

### M9: `create_observer("otel")` and `"log+otel"` factory paths untested
- **Files:** `mod.rs`
- **Clippy-checkable:** NO -- missing test, not a lint target

### M10: Unmatched span pairs untested (Response without Request, End without Start)
- **Files:** `otel.rs`
- **Clippy-checkable:** NO

### M11: OTEL E2E never runs in GitHub Actions CI
- **Files:** `.github/workflows/`
- **Clippy-checkable:** N/A

### M12: No docker-compose healthcheck
- **Files:** `docker-compose.otel-test.yml`
- **Clippy-checkable:** N/A

### M13: 3s sleep in E2E test may be shorter than SDK batch delay (5s default)
- **Files:** `otel_e2e.rs:88`
- **Clippy-checkable:** NO

### M14: All-events span count test uses `>= 7` floor, not exact `== 7`
- **Files:** `otel.rs:541`
- **Clippy-checkable:** NO

## LOW

### L1: Missing recommended attributes (response.id, top_p, agent.name, error.type)
- **Clippy-checkable:** NO

### L2: `error.message` span attribute is not standard OTEL
- **Clippy-checkable:** NO

### L3: `turn_complete` is not a spec-defined `gen_ai.operation.name` value
- **Clippy-checkable:** NO

### L4: `pub use` re-exports in `observability/mod.rs`
- **Clippy-checkable:** YES -- `clippy::wildcard_reexports` or a custom `pub use` audit lint could flag this

---

## Clippy/Quality Gate Analysis

### Issues that clippy CAN catch (with config):

| Issue | Clippy mechanism |
|-------|-----------------|
| **C1:** `.unwrap()` on Mutex in prod | `clippy::disallowed_methods` banning `Mutex::lock().unwrap()` patterns, or grep-based pre-commit check |
| **H9:** `let _ =` discarding `Result` from `OnceLock::set()` | `clippy::let_underscore_must_use` (already a lint) |
| **L4:** `pub use` re-exports | `clippy::wildcard_reexports` or manual audit |

### Issues that a custom test/script could catch:

| Issue | Mechanism |
|-------|-----------|
| **H8:** Enum variants with zero non-test call sites | A build script or test that parses `ObserverEvent` variants and greps `src/` (excluding tests) for each variant name |
| **H4/H5/M3:** Non-standard OTEL attribute names | A test that collects all `KeyValue::new("gen_ai.*", ...)` string literals and validates against an allowlist of spec attributes |
| **H6:** Unused config field | `#[warn(dead_code)]` won't catch it since the field is read, but a test asserting config fields influence behavior would |
| **M8:** Feature isolation | Already a quality-gate step, just needs adding |

### Issues that are fundamentally NOT lint-checkable:

| Category | Issues | Why |
|----------|--------|-----|
| Missing emission sites | C2, C3, C4, H1, H7, H8 | Business logic: "this code path should call this function" -- requires semantic understanding of the observer contract |
| Span hierarchy | H2 | Architectural design decision |
| Concurrency correctness | H3 | Requires understanding of parallel execution patterns |
| OTEL spec compliance | H4, H5, H10, M1, M2, M3, L1-L3 | Domain-specific knowledge about OTEL semantic conventions |
| Test completeness | M6, M7, M9, M10, M14 | Missing tests are invisible to lints |
| CI/shell script issues | M4, M5, M8, M11, M12, M13 | Not Rust code |

### Recommendation: what to add to quality gates

1. **`clippy.toml` / `.clippy.toml`** -- Add `disallowed-methods` for `std::sync::Mutex::lock` paired with `.unwrap()` (catches C1 and prevents future occurrences)
2. **A "variant coverage" test** -- Compile-time or runtime test that every `ObserverEvent` variant has at least one non-test `record_event()` call site (catches C2, H8)
3. **An OTEL attribute allowlist test** -- Test in `otel.rs` that collects all `gen_ai.*` attribute keys and validates against the spec's attribute registry (catches H4, H5, M3)
4. **`cargo check` with each feature flag in isolation** -- Already in CLAUDE.md, add to `quality-gate.sh` (catches M8)
