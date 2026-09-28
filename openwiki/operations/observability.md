---
type: operations-guide
title: Observability, Debugging & Logs
description: Guide to event sourcing, structured logging, event projections, debug bundles, and techniques for troubleshooting agent behavior and diagnosing issues in production.
tags: [observability, debugging, logging, event-sourcing, tracing, events, projections]
verified:
  - by: openwiki/0.6.0
    at: 2026-09-28T08:16:43.565Z
sources:
  - id: openwiki-source-790146f7f4bc594715f8b5dd
    resource: repo://crates/events/ironclaw_event_log/README.md
  - id: openwiki-source-0442b291f39341f9d8645860
    resource: repo://crates/events/ironclaw_event_log/src/lib.rs
  - id: openwiki-source-76d4217cb9fe5244a9e71f42
    resource: repo://crates/events/ironclaw_event_log/src/runtime_event.rs
  - id: openwiki-source-49e49fc90c661adfa187da93
    resource: repo://crates/events/ironclaw_event_log/src/sink.rs
  - id: openwiki-source-de89fb85d3423e6af9502ff6
    resource: repo://crates/events/ironclaw_event_projections/README.md
  - id: openwiki-source-d408668254972e1be9ab35cd
    resource: repo://crates/events/ironclaw_event_projections/src/lib.rs
  - id: openwiki-source-015b0b19aac0c6621cb3c714
    resource: repo://crates/substrates/ironclaw_observability/README.md
  - id: openwiki-source-cdad48c80ac49c47b0814f5c
    resource: repo://crates/substrates/ironclaw_observability/src/lib.rs
  - id: openwiki-source-36af6e056ea227247885af9c
    resource: repo://docs/internal/reborn/harness/local-dev.md
  - id: openwiki-source-a8dcb0e0f82eac282904addb
    resource: repo://docs/internal/reborn/harness/observability.md
  - id: openwiki-source-54d934d4881dc328bdd47bda
    resource: repo://tests/fixtures/llm_traces/README.md
generated: { by: "openwiki/0.6.0", at: "2026-09-28T08:16:43.565Z" }
---

# Observability, Debugging & Logs

This page guides you through Reborn's observability primitives — event sourcing as an audit trail, structured logging, event projections as read models, and debug bundle collection for troubleshooting.

## Event Sourcing as Audit Trail

Every user turn, system run, tool call, and side effect is recorded as an immutable event in the durable event log. This creates a complete, replayable record of what happened and why — the foundation for debugging, auditing, and deterministic testing.

### Event Recording Pipeline

```mermaid
sequenceDiagram
    participant Runner as TurnRunner
    participant Loop as AgentLoop
    participant Dispatch as RuntimeDispatcher
    participant EventSink as EventSink
    participant DurableLog as DurableEventLog
    
    Runner->>Loop: invoke_driver(run)
    activate Loop
    Loop->>Loop: Assemble prompt<br/>Call model
    Loop->>Dispatch: request_capability(tool_id)
    activate Dispatch
    Dispatch->>Dispatch: Authorize & execute
    Dispatch->>EventSink: emit(CapabilityActivityRequested)
    EventSink-->>EventSink: best-effort
    Dispatch->>EventSink: emit(CapabilityActivitySucceeded)
    Dispatch-->>Loop: CapabilityOutcome
    deactivate Dispatch
    deactivate Loop
    
    Runner->>EventSink: emit(LoopCompleted)
    
    EventSink->>DurableLog: append(RuntimeEvent)
    DurableLog->>DurableLog: Persist to backend
    DurableLog-->>EventSink: EventLogEntry
    
    Note over EventSink: Sink failure is<br/>best-effort only<br/>never outcome-altering
```

Events are recorded at every major step: turn submission, capability requests, runtime dispatch outcomes, model calls, and loop termination. Each event includes correlation IDs (turn_id, run_id, invocation_id) so you can trace a user action through all its side effects.

**Key invariant:** Event sink failures must never alter runtime outcomes. Producers emit events best-effort only; append failures are recorded but do not block the surrounding workflow.

### Event Kinds and Correlation Fields

Runtime events record one of these event kinds:

- **Turn lifecycle:** `LoopCompleted`, `LoopCancelled`, `LoopFailed`
- **Capability activity:** `CapabilityActivityRequested`, `CapabilityActivitySucceeded`, `CapabilityActivityFailed`
- **Dispatch:** `DispatchRequested`, `RuntimeSelected`, `DispatchSucceeded`, `DispatchFailed`
- **Model calls:** `ModelStarted`, `ModelCompleted`, `ModelFailed`
- **Assistant replies:** `AssistantReplyFinalized`
- **Background processes:** `ProcessStarted`, `ProcessCompleted`, `ProcessFailed`, `ProcessKilled`
- **Hooks:** `HookDispatched`, `HookDecisionEmitted`, `HookFailed`
- **Recovery:** `FailureRecovered`

Every event includes these correlation fields so you can reconstruct what happened to a given user's turn:

```text
tenant_id          → Which tenant
user_id            → Which user in that tenant
project_id         → Which project
thread_id          → Which conversation thread (mission)
turn_id            → Which user turn (optional, per-runtime)
run_id             → Which execution (invocation_id)
invocation_id      → Specific invocation in the DAG
capability_id      → Which capability was invoked
extension_id       → Which extension provided it (if applicable)
runtime_kind       → Which runtime executed (MCP, WASM, Script, etc.)
process_id         → Background process if applicable
lease_id           → Runner lease token for multi-attempt recovery
```

### Redaction at Construction

All events are sanitized at construction time to prevent sensitive data from leaking into logs or durable storage. The following are **never** recorded:

- Raw secrets or API keys
- Bearer tokens or OAuth refresh tokens
- Host filesystem paths (virtual paths are safe)
- Raw error messages that expose infrastructure details
- Approval reasons or unapproved input/output
- Private network configuration beyond policy diagnostics
- Lease contents or invocation fingerprints

Constructor methods like `RuntimeEvent::with_error_summary()` automatically call sanitizers. Serialization re-runs sanitization on the wire to enforce the invariant even if a caller constructs a struct directly and bypasses the constructor.

### Durable Storage Backends

Events persist to one of several backends depending on deployment:

- **In-memory** (tests only): `InMemoryDurableEventLog`
- **JSONL** (single-node dev): Append-only JSON lines file
- **PostgreSQL** (production): SQL schema with partitioning
- **libSQL** (embedded/edge): SQLite-compatible with replication

Local dev and test use in-memory or JSONL. Production must never silently fall back to in-memory storage; backend selection fails closed if the configured backend is unavailable.

Read the reference: [`ironclaw_event_log` README](repo://crates/events/ironclaw_event_log/README.md)

---

## Structured Logging: Levels and Targets

Reborn uses Rust's `tracing` crate for structured logging. Log output is controlled by the `RUST_LOG` environment variable, which filters by **target** (crate name) and **level** (trace, debug, info, warn, error).

### Logging Levels

| Level | When to use | Example |
|-------|------------|---------|
| `TRACE` | Every operation detail (hot loops should not use this) | `Calling model gateway with 12 tokens` |
| `DEBUG` | High-level operation steps and state transitions | `Turn admitted, queued for execution` |
| `INFO` | Notable events that operators should see | `Agent loop completed successfully` |
| `WARN` | Recoverable issues or degraded behavior | `Event sink delivery failed, continuing` |
| `ERROR` | Fatal errors that require attention | `Loop execution failed: auth denied` |

### Setting RUST_LOG Patterns

```bash
# Everything at DEBUG or higher
RUST_LOG=debug cargo test

# Only ironclaw_loop_host and ironclaw_turn_runner at TRACE; others at WARN
RUST_LOG=warn,ironclaw_loop_host=trace,ironclaw_turn_runner=trace cargo test

# Only latency traces (zero-cost when off)
RUST_LOG=ironclaw_latency=trace cargo test

# Trace tracing span events and structured fields
RUST_LOG=debug,tracing=trace cargo test

# All events, very verbose
RUST_LOG=trace cargo test
```

The `RUST_LOG` syntax is a comma-separated list of `[target]=[level]` filters, with earlier patterns overriding later ones. A bare level like `debug` sets the default; specific targets with `crate_name=level` override it.

### Latency Tracing

The `ironclaw_observability` crate provides zero-cost-when-off latency macros for timing operations without acquiring a tracing dependency:

```rust
use ironclaw_observability::{live_latency_trace_ok, elapsed_ms};

let started_at = ironclaw_observability::live_latency_started_at();
// ... do work ...
live_latency_trace_ok!(
    "dispatcher",
    "capability_dispatch",
    started_at,
    capability_id = cap_id,
    invocation_id = inv_id
);
```

When the `ironclaw_latency` target is not enabled in `RUST_LOG`, these macros compile to zero instructions. Enable latency traces with:

```bash
RUST_LOG=ironclaw_latency=trace cargo run
```

### Crates and Their Log Levels

Different subsystems use different default levels:

- **Runtime critical paths** (`ironclaw_turn_runner`, `ironclaw_host_runtime`): Prefer `info` or `warn` for steady-state
- **Capability dispatch** (`ironclaw_capabilities`): `info` for authorizations, `debug` for state transitions
- **Loop execution** (`ironclaw_loop_host`): `debug` for prompt assembly, `info` for terminal states
- **Approval/auth** (`ironclaw_approvals`): `info` for decision points, `warn` for blocks
- **Model gateway** (`ironclaw_llm`): `info` for model selection, `debug` for token counts

Read the source: [`ironclaw_observability` latency macros](repo://crates/substrates/ironclaw_observability/src/lib.rs)

---

## Event Projections: Read Models Over Events

Event projections are metadata-only read models computed by replaying the event log. They provide scoped, typed views of what happened to a turn or thread without touching durable storage directly.

### Projection Service

The `EventProjectionService` and `ReplayEventProjectionService` reconstruct three kinds of state:

#### Thread Timeline
A chronological list of all events in a thread (conversation), with `TimelineEntry` objects capturing:
- Event ID and timestamp
- Event kind (capability activity, process completion, etc.)
- Invocation and thread IDs
- Error classification (if the event is a failure)
- Hook metadata (if the event is a hook decision)

```rust
pub struct ThreadTimeline {
    pub entries: Vec<TimelineEntry>,
}

pub struct TimelineEntry {
    pub event_id: RuntimeEventId,
    pub timestamp: Timestamp,
    pub kind: TimelineEntryKind,
    pub invocation_id: InvocationId,
    pub error_kind: Option<String>,  // Closed vocabulary: "auth_denied", etc.
    pub hook_id: Option<String>,      // Blake3 hash of hook identity
    pub hook_point: Option<String>,   // "before_capability", "after_loop", etc.
    pub hook_decision: Option<String>, // "allow", "deny", "pause_approval", etc.
}
```

#### Run Status
A summary of a single execution including:
- Current state (Queued, Running, Completed, Failed, Cancelled)
- Start and end timestamps
- Resource usage (token counts, output bytes)
- Terminal error (if failed)
- Approval lease status (if blocked on approval)

#### Capability Activity
A projection of a specific capability call with:
- Request timestamp and invocation ID
- Runtime kind and capability ID
- Authorization and execution outcome
- Error classification if it failed

### Scoped Reads and Cursors

Projections are always scoped to a specific `(tenant, user, agent)` stream and may be further filtered by project, thread, or process:

```rust
pub struct ProjectionScope {
    pub stream: EventStreamKey,      // (tenant, user, agent)
    pub read_scope: ReadScope,        // (project, thread, process)
}
```

Callers cannot observe neighboring projects' or threads' events. Scope mismatches are detected and return an error (`ProjectionError::RebaseRequired`) so a consumer knows it must re-snapshot rather than silently see partial data.

Every projection call returns a cursor for the next read:

```rust
pub async fn updates(
    service: &dyn EventProjectionService,
    request: ProjectionRequest,
) -> Result<ProjectionReplay, ProjectionError> {
    // ...returns events after the cursor...
    Ok(ProjectionReplay {
        updates: vec![...],  // New timeline entries since last cursor
        runs: vec![...],     // All run states in the scope
        next_cursor: ...,    // Pass this to the next updates() call
        truncated: false,    // true if we hit the page limit
    })
}
```

### Read-Model Invariants

Event projections are **provably non-writing**. The `ironclaw_event_projections` crate has no dependency on any durable writer (no `ironclaw_event_store`, no `ironclaw_filesystem`). A projection failure is observable but never mutating.

Projection output is **metadata-only**: it never includes raw inputs/outputs, host paths, secrets, approval reasons, invocation fingerprints, or backend detail strings. The one display exception is `error_detail`, which carries only the sanitized `RuntimeEvent` error summary and is re-sanitized during replay.

Read the reference: [`ironclaw_event_projections` README](repo://crates/events/ironclaw_event_projections/README.md)

---

## Debug Bundle Collection

A **debug bundle** is a portable, redacted artifact containing logs, events, configuration, and process state. It lets an agent answer: "What happened during this run?" without access to a live system.

### What Goes Into a Bundle

Future tooling will collect the following under `.pi/reborn-dev/artifacts/reborn-debug/<timestamp>/`:

```text
config-redacted.json          Deployment configuration with secrets removed
logs.jsonl                    Structured logs in JSON Lines format
events.jsonl                  Runtime events (turn, capability, process, hook)
audit.jsonl                   Security audit and approval decisions
process-tree.json             Background process states and results
failed-invocations.json       Detailed failure classification and stack traces
screenshots/                  Browser artifacts from E2E runs
replay-command.txt            Command to replay the session deterministically
```

Each entry is redacted to remove:
- Raw secrets and OAuth tokens
- Host filesystem paths (virtual paths preserved)
- Approval lease contents and private reasoning
- Unapproved input/output
- Backend error internals

### Doctor Command

The doctor command (planned) will create a debug bundle:

```bash
scripts/reborn-dev doctor
# or
ironclaw reborn doctor --bundle
```

Output: `.pi/reborn-dev/artifacts/reborn-debug/<timestamp>/`

An agent should be able to use the bundle to:
- Identify which tenant, user, project, agent, and thread were involved
- Trace a user message through turn submission, loop execution, and capability calls
- Find which capability or runtime failed and why
- Determine if the failure is authorization, approval, networking, or code-level
- Reproduce the session with the recorded trace (if replay is available)

See plan: [`reborn-dev doctor`](repo://docs/internal/reborn/harness/local-dev.md#doctor-bundle)

---

## Replay Fixtures: Deterministic LLM Traces

For testing without a live LLM, Reborn uses **LLM trace fixtures** — JSON files that script model responses and replay them in order. This enables deterministic E2E testing of the full agent loop without external API calls.

### Trace Format

A trace file (`tests/fixtures/llm_traces/*.json`) contains a model name and a list of turns. Each turn pairs a user message with a sequence of LLM response steps:

```json
{
  "model_name": "spot-tool-chain",
  "turns": [
    {
      "user_input": "Write the date to /tmp/test.txt and then read it back",
      "steps": [
        {
          "response": {
            "type": "tool_calls",
            "tool_calls": [
              {
                "id": "c1",
                "name": "write_file",
                "arguments": {"path": "/tmp/test.txt", "content": "2025-01-21"}
              }
            ],
            "input_tokens": 120,
            "output_tokens": 50
          }
        },
        {
          "response": {
            "type": "tool_calls",
            "tool_calls": [
              {
                "id": "c2",
                "name": "read_file",
                "arguments": {"path": "/tmp/test.txt"}
              }
            ],
            "input_tokens": 140,
            "output_tokens": 45
          }
        },
        {
          "response": {
            "type": "text",
            "content": "Done! I wrote the date and read it back: 2025-01-21",
            "input_tokens": 160,
            "output_tokens": 20
          }
        }
      ]
    }
  ]
}
```

### Two Artifacts Per Scenario

A trace scenario has two files with different owners and purposes:

| File | Owner | Changes When | Purpose |
|------|-------|--------------|---------|
| `tests/fixtures/llm_traces/spot-tool-chain.json` | `RecordingLlm` or hand-written | LLM model changes, re-recording | Scripts what the LLM would say (the stub) |
| `tests/snapshots/replay__spot_tool_chain.snap` | `insta` via `ReplayOutcome` | Engine dispatch, tools, safety changes | Records what the agent actually did (the contract) |

The JSON is the **driver** — it prescribes LLM behavior. The snapshot is the **regression contract** — it captures agent behavior and is what reviewers diff on every PR that touches engine or tool code.

Editing the JSON without regenerating the snapshot is a code smell. It means the regression the snapshot pinned moved. Run `cargo insta review` to inspect and accept the drift.

### Recording a New Fixture

```bash
scripts/replay-snap.sh record <name>
# or manually with the harness
cargo test --test <test> --features recording
```

The recorded fixture includes extras (memory snapshots, HTTP exchanges, expected tool results) that enable fully deterministic replay.

### Replay Commands

```bash
scripts/replay-snap.sh review       # Interactive diff of pending snapshots (cargo insta review)
scripts/replay-snap.sh accept       # Accept all pending snapshots
scripts/replay-snap.sh test         # Run the replay gate locally (cargo insta test --check)
scripts/replay-snap.sh record <name> # Record a fresh fixture
```

Read the guide: [`LLM Trace Fixtures`](repo://tests/fixtures/llm_traces/README.md)

---

## Observability Best Practices

### When to Use Events vs. Logs

| Use Events | Use Logs |
|-----------|----------|
| Turn submission, admission, state transitions | Prompt assembly, model API calls |
| Capability dispatch, authorization decisions | Token counting, memory search |
| Side effect outcomes, approval gates | Configuration loaded, service started |
| Failures and recoveries (audit trail) | Performance metrics, latency traces |

Events are **durable** and **scoped by tenant**. Use them for anything an operator or auditor will need to query later. Logs are **ephemeral** and **system-wide**. Use them for debugging during development or triage.

### Correlation IDs in Logs

When you emit a log that touches a turn or run, include the correlation IDs:

```rust
tracing::info!(
    tenant_id = %scope.tenant_id,
    user_id = %scope.user_id,
    project_id = %scope.project_id,
    thread_id = %scope.mission_id,
    turn_id = %run_id,
    invocation_id = %invocation_id,
    "Capability authorization granted"
);
```

This allows log aggregation systems to reconstruct the full trace of a user's turn.

### Error Redaction in Events

Always use the sanitizers when constructing error events:

```rust
use ironclaw_event_log::sanitize_error_kind;

let error_kind = sanitize_error_kind("connection_refused_to_backend_db");  // Safe
let error_kind = sanitize_error_kind(&format!("Failed: {}", err));         // Unsafe, returns Unclassified
```

For free-form error summaries, use bounded display strings:

```rust
let summary = match err {
    DispatchError::AuthDenied => Some("Authorization policy blocked this capability"),
    DispatchError::ResourceLimit => Some("Resource limit exceeded"),
    _ => None,
};
```

### Failure Classification

Emit events with closed-vocabulary failure categories so downstream analysis can pattern-match without parsing free-form text:

```rust
RuntimeEvent {
    kind: RuntimeEventKind::CapabilityActivityFailed,
    error_kind: Some("auth_denied"),        // Closed vocabulary
    error_summary: Some("Policy denied"),   // Display-safe summary
    // ...
}
```

Supported categories:
- `authorization`, `approval`, `auth_blocked`, `resource_limit`, `network_policy`
- `secret_unavailable`, `filesystem`, `memory`, `runtime_dispatch`, `process`
- `provider`, `event_sink`, `projection`, `transport`, `unclassified`

---

## Observability Configuration in Production

### Event Sink Selection

Composition selects a durable event sink based on features and configuration. Backends are validated at startup — no silent fallback to in-memory:

```rust
// Pseudocode: composition validates the selected backend
let backend = match config.event_backend {
    "postgres" => {
        #[cfg(feature = "postgres")]
        PostgresEventLog::connect(config.database_url).await?
        #[cfg(not(feature = "postgres"))]
        return Err("postgres backend requested but postgres feature not enabled")
    }
    "libsql" => { /* ... */ }
    "jsonl" => { /* local dev only */ }
    _ => return Err("unknown event backend"),
};
```

Production must use PostgreSQL or libSQL. JSONL is accepted only when explicitly configured and is safe for single-node deployments.

### Audit Sink Best-Effort vs. Fail-Closed

**Observability failures must not silently change security semantics.**

- **Best-effort sinks** (general events, logs): Failure is observable but must not alter runtime outcomes. Use `emit(...).await` and record any error; continue with the original success/failure result.
- **Fail-closed sinks** (audit, approval decisions): Append failure must follow the domain contract. Approval resolution and authorization do not continue if the audit record cannot be durably persisted.

---

## Reference: Harness Observability Design

See the complete target design document: [`docs/internal/reborn/harness/observability.md`](repo://docs/internal/reborn/harness/observability.md)

It covers:
- Common correlation field vocabulary
- Evidence surface tiers (logs, events, audit, process records, replay snapshots, doctor bundles)
- Redaction rules and failure classification
- Durable store evidence and backend selection
