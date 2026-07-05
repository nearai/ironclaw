# Iteration Log - Hosted Single-Tenant Postgres Latency

Started: 2026-07-05
Budgets: 10 hours wall-clock / $0 spend

## Cycle 0 - Harness Bootstrap

- Score (dev): not yet measured
- Probe gap: not yet measured
- Hypothesis: A standalone storage-level harness over the real
  `RootFilesystem` implementations will expose the largest Postgres latency
  gaps before the full hosted WebUI/profile workload is wired.
- Expected failure mode: The first harness is narrower than the final goal and
  could overfit storage hot paths while missing startup/session overhead.
- Diagnostic: The harness must report dev-only status, real libSQL/Postgres
  histograms, and explicit TODO coverage gaps rather than claiming acceptance.
- Change: Add `spec.md`, `harness/latency`, and an initial Rust runner.
- Result: Runner compiles with `cargo check --manifest-path
  harness/latency/runner/Cargo.toml`. `harness/latency/score.sh --dev`
  runs against local Postgres and reports real histograms/state hashes. First
  standard dev score (5 warmups, 40 samples, concurrency 1 and 4) shows
  Postgres passing put/get and query hot paths, but failing append-tail hard
  thresholds: concurrency 4 p95 is about 2.7x libSQL and throughput about 17%
  of libSQL in that run. Reserve-sequence has p99 variance at concurrency 1 but
  passes concurrency 4.
- Reflection: The next change should target Postgres event append/tail shape
  before broader hosted-profile coverage. The harness is still dev-only and is
  not acceptance-ready because launch-ref baseline, WebUI/session, turns,
  triggers, approvals, secrets, and resources are not wired yet.

## Cycle 1 - Invalid Pool-Sizing Detour

- Score (dev): append_tail p95 fails hard threshold in standard dev run when
  the Postgres pool falls back to 2 connections; the same scorer passes all
  dev workloads with `IRONCLAW_REBORN_POSTGRES_POOL_MAX_SIZE=16`.
- Probe gap: not yet measured with path/payload perturbations.
- Hypothesis: The first append-tail regression is pool serialization in the
  hosted Postgres profile, not the `root_filesystem_events` schema.
- Expected failure mode: Optimizing append IDs or batching could accidentally
  change event replay semantics, hide durable writes, or only improve the
  synthetic harness path.
- Diagnostic: Inspect `PostgresRootFilesystem::append_batch`, `tail_bounded`,
  and `V30__root_filesystem_events.sql`; compare query plans and round trips
  before changing schema or SQL.
- Change: Reverted the Reborn Postgres default pool size, generated config
  hints, Docker production config, docs, tests, and latency harness fallback
  back to 2 because the active goal requires scoring pool size 1 and 2 and
  explicitly voids scores that raise pool size to win.
- Result: SQL plans for existing event paths are sub-millisecond
  (`append_batch` about 0.35 ms, `tail_bounded` about 0.11 ms on the local
  probe). The pool-16 score is useful diagnostic evidence only, not a valid
  optimization result for this goal.
- Reflection: The next valid cycle must improve pool-size-1/2 behavior without
  changing the scoring pool cap. Candidate approaches are reducing checkout
  count per operation pair, collapsing append+tail round trips where the public
  contract allows it, improving transaction/query shape, or schema/index
  changes that reduce per-connection hold time.

## Cycle 2 - Encode Pool-Cap Scoring In Harness

- Score (dev): pool-size-1 dev score passes current storage workloads; pool-size-2
  dev/probe runs expose invalid libSQL baseline errors in `query_exact` under
  concurrent writer pressure (`bad parameter or other API misuse`), which makes
  the state hash comparison fail for the wrong reason.
- Probe gap: current harness still measures storage hot paths only. It does not
  yet run the launch-ref hosted-volume worktree or hosted WebUI/session/turn/
  trigger/approval/secret/resource paths.
- Hypothesis: The scorer must make pool size an explicit comparison dimension
  before production optimization, otherwise a passing run can accidentally use
  an out-of-policy Postgres pool and look valid.
- Expected failure mode: Adding a pool dimension can multiply baseline work and
  hide flaky libSQL baseline errors if each comparison reuses a different
  baseline sample.
- Diagnostic: Run libSQL baseline once per scorer invocation, run Postgres once
  for each configured scoring pool size, and compare every Postgres pool result
  to the same libSQL row for the workload/concurrency tuple.
- Change: Added `LATENCY_POSTGRES_POOL_SIZES` with default `1,2`; the runner
  now executes one libSQL baseline and separate Postgres runs for every scored
  pool size. Result rows include `postgres_pool_size`, comparison rows include
  the same dimension, and the harness README/scripts document that raising the
  pool size is diagnostic-only.
- Result: `cargo fmt --manifest-path harness/latency/runner/Cargo.toml --check`
  and `cargo check --manifest-path harness/latency/runner/Cargo.toml` pass.
  `harness/latency/score.sh --dev` with local Postgres reports
  `postgres_pool_sizes: [1, 2]`; both pool sizes pass all current storage-only
  dev comparisons in that run with zero errors and matching state hashes.
- Reflection: The scorer is now harder to game, but this is still not an
  acceptance result. The next useful cycle should either add hosted
  single-tenant profile coverage or inspect the real Postgres schema/write path
  under the hosted workload before choosing row-based schema changes.

## Cycle 3 - LFD Goal And Constraint Scaffold

- Score (dev): `harness/latency/score.sh --dev` runs through lint and reports
  `postgres_pool_sizes: [1, 2]`, but this sample has libSQL baseline errors at
  concurrency 4 (`bad parameter or other API misuse`) in `put_get` and
  `query_exact`, producing hard-fail state-hash comparisons for both Postgres
  pool sizes even though Postgres itself has zero errors.
- Probe gap: `harness/latency/probe.sh` repeats the same baseline issue at
  `query_exact` concurrency 8; Postgres remains zero-error and faster on the
  current storage-only workloads, but rows hard-fail because the libSQL state
  hash is computed over fewer successful samples.
- Hypothesis: The loop needs the LFD target, cheat fences, and lint/status
  instruments before production optimization; otherwise the optimizer can
  drift into dev-score victory, pool-size gaming, or harness edits that change
  the target.
- Expected failure mode: A lint that is too specific becomes an oracle, or a
  lint that is too broad voids legitimate production code.
- Diagnostic: Lint must run before score, report only `VOID: constraint
  violation` on policy failure, and allow the existing source tree to score.
- Change: Added `goal.md`, added `harness/latency/lint.sh`, wired `score.sh`
  to call it, and expanded `status.sh` with pool-size/env/worktree signals.
- Result: `harness/latency/status.sh`, `harness/latency/score.sh --dev`, and
  `harness/latency/probe.sh` all run. The lint does not void the current tree.
  The dev/probe hard failures are attributable to libSQL baseline errors, not
  Postgres errors.
- Reflection: Do not change the Postgres schema based on the current
  storage-only dev scores; Postgres is already row-shaped for events/sequences
  and uses typed `Entry` rows with JSONB indexes. The next cycle should wire a
  hosted-profile workload or stabilize the launch-ref libSQL baseline so the
  comparison is meaningful.

## Cycle 4 - Hosted Substrate Build Workload

- Score (dev): storage-only harness currently has noisy libSQL baseline errors
  under concurrent dev/probe writes, while Postgres itself reports zero errors.
- Probe gap: storage-only probe does not exercise hosted runtime substrate
  construction, readiness validation, secrets/resources/approvals wiring, or
  profile-specific production service setup.
- Hypothesis: Adding a production-shaped hosted substrate build/readiness
  workload will expose profile-level Postgres overhead before any schema
  changes, while reusing existing deterministic composition seams.
- Expected failure mode: Pulling in composition dependencies could accidentally
  use test-only helpers, live network/model providers, or a looser runtime
  profile than hosted single tenant needs.
- Diagnostic: The runner must build libSQL and Postgres production host runtime
  services through exported composition APIs, use recording sandbox/wake fakes,
  require production wiring validation, and keep `acceptance_ready=false` until
  launch-ref/WebUI/turn workloads are added.
- Change: Added a production-shaped `hosted_substrate_build` workload that
  builds libSQL and Postgres host-runtime services through exported
  composition APIs with deterministic fake process/wake seams and production
  wiring validation. Added error-chain capture and workload filtering to the
  runner. Fixed a concurrent Postgres substrate correctness failure by taking a
  transaction-scoped advisory lock around root filesystem migrations, then
  removed one duplicate Postgres migration pass from the hosted substrate
  builder by reusing the already-migrated root filesystem for Reborn event
  stores.
- Result: `cargo fmt -p ironclaw_reborn_event_store -p
  ironclaw_reborn_composition -p ironclaw_filesystem --check` passes.
  `cargo check --manifest-path harness/latency/runner/Cargo.toml` passes with
  the existing `OutboundDeliveryTargetEntry` unused-import warning in
  composition. `cargo test -p ironclaw_filesystem --features libsql,postgres`
  passes (198 tests/doc-tests across filesystem targets). `cargo test -p
  ironclaw_reborn_composition --features postgres postgres_substrate --test
  postgres_substrate` passes (4 tests). The focused post-change score
  `LATENCY_WORKLOADS=hosted_substrate_build LATENCY_WARMUP=0
  LATENCY_SAMPLES=12 LATENCY_CONCURRENCY=3 harness/latency/score.sh --dev`
  reports zero errors and matching state hashes. LibSQL p50/p95 is
  21.17/31.87 ms. Postgres pool 1 is 55.31/68.77 ms (p50 ratio 2.61, p95
  ratio 2.16, throughput ratio 0.38). Postgres pool 2 is 39.73/52.97 ms (p50
  ratio 1.88, p95 ratio 1.66, throughput ratio 0.51). Both Postgres pool sizes
  still hard-fail the dev scorer for this workload.
- Reflection: The advisory lock fixed the correctness hole under concurrent
  startup, and removing the duplicate event-store migration improved the pool 2
  startup path, but hosted substrate build latency is still far from the libSQL
  baseline at the required pool sizes. The remaining gap appears to be cold
  service construction/migration/write amplification, not evidence that the
  root filesystem should move from blob-style storage to a row-per-domain
  schema yet. Next cycle should split cold migration cost from warm
  hosted-request paths and measure which production stores still issue startup
  writes during every substrate build.

## Cycle 5 - Postgres Migration Memoization

- Score (dev): Before this cycle, focused `hosted_substrate_build` at
  concurrency 3 was zero-error but still hard-failed: pool 1 p50/p95 ratios
  were 2.61/2.16 and pool 2 p50/p95 ratios were 1.88/1.66.
- Probe gap: The hosted substrate workload measures repeated service graph
  construction inside one process. It is useful for startup overhead, but still
  not a full hosted WebUI/session/turn acceptance path.
- Hypothesis: The remaining Postgres gap is repeated idempotent root
  filesystem migration work in the same process/database, not the row shape of
  runtime data. A per-database/schema migration success memo should preserve
  first-run advisory-lock safety while avoiding repeated `CREATE IF NOT EXISTS`
  batches.
- Expected failure mode: A process-global memo could skip migrations for a
  different database, schema, or server if keyed too broadly, or could mark a
  schema migrated before the transaction commits.
- Diagnostic: Key the memo by server host/port, database, and current schema;
  insert only after the migration transaction commits; rerun filesystem
  contracts, Postgres substrate tests, and the pool-size-1/2 dev score.
- Change: Added process-local Postgres root filesystem migration memoization
  keyed by `inet_server_addr`, `inet_server_port`, `current_database`, and
  `current_schema`. The first caller for a key still takes the transaction
  advisory lock and runs the full schema batch; later callers in the same
  process skip the batch after checking the database identity.
- Result: `cargo fmt -p ironclaw_filesystem -p ironclaw_reborn_event_store -p
  ironclaw_reborn_composition --check` passes. `cargo check --manifest-path
  harness/latency/runner/Cargo.toml` passes with the pre-existing composition
  unused-import warning. `cargo test -p ironclaw_reborn_composition --features
  postgres postgres_substrate --test postgres_substrate` passes (4 tests).
  `cargo test -p ironclaw_filesystem --features libsql,postgres --
  --test-threads=1` passes (198 tests/doc-tests across filesystem targets).
  Focused `hosted_substrate_build` at concurrency 3 now passes for both scored
  pool sizes: libSQL p50/p95 21.39/31.34 ms, Postgres pool 1 17.45/18.05 ms
  (p50 ratio 0.82, p95 ratio 0.58), and Postgres pool 2 18.09/20.89 ms (p50
  ratio 0.85, p95 ratio 0.67), all zero-error with matching state hashes. The
  full dev score also shows hosted substrate passing at concurrency 1 and 4
  for pool sizes 1 and 2; the only hard failures are `query_exact` concurrency
  4 comparisons where the libSQL baseline has one `bad parameter or other API
  misuse` error and a mismatched state hash while Postgres remains zero-error.
- Reflection: The Postgres hosted-substrate timing gap is closed in the dev
  harness without increasing pool size or changing benchmark semantics. The
  next blocker is harness/baseline hygiene: stabilize or isolate the libSQL
  concurrent `query_exact` baseline so the full score can distinguish real
  Postgres regressions from baseline state-hash noise. Full acceptance still
  requires launch-ref and hosted request/turn coverage before making
  `goal.md`, `spec.md`, or the harness read-only.

## Cycle 6 - Stabilize Setup And Add Trigger Store Coverage

- Score (dev): Current committed dev score passes every comparison for
  Postgres pool sizes 1 and 2 with zero Postgres errors and matching state
  hashes.
- Probe gap: `probe.sh` can still produce libSQL baseline
  `bad parameter or other API misuse` errors under high-concurrency
  `put_get`/`query_exact`, causing state-hash hard failures even when
  Postgres has zero errors and matching semantics. One probe invocation also
  exited with `Trace/BPT trap`, but an immediate rerun completed, pointing to
  an intermittent libSQL/runtime baseline issue rather than deterministic
  Postgres behavior.
- Hypothesis: The remaining probe failures are shared-prefix setup noise,
  fixed-parent path races, and mixed query/write semantics: measured samples
  lazily materialize the same workload directories and `query_exact` writes
  seed records inside the timed span. Pre-creating each sample's prefix/fixed
  parent during setup and making `query_exact` read-only after setup should
  remove the parent-directory race and libSQL concurrent writer noise without
  changing the measured durable leaf writes for `put_get`, `append_tail`, or
  `reserve_sequence`.
- Expected failure mode: Pre-creating too much of the sample path or moving the
  wrong writes out of the timed span would remove real durable work from
  `put_get`, `query_exact`, `append_tail`, or `reserve_sequence` and make the
  harness easier than production. Only the workload prefix, fixed parent
  directory, and query fixture records may be created during setup;
  sample-specific put/get entries, append streams, and sequence rows must still
  be written inside the measured span.
- Diagnostic: Rerun `probe.sh` and confirm libSQL baseline errors/state-hash
  mismatches disappear without any Postgres regression. Add a durable trigger
  repository workload to cover the next hosted-profile row store rather than
  continuing to tune only filesystem hot paths.
- Change: Added bounded setup retries for workload prefix/fixed-parent
  creation and index creation, moved `query_exact` seed records into setup so
  the timed operation is a pure indexed query, and added `trigger_seed_list`
  over the real libSQL and Postgres `TriggerRepository` implementations. The
  trigger workload upserts a scheduled trigger, lists by tenant, and lists by
  scoped tenant/user/agent/project, with state isolated by backend, pool size,
  run ID, and sample so pool-size comparisons do not share durable rows. The
  trigger Postgres repository now uses deadpool's per-connection
  `prepare_cached` path for the hot fixed upsert/list/list-scoped SQL, matching
  the existing filesystem Postgres optimization pattern and avoiding a parse
  round trip on every trigger management operation.
- Result: `cargo fmt -p ironclaw_triggers --check` and `cargo fmt
  --manifest-path harness/latency/runner/Cargo.toml --check` pass after
  formatting. `cargo check --manifest-path harness/latency/runner/Cargo.toml`
  passes with the pre-existing `OutboundDeliveryTargetEntry` unused-import
  warning. `cargo test -p ironclaw_triggers --features libsql,postgres` passes
  (149 tests/doc-tests). A focused `trigger_seed_list` score at concurrency 4
  passes with zero errors and matching state hashes; after cached statements,
  Postgres pool 1 p50/p95 is 2.06/2.98 ms versus libSQL 2.70/5.37 ms, and
  Postgres pool 2 p50/p95 is 1.01/1.37 ms. A focused `put_get` rerun with the
  required default pool set 1 and 2 passes after a previous full-run pool-2 p99
  outlier; a pool-size-2-only diagnostic run was correctly voided by lint. The
  final full dev score passes all expanded workloads (`put_get`,
  `query_exact`, `append_tail`, `reserve_sequence`, `trigger_seed_list`, and
  `hosted_substrate_build`) for Postgres pool sizes 1 and 2 with zero errors,
  matching state hashes, and no hard failures. `probe.sh` also completes with
  zero errors, matching state hashes, and no hard failures across concurrency
  1, 3, and 8.
- Reflection: The libSQL baseline flake is no longer blocking dev/probe signal,
  and the scorer now covers one real row-based hosted store beyond the root
  filesystem and substrate construction. The next cycles should add launch-ref
  baseline capture, WebUI/session readiness, local-runtime turn
  admission/queue/resume/cancel, and approvals/secrets/resource snapshot paths
  before treating the harness as acceptance-ready.

## Cycle 7 - Control-Plane Snapshot Coverage

- Score (dev): Current committed dev score/probe pass expanded storage,
  trigger, and hosted substrate workloads for Postgres pool sizes 1 and 2 with
  zero errors, matching state hashes, and no hard failures.
- Probe gap: The harness still does not time the persisted approval request,
  secret metadata/lease, or resource governor snapshot paths as actual
  workload operations. `hosted_substrate_build` validates production wiring,
  but it does not mutate these stores inside the measured span.
- Hypothesis: Adding a combined filesystem-backed control-plane workload will
  expose the next blob-style JSON snapshot/CAS paths the hosted profile uses:
  `FilesystemApprovalRequestStore`, `FilesystemSecretStore`, and
  `PersistentResourceGovernor<FilesystemResourceGovernorStore>`. If Postgres
  still matches libSQL here, the next bottleneck is more likely request/server
  orchestration than row-vs-blob schema for these stores.
- Expected failure mode: A synthetic workload could accidentally benchmark
  in-memory stores, bypass `ScopedFilesystem` mount routing, or move durable
  mutations into setup. It must construct the same filesystem-backed stores
  over the real libSQL/Postgres root filesystems, use valid resource scopes and
  mount aliases, and perform the approval/secret/resource writes inside the
  timed span.
- Diagnostic: Add the workload, run its focused score at the required pool
  sizes, then run full dev score and probe. If the control-plane workload hard
  fails, inspect which store dominates before changing schema or query shape.
- Change: Added `control_plane_snapshot`, a timed workload that saves and
  approves a durable approval request, stores and consumes a one-shot secret
  lease, and sets/reserves/reconciles a resource-governor account through the
  hosted filesystem-backed stores. The workload keeps setup empty for these
  operations so the durable mutations remain inside the measured span.
- Result: The runner compiles with `cargo check --manifest-path
  harness/latency/runner/Cargo.toml`. A focused score
  (`LATENCY_WORKLOADS=control_plane_snapshot LATENCY_WARMUP=1
  LATENCY_SAMPLES=20 LATENCY_CONCURRENCY=1,4`) exposed a real Postgres
  control-plane failure: concurrency 1/pool 1 passed with matching state hash,
  but concurrency 4 hit `secret lease consume retry limit exceeded`; pool 2
  was also too slow at concurrency 1 and hit the same consume retry class at
  concurrency 4.
- Reflection: The added workload is useful and should stay in the loop. It
  confirms the user's concern that some control-plane paths are still
  blob/CAS-shaped rather than row-shaped for Postgres. The first schema target
  should be secrets, because that is the failing operation before the resource
  governor is isolated.

## Cycle 8 - Postgres Secret Rows

- Score (dev): Cycle 7 focused score fails `control_plane_snapshot` on
  Postgres secret lease consumption under concurrency and shows pool-2
  single-concurrency latency well above libSQL.
- Probe gap: This is still a focused dev workload, not full hosted WebUI/turn
  acceptance. It covers real approval/secret/resource persistence but not the
  server request path.
- Hypothesis: Moving Postgres secrets from generic filesystem records to
  native `ironclaw_secret_records` and `ironclaw_secret_leases` rows will remove
  the secret lease CAS retry failure and let the combined workload reveal the
  next bottleneck, likely the resource governor's single JSON snapshot.
- Expected failure mode: A row store could weaken tenant/user/project lease
  isolation, expose secret material, or diverge from `SecretStore` one-shot
  semantics. The implementation must keep encrypted material only in the row
  payload, validate full `ResourceScope` after reads, lock the lease row during
  consume/revoke, and keep libSQL on the existing store.
- Diagnostic: Wire the row store only for the Postgres latency path first,
  rerun the focused control-plane score, and inspect the first failing store.
- Change: Added `PostgresSecretStore` behind `ironclaw_secrets/postgres` with
  one row per secret and one row per lease, row-level `FOR UPDATE` on
  consume/revoke, and the same `SecretStore` trait surface. The latency runner
  now uses this row-backed secret store only for Postgres; libSQL remains on
  `FilesystemSecretStore`.
- Result: `cargo check --manifest-path harness/latency/runner/Cargo.toml`
  passes with the pre-existing `OutboundDeliveryTargetEntry` warning. A
  focused five-sample control-plane score confirms the secret consume retry is
  gone: concurrency 1 has zero errors and matching state hashes for Postgres
  pool sizes 1 and 2. The combined span still hard-fails latency
  (`postgres_p95_ratio` about 2.86 for pool 1 and 3.13 for pool 2 in that
  sample). A five-sample concurrency-4 run no longer reports secret-store
  errors; it fails in `resource governor storage error`, with very large
  latencies caused by the filesystem resource governor's single
  `/resources/snapshot.json` CAS path.
- Reflection: Secret rows fixed the first correctness failure class but did not
  make the combined control-plane workload pass. The next valid optimization is
  not more secret tuning; it is a row-based Postgres resource governor or a
  trait change that lets the resource governor persist account/reservation rows
  instead of rewriting one JSON snapshot.

## Cycle 9 - Postgres Resource Rows

- Score (dev): Cycle 8 focused five-sample `control_plane_snapshot` run no
  longer reports secret-store errors, but concurrency 4 fails in `resource
  governor storage error` and shows very large latencies. Concurrency 1 remains
  well slower than libSQL on the combined span.
- Probe gap: This is still dev-only focused control-plane coverage, not
  launch-ref/WebUI/turn acceptance.
- Hypothesis: The resource governor's filesystem store rewrites one
  `/resources/snapshot.json` for all accounts/reservations, so concurrent
  Postgres control-plane samples contend on one CAS blob. A Postgres
  `ResourceGovernor` backed by row-locked account and reservation tables should
  preserve reservation semantics while limiting contention to the affected
  account cascade and reservation row.
- Expected failure mode: A row governor could accidentally weaken cascade
  limits, period rollover, reservation idempotency, or fail-closed storage
  semantics. It must reuse the existing state transition functions, lock
  account rows in deterministic cascade order, lock reservation rows on close,
  and keep libSQL on the existing filesystem-backed governor.
- Diagnostic: Wire the row governor only for the Postgres latency path first,
  run resource crate tests, then rerun focused `control_plane_snapshot` before
  considering production composition wiring.
- Change: Added `PostgresResourceGovernor` behind the resources `postgres`
  feature with row-backed account and reservation tables, deterministic
  account-row locking, reservation-row locking on close, and the existing
  resource state transition functions reused for cascade limits, reconciliation,
  release, and snapshots. The latency runner now uses this governor for the
  Postgres backend only. I also offloaded the synchronous resource operations
  in `control_plane_snapshot` through `spawn_blocking`, predeclared the
  `/secrets` tenant index during setup, and moved the filesystem secret-store
  tenant index to the `/secrets` mount root to avoid per-owner index DDL churn.
- Result: `cargo fmt -p ironclaw_secrets -p ironclaw_resources --check`,
  `cargo fmt --manifest-path harness/latency/runner/Cargo.toml --check`,
  `cargo check --manifest-path harness/latency/runner/Cargo.toml`,
  `cargo test -p ironclaw_secrets --features postgres`, and
  `cargo test -p ironclaw_resources --features postgres` passed. A focused
  `control_plane_snapshot` run with warmup 1, 20 samples, and concurrency 1/4
  passed for Postgres pool sizes 1 and 2 with zero errors and matching hashes.
  Full `harness/latency/score.sh --dev` has zero errors and matching hashes;
  `control_plane_snapshot` passes in the full run for both pool sizes
  (pool 1 c1 p50/p95 8.91/11.36ms vs libSQL 59.71/77.60ms; pool 1 c4
  35.96/40.55ms vs libSQL 240.68/272.85ms; pool 2 c1 10.09/17.56ms; pool 2 c4
  25.00/30.49ms). The full score still has hard-fail flags on existing
  `put_get` and `query_exact` concurrency-1 p99 ratios for pool 2. The perturbed
  probe also has zero errors and matching hashes; `control_plane_snapshot`
  passes through concurrency 8, while `hosted_substrate_build` remains above
  dev thresholds at concurrency 1 for both pool sizes and concurrency 8 for
  pool 2.
- Reflection: The control-plane contention moved from a single filesystem JSON
  snapshot to row-level Postgres state in the harness, and the measured path is
  now faster than the libSQL baseline for this diagnostic workload. This is not
  the goal finish line: production hosted Postgres composition still constructs
  filesystem-backed resources, and acceptance still needs launch-ref baseline
  worktree scoring plus hosted profile startup, WebUI/session, turn
  admission/queue/resume/cancel, and holdout concurrency 1/4/16 runs.

## Cycle 10 - Production Resource Wiring

- Graph: `codebase-memory-mcp` transport is still closed, so this cycle falls
  back to local code reads after one probe.
- Score (dev): Cycle 9 full score/probe have zero errors and matching hashes;
  `control_plane_snapshot` passes for Postgres pool sizes 1 and 2, including
  probe concurrency 8. The remaining probe/dev misses are latency ratio gaps in
  `hosted_substrate_build` and p99 outliers in existing filesystem workloads.
- Hypothesis: The latency harness now uses row-backed Postgres resources, but
  production hosted Postgres still wires `PersistentResourceGovernor` over the
  filesystem blob store. Moving production Postgres composition to
  `PostgresResourceGovernor` should make hosted-substrate construction and real
  host-runtime paths exercise the same lower-contention resource state as the
  diagnostic workload.
- Expected failure mode: The public production service type and generic
  `build_backend_production` builder currently bake in the filesystem governor.
  A careless change could slow or alter libSQL, lose budget event sinks,
  bypass migrations, or break tests that depend on the concrete returned service
  type. Keep libSQL on its existing governor, keep the budget event sink wired,
  run Postgres resource migrations during production assembly, and adjust only
  the hosted Postgres composition path.
- Diagnostic: Compile the composition and CLI with `webui-v2-beta,libsql,postgres`,
  run the Postgres substrate tests that build the public services type, then
  rerun full dev score/probe to see whether `hosted_substrate_build` improves.
- Change: Changed the public Postgres production services alias to use
  `PostgresResourceGovernor`, threaded backend-specific resource governors
  through the substrate-only and full production builders, kept libSQL on
  `PersistentResourceGovernor<FilesystemResourceGovernorStore<_>>`, and added a
  private adapter so both concrete governors still receive the production budget
  event sink. Postgres resource governor migrations now run during production
  assembly. The first attempt called the synchronous migration bridge directly
  from async composition and hung the latency runner; sampling showed the stack
  blocked in `PostgresResourceGovernor::run_migrations`, so production
  composition now runs that migration on `spawn_blocking`.
- Result: `cargo check -p ironclaw_reborn_composition --features
  webui-v2-beta,libsql,postgres`, `cargo check -p ironclaw_reborn_cli --features
  webui-v2-beta,libsql,postgres`, and `cargo test -p
  ironclaw_reborn_composition --features webui-v2-beta,libsql,postgres --test
  postgres_substrate --test libsql_substrate` passed. A one-sample focused
  `hosted_substrate_build` run completed after the `spawn_blocking` fix and
  passed both Postgres pool sizes. Full `harness/latency/score.sh --dev` has
  zero errors and matching hashes; `hosted_substrate_build` passes for pool 1
  and 2 at concurrency 1 and 4, and `control_plane_snapshot` still passes.
  Full-score hard fails remain in filesystem `put_get`, `query_exact`, and
  `append_tail` p95/p99 ratio outliers. `harness/latency/probe.sh` also has
  zero errors and matching hashes; hosted-substrate passes through concurrency
  8 for both pools, and remaining probe hard fails are `query_exact` pool 1/2
  concurrency 1 and `put_get` pool 2 concurrency 1.
- Reflection: The row-backed resource governor is no longer only a harness
  diagnostic; hosted Postgres production assembly now exercises it too. This
  removes the prior hosted-substrate latency gap in dev/probe, but the goal is
  still incomplete because the filesystem blob-store hot paths (`query_exact`,
  `put_get`, and occasionally `append_tail`) dominate remaining hard failures,
  and launch-ref/WebUI/turn holdout acceptance is still missing.

## Cycle 11 - Shared Postgres Query Indexes

- Graph: `codebase-memory-mcp` transport is still closed, so this cycle falls
  back to local code reads after one probe.
- Score (dev): Fresh Cycle 11 baseline `score.sh --dev` and `probe.sh` have
  zero errors and matching hashes. Hosted-substrate and control-plane still
  pass. `query_exact` is the stable hard failure: full score fails it for both
  pools at concurrency 1/4, and probe fails it for both pools at concurrency
  1/3/8. `put_get` and `append_tail` show intermittent p99 outliers, but their
  p50/p95 are usually faster than libSQL.
- Hypothesis: Postgres `ensure_index` creates one global expression index per
  declaring prefix, with the path embedded only in the index name. Repeated
  latency/prod prefixes therefore accumulate many duplicate indexes over the
  same `indexed->>'bucket'` expression. `query_exact` then pays planning and
  sort cost around a path-filtered equality lookup. A single shared
  exact/prefix index keyed by `(indexed projection..., path)` should preserve
  semantics, avoid duplicate DDL/index bloat, and let equality queries return
  path-ordered rows with less planner work.
- Expected failure mode: Index declarations are prefix-scoped for conflict
  detection, but the physical exact/prefix index can be shared only if the
  projection and index kind match. The change must not weaken `ensure_index`
  conflict checks, FTS prefix isolation, range filter correctness, or libSQL
  behavior. Existing duplicate indexes in the local dev database may still
  affect one run until the database is rebuilt or old indexes are dropped, so
  focused fresh-DB checks matter.
- Diagnostic: Change only Postgres exact/prefix physical index naming/DDL,
  add tests for the shared name shape, run filesystem tests with
  `libsql,postgres`, then rerun focused `query_exact` and full dev/probe.
- Change: Postgres exact/prefix `ensure_index` now creates one shared physical
  projection index per spec kind/key/name, with `path` appended as the final
  btree column, instead of creating one prefix-named global expression index
  per declaring prefix. `query` now prepares the generated SQL through
  `prepare_cached`, with paths and values still bound as parameters. Postgres
  filesystem migration cleanup drops legacy prefix-named btree projection
  indexes so existing dev/prod databases stop paying planner cost for duplicate
  `indexed->>'...'` indexes; FTS indexes remain prefix-scoped.
- Result: `cargo test -p ironclaw_filesystem --features libsql,postgres`
  passed. Focused `query_exact` score with concurrency 1/4 passed all
  comparisons: pool 1 p50/p95 dropped to 0.476/0.760ms at c1 and
  0.389/0.606ms at c4; pool 2 dropped to 0.104/0.320ms at c1 and
  0.116/0.241ms at c4. The live Postgres DB has 4
  `root_filesystem_entries` indexes after cleanup, with 2 shared projection
  indexes and 0 legacy projection duplicates. Full `harness/latency/score.sh
  --dev` has zero failing comparisons, zero errors, and matching hashes;
  `query_exact` remains sub-millisecond for both pools and both dev
  concurrencies. `harness/latency/probe.sh` was rerun twice: both runs show
  `query_exact` passing for both pools through concurrency 8, but both are not
  clean probe evidence because the libSQL baseline hit one
  `control_plane_snapshot` concurrency-8 filesystem/secret-store error, causing
  state-hash mismatch for that workload.
- Reflection: The stable Postgres query hard failure is removed in dev score
  and probe query rows. The next cycle should not keep tuning Postgres query;
  it should address the libSQL baseline control-plane instability at probe
  concurrency 8 or move the harness closer to the real launch-ref/WebUI/turn
  acceptance path. This is still not holdout acceptance.

## Cycle 12 - LibSQL Trigger PRAGMA Drain

- Graph: `codebase-memory-mcp` transport is still closed, so this cycle falls
  back to local code reads after one probe.
- Score (dev): Fresh Cycle 12 baseline `score.sh --dev` has one noisy
  `reserve_sequence` p95/p99 hard failure at concurrency 1; Postgres itself is
  zero-error, hashes match, and `query_exact` remains fixed. `probe.sh` is
  clean: zero failing comparisons, zero error rows, and matching hashes.
- Holdout-shaped diagnostic: Local `score.sh --holdout` exposes invalid
  comparisons, but the error rows are in the libSQL baseline:
  `trigger_seed_list` concurrency 4 reports two
  `query tenant trigger records: SQLite failure: bad parameter or other API
  misuse` errors, and `control_plane_snapshot` concurrency 16 reports three
  filesystem secret-store `stat` errors with the same SQLite misuse class.
  Postgres has zero errors in those rows and is much faster for
  `control_plane_snapshot`.
- Hypothesis: The trigger-specific libSQL error is caused by
  `LibSqlTriggerRepository::connect` issuing `PRAGMA busy_timeout` via
  `query()` and dropping the returned row stream before subsequent statements
  on that same connection. The filesystem backend already uses
  `execute_batch()` for connection PRAGMAs, which drains/discards returned
  rows. Matching that pattern should remove the trigger baseline correctness
  failure without changing Postgres or benchmark workload logic.
- Expected failure mode: This may fix only `trigger_seed_list`; the
  `control_plane_snapshot` error can still be the known libSQL driver limit
  around concurrent independent local-file handles. Do not treat a partial
  libSQL-baseline cleanup as Postgres holdout acceptance.
- Diagnostic: Change only the libSQL trigger connection PRAGMA path, run the
  trigger repository tests, then rerun a trigger-focused latency score with
  holdout concurrency and enough samples to reproduce the prior c4 failure.
- Change: Replaced the trigger repository's un-drained
  `conn.query("PRAGMA busy_timeout = 5000", ())` with
  `conn.execute_batch("PRAGMA busy_timeout = 5000;")`, matching the
  filesystem backend's connection-setup pattern. A subsequent full
  holdout-shaped run crashed inside native SQLite/libSQL while concurrent
  trigger tasks were preparing/executing `upsert_trigger` and
  `list_scoped_triggers`, so `LibSqlTriggerRepository` now serializes public
  DB operations behind a repository-local async mutex. The delegating
  `list_active_triggers` method does not take the lock itself; its callee does.
- Result: `cargo fmt -p ironclaw_triggers --check` passed. The full
  `cargo test -p ironclaw_triggers --features libsql,postgres --test
  repository_contract` suite passed twice after the final lock shape (49
  tests). The repeated trigger-focused holdout shape
  (`LATENCY_WORKLOADS=trigger_seed_list`, 30 warmups, 300 samples,
  concurrency 1/4/16) completed without native crashes, has zero error rows,
  and has matching state hashes. One trigger-focused run caught a Postgres
  pool-1 c4 p99 outlier, but the immediate repeat passed all comparisons; a
  focused c1 trigger score with 10 warmups and 120 samples also passed all
  comparisons. Full post-change `harness/latency/probe.sh` is clean. Full
  `score.sh --dev` is not stable evidence yet: one run hit the known libSQL
  filesystem/control-plane `bad parameter or other API misuse` row at
  concurrency 4, and the repeat hit a Postgres trigger p95 outlier that the
  focused c1 check did not reproduce.
- Reflection: This cleans up the libSQL trigger baseline correctness failure
  and native crash without touching Postgres or relaxing score policy. It is
  still not acceptance: the libSQL filesystem/control-plane misuse remains
  unresolved under dev/holdout concurrency, and the goal still lacks
  launch-ref hosted-volume, WebUI/session, and turn-path acceptance.
