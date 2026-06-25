# Native Hot-Store Decomposition on the Unified `RootFilesystem` Trait — Fusion Design

**Status:** Proposed (implementation-grade). Supersedes the three angle drafts (data-model, new-primitives, per-store). Incorporates the required changes from all three structured reviews.

**Owners:** turns / resources / threads / filesystem crate owners.

---

## 1. Problem

All four hot stores funnel every mutation through a **whole-blob read-modify-write CAS on a single `Entry`**, which serializes writers that touch logically-disjoint state:

| Store | Today (GROUNDING 3) | Contention shape |
|---|---|---|
| **Resource governor** | One **process-global** `/resources/snapshot.json` under `ResourceScope::system()` holding `limits`/`reserved_by_account`/`usage_by_account`/`reservations`/`period_anchors`. Each model call CAS-writes it ~2× (reserve + reconcile). `VersionMismatch` is a **hard error with no retry** (`cas_snapshot.rs:204` "cross-process CAS contention"). | **Global across unrelated tenants.** Two agents in different tenants serialize on one row and the loser fails. This is the worst contention point and the top priority. |
| **Turn-state** | One `/turns/state.json` blob per *scope-mux* holding all runs + embedded events. Every transition does full read-modify-write CAS, `FILESYSTEM_CAS_RETRIES = 32`. | Every run in the mux serializes on one row. The 32-retry constant is the smoking gun. |
| **Event-log** | Not a separate store — an `events: Vec<TurnLifecycleEvent>` **field inside** the turn snapshot. | Every event append rewrites the entire snapshot; events can never be queried without a full-blob scan. |
| **Threads** | Already per-file (`thread.json`, `messages/{id}.json`, `sequences/{seq}.json`, `idempotency/{sha}.json`) with a txn-or-CAS multi-put. | No whole-blob CAS. The remaining work is queryability and an operational escape hatch, not decomposition. `list_threads_for_scope` is an N+1 `list_dir` + per-file `get`. |

The unified `RootFilesystem` trait already exposes everything needed to split most of these blobs into fine-grained per-entity records with **CAS-by-version on each small record**. Two distinct writers then touch different rows and never collide. Because both SQL backends store identical `(path, body, kind, indexed, version)` tuples, the decomposed records are logically identical across libSQL and Postgres, so a backend migration stays a **copy, not a transform**.

The one place where decomposition alone is *not* correctness-equivalent is the resource governor's **multi-account admission**, which is an all-or-nothing check across 1–6 cascade accounts. That requires either a true multi-key transaction (Postgres) or an explicit atomic admission gate on the CAS-only floor (libSQL). This doc resolves that honestly rather than hand-waving a "fail-soft reconcile."

---

## 2. Goals / Non-goals

### Goals

1. Keep `RootFilesystem` the **one contract**. No caller forks, no per-backend `cfg` in callers.
2. Fix the governor global-contention bug **without weakening the all-or-nothing limit invariant** on either backend.
3. Decompose turn-state, event-log, and (lightly) threads onto per-entity records / the native append plane.
4. Add the **minimal** set of new native primitives — only those a store genuinely needs and that have a correct default fallback on every backend.
5. Prove dual-backend parity with a CI test that asserts both backends produce identical logical records for every op.
6. Ship each store **independently**, feature-flagged, reversible, green on **both** backends at every step.

### Non-goals

- No new "real-backend vs test-backend" trait. One trait, one entry, one mount (GROUNDING 4 §5).
- No removal of the legacy snapshot rows (LLM-data-never-deleted invariant). "Migrated" means *unreferenced*, never *deleted*.
- No re-decomposition of threads (already per-file). Threads gets index/queryability + an escape hatch only (Rule 3).
- No general expression engine inside the filesystem. Backends never parse `body` for logic (GROUNDING 1 #4).

---

## 3. Constraints & Invariants (carried from grounding, non-negotiable)

1. **CAS is the floor.** Every backend supports `put` + `CasExpectation::Version`. Multi-key txns (`begin`/`StorageTxn`) are strictly optional; libSQL is CAS-tier, Postgres is `MultiKey`.
2. **Versions are opaque, monotonic-per-path `u64`**, compared only for equality. `version = version + 1` in SQL on both dialects. A backend copy need not preserve version *numbers* — only logical state.
3. **Backends never look inside `body`.** Everything queryable or mutated by a native primitive lives in `Entry.indexed` (`BTreeMap<IndexKey, IndexValue>`) or in the version counter.
4. **Capabilities are honest or you don't mount.** `validate_mount_capabilities` (`catalog.rs`) refuses a mount whose descriptor claims a capability the backend lacks.
5. **Callers do not change.** A consumer writes against the trait; native fast path vs default fallback is invisible except in latency/contention.
6. **Consumers reach the filesystem through `ScopedFilesystem`**, not `RootFilesystem` directly. `ScopedFilesystem::resolve((scope, ScopedPath)) -> VirtualPath` applies per-operation permission checks (`scoped.rs:100-222`) before delegating. Any new primitive must have a `ScopedFilesystem` wrapper and a `operation_allowed` arm.
7. **MIGRATION-SAFETY INVARIANT (stated once, the spine of this doc):**
   > For every store operation, given identical inputs, both backends MUST end with **identical logical records** — the same set of `(VirtualPath, kind, body, indexed)` tuples and the same append-plane payloads in the same seq order — so that a libSQL↔Postgres switch is a verbatim row copy. Version *numbers* may differ; logical state may not.

   This is enforced by a CI parity test (§9.4) that drives every op against both backends and asserts the record sets are equal.

---

## 4. Trait-preservation mechanism

The trait stays frozen as the single contract. Decomposition is a **pure consumer-side key/record-layout change**: stores write many small records instead of one blob, each CAS'd by its own `RecordVersion`. For the three places where a single round-trip or true atomicity materially changes contention, we add **optional native primitives with correct default impls built from existing ops**, so byte-only backends (`local`, in-memory) keep working with zero changes.

Three mechanisms, in priority order:

**(a) Decomposition with zero new ops (covers turn-state, event-log, threads, and the *single-account* governor counter math).** `get`/`put`+CAS, `query`/`ensure_index`, `append`/`tail`/`tail_bounded`/`head_seq` already cover every access pattern. The whole-blob CAS loop becomes a per-record CAS loop. No trait change, no caller fork.

**(b) Two new optional native primitives where decomposition needs atomic multi-record commits or contention-free counter math** — `put_batch` and `adjust_indexed` (§6). Each:
- is a `RootFilesystem` method with a **default impl** built from existing ops (so every backend compiles and behaves identically);
- has a **native override** on Postgres and libSQL that collapses round-trips / takes a row or write lock;
- is gated behind a new `Capability` bit enforced at mount;
- is mirrored on `ScopedFilesystem` with a permission-checked wrapper.

**(c) The capability bit means "stronger guarantee," not "callable."** This matches the existing `begin`/`MultiKey` contract: a primitive's *default* impl works on every backend, so a consumer can always call it. The bit advertises whether the backend provides the **atomic / single-round-trip** form. A descriptor declares the bit only when the consumer *requires* that guarantee. **Honest caveat (review B):** unlike `begin` (which returns a typed `Unsupported` the caller branches on), `put_batch`/`adjust_indexed` *succeed* via the default path — there is no return-value distinction between native and default, only latency and, for `put_batch` on a CAS-only backend with N>1, a typed `Unsupported`. Consumers that **require** atomicity MUST gate on the capability bit (or on `begin` returning a usable txn) and take their explicit fallback; they must not assume the default path is atomic. Each consumer states which guarantee it requires (§7, §8).

**Why not expose `begin` directly to every caller?** The thread store already hand-rolls a `begin`/`StorageTxn` 4-op txn with a CAS-loop fallback and a `TransactionalMessageWrite::Unsupported` enum. `put_batch` lets a store express *intent* (these N records commit together) in one call; its default impl **is** that txn loop, so `MultiKey` backends behave identically and CAS-only backends still take their fallback — but only for the genuine N>1-no-txn case.

**No `merge_indexed`.** The third angle proposed a `merge_indexed` "patch indexed without rewriting body" primitive for turn-state/`next_sequence`. We **drop it**: every hot field it targets (`status`, `updated_at`, `next_sequence`) is the *only* mutating field on records whose `body` is small, so a plain `get`→`put(Version)` (the default that `merge_indexed` would fall back to anyway) is sufficient and adds no new trait surface. Adding `merge_indexed` would buy a marginal round-trip saving at the cost of the PG-`#-`-vs-SQLite-`json_patch`-null parity hazard flagged by review B. **Simplicity First (Rule 2):** two primitives, not three.

---

## 5. The parity argument, stated once (applies to every record below)

Every record is an `Entry { body: <JSON bytes>, content_type: JSON, kind: Some(<RecordKind>), indexed: <BTreeMap> }` written with `put` under a `CasExpectation`, read with `get`/`query`, and (for events) appended with `append`/`tail`.

- Both SQL backends persist a record as the same logical row `(path, contents, content_type, kind, indexed JSON, version)`. `body` is opaque bytes; `indexed` is JSON; `version` is bumped `version = version + 1` in the same statement on both dialects (GROUNDING 2).
- CAS is identical: `Absent` → `INSERT … ON CONFLICT DO NOTHING`; `Version(v)` → `UPDATE … WHERE version = v`; `Any` → upsert. `VersionMismatch` surfaces identically.
- `query` filters touch **only** `indexed` (`indexed->>'k'` on PG, `json_extract(indexed,'$.k')` on libSQL), never the body, so identical `Filter`s over identical `indexed` projections produce identical result sets.
- The append plane assigns a monotonic `SeqNo` per path (`BIGSERIAL`+`RETURNING` on PG; `AUTOINCREMENT`+`last_insert_rowid()` on libSQL).

Therefore: **the same Rust code calling the same trait against both backends produces identical logical state.** This is exactly the migration-safety invariant (§3.7), and decomposition does nothing to weaken it — it multiplies one row into N rows under the same mount.

---

## 6. New native primitives

Only **two** new primitives. Each section gives: signature, default impl, native Postgres impl, native libSQL impl, semantic-parity argument, capability advertisement, and the `ScopedFilesystem` wrapper.

### 6.0 Shared capability + operation + scoped-wrapper surface

**`Capability` (types.rs)** — append two bits (discriminant order only grows; `bit = 1 << (self as u32)`; unknown capability strings already decode as "missing"):

```rust
pub enum Capability {
    // … existing variants, unchanged order …
    Events,
    BatchPut,       // native atomic put_batch
    AdjustIndexed,  // native atomic conditional numeric delta on an indexed key
}
```

Add both to `Capability::all()` in trailing order.

**`FilesystemOperation` (types.rs)** — append for honest error attribution:

```rust
pub enum FilesystemOperation { /* … */ HeadSeq, PutBatch, AdjustIndexed }
```
…with `Display` arms `"put_batch"`, `"adjust_indexed"`.

**Convenience constructor (types.rs):**

```rust
/// `sql_typical_full()` plus the two native hot-path primitives.
pub const fn sql_typical_hotpath() -> Self {
    Self::sql_typical_full()
        .with(Capability::BatchPut)
        .with(Capability::AdjustIndexed)
}
```
`in_memory_full()` is widened to `sql_typical_hotpath()` (the in-memory backend serves both via default impls under its per-op lock — see parity notes).

**Mount validator (catalog.rs)** — add both bits to `NEW_AXES`:

```rust
const NEW_AXES: &[Capability] = &[
    Capability::Records, Capability::Query,
    Capability::IndexExact, Capability::IndexPrefix,
    Capability::IndexFts, Capability::IndexVector,
    Capability::Events,
    Capability::BatchPut, Capability::AdjustIndexed,
];
```
That is the entire validator change; the existing `declared.has(cap) && !backend.has(cap) → DescriptorOverclaims` loop handles the new bits by construction.

**`ScopedFilesystem` wrappers (scoped.rs) — REQUIRED (review B blocker).** Consumers cannot reach `RootFilesystem` methods directly. Add scope-relative wrappers that resolve+permission-check before delegating, and extend the **exhaustive** `operation_allowed` match (`scoped.rs:417`, no catch-all — omitting these will fail to compile):

```rust
// scoped.rs operation_allowed: add arms
FilesystemOperation::PutBatch      => self.permissions.allows_write(),
FilesystemOperation::AdjustIndexed => self.permissions.allows_write(),

// scoped.rs wrappers
impl<F: RootFilesystem> ScopedFilesystem<F> {
    pub async fn put_batch(&self, scope: &ResourceScope, puts: Vec<ScopedBatchPut>)
        -> Result<Vec<RecordVersion>, FilesystemError> {
        // resolve+permission-check EACH entry's ScopedPath -> VirtualPath,
        // assert all resolve to the SAME mount, then delegate to self.root.put_batch
    }
    pub async fn adjust_indexed(&self, scope: &ResourceScope, path: &ScopedPath,
        key: &IndexKey, delta: i64, guard: AdjustGuard, init_absent: Option<i64>)
        -> Result<AdjustOutcome, FilesystemError> {
        // resolve+permission-check, delegate
    }
}
```
`ScopedBatchPut.path` is a scope-relative `ScopedPath`; it resolves to `VirtualPath` only at the scoped→root boundary. `RootFilesystem::put_batch` takes `VirtualPath`.

---

### 6.1 Primitive (i) — `put_batch`: atomic/efficient multi-put

**Consumer:** thread message append (idempotency + thread `next_sequence` bump + message + seq index, mixed `Absent`+`Version` CAS, all-or-nothing). Also the governor's reservation-record-plus-account-deltas commit on the Postgres tier (§7).

**Signature (root.rs):**

```rust
pub struct BatchPut { pub path: VirtualPath, pub entry: Entry, pub cas: CasExpectation }

/// Apply a set of puts atomically: all succeed or none do. Returns one
/// RecordVersion per input in request order. On any CAS failure, fails with
/// the FIRST offending path's VersionMismatch and writes nothing. All paths
/// MUST share the mount that received the call (composite enforces this).
async fn put_batch(&self, puts: Vec<BatchPut>)
    -> Result<Vec<RecordVersion>, FilesystemError>;
```

**Default impl:**

```rust
// Single-key fast path: every backend supports this with no txn.
if puts.len() == 1 {
    let BatchPut { path, entry, cas } = puts.into_iter().next().expect("len==1");
    return Ok(vec![self.put(&path, entry, cas).await?]);
}
// Multi-key: delegate to the txn plane (Unsupported when backend lacks MultiKey —
// exactly today's TransactionalMessageWrite::Unsupported behavior).
let prefix = puts.first().map(|p| p.path.clone()).ok_or(/* Backend: empty put_batch */)?;
let mut txn = self.begin(&prefix).await?;       // Unsupported => caller takes its fallback
let mut versions = Vec::with_capacity(puts.len());
for BatchPut { path, entry, cas } in puts {
    match txn.put(&path, entry, cas).await {
        Ok(v) => versions.push(v),
        Err(e) => { txn.rollback().await; return Err(e); }
    }
}
txn.commit().await?;
Ok(versions)
```

**Native Postgres impl (postgres.rs):** one `BEGIN … COMMIT` on a single pooled connection, issuing each put as the **exact per-CAS SQL the single `put()` emits** (GROUNDING 2), short-circuiting on the first affected-row-count of 0:

```sql
BEGIN;
-- Absent:  INSERT … VALUES (…,1) ON CONFLICT (path) DO NOTHING RETURNING version;  -- 0 rows ⇒ VersionMismatch{expected:None} ⇒ ROLLBACK
-- Version: UPDATE … SET …, version=version+1, updated_at=NOW()
--          WHERE path=$ AND is_dir=FALSE AND version=$ RETURNING version;            -- 0 rows ⇒ VersionMismatch{expected:Some(v)} ⇒ ROLLBACK
-- Any:     INSERT … ON CONFLICT (path) DO UPDATE SET …, version=…+1 RETURNING version;
COMMIT;
```
`RETURNING version` gives every assigned version in the same round-trip.

**Connection-pool bound (review B blocker — PR #5081 deadlock).** A `put_batch` holds **one** deadpool connection for the duration of `BEGIN…COMMIT`. Bound it: (a) `put_batch` batches are **statically sized** by their consumer (thread append = ≤4 puts; governor commit = ≤7), never unbounded; (b) the existing 30s checkout timeout + `pool_max_size=16` (set in the #5081 fix) apply unchanged; (c) document a hard cap `MAX_BATCH_PUTS = 64` and reject larger batches with a typed `Backend` error so a held `BEGIN` can never starve the pool. No long-lived interactive handle is exposed.

**Native libSQL impl (libsql.rs):** `BEGIN IMMEDIATE … COMMIT` (the dialect gotcha — *not* deferred, so the write lock is taken up front; a deferred txn that upgrades mid-statement can hit `SQLITE_BUSY` after partial work and violate all-or-nothing). Per-put SQL is the libsql `?N`/`is_dir=0`/`strftime` variant.

**Version-readback (review B — libsql `put()` never reads version back today).** libsql `put()` returns version arithmetically (`expected.next()` / `from_backend(1)`), and there is **no `RETURNING` usage in libsql.rs today**. `put_batch` needs the real assigned version. Resolution, in order:
1. At store init, **probe once** whether the bundled libSQL build supports statement-level `RETURNING` inside `BEGIN IMMEDIATE` (run `INSERT … RETURNING version` against a scratch row in a txn). Cache the result.
2. If supported, use `… RETURNING version` like Postgres.
3. If not, after each `INSERT/UPDATE` do `SELECT version FROM root_filesystem_entries WHERE path=?1` **inside the same `BEGIN IMMEDIATE`** before `COMMIT`. Still atomic (same write lock), +N reads.

**The capability advertisement gates on the probe (review B).** libSQL advertises `BatchPut` **only after** the probe confirms a working version-readback mechanism. If neither `RETURNING` nor in-txn `SELECT` works (should not happen, but Fail Loud, Rule 12), libSQL does **not** advertise `BatchPut`, and the thread store falls back to its CAS-loop path. No assumption; verified at init.

**Semantic-parity argument:** N puts either all commit (N versions, each prior+1 or 1) or none commit and the first failing CAS surfaces `VersionMismatch{expected,found}`. Identical observable result on both dialects and on the default impl (which produces the same commit-or-rollback via `begin`). Version increment per path is `version+1` in SQL on both. A consumer cannot distinguish native from default except by latency.

**Contract gotcha (review B).** On libSQL *without* a native override (PR-1, defaults only), a multi-key `put_batch` returns `Unsupported` because libsql has no `begin` override. So PR-1 contract tests for N>1 `put_batch` run **against Postgres and in-memory** (which inherit working defaults), and the single-key (N==1) path runs everywhere. The libSQL N>1 path goes green in PR-3 when the native override lands. This is stated, not hidden.

**Capability:** `BatchPut` bit, independent of `TxnCapability::MultiKey`. A closed, statically-known write set is optimizable to one statement even on a CAS-tier backend; consumers needing only batch-append gate on `BatchPut`, not the open `StorageTxn` tier.

**Composite dispatcher:** override `put_batch` to verify **every** `BatchPut.path` resolves to the **same** mount (longest-prefix); else `PathOutsideMount`, nothing written (mirrors `StorageTxn` prefix scoping). Then delegate.

---

### 6.2 Primitive (ii) — `adjust_indexed`: atomic conditional numeric delta

**Consumer:** resource governor per-account counter math. Turns "increment one I64 counter under a ceiling" into a single conditional `UPDATE`, eliminating the read-modify-write of the blob and taking a row/write lock so concurrent adjusts to the *same* account serialize at the storage layer (no Rust-side mutex, no cross-process CAS-contention hard error).

**Scope decision:** operates on a **single I64 indexed key** of a **single record**, never on a counter buried in `body`. Minimal primitive that unblocks the governor *once its per-account counters live in per-account records with the counter projected into `indexed`*. Not a general expression engine (Rule 2 + invariant 3).

**Signature (root.rs):**

```rust
pub enum AdjustGuard { None, AtLeast(i64), AtMost(i64) }   // checked on POST-adjustment value
pub struct AdjustOutcome { pub value: i64, pub version: RecordVersion, pub applied: bool }

/// Atomically add `delta` to the I64 indexed key `key` on the record at `path`,
/// subject to `guard`, returning the new value. Guard violation is a NORMAL
/// result (applied=false, no write), not an error — the governor uses it to deny.
/// Absent record: if init_absent=Some(base), create carrying {key: base+delta}
/// under Absent semantics; else NotFound. body and other indexed keys untouched.
async fn adjust_indexed(&self, path: &VirtualPath, key: &IndexKey, delta: i64,
    guard: AdjustGuard, init_absent: Option<i64>)
    -> Result<AdjustOutcome, FilesystemError>;
```

**Default impl (read-modify-write loop, preserves today's governor exactly):**

```rust
for _ in 0..ADJUST_CAS_RETRIES {
    match self.get(path).await? {
        Some(v) => {
            let old = read_i64(&v.entry.indexed, key)?;                 // typed err if not I64
            let new = old.checked_add(delta).ok_or(/* overflow Backend err */)?;
            if !guard.permits(new) {
                return Ok(AdjustOutcome { value: old, version: v.version, applied: false });
            }
            let mut entry = v.entry.clone();
            entry.indexed.insert(key.clone(), IndexValue::I64(new));
            match self.put(path, entry, CasExpectation::Version(v.version)).await {
                Ok(nv) => return Ok(AdjustOutcome { value: new, version: nv, applied: true }),
                Err(FilesystemError::VersionMismatch { .. }) => continue,   // retry
                Err(e) => return Err(e),
            }
        }
        None => match init_absent {
            Some(base) => {
                let new = base.checked_add(delta).ok_or(/* overflow */)?;
                let mut entry = Entry::json_record(/* kind */); entry.indexed.insert(key.clone(), IndexValue::I64(new));
                match self.put(path, entry, CasExpectation::Absent).await {
                    Ok(nv) => return Ok(AdjustOutcome { value: new, version: nv, applied: true }),
                    Err(FilesystemError::VersionMismatch { .. }) => continue,   // lost create race ⇒ RETRY against now-existing row
                    Err(e) => return Err(e),
                }
            }
            None => return Err(FilesystemError::NotFound { path: path.clone(),
                                operation: FilesystemOperation::AdjustIndexed }),
        }
    }
    Err(/* Backend: adjust_indexed retries exhausted */)
}
```

**Native Postgres impl (postgres.rs):** one statement; guard evaluated in-row on the post-adjustment value, reusing the **exact** `jsonb_typeof='number'` + `::bigint` discipline already in `query` Range filters (PR #3659) so the counter is interpreted identically to how `Filter::Range` reads it:

```sql
UPDATE root_filesystem_entries
SET indexed = jsonb_set(indexed, ARRAY[$key], to_jsonb(((indexed->>$key)::bigint) + $delta)),
    version = version + 1, updated_at = NOW()
WHERE path = $path AND is_dir = FALSE
  AND jsonb_typeof(indexed->$key) = 'number'
  AND ( $guard='none'
        OR ($guard='at_most'  AND ((indexed->>$key)::bigint)+$delta <= $bound)
        OR ($guard='at_least' AND ((indexed->>$key)::bigint)+$delta >= $bound) )
RETURNING ((indexed->>$key)::bigint) AS value, version;
```
- 1 row ⇒ applied; `value`/`version` from `RETURNING`.
- 0 rows ⇒ disambiguate with one `SELECT (indexed->>$key)::bigint, version WHERE path=$path` on the same connection:
  - present & numeric ⇒ guard rejection → `applied=false`, current `value`/`version`.
  - absent ⇒ honor `init_absent`: `INSERT … indexed = jsonb_build_object($key, base+delta), version=1 ON CONFLICT (path) DO NOTHING RETURNING …`; **on 0 rows (lost create race) RETRY the whole adjust against the now-existing row** (review B — native and default must agree on the create-race); else `NotFound`.

**Native libSQL impl (libsql.rs):** SQLite has weaker numeric typing, so the cast safety Postgres gets for free is made explicit with `typeof(...) IN ('integer','real')` and `+ 0` coercion, wrapped in `BEGIN IMMEDIATE` (write lock held through the disambiguation read):

```sql
UPDATE root_filesystem_entries
SET indexed = json_set(indexed, '$.'||?key, json_extract(indexed,'$.'||?key) + ?delta),
    version = version + 1, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
WHERE path = ?path AND is_dir = 0
  AND typeof(json_extract(indexed,'$.'||?key)) IN ('integer','real')
  AND ( ?guard='none'
        OR (?guard='at_most'  AND json_extract(indexed,'$.'||?key)+?delta <= ?bound)
        OR (?guard='at_least' AND json_extract(indexed,'$.'||?key)+?delta >= ?bound) )
RETURNING json_extract(indexed,'$.'||?key) AS value, version;
```
`changes()` distinguishes applied vs 0-row; same disambiguation + **create-race retry** as Postgres; same `RETURNING`-or-in-txn-`SELECT` version-readback probe as §6.1. `IndexKey` is validated `[A-Za-z_][A-Za-z0-9_]*` (PR #3661, no `.`/`-`), so `'$.'||?key` cannot inject a JSON-path traversal; still **bind** `key`, never string-format it.

**Semantic-parity argument:** native and default produce the same `(value, version, applied)`. Guard rejection never writes. `version = version + 1` once per applied adjust, identical to the default's `put(Version)`, so two backends running the same adjust sequence end at the same value (versions may differ in number but the stored I64 is identical). The PG/libSQL numeric guards are chosen so an `IndexValue::I64` round-trips identically and the guard comparison is numeric on both — the **same** discipline `Filter::Range` already enforces, so a counter written by `adjust_indexed` stays queryable by `Filter::Range` on both backends. Concurrency: PG `UPDATE` row lock; libSQL `BEGIN IMMEDIATE` write lock — both serialize concurrent adjusts so no two see the same `old`, eliminating the cross-process CAS-contention hard error.

**Capability:** `AdjustIndexed` bit. Advertised only after the same version-readback probe (libSQL).

**`adjust_indexed` alone does NOT fix the governor (review B blocker — acknowledged).** A cascade reserve touches 1–6 accounts plus a reservation create; that is a multi-counter, multi-record *transaction*, not one adjust. `adjust_indexed` provides the **contention-free, atomic single-account increment**; the **multi-account atomicity** is provided by `put_batch`/`begin` on Postgres and an explicit admission protocol on libSQL (§7). The two primitives compose; neither alone suffices for the governor. This is stated, not papered over.

---

## 7. Hot-store decomposition — RESOURCE GOVERNOR (top priority)

### 7.1 Before

One process-global `/resources/snapshot.json` under `ResourceScope::system()` holding four `HashMap`s + reservations + period anchors. `update_snapshot` (`cas_snapshot.rs:177`) does full read-modify-write CAS, serialized in-process by one global `filesystem_record_lock`, cross-process by blob CAS with **no retry** — the loser gets `"cross-process CAS contention"` and the reserve fails. Two agents in **unrelated tenants** contend on this one row. `filesystem_store.rs:62-65` is explicit: resources is process-global; per-tenant accounting is a "future capability." **This decomposition introduces per-account row separation where today there is one system() blob — it is therefore also a (small) scoping change, not pure granularity. Stated per review A.**

### 7.2 After — record schema

| Family | VirtualPath | `RecordKind` | `body` (JSON) | `indexed` |
|---|---|---|---|---|
| Account ledger | `/resources/accounts/{account_seg}.json` | `resource_account` | `{ schema_version, account, limits, reserved: I64-tally, usage: I64-tally, period_end_at_anchor, applied_reservations: BTreeSet<ResId> }` | `{ "kind_tag": Text, "tenant": Text, "reserved_usd": I64, … per-resource I64 counters that adjust_indexed targets … }` |
| Reservation | `/resources/reservations/{reservation_id}.json` | `resource_reservation` | `ReservationRecord { reservation, accounts, tally, status, actual }` | `{ "account_seg": Text(owner), "status": Text("pending"\|"active"\|"reconciled"\|"released") }` |

The four `HashMap`s collapse into **one record per account**; `reserved`/`usage` (already per-account sums) become fields on that account's own record. Each per-resource counter that `adjust_indexed` must touch (e.g. `reserved_usd`, `reserved_input_tokens`) is **projected into `indexed` as an I64** so the native conditional `UPDATE` can read+guard+write it without parsing `body`. The authoritative tally also lives in `body` for full fidelity; the `indexed` I64 is the queryable+adjustable projection, kept in sync on every write (the same value, two places — body for fidelity, indexed for native ops).

#### `{account_seg}` — injective key (review C blocker)

`ResourceAccount::Display` renders absent slots as the literal `_` and is **not proven injective** — a real id equal to `_` collides two distinct accounts; ids have **no verified char validation** in the resources crate, so `/`/`..`/empty/control chars would mis-map or be rejected by `VirtualPath::new`. **Do not use `Display` as a storage key.** Instead:

```
account_seg = hex(sha256(canonical_json(&account)))   // fixed-length, path-safe, structurally injective
```
`canonical_json` is a deterministic, field-ordered serialization of the `ResourceAccount` enum (variant tag + all id fields). This is collision-free regardless of id contents, needs no id char validation, and is a safe fixed-length path component. The human-readable identity stays in `body.account` and `indexed.tenant`/`indexed.kind_tag` for query/admin. **Requirement:** add a `canonical_json` impl + a unit test asserting two distinct accounts (including the `_`-collision case and an id containing `/`) hash to distinct segs.

### 7.3 After — reserve algorithm (the global-contention fix)

`cascade(scope)` returns 1–6 accounts ordered **shallow→deep** (tenant first; `lib.rs:222`, min 2 = tenant+user). Two-phase, ordered:

```
reserve(scope, estimate, reservation_id):
  accounts  = cascade(scope)                  // ordered tenant→…→thread
  requested = ResourceTally::from_estimate(estimate)

  // PHASE 1 — read+check each account ledger (no writes)
  loaded = []
  for acc in accounts (shallow→deep):
     (ledger, version) = get(/resources/accounts/{seg(acc)}) or (fresh_ledger, Absent)
     advance_period_if_rolled_over(ledger, now)        // see 7.5 — MUST be persisted
     evaluate_cascade_for_account(acc, ledger.limits, ledger.usage, ledger.reserved, requested)?  // Deny/Approval short-circuit, UNCHANGED business logic
     loaded.push((acc, ledger, version))

  // PHASE 2 — commit atomically across the touched accounts
  if backend.capabilities().txn() == MultiKey:        // Postgres
     // single transaction: all account deltas + reservation create commit together
     puts = loaded.map(|(acc,ledger,ver)| BatchPut{
              path: /resources/accounts/{seg(acc)},
              entry: ledger.with_reserved_added(requested).into_entry(),   // body + indexed I64 both bumped
              cas: ver.map(Version).unwrap_or(Absent) })
          ++ [ BatchPut{ path:/resources/reservations/{id}, entry: reservation(active), cas: Absent } ]
     put_batch(puts)?            // VersionMismatch on any leg ⇒ whole batch rolls back ⇒ retry reserve
  else:                                                // libSQL (CAS-only floor)
     admission_protocol(loaded, requested, reservation_id)   // see 7.4
```

**The win:** two reserves in different tenants now CAS **disjoint** account paths (`sha256(tenant-A…)` vs `sha256(tenant-B…)`) and **never contend**. The `filesystem_record_lock` is taken per-account-path, not globally. Same-tenant reserves serialize only on the *shared* shallow ledgers (tenant + user) — the correct, minimal contention surface, because they genuinely share that budget. `cascade`'s shallow→deep order doubles as **deadlock-free lock ordering** for the Postgres `put_batch` (row locks taken in a consistent order). `evaluate_cascade_for_account` is **untouched** (Rule 3).

`reconcile`/`release` are the symmetric inverse keyed off `record.accounts`: decrement `reserved`, accrue `usage` (reconcile), via the same `put_batch` (MultiKey) or admission protocol (CAS-only), and flip the reservation status.

### 7.4 The multi-account atomicity hole on the CAS-only floor (review A + B blocker — resolved honestly)

**The problem, stated plainly:** admission is all-or-nothing across the cascade (`lib.rs:218-222`: "succeeds only if every account remains within its limit"). On the CAS-only (libSQL) tier, if we naively apply per-account `adjust_indexed` calls one at a time, two concurrent reserves sharing the tenant row can **each** read tenant-under-limit, each pass independent per-row guards, and **both commit** — transiently breaching the tenant limit before either observes the other. Compensation-after-the-fact does **not** prevent this; the breach is already visible and costed work may already be authorized. Framing this as a "safe fail-soft reconcile" (as the first angle did) is **wrong** for a hard quota. We pick and state the trade:

**Chosen design: a single atomic admission gate per cascade root, preserving the invariant on both backends.**

- **Postgres (`MultiKey`):** the `put_batch` in §7.3 *is* the atomic gate. All account-delta `Version` CAS-puts + the reservation `Absent` insert commit in one transaction. Because Phase-1 read each ledger's `version` and Phase-2 CAS-bumps each against that exact version, any concurrent reserve that touched a shared (tenant/user) ledger first will have bumped its version, failing this batch's `Version` CAS on that leg → whole batch rolls back → retry. Two reserves sharing the tenant row are therefore **serialized by the shared ledger's version**, and the all-or-nothing check holds. Correct, no extra machinery.

- **libSQL (CAS-only floor):** there is no multi-key transaction, so we introduce a **narrow atomic admission record per cascade root** — the **broadest shared account in the cascade** (the tenant ledger, always present, `lib.rs:222` min). The reserve does its Phase-1 check, then performs the **commit as a single `adjust_indexed` on the tenant ledger's `reserved_*` counter with `AtMost(limit)` guard** — this one conditional `UPDATE` takes the libSQL write lock and is the linearization point. If it `applied=false` (guard violation under concurrency), the reserve is denied — **correctly, atomically, no overshoot**. Only after the tenant-leg admission succeeds does the reserve apply the narrower-account deltas (user/project/agent/…) and write the reservation record, each idempotent (see below). The deeper accounts are *subordinate* to the tenant admission; they can be applied with ordinary CAS retries because a breach of a deeper, narrower limit is caught by *its own* `adjust_indexed` `AtMost` guard in the same way — the key property is that **each account's own limit is enforced by its own atomic guarded adjust**, and the *broadest* account (tenant) is the single serialization point that all reserves in that tenant funnel through.

  This re-introduces a contention point, but a **per-tenant** one (the tenant ledger), not a **global** one. That is the correct and minimal surface: reserves in *different* tenants still never contend; reserves in the *same* tenant serialize on the tenant admission, which they must, because they share the tenant budget. This is strictly better than today's global, no-retry blob.

**Idempotency on the CAS-only floor (review C — bounding `applied_reservations`):** each account ledger carries `applied_reservations: BTreeSet<ResId>`. An increment is "if `id ∉ applied`, `reserved += requested; applied.insert(id)`" — a retry that already applied is a no-op, collapsing at-least-once into exactly-once across the multi-step libSQL commit. **Bounding:** `applied_reservations` is pruned on reconcile/release. A leaked `Pending`/`active` reservation that is never reconciled keeps its id in the set until a **crash-sweeper GC** (already conceptually present) reclaims it. **The sweeper is the stated bounding mechanism** — set growth is bounded by the count of in-flight + leaked reservations, and the sweeper is what reclaims leaks. On `MultiKey` backends `applied_reservations` is still written (parity: identical records both backends) but is belt-and-suspenders since the txn already guarantees once-only.

**Retry posture:** restore a bounded retry loop, `RESOURCE_CAS_RETRIES = 16` with jittered backoff (matching turn-state's posture), replacing today's no-retry hard failure. Exhausting it surfaces a typed `Unavailable`, not corruption.

### 7.5 Period rollover must be persisted (review C)

`advance_period_if_rolled_over` mutates `ledger.usage`/`period_end_at_anchor` in memory during the Phase-1 read. In the per-record model this in-memory mutation **must be committed via the Phase-2 write even when the reserve check is otherwise non-mutating** (e.g. a check that ultimately denies, or a pure rollover) — otherwise rollover silently never persists and the period never advances. Concretely: if Phase-1 advanced an anchor, that account's ledger is included in the Phase-2 `put_batch`/admission write with its rolled-over `usage`+`anchor`, CAS'd by its read version, **even if `requested` is zero or the reserve is denied on a different account**. A rollover write that loses its CAS simply retries (the next read sees the advanced anchor). This is an explicit per-account CAS write inside the cascade, with the same retry/idempotency as the reserve.

### 7.6 Indexes

```rust
ensure_index("/resources/accounts",     IndexSpec{ name:"acct_by_tenant",  keys:["tenant"],            kind: Exact });
ensure_index("/resources/reservations", IndexSpec{ name:"resv_by_status",  keys:["status"],            kind: Exact });
ensure_index("/resources/reservations", IndexSpec{ name:"resv_by_account", keys:["account_seg","status"], kind: Exact });
```
`resv_by_status` lets the GC sweeper `query(reservations, Eq{status:"pending"|"active"})` instead of a full-blob scan. Exact only; no FTS/Vector ⇒ `sql_typical()` suffices ⇒ mount validator cannot reject. Composite `["account_seg","status"]` is a multi-expression index on both SQL backends.

### 7.7 In-place migration (blob → per-account records)

Forward backfill, online in `DualWrite`, idempotent, value-preserving, **`CasExpectation::Absent` skip-if-present** (so it never clobbers a live native increment):

```
migrate_resources_to_native():
  (snapshot,_) = get(/resources/snapshot.json)   // None on fresh deploy ⇒ return
  for acc in union(snapshot.state.limits.keys, reserved.keys, usage.keys, anchors.keys):
     ledger = ResourceAccountRecord{ schema_version:2, account:acc, limits:limits[acc],
              reserved:reserved[acc].unwrap_or_default(), usage:usage[acc].unwrap_or_default(),
              period_end_at_anchor:anchors[acc],
              applied_reservations:{ res ids whose .accounts ∋ acc } }
     put(/resources/accounts/{seg(acc)}, ledger.into_entry(/* body + indexed I64s */), Absent)
  for (id,rec) in snapshot.state.reservations:
     put(/resources/reservations/{id}, rec.into_entry(), Absent)
```
The legacy blob stays (never-delete). Sequence: flip `DualWrite` → backfill `Absent` → `verify_parity` until clean → flip `Native`.

---

## 8. Hot-store decomposition — TURN-STATE + EVENT-LOG, THREADS

### 8.0 The turn-scope premise correction (review A blocker)

The first angle claimed `/turns/state.json` is "already per-tenant-scope physically." **This is FALSE in code:** `io.rs:92` and `filesystem_store.rs:197` call `put`/`get` with `ResourceScope::system()` — a **constant** — for every tenant. The blob therefore **multiplexes all scopes' runs**, and tenant isolation comes from `TurnScope` fields *inside the body* (`scope.rs:5`: tenant/agent/project/thread/thread_owner), not from the mount. Consequences threaded through this design:
- `scope_seg` is derived from **each run's own `TurnScope`** in the body, not from a physically distinct blob.
- Shape migration iterates `snapshot.runs` and **re-keys each run by its own scope**, not "one blob = one scope."
- The fail-open fallback keys off the body-derived scope.

### 8.1 Turn-state before → after

**Before:** one `system()`-scoped `/turns/state.json` multiplexing all scopes' runs + embedded events; 32-retry whole-blob CAS per transition.

**After (paths are alias-relative under the `/turns` mount; `{scope_seg}` derived from each run's `TurnScope`):**

| Family | VirtualPath | `RecordKind` | `body` | `indexed` |
|---|---|---|---|---|
| Run record | `/turns/runs/{run_id}.json` | `turn_run` | `TurnRunRecord` (status, lease, loop checkpoint) verbatim | `{ "scope_seg": Text, "status": Text, "updated_at": I64(ms), "owner_user": Text, "root_run": Text }` |
| Loop checkpoint | `/turns/runs/{run_id}/checkpoint.json` | `turn_loop_checkpoint` | checkpoint | `{ "scope_seg": Text }` |
| Runner lease | `/turns/runner-leases/{run_id}.json` | `turn_runner_lease` | sidecar (unchanged) | `{ "scope_seg": Text }` |
| Idempotency | `/turns/idem/{sha256}.json` | `turn_idempotency` | record | — |
| Admission reservation | `/turns/admission/{reservation_id}.json` | `turn_admission_reservation` | record | — |
| Spawn-tree reservation | `/turns/spawn-tree/{root_run_id}.json` | `turn_spawn_tree_reservation` | record | — |
| Event floor | `/turns/meta/event-floor.json` | `turn_event_floor` | small record (CAS-by-version) | — |
| **Events** | `/turns/events/log` | n/a — **append/tail plane** | `TurnLifecycleEvent` JSON bytes | — |

`{run_id}` is the `TurnRunId` (`Uuid`, `ids.rs:36`) hyphenated — path-safe. `scope_seg` is an indexed **Text value** (no `IndexKey` charset constraint), derived deterministically from `TurnScope` fields; it is never a path component, so the PR #3661 charset tightening does not apply. The read-mostly `turns` (`TurnRecord` list) collapses into the run record where 1:1, or stays a single `/turns/turns.json` record in phase 1 — it is not a hot contention point.

**Access patterns (no whole-blob deserialize):**
- Load one run: `get(/turns/runs/{run_id}.json)` → O(1), returns version for next CAS.
- List my runs: `query(/turns/runs, Eq{scope_seg}, Page)` (Exact index).
- List active: `query(/turns/runs, And[Eq{scope_seg}, Eq{status:"running"}], Page)`.
- Recency sidebar: `query` page, sort by projected `updated_at` in Rust (no ORDER BY in `Filter`, status quo).
- Spawn-tree descendants (`children_of`, `reserve_tree_descendants`, `lifecycle.rs:533-565`): `query(/turns/runs, Eq{root_run}, Page)`.

### 8.2 Event-log on the append plane

Events un-embed onto the native `append`/`tail`/`tail_bounded`/`head_seq` plane at `/turns/events/log`:
- Append: after the run-record CAS succeeds, `append(/turns/events/log, serde_json::to_vec(&event)?)` → `SeqNo`. PG `INSERT … RETURNING id`; libSQL `INSERT` + `last_insert_rowid()`.
- Read after cursor: `tail(from)` / `tail_bounded(from, max)` — `WHERE id > ? ORDER BY id ASC LIMIT ?` on both. Replaces in-Rust `project_turn_events()`.
- Floor/retention: `event_retention_floor` → `/turns/meta/event-floor.json` (CAS-by-version) + `head_seq` for O(1) high-water mark.
- `EventCursor(u64)` (`events.rs:20`) maps directly onto `SeqNo(u64)`.

**`head_seq` override verification (review A).** The retention floor relies on `head_seq` being O(1) `MAX(id)`. **Verified present** on both SQL backends (postgres.rs:678, libsql.rs:990 — confirmed in review C). The default impl materializes the gap, so this is load-bearing and confirmed, not assumed.

### 8.3 Run-transition + event-append atomicity (reviews A + C — sharpened)

The split makes the run-record CAS and the event append two operations. **Events cannot join a `StorageTxn`** (it has put/get/delete only; events use the append plane). Resolution:

- **Run record is the single source of truth; the event log is a downstream idempotent projection.** Order: **CAS the run record first (authoritative), then `append` the event.**
- **Idempotency key:** the event payload embeds `(run_id, transition_version)` where `transition_version` is the run's `RecordVersion` after the transition (monotonic per run ⇒ unique stable key). A recovery appender tails and skips if `(run_id, version)` already present.
- **Downgrade the claim (review C): at-least-once with idempotent recovery, NOT exactly-once.** `append` has no CAS (GROUNDING 1 #11); the dedup tail-scan is non-atomic with a concurrent-appender race. So: **duplicate events are tolerated** because an event is a non-corrupting projection of the authoritative run record. The recovery scan is **bounded** (tail at most `RECOVERY_SCAN_WINDOW` records back from `head_seq`); beyond that window we accept a possible duplicate rather than scan unbounded.
- **Crash-loss honesty (review A): non-terminal events are genuinely lossy on crash-between-CAS-and-append.** A crash after the run CAS but before the append drops *that* lifecycle event. For **terminal** transitions the event is re-derivable from the run's terminal status on recovery. For **intermediate/non-terminal** transitions (heartbeat, `blocked_auth`) the event is **not** re-derivable and is genuinely lost — stated plainly (Rule 12). This is strictly better than today (a torn blob write loses run state *and* event), and the run record remains correct. Where a deployment cannot tolerate non-terminal event loss, the run+event pair can use `begin` on `MultiKey` backends for the run-record write and accept the event append as the post-commit projection — but the append itself still cannot be transactional, so the residual non-terminal-loss window is inherent to the append plane and is documented as such.

### 8.4 Turn-state indexes + migration

```rust
ensure_index("/turns/runs", IndexSpec{ name:"runs_by_scope",  keys:["scope_seg"],          kind: Exact });
ensure_index("/turns/runs", IndexSpec{ name:"runs_by_status", keys:["scope_seg","status"], kind: Exact });
ensure_index("/turns/runs", IndexSpec{ name:"runs_by_root",   keys:["root_run"],           kind: Exact });
```

Migration (per the corrected premise — iterate runs, re-key by each run's own scope), online in `DualWrite`, idempotent:
```
migrate_turns_to_native():
  (snap,_) = get(/turns/state.json [system() scope])     // the one multiplexing blob
  for run in snap.runs:            put(/turns/runs/{run_id}, run_entry(scope_seg from run.scope), Absent)   // skip-if-present
  for lease in snap.runner_lease_sidecars: put(/turns/runner-leases/{run_id}, …, Absent)
  // events: one non-idempotent step, guarded by a marker
  if get(/turns/meta/events-backfilled).is_none():
     put(/turns/meta/events-backfilled, marker, Absent)   // claim
     for ev in snap.events (cursor order):  append(/turns/events/log, ev_payload)   // ONCE
  put(/turns/meta/event-floor.json, snap.event_retention_floor, Absent)
  migrate idem/admission/spawn-tree/checkpoints to per-id paths (Absent)
```
Run/lease/idem backfill is `Absent` skip-if-present (idempotent). The event `append` backfill is the **one non-idempotent step**, guarded by the `events-backfilled` marker (`Absent` claim); a re-run sees the marker and skips. Legacy blob retained. Event-log advances to `Native` **after** the run plane (in `DualWrite`, events still live in the snapshot too; only after the run plane stops writing the blob do we stop double-storing events).

### 8.5 Threads — index + escape hatch only (no row decomposition)

Threads is already per-file (`thread.json`, `messages/{id}.json`, `sequences/{seq}.json`, `idempotency/{sha}.json`) with txn-or-CAS multi-put. **No decomposition owed (Rule 3).** Changes:

1. **Adopt `put_batch`** for the message append, replacing the hand-rolled `begin`/`StorageTxn` 4-op txn + `TransactionalMessageWrite::Unsupported` enum. The store **requires** atomicity, so it gates on the `BatchPut` capability (or `begin` availability) and keeps its existing `reserve_sequence`+sequential-CAS fallback for backends advertising neither. Public API unchanged.
2. **Native query indexes** replace bespoke index files:
   ```rust
   ensure_index("/threads", IndexSpec{ name:"msg_by_seq", keys:["thread_seg","sequence"], kind: Exact });
   ensure_index("/threads", IndexSpec{ name:"msg_by_run", keys:["run_id"],                kind: Exact });
   ensure_index("/threads", IndexSpec{ name:"threads_by_scope", keys:["scope_seg"],       kind: Exact });
   ```
   Message `indexed` gains `{ "thread_seg": Text, "sequence": I64, "run_id": Text }`; thread `indexed` gains `{ "scope_seg": Text, "updated_at": I64, "owner_user": Text }`. `list_threads_for_scope` becomes `query(/threads, Eq{scope_seg})` + Rust sort by `updated_at`, retiring the N+1 `list_dir`+per-file `get` (`filesystem_service.rs:1992-2048`). Range "messages [lo,hi]" → `query(Range{key:"sequence", lo:I64, hi:I64})`.
3. **`ForceCas` escape hatch:** an operational flag (`ThreadWritePath { TxnOrCas (default), ForceCas }`) to disable the transactional path instantly if a backend's `begin`/`put_batch` misbehaves. Zero data migration, reversible.

**Range parity (reviews A + C — corrected attribution):** libSQL Range **does** apply a JSON-type guard (`libsql.rs:1590`, PR #3659) — the first angle's "no guard" claim is stale. Because we always write `sequence` as `IndexValue::I64`, the stored JSON is numeric on both backends. A dedicated boundary test (9→10→100) on libSQL specifically asserts numeric, not lexicographic, range correctness.

**Threads migration:** no row rewrite — message records already carry the data; only `indexed` projections must be present. A one-time `put(CasExpectation::Version)` re-projection pass adds `indexed` to pre-existing messages without touching `body` (skip if already populated; value-preserving; parity-safe). `ensure_index`'s `CREATE INDEX` covers existing rows on both backends. Bespoke index files keep being written until the native indexes are backfilled and `verify_parity` confirms `query` returns the same set; then a follow-up change stops writing them.

---

## 9. Dual-backend + migration-safety

### 9.1 Both backends store identical logical records

Every record above is `(VirtualPath, kind, body-JSON, indexed-JSON)` written by the **same Rust code through the same trait**. Per §5, both SQL backends translate the same `put`/`query`/`append`/`ensure_index` calls into dialect SQL that yields byte-identical `contents`/`indexed`/`kind` and equality-comparable `version`. The only new parity surfaces vs today are (a) the set of declared indexes (Exact-only, contract-covered on both) and (b) the two native primitives, whose parity is argued per-primitive (§6) and locked by the native-vs-default harness (§9.5).

### 9.2 libSQL↔Postgres stays a COPY

Shape migration (blob→records) produces records that the backend copy tool transfers verbatim: `query(prefix, Filter::All, page)` paging `/turns`/`/resources`/`/threads`, `put(dest, entry, Any)` each; copy the event plane with `tail`/`append`. No transform — there is no schema embedded in a consumer record beyond JSON + indexed projection, both of which round-trip identically. Version numbers are not preserved bit-for-bit (opaque, equality-only); the destination assigns its own monotonic versions and consumers re-read before their next CAS.

### 9.3 The DualWrite divergence window (review C — stated)

In `DualWrite`, the snapshot write and the native write are **two separate non-transactional CAS operations**. A crash between them leaves native stale relative to the authoritative snapshot until the next write. This is **expected and tolerated**, not atomic: the snapshot stays authoritative through `DualWrite`, and `verify_parity` (the promotion gate) asserts convergence **at quiescence**, tolerating in-flight skew. This is stated, not implied to be atomic.

### 9.4 The migration-safety invariant + CI parity test (required deliverable)

The invariant is §3.7. The CI test that asserts it:

> **`store_parity` test (per store, both backends):** drive an identical sequence of store operations (the store's full public API surface — reserve/reconcile/release; submit/heartbeat/complete/fail; thread append/range/list) against a libSQL-backed and a Postgres-backed instance. After each op, dump every record under the store's prefix (`query(prefix, All)` → multiset of `{path, kind, body, indexed}`) **and** the append-plane payloads (`tail` from 0). Assert the two backends' multisets and payload sequences are **equal** (versions excluded). Any divergence fails CI.

This runs under `--features integration` (Postgres) and is the executable form of the invariant.

### 9.5 Native-vs-default equivalence harness (review B — fixed)

To prove a native override is observably indistinguishable from its default fallback (the heart of the migration-safety claim for the new primitives): run the entire primitive suite twice against each SQL backend — once native, once forced through the default impl. **The default path is forced by a non-overriding newtype wrapper** around the backend that does **not** override `put_batch`/`adjust_indexed` (so Rust dispatches to the trait defaults) — **NOT** by masking capability bits (bit-masking does not change method dispatch; review B). Dump `(path, contents, indexed, version-equality)` after each and assert identical final state.

### 9.6 Contract-test reality (review B — corrected)

The db contract tests are **hand-duplicated per backend** (`libsql_root_filesystem_*` vs `postgres_root_filesystem_*` in `db_root_filesystem_contract.rs`), **not** a single parameterized harness. Building the parameterized matrix (each new contract case × both backends × native/default) is **net-new test infrastructure**, sized into PR-1, not a free extension.

---

## 10. Test & rollout

### 10.1 Per-store flag + reverse job

Each store: one enum config read **once at construction**, frozen into the store struct, advanced independently:

```rust
pub enum StoreModel { #[default] Snapshot, DualWrite, Native }   // turns, resources, event-log
pub enum ThreadWritePath { #[default] TxnOrCas, ForceCas }       // threads (no snapshot)
```
Env keys: `IRONCLAW_RESOURCES_MODEL`, `IRONCLAW_TURNS_MODEL`, `IRONCLAW_EVENTLOG_MODEL`, `IRONCLAW_THREADS_WRITEPATH`.

State machine: `Snapshot ⇄ DualWrite` is free/instant (snapshot stays authoritative through `DualWrite`). `DualWrite → Native` is the only gated step (backfill + `verify_parity` clean). `Native → DualWrite` requires `rematerialize_snapshot()` (reads native via `query`, writes the legacy blob with `Any`) — a first-class rollout artifact, not an afterthought.

### 10.2 Contract tests (both backends)

Extend `db_root_filesystem_contract.rs` (net-new parameterization, §9.6):
- `put_batch` all-or-nothing: 4 mixed-CAS puts all land with correct versions; a stale `Version` leg ⇒ `VersionMismatch` and **none** of the others written. (N>1 runs PG+in-memory in PR-1; libSQL in PR-3.)
- `put_batch` cross-mount rejection ⇒ `PathOutsideMount`, nothing written.
- `adjust_indexed` guard semantics: `{count:5}`, `+3 AtMost(10)` ⇒ value 8 applied, version bumped; `+5 AtMost(10)` ⇒ value 8, **applied=false, no write**; `-100 AtLeast(0)` ⇒ rejected. Assert resulting values equal across backends.
- `adjust_indexed` numeric-type parity: read the counter back via `Filter::Range` and `get`→`IndexValue::I64`; assert numeric (not text) on both (guards the SQLite-typing hazard).
- `adjust_indexed` create-race: concurrent `init_absent` creators ⇒ one creates, the other **retries against the now-existing row** and applies its delta (native == default).
- Native-vs-default equivalence (§9.5).

### 10.3 Consumer-level tests (test-through-the-caller, CLAUDE.md)

- **Governor:** (a) two reserves in disjoint tenants succeed concurrently touching **disjoint** account paths (instrumented filesystem asserts no shared path CAS'd); (b) two same-tenant reserves serialize on the tenant ledger; (c) reserve→reconcile→release returns `reserved` to baseline + accrues `usage`, **byte-identical ledger JSON on both backends**; (d) **crash-injected mid-cascade** reserve leaves a `Pending` reservation the sweeper completes/rolls back with no double-charge; (e) **CAS-only path (force `Cas` tier) produces the same final ledger state as the `MultiKey` path — the parity floor test**; (f) concurrent same-tenant reserves at the limit never over-admit (the admission-gate correctness test, §7.4).
- **Turns/event-log:** (a) N concurrent transitions on distinct runs in one scope no longer contend; (b) crash between run-`put` and event-`append` → run state intact, terminal event re-derived on recovery, no run-state loss; **non-terminal event loss is asserted as tolerated** (the test documents it, not "fixed"); (c) `tail(cursor)` ordering matches the old snapshot projection; (d) `head_seq` returns `MAX(seq)` without materializing the gap (assert query count).
- **Threads:** (a) composite `msg_by_seq` Range across the 9→10→100 boundary on **libSQL specifically**; (b) `TxnOrCas` and `ForceCas` produce identical final state; (c) concurrent appends produce gapless unique sequences with no lost messages on both backends; (d) idempotency record blocks duplicates identically.

### 10.4 Fault/latency-injection harness

A `FaultInjectingFilesystem` decorator (itself a `RootFilesystem`, composes per invariant 1):
- **CAS-loss:** force `VersionMismatch` on the Nth `put` ⇒ assert reserve/transition retries and converges within bound (governor 16, turns 32, threads 8); exceeding ⇒ typed `Unavailable`, not corruption.
- **Latency:** per-call delay ⇒ assert the governor's per-account decomposition yields higher cross-tenant throughput than the snapshot baseline (50 concurrent reserves across 50 tenants must not serialize: wall-clock ≤ k·single-reserve-latency, vs ≈50·latency for the blob). This is the regression test for the *value* of the fix.
- **Crash:** panic-after-leg in the admission/`put_batch` paths ⇒ restart ⇒ sweeper restores the invariant, no double-charge.
- **Unsupported:** force `begin` → `Unsupported` ⇒ governor + threads fall through to CAS-only/compensation and still pass the equality test (proves the floor).

Run against in-memory every PR; both SQL backends under `--features integration`.

### 10.5 Rollout (per-store, independent, reversible, green on both)

Order by value/risk: **Governor → Turn-state → Event-log → Threads** (threads can also go first as a warm-up — no row migration).

Shared per-store steps:

| Step | Action | Reversible? |
|---|---|---|
| S0 | Land record model + indexes + backfill + `verify_parity` + `rematerialize_snapshot` + fault tests behind flag, default `Snapshot`/`TxnOrCas`. No prod behavior change. | n/a (dead code) |
| S1 | `DualWrite` in staging on **libSQL**. Native best-effort, snapshot authoritative. | Instant flip to `Snapshot`. |
| S2 | Backfill (`Absent` skip-if-present) + `verify_parity` on libSQL until zero divergence. | Idempotent re-run. |
| S3 | Flip libSQL `Native`; full fault suite + cross-tenant throughput test. | Flip `DualWrite` + run `rematerialize_snapshot`. |
| S4 | Repeat S1–S3 on **Postgres** (independent; parity guaranteed by contract tests). | Same. |
| S5 | Production: `DualWrite` → backfill → verify → `Native`, one backend at a time, throughput dashboard watched. | Same at every sub-step. |

**Filesystem-primitive prerequisite PRs (land before any consumer migration):**
1. **PR-1:** trait + capability surface, default impls only; `Capability`/`FilesystemOperation`/`sql_typical_hotpath`/`NEW_AXES`; composite delegation; **`ScopedFilesystem` wrappers + `operation_allowed` arms**; net-new parameterized contract infra against the **default path**. Pure addition, zero behavior change, reversible.
2. **PR-2:** Postgres native overrides + advertise; native-vs-default equivalence green for PG; pool-occupancy bound documented.
3. **PR-3:** libSQL native overrides + the version-readback **probe**; advertise `BatchPut`/`AdjustIndexed` only after probe confirms; suite green for libSQL.
4. **PR-4:** in-memory atomicity audit — confirm/lock-fix that the default path holds the per-op lock across the whole primitive (the one place "defaults are free" could be wrong); advertise the bits.

**Independence guarantee:** per-store flag + disjoint path prefixes (`/resources`, `/turns`, `/threads`) ⇒ a store can be `Native` while others are `Snapshot`; a regression reverts that store alone.

**Both-backends guarantee:** every promotion is gated on contract + `store_parity` passing on *that* backend; libSQL and Postgres promote independently.

**The one one-way door (Rule 12):** the turn-state **event backfill** uses `append`, which assigns fresh seqs and cannot be cleanly un-appended. After event-log reaches `Native`, reversal accepts the native event log as authoritative (the snapshot's `events[]` is rematerialized from `tail()` on reverse). The run plane stays fully reversible. Keep event-log at `DualWrite` for a longer soak than the run plane before its independent `Native` cutover.

Each PR green under `cargo clippy --all --tests --all-features` + `cargo test --features integration` (both backends), per the merge-queue gates.

---

## 11. Risks & mitigations

| Risk | Mitigation |
|---|---|
| **CAS-only governor over-admission** (concurrent same-tenant reserves both pass) | Single atomic admission gate on the **broadest (tenant) ledger** via guarded `adjust_indexed` (libSQL) / `put_batch` version-CAS serialization (PG). Per-tenant contention, not global. Asserted by test 10.3(f). |
| **Account-seg collision** (`Display` `_`-placeholder, unvalidated ids) | Key by `hex(sha256(canonical_json(account)))`; injectivity unit test incl. `_` and `/`-in-id cases. |
| **libSQL `RETURNING`-in-txn unavailability** | Init-time probe; fall back to in-txn `SELECT` readback; advertise `BatchPut`/`AdjustIndexed` only after probe confirms. Fail Loud if neither works. |
| **Pool starvation from held `BEGIN`** (PR #5081) | Statically-sized batches (≤7), `MAX_BATCH_PUTS=64` cap, existing 30s checkout + `pool_max_size=16`. |
| **Non-terminal turn event loss on crash-between-CAS-and-append** | Documented as tolerated (run record authoritative; terminal events re-derived). `begin` for the run write on `MultiKey` narrows but cannot eliminate the append-plane window. |
| **PG `||` vs SQLite `json_patch` null divergence** | Avoided entirely — `merge_indexed` dropped (§4); only `adjust_indexed`/`put_batch` added, neither does null-removal. |
| **DualWrite divergence window** | Snapshot authoritative through `DualWrite`; `verify_parity` asserts convergence at quiescence only. |
| **Period rollover never persists** | Phase-1 anchor advance committed via Phase-2 CAS write even on a non-mutating/denied reserve. |
| **`applied_reservations` unbounded growth** | Pruned on reconcile/release; crash-sweeper GC bounds leaked-reservation growth (stated mechanism). |
| **Composite index unsupported on some backend** | Both SQL backends build multi-expression indexes; mount validator refuses a declared index a backend can't serve; in-memory linear-scans (contract still green). |

---

## 12. Open questions

1. **Tenant-ledger admission hot-row on libSQL:** under extreme same-tenant fan-out, the tenant ledger becomes the serialization point. Acceptable (it's the shared budget), but do we want a future sharded-counter scheme for very-high-QPS tenants, or is per-tenant serialization sufficient indefinitely? (Defer; measure first.)
2. **Event recovery scan window:** what is the right `RECOVERY_SCAN_WINDOW` bound vs duplicate-tolerance trade for non-terminal events? Pick empirically from event volume per scope.
3. **Turns `turns.json` (TurnRecord list) split:** keep as one phase-1 record or split per-thread? It is read-mostly and not a contention point; defer the split unless a query pattern demands it.
4. **Should `put_batch` ever raise libSQL to `TxnCapability::MultiKey`?** Its native override uses real `BEGIN IMMEDIATE` multi-statement atomicity, so libSQL *could* implement `begin`/`StorageTxn` too. Out of scope here (`BatchPut` is advertised independently of the txn tier), but it's a natural follow-up that would let the governor use the PG `put_batch` commit path on libSQL as well, retiring the admission-gate special case.
5. **In-memory default-path atomicity (PR-4):** confirmed-or-fixed that the per-op lock is held across the whole primitive; if the backend releases the map lock between `get` and `put` in the default path, a ~10-line native override is owed. Verify at implementation time, do not assume.
