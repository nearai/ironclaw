# ironclaw_events

Runtime/process events and control-plane audit envelope sinks plus durable
append-log substrate for IronClaw Reborn.

This crate is the substrate that downstream auth/dispatcher/process/runtime
crates use to record what happened. It defines:

- typed redacted [`RuntimeEvent`] records for already-authorized dispatch and
  process lifecycle transitions;
- redaction-aware constructors that collapse unsafe error detail into
  `Unclassified` rather than leak it;
- best-effort [`EventSink`] / [`AuditSink`] traits whose failures must not
  alter runtime/control-plane outcomes;
- explicit-error [`DurableEventLog`] / [`DurableAuditLog`] traits with a
  monotonic per-scope cursor envelope and replay-after semantics;
- in-memory durable backends used by tests and reference loops;
- `DurableEventSink` / `DurableAuditSink` adapters that let service composition pass durable logs where producer crates expect live sink traits.

Reborn-owned production backend selection lives in
`crates/ironclaw_reborn_event_store/`. Keep storage drivers out of this
substrate crate: downstream store crates should depend on `ironclaw_events`,
not the other way around. The byte-level `parse_jsonl` and `replay_jsonl`
helpers remain available for simple durable adapters, but compacting backends
must store explicit cursors and cannot rely on line indexes.

Forbidden dependencies (enforced by `ironclaw_architecture`): authorization,
approvals, capabilities, dispatcher, extensions, host_runtime, secrets,
network, mcp, processes, resources, run_state, scripts, wasm.

`ironclaw_filesystem` and `ironclaw_memory` are **deliberately not forbidden**
by this local note: this crate has no need for them today, but no boundary case
has been made for adding them to the forbidden list. The authoritative
forbidden list lives in
`crates/ironclaw_architecture/tests/reborn_dependency_boundaries.rs`; if the
two ever disagree, the test wins and this doc is stale.
