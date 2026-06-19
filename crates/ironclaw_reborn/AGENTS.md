# Agent Map — ironclaw_reborn

## Start Here

- Read `CLAUDE.md` first; it is the crate-local guardrail file.
- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these neighboring contracts before changing behavior:
  - `crates/ironclaw_agent_loop/CLAUDE.md`
  - `crates/ironclaw_turns/AGENTS.md`
  - `crates/ironclaw_loop_support/CLAUDE.md`
  - `crates/ironclaw_reborn_composition/CLAUDE.md`

## What This Crate Owns

- Standalone Reborn composition/adapters bridging neutral contracts to concrete Reborn loop execution.
- `planned_driver.rs`, `planned_driver_factory.rs`, `driver_registry.rs`, and `text_loop_driver.rs` driver behavior/registration/readiness.
- `loop_driver_host.rs` concrete loop host-port composition for claimed runs.
- `loop_exit_applier.rs` validation/application of loop exits and runner transitions.
- `app_loop_family.rs` app loop-family composition and `milestone_events.rs` milestone event surfacing.
- `turn_runner.rs` the concrete turn-runner composition over the neutral `ironclaw_turns` runner contract.
- `runtime.rs`, `model_gateway.rs`, `model_routes.rs`, `production_readiness.rs`, and secrets/model runtime seams.

## Do Not Move In Here

- Loop family/executor behavior owned by `ironclaw_agent_loop`.
- Neutral runner/host contracts owned by `ironclaw_turns`.
- Product-facing binding/idempotency/gate routing owned by product workflow.
- Hidden fallback from planned to text-only paths; fallback must be explicit product/ops policy.

## Validation

- Fast local check: `cargo test -p ironclaw_reborn`
- Run specific integration tests when touched: `driver_registry`, `planned_driver_e2e`, `loop_driver_host`, `model_routes`, `production_readiness`.
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture`

## Agent Notes

- Add a new file when adding a new driver, registry concern, host factory concern, or runtime adapter.
- Keep `runtime.rs` limited to planned-runtime composition and explicit profile/runtime setup.
- Do not expose planner strategy slots through Reborn APIs.
- Do not duplicate neutral DTOs from `ironclaw_turns`.
