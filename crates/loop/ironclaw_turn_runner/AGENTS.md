# Agent Map — ironclaw_turn_runner

Working rules for the claimed-work control plane. Orientation lives in
`README.md`; family rules in `crates/loop/AGENTS.md`.

## Start Here

- Read `README.md` for what this crate is; read `Cargo.toml` for the
  dependency shape.
- Use these neighboring contracts before changing behavior:
  - `crates/loop/ironclaw_agent_loop/AGENTS.md`
  - `crates/loop/ironclaw_loop_host/AGENTS.md`
  - `crates/kernel/ironclaw_turns/AGENTS.md`
  - `crates/app/ironclaw_composition/CLAUDE.md`

## What This Crate Owns

- The agent-turn execution adapter (`turn_run_executor.rs`,
  `RebornTurnRunExecutor`), registered as the `ProcessKind::AgentTurn`
  executor with the kernel's process supervisor.
- `turn_scheduler.rs` — an **agent-turn projection over the generic process
  supervisor** (`ironclaw_processes::ProcessSupervisor`, since #6696): the
  claiming/heartbeat/lease mechanics live in the kernel; this file adapts them
  to turn vocabulary.
- `planned_driver.rs`, `planned_driver_factory.rs`, `driver_registry.rs`, and
  `text_loop_driver.rs` — driver behavior, registration, and readiness.
- `loop_driver_host.rs` — concrete loop host-port composition for claimed
  runs; `runtime.rs` — planned-runtime composition and the capability-port
  decorator chain (the decorator *ordering* is this crate's by charter).
- `loop_exit_applier.rs` — validation/application of loop exits and runner
  transitions; failure-lane/retry disposition (`failure_lane.rs`,
  `retry_disposition.rs`, `model_failure_mapping.rs` — its only callers are
  the two drivers, which is why it did not travel with the model gateway).
- `subagent/` incl. `subagent/await_edge/` — spawn admission and the
  await-edge machinery: store = journal-edge projection, resolver = loop-tier
  responsibility (PROPOSAL §12.13 D-S; see `README.md`).
- `trace_capture.rs` — the observer seam of the WS6 trace-capture split.
- `app_loop_family.rs` app loop-family composition and `milestone_events.rs`
  milestone event surfacing.
- `production_readiness.rs` — **no production caller**; a pure reporting slice
  awaiting either a startup gate or deletion (CHECKLIST WS4/WS8).

## Do Not Move In Here

- Host-port adapter implementations owned by `ironclaw_loop_host` — the model
  gateway, model-route policy, the driver-host port adapters
  (`HostManagedLoopCheckpointPort`, `HostManagedLoopProgressPort`,
  `NoExtraLoopInputPort`), and the tool-disclosure decorator all live there
  since the WS3 sheds. Import them; do not re-create them
  (`reborn_runner_sheds.rs` pins it).
- Loop family/executor behavior owned by `ironclaw_agent_loop`.
- Neutral runner/host contracts owned by `ironclaw_turns` and
  `ironclaw_loop_contracts` — do not duplicate their DTOs.
- Scheduling mechanics — claim, lease, heartbeat, recovery belong to
  `ironclaw_processes`; this crate only registers the executor and projects.
- Product-facing binding/idempotency/gate routing owned by product workflow.
- Hidden fallback from planned to text-only paths; fallback must be an
  explicit profile or readiness decision.

## Dependencies this crate no longer has

`ironclaw_llm`, `ironclaw_common`, `base64`, and `jsonschema` all left with
the model-gateway and tool-disclosure clusters (WS3). Re-adding any of them
means behavior came back with it; put that behavior in `ironclaw_loop_host`.
`reborn_runner_llm_wiring_is_isolated` (in `reborn_dependency_boundaries.rs`)
is the arbiter.

## Layer

`layer = "loops"` (PROPOSAL §6.7.3). It was `kernel` until the WS3 sheds:
with #6696's supervisor inversion the kernel reaches this crate only through
the registered `ProcessKind::AgentTurn` executor, and the two
`LAYER_MATRIX_EXCEPTIONS` it carried (`runner -> agent_loop`,
`runner -> loop_host`) dissolved rather than being allowlisted.

## Adding code

- Add a new file when adding a new driver, registry concern, host-factory
  concern, readiness check, or runtime-composition concern.
- Keep `runtime.rs` limited to planned-runtime composition and
  `planned_driver_factory.rs` limited to driver/profile factory wiring; move
  policy, readiness, or host-port construction into the owning file instead of
  growing either into a catch-all.
- Keep host factory code in `loop_driver_host.rs` only while it remains about
  composing loop ports for a claimed run.
- Add integration tests in `tests/` when behavior crosses driver, host,
  runner, or runtime composition.

## Common mistakes

- Do not expose planner strategy slots through Reborn APIs.
- Do not append product-live special cases to `PlannedDriver`.
- Do not hide new readiness checks or product policy inside runtime/factory
  wiring just because those files already touch many dependencies.

## Validation

- Fast local check: `cargo test -p ironclaw_turn_runner`
- Run specific integration tests when touched: `driver_registry`,
  `planned_driver_e2e`, `production_readiness`. (The `llm_gateway` and
  `model_routes` targets moved to `ironclaw_loop_host`.)
- Boundary check after dependency/API changes:
  `cargo test -p ironclaw_architecture_tests`
