# Reborn Contract — Turn Persistence and Active Locks

**Status:** Contract-freeze draft  
**Date:** 2026-05-05  
**Depends on:** [`turns-agent-loop.md`](turns-agent-loop.md), [`host-api.md`](host-api.md), [`events-projections.md`](events-projections.md), [`runtime-profiles.md`](runtime-profiles.md)

---

## 1. Purpose

Turn persistence owns durable control-plane state for host-layer turn coordination:

- accepted turn metadata and canonical binding references;
- executable turn-run lifecycle state;
- one-active-run-per-canonical-thread locks;
- runner lease/checkpoint metadata;
- durable turn-admission reservations for active accepted runs;
- idempotency outcomes for adapter-facing mutations;
- redacted lifecycle cursors needed for replay/recovery.

It does **not** own canonical transcript/message storage. Transcript and thread-message history remain in the transcript/thread storage boundary.

---

## 2. Logical records

The `ironclaw_turns` contract models persistence with these record families:

| Record | Ownership |
| --- | --- |
| `turns` | One accepted inbound message: scope, actor, accepted-message ref, source/reply binding refs, created timestamp. |
| `turn_runs` | Executable state for one run: current source/reply binding refs, status, resolved run-profile snapshot, latest checkpoint/gate refs, runner lease fields, event cursor. |
| `turn_active_locks` | One lock per canonical scoped thread while a run is active or resumable. |
| `turn_checkpoints` | Dedicated checkpoint/gate records written when a running run blocks. |
| `turn_admission_reservations` | Reservation evidence tying each accepted run to tenant/actor/project/agent total and class buckets until terminal release. |
| `turn_idempotency_keys` | Prior sanitized outcomes for scoped submit/resume/cancel idempotency keys. |

The initial PostgreSQL/libSQL adapter slice stores each logical record family in its own table with indexed metadata columns plus a serialized contract payload. Mutations hold a backend transaction/write lock across snapshot load, in-memory contract mutation, and snapshot replacement so active-lock and idempotency semantics remain atomic. Backends must preserve the same semantics as the in-memory contract tests while later slices add incremental row-level updates, targeted read paths, and service-graph wiring.

Legacy `turn_checkpoints` rows created before scoped checkpoint metadata may carry empty indexed `scope_key` values after migration. Those rows remain readable through the serialized payload; any future targeted `turn_checkpoints` read path must first add a scoped backfill plan or explicitly reject unbackfilled legacy rows instead of treating empty scope as a real owner.

The RootFilesystem-backed row store uses `/turns/rows/v1` as its durable shape.
Each logical family has its own keyed row collection under that root, with
`/turns/rows/v1/meta/state.json` carrying the last fully materialized
delta-journal sequence (`journal_seq`) and the event retention floor. Writers
update the hot in-process row cache, enqueue a `SnapshotDelta`, and return only
after the delta is durably appended. Row materialization is allowed to lag
behind the foreground ack: the background materializer coalesces journal tails
into row updates and advances `journal_seq`, while restart and durable read
paths replay any journal tail newer than `journal_seq` before trusting row
projections. Materialization writers are serialized within the runtime so an
older projector cannot overwrite rows after a newer projector has advanced the
projection. This keeps the journal as the crash-recovery source of truth while
avoiding full-snapshot rewrites on hot writes. Tier-2 run-record rows (`turns`,
`turn_runs`, and lifecycle events) remain durable even when terminal runs are
evicted from hot in-memory indexes; cache limits are eviction thresholds, not
deletion thresholds for the durable run record.

Legacy `/turns/state.json` blobs migrate into `/turns/rows/v1` through the same
delta journal. On first row-store load, if materialized rows and replayed
journal state are still empty, the store reads the legacy blob, appends one
full-snapshot `SnapshotDelta`, waits for the durable append ack, and then
materializes rows. The legacy blob is not deleted. Once any row data exists,
rows are authoritative and later stale blobs are ignored. Hosted rollout must
run this migration as the final stack step with no live turn writers, then
verify the row projection before enabling row-store-only production traffic.

---

## 3. Active-lock rules

- Active-lock key is the canonical `TurnScope`: tenant, agent, optional project, and thread.
- The key excludes `TurnActor.user_id`, channel IDs, source binding refs, and reply binding refs.
- A lock stores the current owning `TurnRunId`, explicit `TurnStatus`, monotonically increasing `TurnLockVersion`, `acquired_at`, and `updated_at`.
- Queued, running, cancel-requested, and blocked runs keep the lock.
- Current terminal transitions release their owned lock exactly once through `Inner::release_active_lock` in `crates/ironclaw_turns/src/filesystem_store/turn_state_engine/transitions.rs`.
- A persisted legacy `RecoveryRequired` run is terminal and does not keep effective active-lock ownership. `Inner::from_persistence_snapshot` in `crates/ironclaw_turns/src/filesystem_store/turn_state_engine/snapshot.rs` rehydrates the status; `TurnStatus::keeps_active_lock` and `Inner::thread_busy` then make any stale matching lock row non-blocking. New submissions may replace that stale row.
- Runner claim/resume/block/cancel-request transitions update the lock status/version while keeping ownership with the same run.

---

## 4. Idempotency rules

Adapter-facing mutations persist sanitized idempotency outcomes:

- `submit_turn` success records the accepted turn/run IDs and accepted response kind.
- `submit_turn` same-thread busy is transient: it does not create a turn/run, does not acquire admission, and is not cached as a submit idempotency replay.
- Capacity/policy admission rejections are replayable and do not create turn/run or reservation records.
- `resume_turn` and `cancel_run` record scoped run-operation outcomes.
- Idempotency records include a redacted replay envelope with response-critical fields such as status, event cursor, admission reason/capacity metadata, retry metadata, and cancellation `already_terminal` state.

A duplicate idempotency key must replay prior accepted submit and admission-rejection outcomes instead of re-running admission, lock acquisition, or state transitions. A duplicate same-thread busy submit with the same key may succeed later after the thread unlocks; legacy persisted `SubmitThreadBusy` replay rows are ignored on snapshot/DB load.

---

## 5. Turn-admission reservation rules

- Admission reservation is not a predicate: all configured tenant, actor-user, project, and agent total/class buckets must be checked and inserted atomically with turn/run creation.
- Each accepted V1 run records unlimited and limited canonical bucket reservations for telemetry and future limit changes.
- Submit admission policy checks that can reject unauthorized/profile-invalid requests run before returning same-thread busy metadata; same-thread busy is still checked before capacity reservation and never consumes admission slots.
- Capacity denial returns one deterministic safe `AdmissionRejected` payload with axis kind, total/class bucket, admission class when applicable, limit, active count, and optional retry hint. It must not expose foreign bucket IDs or raw provider internals.
- Missing limits mean unlimited. A non-AllowAll provider that is unavailable fails closed with `AdmissionRejectionReason::Unavailable` and creates no run/reservation.
- Queued, running, blocked, and cancel-requested runs keep reservations. Resume reuses the existing reservation.
- Terminal transitions (`Completed`, `Failed`, `Cancelled`, and future terminal states) release reservations exactly once. Released reservation evidence is retained only while the corresponding terminal run remains within the bounded terminal-record retention window; active capacity accounting must not scan unbounded released history.
- Limit changes do not evict existing runs; new admissions are denied until active reservations drop below the configured limit.
- Snapshot/DB loaders must synthesize unreleased reservation evidence for legacy non-terminal runs that predate persisted reservation rows so active capacity is not bypassed after migration/restart.

---

## 6. Runner lease and checkpoint rules

- Claiming a queued run atomically moves it to `Running`, stores runner ID/lease token, increments `claim_count`, records `last_heartbeat_at`, records `lease_expires_at`, and updates active-lock metadata.
- Heartbeats only renew metadata for matching, unexpired runner ID/lease token on actively `Running` work; heartbeat requests are rejected once the run is `CancelRequested`. Successful heartbeats refresh `last_heartbeat_at` and extend `lease_expires_at`.
- Physical adapters may split high-churn runner lease metadata from lower-churn turn snapshots/tables, as long as all read, recovery, and terminal transition APIs expose one logical run state. Liveness decisions must use durable lease metadata, not require one lifecycle event per heartbeat.
- An expired `CancelRequested` lease becomes terminal `Cancelled`, clears runner ownership, and releases its active lock and admission reservation.
- An expired `Running` lease with any loop checkpoint becomes terminal `Failed(lease_expired)`, with its latest resumable checkpoint attached when one exists; it clears runner ownership and releases its active lock and admission reservation.
- An expired checkpointless `Running` lease below `max_crash_recovery_reclaims` clears runner ownership and returns to `Queued`, retaining its active lock, admission reservation, and `claim_count`. At the reclaim bound it instead becomes terminal `Failed(crash_retry_exhausted)` and releases the lock and reservation.
- `RecoveryRequired` remains readable as a legacy terminal status, follows the non-blocking legacy-row rule in §3, and is not produced by current lease recovery.
- Blocking a running run requires a matching, unexpired lease, writes a checkpoint record, stores the latest checkpoint/gate refs on the run, clears current lease ownership, and keeps the active lock.
- Loop-driver resume payloads are staged in a host-owned `CheckpointStateStore` before a public checkpoint record is written. The store returns an opaque `LoopCheckpointStateRef`; callers cannot choose arbitrary refs for durable records.
- Checkpoint-state records are scoped by `TurnScope`, `TurnId`, and `TurnRunId`. Reads with a matching ref but foreign scope or run return no state, preserving tenant/thread/run isolation.
- Checkpoint-state payload bytes are bounded and debug-redacted. Public checkpoint/run/event/idempotency records may store only metadata and refs, never raw checkpoint payload bytes.
- Terminal runner outcomes require the matching, unexpired runner ID/lease token and release the active lock only if the run still owns it.

---

## 7. Redaction boundary

Turn persistence stores metadata and references only. It must not persist raw prompts, assistant content, tool input, secrets, host paths, or backend error details in turn/run/checkpoint/idempotency records.
