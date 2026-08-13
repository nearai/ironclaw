# ironclaw_turn_runner

The agent-turn executor, the driver registry, and the loop-host factory — the
trusted adapter between a kernel-claimed run and loop userland. It registers
the `ProcessKind::AgentTurn` executor with the kernel's process supervisor
(dependency inversion: the kernel defines the port, this crate registers into
it), hands each claimed run only the ports scoped to it, and carries the
loop's claimed exit to the turn kernel's exit applier — which validates it
before anything durable commits. It never decides durability itself.

- **Family / layer:** `loop` / `loops` · **Package:** `ironclaw_turn_runner` · **Manifest:** `crates/loop/ironclaw_turn_runner/Cargo.toml`
- **Use this when:** changing drivers, driver readiness, the capability-port
  decorator *ordering*, failure-lane/retry disposition, exit application, or
  the subagent await-edge resolver.
- **Don't use this when:** changing loop strategy → `ironclaw_agent_loop`;
  implementing a port adapter → `ironclaw_loop_host`; scheduling mechanics
  (claim/lease/heartbeat/recovery) → `crates/kernel/ironclaw_processes`.

## Public surface

- `RebornTurnRunExecutor` (`turn_run_executor.rs`) — the executor registered
  with the process supervisor; `turn_scheduler.rs` is an agent-turn
  *projection* over the generic supervisor (#6696), not a scheduler of its
  own.
- `DriverRegistry` + the two production drivers: `PlannedDriver` (adapts
  `ironclaw_agent_loop`) and `text_loop_driver.rs` (smallest supported
  behavior). Fallback between them is an explicit profile/readiness decision,
  never silent.
- `loop_driver_host.rs` / `runtime.rs` — the loop-host factory that composes a
  claimed run's port set and the capability-port decorator chain (the
  *ordering* lives here by charter).
- `loop_exit_applier.rs` — validates loop exits against host-minted evidence.
- `subagent/await_edge/` — the await-edge machinery: the *store* is a pure
  projection over `ironclaw_processes::ProcessDependencyPort`; the *resolver*
  stays here as a genuine loop-tier responsibility (owner recovery,
  child-transcript materialization behind the untrusted-text fence, batch-gate
  drain with exactly-once parent resume, `BlockedDependentRunGate` resume
  policy) — PROPOSAL §12.13 D-S.
- `trace_capture.rs` — the turn-runner half of the WS6 trace-capture split
  (`TurnEventSink` + history port); the pipeline lives in
  `ironclaw_trace_commons::capture`.

## Depends on / consumed by

- **Normal workspace deps (17):** `ironclaw_agent_loop`, `ironclaw_loop_host`,
  `ironclaw_hooks` (the declared decorator chain), kernel (`ironclaw_turns`,
  `ironclaw_processes`, `ironclaw_host_runtime`, `ironclaw_approvals`),
  contracts (`ironclaw_host_api`, `ironclaw_loop_contracts`), plus
  `ironclaw_event_log`, `ironclaw_filesystem`, `ironclaw_memory`,
  `ironclaw_observability`, `ironclaw_outbound`, `ironclaw_safety`,
  `ironclaw_threads`, `ironclaw_trace_commons`. **Not `ironclaw_llm`** — the
  provider cone left with the model gateway (WS3); re-adding it means provider
  behavior came back.
- **Consumed by (1):** `ironclaw_composition`. Never the reverse.

## Invariants

- **The shed items stay in `ironclaw_loop_host`** — model gateway, model
  routes, driver-host adapters, tool disclosure
  (`--test reborn_runner_sheds`), and the runner's LLM isolation
  (`--test reborn_dependency_boundaries reborn_runner_llm_wiring_is_isolated`).
- **The await-edge store is a projection, not a second journal** — pinned at
  the integration tier by
  `tests/integration/subagent_await_edge.rs::runner_await_edge_is_a_projection_over_process_dependencies`.
- No planner strategy slot is exposed through a public API; no neutral
  vocabulary is duplicated from `ironclaw_turns`/`ironclaw_loop_contracts`.
- `production_readiness.rs` has no production caller — a reporting slice
  awaiting a startup gate or deletion (CHECKLIST WS4/WS8); do not build on it.

## Tests

```bash
cargo test -p ironclaw_turn_runner
cargo test -p ironclaw_architecture_tests    # after dependency/API changes
```

## See also

Family rules: `crates/loop/AGENTS.md` · working rules: `AGENTS.md` beside this
file · design record: `docs/internal/reborn/target-architecture/families/loop.md`
(§6.7.3) + PROPOSAL §12.13 D-S.
