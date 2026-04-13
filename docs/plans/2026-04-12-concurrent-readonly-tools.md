# Concurrent Read-Only Tool Execution

**Status:** Proposed
**Priority:** High (performance win on every multi-tool turn)
**Estimated LOC:** 400-600
**Scope:** ChatDelegate only (`src/agent/dispatcher.rs`). JobDelegate and ContainerDelegate are follow-up work. ContainerDelegate intentionally runs tools sequentially for sandbox safety.

## Problem

When the LLM returns multiple tool calls in a single response, IronClaw currently runs them all in parallel without distinguishing read-only from mutating tools. This is both too aggressive (mutating tools shouldn't run concurrently) and not intentionally safe (no classification exists). Claude Code explicitly classifies each tool via `isConcurrencySafe()` and partitions calls into concurrent read-only batches and serial mutating batches.

## Current Behavior

**File:** `src/agent/dispatcher.rs` (lines 707-1050)

```
When LLM returns N tool calls:
  if N == 1:
    Execute single tool sequentially
  if N > 1:
    Spawn all N in JoinSet (tokio) concurrently — no safety classification
```

- All tools share the same `Arc`-cloned state: `JobContext`, `ToolRegistry`, `SafetyLayer`, `ChannelManager`
- No `isConcurrencySafe()` or equivalent on the `Tool` trait (`src/tools/tool.rs`)
- `RiskLevel` enum (`Low`, `Medium`, `High`) exists but indicates approval risk, not concurrency safety
- Rate limiter is per-tool but not checked for concurrent invocations of the same tool

**Tool trait:** `src/tools/tool.rs` (lines 321-495)
- Methods: `name()`, `description()`, `parameters_schema()`, `execute()`, `requires_approval()`, `risk_level_for()`, `rate_limit_config()`
- No concurrency classification method

## Gaps

| Gap | Impact |
|-----|--------|
| No read-only/mutating classification | Mutating tools (shell, write_file, memory_write) can race with each other |
| All-or-nothing parallelism | Either 1 tool serial or all tools parallel; no smart batching |
| Race conditions possible | Two concurrent write_file calls to same path = undefined behavior |
| No batch partitioning | Claude Code preserves tool call order within serial batches; IronClaw doesn't |
| Rate limiter not concurrent-aware | Concurrent calls to same rate-limited tool may exceed per-minute quota |

## Tool Classification

Based on codebase analysis, proposed classifications. Categories use "Concurrent-Safe" rather than "Read-Only" because some tools (e.g., `image_analyze`) make external API calls but don't mutate local state.

### Concurrent-Safe (can run in parallel)

| Tool | Rationale |
|------|-----------|
| `echo` | Pure function, no side effects |
| `time` | Stateless system call |
| `json` | Pure transformation |
| `glob` | Read-only filesystem scan |
| `grep` | Read-only content search |
| `memory_search` | Read-only workspace query |
| `memory_read` | Read-only workspace query |
| `memory_tree` | Read-only workspace query |
| `tool_info` | Read-only registry query |
| `system_version` | Read-only system info |
| `system_tools_list` | Read-only tool listing |
| `list_jobs` / `job_status` | Read-only scheduler query |
| `job_events` | Read-only event stream |
| `skill_list` / `skill_search` | Read-only registry query |
| `read_file` / `list_dir` | Read-only filesystem access |
| `image_analyze` | External API, no local state mutation |
| `routine_list` / `routine_history` | Read-only routine queries |
| `secret_list` | Read-only secrets listing |
| `tool_list` / `tool_search` | Read-only extension listing |
| `extension_info` | Read-only extension detail |

### Mutating (Serial Only)

| Tool | Rationale |
|------|-----------|
| `shell` | Arbitrary commands, can mutate filesystem/processes |
| `write_file` | Filesystem mutation |
| `apply_patch` | Filesystem mutation (rate-limited 20/min) |
| `memory_write` | Workspace state mutation |
| `create_job` | Spawns new agent job |
| `cancel_job` | Job state mutation |
| `job_prompt` | Sends prompt to running job |
| `routine_create` | Routine DB insert |
| `routine_update` | Routine DB update |
| `routine_delete` | Routine DB delete |
| `routine_fire` | Triggers routine execution, may spawn job |
| `event_emit` | Emits system event, triggers routines |
| `secret_delete` | Persistent state mutation |
| `tool_install` | Extension registry mutation |
| `tool_upgrade` | Extension upgrade |
| `tool_remove` | Extension deletion |
| `tool_activate` | Extension state change |
| `tool_permission_set` | Permission mutation |
| `tool_auth` | Initiates OAuth flow |
| `skill_install` / `skill_remove` | Skill registry mutation |
| `file_undo` | Filesystem rollback |
| `build_software` | Creates artifacts, mutates filesystem |
| `plan_update` | Workspace file mutation |
| `restart` | Restarts agent system |
| `message` | Always sends to channels, always has side effects |
| `image_edit` | External API, generates new content |
| `image_generate` | External API, generates new content |

### Parameter-Dependent

| Tool | Concurrent-Safe When | Mutating When |
|------|---------------------|---------------|
| `http` | GET requests (default) | POST/PUT/DELETE requests |

## Proposed Approach

### Phase 1: Add `is_concurrent_safe()` to Tool Trait

```rust
// src/tools/tool.rs
pub trait Tool: Send + Sync {
    // ... existing methods ...

    /// Whether this tool is safe to execute concurrently with other
    /// concurrent-safe tools. Default: false (conservative).
    fn is_concurrent_safe(&self, params: &serde_json::Value) -> bool {
        false
    }
}
```

- Default `false` for safety (new tools must opt in)
- Takes `params` for parameter-dependent tools (e.g., `http` GET vs POST)
- Built-in read-only tools override to return `true`
- WASM tools default to `false` (unknown side effects)
- MCP tools default to `false` (unknown side effects)

### Phase 2: Batch Partitioning in Dispatcher

Replace the current all-or-nothing parallel logic in `execute_tool_calls()` with partitioning.

**Important:** The batch partitioner only sees tools that passed preflight (Phase 1 in dispatcher.rs, lines 817-942). Preflight `break`s at the first approval-required tool (line 935), so tools after an unapproved tool never enter `runnable`. Batching operates exclusively on already-approved tools.

```
Given tool calls [A, B, C, D, E] (all approved):
  A=safe, B=safe, C=mutating, D=safe, E=safe

Partition into batches:
  Batch 1: [A, B] — concurrent (both concurrent-safe)
  Batch 2: [C]    — serial (mutating)
  Batch 3: [D, E] — concurrent (both concurrent-safe)

Execute: Batch 1 (parallel) -> Batch 2 (serial) -> Batch 3 (parallel)
```

**Execution functions per batch type:**
- Concurrent-safe batches: use `execute_chat_tool_standalone()` in JoinSet (same as current parallel path, line 1014)
- Serial mutating tools: use `self.agent.execute_chat_tool()` one at a time (same as current sequential path, line 964)

### Phase 3: Concurrency Limit

Add `max_concurrent_tools: Option<usize>` to `AgentConfig` in `src/config/agent.rs`. Env var: `AGENT_MAX_CONCURRENT_TOOLS`. Default: 10 (matching Claude Code). Scope: per-turn (limits concurrent tools within a single LLM response).

```rust
let max_concurrent = config.max_concurrent_tools.unwrap_or(10);
```

### Phase 4: Rate Limiter Optimization

Skip the rate limiter check entirely for tools where `rate_limit_config()` returns `None`. The rate limiter (`src/tools/rate_limiter.rs:148`) acquires a write lock for every check, which serializes all concurrent calls through one bottleneck. Most concurrent-safe tools have no rate limit, so skipping prevents the lock from negating the concurrency win.

## Migration Risks

| Risk | Severity | Mitigation |
|------|----------|------------|
| **Race conditions on shared state** | High | `JobContext` is cloned per-task; `tool_output_stash` writes are deferred to Phase 3 (sequential). Verify no mutable state is shared between concurrent tools during Phase 2. Confirmed: tools, safety, channels are all Arc-wrapped immutable references |
| **Rate limiter contention** | Medium | Rate limiter uses a write lock (`src/tools/rate_limiter.rs:148`) that serializes all concurrent calls. Mitigate by skipping rate limiter for tools with `rate_limit_config() -> None` (Phase 4). For rate-limited tools, the lock ensures correctness but prevents parallelism |
| **SafetyLayer thread safety** | Low | SafetyLayer is stateless (no Mutex/RwLock in `crates/ironclaw_safety/`). All components (Sanitizer, Validator, LeakDetector, Policy) are immutable after initialization. Confirmed safe for unlimited concurrent calls |
| **Tool output ordering** | Low | Concurrent batch results may arrive in any order. Must maintain tool_call_id mapping so LLM sees results in request order. Current `pf_idx` vector indexing already handles this correctly |
| **WASM tool isolation** | Low | Each WASM execution creates a fresh `Store` instance (`src/tools/wasm/wrapper.rs:128-167`) with independent `WasiCtx`, `ResourceTable`, and HTTP runtime. No shared mutable state. Default `false` is conservative; WASM tools could opt in via a `concurrent_safe` capabilities manifest field in the future |
| **Incorrect classification** | Medium | A tool classified as concurrent-safe but with hidden side effects. Mitigate with conservative `default false` + comprehensive audit of all 54+ tools |
| **Behavioral change** | Low | Current code already runs multiple tools in parallel; this makes it safer, not more aggressive |

## Key Files to Modify

- `src/tools/tool.rs` — Add `is_concurrent_safe()` to `Tool` trait
- `src/tools/builtin/*.rs` — Implement `is_concurrent_safe()` for each built-in tool (54+ tools across 17 files)
- `src/tools/builtin/http.rs` — Parameter-dependent: inspect `params["method"]`, return `true` for GET
- `src/agent/dispatcher.rs` — Replace all-or-nothing parallel with batch partitioning (ChatDelegate only)
- `src/tools/wasm/wrapper.rs` — Default `is_concurrent_safe() -> false` for WASM tools
- `src/tools/mcp/client.rs` — Default `is_concurrent_safe() -> false` for MCP tools
- `src/config/agent.rs` — Add `max_concurrent_tools: Option<usize>` field
- `src/tools/rate_limiter.rs` — Skip check for tools with `rate_limit_config() -> None`

## Verification

### Unit tests (partitioning logic)
1. Given [safe, safe, mutating, safe] tool calls, verify partition is [[safe, safe], [mutating], [safe]]
2. Given all concurrent-safe tools, verify single concurrent batch
3. Given all mutating tools, verify serial execution (one per batch)
4. Given `http` tool with GET params, verify `is_concurrent_safe() -> true`
5. Given `http` tool with POST params, verify `is_concurrent_safe() -> false`

### Caller-level tests (per `.claude/rules/testing.md` — required because `is_concurrent_safe` gates execution behavior)
6. Drive `execute_tool_calls()` with [safe, safe, mutating, safe] and verify correct batch execution order (e.g., via timing assertions or execution-order tracking)
7. Drive `execute_tool_calls()` with mixed tools and verify tool results map correctly to tool_call_ids regardless of completion order

### Integration tests
8. Execute 5 concurrent `memory_search` calls, verify no race conditions
9. Verify rate limiter is skipped for tools with no rate limit config

### Benchmark
10. Measure wall-clock time for 5 concurrent-safe tools: batched concurrent vs sequential baseline
