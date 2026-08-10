# ironclaw_agent_loop

The canonical, sealed loop framework: loop-family identity and registry, the
planner service and its sealed strategy composition, the canonical executor
with its ordered lifecycle stages, and resumable execution state. It is the one
artifact in the system meant to be replaced wholesale without touching anything
privileged — its contracts-only dependency set is the loop trust story made
mechanical, a fact the compiler enforces rather than a review convention.

- **Family / layer:** `loop` / `loops` · **Package:** `ironclaw_agent_loop` · **Manifest:** `crates/loop/ironclaw_agent_loop/Cargo.toml`
- **Use this when:** changing what a turn decides to do next — a strategy, a
  loop family, an executor lifecycle stage, or a typed state slot.
- **Don't use this when:** adapting a host service to a loop port → use
  `ironclaw_loop_host`; bridging a kernel work claim to a driver → use
  `ironclaw_turn_runner`; policy/audit wrapping of port calls → use
  `ironclaw_hooks`.

## Public surface

- `CanonicalAgentLoopExecutor` (`executor.rs`) — the canonical executor;
  `DefaultExecutorPipeline` and stage types stay crate-internal.
- `AgentLoopPlanner` (`planner.rs`) + `default_planner.rs` — the public
  planner service; strategy traits are deliberately **not** public.
- `family.rs` / `families/` — `LoopFamily`, `LoopFamilyId`, the registry, and
  built-in family factories.
- `state.rs` / `state/` — resumable state: refs, cursors, counters, versions,
  and safe summaries only.
- `test_support/` — fixtures for framework and driver tests.

## Depends on / consumed by

- **Normal workspace deps (3, contracts-tier only):** `ironclaw_common`,
  `ironclaw_host_api`, `ironclaw_loop_contracts`. Nothing else — no substrate,
  domain, kernel, lane, product, or app crate.
- **Consumed by (1):** `ironclaw_turn_runner`, whose `PlannedDriver` adapts
  the executor to the `AgentLoopDriver` contract. The framework never sees a
  driver.

## Invariants

- **Contracts-only dependency set** — the special matrix rule for this crate;
  enforced by its `BoundaryRule`
  (`cargo test -p ironclaw_architecture_tests --test reborn_dependency_boundaries reborn_crate_dependency_boundaries_hold`).
- **No `Loop*Port` is defined here** — port definitions live in
  `ironclaw_loop_contracts` with one import path per port
  (`--test reborn_loop_port_location_scan`).
- **State stores no raw content** — never a raw prompt, raw model output, tool
  argument, secret, host path, or provider diagnostic.
- New behavior is a new sealed strategy, never a branch in the canonical
  executor.

## Tests

```bash
cargo test -p ironclaw_agent_loop
cargo test -p ironclaw_turn_runner            # when drivers/loop-host integration is affected
cargo test -p ironclaw_architecture_tests     # after dependency/API changes
```

## See also

Family rules: `crates/loop/AGENTS.md` · working rules: `AGENTS.md` beside this
file · design record: `docs/reborn/target-architecture/families/loop.md`
(§6.7.1).
