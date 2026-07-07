# Goal: store useful memory from authorized turn events

Source page: https://app.notion.com/p/38729a6526bf81c4afb6d1016c5c85a4

Read `../COMMON.md` first. It is part of this goal.

## Stage 0 - Build to spec (inner loop)

Write `spec.md` for the self-learning write pipeline. The pipeline observes authorized turn events and writes durable memory only when there is a useful, allowed learning signal.

The spec must define signal classes, typed artifact schema, validation, idempotency, source/provenance, confidence, scope, policy decision, audit, and no-write cases.

Signal classes must include explicit remember requests, corrections, failures, recoveries, repeated workflows, successes, preferences, project facts, and no-op chatter.

## Target (outer loop)

Optimize write-pipeline F1:

- Recall: eligible explicit remember requests, corrections, recoveries, repeated workflow lessons, successful completion lessons, preferences, and project facts are written.
- Precision: chatter, untrusted instructions, sensitive disallowed content, cross-scope facts, and unsafe prompt-injection are not written.
- Metadata: source, confidence, provenance, scope, idempotency key, type, and policy decision are correct.

Bar: at least 0.90 F1 on holdout, zero policy-blocked writes persisted, zero untyped raw transcript dumps.

## Eval design

Create 120 dev and 300 holdout turn traces with labeled memory artifacts. Inputs include explicit remember, correction, mistake recovery, repeated workflow, successful completion, user style preference, project fact, no-op turn, sensitive content, cross-tenant content, and prompt injection.

Answers include whether to write, artifact type, normalized content, scope, confidence, source ids, idempotency key, policy decision, and audit event.

## Harness design

Feed recorded or fake turn events through the write classifier and product-layer memory provider boundary. Score structured artifacts and audit events. Dev misses reveal at most five cases.

`harness/score.sh` must:

- Construct authorized and unauthorized turn event envelopes.
- Run classifier/extractor under schema validation.
- Pass accepted artifacts through provider boundary and policy.
- Verify no-write cases leave no persisted artifact.
- Verify idempotency across duplicate events.
- Scan persisted artifacts for raw transcript dumps.

## Constraints

- Wall-clock budget: 14 hours.
- Spend ceilings: $25 LLM/API spend for extraction/eval; no live private transcripts.
- Surface allowlist: turn/event projections, memory provider boundary, policy/safety validation, skill-learning artifacts, product workflow, tests, benchmark fixtures, and docs.
- Capacity caps: shared caps; prompt examples capped at 12; artifact type enum is closed unless the spec updates tests.
- Classifier output is not authority until schema, policy, and scope validation pass.

## Cycle protocol

Follow the common cycle protocol. Each cycle must include one write-positive, one no-write negative, and one duplicate/idempotency case. Before commit, inspect artifact schema changes against retrieval lane compatibility.

## Entropy rules

- Rotate signal classes every 3 cycles.
- If recall improves but precision drops, all further cycles target precision until policy-blocked writes are zero.
- If gains come from larger examples, the next cycle must improve schema, policy, or feature extraction instead.

## Cheat audit

Lane-specific cheap wins to block:

1. Write every message; precision cases fail.
2. Write raw transcripts; lint rejects untyped dumps.
3. Ignore policy-blocked content; hard gate fails.
4. Drop scope or provenance; metadata score fails.
5. Treat duplicate events as new learning; idempotency cases fail.
6. Learn prompt-injection instructions; safety cases fail.
7. Hardcode trace phrases; probe paraphrases learning events.
8. Store memory outside provider boundary; placement checks fail.
9. Use classifier confidence as permission; policy validation is scored.
10. Optimize for write count; F1 and no-write cases penalize false positives.

## Stop conditions

Stop when holdout F1 is at least 0.90 with zero policy-blocked writes and Stage 0 tests green, budget is exhausted, score is flat for 3 cycles, or any unsafe/cross-scope artifact is persisted.

