# Agent Map — ironclaw_approvals

## Start Here

- Read `README.md` for orientation (charter, measured deps/consumers, gates).
  This file is the canonical working-rules home; `CLAUDE.md` is a pointer here.
- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these Reborn contracts as the source of truth before changing behavior:
- `docs/internal/reborn/contracts/approvals.md`
- `docs/internal/reborn/contracts/run-state.md`
- `docs/internal/reborn/contracts/capability-access.md`

## What This Crate Owns

- Durable approval requests, decisions, and model-visible gate records.
- The approval resolution workflow: resolving a pending approval record into a scoped capability lease or a denial. Currently:
- `ApprovalResolver` — the fail-closed resolver (persists the `approve` authority record before issuing the lease) and `ApprovalResolutionError`.
- Resolution outcomes: `LeaseApproval` (issued scoped lease) and `DenyApproval` (no lease).
- Best-effort, metadata-only approval audit emission (never alters resolution outcomes).
- Crate-local public API, tests, and fixtures needed to prove that ownership.
- Filesystem-backed `ApprovalRequestStore` and `GateRecordStore` persistence.

## Guardrails

- Own durable approval requests, gate records, and the approval resolution
  workflow from pending record to scoped lease or denial.
- Do not prompt users, dispatch capabilities, manage processes, reserve
  resources, or import runtime/dispatcher/capability workflow crates.
- Approve fail-closed: persist `approve` (the authority record) first, then
  issue the lease. If the lease store fails after approval is persisted, the
  request stays `Approved` and the caller surfaces the lease error — no
  rollback to `Pending`. The approval record is the durable decision; lease
  re-issuance against an already-decided request is recoverable.
- Denials issue no lease.
- Audit emission is metadata-only and best-effort. Failures are logged at
  `debug!` and never alter resolution outcomes.

## Do Not Move In Here

- reusable scoped approvals or dispatch before matching fingerprinted lease validation/claim.
- Secrets, raw host paths, backend error details, and unredacted user content in errors, events, snapshots, logs, or docs.

## Validation

- Fast local check: `cargo test -p ironclaw_approvals`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture_tests`
- If production persistence behavior changes, add/maintain PostgreSQL and libSQL parity tests.

## Agent Notes

- Keep edits inside this crate unless a contract explicitly requires a neighboring crate change.
- Prefer caller-level tests when a helper gates dispatch, persistence, network, secrets, approvals, resources, events, or process side effects.
- If the contract and code disagree, stop and treat the task as a contract-change request instead of silently changing ownership.
