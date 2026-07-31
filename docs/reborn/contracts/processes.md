# IronClaw Reborn process lifecycle contract

**Date:** 2026-07-28
**Status:** Row-native journal contract
**Owner:** `crates/ironclaw_processes`

## Purpose and authority

`ironclaw_processes` owns the durable lifecycle of agent turns, capability
invocations, background work, dependencies, checkpoints, and spawn-tree
reservations. Authorization remains in `ironclaw_authorization`; approvals
remain in `ironclaw_approvals`; domain adapters such as `ironclaw_turns`
translate their vocabulary at the process boundary.

The sole lifecycle authority is `ProcessJournalStore`:

```text
typed process command
  -> validate scope, lease, lineage, quota, and dependency invariants
  -> atomically update participating materialized rows
  -> append one immutable lifecycle row
  -> return the committed snapshot
  -> emit an in-process wake hint
```

Observers and transport projections do not own state. A committed journal
mutation is successful even if an in-process wake hint fails; required
consumers must be replayable from durable journal cursors.

## Durable layout

The store uses `RootFilesystem` multi-key transactions under:

```text
/processes/materialized/metadata
/processes/materialized/process/{process_id}
/processes/materialized/input/{process_id}
/processes/materialized/checkpoint/{checkpoint_id}
/processes/materialized/tree/{root_process_id}
/processes/materialized/dependency/{dependent_id}/{dependency_id}
/processes/materialized/journal/{zero_padded_cursor}
/processes/materialized/{control,submission}/...
```

Each journal entry is immutable and cursor-keyed. Current-state reads address
typed rows and ordered sparse indexes; they never rebuild state by replaying
the journal. Mutations require `TxnCapability::MultiKey` and fail closed on
CAS-only backends.

`ProcessRuntimePort` is the complete composition surface. Consumers should
accept its narrower ports: submission, transition, control, snapshots,
journal, dependencies, trees, checkpoints, inputs, gates, and lifecycle
lookups.

## Lifecycle and leases

`ProcessLifecycleStatus` is the canonical status vocabulary:

```text
Queued -> Running -> Suspended/Completed/Failed/Cancelled/Stopped
                  -> StopRequested/CancelRequested
```

Claims mint scoped worker leases. Heartbeats extend the active lease only when
worker and token match. Expiry policy preserves the prior turn guarantees:

- expired `CancelRequested` work becomes terminal `Cancelled`;
- checkpoint-free work is safely requeued only within the bounded crash
  reclaim budget;
- checkpointed or reclaim-exhausted work becomes terminal `Failed` with
  sanitized `lease_expired` or `crash_retry_exhausted` evidence.

Retries must target the latest authoritative run for a turn. A replacement
checkpoint is linked to the new process identity, and child retries retain the
root lineage and descendant-cap policy.

## Trees, dependencies, and capability causality

Subagent children carry `parent_process_id`, the authoritative root, and a
descendant cap. Child creation, tree reservation, and optional dependency
creation are one journal command. Consuming or abandoning the dependency
releases the reservation idempotently.

Capability causality is stored in `CapabilityProcessMetadata.parent_process_id`
instead of the subagent reservation relation. Capability work therefore
retains its authoritative parent without consuming or forging a subagent
descendant slot.

## Idempotency and pagination

Submission idempotency keys use stable owner/scope axes, process kind, and
operation ID. The per-request `invocation_id` is deliberately excluded because
logical retries may mint a fresh invocation identity.

Queue claims keyset-page until the queue is exhausted or enough eligible work
is found. Owner and concurrency-class quota reads are reused per unique key;
quota-blocked prefixes cannot starve later eligible work.

## Compatibility and migration

Pre-row-native deployments may contain lifecycle data in:

```text
/turns/rows/v1
/turns/state.json
/run-state/.../runs
```

Upgrade code must import these deployed layouts idempotently, verify durable
read-back, and only then mark row-native authority initialized. Normal traffic
must fail with a typed migration-required result while a known legacy authority
is present. The transitional `/processes/journal/records` and
`/processes/journal/state.json` layouts are also read-only import inputs.

Ordered-index and projection migrations are separately restartable. Completion
markers are written only after read-back verification. Rollback after a new
writer commits requires restoring the pre-migration backup or deploying a
compatibility reader; an older binary cannot interpret row-native-only writes.

## Capability process compatibility surface

`ProcessManager`, `ProcessHost`, and `ProcessRecord` remain the host-facing
capability-process API, but their lifecycle persistence delegates to the
canonical journal. `ProcessResultStore` owns bounded result/output records; raw
input is stored through the private process-input port and is not exposed by
status APIs.

The mutex-backed `ProcessInvocationStateStore` under `test_support` is
explicitly a pure fake. Production-store and durable-parity tests use
`ProcessJournalStore<InMemoryBackend>`, libSQL, or PostgreSQL.

## Validation

Changes must cover the smallest relevant contract tier, including:

- transaction and idempotency replay behavior;
- lease cancellation, checkpointed expiry, bounded reclaim, and exhaustion;
- child lineage, dependency settlement, and retry checkpoint ownership;
- paginated quota-blocked claims;
- restartable migration with malformed and interrupted inputs;
- libSQL/PostgreSQL parity for ordered keysets and durable restart.

Production code does not use `.unwrap()` or `.expect()`, and failures retain
their underlying cause while exposing only bounded, sanitized public evidence.
