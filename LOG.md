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

## Cycle 13 - Single-Query Postgres Stat

- Graph: `codebase-memory-mcp` transport is still closed, so this cycle falls
  back to local code reads after one probe.
- Score (dev): Fresh Cycle 13 baseline `score.sh --dev` has zero error rows
  and matching state hashes, but p95/p99 hard-fail outliers on Postgres pool 1
  across tiny filesystem and trigger workloads (`put_get`, `query_exact`,
  `append_tail`, `reserve_sequence`, `trigger_seed_list`). The absolute
  failures are tail-only; p50 and throughput are generally faster than libSQL.
- Probe gap: Fresh `probe.sh` is clean: zero failing comparisons and zero
  error rows. In probe, Postgres pool 1 filesystem p95/p99 rows stay low
  (usually ~0.5-2.9ms for the storage hot paths), which suggests the dev score
  outliers are not a stable semantic or schema failure.
- Hypothesis: `PostgresRootFilesystem::stat` still performs an exact-row
  lookup and then a descendant lookup on misses/implicit directories. The
  control-plane and upcoming WebUI/session paths rely on metadata probes; on
  pool size 1 those extra round trips increase tail exposure and connection
  hold time even though the current storage-only probe does not fail. A single
  query that returns exact file/dir metadata or an implicit-directory marker
  should preserve semantics while reducing one common metadata hot path.
- Expected failure mode: The query must still prefer exact entries over
  implicit descendants, preserve file length/updated_at handling, return
  `NotFound` only when neither exact nor child rows exist, and avoid changing
  libSQL baseline behavior.
- Diagnostic: Change only Postgres `stat`, run filesystem tests with
  `libsql,postgres`, then rerun focused control-plane/stat-adjacent latency
  checks plus the required dev score/probe.
- Change: `PostgresRootFilesystem::stat` now uses one cached query that
  returns the exact entry when present, otherwise one implicit-directory
  descendant marker. The query preserves exact entry precedence, file length,
  directory length, `updated_at`, and `NotFound` behavior while removing the
  second round trip on misses/implicit directories.
- Result: `cargo fmt -p ironclaw_filesystem --check` passed.
  `cargo test -p ironclaw_filesystem --features libsql,postgres` passed
  (unit, catalog, DB root filesystem, filesystem contract, and doc tests).
  Focused `control_plane_snapshot` with 10 warmups, 120 samples, and
  concurrency 1/4 passed with zero errors and matching hashes; Postgres pool 1
  measured p95 9.36ms at c1 and 36.22ms at c4, and pool 2 measured p95
  9.52ms at c1 and 28.63ms at c4. Full post-change `score.sh --dev` had zero
  errors and matching hashes, with one non-reproduced `append_tail` pool-2 c4
  p95/p99 outlier. Post-change `probe.sh` had zero errors and matching hashes,
  with one non-reproduced `put_get` pool-1 c1 p99 spike; a focused
  `put_get` c1 rerun with 10 warmups and 120 samples passed all comparisons.
  Full `cargo test` for `ironclaw_reborn_composition` and
  `ironclaw_reborn_cli` could not complete because the local filesystem ran out
  of disk during test linking/WebUI output generation. After removing generated
  `target/debug` build artifacts, `cargo check -p ironclaw_reborn_composition
  --features webui-v2-beta,libsql,postgres` and `cargo check -p
  ironclaw_reborn_cli --features webui-v2-beta,libsql,postgres` both passed
  with the pre-existing `OutboundDeliveryTargetEntry` unused-import warning.
- Reflection: The stat metadata path is now one round trip on Postgres and the
  focused control-plane path stays comfortably faster than libSQL. The broader
  dev/probe evidence still shows intermittent tail spikes that do not
  reproduce in focused reruns, and the full goal remains incomplete: launch-ref
  hosted-volume, WebUI/session, turn-path, and holdout acceptance are still not
  proven.

## Cycle 14 - Postgres Resource Migration Memoization

- Graph: `codebase-memory-mcp` transport is still closed, so this cycle falls
  back to local code reads after one probe.
- Score (dev): Fresh Cycle 14 `score.sh --dev` has zero errors and matching
  hashes. The only hard failures are `trigger_seed_list` concurrency 1 p99
  outliers for Postgres pool 1 and 2; p50 and throughput are faster than
  libSQL, and the probe does not reproduce the trigger outliers.
- Probe gap: Fresh `probe.sh` has zero errors and matching hashes, but
  `hosted_substrate_build` is consistently just over the dev thresholds:
  Postgres pool 1 concurrency 1 is 16.60/17.59/18.08ms p50/p95/p99 versus
  libSQL 13.70/13.99/14.09ms, pool 1 concurrency 3 throughput is 89.7% of
  libSQL, and pool 2 concurrency 1 is 16.47/17.34/17.49ms versus the same
  libSQL baseline.
- Hypothesis: Postgres production/substrate builders still run
  `PostgresResourceGovernor::run_migrations()` on every construction, while
  `PostgresRootFilesystem::run_migrations()` is already memoized per database
  schema. Hosted-substrate build repeatedly constructs services in one
  process; skipping already-successful resource DDL for the same
  database/schema should remove a few milliseconds from warm production
  construction without changing runtime behavior or score pool sizes.
- Expected failure mode: A process-global memoization key that is too broad
  could skip migrations for a different Postgres database/schema in tests or
  multi-tenant local runs. The key must distinguish the configured Postgres
  target without retaining the raw connection secret in memory, and the
  migration must only be marked complete after `run_migrations()` succeeds.
- Diagnostic: Add composition-local resource-migration memoization, use it in
  both Postgres production builders, run composition/CLI checks, and rerun
  hosted-substrate focused score plus full dev/probe.
- Change: Postgres production and hosted-substrate builders now route resource
  governor DDL through a process-global success registry keyed by a SHA-256
  digest of the configured Postgres URL. The first builder for a target still
  runs `PostgresResourceGovernor::run_migrations()`; later builders in the same
  process skip the already-successful resource migration.
- Result: `cargo fmt -p ironclaw_reborn_composition --check`,
  `cargo check -p ironclaw_reborn_composition --features
  webui-v2-beta,libsql,postgres`, and `cargo check -p ironclaw_reborn_cli
  --features webui-v2-beta,libsql,postgres` passed with the pre-existing
  `OutboundDeliveryTargetEntry` unused-import warning. Focused
  `hosted_substrate_build` score reruns had zero errors and matching hashes;
  the hard outlier disappeared, but the path still misses dev thresholds with
  Postgres around 1.15-1.18x libSQL p50/p95 and 0.86-0.88x libSQL throughput at
  c1/c3. Full `score.sh --dev` had only a recurring `trigger_seed_list` c1
  tail outlier. Full `probe.sh` exposed a libSQL control-plane filesystem
  error at c8 and one hosted-substrate pool-1 c3 tail outlier that did not
  reproduce in the focused hosted score.
- Reflection: Removing repeated resource-governor DDL is a real hosted build
  win and keeps correctness stable, but it is not enough to hit libSQL timings.
  The remaining hosted-substrate gap now looks like steady production
  construction overhead rather than migration DDL alone, so the next cycle
  should profile the hosted builder around store/secret/config construction
  before touching schema again.

## Cycle 15 - Postgres Root Migration Front-Guard

- Graph: `codebase-memory-mcp` tools are not exposed in this session and the
  prior transport probes failed closed, so this cycle continues with targeted
  local reads.
- Score gap: After Cycle 14, focused `hosted_substrate_build` has zero errors
  and matching hashes, with no hard failures, but Postgres still sits around
  1.15-1.18x libSQL p50/p95 and 0.86-0.88x libSQL throughput at c1/c3.
- Hypothesis: `PostgresRootFilesystem::run_migrations()` already memoizes DDL
  internally, but it still has to open a connection and query
  `current_database()`/`current_schema()` on every hosted-substrate build to
  find the memoization key. The hosted Postgres builder already receives the
  configured event-store URL; adding a composition-level success front-guard
  keyed by a secret-safe digest of that URL should skip the connection checkout
  entirely after the first successful root filesystem migration.
- Expected failure mode: The guard must not mark a target migrated before
  `run_migrations()` succeeds, must not retain the raw URL, and must not
  increase the configured pool sizes. The key is URL-based, so correctness
  relies on schema-affecting options such as `search_path` being represented in
  the URL; this matches the Cycle 14 resource-governor guard.
- Diagnostic: Add the front-guard to the Postgres production substrate and
  full production builders, keep libSQL untouched, run composition/CLI checks,
  and rerun focused hosted-substrate score before deciding whether to commit.
- Change: Added a composition-level Postgres root-filesystem migration
  success registry keyed by the same SHA-256 URL digest used for the
  resource-governor migration guard. Both Postgres production builders now run
  the real root filesystem migration once per target and skip the later
  connection/key discovery path after success.
- Result: `cargo fmt -p ironclaw_reborn_composition --check`,
  `cargo check -p ironclaw_reborn_composition --features
  webui-v2-beta,libsql,postgres`, and `cargo check -p ironclaw_reborn_cli
  --features webui-v2-beta,libsql,postgres` passed with the pre-existing
  `OutboundDeliveryTargetEntry` unused-import warning. Focused
  `hosted_substrate_build` score with 30 warmups, 300 samples, and c1/c3 passed
  with zero errors, matching hashes, and no dev failures; Postgres measured
  ~10.9-11.8ms p50/p95 versus libSQL ~13.6-15.2ms, with Postgres throughput
  higher for both pool sizes. Full post-change `score.sh --dev` and
  `probe.sh` both passed with zero failures and zero error rows.
- Reflection: The hosted-substrate gap was dominated by repeated Postgres
  migration key discovery rather than schema DDL itself. The benchmark-visible
  behavior now hits the hosted-substrate timing target without increasing pool
  sizes or slowing libSQL, but the larger slash goal remains incomplete until
  launch-ref, WebUI/session, turn-path, and holdout acceptance are run cleanly.

## Cycle 16 - Stress E2E Thread Transaction Pool Deadlock

- Stress signal: Per user request, switched validation from the latency harness
  to `tools/ironclaw_stress` E2E user-turn flows. A tiny libSQL
  `mixed-user-session` smoke with `memory-persist-on-block` completed 10/10
  operations in 148ms. The same Postgres smoke with pool size 2 wedged for more
  than 75s.
- Diagnosis: `pg_stat_activity` showed both Postgres pool connections idle in
  transaction after a `root_filesystem_entries` `SELECT`, while Rust tasks were
  waiting for more pool capacity. The thread service opens a filesystem
  transaction in `try_write_new_message_transactionally`, then calls
  `reserve_sequence`, which checks out a second connection. With concurrency 2
  and pool size 2, both workers hold a transaction connection and wait forever
  for the nested sequence checkout.
- Hypothesis: Move sequence reservation into the active filesystem transaction
  for backends that support it. This keeps message/idempotency/thread/index
  writes atomic, preserves duplicate-idempotency behavior without burning
  sequence numbers, and removes the nested Postgres pool checkout.
- Expected failure mode: The transaction-local sequence operation must stay
  path-local, monotonic, and rollback-safe. Backends that do not implement it
  must continue to fall back to the existing non-transactional path or the
  legacy thread-record CAS path.
- Diagnostic: Add a transaction `reserve_sequence` primitive, implement it for
  Postgres transactions and scoped transactions, use it in transactional thread
  writes, then rerun the exact Postgres stress smoke that wedged.
- Follow-up: The transaction-local sequence patch removed the idle-in-transaction
  Postgres pool deadlock. A second mixed-user stress hang came from the stress
  harness itself: synchronous resource-governor calls were made directly from
  async user-turn tasks, while the Postgres governor bridge waited on async DB
  work. The stress runner now offloads those calls with `spawn_blocking`, and
  Postgres stress is wired to the existing row-based `PostgresResourceGovernor`
  instead of the filesystem snapshot governor.
- Result: `ironclaw_stress mixed-user-session` with
  `memory-persist-on-block`, concurrency 2, pool size 2, model/tool latency 0
  completed cleanly. 10-op smoke: Postgres 38.6ms p95 vs libSQL 34.9ms p95.
  50-op E2E sample: Postgres 27.1ms p95 vs libSQL 51.4ms p95. Postgres top
  local bottleneck is now the row resource governor
  (`resource_governor` p95 17.5ms in the 50-op sample); libSQL remains dominated
  by thread store writes (`thread_store_writes` p95 44.5ms).

## Cycle 17 - Postgres Resource Governor Worker Serialization

- Graph note: `codebase-memory-mcp` was available but its transport closed on
  `list_projects`, so this cycle falls back to targeted local reads.
- Stress signal: `ironclaw_stress mixed-user-session` with
  `memory-persist-on-block`, 16 users, 4 threads per owner, model/tool latency
  0, and at least 300 attempted operations per concurrency level showed
  Postgres pool size 2 succeeding cleanly at c16 but dominated by resource
  accounting: operation p95 104.0ms, resource-governor p95 95.9ms,
  thread-store p95 19.7ms. Pool size 1 also succeeded but was slower:
  operation p95 380.4ms, thread-store p95 196.4ms, resource-governor p95
  134.9ms. The same libSQL stress configuration produced thread-busy/backend
  failures at c4 and segfaulted at c16, so it is not a stable acceptance
  baseline for this diagnostic grid.
- Hypothesis: `PostgresResourceGovernor` is row-based but still serializes every
  operation through one `run_on_worker` current-thread runtime. Under E2E
  concurrency this turns reserve/reconcile into an artificial single-lane
  queue and prevents the configured Postgres pool size 2 from doing useful
  parallel work. Replacing the single worker with bounded parallel blocking
  workers should lower resource-governor p95 without changing persistence
  semantics, operation order within each transaction, or pool size.
- Expected failure mode: Running more than one governor operation at once can
  expose row-lock contention or deadlocks if account rows are locked in
  inconsistent order. The code already uses `ResourceAccount::cascade` order
  consistently, but the retest must include c16 pool-size 1 and 2 stress plus
  resource-governor contract tests.
- Change: Added a bounded `AsyncStorageWorkerPool` helper and moved
  `PostgresResourceGovernor` from one worker to one worker per configured
  deadpool Postgres connection. This keeps synchronous callers off Tokio worker
  threads while allowing pool-size-2 row-governor transactions to overlap.
- Result: c16 pool-size-2 mixed-user stress completed 320/320 operations with
  operation p95 improving from 104.0ms to 86.0ms and resource-governor p95
  improving from 95.9ms to 26.2ms. c16 pool-size-1 also completed 320/320 and
  improved from 380.4ms to 228.3ms p95. c1 remained stable around 9-10ms p95;
  c4 pool-size-2 completed 300/300 at 23.1ms p95 with resource-governor p95
  down to 10.3ms. Remaining c16 pool-size-2 bottleneck is now split across
  row-governor latency and thread/context writes rather than a single
  serialized governor queue.

## Cycle 18 - Holdout LibSQL Connection Setup Flake

- Harness signal: `score.sh --dev` and `probe.sh` both passed with zero
  failures. First `score.sh --holdout` exited 0 but hard-failed three rows:
  `query_exact` c1 pool-size-1 on p95 ratio, plus `put_get` c16 pool-size 1/2
  state-hash mismatches. Focused reruns for `query_exact` c1 and `put_get` c16
  passed, showing the query tail was noise and the put/get mismatch was caused
  by one libSQL baseline error. A second full holdout eliminated those rows but
  hard-failed `control_plane_snapshot` c16 pool-size 1/2 because libSQL again
  had one baseline error. In both holdouts the libSQL error was:
  `filesystem backend infrastructure error during stat: SQLite failure:
  bad parameter or other API misuse`.
- Diagnosis: The libSQL backend maps per-connection PRAGMA setup failures to
  `FilesystemOperation::Stat` inside `connect_with_retry`, but the retry loop
  only retries `db.connect()` failures. Under high-concurrency holdout, the
  connection opens successfully and then `execute_batch(LIBSQL_CONNECTION_PRAGMAS)`
  occasionally returns the transient misuse error, which escapes immediately and
  poisons the deterministic state hash.
- Hypothesis: Treat PRAGMA setup failure as a connection-setup failure and
  retry the whole open/setup cycle with the existing short retry budget. This
  should remove the rare libSQL baseline error without changing successful
  connection setup, workload semantics, durability, or Postgres behavior.
- Expected failure mode: Retrying every PRAGMA error could hide a persistent
  configuration bug. The retry budget is still bounded at three attempts and
  will surface the final error with context, so persistent failures stay loud.
- Change: `connect_with_retry` now treats a failed
  `execute_batch(LIBSQL_CONNECTION_PRAGMAS)` as a connection setup failure,
  waits with the existing bounded backoff, and retries the full open/setup
  cycle. Persistent failures still surface after the three-attempt budget with
  an explicit "create or initialize" infrastructure error.
- Result: Focused post-change `control_plane_snapshot` c16 and `put_get` c16
  rows passed for Postgres pool sizes 1 and 2 with zero errors and matching
  state hashes. Full locked `score.sh --holdout` passed all 42 comparison rows:
  no hard failures, no dev failures, no error mismatches, and no state-hash
  mismatches.
- Stress E2E: Per the full-flow validation requirement, reran
  `ironclaw_stress mixed-user-session` with `memory-persist-on-block`, c16,
  16 users, 4 threads per owner, 320 attempted operations, and model/tool
  latency 0. Postgres pool-size 2 completed 320/320 at 76.7ms p95
  (`thread_store_writes` 39.4ms p95, `resource_governor` 21.8ms p95);
  pool-size 1 completed 320/320 at 94.7ms p95 (`thread_store_writes` 55.1ms
  p95, `resource_governor` 31.2ms p95). The `postgres-pool-pressure` suite
  also passed chat/context/tool E2E cases at c16 pool-size 2 with zero
  failures: chat p95 31.5ms, context p95 51.6ms, tool p95 195.9ms.
- Validation: `cargo fmt -p ironclaw_filesystem --check`,
  `cargo check -p ironclaw_filesystem --features libsql,postgres`,
  `cargo test -p ironclaw_filesystem --features libsql,postgres connect_`,
  full `cargo test -p ironclaw_filesystem --features libsql,postgres`,
  `cargo check -p ironclaw_stress`, `cargo test -p ironclaw_reborn_cli
  --features webui-v2-beta,libsql,postgres`, and `cargo test -p
  ironclaw_architecture reborn` passed. The full
  `ironclaw_reborn_composition` suite first produced 987/1001 passes with
  three env-determinism failures from the shell's `NEARAI_API_KEY` plus
  timeout-only failures in parallel runtime tests; the failed groups were
  rerun with `NEARAI_*` unset and `--test-threads=1`, and all reruns passed.

## Cycle 19 - Turn-State Blocked-Flow Attribution

- Graph note: `codebase-memory-mcp` transport closed on the first project
  probe, so this cycle falls back to targeted local reads.
- Stress signal: Focused `ironclaw_stress` E2E runs with Postgres pool size 2
  show the durable filesystem turn-state backend is still a per-user
  `/turns/state.json` blob. In no-gate `chat-turn`, `memory-persist-on-block`
  completed 320/320 at 38.7ms p95 with `turn_store` p95 29us and +2.5MB DB
  growth, while filesystem turn state completed 320/320 at 66.8ms p95 with
  `turn_store` p95 23.6ms and +11.2MB DB growth. In gated
  `mixed-user-session`, the stress report under-counts the turn-store group
  because `block_run`, `resume_turn`, and the re-claim after resume are not
  stage-timed.
- Hypothesis: The next correct turn-state signal should measure every
  turn-store operation in the full blocked user flow. Adding stage timings for
  block, resume, and reclaim should expose whether the durable blob path is
  paying whole-snapshot CAS cost during blocked gates, without changing the
  workload or persistence semantics.
- Expected failure mode: Instrumentation drift could alter the async execution
  order or double-count unrelated stages. The change must reuse the existing
  `time_stage` helper around only the existing turn-store futures and update
  summaries/spans/human output consistently.
- Diagnostic: Patch `ironclaw_stress` attribution, run targeted compile/tests,
  then rerun the same Postgres E2E blocked and no-gate turn-state loops so the
  reported `turn_store` group includes submit, claim, block, resume, reclaim,
  and complete.
- Result: Added `block_run`, `resume_turn`, and `reclaim_run` stage timings to
  `ironclaw_stress` and included them in `turn_store` attribution. With
  corrected attribution, Postgres pool-size-2 gated `mixed-user-session`
  reports `memory-persist-on-block` at 134.9ms operation p95 and 7.15ms
  `turn_store` p95, while durable filesystem turn state reports 110.3ms
  operation p95 and 49.0ms `turn_store` p95. No-gate `chat-turn` isolates the
  blob cost: memory-persist completes 1600/1600 at 15.6ms operation p95 with
  `turn_store` p95 about 20us; filesystem completes 1600/1600 at 280.9ms
  operation p95 with `turn_store` p95 101.0ms.
- Growth diagnostic: Per-operation spans from the 1600-op filesystem `chat-turn`
  run show turn-state p95 climbing by operation-index quartile as the same
  per-user `/turns/state.json` grows: 32.7ms, 60.9ms, 78.8ms, then 114.4ms.
  The memory-persist control stays flat at 17-24us. This verifies the user's
  size-growth concern directly: blob CAS cost grows with snapshot body size.
- Mitigation result: Added stress retention-cap flags and fixed terminal
  pruning so old terminal runs also remove their orphaned `TurnRecord`s.
  A tiny filesystem hot window (`terminal=4`, `events=24`, `idempotency=8`)
  flattens the 1600-op `chat-turn` curve to 17.5ms, 15.7ms, 12.8ms, 13.2ms
  turn-store p95 by quartile and cuts operation p95 from 280.9ms to 45.7ms.
  The same cap on gated `mixed-user-session` cuts DB growth to +6.0MB and
  operation p95 to 94.0ms, but `turn_store` p95 is still 40.5ms.
- Conclusion: Retention caps are a useful guardrail and fix unbounded blob
  growth, but they cannot hit the target by themselves. Even the tiny hot
  window still pays several filesystem CAS round trips per turn transition.
  The durable filesystem turn-state solution needs a typed row/append store
  where submit/claim/block/resume/complete mutate small records/log entries
  directly instead of rehydrating and rewriting a per-user snapshot.

## Cycle 20 - Filesystem Turn-State Row Layout

- Graph note: `codebase-memory-mcp` still fails closed with a closed transport
  and the local graph artifact is stale/empty, so this cycle continues with
  targeted source reads.
- Harness signal: Cycle 19 proved the filesystem turn-state blob has a
  size-dependent curve: full-flow `chat-turn` p95 climbed from 32.7ms to
  114.4ms by operation-index quartile as `/turns/state.json` grew. Tiny
  retention caps flatten the growth but still leave filesystem turn-store p95
  at 16.2ms no-gate and 40.5ms in gated `mixed-user-session`.
- Hypothesis: A filesystem turn-state store that persists typed rows under
  `/turns/*` and writes only changed row files can preserve the existing
  transition semantics while removing the full-snapshot write-size term. The
  first slice should live beside the blob store and be selected explicitly by
  stress so we can compare it against the locked E2E flows before changing the
  hosted-single-tenant default.
- Expected failure mode: A naive row store that reloads every row on every
  transition could trade large blob writes for many small round trips. The
  initial success criterion is therefore semantic parity plus a measurable
  reduction in DB growth/write-size pressure; if per-transition latency remains
  dominated by row rehydration, the next iteration needs a hot in-process row
  cache or operation-specific row mutations.
- Diagnostic: Add the row-store implementation with targeted parity tests,
  wire an `ironclaw_stress` turn-state backend option for it, then rerun the
  same Postgres E2E `chat-turn` and gated `mixed-user-session` loops used in
  Cycle 19.
- Result: Added `FilesystemTurnStateRowStore` and `--turn-state-backend
  filesystem-row`. The store replays typed append-log deltas from
  `/turns/rows/v1/deltas/log`, keeps a hot per-user in-process
  `InMemoryTurnStateStore`, and persists targeted deltas for the hot
  `submit_turn`, `claim_next_run`, and `complete_run` path. Contract tests
  verify it does not write `/turns/state.json`, can reopen from the append log,
  and heartbeats do not rewrite durable run rows.
- Row-store iteration signal: The first row-file version reduced DB growth but
  was too slow (`chat-turn` 1600/1600 operation p95 790ms, `turn_store` p95
  346ms, +38.2MB). A generic append-log version with a hot store still paid
  whole-snapshot clone/diff cost (`chat-turn` p95 706ms, `turn_store` p95
  156.8ms). Targeted deltas removed most of that turn-state growth:
  Postgres/pool-2 `chat-turn` 1600/1600 now reports operation p95 234.2ms,
  `turn_store` p95 48.7ms, and +23.7MB DB growth. The same row backend on
  gated `mixed-user-session` 160/160 reports operation p95 187.2ms,
  `turn_store` p95 49.7ms, resource governor p95 40.3ms, and +4.3MB DB
  growth.
- Controls and remaining gap: The same Postgres full-flow memory turn-state
  control reports operation p95 46.7ms and `turn_store` p95 69us, so row
  turn-state is much better than blob CAS but still not at the target. At this
  point the full `chat-turn` flow is dominated by thread/context writes
  (`thread_store_writes` p95 163.2ms, `append_assistant` p95 91.5ms), but row
  `submit_turn` still costs 23.0ms p95 and should be optimized further. A
  libSQL `filesystem-row` concurrency run aborts with exit code 134; concurrency
  1 succeeds, and libSQL memory succeeds, so the row append path also needs a
  libSQL concurrency fix before it can be called portable.
- Conclusion: The solution shape is validated as typed append/row state with
  operation-specific deltas, not a blob snapshot and not a generic snapshot
  diff. The current slice is a measurable Postgres improvement but not yet
  sufficient for hosted-single-tenant parity; next work should make remaining
  hot transitions row-native, reduce `submit_turn` to one durable batch/append,
  compact or snapshot the append log for restart cost, and address the thread
  store blob path that now dominates the full user flow.

## Cycle 21 - Harness Turn-State Blob Lifecycle Signal

- Graph note: `codebase-memory-mcp` still fails closed with a closed transport
  on project/status probes, so this cycle uses targeted source reads.
- Baseline: The locked dev scorer and probe still pass the current storage and
  control-plane workloads, but `acceptance_ready` remains false because the
  harness does not yet run the local-runtime turn admission/queue/resume/cancel
  flow required by `spec.md`. The current production-shaped turn store is still
  `FilesystemTurnStateStore`, which persists a per-user `/turns/state.json`
  blob and applies each mutation by reading, overlaying, mutating, and CAS
  rewriting the snapshot.
- Hypothesis: Adding a scorer workload that drives submit -> claim -> block ->
  resume -> reclaim -> complete plus a separate submit -> claim ->
  request_cancel -> cancel flow through `FilesystemTurnStateStore` will make
  the blob growth problem visible in the locked latency harness. The workload
  should use the same libSQL/Postgres root filesystem comparison and state-hash
  parity checks as the rest of the scorer, so future row/append turn-state
  changes can be evaluated without ad hoc stress-only commands.
- Expected failure mode: This first harness slice may make Postgres fail the
  current dev ratios because it intentionally measures the blob CAS path rather
  than the experimental row store. That is acceptable diagnostic pressure; the
  fix should then be a production-shaped row/append turn-state path, not a
  memory-only shortcut or a benchmark-specific bypass.
- Diagnostic: Add the workload to `harness/latency/runner`, run the runner
  focused on that workload for compile/semantic parity, then run `lint.sh` and
  the dev scorer to capture p50/p95/p99 and state-hash behavior.
- Result: Added `turn_lifecycle_blob` to the locked latency runner. Each sample
  now drives a blocked/resumed run and a cancelled run through
  `FilesystemTurnStateStore`, including terminal readback, over the same libSQL
  vs Postgres root filesystem comparison used by the other workloads. A first
  c4 run exposed a harness bug: the c4 pass reused c1 sample/idempotency keys,
  so both backends replayed terminal runs and had nothing to claim. The runner
  now isolates workload run keys by workload and concurrency.
- Score signal: Focused `turn_lifecycle_blob` c4 with six samples completed
  with zero errors and matching state hashes. Full `score.sh --dev` also
  completed with zero errors and matching state hashes for the new workload.
  The new blob lifecycle rows pass at concurrency 1, but hard-fail at
  concurrency 4: libSQL c4 p95 was 5.63s, Postgres pool-1 c4 p95 was 15.25s
  (2.71x), and Postgres pool-2 c4 p95 was 9.12s (1.62x). This locks the
  filesystem turn-state blob/CAS contention problem into the scorer instead of
  leaving it only in stress-only diagnostics.
- Validation: `cargo fmt --manifest-path harness/latency/runner/Cargo.toml
  --check`, `cargo check --manifest-path harness/latency/runner/Cargo.toml`,
  focused `turn_lifecycle_blob` c1/c4 runner invocations, and
  `harness/latency/lint.sh` passed. Full `harness/latency/score.sh --dev`
  completed and reported the intended hard-fail comparison rows for
  `turn_lifecycle_blob` c4.

## Cycle 22 - Postgres Turn-State Row Wiring

- Baseline: Cycle 21 added the missing local-runtime turn lifecycle workload
  and showed the current filesystem blob store hard-fails under concurrent
  turn-state pressure. Full `score.sh --dev` reports zero errors and matching
  state hashes, but `turn_lifecycle_blob` c4 hard-fails: libSQL p95 5.63s,
  Postgres pool-1 p95 15.25s (2.71x), and Postgres pool-2 p95 9.12s (1.62x).
  A focused perturbed probe with payload sizes 128/2048 and path depths 2/5
  also passes c1 but fails c3; Postgres pool-2 c3 p95 is 2.82s vs libSQL
  1.87s (1.51x).
- Code signal: Hosted production still constructs `FilesystemTurnStateStore`
  directly. The existing `FilesystemTurnStateRowStore` already implements the
  turn-state, spawn-tree, event projection, loop checkpoint, and runner
  transition traits, but it is only used by tests/stress and is not selectable
  through production composition. The host-runtime builder also only exposes a
  helper that constructs the blob store.
- Hypothesis: Add a concrete filesystem turn-state wrapper that can hold either
  the blob store or the row store, then wire libSQL production to blob and
  Postgres production to row. The latency runner should mirror that
  production-shaped choice for the turn lifecycle workload: libSQL baseline
  stays blob, Postgres treatment uses row. This should remove the per-user
  `/turns/state.json` rewrite from hosted-single-tenant Postgres without adding
  a benchmark flag, path special case, or in-memory bypass.
- Expected failure mode: The row store currently improves the stress path but
  still has gaps: some transitions use generic snapshot deltas, and previous
  stress runs showed libSQL row concurrency aborting. This cycle deliberately
  leaves libSQL production on blob and may still fail the latency scorer if the
  row implementation's generic transitions are too expensive. If so, the next
  fix must make `block_run`, `resume_turn`, `request_cancel`, and `cancel_run`
  row-native instead of falling back to whole-snapshot deltas.
- Diagnostic: Implement the wrapper and production/harness wiring, run targeted
  compile tests, then rerun the focused turn lifecycle scorer and the full dev
  score to see whether Postgres c4 exits the hard-fail range.
- Result: Added `FilesystemTurnStateStoreKind`, a concrete wrapper over the
  blob and row filesystem stores, and delegated the turn-state, spawn-tree,
  event projection, loop checkpoint, and runner transition traits through it.
  LibSQL production and the latency baseline stay on the existing blob store;
  Postgres production and the latency treatment now use
  `FilesystemTurnStateRowStore`. The shared production host-runtime substrate
  path also takes the selected layout instead of always constructing a blob
  store. The latency workload is renamed from `turn_lifecycle_blob` to
  `turn_lifecycle` because it now mirrors the production backend choice.
- Growth signal: `turn_lifecycle` now writes and verifies loop-checkpoint
  metadata during the blocked/resumed path; the checkpoint count is derived
  from `LATENCY_PAYLOAD_BYTES / 256` and capped at 16, so probe payload
  perturbations grow persisted turn-state records instead of only changing
  unrelated filesystem payloads. Focused growth score
  (`LATENCY_PAYLOAD_BYTES=512,4096`, c1/c4, 12 samples) reports zero errors and
  matching hashes: libSQL blob c4 p95 6100.6ms, Postgres row pool-1 c4 p95
  1416.1ms (0.23x), and Postgres row pool-2 c4 p95 1448.0ms (0.24x).
- Locked score signal: Full `harness/latency/score.sh --dev` reports no
  failing comparison rows. The full-score `turn_lifecycle` rows all pass with
  matching hashes: libSQL blob c4 p95 8293.6ms, Postgres row pool-1 c4 p95
  1776.3ms (0.21x), and Postgres row pool-2 c4 p95 1869.6ms (0.23x). The
  previous Cycle 21 c4 hard fail is closed without increasing pool size or
  changing score semantics.
- `ironclaw_stress` E2E signal: Built `ironclaw_stress` in release mode and
  ran the same `mixed-user-session` flow with 8 prefilled threads x 10 turns,
  c4, 32 measured operations, blocked/resumed every operation, 2KiB user and
  assistant messages, and `context_max_messages=100`. All three runs completed
  with 32/32 measured operations and zero failures. LibSQL filesystem blob:
  operation p95 60.0ms, throughput 87.0 ops/s, turn_store p95 33.5ms.
  Postgres filesystem blob: operation p95 46.7ms, throughput 95.9 ops/s,
  turn_store p95 21.6ms. Postgres filesystem-row: operation p95 25.1ms,
  throughput 184.4 ops/s, turn_store p95 6.3ms. The stress result validates
  the same solution in the full user-turn path, not just the locked latency
  micro-workload.
- Validation: Passed `cargo fmt --manifest-path
  harness/latency/runner/Cargo.toml`, `cargo fmt -p ironclaw_turns -p
  ironclaw_reborn_composition`, `cargo check -p ironclaw_turns`, `cargo check
  -p ironclaw_reborn_composition --features libsql,postgres`, `cargo check
  --manifest-path harness/latency/runner/Cargo.toml`,
  `harness/latency/lint.sh`, focused/full latency scores, `cargo test -p
  ironclaw_turns filesystem_turn_state_row_store -- --nocapture`, and the
  release `ironclaw_stress` runs above. The existing
  `OutboundDeliveryTargetEntry` unused-import warning remains. A broader
  `cargo test -p ironclaw_reborn_composition --features libsql,postgres
  production_libsql_turn_state -- --nocapture` attempt was not usable: it
  pulled in a large transitive debug test graph and failed with `No space left
  on device` before reaching a meaningful filtered test result; generated
  `target/debug` artifacts were removed afterward to restore disk space.
- Conclusion: For filesystem turn state, the fix is a row/append layout for
  Postgres hosted-single-tenant, with libSQL left on the known blob baseline
  until the libSQL row concurrency abort from Cycle 20 is fixed. This closes
  the scorer-visible blob growth problem for Postgres turn lifecycle and moves
  the next latency frontier back to full-flow thread/context/resource costs and
  remaining row-native transition cleanup.

## Cycle 23 - High-Concurrency Full-Flow Resource Pressure

- Graph note: `codebase-memory-mcp` still fails with `Transport closed`; the
  local graph artifact is stale and empty, so this cycle uses targeted source
  reads after the failed graph probe.
- Baseline: Current `harness/latency/status.sh` reports a clean worktree at
  `988641e37` and Postgres ready on localhost. Full `harness/latency/score.sh
  --dev` exits 0 with zero failing comparison rows and
  `acceptance_ready=false`; the slowest dev rows are now libSQL blob
  `turn_lifecycle`, while Postgres row turn-state is faster and hash-matched.
  A broader `probe.sh` was stopped when the requested c32/c100 diagnostic
  superseded it.
- High-concurrency signal: `ironclaw_stress mixed-user-session` with
  Postgres pool size 2, `filesystem-row` turn state, gated every operation,
  2KiB user/assistant messages, and `context_max_messages=100` completes
  c32 320/320 at operation p95 321.6ms, p99 328.4ms, throughput 186.6 ops/s.
  c100 500/500 completes at operation p95 660.5ms, p99 719.0ms, throughput
  205.5 ops/s. At c100 the dominant group is `resource_governor` p95 429.3ms
  with `resource_reserve` p95 230.2ms and `resource_reconcile` p95 254.5ms;
  turn-state row p95 is 109.5ms and thread writes p95 is 106.4ms.
- Baseline caveat: The current libSQL hosted-volume stress baseline using
  `memory-persist-on-block` crashes before JSON at c32, even with one measured
  operation per worker, and also crashes before JSON at c100. This prevents a
  c32/c100 ratio from this stress binary today; the Postgres treatment numbers
  are still useful target-side saturation data.
- Hypothesis: The next Postgres full-flow bottleneck is row resource-governor
  transaction shape under high concurrency, not turn-state CAS. If reserve and
  reconcile serialize more work than necessary, tightening lock scope or
  reducing duplicated account-row work should lower c32/c100 p95 without
  changing resource accounting semantics.
- Expected failure mode: A resource-governor optimization can easily lose
  ancestor budget propagation, reservation close idempotency, or deterministic
  lock ordering. Any change must preserve existing resource-governor contract
  tests and re-run the c32/c100 stress slice.
- Diagnostic: Inspect `PostgresResourceGovernor` reserve/reconcile paths,
  identify whether account locks or transaction boundaries explain the c100
  profile, then patch only if the fix is scoped and covered by tests.
- Boundary result: A scoped batching patch to `PostgresResourceGovernor` was
  tested locally (`cargo check -p ironclaw_resources` and `cargo test -p
  ironclaw_resources` passed), but it was not kept because it optimizes a
  native Postgres domain store that bypasses `RootFilesystem` and is outside
  this goal's stated filesystem/composition/CLI surface. The next diagnostic
  must keep the filesystem abstraction in the measured path.
- Filesystem-path c32 signal: Running the locked latency runner directly with
  `LATENCY_WORKLOADS=turn_lifecycle`, `LATENCY_CONCURRENCY=32`,
  `LATENCY_SAMPLES=32`, and `LATENCY_PAYLOAD_BYTES=512` keeps both backends on
  the filesystem abstraction. LibSQL blob reported 13/32 errors with
  `turn state filesystem CAS retries exhausted`, p95 9014.4ms for successful
  samples, and mismatched state hash. Postgres row completed 32/32 with zero
  errors: pool-1 p95 3473.6ms and pool-2 p95 3420.4ms. This proves the row
  treatment avoids the libSQL blob CAS failure at c32, but the row store still
  serializes enough same-user lifecycle work to produce multi-second p95 in
  the micro-workload.
- Filesystem-path c100 signal: The same runner at
  `LATENCY_CONCURRENCY=100` and `LATENCY_SAMPLES=100` aborted with exit code
  134 before JSON, consistent with the libSQL baseline failing before the
  runner can reach Postgres treatment rows. A Postgres-only c100 number is
  therefore not available from the current locked runner without adding a
  diagnostic backend filter.

## Cycle 24 - Diagnostic Backend Filter For c100 Treatment Rows

- Graph note: `codebase-memory-mcp` remains unavailable (`Transport closed`),
  so this cycle uses targeted harness reads.
- Baseline: Cycle 23 found that the locked filesystem `turn_lifecycle` runner
  can produce c32 rows, but c100 aborts before JSON because the libSQL blob
  baseline fails before the runner reaches Postgres treatment. This blocks a
  Postgres c100 filesystem-path treatment number from the locked runner even
  though full-flow `ironclaw_stress` shows Postgres c100 can complete.
- Hypothesis: Add a diagnostic-only `LATENCY_BACKENDS` filter to the latency
  runner. The default must remain both backends so normal dev/holdout scoring
  still compares libSQL and Postgres and cannot be used as acceptance when a
  backend is skipped. The filter should make c100 Postgres filesystem-path
  treatment rows observable without bypassing `ScopedFilesystem` or changing
  workload semantics.
- Expected failure mode: A backend filter could be misused as a fake pass by
  omitting the failing baseline. The JSON report must expose the selected
  backends and continue to mark `acceptance_ready=false`; docs must call this
  diagnostic-only.
- Diagnostic: Patch the runner, run a default both-backend smoke to confirm
  existing behavior, then run `LATENCY_BACKENDS=postgres` at c100 for
  `turn_lifecycle`.
- Result: Added diagnostic-only `LATENCY_BACKENDS` support. Default output now
  reports `backends=["libsql","postgres"]`; comparison rows still populate.
  `harness/latency/lint.sh` passes by default, voids `LATENCY_BACKENDS=postgres`
  without `LATENCY_ALLOW_DIAGNOSTIC_BACKENDS=1`, and passes when that
  diagnostic opt-in is present. The README and `status.sh` document/report the
  backend filter.
- c100 treatment signal: With `LATENCY_BACKENDS=postgres`,
  `LATENCY_WORKLOADS=turn_lifecycle`, `LATENCY_CONCURRENCY=100`,
  `LATENCY_SAMPLES=100`, and `LATENCY_PAYLOAD_BYTES=512`, the locked runner now
  reaches Postgres treatment rows through the filesystem row turn-state store.
  Pool size 1 completes 100/100 with p50 31.20s, p95 31.34s, p99 31.35s,
  throughput 3.19 ops/s. Pool size 2 completes 100/100 with p50 31.31s,
  p95 31.45s, p99 31.47s, throughput 3.18 ops/s. This is much slower than the
  full-flow `ironclaw_stress` c100 result because the locked runner drives 100
  concurrent lifecycle samples through one mounted per-user turn-state store;
  the next filesystem-aligned optimization target is row-store same-user
  serialization under high concurrency.

## Cycle 25 - WebUI Session Request Path

- Graph note: `codebase-memory-mcp` still fails with `Transport closed`; the
  local graph artifact is stale and contains zero indexed nodes, so this cycle
  uses targeted source reads.
- Baseline: The locked harness now covers filesystem hot paths, hosted
  substrate build/readiness, and filesystem-backed turn lifecycle pressure, but
  it still does not time a real WebUI request. The `/api/webchat/v2/session`
  handler is useful because it crosses bearer auth middleware, descriptor
  policy layers, `RebornServicesApi`, and the global auto-approve settings read
  without invoking any LLM/provider/network call.
- Hypothesis: Add a `webui_session` workload that builds one cached
  `build_reborn_runtime -> build_webui_services -> webui_v2_app` stack per
  backend, then measures authenticated `GET /api/webchat/v2/session` requests
  through Axum `oneshot`. This should close a Stage 0 WebUI gap while staying
  inside composition/runtime abstractions. It must not reach into Postgres
  tables, thread stores, or filesystem paths directly.
- Workload shape: The session route has a real read rate limit of 120 requests
  per caller per minute, so the harness should use deterministic multi-user
  session bootstrap tokens instead of measuring a guaranteed 429 after the
  first 120 samples for one caller. The sample-to-user mapping must be identical
  for libSQL and Postgres so visible response hashes remain comparable.
- Expected failure mode: Building this through a stub service facade would hide
  runtime/store latency, while building it with ad hoc DB handles would bypass
  the abstraction the user explicitly asked about. The workload should use
  `local_runtime_build_input` for hosted-volume libSQL and hosted
  single-tenant Postgres build input for Postgres, with the same production-
  relevant Postgres pool caps as the rest of the harness.
- Diagnostic: Patch the harness only, run formatting/check/lint, then run a
  tiny `webui_session` smoke for both backends before any larger score.
- Result: Added `webui_session` to the locked runner and documented it in the
  harness README. The workload builds one cached hosted-volume libSQL runtime
  and one cached hosted-single-tenant Postgres runtime per pool size through
  `build_reborn_runtime`, `build_webui_services`, and `webui_v2_app`; measured
  samples are Axum `oneshot` requests to `/api/webchat/v2/session`. Added a
  non-env `RebornBuildInput::hosted_single_tenant_postgres` constructor so the
  harness can pass the already-capped Postgres pool into composition instead of
  reopening storage through process env.
- Validation: `cargo fmt --manifest-path harness/latency/runner/Cargo.toml
  --check`, `cargo fmt -p ironclaw_reborn_composition --check`,
  `cargo check --manifest-path harness/latency/runner/Cargo.toml`,
  `cargo check -p ironclaw_reborn_composition --features
  webui-v2-beta,libsql,postgres`, `harness/latency/lint.sh`, and
  `git diff --check` passed. The first smoke run hit `No space left on device`
  while writing debug archives; removing only the generated
  `harness/latency/runner/target/debug/incremental` cache freed space, and the
  rerun passed.
- Tiny smoke: With `LATENCY_WORKLOADS=webui_session`,
  `LATENCY_WARMUP=1`, `LATENCY_SAMPLES=4`, and `LATENCY_CONCURRENCY=1`, both
  backends completed with zero errors and matching state hash
  `88db09960433a88e`. libSQL p95 was 1.92ms; Postgres pool-1 p95 was 0.62ms;
  Postgres pool-2 p95 was 0.51ms.
- Dev score: `harness/latency/score.sh --dev` completed with all c1/c4
  comparisons passing and zero errors. The new `webui_session` rows matched
  state hash `d361edd7550f85f2`; at c4, libSQL p95 was 2.36ms, Postgres pool-1
  p95 was 0.88ms, and Postgres pool-2 p95 was 0.52ms.
- c32/c100 WebUI signal: A targeted both-backend run with
  `LATENCY_WORKLOADS=webui_session`, `LATENCY_WARMUP=4`,
  `LATENCY_SAMPLES=100`, and `LATENCY_CONCURRENCY=32,100` completed with zero
  errors and matching state hash `986f2b6685239bb2`. At c32, pool-2 is close
  enough for dev ratio (libSQL p95 5.05ms, Postgres pool-2 p95 5.74ms), while
  pool-1 is slower (p95 7.08ms). At c100, both Postgres pool sizes hard-fail
  ratio checks despite higher throughput: libSQL p95 4.41ms, Postgres pool-1
  p95 16.21ms, Postgres pool-2 p95 11.67ms. Next high-concurrency work should
  inspect the session request's `global_auto_approve_enabled` read path and
  WebUI middleware contention before touching lower-level stores.

## Cycle 26 - Target Loop Checkpoint Row Deltas

- Graph note: `codebase-memory-mcp` still fails with `Transport closed`; the
  local graph artifact is stale and contains zero indexed nodes, so this cycle
  uses the `crates/ironclaw_turns` subsystem docs plus targeted source reads.
- Baseline: `harness/latency/score.sh --dev` still passes all c1/c4 rows with
  zero errors. The long probe shows `turn_lifecycle` as the dominant remaining
  pressure point: libSQL c8 hits `turn state filesystem CAS retries exhausted`
  after multi-second p95s, while Postgres row store completes c8 but remains in
  the seconds under same-user concurrency.
- Abstraction check: The row-store path owns an `Arc<ScopedFilesystem<F>>` and
  persists typed append-log deltas through the filesystem abstraction. This
  cycle must not add direct Postgres table writes or reads from
  `ironclaw_turns`; Postgres remains a filesystem backend selected by the
  hosted-single-tenant runtime profile.
- Hypothesis: `put_loop_checkpoint` still uses the generic `apply` path, which
  asks the row store to compute a full snapshot diff after each checkpoint
  write. The `turn_lifecycle` workload writes multiple loop checkpoints per
  sample as payload size grows, so this keeps checkpoint cost coupled to total
  turn-state size. Switching loop checkpoint writes to `apply_with_targeted_delta`
  should persist only the new checkpoint row plus any new event rows, matching
  the existing targeted submit/claim/complete paths.
- Expected failure mode: A targeted delta that forgets side-effect rows would
  leave the hot snapshot and reopened snapshot divergent. Contract coverage
  should reopen the row store through the same `ScopedFilesystem` and verify
  loop checkpoint records survive without writing `/turns/state.json`.
- Result: `FilesystemTurnStateRowStore::put_loop_checkpoint` now uses
  `apply_with_targeted_delta` and persists a delta containing only the returned
  `LoopCheckpointRecord` plus any newly emitted lifecycle events. The write
  still flows through `persist_delta` and `ScopedFilesystem::append`; there are
  no direct Postgres reads or writes in `ironclaw_turns`.
- Contract validation: Added a row-store loop-checkpoint contract that writes
  two checkpoint records, verifies `/turns/state.json` is not created, and
  reopens through the same `ScopedFilesystem` to confirm the records survive.
  `cargo fmt -p ironclaw_turns --check` and
  `cargo test -p ironclaw_turns --test loop_checkpoint_store_contract` passed.
- Full-flow stress signal: `ironclaw_stress` with `--backend postgres`,
  `--scenario mixed-user-session`, `--turn-state-backend filesystem-row`,
  `--postgres-pool-size 2`, and `--users >= --concurrency` passed at c32 and
  c100. c32 completed 128/128 with operation p95 161ms and turn-store p95 33ms.
  c100 completed 200/200 with operation p95 437ms and turn-store p95 59ms. In
  both runs the stress report identifies the resource governor, not turn state,
  as the top operation group. Matching libSQL c32/c100 filesystem baselines are
  currently blocked in this checkout: c32 aborts before JSON, and a memory
  turn-state control still reports libSQL thread-store failures (`bad parameter
  or other API misuse`) under c32.
- Exact lifecycle diagnostic: A Postgres-only diagnostic
  `turn_lifecycle` run with `LATENCY_CONCURRENCY=32,100`,
  `LATENCY_PAYLOAD_BYTES=2048`, `LATENCY_SAMPLES=64`, and pool size 2 completed
  with zero errors and stable state hash `660086a8484d5400`, but the latency is
  still not acceptable: c32 p95 7.66s, c100 p95 29.96s. The targeted loop
  checkpoint delta is therefore a scoped correctness/row-growth improvement,
  not the final c32/c100 parity fix. The remaining lifecycle bottleneck is the
  same-user row-store serialization/write path.
- Dev score: `harness/latency/score.sh --dev` passed with 54 results and 36
  comparisons, zero dev failures, and zero hard failures. `turn_lifecycle`
  c4 remains much faster on Postgres row store than libSQL blob in the locked
  dev score (libSQL p95 7.51s; Postgres pool-2 p95 1.33s), but c32/c100 still
  needs the next fix.

## Cycle 27 - Target Lifecycle Row Deltas

- Graph note: `codebase-memory-mcp` still fails with `Transport closed`; the
  local graph artifact is stale and contains zero indexed nodes, so this cycle
  uses crate guardrails plus targeted source reads.
- Baseline: `harness/latency/score.sh --dev` passed with 54 results, 36
  comparisons, and zero failures. The dev `turn_lifecycle` rows remain stable:
  libSQL c4 p95 7.42s, Postgres pool-1 c4 p95 1.33s, and Postgres pool-2 c4
  p95 1.37s.
- Probe: `harness/latency/probe.sh` completed with only `turn_lifecycle` c8
  hard failures. libSQL c8 hit 3 `turn state filesystem CAS retries exhausted`
  errors and a mismatched state hash. Postgres row store completed c8 with zero
  errors and matching state hash, but remained slow: pool-1 c8 p95 7.32s,
  pool-2 c8 p95 7.09s.
- Hypothesis: The row store now writes compact targeted deltas, but every
  state transition still holds the hot-state mutex across one durable
  `ScopedFilesystem::append` to `/turns/rows/v1/deltas/log`. The filesystem
  backends already expose atomic `append_batch`; using a small row-store delta
  write queue should let concurrent transitions flush multiple durable deltas
  in one backend round trip while every caller still waits for its write to
  commit before returning.
- Expected failure mode: Moving the durable write outside the hot-state mutex
  can expose in-process hot state before the append is acknowledged. The patch
  must fail closed by clearing the row-store snapshot cache on write failure,
  must not return success before the queued durable write is acknowledged, and
  must preserve reopen-from-filesystem contracts.
- Queue attempt result: A queued append experiment was rejected before commit.
  The queue-only treatment did not improve the exact lifecycle diagnostic
  enough (c32 p95 7.49s, c100 p95 29.19s), and the full
  `ironclaw_stress` c100 mixed user-session flow regressed to operation p95
  692ms with turn-store p95 85ms and resource-governor p95 491ms. A
  250us coalescing delay made the synthetic lifecycle slightly better but
  still left c100 near 29.03s and worsened full-flow c32. The queue patch was
  fully reverted.
- Revised hypothesis: The remaining `turn_lifecycle` workload still exercises
  full snapshot diffs on `resume_turn`, `request_cancel`, `block_run`, and
  `cancel_run`. These operations mutate one run row plus lock/reservation,
  event, checkpoint, and idempotency rows. Persisting those as targeted row
  deltas should keep durable writes synchronous while removing the snapshot
  clone/diff cost from the hot lifecycle path.
- Result: `resume_turn` and `request_cancel` now use targeted deltas that
  include run state, active lock, admission reservation, events, and the
  relevant idempotency row. `block_run` uses a targeted delta that also
  persists the block-created checkpoint row. `cancel_run` uses a targeted
  terminal run-state delta with the existing full-snapshot fallback when the
  terminal retention cap could prune old rows. The generic full-snapshot
  transition helper remains in place for less common paths whose side effects
  are broader.
- Contract validation: Extended the row-store filesystem contract to submit,
  claim, block, reopen, verify the blocked checkpoint row, resume, verify
  resume idempotency, reclaim, complete, and reopen without creating the
  blob-shaped `/turns/state.json`. `cargo fmt -p ironclaw_turns --check`,
  `cargo test -p ironclaw_turns --test filesystem_turn_state_contract
  filesystem_turn_state_row_store_persists_rows_without_state_blob`,
  `cargo test -p ironclaw_turns --test loop_checkpoint_store_contract
  filesystem_turn_state_row_store_loop_checkpoint_roundtrip_and_snapshot`,
  and `cargo check -p ironclaw_turns` passed.
- Dev score: `harness/latency/score.sh --dev` passed with 54 results, 36
  comparisons, and zero failures. Dev `turn_lifecycle` state hashes match
  `7fc054292d2f85f0`; libSQL c4 p95 was 7.46s, Postgres pool-1 c4 p95 was
  629ms, and Postgres pool-2 c4 p95 was 623ms.
- Exact lifecycle diagnostic: Postgres-only `turn_lifecycle` with
  `LATENCY_CONCURRENCY=32,100`, `LATENCY_PAYLOAD_BYTES=2048`,
  `LATENCY_SAMPLES=64`, and pool size 2 completed with zero errors and stable
  state hash `660086a8484d5400`. c32 p95 improved from 7.66s to 3.84s; c100
  p95 improved from 29.96s to 15.04s. This is a material improvement, but not
  final c100 parity.
- Full-flow stress signal: `ironclaw_stress` with `--backend postgres`,
  `--scenario mixed-user-session`, `--turn-state-backend filesystem-row`,
  `--postgres-pool-size 2`, and per-user concurrency completed with zero
  errors. c32 completed 128/128 with operation p95 160.5ms and turn-store
  p95 30.8ms. c100 completed 200/200 with operation p95 456.3ms and
  turn-store p95 68.6ms. The full-flow c100 report now identifies the
  resource governor as the top operation group (p95 277.8ms), followed by
  thread-store writes (p95 99.3ms); turn state is no longer the top full-flow
  bottleneck.

## Cycle 28 - Resource Governor Shared-Row Contention

- Graph note: `codebase-memory-mcp` still fails with `Transport closed` for
  both status and indexing; the local graph artifact remains stale and empty,
  so this cycle uses crate guardrails plus targeted source reads.
- Baseline: `harness/latency/score.sh --dev` passed with 54 results, 36
  comparisons, and zero failures. Dev `turn_lifecycle` remains fast on
  Postgres row store: libSQL c4 p95 7.96s, Postgres pool-1 c4 p95 630ms, and
  Postgres pool-2 c4 p95 625ms.
- Probe: `harness/latency/probe.sh` again fails only on `turn_lifecycle` c8
  state-hash comparisons because libSQL hits five `turn state filesystem CAS
  retries exhausted` errors and produces a different state hash. Postgres
  pool-1/pool-2 complete c8 with zero errors, matching state hash
  `3fa07e3dc3c7e320`, and p95 around 3.42-3.46s.
- High-concurrency turn-state diagnostic: Postgres-only `turn_lifecycle` with
  c32/c100, payload 2048, 64 samples, and pool size 2 completed with zero
  errors and state hash `660086a8484d5400`. c32 p95 is 3.78s and c100 p95 is
  14.72s, stable against cycle 27.
- Full-flow baseline: `ironclaw_stress` mixed user-session with Postgres row
  turn state and pool size 2 completed with zero errors. c32 completed 128/128
  with operation p95 188.5ms, turn-store p95 38.6ms, and resource-governor
  p95 79.8ms. c100 completed 200/200 with operation p95 458.2ms, turn-store
  p95 62.8ms, and resource-governor p95 286.4ms. The top stages are
  `resource_reserve` and `resource_reconcile` at roughly 143-149ms each.
- Resource-only baseline: `ironclaw_stress --scenario reserve-reconcile` at
  c100/pool-2 completed 400/400 with operation p95 198.0ms and reported
  Postgres waiting connections. This isolates the governor as a real
  production-shaped bottleneck, not a side effect of turn/thread stores.
- Hypothesis: `PostgresResourceGovernor` always ensures, locks, and rewrites
  every account row in the resource-scope cascade. With one hosted tenant,
  every reservation/reconcile serializes on the same tenant account row even
  when there are no finite limits installed. We need a row-store shape that
  keeps durable reservation lifecycle and finite-limit enforcement, but avoids
  hot shared aggregate-row writes for unlimited accounts. A safe first step is
  to move no-finite-limit reservations onto append/row lifecycle writes while
  leaving finite-limit accounts on the existing locked aggregate path.
- Expected failure mode: Skipping aggregate writes blindly would break
  `usage_for`, `reserved_for`, `account_snapshot`, and future finite-limit
  installation after no-limit activity. The patch must either reconstruct
  unlimited account snapshots from durable reservation rows or merge prior
  reservation rows when a finite limit is installed. It must not return success
  before a durable reservation lifecycle write is committed.
- Result: `PostgresResourceGovernor` now keeps the finite-limit path on the
  existing locked account aggregates, but moves unlimited accounts to durable
  reservation lifecycle rows instead of rewriting hot shared account rows for
  every reserve/reconcile/release. The reservation table now stores indexed
  `account_keys` so `account_snapshot` and later finite-limit installation can
  rebuild reserved/spent tallies from reservation rows without scanning every
  reservation. `set_limit` takes an exclusive account advisory lock and
  lifecycle operations take shared account advisory locks so a finite limit
  cannot be installed concurrently with an unlimited-path reservation update.
- Abstraction boundary: This cycle does not bypass the turn-state filesystem
  abstraction. The optimization is in the native hosted-single-tenant
  Postgres resource governor path, which was already separate from the
  filesystem-backed resource governor. It still waits for the durable
  reservation row write to commit before returning success; it only skips
  aggregate account-row writes when no finite limit exists.
- Correctness fix during review: reservation creation now uses an atomic
  insert-and-conflict check instead of the lifecycle update upsert, so
  concurrent callers cannot both succeed with the same reservation id on the
  shared-lock unlimited path. This post-measurement fix was covered by
  compile/tests; I did not rerun full c32/c100 stress after it because the
  workspace had less than 800MiB free and the hot-path shape is unchanged.
- Resource-only treatment: `ironclaw_stress --scenario reserve-reconcile` at
  c100/pool-2 completed 400/400 with operation p95 158.9ms and throughput
  859.5 ops/sec, down from the c100 baseline p95 198.0ms and throughput
  496.8 ops/sec.
- Full-flow stress signal: `ironclaw_stress` mixed user-session with Postgres
  row turn state and pool size 2 completed with zero errors after the indexed
  reservation-key patch. c32 completed 128/128 with operation p95 154.4ms,
  turn-store p95 40.6ms, and resource-governor p95 52.2ms. c100 completed
  200/200 with operation p95 388.1ms, turn-store p95 71.8ms, and
  resource-governor p95 213.3ms. The top c100 resource stages dropped to
  `resource_reserve` p95 106.5ms and `resource_reconcile` p95 114.1ms.
- Dev score and validation: The first treatment exposed slow
  `control_plane_snapshot` rows because unlimited snapshots scanned all
  reservations; the indexed `account_keys` query fixed that. Final
  `harness/latency/score.sh --dev` passed with 54 results, 36 comparisons,
  and zero failures. `cargo fmt -p ironclaw_resources --check`,
  `cargo check -p ironclaw_resources --features postgres`,
  `cargo test -p ironclaw_resources`, `cargo test -p ironclaw_resources
  --features postgres`, and `git diff --check` passed.

## Cycle 29 - Turn Checkpoint Readback Size Dependence

- Graph note: `codebase-memory-mcp` still fails with `Transport closed` for
  both `index_status` and a fast `index_repository`; the local graph artifact
  remains stale and empty, so this cycle uses crate guardrails plus targeted
  source reads.
- Baseline: `harness/latency/score.sh --dev` passed with 54 results, 36
  comparisons, and zero failures. Dev `turn_lifecycle` remains the slowest
  Postgres row-store workload even though it clears the libSQL baseline:
  libSQL c4 p95 8.30s, Postgres pool-1 c4 p95 628ms, and Postgres pool-2
  c4 p95 624ms.
- Probe: `harness/latency/probe.sh` passed with 81 results, 54 comparisons,
  and zero failures. The perturbed `turn_lifecycle` c8 row was stable on
  Postgres with zero errors and matching state hash `3fa07e3dc3c7e320`, but
  remained slow at p95 3.52s for pool-1 and 3.45s for pool-2. libSQL c8 hit
  four `turn state filesystem CAS retries exhausted` errors and took p95
  52.35s, which is useful baseline context but not a reason to stop optimizing
  hosted Postgres.
- Hypothesis: The row store no longer writes `/turns/state.json`, and the hot
  lifecycle writes use targeted deltas, but `get_loop_checkpoint` still clones
  the full row-store snapshot and rebuilds an `InMemoryTurnStateStore` on every
  checkpoint readback. The lifecycle workload writes and reads up to 16 loop
  checkpoints per sample for 2048-byte payloads, so this read path grows with
  accumulated state and sits inside the same global row-store snapshot mutex.
  `put_loop_checkpoint` also calls `add_event_delta` even though the in-memory
  checkpoint write does not emit lifecycle events, causing repeated event scans
  as state grows.
- Expected failure mode: Direct checkpoint projection must exactly preserve
  `LoopCheckpointStore::get_loop_checkpoint` scope/turn/run/checkpoint matching
  semantics and must still observe freshly persisted targeted deltas before
  returning. Removing the event scan from checkpoint writes is only valid if
  checkpoint writes remain event-free; lifecycle event counts and state hashes
  must stay identical in the score/probe.
- Result: Row-store loop checkpoint readback now projects directly from the
  cached row snapshot instead of cloning the full snapshot and rebuilding an
  `InMemoryTurnStateStore` per read. Loop checkpoint targeted deltas now write
  only the checkpoint row because the underlying in-memory checkpoint write is
  event-free.
- Dev score: `harness/latency/score.sh --dev` passed with 54 results, 36
  comparisons, and zero failures. The scored `turn_lifecycle` state hash
  stayed `7fc054292d2f85f0`; Postgres pool-2 c4 p95 was 628.7ms, effectively
  stable against the 624.2ms baseline.
- High-concurrency turn-state diagnostic: Postgres-only `turn_lifecycle` with
  c32/c100, payload 2048, 64 samples, and pool size 2 completed with zero
  errors and state hash `660086a8484d5400`. c32 p95 improved from 3.78s to
  3.42s; c100 p95 improved from 14.72s to 13.23s. This confirms the checkpoint
  readback path was contributing to state-size growth, but the remaining c100
  latency still points at the global row-store/in-memory critical section.
- Validation: `cargo fmt -p ironclaw_turns --check`, `cargo test -p
  ironclaw_turns --test loop_checkpoint_store_contract
  filesystem_turn_state_row_store_loop_checkpoint_roundtrip_and_snapshot`,
  `cargo test -p ironclaw_turns --test filesystem_turn_state_contract
  filesystem_turn_state_row_store_persists_rows_without_state_blob`, `cargo
  test -p ironclaw_turns --test loop_checkpoint_store_contract`, `cargo check
  -p ironclaw_turns`, and `git diff --check` passed. I removed only generated
  incremental build caches to recover disk before rerunning the score.

## Cycle 30 - Turn Run-State Readback Projection

- Graph note: `codebase-memory-mcp` still fails with `Transport closed` for
  both `index_status` and a fast `index_repository`; the local graph artifact
  remains stale and empty, so this cycle uses crate guardrails plus targeted
  source reads.
- Baseline: `harness/latency/score.sh --dev` passed with 54 results, 36
  comparisons, and zero failures. Dev `turn_lifecycle` remains stable after
  the checkpoint projection change: libSQL c4 p95 7.51s, Postgres pool-1 c4
  p95 601ms, and Postgres pool-2 c4 p95 601ms.
- Probe: `harness/latency/probe.sh` passed with 81 results, 54 comparisons,
  and zero failures. The perturbed `turn_lifecycle` c8 row completed on
  Postgres with zero errors and matching state hash `3fa07e3dc3c7e320`;
  pool-1 p95 was 3.11s and pool-2 p95 was 3.16s. libSQL c8 again hit
  filesystem CAS retry exhaustion, so it is not a useful parity ceiling for
  high-concurrency tuning.
- Hypothesis: Cycle 29 removed full rebuilds from loop checkpoint readback,
  but `FilesystemTurnStateRowStore::get_run_state` still clones the cached
  row snapshot, applies runner-lease overlay to a full snapshot, and rebuilds
  an `InMemoryTurnStateStore` to read one run. The lifecycle workload performs
  two terminal readbacks per sample, so this keeps a size-dependent read inside
  the global row-store mutex. Directly projecting the requested run state from
  the cached row snapshot, then applying the per-run runner-lease overlay, should
  reduce read amplification without changing persistence.
- Expected failure mode: Direct run-state projection must preserve
  `GetRunStateRequest` scope-not-found behavior, include the turn actor from
  the matching `TurnRecord`, and preserve runner-lease overlay behavior for
  running/cancel-requested runs. State hashes and lifecycle event counts must
  remain unchanged.
- Result: Row-store `get_run_state` now projects the requested run directly
  from the cached row snapshot, fetches the matching turn actor, and applies
  runner-lease overlay to that single run record instead of cloning and
  rebuilding the whole in-memory store.
- Dev score: `harness/latency/score.sh --dev` passed with 54 results, 36
  comparisons, and zero failures. The scored `turn_lifecycle` state hash
  stayed `7fc054292d2f85f0`; Postgres pool-2 c4 p95 improved from the
  baseline 601ms to 575ms.
- High-concurrency turn-state diagnostic: Postgres-only `turn_lifecycle` with
  c32/c100, payload 2048, 64 samples, and pool size 2 completed with zero
  errors and state hash `660086a8484d5400`. c32 p95 improved from 3.42s to
  3.24s; c100 p95 improved from 13.23s to 12.50s. The remaining latency still
  appears dominated by serialized writes in the row-store/in-memory critical
  section.
- Validation: `cargo fmt -p ironclaw_turns --check`, `cargo test -p
  ironclaw_turns --test filesystem_turn_state_contract
  filesystem_turn_state_row_store_persists_rows_without_state_blob`, `cargo
  test -p ironclaw_turns --test loop_checkpoint_store_contract
  filesystem_turn_state_row_store_loop_checkpoint_roundtrip_and_snapshot`,
  `cargo test -p ironclaw_turns --test loop_checkpoint_store_contract`,
  `cargo check -p ironclaw_turns`, and `git diff --check` passed.

## Cycle 31 - Turn Event Tail Tracking

- Graph note: `codebase-memory-mcp` still fails with `Transport closed` for
  both `index_status` and a fast `index_repository`; the local graph artifact
  remains stale and empty, so this cycle uses crate guardrails plus targeted
  source reads.
- Baseline: `harness/latency/score.sh --dev` passed with 54 results, 36
  comparisons, and zero failures. Dev `turn_lifecycle` remains stable after
  the run-state readback projection: libSQL c4 p95 8.44s, Postgres pool-1 c4
  p95 585ms, and Postgres pool-2 c4 p95 576ms.
- Probe: `harness/latency/probe.sh` passed with 81 results, 54 comparisons,
  and zero failures. The perturbed `turn_lifecycle` c8 row completed on
  Postgres with zero errors and matching state hash `3fa07e3dc3c7e320`;
  pool-1 p95 was 2.96s and pool-2 p95 was 2.99s. libSQL c8 still hit
  filesystem CAS retry exhaustion and produced a different hash, so high
  concurrency work remains a treatment-side optimization exercise.
- Hypothesis: The remaining row-store write path still does size-dependent
  work inside the global `snapshot_state` mutex. Every targeted lifecycle
  write that may emit an event calls `add_event_delta`, which scans all
  retained events to find the latest cursor before asking the in-memory store
  for newer events. The lifecycle workload emits events on submit, claim,
  block, resume, reclaim, complete, request-cancel, and cancel, so this scan
  grows with accumulated state and serializes unrelated turn scopes. Tracking
  the latest retained event cursor in `RowSnapshotState` should preserve the
  durable delta shape while removing that per-write scan.
- Expected failure mode: The cached event cursor must stay synchronized after
  replay, targeted deltas, full-snapshot fallbacks, and retention-floor
  changes. If it falls behind, duplicate events can be appended; if it jumps
  ahead, lifecycle events can be skipped. State hashes, event counts, and
  reopen-from-delta behavior must remain stable.
- Result: `FilesystemTurnStateRowStore` now caches the latest retained event
  cursor alongside the cached row snapshot. Targeted lifecycle deltas pass the
  cached cursor into `add_event_delta`, and the cache advances only after the
  durable delta append succeeds. Full-snapshot fallbacks recompute the cursor
  from the replacement snapshot.
- Dev score: treatment `harness/latency/score.sh --dev` passed with 54
  results, 36 comparisons, and zero failures. The scored `turn_lifecycle`
  state hash stayed `7fc054292d2f85f0`; Postgres pool-2 c4 p95 was 584ms,
  effectively flat against the 576ms cycle baseline.
- High-concurrency turn-state diagnostic: Postgres-only `turn_lifecycle` with
  c32/c100, payload 2048, 64 samples, and pool size 2 completed with zero
  errors and state hash `660086a8484d5400`. c32 p95 was 3.27s and c100 p95
  was 12.43s, essentially flat against cycle 30. Next cycle should change
  approach rather than tune event-tail tracking further; the remaining signal
  is still serialized write-critical-section work.
- Validation: `cargo fmt -p ironclaw_turns --check`, `cargo check -p
  ironclaw_turns`, `cargo test -p ironclaw_turns --test
  filesystem_turn_state_contract
  filesystem_turn_state_row_store_persists_rows_without_state_blob`, `cargo
  test -p ironclaw_turns --test loop_checkpoint_store_contract`, full `cargo
  test -p ironclaw_turns --test filesystem_turn_state_contract`, final `cargo
  check -p ironclaw_turns`, and `git diff --check` passed.

## Cycle 32 - In-Place Row Delta Application

- Graph note: `codebase-memory-mcp` still fails with `Transport closed` for
  `index_status`, `index_repository`, and `search_code`; the local graph
  artifact remains stale and empty, so this cycle uses targeted source reads.
- Baseline: current-commit `harness/latency/score.sh --dev` passed with 54
  results, 36 comparisons, and zero failures. Dev `turn_lifecycle` state hash
  stayed `7fc054292d2f85f0`; Postgres pool-1 c4 p95 was 578ms and pool-2 c4
  p95 was 581ms.
- Probe: current-commit `harness/latency/probe.sh` passed with 81 results, 54
  comparisons, and zero failures. Perturbed Postgres `turn_lifecycle` c8
  completed with zero errors and matching state hash `3fa07e3dc3c7e320`;
  pool-1 p95 was 3.02s and pool-2 p95 was 2.99s. libSQL c8 still hit CAS
  retry exhaustion and a mismatched hash.
- Hypothesis: `apply_delta` still applies every tiny targeted lifecycle delta
  by rebuilding a `HashMap` from the whole affected snapshot vector, cloning
  every retained record, applying one or two upserts/deletes, then collecting a
  replacement vector. This happens for runs, events, active locks,
  idempotency, reservations, and checkpoints while the row-store writer mutex
  is held. Replacing that with in-place retain/replace/push mutation should
  remove another blob-store-shaped allocation/copy step without changing the
  durable delta log or row schema.
- Expected failure mode: In-place updates must preserve current upsert-wins
  semantics when a key appears in both delete and upsert, must not retain
  deleted records, and must not introduce duplicate keyed records. Reopen,
  event projection, state hashes, and row-store contract tests must remain
  stable.
- Result: `apply_delta_collection` now mutates cached row vectors in place:
  deletes drain only matching keys, and upserts replace an existing keyed
  record or append a new one. This keeps durable delta-log semantics unchanged
  while avoiding full-vector record cloning during every targeted cache update.
- Dev score: treatment `harness/latency/score.sh --dev` passed with 54
  results, 36 comparisons, and zero failures. The scored `turn_lifecycle`
  state hash stayed `7fc054292d2f85f0`; Postgres pool-2 c4 p95 improved from
  the 581ms cycle baseline to 517ms.
- High-concurrency turn-state diagnostic: Postgres-only `turn_lifecycle` with
  c32/c100, payload 2048, 64 samples, and pool size 2 completed with zero
  errors and state hash `660086a8484d5400`. c32 p95 improved from 3.27s to
  2.75s; c100 p95 improved from 12.43s to 10.56s.
- Validation: `cargo fmt -p ironclaw_turns --check`, `cargo check -p
  ironclaw_turns`, `cargo test -p ironclaw_turns --test
  filesystem_turn_state_contract
  filesystem_turn_state_row_store_persists_rows_without_state_blob`, `cargo
  test -p ironclaw_turns --test loop_checkpoint_store_contract`, full `cargo
  test -p ironclaw_turns --test filesystem_turn_state_contract`, final `cargo
  check -p ironclaw_turns`, and `git diff --check` passed.

## Cycle 33 - Single-Run Lease Preparation / Pool Sweep

- Graph note: `codebase-memory-mcp` continues to fail with `Transport closed`;
  the local graph artifact remains stale and empty, so this cycle uses targeted
  source reads.
- Baseline: the current commit's treatment `harness/latency/score.sh --dev`
  passed with 54 results, 36 comparisons, and zero failures after cycle 32.
  Dev `turn_lifecycle` state hash stayed `7fc054292d2f85f0`; Postgres pool-2
  c4 p95 was 517ms.
- Probe: current-commit `harness/latency/probe.sh` passed with 81 results, 54
  comparisons, and zero failures. Perturbed Postgres `turn_lifecycle` pool-2
  c8 p95 is now 2.56s with matching state hash `3fa07e3dc3c7e320`; libSQL c8
  still hits CAS retry exhaustion and a mismatched hash.
- Hypothesis: `prepare_runner_lease_retirement` and
  `prepare_cancel_requested_runner_lease` call `read_snapshot()`, which clones
  the whole cached row snapshot just to find one run and seed/update the
  in-memory runner lease. Every block/complete/cancel path pays that cost
  before the actual targeted write, so the lifecycle workload still performs
  extra blob-shaped snapshot copies. Preparing the lease from a single cloned
  `TurnRunRecord` projected under the cache lock should remove that copy while
  preserving the runner-lease validation and rollback behavior.
- Expected failure mode: The single-run path must preserve `ScopeNotFound`
  versus `InvalidTransition` behavior for missing/non-running records, must
  still validate runner id and lease token, and must not weaken cancel-requested
  or terminal transition rollback semantics.
- Result: The single-run lease preparation experiment compiled and passed the
  narrow row-store contract, but it did not improve the score. Dev score still
  passed 54 results and 36 comparisons with zero failures, but Postgres pool-2
  c4 `turn_lifecycle` regressed from the 517ms baseline to 567ms. The c32/c100
  diagnostic was mixed: c32 p95 worsened from 2.75s to 2.78s while c100 p95
  moved from 10.56s to 10.35s. The code was abandoned before commit.
- Pool-size diagnostic: A clean rerun of the cycle-32 implementation swept
  Postgres pool sizes 2, 4, 8, 16, and 32. The c32/c100 `turn_lifecycle`
  diagnostic stayed flat: pool-2 c100 p95 10585ms, pool-4 10547ms, pool-8
  10708ms, pool-16 10606ms, and pool-32 10593ms. The dev-shaped c4 sweep was
  also flat around 269-276ms. Pool size is not the turn-state bottleneck.
- Decision: Do not tune pool size or continue with single-run lease preparation.
  The next cycle must change structure around the remaining serialized
  row-store critical section.

## Cycle 34 - Direct Loop Checkpoint Row Deltas

- Graph note: `codebase-memory-mcp` is available in this turn, but
  `index_status` still fails immediately with `Transport closed`; this cycle
  falls back to crate guardrails and targeted source reads.
- Baseline: cycle 32 remains the last committed implementation. Postgres-only
  `turn_lifecycle` c32/c100 with payload 2048 and pool size 2 sits at c32 p95
  2.75-2.80s and c100 p95 10.56-10.59s with zero errors and state hash
  `660086a8484d5400`. Increasing the pool to 32 does not change that.
- Hypothesis: The 2048-byte lifecycle diagnostic performs 16
  `put_loop_checkpoint` + `get_loop_checkpoint` pairs per sample. Row-store
  checkpoint writes currently enter `apply_with_targeted_delta`, lock the
  global `snapshot_state`, invoke the in-memory turn-state authority, and
  append one metadata row. Loop checkpoint writes do not emit lifecycle events
  and are independent metadata keyed by checkpoint id/scope/run. Persisting the
  checkpoint row directly as a durable targeted delta, then updating only the
  cached snapshot, should remove 16 serialized in-memory transition hops per
  sample without changing visible records or hashes.
- Expected failure mode: Direct checkpoint writes must still create durable
  `LoopCheckpointRecord`s, fail closed on cross-scope/cross-run reads, survive
  row-store reopen, and appear in `persistence_snapshot()`. Because the
  in-memory transition authority will no longer own loop checkpoints, any
  full-snapshot fallback must preserve existing loop checkpoint rows instead
  of treating them as deleted.
- Result: `FilesystemTurnStateRowStore::put_loop_checkpoint` now creates the
  `LoopCheckpointRecord` directly, appends a typed `loop_checkpoints_upsert`
  delta, and applies that delta to the hot cached snapshot if it is already
  loaded. The cache is not initialized just for checkpoint writes. Generic
  full-snapshot diffs now preserve existing loop checkpoint rows so later
  lifecycle transitions do not delete checkpoint rows that no longer live in
  the in-memory store authority.
- Dev score: `harness/latency/score.sh --dev` passed with 54 results, 36
  comparisons, and zero failures. The scored `turn_lifecycle` state hash
  stayed `7fc054292d2f85f0`; libSQL c4 p95 was 8563ms, Postgres pool-1 c4
  p95 was 495ms, and Postgres pool-2 c4 p95 was 502ms. Postgres remains far
  faster than libSQL at c4 in the dev-shaped score, so c4 parity is not the
  problem.
- High-concurrency turn-state diagnostic: Postgres-only `turn_lifecycle` with
  c32/c100, payload 2048, 64 samples, and pool size 2 completed with zero
  errors and state hash `660086a8484d5400`. c32 p95 moved from the cycle-32
  2749ms baseline to 2651ms. c100 p95 moved from 10562ms to 10484ms. This is
  a real but small improvement; it does not materially solve the c100 lifecycle
  latency.
- Decision: Keep this change because it removes an unnecessary serialized
  in-memory hop from typed checkpoint metadata and preserves contracts, but
  the next cycle must address the remaining lifecycle state transitions rather
  than checkpoint writes or pool sizing.
- Validation: `cargo fmt -p ironclaw_turns`, `cargo test -p ironclaw_turns
  --test loop_checkpoint_store_contract
  filesystem_turn_state_row_store_loop_checkpoint_roundtrip_and_snapshot`,
  `cargo test -p ironclaw_turns --test filesystem_turn_state_contract
  filesystem_turn_state_row_store_persists_rows_without_state_blob`, `cargo
  check -p ironclaw_turns`, and `git diff --check` passed.

## Cycle 35 - Claim Lease Seeding Without Snapshot Clone

- Graph note: `codebase-memory-mcp` still fails immediately with
  `Transport closed` for `index_status`; this cycle falls back to crate
  guardrails and targeted source reads.
- Baseline: cycle 34 is the current committed implementation. Full dev score
  passed 54 results and 36 comparisons; dev-shaped c4 `turn_lifecycle` was
  libSQL p95 8563ms versus Postgres pool-2 p95 502ms. The unresolved
  high-concurrency diagnostic is Postgres-only `turn_lifecycle` c100 p95
  10484ms at payload 2048, 64 samples, pool size 2, with state hash
  `660086a8484d5400`.
- Hypothesis: Each `turn_lifecycle` sample claims three runs. After the
  row-store claim transition already persists and applies the claimed run row,
  `claim_next_run` calls `seed_runner_lease_from_snapshot_inner`, which clones
  the whole cached row snapshot and scans it just to seed one external runner
  lease. Seeding the lease from the single claimed `TurnRunRecord` in the hot
  snapshot should remove three post-claim snapshot clones per sample without
  changing durable rows or lease validation semantics.
- Expected failure mode: The new path must preserve the current
  `ScopeNotFound` and `InvalidTransition` errors if the claimed run row is
  missing or no longer lease-eligible, must keep exact lease metadata from the
  persisted run row, and must preserve claim compensation if lease seeding
  fails. State hashes, event counts, and runner lease overlay behavior must
  remain unchanged.
- Result: `FilesystemTurnStateRowStore::claim_next_run` now seeds the external
  runner-lease cache from the single claimed `TurnRunRecord` in the hot
  snapshot instead of cloning the whole row snapshot and scanning it after each
  claim. `RunnerLeaseStore` gained a single-row seeding helper, covered by a
  unit test that asserts the persisted runner id, token, lease expiry,
  heartbeat timestamp, status, and event cursor are copied exactly.
- Dev score: `harness/latency/score.sh --dev` passed with 54 results, 36
  comparisons, and zero failures. The scored `turn_lifecycle` state hash
  stayed `7fc054292d2f85f0`; libSQL c4 p95 was 7244ms, Postgres pool-1 c4
  p95 was 509ms, and Postgres pool-2 c4 p95 was 487ms.
- Probe: `harness/latency/probe.sh` completed with 81 results and 54
  comparisons, but was not clean because the known high-concurrency libSQL
  `turn_lifecycle` c8 baseline produced 3 errors and a mismatched hash
  `22d877ede06452c0`. Postgres pool-1 and pool-2 c8 had zero errors and the
  expected hash `3fa07e3dc3c7e320`, with p95 2622ms and 2646ms respectively.
  This is not a treatment regression, but the probe result is recorded as
  non-clean rather than hidden.
- High-concurrency turn-state diagnostic: Postgres-only `turn_lifecycle` with
  c32/c100, payload 2048, 64 samples, and pool size 2 completed with zero
  errors and state hash `660086a8484d5400`. c32 p95 was essentially flat
  against cycle 34, moving from 2651ms to 2666ms, while c32 p50 improved from
  2287ms to 1923ms. c100 p95 improved from 10484ms to 10195ms and throughput
  improved from 6.10 to 6.28 ops/sec.
- Decision: Keep the change because it removes three post-claim whole-snapshot
  clones per lifecycle sample and improves the c100 bottleneck without changing
  durable rows. The gain is still marginal relative to the remaining 10s c100
  p95, so the next meaningful cycle needs to address the global transition
  serialization itself rather than another post-transition read copy.
- Validation: `cargo fmt -p ironclaw_turns`, `cargo test -p ironclaw_turns
  filesystem_store::runner_lease::tests`, `cargo test -p ironclaw_turns --test
  filesystem_turn_state_contract
  filesystem_turn_state_row_store_persists_rows_without_state_blob`, `cargo
  test -p ironclaw_turns --test loop_checkpoint_store_contract
  filesystem_turn_state_row_store_loop_checkpoint_roundtrip_and_snapshot`,
  `cargo check -p ironclaw_turns`, and `git diff --check` passed. The
  runner-lease unit-test target still emits the pre-existing
  `with_apply_timeout` dead-code warning under `cfg(test)`.

## Cycle 36 - Single-Run Overlay Without Full Store Rebuild

- Graph note: `codebase-memory-mcp` still fails immediately with
  `Transport closed` for `index_status`; this cycle falls back to crate
  guardrails and targeted source reads.
- Baseline: cycle 35 is the current committed implementation. Full dev score
  passed 54 results and 36 comparisons; dev-shaped c4 `turn_lifecycle` was
  libSQL p95 7244ms versus Postgres pool-2 p95 487ms. The unresolved
  high-concurrency diagnostic is Postgres-only `turn_lifecycle` c100 p95
  10195ms at payload 2048, 64 samples, pool size 2, with state hash
  `660086a8484d5400`.
- Hypothesis: Most remaining lifecycle transitions use
  `RunnerLeaseOverlay::Run`. The row store currently handles that by cloning
  the entire hot snapshot, overlaying one runner lease, and rebuilding a full
  `InMemoryTurnStateStore` before running a single-run transition. The
  transition authority only needs the current lease metadata on that run, so
  applying the overlaid lease metadata directly to the hot in-memory store's
  single run before the transition should remove full snapshot clone/rebuild
  from `block_run`, `request_cancel`, `complete_run`, and `cancel_run` without
  changing durable delta semantics.
- Expected failure mode: The in-place overlay must preserve current no-op
  behavior when the runner id/token no longer match, must not resurrect a
  non-running/non-cancel-requested run, must ignore stale heartbeat timestamps,
  and must still let the transition authority raise `LeaseMismatch`, expired
  lease, or invalid-transition errors. If the subsequent transition fails, the
  row-store cache must still be discarded exactly as before so overlay-only
  metadata is not treated as a durable write.
- Result: `apply_with_targeted_delta` now handles `RunnerLeaseOverlay::Run` by
  reading the single run row from the hot snapshot, applying any external
  runner-lease heartbeat overlay to that row, and copying only the overlaid
  lease metadata into the hot `InMemoryTurnStateStore`. `RunnerLeaseOverlay::All`
  still uses the full snapshot overlay path. A new in-memory unit test verifies
  that stale heartbeat overlays are ignored and newer heartbeat/expiry metadata
  is accepted.
- Dev score: The first `harness/latency/score.sh --dev` run had two unrelated
  pool-1 hard-fail outliers in `append_tail` and `trigger_seed_list`, both
  outside this diff and both clean for pool-2. A full rerun passed with 54
  results, 36 comparisons, and zero failures. The rerun `turn_lifecycle` state
  hash stayed `7fc054292d2f85f0`; libSQL c4 p95 was 7310ms, Postgres pool-1
  c4 p95 was 429ms, and Postgres pool-2 c4 p95 was 430ms.
- Probe: `harness/latency/probe.sh` completed with 81 results and 54
  comparisons, but was not clean because the known high-concurrency libSQL
  `turn_lifecycle` c8 baseline produced 5 errors and a mismatched hash
  `0fa873b89e21ddc0`. Postgres pool-1 and pool-2 c8 had zero errors and the
  expected hash `3fa07e3dc3c7e320`, with p95 2229ms and 2187ms respectively.
  This is not a treatment regression, but the probe result is recorded as
  non-clean rather than hidden.
- High-concurrency turn-state diagnostic: Postgres-only `turn_lifecycle` with
  c32/c100, payload 2048, 64 samples, and pool size 2 completed with zero
  errors and state hash `660086a8484d5400`. c32 p95 improved from cycle 35's
  2666ms to 2314ms, and throughput improved from 16.22 to 18.36 ops/sec. c100
  p95 improved from 10195ms to 8666ms, and throughput improved from 6.28 to
  7.39 ops/sec.
- Decision: Keep the change. This is the first cycle in this stretch that
  materially attacks the global transition serialization cost: single-run
  overlay transitions no longer rebuild the whole in-memory authority. The
  remaining c100 p95 is still high, so the next structural step should continue
  reducing whole-snapshot work inside targeted transitions, especially terminal
  fallback scans and full-vector delta construction.
- Validation: `cargo fmt -p ironclaw_turns`, `cargo test -p ironclaw_turns
  memory::tests::overlay_runner_lease_record_ignores_stale_heartbeat`, `cargo
  test -p ironclaw_turns --test filesystem_turn_state_contract
  filesystem_turn_state_row_store_persists_rows_without_state_blob`, `cargo
  test -p ironclaw_turns --test loop_checkpoint_store_contract
  filesystem_turn_state_row_store_loop_checkpoint_roundtrip_and_snapshot`,
  `cargo check -p ironclaw_turns`, full dev score rerun, probe, and `git diff
  --check` passed. The unit-test target still emits the pre-existing
  `with_apply_timeout` dead-code warning under `cfg(test)`.

## Cycle 37 - Sparse Snapshot Delta Encoding

- Graph note: `codebase-memory-mcp` still fails immediately with
  `Transport closed` for `index_status`; this cycle falls back to crate
  guardrails and targeted source reads.
- Baseline: cycle 36 is the current committed implementation. Full dev score
  rerun passed 54 results and 36 comparisons; dev-shaped c4 `turn_lifecycle`
  was libSQL p95 7310ms versus Postgres pool-2 p95 430ms. The unresolved
  high-concurrency diagnostic is Postgres-only `turn_lifecycle` c100 p95
  8666ms at payload 2048, 64 samples, pool size 2, with state hash
  `660086a8484d5400`.
- Hypothesis: Every row-store transition durably appends a JSON
  `SnapshotDelta`. The common targeted deltas touch one or two row collections,
  but the serialized JSON still contains every empty vector field and a null
  `event_retention_floor`. Marking delta fields as serde-default and skipping
  empty vectors/options should reduce serialization and filesystem append bytes
  for every transition without changing replay semantics. Existing full-object
  deltas should still deserialize because defaults are explicit.
- Expected failure mode: Sparse delta JSON must deserialize with missing fields
  as empty/default values, preserve full-snapshot fallback fields when present,
  and leave state hashes unchanged. If any field lacks a default, replay from a
  sparse delta log could drop records or fail after restart.
- Result: The sparse delta encoding compiled and passed the focused row-store
  contracts, but did not improve the high-concurrency signal. The code change
  was abandoned before commit, so the durable delta wire format remains the
  cycle-36 full-object JSON shape.
- High-concurrency turn-state diagnostic: Postgres-only `turn_lifecycle` with
  c32/c100, payload 2048, 64 samples, and pool size 2 completed with zero
  errors and state hash `660086a8484d5400`. c32 was essentially flat/slightly
  better, moving from cycle 36 p95 2314ms to 2299ms. c100 regressed slightly,
  moving from 8666ms to 8700ms and throughput from 7.39 to 7.36 ops/sec.
- Decision: Do not keep sparse delta encoding. The next cycle must change
  approach rather than continue tuning durable delta JSON size; the remaining
  signal is more likely in transition count/critical-section structure than
  field-name payload overhead.
- Validation before abandoning: `cargo fmt -p ironclaw_turns`, `cargo test -p
  ironclaw_turns
  filesystem_store::row_store::tests::snapshot_delta_serializes_sparse_and_defaults_missing_fields`,
  `cargo test -p ironclaw_turns --test filesystem_turn_state_contract
  filesystem_turn_state_row_store_persists_rows_without_state_blob`, `cargo
  test -p ironclaw_turns --test loop_checkpoint_store_contract
  filesystem_turn_state_row_store_loop_checkpoint_roundtrip_and_snapshot`, and
  `cargo check -p ironclaw_turns` passed. The unit-test target emitted the
  pre-existing `with_apply_timeout` dead-code warning under `cfg(test)`.

## Cycle 38 - Group-Commit Delta Journal

- Graph note: `codebase-memory-mcp` still fails immediately with
  `Transport closed` for `index_status`; this cycle falls back to crate
  guardrails and targeted source reads.
- Baseline: cycle 37 is log-only, so cycle 36 remains the current
  implementation baseline. Full dev score rerun passed 54 results and 36
  comparisons; dev-shaped c4 `turn_lifecycle` was libSQL p95 7310ms versus
  Postgres pool-2 p95 430ms. The unresolved high-concurrency diagnostic is
  Postgres-only `turn_lifecycle` c100 p95 8666ms at payload 2048, 64 samples,
  pool size 2, with state hash `660086a8484d5400`.
- Hypothesis: the c100 tail is a write convoy: each transition builds a small
  row-store `SnapshotDelta`, then holds the snapshot mutex while awaiting one
  filesystem append. A single delta-journal flusher that drains queued deltas
  and persists a batch in one append should amortize CAS/filesystem overhead.
  The snapshot mutex should cover only in-memory apply plus delta construction,
  with per-delta acks awaited after the mutex is released.
- Validation target: focused Postgres-only `turn_lifecycle` c100 sweep first.
  The gate is p95 magnitude plus throughput. Packed p50/p99 under closed-loop
  simultaneous arrivals and a FIFO-fair mutex is expected and is not a failure
  signal unless the harness moves to open-loop arrivals. Replay remains
  unchanged because the flusher uses `filesystem.append_batch` with one
  serialized delta per record.
- Result: Kept the yield-only group-commit flusher. `FilesystemTurnStateRowStore`
  now enqueues each non-empty `SnapshotDelta` with a per-delta ack, a single
  flusher drains queued deltas into one `ScopedFilesystem::append_batch` call
  per flush, and both generic and targeted apply paths release the snapshot
  mutex before awaiting durable persistence. Empty deltas still short-circuit,
  single-delta flushes use `append`, and journal replay remains compatible
  with existing single-delta records.
- High-concurrency turn-state diagnostic: Postgres-only `turn_lifecycle` with
  payload 2048, 64 samples, and pool size 2 completed with zero errors and
  stable state hash `660086a8484d5400`. c32 p95 improved from the cycle-36
  2314ms baseline to 2142ms, with p50 1773ms, p99 2230ms, and throughput
  19.96 ops/sec. c100 p95 improved from 8666ms to 3519ms, with p50 3445ms,
  p99 3520ms, and throughput 18.18 ops/sec.
- Full-flow stress signal: `ironclaw_stress` mixed-user-session with Postgres
  row turn state, pool size 2, c100, users 100, model/tool latency 0, and
  200 total operations completed 200/200 with operation p95 400.6ms,
  throughput 339.7 ops/sec, turn-store p95 70.6ms, and resource-governor p95
  237.4ms. A longer sustained c100 run with 20,000 total operations also
  completed 20,000/20,000; operation p95 was 722.1ms, throughput 217.2 ops/sec,
  turn-store p95 22.0ms, and resource-governor p95 607.2ms, keeping the
  governor as the top full-flow bottleneck.
- Dev score and validation: `harness/latency/score.sh --dev` passed 54 results
  and 36 comparisons with zero failures. Dev-shaped c4 `turn_lifecycle` stayed
  well ahead of libSQL: libSQL p95 8415ms, Postgres pool-1 p95 447ms, and
  Postgres pool-2 p95 423ms. Additional checks passed:
  `harness/latency/lint.sh`, `cargo check -p ironclaw_turns`, `cargo test -p
  ironclaw_turns --test filesystem_turn_state_contract
  filesystem_turn_state_row_store_persists_rows_without_state_blob`, and
  `cargo test -p ironclaw_turns --test loop_checkpoint_store_contract
  filesystem_turn_state_row_store_loop_checkpoint_roundtrip_and_snapshot`.
- Next lever: shard the in-memory turn state per run. Use per-run locks for
  targeted transitions, keep the global lock only for `claim_next_run`'s
  cross-run view, and keep journal ordering in the single flusher. While doing
  that, audit `build_delta`/`apply_delta` for O(snapshot) vector scans or
  rebuilds per write and consider row-keyed maps for run storage.

## Cycle 39 - Indexed Row Snapshot Delta Apply

- Graph note: `codebase-memory-mcp` still fails immediately with
  `Transport closed` for `index_status`; this cycle falls back to crate
  guardrails and targeted source reads.
- Baseline: cycle 38 is the current committed implementation. The focused
  Postgres-only c100 `turn_lifecycle` diagnostic improved from p95 8666ms to
  3519ms and throughput 7.39 to 18.18 ops/sec, but the remaining p95 is still
  too high for the hosted-single-tenant target.
- Gate update: do not use p50/p99 separation as the health gate for this closed
  loop harness. With simultaneous arrivals and a FIFO-fair mutex, packed
  percentiles are expected. Gate on p95 magnitude and throughput unless the
  c100 harness moves to open-loop arrivals.
- Hypothesis: after group commit removed durable append from the snapshot mutex,
  the next visible cost is per-transition O(snapshot) work while holding that
  mutex. In the common targeted paths, `apply_delta_collection` scans Vec-backed
  rows to replace one run, active lock, reservation, or event. Maintaining
  row-keyed indexes for the hot cached row snapshot should make common upserts
  O(delta) while preserving the existing Vec snapshot contract, delta replay,
  and journal ordering.
- Expected failure mode: index maintenance must preserve record order for
  existing snapshots, rebuild indexes after deletes, and keep restart replay
  compatible with the unindexed `TurnPersistenceSnapshot` representation. If
  this does not move c100 p95/throughput, the next larger lever is true per-run
  in-memory lock sharding for targeted transitions.
- Result: Keep the indexed hot row snapshot change. `RowSnapshotState` now
  builds row-keyed indexes next to the cached `TurnPersistenceSnapshot`, uses
  those indexes for targeted delta apply, and keeps the plain Vec snapshot for
  persistence snapshots and replay. Deletes still preserve row order and
  rebuild only the touched collection index; common upserts replace by key
  without scanning the whole Vec. Replay remains on the unindexed
  `apply_delta` path so existing delta logs stay compatible.
- High-concurrency turn-state diagnostic: Postgres-only `turn_lifecycle` with
  `LATENCY_CONCURRENCY=32,100`, `LATENCY_PAYLOAD_BYTES=2048`,
  `LATENCY_SAMPLES=64`, pool size 2, and the default 30 warmups completed with
  zero errors and stable state hash `660086a8484d5400`. c32 p95 improved from
  cycle 38's 2142ms to 258ms and throughput from 19.96 to 138.85 ops/sec.
  c100 p95 improved from 3519ms to 790ms and throughput from 18.18 to
  80.95 ops/sec.
- Dev score: `harness/latency/score.sh --dev` passed all 54 result rows and
  36 comparisons with zero failures. Dev-shaped `turn_lifecycle` stayed far
  ahead of libSQL: libSQL c4 p95 7816ms, Postgres pool-1 c4 p95 32ms, and
  Postgres pool-2 c4 p95 35ms, with matching state hash `7fc054292d2f85f0`.
- Full-flow stress signal: `ironclaw_stress` mixed-user-session with Postgres
  row turn state and pool size 2 completed c32 128/128 and c100 200/200 with
  zero failures. c32 operation p95 was 158.8ms, throughput 280.2 ops/sec, and
  turn-store p95 30.8ms. c100 operation p95 was 467.7ms, throughput
  287.4 ops/sec, and turn-store p95 85.5ms. The resource governor remains the
  top full-flow bottleneck at c100 with p95 271.0ms.
- Validation: `cargo fmt -p ironclaw_turns`, `cargo check -p
  ironclaw_turns`, `cargo test -p ironclaw_turns --test
  filesystem_turn_state_contract
  filesystem_turn_state_row_store_persists_rows_without_state_blob`, `cargo
  test -p ironclaw_turns --test loop_checkpoint_store_contract
  filesystem_turn_state_row_store_loop_checkpoint_roundtrip_and_snapshot`,
  focused c32/c100 lifecycle diagnostic, full dev score, and the c32/c100
  mixed-flow stress slices passed.
- Next lever: the current change removes the dominant O(snapshot) Vec scan in
  hot row delta apply. It does not yet shard the in-memory transition authority
  per run; if focused c100 p95 around 790ms is still too high, implement
  per-run locks for targeted transitions next, with the global view retained
  only for `claim_next_run`/cross-run operations and journal ordering still
  centralized in the flusher.

## Cycle 40 - RootFilesystem Journaled Resource Governor

- Graph note: `codebase-memory-mcp` still fails with `Transport closed` for
  `index_status`, `index_repository`, and `search_code`; this cycle falls back
  to crate guardrails and targeted source reads.
- Baseline: cycle 39 moved turn-state out of the top full-flow position.
  `ironclaw_stress` mixed-user-session c100/pool-2 reported operation p95
  467.7ms, turn-store p95 85.5ms, and resource-governor p95 271.0ms.
- Direction change: abandon direct `postgres_governor.rs` SQL optimization.
  Quotas are documented as process-global, so this cycle makes the in-process
  governor authority the production path for both libSQL and Postgres,
  persisting through the existing `RootFilesystem` abstraction instead of a
  resource-governor-specific Postgres schema.
- Result: added `FilesystemResourceGovernor`, rewired hosted/local/stress/
  latency-runner construction to use it for both filesystem-backed backends,
  and removed the direct Postgres resource governor module plus its optional
  direct database dependencies. The `postgres` Cargo feature remains as a
  compatibility feature but no longer exposes direct resource-governor SQL.
- Runtime shape: hot reserve/reconcile/release updates per-account-sharded
  in-memory tallies plus a reservation map, then enqueues one
  `ResourceGovernorDelta`. A single delta-journal flusher batches queued
  deltas into `ScopedFilesystem::append_batch` and acks callers only after the
  append returns. Set-limit, reserve denial/warning, reconcile, release, period
  rollover, and budget events still reuse the existing shared governor
  semantics.
- Recovery and compaction: startup reads the compacted
  `FilesystemResourceGovernorStore` snapshot, rebuilds tallies from persisted
  reservations, and replays `/resources/deltas/log` from `journal_seq`.
  Compaction is best-effort background maintenance only: it rebuilds a new
  compacted snapshot from the durable snapshot plus durable journal records,
  then records the matching `journal_seq`. It deliberately does not snapshot
  the hot in-memory authority because memory can include deltas that have not
  yet received durable journal sequence numbers.
- Correctness fix during validation: the first focused c100 reserve/reconcile
  control stopped at 822/1000 after synchronous compaction entered the hot
  path. Moving compaction to the durable background replay path fixed the hang
  and kept reserve/reconcile durability gated only on the delta journal ack.
- Full-flow gate: `ironclaw_stress` mixed-user-session with Postgres
  filesystem-row turn state, pool size 2, c100, users 100, and zero synthetic
  model/tool latency completed 200/200 with zero failures. Final operation p95
  was 291.2ms, throughput 482.5 ops/sec, turn-store p95 124.7ms, and
  resource-governor p95 31.1ms. The old c100 governor p95 was 271.0ms, so the
  governor is no longer the top attributed group; thread-store writes are now
  top at p95 125.7ms.
- Resource-only control: Postgres reserve-reconcile c100/pool-2 completed
  1000/1000 with zero failures after the compaction fix, operation p95 41.9ms,
  and throughput 5584.5 ops/sec. A pre-fix run reached 822/1000 and stopped
  making progress, which is now covered by the validation note above.
- Validation: `cargo fmt -p ironclaw_resources -p ironclaw_stress -p
  ironclaw_reborn_composition -p ironclaw_host_runtime`, `cargo check -p
  ironclaw_resources --features postgres,libsql`, `cargo test -p
  ironclaw_resources --features postgres,libsql --test
  resource_governor_contract`, `cargo check -p ironclaw_host_runtime --features
  postgres,libsql`, `cargo check -p ironclaw_stress --features
  postgres,libsql`, `cargo check -p ironclaw_reborn_composition --features
  postgres,libsql`, `cargo check --manifest-path
  harness/latency/runner/Cargo.toml`, `cargo test -p ironclaw_resources
  --features postgres,libsql
  compaction_snapshot_cursor_does_not_double_apply_journal_on_restart`, the
  c100 mixed-flow gate, and the c100 reserve-reconcile control passed.
- Separate acceptance issue: the libSQL c4 `turn_lifecycle` baseline remains a
  launch-parity concern from earlier cycles. This cycle removes the Postgres
  resource-governor bottleneck; it does not repair the libSQL turn-state
  baseline.

## Cycle 41 - Post-Governor Thread Store Write Path

- Graph note: `codebase-memory-mcp` is discoverable in this session, but
  `list_projects` fails with `Transport closed`; this cycle falls back to
  crate guardrails, the locked latency harness, and targeted source reads.
- Required dev score: `harness/latency/score.sh --dev` passed. All dev
  comparisons were green for Postgres pool sizes 1 and 2. The dev profile is
  still not acceptance-ready; it reports 5 warmups, 40 measured samples,
  concurrencies 1/4, and notes that launch-ref libSQL plus request-level
  trigger/approval/resource workloads are still required for acceptance.
- Required probe: `harness/latency/probe.sh` completed the larger perturbed
  profile with path depths 2/5, payload sizes 128/2048, concurrencies 1/3/8,
  and pool sizes 1/2. The probe still hard-fails only where the libSQL side
  changes state hash under c8 pressure: `control_plane_snapshot` c8 has one
  libSQL secret-store filesystem error, and `turn_lifecycle` c8 has two libSQL
  `turn state filesystem CAS retries exhausted` errors. Postgres remains
  error-free in those rows and materially faster, so the probe is useful as a
  baseline-health warning rather than a Postgres latency regression.
- Current Postgres bottleneck from the full mixed-flow gate after Cycle 40:
  c100/pool-2 operation p95 is 291.2ms; resource-governor p95 is down to
  31.1ms, while `thread_store_writes` p95 is 125.7ms and `turn_store` p95 is
  124.7ms. The next Postgres latency lever should target the filesystem-backed
  thread/turn write path, not the governor.
- Hypothesis: mixed-user-session still pays too many independent durable
  filesystem writes during turn admission and assistant append/finalization.
  The governor fix proved RootFilesystem group-commit can remove a hot
  Postgres-attributed span without bypassing persistence. Inspect the thread
  store and turn-store write path for serial per-message/metadata writes that
  can be collapsed into existing batch primitives (`put_many`, `append_batch`,
  or transaction-shaped helpers) while keeping identical visible responses,
  event counts, and state hashes.
- Expected failure mode: batching thread writes incorrectly could change
  message ordering, record-kind metadata, search/query visibility, or state
  hashes. The change must stay behind the real RootFilesystem abstraction,
  preserve durable acks, and avoid any benchmark-path or payload-size special
  case. If the thread-store path already uses the available batch primitive,
  switch approach to the turn-store append/claim/submit path rather than
  tuning the same knob.

## Cycle 42 - LibSQL Turn-State Cliff Diagnostic

- Supersedes the Cycle 41 implementation direction. The next gate is the
  libSQL `turn_lifecycle` cliff on the identical RootFilesystem-backed row
  store: libSQL c4 p95 is about 7.8s while Postgres c4 is about 35ms. A
  200x gap is treated as one pathological cause, not general slowness.
- Graph note: `codebase-memory-mcp` remains unavailable (`Transport closed`),
  so this cycle uses crate guardrails and targeted source reads.
- Pre-instrumentation observations: `LibSqlRootFilesystem` already migrates to
  `PRAGMA journal_mode = WAL` and applies `synchronous = NORMAL` per
  connection, so fsync-per-append is not assumed to be dominant. The backend
  does create a fresh libSQL connection for every RootFilesystem operation and
  applies the PRAGMA batch on each connect. The turn-state row store already
  uses the delta-journal flusher and `append_batch` for grouped deltas, with
  single-delta flushes falling back to `append`.
- Diagnostic plan before any fix: add opt-in libSQL RootFilesystem timing that
  emits per-phase timings for `connect`/PRAGMA setup, write lock/transaction
  begin, SQL execution, row iteration, and commit for `append`,
  `append_batch`, `put`, `get`, `tail`, and `reserve_sequence`. Attribute the
  c4 `turn_lifecycle` time across the requested suspects: journal/synchronous
  mode, connection-per-op cost, append shape, unprepared statement execution,
  and write-lock serialization. Record the dominant cause with measured
  numbers here before applying a fix.
- Diagnostic run: `LATENCY_WORKLOADS=turn_lifecycle`, c4, 5 warmups, 40
  measured samples, libSQL + Postgres pool-2. Output captured under
  `target/latency-diagnostics/cycle42-c4.*` with
  `IRONCLAW_LIBSQL_FS_DIAG=1`.
- Measured result before fix: libSQL p50 2029.9ms, p95 4691.5ms, p99
  5329.8ms, throughput 1.82 ops/sec; Postgres pool-2 p50 27.2ms, p95
  37.7ms, p99 44.1ms, throughput 138.0 ops/sec. State hashes matched
  (`7fc054292d2f85f0`) and errors were zero.
- Dominant cause: the libSQL `turn_lifecycle` path is not actually using the
  row store in the latency runner. `harness/latency/runner` constructs
  libSQL with `FilesystemTurnStateStoreKind::blob(scoped)` while Postgres uses
  `FilesystemTurnStateStoreKind::row(scoped)`. The diagnostic log confirms all
  turn-state filesystem traffic for libSQL is `/turns/state.json` blob
  `get`/`put`, with no turn-state `append`/`append_batch` traffic.
- Blob-store numbers: one run produced 1,753 turn-state `get`s and 1,348
  versioned `put` attempts against the single `/turns/state.json` blob. The
  snapshot body was p50 732,946 bytes, p95 1,234,918 bytes, max 1,293,813
  bytes. Only 540 of 1,348 versioned updates succeeded; 808 returned zero
  rows and paid a `current_version` lookup, so 60.0% of write attempts were CAS
  retries over the growing full snapshot.
- Suspect attribution: WAL/synchronous is already `journal_mode=WAL` plus
  `synchronous=NORMAL`, and append is not on the hot path in this libSQL run.
  Fresh connections/PRAGMAs are measurable but secondary: 6,607 opens totaled
  527ms and 6,606 PRAGMA batches totaled 3,764ms (PRAGMA p95 1.76ms). The
  dominant cost is full-blob read/modify/write amplified by CAS conflicts and
  SQLite's single-writer serialization: `put` preflight totaled 2,928ms,
  `put` execute totaled 3,383ms, successful `put` total p95 was 10.0ms, and
  each losing CAS attempt still rewrote the same large logical state path.
- Fix direction after measurement: stop using the blob turn-state layout for
  libSQL in the latency/hosted-single-tenant path; move libSQL to the existing
  RootFilesystem row store so it uses grouped delta `append_batch` like
  Postgres. Keep libSQL backend PRAGMA/config changes out of the first fix
  because the measured cliff is not fsync mode or statement parse in the row
  append path.
- Fix applied: production libSQL hosted-single-tenant wiring and the latency
  runner now use `FilesystemTurnStateStoreKind::row` for turn state. The
  temporary libSQL RootFilesystem diagnostic hooks were removed after this
  attribution so the final hot path does not retain benchmark instrumentation
  overhead.
- Post-fix diagnostic run with the same c4 focused profile and temporary
  filesystem timing confirmed the path changed from `/turns/state.json`
  `get`/`put` traffic to row-store delta journal traffic:
  `/turns/rows/v1/deltas/log` saw 580 `append` and 545 `append_batch` phase
  records, while the meta snapshot path saw only four `get`s and four `tail`s.
  libSQL c4 p95 fell to 30.0ms, Postgres pool-2 c4 p95 was 31.2ms, errors were
  zero, and state hashes matched (`7fc054292d2f85f0`).
- Clean launch-ref-shaped baseline rerun without diagnostics:
  `LATENCY_WORKLOADS=turn_lifecycle`, c4, 5 warmups, 40 measured samples,
  libSQL + Postgres pool-2. libSQL p50/p95/p99 was 27.7/30.9/32.2ms with
  throughput 143.9 ops/sec; Postgres p50/p95/p99 was 23.4/27.6/28.8ms with
  throughput 168.2 ops/sec. Errors were zero and state hashes matched. This
  clears the requested libSQL c4 gate (target <=100ms, within about 2x of
  Postgres); libSQL is 1.12x Postgres p95 in this clean focused run.
- Acceptance note: this run is recorded as the libSQL turn-lifecycle baseline
  evidence for the acceptance-ready path, but the runner still reports
  `acceptance_ready=false` because the full harness flag is currently hard-coded
  and still represents missing request-level trigger/approval/resource gates,
  not this focused turn-state baseline.
- Full dev scorer after the fix: `harness/latency/score.sh --dev` produced 54
  result rows and 36 comparison rows. The turn-lifecycle c4 rows passed for
  both Postgres pool sizes with zero errors and matching hashes; libSQL c4 p95
  was 39.8ms, Postgres pool-1 c4 p95 was 31.4ms, and Postgres pool-2 c4 p95
  was 34.0ms. The run had two c1 p99-only hard-fail rows (`put_get` pool-1 and
  `turn_lifecycle` pool-2) despite faster Postgres p50/p95 and matching state
  hashes.
- Small-sample outlier check: reran the two affected workloads at c1 with 30
  warmups and 300 measured samples. All four comparisons passed with zero
  failures and matching hashes. `put_get` Postgres p99 ratios were 0.29
  (pool-1) and 0.13 (pool-2); `turn_lifecycle` Postgres p99 ratios were 0.52
  (pool-1) and 0.92 (pool-2).
- Postgres c100 mixed-flow regression gate: `ironclaw_stress` with
  mixed-user-session, filesystem-row turn state, pool size 2, concurrency 100,
  users 100, and zero synthetic model/tool latency completed 200/200 with zero
  failures. Operation p95 was 245.8ms, throughput was 517.4 ops/sec,
  turn-store p95 was 81.4ms, resource-governor p95 was 15.8ms, and
  thread-store writes remain the top group at p95 148.3ms. This does not
  regress the prior c100 gate (291.2ms op p95, 482.5 ops/sec).
- Validation: `cargo fmt -p ironclaw_reborn_composition --check`,
  `cargo check -p ironclaw_reborn_composition --features libsql,postgres`,
  `cargo check --manifest-path harness/latency/runner/Cargo.toml`,
  `cargo test -p ironclaw_turns --test filesystem_turn_state_contract`,
  `cargo test -p ironclaw_filesystem --features libsql,postgres --test
  db_root_filesystem_contract`, `cargo test -p ironclaw_reborn_composition
  --features libsql,postgres --test libsql_substrate --test
  postgres_substrate`, the focused c4 clean latency run, the focused c1
  outlier rerun, the full dev scorer, and the c100 mixed-flow stress gate were
  run. Contract tests passed for both backends.
