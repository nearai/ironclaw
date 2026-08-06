# ironclaw_turn_runner

Owns driver-side Reborn loop integration.

## Main entry points

- `planned_driver.rs` adapts `ironclaw_agent_loop` families and executor to the
  runner-facing `AgentLoopDriver` contract.
- `text_loop_driver.rs` is the legacy text-only Reborn driver.
- `driver_registry.rs` owns driver registration and readiness metadata.
- `planned_driver_factory.rs` wires the default planned driver and profile.
- `loop_driver_host.rs` composes concrete loop host ports for claimed runs.
- `loop_exit_applier.rs` validates loop exits and applies runner transitions.
- `turn_scheduler.rs` owns scheduler-backed claiming, heartbeat, lease recovery,
  bounded concurrency, wake, and shutdown behavior next to the per-run executor.
- `runtime.rs` builds default and product-live planned runtime compositions,
  and composes the capability-port decorator chain for a claimed run.
- `production_readiness.rs` validates production readiness of the Reborn loop
  composition. **No production caller** — it is a pure reporting slice awaiting
  either a startup gate or deletion (CHECKLIST WS4/WS8).

## Boundaries

- This crate bridges neutral contracts to concrete Reborn composition. It does
  not define strategy traits, loop state, or canonical executor mechanics.
- `ironclaw_agent_loop` owns loop families and executor behavior.
- `ironclaw_turns` owns runner and host contracts.
- `ironclaw_host_runtime` owns host services and production validation of the
  transition port supplied to the scheduler; it does not own runner control.
- `ironclaw_loop_host` owns reusable host-port adapters — including, since the
  WS3 sheds, the **model gateway** (`LlmProviderModelGateway`,
  `RoutedLlmProviderModelGateway`, `ThreadResolvingLoopModelGateway`), the
  **model-route policy vocabulary** (`ModelRoute`, `ModelSlot`,
  `ModelRouteResolver`, …), the **driver-host port adapters**
  (`HostManagedLoopCheckpointPort`, `HostManagedLoopProgressPort`,
  `NoExtraLoopInputPort`), and **progressive tool disclosure**
  (`ToolDisclosureCapabilityDecorator`, `ToolDisclosureMode`). Import them from
  there; do not re-create them here (`reborn_runner_sheds.rs` pins it).
- Product workflow owns product-facing binding/idempotency/gate routing; do not
  call around it from here.

## Adding code

- Add a new file when adding a new driver, registry concern, host-factory
  concern, readiness check, or runtime-composition concern.
- Keep `runtime.rs` limited to planned-runtime composition and
  `planned_driver_factory.rs` limited to driver/profile factory wiring. Move
  policy, readiness, or host-port construction into the owning file instead of
  growing either file into a composition catch-all.
- Keep host factory code in `loop_driver_host.rs` only while it remains about
  composing loop ports for a claimed run; move unrelated readiness or product
  policy elsewhere.
- Add integration tests in `tests/` when behavior crosses driver, host, runner,
  or runtime composition.

## Common mistakes

- Do not expose planner strategy slots through Reborn APIs.
- Do not duplicate neutral DTOs from `ironclaw_turns`.
- Do not append product-live special cases to `PlannedDriver`.
- Do not hide new readiness checks or product policy inside runtime/factory
  wiring just because those files already touch many dependencies.
- Do not silently fall back from planned to text-only paths; fallback must be an
  explicit profile or readiness decision.

## Layer

`layer = "loops"` (PROPOSAL §6.7.3). It was `kernel` until the WS3 sheds: with
#6696's scheduler inversion the kernel reaches this crate only through the
registered `ProcessKind::AgentTurn` executor, so `loops` is what it is — and the
two `LAYER_MATRIX_EXCEPTIONS` it carried (`runner -> agent_loop`,
`runner -> loop_host`) dissolved rather than being allowlisted.

## Dependencies this crate no longer has

`ironclaw_llm`, `ironclaw_common`, `base64`, and `jsonschema` all left with the
model-gateway and tool-disclosure clusters. Re-adding any of them means
behaviour came back with it; put that behaviour in `ironclaw_loop_host`.
