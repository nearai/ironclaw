# Routines System — IronClaw Autonomous Automation

## Overview

IronClaw Routines enable autonomous, recurring task execution with flexible triggers, guardrails, and notification. Routines can run as lightweight inline operations or full background jobs with complete agentic loops.

**Key capabilities:**
- Cron-based scheduling (e.g., daily standup, weekly audits)
- Event-driven triggers (message patterns, system events, webhooks)
- Manual on-demand execution
- Two execution modes: lightweight (inline) and full-job (scheduler)
- Comprehensive guardrails (rate limits, tool allowlists, sandbox requirements)
- Configurable notifications on success/failure
- Circuit-breaking on consecutive failures
- Persistent runtime state in database

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│  Agent Main Loop (agent_loop.rs)                            │
│                                                              │
│  ┌──────────────────┐    ┌──────────────────────────────┐  │
│  │ Cron Ticker      │    │ Event Matcher                │  │
│  │ (background task)│    │ (on each IncomingMessage)    │  │
│  │                  │    │                              │  │
│  │ Polls DB every N │    │ Checks routine pattern match │  │
│  │ seconds for due  │───▶│ → routine_matches_message()  │  │
│  │ cron routines    │    │                              │  │
│  └──────────────────┘    └──────────────┬───────────────┘  │
│                                          │                   │
│                                          ▼                   │
│                          ┌───────────────────────────────┐  │
│                          │ RoutineEngine                 │  │
│                          │ (routine_engine.rs)           │  │
│                          │                               │  │
│                          │ ┌───────────────────────────┐ │  │
│                          │ │ check_guardrails()        │ │  │
│                          │ └───────────┬───────────────┘ │  │
│                          │             │                 │  │
│                          │             ▼                 │  │
│                          │ ┌───────────────────────────┐ │  │
│                          │ │ execute_lightweight()     │ │  │
│                          │ │ OR                        │ │  │
│                          │ │ execute_full_job()        │ │  │
│                          │ └───────────┬───────────────┘ │  │
│                          │             │                 │  │
│                          │             ▼                 │  │
│                          │ ┌───────────────────────────┐ │  │
│                          │ │ notify_user()             │ │  │
│                          │ │ (if configured)           │ │  │
│                          │ └───────────────────────────┘ │  │
│                          └───────────────────────────────┘  │
└─────────────────────────────────────────────────────────────┘
```

**Key files:**
- `routine.rs` — Core types: `Routine`, `Trigger`, `RoutineAction`, `RoutineGuardrails`, `NotifyConfig`
- `routine_engine.rs` — Execution engine with cron ticker and event matcher (2561 lines)
- `routine_integration_tests.rs` — Integration tests for routine type system

## Trigger Types

| Type | Description | Example |
|------|-------------|---------|
| `Cron` | Fire on cron schedule | `"0 9 * * MON-FRI"`, `"every 2h"` |
| `Event` | Fire when channel message matches regex | pattern: `"(urgente\|prioridade)"`, channel: `"telegram"` |
| `SystemEvent` | Fire on structured system events | source: `"github"`, event_type: `"issue.opened"` |
| `Webhook` | Fire on POST to `/api/webhooks/{path}` | path: `"deploy-complete"`, secret: HMAC |
| `Manual` | Only via tool call or CLI | — |

### Trigger Details

**Cron:**
- Supports standard 5-field cron syntax
- Optional timezone (defaults to UTC)
- Natural language aliases: `"every 2h"`, `"daily"`, `"weekly"`

**Event:**
- Regex pattern matched against message content (case-insensitive)
- Optional channel filter (e.g., `"telegram"`, `"slack"`)
- Fires synchronously before the agentic turn starts

**SystemEvent:**
- Structured events from system components
- Source + event_type matching (e.g., `github` + `issue.opened`)
- Optional payload filters (key-value matching)
- Emitted via `RoutineEngine::emit_system_event()`

**Webhook:**
- HTTP POST to `/webhook/tools/{tool}` or `/api/webhooks/{path}`
- Optional HMAC secret validation
- Payload normalized by target tool into system events

**Manual:**
- Only fires when explicitly invoked via tool call or CLI
- No automatic triggers
- Useful for ad-hoc analysis and on-demand workflows

## Execution Modes

### Lightweight (Default)

**Characteristics:**
- Single LLM call executed inline in the agent loop
- No scheduler slot consumed
- Tool calls allowed but limited by guardrails
- Execution time: typically <5 seconds
- Runs synchronously — blocks the agent loop briefly

**Best for:**
- Notifications and reminders
- Quick summaries
- Status checks
- Simple data lookups

**Configuration:**
```yaml
action:
  type: lightweight
  prompt: "List all incomplete tasks from the last 24h"
  context_paths:
    - workspace/tasks/pending.md
  max_tokens: 2048
  use_tools: true
  max_tool_rounds: 2
```

### Full-Job

**Characteristics:**
- Delegated to `Scheduler` (runs as background job)
- Full agentic loop with multiple turns
- Can use sandbox for isolation
- Supports long-running operations
- Independent of user session — no session lock held

**Best for:**
- Complex multi-step workflows
- Repository audits
- Code analysis
- Tasks requiring extensive tool use

**Configuration:**
```yaml
action:
  type: full_job
  title: "Weekly Repository Audit"
  description: "Run tests, check coverage, analyze diffs, generate report"
  max_iterations: 100
```

## Guardrails

`RoutineGuardrails` enforces safety constraints before execution:

| Guardrail | Description | Default |
|-----------|-------------|---------|
| `max_duration` | Maximum execution time | 5min (lightweight) |
| `max_tool_calls` | Limit tool invocations per run | — |
| `allowed_tools` | Whitelist of tools | Subset of `autonomous_allowed_tool_names` |
| `forbidden_tools` | Explicit deny list | — |
| `require_sandbox` | Force sandbox execution | false |
| `max_concurrent` | Max concurrent runs of this routine | 1 |
| `max_daily_runs` | Max executions per day | — |
| `max_weekly_runs` | Max executions per week | — |

**Circuit-Breaking:**
- Tracks `consecutive_failures` in database
- Routine disabled after threshold (default: 3 failures)
- Auto-reenabled after manual intervention or successful run

## Notification Config

`NotifyConfig` controls post-execution notifications:

```yaml
notify:
  on_success: true      # Notify when routine completes successfully
  on_failure: true      # Notify on error/timeout
  always: false         # Notify on every run regardless of outcome
  channel: telegram     # Target channel (defaults to routine's channel)
```

**Notification delivery:**
- Sent via the routine's configured channel
- Includes execution summary (duration, result, errors)
- Full-job routines can attach detailed reports

## Runtime State (Database-Managed)

Each routine tracks persistent state:

| Field | Type | Description |
|-------|------|-------------|
| `last_run_at` | `TIMESTAMP` | Last successful execution timestamp |
| `next_fire_at` | `TIMESTAMP` | Next scheduled fire time (cron only) |
| `run_count` | `INTEGER` | Total successful executions |
| `consecutive_failures` | `INTEGER` | Failure counter for circuit-breaking |
| `state` | `JSON` | Routine-specific persistence blob |

**State updates:**
- Atomic updates via database transactions
- `last_run_at` and `run_count` incremented on success
- `consecutive_failures` reset on success, incremented on failure
- `next_fire_at` recalculated from cron schedule after each run

## Engine Execution Flow

### Cron Ticker Loop

**Location:** `routine_engine.rs::spawn_cron_ticker()`

```rust
loop {
    sleep(interval).await;
    
    let due_routines = db.query_due_cron_routines().await?;
    
    for routine in due_routines {
        if check_guardrails(&routine).await? {
            match routine.action {
                Lightweight { .. } => execute_lightweight(&routine).await?,
                FullJob { .. } => execute_full_job(&routine).await?,
            }
            notify_user(&routine, result).await?;
            db.update_runtime_state(&routine, Success).await?;
        }
    }
}
```

**Steps:**
1. Poll database every N seconds (configurable, default: 30s)
2. Query routines where `next_fire_at <= NOW()`
3. For each due routine:
   - Check guardrails (concurrent runs, daily limits, etc.)
   - Execute (lightweight inline, full-job via scheduler)
   - Send notification if configured
   - Update runtime state in database

### Event Matcher

**Location:** Called from `agent_loop.rs` on each `IncomingMessage`

```rust
if let Some(engine) = agent.routine_engine().await {
    let fired = engine.match_event_routines(&message).await;
    if fired > 0 {
        tracing::info!("Fired {} event routines", fired);
    }
}
```

**Steps:**
1. On each incoming message, call `routine_matches_message()`
2. Check if message content matches any event routine's regex pattern
3. Apply channel and user filters
4. Execute matching routines inline (synchronously)
5. Event routines run **before** the agentic turn starts

### System Event Emitter

**Location:** `routine_engine.rs::emit_system_event()`

```rust
pub async fn emit_system_event(
    &self,
    source: &str,
    event_type: &str,
    payload: &serde_json::Value,
    user_id: Option<&str>,
) -> usize {
    // 1. Check cache for system-event matchers
    // 2. Batch-query concurrent run counts
    // 3. Filter by source + event_type (case-insensitive)
    // 4. Apply user scope filter
    // 5. Match payload filters (key-value)
    // 6. Fire matching routines
    // 7. Return count of fired routines
}
```

**Usage:**
```rust
// Emit from a tool
engine.emit_system_event("github", "issue.opened", &payload, Some(&user_id)).await;

// Emit from webhook handler
engine.emit_system_event("webhook", "deploy.complete", &payload, Some(&user_id)).await;
```

## Adding a New Routine

### Step 1: Define Routine Structure (if new fields needed)

Most routines use existing `Routine` struct. Add fields only if truly necessary:

```rust
// routine.rs
pub struct Routine {
    pub id: Uuid,
    pub user_id: String,
    pub name: String,
    pub trigger: Trigger,
    pub action: RoutineAction,
    pub guardrails: RoutineGuardrails,
    pub notify: NotifyConfig,
    // Add new fields here if needed
}
```

### Step 2: Add Trigger Parsing

```rust
// routine.rs::Trigger::from_db()
"system_event" => {
    let source = get_str("source")?;
    let event_type = get_str("event_type")?;
    let filters = get_json_object("filters")?.unwrap_or_default();
    Ok(Trigger::SystemEvent { source, event_type, filters })
}
```

### Step 3: Implement Execution Logic

```rust
// routine_engine.rs
async fn execute_lightweight(&self, routine: &Routine) -> Result<String, RoutineError> {
    // 1. Build prompt with context
    // 2. Call LLM (via cheap_llm or main llm)
    // 3. Process tool calls if use_tools=true
    // 4. Return result string
}

async fn execute_full_job(&self, routine: &Routine) -> Result<JobHandle, RoutineError> {
    // 1. Create job context
    // 2. Dispatch to scheduler
    // 3. Return job handle for tracking
}
```

### Step 4: Add Guardrail Checks

```rust
// routine_engine.rs::check_guardrails()
async fn check_guardrails(&self, routine: &Routine) -> Result<(), RoutineError> {
    // Check concurrent runs
    let concurrent = self.db.count_concurrent_runs(routine.id).await?;
    if concurrent >= routine.guardrails.max_concurrent {
        return Err(RoutineError::GuardrailViolated("max_concurrent".into()));
    }
    
    // Check daily runs
    let daily = self.db.count_daily_runs(routine.id).await?;
    if daily >= routine.guardrails.max_daily_runs {
        return Err(RoutineError::GuardrailViolated("max_daily_runs".into()));
    }
    
    Ok(())
}
```

### Step 5: Wire Up Notification

```rust
// routine_engine.rs::notify_user()
async fn notify_user(&self, routine: &Routine, result: &ExecutionResult) -> Result<(), RoutineError> {
    let should_notify = match result {
        ExecutionResult::Success => routine.notify.on_success || routine.notify.always,
        ExecutionResult::Failure(_) => routine.notify.on_failure || routine.notify.always,
    };
    
    if !should_notify {
        return Ok(());
    }
    
    let message = format!("Routine '{}' completed: {}", routine.name, result.summary());
    self.channel_manager.send_message(&routine.notify.channel, &message).await?;
    
    Ok(())
}
```

### Step 6: Add Database Migration (if schema changes)

```sql
-- src/db/migrations/YYYYMMDD_add_routine_field.sql
ALTER TABLE routines ADD COLUMN new_field TEXT DEFAULT NULL;
```

## Routine Examples

### Example 1: Daily Standup Reminder (Cron + Lightweight)

```yaml
name: daily-standup
trigger:
  cron: "0 9 * * MON-FRI"  # 9 AM on weekdays
  timezone: "America/Sao_Paulo"
action:
  type: lightweight
  prompt: |
    List all incomplete tasks from the last 24h and ask the user to prioritize.
    Format as a numbered list with estimated effort for each.
  context_paths:
    - workspace/tasks/pending.md
    - workspace/notes/current-focus.md
  max_tokens: 2048
  use_tools: true
  max_tool_rounds: 2
guardrails:
  max_daily_runs: 1
  sandbox_required: false
notify:
  on_success: true
```

**Expected behavior:**
- Fires at 9 AM on weekdays (Sao Paulo timezone)
- Reads pending tasks from workspace
- Uses tools to query task database if needed
- Sends reminder via configured channel
- Skips if already run today (max_daily_runs guard)

### Example 2: Urgent Message Alert (Event-triggered + Full Job)

```yaml
name: urgent-alert
trigger:
  event:
    channel: telegram
    pattern: "(urgente|emergência|prioridade máxima|socorro)"
action:
  type: full_job
  title: "Processar Mensagem Urgente"
  description: |
    Analisar mensagem urgente, verificar contexto relevante,
    identificar ações necessárias e gerar plano de ação imediato.
  max_iterations: 30
guardrails:
  max_concurrent: 1
  max_daily_runs: 10
  sandbox_required: true
  allowed_tools:
    - file_read
    - file_write
    - shell
    - web_search
notify:
  always: true
```

**Expected behavior:**
- Fires on Telegram messages containing urgent keywords
- Spawns full background job with agentic loop
- Runs in sandbox for safety
- Notifies user regardless of outcome
- Limited to 10 runs/day to prevent spam

### Example 3: Weekly Repository Audit (Cron + Full Job)

```yaml
name: weekly-audit
trigger:
  cron: "0 10 * * MON"  # Monday 10 AM
action:
  type: full_job
  title: "Auditoria Semanal do Repositório"
  description: |
    Executar análise completa do repositório:
    1. Rodar testes e verificar coverage
    2. Analisar diffs da semana anterior
    3. Verificar dependências desatualizadas
    4. Gerar report consolidado em workspace/audits/
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
    - web_search
notify:
  on_success: true
  on_failure: true
```

**Expected behavior:**
- Fires every Monday at 10 AM
- Runs comprehensive audit as background job
- Writes report to workspace
- Notifies on both success and failure
- Requires sandbox for shell access

### Example 4: GitHub Issue Triage (System Event + Lightweight)

```yaml
name: github-issue-triage
trigger:
  system_event:
    source: github
    event_type: issue.opened
    filters:
      repository: "myorg/myrepo"
action:
  type: lightweight
  prompt: |
    A new GitHub issue was opened. Analyze the issue content:
    1. Categorize as: bug, feature, question, or other
    2. Estimate priority: critical, high, medium, low
    3. Suggest initial labels
    4. Draft a response acknowledging receipt
    
    Issue: {{payload.title}} - {{payload.body}}
  max_tokens: 1024
  use_tools: true
  max_tool_rounds: 1
guardrails:
  max_concurrent: 5
  sandbox_required: false
notify:
  on_success: false  # Silent triage, results logged only
```

**Expected behavior:**
- Fires when GitHub webhook emits `issue.opened` event
- Filters to specific repository
- Performs lightweight triage inline
- Uses tools to apply labels via GitHub API
- No user notification (silent automation)

### Example 5: Manual Ad-Hoc Analysis (Manual Trigger)

```yaml
name: adhoc-workspace-analysis
trigger:
  manual: true
action:
  type: lightweight
  prompt: |
    Analyze the current workspace state and suggest improvements:
    1. Review recent activity logs
    2. Identify stalled tasks or blockers
    3. Suggest next actions based on current focus
    4. Highlight any anomalies or concerns
  context_paths:
    - workspace/notes/current-focus.md
    - workspace/logs/recent-activity.md
  max_tokens: 4096
  use_tools: true
  max_tool_rounds: 5
guardrails:
  sandbox_required: false
```

**Expected behavior:**
- Only fires when explicitly invoked via tool call
- Useful for on-demand workspace check-ins
- No automatic triggers
- Full tool access for comprehensive analysis

## Testing Routines

### Unit Tests

Test trigger parsing and serialization:

```rust
#[test]
fn test_routine_cron_trigger_parse() {
    let trigger = Trigger::Cron {
        schedule: "0 9 * * MON-FRI".to_string(),
        timezone: Some("America/Sao_Paulo".to_string()),
    };
    assert!(matches!(trigger, Trigger::Cron { .. }));
}

#[test]
fn test_routine_system_event_trigger_roundtrip() {
    let trigger = Trigger::SystemEvent {
        source: "github".to_string(),
        event_type: "issue.opened".to_string(),
        filters: HashMap::new(),
    };
    
    let serialized = trigger.to_db_values();
    let parsed = Trigger::from_db("system_event", serialized).unwrap();
    
    assert!(matches!(parsed, Trigger::SystemEvent { source, event_type, .. } 
        if source == "github" && event_type == "issue.opened"));
}
```

### Integration Tests

Test full routine lifecycle:

```rust
#[tokio::test]
async fn test_routine_lightweight_execution() {
    let engine = create_test_engine().await;
    
    let routine = Routine {
        name: "test-routine".into(),
        trigger: Trigger::Manual { .. },
        action: RoutineAction::Lightweight {
            prompt: "Say hello".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    
    let result = engine.execute_lightweight(&routine).await.unwrap();
    assert!(!result.is_empty());
}
```

See `src/agent/routine_integration_tests.rs` for comprehensive examples.

## Key Invariants

1. **Routines fire independently of user sessions** — they don't hold session locks
2. **Lightweight routines execute synchronously** in the agent loop — keep them fast (<5s)
3. **Full-job routines are asynchronous** — safe for long-running operations
4. **Guardrails are checked before execution** — failures return `RoutineError::GuardrailViolated`
5. **Consecutive failures trigger circuit-breaking** — routine disabled after threshold (default: 3)
6. **Event routines match case-insensitively** on message content
7. **Cron schedules use user-configured timezone** (defaults to UTC)
8. **System event matching is optimized** — batch queries, early returns, case-insensitive comparison
9. **Runtime state is atomically updated** — database transactions prevent race conditions
10. **Notification failures are logged but non-fatal** — routine execution succeeds even if notification fails

## Troubleshooting

### Routine Not Firing

**Check:**
1. Is the routine enabled in the database? (`SELECT enabled FROM routines WHERE id = ?`)
2. Has it hit `max_daily_runs` or `max_concurrent` guardrails?
3. For cron: is `next_fire_at` in the past? (`SELECT next_fire_at FROM routines WHERE id = ?`)
4. For event: does the message actually match the regex pattern? (test with regex101.com)
5. For system_event: is the source/event_type spelled correctly? (case-insensitive but exact match required)

### Routine Stuck in Concurrent Run

**Symptoms:** Routine won't fire, `concurrent_runs > 0` but no active execution

**Fix:**
```sql
-- Reset concurrent run counter
UPDATE routines SET concurrent_runs = 0 WHERE id = '<routine_id>';
```

### Circuit-Breaker Triggered

**Symptoms:** Routine disabled after consecutive failures

**Fix:**
```sql
-- Reset failure counter and re-enable
UPDATE routines SET consecutive_failures = 0, enabled = true WHERE id = '<routine_id>';
```

**Root cause analysis:** Check routine execution logs for the underlying error:
```sql
SELECT * FROM routine_logs WHERE routine_id = '<routine_id>' ORDER BY created_at DESC LIMIT 10;
```

### Notification Not Sent

**Check:**
1. Is `notify.on_success` or `notify.on_failure` set correctly?
2. Is the target channel configured and connected?
3. Check notification logs: `SELECT * FROM notification_logs WHERE routine_id = ?`

## Performance Considerations

**Cron ticker:**
- Default poll interval: 30 seconds
- Batch queries for due routines (single query, not N queries)
- Early return if no routines due

**Event matcher:**
- Cached event matchers (refreshed on routine create/update/delete)
- Case-insensitive regex compiled once per routine
- Early exit if no event routines registered

**System event emitter:**
- Batch query for concurrent run counts (single query for all matching routines)
- Case-insensitive string comparison (`.eq_ignore_ascii_case()`)
- Payload filter matching short-circuits on first mismatch

**Guardrails:**
- Database-level counters (not in-memory)
- Atomic increment/decrement to prevent race conditions
- Configurable limits per routine

## Security Considerations

1. **Tool allowlists** — restrict which tools routines can use
2. **Sandbox requirements** — force isolation for untrusted operations
3. **Rate limiting** — prevent abuse via `max_daily_runs` and `max_concurrent`
4. **User scoping** — routines isolated by `user_id` in multi-user deployments
5. **Webhook secrets** — validate HMAC signatures on webhook triggers
6. **Credential injection** — use `CredentialInjector` for safe secret handling in routines

## Related Documentation

- [`src/agent/CLAUDE.md`](../src/agent/CLAUDE.md) — Agent module overview with routine integration details
- [`docs/SSE_EVENT_SYSTEM.md`](./SSE_EVENT_SYSTEM.md) — Real-time event broadcast system
- [`src/webhooks/mod.rs`](../src/webhooks/mod.rs) — Webhook ingress for system events
- [`src/tools/builtin/routine.rs`](../src/tools/builtin/routine.rs) — Routine tool implementation
