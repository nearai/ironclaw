# Agent Map — ironclaw_run_state

## Start Here

- Read `CLAUDE.md` first; it is the crate-local guardrail file.
- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these Reborn contracts as the source of truth before changing behavior:
- `docs/reborn/contracts/run-state.md`
- `docs/reborn/contracts/turn-runner.md`
- `docs/reborn/contracts/kernel-boundary.md`

## What This Crate Owns

- Durable invocation state and approval request records used by capability and approval flows, currently:
- Run lifecycle records: `RunStatus`, `RunRecord`, `RunStart`.
- Approval request records (control-plane state, not authority): `ApprovalStatus`, `ApprovalRecord`.
- Store traits `RunStateStore`, `ApprovalRequestStore`, `RunStateApprovalStore`, with in-memory (`InMemoryRunStateStore`, `InMemoryApprovalRequestStore`) and filesystem (`FilesystemRunStateStore`, `FilesystemApprovalRequestStore`) backends; `RunStateError`.
- Crate-local public API, tests, and fixtures needed to prove that ownership.
- All lookups/transitions are resource-owner scoped. Durable persistence is the `Filesystem*Store` pair over a `ScopedFilesystem` — there are no separate per-backend run-state stores; the PostgreSQL/libSQL choice (gated by the `postgres`/`libsql` features) is made at the `RootFilesystem` layer underneath. Writes use compare-and-swap (`CasExpectation::Version`) over versioned roots, degrading to a process-local mutation lock only on byte-only/`Unsupported` roots.

## Do Not Move In Here

- runtime execution, product projections, or raw prompts/assistant text/secrets/backend details in state.
- Secrets, raw host paths, backend error details, and unredacted user content in errors, events, snapshots, logs, or docs.

## Validation

- Fast local check: `cargo test -p ironclaw_run_state`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture`
- If production persistence behavior changes, add/maintain PostgreSQL and libSQL parity tests.

## Agent Notes

- Keep edits inside this crate unless a contract explicitly requires a neighboring crate change.
- Prefer caller-level tests when a helper gates dispatch, persistence, network, secrets, approvals, resources, events, or process side effects.
- If the contract and code disagree, stop and treat the task as a contract-change request instead of silently changing ownership.
