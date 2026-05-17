# ironclaw_agent_loop

Owns the reusable Reborn loop framework: loop-family identity and registry,
sealed planner composition, strategy traits, default strategy impls, the
canonical executor, loop execution state, and executor test support.

## Main entry points

- `src/executor.rs` is the loop mechanics entry point. Add canonical tick
  behavior here only when it applies to every family.
- `src/family.rs` owns `LoopFamily`, `LoopFamilyId`, component identity, and
  registry rules.
- `src/planner.rs` owns the public planner facade and crate-private strategy
  access.
- `src/default_planner.rs` wires the built-in default strategy composition.
- `src/state.rs` and `src/state/` own resumable execution state.
- `src/strategies/` owns one decision axis per file.
- `src/families/` owns built-in family factories.
- `src/test_support/` is fixture code for framework and driver tests only.

## Boundaries

- This crate depends upward on neutral contracts in `ironclaw_turns`.
- This crate must not depend on `ironclaw_reborn`, host runtime crates, product
  adapters, dispatcher, capability host, filesystem, network, secrets, or DB
  backends.
- The framework never sees `AgentLoopDriver`; `PlannedDriver` in
  `ironclaw_reborn` adapts runner-facing driver calls to this crate's executor.
- State stores refs, cursors, counters, versions, and safe summaries only. Do
  not store raw prompts, raw model output, tool args, secrets, host paths,
  provider errors, or stack traces in state or strategy slots.

## Adding code

- Add a new strategy file only for a new independent decision axis.
- Add a new state-slot type only when a strategy needs typed resumable state.
- Add a new family file only for a built-in loop family composed from sealed
  strategies.
- Add executor helpers only when they are part of canonical loop mechanics.
- Introduce a submodule before a file becomes a mixed bag of unrelated helpers.

## Common mistakes

- Do not append product-specific logic to the executor.
- Do not expose strategy traits publicly to downstream crates.
- Do not add `misc`, `utils`, `common`, or broad helper modules.
- Do not use stringly typed strategy state or `serde_json::Value` as a shortcut
  for a known shape.
- Do not make strategies mutate shared state by reference; strategies return
  outcomes, and the executor swaps typed slots into the next whole state.
