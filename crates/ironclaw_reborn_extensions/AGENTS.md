# Agent Map — ironclaw_reborn_extensions

## Start Here

- Read `Cargo.toml` for actual dependencies and feature shape.
- Read `crates/ironclaw_loop_support/AGENTS.md` before changing loop-facing skill context behavior.
- Read `crates/ironclaw_filesystem/AGENTS.md` before changing scoped filesystem handle usage.

## What This Crate Owns

- First-party userland Reborn extensions that receive explicit scoped handles.
- Narrow extension ports that adapt first-party packages into loop-facing services.
- Reborn skill extension handle validation and composition helpers.

## Do Not Move In Here

- Ambient runtime authority, dispatcher/network/secrets handles, or approval policy.
- Reborn runtime composition wiring; keep that in `ironclaw_reborn_composition`.
- Low-level filesystem, skill parsing, or loop snapshot policy that belongs to owning crates.

## Validation

- Fast local check: `cargo test -p ironclaw_reborn_extensions`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture reborn_`
- Run `cargo test -p ironclaw_reborn_composition local_dev_runtime` when default runtime wiring changes.
