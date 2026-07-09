# MCP Background Jobs — Per-Server Timeouts + Generic MCP→Job Bridge

**Date:** 2026-07-08
**Status:** Approved design, pre-implementation
**Scope:** Two composable changes — (1) per-MCP-server configurable call timeout, and (2) a generic bridge that runs a long-running MCP tool call as a first-class, durable IronClaw job with auto-resume of the originating agent thread.

## Problem

Every MCP tool call (code sandbox, Playwright, TREK, OCR, …) is capped by **three nested timeouts**, all hardcoded:

| Layer | Value | Location |
|-------|-------|----------|
| Transport (per request) | **30s** — fires first | `src/tools/mcp/stdio_transport.rs:126` (stdio), `src/tools/mcp/http_transport.rs:62` (HTTP/reqwest) |
| Tool execution | 60s | `src/tools/tool.rs:422` default; `McpToolWrapper` (`src/tools/mcp/client.rs:905`) does not override it |
| Gateway Responses API | 600s (now env-configurable) | `src/channels/web/responses_api.rs` |

The real first-firing ceiling on any single MCP call is therefore **30s**. `McpServerConfig` has no timeout field and neither transport reads any override, so there is no way to run a longer MCP operation. This blocks any genuinely long sandbox job (a model training run, a large build, a multi-minute data-processing script, a long browser crawl): it dies at 30s with a transport `Timeout`, surfacing to the agent as a failed tool call.

## Goals

1. **Per-server configurable timeout** so a specific server (e.g. `msbsandbox`) can run calls far longer than 30s, while other servers keep the tight, fail-fast default.
2. **Generic MCP→job bridge**: an explicit "run this MCP tool as a background job" capability, usable by *any* MCP server, that:
   - returns immediately with a job handle,
   - runs the work in a first-class IronClaw job (persisted, visible in `/api/jobs`, event-streamed),
   - **auto-resumes** the originating agent thread on completion (injects the result so the agent continues reasoning unattended),
   - degrades honestly across a service restart (job row survives; in-flight work is marked `Stuck`).

## Non-goals (v1)

- **Full in-flight recovery** across a restart (re-attaching to a still-running microVM). Deferred.
- **Auto-escalation** (start sync, transparently convert to a job on timeout). Rejected in favor of explicit async tools.
- **Per-server generated async tool variants** (`msbsandbox_run_python_async`). One generic builtin covers all servers.
- **Routing through the Docker per-project sandbox** machinery — that is a filesystem mount backend, not a job system.
- `/api/jobs/{id}/restart` re-dispatch for MCP-tool jobs. Noted follow-up.

## Design decisions (settled during brainstorming)

| Decision | Choice |
|----------|--------|
| Completion model | **Auto-resume** the originating thread via the existing job-monitor injection path |
| Async trigger | **Explicit async tools** — the LLM deliberately calls a "run as job" tool |
| Abstraction level | **Generic** MCP→job bridge; the sandbox is the first consumer |
| Restart durability | **Job row survives; in-flight → `Stuck`** (retry is manual in v1); completed results durable |
| Bridge implementation | **Approach A** — model the MCP job on the existing Container/ClaudeCode job path (external work + `JobEvent` stream + `spawn_job_monitor` injection), not the agentic LLM `Worker` loop |

## Architecture

### Part 1 — Per-server configurable timeout (foundation)

Both the synchronous path and the background runner need a call to be *able* to exceed 30s, so this is shared groundwork.

- **`McpServerConfig`** (`src/tools/mcp/config.rs`) gains two optional fields:
  - `timeout_secs: Option<u64>` — serde default `None`; when set, clamped to a sane range (≈ 5s..=6h). `None` preserves today's 30s behavior.
  - `allow_background: bool` — serde default `false`; gates which servers may be run as jobs (Part 2).
- **Transports** take the timeout as a constructor/param instead of the hardcoded `Duration::from_secs(30)`:
  - `src/tools/mcp/stdio_transport.rs` (the `stream_transport_send` call at :126)
  - `src/tools/mcp/http_transport.rs` (the reqwest `.timeout(...)` at :62)
  - `src/tools/mcp/factory.rs` reads `server.timeout_secs` (default 30s) and passes it when constructing each transport.
- **`McpToolWrapper`** (`src/tools/mcp/client.rs:896`) carries the resolved timeout as a field (set at construction from config) and returns it from `execution_timeout()`, so the 60s tool-level cap in `src/tools/execute.rs` / `src/tools/dispatch.rs` cannot re-clip a server configured for longer.

This alone removes the 30s sync wall for configured servers.

### Part 2 — Generic MCP→job bridge (modeled on the Container/ClaudeCode job path)

IronClaw already runs external, non-LLM work as first-class jobs: the Claude Code / container jobs execute an external process, stream `JobEvent`s, and use `spawn_job_monitor` (`src/agent/job_monitor.rs:45`) to inject the result back into the thread. An MCP-tool job is the same shape, simpler — one `call_tool` instead of an interactive process.

**New job mode.** Add `JobMode::McpTool` to the job-mode enum (alongside `Worker` / `ClaudeCode` / `Acp`). The MCP payload `{ server, tool, params }` is carried in `JobContext` metadata.

**Trigger — two new generic builtin tools** (`src/tools/builtin/tool_job.rs`, new):

- `tool_job_start { tool: "<prefixed_tool_name>", arguments: {…} }`
  - Resolves the prefixed tool name → `(server_name, mcp_tool_name)`.
  - Verifies the server exists, is active for the user, and has `allow_background: true`. On any failure it returns a synchronous error (no job created).
  - Honors the underlying tool's `requires_approval` gate *before* dispatch (no privilege escalation via backgrounding).
  - Creates an `McpTool` job via the scheduler and **returns `{ job_id, state }` immediately**.
- `tool_job_status { job_id }`
  - Returns the job state, and the (safety-scanned) result if completed. Belt-and-suspenders alongside auto-resume.

One generic builtin (not per-server generated variants) keeps the mechanism universal: any `allow_background` server is instantly usable.

**Dispatch.** `Scheduler::dispatch_mcp_job()` (`src/agent/scheduler.rs`, parallel to `dispatch_job()`):
1. Creates and persists the `JobContext` (via `ContextManager` + `JobStore`) so the row and FK targets exist immediately.
2. Spawns the runner task.
3. Registers `spawn_job_monitor` for completion injection into the originating thread.

**Runner** (`src/worker/mcp_job.rs`, new; mirrors `src/worker/container.rs`):
1. Transition `Pending → InProgress` (persisted).
2. Emit `JobStatus` ("running `<tool>` in background") through the existing `log_event` path → SSE + `job_events` persistence.
3. Call `mcp_client.call_tool(mcp_tool_name, params)` at the server's configured (long) timeout. Because the runner calls the client directly, it uses the transport timeout from Part 1, not the 60s wrapper cap.
4. Run the output through the **safety layer** (sanitize + leak-detect, `<tool_output>` wrap) exactly as `src/tools/execute.rs::process_tool_result` does — the result will reach the LLM via injection, so it must be scanned first.
5. Emit `JobResult(Completed, <scanned output>)` on success, or `JobResult(Failed, <sanitized error>)` on error/timeout; transition state accordingly.

**Auto-resume.** `spawn_job_monitor` subscribes to the job's event stream; on `JobResult` it injects an internal `IncomingMessage` into the originating `thread_id` (via `inject_tx`) summarizing the outcome, so the agent wakes, reads the result, and continues — even if the user has walked away. This is the exact mechanism already used for Claude Code jobs.

## Data flow (happy path)

1. Agent (in a thread) judges a sandbox task to be long → calls `tool_job_start(tool="msbsandbox__run_python", arguments={code})`.
2. Builtin resolves `server=msbsandbox`, checks `allow_background`, creates the `McpTool` job, returns `job_id`. The agent's current turn can end ("started job X — I'll report when it's done").
3. The runner executes in the background; its `JobEvent`s stream to SSE and persist → the job is visible in `/api/jobs` and the chat event stream.
4. On completion, the job monitor injects "Background job X (`run_python`) completed: `<result>`" into the thread → the agent auto-resumes, reads the result, and continues (e.g. summarizes to the user).

## Error handling

- **Synchronous rejection** (no job created): unknown/inactive server, tool not found, or `allow_background` unset → `tool_job_start` returns a descriptive error.
- **Runtime failure:** `call_tool` error or timeout → `JobResult(Failed, <sanitized error>)`, job `Failed`; the injected completion states the failure so the agent may retry. Channel-edge error mapping per `.claude/rules/error-handling.md` — no raw transport errors or filesystem paths leak to the user.
- **Safety:** the runner's output passes through the safety layer before it is persisted or injected (per `.claude/rules/safety-and-sandbox.md` "every ingress scans before LLM"). The injection is never raw tool output.
- **Approval:** if the underlying tool `requires_approval`, the gate is honored before dispatch. (In this deployment the sandbox tools are auto-approved, so it is transparent; the rule is correct generically.)

## Durability

- `JobContext` persists to `agent_jobs`; events to `job_events`. Both PostgreSQL and libSQL backends.
- **Startup reconcile:** on boot, any `McpTool` job still `InProgress` (its runner did not survive the restart) transitions to `Stuck`, visible in `/api/jobs`. Completed results remain durable. In-flight external calls are not resumed — a documented limit. `/api/jobs/{id}/restart` re-dispatch for `McpTool` mode is a follow-up.

## Testing (test through the caller — `.claude/rules/testing.md`)

- **Unit:** `timeout_secs` parse + clamp; transports use the configured timeout; `McpToolWrapper::execution_timeout()` reflects config.
- **Integration** (stub MCP server; both DB backends):
  - Drive `tool_job_start` end-to-end → job created, runs, `JobResult` emitted, state `Completed`, **and** a completion message is injected into the thread — asserted at the builtin + scheduler layer, not just the runner helper.
  - Failure path: stub tool errors → job `Failed`, error sanitized.
  - Restart reconcile: an `InProgress` `McpTool` job transitions to `Stuck` on startup.
- Regression test accompanies each fix (repo commit hook enforces).

## File-by-file touch list

| File | Change |
|------|--------|
| `src/tools/mcp/config.rs` | `+ timeout_secs`, `+ allow_background` |
| `src/tools/mcp/factory.rs` | pass configured timeout to transports |
| `src/tools/mcp/stdio_transport.rs`, `src/tools/mcp/http_transport.rs` | timeout as parameter (drop hardcoded 30s) |
| `src/tools/mcp/client.rs` | `McpToolWrapper` carries timeout; `execution_timeout()` returns it |
| `src/context/state.rs` *or* `src/orchestrator/job_manager.rs` | `JobMode::McpTool` (locate the canonical mode enum during implementation) |
| `src/worker/mcp_job.rs` *(new)* | the runner (mirrors `container.rs`) |
| `src/agent/scheduler.rs` | `dispatch_mcp_job()` |
| `src/tools/builtin/tool_job.rs` *(new)* | `tool_job_start` + `tool_job_status` |
| `src/agent/job_monitor.rs` | reuse; add a route variant if the existing one is Claude-Code-specific |
| `src/app.rs` | wire builtins (mcp client store + scheduler slot), startup reconcile |
| `src/channels/web/features/jobs/`, `src/channels/web/types.rs` | surface a `mode`/`kind` so MCP jobs are distinguishable in `/api/jobs` |
| `src/cli/mcp.rs` | `--timeout-secs` / `--allow-background` flags on `mcp add` / `mcp update` |
| `.env.example`, `src/channels/web/CLAUDE.md` | document the new config + tools |

## Config surface (enabling msbsandbox after build)

```
ironclaw mcp update msbsandbox --timeout-secs 3600 --allow-background
```

(or the DB-settings equivalent). The agent can then background long sandbox jobs; every other server and every quick call stays fast-and-synchronous.

## Open implementation questions (resolve during planning, not blocking)

- Exact home of the canonical `JobMode` enum (`src/context/state.rs` vs `src/orchestrator/job_manager.rs`).
- Whether `spawn_job_monitor` is reusable as-is or needs an `McpTool` route variant.
- Whether the startup reconcile belongs in `app.rs` boot or a scheduler init hook.
