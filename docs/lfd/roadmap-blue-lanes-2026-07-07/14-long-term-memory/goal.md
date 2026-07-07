# Goal: split resident short-term memory from retrievable long-term memory

Source page: https://app.notion.com/p/37929a6526bf8060ae47ea982bb4392a

Read `../COMMON.md` first. It is part of this goal.

## Stage 0 - Build to spec (inner loop)

Write `spec.md` as product acceptance framing for long-term memory. This lane should not build a separate memory system; it should drive and verify the Reborn memory platform lanes.

The spec must define:

- Resident memory: hard-capped facts needed every turn.
- Long-term memory: retrievable history or learned artifacts searched only when relevant.
- Source attribution, scope, confidence, recency, conflict handling, and stale suppression.
- Context budget and prompt-payload format.
- No-memory fallback and negative cases.

## Target (outer loop)

Optimize long-term memory usefulness:

- 30% relevant long-term facts are retrieved when needed.
- 20% resident memory stays small and stable.
- 20% irrelevant, stale, conflicting, or cross-scope memories are suppressed.
- 20% repeated-question and repeated-mistake rates decrease in longitudinal tasks.
- 10% prompt context budget and source attribution are respected.

Bar: at least 0.90 holdout, cross-scope injection rate equals 0, resident memory remains under the lane cap.

## Eval design

Create 80 dev and 180 holdout memory scenarios. Each scenario has a memory store, a current task, expected resident memory, expected long-term retrieval set, and downstream task outcome. Include months-old facts, recent corrections, stale facts, conflicting facts, same entity in different projects, and no-memory tasks.

Answers include retrieved ids, suppressed ids, resident-memory contents, attached prompt envelope, and downstream behavior.

## Harness design

Use synthetic memory stores plus recorded-style turn traces. The scorer must compare retrieval quality and prompt envelope content, not just final answer quality.

`harness/score.sh` must:

- Load scoped resident and long-term memories.
- Run the product retrieval path before the model call or fake model call.
- Inspect attached context and source attribution.
- Run no-memory and stale-memory suppression probes.
- Measure downstream task delta against a no-retrieval baseline.

## Constraints

- Wall-clock budget: 12 hours.
- Spend ceilings: $15 LLM/API spend; no live private histories.
- Surface allowlist: Reborn memory platform lanes, memory provider/search, prompt envelope, turn coordination, product workflow, benchmark fixtures, tests, and docs.
- Capacity caps: resident memory at most 500 tokens per subject by default; attached long-term memory at most 1,200 tokens per turn.
- Do not preserve months of memory by putting it all in every prompt.

## Cycle protocol

Follow the common cycle protocol. Each cycle must include a relevant retrieval case and a suppression/no-memory case. Measure prompt-token budget on every cycle.

## Entropy rules

- Rotate memory failure modes every 3 cycles: stale, conflicting, cross-scope, irrelevant, and missing relevant.
- If downstream quality improves only by adding more context, the next cycle must improve ranking or compaction.
- If resident memory grows without improving recurring tasks, shrink or reclassify resident facts.

## Cheat audit

Lane-specific cheap wins to block:

1. Attach all memories; token cap and precision fail.
2. Move long-term history into resident memory; resident cap fails.
3. Retrieve by keyword only; paraphrase probes fail.
4. Ignore stale/conflicting facts; suppression cases fail.
5. Drop source attribution; prompt envelope score fails.
6. Use final answer quality only; envelope inspection is required.
7. Hardcode memory ids; probe randomizes ids and wording.
8. Hide cross-scope memory in summaries; context scan catches source ids/scopes.
9. Optimize no-memory baseline by degrading it; baseline is fixed by harness.
10. Treat retrieval miss as harmless; repeated-question/mistake delta is scored.

## Stop conditions

Stop when holdout is at least 0.90 with zero cross-scope injection and green Stage 0 tests, budget is exhausted, score is flat for 3 cycles, or memory context exceeds caps without a human-approved spec change.

