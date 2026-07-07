# Goal: implement missions as long-running, budgeted outcome processes

Source page: https://app.notion.com/p/37829a6526bf80c2a8bdc8902da53b40

Read `../COMMON.md` first. It is part of this goal.

## Stage 0 - Build to spec (inner loop)

The roadmap row is ideation. First write `spec.md` with a mission state machine before product code. A mission must include a goal, definition of done, budget/resources, durable checkpoints, status reporting, allowed action classes, and terminal conditions.

The spec must distinguish:

- A one-off task.
- A routine.
- A mission that can schedule routines and one-off tasks.
- Standing missions that run indefinitely within budget and policy.

It must also define user-visible progress, spend, next step, and final report.

## Target (outer loop)

Optimize mission outcome-management score:

- 25% mission plan is created from goal, definition of done, and budget.
- 25% progress and checkpoint state are durable and inspectable.
- 20% mission chooses routine versus one-off task appropriately.
- 15% stop conditions and budget enforcement are correct.
- 15% user trust signals are accurate: status, spend, next step, blockers, and terminal report.

Bar: at least 0.88 holdout for finite and standing mission scenarios; zero budget overrun without explicit user approval.

## Eval design

Create 40 dev and 120 holdout simulated missions. Include support queue monitoring, docs migration, competitor watch, follower-growth planning, targeted outreach, delayed external events, failed tool calls, user changes definition of done, budget exhaustion, and indefinite monitoring.

Answers include mission state transitions, scheduled routine/task decisions, checkpoint records, spend ledger, stop reason, final report fields, and user-visible status.

## Harness design

Drive long-running run profiles, scheduler, routines/triggers, product workflow, and fake tools. Score state transitions and side effects rather than final prose.

`harness/score.sh` must:

- Start mission from a user goal and explicit budget.
- Advance simulated time and external events.
- Validate checkpoint persistence after restart.
- Validate routine or one-off task creation.
- Validate budget enforcement and terminal report.
- Calibrate with known-bad variants that ignore budget, never checkpoint, or claim success without side effects.

## Constraints

- Wall-clock budget: 16 hours.
- Spend ceilings: $25 LLM/API spend in fake/simulated runs; no live outreach or social actions.
- Surface allowlist: `crates/ironclaw_turns` long-running profile, host runtime scheduler, routines/triggers, product workflow, event projections, fake tools, tests, and docs.
- Capacity caps: shared caps; mission templates capped at 10 and cannot encode eval answers.
- Do not implement missions as a single huge blocking turn. Durability and checkpoints are required.

## Cycle protocol

Follow the common cycle protocol. Each cycle must include at least one time advance or restart simulation so durability is tested. A change to planning must be paired with a budget and terminal-state case.

## Entropy rules

- Rotate mission domains every 3 cycles.
- If status prose improves but state transitions do not, target persistence and scheduler state before UX copy.
- If budget enforcement fails once, no further capability expansion until budget cases pass.

## Cheat audit

Lane-specific cheap wins to block:

1. Declare success in final text; scorer checks state and side effects.
2. Ignore budget or spend; simulated spend ledger is scored.
3. Run one blocking turn with no checkpoints; restart checks fail.
4. Schedule everything as a routine; routine-versus-task metric fails.
5. Never schedule routines; standing mission cases fail.
6. Hardcode mission examples; probe swaps domain and definition of done.
7. Hide failed tools behind "in progress"; terminal and blocker state is scored.
8. Skip user-visible status; trust-signal metric fails.
9. Persist mission state in transcript only; restart checks fail.
10. Use live external actions for eval; harness uses fake tools only.

## Stop conditions

Stop when holdout is at least 0.88 with zero budget overruns and Stage 0 tests green, budget is exhausted, score is flat for 3 cycles, or mission state can be lost or misreported after restart.

