# IronClaw Engine v2

Engine v2 is IronClaw's unified execution engine. It replaces ~10 separate v1
abstractions (Session, Job, Routine, Channel, Tool, Skill, Hook, Observer,
Extension, LoopDelegate) with 5 primitives: **Thread**, **Step**,
**Capability**, **MemoryDoc**, **Project**. See the architecture plan at
`docs/plans/2026-03-20-engine-v2-architecture.md` for the full design.

## Current status

Engine v2 is **opt-in**. The default flip is tracked in issue #2800. Until
that lands:

- `ENGINE_V2=true` turns engine v2 on.
- Without it, IronClaw keeps using the legacy v1 agent loop.

## How to opt in

```bash
# Start the CLI with v2
ENGINE_V2=true ironclaw

# Or during development
ENGINE_V2=true cargo run
```

You can confirm which engine served a given request via the web gateway
status endpoint — the `engine_v2_enabled` field in `/api/status` reflects
the current routing decision.

## How to opt out (once v2 becomes the default)

When the default flips, set `IRONCLAW_LEGACY_ENGINE=true` to keep v1:

```bash
IRONCLAW_LEGACY_ENGINE=true ironclaw
```

This overrides any other engine flag — including `ENGINE_V2=true`. Use it
if you hit a regression during the rollout and need to fall back. The
opt-out will be removed one release after v2 is stable as the default.

## Canary rollout

During the staged rollout, operators can run a percentage-based canary:

```bash
# Route 10% of users (by user_id hash) to v2, keep the rest on v1
ENGINE_V2_CANARY_PCT=10 ironclaw
```

Properties:

- **Deterministic per user** — a given `user_id` always falls in the same
  cohort across restarts. Users aren't shuffled between engines.
- **Ignored without a user** — boot-time observations and aggregate status
  endpoints report v1 until a user's request arrives.
- **Beaten by explicit flags** — `ENGINE_V2=true` forces v2 on, and
  `IRONCLAW_LEGACY_ENGINE=true` forces v1 off, regardless of the canary %.

## What users will notice on v2

- **`create_job` / `cancel_job`** — translated to the mission manager. New
  `create_job` calls show up under `/missions`, not `/jobs`.
- **`routine_create` / `routine_list` / `routine_fire` / `routine_pause` /
  `routine_resume` / `routine_delete` / `routine_update` / `routine_history`** —
  all aliased to the corresponding `mission_*` actions. The `/routine`
  slash command still works via the shared manager.
- **Cost reporting** — v1 and v2 now compute identical USD numbers for the
  same LLM call (shared formula in `src/llm/costs.rs`).
- **Reliability data** — v2 records per-action success rate and latency in
  a tracker that will surface "recently unreliable" hints into the system
  prompt once wired. Toggle with `ENGINE_V2_RELIABILITY_HINTS=false`.

## What v1 does that v2 does not (yet)

- **`build_software`** — remains v1-only; no mission equivalent exists.

## Debugging

```bash
# Full engine v2 debug logs
ENGINE_V2=true RUST_LOG=ironclaw_engine=debug cargo run

# Trace recording (for E2E replay)
ENGINE_V2=true IRONCLAW_RECORD_TRACE=1 cargo run
```

Traces live under `~/.ironclaw/traces/` by default (override with
`IRONCLAW_TRACE_OUTPUT`).

## Reporting v2-specific issues

Tag issues with `engine-v2` and include:

- the `ENGINE_V2` / `IRONCLAW_LEGACY_ENGINE` / `ENGINE_V2_CANARY_PCT`
  values active at the time
- whether v2 was selected for your request (see `/api/status`
  `engine_v2_enabled` field)
- a trace file from `IRONCLAW_RECORD_TRACE=1` if possible

The umbrella tracker for the default flip is #2800.
