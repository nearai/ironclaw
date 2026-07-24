# executor/ — `canonical.rs` Is The Spine, Not A Dumping Ground

`canonical.rs` dispatches to stages and sequences their typed outputs. It is
never the place for new decision logic or hand-rolled cross-cutting
mechanics — those belong in the stage or method that already owns the
related state or decision. See `../CLAUDE.md` ("Executor stage ownership")
for the crate-wide version of this rule.

## The rule

Any new decision logic (a stop/nudge/retry/gate policy check, a threshold
comparison, a `state.*` mutation gated on a condition) or repeated
cross-cutting mechanics (timing, tracing, progress emission) goes into the
stage or method that already owns the related state or decision — never
inline in `canonical.rs`'s `execute()` or its per-branch `execute_*_turn`
methods. If no existing stage owns the new behavior, that's a signal to add
one, not to inline the logic.

The per-branch methods (`execute_prepared_turn`, `execute_resume_turn`,
`execute_skip_model_turn`) exist so each branch gets its own stack frame and
a diff to one branch stays visibly scoped to that branch. They are not a
looser license to inline stage-owned logic.

## Review flag

Flag a diff to `canonical.rs` (or its `execute_*_turn` methods) that adds any
of the following without a matching change in the file that owns the
decision:

- an `if`/`match` arm reading or mutating a `state.*` field on a condition;
- a hand-rolled timing block (`Instant::now()`/manual duration math) instead
  of the shared `latency::stage!` macro or a `timed`/`_timed` sibling method;
- a helper function in `canonical.rs` called by only one `execute_*_turn`
  method — that logic belongs inside that method's owning stage.

Shared post-processing glue (e.g. matching a stage's typed output the same
way from every call site) is not decision logic and may stay in
`canonical.rs`; don't let duplicate copies of it accumulate — compute once,
match once.

## Exception: ordering-dependent mutations

A mutation that looks stage-owned may still need to live in `canonical.rs`
if another spine-owned ordering (a checkpoint write, an ack, a cancellation
boundary) depends on it happening at that exact point in sequence. Before
relocating such a mutation into the stage that computes its trigger
condition, check whether a checkpoint/ack/cancellation write brackets it —
moving it can silently change what a resumed run observes. When that's the
case, leave it in place with a comment explaining why the ordering is
load-bearing.

## What this rule does not cover

- Sequencing calls to stages and matching on their typed outputs — that *is*
  `canonical.rs`'s job.
- Spine-owned bookkeeping (`state.iteration = state.iteration.saturating_add(1)`,
  the per-iteration `IterationStarted` progress event) that belongs to the
  loop's own protocol, not to any single stage's decision.

## Reference

Sibling rule with the same shape: `.claude/rules/agent-loop-capabilities.md`.
