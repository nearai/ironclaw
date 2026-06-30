# Reborn Integration-Test Framework — Slice 3 Implementation Plan

**Date:** 2026-06-27 (rev 2 — after thermo-nuclear + approach/local-patterns/maintainability plan-review)
**Branch:** `reborn-itest-framework-slices` (combined framework branch; carries slice 4 + slice 9 docs already)
**Base:** PR #5381 merge `6a3b10fa5` (slices 1–2)
**Scope:** `StorageMode { InMemory, LibSql }` + the backend-matrix `rstest` template
(design spec §3.2, §3.8, §7, build-order step 4 + the rstest part of step 6).
This is the **foundational** slice — once it lands, auth/approval/install/skill/
secret coverage is "mostly wiring" (§3.8).

---

## Design decision (locked by thermo-nuclear review): Option C — one composite, mirror production, contained blast radius

The integration harness builds **one `CompositeRootFilesystem`** exactly as
production's `build_default_local_dev_database_roots`
(`crates/ironclaw_reborn_composition/src/factory.rs:2238`) does. `StorageMode`
selects **only the durable backend mounted into that composite** — an
`InMemoryBackend` (default) or a `LibSqlRootFilesystem` over a per-`build()` tmp
`.db` — handed to the production `mount_local_dev_database_roots` helper.

**Scope of "control-plane" for this slice = the turn-lifecycle stores the agent
turn actually persists through and the tests assert on: thread history + turn
state.** Both ride the one composite at the **production** path layout
`/tenants/<tenant>/users/<user>/...`. The product-binding harness stays on its
existing build-time backend this slice (it is resolved once at `build()` and
neither matrix test asserts product persistence — see §6). Product / auth /
approval / install / skill / secret stores **join the same composite** in their
own §3.8 coverage slices; the design keeps **one** composite and grows the set of
stores mounted onto it incrementally — it does not stand up a second durable
mechanism.

### Why this shape (and not the alternatives)

Production proves the pattern: `build_default_local_dev_database_roots` mounts the
durable backend through the *same* `mount_local_dev_database_roots` whether it is
libSQL, Postgres, **or** `InMemoryBackend` — only `LocalDevStorageBackendInput`
varies. `StorageMode` maps 1:1 onto that single axis.

- **Rejected — `Arc<dyn RootFilesystem>` at every store.** Unnecessary: the
  *composite* is the single concrete backend; the `dyn` already lives **inside**
  `CompositeRootFilesystem` (`CompositeMount.backend: Arc<dyn RootFilesystem>`,
  `catalog.rs:72`). Stores ride the concrete `CompositeRootFilesystem`.
- **Rejected — fully migrate every harness to the composite in one slice.** The
  store harnesses are **shared with the binary E2E tier** (see Blast radius);
  migrating their field types in place ripples into the 3822-line `harness.rs`.
  The generic-with-default mechanism below gets the composite into the
  integration tier while leaving the binary tier byte-identical.
- **Rejected — leave thread+turn on three separate backends and bolt LibSql on
  as a parallel arm.** Two storage mechanisms by design, leaves the promoted
  `mount_local_dev_database_roots` half-dead, forfeits §3.8's payoff. Spaghetti
  (`.claude/rules/architecture.md` §4).

### The mechanism (verified feasible against real types)

- `ScopedFilesystem<F: ?Sized>` (`crates/ironclaw_filesystem/src/scoped.rs:39`)
  and `with_fixed_view(root: Arc<F>, view)` accept any backend.
- `CompositeRootFilesystem` is itself a concrete `RootFilesystem`, holding its
  durable/project mounts as `Arc<dyn RootFilesystem>` internally (`mount_dyn`,
  `catalog.rs`). So the harness rides the **concrete** `CompositeRootFilesystem`.
- **Generic-with-default on the store harness, so the binary tier is untouched.**
  `RebornThreadHarness<F = LocalFilesystem>` (field
  `service: Arc<FilesystemSessionThreadService<F>>`, `backend: Arc<F>`). Existing
  `RebornThreadHarness` (no params) resolves to `RebornThreadHarness<LocalFilesystem>`
  via the default → `RebornBinaryE2EHarness` (`harness.rs:849`),
  `RebornHarnessSharedStorage` (`harness.rs:200`), and
  `HarnessLoopExitEvidencePort<…<LocalFilesystem>>` (`harness.rs:1376`) compile
  unchanged. `scoped_threads_fs_at<F>` is *already* generic
  (`session_thread.rs:116`). Split the impl blocks: a generic
  `impl<F: RootFilesystem> RebornThreadHarness<F>` for the shared methods
  (`history`/`assert_final_reply`/`reopened`/`service_instance`), the existing
  `impl RebornThreadHarness<LocalFilesystem>` for `filesystem_temp`, and a new
  composite constructor (§4).
- `LibSqlRootFilesystem::new(Arc<libsql::Database>)` + `.run_migrations().await`
  over `libsql::Builder::new_local(<TempDir>/itest.db).build().await`
  (`crates/ironclaw_filesystem/src/libsql.rs:1878–1937`). **Not re-implemented in
  the harness** — reused through a test-support accessor (§2b) so the four-step
  sequence lives once.

### Per-`build()` isolation (test concurrency)

Cargo runs `#[tokio::test]` fns concurrently within a binary and runs test
binaries in parallel. Unit of isolation = **one composite + one durable backend
per `build()`**: the libSQL `.db` lives in a per-`build()` `TempDir` the
integration harness owns (kept as `_turn_root`, the same field name
`RebornBinaryE2EHarness` uses — `harness.rs:187`). `InMemoryBackend` is
per-`build()`. **Never shared across tests.** `assert_reply_persists_after_reopen`
re-opens the *same* `.db` (the harness keeps the `TempDir` alive and passes an
`Arc<TempDir>` clone into the thread harness so `reopened()` keeps it alive),
proving on-disk durability, not an in-process cache.

---

## Verified production seam (do not re-derive)

| Fact | Location |
|---|---|
| One composite per runtime; durable backend mounted via the helper | `factory.rs:2207–2298` (`build_local_dev_root_filesystem`, `build_default_local_dev_database_roots`) |
| `mount_local_dev_database_roots` mounts `/tenants`,`/memory`,`/events` | `factory.rs:2323` (`pub(crate)`, this slice) |
| Production durable paths: `/tenants/<t>/users/<u>/...` | `factory.rs:5123–5311`, `lib.rs:712–790` |
| `LibSqlRootFilesystem::new` + `run_migrations` | `libsql.rs:1878–1937` |
| `CompositeRootFilesystem` / `mount_dyn` / object-safe `RootFilesystem` | `catalog.rs:60–110` |
| `ScopedFilesystem<F: ?Sized>` | `scoped.rs:39` |
| `scoped_threads_fs_at<F>` already generic | `session_thread.rs:116` |
| Shared store harnesses + binary-tier callers (DO NOT break) | `harness.rs:187,200,849,1376,3712`; `session_thread.rs:37,43`; `product_workflow.rs:47,55` |

The harness's slice-1 `/engine/tenants/...` prefix (`session_thread.rs:127`,
`harness.rs::scoped_turns_fs`) is a **divergence** from production. Moving the
*integration* path to `/tenants/...` is required for store data to land under the
mounted roots — it is what makes the promoted helper non-dead. The **binary tier
keeps `/engine/tenants/...`** (it is out of scope; see Blast radius).

---

## File-by-file

### 1. `crates/ironclaw_reborn_composition/src/factory.rs` (production, DONE)
`mount_local_dev_database_roots` promoted `fn` → `pub(crate) fn` with a doc-comment
naming production callers + the test accessor. Behavior-preserving. **On-branch.**

### 2. `crates/ironclaw_reborn_composition/src/test_support.rs` (production, DONE + 2b)
- (DONE) `mount_local_dev_database_roots_for_test<F>(...)` — `#[cfg(feature = "test-support")]`
  accessor forwarding to the `pub(crate)` mount helper. **On-branch.**
- **(2b, new) Avoid duplicating the libsql ctor sequence.** Promote
  `build_default_local_dev_database_roots` to `pub(crate)` and add
  `#[cfg(feature = "test-support")] pub async fn build_default_local_dev_database_roots_for_test(root: &Path, composite: &mut CompositeRootFilesystem) -> Result<(), RebornBuildError>`
  forwarding to it (doc-comment naming the production call site, per
  `ironclaw_reborn_composition/CLAUDE.md`). The harness's `LibSql` arm calls this
  instead of re-implementing `libsql::Builder::new_local + LibSqlRootFilesystem::new +
  run_migrations + mount` (maintainability finding — single source of the
  database-roots truth). (The `#[cfg(not(feature="libsql"))]` branch of that
  function already wires `InMemoryBackend`, so the accessor also covers the
  default mode if we want one entry point; but the harness builds InMemory
  directly via `mount_local_dev_database_roots_for_test` to avoid pulling the
  `reborn-local-dev.db` filename into the InMemory path.)

### 3. `tests/support/reborn/builder.rs` — `StorageMode` + one-composite build
- `pub enum StorageMode { InMemory, LibSql }` (default `InMemory`). Builder field
  `storage: StorageMode`; method `pub fn storage(mut self, mode) -> Self`.
- Add a focused helper (in `builder.rs`, or extract to a new `storage.rs` support
  module if `builder.rs` nears the 1k ceiling):
  `async fn build_storage_composite(mode, dir: &Path) -> HarnessResult<Arc<CompositeRootFilesystem>>`:
  - `let mut composite = CompositeRootFilesystem::new();`
  - `InMemory` → `mount_local_dev_database_roots_for_test(&mut composite, Arc::new(InMemoryBackend::new()))?`
  - `LibSql` → `build_default_local_dev_database_roots_for_test(dir, &mut composite).await?` (§2b)
  - `Ok(Arc::new(composite))`
- `build()` constructs the composite **once** over the harness `TempDir`
  (`_turn_root` — keep the field name for parity with `RebornBinaryE2EHarness`
  `harness.rs:187`, but add a doc-comment: post-migration this TempDir is the
  durable root for the **whole composite** (thread + turn), not just turns) and
  threads the *same* `Arc<CompositeRootFilesystem>` into the
  thread harness (§4) and the turn store (§5). This replaces slice-1's
  thread/turn `filesystem_temp` + `InMemoryBackend` turn backend with the one
  composite for **both** modes (deletes the per-store-backend split for the
  integration tier).

### 4. `tests/support/reborn/session_thread.rs` — generic backend, composite ctor, prod path
- Make `RebornThreadHarness<F = LocalFilesystem>` (fields
  `service: Arc<FilesystemSessionThreadService<F>>`, `backend: Arc<F>`,
  `root: Arc<tempfile::TempDir>`, `scope`). Default param keeps every existing
  un-parameterized use compiling as `<LocalFilesystem>`.
- Move shared methods into `impl<F: RootFilesystem> RebornThreadHarness<F>`
  (`history`, `assert_final_reply`, `reopened`, `service_instance`); keep
  `filesystem_temp` in `impl RebornThreadHarness<LocalFilesystem>`.
- New `pub fn filesystem_shared_composite(scope, backend: Arc<CompositeRootFilesystem>, root: Arc<tempfile::TempDir>) -> Result<RebornThreadHarness<CompositeRootFilesystem>, _>`
  — **named to parallel the existing `filesystem_shared_backend`** (local-patterns
  finding), same `(scope, backend, root)` shape so `reopened()` keeps the
  `Arc<TempDir>` alive (thermo TempDir-lifecycle finding).
- **Parameterize the thread-path prefix — `scoped_threads_fs_at` hardcodes
  `/engine/tenants/...` today (`session_thread.rs:127`), so the composite ctor
  CANNOT "just change the path string" by calling it as-is — that would write
  threads outside the composite's `/tenants` mount → silent empty-history
  failure.** Add a `root_prefix: &str` param to `scoped_threads_fs_at`: existing
  `filesystem_temp`/`filesystem_shared_backend` callers pass `"/engine"`
  (behavior-preserving for the binary tier); `filesystem_shared_composite` passes
  the production root so the target is `/tenants/{tenant}/users/{user}/threads`.
  One generic helper, prefix-selected — one source of thread-path truth shared
  across both tiers, no second copy.

### 5. Turn store on the composite — a NEW helper, binary `scoped_turns_fs` untouched
- **Do not change** `scoped_turns_fs`, the `HarnessTurnStorageBackend` /
  `HarnessTurnBackend` aliases, or `RebornHarnessSharedStorage`'s
  block/wait/release turn-state-put primitives — the binary E2E harness
  (`harness.rs:3712,833`) depends on them (thermo Blocker 2).
- **Extract the turn-path segment logic so it is NOT duplicated.** `scoped_turns_fs`
  has a 4-arm `match (agent_id, project_id)` building
  `/engine/tenants/.../turns` (`harness.rs:3724`). A composite turn helper needs
  the *same* 4 arms with a different prefix — copying them invites silent drift
  when a 5th arm (e.g. `mission_id`) is added. Extract
  `turns_scope_path(root_prefix: &str, binding: &ResolvedBinding) -> String`
  (the 4-arm match, prefix-parameterized) into the integration support
  (`storage.rs` or `filesystem.rs`); rewrite the existing `scoped_turns_fs` to call
  it with `"/engine"` (behavior-preserving, and this *shrinks* `harness.rs`).
  One source of turn-path truth across both tiers.
- Add `scoped_turns_fs_composite(composite: Arc<CompositeRootFilesystem>, binding) -> HarnessResult<Arc<ScopedFilesystem<CompositeRootFilesystem>>>`
  calling `turns_scope_path("/tenants", &binding)`. **Place the new helper +
  `turns_scope_path` in the integration support (builder.rs / storage.rs), NOT by
  growing `harness.rs`** — it is already 3822 lines (> architecture.md §5
  threshold). `scoped_turns_fs` keeps living in `harness.rs` but gets *shorter*
  (its match body moves to the shared helper). The integration turn store becomes
  `FilesystemTurnStateStore::new(scoped_turns_fs_composite(composite, &binding)?)`
  (no `BlockingTurnStatePutFilesystem` wrapper — the integration tier does not
  exercise the blocking primitive; that primitive is binary-tier-only and operates
  below the composite, so it legitimately does not unify — DEPTH verdict).

### 6. Product-binding harness — stays build-time this slice (decision, not a hedge)
`RebornProductWorkflowHarness` is **not** migrated to the composite in slice 3.
Rationale (thermo + maintainability + approach all flagged the earlier hedge):
the product binding is resolved once at `build()`; both matrix tests
(`backend_parity_replies_to_greeting`, `libsql_persists_reply_across_reopen`)
assert only on thread-history durability via `RebornThreadHarness::reopened()` and
never read product state back. Migrating it would change
`product_workflow.rs:595` `/engine/tenants/...` paths and the TempDir-keyed
`idempotency_lock_for_workflow_root` (`product_workflow.rs:349`) for zero
observable test value. **Product / auth / approval / install / skill / secret
stores join the one composite in their §3.8 coverage slices**, reusing
`idempotency_lock_for_filesystem` (`product_workflow.rs:360`, the Arc-pointer-keyed
seam) at that point. Documented in §10 so the boundary is explicit, not silent.

### 7. `assert_reply_persists_after_reopen` (assertions.rs / co-located)
`pub async fn assert_reply_persists_after_reopen(&self, text: &str) -> HarnessResult<()>`:
reconstruct the thread service from the **same** composite via
`RebornThreadHarness::reopened()` (fresh `FilesystemSessionThreadService` over the
same backend → for `LibSql`, a fresh read path over the same `.db`) and assert the
finalized reply contains `text`. Lives beside the other `assert_*` (today
`assertions.rs`, from slice 4) with `pub(super)` field accessors — no new
mechanism.

### 8. `Cargo.toml` — `rstest` dev-dependency
Add `rstest = "0.23"` to `[dev-dependencies]` (latest 0.x at time of writing;
`rstest` is not currently in the workspace, so pick a concrete version rather than
matching a nonexistent policy — thermo finding). Named `#[case]` parametrization,
no proc-macro of our own (design §2).

### 9. `tests/reborn_integration_backend_matrix.rs` (DONE — drafted on-branch)
- `#[rstest] #[case(InMemory)] #[case(LibSql)] backend_parity_replies_to_greeting`
  — the canonical matrix exemplar (§7 parity self-test).
- `libsql_persists_reply_across_reopen` — `LibSql`-only durability (§3.8 guardrail).
- Standard `#[path]` module preamble + `#[allow(dead_code)]` on includes (siblings).

### 10. Docs
- `tests/support/reborn/CLAUDE.md`: move `StorageMode::LibSql` + the backend
  matrix from "Planned" to implemented; note the integration tier now rides one
  composite (thread + turn) at `/tenants/...`, that the binary tier is unchanged,
  and that product/auth/etc. join the composite in later §3.8 slices.
- Design spec §3.2 / §9: mark step 4 done; record the Option-C decision (one
  composite; `StorageMode` = mounted-backend selection; integration thread+turn
  migrated; product deferred) and the production visibility promotions.

---

## Verification (commit-checkpointed; long clippy LAST)

1. `cargo test --test reborn_integration_backend_matrix` — both `#[case]`s + the
   reopen test green. **Commit.**
2. Regression (the migrated integration default + the untouched binary tier):
   `cargo test --test reborn_integration_greeting --test reborn_integration_tool_call --test reborn_integration_http_matcher`,
   and the binary-E2E reborn tests that exercise `RebornBinaryE2EHarness` /
   `RebornHarnessSharedStorage` (e.g. `reborn_turn_state_lock_free_submit_parity`,
   `reborn_recorded_trace_parity`). **Commit.**
3. `cargo test -p ironclaw_reborn_composition` — production promotions
   behavior-preserving.
4. `cargo fmt --check`.
5. **Open the PR before the long gate** (survives a watchdog kill on the gate).
6. `cargo clippy --all --tests --examples --all-features -- -D warnings`
   (all-features lane catches shared-support dead-code).

---

## Blast radius & risks (called out, not hidden)

- **Binary E2E tier must stay byte-identical.** The store harnesses are shared
  (`harness.rs:849,200,1376,3712`). The generic-with-default param (§4),
  the separate `scoped_turns_fs_composite` (§5), and leaving
  `RebornProductWorkflowHarness` untouched (§6) keep the binary tier compiling and
  behaving exactly as today. Verification step 2 re-runs the binary-tier tests to
  prove it.
- **The integration InMemory default changes wiring** (three backends → one
  composite). Required for one storage truth; re-verified by step 2. Parity
  becomes *structural* (both modes: identical composite + paths, differ only at
  the mounted backend).
- **Reuse prod truth, don't copy it.** Database-roots construction comes from the
  §2b accessor; mounts from the §2 accessor. Only the turn/thread *scope* path
  strings (`/tenants/.../{turns,threads}`) are written harness-side, because no
  promotable per-store scope helper exists for them (the production turn mount at
  `factory.rs:5123–5311` is wired inline in `build_reborn_runtime`, not a
  promotable helper). If a clean promotion surfaces during implementation, prefer
  it.
- **`harness.rs` size.** This slice must not add to `harness.rs` (already 3822
  lines > architecture.md §5 threshold). New composite turn-scoping lives in the
  integration support (builder.rs / storage.rs). If any edit to `harness.rs` is
  unavoidable, keep it to the visibility/alias level and add a
  `// arch-exempt: large_file, …, plan <this doc>` note; the file's decomposition
  is a pre-existing tracking item, not this slice's to fix.
- **`build_reborn_runtime` is deliberately not the seam.** It starts the full
  production runtime (LLM config, runtime policy, trigger poller, scheduler) and
  cannot take the scripted raw provider the harness injects at
  `DefaultPlannedRuntimeParts`. The harness mirrors only the *storage* builder
  (`build_default_local_dev_database_roots`), which is the correct altitude.

## Deferred (NOT this slice)
`StorageMode::Postgres` (with the CI container lane, §8 — one `#[case]` + a builder
arm mirroring `LocalDevStorageBackendInput::Postgres`); product/auth/approval/
install/skill/secret coverage tests + their migration onto the composite (§6, the
wiring §3.8 unlocks); the pre-commit test-style grep.
