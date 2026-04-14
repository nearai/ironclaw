# Agent Module

Core agent logic. This is the most complex subsystem — read this before working in `src/agent/`.

## Module Map

| File | Role |
|------|------|
| `agent_loop.rs` | `Agent` struct, `AgentDeps`, main `run()` event loop. Delegates to siblings. |
| `dispatcher.rs` | Agentic loop for conversational turns: LLM call → tool execution → repeat. Injects skill context. Returns `Response` or `NeedApproval`. |
| `thread_ops.rs` | Thread/session operations: `process_user_input`, undo/redo, approval, auth-mode interception, DB hydration, compaction. |
| `commands.rs` | System command handlers (`/help`, `/model`, `/status`, `/skills`, etc.) and job intent handlers. |
| `session.rs` | Data model: `Session` → `Thread` → `Turn`. State machines for threads and turns. |
| `session_manager.rs` | Lifecycle: create/lookup sessions, map external thread IDs to internal UUIDs, prune stale sessions, manage undo managers. |
| `router.rs` | Routes explicit `/commands` to `MessageIntent`. Natural language bypasses the router entirely. |
| `scheduler.rs` | Parallel job scheduling. Maintains `jobs` map (full LLM-driven) and `subtasks` map (tool-exec/background). |
| *(moved to `src/worker/job.rs`)* | Per-job execution now lives in `src/worker/job.rs` as `JobDelegate`, using the shared `run_agentic_loop()` engine. |
| `agentic_loop.rs` | Shared agentic loop engine: `run_agentic_loop()`, `LoopDelegate` trait, `LoopOutcome`, `LoopSignal`, `TextAction`. All three execution paths (chat, job, container) delegate to this. |
| `compaction.rs` | Context window management: summarize old turns, write to workspace daily log, trim context. Three strategies. |
| `context_monitor.rs` | Detects memory pressure. Suggests `CompactionStrategy` based on usage level. |
| `self_repair.rs` | Detects stuck jobs and broken tools, attempts recovery. |
| `heartbeat.rs` | Proactive periodic execution. Reads `HEARTBEAT.md`, notifies via channel if findings. |
| `submission.rs` | Parses all user submissions into typed variants before routing. |
| `undo.rs` | Turn-based undo/redo with checkpoints. Checkpoints store message lists (max 20 by default). |
| `routine.rs` | `Routine` types: `Trigger` (cron/event/system_event/manual) + `RoutineAction` (lightweight/full_job) + `RoutineGuardrails`. |
| `routine_engine.rs` | Cron ticker and event matcher. Fires routines when triggers match. Lightweight runs inline; full_job dispatches to `Scheduler`. |
| `task.rs` | Task types for the scheduler: `Job`, `ToolExec`, `Background`. Used by `spawn_subtask` and `spawn_batch`. |
| `cost_guard.rs` | LLM spend and action-rate enforcement. Tracks daily budget (cents) and hourly call rate. Lives in `AgentDeps`. |
| `job_monitor.rs` | Subscribes to SSE broadcast and injects Claude Code (container) output back into the agent loop as `IncomingMessage`. |

## Session / Thread / Turn Model

```
Session (per user)
└── Thread (per conversation — can have many)
    └── Turn (per request/response pair)
        ├── user_input: String
        ├── response: Option<String>
        ├── tool_calls: Vec<ToolCall>
        └── state: TurnState (Pending | Running | Complete | Failed)
```

- A session has one **active thread** at a time; threads can be switched.
- Turns are append-only. Undo rolls back by restoring a prior checkpoint (message list, not a full thread snapshot).
- `UndoManager` is per-thread, stored in `SessionManager`, not on `Session` itself. Max 20 checkpoints (oldest dropped when exceeded).
- Group chat detection: if `metadata.chat_type` is `group`/`channel`/`supergroup`, `MEMORY.md` is excluded from the system prompt to prevent leaking personal context.
- **Auth mode**: if a thread has `pending_auth` set (e.g. from `tool_auth` returning `awaiting_token`), the next user message is intercepted before any turn creation, logging, or safety validation and sent directly to the credential store. Any control submission (undo, interrupt, etc.) cancels auth mode.
- `ThreadState` values: `Idle`, `Processing`, `AwaitingApproval`, `Completed`, `Interrupted`.
- `SessionManager` maps `(user_id, channel, external_thread_id)` → internal UUID. Prunes idle sessions every 10 minutes (warns at 1000 sessions).

## Agentic Loop (dispatcher.rs)

All three execution paths (chat, job, container) now use the shared `run_agentic_loop()` engine in `agentic_loop.rs`, each providing their own `LoopDelegate` implementation:

- **`ChatDelegate`** (`dispatcher.rs`) — conversational turns, tool approval, skill context injection
- **`JobDelegate`** (`src/worker/job.rs`) — background scheduler jobs, planning support, completion detection
- **`ContainerDelegate`** (`src/worker/container.rs`) — Docker container worker, sequential tool exec, HTTP event streaming

```
run_agentic_loop(delegate, reasoning, reason_ctx, config)
  1. Check signals (stop/cancel) via delegate.check_signals()
  2. Pre-LLM hook via delegate.before_llm_call()
  3. LLM call via delegate.call_llm()
  4. If text response → delegate.handle_text_response() → Continue or Return
  5. If tool calls → delegate.execute_tool_calls() → Continue or Return
  6. Post-iteration hook via delegate.after_iteration()
  7. Repeat until LoopOutcome returned or max_iterations reached
```

**Tool approval:** Tools flagged `requires_approval` pause the loop — `ChatDelegate` returns `LoopOutcome::NeedApproval(pending)`. The web gateway stores the `PendingApproval` in session state and sends an `approval_needed` SSE event. The user's approval/deny resumes the loop.

**Shared tool execution:** `tools/execute.rs` provides `execute_tool_with_safety()` (validate → timeout → execute → serialize) and `process_tool_result()` (sanitize → wrap → ChatMessage), used by all three delegates.

**ChatDelegate vs JobDelegate:** `ChatDelegate` runs for user-initiated conversational turns (holds session lock, tracks turns). `JobDelegate` is spawned by the `Scheduler` for background jobs created via `CreateJob` / `/job` — it runs independently of the session and has planning support (`use_planning` flag).

## Command Routing (router.rs)

The `Router` handles explicit `/commands` (prefix `/`). It parses them into `MessageIntent` variants: `CreateJob`, `CheckJobStatus`, `CancelJob`, `ListJobs`, `HelpJob`, `Command`. Natural language messages bypass the router entirely — they go directly to `dispatcher.rs` via `process_user_input`. Note: most user-facing commands (undo, compact, etc.) are handled by `SubmissionParser` before the router runs, so `Router` only sees unrecognized `/xxx` patterns that haven't already been claimed by `submission.rs`.

## Compaction

Triggered by `ContextMonitor` when token usage approaches the model's context limit.

**Token estimation**: Word-count × 1.3 + 4 overhead per message. Default context limit: 100,000 tokens. Compaction threshold: 80% (configurable).

Three strategies, chosen by `ContextMonitor.suggest_compaction()` based on usage ratio:
- **MoveToWorkspace** — Writes full turn transcript to workspace daily log, keeps 10 recent turns. Used when usage is 80–85% (moderate). Falls back to `Truncate(5)` if no workspace.
- **Summarize** (`keep_recent: N`) — LLM generates a summary of old turns, writes it to workspace daily log (`daily/YYYY-MM-DD.md`), removes old turns. Used when usage is 85–95%.
- **Truncate** (`keep_recent: N`) — Removes oldest turns without summarization (fast path). Used when usage >95% (critical).

If the LLM call for summarization fails, the error propagates — turns are **not** truncated on failure.

Manual trigger: user sends `/compact` (parsed by `submission.rs`).

## Scheduler

`Scheduler` maintains two maps under `Arc<RwLock<HashMap>>`:
- `jobs` — full LLM-driven jobs, each with a `Worker` and an `mpsc` channel for `WorkerMessage` (`Start`, `Stop`, `Ping`, `UserMessage`).
- `subtasks` — lightweight `ToolExec` or `Background` tasks spawned via `spawn_subtask()` / `spawn_batch()`.

**Preferred entry point**: `dispatch_job()` — creates context, optionally sets metadata, persists to DB (so FK references from `job_actions`/`llm_calls` are valid immediately), then calls `schedule()`. Don't call `schedule()` directly unless you've already persisted.

Check-insert is done under a single write lock to prevent TOCTOU races. A cleanup task polls every second for job completion and removes the entry from the map.

`spawn_subtask()` returns a `oneshot::Receiver` — callers must await it to get the result. `spawn_batch()` runs all tasks concurrently and returns results in input order.

## Self-Repair

`DefaultSelfRepair` runs on `repair_check_interval` (from `AgentConfig`). It:
1. Calls `ContextManager::find_stuck_jobs()` to find jobs in `JobState::Stuck`.
2. Attempts `ctx.attempt_recovery()` (transitions back to `InProgress`).
3. Returns `ManualRequired` if `repair_attempts >= max_repair_attempts`.
4. Detects broken tools via `store.get_broken_tools(5)` (threshold: 5 failures). Requires `with_store()` to be called; returns empty without a store.
5. Attempts to rebuild broken tools via `SoftwareBuilder`. Requires `with_builder()` to be called; returns `ManualRequired` without a builder.

The `stuck_threshold` duration is used for time-based detection of `InProgress` jobs that have been running longer than the threshold. When `detect_stuck_jobs()` finds such jobs, it transitions them to `Stuck` before returning them, enabling the normal `attempt_recovery()` path.

Repair results: `Success`, `Retry`, `Failed`, `ManualRequired`. `Retry` does NOT notify the user (to avoid spam).

## Key Invariants

- Never call `.unwrap()` or `.expect()` — use `?` with proper error mapping.
- All state mutations on `Session`/`Thread` happen under `Arc<Mutex<Session>>` lock.
- The agent loop spawns concurrent message-handling tasks bounded by a semaphore (`max_parallel_threads`, default 20). Same-user threads are serialized by `Arc<Mutex<Session>>` — parallelism is cross-user, not cross-thread-same-user. Same-thread messages queue via the `ThreadState::Processing` gate + TOCTOU re-check under lock. Response ordering across different threads/users is **not** guaranteed.
- Skills are selected **deterministically** (no LLM call) — see `skills/selector.rs`.
- Tool results pass through `SafetyLayer` before returning to LLM (sanitizer → validator → policy → leak detector).
- `SessionManager` uses double-checked locking for session creation. Read lock first (fast path), then write lock with re-check to prevent duplicate sessions.
- `Scheduler.schedule()` holds the write lock for the entire check-insert sequence — don't hold any other locks when calling it.
- `cheap_llm` in `AgentDeps` is used for heartbeat and other lightweight tasks. Falls back to main `llm` if `None`. Use `agent.cheap_llm()` accessor, not `deps.cheap_llm` directly.
- `CostGuard.check_allowed()` must be called **before** LLM calls; `record_llm_call()` must be called **after**. Both calls are separate — the guard does not auto-record.
- `BeforeInbound` and `BeforeOutbound` hooks run for every user message and agent response respectively. Hooks can modify content or reject. Hook errors are logged but **fail-open** (processing continues).

## Complete Submission Command Reference

All commands parsed by `SubmissionParser::parse()`:

| Input | Variant | Notes |
|-------|---------|-------|
| `/undo` | `Undo` | |
| `/redo` | `Redo` | |
| `/interrupt`, `/stop` | `Interrupt` | |
| `/compact` | `Compact` | |
| `/clear` | `Clear` | |
| `/heartbeat` | `Heartbeat` | |
| `/summarize`, `/summary` | `Summarize` | |
| `/suggest` | `Suggest` | |
| `/expected <desc>` | `Expected` | Fires self-improvement with conversation context |
| `/new`, `/thread new` | `NewThread` | |
| `/thread <uuid>` | `SwitchThread` | Must be valid UUID |
| `/resume <uuid>` | `Resume` | Must be valid UUID |
| `/status [id]`, `/progress [id]`, `/list` | `JobStatus` | `/list` = all jobs |
| `/cancel <id>` | `JobCancel` | |
| `/quit`, `/exit`, `/shutdown` | `Quit` | |
| `yes/y/approve/ok` and aliases | `ApprovalResponse { approved: true, always: false }` | |
| `always/a` and aliases | `ApprovalResponse { approved: true, always: true }` | |
| `no/n/deny/reject/cancel` and aliases | `ApprovalResponse { approved: false }` | |
| JSON `ExecApproval{...}` | `ExecApproval` | From web gateway approval endpoint |
| `/help`, `/?` | `SystemCommand { "help" }` | Bypasses thread-state checks |
| `/version` | `SystemCommand { "version" }` | |
| `/tools` | `SystemCommand { "tools" }` | |
| `/skills [search <q>]` | `SystemCommand { "skills" }` | |
| `/ping` | `SystemCommand { "ping" }` | |
| `/debug` | `SystemCommand { "debug" }` | |
| `/model [name]` | `SystemCommand { "model" }` | |
| Everything else | `UserInput` | Starts a new agentic turn |

**`SystemCommand` vs control**: `SystemCommand` variants bypass thread-state checks entirely (no session lock, no turn creation). `Quit` returns `Ok(None)` from `handle_message` which breaks the main loop.

## Adding a New Submission Command

Submissions are special messages parsed in `submission.rs` before the agentic loop runs. To add a new one:
1. Add a variant to `Submission` enum in `submission.rs`
2. Add parsing in `SubmissionParser::parse()`
3. Handle in `agent_loop.rs` where `SubmissionResult` is matched (the `match submission { ... }` block in `handle_message`)
4. Implement the handler method (usually in `thread_ops.rs` for session operations, or `commands.rs` for system commands)
