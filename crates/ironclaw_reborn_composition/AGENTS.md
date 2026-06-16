# Agent Map — ironclaw_reborn_composition

## Start Here

- Read `CLAUDE.md` first; it is the crate-local guardrail file.
- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these neighboring contracts before changing behavior:
  - `crates/ironclaw_reborn/AGENTS.md`
  - `crates/ironclaw_reborn_config/AGENTS.md`
  - `crates/ironclaw_host_runtime/AGENTS.md`
  - `crates/ironclaw_turns/AGENTS.md`

## What This Crate Owns

- Facade-shaped production composition root for Reborn.
- Top-level factories that expose `HostRuntime`, `TurnCoordinator`, readiness, runtime/profile inputs, and LLM catalog wiring: `RebornServices`/`build_reborn_services` (`factory`), `RebornBuildInput`/`RebornBuildError`, and the feature-gated LLM catalog resolvers (`llm_catalog`).
- The `RebornRuntime` conversation-level facade (`RebornRuntime`/`build_reborn_runtime`, `AssistantReply`, `ConversationId`, `RebornRuntimeError`) and its runtime inputs (`RebornRuntimeInput`/`RebornRuntimeIdentity`, `TurnRunnerSettings`/`PollSettings`, heartbeat/poll-interval defaults).
- Product-live adapter wiring (`product_live_adapters`): `ProductLivePlannedRuntimeAdapters`, capability authority/IO/model-route settings, `capability_allowlist`, `visible_capability_request_for_run`; and the WebUI facade (`webui`).
- Production and migration-dry-run profile validation for required handles (`profile`, `readiness`).

## Do Not Move In Here

- Root `ironclaw` crate or `src/` module dependencies.
- Lower substrate handles in public facade APIs.
- Legacy bridge modes without accepted migration contract.
- Live v1/product traffic routing; callers must opt into explicit Reborn adapters.
- Low-level policy internals owned by service crates.

## Validation

- Fast local check: `cargo test -p ironclaw_reborn_composition`
- Run profile/runtime tests when composition/profile behavior changes.
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture`
- Run `scripts/reborn-e2e-rust.sh` for production wiring changes.

## Agent Notes

- Keep composition facade small and explicit.
- Fail closed on local-only or missing required handles in production/migration-dry-run profiles.
- Add readiness checks near the composed dependency they validate.
