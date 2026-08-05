# Agent Map — ironclaw_composition

## Start Here

- Read `CLAUDE.md` first; it is the crate-local guardrail file.
- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these neighboring contracts before changing behavior:
  - `crates/ironclaw_turn_runner/AGENTS.md`
  - `crates/ironclaw_config/AGENTS.md`
  - `crates/ironclaw_host_runtime/AGENTS.md`
  - `crates/ironclaw_turns/AGENTS.md`

## What This Crate Owns

- Service-shaped production composition root for Reborn.
- Top-level factories for runtime/profile inputs and storage substrate wiring: `build_reborn_runtime` (`src/runtime.rs`) is the assembly entry point callers use — it is the name pinned by the composition boundary test, and it delegates to the equally public `build_runtime` beside it; `build_runtime_substrate` in `src/factory.rs` is the `pub(crate)` builder underneath both, plus `RebornHostBindings`/`RebornBuildError` (`src/input.rs`, `src/error.rs`).
- LLM catalog wiring is **not** owned here — `llm_catalog` belongs to `ironclaw_operator` (`crates/ironclaw_operator/src/llm_admin/llm_catalog.rs`); this crate's `src/llm_admin/` holds only `nearai_login_serve`, `nearai_mcp`, and `openai_compat_serve`.
- The `RebornRuntime` conversation-level service (`RebornRuntime`/`build_reborn_runtime`, `AssistantReply`, `ConversationId`, `RebornRuntimeError`) owns the composed `HostRuntime`, `TurnCoordinator`, readiness, and runtime inputs (`RebornRuntimeInput`/`RebornRuntimeIdentity`, `TurnRunnerSettings`/`PollSettings`, heartbeat/poll-interval defaults).
- Test-support-only ProductLive adapter fixtures, plus production WebUI service wiring.
- Production and migration-dry-run profile validation for required handles (`profile`, `readiness`).

## Do Not Move In Here

- Root `ironclaw` crate or `src/` module dependencies.
- Lower substrate handles in public service APIs.
- Legacy bridge modes without accepted migration contract.
- Live v1/product traffic routing; callers must opt into explicit Reborn adapters.
- Low-level policy internals owned by service crates.

## Validation

- Fast local check: `cargo test -p ironclaw_composition`
- Run profile/runtime tests when composition/profile behavior changes.
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture_tests`
- Run `scripts/reborn-e2e-rust.sh` for production wiring changes.

## Agent Notes

- Keep composition service small and explicit.
- Fail closed on local-only or missing required handles in production/migration-dry-run profiles.
- Add readiness checks near the composed dependency they validate.
