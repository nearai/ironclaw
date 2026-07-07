# Goal: implement Missions as durable, budgeted outcome-management state machines that schedule routines and tasks without duplicate identity or lost progress

This lane merges `docs/lfd/roadmap-blue-lanes-2026-07-07/11-missions/goal.md`, the lane-11 addendum, `lfd/_briefs/missions.md`, and `lfd/_briefs/COMMON.md`. Reconciliation: the addendum makes the state-machine framing primary. The older brief's surviving themes are meta-prompt-from-memory data flow, probe-varied adaptation via `next_focus`, no duplicate mission identity, and restart durability. The stricter portfolio bar applies: **0.90 holdout**.

## Stage 0 — Build to spec (inner loop)

Implement `spec.md` first. Do not score against the eval until the inner loop is green. Tests stay green every cycle thereafter.

Required Stage-0 gates:

1. `cargo fmt` clean.
2. `cargo clippy --all --benches --tests --examples --all-features` with zero warnings.
3. `cargo test -p ironclaw_triggers` and `cargo test -p ironclaw_triggers --features libsql`.
4. `cargo test -p ironclaw_turns --test run_profile_contract` plus new long-running mission state/checkpoint tests required by `spec.md`.
5. Trigger/composition scheduler coverage: the existing Reborn trigger group tests in this worktree plus any new MissionManager integration test that drives the production trigger-to-turn seam.
6. Make `tests/integration/lfd/profiles/missions.rs` execute every dev case with `status: "ran"`; `unsupported` scores zero and is expected only before the profile exists.

Stage 0 must prove a mission has: goal, definition of done, budget/resources, durable checkpoints, status reporting, allowed action classes, terminal conditions, routine-vs-task scheduling decisions, and restart recovery.

## Target (outer loop)

Optimize the mission outcome-management score on contract cases:

- 25% mission plan is created from goal, definition of done, and budget/resources.
- 25% progress and checkpoint state are durable and inspectable after restart.
- 20% mission chooses routine versus one-off task appropriately.
- 15% stop conditions and budget enforcement are correct.
- 15% user trust signals are accurate: status, spend, next step, blockers, and terminal report.

The scorer prices both directions: missing required state starves the numerator; forbidden duplicate identity, budget overrun, post-completion fire, live external send, or unbounded spawn halves or voids relevant cases. Contracts assert state queries, events, gates, and tool/egress effects, not final prose.

Bar: **0.90 on holdout only**, with zero budget-overrun cases. Score with `harness/score.sh`. A `VOID: constraint violation` result means a constraint was violated; remove the violation. The harness will not identify eval-sensitive details. Holdout is aggregate-only, max 3 calls per 24h, audit logged. Acceptance is measured on holdout exclusively.

Small-eval warning: Per-feature evals are 30-60 dev + 10-15 holdout cases: far below the ~200 enumerability threshold. The compensating controls are (a) contract-style scoring (satisfying a behavioral contract usually requires the machinery, unlike data-lookup evals), (b) probe gap as the memorization gauge, (c) feedback capped to aggregate + <=5 worst case ids, (d) holdout answers off-repo.

## Constraints

- Wall-clock budget: **12h**. Check `harness/status.sh` every cycle for elapsed time, score history, spend, holdout call budget, and trend.
- Spend ceiling: **$25** simulated LLM/API budget. The eval uses fake tools and synthetic events. No live outreach, purchases, social actions, scraping gated sources, or live external sends.
- Surface allowlist for the optimizer: `crates/ironclaw_triggers/**`, `crates/ironclaw_turns/**`, Reborn composition scheduler/runtime seams needed to wire production MissionManager behavior, `tests/**`, `lfd/missions/LOG.md`, and the single runner profile `tests/integration/lfd/profiles/missions.rs` if present.
- Read-only: this `goal.md`, `spec.md`, `lfd/missions/harness/**`, `lfd/missions/eval/**`, `lfd/_shared/**`, other lane directories, and shared integration support. Eval inputs may be read where the harness exposes them; eval answers never.
- Banned: reading `lfd/missions/harness/answers.dev.json`, reading or writing `/Volumes/NVME/ironclaw-lfd/holdout/missions/**` during optimization, editing scorer/template/shared LFD code, and weakening Stage-0 tests.
- Capacity caps are lint-enforced in `harness/caps.json`: eval case-id branches = 0, eval seed literals in product code = 0, dev failure-category literal tables = 0, budget/checkpoint bypass markers = 0, static meta-prompt shortcuts = 0, unbounded spawn loops = 0, `#[ignore]`/`#[cfg(never)]` additions in relevant tests = 0.
- Do not implement missions as a single blocking turn. Mission state must survive restart and must be inspectable through state queries.

## Cycle Protocol

1. Run `harness/score.sh` on dev with the current outcomes.
2. Run `harness/probe.sh`; if the probe gap grows, the next change must remove or blind an eval-shaped artifact rather than adding special cases.
3. Run `harness/status.sh` and confirm time, spend, and holdout call budget remain.
4. Append the next `LOG.md` cycle entry before changing code: hypothesis, expected failure mode, diagnostic, and planned scope.
5. Make one scoped change through production seams.
6. Run Stage-0 tests and the scorer again.
7. Commit the cycle as `cycle <n>: <score>` even if score is flat.

Every cycle must include at least one durability signal: a time advance, restart simulation, or checkpoint replay check. A planning change must be paired with a budget and terminal-state case.

## Entropy Rules

- Rotate mission domains every 3 cycles; do not tune only support/docs cases.
- If status prose improves but state transitions do not, target persistence, scheduler, and budget state before UX copy.
- If a budget case fails once, stop expanding capabilities until budget gates pass.
- If duplicate identity or restart-loss appears once, no further scheduling feature work until identity/checkpoint recovery is fixed.
- If the metric is flat for one cycle, the next attempt must be structural. Same-knob-harder is banned.
- Every 5 cycles, try a structurally different approach even if the score is inching up.

## Cheat Audit

1. Declare success in final text -> contracts score persisted state, events, checkpoints, budget ledger, and terminal report fields.
2. Ignore budget or spend -> budget ledger and forbidden overrun matchers price violations; caps reject bypass markers.
3. Run one blocking turn with no checkpoints -> required checkpoint counts, final checkpoint, and restart recovery fail.
4. Schedule everything as a routine -> finite task cases require zero routines and task records.
5. Never schedule routines -> standing mission cases require routine decisions.
6. Hardcode dev examples -> probe renames entities/dates and caps reject case-id/seed literals in product code.
7. Hide failed tools behind "in progress" -> failure cases require blocker state, failure category, next_focus, and approach history.
8. Spawn a fresh mission on every fire -> duplicate-identity and same-slot cases require one accepted fire and one mission id.
9. Let recursive missions spawn forever -> spawn-guard cases require recursive spawn denial and bounded child count.
10. Use static meta-prompt text -> memory data-flow contracts require prompt envelopes to contain seeded memory doc paths and digest terms.
11. Edit scorer/eval/answers -> read-only surface, pins, canaries, and VOID semantics.
12. Mine miss lists or lint reports -> dev feedback is capped to <=5 ids; lint reports go outside optimizer-readable surfaces and print only VOID.

## Stop Conditions

Stop when holdout is at least 0.90 with zero budget-overrun cases and Stage 0 green; any wall-clock/spend/holdout-call budget is exhausted; marginal dev gain is <0.01 for 4 consecutive cycles; a mission can lose state after restart; duplicate mission identity is observed; a budget overrun occurs without explicit approval; or the scorer is found invalid and cannot be repaired within the remaining budget.

On stop, write a final `LOG.md` report with best dev score, best holdout score if any, probe gap trend, what generalized, what was abandoned, remaining risks, and highest-leverage next steps.

## Pre-flight

Use a disposable provider key with a provider-side spend limit if live-model experiments are later added. Babysit cycle 1 and confirm the agent uses `score.sh`, `probe.sh`, `status.sh`, Stage-0 tests, and `LOG.md` instead of reading sealed answers or editing the harness.
