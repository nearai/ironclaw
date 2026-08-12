# Agent Map — ironclaw_agent_loop

Working rules for the sealed loop framework. Orientation lives in `README.md`;
family rules in `crates/loop/AGENTS.md`.

## Start Here

- Read `README.md` for what this crate is; read `Cargo.toml` for the
  (contracts-only) dependency shape.
- Use these neighboring contracts before changing cross-crate behavior:
  - `crates/contracts/ironclaw_loop_contracts/` — the port and driver contracts
    this framework consumes.
  - `crates/loop/ironclaw_loop_host/AGENTS.md`
  - `crates/loop/ironclaw_turn_runner/AGENTS.md`

## What This Crate Owns

- Agent-loop framework state and strategy contracts for Reborn.
- `executor.rs` loop mechanics, canonical tick behavior, and deterministic
  execution flow; `executor/canonical.rs` is the ordered lifecycle spine.
- `family.rs`, `families/`, `planner.rs`, `default_planner.rs`, and
  `strategies/` for sealed built-in loop-family/planning strategy composition
  (one decision axis per strategy file).
- `state.rs` and `state/` for resumable loop state: refs, cursors, counters,
  versions, and safe summaries only.
- `test_support/` fixture code for framework and driver tests.

## Do Not Move In Here

- Product-specific logic, product adapters, transport behavior, or Reborn app
  composition.
- `AgentLoopDriver` / `PlannedDriver` host wiring; that bridge belongs in
  `ironclaw_turn_runner`. The framework never sees a driver — only its own
  executor contract.
- Runtime lanes, host-runtime services, provider auth, network/secrets, or UI
  concerns.
- Raw prompts, raw assistant content, tool input JSON, secrets, host paths, or
  backend diagnostics in state.

## Boundaries

- Dependencies are contracts-tier only: `ironclaw_common`,
  `ironclaw_host_api`, `ironclaw_loop_contracts`. This crate must not depend
  on `ironclaw_turn_runner`, `ironclaw_turns`, host runtime crates, product
  adapters, the dispatcher, capability host, filesystem, network, secrets, or
  DB backends — the `BoundaryRule` in
  `crates/app/ironclaw_architecture_tests/tests/reborn_dependency_boundaries.rs`
  is the arbiter.
- State stores refs, cursors, counters, versions, and safe summaries only.

## Executor stage ownership

- Keep `src/executor/canonical.rs` as the ordered lifecycle spine; put
  lifecycle mechanics in the owning executor stage instead of branch logic in
  `canonical.rs`.
- Keep `CanonicalAgentLoopExecutor` public; keep `DefaultExecutorPipeline` and
  stage types crate-internal.
- Do not pass sibling stages through another stage's input; a phase's helper
  behavior stays owned inside the stage module.
- Do not add stages for pure mapping helpers or one-line wrappers.
- Keep cancellation, checkpoint, and pending-input-ack ordering explicit at
  the stage boundary that owns the state transition.

## Adding code

- Add a new strategy file only for a new independent decision axis.
- Add a new state-slot type only when a strategy needs typed resumable state —
  no `serde_json::Value` shortcuts for known shapes.
- Add a new family file only for a built-in loop family composed from sealed
  strategies.
- Add executor helpers only when they are part of canonical loop mechanics.
- Introduce a submodule before a file becomes a mixed bag of unrelated
  helpers; no `misc`/`utils`/`common` modules.

## Common mistakes

- Do not append product-specific logic to the executor.
- Do not expose strategy traits publicly to downstream crates.
- Do not make strategies mutate shared state by reference; strategies return
  outcomes, and the executor swaps typed slots into the next whole state.

## Validation

- Fast local check: `cargo test -p ironclaw_agent_loop`
- Boundary check after dependency/API changes:
  `cargo test -p ironclaw_architecture_tests`
- Run `cargo test -p ironclaw_turn_runner` when changes affect drivers or
  loop-host integration.
