# LFD Brief: long-term-memory — Long-term memory

**State**: built (`ironclaw_memory` + `ironclaw_memory_native` +
embeddings) — this LFD is retrieval QUALITY + durability hardening, the
roadmap's August item. **Bar**: 0.95 holdout. **Profile**: `memory`.

## Outcome

Memory that is precise as well as recallful: turn-context assembly retrieves
the RIGHT docs (and not the wrong ones) within envelope budget; writes from
turns persist with versioning; typed docs behave per contract; nothing is
ever deleted; state survives restarts; cross-thread and cross-project
scoping holds.

## Spec sources

- `contracts/memory.md`, `contracts/memory-profiles.md`,
  `contracts/storage-placement.md`
- `crates/ironclaw_memory/`, `crates/ironclaw_memory_native/` (chunking,
  indexing, search, prompt-context assembly), `crates/ironclaw_embeddings/`
- Root CLAUDE.md "LLM data is never deleted" invariant
- v1 `src/workspace/README.md` (hybrid FTS+vector RRF — parity reference)

## Stage 0 inner suite

`tests/integration/group_memory/` + `ironclaw_memory*` crate tests +
`tests/reborn_qa_doc_grounding.rs`. Green every cycle.

## Eval themes (dev ~40 / holdout ~14)

1. Retrieval precision/recall (14): seeded doc corpus (12–20 docs per
   case, mixed topics) + query turn → envelope must contain the K relevant
   docs' content (required reply/state contracts per doc) AND not contain
   the distractor docs (forbidden matchers per distractor). Both directions
   priced per case. Probe renames entities in docs+queries consistently.
2. Write path (6): model writes memory via tool → doc persisted with type,
   scope, timestamp (state query); subsequent read round-trips.
3. Versioning + retention (6): overwrite/update → new version, old version
   retrievable (state query on versions; forbidden: deletion/truncation
   events, forbidden: version count decrease).
4. Typed docs (5): Summary/Lesson/Skill/Issue/Spec/Note behaviors — type
   filters honored in retrieval (state_pred).
5. Restart durability (5): index + docs survive restart (group reload);
   re-index produces same retrieval result (state_eq).
6. Scoping (4): project A docs never retrieved into project B context
   (forbidden), same for tenant boundaries.

## Feature-specific cheats → fences

- **Return-everything retrieval** → distractor forbidden matchers +
  envelope budget state_pred (assembled context ≤ configured token
  budget); precision priced per case.
- **Verbatim query indexing** (memorize dev queries) → probes rename
  entities in docs and queries; holdout uses new corpus domains.
- **Stuff summaries with full corpus** → budget contract + distractor
  forbidden matchers fire on summary content too (scan covers whole
  envelope).
- **Fake versioning** (new version, old data gone) → old-version
  content state query must round-trip its content, not just count.
- **Embedding-provider stub returning constant vectors** (degrades to FTS
  but dev might still pass) → cases include synonym-retrieval scenarios
  where lexical match fails and only semantic match succeeds (mock
  embedding provider with deterministic per-token vectors is part of the
  pinned profile support, not agent-writable).

## caps.json extras

Dev corpus distinctive literals in `crates/**` diff: max 0. Retrieval-K
hardcoding: pattern `top_k\s*=\s*\d+` additions outside config plumbing,
max 2.

## Live mode

3 live cases: real model + real embedding provider (spend-capped) on one
seeded corpus → recall contracts only (structural); the deterministic dev
suite carries precision.
