# Agent Map — ironclaw_loop_support

## Start Here

- Read `CLAUDE.md` first; it is the crate-local guardrail file.
- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these neighboring contracts before changing behavior:
  - `crates/ironclaw_turns/AGENTS.md`
  - `crates/ironclaw_capabilities/AGENTS.md`
  - `crates/ironclaw_skills/AGENTS.md`
  - `crates/ironclaw_reborn/CLAUDE.md`

## What This Crate Owns

- Loop host support services for `AgentLoopHost` / `ironclaw_turns` loop ports.
- `skill_context.rs` and `identity_context.rs` prompt-safe instruction/context builders.
- `capability_port.rs`, `capability_surface_filter.rs`, and `capability_allow_set.rs` capability-surface adapters.
- `input_queue.rs` / `input_port.rs` steering and followup queues.
- `cancellation_port.rs` cancellation observation adapter.
- `skill_bundle_source.rs` / `filesystem_skill_bundle_source.rs` skill-bundle source ports (`SkillBundleSource`, `FilesystemSkillBundleSource`, `SkillBundleDescriptor`/`SkillBundleId`/`SkillBundleProvenance`).

## Do Not Move In Here

- Core loop strategy or runner state transitions.
- Product workflow composition, runtime lane execution, or Reborn app wiring.
- Bypasses around `CapabilityHost` or dispatcher authority paths.
- Full prompt content where safe summaries/refs are required.

## Validation

- Fast local check: `cargo test -p ironclaw_loop_support`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture`
- Run `cargo test -p ironclaw_turns` and `cargo test -p ironclaw_reborn` when host-port contracts change.

## Agent Notes

- Add one file per host adapter or context source.
- Put capability filtering policy in `capability_surface_filter.rs`.
- Add traits here only for host-owned inputs to existing loop ports.
- Do not fold unrelated ports into `lib.rs`.
