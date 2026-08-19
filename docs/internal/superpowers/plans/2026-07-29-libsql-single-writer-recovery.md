# libSQL Single-Writer Runtime and Recoverable Turn Journal Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Route every production libSQL write through one shared writer lane per database and make retryable turn-journal contention recover without a process restart.

**Architecture:** Add a libSQL-only substrate that owns separate pooled reader and single-connection writer lanes. Production composition shares one runtime across the filesystem, trigger repository, and event store and does not accept a second libSQL event target; backend-neutral turn persistence retains and retries an atomic batch only when `FilesystemError::BackendBusy` says replay is safe.

**Tech Stack:** Rust 2024, Tokio, libSQL 0.9, deadpool 0.12, async-trait, tracing, Cargo workspace tests.

## Global Constraints

- PostgreSQL keeps its concurrent pool and does not depend on the libSQL runtime.
- File and other storage backends keep their own error mappings.
- `FilesystemError::BackendBusy` is retryable only for atomic, whole-operation replay.
- No persistence schema or durable-record format changes.
- Do not use `.unwrap()` or `.expect()` in production code.
- Read concurrency stays pooled; only libSQL writes are serialized.
- Production composition must share one runtime for each configured libSQL database.
- Keep credentials, database URLs, local paths, payloads, and tenant content out of logs.

---

### Task 1: Shared libSQL Runtime

**Files:**
- Create: `crates/ironclaw_libsql_runtime/Cargo.toml`
- Create: `crates/ironclaw_libsql_runtime/src/lib.rs`
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `crates/ironclaw_architecture_tests/tests/reborn_dependency_boundaries.rs`

**Interfaces:**
- Produces: `LibSqlRuntime::new(Arc<libsql::Database>) -> Result<Self, LibSqlRuntimeError>`
- Produces: `async fn LibSqlRuntime::open(path_or_url, auth_token) -> Result<Self, LibSqlRuntimeError>`
- Produces: `async fn LibSqlRuntime::read(&self) -> Result<LibSqlReadConnectionLease, LibSqlRuntimeError>`
- Produces: `async fn LibSqlRuntime::write(&self) -> Result<LibSqlWriteConnectionLease, LibSqlRuntimeError>`
- Produces: a query-only read lease whose pooled connections enforce
  `PRAGMA query_only = ON`
- Produces: `LibSqlWriteConnectionLease: Deref<Target = libsql::Connection>`
- Produces: `LIBSQL_READ_POOL_MAX_CONNECTIONS = 8`
- The writer pool maximum is exactly one and is not configurable in production.

- [ ] **Step 1: Add the crate scaffold and a failing public-behavior test**

Register `crates/ironclaw_libsql_runtime` as a workspace member and add a
substrates-layer manifest with `deadpool`, `libsql`, `thiserror`, `tokio`, and
`tracing`. Start `src/lib.rs` with a test that constructs the wished-for API:

```rust
#[tokio::test]
async fn one_writer_waits_while_a_reader_remains_available() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("runtime.db");
    let database = Arc::new(
        libsql::Builder::new_local(path)
            .build()
            .await
            .expect("database"),
    );
    let runtime = Arc::new(LibSqlRuntime::new(database));

    let first_writer = runtime.write().await.expect("first writer");
    let waiting_runtime = Arc::clone(&runtime);
    let mut second_writer =
        tokio::spawn(async move { waiting_runtime.write().await.expect("second writer") });

    assert!(
        tokio::time::timeout(Duration::from_millis(25), &mut second_writer)
            .await
            .is_err(),
        "a second writer must wait for the sole writer lane"
    );
    tokio::time::timeout(Duration::from_millis(250), runtime.read())
        .await
        .expect("reader must not queue behind writer")
        .expect("reader checkout");

    drop(first_writer);
    tokio::time::timeout(Duration::from_millis(250), second_writer)
        .await
        .expect("second writer admitted")
        .expect("writer task");
}
```

- [ ] **Step 2: Run the test and verify RED**

Run:

```bash
cargo test -p ironclaw_libsql_runtime one_writer_waits_while_a_reader_remains_available -- --exact --nocapture
```

Expected: compilation fails because `LibSqlRuntime` and its lease methods do
not exist.

- [ ] **Step 3: Implement the minimal runtime**

Implement two deadpool pools over the same `Arc<libsql::Database>`. The read
pool uses eight connections; the writer pool uses one. Wrap the deadpool object
so the manager type remains private:

```rust
pub struct LibSqlConnectionLease(deadpool::managed::Object<LibSqlConnectionManager>);

impl Deref for LibSqlConnectionLease {
    type Target = libsql::Connection;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub struct LibSqlRuntime {
    read_pool: Pool<LibSqlConnectionManager>,
    write_pool: Pool<LibSqlConnectionManager>,
}
```

Use the existing connection policy exactly: 10-second checkout timeout, three
connection attempts with 50/100/200 ms backoff, and the PRAGMA batch:

```sql
PRAGMA busy_timeout = 5000;
PRAGMA synchronous = NORMAL;
PRAGMA temp_store = MEMORY;
PRAGMA cache_size = -16000;
PRAGMA mmap_size = 268435456;
PRAGMA wal_autocheckpoint = 1000;
```

Reject a recycled connection when `is_autocommit()` is false. Map open,
initialization, and checkout failures to a redacted `LibSqlRuntimeError`.
Measure checkout elapsed time and emit structured `lane`, `wait_ms`, and
`queued` fields without logging the target.

- [ ] **Step 4: Verify GREEN and architecture**

Run:

```bash
cargo test -p ironclaw_libsql_runtime -- --nocapture
cargo test -p ironclaw_architecture_tests reborn_workspace_crates_declare_layers_and_follow_layer_matrix -- --exact
```

Expected: both pass.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock crates/ironclaw_libsql_runtime crates/ironclaw_architecture_tests/tests/reborn_dependency_boundaries.rs
git commit -m "feat(libsql): add shared read and writer runtime"
```

---

### Task 2: Route the Root Filesystem Through the Runtime

**Files:**
- Modify: `crates/ironclaw_filesystem/Cargo.toml`
- Modify: `crates/ironclaw_filesystem/src/lib.rs`
- Modify: `crates/ironclaw_filesystem/src/libsql.rs`
- Delete: `crates/ironclaw_filesystem/src/libsql_pool.rs`
- Test: `crates/ironclaw_filesystem/src/libsql.rs`
- Modify: `docs/internal/reborn/contracts/filesystem.md`

**Interfaces:**
- Consumes: `Arc<LibSqlRuntime>`, `LibSqlRuntime::read`, and `LibSqlRuntime::write`
- Produces: `LibSqlRootFilesystem::from_runtime(Arc<LibSqlRuntime>) -> Self`
- Preserves: `LibSqlRootFilesystem::new(Arc<libsql::Database>) -> Self`

- [ ] **Step 1: Write a failing atomic-delete regression**

Add a backend test that creates an entry and append events under one path,
installs a temporary SQLite trigger that aborts deletion from
`root_filesystem_events`, calls `RootFilesystem::delete`, and then asserts
through `get` and `tail` that both the entry and events remain. The essential
assertions are:

```rust
let result = filesystem.delete(&path).await;
assert!(matches!(result, Err(FilesystemError::Backend { .. })));
assert!(filesystem.get(&path).await.expect("get").is_some());
assert_eq!(
    filesystem
        .tail(&path, SeqNo::from_backend(0).expect("zero"))
        .await
        .expect("tail")
        .len(),
    1
);
```

Use a writer lease from the shared runtime to install and later remove the
test-only abort trigger.

- [ ] **Step 2: Run the atomic-delete test and verify RED**

Run:

```bash
cargo test -p ironclaw_filesystem --lib delete_rolls_back_all_tables_when_event_cleanup_fails -- --exact --nocapture
```

Expected: the delete returns an error but the entry is missing, proving the
current three-autocommit implementation partially applied.

- [ ] **Step 3: Inject the runtime and classify leases**

Replace the private pool field with:

```rust
pub struct LibSqlRootFilesystem {
    runtime: Arc<ironclaw_libsql_runtime::LibSqlRuntime>,
}
```

Keep `new(db)` by constructing a runtime and add `from_runtime(runtime)`.
Replace `connect()` with private `read_connection()` and
`write_connection()` helpers that map `LibSqlRuntimeError` to
`FilesystemError` without exposing targets.

Audit every `RootFilesystem` implementation method:

- `get`, reads, list/stat, tail/head, queries, and read-only index lookup use
  `read_connection()`;
- migrations, `put`, byte writes/appends, delete, CAS delete, sequence append
  and reservation, directory creation, and index declaration use
  `write_connection()`;
- a read-modify-write method must perform its precondition reads on the same
  writer lease and must not call another method that checks out a connection.

Move the pool manager and connection initialization tests into the runtime
crate, and preserve filesystem-specific checkout error mapping tests at the
filesystem boundary.

- [ ] **Step 4: Make multi-statement writes atomic**

Wrap the three-table `delete` in `BEGIN IMMEDIATE` / `COMMIT` with a single
best-effort `ROLLBACK` path. Replace `append_batch().transaction()` with an
explicit `BEGIN IMMEDIATE`, run the whole payload loop on that writer lease,
and commit only after every payload succeeds. Preserve
`FilesystemError::BackendBusy` mapping for `BUSY` and `LOCKED`.

- [ ] **Step 5: Verify GREEN and filesystem regressions**

Run:

```bash
cargo test -p ironclaw_filesystem --lib delete_rolls_back_all_tables_when_event_cleanup_fails -- --exact --nocapture
cargo test -p ironclaw_filesystem --lib append_batch_surfaces_real_writer_contention_as_backend_busy -- --exact --nocapture
cargo test -p ironclaw_filesystem --test concurrent_cas_storm -- --nocapture
```

Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add crates/ironclaw_filesystem docs/internal/reborn/contracts/filesystem.md
git commit -m "refactor(filesystem): use shared libsql writer lane"
```

---

### Task 3: Share the Runtime Across Triggers, Events, and Production Composition

**Files:**
- Modify: `crates/ironclaw_triggers/Cargo.toml`
- Modify: `crates/ironclaw_triggers/src/libsql.rs`
- Test: `crates/ironclaw_triggers/src/libsql.rs`
- Modify: `crates/ironclaw_event_store/src/lib.rs`
- Test: `crates/ironclaw_event_store/src/lib.rs`
- Modify: `crates/ironclaw_composition/Cargo.toml`
- Modify: `crates/ironclaw_composition/src/factory.rs`
- Test: `crates/ironclaw_composition/src/factory.rs`
- Modify: `docs/internal/reborn/contracts/triggers.md`
- Modify: `docs/internal/reborn/contracts/events.md`

**Interfaces:**
- Consumes: `Arc<LibSqlRuntime>`
- Produces: `LibSqlTriggerRepository::from_runtime(Arc<LibSqlRuntime>) -> Self`
- Produces: `RebornEventStoreConfig::LibsqlFilesystem { filesystem: Arc<LibSqlRootFilesystem> }`
- Preserves: standalone `LibSqlTriggerRepository::new(Arc<libsql::Database>)`
- Preserves: standalone `RebornEventStoreConfig::Libsql { path_or_url, auth_token }`

- [ ] **Step 1: Write a failing cross-adapter serialization test**

In the trigger libSQL tests, construct one runtime, one
`LibSqlRootFilesystem::from_runtime`, and one
`LibSqlTriggerRepository::from_runtime`. Run both migrations, hold a writer
lease, spawn one filesystem append and one trigger upsert, and assert neither
finishes before the held lease drops:

```rust
let held_writer = runtime.write().await.expect("held writer");
let mut filesystem_write = tokio::spawn(async move {
    filesystem.append(&path, b"event".to_vec()).await
});
let mut trigger_write =
    tokio::spawn(async move { repository.upsert_trigger(record).await });

assert!(timeout(Duration::from_millis(25), &mut filesystem_write).await.is_err());
assert!(timeout(Duration::from_millis(25), &mut trigger_write).await.is_err());
drop(held_writer);
assert!(timeout(Duration::from_secs(1), filesystem_write).await.expect("fs task").is_ok());
assert!(timeout(Duration::from_secs(1), trigger_write).await.expect("trigger task").is_ok());
```

- [ ] **Step 2: Run the test and verify RED**

Run:

```bash
cargo test -p ironclaw_triggers libsql::tests::filesystem_and_trigger_writes_share_one_admission_lane -- --exact --nocapture
```

Expected: compilation fails because the trigger repository has no shared
runtime constructor and still opens direct connections.

- [ ] **Step 3: Route trigger reads and writes**

Store `Arc<LibSqlRuntime>` in `LibSqlTriggerRepository`. Keep `new(db)` as a
compatibility wrapper and add `from_runtime(runtime)`. Replace `connect()` with
read/write helpers. Migrations, upserts, renames, deletes, claims, lease/state
transitions, and run-history mutations use a writer lease; pure get/list/scan
methods use a read lease. A multi-statement trigger transition keeps one writer
lease until commit or rollback.

- [ ] **Step 4: Reuse the filesystem for event logs**

Add:

```rust
RebornEventStoreConfig::LibsqlFilesystem {
    filesystem: Arc<LibSqlRootFilesystem>,
}
```

The builder wraps this already-migrated filesystem with
`build_reborn_event_stores_from_root_filesystem` and does not reopen a target or
rerun migrations. Keep the existing config-based libSQL builder for standalone
use.

Add an event-store test that builds from the prebuilt variant, appends one
durable event through the returned log, and reads it back through the same
filesystem-backed store.

- [ ] **Step 5: Wire exactly one production runtime**

Before `build_libsql_production`, create one `Arc<LibSqlRuntime>` from the
configured or supplied database. Construct filesystem and trigger repository
from that same `Arc`. Pass the already-migrated filesystem in
`LibsqlFilesystem`; retain `path_or_url` only for validation and never pass
credentials to a builder that can reopen the database. The libSQL production
substrate input likewise accepts the primary runtime and target, not an
independent event-store configuration.

Add a factory test that constructs the libSQL backend through the production
builder test seam and proves a held runtime writer blocks both a filesystem
mutation and trigger mutation, or—if the production bundle does not expose
those concrete adapters—asserts the selected event-store configuration is the
prebuilt-filesystem variant before it is consumed.

- [ ] **Step 6: Verify GREEN**

Run:

```bash
cargo test -p ironclaw_triggers libsql::tests::filesystem_and_trigger_writes_share_one_admission_lane -- --exact --nocapture
cargo test -p ironclaw_triggers --lib libsql::tests -- --nocapture
cargo test -p ironclaw_event_store --lib -- --nocapture
cargo test -p ironclaw_composition --lib libsql -- --nocapture
```

Expected: all pass.

- [ ] **Step 7: Commit**

```bash
git add crates/ironclaw_triggers crates/ironclaw_event_store crates/ironclaw_composition docs/internal/reborn/contracts/triggers.md docs/internal/reborn/contracts/events.md
git commit -m "fix(composition): share one libsql runtime"
```

---

### Task 4: Recover the Exact Turn-Journal Batch After `BackendBusy`

**Files:**
- Modify: `crates/ironclaw_turns/src/turn_state_row_store/row_store/journal.rs`
- Modify: `crates/ironclaw_turns/src/turn_state_row_store/row_store/write_behind.rs`
- Modify: `crates/ironclaw_turns/src/turn_state_row_store/row_store.rs`
- Modify: `crates/ironclaw_turns/tests/row_store_crash_consistency.rs`
- Modify: `docs/internal/reborn/contracts/turn-persistence.md`

**Interfaces:**
- Consumes: `FilesystemError::BackendBusy`
- Produces internal `JournalHealth::{Healthy, RecoveringContention, FailedFatal}`
- Preserves public `TurnStateStore` and `RootFilesystem` interfaces.

- [ ] **Step 1: Write a failing recovery regression**

Extend `FaultBackend` with a `busy_next_appends` counter whose injected error is:

```rust
FilesystemError::BackendBusy {
    path: path.clone(),
    operation: FilesystemOperation::Append,
}
```

Add `write_behind_backend_busy_retries_exact_batch_and_recovers_without_reopen`.
Submit a durable run, inject one busy append, perform a non-critical claim,
call `drain`, and assert:

```rust
store.drain().await.expect("the retained batch recovers");
let state = store
    .get_run_state(GetRunStateRequest {
        scope: scope.clone(),
        run_id,
    })
    .await
    .expect("recovered state");
assert_eq!(state.status, TurnStatus::Running);
store
    .submit_turn(
        submit_request(scope, TurnRunId::new(), "idem-after-busy-recovery"),
        &AllowAllTurnAdmissionPolicy,
        &InMemoryRunProfileResolver::default(),
    )
    .await
    .expect("same store accepts mutations after recovery");
```

Also assert the recorded durable append payload appears once, not once per
attempt.

- [ ] **Step 2: Run the recovery test and verify RED**

Run:

```bash
cargo test -p ironclaw_turns --test row_store_crash_consistency write_behind_backend_busy_retries_exact_batch_and_recovers_without_reopen -- --exact --nocapture
```

Expected: `drain` returns `TurnError::Unavailable` because the existing flusher
halts on every append error.

- [ ] **Step 3: Preserve the filesystem error until classification**

Serialize request deltas once before the retry loop. Make the filesystem append
helper return `FilesystemError` directly. Validate the returned acknowledgement
count separately as a fatal `TurnError`. Do not convert `BackendBusy` through
`fs_error` until the flusher decides an error is fatal.

- [ ] **Step 4: Implement the health state and retained-batch loop**

Replace `AtomicBool` with `AtomicU8` encoded by:

```rust
#[repr(u8)]
enum JournalHealth {
    Healthy = 0,
    RecoveringContention = 1,
    FailedFatal = 2,
}
```

For `BackendBusy`, retain the same requests and serialized payloads, set
`RecoveringContention`, and retry with 25 ms jittered exponential backoff capped
at one second. Continue slow probes until success or task shutdown; do not
resolve acknowledgements or accept new mutations while recovering. On success,
acknowledge once, set `Healthy`, emit recovery duration/attempt fields, and
continue with the original queue. On any other error, set `FailedFatal`, return
errors to the current batch, close and drain the queue, and stop.

Make mutation admission return retryable unavailable in both recovering and
fatal states, but clear the hot snapshot only for `FailedFatal`. Reads may use
the hot snapshot during contention recovery because it includes the retained
accepted batch. A caller timeout must not clear the cache while the journal
still owns and may commit the acknowledgement.

- [ ] **Step 5: Verify GREEN and fatal parity**

Run:

```bash
cargo test -p ironclaw_turns --test row_store_crash_consistency write_behind_backend_busy_retries_exact_batch_and_recovers_without_reopen -- --exact --nocapture
cargo test -p ironclaw_turns --test row_store_crash_consistency write_behind_append_failure_halts_degrades_and_recovers_consistently -- --exact --nocapture
cargo test -p ironclaw_turns --test row_store_crash_consistency -- --nocapture
```

Expected: retryable contention recovers in place; fatal failure still fails
closed with no later durable gap; the full crash-consistency suite passes.

- [ ] **Step 6: Commit**

```bash
git add crates/ironclaw_turns docs/internal/reborn/contracts/turn-persistence.md
git commit -m "fix(turns): recover journal after transient backend contention"
```

---

### Task 5: Final Verification and Pull Request

**Files:**
- Modify if required by findings: files already listed in Tasks 1–4
- Create: pull-request body from `.github/pull_request_template.md`

**Interfaces:**
- Verifies all preceding tasks; produces no new runtime interface.

- [ ] **Step 1: Scan production changes for forbidden patterns**

Run:

```bash
git diff --unified=0 origin/main...HEAD -- \
  'crates/**/*.rs' \
  ':(exclude)crates/**/tests/**' \
  ':(exclude)crates/**/*_test.rs' \
  ':(exclude)crates/**/test_*.rs' \
  | rg '^\+[^+]' \
  | rg -n '\.unwrap\(|\.expect\(|/tmp|std::env::temp_dir|\[[^]]+\.\.[^]]+\]'
```

Expected: no new production `.unwrap()`/`.expect()`, hardcoded temporary path,
or suspicious byte-slicing findings.

- [ ] **Step 2: Run formatting and focused package checks**

Run:

```bash
cargo fmt --check
cargo test -p ironclaw_libsql_runtime
cargo test -p ironclaw_filesystem
cargo test -p ironclaw_triggers --lib
cargo test -p ironclaw_event_store --lib
cargo test -p ironclaw_composition --lib
cargo test -p ironclaw_turns --test row_store_crash_consistency
cargo test -p ironclaw_architecture_tests
```

Expected: all pass with no warnings.

- [ ] **Step 3: Run clippy and PostgreSQL parity checks**

Run:

```bash
cargo clippy -p ironclaw_libsql_runtime -p ironclaw_filesystem -p ironclaw_triggers -p ironclaw_event_store -p ironclaw_composition -p ironclaw_turns --all-targets -- -D warnings
cargo test -p ironclaw_filesystem db::tests::postgres_transient_write_conflicts_are_retryable_contention -- --exact
```

If PostgreSQL integration tests require Docker, use the environment's
configured Docker endpoint and run only the package-selected storage contract
named by the testing playbook.

- [ ] **Step 4: Review the complete diff**

Run:

```bash
git status --short
git diff --stat origin/main...HEAD
git diff --check origin/main...HEAD
git diff origin/main...HEAD
```

Confirm the implementation matches the approved spec, existing defaults remain
unchanged outside libSQL, rollback is code-only, and generated files contain
only the required lockfile update.

- [ ] **Step 5: Commit any verification-only fixes**

```bash
git add Cargo.toml Cargo.lock crates/ironclaw_libsql_runtime crates/ironclaw_filesystem crates/ironclaw_triggers crates/ironclaw_event_store crates/ironclaw_composition crates/ironclaw_turns crates/ironclaw_architecture_tests docs/internal/reborn/contracts
git commit -m "test(libsql): complete writer recovery coverage"
```

Skip this commit when the worktree is already clean.

- [ ] **Step 6: Push and open a draft pull request**

Verify GitHub authentication, push `codex/libsql-writer-runtime`, and create a
draft PR. Complete every pull-request-template test tier with command evidence
or `Not applicable:` followed by a concrete reason. The body must state:

- the verified SQLite writer-lock contention and permanent-journal-latch cause;
- libSQL-only single writer mechanics;
- backend-neutral `BackendBusy` retained-batch recovery;
- PostgreSQL compatibility;
- no schema change;
- code-only rollback;
- QA soak plan and remaining multi-process-local-file limitation.

Expected: the remote branch exists and the draft PR URL resolves.
