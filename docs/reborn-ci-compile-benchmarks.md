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

The current branch has restored the workflow after each benchmark. The PR diff
is report-only unless a new benchmark experiment is intentionally in flight.

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
| H12 | Move `ironclaw_reborn_cli` from `reborn-core` to `webui-ingress`. | Keep job count flat while grouping the WebUI-shaped CLI build with the WebUI ingress bucket instead of the core Reborn bucket. | Running | Pending CI result. |

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
- Workflow run: pending.
