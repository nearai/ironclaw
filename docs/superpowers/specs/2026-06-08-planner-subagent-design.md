# Planner subagent + spawn_subagent schema redesign

Date: 2026-06-08
Status: Approved (Scope A — narrow PR)
Owner: Henry Park

## Goal

Replace the `researcher` subagent flavor with a `planner` flavor that produces
structured implementation plans, and redesign the `spawn_subagent` tool surface
so the model can discover available flavors and is nudged toward planning for
complex tasks.

## Motivation

Investigation across `pi`, `opencode`, and Claude Code confirmed a common
pattern: separate "plan" and "execute" responsibilities, expose flavor choices
to the model via enum, and prompt the parent agent on when to plan first.
IronClaw Reborn today has none of these:

- No `planner` flavor.
- `flavor_id` parameter is free-form `string` with no enum — model hallucinates
  flavor names and gets `unknown_subagent_kind` denials with no recovery hint.
- Tool description is 10 words; no guidance on when to spawn or which flavor.
- `researcher` flavor exists but its role overlaps planning; consolidating
  removes one decision the model has to make.

Reference systems surveyed:

- **pi** (the pi-mono extension examples (`packages/coding-agent/examples/extensions/`)):
  plan-mode extension + planner subagent (`planner.md`) returning
  Goal/Plan/Files/Risks structure; scout → planner → worker chain.
- **opencode** (`sst/opencode`): first-class `plan` agent with `.opencode/plans/*.md`
  artifact, `plan_enter`/`plan_exit` tools, parallel `explore` subagents in plan
  phase.
- **Claude Code**: `EnterPlanMode`/`ExitPlanMode` permission-mode gates, read-only
  enforced at tool layer, freeform plan text approved by user before execution.

Scope A is the minimum that improves discoverability and makes the planner the
default answer for complex tasks. Plan-mode for the parent agent, plan-file
persistence, nested spawning, and live todo widgets are deferred (Scope B/C).

## Decisions (verbatim from brainstorming)

1. **Scope A** — flavor + schema only, ships with the rename PR already in flight.
2. **Strict structured Markdown output** — pi-style Goal/Plan/Files/Risks.
3. **Codebase + web tool allowlist** — planner absorbs researcher's `http`.
4. **Remove `researcher`** flavor.
5. **Keep tool name `spawn_subagent`** — rename is bikeshed; internal types
   (`SubagentDefinition`, `SubagentKindId`, `SubagentFlavor`) already use this
   vocabulary.

## Architecture changes

### Flavor registry (`crates/ironclaw_reborn/src/subagent/flavors.rs`)

Final registry: `general`, `explorer`, `coder`, **`planner`**.

```rust
SubagentFlavor {
    id: SubagentFlavorId::Planner,
    id_str: "planner",
    allow_nesting: false,
    tool_allowlist: &[
        "read_file", "list_dir", "grep", "glob",
        "http",
    ],
    direction: PLANNER_DIRECTION,
}
```

Drop `SubagentFlavorId::Researcher` variant + its entry in
`BUILTIN_SUBAGENT_FLAVORS` + the `"researcher"` arm of `parse_flavor_id`.

Export a new public API for downstream schema builders:

```rust
pub struct FlavorDescriptor {
    pub id: &'static str,
    pub summary: &'static str,
}

pub fn builtin_flavor_catalog() -> Vec<FlavorDescriptor> { ... }
```

`FlavorDescriptor::summary` is a one-line human prose hint of what the flavor
does + its tool surface, used by the schema description builder.

### Direction prompt (`crates/ironclaw_reborn/src/subagent/directions/planner.md`)

New file. Content:

```markdown
# Planner subagent

You produce implementation plans. You do NOT execute changes — your job is to
study the problem, gather context (codebase + web), and return a structured plan
the parent agent will then execute.

## Available tools

- `read_file`, `list_dir`, `grep`, `glob` — codebase exploration (read-only)
- `http` — fetch library docs, API references, RFCs, or other web context

You CANNOT write files, run shell commands, or spawn other subagents. If a task
requires changes, return the plan; the parent will dispatch a coder subagent.

## Workflow

1. Read the task and handoff carefully. Identify the goal in one sentence.
2. Explore the codebase: locate relevant files, understand existing patterns,
   identify the seams where changes belong.
3. If the task references unfamiliar libraries, APIs, or external systems, use
   `http` to fetch the official docs. Cite source URLs in the plan.
4. Synthesize. Pick ONE recommended approach — do not present alternatives.
5. Return the plan in the format below. Nothing else.

## Output format (strict)

Return ONLY this Markdown. No preamble, no postscript.

\`\`\`

## Goal
<one sentence — what needs to be done>

## Plan
1. <small, actionable step — name the file/function/area>
2. <next step>
3. ...

## Files to Modify
- `path/to/file.rs` — <what changes>
- `path/to/other.rs` — <what changes>

## New Files
- `path/to/new.rs` — <purpose>
(omit this section if no new files)

## Risks
- <constraint, edge case, or rollback concern>
(omit this section if none)

## References
- <URL> — <what it informed>
(omit this section if no external sources consulted)

\`\`\`

## Discipline

- Steps must be small enough that a coder subagent can execute one without
  asking clarifying questions.
- Name specific files and functions, not vague areas.
- Do not include rationale prose outside the plan — the structure is the plan.
- If you cannot produce a plan (insufficient information), return ONLY a
  `## Goal` section followed by `## Blocked` listing what's missing. Do not
  guess.
```

Drop `crates/ironclaw_reborn/src/subagent/directions/researcher.md`.

Update `crates/ironclaw_reborn/src/subagent/directions/mod.rs`:

```rust
const PLANNER_DIRECTION: &str = include_str!("planner.md");
// drop: const RESEARCHER_DIRECTION: &str = include_str!("researcher.md");
```

### Tool schema (`crates/ironclaw_loop_support/src/subagent_spawn_port.rs`)

`SpawnSubagentArgs` — wire rename with backwards-compatible alias:

```rust
pub struct SpawnSubagentArgs {
    #[serde(rename = "subagent_type", alias = "flavor_id")]
    pub subagent_kind: SubagentKindId,
    pub task: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff: Option<String>,
}
```

`SubagentSpawnCapabilityPort::new` gains a `flavor_catalog: Vec<FlavorDescriptor>`
argument. Schema builder is dynamic at port construction:

```json
{
  "type": "object",
  "required": ["subagent_type", "task"],
  "additionalProperties": false,
  "properties": {
    "subagent_type": {
      "type": "string",
      "enum": ["general", "explorer", "coder", "planner"],
      "description": "Which subagent profile to spawn. Options:\n- general: ...\n- explorer: ...\n- coder: ...\n- planner: read codebase + web research, returns a structured implementation plan (read_file, list_dir, grep, glob, http)"
    },
    "task": { "type": "string", "maxLength": ..., "description": "..." },
    "handoff": { "type": "string", "maxLength": ..., "description": "..." }
  }
}
```

The enum and per-flavor descriptions are built from `flavor_catalog` — single
source of truth, no drift risk.

Tool description rewrite:

```
Delegate a focused task to a fresh child agent with its own context window and
tool scope. The child runs to completion and returns its final result. Use when
the task would otherwise pollute your context (deep file reads, multi-step
research) or needs different tool permissions than your current scope. For
complex tasks involving multiple steps, design choices, or unfamiliar libraries,
spawn a `planner` first — it returns a structured plan you can then execute or
hand to a `coder`. Pick `subagent_type` based on what the child needs:
exploration, planning, or code changes.
```

### Wire-up (`crates/ironclaw_reborn/src/model_gateway.rs`)

Single-line change: pass `flavors::builtin_flavor_catalog()` into the existing
`SubagentSpawnCapabilityPort::new(...)` call site.

## Files touched

| File | Change |
|------|--------|
| `crates/ironclaw_reborn/src/subagent/flavors.rs` | drop Researcher, add Planner variant + entry; export `builtin_flavor_catalog()` + `FlavorDescriptor` |
| `crates/ironclaw_reborn/src/subagent/directions/mod.rs` | drop `RESEARCHER_DIRECTION`, add `PLANNER_DIRECTION` |
| `crates/ironclaw_reborn/src/subagent/directions/researcher.md` | DELETE |
| `crates/ironclaw_reborn/src/subagent/directions/planner.md` | NEW |
| `crates/ironclaw_loop_support/src/subagent_spawn_port.rs` | wire rename + alias, port `new` takes catalog, dynamic schema, description rewrite |
| `crates/ironclaw_reborn/src/model_gateway.rs` | pass catalog into constructor |
| `crates/ironclaw_reborn/src/subagent/capability_surface.rs` | verify `http` capability resolves for planner allowlist (no behavior change expected) |

## Test plan

1. **Flavor unit tests** (`flavors.rs::tests`):
   - `planner` present in `BUILTIN_SUBAGENT_FLAVORS`
   - `parse_flavor_id("planner")` returns `Some(Planner)`
   - `parse_flavor_id("researcher")` returns `None`
   - planner allowlist matches the 5 expected tools exactly
   - `builtin_flavor_catalog()` returns one entry per registered flavor
2. **Schema test** (`subagent_spawn_port.rs::tests`):
   - construct port with catalog, assert generated schema's `subagent_type.enum`
     equals `["general","explorer","coder","planner"]` in order
   - description contains each flavor's summary substring
3. **Wire compatibility** (`subagent_spawn_port.rs::tests`):
   - deserialize `{"flavor_id":"general","task":"x"}` succeeds via alias
   - deserialize `{"subagent_type":"general","task":"x"}` succeeds via primary
   - serialize emits `subagent_type` not `flavor_id`
   - duplicate-field guard: `{"flavor_id":"x","subagent_type":"y",...}` either
     errors or last-wins consistently (document behavior in test)
4. **Direction prompt test**: assert `PLANNER_DIRECTION` non-empty (parallels
   existing direction tests in `directions/mod.rs`).
5. **Integration**: existing subagent E2E tests pass; substitute `planner` for
   `researcher` in fixtures.

Commands: `cargo test -p ironclaw_reborn -p ironclaw_loop_support` +
`cargo clippy --all --tests`.

## Risks + mitigations

| Risk | Mitigation |
|------|-----------|
| Persisted runs with `flavor_id:"researcher"` fail replay | Accepted. `unknown_subagent_kind` is the existing failure mode for any deprecated id. Document in PR description. |
| `serde(alias)` + duplicate-field behavior | Add explicit test covering both-fields-present input; document last-wins or error semantics. |
| `http` policy gating already handles network — planner allowlist alone does not grant access | Confirm `http` tool resolution through `EffectiveRuntimePolicy` is unchanged. Acceptable that a network-blocked policy will return a denial to planner. |
| Provider tool-schema `enum` support | OpenAI + Anthropic both honor `enum`; `provider_tool_definition_to_llm` already passes `parameters` through verbatim. |
| Flavor catalog drift | Catalog derived FROM `BUILTIN_SUBAGENT_FLAVORS`. Add test asserting parity. |

## Out of scope (explicit)

- Plan-mode for parent agent (read-only toggle) — Scope C
- Planner nesting / spawning explore children — Scope B
- Durable `.ironclaw/plans/*.md` artifact — Scope C
- Live todo widget / `[DONE:n]` extraction — Scope C
- Tool rename `spawn_subagent` → other — kept as-is
- Rust field rename `subagent_kind` → `subagent_type` — wire-only rename
- Parent-agent system prompt rewrite — partial via tool description; full
  rewrite deferred

## Estimate

~150 lines code + ~80 lines tests. Single PR targeting `main`. Parallel
implementation split: Agent A owns reborn-side flavor registry + directions,
Agent B owns loop_support port schema and rename. Main thread does wire-up in
`model_gateway.rs` after both land + runs the full test suite.
