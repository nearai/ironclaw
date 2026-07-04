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

Evidence from the prior `main` run `28717545727` showed the long buckets are
mostly compile/link/setup time, with one notable runtime-heavy exception:

- `host-runtime`: `tests/host_runtime_services.rs` took `210.72s`.
- `webui-ingress`: test binaries were roughly tens of seconds total; most time
  was compile/link/setup.
- `composition-core`: first result arrived after roughly `4m39s`; test runtime
  was meaningful but still not the dominant cost.

## Hypotheses

| ID | Hypothesis | Expected effect | Status | Result |
| --- | --- | --- | --- | --- |
| H1 | Narrow Reborn crate bucket targets from `--all-targets` to the default `cargo test` target set. | Less compile/link work per crate bucket. | Tested | Rejected: wall clock regressed from `8m35s` to `8m43s`; job count unchanged. |
| H2 | Split compile-heavy buckets by dependency shape instead of package count. | Lower max bucket duration if closures are separable. | Not started | Pending. |
| H3 | Move `host_runtime_services.rs` out of the normal `host-runtime` bucket. | Reduce the slowest bucket and isolate the runtime-heavy WASM service tests. | Not started | Pending. |
| H4 | Reduce feature flags for slow crates where the coverage is duplicated elsewhere. | Less compile graph expansion in PR crate buckets. | Not started | Pending. |
| H5 | Remove OVH sccache from Reborn crate buckets. | Verify whether remote cache overhead is hiding any local cache gain. | Not started | Pending. |

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
