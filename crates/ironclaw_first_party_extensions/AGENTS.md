# Agent Map — ironclaw_first_party_extensions

## Start Here

- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these neighboring contracts before changing behavior:
  - `crates/ironclaw_extensions/AGENTS.md`
  - `crates/ironclaw_loop_support/AGENTS.md`
  - `docs/reborn/contracts/skills-extension.md`

## What This Crate Owns

- Concrete first-party userland extensions that ship with IronClaw.
- Narrow in-process ports exposed back to Reborn composition.
- Scoped handles granted to bundled extension implementations.

## Do Not Move In Here

- Generic extension manifests, install state, activation lifecycle, registry, or store contracts.
- Runtime authority, raw host services, secrets, network clients, dispatcher handles, or lower substrate handles.
- Product workflow or root application composition.

## Validation

- Fast local check: `cargo test -p ironclaw_first_party_extensions`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture reborn_crate_dependency_boundaries_hold`
- Composition check when exposed ports change: `cargo test -p ironclaw_reborn_composition local_dev_runtime`

## Agent Notes

- Keep concrete extension implementation separate from `ironclaw_extensions`, which owns generic extension platform contracts.
- Preserve explicit scoped handles; do not introduce ambient runtime authority.
- Add or update architecture boundary rules whenever dependencies change.
