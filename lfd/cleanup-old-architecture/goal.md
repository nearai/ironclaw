# Goal: `ironclaw-reborn migrate` converts a populated v1 database into Reborn substrate with verified fidelity across all categories — the six documented migration gaps closed, idempotent, dry-run-honest, v1 never mutated

This is the lane-06 launch doc: the roadmap lane goal
(`docs/lfd/roadmap-blue-lanes-2026-07-07/06-clean-up-old-architecture/goal.md`
+ `../COMMON.md` + `../INSTRUMENTS.md` + the lane-06 LANE-ADDENDA entry)
merged with the designer brief (`lfd/_briefs/cleanup-old-architecture.md` +
`lfd/_briefs/COMMON.md`). Where they differ, the ADDENDA reconciliation
applies: **the brief's migration-fidelity contracts ARE this lane's
"replacement evidence" hard gate; the lane's deletion-ledger scoring stands
on top as the unscored-stretch process.** Both agree deletion never scores
by LOC.

## Scope (deliberate)

The SCORED target is **migration completeness + CLI wiring**, not code
deletion. Deleting `src/` (and engine-v2 remnants) is a stretch goal:
**unscored**, gated on the FULL suite green, and driven by the deletion
ledger in `spec.md` — every deletion names its Reborn replacement, the
tests proving replacement behavior, and the docs to update, BEFORE any
removal. Scoring deletion invites reckless removal; scoring fidelity does
not. If you reach the deletion stretch: one coherent surface per cycle,
run the targeted replacement tests and the deleted-symbol checks before
commit, and never delete a test merely because it fails after a deletion.

## Stage 0 — Build to spec (inner loop)

Implement `spec.md`. Make the test suite pass. Do not score against the
eval until tests are green. Tests stay green every cycle thereafter.

Stage-0 command list (additive to the repo-wide gate `cargo fmt` +
`cargo clippy --all --benches --tests --examples --all-features` with zero
warnings):

1. `cargo test -p ironclaw_reborn_migration --features libsql --test migration_roundtrip`
   — **spec-governed**: its expected gap set may only SHRINK as gaps
   close (red→green per category). Weakening it (`#[ignore]`, deleted
   assertions, loosened counts without a closed gap) is a violation.
2. `cargo test -p ironclaw_reborn_migration` — the crate's own tests.
3. A new `Command::Migrate` CLI test in `crates/ironclaw_reborn_cli`
   (subcommand parses, dispatches to `run_migration`, dry-run honored,
   report emitted) — write it red first, per repo testing discipline.
4. Make `tests/integration/lfd/profiles/cleanup-old-architecture.rs`
   (profile name `migration`) execute every dev case with
   `status: "ran"` — the skeleton ships as `unsupported`, which scores 0
   and is expected until the profile is built.

Only then begin descending on the eval.

## Target (outer loop)

Metric: migration fidelity, both directions. A seeded v1 database is
migrated by the real CLI path; `state_queries` project the ACTUAL Reborn
stores (never the MigrationReport as truth); contracts diff those
projections against sealed expectations. Missing migrated content starves
the required numerator; forbidden observations (v1 DB mutated, silent row
drops, plaintext secret echo, dry-run writes, dropped LLM-data rows)
halve the case score per violation class; harness errors zero the case.

**Bar: 0.90 on holdout** (brief bar; the lane's 1.00 hard-gate component
is folded in as the required v1-read-only + fully-accounted matchers
present in every case). Score with `harness/score.sh`. A VOID result
means a constraint was violated — find and remove the violation; the
harness will not tell you which it was. Holdout: aggregate-only, max 3
calls per 24 h, audit-logged. Acceptance is measured on holdout
exclusively.

Small-eval warning (verbatim per portfolio COMMON): Per-feature evals are
30–60 dev + 10–15 holdout cases: far below the ~200 enumerability
threshold. The compensating controls are (a) contract-style scoring
(satisfying a behavioral contract usually requires the machinery, unlike
data-lookup evals), (b) probe gap as the memorization gauge, (c) feedback
capped to aggregate + ≤5 worst case ids, (d) holdout answers off-repo.

## Constraints

- Wall-clock budget: **10 h** (lane). Check `harness/status.sh` every
  cycle — elapsed, score history, spend. Watch gain per cycle; a flat
  gradient at high burn means stop.
- Spend ceiling: **$5** LLM/API (lane; `caps.json.spend_ceiling_usd`).
  This loop is fully deterministic — there are NO live cases
  (`eval/dev/cases-live/` intentionally absent) and **expected spend is
  $0**; the ceiling exists only as a backstop. No external service calls.
- Surface (allowlist; everything else denied):
  - Read/write: `crates/**` (primarily `ironclaw_reborn_migration`,
    `ironclaw_reborn_cli`), `src/**` (v1 IS in scope for this lane — but
    for the deletion ledger and reference cleanup ONLY; **no new feature
    behavior in `src/`**, and the migration must only ever READ v1
    stores), `tests/**`, `lfd/cleanup-old-architecture/LOG.md`,
    `tests/integration/lfd/profiles/cleanup-old-architecture.rs` (the ONE
    writable runner file).
  - Read-only: this `goal.md`, `spec.md`, `harness/**`, `eval/**` (inputs
    are readable by design), `lfd/_shared/**`,
    `tests/integration/lfd/**` (except your profile),
    `tests/integration/support/**`.
  - BANNED (detected, VOIDs): reading
    `lfd/cleanup-old-architecture/harness/answers.dev.json`, anything
    under `$LFD_STATE_ROOT/`, any other lane's `lfd/` package.
- Capacity caps (`harness/caps.json`, lint-enforced; base_ref
  `d9eaae2bb`):
  - Dev-seed literal values appearing in `src/**`/`crates/**` diff: **0**.
  - `DELETE FROM` / `DROP TABLE` / `TRUNCATE` additions targeting v1
    stores in migration code: **0** — migration reads v1, never mutates
    it; `state_eq` on the v1 DB digest before/after is a contract in
    every case.
  - `#[ignore]` additions: **0**. Deleted test functions in diff: **0**.
  - Per-case branching on eval-distinctive strings: **0**.
- LLM-data retention invariant (repo-wide): migration must never drop
  LLM output rows — conversation messages, job events/actions. Contracts
  price this as forbidden matchers; a "cleanup" that deletes v1 LLM rows
  is a critical-stop condition, not an optimization.
- `goal.md`, `spec.md`, `harness/`, and `eval/` are read-only during
  optimization. Eval inputs may be read where the harness exposes them;
  eval answers never.

## Cycle protocol

1. Score (dev): `harness/score.sh`. 2. Reflect: run `harness/probe.sh` —
am I generalizing or memorizing? If the probe gap is growing, the next
change must REMOVE an eval-shaped artifact (cap a list, blind a feature,
reject a seed), never add one. 3. Hypothesize: log hypothesis, expected
failure mode, and diagnostic in LOG.md BEFORE changing code.
4. Change (one scoped change; run Stage-0 tests). 5. Log the result.
6. Checkpoint: `git commit -am "cycle <n>: <score>"` — every cycle, gain
or no gain, so the run is bisectable and crash-safe.

## Entropy rules

- Stall rule: if the metric didn't move last cycle, the next attempt must
  be a structural change — same-knob-harder is banned.
- Exploration quota: every 5 cycles, try a structurally different
  approach even if the current one is still inching up.
- Lane rule: if closing a gap uncovers a hidden Reborn representation gap
  (no store to write into), pause and add the Reborn-side seam first —
  do not fake the projection.

## Stop conditions

Bar (0.90) hit on holdout with Stage 0 green · any budget exhausted ·
marginal dev gain < 0.01 for 4 consecutive cycles · a critical
data-loss / secret-leakage / isolation issue is discovered · the scorer
is found invalid and cannot be repaired in budget. On stop: write a
final report in LOG.md — best score, what generalized, what was
abandoned, highest-leverage next steps (including the state of the
deletion ledger if the stretch was reached).
