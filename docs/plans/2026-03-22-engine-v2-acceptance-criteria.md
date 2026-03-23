# Engine v2 Acceptance Criteria

**Date:** 2026-03-22
**Status:** Active
**Author:** Zaki Manian
**Goal:** Define the merge bar for replacing the v1 agent loop with the v2 engine (`crates/ironclaw_engine/`). Phase 6 is not done until every criterion below is met.

---

## Overview

The v2 engine replaces ~10 v1 abstractions (Session, Job, Routine, Channel, Tool, Skill, Hook, Observer, Extension, LoopDelegate) with 5 primitives (Thread, Step, Capability, MemoryDoc, Project). Phases 1-5 are complete: types, execution loop, CodeAct/Monty, budget controls, and conversation surface.

Phase 6 delivers the bridge adapters (`LlmBridgeAdapter`, `StoreBridgeAdapter`, `EffectBridgeAdapter`) that connect the engine to existing IronClaw infrastructure. The acceptance criteria below define what "ready to replace v1" means. Nothing merges to `staging` until all pass.

---

## Acceptance Criteria

### 1. Behavioral Equivalence

Every observable behavior of v1 must be reproduced by v2 running through bridge adapters.

| # | Criterion | Verification |
|---|-----------|-------------|
| 1.1 | All existing E2E test fixtures pass through `EngineV2Delegate` | `cargo test --features integration -p ironclaw -- engine_v2` and `cd tests/e2e && pytest` with `ENGINE_V2=true` |
| 1.2 | Tool dispatch produces identical outputs for identical inputs | Add property test: for each built-in tool, run same `(name, params)` through v1 `execute_tool_with_safety()` and v2 `EffectBridgeAdapter::execute_action()`, assert outputs match |
| 1.3 | Error handling is equivalent: no silent failures where v1 errors, no errors where v1 succeeds | Diff test: run full E2E trace fixtures through both paths, compare `LoopOutcome` variants. Specifically test: invalid tool name, malformed params, timeout, policy deny |
| 1.4 | Approval flows work identically | Test sequence: tool with `requires_approval` -> pause -> user approves -> resume -> completion. Must produce same SSE events (`approval_needed`, `approval_resolved`) |
| 1.5 | System commands (`/help`, `/model`, `/status`, `/skills`, `/job`) produce equivalent responses | Command parity test: submit each system command through v2 conversation surface, compare output structure |
| 1.6 | Compaction produces equivalent context reduction | Run a 50-turn conversation through both engines, trigger compaction, compare resulting context window token count (must be within 5%) |

**Blocking:** 1.1, 1.2, 1.3, 1.4 are hard blockers. 1.5 and 1.6 may be deferred to Phase 7 with written justification.

### 2. Performance

No performance regressions. Improvements expected from context-as-variables but not required.

| # | Criterion | Target | Verification |
|---|-----------|--------|-------------|
| 2.1 | P50 step latency | Within +10% of v1 | Benchmark harness: `cargo bench -p ironclaw --bench step_latency` with mock LLM (fixed 50ms response). Run 1000 steps, compare distributions. Harness must test both engines in the same binary. |
| 2.2 | P95 step latency | Within +10% of v1 | Same harness as 2.1 |
| 2.3 | P99 step latency | Within +15% of v1 | Same harness as 2.1 (wider margin for tail latency) |
| 2.4 | Monty VM startup | < 1ms (verify the 0.06ms claim) | Dedicated microbenchmark: `cargo bench -p ironclaw_engine --bench monty_startup`. Time `MontyVm::new()` over 10,000 iterations, report P50/P99. Must include independent measurement, not self-reported. |
| 2.5 | Token efficiency | Neutral or improved | Measure total tokens (prompt + completion) for the same 10-turn conversation fixture through both engines. v2 must not use more tokens than v1. Context-as-variables should reduce prompt tokens by 10-30% on conversations with tool output > 4KB. |
| 2.6 | Memory per thread | No regression | Measure RSS delta when spawning 100 threads with mock LLM. v2 must not exceed v1 by more than 10%. |

**Blocking:** 2.1, 2.2, 2.3 are hard blockers. 2.4, 2.5, 2.6 are soft blockers (documented regressions acceptable with mitigation plan).

### 3. Safety and Security

The engine itself contains no safety logic by design. Safety is enforced at the bridge boundary (`EffectBridgeAdapter`). This must be airtight.

| # | Criterion | Verification |
|---|-----------|-------------|
| 3.1 | `SafetyLayer` (prompt injection, leak detection, content validation) is applied on every action execution through `EffectBridgeAdapter` | Unit test: mock `EffectExecutor` that logs calls, verify `SafetyLayer::validate_tool_input()` and `SafetyLayer::sanitize_tool_output()` are called for every `execute_action()` invocation. No code path bypasses this. |
| 3.2 | Policy engine enforces `Deny > RequireApproval > Allow` with zero bypasses | Test matrix: for each `EffectType` variant (ReadLocal, ReadExternal, WriteLocal, WriteExternal, CredentialedNetwork, Compute, Financial), create conflicting rules and verify Deny always wins, RequireApproval wins over Allow. Cover the case where a single action triggers multiple effect types. |
| 3.3 | Thread tree is acyclic with bounded depth | `ThreadTree::attach()` must reject cycles (test: A->B->C->A). `ThreadConfig::max_depth` must be enforced (test: exceed depth limit, verify `ThreadError::DepthExceeded`). Default max depth: 8. |
| 3.4 | Capability leases are checked before every action execution | Audit `ExecutionLoop::run()` and `execute_action_calls()`: no path from LLM response to `EffectExecutor::execute_action()` that skips `LeaseManager::check_lease()`. Verify with test: expired lease -> action denied, revoked lease -> action denied, exhausted `max_uses` -> action denied. |
| 3.5 | Monty VM panics cannot crash the host | Test: inject Python code that triggers a Monty panic (e.g., stack overflow, infinite allocation). Verify the step completes with `StepStatus::Failed`, thread continues or fails gracefully, no process abort. Specifically test all resource limits: 30s timeout, 64MB memory, 1M allocations. |
| 3.6 | No new attack surfaces | Review checklist (manual, documented in PR): (a) lease forgery: `LeaseId` cannot be guessed or constructed outside `LeaseManager::grant()`, (b) policy bypass: no public method on `ExecutionLoop` that executes actions without policy check, (c) effect escalation: action's declared `EffectType` cannot be changed after capability registration, (d) cross-thread lease usage: lease bound to `thread_id` is enforced. |

**Blocking:** All items are hard blockers. 3.6 is a manual review checklist that must be signed off in the merge PR.

### 4. Persistence and Migration

Production requires durable state. `InMemoryStore` is for tests only.

| # | Criterion | Verification |
|---|-----------|-------------|
| 4.1 | `StoreBridgeAdapter` implements the full `Store` trait (18 methods) for both PostgreSQL and libSQL | Integration test per backend: create thread -> add steps -> append events -> save leases -> restart process -> load thread -> verify all data intact. Run with `cargo test --features integration` (postgres) and default (libSQL). |
| 4.2 | Database migrations create all required tables | Migration V14+ creates: `engine_threads`, `engine_steps`, `engine_events`, `engine_projects`, `engine_memory_docs`, `engine_capability_leases`. Test: run migrations on empty database, verify tables exist with correct schemas. Both backends. |
| 4.3 | Thread state survives process restart | Integration test: start thread -> execute 3 steps -> kill process -> restart -> resume thread -> verify step count is 3, thread state is correct, events are intact. |
| 4.4 | In-flight v1 sessions continue working when v2 is enabled | Test: create v1 session with active thread -> enable `ENGINE_V2=true` -> new messages on the existing session use v1 path (not v2). Only new threads use v2. Verify with assertion on delegate type. |
| 4.5 | Data migration path is documented | `docs/plans/` must contain a migration guide covering: (a) which v1 tables map to which v2 tables, (b) whether historical data is migrated or v2 starts fresh, (c) rollback procedure if migration fails. |

**Blocking:** 4.1, 4.2, 4.3, 4.4 are hard blockers. 4.5 is required documentation but may ship as a separate document in the same milestone.

### 5. Observability

The engine must emit enough telemetry to debug production issues without attaching a debugger.

| # | Criterion | Verification |
|---|-----------|-------------|
| 5.1 | Step execution duration is recorded | Each `Step` must have `started_at` and `completed_at` timestamps. Verify via unit test: execute a step, assert both fields are set and `completed_at > started_at`. |
| 5.2 | Token usage is tracked per step and per thread | `Step::token_usage` must be populated from `LlmOutput`. Thread-level aggregation: `thread.steps.iter().map(|s| s.token_usage).sum()`. Verify: run 5 steps with known token counts from mock LLM, assert thread total matches. |
| 5.3 | Policy decision counters | `PolicyEngine` must expose counts of `Allow`, `Deny`, and `RequireApproval` decisions. Verify: run 10 actions with mixed policies, assert counters match expected values. These must be queryable (not just logged). |
| 5.4 | Active lease gauge | `LeaseManager` must expose current active lease count. Verify: grant 5 leases, revoke 2, expire 1, assert gauge reads 2. |
| 5.5 | Event sourcing query performance | `Store::load_events(thread_id)` must return within 100ms for a thread with 1000 events. Benchmark test with both backends. |
| 5.6 | Structured logging for execution loop | Each step must emit `tracing` spans with: `thread_id`, `step_index`, `execution_tier`, `duration_ms`, `token_count`. Verify by capturing tracing output in test and asserting field presence. |

**Blocking:** 5.1, 5.2, 5.6 are hard blockers. 5.3, 5.4, 5.5 are soft blockers (must be filed as issues if deferred).

### 6. Rollout Strategy

No big-bang cutover. Gradual rollout with rollback capability.

| # | Criterion | Verification |
|---|-----------|-------------|
| 6.1 | Feature flag `ENGINE_V2` controls engine selection | When `ENGINE_V2=true`: new threads use `EngineV2Delegate`. When `ENGINE_V2=false` (default): all threads use v1. Verify: start with flag off, create thread (v1), set flag on, create thread (v2), both work. |
| 6.2 | Existing threads continue on their original engine | A thread started on v1 must remain on v1 even when `ENGINE_V2=true`. Thread metadata must record which engine version created it. Verify: create v1 thread, enable v2, send message to v1 thread, assert v1 delegate is used. |
| 6.3 | Rollback path: disable flag, no data loss | Enable v2, create threads, disable v2. v2 threads become read-only (no new messages accepted) but their data persists. New threads use v1. No data corruption in either direction. |
| 6.4 | Percentage-based rollout support | `ENGINE_V2_ROLLOUT_PERCENT=10` routes 10% of new threads to v2 (hash of thread_id mod 100). This enables canary deployment. Verify: create 100 threads with rollout at 10%, assert approximately 10 use v2. |
| 6.5 | Canary validation period | Before full rollout, v2 must run on >= 10% of new threads for at least 1 week with no P0/P1 incidents. This is a process gate, not a code test. Document the canary checklist in the rollout runbook. |

**Blocking:** 6.1, 6.2, 6.3 are hard blockers. 6.4 is a soft blocker. 6.5 is a process requirement.

---

## Non-Goals for Phase 6

These are explicitly out of scope. Do not implement them as part of Phase 6 acceptance.

- **Full reflection pipeline** (Phase 7) -- thread post-mortem analysis and lesson extraction
- **WASM/Docker thread isolation** (Phase 8) -- running threads in sandboxed containers
- **Performance optimization beyond parity** -- v2 should match v1, not beat it (improvements are welcome but not required)
- **Mission system** -- `Mission` type is defined but not wired up
- **Provenance tracking / taint analysis** -- structs exist but enforcement is Phase 7
- **Two-phase commit for Financial effects** -- design is documented in Phase 6 spec, but implementation may defer to Phase 7 if no Financial-effect tools exist yet
- **Dual model routing** -- `LlmBridgeAdapter` should support it structurally but it is not a Phase 6 acceptance criterion

---

## Verification Plan

### Automated Tests (CI-blocking)

```bash
# 1. Engine unit tests (existing)
cargo test -p ironclaw_engine

# 2. Bridge adapter tests (new)
cargo test -p ironclaw -- bridge

# 3. Integration tests with both backends
cargo test --features integration -- engine_v2
cargo test -- engine_v2  # libSQL path

# 4. E2E tests with v2 engine
cd tests/e2e && ENGINE_V2=true pytest

# 5. Behavioral equivalence diff tests
cargo test -- behavioral_equivalence

# 6. Performance benchmarks (CI-reported, not CI-blocking)
cargo bench -p ironclaw --bench step_latency
cargo bench -p ironclaw_engine --bench monty_startup
```

### Manual Review (PR-blocking)

- [ ] Security audit checklist (criterion 3.6) signed off by reviewer
- [ ] Migration documentation (criterion 4.5) exists and reviewed
- [ ] Canary runbook (criterion 6.5) exists

### Test Fixtures Required

| Fixture | Purpose | Location |
|---------|---------|----------|
| `trace_basic_conversation.json` | Multi-turn chat with tool calls | `tests/fixtures/engine_v2/` |
| `trace_approval_flow.json` | Tool requiring approval -> approve -> complete | `tests/fixtures/engine_v2/` |
| `trace_error_handling.json` | Invalid tool, malformed params, timeout | `tests/fixtures/engine_v2/` |
| `trace_compaction.json` | 50-turn conversation triggering compaction | `tests/fixtures/engine_v2/` |
| `trace_codeact.json` | CodeAct/Monty execution with tool dispatch | `tests/fixtures/engine_v2/` |

### Benchmark Harness Requirements

The step latency benchmark must:
1. Use the same mock LLM (fixed response, configurable latency) for both engines
2. Run in the same binary to eliminate process-level variance
3. Report P50/P95/P99 with confidence intervals
4. Run at least 1000 iterations per engine
5. Warm up with 100 iterations before measurement
6. Be added to CI as a reporting job (not a gate) with regression alerts at +15%

### Definition of Done

Phase 6 is complete when:
1. All hard-blocker criteria pass in CI
2. All soft-blocker criteria either pass or have filed issues with mitigation plans
3. Security review checklist is signed off
4. Migration documentation exists
5. Canary runbook exists
6. PR is approved by at least one reviewer who has read this document
