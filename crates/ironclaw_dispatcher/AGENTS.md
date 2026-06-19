# Agent Map — ironclaw_dispatcher

## Start Here

- Read `CLAUDE.md` first; it is the crate-local guardrail file.
- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these Reborn contracts as the source of truth before changing behavior:
- `docs/reborn/contracts/dispatcher.md`
- `docs/reborn/contracts/runtime-workflows.md`
- `docs/reborn/contracts/capability-access.md`

## What This Crate Owns

- The neutral runtime dispatch port that routes already-authorized capability requests to runtime lanes. Currently:
- `RuntimeDispatcher` and the `RuntimeAdapter` trait (all runtime lanes register through it — no direct WASM/Script/MCP deps), with `RuntimeAdapterRequest` / `RuntimeAdapterResult`.
- The re-exported `ironclaw_host_api` dispatch contracts: `CapabilityDispatcher`, `CapabilityDispatchRequest`, `CapabilityDispatchResult`, `DispatchError`, `RuntimeDispatchErrorKind` (runtime errors redacted to stable kinds at the public surface).
- Crate-local public API, tests, and fixtures needed to prove that ownership.

## Do Not Move In Here

- authorization, approval, trust, or obligation decisions; those must happen before dispatch.
- Secrets, raw host paths, backend error details, and unredacted user content in errors, events, snapshots, logs, or docs.

## Validation

- Fast local check: `cargo test -p ironclaw_dispatcher`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture`
- If production persistence behavior changes, add/maintain PostgreSQL and libSQL parity tests.

## Agent Notes

- Keep edits inside this crate unless a contract explicitly requires a neighboring crate change.
- Prefer caller-level tests when a helper gates dispatch, persistence, network, secrets, approvals, resources, events, or process side effects.
- If the contract and code disagree, stop and treat the task as a contract-change request instead of silently changing ownership.
