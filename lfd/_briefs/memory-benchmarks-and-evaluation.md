# LFD Brief: memory-benchmarks-and-evaluation — Memory benchmarks & evaluation

**State**: greenfield benchmark infrastructure (product eval harness,
reusable in CI) over the built memory stack + Reborn QA fixtures. It does NOT
replace `lfd/_shared`: its own calibration runs UNDER the shared scorer as
meta-contracts. **Bar**: benchmark meta-quality ≥ 0.90 holdout AND
calibration passes (known-good accepted, every known-bad rejected for its
intended reason). **Profile**: `memory_bench`.

## Outcome

A memory benchmark that isolates write quality, retrieval quality,
negative-safety, trend reporting, and Reborn-QA whole-path integration.
Before scoring any live implementation it runs calibration: the known-good
reference passes above the bar and every seeded known-bad variant fails for
its intended reason class. Verdicts derive from structured artifact / id /
scope diffs — never an LLM judge as the primary metric.

## META-EVAL shape (per ADDENDA)

The benchmark's verdict on each seeded implementation variant is the
**outcome**; the sealed expected verdicts are the **contract**. Launch set:
7 seeded variants (good, write-everything, write-nothing, retrieve-everything,
retrieve-nothing, cross-scope-leak, stale-injection, no-attribution) × 5–6
scenario families ≈ 40 dev / 14 holdout meta-cases. Goal's 220 dev / 560
holdout are designer GROWTH TARGETS.

## Spec sources

- Goal §known-good/known-bad calibration; the pillar packages (16/17/18) as
  the product paths under test
- `scripts/reborn_qa_matrix/` (`run_hermetic_qa.py`, `report_coverage.py`)
  and `tests/fixtures/llm_traces/reborn_qa/` (whole-path traces)
- `scripts/ci/check-reborn-qa-fixtures.sh` (the fixture leak-scan pattern set
  the negative-safety verdicts reuse)
- `crates/ironclaw_memory_native/src/contract_tests.rs` (the trait-level
  scaffolding pattern this benchmark mirrors — one suite, every impl)

## Stage 0 inner suite

Benchmark harness unit tests + one `tests/reborn_qa_doc_grounding.rs` trace
replay. Green every cycle.

## Eval themes (dev ~40 / holdout ~14 meta-cases)

Each meta-case = (scenario family, implementation variant, sealed expected
verdict). Each cycle runs calibration + ≥1 live product-memory path.

1. Write-quality meta (10): the benchmark must REJECT write-everything
   (precision) and write-nothing (recall) and ACCEPT good — verdict AND
   reason class checked (`verdict == reject`, `reason ∈ {precision,recall}`).
2. Retrieval-quality meta (10): rejects retrieve-everything (token/precision)
   and retrieve-nothing (recall); accepts good.
3. Negative-safety meta (10): rejects cross-scope-leak / stale-injection /
   no-attribution for the leak / stale / attribution reason; hard-gated.
4. Calibration gate (6): known-good passes above bar; a live-impl score is
   emitted ONLY if calibration passes (meta-contract: a should-pass variant
   passes, a should-fail variant fails-for-reason).
5. Trend + whole-path (4): the regression trend is stable across cycles
   against a FIXED baseline id, and one `reborn_qa` whole-path memory trace
   replays with a verdict matching the sealed expectation.

Cross-ref: this lane calibrates the benchmark; concrete write/retrieval/
placement behavior is **scored in lanes 16/17/18**. Bench never re-scores
their product contracts — it scores whether the benchmark CATCHES their
known-bad variants.

## Feature-specific cheats → fences

- **Let known-bad variants pass (hollow calibration)** → each known-bad
  meta-case REQUIRES `verdict == reject` WITH the intended reason class (not
  a bare reject); a benchmark that passes a known-bad fails its meta-case.
- **Encode the expected verdict in fixture names** → `probe.sh` randomizes
  fixture filenames + memory ids + wording + tenant/project labels; lint:
  visible benchmark fixture names cannot contain verdict tokens
  (pattern scan = 0).
- **Use an LLM judge as the primary scorer** → the verdict must carry
  structured evidence (artifact diff / retrieval-id diff / scope-policy
  check); forbidden matcher fires if the verdict is justified only by an
  `llm_judge` field (mechanical evidence-field check).
- **Score only helper functions, skip whole-path** → theme 5 REQUIRES a
  `reborn_qa` whole-path trace replay verdict; a helper-only benchmark misses
  it.
- **Overfit benchmark ids/wording** → probe randomizes ids/wording;
  probe-gap gauge; dev variant/scenario literals in diff = 0.
- **Report trend without a stable baseline** → trend cases require a pinned
  baseline id (`state_pred`); a moving baseline fails.

## caps.json extras

Visible benchmark fixture names cannot encode verdicts (name-pattern lint =
0); no primary LLM-judge scorer (`llm_judge`-as-sole-evidence lint = 0); dev
variant/scenario literals in benchmark diff = 0; known-bad variant enum
locked at 7 (variant additions require spec + scorer to change together).

## Live mode

No live private data (goal). 2 live cases: run the benchmark's calibration
against the live product memory path (real model, spend-capped) →
structural contract that calibration completes and the good / known-bad
verdicts match the sealed expectations; benchmark meta-quality is carried by
the deterministic variant suite. Spend ceiling $15.
