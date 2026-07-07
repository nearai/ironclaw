# Goal: attach correct long-term memory context before action

Source page: https://app.notion.com/p/38729a6526bf81edac69eb004939ecf1

Read `../COMMON.md` first. It is part of this goal.

## Stage 0 - Build to spec (inner loop)

Write `spec.md` for memory retrieval before model/tool action. The spec must define retrieval trigger, scope filters, ranking, stale/conflict suppression, confidence and source handling, prompt envelope, token budget, no-memory fallback, and downstream use.

Retrieval must happen through the product-layer provider boundary and must not attach memory that policy, scope, or relevance rules exclude.

## Target (outer loop)

Optimize retrieval quality:

- 35% relevant memory is retrieved.
- 25% irrelevant, stale, conflicting, or cross-scope memory is suppressed.
- 20% attached context is compact, source-attributed, and policy-compliant.
- 10% no-memory fallback works without hallucinated memory.
- 10% downstream task quality improves versus no retrieval.

Bar: at least 0.88 retrieval F1 on holdout, cross-scope injection rate equals 0, attached context stays within token cap.

## Eval design

Create 100 dev and 250 holdout tasks over synthetic memory stores. Inputs include user/project/tenant scope, task query, memory store with relevant, stale, conflicting, irrelevant, and cross-scope entries, and downstream scripted task.

Answers include retrieved ids, suppressed ids, attached prompt envelope, source attribution, no-memory behavior, and downstream result.

## Harness design

Run retrieval before a fake or planned model call and inspect the attached prompt envelope. Compare downstream scripted task outcome with and without retrieval.

`harness/score.sh` must:

- Seed scoped memory stores through provider boundary.
- Run retrieval/ranking/filtering.
- Inspect attached context and token budget.
- Run downstream scripted action or fake model route.
- Score false positives and false negatives separately.
- Run probe variants with paraphrases, changed dates, renamed entities, and tenant swaps.

## Constraints

- Wall-clock budget: 14 hours.
- Spend ceilings: $20 LLM/API spend; no live private memories.
- Surface allowlist: memory provider/search, retrieval/ranking/filtering, prompt envelope, turn coordination, product workflow, benchmark fixtures, tests, and docs.
- Capacity caps: attached memory at most 1,200 tokens per turn; retrieval result count capped at 8 unless spec and scorer change together.
- Do not attach raw large histories as summaries to bypass the cap.

## Cycle protocol

Follow the common cycle protocol. Each cycle must include one relevant retrieval, one suppression case, and one no-memory fallback. Log whether the change affects ranking, filtering, compaction, or prompt envelope.

## Entropy rules

- Rotate retrieval failure modes every 3 cycles: missing relevant, stale, conflicting, irrelevant, cross-scope, no-memory, and downstream use.
- If recall rises while precision falls, target filtering before ranking expansion.
- If downstream task improves only by exceeding token budget, the next cycle must improve compaction or selection.

## Cheat audit

Lane-specific cheap wins to block:

1. Always attach top N memories; precision and token cap fail.
2. Retrieve by title keyword only; paraphrase probes fail.
3. Ignore scope filters; tenant-swap probes fail.
4. Hide cross-scope memory in summaries; source/scope scan fails.
5. Ignore stale/conflicting memories; suppression cases fail.
6. Claim retrieval success from final answer only; envelope ids are scored.
7. Degrade no-retrieval baseline; harness owns baseline.
8. Hardcode memory ids; probe randomizes ids.
9. Attach no source attribution; envelope score fails.
10. Skip no-memory fallback; no-memory cases fail.

## Stop conditions

Stop when holdout F1 is at least 0.88 with zero cross-scope injection and Stage 0 tests green, budget is exhausted, score is flat for 3 cycles, or attached context violates policy or token caps.

