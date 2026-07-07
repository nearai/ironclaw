# Goal: build memory benchmarks that catch useful-memory and unsafe-memory regressions

Source page: https://app.notion.com/p/38729a6526bf81d8a784fdeac648e7f3

Read `../COMMON.md` first. It is part of this goal.

## Stage 0 - Build to spec (inner loop)

Write `spec.md` for memory benchmark suites. The benchmark must isolate write quality, retrieval quality, negative safety cases, trend reporting, and integration with Reborn QA fixtures.

The spec must define known-good and known-bad calibration implementations before live scoring. Known-bad variants must include write-everything, write-nothing, retrieve-everything, retrieve-nothing, cross-scope leak, stale-memory injection, and no-source-attribution.

## Target (outer loop)

Optimize benchmark quality score:

- 30% write cases isolate whether the right memory is recorded.
- 30% retrieval cases isolate whether the right memory is retrieved and attached.
- 20% negative safety cases catch irrelevant, stale, cross-scope, sensitive, or policy-blocked memory.
- 10% regression trend reporting is useful and stable across cycles.
- 10% integration with Reborn QA fixtures protects whole-path behavior.

Bar: benchmark harness rejects all seeded known-bad implementations, accepts the known-good reference, and scores at least 0.90 on benchmark meta-quality holdout.

## Eval design

Create benchmark meta-eval fixtures rather than only product fixtures. Inputs include memory tasks, expected benchmark verdict, and implementation variant under test. Holdout answers live outside repo.

Minimum set:

- 80 dev and 180 holdout write-quality cases.
- 80 dev and 180 holdout retrieval-quality cases.
- 60 dev and 140 holdout negative safety cases.
- 20 dev and 60 holdout whole-path Reborn QA memory traces.

## Harness design

`harness/score.sh` must run calibration before scoring a live implementation:

- Known-good implementation must pass above the acceptance bar.
- Every known-bad implementation must fail for the intended reason class.
- Live implementation score is reported only if calibration passes.
- Holdout scoring returns aggregate-only and does not expose failing case names.

`harness/probe.sh` must randomize memory ids, wording, dates, tenant/project labels, and fixture filenames to detect overfitting.

## Constraints

- Wall-clock budget: 14 hours.
- Spend ceilings: $15 LLM/API spend; no live private data.
- Surface allowlist: memory benchmark harnesses, `tests/fixtures/llm_traces/reborn_qa/` where applicable, `scripts/reborn_qa_matrix`, memory crate contract tests, integration tests, and docs.
- Capacity caps: shared caps; visible benchmark fixture names cannot encode expected verdicts.
- Do not rely on an LLM judge as the primary scorer. Use structured artifact diffs, retrieval id diffs, scope/policy checks, and calibrated downstream outcomes.

## Cycle protocol

Follow the common cycle protocol. Each cycle must run calibration plus at least one live product-memory path. If calibration fails, do not score live changes until the benchmark is repaired.

## Entropy rules

- Rotate benchmark dimensions every 3 cycles: write, retrieval, safety, trend, and whole-path integration.
- If known-bad variants start passing, the next cycle must strengthen the benchmark before any product work.
- If the benchmark becomes flaky, fix determinism before expanding case count.

## Cheat audit

Lane-specific cheap wins to block:

1. Score only helper functions; whole-path QA traces are required.
2. Let known-bad implementations pass; calibration gates live scoring.
3. Encode expected verdict in fixture names; probe randomizes names and lint checks patterns.
4. Use LLM judge approval as primary metric; structured diffs are required.
5. Overfit benchmark ids; probe randomizes ids and wording.
6. Skip negative cases; safety is 20% and hard-gated for leaks.
7. Hide cross-scope leaks in summaries; scope/source scans catch them.
8. Accept retrieve-everything as high recall; precision and token caps fail.
9. Accept write-everything as high recall; precision and policy cases fail.
10. Report trend without stable baseline; regression reporting requires fixed baseline and calibration.

## Stop conditions

Stop when calibration passes, benchmark meta-quality holdout is at least 0.90, and Stage 0 tests are green; or when budget is exhausted, score is flat for 3 cycles, calibration cannot be stabilized, or benchmark feedback leaks holdout answers.

