# Agent Map — ironclaw_memory

## Start Here

- Read `CLAUDE.md` first; it is the crate-local guardrail file.
- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these Reborn contracts as the source of truth before changing behavior:
- `docs/reborn/contracts/memory.md`
- `docs/reborn/contracts/storage-placement.md`
- `docs/reborn/contracts/kernel-boundary.md`

## What This Crate Owns

- Memory document services, backend adapters, metadata/search, filesystem projection, and prompt-context safety seams.
- Crate-local public API, tests, and fixtures needed to prove that ownership.

## Do Not Move In Here

- generic filesystem semantics, direct provider HTTP, raw secret handling, and loop prompt strategy.
- Secrets, raw host paths, backend error details, and unredacted user content in errors, events, snapshots, logs, or docs.

## Validation

- Fast local check: `cargo test -p ironclaw_memory`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture`
- If production persistence behavior changes, add/maintain PostgreSQL and libSQL parity tests.

## Agent Notes

- Keep edits inside this crate unless a contract explicitly requires a neighboring crate change.
- Prefer caller-level tests when a helper gates dispatch, persistence, network, secrets, approvals, resources, events, or process side effects.
- If the contract and code disagree, stop and treat the task as a contract-change request instead of silently changing ownership.
