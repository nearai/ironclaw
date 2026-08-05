# Agent Map — ironclaw_processes

## Start Here

- Read `CLAUDE.md` first; it is the crate-local guardrail file.
- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these Reborn contracts as the source of truth before changing behavior:
- `docs/reborn/contracts/processes.md`
- `docs/reborn/contracts/resources.md`
- `docs/reborn/contracts/events.md`

## What This Crate Owns

- Process lifecycle, journal, dependencies, invocation projections,
  cancellation, and generic process supervision.
- Journal-native capability invocation state: `ProcessInvocationRecord`,
  `ProcessInvocationStatus`, `ProcessInvocationStatePort`, and
  `ProcessInvocationStore`.
- Lifecycle types (`types`): `ProcessRecord`/`ProcessStatus`/`ProcessStart`/`ProcessExit`, `ProcessManager`, the `ProcessExecutor` trait and `ProcessExecutionRequest`/`ProcessExecutionResult`, `ProcessResultRecord`, and `ProcessError`/`ProcessExecutionError`.
- Stores: the row-native `ProcessJournalStore`, its lifecycle/dependency
  projections, and externalized `ProcessResultStore`.
- Process dependencies: atomic child submission/open, settle, consume/abandon,
  scoped group queries, and unresolved recovery enumeration.
- Cancellation (`cancellation`): `ProcessCancellationRegistry`, `ProcessCancellationToken`.
- Host + background management: `ProcessHost`/`ProcessSubscription` (`host`);
  the compatibility `BackgroundProcessManager` registers
  `ProcessKind::CapabilityInvocation` with `ProcessSupervisor`.
- Crate-local public API, tests, and fixtures needed to prove that ownership.

## Do Not Move In Here

- capability authorization, approval policy, or runtime lane internals outside adapter-facing contracts.
- Secrets, raw host paths, backend error details, and unredacted user content in errors, events, snapshots, logs, or docs.

## Validation

- Fast local check: `cargo test -p ironclaw_processes`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture_tests`
- If production persistence behavior changes, add/maintain PostgreSQL and libSQL parity tests.

## Agent Notes

- Keep edits inside this crate unless a contract explicitly requires a neighboring crate change.
- Do not create domain-owned dependency stores or recovery rosters. Domain
  wait/gate types are projections over `ProcessDependencyPort`.
- Prefer caller-level tests when a helper gates dispatch, persistence, network, secrets, approvals, resources, events, or process side effects.
- If the contract and code disagree, stop and treat the task as a contract-change request instead of silently changing ownership.
