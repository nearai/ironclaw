# CAS-migration: drop redundant per-record mutexes onto the shared `cas_update` helper

Date: 2026-06-25
Branch: `fix/reborn-p1-cas-helper`
Owner: P1 runtime-wedge track (Phase 2)

## Background

IronClaw Reborn persistence stores keep durable state as a single versioned
snapshot per scoped key. Mutating it is a read-modify-write (RMW): load the
snapshot + its `RecordVersion`, compute the next snapshot, `put` it back with a
`CasExpectation::Version` precondition so a concurrent writer is detected as a
`VersionMismatch` instead of silently clobbered.

Historically each store wrapped that RMW in a *per-record*
`tokio::sync::Mutex` (`FILESYSTEM_RECORD_LOCKS`) **held across the `.await`** of
the backend `get`/`put`. That mutex is a redundant in-process serializer over
backends that already do versioned CAS. Under burst, one writer stalled inside
its critical section (slow backend op) parks every other writer for that scope —
the convoy that contributed to the 2026-06-24 runtime wedge.

PR #5142 removed exactly this from `ironclaw_turns` and proved the lock-free
pattern: an optimistic CAS-retry loop with bounded retries, jittered backoff,
and an overall timeout. Phase 1 (commit `bf75799cd`) extracted that pattern into
`ironclaw_filesystem::cas_update`. Phase 2 (this plan) routes the four remaining
stores through it and deletes their mutexes, and re-homes the `ironclaw_turns`
local CAS copy onto the shared helper so there is ONE implementation.

## Red→green evidence

`ironclaw_resources` is the canary: it has NO retry loop, so its per-record
mutex was the *only* serializer. A storm test (`SlowGetBackend` widens the race
window; N writers each `+1` via their own worker thread) demonstrates:

- **Current locked code:** GREEN — the per-record mutex serializes writers
  (correct, but convoyed: a stalled writer parks the rest).
- **Lock removed, no retry (the naive "just delete the convoy" fix):** RED —
  `cross-process CAS contention on snapshot /resources/counter.json`; a racing
  writer cannot make progress because the single-attempt CAS detects the race
  and errors instead of retrying. Proves the helper's CAS-retry is *required*.
- **Migrated to `cas_update`:** GREEN — bounded CAS-retry recovers every racing
  writer, every increment lands, no convoy, no per-record mutex.

Test: `crates/ironclaw_resources/src/cas_snapshot.rs` →
`tests::concurrent_increments_have_no_lost_updates`.

## Backend safety (fail-closed)

The shared helper FAILS CLOSED: a non-CAS backend yields
`CasUpdateError::CasUnsupported` instead of falling back to a blind
`CasExpectation::Any` overwrite. The four stores currently fail OPEN (fallback
to `Any` on `Unsupported(WriteFile)`).

Verified safe: in every production Reborn config the four store aliases
(`/run-state`, `/threads`, `/resources`, `/secrets`) resolve via the composite
router to a db-backed (libsql/postgres) or `InMemoryBackend` backend — all
advertise and implement `TxnCapability::Cas`. `LocalFilesystem` (the only
byte-only, CAS-incapable backend) is mounted ONLY at `/projects` and is
structurally unreachable from these aliases. The fail-open fallback is dead code
in production; removing it tightens the invariant and matches the `ironclaw_turns`
standard (which already fails closed).

## Per-store migration

### `ironclaw_resources` (`cas_snapshot.rs`) — highest risk, no prior retry
- Route `update_snapshot` through `cas_update`. DELETE `FILESYSTEM_RECORD_LOCKS`,
  `filesystem_record_lock`, `FilesystemRecordLock`, and the lock acquire in
  `update_with_scope`.
- The public `update`/`update_with_scope` closure is sync `FnOnce(&mut S)`. The
  helper re-runs `apply` per retry, so widen to `FnMut(&mut S)` and run it
  against a fresh snapshot each attempt inside an async `apply` adapter. (The
  existing closures are pure field mutations — re-runnable.)
- DELETE `put_with_cas` + `PutError` fail-open fallback; map `CasUpdateError`:
  `Apply→E`, `RetriesExhausted|Timeout→E::storage(contention/unavailable)`,
  `CasUnsupported→E::storage(backend-unsupported)`, `Backend→E::storage_from`.
- Add `PartialEq` to `BudgetGateSnapshot` (helper needs `S: Clone + PartialEq`).

### `ironclaw_run_state` (`lib.rs`) — has retry + lock (belt-and-suspenders)
- Replace both store-local CAS-retry loops (`apply_update`, `update_status`) with
  `cas_update`. DELETE the per-record lock acquires (start/block_*/complete/fail
  and save_pending/approve/deny/discard_pending) and the lock infra at ~1087.
- `update_status`'s `Pending` guard maps to returning `Err(ApprovalNotPending)`
  from the `apply` closure.
- DELETE the fail-open `put_with_cas`. Map `CasUpdateError`:
  `RetriesExhausted|Timeout→Backend`, `CasUnsupported→Backend(unsupported msg)`,
  `Backend→Filesystem`, `Apply→unwrap`. Keep the create-if-absent `start`/
  `save_pending` paths (cas_update handles `CasExpectation::Absent` natively).
- `RunRecord`/`ApprovalRecord` already derive `Clone + PartialEq`.

### `ironclaw_threads` (`filesystem_service.rs`) — single lock at `ensure_thread`
- `ensure_thread` is a check-then-create (CasExpectation::Absent). Route it
  through `cas_update` (which serializes create-if-absent + reconcile via the
  CAS precondition, replacing the lock). DELETE the lock acquire at ~1065 and
  the lock infra at ~2693. The three other loops are already lock-free; re-home
  them onto `cas_update` opportunistically only if low-risk.
- Add `PartialEq` to `StoredThreadRecord`.
- Map `CasUpdateError` into `SessionThreadError::Backend`.

### `ironclaw_secrets` (`filesystem_store.rs`) — Arc leak
- Route `consume`/`revoke`/`consume_session_use` through `cas_update`. DELETE the
  read-only lock in `validate_session` (pure read needs no serializer). DELETE
  `FILESYSTEM_RECORD_LOCKS` + `filesystem_secret_lock*` (the `Arc`-not-`Weak`
  map → unbounded leak; deletion removes the leak entirely).
- The local `cas_mutate`/`CasDecision` (Commit/Settle/BestEffortCommit) maps onto
  the helper: `Settle(Ok|Err)` = no-op (return unchanged snapshot) carrying the
  outcome/error; `Commit` = changed snapshot; `BestEffortCommit` = changed
  snapshot. DELETE `put_with_version_fallback` fail-open path.
- Add `PartialEq` to `StoredLease`, `StoredSession` (the two mutated records).
- Map `CasUpdateError` into `SecretStoreError`/`CredentialBrokerError`
  (`*Unavailable` for Timeout/RetriesExhausted/CasUnsupported, backend for Backend).

### `ironclaw_turns` (`filesystem_store.rs`) — re-home the local copy
- Replace the local `apply_with_retry` loop, `put_with_cas`, `PutError`, and
  `cas_retry_backoff` with `cas_update`. Keep the 500ms `SNAPSHOT_READ_CACHE_TTL`
  read cache as a separate layer wrapping the read-only path — it does NOT thread
  through the helper. On a successful write the helper does not surface the new
  `RecordVersion`, so clear the snapshot cache on write and let the next read
  repopulate from the backend (cache is a read optimization only).
- `TurnPersistenceSnapshot` already derives `Clone + PartialEq`.

## Post-condition

```
grep -rn "FILESYSTEM_RECORD_LOCKS\|record_lock.lock().await\|filesystem_secret_lock" \
  crates/ironclaw_run_state crates/ironclaw_threads crates/ironclaw_resources \
  crates/ironclaw_secrets crates/ironclaw_turns
```
must be EMPTY.

## Guardrail

Add to the filesystem/turns spec + `.claude/rules`: filesystem read-modify-write
MUST go through `ironclaw_filesystem::cas_update`; never wrap it in a per-record
`tokio::sync::Mutex` held across `.await` (redundant serializer + convoy/wedge
risk).

## Quality gate

Per touched crate:

- `cargo fmt --all -- --check`
- `cargo check` (default / postgres build)
- `cargo check --no-default-features --features libsql`
- `cargo check --all-features`
- `cargo clippy --all --benches --tests --examples --all-features -- -D warnings`
- `cargo test` (+ `--features integration` where stores have it)

## No new abstractions

Only the existing `cas_update` helper. No new traits, no new wrappers. Commit
per-crate as each migrates + gates green.
