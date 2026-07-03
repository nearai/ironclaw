# Agent Map — ironclaw_approvals

## Start Here

- Read `CLAUDE.md` first; it is the crate-local guardrail file.
- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these Reborn contracts as the source of truth before changing behavior:
- `docs/reborn/contracts/approvals.md`
- `docs/reborn/contracts/run-state.md`
- `docs/reborn/contracts/capability-access.md`

## What This Crate Owns

- The approval resolution workflow: resolving a pending (run-state-owned) approval record into a scoped capability lease or a denial. Currently:
- `ApprovalResolver` — the fail-closed resolver (persists the `approve` authority record before issuing the lease) and `ApprovalResolutionError`.
- Resolution outcomes: `LeaseApproval` (issued scoped lease) and `DenyApproval` (no lease).
- Best-effort, metadata-only approval audit emission (never alters resolution outcomes).
- Crate-local public API, tests, and fixtures needed to prove that ownership.
- Note: the durable approval *request records* are owned by `ironclaw_run_state`; this crate consumes them and produces the lease/denial decision.

## Do Not Move In Here

- reusable scoped approvals or dispatch before matching fingerprinted lease validation/claim.
- Secrets, raw host paths, backend error details, and unredacted user content in errors, events, snapshots, logs, or docs.

## Validation

- Fast local check: `cargo test -p ironclaw_approvals`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture`
- If production persistence behavior changes, add/maintain PostgreSQL and libSQL parity tests.

## Agent Notes

- Keep edits inside this crate unless a contract explicitly requires a neighboring crate change.
- Prefer caller-level tests when a helper gates dispatch, persistence, network, secrets, approvals, resources, events, or process side effects.
- If the contract and code disagree, stop and treat the task as a contract-change request instead of silently changing ownership.
