# Agent Map — ironclaw_extension_support

## Start Here

- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these neighboring contracts before changing behavior:
  - `crates/ironclaw_host_runtime/AGENTS.md`
  - `crates/ironclaw_filesystem/AGENTS.md`

## Where The Packages Live

- **Not here.** Every installable package is its own self-contained directory at
  `crates/extensions/packages/<extension-id>/` — manifest, prompts, schemas,
  committed `wasm/`, and the `wasm-src/` guest that produced it, together
  (PROPOSAL §5). This crate is the shared *support* crate beside them: the
  package inventory (`src/packages/`) and the native executors that serve many
  packages at once.
- A package module here embeds its package's data with
  `include_str!`/`include_bytes!` through `../../../packages/<id>/…`. If you add
  a package, add its directory under `packages/` and its module here, and put it
  in the `PACKAGES` table — the catalog is derived from that table, not from a
  directory scan.
- Rebuilding a WASM guest: `./scripts/build-wasm-extensions.sh --first-party`,
  then `python3 scripts/ci/check-wasm-artifact-freshness.py --update`. The
  freshness gate fails if you edit `wasm-src/` and do not do both.

## What This Crate Owns

- Concrete first-party userland extension implementations that ship with IronClaw.
- Deterministic tool behavior behind narrow explicit request types.
- Scoped handles granted by host runtime or composition.

## Do Not Move In Here

- Host runtime composition, authorization, approvals, resource accounting, or capability registry wiring.
- Loop-facing skill context ports, turn-run adapters, or Reborn composition wiring.
- Raw secrets, network clients, dispatcher handles, or ambient host authority.

## Validation

- Fast local check: `cargo test -p ironclaw_extension_support`
- Caller check after tool behavior changes: `cargo test -p ironclaw_host_runtime --test first_party_coding_tools`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture reborn_crate_dependency_boundaries_hold`
