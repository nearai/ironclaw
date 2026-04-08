# Self-Improving Engine

This document describes how IronClaw improves itself at runtime — fixing bugs, evolving prompts, and patching its own execution loop without a Rust rebuild.

## The Problem

During development, 5 consecutive debugging sessions revealed the same pattern:

1. A thread runs and hits a bug (wrong tool name, bad output format, UTF-8 crash)
2. The LLM tries to work around it but can't fix the Rust code
3. A human reads the trace, identifies the root cause, edits Rust, rebuilds
4. The fix takes effect on the next run

Every step of this loop is something the engine can do. The key insight: **if the orchestration layer were Python (not Rust), the engine could fix its own bugs at runtime**.

## Architecture

### Three Self-Improvement Levels

| Level | What Changes | Risk | Who Approves | Mechanism |
|-------|-------------|------|-------------|-----------|
| **1: Prompt** | System prompt rules | Low | Auto | MemoryDoc overlay appended to compiled preamble |
| **1.5: Orchestrator** | Python execution loop | Medium | Auto (3-failure rollback) | Versioned MemoryDoc, loaded at thread start |
| **2: Config** | Engine defaults, constants | Medium | Auto if tests pass | Git branch + cargo test |
| **3: Code** | Rust source in engine/bridge | High | Human via PR | Proposed, not applied |

### The Self-Improvement Mission

A built-in Mission with `OnSystemEvent` cadence fires when threads complete with issues:

```
Thread completes → Trace analysis (8 issue categories)
                → Reflection (produces Lesson/Spec/Issue docs)
                → Emit "thread_completed_with_issues" event
                        ↓
              MissionManager event listener
                        ↓
              Self-improvement Mission fires
                        ↓
              Mission thread (CodeAct, all tools)
                  ├── Reads trigger payload (trace issues + error messages)
                  ├── Checks fix pattern database for known solutions
                  ├── Diagnoses root cause (PROMPT / ORCHESTRATOR / CONFIG / CODE)
                  └── Applies fix at appropriate level
```

### Trigger Payload

The event listener builds a JSON payload containing:

```json
{
  "source_thread_id": "uuid",
  "goal": "what the thread was trying to do",
  "issues": [
    {"severity": "Error", "category": "tool_error", "description": "Tool web_search not found", "step": 1}
  ],
  "error_messages": ["web_search: no lease for action 'web_search'"],
  "reflection": {"docs_produced": 3, "doc_types": ["Summary", "Lesson", "Spec"]}
}
```

The self-improvement thread receives this as `state["trigger_payload"]` and uses it to diagnose and fix the issue.

## Level 1: Prompt Evolution

The system prompt is built from:
1. **Compiled preamble** (`include_str!` from `prompts/codeact_preamble.md`) — never modified at runtime
2. **Runtime overlay** (MemoryDoc with tag `prompt_overlay`) — appended after the preamble
3. **Tool documentation** — dynamically generated from available actions
4. **Compiled postamble** — strategy section

The self-improvement thread can append rules to the overlay:

```python
# In the self-improvement thread:
memory_write(
    title="prompt:codeact_preamble",
    content="9. Never call web_fetch — use http() instead.\n10. Always access state dict for prior results.",
    tags=["prompt_overlay"]
)
```

The overlay is capped at 4000 characters. Next thread picks up the updated prompt.

## Level 1.5: Orchestrator Patches

The execution loop itself is Python code stored as a versioned MemoryDoc:

```
v0 (compiled-in default.py)
  → v1 (self-improvement fix: better output formatting)
    → v2 (self-improvement fix: tool name alias)
      → v3 (bad fix, causes crashes)
          ↑ auto-rollback after 3 failures → back to v2
```

### Versioning

Each orchestrator version is a MemoryDoc:
- Title: `orchestrator:main`
- Tag: `orchestrator_code`
- Metadata: `{"version": N, "parent_version": N-1}`

Loading priority: highest version number wins. If the latest version has 3+ consecutive failures (tracked via `orchestrator:failures` doc), it's skipped and the previous version is loaded.

### Auto-Rollback

```
Thread starts → load_orchestrator() checks failure tracker
  ├── Latest version has < 3 failures → use it
  ├── Latest version has >= 3 failures → skip, try previous
  └── All versions failed → use compiled-in v0

Thread succeeds → reset failure counter
Thread fails → increment failure counter for current version
```

### What the Orchestrator Controls

The Python orchestrator handles all the "glue" between the LLM and tools:

- **Tool dispatch**: How function calls are resolved and executed
- **Output formatting**: How tool results are presented to the LLM
- **State management**: How variables persist across code steps
- **Truncation**: How large outputs are compacted
- **FINAL() extraction**: How termination signals are parsed
- **Nudge detection**: When to prompt the LLM to write code instead of describing

These are exactly the functions that had bugs during development (wrong tool names, JSON double-serialization, UTF-8 panics, missing state). Now they can be fixed at runtime.

## Level 2: Configuration Tuning

The self-improvement thread can create git branches and modify engine defaults:

```python
# In the self-improvement thread:
shell("git checkout -b self-improve/increase-truncation")
read_file("crates/ironclaw_engine/src/executor/scripting.rs")
apply_patch(...)
result = shell("cargo test -p ironclaw_engine")
if "test result: ok" in result:
    shell("git commit -am 'Increase output truncation to 12000 chars'")
else:
    shell("git checkout main")
```

## Level 3: Code Patches

For Rust bugs in the engine or bridge, the self-improvement thread describes the fix but does not apply it directly. The recommendation appears in the thread's FINAL() response and in the mission's approach_history.

## Fix Pattern Database

A Note MemoryDoc maps known trace symptoms to fix strategies:

| Trace Pattern | Fix Strategy | Location |
|---|---|---|
| Tool X not found | Add name alias or prompt hint | prompt overlay or effect_adapter |
| TypeError: str indices must be integers | Parse JSON before wrapping | output conversion |
| NameError: name 'X' not defined | Add prompt hint about state dict | prompt overlay |
| byte index N is not a char boundary | Replace byte slicing with chars() | string truncation |
| Model calls nonexistent tool | Add prompt rule with correct name | prompt overlay |
| Model ignores tool results | Improve output metadata format | orchestrator |
| Excessive steps (>5) for simple task | Add prompt rule or fix tool schema | prompt overlay |
| Code error in REPL output | Add prompt hint about correct API | prompt overlay |

The database grows over time — after successfully fixing an issue, the self-improvement thread adds a new pattern entry.

## Safety Boundaries

**Hard boundaries (never auto-modify):**
- Security-sensitive code (safety layer, policy engine, leak detection)
- Database schemas / migrations
- Test files (never weaken tests to make a fix pass)
- Files outside `crates/ironclaw_engine/` and `src/bridge/` without human approval

**Orchestrator safety:**
- Auto-rollback after 3 consecutive failures
- Compiled-in v0 always available as last resort
- Each version tracked with parent_version for audit trail
- Resource limits (5min timeout, 128MB memory) on orchestrator VM

## Creating the Self-Improvement Mission

On engine init (`src/bridge/router.rs`), `ensure_self_improvement_mission()` is called. It:

1. Checks if a self-improvement mission already exists for the project
2. If not, creates one with `OnSystemEvent { source: "engine", event_type: "thread_completed_with_issues" }`
3. Seeds the fix pattern database with known patterns
4. Starts the event listener (`start_event_listener()`)

The mission is capped at 5 threads per day (`max_threads_per_day: 5`).

## Key Files

| File | Purpose |
|------|---------|
| `crates/ironclaw_engine/orchestrator/default.py` | The v0 orchestrator (self-modifiable) |
| `crates/ironclaw_engine/src/executor/orchestrator.rs` | Loading, versioning, rollback, host functions |
| `crates/ironclaw_engine/src/executor/prompt.rs` | Prompt overlay loading |
| `crates/ironclaw_engine/src/runtime/mission.rs` | Self-improvement mission, OnSystemEvent wiring, fix patterns |
| `docs/plans/2026-03-23-self-improving-engine.md` | Original design doc |
| `docs/plans/2026-03-25-python-orchestrator.md` | Python orchestrator design doc |

## Debugging Self-Improvement

Enable trace logging to see the self-improvement loop in action.
`IRONCLAW_RECORD_TRACE=1` is the unified flag — it enables `RecordingLlm`,
which captures every LLM interaction into a shared `trace_*.json` fixture
file. Engine v2 reuses the same provider chain, so its LLM calls are recorded
through the same mechanism (no separate engine trace file):

```bash
ENGINE_V2=true IRONCLAW_RECORD_TRACE=1 RUST_LOG=ironclaw_engine=debug cargo run
```

Look for:
- `"loaded runtime orchestrator"` — which version was loaded
- `"orchestrator version has too many failures, skipping"` — rollback in action
- `"self-improvement: updated prompt overlay"` — Level 1 fix applied
- `"event listener: failed to fire self-improvement"` — event listener errors
- `SelfImprovementStarted` / `SelfImprovementComplete` events in traces
