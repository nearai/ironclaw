# Agent Map — ironclaw_agent_loop

## Start Here

- Read `CLAUDE.md` first; it is the crate-local guardrail file.
- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these neighboring contracts before changing cross-crate behavior:
  - `crates/ironclaw_turns/AGENTS.md`
  - `crates/ironclaw_loop_support/CLAUDE.md`
  - `crates/ironclaw_reborn/CLAUDE.md`

## What This Crate Owns

- Agent-loop framework state and strategy contracts for Reborn.
- `executor.rs` loop mechanics, canonical tick behavior, and deterministic execution flow.
- `family.rs`, `families/`, `planner.rs`, `default_planner.rs`, and `strategies/` for sealed built-in loop-family/planning strategy composition.
- `state.rs` and `state/` for resumable loop state: refs, cursors, counters, versions, and safe summaries only.
- `test_support/` fixture code for framework and driver tests.

## Do Not Move In Here

- Product-specific logic, product adapters, transport behavior, or Reborn app composition.
- `AgentLoopDriver` / `PlannedDriver` host wiring; that bridge belongs in `ironclaw_reborn`.
- Runtime lanes, host-runtime services, provider auth, network/secrets, or UI concerns.
- Raw prompts, raw assistant content, tool input JSON, secrets, host paths, or backend diagnostics in state.

## Validation

- Fast local check: `cargo test -p ironclaw_agent_loop`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture`
- Run `cargo test -p ironclaw_reborn` when changes affect drivers or loop-host integration.

## Agent Notes

- Add new strategy files only for independent decision axes.
- Add typed state-slot types instead of `serde_json::Value` shortcuts.
- Keep strategy traits private unless a contract explicitly makes them public.
- Strategies return decisions/state deltas; they should not mutate shared state by reference.
