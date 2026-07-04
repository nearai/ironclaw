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
| H1 | Narrow Reborn crate bucket targets from `--all-targets` to the default `cargo test` target set. | Less compile/link work per crate bucket. | In progress | Pending CI benchmark. |
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

- Branch/PR: pending
- Workflow run: pending
- Wall clock: pending
- Crate bucket job count: pending
- Slowest bucket: pending
- Decision: pending
