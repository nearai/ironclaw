# CI build-time LFD target

This is a loss-function design target for reducing IronClaw CI build time without reducing test coverage.

## Current conclusion

The OVH "build one nextest archive, then fan out" experiment is useful evidence, but it is not a PR-CI win in its current form. The consumer jobs got faster, but the new archive producer became a serial gate, so the workflow finished later overall.

That does not make OVH useless. It means OVH should be treated as a trusted cache/build producer only where the downstream jobs already need a shared artifact or where the current jobs repeatedly do the same trusted build work. It should not replace already-wide GitHub parallelism unless the measured critical path improves.

## Success target

Reduce build-related CI critical-path time by at least 30 percent on the selected workflow set, with no reduction in tests, features, security guards, or checked behavior.

Primary workflow set:

- `reborn-tests.yml`
- `reborn-e2e.yml`
- `reborn-coverage.yml`
- `test.yml`

Probe workflow set:

- `live-canary.yml`
- `coverage.yml`
- `reborn-playwright.yml`

`live-canary.yml` is a probe because scheduled runs are often red for product reasons. It is still important because it shows repeated Rust/WASM setup across Reborn WebUI v2 live QA shards.

## Observed baseline

Collected on July 4, 2026 from recent `main` GitHub Actions runs.

| Workflow | Run | Active time | Heavy jobs |
| --- | --- | ---: | --- |
| Tests (Reborn) | `28696684191` | 597s | `ironclaw_host_runtime` 557s, root partitions 381-462s, group tests 378s, `ironclaw_reborn_composition` 418s |
| Reborn E2E | `28696684177` | 487s | gateway smoke 481s, architecture 319s, WebUI smoke 192s |
| Reborn Coverage | `28696684175` | 574s | Reborn Coverage 571s |
| Tests (Legacy) | `28228462994` | 1888s | all-features 1475s, libsql-only 1466s, default 1443s, Telegram integration 569-621s, Slack 593s |
| Live Canary | `28694683262` | 1403s | Reborn WebUI v2 QA shards 1008-1354s; run failed, but still useful as a build-time probe |

The earlier OVH archive experiment on `codex/ovh-nextest-archive` produced these measured results:

- Baseline Reborn run `28681653394`: active time 603s.
- Full-feature archive run `28684600515`: active time 711s.
- `--no-default-features --features libsql` archive run `28685265502`: active time 793s.

The archive path should stay unmerged unless a later variant beats the baseline on total workflow time, not just consumer-job time.

## Loss function

The scorer compares recent baseline runs against candidate-branch runs.

Primary metric:

```text
speedup_percent = 100 * (baseline_active_seconds - candidate_active_seconds) / baseline_active_seconds
```

Aggregate score:

```text
score = speedup_percent - stability_penalty - guard_penalty
```

Hard void conditions:

- Any required workflow becomes red for a reason not present in the baseline.
- A test job, shard, or check is removed without an equivalent replacement.
- Tests are skipped, ignored, filtered out, or made non-fatal.
- Required feature sets are weakened.
- Trigger/path filters are narrowed to avoid CI.
- Secrets/live lanes are moved to untrusted runners.
- The scoring harness is modified in the same PR as an optimization without reviewer approval.

Soft penalties:

- More than 10 percent slower on any probe workflow.
- Reliance on a single warm-cache run without a second confirming run.
- More total runner minutes with only small wall-clock improvement.
- Higher live-secret exposure or broader self-hosted runner permissions.

## Allowed implementation levers

- Reuse the proven `reborn-playwright.yml` pattern: build once, upload a binary or exact test artifact, then fan out consumers.
- Add artifact producers only where consumers already wait on the producer or duplicate identical compile work.
- Improve Rust cache key sharing where feature sets and target dirs are genuinely compatible.
- Use OVH Redis/sccache as a persistent cache backend where credentials are available and HTTPS/SSH boundaries remain locked down.
- Use trusted OVH self-hosted runners only for maintainer/secret-safe jobs, never for fork PR code with secrets.
- Add timing/stat reporting that makes cache hit rates and build phases visible.

## First experiments to run

1. Live canary Reborn WebUI v2 QA: build WASM channels and any reusable Rust binaries once, upload artifacts, and let QA shards consume them. This is the best OVH/trusted-producer candidate because each shard repeats setup before long live tests.
2. Reborn E2E gateway/WebUI smoke: mirror the Reborn Playwright binary-artifact pattern if the Python fixtures can consume prebuilt `target/debug` binaries without changing test behavior.
3. Reborn Coverage: measure mold plus OVH Redis cache impact for `cargo llvm-cov`, and only keep it if total active time improves across two runs.
4. Legacy PR tests: identify duplicated WASM/channel builds and exact feature-compatible target cache sharing before attempting a central archive. Avoid serializing the three feature matrices unless the critical path improves.

## Non-goals

- Do not reduce the number of tests.
- Do not remove live canary coverage.
- Do not make red jobs informational.
- Do not move unsafe fork PR work onto OVH.
- Do not optimize only the visible dev workflows while slowing coverage/canary probes.
