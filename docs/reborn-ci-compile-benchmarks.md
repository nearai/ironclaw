# Reborn CI Compile Benchmark Report

## Goal

Find CI changes that either:

- reduce the total number of jobs while keeping the current wall-clock baseline, or
- reduce Reborn CI build time by 30-50%.

This report tracks benchmark hypotheses against the current `main` baseline. A
change is a candidate to keep only when the measured CI result improves one of
those acceptance criteria without weakening test coverage.

## Current Baseline

Baseline run: [`28718496861`](https://github.com/nearai/ironclaw/actions/runs/28718496861)

- Commit: `28c3e9448`
- Workflow: `Tests (Reborn)`
- Status: success
- Wall clock: `8m35s` (`2026-07-04T20:20:32Z` to `2026-07-04T20:29:07Z`)
- Total Reborn jobs: `28`
- Reborn crate bucket jobs: `12`

Slowest crate buckets:

| Bucket | Duration |
| --- | ---: |
| `host-runtime` | `473s` |
| `composition-core` | `419s` |
| `reborn-core` | `367s` |
| `webui-ingress` | `334s` |
| `wasm-sandbox` | `307s` |

Evidence from the baseline `main` run shows the long buckets are mostly
compile/link/setup time, with one notable runtime-heavy exception:

- `host-runtime`: first test output arrived at `20:26:28`, about `5m21s`
  after the bucket started; `tests/github_wasm_runtime_contract.rs` then took
  `142.73s`. The bucket still spent more time before tests than inside that
  slow test binary.
- `webui-ingress`: test binaries were roughly seconds to low tens of seconds;
  most time was compile/link/setup.
- `composition-core`: first test output arrived at `20:26:22`, about `5m15s`
  after the bucket started; the unit-test binary took `47.24s` and several
  integration binaries took `11-16s`, but compile/link/setup remained the
  dominant cost.

The current branch has restored the workflow after each rejected benchmark.
H13 is the first retained benchmark because it reduces total Reborn jobs while
improving measured wall clock.

## Hypotheses

| ID | Hypothesis | Expected effect | Status | Result |
| --- | --- | --- | --- | --- |
| H1 | Narrow Reborn crate bucket targets from `--all-targets` to the default `cargo test` target set. | Less compile/link work per crate bucket. | Tested | Rejected: wall clock regressed from `8m35s` to `8m43s`; job count unchanged. |
| H2 | Split compile-heavy buckets by dependency shape instead of package count. | Lower max bucket duration if closures are separable. | Evidence review | Rejected for the current long poles: `host-runtime` and `composition-core` are already single-crate buckets, and splitting them would duplicate compile work. |
| H3 | Move the runtime-heavy host WASM contract tests out of the normal `host-runtime` bucket. | Reduce the slowest bucket and isolate slow test execution. | Evidence review | Rejected as a compile-time optimization: `github_wasm_runtime_contract.rs` takes `142.73s`, but the bucket spends about `5m21s` compiling before tests start. Splitting would add a job and duplicate compile. |
| H4 | Reduce feature flags for slow crates where the coverage is duplicated elsewhere. | Less compile graph expansion in PR crate buckets. | Evidence review | Still the best compile-time lever, but no safe duplicate coverage was found for removing `webui-v2-beta` / `slack-v2-host-beta` from Reborn crate tests. |
| H5 | Remove OVH sccache from Reborn crate buckets. | Verify whether remote cache overhead is hiding any local cache gain. | Tested | Rejected: wall clock regressed from `8m35s` to `9m45s`; job count unchanged. |
| H6 | Disable incremental compilation across all Reborn crate buckets. | Avoid CI-only incremental bookkeeping and target-dir churn for one-shot builds. | Tested | Rejected: wall clock regressed from `8m35s` to `8m49s`; job count unchanged. |
| H7 | Remove the duplicate instrumented `reborn_group_*` coverage lane from PR CI. | Reduce Reborn job count while keeping the uninstrumented group pass/fail gate. | Tested | Rejected: total jobs dropped from `28` to `27`, but wall clock regressed from `8m35s` to `9m39s`. |
| H8 | Merge only the two smallest Reborn crate buckets, `auth-security` and `memory-skills`. | Reduce crate bucket jobs by one while leaving all long-pole buckets unchanged. | Tested | Reverted by decision: total jobs dropped from `28` to `27`, but the compile-heavy long poles remained unchanged. |
| H9 | Enable sccache distributed compilation for all Reborn crate buckets. | Reduce compile time for the existing long-pole buckets without changing coverage. | Tested | Rejected: wall clock regressed from `8m35s` to `9m11s`; long-pole buckets were slower. |
| H10 | Remove the duplicate `libsql-restart-tests` feature from the broad `ironclaw_reborn` crate bucket. | Reduce `reborn-core` runtime while preserving the dedicated restart-test PR gate. | Reverted | Abandoned: package-node evidence showed no compile-graph shrink, and the active run still left the compile-heavy long poles as the blocker. |
| H11 | Move libSQL-heavy coverage out of broad long-pole crate buckets into the existing Reborn group job. | Reduce compile graph size for `host-runtime` and `reborn-core` without dropping persistence coverage or adding a new job. | Tested | Rejected: wall clock regressed from `8m35s` to `9m50s`; `host-runtime`, `reborn-core`, and `composition-core` were all slower. |
| H12 | Move `ironclaw_reborn_cli` from `reborn-core` to `webui-ingress`. | Keep job count flat while grouping the WebUI-shaped CLI build with the WebUI ingress bucket instead of the core Reborn bucket. | Tested | Rejected: `reborn-core` improved from `367s` to `238s`, but `webui-ingress` grew from `334s` to `377s`; wall clock stayed flat at `8m37s` with no job-count reduction. |
| H13 | Remove the separate uninstrumented `reborn-group-tests` job and rely on the existing instrumented coverage `groups` lane for those same suites. | Reduce total Reborn jobs by one without dropping group-suite pass/fail coverage. | Retained | Accepted: total Reborn jobs dropped from `28` to `27`, and wall clock improved from `8m35s` to `8m21s`. |
| H14 | Seed fresh shared Rust caches from deterministic broad producers: `reborn-core` for crate buckets and `groups` for coverage lanes. | Improve warm-start quality for compile-heavy jobs without adding jobs or changing coverage. | Retained with caveat | Accepted as a targeted cache-policy cleanup: the controlled measurement improved from H13 `8m21s` to `7m49s`, but later final-state validation runs regressed to `12m07s`, `12m44s`, and `12m22s`, so this is not a stable wall-clock fix. |
| H15 | Add a `cargo-hakari` workspace-hack crate to unify dependency feature sets across buckets. | Improve cross-bucket cache reuse by making shared dependency artifacts compatible across package feature combinations. | Tested | Rejected: the benchmark run failed architecture dependency-boundary tests and was already slower than the original baseline, with root `1` at `10m21s`, `host-runtime` at `8m24s`, and `composition-core` at `8m21s`. |
| H16 | Build a nightly dependency-warmed CI container image in GHCR and run heavy Reborn jobs inside it. | Escape GitHub cache LRU limits by pre-baking a broad warm target/dependency layer. | Feasibility review | Not safe to add as an inline benchmark: no existing Reborn dependency image path; requires package write permissions, GHCR retention policy, container hardening, and a separate seed/build workflow before PR timing is meaningful. |
| H17 | Skip Reborn buckets by changed-file reverse dependency scope. | Improve average PR time by running only affected buckets while merge queue still runs everything. | Feasibility review | Not a compile-time benchmark for the current full-PR target: current scope detection is all-or-nothing for Reborn, and safe bucket-level skipping needs crate ownership plus reverse-dependency mapping before it can be trusted. |
| H18 | Increase Reborn root-test partitions from 4 to 6. | Shave the new post-H14 root-test long pole without changing test coverage. | Tested | Rejected: wall clock regressed from H14 `7m49s` to `10m41s`; the slowest root partition worsened from `450s` to `595s`. |

## H1: Narrow Crate Bucket Targets

Change under test:

```diff
- cargo test -p "$package" ${feature_flags} --all-targets -- --nocapture
+ cargo test -p "$package" ${feature_flags} -- --nocapture
```

Why this is safe to test:

- The experiment changes only the benchmark branch.
- The default `cargo test -p` target set still runs the package's normal test
  suite without explicitly forcing every target kind.
- It avoids the extra compile/link work caused by explicitly requesting all
  targets for every Reborn crate bucket.

Benchmark result:

- Branch/PR: [`codex/ci-compile-benchmarks`, PR #5648](https://github.com/nearai/ironclaw/pull/5648)
- Workflow run: [`28719290187`](https://github.com/nearai/ironclaw/actions/runs/28719290187)
- Status: success
- Wall clock: `8m43s` (`2026-07-04T20:51:27Z` to `2026-07-04T21:00:10Z`)
- Crate bucket job count: `12`
- Slowest bucket: `host-runtime` at `461s`
- Decision: reject. This did not meet either acceptance criterion.

Comparison against baseline:

| Metric | Baseline | H1 | Delta |
| --- | ---: | ---: | ---: |
| Reborn workflow wall clock | `8m35s` | `8m43s` | `+8s` |
| Crate bucket job count | `12` | `12` | `0` |
| `host-runtime` | `473s` | `461s` | `-12s` |
| `composition-core` | `419s` | `457s` | `+38s` |
| `reborn-core` | `367s` | `309s` | `-58s` |
| `webui-ingress` | `334s` | `310s` | `-24s` |
| `wasm-sandbox` | `307s` | `279s` | `-28s` |

Interpretation:

Removing `--all-targets` helped some buckets, but it made `composition-core`
slower and did not reduce overall wall clock. The result is within normal CI
variance for several buckets and does not justify weakening or changing the
crate-test target selection.

## H2: Split Compile-Heavy Buckets by Dependency Shape

Change considered:

- Split the slowest crate buckets further by dependency closure instead of
  package count.

Evidence:

- The current slowest bucket, `host-runtime`, contains only
  `ironclaw_host_runtime`.
- The second slowest bucket, `composition-core`, contains only
  `ironclaw_reborn_composition`.
- The third slowest bucket, `reborn-core`, contains multiple packages, but it is
  not the wall-clock long pole while `host-runtime` and `composition-core`
  remain slower.
- `webui-ingress` contains multiple packages, but its tests mostly run in
  seconds after compile completes, so splitting it would mostly trade one
  shared dependency compile for multiple per-job setup/cache/compile costs.

Decision:

Reject for the current long poles. The buckets that determine wall clock are
already single-crate buckets, so a dependency-shape split cannot reduce their
compile graph without changing feature sets or target selection. Splitting them
by test binary would add jobs and duplicate the same crate compile, which fails
the job-count acceptance criterion unless it also produces a very large wall
clock win. The baseline logs do not support that.

## H3: Split Host Runtime WASM Contract Tests

Change considered:

- Move the runtime-heavy host WASM contract test binary out of the normal
  `host-runtime` bucket.

Evidence:

- Baseline `host-runtime` bucket duration: `473s`.
- First test output in that bucket appeared at `20:26:28`, roughly `321s` after
  the bucket started at `20:21:07`.
- `tests/github_wasm_runtime_contract.rs` then ran for `142.73s`.
- `tests/host_runtime_services_contract.rs` ran for only `0.37s`; it is not the
  runtime-heavy part of this bucket.

Decision:

Reject as a compile-time optimization. A dedicated host WASM contract job would
likely reduce the original `host-runtime` test-runtime tail, but it would also
compile `ironclaw_host_runtime` and its WASM/product dependency graph again in a
second job. That increases total jobs and duplicate compilation. It only makes
sense if the goal changes from reducing compile time to reducing a single
runtime-heavy test binary's queueing effect.

## H4: Reduce Slow-Crate Feature Flags

Change considered:

- Remove heavy feature flags from slow Reborn crate buckets when the same
  coverage exists elsewhere.
- The main candidate was removing `slack-v2-host-beta` from
  `ironclaw_reborn_composition`, because that feature pulls in
  `webui-v2-beta`, `ironclaw_slack_v2_adapter`,
  `ironclaw_wasm_product_adapters`, and `ironclaw_product_workflow/storage`.

Evidence:

- `.github/workflows/test.yml` already runs
  `cargo test -p ironclaw_reborn_composition --no-default-features --features libsql,postgres --tests`.
- That legacy substrate job does not enable `webui-v2-beta` or
  `slack-v2-host-beta`.
- `slack-v2-host-beta` gates Reborn composition code in `webui_serve`,
  `trigger_poller`, `slack_delivery`, runtime construction, and Slack serve
  tests. Removing it from the Reborn crate bucket would stop exercising the
  host-mounted Slack/WebUI integration surface in PR CI.
- Live canary workflows cover Slack behavior, but live canaries are drift checks
  and not a replacement for hermetic PR-gate compile/test coverage.

Decision:

Do not benchmark this as a candidate to keep. It is the only remaining lever
that directly reduces the compile graph, but the currently obvious reduction
would weaken PR coverage. A safe version of this hypothesis requires either:

- adding smaller hermetic Slack/WebUI contract tests under a cheaper feature
  profile, then removing the broad beta feature from the crate bucket, or
- moving the broad beta composition compile to a single dedicated job and
  removing duplicate feature-heavy compiles elsewhere.

## H5: Remove OVH sccache From Crate Buckets

Change under test:

- Remove `./.github/actions/setup-sccache-dist` from the `crate-tests` job only.
- Keep `Swatinem/rust-cache` and every `cargo test` command unchanged.
- Keep OVH sccache in root tests, group tests, QA fixtures, and coverage lanes
  so this benchmark isolates the crate bucket path.

Why this is safe to test:

- This is a benchmark branch, not a production removal.
- It does not reduce test coverage.
- It directly tests whether the Redis/SSH/sccache setup and remote cache reads
  are helping the crate buckets enough to justify the OVH dependency.

Benchmark result:

- Branch/PR: [`codex/ci-compile-benchmarks`, PR #5648](https://github.com/nearai/ironclaw/pull/5648)
- Workflow run: [`28719569749`](https://github.com/nearai/ironclaw/actions/runs/28719569749)
- Status: success
- Wall clock: `9m45s` (`2026-07-04T21:02:40Z` to `2026-07-04T21:12:25Z`)
- Crate bucket job count: `12`
- Slowest bucket: `host-runtime` at `535s`
- Decision: reject. Removing OVH sccache from crate buckets did not meet either
  acceptance criterion and made the critical path worse.

Comparison against baseline:

| Metric | Baseline | H5 | Delta |
| --- | ---: | ---: | ---: |
| Reborn workflow wall clock | `8m35s` | `9m45s` | `+70s` |
| Crate bucket job count | `12` | `12` | `0` |
| `host-runtime` | `473s` | `535s` | `+62s` |
| `composition-core` | `419s` | `441s` | `+22s` |
| `reborn-core` | `367s` | `465s` | `+98s` |
| `webui-ingress` | `334s` | `438s` | `+104s` |
| `wasm-sandbox` | `307s` | `408s` | `+101s` |

Interpretation:

OVH sccache is not producing a 30-50% win overall, but fully removing it from
crate buckets made this benchmark worse. The current evidence says the OVH path
still helps enough on repeated Reborn crate builds that removal should not be
merged as a speed optimization.

## H6: Disable Incremental Compilation for All Crate Buckets

Change under test:

- Set `CARGO_INCREMENTAL=0` at the `crate-tests` job level.
- Remove the per-package special case that only disabled incremental
  compilation for `ironclaw_reborn_composition`.

Why this is safe to test:

- It does not change which crates, features, or tests run.
- GitHub hosted CI runs one-shot builds, where incremental bookkeeping and
  larger target directories can be overhead rather than a win.
- The workflow already disabled incremental for the heaviest composition crate
  to keep disk usage bounded; this tests whether applying the same policy to
  every crate bucket improves compile time.

Benchmark result:

- Branch/PR: [`codex/ci-compile-benchmarks`, PR #5648](https://github.com/nearai/ironclaw/pull/5648)
- Workflow run: [`28720040658`](https://github.com/nearai/ironclaw/actions/runs/28720040658)
- Status: success
- Wall clock: `8m49s` (`2026-07-04T21:22:00Z` to `2026-07-04T21:30:49Z`)
- Crate bucket job count: `12`
- Slowest bucket: `host-runtime` at `475s`
- Decision: reject. This did not meet either acceptance criterion.

Comparison against baseline:

| Metric | Baseline | H6 | Delta |
| --- | ---: | ---: | ---: |
| Reborn workflow wall clock | `8m35s` | `8m49s` | `+14s` |
| Crate bucket job count | `12` | `12` | `0` |
| `host-runtime` | `473s` | `475s` | `+2s` |
| `composition-core` | `419s` | `379s` | `-40s` |
| `reborn-core` | `367s` | `426s` | `+59s` |
| `webui-ingress` | `334s` | `344s` | `+10s` |
| `wasm-sandbox` | `307s` | `293s` | `-14s` |

Interpretation:

Disabling incremental globally helped `composition-core` and `wasm-sandbox`, but
it made `reborn-core`, `webui-ingress`, and the overall workflow slower. Keeping
the narrower existing composition-only incremental disable remains the better
shape.

## H7: Remove Duplicate Instrumented Group Coverage Lane

Change under test:

- Remove the `groups` member from the `reborn-integration-coverage` matrix.
- Keep the existing `Reborn group tests` job, which runs the 7 `reborn_group_*`
  suites as the PR pass/fail gate.
- Keep coverage lanes `0`, `1`, `2`, and `3`, which are the only execution of
  the 27 flat `reborn_integration_*` suites.

Why this is safe to test:

- The workflow comments state that the `groups` coverage lane is an additional,
  instrumented run of suites already covered by `reborn-group-tests`.
- The experiment reduces duplicate instrumented coverage work, not the normal
  group test pass/fail coverage.
- The expected win is one fewer Reborn job at roughly the same wall clock,
  because the baseline `groups` coverage lane was not the overall long pole.

Benchmark result:

- Branch/PR: [`codex/ci-compile-benchmarks`, PR #5648](https://github.com/nearai/ironclaw/pull/5648)
- Workflow run: [`28720349622`](https://github.com/nearai/ironclaw/actions/runs/28720349622)
- Status: success
- Wall clock: `9m39s` (`2026-07-04T21:34:23Z` to `2026-07-04T21:44:02Z`)
- Total Reborn jobs: `27` versus baseline `28`
- Reborn integration coverage jobs: `4` versus baseline `5`
- Crate bucket job count: `12`
- Slowest bucket: `host-runtime` at `519s`
- Decision: reject. The job-count reduction worked, but it did not preserve the
  current wall-clock baseline.

Comparison against baseline:

| Metric | Baseline | H7 | Delta |
| --- | ---: | ---: | ---: |
| Reborn workflow wall clock | `8m35s` | `9m39s` | `+64s` |
| Total Reborn jobs | `28` | `27` | `-1` |
| Reborn integration coverage jobs | `5` | `4` | `-1` |
| Crate bucket job count | `12` | `12` | `0` |
| `host-runtime` | `473s` | `519s` | `+46s` |
| `composition-core` | `419s` | `418s` | `-1s` |
| `reborn-core` | `367s` | `379s` | `+12s` |
| `webui-ingress` | `334s` | `312s` | `-22s` |
| `wasm-sandbox` | `307s` | `363s` | `+56s` |

Interpretation:

Removing the duplicate group coverage lane is the first experiment that reduced
job count without reducing the normal group pass/fail test job. However, this
run missed the acceptance criterion because `host-runtime` and `wasm-sandbox`
regressed enough that total wall clock increased by `64s`. Keep this idea as a
possible cleanup only if repeated runs show the wall-clock regression was
variance; do not merge it from this benchmark alone.

## H8: Merge the Two Smallest Crate Buckets

Change under test:

- Merge `auth-security` and `memory-skills` into one `auth-memory` crate bucket.
- Keep every package, feature flag, and cargo test invocation unchanged.
- Leave the known long-pole buckets (`host-runtime`, `composition-core`,
  `reborn-core`, `webui-ingress`, `wasm-sandbox`) untouched.

Why this is safe to test:

- This changes scheduling only; it does not drop coverage.
- Baseline durations were `auth-security = 165s` and `memory-skills = 150s`,
  both far below the `host-runtime = 473s` long pole.
- The expected win is one fewer crate bucket job, and therefore one fewer total
  Reborn job, without changing wall clock if the merged bucket stays below the
  long-pole duration.

Benchmark result:

- Branch/PR: [`codex/ci-compile-benchmarks`, PR #5648](https://github.com/nearai/ironclaw/pull/5648)
- Workflow run: [`28720713733`](https://github.com/nearai/ironclaw/actions/runs/28720713733)
- Status: success
- Wall clock: `8m04s` (`2026-07-04T21:49:32Z` to `2026-07-04T21:57:36Z`)
- Total Reborn jobs: `27` versus baseline `28`
- Crate bucket job count: `11` versus baseline `12`
- Slowest bucket: `host-runtime` at `443s`
- Decision: revert. This did meet the original job-count criterion on this
  manual run, but it does not address the compile-time bottleneck. The same
  single-crate compile-heavy buckets still decide the workflow critical path.

Comparison against baseline:

| Metric | Baseline | H8 | Delta |
| --- | ---: | ---: | ---: |
| Reborn workflow wall clock | `8m35s` | `8m04s` | `-31s` |
| Total Reborn jobs | `28` | `27` | `-1` |
| Crate bucket job count | `12` | `11` | `-1` |
| `host-runtime` | `473s` | `443s` | `-30s` |
| `composition-core` | `419s` | `397s` | `-22s` |
| `reborn-core` | `367s` | `375s` | `+8s` |
| `webui-ingress` | `334s` | `302s` | `-32s` |
| `wasm-sandbox` | `307s` | `283s` | `-24s` |

Interpretation:

Merging two small buckets is a reasonable queue-pressure cleanup, but it is not
a compile-time optimization. The result still spends the critical path in
`host-runtime`, `composition-core`, and `reborn-core`, all of which are driven
by compiling large dependency graphs. Per the updated direction, this benchmark
branch restored the original buckets and should focus next on reducing the
compile graph or avoiding repeated compiles for those long poles.

## H9: Enable Distributed Compilation for All Crate Buckets

Change under test:

- Remove the per-package `sccache_dist_enabled=false` override in
  `crate-tests`.
- Keep the existing OVH Redis cache and sccache action.
- Keep every crate bucket, package, feature flag, and `cargo test` command
  unchanged.

Why this is safe to test:

- This changes compile execution only; it does not drop or move tests.
- The current workflow already configures the sccache action and passes the
  scheduler URL/token for buckets that are not in the opt-out list.
- Prior benchmarks show the long poles are compile-bound, and removing OVH
  cache entirely regressed. This isolates the remaining question: whether the
  distributed scheduler helps or hurts the compile-heavy buckets that currently
  opt out.

Benchmark result:

- Branch/PR: [`codex/ci-compile-benchmarks`, PR #5648](https://github.com/nearai/ironclaw/pull/5648)
- Workflow run: [`28721039930`](https://github.com/nearai/ironclaw/actions/runs/28721039930)
- Status: success
- Wall clock: `9m11s` (`2026-07-04T22:02:40Z` to `2026-07-04T22:11:51Z`)
- Total Reborn jobs: `28`
- Crate bucket job count: `12`
- Slowest bucket: `host-runtime` at `512s`
- Decision: reject. Enabling distributed compilation for the buckets that were
  intentionally cache-only made the critical path slower.

Comparison against baseline:

| Metric | Baseline | H9 | Delta |
| --- | ---: | ---: | ---: |
| Reborn workflow wall clock | `8m35s` | `9m11s` | `+36s` |
| Total Reborn jobs | `28` | `28` | `0` |
| Crate bucket job count | `12` | `12` | `0` |
| `host-runtime` | `473s` | `512s` | `+39s` |
| `composition-core` | `419s` | `436s` | `+17s` |
| `reborn-core` | `367s` | `378s` | `+11s` |
| `webui-ingress` | `334s` | `298s` | `-36s` |
| `wasm-sandbox` | `307s` | `253s` | `-54s` |

Interpretation:

The distributed scheduler helped some non-critical buckets, but it regressed
all three long-pole buckets that determine wall clock. The existing opt-out
list is justified by this benchmark; the remaining compile-time work needs to
reduce the dependency/feature graph or avoid repeated compiles, not simply send
the same graph through sccache-dist.

## H10: Remove Duplicate Reborn Restart Feature From Crate Bucket

Change under test:

- Remove `libsql-restart-tests` from the broad `ironclaw_reborn` feature set in
  `scripts/ci/package-feature-flags.sh`.
- Keep `root-llm-provider`, `libsql-secrets`, and `webui-user-store` enabled in
  the `reborn-core` crate bucket.
- Keep the dedicated code-style PR job that runs the exact restart test:
  `cargo test -p ironclaw_reborn --features libsql-restart-tests --test loop_driver_host turn_runner_worker_completes_after_libsql_turn_and_thread_services_reopen`.

Why this is safe to test:

- The restart integration is already covered by a dedicated PR job outside the
  Reborn crate bucket.
- This does not remove the `ironclaw_reborn` crate from the Reborn bucket and
  does not change other Reborn test features.
- Dependency-node evidence says this will not shrink the compile graph
  (`478` package nodes before and after), so the only plausible win is removing
  duplicate restart-test runtime from `reborn-core`.

Benchmark result:

- Branch/PR: [`codex/ci-compile-benchmarks`, PR #5648](https://github.com/nearai/ironclaw/pull/5648)
- Workflow run: [`28721369125`](https://github.com/nearai/ironclaw/actions/runs/28721369125), canceled after the compile-heavy buckets were still the remaining long poles.
- Decision: reverted. This experiment did not target the measured bottleneck:
  the `ironclaw_reborn` package-node count stayed at `478` before and after
  removing `libsql-restart-tests`, so it could not materially reduce compile
  time.

## H11: Move libSQL Coverage Out Of Broad Long-Pole Buckets

Change under test:

- Remove the broad `libsql`-pulling feature sets from the slow crate buckets
  only after adding narrower replacement coverage for the libSQL-specific
  integration paths.
- Candidate crate-bucket feature reductions:
  - `ironclaw_host_runtime`: `--features test-support,libsql` ->
    `--features test-support`
  - `ironclaw_reborn`: `--features root-llm-provider,libsql-secrets,libsql-restart-tests,webui-user-store`
    -> `--features root-llm-provider`
  - `ironclaw_reborn_composition`: unchanged for this benchmark because
    `webui-v2-beta` itself enables `libsql`; removing that surface safely needs
    a larger coverage redesign.

Evidence:

Dependency-tree counts were collected with the same package feature script that
the Reborn crate bucket workflow uses, plus explicit candidate variants:

```text
host-runtime: no features                                      deps=648 features=2243
host-runtime: test-support                                     deps=648 features=2243
host-runtime: libsql                                           deps=735 features=2515
host-runtime: test-support,libsql CI                           deps=735 features=2515
composition: no features                                       deps=707 features=2345
composition: test-support only                                 deps=707 features=2345
composition: test-support,libsql                               deps=792 features=2617
composition: test-support,webui-v2-beta,slack-v2-host-beta     deps=804 features=2659
composition: full CI                                           deps=804 features=2659
reborn: no features                                            deps=639 features=2180
reborn: root-llm-provider only                                 deps=640 features=2181
reborn: root,libsql-secrets                                    deps=729 features=2465
reborn: root,libsql-secrets,webui-user-store                   deps=729 features=2465
reborn: full CI                                                deps=729 features=2465
webui-ingress: no features                                     deps=792 features=2669
webui-ingress: dev-in-memory-session CI                        deps=792 features=2669
```

Interpretation:

- `libsql` is the meaningful compile-graph expander in the slowest buckets:
  `+87` dependency-tree entries for `host-runtime`, `+85` for composition's
  `test-support,libsql` variant, and `+89` for `reborn` through
  `libsql-secrets`.
- `test-support`, `webui-user-store`, `libsql-restart-tests`, and
  `dev-in-memory-session` do not materially change the graph in these
  measurements.
- `webui-v2-beta` / `slack-v2-host-beta` still cannot simply be removed from
  composition: they are part of the host-mounted WebUI/Slack coverage surface.
- Existing CI already has explicit libSQL coverage in
  `scripts/ci/run-reborn-group-tests.sh` (`cargo test --test reborn_group_* --features libsql`)
  and coverage lanes (`cargo llvm-cov --workspace --features libsql test ...`),
  but those do not prove every crate-bucket libSQL test can be removed safely.

Decision:

- Do not run or keep a naked "remove libSQL from the slow buckets" change; that
  would optimize compile time by weakening persistence coverage.
- This benchmark keeps the Reborn job count flat by folding the exact
  feature-gated libSQL tests into the existing Reborn group job instead of
  adding a new matrix job.

Focused replacement coverage now run by
`scripts/ci/run-reborn-group-tests.sh`:

- `ironclaw_host_runtime::first_party_builtin_tools::builtin_coding_blocks_sensitive_resolved_libsql_paths`
- `ironclaw_host_runtime::host_runtime_services_contract` libSQL selection and
  persistence guardrail tests:
  - `production_root_filesystem_selection_accepts_libsql_root_filesystem`
  - `production_turn_state_selection_accepts_filesystem_turn_state_store`
  - `production_turn_coordinator_uses_configured_store_and_notifier`
  - `production_turn_coordinator_requires_explicit_run_profile_resolver`
  - `host_runtime_services_preserves_combined_store_after_root_filesystem_selection`
- `ironclaw_host_runtime::reborn_durable_restart_integration::approval_resume_survives_durable_libsql_reopen_and_consumes_lease_once`
- `ironclaw_reborn::secrets` with `--features libsql-secrets`
- `ironclaw_reborn::loop_driver_host::turn_runner_worker_completes_after_libsql_turn_and_thread_services_reopen`
  with `--features libsql-restart-tests`

Benchmark result:

- Branch/PR: [`codex/ci-compile-benchmarks`, PR #5648](https://github.com/nearai/ironclaw/pull/5648)
- Workflow run: [`28721862670`](https://github.com/nearai/ironclaw/actions/runs/28721862670)
- Status: success
- Wall clock: `9m50s` (`2026-07-04T22:36:17Z` to `2026-07-04T22:46:07Z`)
- Total Reborn jobs: `28`
- Crate bucket job count: `12`
- Decision: reject and revert. The broad feature reduction did not reduce the
  actual long-pole bucket durations, and the focused libSQL work increased the
  group job from `5m24s` baseline to `6m59s`.

Comparison against baseline:

| Metric | Baseline | H11 | Delta |
| --- | ---: | ---: | ---: |
| Reborn workflow wall clock | `8m35s` | `9m50s` | `+1m15s` |
| Total Reborn jobs | `28` | `28` | `0` |
| Crate bucket job count | `12` | `12` | `0` |
| `host-runtime` | `473s` | `547s` | `+74s` |
| `composition-core` | `419s` | `438s` | `+19s` |
| `reborn-core` | `367s` | `392s` | `+25s` |
| `webui-ingress` | `334s` | `303s` | `-31s` |
| `wasm-sandbox` | `307s` | `360s` | `+53s` |
| Reborn group tests | `324s` | `419s` | `+95s` |

Interpretation:

This disproves the simple libSQL-split version of H11. Even though the
dependency graph is smaller on paper for `host-runtime` and `reborn-core`, CI
did not translate that into shorter buckets. The moved focused tests also made
the existing group job heavier, so this does not meet either acceptance
criterion.

## H12: Move Reborn CLI Into WebUI Ingress Bucket

Change under test:

- Move `ironclaw_reborn_cli` from the `reborn-core` crate bucket to the
  `webui-ingress` bucket in `scripts/ci/reborn-crate-test-buckets.sh`.
- Keep total crate bucket count unchanged at `12`.
- Keep the same package test coverage; this changes only bucket placement.

Why this is safe to test:

- `ironclaw_reborn_cli` is still tested by the Reborn crate buckets with the
  same feature flags: `--features webui-v2-beta,slack-v2-host-beta`.
- The CLI package has a WebUI-shaped dependency graph because those features
  pull in `ironclaw_reborn_composition/webui-v2-beta`,
  `ironclaw_reborn_webui_ingress`, and Slack host-beta wiring.
- `webui-ingress` already installs Node and builds WebUI-shaped dependencies,
  while `reborn-core` otherwise does not need to be the bucket that pays that
  setup/compile cost.

Dependency-tree evidence:

```text
ironclaw_reborn_cli --features webui-v2-beta,slack-v2-host-beta
  deps=799, feature tree entries=2666

ironclaw_reborn_webui_ingress --features dev-in-memory-session
  deps=792, feature tree entries=2669

ironclaw_webui_v2 --features webui-v2-beta
  deps=413, feature tree entries=1408
```

Benchmark result:

- Branch/PR: [`codex/ci-compile-benchmarks`, PR #5648](https://github.com/nearai/ironclaw/pull/5648)
- Workflow run: [`28722205751`](https://github.com/nearai/ironclaw/actions/runs/28722205751)
- Status: success.
- Wall clock: `8m37s` (`2026-07-04T22:50:54Z` to `2026-07-04T22:59:31Z`).
- Total Reborn jobs: `28` unchanged.
- Reborn crate bucket jobs: `12` unchanged.

| Metric | Baseline | H12 | Delta |
| --- | ---: | ---: | ---: |
| Reborn workflow wall clock | `8m35s` | `8m37s` | `+2s` |
| Total Reborn jobs | `28` | `28` | `0` |
| Reborn crate bucket jobs | `12` | `12` | `0` |
| `reborn-core` | `367s` | `238s` | `-129s` |
| `webui-ingress` | `334s` | `377s` | `+43s` |
| `composition-core` | `419s` | `445s` | `+26s` |
| `host-runtime` | `473s` | `480s` | `+7s` |
| `wasm-sandbox` | `307s` | `296s` | `-11s` |
| Reborn group tests | `324s` | `406s` | `+82s` |

Decision:

- Reject and revert. The change validated that `ironclaw_reborn_cli` is a large
  part of the `reborn-core` bucket, but moving it only shifts compile cost into
  `webui-ingress`.
- The workflow still has the same job count and the same compile-bound long
  poles: `host-runtime`, `composition-core`, `webui-ingress`, root tests, and
  group tests.

## H13: Remove Duplicate Uninstrumented Group Job

Change under test:

- Remove the standalone `reborn-group-tests` job from
  `.github/workflows/reborn-tests.yml`.
- Keep the `reborn-integration-coverage` matrix lane `groups`, which already
  runs the same `reborn_group_*` test binaries through `cargo llvm-cov`.
- Keep the aggregate `Tests (Reborn)` gate on `reborn-integration-coverage`, so
  group-suite failures still fail PR CI.

Why this is safe to test:

- `cargo llvm-cov ... test` keeps normal test pass/fail semantics while adding
  instrumentation.
- The workflow already treats `reborn-integration-coverage` as a real test gate
  for the flat `reborn_integration_*` suites.
- The baseline `Reborn group tests` job took `316s`, while the existing
  instrumented `groups` lane took `325s`, so keeping both jobs appears to
  duplicate compile/test work without improving the critical path.

Benchmark result:

- Branch/PR: [`codex/ci-compile-benchmarks`, PR #5648](https://github.com/nearai/ironclaw/pull/5648)
- Workflow run: [`28722534492`](https://github.com/nearai/ironclaw/actions/runs/28722534492)
- Status: success.
- Wall clock: `8m21s` (`2026-07-04T23:05:00Z` to `2026-07-04T23:13:21Z`).
- Total Reborn jobs: `27`.
- Reborn crate bucket jobs: `12` unchanged.

| Metric | Baseline | H13 | Delta |
| --- | ---: | ---: | ---: |
| Reborn workflow wall clock | `8m35s` | `8m21s` | `-14s` |
| Total Reborn jobs | `28` | `27` | `-1` |
| Reborn crate bucket jobs | `12` | `12` | `0` |
| `reborn-group-tests` job | `316s` | removed | `-1 job` |
| `Reborn integration coverage (groups)` | `325s` | `391s` | `+66s` |
| `host-runtime` | `473s` | `438s` | `-35s` |
| `composition-core` | `419s` | `457s` | `+38s` |
| `reborn-core` | `367s` | `462s` | `+95s` |
| Slowest job | `host-runtime` `473s` | `reborn-core` `462s` | `-11s` |

Decision:

- Retain. This satisfies the job-count acceptance criterion: one fewer total
  Reborn job with no wall-clock regression.
- The result does not solve the core compile-time issue; it removes duplicated
  group-suite execution while keeping group failures gated through the existing
  instrumented coverage `groups` lane.
- The remaining compile-time bottlenecks are still crate/root build graphs,
  especially `reborn-core`, `composition-core`, and `host-runtime`.

## H14: Deterministic Superset Rust Cache Seeding

Advice under test:

- The shared crate-bucket Rust cache may be seeded by whichever matrix bucket
  first creates a new cache key. If that producer is a small bucket, long-pole
  buckets restore a weak target-dir warm start.
- Use a deterministic broad producer instead:
  - `reborn-core` for crate buckets, because the bucket includes
    `ironclaw_reborn_cli`.
  - `groups` for instrumented integration coverage lanes.

Red-team notes before benchmarking:

- Current PR/manual branch runs do not save Rust caches at all. Existing
  `save-if` allows saves only on `push` to `main` and `merge_group`, so H14
  cannot be measured in one normal PR run.
- GitHub cache entries are immutable. Existing full-match keys report
  `Cache up-to-date`, so changing only `save-if` would not replace the current
  cache contents until a new key is minted.
- Baseline logs do show every crate bucket restoring the same full-match
  `reborn-tests-crates` cache. In the sampled baseline run, `auth-security`,
  `host-runtime`, and the coverage `groups` lane all restored full-match keys
  and ended with `Cache up-to-date`; that weakens the claim that a small bucket
  was actively overwriting the cache in that run.
- Dependency overlap is plausible for the bucket-level producer, but not for
  `ironclaw_reborn` alone:
  - `ironclaw_reborn` closure: `44` workspace crates.
  - `ironclaw_reborn_cli` closure: `57` workspace crates.
  - `ironclaw_reborn_composition` closure: `56` workspace crates.
  - `ironclaw_host_runtime` closure: `39` workspace crates.
  - `ironclaw_reborn` alone does not cover several composition crates, but
    `ironclaw_reborn_cli` covers both `ironclaw_host_runtime` and
    `ironclaw_reborn_composition` workspace closures in this check.

Benchmark method:

- Use fresh benchmark keys:
  - `reborn-tests-crates-h14-reborn-core`
  - `reborn-integration-cov-h14-groups`
- Allow `workflow_dispatch` cache saves only on this benchmark branch and only
  from the intended producers:
  - `matrix.bucket.name == 'reborn-core'`
  - `matrix.lane == 'groups'`
- Run the workflow twice:
  - Seed run: expected to be cold for the fresh H14 keys; only the intended
    producers should save.
  - Measurement run: expected to restore the deterministic H14 keys and show
    whether long-pole compile times improve.

Benchmark result:

- Branch/PR: [`codex/ci-compile-benchmarks`, PR #5648](https://github.com/nearai/ironclaw/pull/5648)
- Seed workflow run: [`28730811648`](https://github.com/nearai/ironclaw/actions/runs/28730811648)
- Measurement workflow run: [`28731057678`](https://github.com/nearai/ironclaw/actions/runs/28731057678)
- Measurement status: success.
- Measurement wall clock: `7m49s`
  (`2026-07-05T05:45:20Z` to `2026-07-05T05:53:09Z`).
- Total Reborn jobs: `27`, unchanged from H13.
- Reborn crate bucket jobs: `12`, unchanged.

Cache evidence:

- Seed run: `reborn-core` had `save-if: true` for
  `reborn-tests-crates-h14-reborn-core` and reached `... Saving cache ...`;
  sampled non-producer `auth-security` had `save-if: false`.
- Seed run: coverage `groups` had `save-if: true` for
  `reborn-integration-cov-h14-groups` and reached `... Saving cache ...`;
  sampled non-producer coverage lane `1` had `save-if: false`.
- Measurement run: `host-runtime` and `composition-core` restored a full-match
  `reborn-tests-crates-h14-reborn-core` cache, size about `1138 MB`.
- Measurement run: coverage `groups` restored a full-match
  `reborn-integration-cov-h14-groups` cache, size about `844 MB`.

| Metric | Baseline | H13 | H14 measurement | Delta vs H13 |
| --- | ---: | ---: | ---: | ---: |
| Reborn workflow wall clock | `8m35s` | `8m21s` | `7m49s` | `-32s` |
| Total Reborn jobs | `28` | `27` | `27` | `0` |
| Reborn crate bucket jobs | `12` | `12` | `12` | `0` |
| `reborn-core` | `367s` | `462s` | `160s` | `-302s` |
| `composition-core` | `419s` | `457s` | `308s` | `-149s` |
| `host-runtime` | `473s` | `438s` | `373s` | `-65s` |
| `webui-ingress` | `334s` | `304s` | `243s` | `-61s` |
| `wasm-sandbox` | `307s` | `300s` | `255s` | `-45s` |
| `Reborn integration coverage (groups)` | `325s` | `391s` | `265s` | `-126s` |
| Slowest job | `host-runtime` `473s` | `reborn-core` `462s` | root `0` `450s` | root-bound |

Seed-run note:

- The seed run itself was slow: `10m53s`. That is expected for fresh immutable
  cache keys and is not the H14 steady-state measurement.

Decision:

- Retain the deterministic producer save policy. It does not deliver the
  advice's optimistic `~6.3m` wall-clock estimate, but it materially improves
  the compile-heavy crate buckets and coverage lanes without changing coverage
  or adding jobs.
- After H14, root tests are the measured long pole. This strengthens the advice
  that root partitioning/nextest only becomes interesting after cache quality
  improves.

Final-state validation caveat:

- After reverting the rejected H18 workflow change and adding the remaining
  report sections, the final branch run
  [`28732112974`](https://github.com/nearai/ironclaw/actions/runs/28732112974)
  passed with the retained H13/H14 workflow shape, but took `12m07s`
  (`2026-07-05T06:33:28Z` to `2026-07-05T06:45:35Z`).
- A second validation run,
  [`28732426390`](https://github.com/nearai/ironclaw/actions/runs/28732426390),
  also passed with the same retained workflow shape and `27` Reborn jobs, but
  took `12m44s` (`2026-07-05T06:47:48Z` to `2026-07-05T07:00:32Z`).
- A third validation run,
  [`28732787177`](https://github.com/nearai/ironclaw/actions/runs/28732787177),
  also passed with the same retained workflow shape and `27` Reborn jobs, but
  took `12m22s` (`2026-07-05T07:05:13Z` to `2026-07-05T07:17:35Z`).
- These validation runs still had `27` total Reborn jobs, but their long poles
  were much slower than the H14 measurement. The first validation run's slowest
  jobs were:
  - `Reborn root tests (3)`: `676s`.
  - `Reborn root tests (2)`: `650s`.
  - `adapters-misc`: `590s`.
  - `host-runtime`: `540s`.
  - coverage `groups`: `522s`.
- The second validation run's slowest jobs were:
  - `adapters-misc`: `706s`.
  - `Reborn root tests (0)`: `637s`.
  - `Reborn root tests (1)`: `637s`.
  - `composition-core`: `580s`.
  - coverage `2`: `568s`.
  - `host-runtime`: `545s`.
- The third validation run's slowest jobs were:
  - `adapters-misc`: `676s`.
  - `Reborn root tests (1)`: `627s`.
  - `Reborn root tests (3)`: `602s`.
  - `Reborn root tests (2)`: `593s`.
  - `host-runtime`: `587s`.
  - `composition-core`: `564s`.
- Log evidence points to cache quality/latency variance, not test failures:
  - root partition `3` restored `reborn-tests-root` from GitHub cache and got
    `No cache found`, then relied on OVH Redis sccache.
  - root partition `3` reported `1298` sccache hits, `1` miss, and average
    cache read hit `0.600s`; the aggregate cache-read-hit time was about
    `779s`.
  - `host-runtime` and `reborn-core` reported `100%` sccache hit rates, but
    still took `540s` and `448s` respectively.
- This downgrades H14 from "stable wall-clock solution" to "reasonable
  deterministic cache seeding cleanup". It should not be sold as solving the
  compile-time problem by itself.

## H15: Cargo Hakari Workspace Hack

Advice under review:

- Use `cargo-hakari` to generate a workspace-hack crate that unifies dependency
  feature sets across workspace packages, improving cache reuse for dependencies
  compiled by different buckets with different package feature flags.

Red-team notes:

- There is no existing `hakari`, `workspace-hack`, or `workspace_hack` setup in
  the repo before the benchmark.
- The root manifest does use `[workspace.dependencies]`, but adding hakari would
  still require a new workspace member, manifest edits across crates, and
  `Cargo.lock` churn.
- This is a qualitatively higher-risk benchmark than H14 because it changes the
  Rust dependency graph rather than only cache key/save policy.

Change tested:

- Installed `cargo-hakari 0.9.38` locally.
- Added `.config/hakari.toml` for `x86_64-unknown-linux-gnu`.
- Generated `crates/ironclaw_workspace_hack`.
- Ran `cargo hakari manage-deps --yes`, which added a normal
  `ironclaw_workspace_hack` dependency across workspace packages.
- Local validation passed before pushing:
  - `cargo hakari verify`
  - `cargo metadata --no-deps --format-version 1`, which reported `72`
    packages
  - `git diff --check`

Benchmark result:

- Commit tested:
  [`e9efeceee`](https://github.com/nearai/ironclaw/commit/e9efeceeed91960b363e25f0124a8527e3d1f22d)
- Workflow run:
  [`28733406095`](https://github.com/nearai/ironclaw/actions/runs/28733406095)
- Trigger: `workflow_dispatch` on the exact H15 commit, because the automatic
  PR `pull_request` workflow did not appear for that commit.
- Status: failure.
- Wall clock to failed aggregate gate: `10m33s`
  (`2026-07-05T07:30:27Z` to `2026-07-05T07:41:00Z`).
- Failed job: `Test Reborn crate bucket (adapters-misc)`.

Failure evidence:

- The failing test was
  `cargo test -p ironclaw_architecture --test reborn_dependency_boundaries`.
- `reborn_cli_binary_crate_stays_separate_from_v1_root` failed because
  `ironclaw_reborn_cli` gained a normal dependency on
  `ironclaw_workspace_hack`.
- `reborn_crate_dependency_boundaries_hold` failed because crates such as
  `ironclaw_host_api` gained a normal dependency on
  `ironclaw_workspace_hack`.
- `wasm_product_adapter_crate_keeps_minimal_host_glue_dependencies` and
  `wasm_sandbox_core_is_standalone_v1_parity_kernel` failed for the same
  reason: the generated workspace-hack dependency violated explicit dependency
  boundary contracts.

Timing evidence from the failed run:

| Metric | H14 retained validation best of latest 3 | H15 run | Delta |
| --- | ---: | ---: | ---: |
| Workflow wall clock | `12m07s` | failed at `10m33s` | not comparable; failed |
| Original baseline wall clock | `8m35s` | failed at `10m33s` | `+1m58s` and failed |
| `Reborn root tests (1)` | `10m27s` | `10m21s` | `-6s` |
| `Reborn root tests (0)` | `10m37s` | `9m48s` | `-49s` |
| `host-runtime` | `9m00s` | `8m24s` | `-36s` |
| `composition-core` | `9m24s` | `8m21s` | `-63s` |
| `reborn-core` | `7m28s` | `7m01s` | `-27s` |
| `webui-ingress` | `4m37s` | `6m49s` | `+2m12s` |

Decision:

- Reject and revert from the PR branch.
- Even ignoring the architecture-boundary failure, the measured run was not a
  compelling compile-time improvement. The long poles stayed around
  `8-10m`, several crate buckets were slower, and the dependency-boundary
  failures are exactly the kind of maintainability cost this benchmark needed
  to avoid.

## H16: Nightly Dependency-Warmed GHCR Image

Advice under review:

- Build a nightly container image that pre-warms the Rust dependency/target
  layer, push it to GHCR, and run heavy Reborn jobs with `container:` so jobs
  start from a deterministic warm build image instead of relying on the 10 GB
  GitHub Actions cache.

Red-team notes:

- There is no existing Reborn dependency-image workflow or Dockerfile target in
  this branch. Existing Docker workflows build product/release images and use
  GitHub Actions cache, but they do not publish a Rust target-dir warm-start
  image for PR jobs.
- A meaningful benchmark needs two phases:
  - seed/build a GHCR image from `main` or `Cargo.lock` changes, then
  - run PR jobs inside that image and compare against the same Reborn workflow
    baseline.
- Adding this inline to PR #5648 would mix infrastructure work, package write
  permissions, image retention policy, container hardening, and benchmark
  measurement into one CI optimization PR.
- A target-dir baked into an image can be brittle across absolute paths,
  toolchain hashes, feature sets, and `RUSTFLAGS`; a bad image can turn into a
  large pull plus a full rebuild.

Benchmark status:

- Not started in this PR. Treat as a separate H16 infrastructure benchmark with
  explicit rollback criteria: image pull time, cache-hit proof, image size,
  GHCR permissions, and before/after wall clock for the same heavy Reborn jobs.

## H17: Per-Bucket Change Detection Skipping

Advice under review:

- Extend scope detection from all-or-nothing Reborn CI to bucket-level
  selection based on changed files, owning crates, and reverse dependencies.

Red-team notes:

- The current `scripts/ci/classify-test-scope.sh` emits broad booleans:
  `docs_only`, `has_core_code`, `has_legacy_tests`, and `has_reborn_tests`.
  It does not emit crate ownership, reverse-dependency closures, bucket names,
  or lane selections.
- This is an average-case PR optimization, not a compile-time reduction for the
  current "run everything on every PR" target. A typical small PR may improve a
  lot, but the full Reborn suite wall clock remains unchanged when Reborn-wide
  paths or shared crates change.
- The safety risk is higher than H14/H18 because a bad mapping skips tests
  instead of only changing cache quality or partition count.
- Merge-group full-suite execution is a useful safety valve, but it would move
  some regressions from PR feedback to merge-queue feedback unless the mapping
  is very conservative.

Benchmark status:

- Not started in this PR. Treat as a separate H17 safety-first benchmark:
  first add a dry-run classifier that reports selected buckets without skipping
  them, compare selected buckets against actual changed crates across recent
  PRs, then enable skipping only after the dry-run data is convincing.

## H18: Increase Root-Test Partitions

Advice under test:

- Root tests become worth partitioning only after compile cache quality
  improves enough for root-test execution to be on the critical path.

Change under test:

- Increase `root-reborn-parity-tests` from `4` partitions to `6` partitions:
  - `REBORN_ROOT_TEST_PARTITIONS: 6`
  - matrix `partition: [0, 1, 2, 3, 4, 5]`
- Keep `scripts/ci/run-reborn-root-partition.sh` unchanged. The script already
  validates arbitrary positive partition counts and modulo-partitions the same
  sorted `tests/reborn_*.rs` plus `tests/support_unit_tests.rs` set.

Red-team notes before benchmarking:

- This does not reduce compilation. Every root partition still compiles the
  same root-package test graph before running its assigned test binaries.
- It adds two jobs, which increases runner-slot pressure. It is only worth
  retaining if the Reborn workflow wall clock improves materially enough to
  compensate for the extra jobs.
- Because H14's measurement made root partition `0` the slowest observed job
  at `450s`, this is the first point where the root partition experiment is
  directionally plausible.

Benchmark result:

- Branch/PR: [`codex/ci-compile-benchmarks`, PR #5648](https://github.com/nearai/ironclaw/pull/5648)
- Workflow run: [`28731792679`](https://github.com/nearai/ironclaw/actions/runs/28731792679)
- Status: success.
- Wall clock: `10m41s`
  (`2026-07-05T06:19:06Z` to `2026-07-05T06:29:47Z`).
- Total Reborn jobs: `29`, up from H14's `27`.
- Root-test jobs: `6`, up from `4`.

| Metric | H14 retained state | H18 measurement | Delta |
| --- | ---: | ---: | ---: |
| Reborn workflow wall clock | `7m49s` | `10m41s` | `+2m52s` |
| Total Reborn jobs | `27` | `29` | `+2` |
| Root-test jobs | `4` | `6` | `+2` |
| Slowest root partition | `450s` | `595s` | `+145s` |
| `host-runtime` | `373s` | `512s` | `+139s` |
| `composition-core` | `308s` | `481s` | `+173s` |
| `Reborn integration coverage (groups)` | `265s` | `493s` | `+228s` |

Decision:

- Reject and revert the workflow change. Increasing root partitions does not
  reduce compile time, and in this run the extra fanout appears to add runner
  and cache contention across the same Rust jobs.
- The result supports the red-team concern: root partitioning is not a useful
  lever while each partition still recompiles the same root-package graph.
