# LFD Brief: long-term-memory-retrieval-pipeline — Long-term memory retrieval

**State**: built retrieval substrate (`ironclaw_memory_native` chunking /
indexing / RRF search + `ironclaw_embeddings`); this LFD is retrieval
precision/recall + envelope discipline before a model/tool action. **Bar**:
0.88 retrieval F1 holdout, cross-scope injection rate == 0, attached context
within the token cap. **Profile**: `memory_retrieval`.

## Outcome

Before an action, retrieval attaches the RIGHT docs and not the wrong ones,
inside a bounded, source-attributed prompt envelope. Scope/policy/relevance
filters exclude cross-scope / stale / conflicting / irrelevant entries;
semantic (not merely lexical) matches surface; the no-memory fallback
attaches nothing (no hallucinated memory); retrieval routes through the
product-layer provider boundary.

## Spec sources

- `docs/reborn/contracts/memory.md` (§5.2 search service — RRF default,
  identity-doc filtering; §8 multi-scope read precedence)
- `docs/plans/2026-06-23-hermes-style-context-management.md` (§4.2 volatile-
  last envelope tiers + token budget, §4.3 result-handle / bounded growth)
- `crates/ironclaw_memory_native/src/search.rs` (`FusionStrategy`,
  `MAX_LIMIT`), `indexer.rs`, `embedding.rs`;
  `crates/ironclaw_embeddings/src/mock.rs` (the **pinned deterministic
  embedding fake** the semantic cases ride on)
- `crates/ironclaw_agent_loop/src/strategies/compaction.rs` (envelope budget
  enforcement). Descends from `lfd/_briefs/long-term-memory.md` retrieval
  themes.

## Stage 0 inner suite

`tests/integration/group_memory/` retrieval cases
(`scenario_memory_search_finds_seeded`) + `tests/reborn_qa_doc_grounding.rs`.
Green every cycle.

## Eval themes (dev ~40 / holdout ~14)

Goal's 100 dev / 250 holdout are designer GROWTH TARGETS. Each cycle
includes ≥1 relevant retrieval, ≥1 suppression, ≥1 no-memory case.

1. Per-doc precision/recall (14): seeded corpus (12–20 docs, mixed topics) +
   a query turn → the envelope MUST contain each relevant doc's content
   (per-doc required reply/state matcher) AND MUST NOT contain each
   distractor (per-doc forbidden matcher). Both directions priced per doc.
2. Suppression: stale / conflicting / irrelevant / cross-scope (10) →
   excluded from the envelope (forbidden per suppressed id); cross-scope
   injection rate hard-gated to 0 (tenant/project swap cases).
3. Envelope discipline (6): attached context ≤ token cap
   (`state_pred` on envelope token count ≤ 1200), source-attributed (each
   snippet carries a source id), policy-compliant.
4. Semantic-vs-lexical (5): paraphrase/synonym queries where lexical match
   fails and only the embedding path succeeds → the relevant doc is still
   retrieved. The pinned deterministic embedding fake makes the semantic
   path reproducible and the ONLY route to a pass.
5. No-memory fallback (5): empty/irrelevant store → the envelope attaches NO
   memory and no hallucinated content (forbidden: any memory snippet;
   required: fallback-behavior state).

Cross-ref: which docs EXIST / are typed / versioned is **scored in
self-learning-write-pipeline**; provider-boundary parity is **scored in
memory-placement**. Type-filter honored on read is scored HERE
(`state_pred`). The retrieval-count cap is scorer-side (only the designer
changes it, per ADDENDA lane 18).

## Feature-specific cheats → fences

- **Always attach top-N memories** → per-distractor forbidden matchers +
  envelope token-budget `state_pred`; precision priced per case; retrieval
  result count ≤ 8 enforced scorer-side (caps).
- **Retrieve by title/keyword only (lexical shortcut)** → the
  semantic-vs-lexical cases where lexical fails; `probe.sh` paraphrases
  queries + renames entities; the pinned deterministic embedding fake makes
  the semantic path the only way through.
- **Ignore scope filters / hide cross-scope in summaries** → tenant-swap
  probe + a source/scope scan over the WHOLE envelope (summaries scanned
  too); cross-scope injection hard-gated to 0.
- **Claim retrieval success from the final answer only** → envelope ids are
  scored (state query on attached-context ids), not the reply text.
- **Attach no source attribution** → each envelope snippet requires a source
  id (`state_pred`); missing attribution fails theme 3.
- **Bypass the token cap via a giant summary** → envelope token-count
  `state_pred` + a raw-history-as-summary forbidden scan.

## caps.json extras

Retrieval result count ≤ 8 and attached ≤ 1200 tokens/turn, enforced
scorer-side (not agent-tunable); `top_k\s*=\s*\d+` additions outside config
plumbing ≤ 2; dev corpus distinctive literals in `crates/**` diff = 0.

## Live mode

No live private memories (goal). 3 live cases: real model + real embedding
provider (spend-capped) on one seeded corpus → recall + scope-suppression
structural contracts only; the deterministic dev suite carries precision.
Spend ceiling $20.
