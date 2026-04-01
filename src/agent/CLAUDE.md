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
- The agent loop is single-threaded per thread; parallel execution happens at the job/scheduler level.
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
| `/new`, `/thread new` | `NewThread` | |
| `/thread list` | `SystemCommand { "history" }` | Alias for /history |
| `/thread <uuid>` | `SwitchThread` | Must be valid UUID |
| `/resume <uuid>` | `Resume` | Must be valid UUID |
| `/status [id]`, `/progress [id]`, `/list` | `JobStatus` | `/list` = all jobs |
| `/cancel <id>` | `JobCancel` | |
| `/quit`, `/exit`, `/shutdown` | `Quit` | |
| `yes/y/approve/ok` and aliases | `ApprovalResponse { approved: true, always: false }` | |
| `always/a` and aliases | `ApprovalResponse { approved: true, always: true }` | |
| `no/n/deny/reject/cancel` and aliases | `ApprovalResponse { approved: false }` | |
| JSON `ExecApproval{...}` | `ExecApproval` | From web gateway approval endpoint |
|| `/help`, `/?` | `SystemCommand { "help" }` | Bypasses thread-state checks |
|| `/version` | `SystemCommand { "version" }` | |
|| `/tools` | `SystemCommand { "tools" }` | |
|| `/skills [search <q>]` | `SystemCommand { "skills" }` | |
|| `/ping` | `SystemCommand { "ping" }` | |
|| `/debug` | `SystemCommand { "debug" }` | |
|| `/model [name]` | `SystemCommand { "model" }` | |
|| `/reasoning [on|off|<model>]` | `SystemCommand { "reasoning" }` | Configure reasoning mode |
|| `/restart` | `SystemCommand { "restart" }` | Restart agent loop |
|| Everything else | `UserInput` | Starts a new agentic turn |

**`SystemCommand` vs control**: `SystemCommand` variants bypass thread-state checks entirely (no session lock, no turn creation). `Quit` returns `Ok(None)` from `handle_message` which breaks the main loop.

### Thread/History Command Patterns

**List all conversations:**
```bash
/history                    # List persistent (DB) + in-memory threads
/thread list                # Alias for /history
```

**Output format:**
```
Session: <session-uuid>
Active thread: <thread-uuid>

Persistent threads (use /thread <id> to hydrate):
* telegram/private messages=42 updated=2026-04-01T12:00:00Z — Previous Chat [DB]
  web/browser messages=15 updated=2026-04-01T11:30:00Z — Web Session [DB]

Current session threads:
* Idle turns=3 updated=2026-04-01T12:05:00Z — Active Web Thread

Use /thread <id> to switch threads.
```

**Switch to a thread:**
```bash
/thread 11111111-1111-1111-1111-111111111111   # Switch to existing thread
/thread new                                     # Create new thread
```

**Key behaviors:**
- `[DB]` indicator = thread exists in database but not yet hydrated in current session
- `*` prefix = currently active thread
- Persistent threads sorted by `last_activity` (most recent first)
- In-memory threads filtered to exclude already-listed DB threads (no duplicates)
- Use `/thread <uuid>` to hydrate a DB thread into the current session

**Implementation:** `commands.rs::handle_history_command()` — lists from `tenant.store().list_conversations_all_channels(50)` then appends in-memory-only threads.

## Routines System

Routines are named, persistent, user-owned automated tasks that fire independently when their trigger conditions are met. Each routine runs with only its own prompt and context — not the full session history.

### Architecture

```
┌──────────────┐     ┌─────────────┐     ┌────────────────────┐
│   Trigger     │────▶│   Engine    │────▶│  Execution Mode    │
│ cron/event/   │     │  guardrails │     │ lightweight│full_job│
│ system/manual │     │  check      │     └────────────────────┘
└──────────────┘     └─────────────┘              │
                                                  ▼
                                         ┌────────────────┐
                                         │ Notify user    │
                                         │ if configured  │
                                         └────────────────┘
```

**Key files:**
- `routine.rs` — Core types: `Routine`, `Trigger`, `RoutineAction`, `RoutineGuardrails`, `NotifyConfig`
- `routine_engine.rs` — Execution engine with cron ticker and event matcher (2561 lines)

### Trigger Types

| Type | Description | Example |
|------|-------------|---------|
| `Cron` | Fire on cron schedule | `"0 9 * * MON-FRI"`, `"every 2h"` |
| `Event` | Fire when channel message matches regex | pattern: `"daily report"`, channel: `"telegram"` |
| `SystemEvent` | Fire on structured system events | source: `"github"`, event_type: `"issue.opened"` |
| `Webhook` | Fire on POST to `/api/webhooks/{path}` | path: `"deploy-complete"`, secret: HMAC |
| `Manual` | Only via tool call or CLI | — |

### Execution Modes

**Lightweight** (default for simple routines):
- Single LLM call executed inline
- No scheduler slot consumed
- Tool calls allowed but limited
- Best for: notifications, summaries, quick checks

**Full-job**:
- Delegated to `Scheduler` (runs as background job)
- Full agentic loop with multiple turns
- Can use sandbox, long-running operations
- Best for: complex workflows, multi-step tasks

### Guardrails

`RoutineGuardrails` enforces:
- `max_duration`: Maximum execution time (default: 5min for lightweight)
- `max_tool_calls`: Limit tool invocations per run
- `allowed_tools`: Whitelist of tools (subset of `autonomous_allowed_tool_names`)
- `forbidden_tools`: Explicit deny list
- `require_sandbox`: Force sandbox execution (full-job only)

### Notification Config

`NotifyConfig` controls post-execution notifications:
- `on_success`: Notify when routine completes successfully
- `on_failure`: Notify on error/timeout
- `always`: Notify on every run regardless of outcome
- `channel`: Target channel for notifications (defaults to routine's channel)

### Runtime State (DB-managed)

Each routine tracks:
- `last_run_at`: Last successful execution timestamp
- `next_fire_at`: Next scheduled fire time (cron only)
- `run_count`: Total successful executions
- `consecutive_failures`: Failure counter for circuit-breaking
- `state`: JSON blob for routine-specific persistence

### Engine Execution Flow

**Cron ticker loop** (`routine_engine.rs`):
1. Poll DB every N seconds for due cron routines
2. For each due routine: check guardrails → execute → notify
3. Update `last_run_at`, `next_fire_at`, `run_count` in DB

**Event matcher** (called from agent main loop):
1. On each `IncomingMessage`, check `routine_matches_message()`
2. If pattern matches and filters pass → execute routine inline
3. Event routines run synchronously before the agentic turn starts

### Adding a New Routine

1. Define the routine struct in `routine.rs` (if new fields needed)
2. Add trigger parsing in `Trigger::from_db()` (routine.rs)
3. Implement execution logic in `routine_engine.rs`:
   - `execute_lightweight()` for single-call routines
   - `execute_full_job()` for scheduler-delegated routines
4. Add guardrail checks in `check_guardrails()`
5. Wire up notification in `notify_user()`
6. Add DB migration if schema changes (see `src/db/migrations/`)

### Routine Examples

**Example 1: Daily Standup Reminder (Cron + Lightweight)**

```yaml
name: daily-standup
trigger:
  cron: "0 9 * * MON-FRI"  # 9 AM on weekdays
  timezone: "America/Sao_Paulo"
action:
  type: lightweight
  prompt: "List all incomplete tasks from the last 24h and ask the user to prioritize"
  context_paths:
    - workspace/tasks/pending.md
  max_tokens: 2048
  use_tools: true
  max_tool_rounds: 2
guardrails:
  max_daily_runs: 1
  sandbox_required: false
notify:
  on_success: true
```

**Example 2: Urgent Message Alert (Event-triggered + Full Job)**

```yaml
name: urgent-alert
trigger:
  event:
    channel: telegram
    pattern: "(urgente|emergência|prioridade máxima)"
action:
  type: full_job
  title: "Processar Mensagem Urgente"
  description: "Analisar mensagem urgente, verificar contexto relevante e gerar plano de ação"
  max_iterations: 30
guardrails:
  max_concurrent: 1
  max_daily_runs: 10
  sandbox_required: true
notify:
  always: true
```

**Example 3: Weekly Repository Audit (Cron + Full Job)**

```yaml
name: weekly-audit
trigger:
  cron: "0 10 * * MON"  # Monday 10 AM
action:
  type: full_job
  title: "Auditoria Semanal do Repositório"
  description: "Rodar testes, verificar coverage, analisar diffs da semana, gerar report"
  max_iterations: 100
guardrails:
  max_daily_runs: 1
  max_concurrent: 1
  sandbox_required: true
  allowed_tools:
    - shell
    - file_read
    - file_write
    - git_diff
notify:
  on_success: true
  on_failure: true
```

**Example 4: Manual One-Off Routine (Manual Trigger)**

```yaml
name: adhoc-analysis
trigger:
  manual: true  # Only fires when explicitly invoked
action:
  type: lightweight
  prompt: "Analyze the current workspace state and suggest improvements"
  context_paths:
    - workspace/notes/current-focus.md
  max_tokens: 4096
  use_tools: true
  max_tool_rounds: 5
guardrails:
  sandbox_required: false
```

### Key Invariants

- Routines fire **independently** of user sessions — they don't hold session locks
- Lightweight routines execute **synchronously** in the agent loop — keep them fast (<5s)
- Full-job routines are **asynchronous** — safe for long-running operations
- Guardrails are checked **before** execution — failures return `RoutineError::GuardrailViolated`
- Consecutive failures trigger **circuit-breaking** — routine disabled after threshold (default: 3)
- Event routines match **case-insensitively** on message content
- Cron schedules use **user-configured timezone** (defaults to UTC)

## Adding a New Submission Command

Submissions are special messages parsed in `submission.rs` before the agentic loop runs. To add a new one:
1. Add a variant to `Submission` enum in `submission.rs`
2. Add parsing in `SubmissionParser::parse()`
3. Handle in `agent_loop.rs` where `SubmissionResult` is matched (the `match submission { ... }` block in `handle_message`)
4. Implement the handler method (usually in `thread_ops.rs` for session operations, or `commands.rs` for system commands)
