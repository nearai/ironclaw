# Goal: make agents improve from corrections, recoveries, and repeated workflows

Source page: https://app.notion.com/p/36e29a6526bf8083af81d838c140d619

Read `../COMMON.md` first. It is part of this goal.

## Stage 0 - Build to spec (inner loop)

Write `spec.md` as an umbrella acceptance spec over memory write, memory retrieval, and memory benchmark lanes. Do not implement a separate self-learning stack. The spec must map self-learning signals to the product-layer memory and skill mechanisms that own them.

The spec must cover:

- Explicit "remember this" requests.
- User corrections and style preferences.
- Failure recovery lessons.
- Repeated workflow extraction.
- Successful completion patterns.
- No-op chatter and unsafe over-learning.
- Scope, provenance, confidence, and source attribution.

Hermes-inspired ideas from the page may inform the eval, but IronClaw architecture and policy boundaries are authoritative.

## Target (outer loop)

Optimize compound learning score:

- 30% useful lessons are written with correct type, source, confidence, scope, and policy decision.
- 25% useful lessons are retrieved or activated later when relevant.
- 20% repeated mistakes decrease across repeated scenarios.
- 15% repeated workflows become reusable without becoming eval-shaped lookup tables.
- 10% unsafe over-learning, prompt bloat, and cross-scope injection remain controlled.

Bar: at least 0.90 aggregate across holdout, zero policy-blocked writes persisted, zero cross-scope retrieval.

## Eval design

Create 80 dev and 180 holdout longitudinal scenarios. Each scenario has at least two sessions: a learning event and a later opportunity to use or suppress it. Include corrections, preference style, project fact, mistake recovery, repeated command sequence, successful workflow, irrelevant chatter, sensitive content, and prompt-injection attempts.

Answers include expected memory/skill artifact, whether it should be written, when it should be retrieved, downstream behavior delta, and safety decision.

## Harness design

Use the concrete memory lanes as sub-scorers where possible:

- Section 17 write-pipeline score for whether the right artifact is recorded.
- Section 18 retrieval-pipeline score for whether the right artifact is attached later.
- Section 19 benchmark score for regression coverage.

The umbrella `harness/score.sh` must also run longitudinal task pairs to show behavior improves, not only artifacts exist.

## Constraints

- Wall-clock budget: 12 hours.
- Spend ceilings: $20 LLM/API spend; no live private user history.
- Surface allowlist: memory provider boundary, self-learning write/retrieval lanes, product workflow, turn/event projections, skill-learning artifacts, benchmark fixtures, tests, and docs.
- Capacity caps: shared caps; learned workflow examples capped at 12 visible examples.
- No raw transcript dumps as "learning". Every durable artifact must be typed, scoped, source-attributed, and policy-validated.

## Cycle protocol

Follow the common cycle protocol. Each cycle must include one write case, one later retrieval/use case, and one negative suppression case. If one sub-lane regresses, stop umbrella optimization and repair the sub-lane.

## Entropy rules

- Rotate signal types every 3 cycles: explicit remember, correction, failure recovery, repeated workflow, success pattern, and suppression.
- If repeated-workflow success rises by adding examples, the next cycle must improve abstraction or validation rather than adding examples.
- If context size grows while task quality is flat, target retrieval compaction before more write coverage.

## Cheat audit

Lane-specific cheap wins to block:

1. Write every message; precision and policy cases fail.
2. Retrieve every memory; retrieval precision and token cap fail.
3. Store raw transcripts as memory; lint rejects untyped dumps.
4. Learn prompt-injection instructions; safety cases fail.
5. Treat one repeated example as a workflow; holdout requires generalization.
6. Claim improvement from artifact creation only; longitudinal task score is required.
7. Hardcode user preferences from eval; probe swaps users and styles.
8. Ignore source/provenance; metadata score fails.
9. Use global memory across projects; cross-scope probes fail.
10. Optimize write and break retrieval; umbrella scorer includes both.

## Stop conditions

Stop when aggregate holdout is at least 0.90 with zero policy or scope failures and Stage 0 tests green, budget is exhausted, score is flat for 3 cycles, or learning artifacts create unsafe over-injection or cross-scope leakage.

