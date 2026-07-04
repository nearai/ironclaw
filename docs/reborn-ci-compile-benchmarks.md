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
