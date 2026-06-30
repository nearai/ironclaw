# Agent Map — ironclaw_events

## Start Here

- Read `CLAUDE.md` first; it is the crate-local guardrail file.
- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these Reborn contracts as the source of truth before changing behavior:
- `docs/reborn/contracts/events.md`
- `docs/reborn/contracts/events-projections.md`
- `docs/reborn/contracts/kernel-boundary.md`

## What This Crate Owns

- Typed redacted event/audit substrate, currently:
- Runtime/process event records: `RuntimeEvent`, `RuntimeEventId`, `RuntimeEventKind`, plus redaction helpers `sanitize_error_kind` / `UNCLASSIFIED_ERROR_KIND` (`runtime_event`).
- Best-effort sink traits (`EventSink`, `AuditSink`) and explicit-error durable-log traits (`DurableEventLog`, `DurableAuditLog`) with their `DurableEventSink` / `DurableAuditSink` adapters (`sink`).
- Per-scope cursor/replay envelope: `EventCursor`, `EventLogEntry`, `EventReplay`, `EventStreamKey`, `ReadScope` (`cursor`).
- In-memory durable/sink backends for tests and reference loops (`in_memory`) and the byte-level `parse_jsonl` / `replay_jsonl` helpers (`jsonl`).
- `EventError` (`error`).
- Crate-local public API, tests, and fixtures needed to prove that ownership.
- Production backend selection lives in `ironclaw_reborn_event_store`, not here — downstream store crates depend on this substrate, never the reverse.

## Do Not Move In Here

- SSE/WebSocket product transport.
- Secrets, raw host paths, backend error details, and unredacted user content in errors, events, snapshots, logs, or docs.

## Validation

- Fast local check: `cargo test -p ironclaw_events`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture`
- If production persistence behavior changes, add/maintain PostgreSQL and libSQL parity tests.

## Agent Notes

- Keep edits inside this crate unless a contract explicitly requires a neighboring crate change.
- Prefer caller-level tests when a helper gates dispatch, persistence, network, secrets, approvals, resources, events, or process side effects.
- If the contract and code disagree, stop and treat the task as a contract-change request instead of silently changing ownership.
