# Engine v2 Architecture

This document describes the IronClaw Engine v2 architecture for new contributors. It covers the execution model, the Python orchestrator, the bridge layer, and how everything fits together.

## Overview

IronClaw Engine v2 replaces ~10 fragmented abstractions (Session, Job, Routine, Channel, Tool, Skill, Hook, Observer, Extension, LoopDelegate) with a unified model built on 5 primitives. The engine lives in `crates/ironclaw_engine/` as a standalone crate with no dependency on the main `ironclaw` crate.

The key architectural innovation: **the execution loop is Python code running inside the Monty interpreter, not Rust**. Rust provides the infrastructure (LLM calls, tool execution, safety, persistence). Python provides the orchestration (tool dispatch, output formatting, state management). This makes the glue layer self-modifiable at runtime by the self-improvement Mission.

## Five Primitives

| Primitive | Purpose | Replaces |
|-----------|---------|----------|
| **Thread** | Unit of work with lifecycle, parent-child tree, capability leases | Session + Job + Routine + Sub-agent |
| **Step** | Unit of execution (one LLM call + its action executions) | Agentic loop iteration + tool calls |
| **Capability** | Unit of effect (actions + knowledge + policies) | Tool + Skill + Hook + Extension |
| **MemoryDoc** | Unit of durable knowledge (summaries, lessons, skills) | Workspace memory blobs |
| **Project** | Unit of context (scopes memory, threads, missions) | Flat workspace namespace |

## Execution Model

### The Two-Layer Architecture

```
Rust Layer (stable kernel — rarely changes)
  ├── LlmBackend trait     → make LLM API calls
  ├── EffectExecutor trait  → run tools with safety/policy/hooks
  ├── Store trait           → persist threads, steps, events, docs
  ├── LeaseManager          → grant/check/consume/revoke capability leases
  ├── PolicyEngine          → deterministic allow/deny/require-approval
  ├── ThreadManager         → spawn, stop, inject messages, join threads
  ├── Monty VM              → embedded Python interpreter
  └── Safety layer          → sanitization, leak detection, policy enforcement

Python Layer (self-modifiable orchestrator — where bugs get fixed)
  ├── The step loop         → call LLM → handle response → repeat
  ├── Tool dispatch         → name resolution, alias mapping
  ├── Output formatting     → truncation, context assembly
  ├── State management      → persisted_state dict across code steps
  ├── FINAL() extraction    → parse termination signals from text
  ├── Tool intent nudging   → detect when LLM describes instead of acts
  └── Doc injection         → format memory docs for context
```

### How It Works

1. **Bootstrap** (`ExecutionLoop::run()` in `loop_engine.rs`, ~80 lines):
   - Transition thread to Running state
   - Inject CodeAct system prompt (with runtime prompt overlay if available)
   - Load versioned Python orchestrator from Store (or compiled-in default)
   - Execute orchestrator via Monty VM
   - Map return value to `ThreadOutcome`
   - Persist final state

2. **Orchestrator** (`orchestrator/default.py`, ~230 lines):
   - Calls host functions to interact with Rust infrastructure
   - Runs the step loop: check signals → check budget → call LLM → handle response
   - For text responses: extract FINAL(), check nudge, or complete
   - For code responses: run user code in nested Monty VM, format output
   - For action calls: execute each action, handle approval flow
   - Returns outcome dict: `{outcome, response, error, ...}`

3. **Host functions** (Rust, called via Monty's suspension mechanism):
   - `__llm_complete__` → call `LlmBackend::complete()`
   - `__execute_code_step__` → run user CodeAct code in a nested Monty VM
   - `__execute_action__` → execute a tool with lease + policy + safety
   - `__check_signals__` → poll for stop/inject signals
   - `__emit_event__` → broadcast ThreadEvent + record in thread
   - `__add_message__` → append message to thread history
   - `__save_checkpoint__` → persist state to thread metadata
   - `__transition_to__` → validated thread state transition
   - `__retrieve_docs__` → query memory docs from Store
   - `__check_budget__` → remaining tokens/time/USD
   - `__get_actions__` → available tool definitions from leases

### Nested Execution (CodeAct)

When the LLM responds with Python code, the orchestrator calls `__execute_code_step__(code, state)`. This suspends the orchestrator VM and creates a **second Monty VM** for the user's code:

```
Orchestrator VM (Monty #1)
  → calls __execute_code_step__(code, state)
  → suspends
      → Rust creates Monty #2 (user code VM)
      → User code calls web_search() → suspends → Rust executes tool → resumes
      → User code calls FINAL("answer") → terminates
      → Rust collects results
  → Orchestrator VM resumes with results dict
  → Orchestrator formats output, decides next step
```

This is the same mechanism as `rlm_query()` (recursive sub-agent). Each VM owns its own heap — no shared state, no locks.

### Thread State Machine

```
Created → Running → Waiting → Running (resume)
                  → Suspended → Running (resume)
                  → Completed → Done
                  → Failed
```

Terminal states: `Done`, `Failed`. Validated by `ThreadState::can_transition_to()`.

## Bridge Layer (`src/bridge/`)

The bridge connects the engine to existing IronClaw infrastructure:

| Adapter | Wraps | Purpose |
|---------|-------|---------|
| `LlmBridgeAdapter` | `LlmProvider` | Converts `ThreadMessage` ↔ `ChatMessage`, depth-based model routing, code block detection |
| `EffectBridgeAdapter` | `ToolRegistry` + `SafetyLayer` | Tool execution with all v1 security controls, name normalization (underscore ↔ hyphen), rate limiting |
| `HybridStore` | `Workspace` | In-memory for ephemeral data, workspace files for MemoryDocs |
| `EngineRouter` | `Agent` | Routes messages through engine when `ENGINE_V2=true`, manages SSE events |

### Enabling Engine v2

Set `ENGINE_V2=true` environment variable. The router in `src/bridge/router.rs` intercepts messages and routes them through the engine instead of the v1 agent loop.

For trace debugging: `ENGINE_V2_TRACE=1` writes full JSON traces to `engine_trace_*.json`.

## Memory System

### MemoryDoc Types

| Type | Purpose | Produced By |
|------|---------|-------------|
| `Summary` | What a thread accomplished | Conversation insights mission |
| `Lesson` | Durable learning from experience | Self-improvement mission |
| `Playbook` | Reusable multi-step procedure | Legacy (superseded by Skill) |
| `Skill` | Reusable skill with activation metadata and code snippets | Skill extraction mission, v1 migration |
| `Issue` | Detected problem for follow-up | Self-improvement mission |
| `Spec` | Missing capability request | Self-improvement mission |
| `Note` | Working memory / scratch | Orchestrator, prompt overlays |

### Learning Missions (replaced Reflection)

Instead of a separate reflection pipeline, knowledge extraction is handled by three event-driven **learning missions** that fire automatically after thread completion:

1. **Self-improvement** (`self-improvement`) — fires when a thread completes with trace issues (errors, tool-not-found, etc.). Diagnoses root cause, applies prompt overlays or orchestrator patches. Graduated risk: Level 1 (prompt) → Level 2 (config) → Level 3 (code, propose only).

2. **Skill extraction** (`skill-extraction`) — fires when a thread succeeds with 5+ steps and 3+ distinct tool actions. Extracts reusable skills with structured metadata: activation keywords/patterns, CodeAct code snippets, domain tags. Output is a `DocType::Skill` MemoryDoc with `V2SkillMetadata` JSON.

3. **Conversation insights** (`conversation-insights`) — fires every 5 completed threads in a project. Extracts user preferences, domain knowledge, workflow patterns, and corrections.

### Context Injection

On each LLM call, two knowledge sources are injected into the system prompt:

1. **Memory docs** — `build_step_context()` retrieves up to 5 relevant MemoryDocs (lessons, issues, specs) from the project via keyword scoring and appends them as "## Prior Knowledge".

2. **Active skills** — The `SkillSelector` scores all `DocType::Skill` docs against the thread goal using the deterministic 4-phase pipeline (gating → scoring → budget → attenuation). Selected skills are injected as `<skill>` XML blocks with their full prompt content and code snippet documentation.

## Skills System

Skills are the v2 replacement for both v1 SKILL.md prompt extensions and v1 playbooks. They provide deterministic, keyword-driven knowledge injection with optional executable code snippets for the CodeAct runtime.

### Architecture

Skills live in the `ironclaw_skills` crate (extracted from `src/skills/`), shared by both v1 and v2 engines. The engine crate depends on `ironclaw_skills` with `default-features = false` (no catalog/registry — just types + selection).

```
ironclaw_skills crate (shared)
  ├── types.rs       — SkillManifest, ActivationCriteria, LoadedSkill, SkillTrust
  ├── v2.rs          — V2SkillMetadata, CodeSnippet, SkillMetrics
  ├── selector.rs    — Deterministic scoring + confidence factor
  ├── parser.rs      — SKILL.md frontmatter parsing
  ├── validation.rs  — Name/content escaping, credential validation
  ├── gating.rs      — Binary/env/config requirements checking
  ├── registry.rs    — Filesystem discovery (feature-gated)
  └── catalog.rs     — ClawHub HTTP catalog (feature-gated)

ironclaw_engine crate (v2 integration)
  ├── capability/skill_selector.rs  — MemoryDoc → LoadedSkill bridge
  ├── capability/skill_tracker.rs   — Confidence tracking + rollback

src/skills/ (v1 shim)
  ├── mod.rs          — Re-exports from ironclaw_skills + credential conversion
  └── attenuation.rs  — Trust-based tool filtering (depends on ToolDefinition)

src/bridge/
  └── skill_migration.rs  — V1 SKILL.md → V2 MemoryDoc conversion
```

### Deterministic Selection Pipeline

Skill selection is entirely deterministic — no LLM involvement, preventing circular manipulation:

1. **Gating** — Check binary/env/config requirements; skip skills whose prerequisites are missing
2. **Scoring** — Keyword exact (10pts, cap 30) + substring (5pts) + tag (3pts, cap 15) + regex pattern (20pts, cap 40). Exclude keywords veto (score = 0). Confidence factor for extracted skills: `0.5 + 0.5 * confidence`
3. **Budget** — Greedy top-down selection within `max_context_tokens` (default 4000)
4. **Attenuation** — Minimum trust across active skills determines tool ceiling

### Skill Storage

Skills are stored as `MemoryDoc` with `DocType::Skill`. The `metadata` JSON field carries `V2SkillMetadata`:

```json
{
  "name": "github",
  "version": 2,
  "description": "GitHub API integration",
  "activation": {
    "keywords": ["github", "issues", "pull request"],
    "patterns": ["(?i)(list|show|get).*issue"],
    "tags": ["git", "devops"],
    "max_context_tokens": 1500
  },
  "source": "extracted",
  "trust": "trusted",
  "code_snippets": [{
    "name": "list_issues",
    "code": "def list_issues(owner, repo): ...",
    "description": "List open GitHub issues"
  }],
  "metrics": { "usage_count": 12, "success_count": 10, "failure_count": 2 },
  "parent_version": 1,
  "content_hash": "sha256:..."
}
```

### CodeAct Integration

Skills inject knowledge at two levels:

1. **System prompt** — Skill prompt content wrapped in `<skill name="..." trust="...">` XML blocks, with code snippet documentation listed as callable functions.

2. **Monty NameLookup** — Code snippet function names registered as known actions in the CodeAct runtime, so the LLM can call `list_issues()` directly without reconstructing the logic.

### Confidence Tracking

Auto-extracted skills track usage metrics via `SkillTracker`:
- After each thread: `record_usage(doc_id, success)` increments counters
- Confidence = `success_count / (success_count + failure_count)` (1.0 if no data)
- Low-confidence skills get demoted in scoring via `apply_confidence_factor()`
- `update_skill()` increments version with `parent_version` for rollback
- `rollback_skill()` restores previous version if an update causes failures

### V1 Migration

At engine startup (`init_engine()`), v1 SKILL.md files are converted to v2 MemoryDocs:
- `SkillSource::Workspace/User` → `V2SkillSource::Migrated`
- Trust level preserved
- Code snippets empty (v1 skills are prompt-only)
- Content hash checked for idempotency (unchanged skills are skipped)

## Missions

Missions are long-running goals that spawn threads over time. They replace v1 Routines and the old reflection pipeline.

```
Mission
  ├── goal: "Increase test coverage to 80%"
  ├── cadence: Cron("0 9 * * *") | OnSystemEvent | Manual | Webhook
  ├── current_focus: "Write tests for auth module"  (evolves)
  ├── approach_history: ["Analyzed codebase", "Added 15 tests for db"]
  ├── thread_history: [thread_1, thread_2, ...]
  └── max_threads_per_day: 10
```

### How Missions Fire

- **Cron**: Background ticker checks every 60s, fires missions with past `next_fire_at`
- **OnSystemEvent**: Event listener subscribes to ThreadManager events, fires matching missions when threads complete
- **Manual**: `mission_fire(id)` from CodeAct or API
- **Webhook**: Bridge routes incoming webhooks to matching missions

### Learning Missions (Built-in)

Three missions are created automatically at project bootstrap via `ensure_learning_missions()`:

| Mission | Trigger | Max/day | What it does |
|---------|---------|---------|-------------|
| `self-improvement` | Thread completes with trace issues | 5 | Diagnoses errors, applies prompt overlays or orchestrator patches |
| `skill-extraction` | Thread succeeds with 5+ steps, 3+ tools | 3 | Extracts reusable skills with activation metadata + CodeAct snippets |
| `conversation-insights` | Every 5 completed threads | 2 | Extracts user preferences, domain knowledge, workflow patterns |

### Meta-Prompt Generation

When a mission fires, `build_meta_prompt()` assembles:
- Mission goal + success criteria
- Current focus (what to work on next)
- Approach history (what was tried and what happened)
- Project knowledge (relevant MemoryDocs, up to 10)
- Trigger payload (event data, trace issues, thread stats)

The thread runs with this context and returns: what it accomplished, what to focus on next, whether the goal is achieved. `process_mission_outcome()` extracts these and updates the mission state.

### Self-Improvement Loop

The self-improvement mission creates a feedback loop:

```
Thread fails → trace analysis detects issues → self-improvement fires
  → diagnoses root cause (PROMPT / CONFIG / CODE)
  → Level 1: updates prompt overlay (low risk, auto-apply)
  → Level 2: patches orchestrator code (medium risk, versioned with rollback)
  → Level 3: proposes code change (high risk, human review)
  → records fix in pattern database → next similar failure uses known fix
```

## Capability System

### Leases

Threads don't have static permissions. They receive **leases** — scoped, time-limited, use-limited grants:

```rust
CapabilityLease {
    thread_id,
    capability_name,
    granted_actions: ["web_search", "read_file", ...],
    expires_at: Option<DateTime>,
    max_uses: Option<u32>,
    revoked: bool,
}
```

### Policy Engine

The PolicyEngine evaluates actions against leases deterministically:

1. Check global denied effects (e.g., deny all Financial)
2. Check capability-level policies (per-action rules)
3. Check action's `requires_approval` flag
4. Check effect types against lease grant

Decision priority: **Deny > RequireApproval > Allow**

### Effect Types

Every action declares its side effects:
```
ReadLocal, ReadExternal, WriteLocal, WriteExternal,
CredentialedNetwork, Compute, Financial
```

## Integration Scaling Strategy

### The Problem: Tool List Bloat

A naive approach to adding third-party integrations (Slack, GitHub, Stripe, etc.) is to register each API action as a separate tool — `slack_post_message`, `slack_list_channels`, `github_create_issue`, etc. This fails for LLM-based agents:

- Each tool definition costs ~80-120 tokens in the tool list, sent on **every request**
- 200 actions = ~20,000 tokens always-on context cost
- LLM tool selection accuracy **degrades significantly** beyond ~20-30 tools
- The LLM still has to construct correct parameters — deterministic execution doesn't help if the LLM picks the wrong tool or hallucinates params

This was confirmed by studying [Pica](https://github.com/withoneai/pica) (formerly IntegrationOS), which supports 200+ platforms via data-driven definitions in MongoDB. Pica's approach works for programmatic API access, but registering all those actions as LLM tools would degrade agent performance.

### The Solution: Skills as Knowledge-Bearing Definitions

In engine v2, **Skills** replace both WASM API wrapper tools and static prompt extensions. A Skill bundles **knowledge** (how to call an API) with **activation criteria** (when to load) and optional **CodeAct code snippets** (reusable Python functions). For API integrations:

1. The `http` action is always available (one tool in the LLM's action list)
2. Each integration is a Skill with prompt content that teaches the LLM how to call that platform's API
3. Skills are selected on-demand per thread based on keyword/pattern matching against the goal — not registered globally
4. The LLM reads the skill content, constructs the correct `http` call
5. Credentials are auto-injected at the HTTP boundary — the LLM never sees tokens

```
User: "post hello to #general on slack"
       ↓
Skill activation: "slack" skill selected (keywords: "slack", "message", "channel")
       ↓
LLM reads skill prompt: learns endpoints, body format, pagination
       ↓
LLM writes CodeAct Python:
  result = http(method="POST", url="https://slack.com/api/chat.postMessage",
                body={"channel": "C01234", "text": "hello"})
  FINAL(str(result))
       ↓
EffectExecutor: policy check → credential injection → SSRF protection → leak detection → response
```

Skills can also carry **CodeAct snippets** — pre-built Python functions that the LLM can call directly, avoiding the need to reconstruct API patterns from scratch each time.

### Token Cost Comparison

| Scenario | Dedicated Tools (200 actions) | Capability + http |
|---|---|---|
| User asks about Slack | ~20,000 (all tools in list) | ~700 (http action + slack skill) |
| User asks about nothing | ~20,000 (still there) | ~200 (just http action) |
| Tool selection accuracy | Degrades with count | Always picks `http` — no confusion |
| Adding a new platform | Define N tool schemas + executor | Write a SKILL.md (markdown + YAML) |

### What a Skill Definition Looks Like

A SKILL.md file with YAML frontmatter (activation + credentials) and markdown body (API knowledge):

```yaml
---
name: slack
version: "1.0.0"
description: Slack Web API — post messages, manage channels, search
activation:
  keywords: ["slack", "message", "channel"]
  patterns: ["(?i)(post|send).*slack", "(?i)slack.*(message|channel)"]
  tags: ["chat", "messaging"]
  max_context_tokens: 1500
credentials:
  - name: slack_bot_token
    provider: slack
    location: { type: bearer }
    hosts: ["slack.com"]
---

# Slack API

Base URL: `https://slack.com/api`. Auth injected automatically.

**Post message**: `http(method="POST", url="https://slack.com/api/chat.postMessage", body={"channel": "<id>", "text": "<msg>"})`
**List channels**: `http(method="GET", url="https://slack.com/api/conversations.list?types=public_channel&limit=100")`
**Search**: `http(method="GET", url="https://slack.com/api/search.messages?query=<text>")`

All responses: `{"ok": true, ...}` or `{"ok": false, "error": "<code>"}`.
Paginate with `cursor` param when `response_metadata.next_cursor` is non-empty.
```

~350 tokens of knowledge covers 4+ API endpoints. The LLM generalizes the pattern to other Slack endpoints from training data. Credentials are declared in frontmatter and injected automatically — the LLM never sees token values.

Skills can also be **auto-extracted** by the skill-extraction mission from successful multi-step threads, complete with activation keywords and CodeAct code snippets learned from actual usage.

### Classification of v1 Built-in Tools

Studied all 37 v1 built-in tools to determine which fit the knowledge-driven pattern:

**Can be knowledge-driven (HTTP API wrappers):**
- `image_gen`, `image_analyze`, `image_edit` — pure HTTP calls to external APIs with auth

**Already a generic action (the execution engine):**
- `http` — the action that knowledge-driven Capabilities delegate to

**Must remain dedicated actions (complex local logic):**
- `shell` — 4-layer command validation, Docker sandbox, environment scrubbing
- `file` (read/write/list/patch) — local filesystem with path traversal prevention
- `memory_*` — hybrid FTS + vector search, prompt injection detection
- `job_*` — Docker container lifecycle, context isolation
- `routine_*` — database-backed CRON scheduling
- `extension_tools`, `skill_tools` — registry and system management
- `secrets_tools` — encrypted store management
- `json`, `time`, `echo` — pure local computation
- `message`, `restart`, `tool_info` — internal agent control

**Takeaway**: Only 3 of 37 existing tools are HTTP wrappers. The value is not converting existing tools — it's enabling hundreds of **new** integrations (Slack, GitHub, Jira, Stripe, Salesforce, etc.) without writing Rust or WASM — just a SKILL.md file.

### Where Dedicated Actions Still Win

1. **Autonomous/headless threads** — Missions and background threads with no human oversight benefit from deterministic execution for their 1-2 critical integrations. Register those specific actions via leases.
2. **OAuth token acquisition** — The LLM cannot perform redirect-based OAuth flows. Skills declare OAuth config in their `credentials` frontmatter; the system handles the redirect dance and stores tokens. The skill's prompt content then instructs the LLM to just call `http` — credentials are injected transparently.
3. **High-frequency reliability-critical paths** — If a specific integration is called thousands of times and must never fail, a dedicated action avoids LLM reasoning variance. Over time, the skill-extraction mission learns reliable CodeAct snippets from successful executions, which narrows this gap.
4. **Complex computation or data transformation** — WASM tools still make sense for CPU-intensive processing (image manipulation, format conversion) where the sandbox guarantees matter.

### Comparison with Pica's Approach

[Pica](https://github.com/withoneai/pica) uses a data-driven model where each API action is a MongoDB document (`ConnectionModelDefinition`) with base URL, path, method, auth method, schemas, and JavaScript transform functions. A generic executor dispatches requests. Key patterns:

- **Handlebars secret injection** — entire definition rendered as template with user's secrets as context
- **Passthrough + Unified dual mode** — raw HTTP proxy or normalized CRUD via CommonModels
- **JS sandbox transforms** — `fromCommonModel`/`toCommonModel` functions for data mapping
- **`knowledge` field** — free-text documentation per action for AI tool discovery

Pica's model is optimized for programmatic API access (SDK calls from code). For LLM agents, the skill-as-knowledge approach is superior because it avoids tool list bloat while leveraging the LLM's ability to construct HTTP calls from documentation. The two approaches share the insight that **integrations should be data, not code**. IronClaw extends this further: the skill-extraction mission can learn new skills from successful thread executions, making the integration library self-expanding.

## Key Files

| File | Purpose |
|------|---------|
| `crates/ironclaw_engine/orchestrator/default.py` | The Python execution loop (v0) |
| `crates/ironclaw_engine/src/executor/orchestrator.rs` | Host functions + versioning + loading |
| `crates/ironclaw_engine/src/executor/loop_engine.rs` | Bootstrap (loads + runs orchestrator, skill injection) |
| `crates/ironclaw_engine/src/executor/scripting.rs` | Monty VM integration, user code execution, CodeAct skill snippets |
| `crates/ironclaw_engine/src/executor/prompt.rs` | System prompt construction, skill section formatting |
| `crates/ironclaw_engine/src/runtime/manager.rs` | ThreadManager (spawn, stop, join, skill selector wiring) |
| `crates/ironclaw_engine/src/runtime/mission.rs` | MissionManager (lifecycle, firing, learning missions) |
| `crates/ironclaw_engine/src/capability/skill_selector.rs` | MemoryDoc → LoadedSkill bridge, deterministic selection |
| `crates/ironclaw_engine/src/capability/skill_tracker.rs` | Confidence tracking, versioned updates, rollback |
| `crates/ironclaw_engine/src/types/` | All core data structures |
| `crates/ironclaw_engine/src/traits/` | LlmBackend, Store, EffectExecutor |
| `crates/ironclaw_skills/` | Shared skills crate (types, selector, parser, validation) |
| `src/bridge/router.rs` | Engine v2 entry point, skill migration at startup |
| `src/bridge/skill_migration.rs` | V1 SKILL.md → V2 MemoryDoc conversion |
| `src/bridge/effect_adapter.rs` | Tool execution bridge with safety |
| `src/bridge/llm_adapter.rs` | LLM provider bridge |
| `src/bridge/store_adapter.rs` | HybridStore (in-memory + workspace) |
| `skills/github/SKILL.md` | Reference GitHub skill (API patterns + credential spec) |
| `tests/engine_v2_skill_codeact.rs` | E2E test: skill → CodeAct → mock HTTP → canned response |

## Testing

```bash
cargo check -p ironclaw_skills                                    # skills crate compiles
cargo test -p ironclaw_skills                                     # 94 tests (types, selector, parser, gating, registry, catalog)
cargo check -p ironclaw_engine                                    # engine crate compiles
cargo test -p ironclaw_engine                                     # 203 tests (execution, missions, skills, tracking)
cargo test --test engine_v2_skill_codeact                         # E2E: full CodeAct loop with mock HTTP
cargo clippy --all -- -D warnings                                 # zero warnings across workspace
cargo test                                                        # full suite
```

## Design Influences

- **RLM paper** (arXiv:2512.24601) — context as variable, FINAL() termination, recursive sub-calls
- **karpathy/autoresearch** — the self-improvement loop as a program.md, fixed-budget evaluation, git as state machine
- **Official RLM impl** (alexzhang13/rlm) — 30 max iterations, compaction at 85%, budget inheritance
- **fast-rlm** (avbiswas/fast-rlm) — Step 0 orientation, parallel sub-calls, dual model routing
- **Pica/IntegrationOS** (withoneai/pica) — data-driven integration definitions, Handlebars secret injection, knowledge fields for AI tool discovery. Validated the "integrations as data" principle; diverged on execution model (knowledge-driven Capabilities instead of per-action tool registration)

See also: `docs/plans/2026-03-20-engine-v2-architecture.md` for the full 8-phase roadmap.
