# IronClaw Test Coverage Audit

**Date:** 2026-02-23
**Scope:** Full src/ directory (249 .rs files, ~113,886 LOC, ~1,500 tests)

---

## Executive Summary

**~1,500 tests across 113,886 lines of code = 1.25% test-to-code ratio** (industry standard: 15-30%).

31.7% of files (79/249) have **zero tests**. Tests are concentrated in a few well-tested modules while critical infrastructure (database, web API, session management) is essentially untested. Of the 1,500 tests, only ~300 provide real behavioral confidence.

---

## Test Distribution by Module

| Module | Tests | Quality |
|--------|-------|---------|
| tools/ | 297 | Moderate -- mostly schema/parsing tests |
| llm/ | 186 | Good -- retry, reasoning, adapter well-tested |
| agent/ | 177 | Mixed -- cost_guard excellent, scheduler empty stubs |
| channels/ | 162 | Moderate -- WASM lifecycle, web barely covered |
| skills/ | 77 | Moderate -- selector/catalog scoring tests |
| safety/ | 58 | Good -- leak detector and sanitizer well-tested |
| tunnel/ | 38 | Moderate |
| workspace/ | 33 | Moderate -- chunker good, repository zero |
| registry/ | 33 | Moderate |
| hooks/ | 32 | Moderate |
| worker/ | 31 | Good -- parallel execution verified |
| sandbox/ | 30 | Low -- no security boundary tests |
| cli/ | 30 | Low |
| orchestrator/ | 28 | Low |
| settings/ | 24 | Moderate |
| secrets/ | 23 | Moderate |
| setup/ | 20 | Low |
| extensions/ | 17 | Low |
| context/ | 16 | Low |
| config/ | 13 | Low |
| estimation/ | 11 | Low |
| evaluation/ | 5 | Very low |

---

## The Coverage Desert (Critical Gaps)

### Zero or Near-Zero Coverage

| Module | Lines of Code | Tests | Coverage | Risk |
|--------|--------------|-------|----------|------|
| **Database layer** (postgres + libsql) | 4,480 | 3 | **0.06%** | CRITICAL |
| **Web server** (40+ API endpoints) | 2,920 | 3 | **0.1%** | CRITICAL |
| **Thread operations** (session lifecycle) | 1,390 | 0 | **0%** | CRITICAL |
| **DB migrations** (schema correctness) | 549 | 0 | **0%** | CRITICAL |
| **Workspace repository** (memory CRUD) | 503 | 0 | **0%** | HIGH |
| **Commands module** (job management) | 494 | 0 | **0%** | HIGH |
| **main.rs** (startup/init) | 1,104 | 0 | **0%** | MEDIUM |

### What This Means

- Every `INSERT`, `SELECT`, `UPDATE`, `DELETE` in the database layer is unverified
- All 40+ web API endpoints have no route/status-code/auth tests
- Session lifecycle (thread creation, hydration, cleanup) is untested
- Schema migrations could silently break with no test to catch it

---

## Test Quality Breakdown

### Distribution

| Category | Count | % | Description |
|----------|-------|---|-------------|
| **High quality** (behavioral) | ~300 | 21% | Verify complex state, error semantics, security boundaries |
| **Moderate** (structural/parsing) | ~800 | 56% | Serialization roundtrips, enum matching, conversion logic |
| **Trivial** (padding) | ~300 | 21% | Display impls, default constructors, simple getters |
| **Broken stubs** | 2 | 0.1% | Empty bodies that always pass |

### High Quality Tests (Behavioral)

#### Cost Guard (`src/agent/cost_guard.rs`) -- 10 tests, EXCELLENT
- Daily budget enforcement (prevents overspend)
- Hourly rate limiting (3 actions/hour)
- Per-model cost tracking and aggregation
- Tests verify constraint enforcement, not just data structures

#### Leak Detector (`src/safety/leak_detector.rs`) -- 18 tests, EXCELLENT
- Detects 15+ secret patterns (OpenAI keys, AWS keys, GitHub tokens, PEM keys, Bearer tokens)
- Tests redaction vs blocking vs clean passthrough
- Binary body scanning with UTF-8 lossy conversion
- HTTP request scanning (URLs, headers, bodies for exfiltration)

#### Retry/Circuit Breaker (`src/llm/retry.rs`) -- 13 tests, EXCELLENT
- Exponential backoff with jitter (750-1250ms for attempt 0, 1500-2500ms for attempt 1)
- Retryable vs non-retryable error classification
- Transient vs permanent errors (context length, auth)
- Exhaust retries scenario

#### Session Manager (`src/agent/session_manager.rs`) -- 19 tests, EXCELLENT
- Session lifecycle and thread resolution
- User/channel isolation
- Stale session pruning
- External thread ID handling

#### Worker Parallel Execution (`src/agent/worker.rs`) -- 8 tests, EXCELLENT
- Parallel tool execution speedup (200ms 3-way parallel vs 600ms sequential)
- Result ordering preservation despite async completion order
- Error handling when tool doesn't exist

#### Reasoning (`src/llm/reasoning.rs`) -- 56 tests, GOOD
- Thinking tag stripping (8+ variants: `<thinking>`, `<think>`, `<thought>`, `<reasoning>`, `<reflection>`)
- Regex edge cases (whitespace, case insensitivity, attributes)
- Tool call tag removal

### Trivial Tests (Padding)

#### String Truncation (`src/agent/agent_loop.rs`) -- 8 tests
```
test_truncate_short_input       -- "abc" -> "abc"
test_truncate_empty_input       -- "" -> ""
test_truncate_exact_length      -- 10 chars at limit 10 -> same
test_truncate_over_limit        -- truncates with "..."
test_truncate_collapses_newlines
test_truncate_collapses_whitespace
test_truncate_multibyte_utf8
test_truncate_cjk_characters
```
8 tests for a trivial utility function. Only the UTF-8/CJK tests have marginal value.

#### Scheduler -- 2 EMPTY stubs
```rust
#[test]
fn test_scheduler_creation() {
    // Would need to mock dependencies for proper testing
}

#[tokio::test]
async fn test_spawn_batch_empty() {
    // For now just verify the empty case doesn't panic.
}
```

#### Routine Engine -- 2 trivial tests
```rust
fn test_run_status_icons() {
    // Just calls to_string() on each enum variant
}
fn test_notification_gating() {
    // Asserts struct fields exist
}
```

---

## Untested Error Paths

| Category | Examples | Impact |
|----------|----------|--------|
| **Database failures** | DB down, transaction rollback, constraint violation | No tests |
| **Network errors** | LLM timeout, proxy failure, channel disconnect | No tests |
| **Resource exhaustion** | Memory limits, channel capacity, queue overflow | No tests |
| **Authentication** | Token expiry, invalid creds, permission check | No tests |
| **Tool execution** | Timeout, segfault, resource limits | No tests |
| **Sandbox escapes** | Network policy violation, file access outside workspace | No tests |

## Untested State Transitions

| Transition | Location | Tests |
|------------|----------|-------|
| `Pending -> InProgress -> Completed/Failed/Stuck` | `context/state.rs` | **Zero** |
| Session cleanup / stale detection | `session_manager.rs` | Partially tested |
| Message routing across channels | `channel/manager.rs` | **Zero** |
| Tool approval escalation | `dispatcher.rs` | Partially tested |

## Missing Integration Tests

- No `testcontainers` setup for PostgreSQL
- No HTTP test client for web server routes
- No test database setup/teardown harness
- No multi-user concurrent job execution tests
- No session recovery after crash tests
- No LLM failover chain activation tests
- No WASM module loading/caching integration tests

---

## Anti-Patterns Found

### 1. Stub/Placeholder Tests (2 found)
Empty test functions that pass trivially and provide zero confidence.

### 2. Excessive Trivial Tests
8 tests for string truncation, Display impl tests, enum variant matching tests. These inflate the count without catching logic bugs.

### 3. Happy-Path Only Testing
- Retry logic tested with artificial StubLlm, no real error injection
- Network proxy tested with hardcoded domains, not real requests
- Database tested only for success (3 tests on 4,480 LOC)

### 4. No Error Injection Framework
No mechanism to simulate: database failures, network timeouts, resource exhaustion, malformed inputs from external services.

### 5. Untested Variants of Complex Logic
- `Worker::execute_tools_parallel()` tests 3 tools; missing: 0, 1, 100
- `LeakDetector::scan_http_request()` tests parts separately; missing: combined
- `SessionManager::resolve_thread()` missing: concurrency race conditions

---

## The Agentic Development Pattern

This test distribution reveals AI-generation:

1. **Tests generated alongside code** -- each module's tests only cover what was in the AI's context window
2. **No test-after-the-fact pass** -- 40-endpoint web server has zero route tests
3. **Easy things over-tested** -- 8 tests for string truncation vs 0 for database queries
4. **No integration test infrastructure** -- no testcontainers, test DB, HTTP test client
5. **Tests never planned holistically** -- each module tested in isolation

---

## Adjusted Confidence

| Metric | Raw | Adjusted |
|--------|-----|----------|
| **Total tests** | ~1,500 | ~300 meaningful |
| **Test-to-code ratio** | 1.25% | ~0.26% effective |
| **Module coverage** | 68% of files | ~40% of critical paths |

---

## Recommendations

### Immediate (highest impact)

1. **Database integration tests** -- 50+ tests covering CRUD, transactions, migrations for both PostgreSQL and libSQL backends
2. **Web server route tests** -- 20+ tests covering all 40 endpoints, status codes, auth, error responses
3. **Thread operations tests** -- 15+ tests for session hydration, message restoration, external ID mapping
4. **Job state machine tests** -- Test all transitions in `Pending -> InProgress -> Completed/Failed/Stuck`

### Medium Priority

5. **Error injection tests** -- Simulate DB failures, LLM timeouts, network errors
6. **Remove/consolidate trivial tests** -- Reclaim ~100 tests worth of attention
7. **Sandbox security boundary tests** -- Policy enforcement, escape attempts
8. **Concurrent execution tests** -- Multi-user, multi-channel, parallel jobs

### Infrastructure Needed

9. **testcontainers for PostgreSQL** -- Real DB integration tests
10. **axum-test or tower-test for HTTP** -- Route-level testing
11. **Error injection framework** -- Controllable failure points for resilience testing
12. **Coverage measurement** -- `cargo tarpaulin` or `llvm-cov` to track actual line coverage
