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

Filesystem-backed JSONL durable backends and PostgreSQL/libSQL backends are
deliberately deferred. They live in later grouped Reborn PRs that depend on
`ironclaw_filesystem` and the database substrates. The byte-level
`parse_jsonl` and `replay_jsonl` helpers in this crate are exposed so those
later backends can build on the same redaction and replay invariants.

Forbidden dependencies (enforced by `ironclaw_architecture`): authorization,
approvals, capabilities, dispatcher, extensions, host_runtime, secrets,
network, mcp, processes, resources, run_state, scripts, wasm.

`ironclaw_filesystem` is **deliberately not forbidden**: PR2's JSONL durable
sink will depend on it, and pre-tightening here would force PR2 to relax the
rule before it can land. `ironclaw_memory` is similarly not forbidden — this
crate has no need for it today, but no boundary case has been made for
adding it to the list. The authoritative forbidden list lives in
`crates/ironclaw_architecture/tests/reborn_dependency_boundaries.rs`; if the
two ever disagree, the test wins and this doc is stale.
