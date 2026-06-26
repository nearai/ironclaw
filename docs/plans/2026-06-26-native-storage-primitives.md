# Native Hot-Store Decomposition on the Unified `RootFilesystem` Trait — Fusion Design

**Status:** Proposed (implementation-grade). Supersedes the three angle drafts (data-model, new-primitives, per-store). Incorporates the required changes from all three structured reviews **and the 2026-06-25 multi-lens review pass (approach / maintainability / local-patterns / thermo-nuclear architecture / concurrency-TOCTOU) plus the CodeRabbit and Gemini bot reviews.** The most consequential change from that pass: the resource governor commits through a **single `put_batch` path on both backends** (libSQL raised to `MultiKey`), which retires the libSQL admission-gate special case, the `applied_reservations` idempotency set, the (non-existent) crash-sweeper dependency, and the `adjust_indexed` primitive entirely. See §4 and §7.

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

The one place where decomposition alone is *not* correctness-equivalent is the resource governor's **multi-account, multi-dimensional admission**, which is an all-or-nothing check across 2–6 cascade accounts (`cascade()` always emits at least tenant+user — `lib.rs:222`) and across the 8 independent `ResourceTally` dimensions. That requires a true multi-key, multi-field atomic commit. This doc provides it with **one mechanism on both backends** — `put_batch` over the touched account records — rather than forking governor behavior per backend.

---

## 2. Goals / Non-goals

### Goals

1. Keep `RootFilesystem` the **one contract**. No caller forks, no per-backend `cfg` in callers — the governor commit is the **same `put_batch` call shape on both backends**.
2. Fix the governor global-contention bug **without weakening the all-or-nothing limit invariant** on either backend, across **all** tally dimensions simultaneously.
3. Decompose turn-state, event-log, and (lightly) threads onto per-entity records / the native append plane.
4. Add the **minimal** native primitive surface — exactly **one** new primitive (`put_batch`) with a correct default fallback on every backend.
5. Prove dual-backend parity with a CI test that asserts both backends produce identical logical records for every op.
6. Ship each store **independently**, feature-flagged, reversible, green on **both** backends at every step.

### Non-goals

- No new "real-backend vs test-backend" trait. One trait, one entry, one mount (GROUNDING 4 §5).
- No removal of the legacy snapshot rows (LLM-data-never-deleted invariant). "Migrated" means *unreferenced*, never *deleted*.
- No re-decomposition of threads (already per-file). Threads gets index/queryability + an escape hatch only (Rule 3).
- No general expression engine inside the filesystem. Backends never parse `body` for logic (GROUNDING 1 #4).
- **No `adjust_indexed` primitive.** An earlier draft proposed a conditional single-key numeric delta for the governor; it is dropped (§4) in favor of `put_batch` over per-account records, which is atomic across all dimensions where `adjust_indexed` was not.

---

## 3. Constraints & Invariants (carried from grounding, non-negotiable)

1. **CAS is the floor.** Every backend supports `put` + `CasExpectation::Version`. Multi-key txns (`begin`/`StorageTxn`) are optional in the trait; this design raises **both** SQL backends to `MultiKey` (libSQL via `BEGIN IMMEDIATE`, §6.1) so the governor's atomic commit has one shape everywhere. Byte-only backends (`local`, in-memory) remain CAS-tier and use the `put_batch` default impl.
2. **Versions are opaque, monotonic-per-path `u64`**, compared only for equality. `version = version + 1` in SQL on both dialects. A backend copy need not preserve version *numbers* — only logical state.
3. **Backends never look inside `body`.** Everything queryable lives in `Entry.indexed` (`BTreeMap<IndexKey, IndexValue>`) or in the version counter. A `put_batch` writes the **whole `Entry`** (body + indexed together), so the two never diverge — there is no native op that mutates `indexed` without rewriting `body`.
4. **Capabilities are honest or you don't mount.** `validate_mount_capabilities` (`catalog.rs`) refuses a mount whose descriptor claims a capability the backend lacks. A mount that advertises a capability the backend serves only by brute force (e.g. `IndexVector`) must not be declared — see §7.9.
5. **Callers do not change shape per backend.** A consumer writes against the trait; native fast path vs default fallback differs only in latency/contention, except where a consumer **requires** atomicity and gates on the capability bit (§6.1).
6. **Consumers reach the filesystem through `ScopedFilesystem`**, not `RootFilesystem` directly. `ScopedFilesystem::resolve((scope, ScopedPath)) -> VirtualPath` applies per-operation permission checks (`scoped.rs:100-222`) before delegating. Any new primitive must have a `ScopedFilesystem` wrapper and an `operation_allowed` arm. **Ownership is enforced here:** a decomposed `query(prefix, …)` returns only records the scope is permitted to read; the scope's tenant/user prefix bounds the result set, and per-record `owner_user` projections (§8.5) let a service assert the authenticated user matches the resource owner before acting (Gemini review).
7. **MIGRATION-SAFETY INVARIANT (stated once, the spine of this doc):**
   > For every store operation, given identical inputs, both backends MUST end with **identical logical records** — the same set of `(VirtualPath, kind, body, indexed)` tuples and the same append-plane payloads in the same seq order — so that a libSQL↔Postgres switch is a verbatim row copy. Version *numbers* may differ; logical state may not.

   This is enforced by a CI parity test (§9.4) that drives every op against both backends and asserts the record sets are equal.

---

## 4. Trait-preservation mechanism

The trait stays frozen as the single contract. Decomposition is a **pure consumer-side key/record-layout change**: stores write many small records instead of one blob, each CAS'd by its own `RecordVersion`. For the one place where true multi-record atomicity materially changes contention, we add **one optional native primitive with a correct default impl built from existing ops**, so byte-only backends keep working with zero changes.

Two mechanisms, in priority order:

**(a) Decomposition with zero new ops (covers turn-state, event-log, threads, and the single-account governor counter math).** `get`/`put`+CAS, `query`/`ensure_index`, `append`/`tail`/`tail_bounded`/`head_seq` already cover every access pattern. The whole-blob CAS loop becomes a per-record CAS loop. No trait change, no caller fork.

**(b) One new optional native primitive — `put_batch`** (§6) — where decomposition needs an atomic multi-record commit (governor cascade, thread message append). It:
- is a `RootFilesystem` method with a **default impl** built from existing ops;
- has a **native override** on Postgres (`BEGIN…COMMIT`) and libSQL (`BEGIN IMMEDIATE…COMMIT`) that commits the whole write set atomically and raises that backend to `TxnCapability::MultiKey`;
- is gated behind a new `Capability::BatchPut` bit enforced at mount;
- is mirrored on `ScopedFilesystem` with a permission-checked wrapper.

**Availability contract (CodeRabbit review — stated precisely).** The `put_batch` *default* impl compiles and runs on every backend, but it is **not atomic on every backend**:
- **N == 1:** every backend executes the single `put` directly — always available, always correct.
- **N > 1 on a `MultiKey` backend** (Postgres, libSQL after §6.1, in-memory once §10/PR-4 confirms the per-op lock): atomic via the txn plane.
- **N > 1 on a CAS-only backend with no `begin`** (e.g. `local`): the default impl returns a typed `Unsupported`.

A consumer that **requires** atomicity MUST gate on `Capability::BatchPut` (or on `begin` returning a usable txn) and take its explicit fallback or reject; it must **not** assume the default path is atomic. `ScopedFilesystem::put_batch` returns a typed `Unsupported` that names the missing `BatchPut` capability so the gap is loud, not silent. Each consumer states which guarantee it requires (§7, §8).

**Why not expose `begin` directly to every caller?** The thread store already hand-rolls a `begin`/`StorageTxn` 4-op txn with a CAS-loop fallback and a `TransactionalMessageWrite::Unsupported` enum. `put_batch` lets a store express *intent* (these N records commit together) in one call; its native override **is** that txn, so `MultiKey` backends behave identically and the consumer never re-implements txn orchestration.

**No `adjust_indexed`.** An earlier draft proposed a conditional single-key numeric delta (`adjust_indexed`) for the governor's per-account counters. It is **dropped**, for three converging reasons surfaced in review:
1. **It cannot serve the governor's actual requirement.** `ResourceTally` has **8 independent dimensions** (`usd`, `input_tokens`, `output_tokens`, `wall_clock_ms`, `output_bytes`, `network_egress_bytes`, `process_count`, `concurrency_slots`). A single-key conditional delta admits exactly one dimension atomically; an 8-dimension reserve would need 8 sequential locks, and two concurrent same-user reserves could each pass dimension-1 then dimension-2 independently and **both over-admit** (concurrency review C1). The atomic unit must be *the whole account record*, which is what `put_batch` commits.
2. **It re-introduced a dual-truth hazard.** `adjust_indexed`'s native form wrote `indexed` via `jsonb_set` while leaving `body` untouched, so a tally lived in two places that the hot path kept in sync only by convention — Phase-1 reads of `body` would go stale and produce false denials (review C3). `put_batch` writes the whole `Entry`; body and indexed are one write.
3. **It would have leaned on machinery that does not exist.** The libSQL idempotency story for `adjust_indexed` required an `applied_reservations` set bounded by a "crash-sweeper GC (already conceptually present)" — but **no such sweeper exists in the tree** (review C2; confirmed: only `ironclaw_product_workflow/src/ledger.rs` has an unrelated TTL sweeper). `put_batch`'s all-or-nothing commit needs no idempotency set and no sweeper.

This also removes the second `Capability` bit, a four-impl primitive, the libSQL version-readback nuance for that op, and a `merge_indexed`-style null-divergence surface. **Simplicity First (Rule 2): one new primitive, not three.** (The original `merge_indexed` was already dropped for the PG-`||`-vs-SQLite-`json_patch` null hazard; `adjust_indexed` now joins it.)

---

## 5. The parity argument, stated once (applies to every record below)

Every record is an `Entry { body: <JSON bytes>, content_type: JSON, kind: Some(<RecordKind>), indexed: <BTreeMap> }` written with `put`/`put_batch` under a `CasExpectation`, read with `get`/`query`, and (for events) appended with `append`/`tail`.

- Both SQL backends persist a record as the same logical row `(path, contents, content_type, kind, indexed JSON, version)`. `body` is opaque bytes; `indexed` is JSON; `version` is bumped `version = version + 1` in the same statement on both dialects (GROUNDING 2).
- CAS is identical: `Absent` → `INSERT … ON CONFLICT DO NOTHING`; `Version(v)` → `UPDATE … WHERE version = v`; `Any` → upsert. `VersionMismatch` surfaces identically.
- `query` filters touch **only** `indexed` (`indexed->>'k'` on PG, `json_extract(indexed,'$.k')` on libSQL), never the body, so identical `Filter`s over identical `indexed` projections produce identical result sets.
- The append plane assigns a monotonic `SeqNo` per path (`BIGSERIAL`+`RETURNING` on PG; `AUTOINCREMENT`+`last_insert_rowid()` on libSQL).

Therefore: **the same Rust code calling the same trait against both backends produces identical logical state.** This is exactly the migration-safety invariant (§3.7), and decomposition does nothing to weaken it — it multiplies one row into N rows under the same mount.

---

## 6. New native primitive

Only **one** new primitive: `put_batch`. This section gives signature, default impl, native Postgres impl, native libSQL impl, semantic-parity argument, capability advertisement, and the `ScopedFilesystem` wrapper.

### 6.0 Shared capability + operation + scoped-wrapper surface

**`Capability` (types.rs)** — append **one** bit (discriminant order only grows; `bit = 1 << (self as u32)`; unknown capability strings already decode as "missing"):

```rust
pub enum Capability {
    // … existing variants, unchanged order …
    Events,
    BatchPut,       // native atomic put_batch
}
```

Add it to `Capability::all()` in trailing order.

**`FilesystemOperation` (types.rs)** — append **one** variant for honest error attribution:

```rust
pub enum FilesystemOperation { /* … existing, incl. HeadSeq … */ PutBatch }
```
…with a `Display` arm `"put_batch"`. **Note:** `HeadSeq` already exists at `types.rs:41` (Display arm at `:61`, `operation_allowed` arm at `scoped.rs:431`, wrapper at `scoped.rs:221`) — do **not** re-add it; `PutBatch` is the only net-new operation.

**No new capability constructor.** A SQL mount that wants the native batch path declares `.with(Capability::BatchPut)` inline at its mount-descriptor site (the descriptors are written per mount in the consumer PRs). A named `sql_typical_hotpath()` constructor is deferred until ≥3 mount sites share the exact set — until then the inline `.with(Capability::BatchPut)` is shorter and self-documenting, with no extra name to learn. **Exception for the new stores:** the resources/turns/threads mounts must NOT declare `IndexVector`, so they do **not** start from `sql_typical_full()` (which bundles `IndexVector`). They derive from `database.capabilities().without(Capability::IndexVector)` (§7.9 P4), which already carries `BatchPut`+`MultiKey` from the backend after PR-2/PR-3 — see §7.9 P4 for the determinative pattern.

**`in_memory_full()` stays CAPPED at its current set for now (CodeRabbit review).** PR-4 still has the per-op-lock-retention audit open; the in-memory backend advertises `BatchPut` **only after** PR-4 confirms the default path holds its per-op lock across the whole batch. Until then `in_memory_full()` is unchanged and the in-memory N>1 atomicity is treated as unproven.

**Mount validator (catalog.rs)** — add the one bit to `NEW_AXES` **when its backends actually advertise it** (i.e. as PG/libSQL native overrides land in PR-2/PR-3; in-memory in PR-4):

```rust
const NEW_AXES: &[Capability] = &[
    Capability::Records, Capability::Query,
    Capability::IndexExact, Capability::IndexPrefix,
    Capability::IndexFts, Capability::IndexVector,
    Capability::Events,
    Capability::BatchPut,
];
```
The existing `declared.has(cap) && !backend.has(cap) → DescriptorOverclaims` loop handles the new bit by construction. Adding it to `NEW_AXES` before any backend advertises it is harmless (no descriptor declares it yet) but is sequenced with the override PRs to keep the validator honest.

**`ScopedFilesystem` wrapper (scoped.rs) — REQUIRED.** Consumers cannot reach `RootFilesystem` methods directly. Add a scope-relative wrapper that resolves+permission-checks each entry before delegating, and extend the **exhaustive** `operation_allowed` match (`scoped.rs:417`, no catch-all — omitting the arm will fail to compile):

```rust
// scoped.rs operation_allowed: add arm
FilesystemOperation::PutBatch => self.permissions.allows_write(),

// scoped.rs wrapper
impl<F: RootFilesystem> ScopedFilesystem<F> {
    pub async fn put_batch(&self, scope: &ResourceScope, puts: Vec<ScopedBatchPut>)
        -> Result<Vec<RecordVersion>, FilesystemError> {
        // resolve+permission-check EACH entry's ScopedPath -> VirtualPath,
        // assert all resolve to the SAME mount, then delegate to self.root.put_batch.
        // If the backend lacks BatchPut and N>1, return a typed Unsupported that
        // names the missing capability (never a silent non-atomic fallback).
    }
}
```
`ScopedBatchPut.path` is a scope-relative `ScopedPath`; it resolves to `VirtualPath` only at the scoped→root boundary. `RootFilesystem::put_batch` takes `VirtualPath`.

---

### 6.1 `put_batch`: atomic/efficient multi-put

**Consumers:** (1) thread message append (idempotency + thread `next_sequence` bump + message + seq index, mixed `Absent`+`Version` CAS, all-or-nothing); (2) the governor's per-account ledger updates + reservation create, committed together (§7).

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
// NB: begin() must receive the longest common DIRECTORY prefix of every path in
// the batch, not puts.first().path — StorageTxn::check_path rejects any put whose
// path does not start with the txn prefix (postgres.rs:951-956), so passing the
// first leaf path would make every sibling leg fail PathOutsideMount. This mirrors
// the thread store, which begins on the THREADS_PREFIX scoped path
// (filesystem_service.rs:331 builds the prefix; the begin() call is at :345).
let prefix = common_dir_prefix(puts.iter().map(|p| &p.path)).ok_or(/* Backend: empty put_batch */)?;
let mut txn = self.begin(&prefix).await?;       // Unsupported => caller takes its fallback / rejects
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
`RETURNING version` gives every assigned version in the same round-trip. Postgres advertises `BatchPut` and `TxnCapability::MultiKey`.

**Connection-pool bound (PR #5081 deadlock class) — a hard prerequisite, not a footnote.** A `put_batch` holds **one** pooled connection for the duration of `BEGIN…COMMIT`. The danger is concrete: **production builds ONE shared `deadpool_postgres::Pool`** (`input.rs` `open_postgres_pool_with_tls_options`) and shares it across **four** consumers — the filesystem (`factory.rs:3936`, `pool.clone()`), the trigger repository (`:3939`), a credential-keepalive leader-lock (`:3935`), and the event store (`:3950`, the final move). Its default size is **`DEFAULT_POSTGRES_POOL_MAX_SIZE = 2`** with a **`POOL_CHECKOUT_TIMEOUT = 30s`** (`ironclaw_reborn_event_store/src/lib.rs:55` and `:561`). `PostgresRootFilesystem::new(pool)` (`postgres.rs:39`) takes that shared handle — it is **not** a separate pool. So at the default size 2, two concurrent governor/thread `put_batch` commits check out both connections for their txn duration, and the next caller (a turn heartbeat, an event append) blocks up to 30s on `pool.get()` and then fails — which expires the runner lease and fails the turn. This is exactly the #5081 class. Bound it: (a) batches are **statically sized** by their consumer (thread append ≤ 4 puts; governor cascade ≤ 7), never unbounded; (b) document a hard cap `MAX_BATCH_PUTS = 64` and reject larger batches with a typed `Backend` error; (c) **PR-2 MUST raise the pool size (or give the filesystem its own pool) before the governor/threads migration steps** — sized for `(max_concurrent_model_calls × 1 batch-held-connection) + heartbeat + event-append + trigger-poll headroom`; a floor of 2 is a hard regression under any concurrency and the implementing PR states the chosen minimum and where it is set. No long-lived interactive handle is exposed.

**Native libSQL impl (libsql.rs) — raises libSQL to `MultiKey`:** `BEGIN IMMEDIATE … COMMIT` (the dialect gotcha — *not* deferred, so the write lock is taken up front; a deferred txn that upgrades mid-statement can hit `SQLITE_BUSY` after partial work and violate all-or-nothing). Per-put SQL is the libsql `?N`/`is_dir=0`/`strftime` variant. Because the native override provides genuine multi-statement atomicity, libSQL **also implements `begin`/`StorageTxn` over the same `BEGIN IMMEDIATE`** and advertises `TxnCapability::MultiKey` — this is what lets the governor use one commit path on both backends (it was deferred as "open question Q4" in the prior draft; the multi-lens review pulled it in as a prerequisite because it is what eliminates the libSQL admission-gate special case).

**Honest blast radius on libSQL (do NOT confuse with Postgres row-locking).** SQLite's `BEGIN IMMEDIATE` takes a **database-file-global write lock**, not a per-row lock. All Reborn stores share **one physical libSQL file** (one `Arc<libsql::Database>` — `factory.rs:2217` local-dev, `:3353` production — multiplexed across the `/resources`, `/turns`, `/threads`, `/events` mounts). So while a governor `put_batch` holds `BEGIN IMMEDIATE`, **every other writer on that file blocks** — a concurrent turn heartbeat or thread append waits up to `PRAGMA busy_timeout` (today `5000ms`, `libsql.rs:90`, sized for single-statement contention and worth revisiting for multi-statement holds) regardless of which rows it touches. This is materially different from Postgres, where two `put_batch`es on disjoint user paths proceed in parallel under row locks. The in-process `filesystem_record_lock` (`cas_snapshot.rs:357`) is per-path at the Rust layer, but on libSQL the SQLite write lock underneath is file-global. **For this deployment this is acceptable:** §7.8 collapses the governor hot path to ≤2 small per-user account records and batches commit in microseconds, so the file-global window is short and the contender pressure low. **But it is a real throughput ceiling** for a future high-concurrency or multi-tenant libSQL deployment, and is stated here rather than hidden behind "per-account-path" framing. The mitigation if it ever bites is the per-user libSQL shard/cell split already noted in §7.8 (one file per user → no cross-user file-lock contention).

**Version-readback (scoped precisely — it is NOT load-bearing for the governor).** libsql `put()` already returns the assigned version **arithmetically** for the two CAS modes the governor uses: `Absent ⇒ version 1`, `Version(v) ⇒ v+1` (`libsql.rs:199,237`; Postgres matches at `postgres.rs:1132,1167`). The governor's `put_batch` is entirely `Absent` (reservation insert) + `Version` (ledger CAS), so it needs **no `RETURNING` and no readback** — the versions are known arithmetically and the all-or-nothing commit is what matters. The only mode that needs a real readback is `CasExpectation::Any` (used by threads-style idempotency upserts, not the governor). For that case:
1. At store init, **probe once** whether the bundled libSQL build supports statement-level `RETURNING` inside `BEGIN IMMEDIATE` (run `INSERT … RETURNING version` against a scratch row in a txn, **then `ROLLBACK` the probe txn** so no scratch row persists — Gemini review). Cache the result.
2. If supported, use `… RETURNING version` like Postgres.
3. If not, after the `Any` `INSERT/UPDATE` do `SELECT version FROM root_filesystem_entries WHERE path=?1` **inside the same `BEGIN IMMEDIATE`** before `COMMIT`. Still atomic (same write lock), +1 read.

**Capability advertisement — decoupled so a probe failure never strands the governor.** libSQL advertises `BatchPut` + `MultiKey` as soon as the `BEGIN IMMEDIATE` override lands (PR-3); the **governor can promote to `Native` on this** because its CAS modes return versions arithmetically with no probe dependency. The `RETURNING`-probe result gates only the `Any`-mode exact-readback path: if neither `RETURNING` nor in-txn `SELECT` works (should not happen, but Fail Loud, Rule 12), the threads idempotency path that needs `Any` readback reconfigures to `ForceCas`, while the governor is unaffected. No assumption; verified at init.

**Semantic-parity argument:** N puts either all commit (N versions, each prior+1 or 1) or none commit and the first failing CAS surfaces `VersionMismatch{expected,found}`. Identical observable result on both dialects and on the default impl (which produces the same commit-or-rollback via `begin`). Version increment per path is `version+1` in SQL on both. A consumer cannot distinguish native from default except by latency.

**Contract gotcha (sequencing).** On libSQL **before** the §6.1 native override lands (PR-1/PR-2 window), a multi-key `put_batch` returns `Unsupported` because libsql has no `begin` override yet. So PR-1 contract tests for N>1 `put_batch` run **against Postgres and in-memory**, and the single-key (N==1) path runs everywhere; an explicit test asserts N>1 on libSQL returns a typed `Unsupported` (not a panic, not a silent partial write) in that window. The libSQL N>1 path goes green in PR-3 when the native override + probe land. This is stated, not hidden.

**Composite dispatcher:** override `put_batch` to verify **every** `BatchPut.path` resolves to the **same** mount (longest-prefix); else `PathOutsideMount`, nothing written (mirrors `StorageTxn` prefix scoping). Then delegate.

---

## 7. Hot-store decomposition — RESOURCE GOVERNOR (top priority)

### 7.1 Before

One process-global `/resources/snapshot.json` under `ResourceScope::system()` holding four `HashMap`s + reservations + period anchors. `update_snapshot` (`cas_snapshot.rs:177`) does full read-modify-write CAS, serialized in-process by one path-keyed `filesystem_record_lock` (`cas_snapshot.rs:357`, held across get→put so there is no intra-process TOCTOU window), cross-process by blob CAS with **no retry** — the loser gets `"cross-process CAS contention"` (`cas_snapshot.rs:204`) and the reserve fails. Two agents in **unrelated tenants** contend on this one row. `filesystem_store.rs:62-65` is explicit: resources is process-global; per-tenant accounting is a "future capability." **This decomposition introduces per-account row separation where today there is one system() blob — it is therefore also a (small) scoping change, not pure granularity.**

### 7.2 After — record schema

| Family | VirtualPath | `RecordKind` | `body` (JSON) | `indexed` |
|---|---|---|---|---|
| Account ledger | `/resources/accounts/{account_seg}.json` | `resource_account` | `{ schema_version, account, limits, reserved: ResourceTally, usage: ResourceTally, period_end_at_anchor }` (the full multi-dimensional tally is the single source of truth) | `{ "kind_tag": Text, "tenant": Text, "owner_user": Text, "status": Text }` — **query/routing projections only; never a counter the hot path mutates in isolation** |
| Reservation | `/resources/reservations/{reservation_id}.json` | `resource_reservation` | `ReservationRecord { reservation, accounts, tally, status, actual }` | `{ "account_seg": Text(owner), "status": Text("pending"\|"active"\|"reconciled"\|"released") }` |

The four `HashMap`s collapse into **one record per account**; `reserved`/`usage` (already per-account sums) become the account record's `body` fields. **All eight tally dimensions live in `body` as one `ResourceTally`** and are updated together by `put_batch` writing the whole `Entry`. The `indexed` projection carries only query/routing keys (`tenant`, `owner_user`, `kind_tag`, `status`) — there is **no per-counter `indexed` I64 that a native op mutates without rewriting `body`**, so body and indexed cannot diverge (this closes the dual-truth hazard from the earlier draft). Human-readable identity stays in `body.account` and `indexed.tenant`/`indexed.kind_tag` for query/admin.

#### `{account_seg}` — collision-resistant key

`ResourceAccount::Display` renders absent slots as the literal `_` and is **not collision-free** — a real id equal to `_` collides two distinct accounts; ids have **no verified char validation** in the resources crate, so `/`/`..`/empty/control chars would mis-map or be rejected by `VirtualPath::new`. **Do not use `Display` as a storage key.** Instead:

```
account_seg = hex(sha256(canonical_json(&account)))   // fixed-length, path-safe, collision-resistant
```
`canonical_json` is a deterministic, field-ordered serialization of the `ResourceAccount` enum (variant tag + all id fields). A 256-bit hash is **collision-resistant** (not literally injective — a hash is non-reversible and has a vanishing but non-zero theoretical collision probability; we rely on collision-resistance, which is the standard guarantee and overwhelmingly sufficient for an account-keyspace of this size). The human-readable identity stays in `body.account` and `indexed.tenant`/`indexed.kind_tag` for query/admin. **Requirement:** add a `canonical_json` impl + a unit test asserting two distinct accounts (including the `_`-collision case and an id containing `/`) hash to distinct segs.

### 7.3 After — reserve algorithm (the global-contention fix)

`cascade(scope)` returns 2–6 accounts ordered **shallow→deep** (tenant first; `lib.rs:222`, min 2 = tenant+user). Two-phase, ordered, **one commit path on both backends**:

```
reserve(scope, estimate, reservation_id):
  accounts  = cascade(scope)                  // ordered tenant→…→thread
  requested = ResourceTally::from_estimate(estimate)

  // PHASE 1 — read+check each limited account ledger (no writes)
  loaded = []
  for acc in accounts (shallow→deep):
     (ledger, version) = get(/resources/accounts/{seg(acc)}) or (fresh_ledger, Absent)
     advance_period_if_rolled_over(ledger, now)        // §7.5 — overflow-safe, MUST be persisted
     evaluate_cascade_for_account(acc, ledger.limits, ledger.usage, ledger.reserved, requested)?  // Deny/Approval short-circuit, UNCHANGED business logic, reads body tally
     loaded.push((acc, ledger, version))

  // PHASE 2 — commit atomically across the touched accounts (SAME on both backends)
  puts = loaded
           .filter(|(acc,ledger,_)| !ledger.limits.is_empty())          // §7.8: only write limited ledgers
           .map(|(acc,ledger,ver)| BatchPut{
              path: /resources/accounts/{seg(acc)},
              entry: ledger.with_reserved_added(requested).into_entry(), // whole Entry: body tally + indexed projections
              cas: ver.map(Version).unwrap_or(Absent) })
        ++ [ BatchPut{ path:/resources/reservations/{id}, entry: reservation(active), cas: Absent } ]
  put_batch(puts)?            // all-or-nothing; VersionMismatch on any leg ⇒ whole batch rolls back ⇒ retry reserve (bounded)
```

**The win:** two reserves in different tenants now CAS **disjoint** account paths (`sha256(tenant-A…)` vs `sha256(tenant-B…)`) and **never contend** at the Rust `filesystem_record_lock` layer (taken per-account-path, not globally) **and on Postgres** (row-level locks). **On libSQL there is an additional file-global write-lock window during the `put_batch` commit** — see the §6.1 "Honest blast radius on libSQL" note; for this single-tenant low-concurrency deployment that window is short (≤2 small records, microsecond commit) and acceptable, but it is not "zero contention" the way Postgres is. Same-tenant reserves serialize only on the *shared* shallow ledgers — the correct, minimal contention surface, because they genuinely share that budget. `cascade`'s shallow→deep order doubles as **deadlock-free lock ordering** for the `put_batch` (row/write locks taken in a consistent order on both backends). `evaluate_cascade_for_account` is **untouched** (Rule 3).

`reconcile`/`release` are the symmetric inverse keyed off `record.accounts`: decrement `reserved`, accrue `usage` (reconcile), via the **same `put_batch`**, and flip the reservation status. **Lock-order note:** reconcile/release build their `puts` vector in the **same shallow→deep account order** as reserve (then the reservation record last), so a concurrent reserve and reconcile take row locks in the same order and cannot deadlock. This ordering is explicit, not "symmetric inverse" left to interpretation.

### 7.4 Multi-account, multi-dimensional atomicity — one mechanism, both backends

**The requirement, stated plainly:** admission is all-or-nothing across the cascade (`lib.rs:218-222`: "succeeds only if every account remains within its limit") **and** across all 8 `ResourceTally` dimensions. The earlier draft tried to satisfy this with a per-dimension conditional `adjust_indexed` on libSQL plus an "admission gate" — which is wrong, because N sequential single-dimension guards are not atomic and two concurrent reserves can interleave to over-admit (review C1), and because the idempotency it needed leaned on a non-existent sweeper (review C2).

**Resolution: `put_batch` is the atomic gate on both backends.** Phase-1 reads each ledger's `version`; Phase-2 commits all account-delta `Version` CAS-puts + the reservation `Absent` insert in one `put_batch`:

- **Postgres (`MultiKey`):** one `BEGIN…COMMIT`. Any concurrent reserve that touched a shared (tenant/user) ledger first bumped its version, failing this batch's `Version` CAS on that leg → whole batch rolls back → retry. The full tally (all 8 dimensions) is part of the committed `Entry`, so there is no per-dimension interleaving window. Two reserves sharing a ledger are **serialized by that ledger's version**.
- **libSQL (`MultiKey` via `BEGIN IMMEDIATE`, §6.1):** the **identical** `put_batch` call. The write lock is held across the whole batch; the version-CAS legs serialize exactly as on Postgres. No admission-gate, no `applied_reservations` set, no sweeper. The two backends run the same Rust, satisfying the migration-safety invariant by construction.

Because the whole `Entry` (full tally) commits atomically, **the all-or-nothing, multi-dimensional limit invariant holds on both backends with no per-backend governor code.** Goal §2.1 ("no caller forks") is satisfied — there is no `if MultiKey / else admission` branch.

**Retry posture:** a bounded retry loop, `RESOURCE_CAS_RETRIES = 16` with jittered backoff (matching turn-state's posture), replacing today's no-retry hard failure. Exhausting it surfaces a typed `Unavailable`, not corruption. Because `put_batch` is all-or-nothing, a lost CAS leaves **nothing** partially written, so a retry simply re-reads fresh versions and re-commits — there is no compensation path to get wrong and no leaked partial reservation for a sweeper to chase.

### 7.5 Period rollover must be persisted, overflow-safe

`advance_period_if_rolled_over` mutates `ledger.usage`/`period_end_at_anchor` in memory during the Phase-1 read, using a **single centralized, overflow-safe helper** (Gemini review): extremely large windows/anchors must **saturate** (`saturating_add`/`saturating_sub`, clamp to a far-past/far-future sentinel) rather than wrap to "now" and incorrectly trim or re-roll a period. Centralizing it in one shared function prevents behavioral drift between the PG and libSQL paths. In the per-record model this in-memory mutation **must be committed via the Phase-2 write even when the reserve check is otherwise non-mutating** (a check that denies, or a pure rollover) — otherwise rollover silently never persists. Concretely: when Phase-2 runs (an approved reserve, a zero-`requested` check that still commits, or a pure rollover), any account whose anchor advanced in Phase-1 is included in the `put_batch` with its rolled-over `usage`+`anchor`, CAS'd by its read version. A rollover write that loses its CAS simply retries (the next read sees the advanced anchor). Because `put_batch` is all-or-nothing, a rollover and a reserve in the same batch either both land or both retry — they cannot half-apply.

**Denied-reserve posture (resolves the §7.3 early-exit interaction):** §7.3's Phase-1 short-circuits on the first denying account (`evaluate_cascade_for_account(...)?`), so a *denied* reserve does not reach Phase-2 and does not persist that call's in-memory rollover. This is **safe and intentional, not a lost update**: `advance_period_if_rolled_over` is a deterministic function of the stored anchor + current time (§7.5's overflow-safe helper), so every subsequent read recomputes the identical rolled-over value, and the advance is persisted by the next reserve/reconcile/release that *does* reach Phase-2. The posture is **best-effort persist on the next committing operation**, with correctness guaranteed by deterministic recomputation — never a stale check, because the in-memory advance is applied before every limit evaluation. (If a deployment ever needs the rollover persisted even on a sustained deny streak, the cheap option is a rollover-only `put_batch` issued before returning the deny; not needed for correctness, so deferred.)

### 7.6 Indexes

```rust
ensure_index("/resources/accounts",     IndexSpec{ name:"acct_by_tenant",  keys:["tenant"],            kind: Exact });
ensure_index("/resources/reservations", IndexSpec{ name:"resv_by_status",  keys:["status"],            kind: Exact });
ensure_index("/resources/reservations", IndexSpec{ name:"resv_by_account", keys:["account_seg","status"], kind: Exact });
```
`resv_by_status` lets a `query(reservations, Eq{status:"pending"|"active"})` enumerate live reservations (e.g. for an operator reconciliation report) instead of a full-blob scan. Exact only; no FTS/Vector ⇒ `sql_typical()` suffices ⇒ mount validator cannot reject. Composite `["account_seg","status"]` is a multi-expression index on both SQL backends.

**Prefix-partial indexes (performance — multi-lens review P2).** `ensure_index` for `Exact`/`Prefix` today emits a **global** expression index `((indexed->>'key'))` with no path predicate (`postgres.rs:254-259`), so a selective `query` under one prefix either full-scans the path range or cross-pulls every row sharing that indexed value before the path filter narrows it. The FTS path already does the right thing — a **partial** index scoped by `WHERE path = … OR path LIKE '…/%'` (`postgres.rs:292-298`). This design requires extending `ensure_index` to emit **partial expression indexes** with the same path-prefix predicate for `Exact`/`Prefix`, so the planner gets a combined (prefix-bounded, indexed-key) structure. The CI parity suite (§9.4) adds an `EXPLAIN ANALYZE` assertion per declared index that the intended index is actually chosen, so a planner regression fails CI rather than silently degrading at scale.

### 7.7 In-place migration (blob → per-account records)

Forward backfill, online in `DualWrite`, idempotent, value-preserving, **`CasExpectation::Absent` skip-if-present** (so it never clobbers a live native write):

```
migrate_resources_to_native():
  (snapshot,_) = get(/resources/snapshot.json)   // None on fresh deploy ⇒ return
  for acc in union(snapshot.state.limits.keys, reserved.keys, usage.keys, anchors.keys):
     ledger = ResourceAccountRecord{ schema_version:2, account:acc, limits:limits[acc],
              reserved:reserved[acc].unwrap_or_default(), usage:usage[acc].unwrap_or_default(),
              period_end_at_anchor:anchors[acc] }
     put(/resources/accounts/{seg(acc)}, ledger.into_entry(/* body tally + indexed projections */), Absent)
  for (id,rec) in snapshot.state.reservations:
     put(/resources/reservations/{id}, rec.into_entry(), Absent)
```
The legacy blob stays (never-delete). Sequence: flip `DualWrite` → backfill `Absent` → `verify_parity` until clean → flip `Native`.

### 7.8 Single-tenant / per-user limits: limitless ledgers leave the hot path

**General principle:** maintain a hot per-account `reserved`/`usage` ledger **only where there is a limit to enforce against it.** A cascade account with no limit needs no hot write — its only purpose would be aggregate reporting, which is a *derived read*, not a hot write.

**This deployment.** The hosted profile is `RebornProfile::HostedSingleTenant` — a **single tenant** (`reborn-cli`) with **many users** — and resource **limits are per-user, with no tenant-wide cap.** Therefore:

- The **tenant** and `system()` levels are **limitless** → no guard, no hot ledger needed → they are **dropped from the reserve/reconcile/release write path** (Phase 2 `put_batch`es only ledgers whose `limits` is non-empty — the `.filter(!limits.is_empty())` in §7.3).
- The **broadest account that carries a limit is the `user`** → the per-user ledger becomes the broadest *hot* record.

**Consequence — full per-user isolation:**

| | tenant ledger on hot path (generic design) | per-user limits, tenant dropped (this deployment) |
|---|---|---|
| two **different** users reserving | serialize on the shared tenant ledger | **disjoint ledger paths → zero contention** |
| same user, concurrent reserves | serialize on tenant + user | serialize on **their own** user ledger (correct — shared budget) |

So the governor goes from one process-global hot row all the way to **fully per-user, zero cross-user contention** — the strict best case, and it matches the actual budget model. The multi-tenant headline *"different **tenants** never contend"* gives **nothing** on a single-tenant instance; dropping the limitless tenant ledger is what makes the fix actually land for **one tenant, many users.**

**Tenant/global totals, if ever needed for reporting/billing,** are a **derived aggregate** — `query`-sum the per-user ledgers on demand, or a periodic rollup — **never a hot counter.** Eventual consistency on the aggregate is fine for reporting and keeps the write path per-user.

**Implementation:** make the cascade-write **limit-aware**: in Phase 2 (and reconcile/release), iterate the cascade but `put_batch` only the accounts with non-empty `limits` (here: the `user` level and below). `evaluate_cascade_for_account` already passes trivially for limitless accounts, so the business logic is **untouched** (Rule 3) — we simply stop *writing* a ledger that has nothing to enforce. This also preserves per-user independence as the precondition for a future per-user shard/cell split (the single-node-libSQL scale-out path). **If a tenant-wide cap is ever introduced,** the tenant ledger re-enters the hot path; because the commit is already an atomic `put_batch` across whatever limited ledgers the cascade touches, no protocol change is needed — the tenant ledger simply becomes another leg of the same batch.

### 7.9 Performance & operational characteristics (multi-lens review P1/P3/P4)

The decomposition trades one hot blob for many small rows. The performance consequences are real and must be planned, not assumed away:

- **JSONB/body write churn & VACUUM (P1).** Every `put_batch` leg rewrites a row's `contents` (and bumps `version`), producing an MVCC dead tuple plus index-entry churn on the touched account/run rows. At reserve+reconcile-per-model-call rates this concentrates on the hottest rows. Mitigations the implementing PRs must carry: (a) **aggressive per-table autovacuum** on `root_filesystem_entries` (and the events table) as a deployment prerequisite, with **named starting values** to tune from — `autovacuum_vacuum_scale_factor = 0.02`, `autovacuum_vacuum_threshold = 50`, `autovacuum_vacuum_cost_delay = 2ms` — not just "aggressive"; (b) the partial expression indexes of §7.6 keep index size proportional to the prefix, reducing per-update index work; (c) **interaction with the shared pool (§6.1):** autovacuum and the hot write path draw on the **same** size-bounded Postgres pool, so the PR-2 pool-size increase must leave headroom for the vacuum worker as well — an under-sized pool turns VACUUM into a write-path stall; (d) the structural alternative — a typed-column projection (real `BIGINT` columns enabling Postgres HOT updates, zero index churn when no indexed column changes) — is the relational-sidecar lever **deferred in §12 Q3**; it is the escalation path if these mitigations prove insufficient under production load.
- **Row-count growth & event-log retention (P3).** Turn decomposition multiplies one blob into one run row per turn plus N transition UPDATEs; the event-log moves from an embedded `Vec` to append-only rows in `root_filesystem_events`. **Retention decision (must be made explicit, not implied):** the "LLM-data-never-deleted" invariant (CLAUDE.md) means run records and events are **retained, never purged** — so `root_filesystem_events` grows monotonically by design, and `event_retention_floor`/`head_seq` bound *reads/replay*, not storage. If a future deployment needs bounded storage, that is a separate **tiering** decision (cold-storage offload keeping the path/index row), never an in-place delete. The implementing PR states which of these applies and sizes the events table accordingly.
- **Vector search honesty (P4).** Vector queries are still **brute-force Rust cosine over the prefix** (`postgres.rs:157-163`), not an ANN index. This design adds no vector indexes and **must not advertise `IndexVector`** on the resources/turns/threads mounts (invariant §3.4: capabilities are honest or you don't mount). **Mechanism (the backend already over-advertises today):** `LibSqlRootFilesystem::capabilities()` includes `Capability::IndexVector` (`libsql.rs:130-140`; Postgres `postgres.rs:163`), and a mount descriptor built from `database.capabilities()` (`factory.rs:2297`) inherits it. So the new mounts cannot just "not declare" it — they must **filter it out** using the **existing** `BackendCapabilities::without(Capability)` helper (`types.rs:404`, the inverse of `.with(...)`): build each resources/turns/threads descriptor as `database.capabilities().without(Capability::IndexVector)`. (No new helper to write — it already exists; PR-1 just uses it.) pgvector adoption is out of scope and tracked separately (§12 Q5); until then these mounts declare only the Exact/FTS capabilities they actually serve, and the mount validator (§3.4) keeps them honest.

---

## 8. Hot-store decomposition — TURN-STATE + EVENT-LOG, THREADS

### 8.0 The turn-scope premise correction

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
- List my runs: `query(/turns/runs, Eq{scope_seg}, Page)` (Exact index, prefix-partial per §7.6).
- List active: `query(/turns/runs, And[Eq{scope_seg}, Eq{status:"running"}], Page)`.
- Recency sidebar: `query` page, sort by projected `updated_at` in Rust (no ORDER BY in `Filter`, status quo).
- Spawn-tree descendants (`children_of`, `reserve_tree_descendants`, `lifecycle.rs:530-567`): `query(/turns/runs, Eq{root_run}, Page)`.

### 8.2 Event-log on the append plane

Events un-embed onto the native `append`/`tail`/`tail_bounded`/`head_seq` plane at `/turns/events/log`:
- Append: after the run-record CAS succeeds, `append(/turns/events/log, serde_json::to_vec(&event)?)` → `SeqNo`. PG `INSERT … RETURNING id`; libSQL `INSERT` + `last_insert_rowid()`.
- Read after cursor: `tail(from)` / `tail_bounded(from, max)` — `WHERE id > ? ORDER BY id ASC LIMIT ?` on both. Replaces in-Rust `project_turn_events()`.
- Floor/retention: `event_retention_floor` → `/turns/meta/event-floor.json` (CAS-by-version) + `head_seq` for O(1) high-water mark. (Retention bounds reads/replay, not storage — see §7.9 P3.)
- `EventCursor(u64)` (`events.rs:20`) maps directly onto `SeqNo(u64)`.

**`head_seq` override verification.** The retention floor relies on `head_seq` being O(1) `MAX(id)`. **Verified present** on both SQL backends (postgres.rs:678, libsql.rs:990). The default impl materializes the gap, so this is load-bearing and confirmed, not assumed.

### 8.3 Run-transition + event-append atomicity (sharpened)

The split makes the run-record CAS and the event append two operations. **Events cannot join a `StorageTxn`** (it has put/get/delete only; events use the append plane). Resolution:

- **Run record is the single source of truth; the event log is a downstream idempotent projection.** Order: **CAS the run record first (authoritative), then `append` the event.**
- **Idempotency key:** the event payload embeds `(run_id, transition_version)` where `transition_version` is the run's `RecordVersion` after the transition (monotonic per run ⇒ unique stable key). A recovery appender tails and skips if `(run_id, version)` already present.
- **Downgrade the claim — at-least-once with idempotent recovery, NOT exactly-once.** `append` has no CAS (GROUNDING 1 #11); the dedup tail-scan is non-atomic with a concurrent appender. So: **duplicate events are tolerated** because an event is a non-corrupting projection of the authoritative run record. The recovery scan is **bounded** (tail at most `RECOVERY_SCAN_WINDOW` records back from `head_seq`); beyond that window we accept a possible duplicate rather than scan unbounded. **Consumer requirement (must be verified before Native promotion):** every event consumer MUST be idempotent on `(run_id, transition_version)` — a consumer that re-reads the authoritative run record on receipt is idempotent by construction; a consumer that fires a side effect (SSE push, audit webhook) per raw event payload would double-fire on a duplicate and MUST dedup on the key. The event-log Native cutover gate (§10) enumerates consumers and asserts each is idempotent.
- **Crash-loss honesty: non-terminal events are genuinely lossy on crash-between-CAS-and-append.** A crash after the run CAS but before the append drops *that* lifecycle event. For **terminal** transitions the event is re-derivable from the run's terminal status on recovery. For **intermediate/non-terminal** transitions (heartbeat, `blocked_auth`) the event is **not** re-derivable and is genuinely lost — stated plainly (Rule 12). This is strictly better than today (a torn blob write loses run state *and* event), and the run record remains correct. Where a deployment cannot tolerate non-terminal event loss, the run+event pair can use `begin`/`put_batch` on the run-record write and accept the event append as the post-commit projection — but the append itself still cannot be transactional, so the residual non-terminal-loss window is inherent to the append plane and is documented as such.

### 8.4 Turn-state indexes + migration

```rust
ensure_index("/turns/runs", IndexSpec{ name:"runs_by_scope",  keys:["scope_seg"],          kind: Exact });
ensure_index("/turns/runs", IndexSpec{ name:"runs_by_status", keys:["scope_seg","status"], kind: Exact });
ensure_index("/turns/runs", IndexSpec{ name:"runs_by_root",   keys:["root_run"],           kind: Exact });
```
(All prefix-partial per §7.6.)

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

1. **Adopt `put_batch`** for the message append, replacing the hand-rolled `begin`/`StorageTxn` 4-op txn + `TransactionalMessageWrite::Unsupported` enum. The store **requires** atomicity, so it gates on the `BatchPut` capability (or `begin` availability). **If neither is available, it rejects the atomic write — it does NOT fall back to a non-atomic `reserve_sequence` + sequential-CAS path** (CodeRabbit review: a sequential-CAS fallback cannot satisfy the stated all-or-nothing requirement and would reintroduce torn multi-op writes on exactly the backends that lack atomicity). The only escape from the transactional path is the explicit operational `ForceCas` opt-out below, which a deployment sets knowingly. After §6.1 lands, **both** SQL backends advertise `BatchPut`, so the reject path is reachable only on a misconfigured/byte-only mount. Public API unchanged.
2. **Native query indexes** replace bespoke index files (all prefix-partial per §7.6):
   ```rust
   ensure_index("/threads", IndexSpec{ name:"msg_by_seq", keys:["thread_seg","sequence"], kind: Exact });
   ensure_index("/threads", IndexSpec{ name:"msg_by_run", keys:["run_id"],                kind: Exact });
   ensure_index("/threads", IndexSpec{ name:"threads_by_scope", keys:["scope_seg"],       kind: Exact });
   ```
   Message `indexed` gains `{ "thread_seg": Text, "sequence": I64, "run_id": Text }`; thread `indexed` gains `{ "scope_seg": Text, "updated_at": I64, "owner_user": Text }`. `list_threads_for_scope` becomes `query(/threads, Eq{scope_seg})` + Rust sort by `updated_at`, retiring the N+1 `list_dir`+per-file `get` (`filesystem_service.rs:1992-2048`). **Ownership (Gemini review):** the `query` runs through `ScopedFilesystem`, which bounds results to the caller's scope prefix; the service additionally asserts the authenticated user matches each record's `indexed.owner_user` before exposing it, so the cheaper `query` path cannot widen cross-user/cross-tenant visibility relative to the per-file `get` path it replaces. Range "messages [lo,hi]" → `query(Range{key:"sequence", lo:I64, hi:I64})`.
3. **`ForceCas` escape hatch:** an operational flag (`ThreadWritePath { TxnOrCas (default), ForceCas }`) to disable the transactional path instantly if a backend's `begin`/`put_batch` misbehaves. This is the **single, explicit** opt-out into non-atomic writes (used only as an incident lever, knowingly), distinct from a silent default fallback. Zero data migration, reversible.

**Range parity (corrected attribution):** libSQL Range **does** apply a JSON-type guard (`libsql.rs:1595`, PR #3659) — the first angle's "no guard" claim is stale. Because we always write `sequence` as `IndexValue::I64`, the stored JSON is numeric on both backends. A dedicated boundary test (9→10→100) on libSQL specifically asserts numeric, not lexicographic, range correctness.

**Threads migration:** no row rewrite — message records already carry the data; only `indexed` projections must be present. A one-time `put(CasExpectation::Version)` re-projection pass adds `indexed` to pre-existing messages without touching `body` (skip if already populated; value-preserving; parity-safe). `ensure_index`'s `CREATE INDEX` covers existing rows on both backends. Bespoke index files keep being written until the native indexes are backfilled and `verify_parity` confirms `query` returns the same set; then a follow-up change stops writing them.

---

## 9. Dual-backend + migration-safety

### 9.1 Both backends store identical logical records

Every record above is `(VirtualPath, kind, body-JSON, indexed-JSON)` written by the **same Rust code through the same trait**. Per §5, both SQL backends translate the same `put`/`put_batch`/`query`/`append`/`ensure_index` calls into dialect SQL that yields byte-identical `contents`/`indexed`/`kind` and equality-comparable `version`. The only new parity surfaces vs today are (a) the set of declared indexes (Exact-only, contract-covered on both) and (b) the one native primitive `put_batch`, whose parity is argued in §6.1 and locked by the native-vs-default harness (§9.5). Because the governor now uses `put_batch` on **both** backends (no admission-gate fork), there is no per-backend governor logic to drift.

### 9.2 libSQL↔Postgres stays a COPY

Shape migration (blob→records) produces records that the backend copy tool transfers verbatim: `query(prefix, Filter::All, page)` paging `/turns`/`/resources`/`/threads`, `put(dest, entry, Any)` each; copy the event plane with `tail`/`append`. No transform — there is no schema embedded in a consumer record beyond JSON + indexed projection, both of which round-trip identically. Version numbers are not preserved bit-for-bit (opaque, equality-only); the destination assigns its own monotonic versions and consumers re-read before their next CAS.

### 9.3 The DualWrite divergence window (stated)

In `DualWrite`, the snapshot write and the native write are **two separate non-transactional CAS operations**. A crash between them leaves native stale relative to the authoritative snapshot until the next write. This is **expected and tolerated**, not atomic: the snapshot stays authoritative through `DualWrite`, and `verify_parity` (the promotion gate) asserts convergence **at quiescence**, tolerating in-flight skew. **Period-rollover note:** a rollover anchor that lands in the snapshot but not (yet) in the native ledger is exactly this skew; it is **not** self-healing on its own, so the §10 promotion gate **rejects `Native` if `verify_parity` is not clean** — the stale anchor cannot reach `Native` and cause a double-rollover because the gate blocks promotion until the next write reconciles it. This is an explicit hard gate, not an assumption that skew is harmless.

### 9.4 The migration-safety invariant + CI parity test (required deliverable)

The invariant is §3.7. The CI test that asserts it:

> **`store_parity` test (per store, both backends):** drive an identical sequence of store operations (the store's full public API surface — reserve/reconcile/release; submit/heartbeat/complete/fail; thread append/range/list) against a libSQL-backed and a Postgres-backed instance. After each op, dump every record under the store's prefix (`query(prefix, All)` → multiset of `{path, kind, body, indexed}`) **and** the append-plane payloads (`tail` from 0). Assert the two backends' multisets and payload sequences are **equal** (versions excluded). Any divergence fails CI. The suite also runs an `EXPLAIN ANALYZE` per declared index (§7.6) asserting the intended index is chosen.

This runs under `--features integration` (Postgres) and is the executable form of the invariant.

### 9.5 Native-vs-default equivalence harness

To prove a native override is observably indistinguishable from its default fallback (the heart of the migration-safety claim for `put_batch`): run the entire primitive suite twice against each SQL backend — once native, once forced through the default impl. **The default path is forced by a non-overriding newtype wrapper** around the backend that does **not** override `put_batch` (so Rust dispatches to the trait default) — **NOT** by masking capability bits (bit-masking does not change method dispatch). Dump `(path, contents, indexed, version-equality)` after each and assert identical final state.

### 9.6 Contract-test reality

The db contract tests are **hand-duplicated per backend** (`libsql_root_filesystem_*` vs `postgres_root_filesystem_*` in `db_root_filesystem_contract.rs`), **not** a single parameterized harness. Building the parameterized matrix (each new contract case × both backends × native/default) is **net-new test infrastructure**, sized into PR-1, not a free extension.

---

## 10. Test & rollout

### 10.1 Per-store flag + reverse job

Each store: one enum config read **once at construction**, frozen into the store struct, advanced independently:

```rust
pub enum StoreWriteMode { #[default] Snapshot, DualWrite, Native }   // resources, turns (events ride the turns flag)
pub enum ThreadWritePath { #[default] TxnOrCas, ForceCas }           // threads (no snapshot)
```
Env keys: `IRONCLAW_RESOURCES_WRITE_MODE`, `IRONCLAW_TURNS_WRITE_MODE`, `IRONCLAW_THREADS_WRITEPATH`. (Named `*_WRITE_MODE`, not `*_MODEL` — `StoreModel` would collide with the repo's LLM-routing `*Model` naming family, e.g. `ModelSelectionMode`/`ModelSlot`/`RebornModelRoutesState`; operational modes in this repo use `*Mode`. Local-patterns review.)

**Event-log has NO independent flag — it rides `IRONCLAW_TURNS_WRITE_MODE` (maintainability review).** Events are physically embedded in the turn snapshot blob (`events: Vec<TurnLifecycleEvent>` inside the snapshot, §8.1), so the event-log plane cannot migrate independently of the turn plane: the forbidden combination `TURNS=Native, EVENTLOG=DualWrite` would write events into an orphaned blob the turn path has stopped maintaining — a split-brain. Folding event-log into the turns flag makes that state unrepresentable. The "longer soak before event-log `Native`" guidance (§10.5) becomes an **operational waiting period inside `DualWrite`** before flipping the single turns flag to `Native` — natural, since the turns `verify_parity` gate must clear first. The run plane reaching `Native` and the event plane reaching `Native` happen at the same flip, with the run-plane parity gate (which includes event-replay equivalence) as the guard.

State machine: `Snapshot ⇄ DualWrite` is free/instant (snapshot stays authoritative through `DualWrite`). `DualWrite → Native` is the only gated step (backfill + `verify_parity` clean — and §9.3's hard reject if parity is dirty). `Native → DualWrite` requires `rematerialize_snapshot()` (reads native via `query`, writes the legacy blob with `Any`) — a first-class rollout artifact, not an afterthought.

### 10.2 Contract tests (both backends)

Extend `db_root_filesystem_contract.rs` (net-new parameterization, §9.6):
- `put_batch` all-or-nothing: 4 mixed-CAS puts all land with correct versions; a stale `Version` leg ⇒ `VersionMismatch` and **none** of the others written. (N>1 runs PG+in-memory in PR-1; libSQL in PR-3.)
- `put_batch` N>1 on a non-`MultiKey` mount ⇒ typed `Unsupported` naming the missing `BatchPut` capability, **nothing written, no panic** (the PR-1→PR-2 libSQL window and any byte-only mount).
- `put_batch` cross-mount rejection ⇒ `PathOutsideMount`, nothing written.
- `put_batch` version-readback parity (libSQL): the probe path (`RETURNING` vs in-txn `SELECT`) returns the same assigned versions as Postgres; the probe txn is rolled back and leaves no scratch row.
- Native-vs-default equivalence (§9.5).

### 10.3 Consumer-level tests (test-through-the-caller, CLAUDE.md)

- **Governor:** (a) two reserves in disjoint tenants succeed concurrently touching **disjoint** account paths (instrumented filesystem asserts no shared path CAS'd); (b) two same-tenant reserves serialize on the shared ledger via `put_batch` version-CAS; (c) reserve→reconcile→release returns `reserved` to baseline + accrues `usage`, **byte-identical ledger JSON on both backends**; (d) **crash-injected mid-cascade** reserve leaves **nothing** partially written (all-or-nothing `put_batch`), so a retry simply re-commits with no double-charge and no leaked partial reservation; (e) **multi-dimensional admission**: a reserve that would exceed *any one* of the 8 tally dimensions is denied atomically (no dimension partially admitted), and two concurrent same-user reserves at a multi-dimensional limit never over-admit on **any** dimension — the regression test for review C1; (f) the libSQL `put_batch` path and the Postgres `put_batch` path produce the same final ledger state (the parity floor test).
- **Turns/event-log:** (a) N concurrent transitions on distinct runs in one scope no longer contend; (b) crash between run-`put` and event-`append` → run state intact, terminal event re-derived on recovery, no run-state loss; **non-terminal event loss is asserted as tolerated** (the test documents it, not "fixed"); (c) `tail(cursor)` ordering matches the old snapshot projection; (d) `head_seq` returns `MAX(seq)` without materializing the gap (assert query count); (e) **every enumerated event consumer is idempotent on `(run_id, transition_version)`** — duplicate-event injection produces no double side effect (the §8.3 Native-cutover gate).
- **Threads:** (a) composite `msg_by_seq` Range across the 9→10→100 boundary on **libSQL specifically**; (b) `TxnOrCas` and `ForceCas` produce identical final state; (c) concurrent appends produce gapless unique sequences with no lost messages on both backends; (d) idempotency record blocks duplicates identically; (e) on a mount lacking `BatchPut`/`begin`, the message append **rejects** rather than writing a torn sequence (CodeRabbit review); (f) `query`-based `list_threads_for_scope` exposes no record whose `owner_user` mismatches the caller (Gemini ownership review).

### 10.4 Fault/latency-injection harness

A `FaultInjectingFilesystem` decorator (itself a `RootFilesystem`, composes per invariant 1):
- **CAS-loss:** force `VersionMismatch` on the Nth `put`/`put_batch` leg ⇒ assert reserve/transition retries and converges within bound (governor 16, turns 32, threads 8); exceeding ⇒ typed `Unavailable`, not corruption.
- **Latency:** per-call delay ⇒ assert the governor's per-account decomposition yields higher cross-user throughput than the snapshot baseline (50 concurrent reserves across 50 users must not serialize: wall-clock ≤ k·single-reserve-latency, vs ≈50·latency for the blob). This is the regression test for the *value* of the fix.
- **Crash:** panic mid-`put_batch` ⇒ restart ⇒ all-or-nothing means nothing partial landed; the next reserve re-commits cleanly, no double-charge, no leaked reservation.
- **Unsupported:** force `begin`/`put_batch` → `Unsupported` ⇒ threads **reject** and governor refuses `Native` promotion on that backend (proves the floor; no silent non-atomic fallback).

Run against in-memory every PR; both SQL backends under `--features integration`.

### 10.5 Rollout (per-store, independent, reversible, green on both)

Order by value/risk: **Governor → Turn-state → Event-log → Threads** (threads can also go first as a warm-up — no row migration).

Shared per-store steps:

| Step | Action | Reversible? |
|---|---|---|
| S0 | Land record model + indexes + backfill + `verify_parity` + `rematerialize_snapshot` + fault tests behind flag, default `Snapshot`/`TxnOrCas`. No prod behavior change. | n/a (dead code) |
| S1 | `DualWrite` in staging on **libSQL**. Native best-effort, snapshot authoritative. | Instant flip to `Snapshot`. |
| S2 | Backfill (`Absent` skip-if-present) + `verify_parity` on libSQL until zero divergence. | Idempotent re-run. |
| S3 | Flip libSQL `Native` (hard-gated on clean parity, §9.3); full fault suite + cross-user throughput test. | Flip `DualWrite` + run `rematerialize_snapshot`. |
| S4 | Repeat S1–S3 on **Postgres** (independent; parity guaranteed by contract tests). | Same. |
| S5 | Production: `DualWrite` → backfill → verify → `Native`, one backend at a time, throughput dashboard watched. | Same at every sub-step. |

**Filesystem-primitive prerequisite PRs (land before any consumer migration):**
1. **PR-1:** trait + capability surface, default impl only; `Capability::BatchPut`/`FilesystemOperation::PutBatch`; use the **existing** `BackendCapabilities::without()` helper (`types.rs:404`) at each new mount to drop `IndexVector` (§7.9 P4 — no new helper); composite delegation + same-mount check; default-impl `begin(common_dir_prefix)` (§6.1); **`ScopedFilesystem::put_batch` wrapper + `operation_allowed` arm**; net-new parameterized contract infra against the **default path** (N==1 everywhere; N>1 on PG+in-memory; N>1 on libSQL asserts typed `Unsupported`). Pure addition, zero behavior change, reversible.
2. **PR-2:** Postgres native override + advertise `BatchPut`+`MultiKey`; add `BatchPut` to `NEW_AXES`; native-vs-default equivalence green for PG; **raise the shared Postgres pool size (or split out a filesystem pool) with autovacuum headroom** (§6.1, §7.9 P1) — this lands before any governor/threads migration step.
3. **PR-3:** libSQL native override (`BEGIN IMMEDIATE`) + the version-readback **probe (with ROLLBACK)** + libSQL `begin`/`StorageTxn`; advertise `BatchPut`+`MultiKey` **only after** the probe confirms; suite green for libSQL. This is the PR that retires the would-be admission-gate by giving the governor one commit path on both backends.
4. **PR-4:** in-memory atomicity audit — confirm/lock-fix that the default path holds the per-op lock across the whole `put_batch` (the one place "defaults are free" could be wrong); **only then** widen `in_memory_full()` to advertise `BatchPut` (CodeRabbit review — keep it capped until proven).

**Independence guarantee:** per-store flag + disjoint path prefixes (`/resources`, `/turns`, `/threads`) ⇒ a store can be `Native` while others are `Snapshot`; a regression reverts that store alone.

**Both-backends guarantee:** every promotion is gated on contract + `store_parity` passing on *that* backend; libSQL and Postgres promote independently.

**The one one-way door (Rule 12):** the turn-state **event backfill** uses `append`, which assigns fresh seqs and cannot be cleanly un-appended. After the turns flag reaches `Native`, reversal accepts the native event log as authoritative (the snapshot's `events[]` is rematerialized from `tail()` on reverse). The run plane stays fully reversible. Because event-log rides the single `IRONCLAW_TURNS_WRITE_MODE` flag (§10.1), the run plane and the event plane **promote at the same flip** — there is no separate event-log cutover. The "longer soak" is therefore an **operational waiting period inside `DualWrite`** before that one flip, not an independent promotion; the run-plane `verify_parity` gate (which includes event-replay equivalence) is the guard for both planes.

Each PR green under `cargo clippy --all --tests --all-features` + `cargo test --features integration` (both backends), per the merge-queue gates.

---

## 11. Risks & mitigations

| Risk | Mitigation |
|---|---|
| **Governor over-admission** (concurrent same-account reserves both pass; any of 8 dimensions breached) | Single atomic `put_batch` over the touched account records on **both** backends (PG `BEGIN…COMMIT`, libSQL `BEGIN IMMEDIATE`). The whole multi-dimensional tally commits as one `Entry`; version-CAS on shared ledgers serializes concurrent reserves. No per-dimension interleaving window. Asserted by test 10.3(e). |
| **Account-seg collision** (`Display` `_`-placeholder, unvalidated ids) | Key by `hex(sha256(canonical_json(account)))`; collision-resistant, path-safe; injectivity-of-examples unit test incl. `_` and `/`-in-id cases. (Documented as collision-resistant, not "injective.") |
| **libSQL `RETURNING`-in-txn unavailability** | Init-time probe (rolled back, no scratch row); fall back to in-txn `SELECT` readback; advertise `BatchPut`/`MultiKey` only after probe confirms. Fail Loud if neither works. |
| **Pool starvation from held `BEGIN`** (PR #5081 class — the prod pool-deadlock incident; design note, not yet an in-tree code reference) | The filesystem shares **one** Postgres pool with triggers + event-store + a credential-keepalive lock (`factory.rs:3935/3936/3939/3950`, four consumers, default size **2**). Statically-sized batches (≤7) + `MAX_BATCH_PUTS=64` cap bound hold time, but **PR-2 must raise the pool size (or split the filesystem pool) before the governor/threads migration** — a floor of 2 is a hard regression. Leave headroom for the autovacuum worker (§7.9 P1). |
| **libSQL file-global write lock during `put_batch`** | Acceptable for this single-tenant, low-concurrency profile (≤2 small per-user records, microsecond commit, §7.8); disclosed honestly in §6.1 (not framed as per-account). Escalation if it bites: per-user libSQL shard/cell split (§7.8). Revisit `PRAGMA busy_timeout=5000ms` (`libsql.rs:90`) for multi-statement hold duration. |
| **Non-terminal turn event loss on crash-between-CAS-and-append** | Documented as tolerated (run record authoritative; terminal events re-derived). `put_batch` for the run write narrows but cannot eliminate the append-plane window. |
| **JSONB/body write churn & VACUUM bloat on hot rows** | Aggressive per-table autovacuum on `root_filesystem_entries`/events as a deployment prereq; prefix-partial indexes (§7.6) bound index work; typed-column projection recorded as a future lever (§7.9 P1). |
| **Planner ignores expression index at scale** | `ensure_index` emits **prefix-partial** expression indexes (§7.6); CI asserts the chosen plan via `EXPLAIN ANALYZE` (§9.4). |
| **Vector mount over-claims `IndexVector`** | These mounts do **not** advertise `IndexVector` (brute-force only); honesty invariant §3.4. pgvector tracked separately (§7.9 P4). |
| **Cross-user/tenant leakage via cheaper `query` path** | `query` bounded by `ScopedFilesystem` scope prefix + per-record `owner_user` assertion before exposure (§3.6, §8.5); test 10.3-threads(f). |
| **DualWrite divergence window / stale period anchor** | Snapshot authoritative through `DualWrite`; `verify_parity` asserts convergence at quiescence; **`Native` promotion hard-rejects on dirty parity** (§9.3). |
| **Period rollover never persists / overflow collapses cutoff** | Centralized overflow-safe (saturating) rollover helper; anchor advance committed via Phase-2 `put_batch` even on a non-mutating/denied reserve (§7.5). |
| **Thread torn write on a non-atomic backend** | The append **rejects** when `BatchPut`/`begin` absent; the only non-atomic path is the explicit `ForceCas` incident lever (§8.5); test 10.3-threads(e). |
| **Composite index unsupported on some backend** | Both SQL backends build multi-expression indexes; mount validator refuses a declared index a backend can't serve; in-memory linear-scans (contract still green). |

---

## 12. Open questions

1. **Event recovery scan window:** what is the right `RECOVERY_SCAN_WINDOW` bound vs duplicate-tolerance trade for non-terminal events? Pick empirically from event volume per scope.
2. **Turns `turns.json` (TurnRecord list) split:** keep as one phase-1 record or split per-thread? It is read-mostly and not a contention point; defer the split unless a query pattern demands it.
3. **Q-arch — governor as a relational sidecar (considered, deferred).** The thermo-nuclear review proposed giving the governor its own native relational tables (`resource_account_ledgers`/`resource_reservations`) with real typed columns, exactly as `ironclaw_triggers` already does (`crates/ironclaw_triggers/src/postgres.rs:31`) — which would enable Postgres HOT updates and a real multi-column atomic guard, at the cost of two native schemas and losing the single-copy backend migration. **Decision: deferred.** The `put_batch`-on-both-backends path (§7) resolves the multi-dimensional atomicity and global-contention problems *while preserving the migration-safety invariant (one logical record set, copy-not-transform)* and matching the doc's frozen-trait thesis. The relational sidecar remains the documented escape hatch if JSONB write-churn (§7.9 P1) or multi-column guard expressiveness ever becomes the bottleneck — the triggers crate proves the pattern is available and accepted in this repo. Revisit only if §7.9 P1 mitigations prove insufficient under production load.
4. **In-memory default-path atomicity (PR-4):** confirmed-or-fixed that the per-op lock is held across the whole `put_batch`; if the backend releases the map lock between legs in the default path, a ~10-line native override is owed before `in_memory_full()` advertises `BatchPut`. Verify at implementation time, do not assume.
5. **pgvector adoption:** vector search is brute-force today (§7.9 P4). Adopting pgvector (real column + ANN index) is the path to scalable similarity search and to honestly advertising `IndexVector`; tracked separately from this decomposition.
