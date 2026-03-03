# Audit V2 — `feat/tool-transparency` PR Code Review

Date: 2026-02-24
Branch: `feat/tool-transparency`
Reviewers: 4 independent subagents (observability, LLM, agent/dispatcher, cross-cutting architecture)

---

## Bug Fix Protocol

For each issue below, follow this sequence strictly:

1. **Write regression test** — A failing test that demonstrates the bug before any fix
2. **Research** — Read the relevant code paths, understand root cause, identify all instances of the pattern
3. **Fix** — Apply the minimal correct fix
4. **Run regression test** — Confirm the new test passes
5. **Refactor if needed** — Clean up without changing behavior
6. **Run full tests** — `cargo test --all-features` to catch regressions
7. **Report** — Update the status column in this file

Status key: `[ ]` pending, `[~]` in progress, `[x]` done, `[-]` won't fix (with reason)

---

## Critical Issues

### C1. `AgentEnd` never emitted when compact-and-retry fails `[x]`
- **File:** `src/agent/dispatcher.rs:325-337`
- **Confidence:** 95
- **Impact:** Any observer tracking in-flight requests leaks the span permanently when a compact-and-retry fails. The `?` operator early-returns from the retry path, bypassing all `AgentEnd` emission sites.
- **Fix direction:** Emit `AgentEnd` inside the `map_err` closure before returning, or use a guard/drop pattern.
- **Resolution:** Added `AgentEnd` emission inside the `.map_err` closure on the compact-retry failure path, matching the pattern used by the general error path (lines 352-357). The closure captures `self.observer()` and `agent_start` from the enclosing scope. Regression test `agent_end_emitted_on_compact_retry_failure` verifies that AgentStart/AgentEnd are always paired, even when both the initial LLM call and the compaction retry fail with `ContextLengthExceeded`. Test confirmed failing (1 start, 0 ends) before fix, passing after.

### C2. Retry LLM call is observably invisible `[x]`
- **File:** `src/agent/dispatcher.rs:291-389`
- **Confidence:** 92
- **Impact:** No `LlmRequest` emitted before the retry call. On success, `LlmResponse` reports duration that includes both the failed first call + the retry. `message_count` reflects the uncompacted count. Latency histograms and cost attribution are wrong.
- **Fix direction:** Reset `llm_start` after compaction. Emit a second `LlmRequest` for the retry with the compacted message count.
- **Resolution:** Three changes in the `ContextLengthExceeded` handler: (1) Emit `LlmResponse(success=false)` for the failed first call immediately after the error is caught, before compaction. (2) Reset `llm_start` and emit a new `LlmRequest` with the compacted `context_messages.len()` before the retry call, so latency only measures the retry. (3) In the retry failure `.map_err` closure, emit `LlmResponse(success=false)` for the failed retry before `AgentEnd`. Also extended `StubLlm` with `failing_first_n(n, response)` constructor for tests that need the first N calls to fail then succeed. Regression test `retry_llm_call_emits_separate_observer_events` verifies 2 LlmRequests, 2 LlmResponses (first failed, second success), and compacted message count. Test confirmed failing (1 LlmRequest) before fix, passing after.

### C3. Observer reports wrong model name under SmartRouting/Failover `[x]`
- **File:** `src/agent/dispatcher.rs:293-297`
- **Confidence:** 92
- **Impact:** `emit_llm_request` uses `self.llm().provider_name()` which returns the outermost wrapper's model. A haiku-routed request is reported as sonnet. Cost dashboards attribute cheap calls to the expensive model.
- **Fix direction:** Use `effective_model_name()` after the response returns for `LlmResponse`. For `LlmRequest`, document that it reflects the requested model (not the effective one). Or add an `effective_model` field to `LlmResponse`.
- **Resolution:** Three changes: (1) Added request-scoped routing tracking to `SmartRoutingProvider` using task-scoped `HashMap<tokio::task::Id, RoutedTo>` with `last_routed: AtomicU8` fallback (matching FailoverProvider's pattern). Overrides `effective_model_name()` to delegate to the provider that actually handled the request (cheap or primary). (2) Extended `delegate_llm_provider!` macro with a `skip_effective_model_name` variant so providers with custom routing can override just that method. (3) Changed dispatcher success-path `LlmResponse` to use `self.llm().effective_model_name(None)` instead of `active_model_name()`. Three regression tests added: simple→cheap reports "haiku", complex→primary reports "sonnet", cascade-escalation→primary reports "sonnet". All confirmed failing before fix (returned "sonnet" for simple), passing after.

### C4. Global `PROVIDER` OnceLock vs instance `provider` lifecycle mismatch `[x]`
- **File:** `src/observability/otel.rs:20,47-50`
- **Confidence:** 90
- **Impact:** `shutdown_tracer_provider()` shuts down the first-ever provider, not the active one. If `OtelObserver` is constructed twice (config reload), the second provider's spans are lost. Triple-reference (`self.provider`, global tracer, `PROVIDER` static) prevents clean instance drop.
- **Fix direction:** Make shutdown per-instance via a `shutdown()` method on `Observer`. Remove the `PROVIDER` static. Move `global::set_tracer_provider()` behind the `OnceLock::set()` success check.
- **Resolution:** Five changes: (1) Added `shutdown(&self)` to `Observer` trait with default implementation that calls `self.flush()`. (2) Implemented `OtelObserver::shutdown()` calling `self.provider.shutdown()` — per-instance, not via any static. (3) Removed the `static PROVIDER: OnceLock<SdkTracerProvider>` and the `pub fn shutdown_tracer_provider()` function entirely. (4) Added `MultiObserver::shutdown()` that forwards to all inner observers. (5) In `Agent::run()`, replaced `self.deps.observer.flush()` with `self.deps.observer.shutdown()` (which flushes and releases resources). Removed the `#[cfg(feature = "otel")] shutdown_tracer_provider()` call from `main.rs`. Also removed `OnceLock` import and `PROVIDER.set()` from `OtelObserver::new()` — only `global::set_tracer_provider()` remains for the tracing bridge. Regression test `shutdown_is_per_instance` creates two independent `OtelObserver` instances, records events on each, verifies per-instance flush produces correct span counts, then shuts down each independently. Test confirmed the `shutdown()` trait method didn't exist before the fix (compile error), passing after.

### C5. `observer.flush()` never called at agent shutdown `[x]`
- **File:** `src/main.rs` (missing call)
- **Confidence:** 95
- **Impact:** OTEL spans buffered in the batch exporter are silently dropped on exit. The `#[cfg(feature = "otel")] shutdown_tracer_provider()` call only works for the raw OTEL provider, not through the Observer trait.
- **Fix direction:** Call `observer.flush()` at the end of `Agent::run()` or in `main.rs` before the OTEL-specific shutdown.
- **Resolution:** Added `self.deps.observer.flush()` to the cleanup section of `Agent::run()` (after `channels.shutdown_all()`, before returning). This flushes through the `Observer` trait, so it works for all backends: `OtelObserver` calls `provider.force_flush()`, `MultiObserver` forwards to all inner observers, `NoopObserver`/`LogObserver` no-op. Also enhanced `RecordingObserver` with `flush()` tracking and `with_flush_counter()` constructor for testability. Regression test added: creates a minimal agent that shuts down via `/quit` and asserts `flush_count == 1`. The existing `shutdown_tracer_provider()` in `main.rs` is kept as a belt-and-suspenders measure for the global OTEL provider.

---

## Important Issues

### I1. Mutex poison silently swallowed in OtelObserver `[x]`
- **File:** `src/observability/otel.rs:100,138,154,225,238,308`
- **Confidence:** 88
- **Impact:** All `active_spans.lock()` uses `if let Ok(...)`, silently dropping all telemetry after any panic while holding the lock. The entire OTEL backend goes dark with zero operator indication.
- **Fix direction:** Use `lock().unwrap_or_else(|e| e.into_inner())` to recover from poison (safe for a HashMap), or at minimum log a warning on lock failure.
- **Resolution:** Added private `lock_spans()` helper method that uses `lock().unwrap_or_else(|e| { warn!("active_spans mutex was poisoned, recovering"); e.into_inner() })`. Replaced all 7 `if let Ok(...) = self.active_spans.lock()` patterns with calls to `lock_spans()`: `agent_context()`, `AgentStart`, `LlmRequest`, `LlmResponse`, `ToolCallStart`, `ToolCallEnd`, `AgentEnd`. The HashMap's worst-case inconsistency after poison is a stale or missing span entry — recoverable and far better than permanent telemetry blackout. Regression test `continues_after_mutex_poison` intentionally poisons the mutex via `catch_unwind`, then verifies AgentStart/AgentEnd still produce a span with correct attributes. Test confirmed failing (missing `ironclaw.agent.duration_secs` attribute) before fix, passing after.

### I2. OtelObserver leaks spans when iteration limit fires `[x]`
- **File:** `src/agent/dispatcher.rs:194-199`
- **Confidence:** 88
- **Impact:** The iteration-limit path emits `AgentEnd` without a preceding `LlmResponse`, orphaning the "llm" span in `active_spans` forever. The Context/Span objects prevent export until removed.
- **Fix direction:** Emit `LlmResponse` (with error status) before `AgentEnd` in the iteration-limit path, or have `AgentEnd` clean up all active child spans.
- **Resolution:** Fixed in `OtelObserver::record_event` `AgentEnd` handler: before ending the agent span, drains all remaining entries from `active_spans` (orphaned "llm", "tool:*" spans), sets error status (`"span '{}' orphaned at agent shutdown"`), and ends each one. Lock is acquired once, orphans collected, lock released, then spans are ended outside the lock. This is a defensive belt-and-suspenders fix — while the current dispatcher closes all child spans in normal flow, the iteration-limit/interrupt/cost-guardrail early-exit paths at the top of the loop could leave orphans if the code evolves. Regression test `agent_end_drains_orphaned_child_spans` simulates AgentStart → LlmRequest → ToolCallStart → AgentEnd (without LlmResponse/ToolCallEnd) and verifies all 3 spans are exported, orphaned children have error status, and `active_spans` is empty. Test confirmed failing (1 span exported) before fix, passing (3 spans, error status on orphans) after.

### I3. `compact_messages_for_retry` emits false compaction note `[x]`
- **File:** `src/agent/dispatcher.rs:1041-1047`
- **Confidence:** 88
- **Impact:** When input is `[System, User]`, the guard `idx > 0` is true but nothing was actually dropped. The note "Earlier conversation history was automatically compacted" is a lie. Test at line 1493 asserts and reinforces this wrong behavior.
- **Fix direction:** Change guard to check whether any non-system messages were dropped: `messages[..idx].iter().any(|m| m.role != Role::System)`.
- **Resolution:** Changed the guard from `idx > 0` to `messages[..idx].iter().any(|m| m.role != Role::System)`. The compaction note is now only inserted when non-system messages were actually dropped from the history. Fixed two existing tests that asserted the wrong behavior: `test_compact_single_user_message_keeps_everything` (expected 3 messages with note, now correctly expects 2 without) and `test_compact_no_duplicate_system_after_last_user` (expected 6 with note, now correctly expects 5 without). Two regression tests added: `no_false_compaction_note_when_only_system_precedes_user` and `no_false_compaction_note_with_multiple_systems`. Both confirmed failing before fix, passing after.

### I4. Cache hits produce misleading `LlmResponse` events `[x]`
- **File:** `src/llm/response_cache.rs:156-163`
- **Confidence:** 85
- **Impact:** Near-zero latency with non-zero tokens is physically impossible for a real call. Observer cannot distinguish cache hits from real LLM calls. Cost calculations are inflated.
- **Fix direction:** Add `cached: bool` field to `ObserverEvent::LlmResponse`, or emit a distinct event. Minimal: document the limitation.
- **Resolution:** Four-layer fix propagating cache status end-to-end: (1) Added `cached: bool` field to `CompletionResponse` (default `false`); `CachedProvider::complete()` sets `cached: true` on cache hits. (2) Added `cached: bool` to `RespondOutput` so the reasoning engine propagates the flag from `CompletionResponse` through to the dispatcher. (3) Added `cached: bool` to `ObserverEvent::LlmResponse`; dispatcher sets `cost_usd: None` and skips `cost_guard.record_llm_call()` for cached responses, preventing cost inflation. (4) Updated observer backends: `LogObserver` includes `cached` in structured log output; `OtelObserver` sets `ironclaw.response.cached=true` span attribute for filtering in Jaeger. Regression test `cache_hit_marks_response_as_cached` verifies cache miss returns `cached=false` and cache hit returns `cached=true` with preserved token counts. Feature isolation verified: `--no-default-features --features libsql` compiles cleanly.

### I5. `ChannelMessage` outbound event emitted before hook can block it `[x]`
- **File:** `src/agent/agent_loop.rs:502-508`
- **Confidence:** 85
- **Impact:** If `BeforeOutbound` hook blocks the response, the outbound event was already recorded. Produces phantom counts in dashboards.
- **Fix direction:** Move `record_event` to after the `respond()` call succeeds.
- **Resolution:** Restructured the outbound response block in the agent loop. The `ChannelMessage { direction: "outbound" }` observer event is now emitted only after `respond()` succeeds — not when the hook blocks (returns `Err`), and not when the channel send fails. Refactored the hook match arms to collect `send_result: Option<Result<(), _>>`, then a single post-match block emits the event on `Some(Ok(()))`, logs the error on `Some(Err(e))`, or no-ops on `None` (hook blocked). Regression test `no_outbound_event_when_hook_blocks` sends `/ping` through a `BeforeOutbound` reject hook and asserts zero outbound events (was 1 before fix) while inbound events are still recorded.

### I6. Quality gate script omits `--benches --tests --examples` `[x]`
- **File:** `scripts/quality-gate.sh:45`
- **Confidence:** 82
- **Impact:** Clippy doesn't check test code, so violations in `otel.rs` tests or `otel_e2e.rs` pass silently. CLAUDE.md specifies the full flags.
- **Fix direction:** Change to `cargo clippy --all --benches --tests --examples --all-features -- -D warnings`.
- **Resolution:** Added `--benches --tests --examples` to the clippy invocation on line 45, matching the flags specified in CLAUDE.md. One-line shell script fix.

### I7. `ObservabilityConfig` bypasses the Settings system `[x]`
- **File:** `src/config/mod.rs:200-208`
- **Confidence:** 83
- **Impact:** Constructed via raw `std::env::var()` in `Config::build()`, the only config sub-struct that doesn't use `resolve(settings)`. Can't be set via `ironclaw config set`. Breaks the config pattern.
- **Fix direction:** Add `ObservabilityConfig::resolve(settings)?` following the established pattern. If env-only is intentional, add a comment explaining why.
- **Resolution:** Three changes: (1) Added `ObservabilitySettings` to `Settings` struct in `settings.rs` with fields mirroring `ObservabilityConfig` (backend, otel_endpoint, otel_protocol, otel_service_name) — enables `ironclaw config set observability.backend log` and DB/TOML persistence. (2) Added `ObservabilityConfig::resolve(settings)` method using `optional_env()` / `parse_string_env()` helpers from `crate::config::helpers`, following the established pattern: env var > settings > default. (3) Replaced inline `std::env::var()` construction in `Config::build()` with `ObservabilityConfig::resolve(settings)?`. Four regression tests: `resolve_reads_from_settings` (settings used when env unset), `resolve_env_overrides_settings` (env wins), `observability_settings_visible_in_list` (discoverable via CLI), `observability_settings_db_round_trip` (DB persistence). Feature isolation verified: `--no-default-features --features libsql` compiles cleanly.

### I8. `tracing-opentelemetry` bridge attached unconditionally when otel feature is compiled in `[x]`
- **File:** `src/channels/web/log_layer.rs:207-210`
- **Confidence:** 83
- **Impact:** When compiled with `otel` feature but `OBSERVABILITY_BACKEND=none`, every tracing span routes through OTEL bridge machinery to a no-op provider. Wasteful. Also fragile ordering: the bridge is attached before `OtelObserver::new()` sets the global provider.
- **Fix direction:** Only attach the bridge layer when `OBSERVABILITY_BACKEND` contains `otel`. Pass a flag from config.
- **Resolution:** Three changes: (1) Added `ObservabilityConfig::wants_otel()` method that checks if `self.backend.contains("otel")`. (2) Added `enable_otel: bool` parameter to `init_tracing()` and conditioned the `tracing_opentelemetry::layer()` attachment on it: `if enable_otel { Some(layer) } else { None }`. (3) Updated `main.rs` call site to pass `config.observability.wants_otel()`. When compiled with `otel` feature but `OBSERVABILITY_BACKEND=none`, the bridge layer is now `None` (zero overhead via the `Option` layer). Feature isolation verified: `--no-default-features --features libsql` compiles cleanly (unused `enable_otel` suppressed). Regression test `wants_otel_reflects_backend_config` verifies all backend strings: "none", "noop", "log", "" → false; "otel", "log+otel", "otel+log" → true.

### I9. `ChannelMessage`/`HeartbeatTick` spans are orphaned roots `[x]`
- **File:** `src/observability/otel.rs:281-301`
- **Confidence:** 80
- **Impact:** These use `.start(&self.tracer)` without parent context, unlike `LlmRequest`/`ToolCallStart` which correctly use `start_with_context`. Breaks trace hierarchy in Jaeger — these appear as unconnected root spans.
- **Fix direction:** Add `let parent_cx = self.agent_context()` and use `start_with_context` for both variants, matching the pattern used by other event types.
- **Resolution:** Added `let parent_cx = self.agent_context()` and changed `.start(&self.tracer)` to `.start_with_context(&self.tracer, &parent_cx)` for both `ChannelMessage` and `HeartbeatTick` handlers, matching the pattern used by `LlmRequest`, `ToolCallStart`, and `TurnComplete`. The `agent_context()` helper returns the active agent span's context (or `Context::current()` as fallback), so these spans now correctly appear as children of the `invoke_agent` root span in Jaeger instead of disconnected orphans. Regression test `channel_and_heartbeat_spans_are_children_of_agent` verifies both spans have the agent span as parent. Test confirmed failing (`parent_span_id: 0000000000000000`) before fix, passing after. Note: the `Error` variant also uses `.start(&self.tracer)` without parent context — left as-is since errors can occur outside an agent session.

---

## Design Concerns (non-blocking, address opportunistically)

### D1. LLM decorator boilerplate duplicated 5x `[x]`
- **Files:** `circuit_breaker.rs`, `retry.rs`, `response_cache.rs`, `smart_routing.rs`, `failover.rs`
- **Impact:** Every wrapper re-implements ~9 passthrough methods identically. If `LlmProvider` grows a new method, every decorator must be updated or silently falls through to defaults. This was the root cause of C3 (wrong model name).
- **Fix direction:** Create a `delegate_llm_provider!` macro or a `DelegatingProvider` trait with default passthrough implementations.
- **Resolution:** Created `delegate_llm_provider!` macro in `provider.rs` (9 methods: `provider_name`, `model_name`, `cost_per_token`, `list_models`, `model_metadata`, `effective_model_name`, `active_model_name`, `set_model`, `calculate_cost`). Applied to `CircuitBreakerProvider`, `RetryProvider`, `CachedProvider`, `SmartRoutingProvider`. `FailoverProvider` remains manual (unique delegation pattern: `providers[last_used]`, task-scoped bindings, aggregated `list_models`). Also fixed a latent bug: `RetryProvider` and `SmartRoutingProvider` were missing `effective_model_name` delegation — regression test added and passing.

### D2. `TurnComplete` over-counts tool calls when approval interrupts `[x]`
- **File:** `src/agent/dispatcher.rs` (TurnComplete emission)
- **Impact:** Reports `tool_calls.len()` but tools after the approval check haven't executed. Should use `runnable.len()`.
- **Fix direction:** Use the count of actually-executed tools.
- **Resolution:** Changed `tool_calls_in_turn: tool_calls.len() as u32` to `tool_calls_in_turn: runnable.len() as u32` in the TurnComplete emission (line 894). `runnable` contains only tools that passed preflight AND were actually executed — it excludes hook-rejected tools, the approval-requiring tool itself, and all deferred tools after the approval point. Regression test `turn_complete_counts_only_executed_tools` uses a custom LLM returning 3 tool calls (echo, approve_me, echo) where `approve_me` always requires approval. Verified: before fix reports 3, after fix reports 1 (only the first echo that actually ran). Test confirmed failing (got 3, expected 1) before fix, passing after.

### D3. `RecordingObserver` declared but unused by any test `[x]`
- **File:** `src/observability/recording.rs`, `src/testing.rs`
- **Impact:** `testing.rs` hard-codes `NoopObserver`. The scaffolding exists but no test asserts on recorded events. Dead code relative to its stated purpose.
- **Fix direction:** Wire `RecordingObserver` into `TestHarnessBuilder` so tests can opt-in to event assertions, or document it as scaffolding for future tests.
- **Resolution:** Added `with_observer(Arc<dyn Observer>)` method to `TestHarnessBuilder`, matching the existing pattern of `with_db`, `with_llm`, `with_tools`. The `build()` method uses the provided observer or falls back to `NoopObserver`. Refactored all 4 existing tests that manually overrode `harness.deps.observer` after building (agent_loop C5 flush test, dispatcher C1/C2 tests, D2 turn-count helper) to use the builder API instead, eliminating the `mut harness` + post-build override pattern. Regression test `test_harness_custom_observer` verifies the observer is wired through `deps` and events are captured. Also cleaned up 4 clippy warnings in D2 test code (unused import, unused variable, collapsible `if`, borrowed expression).

### D4. Missing `[[test]]` entry for `otel_e2e` in Cargo.toml `[x]`
- **File:** `Cargo.toml`, `tests/otel_e2e.rs`
- **Impact:** Test binary compiles as a no-op in default builds (no `otel` feature). Wastes compile time.
- **Fix direction:** Add `[[test]] name = "otel_e2e" required-features = ["otel"]`.
- **Resolution:** Added `[[test]] name = "otel_e2e" required-features = ["otel"]` to Cargo.toml, matching the existing `html_to_markdown` pattern. Verified: default `cargo test --no-run` no longer compiles the `otel_e2e` binary; `--features otel` still compiles it correctly. Saves one empty binary link per default test run.

### D5. Double lock acquisition in `OtelObserver` (TOCTOU on span map) `[x]`
- **File:** `src/observability/otel.rs:73-80,130,215,271`
- **Impact:** `agent_context()` acquires and releases the lock, then the caller acquires it again to insert. Under concurrent use (the struct is `Send + Sync`), state can change between the two acquisitions.
- **Fix direction:** Lock once per `record_event` call and pass the guard to helper methods.
- **Resolution:** Changed `agent_context(&self)` to `agent_context(spans: &HashMap<String, Context>)` — a static method that takes a reference to the already-locked map instead of acquiring its own lock. Two high-risk callers (`LlmRequest`, `ToolCallStart`) now hold a single `MutexGuard` across both the context lookup and the span insert, eliminating the TOCTOU window. Three read-only callers (`TurnComplete`, `ChannelMessage`, `HeartbeatTick`) also updated for consistency. Regression test `concurrent_tool_starts_have_correct_parent` spawns 10 threads concurrently starting/ending tool calls and verifies all get the correct agent parent span and `active_spans` is empty after `AgentEnd`.

---

## Priority Order for Fixes

Architecture-impacting issues first (they inform multiple other fixes), then critical, then important:

1. **D1** — Decorator boilerplate (root cause of C3, enables cleaner fixes)
2. **C5** — Observer flush at shutdown (trivial, high impact)
3. **C1** — AgentEnd not emitted on retry failure
4. **C2** — Invisible retry call with wrong timing
5. **C3** — Wrong model name (partially fixed by D1)
6. **C4** — Provider lifecycle mismatch
7. **I1** — Mutex poison handling
8. **I2** — Span leak on iteration limit
9. **I3** — False compaction note
10. **I6** — Quality gate clippy flags
11. **I5** — Outbound event before hook
12. **I9** — Orphaned root spans
13. **I8** — Unconditional tracing bridge
14. **I7** — Config pattern bypass
15. **I4** — Cache hit event ambiguity
16. **D2-D5** — Remaining design concerns
