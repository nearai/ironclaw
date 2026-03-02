# Raw Idea: Agent Reasoning Summary Exposure

**Date:** 2026-02-21  
**Status:** Pre-spec — requirements gathering

---

## One-liner

Expose safe, human-readable summaries of *what the agent decided and why* for each turn — visible in the REPL (CLI) and queryable via the HTTP gateway — without leaking raw chain-of-thought or sensitive context.

---

## Motivation / Problem Being Solved

IronClaw's agent loop (`dispatcher.rs` / `worker.rs`) reasons through tool selection internally, but the user currently sees only:
- Status updates (`Thinking…`, `● tool_name`, `● result preview`)
- A final response

There is no way to ask "why did the agent call `memory_search` before `shell`?" or "what alternatives did it consider?" after the fact. Developers debugging agents and power users auditing decisions have no introspection surface.

---

## Desired Capability

After each turn the agent should record:
1. **Tool-decision log** — for every tool call: which tool was chosen, why (rationale string already produced by `ToolSelection.reasoning`), what parameters were passed, what outcome was observed.
2. **Turn summary** — a brief (2–4 sentence) natural-language narrative of what happened: goal → steps taken → final outcome.

These should be:
- **Accessible from the CLI** (REPL command like `/reasoning` or auto-shown in debug mode)
- **Queryable over HTTP** (e.g. `GET /api/threads/{thread_id}/turns/{turn_number}/reasoning`)
- **In-memory only** — no DB persistence required; cleared when the process restarts or session expires
- **Safe** — no raw `<thinking>` CoT, no leaked credentials, sanitized through existing `SafetyLayer`

---

## Key Existing Building Blocks

| Artifact | Location | Relevance |
|----------|----------|-----------|
| `Turn` struct | `src/agent/session.rs` | Already has `tool_calls: Vec<TurnToolCall>` per turn |
| `TurnToolCall` | `src/agent/session.rs` | Has `name`, `parameters`, `result`, `error` — **missing rationale field** |
| `ToolSelection.reasoning` | `src/llm/reasoning.rs` | LLM-produced reasoning string; **not stored or forwarded to dispatcher** |
| `ToolCall` (provider level) | `src/llm/provider.rs` | Has `id`, `name`, `arguments` — **no reasoning field**; this is what flows to dispatcher |
| `RespondResult::ToolCalls` | `src/llm/reasoning.rs` | Has `tool_calls: Vec<ToolCall>` and optional `content` (pre-tool text) — content is partial rationale signal |
| `StatusUpdate` enum | `src/channels/channel.rs` | Has `Thinking`, `ToolStarted`, `ToolCompleted`, `ToolResult` variants |
| `send_status` in REPL | `src/channels/repl.rs` | CLI rendering of status; already handles `ToolResult` with preview |
| `/debug` toggle in REPL | `src/channels/repl.rs` | `AtomicBool` toggled by `/debug` command; controls verbose output |
| Web server routes | `src/channels/web/server.rs` | Only `/api/chat/threads` at thread level; no turn-level endpoints yet |
| Session in-memory state | `src/agent/session_manager.rs` | Sessions live in `Arc<Mutex<Session>>`; can extend `TurnToolCall` safely |

### Critical gap: reasoning doesn’t reach `TurnToolCall`

`ToolSelection.reasoning` is produced by the planning layer (`Reasoning::respond_with_tools`) but
`RespondResult::ToolCalls` only carries `Vec<ToolCall>` (id/name/arguments) to the dispatcher.
The reasoning string is discarded before `turn.record_tool_call()` is called.
Any implementation must thread reasoning from `RespondResult::ToolCalls` → dispatcher → `TurnToolCall`.
The `content` field (pre-tool explanatory text from the model) could serve as a per-turn narrative
proxy, but it maps to the whole turn rather than individual tool calls.

---

## Scope Boundaries

**In scope:**
- Storing per-turn reasoning summaries in the existing in-memory `Session`/`Turn` structures
- CLI display (REPL command or debug mode expansion)
- HTTP endpoint returning the stored summaries
- Safety: run summary text through existing `LeakDetector`/`Sanitizer` before storage/display

**Out of scope (for this spec):**
- DB persistence
- Raw chain-of-thought (the `<thinking>` content is already stripped by `clean_response`)
- Streaming reasoning tokens in real time
- Worker / job-based execution (focus: chat dispatcher loop only, initially)

---

## Open Questions

*(See QUESTIONS.md for the numbered list to relay to the user.)*
