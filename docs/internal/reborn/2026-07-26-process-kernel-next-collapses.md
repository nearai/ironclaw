# Process kernel next collapses

This inventory is grounded in the production tree at
`fcbdfc7bb1811ab9e7ea19eb41cc2f3c1c2ced3b`. The objective is net deletion:
one process lifecycle, one journal, and domain projections over it.

The stress rerun establishes a prerequisite: first partition journal writes.
Adding the state below to the current growing `state.json` would increase CAS
conflicts and serialization cost.

## Slice 0: row-native process persistence

Implemented on `process-journal-kernel-transition` as transactional,
row-materialized process persistence:

- every lifecycle entry is one immutable cursor-keyed row under
  `/processes/materialized/journal`;
- process snapshots, inputs, tree reservations, dependencies, checkpoints, and
  idempotency records are independently addressable rows under
  `/processes/materialized`;
- libSQL, PostgreSQL, and the in-memory reference backend expose a multi-key
  transaction that commits changed rows and lifecycle appends together;
- optimistic row versions resolve concurrent commands without a process-local
  authority;
- the old command log and `state.json` are accepted only by an explicit
  pre-start migration operation.

The prior append-command design still required an unbounded in-memory replay
projection and made a restart proportional to total history. The transactional
design keeps the event journal as history and the database rows as the
queryable projection without creating two independently writable authorities.

Acceptance:

- zero store failures in both rerun matrices through c100;
- cross-handle and cross-process row-ordering tests preserve every transition;
- no unconditional-write or non-event fallback;
- terminal history does not make one unrelated process transition O(total
  historical processes);
- restart projections reproduce live locks, gates, leases, and dependencies.

The first two gates are complete. The 2026-07-26 rerun through c100 has no
process-journal unavailable/storage failures; remaining failures are expected
exclusive-thread admission. Durable row projections now remove replay and
unbounded in-memory history from steady-state operation.

The scan-free storage pass makes normal initialization an exact metadata probe;
legacy replay and row projection rebuilds are explicit offline methods.
Write-maintained ordered projections never backfill during declaration and bind
their full leading partition on requests. A 2026-07-27 libSQL run prefilled
1,000,000 threads across 1,000 owners with zero failures at 4,226 inserts/sec.
The query plan used `(index_name, scope_key)` rather than a root-table walk.
At 1,000 simultaneous complete owner walks, peak RSS was 2.47 GiB and p95 was
28.17s, identifying concurrent response materialization as the remaining list
pressure rather than a database scan. Sparse process lifecycle projections on
a fresh database completed 5,587 operations with zero failures at 242.5
ops/sec, p95 4.19s, and about 36 KiB of database growth per lifecycle
operation.

PostgreSQL projection declaration is serialized across processes with a
transaction-scoped advisory lock and cached after successful declaration in
each backend instance. A 100-worker, 1,000-user lifecycle run exposed and then
regressed the prior concurrent catalog-update race: after the fix, 3,631 of
3,631 operations succeeded at 234.8 ops/sec with p95 570 ms and 132 MiB peak
runtime RSS. A mechanical architecture gate confines generic collection
enumeration and legacy log tailing in process/thread production code to the
named explicit offline migration methods.

## Slice 1: retire the second process lifecycle

Status on `process-journal-kernel-transition`: production lifecycle persistence
is collapsed. Capability/background records are submitted as
`ProcessKind::CapabilityInvocation`, and `ProcessServices` uses a journal-backed
capability projection. `process_store.rs`, `compatibility.rs`,
`ProcessStorePort`, lifecycle decorators, and their parallel state-machine
tests are deleted. Externalized result bodies remain in the dedicated result
store.

Terminal capability-obligation cleanup is now also a process-journal commit
observer. The observer is registered once against the final runtime and follows
governor replacement without replacing the lifecycle component. This removes
the semantic blocker that previously required host kill and supervisor
completion to pass through a store wrapper; pre-submit handoff
claiming remains the only lifecycle action that must happen before a journal
commit.

`ProcessHost` and the capability background executor now read and terminalize
processes through `ProcessRuntimePort`.

Detached capability authorization re-minting also reads the authoritative
journal snapshot directly. Authorization validation, host control, and
supervisor completion therefore share one persisted process projection.

Capability submission now uses a narrow lifecycle hook for pre-commit
obligation handoff claiming and post-submit notification. The obligation
lifecycle component no longer implements or contains `ProcessStorePort`, and
`ProcessServices` plus `DefaultHostRuntime` now retain `ProcessRuntimePort`
directly.

`ProcessRecord` remains only as the capability-facing view returned by spawn,
status, await, and subscription APIs. Submission and projection helpers live in
`capability_process.rs`; they do not define a second lifecycle or storage port.

## Slice 2: dissolve `ironclaw_run_state`

Status on `process-journal-kernel-transition`: invocation lifecycle is now a
native process-journal projection. The host runtime maps `InvocationId`
directly to `ProcessId`, records authorization and approval waits as process
suspensions, and resumes/claims the same process before terminal transition.
The filesystem-backed `RunStateStore` and `/run-state` record path are deleted.

The compatibility lifecycle DTOs/ports, lifecycle fake, combined
run-state/approval port, and host-runtime combined-store wiring are deleted.
`CapabilityHost` and `DefaultHostRuntime` consume
`ProcessInvocationStatePort` directly. Approval and gate persistence moved into
`ironclaw_approvals`, and the `ironclaw_run_state` crate was deleted.

`ironclaw_run_state/src/lib.rs` is 1,019 lines of invocation lifecycle:
`start`, approval/auth blocking, `complete`, `fail`, scoped lookup, and listing.
Those states overlap process submission, suspension, gates, and terminal
transitions.

Represent each host invocation as a process whose `InvocationId` is indexed
metadata. Approval and authorization blocking become process suspension with a
gate reference. Capability host helpers query the process projection.

Keep approval decision authority in the approval subsystem. The process journal
should record that a process is waiting on an approval and the durable decision
reference; it should not become the policy or approval authority.

Do this with Slice 1 so capability execution does not migrate through a third
temporary lifecycle.

## Slice 3: make child dependencies process edges

Status on `process-journal-kernel-transition`: implemented. Generic dependency
records, transitions, scoped queries, and host-wide unresolved queries are
owned by `ironclaw_processes`. Child submission atomically creates the child
process, reserves tree capacity, and opens its dependency in one journal row;
consume/abandon atomically closes the dependency and releases that reservation.

The runner await-edge store is now a projection adapter over
`ProcessDependencyPort`. The 1,457-line filesystem store, 561-line roster, and
most of the 720-line boot-recovery driver were deleted. Spawn no longer writes
an await edge before child submission, and terminal handling no longer
reconstructs missing process truth from turn/thread metadata. Agent-specific
result framing, group readiness, and parent resume remain in the runner.

The slice currently contributes roughly 3k net deleted lines across production
and tests while adding the generic process contract and atomicity/stress tests.

Historical inventory:

The subagent await-edge implementation duplicates generic process dependency
state:

- `await_edge/store.rs`: 1,457 lines
- `await_edge/boot_recovery.rs`: 720 lines
- `await_edge/resolver.rs`: 1,879 lines
- `await_edge/roster.rs`: 561 lines

Move parent-child wait relationships into a generic process dependency record:
open, settled, consumed/closed, terminal evidence, and reservation-release
state. Make edge mutation atomic with the corresponding process-tree capacity
change where required.

The process journal can enumerate unresolved dependencies directly, eliminating
the roster marker and most boot-recovery machinery. Agent-specific aggregation
of child results remains a runner projection.

This has the highest deletion potential, but follows Slice 0 because it requires
indexed edge queries and atomic edge/tree mutations.

## Slice 4: unify checkpoint metadata and payload

Status on `process-journal-kernel-transition`: implemented. A process
checkpoint command now carries a bounded, debug-redacted opaque payload beside
its ref and schema metadata, and commits one keyed checkpoint row in the same
transaction as the process mutation. Agent-loop checkpoint records read that
row directly.

The host stages bytes only in memory until `checkpoint`, which commits payload
and metadata atomically. Resume and loop-exit evidence read the payload from
the process projection. The separate `/checkpoint-state` mount,
`CheckpointStateStorePort`, filesystem implementation, contract suite, and
composition wiring are deleted. Stable checkpoint scope binds the process
invocation axis to `TurnRunId`, so put/get cannot mint mismatched scopes.

Historical inventory:

Checkpoint state was split between generic process checkpoint
metadata and a separate loop-host payload store:

- `loop_host/checkpoint_state_store.rs`: 447 lines
- `turns/checkpoint_state.rs`: 242 lines
- `turns/process_projection/loop_checkpoint.rs`: 128 lines

Extend the generic process checkpoint contract with a bounded opaque payload or
a host-owned artifact reference. Keep schema interpretation in the agent loop.
Then remove the separate checkpoint-state filesystem record and projection
bridge.

Do not embed unbounded or secret-bearing payloads directly in journal entries.

## Slice 5: make subagent goals immutable process input

Status on `process-journal-kernel-transition`: implemented. Process submission
now accepts a bounded, debug-redacted immutable input payload and exposes only
its opaque schema ref in process snapshots and lifecycle events. The payload is
committed as a keyed input row in the same transaction as child identity, tree
reservation, dependency creation, and lifecycle history, then read through the
scope-bound `ProcessInputPort`.

Subagent spawning serializes the agent-owned `SubagentGoalRecord` as
`subagent-goal:v1` process input. Prompt material projects it from the process
journal, with the persisted child-thread message retained as legacy fallback.
The 527-line goal store, its filesystem records, write/delete compensation,
runtime trait union, composition field, readiness component, and test doubles
are deleted.

The payload is stored directly in the private command row rather than through a
new artifact subsystem. That keeps submission atomic and avoids replacing one
small bespoke store with a larger generic one. The bounded payload is absent
from public process snapshots and event projections.

## Slice 6: generalize scheduler wake and cancellation

Status on `process-journal-kernel-transition`: generic claim-loop wakeup,
bounded concurrency, lease heartbeat/recovery, executor panic containment,
terminal-failure recording, and shutdown lease relinquishment now live in
`ironclaw_processes::ProcessSupervisor`. The former 1,129-line turn scheduler is
a small `ProcessKind::AgentTurn` adapter over that supervisor; its separate
executor-task and latency modules are deleted.

The runner registers the executor for `ProcessKind::AgentTurn`; extension and
host runtimes register their own executors. Scheduling policy, model turns, and
agent-loop behavior stay outside the kernel.

`ProcessServices` no longer spawns a detached lifecycle task. Its
`BackgroundProcessManager` journals bounded durable input, wakes a
`ProcessKind::CapabilityInvocation` supervisor, and registers cancellation when
the process is submitted. The same supervisor now owns claiming, bounded
concurrency, heartbeats, recovery, panic containment, and shutdown for turns and
capability work.

There is no longer a second `JournalProcessStore` object, `ProcessStore` alias,
or `ProcessStorePort`. `ProcessServices` holds the authoritative
`ProcessRuntimePort` directly.

`ProcessServices` is also no longer generic over lifecycle/result-store
implementations. It erases those behind its owned ports while retaining
concrete type identity for production-readiness validation. As a result,
`HostRuntimeServices` carries only its filesystem and resource-governor type
parameters instead of propagating process store types through composition.

Result payload storage and live cancellation are now internal parts of that
single process system. `BackgroundProcessManager` requires `ProcessServices`
instead of accepting a journal and optional result/cancellation collaborators,
and `DefaultHostRuntime` retains one optional `ProcessServices` field instead
of decomposing it into three independently wired ports. Large result bodies
remain externalized from journal rows, and cancellation tokens remain
process-local execution coordination; neither concern leaks through
host-runtime construction anymore. `ProcessHost` is likewise created only by
`ProcessServices`; its raw runtime/result/cancellation constructors and
optional-store failure state are deleted.

## Recommended order

1. Partition journal persistence and rerun the four stress artifacts.
2. Merge `ProcessStorePort` and `ironclaw_run_state` into the journal.
3. Move process results/evidence and cancellation registration behind the
   unified runtime.
4. Replace await-edge/roster/recovery with process dependencies.
5. Fold checkpoint payload and immutable process input into generic references.
6. Generalize scheduler/supervisor wiring and leave agent turns as an executor
   projection.

The surveyed files total roughly 10.7k lines. Not all are deletable, but Slices
1-3 should produce substantial net deletion because they remove complete stores,
state machines, wrappers, and recovery paths rather than merely relocating
types.
