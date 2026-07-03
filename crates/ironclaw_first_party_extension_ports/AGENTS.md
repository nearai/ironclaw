# Agent Map — ironclaw_first_party_extension_ports

## Start Here

- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these neighboring contracts before changing behavior:
  - `crates/ironclaw_first_party_extensions/AGENTS.md`
  - `crates/ironclaw_loop_support/AGENTS.md`
  - `docs/reborn/contracts/skills-extension.md`

## What This Crate Owns

- Loop-facing adapters and ports for first-party userland extensions.
- Skill activation/context/execution adapters that depend on loop-support and turn-run types.
- Scoped handles granted to bundled skill-context implementations.

## Do Not Move In Here

- Concrete tool behavior that can live below host runtime in `ironclaw_first_party_extensions`.
- Generic extension manifests, install state, activation lifecycle, registry, or store contracts.
- Runtime authority, raw host services, secrets, network clients, dispatcher handles, or lower substrate handles.
- Product workflow or root application composition.

## Validation

- Fast local check: `cargo test -p ironclaw_first_party_extension_ports`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture reborn_crate_dependency_boundaries_hold`
- Composition check when exposed ports change: `cargo test -p ironclaw_reborn_composition local_dev_runtime`

## Agent Notes

- Keep loop-facing adapters separate from concrete userland implementation.
- Preserve explicit scoped handles; do not introduce ambient runtime authority.
- Add or update architecture boundary rules whenever dependencies change.
