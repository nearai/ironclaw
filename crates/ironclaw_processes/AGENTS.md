# Agent Map — ironclaw_processes

## Start Here

- Read `CLAUDE.md` first; it is the crate-local guardrail file.
- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these Reborn contracts as the source of truth before changing behavior:
- `docs/reborn/contracts/processes.md`
- `docs/reborn/contracts/resources.md`
- `docs/reborn/contracts/events.md`

## What This Crate Owns

- Process lifecycle, stores, cancellation, and the background process manager, currently:
- Lifecycle types (`types`): `ProcessRecord`/`ProcessStatus`/`ProcessStart`/`ProcessExit`, `ProcessManager`, the `ProcessExecutor` trait and `ProcessExecutionRequest`/`ProcessExecutionResult`, `ProcessResultRecord`, and `ProcessError`/`ProcessExecutionError`.
- Stores: the `ProcessStore`/`ProcessResultStore` traits with one production implementation pair, `FilesystemProcessStore<F>` / `FilesystemProcessResultStore<F>` (`filesystem_store`) — exercised over `InMemoryBackend` in tests via the `test-support` helpers (`test_support`: `in_memory_backed_process_store` / `_result_store` / `_process_services`, arch-simplification §4.3) and over durable backends in production — plus the `EventingProcessStore` / `ResourceManagedProcessStore` wrappers (`wrappers`).
- Cancellation (`cancellation`): `ProcessCancellationRegistry`, `ProcessCancellationToken`.
- Host + background management: `ProcessHost`/`ProcessSubscription` (`host`); `BackgroundProcessManager`, `ProcessServices`, and `BackgroundErrorHandler`/`BackgroundFailure`/`BackgroundFailureStage` (`services`).
- Crate-local public API, tests, and fixtures needed to prove that ownership.

## Do Not Move In Here

- capability authorization, approval policy, or runtime lane internals outside adapter-facing contracts.
- Secrets, raw host paths, backend error details, and unredacted user content in errors, events, snapshots, logs, or docs.

## Validation

- Fast local check: `cargo test -p ironclaw_processes`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture`
- If production persistence behavior changes, add/maintain PostgreSQL and libSQL parity tests.

## Agent Notes

- Keep edits inside this crate unless a contract explicitly requires a neighboring crate change.
- Prefer caller-level tests when a helper gates dispatch, persistence, network, secrets, approvals, resources, events, or process side effects.
- If the contract and code disagree, stop and treat the task as a contract-change request instead of silently changing ownership.
