# Reborn durable event-log append batching (write-behind coalescing)

**Branch:** `fix/reborn-event-log-batch` · **Date:** 2026-06-25 · **Scope:** events only (E1 from the write-classification audit). Audit/CAS/ownership writes are explicitly out of scope.

## Verified problem

The durable runtime event log issues **one single-row INSERT per emitted event**, on the hot path, append-only:

- `EventSink::emit` (`crates/ironclaw_events/src/sink.rs:166`) does `self.log.append(event).await.map(|_| ())` — the returned `EventLogEntry { cursor, .. }` is **discarded**. Nothing downstream gates a side effect on the assigned seq.
- `DurableEventSink` (runtime events) is a *separate* sink from `DurableAuditSink` (compliance audit). Audit must stay synchronous; runtime events need not.
- Production path: `DurableEventSink` → `FilesystemDurableEventLog::append` (`filesystem_store.rs:86`) → `ScopedFilesystem::append` (`scoped.rs:175`) → `RootFilesystem::append` single-row INSERT:
  - Postgres `postgres.rs:608`: `INSERT INTO root_filesystem_events (path, payload) VALUES ($1,$2) RETURNING id` (id = BIGSERIAL = SeqNo).
  - libSQL `libsql.rs:896`: `INSERT ... VALUES (?1,?2)` then `last_insert_rowid()` (seq = INTEGER PK AUTOINCREMENT).
  - in-memory `in_memory.rs:363`: per-path `Vec` push.
- Per-turn cost (write-classification audit, `docs/plans/2026-06-25-write-caching-batching-classification.md`, row E1): **~5K+3I+1** single-row INSERTs ≈ **21 at K=5/I=1, ≈57 at K=10/I=2**. On cross-region Postgres (~100–200 ms/round-trip) this is a top latency contributor.

## Decision (already made — not relitigated)

Keep the event log **durable** (it is the read-model substrate: projections fold it; durable cursors persisted elsewhere outlive it, so resuming from a non-zero cursor against an empty head = ReplayGap/split-brain). Fix = **write-behind + batch**: buffer appends in memory, flush as **one multi-row INSERT per drain window**, preserving order (SeqNo), bounding crash-loss to the unflushed sub-second tail.

## Why the buffer lives at the runtime EventSink seam (not at `RootFilesystem::append`)

`RootFilesystem::append` is shared by the **audit** log (must-stay-sync) and returns a **synchronous cross-process SeqNo** that cannot be safely pre-assigned in-process. `EventSink::emit -> Result<(), EventError>` carries **no cursor**, and the runtime sink is distinct from the audit sink. So buffering at the runtime `EventSink` is audit-safe and contract-clean. The **multi-row INSERT primitive** is added at `RootFilesystem` so all three backends share one implementation.

## Changes

### 1. Multi-row append primitive (`ironclaw_filesystem`) — append fns only

- `RootFilesystem::append_batch(&self, path, payloads: Vec<Vec<u8>>) -> Result<Vec<SeqNo>, _>` (root.rs): **default impl loops `self.append`** (correctness-preserving; backends without an override degrade safely to per-row). All payloads target the **same** path; returns SeqNos in payload order.
- `ScopedFilesystem::append_batch` (scoped.rs): resolve mount/permission **once**, delegate to `root.append_batch`.
- Postgres override (postgres.rs): single cache-friendly fixed-SQL statement —
  `INSERT INTO root_filesystem_events (path, payload) SELECT $1, payload FROM unnest($2::bytea[]) AS t(payload) RETURNING id`, params `[&path, &payloads]`; sort returned ids ASC → payload order (ids monotonic in insert order).
- libSQL override (libsql.rs): one multi-row `INSERT ... VALUES (?,?),(?,?),... RETURNING seq`; sort seqs ASC → payload order. Empty input → `Ok(vec![])`.
- in-memory override (in_memory.rs): lock once, push all, collect SeqNos.

### 2. Event-log batch append (`ironclaw_events` + `ironclaw_reborn_event_store`)

- `DurableEventLog::append_batch(&self, Vec<RuntimeEvent>) -> Vec<Result<EventLogEntry<RuntimeEvent>, _>>` (sink.rs trait): **default loops `append`**.
- `FilesystemDurableEventLog::append_batch` override (filesystem_store.rs): group events by `stream_path` (one path per `(tenant,user,agent)`), serialize, call `fs.append_batch(scope, path, payloads)` **once per path** = one multi-row INSERT per stream.

### 3. Coalescing write-behind sink (`ironclaw_reborn_event_store`)

New `CoalescingEventSink` (`coalescing_sink.rs`) implementing `EventSink`, wrapping `Arc<dyn DurableEventLog>`:

- `emit(event)`: `try_send` onto a bounded `mpsc::Sender` (capacity 8192) and return `Ok(())` immediately (best-effort sink contract — never blocks or short-circuits the caller). On a full channel (drain stalled) the event is dropped and a `dropped_count` counter is incremented (rate-limited `debug!`, not per-event `warn!`); a closed receiver returns the sink-closed diagnostic.
- A single long-lived drain task: on first event, start a `flush_interval` deadline; accumulate via `timeout_at(deadline, rx.recv())` until the deadline elapses **or** `max_batch` is reached, then `log.append_batch(batch)` (one INSERT per stream). Flushes are awaited sequentially in the single task → **global order preserved**. On all-senders-dropped, drain remaining and exit (deterministic shutdown).
- `flush()` control message (oneshot ack) for deterministic test/shutdown flushing.
- `EventBatchConfig { max_batch, flush_interval }` with conservative defaults (`max_batch: 256`, `flush_interval: 50ms`).

### 4. Wiring (`ironclaw_host_runtime/services/builder.rs`)

`with_reborn_event_stores_verified` (line 603): swap `DurableEventSink::new(stores.events)` → `CoalescingEventSink` (production seam only). Audit sink (line 604) unchanged. `with_durable_event_log` (534, test/generic) left synchronous. Rollback = revert this one line.

## Durability / ordering / consistency semantics

- **Ordering:** within a stream, events flush in emit order; multi-row INSERT assigns contiguous SeqNos in row order (`RETURNING id`/`seq` sorted ASC). The single drain task serializes flushes.
- **Crash before flush:** everything not yet flushed is lost — this includes the entire in-memory backlog sitting in the sink's unbounded channel, not merely the active drain window (≤ `flush_interval` ≈ 50 ms, or ≤ `max_batch`). Everything flushed is durable. No torn batch (one INSERT is atomic). The synchronous `DurableEventLog::append` path (direct callers such as `loop_driver_host`, tests) is not lossy.
- **Read-your-writes:** reads (`read_after_cursor`/`head_cursor`) go to the durable backend directly; buffered-but-unflushed events appear after the next drain (sub-second). No hot-path consumer gates on synchronous read-after-emit (cursor is discarded at the sink; the direct `DurableEventLog::append` cursor-users — `loop_driver_host`, tests — bypass the sink and are unaffected). A subscription's startup head snapshot taken mid-buffer classifies the later-flushed events as *live* (cursor > startup_head) — correct, no split-brain, because seqs are assigned by the DB only at flush.
- **Audit (E2):** untouched, stays synchronous.

## Red→green proof

- **Count reduction:** a counting `RootFilesystem` test double counts `append` vs `append_batch`; N same-stream events coalesced into a single flush window → **1 `append_batch` carrying N payloads, 0 single `append`** calls for that stream/window; `tail` returns all N in emit order with correct seqs (one statement + ordering + content). Bursts spanning multiple flush windows or multiple stream paths produce multiple `append_batch` calls (one per distinct stream path, per drain window).
- **Per-backend primitive:** `append_batch` contract tests for in-memory (always runs), libSQL (`--features libsql`, local file), Postgres (`--features postgres`, skip when `IRONCLAW_FILESYSTEM_POSTGRES_URL`/`DATABASE_URL` unset): contiguous ordered SeqNos, `tail` round-trip equals single-append semantics, empty-input → `[]`.
- **Crash-tail bound:** flush a batch, emit more, drop without flush → flushed batch persists, only the unflushed tail is absent.

## Tri-backend

`append_batch` exists for postgres/libsql/in-memory; the trait default loop guarantees any non-overriding `RootFilesystem`/`DurableEventLog` still works (degrades to per-row, never breaks). Gate under `--features postgres,libsql` + default.
