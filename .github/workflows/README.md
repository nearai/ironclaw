# CI Contract

This directory implements a tiered CI contract. Each tier has a distinct job;
a check belongs to exactly one tier on purpose.

| Tier | Event | Job |
|---|---|---|
| PR feedback | `pull_request` | Fast, scoped signal for the author. May run slim matrices and path-scoped subsets. |
| **Production gate** | `merge_group` (merge queue) | The authority on what reaches `main`. Runs deterministic checks in the **same shape as push** on the merged state. |
| Post-merge confirm | `push` to `main` | Confirms the queue's verdict, warms shared caches, feeds Codecov/canaries. Should never be the first place a deterministic failure appears. |
| Deep / scheduled | `schedule` (nightly) | Exhaustive suites too slow for the queue: legacy v1 matrix, full browser E2E, stress scans. |

## The invariant

**No deterministic failure may be main-only.** If a check runs deterministically
on `push` to main, the merge queue must run it in the same shape first
(`merge_group` does not support `paths:` filters — use a `changes` scope job
instead). External/live checks (canaries, deploys, releases, benchmark
thresholds) are exempt: they stay out of the queue by design.

The canonical local composition of the deterministic Reborn gates is
`scripts/ci/run-hermetic-deterministic-suite.sh all`. CI invokes the same
checked-in stages through that runner so credentials, ambient behavior,
mutable roots, clock/seed inputs, and non-loopback egress have one mechanical
boundary. Setup and exclusions are documented in
`docs/internal/hermetic-deterministic-suite.md`.

The WASM WIT compatibility lane uses two risk scopes. Pull requests run it only
for direct WIT, WASM host, extension, compatibility-test, or lane-workflow
changes. Root `Cargo.toml` and `Cargo.lock` changes are broader workspace risk:
they run the lane in the merge queue, before landing, without adding the full
WASM build to ordinary PR feedback. Push and deep-CI runs remain exhaustive.

`reborn-tests.yml` follows the same PR-versus-queue contract. Pull requests use
`reborn_pr_test_plan.py` to run affected crate buckets and exact changed root
and integration suites without LLVM instrumentation. Recorded QA
replay remains a baseline on every pull request because it detects ordering
and cross-surface regressions that cannot be inferred from changed paths. The
full transitive reverse workspace dependency closure is included in PR crate
selection for production-source changes. Package-owned tests, examples, and
benches run only their owning package because they cannot alter a dependent
package's production behavior. A changed top-level Cargo test, example, or
bench target runs directly; nested support changes retain all owning-package
targets. Foundational-crate changes that span more than
three canonical buckets coalesce every changed and dependent package into at
most three PR jobs instead of omitting consumer tests. The merge queue and
pushes to `main` still run every crate bucket, root partition, group suite,
integration lane, recorded replay, and coverage gate. Shared root or
integration support changes run one
representative PR partition or lane, with the exhaustive fan-out required in
merge queue. CI workflow, CI script, coverage-policy, toolchain, and workspace
topology changes use their owning static/compile gates on the PR and receive
the exhaustive Reborn matrix in merge queue. Unknown paths and empty diffs
fail quickly at planning instead of silently launching or skipping an
unbounded matrix. A planner execution or schema failure also fails the
required check loudly.
`Cargo.lock` is scoped only when a structured base/head comparison proves that
the lockfile changed solely in dependency lists for workspace manifests changed
by the same PR. Package additions/removals, versions, checksums, unrelated
workspace edges, and unreadable base state receive their full dependency
breadth in merge queue; changed workspace manifests still select their
production dependency closure on the PR. The stress tool
is owned by `ironclaw-stress.yml`, and changed-line coverage exemptions are
schema-checked in Code Style instead of launching unrelated integration lanes.
The queue therefore preserves exhaustive deterministic evidence while
ordinary PRs avoid consuming 20-plus runners for unrelated lanes. Pull-request
parallelism is capped at three crate buckets, one root partition, and one
integration lane; merge queue and main retain full matrix parallelism so this
feedback optimization does not serialize the production gate.
Full-coverage crate buckets run one multi-package `cargo llvm-cov` invocation
per bucket, preserving every package test and the bucket LCOV artifact while
sharing dependency compilation across packages in the same job.

`Tests (Reborn)` owns Rust crate, root, architecture, runtime, and coverage
contracts. Code Style owns WebUI lint, Vitest, and the production build on all
code events. Pull-request Clippy covers production libraries and binaries for
directly changed workspace packages with all features. Test-only and CI-only
PRs do not compile an unchanged workspace solely for linting. Root
`Cargo.toml` and `Cargo.lock` changes lint every workspace package because
their dependency and feature impact is workspace-wide. Merge queue and main
lint the full workspace, add test and example targets, and run the
default-feature matrix. Code Style's CLI Rust smoke and Reborn E2E's
four Rust groups run on merge queue and main, where they validate the
exhaustive merged state, but do not repeat those contracts on PR runners. The release-binary
smoke harness self-test remains in Code Style's fast deterministic job on every
code PR. Reborn E2E continues to build the real product binary and run all
browser/provider lanes on pull requests.
Critical mutation manifests, selection logic, and changed-function resolution
run on each PR. Actual `cargo-mutants` execution is a merge-queue gate, avoiding
a long and low-frequency compile workload on the author feedback path without
allowing selected mutations to reach `main` untested.
The four provider-operation shards run as two concurrent pairs. Each shard
keeps its own pytest process and hermetic runtime, while each pair shares one
runner and one Emulate checkout/build cycle. This preserves shard isolation
without restoring the two runner allocations removed by pairing.
The fast Responses API and black-box contracts share the browser worker in
separate hermetic pytest processes, capping the E2E fan-out at four workers.

History: the slim-vs-full clippy matrix violated this — the queue linted only
`--all-features` while push linted a broader matrix, so feature-gated dead code
could pass the queue and turn main red post-merge.

## Required checks and where they're enforced

Branch enforcement lives in the repository **ruleset "Main"** (Settings → Rules
→ Rulesets), *not* classic branch protection — the classic API reports
`required_status_checks: null`. Inspect the effective rules with:

```bash
gh api repos/nearai/ironclaw/rules/branches/main
```

The ruleset enables the merge queue and requires these check contexts (stable
roll-up **job names**, never individual matrix jobs):

| Check context (job name) | Workflow | Status |
|---|---|---|
| `Code Style (fmt + clippy)` | `code_style.yml` | required |
| `Tests (Reborn)` | `reborn-tests.yml` | required |
| `Reborn E2E` | `reborn-e2e.yml` | candidate — require once queue cost is confirmed |
| `Platform & Compat` | `platform-and-compat.yml` | candidate — require once queue cost is confirmed |

The 2026-07-30 queue-cost audit found only one retained `merge_group` sample
for each candidate: Reborn E2E took 594 seconds and failed; Platform & Compat
took 364 seconds and passed. Pull-request history was healthier (Reborn E2E
p50/p95 877/1124 seconds; Platform & Compat 415/492 seconds), but one real queue
sample—especially a failing E2E sample—is not enough evidence to alter the
repository ruleset. Both checks therefore remain candidates. Refresh the
workflow-run sample before promotion; a workflow being present on
`merge_group` is not itself proof that it is safe to require.

Rules for a roll-up job that is (or may become) required:

1. Trigger on `merge_group` and report on every run (`if: always()`), so the
   queue never waits on a check that will never arrive.
2. Tolerate `skipped` only for jobs that are event- or scope-gated by design;
   anything that ran must have succeeded.
3. Assert expected coverage where feasible — the Code Style roll-up fails if a
   merge-queue/push run's clippy matrix is missing any required feature lane,
   so a "green but slim" regression cannot come back silently.

Code Style deliberately consolidates formatting, dependency policy, static
guards, panic checks, and composition-budget checks into one
`fast-checks` job. These checks complete in seconds to a few minutes and do not
benefit from separate runners. Clippy and WebUI checks remain separate because
they are expensive independent gates. The CLI Rust smoke remains a separate
merged-state lane; its unique Python harness contract runs in `fast-checks` on
pull requests.

## Reborn release and manual compile preflight

`ironclaw-release.yml` is the tag-only cargo-dist publisher for the shipping Reborn
`ironclaw` package and binary. Matching `ironclaw-v*` tags build the seven
release targets, produce archives and checksums plus shell, PowerShell, and MSI
installers, and create the tag's GitHub Release. After that Release exists, the
workflow publishes the regular `nearaidev/ironclaw` Docker image with version,
`latest`, and source-SHA tags. cargo-dist derives the Release title and body
from the release metadata and `CHANGELOG.md`.

cargo-dist 0.31 generates workflow-wide `contents: write` and does not expose a
setting for built-in job permissions. The checked-in workflow is therefore
intentionally hardened beyond the generated template: repository access
defaults to `contents: read`, only `host` receives `contents: write`, and local
and global build jobs do not receive `GH_TOKEN`. `allow-dirty = ["ci"]` in the
workspace dist config prevents a later `dist generate` from silently restoring
the broader permission. When updating cargo-dist or its CI configuration,
remove that allow-dirty entry temporarily, regenerate the workflow, reapply the
permission boundary, and verify it with:

```bash
cargo test -p ironclaw --test smoke release_ci_ -- --nocapture
rg -n "permissions:|GH_TOKEN" .github/workflows/ironclaw-release.yml
```

`reborn-release-compile.yml` remains an independent compile-and-smoke preflight
that runs only through `workflow_dispatch`. It uploads temporary evidence
artifacts but does not publish a Release, and it is not called by the tag or
pull-request workflows.

| Rust target | GitHub runner |
|---|---|
| `x86_64-unknown-linux-gnu` | `ubuntu-22.04` |
| `x86_64-unknown-linux-musl` | `ubuntu-22.04` |
| `aarch64-unknown-linux-gnu` | `ubuntu-24.04-arm` |
| `aarch64-unknown-linux-musl` | `ubuntu-24.04-arm` |
| `x86_64-apple-darwin` | `macos-15-intel` |
| `aarch64-apple-darwin` | `macos-15` |
| `x86_64-pc-windows-msvc` | `windows-2022` |

The cargo-dist release and manual preflight both build the `ironclaw` package
and binary without backend feature flags; database backends compile
unconditionally. The tag publisher extracts each target's completed cargo-dist
archive and runs the shared release smoke before the artifact can enter the
upload set. The manual preflight runs the same smoke against its exact
dist-profile binary before uploading compile evidence. The smoke uses an
isolated home to verify CLI identity/help, the supported profile contract,
production-derived bundled-extension discovery through a real local runtime
assembly (including first-party, MCP-server, and WASM-tool runtime kinds), the
non-empty libSQL database created after its migrations complete, and the
migration-dry-run profile selection. Its catalog denominator is the shipping
binary itself, so adding or removing a bundled package does not require a second
hand-maintained CI list. The musl entries also use `readelf` to reject a program
interpreter or dynamic-library dependency, which prevents an installed musl
loader on the build runner from hiding a non-portable artifact.

For the `1.0.0-rc.1` to `1.1.0-rc.1` compatibility window, the tag publisher
also runs a blocking `Release upgrade canary` after all cargo-dist artifacts
exist and before `host` receives permission to publish them. It downloads and
checksum-verifies the exact previous Linux x86_64 release archive, compares it
with the exact candidate archive, creates state through the shipping WebChat
API, and verifies upgrade, restart, rollback, and re-upgrade plus the explicit
workspace-snapshot handoff. The local model endpoint is deterministic; this is
a release-artifact/runtime gate rather than live-provider evidence.

The scheduled Postgres capacity lane complements that portable gate by building
the same canonical binary with `--profile dist`, starting `serve`, applying the
Postgres-backed runtime migrations, and driving its authenticated API against a
mock provider. Weekly live provider jobs build the bundled WASM extensions and
exercise the Anthropic and OpenAI-compatible provider paths. The portable
archive smoke itself does not invoke every WASM/MCP/script runtime lane or
execute the generated shell/PowerShell/MSI installers; those remain separately
owned evidence and a green portable smoke must not be read as proof of them.

## Deep tier (nightly)

`nightly-deep-ci.yml` (04:00 UTC) reuses `platform-and-compat.yml`,
`reborn-tests.yml`, and `reborn-e2e.yml` via `workflow_call` at full scope.
`reborn-e2e.yml` owns the deterministic Reborn surface coverage used by pull
requests, the merge queue candidate check, and main. The standalone
`reborn-playwright.yml` schedule owns the broader generated four-shard browser
matrix. `ws12-suite-shards.toml` records the source run, duration weights,
provider-world affinities, and owned waivers; its generator refuses missing
files, affinity splits, stale entries, and retry-enabled deterministic shards.
It is post-merge nightly coverage, not a required merge check. Failed nightly
shards upload server logs, Playwright traces, screenshots, and videos, and the
nightly watchdog owns alerting for that workflow.

The same deep reuse raises all existing property-test generators from 256 to
2,048 random cases, runs the bounded mutation frontier, and replays the complete
hermetic journey/provider-fault inventory. `ironclaw-stress.yml` adds a
15-minute libSQL user-session soak alongside its libSQL ramps and
shipping-profile Postgres API capacity lane. `live-canary.yml` keeps the
three-hour Reborn WebUI cadence and runs the broader credentialed provider
matrix weekly. Workflow-contract sabotage tests fail if any of these schedules,
guards, merge/main triggers, or release gates disappear.

The Reborn E2E job also publishes `product-surface-coverage-<sha>`. Its JSON and
Markdown files join the shipped capability denominator with typed contract,
journey, and fault registries. The generator fails on unclassified or
unevidenced tested capabilities and lists owned gaps, waivers, and live-only
rows separately. Reporting imports the existing registries; it does not own a
duplicate CI capability or journey list. Provider journeys carry typed
scheduled-live bindings to the exact `live-canary.yml` job, case id, and
`results.json` artifact. The matrix reports those cells as `scheduled`, not
`covered`: a recorded trace or declared cron is never presented as a passing
live result.

The legacy v1 suite (`test.yml`) is deliberately not invoked — see the
freeze note in `nightly-deep-ci.yml`. Two hard-won gotchas are encoded in
the configuration:

- **`github.event_name` in a reusable workflow is the caller's event** — it is
  never `workflow_call`. Conditions written as `github.event_name ==
  'workflow_call'` silently skip when invoked from nightly (this hid the
  Windows/bench/docker deep coverage). Called workflows use the `deep` marker
  input instead: it defaults to true and only materializes under
  `workflow_call`.
- **A called workflow that references `secrets.*` needs those secrets passed at
  the call site**, either through an explicit mapping (preferred for a
  narrowly privileged publish job) or `secrets: inherit` when it truly needs
  the caller's full secret set. Otherwise the entire caller run dies at trigger
  time as a `startup_failure` with zero jobs — including any in-run alert job.
  Nightly Deep CI had zero successful runs from its creation (2026-05-06)
  through 2026-07-08 — 65 of its 74 retained runs are startup_failures —
  precisely because this failure mode is invisible from inside the run.
  `nightly-watchdog.yml` (08:00 UTC) exists for exactly that: it checks each
  nightly's latest scheduled run from outside and posts the failure to Slack
  even when the run itself never started.

### Nightly alerting

One path only: `nightly-watchdog.yml` (08:00 UTC) checks the latest scheduled
run of each nightly — Nightly Deep CI, Reborn Playwright, IronClaw Stress. A
run that is missing, stale (>26h: the cron didn't fire),
or concluded anything but success posts a failure line (workflow, conclusion,
failed job names, run link) to the Slack channel behind
`secrets.SLACK_WEBHOOK_URL` — the same webhook the live-canary report uses —
and turns that watchdog matrix job red, so the watchdog's own run history is
the failure record. Successes post nothing, and there is no GitHub-issue
trail: the former in-run alert jobs and `nightly-alert-issue.sh` were removed
in favor of this single external check, because an in-run alert dies with its
own run on a startup_failure and can never see a cron that didn't fire.

### Main branch and merge-queue alerting

`main-ci-slack-alerts.yml` watches completed `workflow_run` events for the
current `push` to `main` and `merge_group` workflows: Code Style, Tests
(Reborn), Reborn E2E, Platform & Compat, Replay Snapshot Gate, Code Coverage,
nearai-bench dispatcher tests, and Release-plz. Any watched run that concludes
`failure`, `timed_out`, `action_required`, or `startup_failure` posts a Slack
message with the workflow, conclusion, failed job and step names, available
failure annotations, commit, actor, and run link. Merge-queue alerts also
resolve the PR number from GitHub's `gh-readonly-queue/main/pr-<number>-...`
ref and include the PR title, author, and link.

Main-branch alerts go to `secrets.MAIN_CI_SLACK_WEBHOOK_URLS`; the value may be
a single webhook URL or multiple URLs separated by newlines or commas.
Merge-queue alerts go to `secrets.SLACK_WEBHOOK_URL`, the existing live-canary
channel. This keeps post-merge CI alerts in their dedicated channels while
making queue bounces visible alongside live-canary failures.
When adding a new workflow that runs on `push` to `main`, add its workflow
`name:` to the watched list in `main-ci-slack-alerts.yml`.

Code Coverage uses same-ref concurrency with cancellation. When merges land
faster than coverage completes, only the newest cumulative `main` commit keeps
running; superseded post-merge coverage runs do not consume runners needed by
pull requests.

## Reborn-only release policy

For #6160, `ironclaw-release.yml` uses cargo-dist to publish only the canonical Reborn
`ironclaw` package. The active tag DAG consists of cargo-dist planning, the
seven target builds, universal installer generation, and GitHub Release
hosting, followed by the regular Reborn Docker image. Legacy v1 artifacts,
independently published WASM extensions, `ironclaw-worker`, and the old
registry-checksum path remain outside this DAG. The generated `announce` job
remains cargo-dist's final release step and does not restore those retired
products.

The `docker-image` job runs only after `host` creates the GitHub Release. If
Docker publishing fails, the existing GitHub Release and its artifacts remain
available while the overall workflow reports failure for retry/repair. Release
builds publish only `nearaidev/ironclaw`; they do not dispatch
`nearai/ironclaw-dind` because the caller explicitly sets `trigger_dind: false`.
The reusable `docker.yml` keeps its independent manual and hourly staging entry
points, including their existing optional DIND dispatch. The manual
`reborn-release-compile.yml` preflight remains independent from publishing.

## Known accepted gaps (deliberate, revisit as needed)

- **Windows clippy** (`code_style.yml` `clippy-windows`) runs on push only;
  **Windows build** (`platform-and-compat.yml` `windows-build`) runs on push
  and in the nightly deep reuse. Windows-only breakage is accepted as
  post-merge; the Linux full feature matrix catches the dominant class
  (feature-gated cfg errors).
- **Benchmark compilation** (`cargo bench --no-run`) runs on push and nightly
  only, and the clippy lanes do not pass `--benches`. Bench targets exist only
  in `crates/ironclaw_safety` today.
- **Replay Snapshot Gate** runs on push + via the nightly legacy suite; it
  covers the retiring v1 engine.
- **The legacy v1 suites are deliberately invoked nowhere** — v1 (`src/`) is
  frozen pending removal. `test.yml` (the only place the root `ironclaw`
  package's tests run) is no longer called by nightly, and the former
  `nightly-e2e.yml` scheduler for the v1 browser suite (`e2e.yml` full mode)
  is deleted — it had zero successful runs in retained history. Until `src/`
  is deleted, a v1 bug fix that must land should temporarily restore the
  `deterministic-deep-tests` call in `nightly-deep-ci.yml` (and/or dispatch
  `e2e.yml` manually). Delete `test.yml` and `e2e.yml` together with `src/`.
- **Broad full-path extension↔provider mutation coverage remains legacy-only**:
  `test_reborn_emulate_full_path.py` still boots the legacy binary. The Reborn
  E2E job now replays every harvested live-QA model trace against Emulate's
  supported provider operations and runs a Reborn-native Drive read path through
  `ironclaw serve`; equivalent standalone mutation paths for every provider are
  still follow-up work.
- **Scope classifiers** (`scripts/ci/classify-test-scope.sh` and per-workflow
  `changes` jobs) are curated allowlists. Adding a new crate or test directory
  requires updating them, or the queue's scoped checks silently narrow. Keep
  `reborn-e2e.yml`'s `changes` regex in sync with its `paths:` filters.
- **Code Coverage**, **IronClaw Stress**, live canaries, Docker/release
  pipelines are informational or post-merge; they are not merge-gating.
