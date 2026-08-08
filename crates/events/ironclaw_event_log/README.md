# ironclaw_event_log

The redacted event and audit vocabulary, and the sink/log traits every producer
in the system records through — with no storage driver of its own. It is a
separate crate because it is the one neutral contract every producer needs, and
its independence from any driver is what lets `ironclaw_event_store` exist as
an isolated crate at all: if evidence vocabulary and durable-backend selection
lived together, every producer would compile a database and TLS stack it never
touches.

- **Family / layer:** `crates/events/` / `substrates` · **Package:** `ironclaw_event_log` · **Manifest:** `crates/events/ironclaw_event_log/Cargo.toml`
- **Use this when:** you record a fact (dispatch outcome, process transition,
  security-audit decision) or define what a recordable fact looks like.
- **Don't use this when:** you need a durable backend constructed (→
  `ironclaw_event_store`, via composition), a read model (→
  `ironclaw_event_projections`), or a subscription (→
  `ironclaw_event_streams`).

## Public surface

Six private modules re-exported flat (`src/lib.rs`):

- `RuntimeEvent` / `RuntimeEventId` / `RuntimeEventKind` and the
  `SecurityAuditEvent` shapes — with **sanitizing constructors**
  (`sanitize_error_kind`, `sanitize_error_summary`, `UNCLASSIFIED_ERROR_KIND`)
  that collapse unsafe error detail rather than leak it.
- Best-effort sink traits: `EventSink` / `AuditSink` (+ `SecurityAuditSink`) —
  a sink failure must never alter a runtime or control-plane outcome.
- Explicit-error durable traits: `DurableEventLog` / `DurableAuditLog`, with
  the `DurableEventSink` / `DurableAuditSink` adapters composition uses to
  hand durable logs to producers expecting sink traits.
- Cursor/replay envelope: `EventCursor` (monotonic, per-stream),
  `EventLogEntry`, `EventReplay`, `EventStreamKey`, `ReadScope`. Replay is
  cursor-based only — no byte-offset or line-index helpers — so a backend can
  compact without breaking a consumer's resume position (the old
  `parse_jsonl`/`replay_jsonl` helpers are deleted).
- In-memory reference implementations for tests and reference loops;
  `EventError`.

## Depends on / consumed by

- **Internal deps:** `ironclaw_host_api` only — the family leaf.
- **Consumed by 14 workspace crates + the integration-test root** (reproduce:
  `grep -rl '^ironclaw_event_log = ' --include=Cargo.toml crates Cargo.toml`):
  producers across kernel (`capabilities`, `approvals`, `host_runtime`,
  `processes`), loop (`turn_runner`, `hooks`), domains (`auth`, `outbound`),
  lanes (`wasm`), product (`assistant`), composition, and the family's own
  store/projections (+ streams as dev-dep).

## Invariants

- **Redaction at construction.** Constructors collapse unsafe error categories
  into bounded classifications and truncate free-form summaries; nothing
  durable or replayable carries a raw secret, host path, token, approval
  reason, fingerprint, or lease. Do not add a constructor that bypasses this.
- **Best-effort sinks stay best-effort** — a failure is observable, never
  outcome-altering; the durable traits are the explicit-error pair.
- **No storage drivers, no transports, no projection policy** — the
  `ironclaw_event_log` `BoundaryRule` in `reborn_dependency_boundaries.rs`
  forbids the privileged tiers, and the layer matrix caps the crate at
  substrates.
- **Cursor-only replay** with an explicit replay-gap error for a cursor older
  than the earliest retained entry.

## Tests

```bash
cargo test -p ironclaw_event_log
cargo test -p ironclaw_architecture_tests
```

## See also

- Working rules: [`AGENTS.md`](./AGENTS.md) (canonical crate guidance;
  `CLAUDE.md` points here).
- Family boundary and the one-way pipeline rule: [`../AGENTS.md`](../AGENTS.md).
- Design record: PROPOSAL §6.3.1;
  `docs/reborn/target-architecture/families/events.md`; frozen contract
  `docs/reborn/contracts/events.md`.
