# Agent Map — ironclaw_network

## Start Here

- Read `CLAUDE.md` first; it is the crate-local guardrail file.
- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these Reborn contracts as the source of truth before changing behavior:
- `docs/reborn/contracts/network.md`
- `docs/reborn/contracts/kernel-boundary.md`
- `docs/reborn/contracts/resources.md`

## What This Crate Owns

- Network policy boundary and hardened host/provider HTTP transport substrate.
- Crate-local public API, tests, and fixtures needed to prove that ownership.

## Do Not Move In Here

- runtime-lane behavior above the boundary or manual credential injection.
- Secrets, raw host paths, backend error details, and unredacted user content in errors, events, snapshots, logs, or docs.

## Validation

- Fast local check: `cargo test -p ironclaw_network`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture`
- If production persistence behavior changes, add/maintain PostgreSQL and libSQL parity tests.

## Agent Notes

- Keep edits inside this crate unless a contract explicitly requires a neighboring crate change.
- Prefer caller-level tests when a helper gates dispatch, persistence, network, secrets, approvals, resources, events, or process side effects.
- If the contract and code disagree, stop and treat the task as a contract-change request instead of silently changing ownership.
