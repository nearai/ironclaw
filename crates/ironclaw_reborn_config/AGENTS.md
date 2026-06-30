# Agent Map — ironclaw_reborn_config

## Start Here

- No crate-local CLAUDE.md exists yet; use this map plus `Cargo.toml` and source files.
- Read `src/lib.rs` for exports, then area files:
  - `home.rs` — Reborn home resolution.
  - `profile.rs` — profile contracts.
  - `boot.rs`, `config_file.rs` — boot/config file loading.
  - `doctor.rs` — config diagnostics.
  - `secrets_guard.rs` — secret/config guardrails.
- Neighboring consumers: `crates/ironclaw_reborn_cli/AGENTS.md`, `crates/ironclaw_reborn_composition/AGENTS.md`.

## What This Crate Owns

- Boot configuration contracts for the standalone IronClaw Reborn binary.
- Reborn home/profile/config-file/doctor/secrets-guard types and validation.
- Pure config parsing/validation helpers that can be shared by CLI and composition.

## Do Not Move In Here

- Runtime execution, product adapter workflow, host-runtime service construction, or CLI command dispatch.
- Writes to v1/current IronClaw state.
- Network, secret retrieval, database connection, or product side effects.

## Validation

- Fast local check: `cargo test -p ironclaw_reborn_config`
- Focused tests: `profile_contract`, `doctor_contract`, `home_contract`.
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture`

## Agent Notes

- Keep config contracts deterministic and side-effect light.
- Use explicit Reborn home/profile inputs; do not read ambient env from deep helpers unless that is the contract being tested.
- Add compatibility tests for serialized config/profile shapes.
