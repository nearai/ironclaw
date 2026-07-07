# Goal: deliver the Reborn memory platform as product-layer capability

Source page: https://app.notion.com/p/38729a6526bf81d3bdc7d339a08f6021

Read `../COMMON.md` first. It is part of this goal.

## Stage 0 - Build to spec (inner loop)

Write `spec.md` as the umbrella design for the memory platform. Read the local memory design memo if present at `docs/reborn/2026-06-22-memory-as-product-layer.md`, and inspect the referenced GitHub issue/PRs when available. If any source is unavailable, record the missing source and substitute observed repo evidence in `LOG.md`.

The platform spec must cover four pillars:

- Memory placement as product/provider boundary.
- Event-driven self-learning write pipeline.
- Bounded long-term retrieval pipeline.
- Benchmarks and regression evaluation.

Native memory remains the default provider, but host/admin policy must own provider authorization and constraints.

## Target (outer loop)

Optimize platform readiness:

- 25% provider/capability boundary exists and is policy-mediated.
- 25% event-driven write pipeline records typed, scoped, validated memory.
- 25% bounded retrieval pipeline attaches relevant, source-attributed context.
- 25% benchmark coverage catches known-bad memory implementations.

Bar: all four pillar holdout bars pass, and the meta-score is at least 0.92. Any pillar security failure makes the platform score zero.

## Eval design

Use the pillar evals from sections 16, 17, 18, and 19. Add 30 dev and 60 holdout cross-pillar scenarios where a memory is written in one session, retrieved in a later session, and measured by benchmark regression logic.

Answers include provider decision, write artifact, retrieval envelope, benchmark classification, audit events, and policy decisions.

## Harness design

Build a meta-scorer that runs:

- Section 16 placement scorer.
- Section 17 write-pipeline scorer.
- Section 18 retrieval-pipeline scorer.
- Section 19 benchmark scorer.
- Cross-pillar longitudinal scenarios.

`harness/score.sh` must fail the platform if any pillar reports a hard-gate security failure, even if the weighted aggregate is high.

## Constraints

- Wall-clock budget: 16 hours.
- Spend ceilings: $25 LLM/API spend; no live private user memory.
- Surface allowlist: `ironclaw_memory`, `ironclaw_memory_native`, product workflow, turn/context, capabilities, events/audit, Reborn composition, benchmark scripts, tests, and docs.
- Capacity caps: shared caps; cross-pillar seed scenarios capped at 30 dev-visible cases.
- Do not fork a parallel memory implementation to make the umbrella score pass. Pillar lanes own concrete behavior.

## Cycle protocol

Follow the common cycle protocol. Each cycle must identify which pillar should move. After any pillar score improves, run the cross-pillar scenario subset before commit.

## Entropy rules

- Rotate pillars every 4 cycles unless a security hard gate is failing.
- If one pillar improves by weakening another, revert or redesign at the boundary before continuing.
- If meta-score is flat, the next cycle must target the lowest pillar rather than the most convenient one.

## Cheat audit

Lane-specific cheap wins to block:

1. Build a new memory stack outside product/provider boundary; dependency lint fails.
2. Make native memory special-cased in product code; fake provider probe fails.
3. Score pillars independently but break write-to-retrieval flow; cross-pillar scenarios fail.
4. Ignore benchmark lane; meta-score requires it.
5. Claim source issue/PR evidence without reading or recording missing sources; `LOG.md` check fails.
6. Use raw transcript memory to boost recall; write lane lint fails.
7. Retrieve everything to boost downstream tasks; retrieval lane precision fails.
8. Hide policy failures behind weighted average; hard gates zero the platform score.
9. Hardcode cross-pillar scenario ids; probe randomizes ids and wording.
10. Skip docs/parity updates after status change; docs check fails.

## Stop conditions

Stop when every pillar hits its holdout bar and meta-score is at least 0.92, budget is exhausted, score is flat for 3 cycles, or any memory provider can bypass host/admin policy.

