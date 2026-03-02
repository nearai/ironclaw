# Spec: Agent Reasoning Summary Exposure

**Date:** 2026-02-21  
**Status:** Finalized  
**Spec folder:** `docs/specs/2026-02-21-agent-reasoning-summary/`

---

## 1. Problem Statement

IronClaw's agent loop (`dispatcher.rs` / `worker/runtime.rs`) reasons through tool selection internally, but users currently see only status updates (`Thinking…`, `● tool_name`, `● result preview`) and a final response. There is no introspection surface for:

- **Developers debugging agents:** "Why did it call `memory_search` before `shell`?"
- **Power users auditing decisions:** "What rationale guided the tool sequence?"
- **Session replay and postmortem:** "What happened in turn 3?"

The core data (`ToolSelection.reasoning`, `RespondResult::ToolCalls.content`) already exists but is discarded before it reaches `TurnToolCall`. This spec defines how to thread that data through, store it in-memory, expose it via CLI and HTTP, and emit it over SSE — without leaking raw chain-of-thought or sensitive context.

---

## 2. Goals

1. **Store per-turn reasoning data** in the existing in-memory `Session`/`Turn` structures (no DB persistence in v1).
2. **Thread per-tool reasoning from LLM output to `TurnToolCall`**: carry `ToolSelection.reasoning` through the provider `ToolCall` shape and persist one rationale per tool decision; keep `RespondResult::ToolCalls.content` as optional turn-level narrative context only.
3. **CLI**: always show reasoning summary after each turn; add `/reasoning` as a first-class command for querying historical turns.
4. **HTTP**: add reasoning inline to turn-level API responses, keyed by `(session_id, thread_id, turn_number)`.
5. **SSE**: emit reasoning events to the web gateway in v1.
6. **Safety**: pass all reasoning text through the full `SafetyLayer` (sanitizer + leak detector + policy pipeline) before storage and before any output.
7. **Scope both paths**: chat dispatcher loop (`dispatcher.rs`) and worker job loop (`worker/runtime.rs`).
8. **Retain** for full in-memory session lifetime (no TTL, no per-turn rolling window).
9. **Parallel tool grouping**: annotate grouped parallel batches in the summary.
10. **Omit `alternatives` field** in v1 (unreliable from LLM).

---

## 3. Non-Goals

- DB persistence of reasoning summaries (deferred).
- Raw `<thinking>` chain-of-thought exposure (already stripped by `clean_response`).
- Token-level streaming from model internals.
- Any new LLM call to generate narratives (deterministic only — no added latency or cost).
- `ToolSelection.alternatives` field (low-signal / always empty; deferred).
- External UI mock assets or visual mockups (spec uses ASCII-art wireframes only).

---

## 4. Architecture Overview

### 4.1 Existing Data Flow (Current State)

```
User message
    │
    ▼
dispatcher.rs::run_agentic_loop()
    │
    ├─► Reasoning::respond_with_tools()
    │       └─► RespondResult::ToolCalls { tool_calls: Vec<ToolCall>, content: Option<String> }
    │               │
    │               └─ content (pre-tool rationale) ──► DISCARDED ─────────┐
    │               └─ tool_calls ──► dispatcher loop                       │
    │                                    │                                   │  GAP
    │                                    ▼                                   │
    │                             turn.record_tool_call(name, params, rationale, parallel_group)
    │                             [TurnToolCall: name, params, rationale, parallel_group, result, error]
    │                             ◄──────────────────────────────────────────┘
    │
    ▼
thread.complete_turn(response)
```

### 4.2 Target Data Flow (Post-Implementation)

```
User message
    │
    ▼
dispatcher.rs::run_agentic_loop()
    │
    ├─► Reasoning::respond_with_tools()
    │       └─► RespondResult::ToolCalls { tool_calls: Vec<ToolCall { id, name, arguments, reasoning }>, content: Option<String> }
    │               │
    │               ├─ content (optional turn narrative) ──► SafetyLayer::sanitize_tool_output() ──► turn.set_narrative(sanitized_content)
    │               └─ tool_calls ──► dispatcher loop
    │                                    │
    │                                    ├─ reasoning (per tool) ──► SafetyLayer::sanitize_tool_output() ──► fallback_if_empty()
    │                                    ▼
    │                             turn.record_tool_call(name, params, rationale, parallel_group)
    │                             [TurnToolCall: name, params, rationale, parallel_group, result, error]
    │                                    │
    │                                    ▼
    │                             SseManager::broadcast(SseEvent::ReasoningUpdate { ... })
    │
    ▼
thread.complete_turn(response)
    │
    ▼
CLI: print reasoning block with per-tool why (always-on)
HTTP: reasoning inline in TurnInfo response (per-tool rationale required)
SSE: reasoning_update includes per-tool rationale per decision
```

### 4.3 Worker Path (worker/runtime.rs)

The worker's `execution_loop` uses `reasoning.respond_with_tools()` in the same pattern. The worker does not have a `Session`/`Turn` structure; it has its own `ConversationMemory` / `JobAction` log in `context/memory.rs`.

For the worker path, the spec targets:
- Thread per-tool rationale into worker `tool_calls` (same `ToolCall.reasoning` shape as chat path).
- Sanitize each per-tool rationale and emit it in a structured job reasoning event via `post_event`.
- Keep `content` as optional turn-level narrative context for the job event.
- No `TurnToolCall` extension is needed in the worker — reasoning remains attached to the worker `JobEventPayload` stream.

---

## 5. Data Model Changes

### 5.1 `TurnToolCall` Extension (`src/agent/session.rs`)

Add a `rationale` field to carry the model's per-tool reasoning from `ToolSelection.reasoning` (threaded through `ToolCall`).
`RespondResult::ToolCalls.content` remains optional turn-level narrative context and must not be used as a replacement for per-tool rationale.

**Current:**
```rust
pub struct TurnToolCall {
    pub name: String,
    pub parameters: serde_json::Value,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
}
```

**New:**
```rust
pub struct TurnToolCall {
    pub name: String,
    pub parameters: serde_json::Value,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
    /// LLM-supplied per-tool rationale from ToolSelection.reasoning,
    /// sanitized through SafetyLayer before storage.
    /// Required for every tool decision; if upstream rationale is missing
    /// after sanitization, store deterministic fallback text.
    pub rationale: String,
    /// Parallel batch group index (0-based). Tools sharing the same index
    /// were dispatched concurrently in Phase 2 of the dispatcher.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_group: Option<usize>,
}
```

**`record_tool_call` signature change:**
```rust
pub fn record_tool_call(
    &mut self,
    name: impl Into<String>,
    params: serde_json::Value,
    rationale: String,  // NEW (required)
    parallel_group: Option<usize>,  // NEW
)
```

### 5.2 `Turn` Extension (`src/agent/session.rs`)

Add a turn-level narrative field to store the pre-tool explanatory text from the assistant.

**Current `Turn` struct:** `user_input`, `response`, `tool_calls`, `state`, `started_at`, `completed_at`, `error`.

**New fields:**
```rust
pub struct Turn {
    // ... existing fields ...

    /// Turn-level narrative: the optional pre-tool explanatory text from the model
    /// (RespondResult::ToolCalls.content), sanitized through SafetyLayer.
    /// Set once per agentic loop iteration where tools are called.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub narrative: Option<String>,
}
```

**New method:**
```rust
impl Turn {
    pub fn set_narrative(&mut self, narrative: String) {
        self.narrative = Some(narrative);
    }
}
```

### 5.3 Naming Convention

All user-facing keys use `"reasoning"` as the top-level label (per requirement #14):
- CLI command: `/reasoning`
- HTTP field: `"reasoning"` in `TurnInfo`
- SSE event type: `"reasoning_update"`
- Internal struct fields: `narrative` (turn-level), `rationale` (per-tool-call)

---

## 6. Safety / Security Handling

**All reasoning text passes through the full `SafetyLayer` before storage or output.** This is non-negotiable (requirement #6, security standard).

### 6.1 Pipeline

```
Raw text (model output / content field)
    │
    ▼
SafetyLayer::sanitize_tool_output("reasoning", text)
    ├─► Length guard — max output size enforcement
    ├─► LeakDetector — 15+ secret patterns (API keys, tokens, conn strings)
                       Actions: block sentinel or redact mask
    ├─► Policy      — severity-based action (Block / Warn / Review / Sanitize)
    └─► Sanitizer   — injection pattern detection, XML/HTML escaping
    │
    ▼
Sanitized text → stored in Turn.narrative / TurnToolCall.rationale
```

### 6.2 Sanitization Point

Sanitization is performed **at write time** for every reasoning field:
- once for optional turn narrative from `RespondResult::ToolCalls.content`;
- once per tool for rationale from `ToolCall.reasoning`.
Stored values are already sanitized; no additional sanitization is needed at read time (HTTP response, CLI output, SSE emission).

If `SafetyLayer::sanitize_tool_output` returns blocked/truncated sentinel content:
- for `Turn.narrative`, set `Turn.narrative = None` and continue;
- for per-tool rationale, store deterministic fallback text (e.g. `"Tool selected to satisfy the current subtask."`) and continue.
In both cases emit `tracing::warn!`; reasoning capture issues must never block the agent loop.

### 6.3 No Raw CoT Exposure

The `content` field in `RespondResult::ToolCalls` is the optional pre-tool explanatory assistant message, not raw chain-of-thought. Raw `<thinking>` tokens are already stripped by `clean_response()` in the LLM layer before `RespondResult` is constructed. This spec does not change that behavior.

---

## 7. CLI Behavior

### 7.1 Always-On Display

After every turn that involves tool calls, the REPL prints a reasoning block **automatically** — no toggle required (requirement #3). This appears after the final response, before the next prompt.

**ASCII wireframe — always-on turn output:**

```
› search for recent project updates

  ○ Thinking...
  ○ memory_search
  ● memory_search
    ▸ Found 3 documents matching "project updates"
  ○ shell
  ● shell
    ▸ Exit 0 — stdout: 142 chars

The latest updates include: [response text]

  ┄ Reasoning (turn 4)
  ┄ memory_search  "searched memory for 'recent project updates'"
    params: { "query": "recent project updates", "limit": 5 }
    → 3 documents found
  ┄ shell  "ran git log to check recent commits"
    params: { "command": "git log --oneline -10" }
    → success (142 chars)

›
```

**Formatting rules:**
- Reasoning block is separated from the response by a blank line and a dim `┄` prefix.
- Each tool call entry: `  ┄ <tool_name>  "<rationale>"`
- Parameters: `    params: { ... }` (sensitive values redacted, output truncated to 200 chars if long)
- Result: `    → <outcome>` (success / error / N bytes / N items)
- Parallel batches: prefixed with `  ┄ [parallel] ` on the first entry; subsequent tools in the same batch are indented identically with `  ┄   ↳ <tool_name>`.
- The block is shown whenever tool calls occurred. If turn narrative is missing, omit only the `Narrative:` line; tool rows must still show per-tool rationale.
- Colors follow existing REPL scheme: dim gray for the section, cyan for tool names.

### 7.2 `/reasoning` Command

A new first-class REPL command (requirement #10):

| Invocation | Behavior |
|------------|----------|
| `/reasoning` | Print reasoning for the most recent completed turn |
| `/reasoning N` | Print reasoning for turn number N (1-indexed for display) |
| `/reasoning all` | Print reasoning for all turns in the active thread |

**Added to:**
- `SLASH_COMMANDS` constant in `src/channels/repl.rs`
- `print_help()` output under a new "Inspection" section
- The input loop's command dispatch (routed to the agent via `IncomingMessage`, handled in `agent_loop.rs`)

**Example output for `/reasoning 2`:**

```
  ┄ Reasoning — thread <uuid>, turn 2
  Narrative: "I'll first search memory for context, then read the relevant file."

  ┄ memory_search
    rationale: "searched memory for 'authentication design'"
    params:    { "query": "authentication design", "limit": 3 }
    outcome:   2 documents found

  ┄ file_read
    rationale: "reading the auth module to understand current implementation"
    params:    { "path": "src/safety/sanitizer.rs" }
    outcome:   success (4,321 bytes)
```

**Error cases:**
- Turn N not found → `  ✗ No reasoning data for turn N in this thread.`
- No tool calls in turn → `  ─ Turn N had no tool calls.`
- Reasoning not captured (e.g. pre-feature turn) → `  ─ No reasoning recorded for turn N.`

---

## 8. HTTP API

### 8.1 Endpoint Design

Reasoning is exposed **inline on the existing turn-level response** (requirement #4). No new dedicated endpoint is added in v1; the `TurnInfo` DTO is extended with a `reasoning` field.

**Existing:** `GET /api/chat/history?thread_id={thread_id}` → `HistoryResponse { turns: Vec<TurnInfo> }`

**Extended `TurnInfo`:**
```json
{
  "turn_number": 3,
  "user_input": "search for recent project updates",
  "response": "The latest updates include...",
  "state": "completed",
  "started_at": "2026-02-21T10:00:00Z",
  "completed_at": "2026-02-21T10:00:05Z",
  "tool_calls": [
    { "name": "memory_search", "has_result": true, "has_error": false }
  ],
  "reasoning": {
    "session_id": "550e8400-e29b-41d4-a716-446655440000",
    "thread_id": "7c9e6679-7425-40de-944b-e07fc1f90ae7",
    "turn_number": 3,
    "narrative": "I'll search memory for context before responding.",
    "tool_decisions": [
      {
        "tool_name": "memory_search",
        "rationale": "searched memory for 'recent project updates'",
        "parameters": { "query": "recent project updates", "limit": 5 },
        "outcome": "success",
        "parallel_group": null
      }
    ]
  }
}
```

`reasoning` is `null` (or omitted via `#[serde(skip_serializing_if = "Option::is_none")]`) when:
- No tool calls occurred in the turn.
- The feature was not active when the turn was recorded.
- Safety layer blocked the content.

### 8.2 Identity Key

All reasoning objects are keyed by `(session_id, thread_id, turn_number)` (requirement #9). The `session_id` is included in the payload for unambiguous addressing even when the response is consumed out of context.

### 8.3 New DTO (`src/channels/web/types.rs`)

```rust
#[derive(Debug, Serialize)]
pub struct ToolDecisionInfo {
    pub tool_name: String,
    pub rationale: String,
    pub parameters: serde_json::Value,
    pub outcome: String,  // "success" | "error" | "rejected"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parallel_group: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct TurnReasoningInfo {
    pub session_id: Uuid,
    pub thread_id: Uuid,
    pub turn_number: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub narrative: Option<String>,
    pub tool_decisions: Vec<ToolDecisionInfo>,
}
```

`TurnInfo` gains:
```rust
#[serde(skip_serializing_if = "Option::is_none")]
pub reasoning: Option<TurnReasoningInfo>,
```

### 8.4 Payload Example — Full Response

```http
GET /api/chat/history?thread_id=7c9e6679-7425-40de-944b-e07fc1f90ae7
Authorization: Bearer <token>

HTTP/1.1 200 OK
Content-Type: application/json

{
  "thread_id": "7c9e6679-7425-40de-944b-e07fc1f90ae7",
  "turns": [
    {
      "turn_number": 1,
      "user_input": "search for recent project updates",
      "response": "The latest updates include...",
      "state": "completed",
      "started_at": "2026-02-21T10:00:00Z",
      "completed_at": "2026-02-21T10:00:05Z",
      "tool_calls": [
        { "name": "memory_search", "has_result": true, "has_error": false },
        { "name": "shell", "has_result": true, "has_error": false }
      ],
      "reasoning": {
        "session_id": "550e8400-e29b-41d4-a716-446655440000",
        "thread_id": "7c9e6679-7425-40de-944b-e07fc1f90ae7",
        "turn_number": 1,
        "narrative": "I'll search memory first, then check git log for recent changes.",
        "tool_decisions": [
          {
            "tool_name": "memory_search",
            "rationale": "searched memory for 'recent project updates'",
            "parameters": { "query": "recent project updates", "limit": 5 },
            "outcome": "success",
            "parallel_group": null
          },
          {
            "tool_name": "shell",
            "rationale": "checked git log for recent commits",
            "parameters": { "command": "git log --oneline -10" },
            "outcome": "success",
            "parallel_group": null
          }
        ]
      }
    }
  ],
  "has_more": false
}
```

### 8.5 Parallel Tool Example

When tools are dispatched in parallel (Phase 2 of the dispatcher), `parallel_group` is set to the same non-null integer for all tools in the batch:

```json
"tool_decisions": [
  {
    "tool_name": "memory_search",
    "rationale": "Searched memory for relevant authentication context.",
    "parameters": { "query": "auth design" },
    "outcome": "success",
    "parallel_group": 0
  },
  {
    "tool_name": "file_read",
    "rationale": "Read the sanitizer source to verify current behavior.",
    "parameters": { "path": "src/safety/sanitizer.rs" },
    "outcome": "success",
    "parallel_group": 0
  }
]
```

---

## 9. SSE Event Design

### 9.1 New SSE Event Variant

Add `ReasoningUpdate` to `SseEvent` in `src/channels/web/types.rs`:

```rust
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum SseEvent {
    // ... existing variants ...

    #[serde(rename = "reasoning_update")]
    ReasoningUpdate {
        thread_id: String,
        session_id: String,
        turn_number: usize,
        /// Turn-level narrative (pre-tool content from model), if available.
        #[serde(skip_serializing_if = "Option::is_none")]
        narrative: Option<String>,
        /// Per-tool decisions captured so far in this turn.
        tool_decisions: Vec<ToolDecisionSsePayload>,
    },
}

/// Minimal SSE payload for a single tool decision (avoids large parameter dumps over SSE).
#[derive(Debug, Clone, Serialize)]
pub struct ToolDecisionSsePayload {
    pub tool_name: String,
    pub rationale: String,
    pub outcome: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parallel_group: Option<usize>,
}
```

**Note:** SSE payloads omit `parameters` to avoid large JSON blobs over the event stream. Parameters are available on the HTTP history endpoint with sensitive values redacted.

### 9.2 Emission Timing

`ReasoningUpdate` is emitted **once per agentic loop iteration** that involves tool calls, after all tools in that iteration have completed (Phase 3 of the dispatcher). It carries the cumulative tool decisions recorded so far in the current turn (including all prior iterations).

This means for a turn with two LLM iterations (tool call → tool call → text response), `ReasoningUpdate` is emitted twice:
- After iteration 1: contains tool decisions from iteration 1.
- After iteration 2: contains all tool decisions accumulated in the turn.

### 9.3 SSE Wire Format Example

```
event: reasoning_update
data: {"type":"reasoning_update","thread_id":"7c9e...","session_id":"550e...","turn_number":3,"narrative":"I'll search memory first.","tool_decisions":[{"tool_name":"memory_search","rationale":"searched memory for project updates","outcome":"success","parallel_group":null}]}

```

### 9.4 Worker Path SSE

The worker path does not have `SseManager` access directly; it uses `post_event()` to the orchestrator. The worker emits a new event type `"reasoning"` alongside existing `"tool_use"` / `"tool_result"` events:

```json
{
  "event": "reasoning",
  "job_id": "<uuid>",
  "narrative": "Checked git log, now writing a summary file.",
  "tool_decisions": [
    {
      "tool_name": "shell",
      "rationale": "ran git log to capture recent commits",
      "outcome": "success"
    }
  ]
}
```

The orchestrator now maps worker `"reasoning"` events to a dedicated `SseEvent::JobReasoning`, and the web frontend listens to `job_reasoning` for activity-stream updates.

---

## 10. Parallel Tool Grouping Behavior

The dispatcher uses a three-phase pattern (preflight → parallel JoinSet → post-flight). Parallel grouping for reasoning is tracked as follows:

### 10.1 Group Assignment

During preflight, a `batch_id: Option<usize>` is assigned per tool call:
- Single-tool iterations: `parallel_group = None`.
- Multi-tool iterations where `runnable.len() > 1`: all tools in `runnable` share `parallel_group = Some(batch_counter)`, where `batch_counter` is a monotonically increasing per-turn counter.

The `batch_counter` increments each time Phase 2 runs with `>1` concurrent tools. This supports turns with multiple agentic iterations, each potentially spawning a parallel batch.

### 10.2 CLI Parallel Display

```
  ┄ [parallel batch 0]
  ┄   ↳ memory_search  "searching memory for auth design"
       params: { "query": "auth design" }
       → 2 documents
  ┄   ↳ file_read  "reading sanitizer source"
       params: { "path": "src/safety/sanitizer.rs" }
       → success (4,321 bytes)
```

### 10.3 HTTP/SSE Parallel Display

`parallel_group` integer on each `ToolDecisionInfo` / `ToolDecisionSsePayload` lets clients group visually. No special grouping structure in the JSON — clients apply grouping by matching `parallel_group` values.

---

## 11. Dispatcher Integration Points

### 11.1 Chat Path (`src/agent/dispatcher.rs::run_agentic_loop`)

**Change 1 — Capture optional turn narrative:**  
After receiving `RespondResult::ToolCalls { tool_calls, content }`, sanitize and store `content` into `Turn.narrative` when present.

**Change 2 — Thread per-tool rationale:**  
Extend provider-level `ToolCall` to include `reasoning: String` and thread `ToolSelection.reasoning` into that field inside `Reasoning::respond_with_tools()`, applying deterministic fallback if the upstream value is empty.

**Change 3 — Assign parallel groups:**  
Before Phase 2, compute `parallel_group` for the current `runnable` slice:
- `if runnable.len() <= 1: None`
- `else: Some(batch_counter); batch_counter += 1;`

**Change 4 — Record required rationale on `TurnToolCall`:**  
`record_tool_call` takes required `rationale: String`.
For each tool call, sanitize `tool_call.reasoning`; if missing/blocked after sanitization, store deterministic fallback rationale text. Then persist via `record_tool_call(name, params, rationale, parallel_group)`.

**Change 5 — Emit `ReasoningUpdate` SSE after Phase 3:**  
After all post-flight results are recorded, collect the current turn's `tool_decisions` from `TurnToolCall` entries and broadcast `SseEvent::ReasoningUpdate`.

### 11.2 Worker Path (`src/worker/runtime.rs::execution_loop`)

After `RespondResult::ToolCalls { content, tool_calls }`:
- sanitize optional `content` into `narrative`;
- for each tool call, sanitize `tool_call.reasoning` and apply deterministic fallback rationale if missing/blocked;
- call `post_event("reasoning", { narrative, tool_decisions })` with tool name, rationale, and outcome.
The existing `tool_use` and `tool_result` events continue unchanged.

---

## 12. Retention Model

**Policy: in-memory for full session lifetime** (requirement #8).

- Reasoning data lives in `TurnToolCall.rationale`, `TurnToolCall.parallel_group`, and `Turn.narrative` — all part of the existing `Session` heap allocation.
- Sessions live in `Arc<Mutex<Session>>` inside `SessionManager`. They are dropped when the session is evicted or the process restarts.
- No separate data structure or cache is allocated for reasoning.
- Memory growth: each turn adds at most one optional narrative and one required rationale string per tool call. For a long session with 200 turns × 5 tool calls each × 500 bytes average rationale = ~500 KB worst-case; acceptable.
- No configurable TTL, rolling window, or per-turn eviction in v1.

---

## 13. Scope: Chat + Worker Paths

| Path | Where | Change |
|------|-------|--------|
| Chat dispatcher | `src/agent/dispatcher.rs` | Thread `content` to optional `Turn.narrative`; thread per-tool `ToolCall.reasoning`, sanitize+fallback, pass required `rationale` + `parallel_group` to `record_tool_call`; emit `ReasoningUpdate` SSE |
| Worker runtime | `src/worker/runtime.rs` | Sanitize optional `content`; sanitize+fallback per-tool rationale from `ToolCall.reasoning`; emit `"reasoning"` job event via `post_event` |
| Session model | `src/agent/session.rs` | Add `narrative` to `Turn`; make `rationale` required on `TurnToolCall`; keep `parallel_group`; update `record_tool_call` signature |
| REPL channel | `src/channels/repl.rs` | Always-on reasoning block after each turn; `/reasoning [N|all]` command |
| Web types | `src/channels/web/types.rs` | Add `TurnReasoningInfo`, `ToolDecisionInfo`, `ToolDecisionSsePayload`; extend `TurnInfo`; add `SseEvent::ReasoningUpdate` |
| Web server | `src/channels/web/server.rs` | Populate `reasoning` field when building `TurnInfo` from `Turn` |
| Safety | `src/safety/` | No changes needed — existing `SafetyLayer::sanitize_tool_output()` API is sufficient |

---

## 14. API Examples

All examples below use user-facing 1-based `turn_number` values.

### 14.1 History with Reasoning — Single Tool Turn

```http
GET /api/chat/history?thread_id={thread_id}
```

```json
{
  "thread_id": "7c9e6679...",
  "turns": [
    {
      "turn_number": 1,
      "user_input": "what time is it?",
      "response": "It's 10:35 AM UTC.",
      "state": "completed",
      "started_at": "2026-02-21T10:35:00Z",
      "completed_at": "2026-02-21T10:35:01Z",
      "tool_calls": [
        { "name": "time", "has_result": true, "has_error": false }
      ],
      "reasoning": {
        "session_id": "550e8400...",
        "thread_id": "7c9e6679...",
        "turn_number": 1,
        "narrative": "The user wants the current time. I'll call the time tool.",
        "tool_decisions": [
          {
            "tool_name": "time",
            "rationale": "retrieved the current UTC time",
            "parameters": {},
            "outcome": "success",
            "parallel_group": null
          }
        ]
      }
    }
  ],
  "has_more": false
}
```

### 14.2 History with Reasoning — No Tool Calls Turn

```json
{
  "turn_number": 1,
  "user_input": "thanks",
  "response": "You're welcome!",
  "state": "completed",
  "tool_calls": [],
  "reasoning": null
}
```

### 14.3 SSE Reasoning Event

```
data: {"type":"reasoning_update","thread_id":"7c9e...","session_id":"550e...","turn_number":2,"narrative":"Searching memory, then reading the file.","tool_decisions":[{"tool_name":"memory_search","rationale":"looked up prior context","outcome":"success","parallel_group":null},{"tool_name":"file_read","rationale":"read sanitizer implementation","outcome":"success","parallel_group":null}]}
```

### 14.4 `/reasoning` CLI Output

```
› /reasoning 2

  ┄ Reasoning — turn 2
  Narrative: "Searching memory, then reading the file."

  ┄ memory_search
    rationale: "looked up prior context"
    params:    { "query": "authentication", "limit": 5 }
    outcome:   success

  ┄ file_read
    rationale: "read sanitizer implementation"
    params:    { "path": "src/safety/sanitizer.rs" }
    outcome:   success (4,321 bytes)

›
```

---

## 15. Rollout and Verification Plan

### 15.1 Implementation Order

1. **Provider model threading** (`src/llm/provider.rs`, `src/llm/reasoning.rs`): add `reasoning: String` to `ToolCall` and thread `ToolSelection.reasoning` into each emitted tool call with deterministic fallback for empty values.
2. **Data model** (`session.rs`): add `narrative` to `Turn`, make `rationale` required + `parallel_group` on `TurnToolCall`, update `record_tool_call` signature. Update all callers.
3. **Dispatcher** (`dispatcher.rs`): sanitize/store optional `content` into `turn.set_narrative`; sanitize/fallback per-tool rationale from `tool_call.reasoning`; assign `parallel_group`; persist required rationale; emit `ReasoningUpdate` SSE after Phase 3.
4. **Worker** (`worker/runtime.rs`): sanitize optional `content`; sanitize/fallback per-tool rationale; emit `"reasoning"` job event.
5. **HTTP types + server** (`web/types.rs`, `web/server.rs`): make rationale required in DTOs; populate `reasoning` field in `TurnInfo` construction.
6. **SSE variant** (`web/types.rs`): ensure `ToolDecisionSsePayload.rationale` is required.
7. **REPL** (`repl.rs`): always-on display after turn completion; `/reasoning` command parsing and dispatch.
8. **Tests**: unit tests per step (see §15.2).
9. **Standards checks**: run `cargo fmt`, `cargo clippy --all --benches --tests --examples --all-features`, `cargo test`, `cargo check --no-default-features --features libsql`, `cargo check --all-features`.
10. **Feature parity**: update `FEATURE_PARITY.md` — add `Agent Reasoning Summary` row with `✅` status.

### 15.2 Test Plan

| Test | Location | What to verify |
|------|----------|----------------|
| `test_turn_narrative_set_and_get` | `session.rs::tests` | `Turn.set_narrative()` stores and `Turn.narrative` returns the value |
| `test_record_tool_call_with_rationale` | `session.rs::tests` | `record_tool_call` captures required `rationale` and `parallel_group` |
| `test_toolcall_reasoning_threaded_from_selection` | `llm/reasoning.rs::tests` (new) | `ToolSelection.reasoning` is propagated into emitted `ToolCall.reasoning` |
| `test_reasoning_safety_block_narrative` | `dispatcher::tests` (new) | If safety blocks `content`, `Turn.narrative` is `None` and turn continues |
| `test_reasoning_safety_block_rationale_fallback` | `dispatcher::tests` (new) | If safety blocks per-tool rationale, fallback rationale string is stored |
| `test_reasoning_safety_redact_rationale` | `dispatcher::tests` (new) | If safety redacts per-tool rationale, stored rationale is redacted and non-empty |
| `test_parallel_group_assigned` | `dispatcher::tests` (new) | Multi-tool iteration sets non-null `parallel_group` on all runnable tools |
| `test_single_tool_no_parallel_group` | `dispatcher::tests` (new) | Single tool iteration sets `parallel_group = None` |
| `test_turn_info_reasoning_serialization` | `web/types.rs::tests` (new) | `TurnReasoningInfo` serializes correctly with required `rationale` per tool decision |
| `test_repl_reasoning_command_routing` | `repl.rs::tests` (new) | `/reasoning 3` produces an `IncomingMessage` with content `/reasoning 3` |

### 15.3 Verification Checklist

- [ ] `Turn.narrative` is `None` for turns without tool calls.
- [ ] `Turn.narrative` is `None` when `content` is `None` in `RespondResult::ToolCalls`.
- [ ] Every `TurnToolCall` stores non-empty `rationale` (real or deterministic fallback), including parallel batches.
- [ ] `TurnToolCall.parallel_group` is `None` for single-tool iterations.
- [ ] Sanitized reasoning never contains raw API keys, tokens, or private keys.
- [ ] `SseEvent::ReasoningUpdate` is serialized with `"type": "reasoning_update"` and per-tool `rationale` present.
- [ ] HTTP `reasoning` field is absent (`null`/omitted) for turns with no tool calls.
- [ ] CLI reasoning block is present for turns with tool calls and shows per-tool rationale.
- [ ] `/reasoning N` for out-of-range N prints a clear error, does not panic.
- [ ] `cargo check` passes for default, libsql-only, and all-features builds.
- [ ] No `.unwrap()` or `.expect()` added in production paths.
- [ ] No `super::` imports introduced; all new imports use `crate::`.

---

## 16. Risks, Tradeoffs, and Alternatives Considered

### 16.1 Risks

| Risk | Likelihood | Mitigation |
|------|-----------|------------|
| `content` field is `None` for many model responses | Medium | `Turn.narrative` is optional by design; per-tool rationale still remains present via `ToolCall.reasoning` or deterministic fallback |
| Reasoning text echoes sensitive user context | Medium | Full `SafetyLayer` pipeline with `LeakDetector` covers API keys, tokens, connection strings; Block action drops the field |
| Safety pipeline mangling useful phrasing | Low | `Sanitize` action (escape, not delete) preserves most text; only `Block` drops the field entirely |
| Memory growth for very long sessions | Low | ~500 KB worst-case for 200-turn session (see §12) |
| `record_tool_call` signature change breaks callers | High (mechanical) | Grep all call sites before implementing; update all at once per review-discipline standard |

### 16.2 Tradeoffs

**Per-tool rationale (chosen) vs. turn-level-only narrative:**  
Per-tool rationale is now required because the product intent is to show *why each tool was selected*, not just which tools ran. This requires extending provider-level `ToolCall` with `reasoning` and plumbing that field through the chat and worker paths. Blast radius is larger than turn-level-only narrative, but behavior matches user intent.

Turn-level `content` remains useful context, but is explicitly optional and not a substitute for per-tool rationale.

**No extra LLM call:**  
Per-tool rationale still comes from existing deterministic model output (`ToolSelection.reasoning`) with deterministic fallback text when missing/blocked after sanitization. This preserves zero added model latency and zero added token cost.

**Alternatives field omitted:**  
`ToolSelection.alternatives` is always empty in practice (LLM does not reliably populate it). Including it in the payload would add noise with no signal. Deferred to a future iteration when prompt engineering can reliably elicit alternatives.

**Separate `/reasoning` endpoint vs. inline on `TurnInfo`:**  
A dedicated `GET /api/threads/{thread_id}/turns/{n}/reasoning` endpoint would allow fetching reasoning independently. The inline approach reduces API surface, requires no routing changes, and keeps the data collocated with the turn. A dedicated endpoint can be added in v2 if clients need it.

**Full `SafetyLayer` vs. `LeakDetector` only:**  
`LeakDetector` alone would prevent credential leakage but miss injection patterns. The `Sanitizer` and `Policy` layers add protection against prompt injection artifacts (e.g., model reasoning that echoes injected instructions). Full pipeline was chosen per the security standard and requirement #6.

**Worker path: `TurnToolCall` extension vs. job event stream:**  
The worker does not have a `Session`/`Turn` structure. Extending the worker to write to `TurnToolCall` would require plumbing a session handle into the container — a significant coupling. Instead, the worker emits reasoning as a structured job event (matching the existing `JobEventPayload` pattern), which is surfaced via SSE to the UI. This is consistent with how worker tools are already reported (`tool_use`, `tool_result`).

---

## 17. Standards Alignment

| Standard | Alignment |
|----------|-----------|
| `adding-features.md` | Feature uses existing trait boundaries (`SafetyLayer`, `Turn`, `SseEvent`); no new crates; minimal blast radius |
| `code-style.md` | `crate::` imports; strong types (`TurnReasoningInfo`, `ToolDecisionInfo`); no `pub use` re-exports |
| `error-handling.md` | `SafetyLayer` handling never panics: narrative may map to `None`; per-tool rationale uses deterministic fallback with warn logs; no `.unwrap()` in production paths |
| `async-patterns.md` | Existing `Arc<Mutex<Session>>` pattern; lock scope kept minimal (take + release before SSE emit) |
| `security.md` | Full `SafetyLayer` pipeline on all reasoning text; no raw credentials in logs or responses |
| `database.md` | No DB persistence in v1; dual-backend rule is not triggered |
| `testing.md` | `mod tests {}` pattern; `#[tokio::test]` for async cases; real `SafetyLayer` in tests (no mocks) |
| `feature-parity.md` | `FEATURE_PARITY.md` updated in same branch when feature ships |
| `review-discipline.md` | `record_tool_call` signature change → grep all callers before committing; verify no `.unwrap()` added |
