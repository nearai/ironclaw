# ironclaw_event_log — working rules

Canonical crate guidance (the crate's `CLAUDE.md` is a pointer here).
Orientation and public surface: [`README.md`](./README.md). Family boundary
and the one-way pipeline rule: [`../AGENTS.md`](../AGENTS.md).

## Start here

- Read `README.md` for what the crate is; read `Cargo.toml` for actual
  dependencies and feature shape.
- Contract docs that outrank intuition: `docs/reborn/contracts/events.md`,
  `docs/reborn/contracts/events-projections.md`,
  `docs/reborn/contracts/kernel-boundary.md`.
- If the contract and code disagree, stop and treat the task as a
  contract-change request instead of silently changing ownership.

## Working rules

- **This crate is the substrate producers record through** — typed redacted
  `RuntimeEvent` records for already-authorized dispatch and process
  lifecycle transitions, `SecurityAuditEvent` envelopes, and the trait pairs:
  best-effort `EventSink`/`AuditSink` (a sink failure must **never** alter a
  runtime or control-plane outcome) and explicit-error
  `DurableEventLog`/`DurableAuditLog` with the monotonic per-scope cursor
  envelope and replay-after semantics.
- **Redaction-aware constructors are the security boundary.** They collapse
  unsafe error detail into `Unclassified` rather than leak it; do not add a
  constructor or field that bypasses `sanitize_error_kind` /
  `sanitize_error_summary`.
- **Replay is cursor-based only.** Compacting backends must store explicit
  cursors and cannot rely on line indexes — the byte-level
  `parse_jsonl`/`replay_jsonl` helpers were deleted with the `jsonl` module
  (PROPOSAL §6.3.1's "dead exports deleted" clause, executed); do not
  reintroduce byte-offset or line-indexed replay.
- **Keep storage drivers out.** Production backend selection lives in
  `crates/events/ironclaw_event_store/`; store crates depend on this
  substrate, never the reverse. The `DurableEventSink`/`DurableAuditSink`
  adapters exist so composition can pass durable logs where producers expect
  live sink traits — that is the whole bridging story; no more is needed here.
- No SSE/WebSocket product transport; no secrets, raw host paths, backend
  error details, or unredacted user content in errors, events, snapshots,
  logs, or docs.

## Dependency boundary

Internal deps: `ironclaw_host_api` only. The enforced deny-list is the
`ironclaw_event_log` `BoundaryRule` in
`crates/app/ironclaw_architecture_tests/tests/reborn_dependency_boundaries.rs`
(authorization, approvals, capabilities, extension_registry, host_runtime,
secrets, network, mcp, processes, resources, sandbox, wasm), plus the layer
matrix. `ironclaw_filesystem` and `ironclaw_memory` are **deliberately not
forbidden** by this note: the crate has no need for them today, but no
boundary case has been made for adding them to the list. If this doc and the
test ever disagree, the test wins and this doc is stale.

## Validation

- Fast local check: `cargo test -p ironclaw_event_log`
- Boundary check after dependency/API changes:
  `cargo test -p ironclaw_architecture_tests`
- If production persistence behavior changes, add/maintain PostgreSQL and
  libSQL parity tests (they live with the store and the fabric).
- Prefer caller-level tests when a helper gates dispatch, persistence,
  network, secrets, approvals, resources, events, or process side effects.
