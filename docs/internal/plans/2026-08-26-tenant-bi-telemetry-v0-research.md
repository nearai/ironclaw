# Tenant BI Telemetry V0 — Shape Research

**Status:** Revised shape selected after scoped-authority audit, 2026-08-27
**Decision:** Extend the existing tenant-aware `ScopedFilesystem`; do not keep
the abandoned direct-SQL telemetry path and do not give the telemetry domain raw
root authority.

## Problem

IronClaw needs tenant-local BI facts without storing every model call, tool call,
or run as a telemetry row. Recording must be synchronous-from-the-producer,
fully asynchronous after intake, bounded, and best-effort. Ordinary deployments
should flush the small queue tail rather than lose an entire hour.

The initial implementation used dedicated libSQL/PostgreSQL tables. Review
showed that this created a second persistence plane even though the filesystem
substrate already provides typed entries, backend parity, ordered indexes,
mount isolation, and bounded CAS.

## Existing system map

| Concern | Existing owner/pattern |
|---|---|
| Trusted tenant authority | `ResourceScope` in `ironclaw_host_api` |
| Tenant path isolation | `ScopedFilesystem` plus composition’s `invocation_mount_view` |
| Production shared handle | `ironclaw_composition::wrap_scoped` |
| Restart-safe increment | shared `ironclaw_filesystem::cas_update` |
| Ordered keyset query | `query_ordered`, `OrderedPage`, `OrderedQueryCursor` |
| Typed indexed records | thread index and process/outbound filesystem stores |
| Trigger settlement | trigger-owned `TriggerFireSettlementObserver` and `active_cleanup` |
| Backend lifecycle | composition-owned composite root and libSQL/PostgreSQL assembly |
| Whole-path test | production trigger-poller tests plus root `tests/integration` harness |

Important constraints:

- Database guidance says consumers receive `ScopedFilesystem`; a typed store
  accepting raw root is a review flag.
- `cas_update` accepts `ScopedFilesystem`, `ResourceScope`, and
  `ScopedPath`.
- `query_ordered` supports equality prefixes plus one ordered key and
  tie-breaker. It does not accept `Filter::Range`.
- The existing trigger observer reports accepted submission and terminal
  failure; terminal success must be added at the trigger-owned settlement seam.
- Production multi-tenant composition uses one scoped handle whose resolver
  maps every operation. It does not create a handle per tenant.

## Alternatives

### A — Dedicated SQL telemetry tables

**Rejected.** It provides natural grouped transactions and arbitrary SQL
filters, but creates driver-specific adapters, migrations, dependency
allowlists, a dedicated ADR, and a second parity suite. The recorder abstraction
would make it replaceable, but the persistence fork remains real.

This was implemented only on an unmerged feature branch. No production
composition wrote rows, so replacement requires code/guidance deletion rather
than a data migration.

### B — Raw trusted RootFilesystem repository

**Rejected after audit.** A trusted multi-tenant worker can technically hold
root authority, but doing so makes the domain manufacture reserved
`/tenants/...` paths and cannot use the shared scoped CAS helper. It bypasses
the mount-view isolation that every ordinary domain store extends.

### C — One tenant-aware ScopedFilesystem repository

**Selected.** Composition creates one `ScopedFilesystem` over the existing
composite root. Each recorder call carries the canonical trusted
`ResourceScope`; each repository operation resolves
`/tenant-shared/telemetry/v0/...` through the existing mount view.

This:

- extends the canonical persistence plane;
- uses the existing tenant boundary rather than duplicating it;
- supports one multi-tenant worker without a per-tenant handle cache;
- preserves libSQL/PostgreSQL/in-memory parity through filesystem backends;
- supports concurrent additive updates through shared bounded CAS; and
- keeps producers and future HTTP handlers away from storage authority.

### D — Event-log observations and hourly projections

**Rejected.** The event log is replayable product truth. Appending every lossy
BI input would persist the high-frequency detail this design intentionally
avoids and would misrepresent best-effort observations as canonical evidence.

### E — Do nothing

**Rejected for the product goal.** Admins would continue lacking tenant-local
activity, automation, model, and retention inputs. Existing operational logs
are neither a privacy-reviewed BI contract nor a bounded tenant export source.

## Selected mechanics

### Intake and authority

```rust
fn try_record(
    &self,
    scope: ResourceScope,
    observation: TelemetryObservation,
) -> RecordOutcome;
```

The scope is trusted host context, not request data. Usage attribution comes
from it. The worker owns the queued scope and passes it to the repository.
Lifecycle events may separately name their bounded subject.

### Paths and writes

The repository owns relative `ScopedPath` grammar below
`/tenant-shared/telemetry/v0`. The mount resolver maps that to the current
tenant’s physical shared subtree. One aggregate record is updated with
`cas_update`.

A drained batch is intentionally not atomic. It may commit a prefix, reports
the applied prefix, and never replays an ambiguous additive write. This is an
accepted consequence of best-effort analytics, not an accidental weakening of
a product invariant.

### Reads

Time is the ordered index key, not a `Range` filter. Every physical ordered
index leads with tenant equality derived from the trusted scope. The reader starts an
ascending keyset page after `(from, minimum_tie_breaker)`; that reserved value
cannot be emitted by the tie-breaker encoder, so real rows at `from` are
included. Reading stops before `to`. Exact equality prefixes select separate
indexes for:

- no provider/model filter;
- provider only;
- model only; and
- provider plus model.

This keeps reads bounded without changing filesystem semantics. If conformance
testing shows the starting cursor still scans history before `from`, the plan
stops and a generic filesystem lower-bound contract is designed before
telemetry proceeds.

### Trigger producer

The trigger owner adds one terminal settlement event emitted by
`active_cleanup` after durable history and active-fire clearing for both
success and failure. The composition lookup preserves completed, failed,
cancelled, and recovery-required outcomes while trigger history keeps its
existing `Ok`/`Error` projection. Active cleanup builds creator scope from the
trusted persisted trigger record. Composition translates the event to
telemetry. No poller tick, submission callback, or parallel trigger observer
guesses completion.

### Deployment behavior

The recorder queue holds at most 8,192 observations, drains at 512 or one
second, and gets five seconds to close during shutdown. Deployments normally
lose at most a small unflushed tail. Crash loss remains acceptable and is
represented by collector coverage rather than a false losslessness claim.

## Replacement completeness

The replacement PR removes:

- telemetry libSQL/PostgreSQL adapters and driver dependencies;
- SQL repository/schema tests;
- ADR 0005;
- telemetry driver allowlists and dependency inventory exceptions;
- database/domain/target-architecture guidance naming the SQL exception; and
- the obsolete SQL implementation plan.

It replaces them with:

- an `ironclaw_filesystem` dependency;
- scoped repository conformance over in-memory and real libSQL backends;
- one production-scoped composition handle;
- trigger terminal settlement wiring; and
- one real-libSQL restart/isolation integration scenario.

The hardest-to-reverse step is the durable record grammar. Versioned record
kinds, deterministic paths, and conformance tests therefore land before the
admin export surface or additional producers.

## Proof and reconsideration triggers

The selected shape is accepted only if tests prove:

1. two tenant scopes using the same relative paths cannot observe each other;
2. exact `[from,to)` keyset reads skip history before `from`;
3. all provider/model filter combinations use compatible indexes;
4. concurrent hourly increments converge through bounded CAS;
5. trigger success and failure notify once after durable settlement;
6. graceful shutdown drains the bounded queue tail;
7. a fresh libSQL-backed root can read the aggregate after runtime teardown.

Reconsider the decision if production SQL telemetry data is discovered,
filesystem lower-bound reads cannot be bounded, CAS contention is unacceptable
under measured load, or required metrics demand forbidden content or per-run
durable rows.
