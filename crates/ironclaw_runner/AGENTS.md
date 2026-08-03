# Agent Map — ironclaw_runner

## Start Here

- Read `CLAUDE.md` first; it is the crate-local guardrail file.
- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these neighboring contracts before changing behavior:
  - `crates/ironclaw_agent_loop/CLAUDE.md`
  - `crates/ironclaw_turns/AGENTS.md`
  - `crates/ironclaw_loop_host/CLAUDE.md`
  - `crates/ironclaw_reborn_composition/CLAUDE.md`

## What This Crate Owns

- Standalone Reborn composition/adapters bridging neutral contracts to concrete Reborn loop execution.
- `planned_driver.rs`, `planned_driver_factory.rs`, `driver_registry.rs`, and `text_loop_driver.rs` driver behavior/registration/readiness.
- `turn_scheduler.rs` scheduler-backed claiming, heartbeat, lease recovery, bounded concurrency, wake, and shutdown behavior.
- `loop_driver_host.rs` concrete loop host-port composition for claimed runs.
- `loop_exit_applier.rs` validation/application of loop exits and runner transitions.
- `app_loop_family.rs` app loop-family composition and `milestone_events.rs` milestone event surfacing.
- `turn_runner.rs` the concrete turn-runner composition over the neutral `ironclaw_turns` runner contract.
- `runtime.rs` planned-runtime composition and the capability-port decorator chain, `production_readiness.rs`, and secrets/model runtime seams. (`model_gateway.rs`, `model_routes.rs`, and the tool-disclosure cluster moved to `ironclaw_loop_host` with the WS3 sheds.)

## Do Not Move In Here

- Host-port adapter implementations owned by `ironclaw_loop_host` — the model
  gateway, model-route policy, the driver-host port adapters, and the
  tool-disclosure decorator all live there since the WS3 sheds.
- Loop family/executor behavior owned by `ironclaw_agent_loop`.
- Neutral runner/host contracts owned by `ironclaw_turns`.
- Product-facing binding/idempotency/gate routing owned by product workflow.
- Hidden fallback from planned to text-only paths; fallback must be explicit product/ops policy.

## Validation

- Fast local check: `cargo test -p ironclaw_runner`
- Run specific integration tests when touched: `driver_registry`, `planned_driver_e2e`, `production_readiness`. The `llm_gateway` and `model_routes` targets moved to `ironclaw_loop_host`.
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture`

## Agent Notes

- Add a new file when adding a new driver, registry concern, host factory concern, or runtime adapter.
- Keep `runtime.rs` limited to planned-runtime composition and explicit profile/runtime setup.
- Do not expose planner strategy slots through Reborn APIs.
- Do not duplicate neutral DTOs from `ironclaw_turns`.
