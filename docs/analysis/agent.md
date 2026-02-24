# IronClaw Agent Runtime System — Deep Dive

**Version:** v0.11.1
**Source tree:** `src/agent/` (21 files)
**Last updated:** 2026-02-24

---

## 1. Overview

The IronClaw agent runtime is a Tokio-based async Rust system that orchestrates
multi-channel AI assistant behavior. It receives messages from any registered
channel (TUI, HTTP, Telegram, Slack, web gateway), routes them through intent
classification, manages session and thread state, dispatches work to LLM-backed
jobs or lightweight handlers, and returns responses — all while running
background tasks for heartbeat monitoring, context compaction, self-repair, and
scheduled routines.

The design philosophy is **defense in depth with extensibility**. Every
subsystem is hidden behind a trait (`SelfRepair`, `TaskHandler`, `Database`),
every tool output is sanitized before reaching the LLM, and every credential
is injected at the host boundary so it never enters a sandboxed process.

### Design Goals

- **Parallel by default** — Jobs run in independent Tokio tasks. Tool calls
  within a job execute concurrently via `JoinSet`.
- **Self-healing** — A background repair loop detects `JobState::Stuck` entries
  and calls `attempt_recovery()` automatically.
- **Context-budget-aware** — The context monitor watches token consumption and
  triggers one of three compaction strategies before the LLM window overflows.
  As of v0.11.0, `ContextLengthExceeded` errors from the LLM also trigger
  auto-compact via `ContextCompactor`, after which the failed call is retried
  automatically.
- **Cost-safe** — A `CostGuard` enforces a daily USD budget ceiling and an
  hourly action rate limit with an `AtomicBool` fast path for zero overhead on
  the hot path.
- **Undo-capable** — Every turn is checkpointed before execution, enabling
  unlimited undo/redo within a configurable stack depth.

---

## 2. Key Subsystems

| Subsystem | File(s) | Responsibility |
|-----------|---------|----------------|
| Agent Loop | `agent_loop.rs` | Top-level message pump; spawns background tasks |
| Router | `router.rs` | Classifies `MessageIntent` from raw text |
| Submission Parser | `submission.rs` | Maps text to typed `Submission` variants |
| Session Manager | `session_manager.rs` | Thread/session lifecycle, pruning |
| Session Model | `session.rs` | `Session`, `Thread`, `Turn` structs and state machines |
| Dispatcher | `dispatcher.rs` | `run_agentic_loop()`: LLM↔tool loop with JoinSet |
| Worker | `worker.rs` | Per-job execution: planning, tool selection, iteration |
| Scheduler | `scheduler.rs` | Job and subtask lifecycle, concurrency control |
| Context Monitor | `context_monitor.rs` | Token budget estimation, compaction threshold checks |
| Compaction | `compaction.rs` | Three compaction strategies for full context windows |
| Context Compactor | `compaction.rs` | Three compaction strategies: Summarize (LLM-generated summary → `daily/{date}.md`), Truncate (drop oldest turns), MoveToWorkspace (archive); triggered on ContextLengthExceeded |
| Cost Guard | `cost_guard.rs` | Daily budget + hourly rate enforcement |
| Self-Repair | `self_repair.rs` | Stuck job detection and recovery; broken tool rebuild |
| Heartbeat | `heartbeat.rs` | Proactive periodic LLM execution from `HEARTBEAT.md` |
| Routine Engine | `routine_engine.rs` | Cron ticker and event pattern matcher |
| Routine Types | `routine.rs` | `Trigger`, `RoutineAction`, `RoutineGuardrails` |
| Undo Manager | `undo.rs` | Checkpoint-based undo/redo with VecDeque |
| Thread Operations | `thread_ops.rs` | Input processing, approval handling, DB hydration |
| Commands | `commands.rs` | System command dispatch (`/help`, `/model`, etc.) |
| Task | `task.rs` | `Task` enum and `TaskHandler` trait |
| Job Monitor | `job_monitor.rs` | SSE-based container output forwarding |

---

## 3. Agent Loop Deep Dive

### 3.1 Startup Sequence

The `Agent` struct is constructed with an `AgentDeps` bundle that carries
every shared resource behind `Arc<T>` pointers:

```rust
pub struct AgentDeps {
    pub store: Arc<dyn Database>,
    pub llm: Arc<dyn LlmProvider>,
    pub cheap_llm: Option<Arc<dyn LlmProvider>>,
    pub safety: Arc<SafetyLayer>,
    pub tools: Arc<ToolRegistry>,
    pub workspace: Arc<Workspace>,
    pub extension_manager: Arc<ExtensionManager>,
    pub skill_registry: Arc<SkillRegistry>,
    pub hooks: Arc<HookRegistry>,
    pub cost_guard: Arc<CostGuard>,
}
```

`Agent::run()` spawns four optional background tasks before entering the main
message loop:

```
Agent::run()
├── spawn repair_handle     (DefaultSelfRepair background loop)
├── spawn pruning_handle    (SessionManager::prune_stale_sessions every 10 min)
├── spawn heartbeat_handle  (HeartbeatRunner if HEARTBEAT_ENABLED)
└── spawn cron_handle       (RoutineEngine::spawn_cron_ticker if ROUTINES_ENABLED)
     │
     └── tokio::select! loop
           ├── Ctrl-C signal  → graceful shutdown
           └── channel msg   → handle_message()
                               └── check_event_triggers() (RoutineEngine)
```

### 3.2 Message Handling Flow

```
IncomingMessage
      │
      ▼
SubmissionParser::parse()          ← keyword matching, JSON detection
      │
      ▼
BeforeInbound hook                 ← extension intercept point
      │
      ▼
maybe_hydrate_thread()             ← load DB history if UUID thread ID
      │
      ▼
SessionManager::resolve_thread()   ← create or find session+thread
      │
      ├── Submission::UserInput ──→ process_user_input()
      │                               └── run_agentic_loop()
      │
      ├── Submission::ExecApproval/
      │   ApprovalResponse      ──→ process_approval()
      │                               └── execute approved tool
      │                               └── run_agentic_loop() (resume)
      │
      ├── Submission::Undo/Redo ──→ process_undo() / process_redo()
      │
      ├── Submission::Compact   ──→ ContextCompactor::compact()
      │
      ├── Submission::SystemCmd ──→ handle_system_command()
      │
      └── Submission::Heartbeat/
          Summarize/Suggest     ──→ dedicated handlers
```

### 3.3 Key Types

```rust
pub struct Agent {
    deps: Arc<AgentDeps>,
    session_manager: Arc<SessionManager>,
    scheduler: Arc<Mutex<Scheduler>>,
    router: Router,
    routine_engine: Option<Arc<RoutineEngine>>,
    config: Arc<Config>,
    channels: Arc<ChannelManager>,
}
```

---

## 4. Session Management

### 4.1 Hierarchy

IronClaw uses a three-level hierarchy for conversation state:

```
Session  (one per user+channel combination)
  └── Thread  (a conversation; a session can have multiple threads)
        └── Turn  (one user↔assistant exchange within a thread)
```

### 4.2 State Machines

**ThreadState:**

```
Idle ──────────────────────────────────────────────┐
  │                                                  │
  └─► Processing ──► AwaitingApproval ──► Processing │
            │                                         │
            └──────────────────────────► Completed ───┘
            │
            └──► Interrupted
```

**TurnState:**

```
Processing ──► Completed
     │
     └──► Failed
     └──► Interrupted
```

### 4.3 Thread Isolation

`SessionManager` uses a `ThreadKey` struct for map lookups:

```rust
struct ThreadKey {
    user_id: String,
    channel: String,
    external_thread_id: Option<String>,
}
```

The `channel` field prevents cross-channel thread leakage. A Telegram thread
ID and a Slack thread ID with the same string value resolve to different
`Thread` objects.

### 4.4 Session Pruning

`prune_stale_sessions()` runs every 10 minutes. It uses `try_lock()` on each
session so it never blocks the main message loop on a contended session. Sessions
idle longer than `session_idle_timeout` (configurable) are removed and an
`OnSessionEnd` hook is fired for each.

A warning is logged when the session count exceeds 1000 (`SESSION_COUNT_WARNING_THRESHOLD`).

### 4.5 Double-Checked Locking

`resolve_thread()` applies the double-checked locking pattern to prevent races
during concurrent session creation:

1. Read lock: check if session already exists.
2. If not found: upgrade to write lock.
3. Re-check: another task may have created the session between steps 1 and 2.
4. Only create if still absent.

---

## 5. Job System

### 5.1 Job Lifecycle — Sequence Diagram

```
User            Agent           Scheduler          Worker           LLM
 │                │                  │                 │              │
 │ send message   │                  │                 │              │
 │───────────────►│                  │                 │              │
 │                │ schedule(job)    │                 │              │
 │                │─────────────────►│                 │              │
 │                │                  │ spawn Worker    │              │
 │                │                  │────────────────►│              │
 │                │                  │ send Start      │              │
 │                │                  │────────────────►│              │
 │                │                  │                 │ call LLM     │
 │                │                  │                 │─────────────►│
 │                │                  │                 │◄─────────────│
 │                │                  │                 │ execute tools│
 │                │                  │                 │─────────────►│ (parallel JoinSet)
 │                │                  │                 │◄─────────────│
 │                │                  │                 │ loop (max 50)│
 │                │                  │                 │ complete     │
 │                │◄────────────────────────────────────│              │
 │◄───────────────│                  │                 │              │
```

### 5.2 Scheduler

`Scheduler` maintains two maps:

- `HashMap<Uuid, ScheduledJob>` — full jobs with a `JoinHandle`
- `HashMap<Uuid, ScheduledSubtask>` — one-shot subtasks

`schedule()` holds the write lock for the entire check+insert+spawn sequence to
prevent TOCTOU races. The job state transitions from `Pending` to `InProgress`,
a `Worker` is constructed, and a `Start` message is sent through an mpsc channel.

`spawn_batch()` spawns all subtasks, then awaits results in their original order,
preserving deterministic output ordering even though execution is parallel.

### 5.3 Worker Execution

`Worker::run()` executes jobs in `execution_loop()` with a configurable timeout:

1. Fetch job context from `ContextManager`.
2. Optionally generate an `ActionPlan` via LLM (when `use_planning = true`).
3. Iterate up to 50 times:
   - `select_tools()` — picks the next tool(s) from the plan.
   - `respond_with_tools()` — calls LLM, requests tool use.
   - `execute_tools_parallel()` — runs all requested tools concurrently.
   - `process_tool_result()` — sanitizes output, records failures for self-repair.
4. Exit when `llm_signals_completion()` returns true (LLM text response, not tool output).

**Injection prevention:** Tool output cannot signal job completion. Only an LLM
text response in the final assistant turn terminates the loop. This blocks
prompt injection via malicious tool return values.

### 5.4 Parallel Tool Dispatch — Three Phases

`dispatcher.rs` implements `run_agentic_loop()` with three sequential phases
per iteration:

**Phase 1 — Preflight (sequential):**

```
for each tool_call:
    check requires_approval() → NeedApproval if yes
    fire BeforeToolCall hook → may modify or block
```

**Phase 2 — Parallel execution:**

```rust
let mut set = JoinSet::new();
for (index, call) in tool_calls.iter().enumerate() {
    set.spawn(execute_chat_tool_standalone(index, call, deps.clone()));
}
// collect results in completion order, reorder by index
```

**Phase 3 — Post-flight (sequential):**

```
for each result:
    check_auth_trigger()
    sanitize via SafetyLayer
    record_action() in ContextManager
```

---

## 6. Routine Engine

### 6.1 Trigger Types

```rust
pub enum Trigger {
    Cron { schedule: String },            // "0 9 * * 1-5" (cron expr)
    Event { channel: String, pattern: String }, // regex on message content
    Webhook { path: String, secret: Option<String> },
    Manual,
}
```

### 6.2 Action Types

```rust
pub enum RoutineAction {
    Lightweight {
        prompt: String,
        context_paths: Vec<String>,  // workspace files to inject
        max_tokens: Option<u32>,
    },
    FullJob {
        title: String,
        description: String,
        max_iterations: Option<u32>,
    },
}
```

### 6.3 Cron Execution Path

`spawn_cron_ticker()` creates a background Tokio task that sleeps for
`cron_check_interval_secs` (default 60 s) between ticks. On each tick it calls
`check_cron_triggers()`, which queries `store.list_due_cron_routines()` and
fires each matching routine via `spawn_fire()`.

### 6.4 Event Execution Path

`check_event_triggers()` is called synchronously inside `handle_message()` after
every inbound message. It reads `event_cache` (an `RwLock<Vec<(Uuid, Routine, Regex)>>`)
and checks each entry:

1. Regex match on message content.
2. Cooldown check (default 300 s between fires).
3. Concurrency check (`running_count` AtomicUsize vs `max_concurrent`).
4. If all pass: `spawn_fire()`.

### 6.5 Guardrails

```rust
pub struct RoutineGuardrails {
    pub cooldown_secs: u64,       // default 300
    pub max_concurrent: usize,    // default 1
    pub dedup_window_secs: Option<u64>,
}
```

The `dedup_window` prevents duplicate fires within a time window — useful for
event triggers that might fire multiple times for the same logical event.

### 6.6 Lightweight Execution Sentinel

For `Lightweight` routines, `execute_lightweight()` makes a single LLM call
and checks whether the response contains the string `"ROUTINE_OK"`. If not, a
notification is sent to the configured channel. This mirrors the heartbeat
sentinel pattern, avoiding noisy notifications for successful routine runs.

---

## 7. Heartbeat System

The heartbeat provides proactive periodic agent execution — the agent checks in
on itself and notifies the user only if something needs attention.

### 7.1 Configuration

```rust
pub struct HeartbeatConfig {
    pub interval: Duration,              // default 30 min
    pub enabled: bool,
    pub max_failures: u32,               // default 3 consecutive failures
    pub notify_user_id: Option<String>,
    pub notify_channel: Option<String>,
}
```

### 7.2 Execution Flow

```
HeartbeatRunner::run()
      │
      └── tokio::time::interval (ticks at HeartbeatConfig::interval)
             │ (skip first tick to avoid firing immediately on startup)
             ▼
          check_heartbeat()
             │
             ├── read HEARTBEAT.md from workspace
             │     └── if is_effectively_empty() → return Skipped
             │
             ├── call LLM with checklist prompt
             │
             ├── response == "HEARTBEAT_OK" → HeartbeatResult::Ok
             │                                 (no notification sent)
             │
             └── response != "HEARTBEAT_OK" → HeartbeatResult::NeedsAttention(msg)
                                               → send notification to channel
```

Additionally, each heartbeat tick runs memory hygiene (workspace cleanup) as a
background Tokio task, independent of the LLM call result.

### 7.3 Failure Handling

If the LLM call itself fails (network error, context overflow, etc.), the runner
counts consecutive failures. After `max_failures` consecutive failures it logs
an error and continues rather than crashing, preserving the background loop.

---

## 8. Cost Guard

`CostGuard` enforces two independent spending limits: a daily USD budget and an
hourly action rate limit.

### 8.1 Data Structure

```rust
pub struct CostGuard {
    daily_cost: Mutex<DailyCost>,              // resets at midnight UTC
    action_window: Mutex<VecDeque<Instant>>,   // sliding 1-hour window
    budget_exceeded: AtomicBool,               // fast-path flag
}

struct DailyCost {
    date: NaiveDate,
    spent_cents: u64,   // tracked in cents to avoid floating-point drift
    limit_cents: u64,
}
```

### 8.2 Check Path

```
check_allowed()
  │
  ├── AtomicBool fast path (load Relaxed)
  │     └── if true → return Err(DailyBudget) immediately (zero lock contention)
  │
  ├── lock daily_cost
  │     ├── if date changed → reset spent to 0
  │     └── if spent >= limit → set AtomicBool, return Err(DailyBudget)
  │
  └── lock action_window
        ├── drain entries older than 1 hour
        └── if window.len() >= hourly_limit → return Err(HourlyRate)
```

### 8.3 Recording

`record_llm_call(cost_usd: Decimal)` adds the cost to `daily_cost.spent_cents`
and pushes the current `Instant` to `action_window`. At 80% of daily budget a
warning is logged without blocking further calls.

### 8.4 Error Types

```rust
pub enum CostLimitExceeded {
    DailyBudget { spent_cents: u64, limit_cents: u64 },
    HourlyRate  { actions: usize,   limit: usize },
}
```

---

## 9. Context Monitor

### 9.1 Constants

```
DEFAULT_CONTEXT_LIMIT  = 100_000 tokens
COMPACTION_THRESHOLD   = 0.80   (80% of limit)
TOKENS_PER_WORD        = 1.3    (word-count estimation)
```

Token counts are estimated from word counts (not from a real tokenizer), making
the monitor a heuristic rather than exact. The generous threshold gives a safety
margin for estimation error.

### 9.2 Compaction Strategy Selection

```
suggest_compaction(breakdown: ContextBreakdown) -> Option<CompactionStrategy>

  usage > 95%  →  Truncate { keep_recent: 3 }
                  (aggressive: only keep the 3 most recent turns)

  usage > 85%  →  Summarize { keep_recent: 5 }
                  (LLM summarizes older turns, keeps 5 recent)

  usage > 80%  →  MoveToWorkspace
                  (archive full turn text to daily workspace log)

  usage <= 80% →  None  (no compaction needed)
```

### 9.3 ContextBreakdown

```rust
pub struct ContextBreakdown {
    pub system_tokens: usize,
    pub user_tokens: usize,
    pub assistant_tokens: usize,
    pub tool_tokens: usize,
    pub total_tokens: usize,
    pub limit: usize,
    pub usage_pct: f64,
}
```

The monitor estimates each role's token contribution separately, allowing the
compaction logic to selectively target high-volume contributors (e.g., verbose
tool outputs in `tool_tokens`).

---

## 10. Compaction

`ContextCompactor` implements three strategies, each producing a `CompactionResult`:

```rust
pub struct CompactionResult {
    pub turns_removed: usize,
    pub tokens_before: usize,
    pub tokens_after: usize,
    pub summary_written: bool,
    pub summary: Option<String>,
}
```

### 10.1 Strategy: Summarize

1. Identify turns older than `keep_recent` (default 5).
2. Serialize those turns to text.
3. Call LLM with a summarization prompt.
4. Write the summary to the workspace daily log (`daily/YYYY-MM-DD.md`).
5. Replace the summarized turns with a single synthetic "summary" turn.
6. Return `CompactionResult` with `summary_written: true`.

This preserves semantic content while reducing token consumption.

### 10.2 Strategy: Truncate

Simple drain: remove all turns older than `keep_recent` (default 3) without
calling the LLM. Fastest option for critical situations where the context is
nearly full and latency matters more than history preservation.

### 10.3 Strategy: MoveToWorkspace

1. Serialize full turn text (user + assistant + tool calls).
2. Append to workspace archive: `context/archive/YYYY-MM-DD.md`.
3. Keep the 10 most recent turns in active memory.
4. Returns `summary_written: false` (content moved, not summarized).

Useful for long-running sessions where the user wants the full history
preserved but not necessarily in the context window.

### 10.4 Context Compaction (v0.11.0)

**Source:** `src/agent/compaction.rs` (346 lines)

In addition to the proactive threshold-based compaction described above,
v0.11.0 introduced reactive compaction: when the LLM returns a
`ContextLengthExceeded` error the agentic loop immediately calls
`ContextCompactor::compact()` and retries the failed LLM call automatically.

The same three strategies are available:

- `Summarize { keep_recent }` — Uses the LLM (max 1024 tokens, temperature 0.3)
  to summarize old turns, preserving the most recent `keep_recent` turns.
  Summary is written to `daily/{YYYY-MM-DD}.md` in the workspace.
- `Truncate { keep_recent }` — Drops the oldest turns without calling the LLM.
  Lowest-latency option for near-full contexts.
- `MoveToWorkspace` — Archives full turn text to the workspace daily log.

All strategies return a `CompactionResult`:

```rust
pub struct CompactionResult {
    pub turns_removed: usize,
    pub tokens_before: usize,
    pub tokens_after: usize,
    pub summary_written: bool,
    pub summary: Option<String>,
}
```

After compaction completes, the agentic loop retries the failed LLM call
automatically with the reduced context.

---

## 11. Self-Repair

### 11.1 Architecture

```rust
#[async_trait]
pub trait SelfRepair: Send + Sync {
    async fn detect_stuck_jobs(&self)  -> Vec<StuckJob>;
    async fn repair_stuck_job(&self, job: &StuckJob) -> Result<RepairResult, RepairError>;
    async fn detect_broken_tools(&self) -> Vec<BrokenTool>;
    async fn repair_broken_tool(&self, tool: &BrokenTool) -> Result<RepairResult, RepairError>;
}
```

### 11.2 Stuck Job Detection

`DefaultSelfRepair::detect_stuck_jobs()` calls
`ContextManager::find_stuck_jobs()`, then filters for entries where
`ctx.state == JobState::Stuck`. For each, it computes `stuck_duration` from
`started_at` to `Utc::now()`.

```rust
pub struct StuckJob {
    pub job_id: Uuid,
    pub last_activity: DateTime<Utc>,
    pub stuck_duration: Duration,
    pub last_error: Option<String>,
    pub repair_attempts: u32,
}
```

### 11.3 Job Recovery Flow

```
repair_stuck_job(job)
  │
  ├── job.repair_attempts >= max_repair_attempts?
  │     └── Yes → ManualRequired { message }
  │
  └── No → context_manager.update_context(job_id, |ctx| ctx.attempt_recovery())
              ├── Ok(Ok(())) → Success { message }
              ├── Ok(Err(e)) → Retry  { message }
              └── Err(e)     → RepairError::Failed
```

### 11.4 Broken Tool Repair

When a WASM tool accumulates 5 or more failures (`get_broken_tools(5)`), the
repair system attempts an automatic rebuild:

1. `increment_repair_attempts()` in database.
2. Construct a `BuildRequirement` with `SoftwareType::WasmTool` and a
   description containing the error message and failure count.
3. Call `builder.build(&requirement)` — the builder uses the LLM to analyze the
   error and rewrite the tool source.
4. On success: call `mark_tool_repaired()` in database; log if auto-registered.
5. On failure: return `Retry` for another attempt later.

### 11.5 Repair Results

```rust
pub enum RepairResult {
    Success        { message: String },
    Retry          { message: String },  // transient failure, try again
    Failed         { message: String },  // terminal failure
    ManualRequired { message: String },  // exceeded attempt limit
}
```

### 11.6 RepairTask Background Loop

`RepairTask::run()` loops forever, sleeping `check_interval` between cycles.
Each cycle: detect+repair stuck jobs, then detect+repair broken tools.

---

## 12. Router and Submission Parser

### 12.1 Router

`Router` classifies inbound text into `MessageIntent`:

```rust
pub enum MessageIntent {
    CreateJob,
    CheckJobStatus,
    CancelJob,
    ListJobs,
    HelpJob,
    Chat,
    Command,
    Unknown,
}
```

`route_command()` returns `Some(intent)` only if the text begins with the
command prefix (default `/`). Natural language text returns `None` and flows
directly to the agentic loop.

Command table:

| Input prefix | Intent |
|---|---|
| `/job` | CreateJob |
| `/status` | CheckJobStatus |
| `/cancel` | CancelJob |
| `/list`, `/jobs` | ListJobs |
| `/help <id>` | HelpJob |
| `/help`, `/ping`, `/version`, `/tools`, `/debug`, `/model` | Command |

### 12.2 Submission Parser

`SubmissionParser::parse()` maps raw text to a typed `Submission` before
intent classification. Matching is done on lowercase-trimmed content:

```rust
pub enum Submission {
    UserInput(String),
    ExecApproval(serde_json::Value),  // JSON detection
    ApprovalResponse { approved: bool, always: bool },
    Interrupt,
    Compact,
    Undo,
    Redo,
    Resume,
    Clear,
    SwitchThread(String),
    NewThread,
    Heartbeat,
    Summarize,
    Suggest,
    Quit,
    SystemCommand(String),
}
```

Approval aliases:

| User types | Result |
|---|---|
| `yes`, `y`, `approve`, `ok` | `ApprovalResponse { approved: true, always: false }` |
| `always`, `a` | `ApprovalResponse { approved: true, always: true }` |
| `no`, `n`, `deny` | `ApprovalResponse { approved: false, always: false }` |

---

## 13. Undo System

### 13.1 Data Structure

```rust
pub struct UndoManager {
    undo_stack: VecDeque<Checkpoint>,   // front = oldest
    redo_stack: Vec<Checkpoint>,        // back  = most recent redo
    max_checkpoints: usize,             // default 20
}

pub struct Checkpoint {
    pub id: Uuid,
    pub turn_number: u32,
    pub messages: Vec<ChatMessage>,
    pub description: String,
}
```

### 13.2 Operations

```
checkpoint(current_messages, description)
  → clears redo_stack (new branch destroys future history)
  → push_undo(Checkpoint)
  → if stack len > max_checkpoints: pop oldest from front

undo(current_messages)
  → save current to redo_stack
  → pop from undo_stack → return checkpoint

redo(current_messages)
  → push current to undo_stack via push_undo()
  → pop from redo_stack → return checkpoint

restore(id)
  → find checkpoint by id in undo_stack
  → truncate all checkpoints after it
  → return found checkpoint
```

### 13.3 Invariant

Undo + redo total size stays constant during undo/redo operations: one item
moves from one stack to the other. Only `checkpoint()` and the max-checkpoint
trim change the total count.

### 13.4 Integration with Thread Operations

`process_user_input()` calls `undo_manager.checkpoint()` before starting each
turn. This ensures every turn can be fully rewound: the checkpoint captures the
pre-turn `messages` vector, so restoring it reverts the thread to the exact
state before that turn's LLM call.

---

## 14. Key Types Reference

### Session Model

| Type | Fields | Purpose |
|------|--------|---------|
| `Session` | id, user_id, active_thread, threads, created_at, last_active_at, metadata, auto_approved_tools | Top-level user session |
| `Thread` | id, session_id, state, turns, pending_approval, pending_auth, last_response_id | Single conversation |
| `Turn` | turn_number, user_input, response, tool_calls, state, started_at, completed_at, error | One exchange |
| `PendingApproval` | request_id, tool_name, parameters, description, tool_call_id, context_messages, deferred_tool_calls | Awaiting user approval |
| `PendingAuth` | extension_name | Thread in credential-entry mode |

### Job and Task Types

| Type | Fields | Purpose |
|------|--------|---------|
| `Task` | `Job{id,title,description}` / `ToolExec{parent_id,tool_name,params}` / `Background{id,handler}` | Unit of executable work |
| `TaskOutput` | result (JSON), duration | Task result |
| `TaskContext` | task_id, parent_id, metadata | Execution metadata |

### Repair Types

| Type | Key Fields | Purpose |
|------|-----------|---------|
| `StuckJob` | job_id, stuck_duration, repair_attempts | Detected stuck job |
| `BrokenTool` | name, failure_count, last_error, repair_attempts | Detected broken WASM tool |
| `RepairResult` | Success / Retry / Failed / ManualRequired | Outcome of a repair attempt |

### Routine Types

| Type | Key Fields | Purpose |
|------|-----------|---------|
| `Routine` | id, name, trigger, action, guardrails, notify | Scheduled/reactive task |
| `Trigger` | Cron / Event / Webhook / Manual | How the routine fires |
| `RoutineAction` | Lightweight / FullJob | What the routine does |
| `RoutineGuardrails` | cooldown_secs, max_concurrent, dedup_window_secs | Safety limits |
| `NotifyConfig` | channel, user, on_attention, on_failure, on_success | Notification policy |

---

## 15. Configuration Reference

All configuration is read from environment variables at startup. Relevant
`agent/` subsystem variables:

| Variable | Default | Subsystem | Description |
|----------|---------|-----------|-------------|
| `MAX_PARALLEL_JOBS` | `5` | Scheduler | Maximum concurrent jobs |
| `HEARTBEAT_ENABLED` | `true` | Heartbeat | Enable periodic LLM checks |
| `HEARTBEAT_INTERVAL_SECS` | `1800` | Heartbeat | Seconds between heartbeat ticks |
| `HEARTBEAT_NOTIFY_CHANNEL` | `tui` | Heartbeat | Channel for attention alerts |
| `HEARTBEAT_NOTIFY_USER` | `default` | Heartbeat | User ID for notifications |
| `ROUTINES_ENABLED` | `true` | Routine Engine | Enable routine execution |
| `ROUTINES_CRON_INTERVAL` | `60` | Routine Engine | Cron tick interval in seconds |
| `ROUTINES_MAX_CONCURRENT` | `3` | Routine Engine | Max concurrent routine runs |
| `AGENT_NAME` | `ironclaw` | Agent Loop | Agent identity for prompts |

**Cost Guard** (no dedicated env vars; configured programmatically via `CostGuard::new()`):

| Parameter | Default | Description |
|-----------|---------|-------------|
| `daily_budget_usd` | no limit | USD ceiling per calendar day (UTC) |
| `hourly_action_limit` | no limit | Max LLM calls per sliding 1-hour window |

**Context Monitor** (compile-time constants in `context_monitor.rs`):

| Constant | Value | Description |
|----------|-------|-------------|
| `DEFAULT_CONTEXT_LIMIT` | 100,000 | Token limit for context window |
| `COMPACTION_THRESHOLD` | 0.80 | Fraction at which compaction triggers |
| `TOKENS_PER_WORD` | 1.3 | Estimation multiplier |

**Session Manager** (compile-time constants in `session_manager.rs`):

| Constant | Value | Description |
|----------|-------|-------------|
| `SESSION_COUNT_WARNING_THRESHOLD` | 1,000 | Session count at which a warning is logged |
| pruning interval | 10 min | How often `prune_stale_sessions()` runs |

**Worker** (configurable per job via `WorkerDeps`):

| Field | Default | Description |
|-------|---------|-------------|
| `timeout` | configurable | Total timeout for a single job |
| `use_planning` | false | Generate `ActionPlan` before execution loop |
| max iterations | 50 | Maximum tool→LLM cycles per job |

**Self-Repair** (configured in `DefaultSelfRepair::new()`):

| Parameter | Description |
|-----------|-------------|
| `stuck_threshold` | Duration before a job is considered stuck |
| `max_repair_attempts` | Attempts before escalating to ManualRequired |

**UndoManager** (configured at construction):

| Parameter | Default | Description |
|-----------|---------|-------------|
| `max_checkpoints` | 20 | Maximum undo/redo entries |

---

*End of agent system analysis. Total source files analyzed: 21.*
