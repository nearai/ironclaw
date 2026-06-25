# Runner concurrency investigation — 2026-06-25

Track: `fix/reborn-runner-concurrency` (off main `3cbde9b21`).

## TL;DR — root cause determination

**The reported symptom ("7 of 23 triggered runs never reached `turn run started`;
single `TurnRunnerId` serving serially → effective concurrency ≈ 1") is NOT a
runner-scheduler regression, and NOT a `worker_count=1` config foot-gun.**

Hard log evidence (operator-provided `logs.1782348290172.log`, ANSI-stripped)
shows the scheduler running **multiple runs concurrently**:

- 23:30 window — three runs overlap:
  - `81de47f4` started 23:30:07.45, finished 23:30:16.85
  - `6200c910` started 23:30:10.59, finished 23:30:23.70
  - `9461329b` started 23:30:13.01, finished 23:30:24.67
  → all three in-flight simultaneously ~23:30:13–23:30:16.
- Same pattern at 00:00 (`747d989b`/`281df473`/`a5a06cb6`) and 00:30
  (`36356f89`/`6c5f386c`/`5ad0055f`).

So production is decidedly **not** running at `worker_count=1`. The single
`TurnRunnerId(ce43288a…)` is one scheduler *instance* (one process), which is by
design — it is NOT a per-run worker. Concurrency is genuinely > 1.

### What actually produced the symptom (different track — do NOT fix here)

The "never started" / "did not finish before Slack delivery timeout" lines all
originate in `ironclaw_reborn_composition::slack_delivery`, not the scheduler:

- Real runs reach `BlockedApproval` within ~7s (e.g. `5e46d384`: started
  23:20:32, BlockedApproval 23:20:39, "turn run finished" 23:20:40).
- The Slack triggered-run delivery (`deliver_triggered_run` →
  `wait_for_actionable_triggered`, `slack_delivery.rs:2308`) delivers the first
  gate, records `delivered_blocked_marker`, then loops and waits for the *next*
  actionable transition. Because the approval is never answered, that second wait
  runs the full `max_wait` (`DEFAULT_TRIGGERED_RUN_DELIVERY_MAX_WAIT = 30*60s`,
  `slack_delivery.rs:60`) and finally logs "wait failed … did not finish before
  Slack delivery timeout" → `outcome=Failed`. The timeout fires exactly 30 min
  after the run blocked (`5e46d384` blocked 23:20:39 → wait-failed 23:50:45;
  `58ddc152` blocked 23:15:39 → wait-failed 23:45:45).
- The 8 "orphan" run_ids (`62aa9709`, …) that appear only in slack_delivery and
  never in the coordinator are prior-cycle / duplicate delivery waits, not
  starved scheduler runs.

This is an availability issue in the Slack delivery layer
(`slack_delivery.rs` / `runner_immediate_ack.rs`), explicitly assigned to a
separate track. It is out of scope here.

## Scheduler facts (verified, file:line)

- `TurnRunScheduler` (`crates/ironclaw_host_runtime/src/turn_scheduler.rs:154`)
  bounds concurrency with a `tokio::sync::Semaphore::new(max_concurrent_runs)`
  created in `run_scheduler_loop` (`:392`). Each claimed run is `tokio::spawn`ed
  onto a `JoinSet`, holding an `OwnedSemaphorePermit` for the run's lifetime
  (`spawn_executor_task`, permit dropped at "turn run finished"). Up to
  `max_concurrent_runs` runs execute concurrently. The model is sound (this is
  PR #5085's design) — confirmed.
- Production wiring: `crates/ironclaw_reborn/src/runtime.rs:659-660` builds the
  config as `TurnRunSchedulerConfig::default().with_max_concurrent_runs(worker_count)`.
  `worker_count` default = `DEFAULT_TURN_RUNNER_WORKER_COUNT = 16`
  (`crates/ironclaw_reborn/src/runtime.rs:65`); per-user cap 3, per-trigger 8 —
  none of which can floor concurrency to 1.
- `worker_count` is config-file only (`[runner].worker_count`), no env var;
  resolved in `runner_settings` (`crates/ironclaw_reborn_cli/src/runtime/mod.rs:954`).

## Real (low-severity) gaps worth a small guardrail

These are genuine, independently verifiable defects — not the reported symptom,
but real maintainability hazards uncovered while verifying. Each is testable.

1. **Divergent default.** `TurnRunSchedulerConfig::default().max_concurrent_runs`
   is the literal `4` (`turn_scheduler.rs:38`), while the production constant is
   `16`. Any `Default`-only caller silently under-provisions. `host_runtime`
   cannot import `ironclaw_reborn`'s constant (that would be a dependency cycle —
   `reborn` depends on `host_runtime`), so define the canonical default as a
   `pub const DEFAULT_MAX_CONCURRENT_RUNS` in `host_runtime` (the crate that owns
   `TurnRunSchedulerConfig`), set to `16`, and make `Default` use it.
   `ironclaw_reborn::DEFAULT_TURN_RUNNER_WORKER_COUNT` then *derives* from that
   constant (it depends on `host_runtime`, so the import is legal), making the
   two equal by construction — no literal duplication, no drift risk. (This is
   the code-review-improved form of the original "duplicate + pin-with-test"
   plan: the test stays as a documented invariant lock + a `> 1` assertion.)

2. **Stale doc.** `crates/ironclaw_reborn_config/src/config_file.rs:158` says
   `worker_count` "defaults to 4" — the real default is 16. Fix the comment and
   note that `1` serializes all runs.

3. **Silent degenerate value.** `runner_settings` accepts an explicit
   `worker_count = 1` with no signal. A `1` is technically valid but serializes
   the whole pool. Emit a startup `warn!` (not a silent override — respect the
   operator's explicit value) when the resolved worker_count is 1. This is the
   cheap recurrence-prevention guardrail.

   Note: production was NOT at `worker_count=1` (the data shows concurrency > 1),
   so this guards a hypothetical misconfiguration, not the observed incident. It
   is included only because it is one line, directly testable, and the project's
   guardrail-placement preference favors making a degenerate config loud.

## Plan

- `host_runtime`: add `pub const DEFAULT_MAX_CONCURRENT_RUNS: usize = 16;`, make
  `TurnRunSchedulerConfig::default()` use it. Add a unit test pinning
  `default().max_concurrent_runs() == DEFAULT_MAX_CONCURRENT_RUNS`.
- `ironclaw_reborn`: add a test asserting
  `DEFAULT_TURN_RUNNER_WORKER_COUNT.get() == ironclaw_host_runtime::DEFAULT_MAX_CONCURRENT_RUNS`
  so the two defaults cannot drift.
- `reborn_cli` `runner_settings`: emit `warn!` when resolved `worker_count == 1`.
  Add a test driving the real resolver that asserts the resolved value is 1 for
  `[runner].worker_count = 1` (the warn path), and that `Default` resolution is
  16. (Test the resolver — the caller — per "Test Through the Caller".)
- `reborn_config`: fix the stale doc comment.
- Spec: add the concurrency invariant to
  `crates/ironclaw_host_runtime/CLAUDE.md` so a future agent does not
  re-introduce a divergent default or assume the scheduler is serial.

## Explicit non-goals

- No scheduler behavioral change — the scheduler is correct.
- No Slack delivery change — that is the separate availability track.
- No fabricated "concurrency regression" — none exists; the data shows
  concurrency > 1.

## Red→green for the guardrail

The default-divergence test is the meaningful red→green: on the unpatched base,
`TurnRunSchedulerConfig::default().max_concurrent_runs()` is `4`, so a test
asserting it equals the production default (16) FAILS red; after the fix it is
`16` → GREEN. The cross-crate drift test likewise fails red (4 ≠ 16) and passes
green. The warn-on-1 resolver test asserts the resolved value for an explicit
`worker_count = 1`.
