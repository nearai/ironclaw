# Agent Map — ironclaw_first_party_extensions

## Start Here

- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these neighboring contracts before changing behavior:
  - `crates/ironclaw_host_runtime/AGENTS.md`
  - `crates/ironclaw_filesystem/AGENTS.md`

## What This Crate Owns

- Concrete first-party userland extension implementations that ship with IronClaw.
- Deterministic tool behavior behind narrow explicit request types.
- Scoped handles granted by host runtime or composition.

## Do Not Move In Here

- Host runtime composition, authorization, approvals, resource accounting, or capability registry wiring.
- Loop-facing skill context ports, turn-run adapters, or Reborn composition wiring.
- Raw secrets, network clients, dispatcher handles, or ambient host authority.

## Validation

- Fast local check: `cargo test -p ironclaw_first_party_extensions`
- Caller check after tool behavior changes: `cargo test -p ironclaw_host_runtime --test first_party_coding_tools`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture reborn_crate_dependency_boundaries_hold`
