# Agent Map — ironclaw_architecture

## Start Here

- Read `CLAUDE.md` first; it is the crate-local guardrail file.
- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these IronClaw contracts as the source of truth before changing behavior:
- `docs/ironclaw/contracts/_contract-freeze-index.md`
- `docs/ironclaw/contracts/kernel-boundary.md`

## What This Crate Owns

- Workspace architecture contract tests and IronClaw dependency-boundary enforcement.
- Crate-local public API, tests, and fixtures needed to prove that ownership.

## Do Not Move In Here

- production runtime code, production dependencies, or soft-only boundaries when a mechanical test can enforce them.
- Secrets, raw host paths, backend error details, and unredacted user content in errors, events, snapshots, logs, or docs.

## Validation

- Fast local check: `cargo test -p ironclaw_architecture`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture`
- If production persistence behavior changes, add/maintain PostgreSQL and libSQL parity tests.

## Agent Notes

- Keep edits inside this crate unless a contract explicitly requires a neighboring crate change.
- Prefer caller-level tests when a helper gates dispatch, persistence, network, secrets, approvals, resources, events, or process side effects.
- If the contract and code disagree, stop and treat the task as a contract-change request instead of silently changing ownership.
