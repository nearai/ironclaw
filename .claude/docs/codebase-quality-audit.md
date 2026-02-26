# IronClaw Codebase Quality Audit: Agentic Development Smells

**Date:** 2026-02-23
**Scope:** Full src/ directory (249 .rs files, ~113,886 LOC)

---

## Executive Summary

Across 249 `.rs` files, the audit found **significant evidence of AI-assisted development patterns** -- code that works but lacks the consistency, abstraction discipline, and refactoring follow-through that human-authored projects typically exhibit. The codebase is *functional* but carries substantial technical debt from patterns being generated independently rather than evolved together.

---

## 1. God Functions (49 found)

Functions exceeding 80 lines that do too many things.

### Critical (>150 lines)

| Function | File | Lines | Problem |
|----------|------|-------|---------|
| `main()` | `main.rs:48` | **554** | 8+ concerns: CLI, config, DB, channels, WASM, gateway, agent, shutdown |
| `run()` | `agent/agent_loop.rs:218` | **366** | 5 concerns: channel startup, repair spawning, heartbeat, routines, event loop |
| `builtin_entries()` | `extensions/registry.rs:192` | **211** | Hardcoded registry data -- should be TOML/JSON |
| `step_channels()` | `setup/wizard.rs:1407` | **190** | 6 sub-steps of channel config in one function |
| `activate_wasm_channel()` | `extensions/manager.rs:1662` | **189** | 6 sequential operations that should be separate methods |
| `auth_tool()` | `cli/tool.rs:528` | **171** | Interactive auth with 4+ nested concerns |
| `find_code_regions()` | `llm/reasoning.rs:829` | **167** | 4-5 nesting levels of byte-by-byte parsing |
| `handle_message()` | `agent/agent_loop.rs:585` | **167** | 10+ dispatch branches in one match |
| `resolve()` | `config/llm.rs:180` | **158** | LLM config resolution with nested matches |
| `initiate_login()` | `llm/session.rs:239` | **138** | OAuth flow + browser + token persistence |

### High Priority (100-150 lines)

| Function | File | Lines |
|----------|------|-------|
| `new()` (sanitizer) | `safety/sanitizer.rs:59` | 140 |
| `validate_bytes()` | `tools/builder/validation.rs:143` | 142 |
| `parse()` | `agent/submission.rs:14` | 134 |
| `run()` (claude bridge) | `worker/claude_bridge.rs:208` | 132 |
| `start()` (repl) | `channels/repl.rs:294` | 131 |
| `save_and_summarize()` | `setup/wizard.rs:2002` | 129 |
| `refresh_active_channel()` | `extensions/manager.rs:1856` | 126 |
| `step_extensions()` | `setup/wizard.rs:1599` | 125 |
| `list_models_inner()` | `llm/nearai_chat.rs:268` | 125 |
| `run()` (worker runtime) | `worker/runtime.rs:97` | 120 |
| `status_to_wit()` | `channels/wasm/wrapper.rs:2422` | 117 |
| `stream_event_to_payloads()` | `worker/claude_bridge.rs:527` | 107 |
| `print_boot_screen()` | `boot_screen.rs:33` | 106 |
| `repair_broken_tool()` | `agent/self_repair.rs:207` | 105 |
| `step_database_libsql()` | `setup/wizard.rs:428` | 104 |
| `init_database()` | `app.rs:113` | 104 |
| `call_on_start()` | `channels/wasm/wrapper.rs:880` | 101 |
| `bind_telegram_owner()` | `setup/channels.rs:236` | 100 |

### Summary by Category

| Category | Count | Total Lines | Issue |
|----------|-------|-------------|-------|
| Initialization/Setup | 15 | 1200+ | Should use builder pattern, extract helpers |
| Parsing/Dispatch | 6 | 600+ | Should use state machines or structured parsers |
| Data Definitions | 4 | 500+ | Should move to external config/TOML/JSON |
| Complex Logic | 12 | 900+ | Should extract helper methods |
| Terminal UI | 5 | 400+ | Should use UI framework or templates |
| Sequential Operations | 7 | 550+ | Should extract private helper methods |

### Top Refactoring Priorities

1. **`main.rs:48-601`** (554 lines) -- Extract channel/gateway/orchestrator setup into separate functions
2. **`agent_loop.rs:218-583`** (366 lines) -- Extract repair/heartbeat/routine spawning
3. **`extensions/registry.rs:192-402`** (211 lines) -- Move bundled registry to TOML manifest
4. **`extensions/manager.rs:1662-1850`** (189 lines) -- Extract into 4-5 private helpers
5. **`setup/wizard.rs:1407-1596`** (190 lines) -- Extract channel sub-steps

---

## 2. Near-Duplicate Code (7 major patterns)

### Critical Duplication

| Pattern | Occurrences | Severity | Fix |
|---------|-------------|----------|-----|
| `.map_err(\|e\| DatabaseError::Query(e.to_string()))?` | **35+** across all libsql files | HIGH | One macro: `macro_rules! db_err` |
| `params.get("x").and_then(\|v\| v.as_str()).unwrap_or("")` | 5+ tool files | MEDIUM | Add `optional_str()` helper |
| WASM `StoreData` structs (channels vs tools) | 2 near-identical structs | MEDIUM | Generic base struct |
| LLM provider creation functions | 5 near-identical in `llm/mod.rs` | MEDIUM | Generic provider builder |
| SQLite PRAGMA `busy_timeout = 5000` | 3 files with identical value | MEDIUM | Shared constant |
| `WorkspaceError::SearchFailed { reason: format!(...) }` | 10+ in workspace/ | MEDIUM | Context helper |
| `serde_json::json!({ "type": "object", ... })` schemas | **36** tool implementations | LOW | Builder or macro |

### Database Error Mapping (Most Pervasive)

Appears 35+ times identically across `src/db/libsql/conversations.rs`, `jobs.rs`, `sandbox.rs`, `tool_failures.rs`, `workspace.rs`:
```rust
.map_err(|e| DatabaseError::Query(e.to_string()))?;
```

**Fix:**
```rust
macro_rules! db_err {
    ($e:expr) => { DatabaseError::Query($e.to_string()) }
}
```

### WASM StoreData Duplication

`src/channels/wasm/wrapper.rs:72-85` and `src/tools/wasm/wrapper.rs:88-103` define near-identical structs. Only the `host_state` type differs.

### LLM Provider Creation

Five functions in `src/llm/mod.rs` (`create_openai_provider`, `create_anthropic_provider`, `create_ollama_provider`, `create_tinfoil_provider`, etc.) follow the same pattern: get config, build client, map errors, wrap in RigAdapter.

---

## 3. Magic Numbers (~110 instances)

### Most Problematic

| Category | Examples | Count | Impact |
|----------|----------|-------|--------|
| **Token limits** | `1024`, `2048`, `4096` in reasoning.rs | 5 unnamed | Silent drift if you update one |
| **Timeouts** | `30`, `10`, `15` secs in 4+ files | 8 unnamed | Inconsistent behavior |
| **SQLite busy_timeout** | `5000` ms in 3 files | 3 duplicate | Copy-paste constant |
| **Scoring weights** | `100`, `50`, `10`, `5` in registry/selector | 6 unnamed | Tuning requires finding literals |
| **Compaction thresholds** | `0.95`, `0.85`, `0.8` | 3 unnamed | Critical params buried in code |
| **Buffer sizes** | `1048576`, `10485760` raw bytes | 2 unnamed | Named constant exists elsewhere! |
| **IP octets** | `100`, `64`, `198`, `127` | 8 unnamed | Security logic with raw numbers |
| **Epoch timestamp** | `1577836800000` (Jan 1 2020 ms) | 1 | Completely opaque |

### Notable: Named Constant Exists But Not Used

`1048576` (1MB) appears as a raw literal in `storage.rs:810` while the same value exists as `DEFAULT_MAX_REQUEST_BYTES` in `capabilities.rs:124`. Classic sign of code generated in separate sessions.

### By File

**`src/llm/reasoning.rs`:**
- Line 313: `2048` (planning max tokens)
- Line 345: `1024` (tool selection max tokens)
- Line 407: `1024` (evaluation max tokens)
- Line 460: `4096` (response generation max tokens)
- Line 527: `4096` (followup generation max tokens)

**`src/agent/context_monitor.rs`:**
- Line 96: `0.95` (critical overage threshold)
- Line 101: `0.85` (high overage threshold)
- Lines 97, 102: `3`, `5` (keep_recent counts)

**`src/tools/wasm/wrapper.rs`:**
- Line 317: `10 * 1024 * 1024` (max request body 10MB)
- Line 338: `10` (connect timeout secs)
- Line 362: `30_000` / `300_000` (timeout range ms)
- Line 790: `15` (request timeout secs)

**`src/tools/wasm/storage.rs`:**
- Line 589: `5000` (SQLite busy_timeout)
- Line 808-813: `60`, `1000`, `30` (RPM, RPH, timeout defaults)
- Line 810: `1048576` (1MB raw)
- Line 811: `10485760` (10MB raw)

---

## 4. God Objects (10 structs with 10+ fields)

| Struct | File | Fields | Methods | Problem |
|--------|------|--------|---------|---------|
| **JobContext** | `context/state.rs:97` | **22** | 2 | Mixes state machine, cost tracking, duration, env vars, LLM tokens |
| **AppComponents** | `app.rs:31` | **21** | 0 | Zero-cohesion bag of all app dependencies |
| **GatewayState** | `web/server.rs:114` | **21** | 4 | Every handler gets 15+ unrelated subsystems |
| **Settings** | `settings.rs:12` | **20** | 1 | All config categories in one struct |
| **WasmChannel** | `wasm/wrapper.rs:571` | **19** | 18 | Relatively cohesive (Channel trait) |
| **BootInfo** | `boot_screen.rs:8` | **19** | 0 | Wizard state accumulator |
| **Config** | `config/mod.rs:60` | **17** | 5 | Top-level config aggregator |
| **NearAiConfig** | `config/llm.rs:129` | **16** | 0 | Conflates credentials + retry + cache + failover + routing |
| **Routine** | `agent/routine.rs:33` | **16** | 3 | Mixes lifecycle and execution config |
| **ExtensionManager** | `extensions/manager.rs:56` | **16** | 11 | Central extension lifecycle |

### Most Problematic

**GatewayState (21 fields):** Every HTTP handler receives all fields. A chat handler gets the extension manager, sandbox job manager, skill catalog, rate limiter, cost guard -- none of which it uses. Violates dependency inversion.

**NearAiConfig (16 fields):** Should be split into `NearAiCredentialConfig`, `RetryPolicy`, `CacheConfig`, `FailoverConfig`, `SmartRoutingConfig`.

### Complex Nested Types

| File | Type | Complexity |
|------|------|------------|
| `session_manager.rs:29` | `RwLock<HashMap<String, Arc<Mutex<Session>>>>` | Double lock |
| `extensions/manager.rs:63` | `RwLock<HashMap<String, Arc<McpClient>>>` | Lock + hash + arc |
| `orchestrator/api.rs:45` | `Arc<Mutex<HashMap<Uuid, VecDeque<PendingPrompt>>>>` | Triple nesting |
| `web/server.rs:40` | `Arc<Mutex<HashMap<Uuid, VecDeque<PendingPrompt>>>>` | Type alias for above |

---

## 5. Inconsistent Patterns Across Modules

### Error Type Style Drift

| File | Style |
|------|-------|
| `error.rs` | Named fields: `{ name: String, reason: String }` |
| `sandbox/error.rs` | Named fields: `{ reason: String }` |
| `tools/wasm/error.rs` | Tuple variants: `(String)` |
| `channels/wasm/error.rs` | Named fields + manual `From` impls |

### Timestamp Library Mixing

| File | Approach |
|------|----------|
| `pairing/store.rs` | `SystemTime::now().duration_since(UNIX_EPOCH)` |
| `tools/wasm/host.rs` | `SystemTime::now().duration_since(UNIX_EPOCH)` |
| `tools/wasm/wrapper.rs` | `chrono::Utc::now()` |
| `tools/rate_limiter.rs` | `std::time::Instant` |

### JSON Serialization Error Handling

| File | Approach |
|------|----------|
| `wasm/wrapper.rs` | `serde_json::to_string(&x).unwrap_or_default()` |
| `builtin/json.rs` | `.map_err(\|e\| ToolError::ExecutionFailed(format!(...)))` |
| `builder/templates.rs` | `.unwrap_or_else(\|e\| format!("error: {}", e))` |

### Config Access

Some modules read env vars directly (`std::env::var("DATABASE_URL")`), others go through the Config struct (`config.database.url`). No consistent boundary.

### Option Handling (within same file)

`src/channels/http.rs` uses four different idioms: `.map()`, `.as_ref().map()`, `if let Some(ref x)`, `if let Some(x) = &y`.

---

## 6. Dead Code / Stubs

| Item | Location | Status |
|------|----------|--------|
| WIT bindgen extraction | `tools/wasm/runtime.rs:268-293` | Returns `"WASM sandboxed tool"` placeholder |
| Self-repair builder | `agent/self_repair.rs:98-114` | `#[allow(dead_code)]` with TODO |
| Self-repair tools field | `agent/self_repair.rs:77` | `#[allow(dead_code)]` -- never used |
| Hot-activation helper | `extensions/manager.rs:922` | `#[allow(dead_code)]` -- upcoming feature |
| Paragraph chunking | `workspace/chunker.rs:119` | `#[allow(dead_code)]` -- alternative strategy |

---

## Recommended Fixes (by impact-to-effort ratio)

### Quick Wins
1. **Add `db_err!()` macro** -- eliminates 35+ duplicate lines
2. **Name token limit constants** -- prevents silent drift
3. **Extract `optional_str/bool/u64()` param helpers** -- dedups tool code
4. **Create shared SQLite PRAGMA constant** -- one value, not three

### Medium Effort
5. **Split `main()` into 4-5 setup functions** -- makes startup readable/testable
6. **Split `NearAiConfig` into 4 sub-configs** -- reduces 16-field god object
7. **Extract `GatewayState` into handler-specific state** -- fixes dependency inversion
8. **Normalize error type style** -- pick one pattern (named fields) and apply everywhere

### Larger Refactors
9. **Move `builtin_entries()` registry to TOML** -- eliminates 211-line data function
10. **Extract LLM provider builder** -- deduplicates 5 creation functions
11. **Generic WASM StoreData** -- share base struct between channels and tools
12. **Standardize timestamp usage** -- chrono for wall clock, Instant for elapsed
