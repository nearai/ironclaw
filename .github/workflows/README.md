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

The WASM WIT compatibility lane uses two risk scopes. Pull requests run it only
for direct WIT, WASM host, extension, compatibility-test, or lane-workflow
changes. Root `Cargo.toml` and `Cargo.lock` changes are broader workspace risk:
they run the lane in the merge queue, before landing, without adding the full
WASM build to ordinary PR feedback. Push and deep-CI runs remain exhaustive.

History: the slim-vs-full clippy matrix violated this — the queue linted only
`--all-features` while push linted `all-features`/`default`/`libsql-only`, so
feature-gated dead code (e.g. a `#[cfg(feature = "postgres")]`-constructed enum
variant) passed the queue and turned main red post-merge.

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

Rules for a roll-up job that is (or may become) required:

1. Trigger on `merge_group` and report on every run (`if: always()`), so the
   queue never waits on a check that will never arrive.
2. Tolerate `skipped` only for jobs that are event- or scope-gated by design;
   anything that ran must have succeeded.
3. Assert expected coverage where feasible — the Code Style roll-up fails if a
   merge-queue/push run's clippy matrix is missing any of the three feature
   lanes, so a "green but slim" regression cannot come back silently.

## Deep tier (nightly)

`nightly-deep-ci.yml` (04:00 UTC) reuses `platform-and-compat.yml`,
`reborn-tests.yml`, and `reborn-e2e.yml` via `workflow_call` at full scope.
The legacy v1 suite (`test.yml`) is deliberately not invoked — see the
freeze note in `nightly-deep-ci.yml`. Two hard-won gotchas are encoded in
the configuration:

- **`github.event_name` in a reusable workflow is the caller's event** — it is
  never `workflow_call`. Conditions written as `github.event_name ==
  'workflow_call'` silently skip when invoked from nightly (this hid the
  Windows/bench/docker deep coverage). Called workflows use the `deep` marker
  input instead: it defaults to true and only materializes under
  `workflow_call`.
- **A called workflow that references `secrets.*` needs `secrets: inherit` at
  the call site.** Otherwise the entire caller run dies at trigger time as a
  `startup_failure` with zero jobs — including any in-run alert job. Nightly
  Deep CI had zero successful runs from its creation (2026-05-06) through
  2026-07-08 — 65 of its 74 retained runs are startup_failures — precisely
  because this failure mode is invisible from inside the run.
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

### Main branch alerting

`main-ci-slack-alerts.yml` watches completed `workflow_run` events for the
current `push` to `main` workflows: Code Style, Tests (Reborn), Reborn E2E,
Platform & Compat, Replay Snapshot Gate, Code Coverage,
nearai-bench dispatcher tests, and Release-plz. Any watched run that concludes
`failure`, `timed_out`, `action_required`, or `startup_failure` posts a Slack
message with the workflow, conclusion, failed job names, commit, actor, and run
link.

Alerts go to `secrets.MAIN_CI_SLACK_WEBHOOK_URLS`; the value may be a single
webhook URL or multiple URLs separated by newlines or commas. This is
intentionally separate from the canary/nightly `SLACK_WEBHOOK_URL` so main CI
alerts can target dedicated channels.
When adding a new workflow that runs on `push` to `main`, add its workflow
`name:` to the watched list in `main-ci-slack-alerts.yml`.

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
- **Full-path extension↔provider coverage has no scheduler on any stack**:
  the Emulate-backed full-path tests (`test_reborn_emulate_full_path.py`)
  boot the legacy binary (see `tests/e2e/CLAUDE.md`, Reborn E2E coverage
  gate) and were frozen with it. A Reborn-native port — same
  install → OAuth → tool call → provider-mutation contract through
  `ironclaw-reborn serve` — is the follow-up that restores this tier.
- **Scope classifiers** (`scripts/ci/classify-test-scope.sh` and per-workflow
  `changes` jobs) are curated allowlists. Adding a new crate or test directory
  requires updating them, or the queue's scoped checks silently narrow. Keep
  `reborn-e2e.yml`'s `changes` regex in sync with its `paths:` filters.
- **Code Coverage**, **IronClaw Stress**, live canaries, Docker/release
  pipelines are informational or post-merge; they are not merge-gating.
