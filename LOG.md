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
