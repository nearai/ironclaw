# Agent Map — ironclaw_host_runtime

## Start Here

- Read `CLAUDE.md` first; it is the crate-local guardrail file.
- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these Reborn contracts as the source of truth before changing behavior:
- `docs/reborn/contracts/host-runtime.md`
- `docs/reborn/contracts/runtime-workflows.md`
- `docs/reborn/contracts/kernel-boundary.md`

## What This Crate Owns

- Host-side composition facade for Reborn runtime lanes and shared kernel-facing services/adapters.
- Crate-local public API, tests, and fixtures needed to prove that ownership.

## Do Not Move In Here

- product loop strategy, prompt assembly, channel UX, migrations, or duplicated low-level network/secrets/resource logic.
- Secrets, raw host paths, backend error details, and unredacted user content in errors, events, snapshots, logs, or docs.

## Validation

- Fast local check: `cargo test -p ironclaw_host_runtime`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture`
- If production persistence behavior changes, add/maintain PostgreSQL and libSQL parity tests.

## Agent Notes

- Keep edits inside this crate unless a contract explicitly requires a neighboring crate change.
- Prefer caller-level tests when a helper gates dispatch, persistence, network, secrets, approvals, resources, events, or process side effects.
- If the contract and code disagree, stop and treat the task as a contract-change request instead of silently changing ownership.
