# Reborn Learning System — "Learn From Mistakes, Never Repeat" (reborn-native)

**Status:** approved for implementation (single PR, sequential Codex gpt-5.5 xhigh; **no PR opened — local review first**)
**Date:** 2026-06-14
**Branch:** `claude/reborn-learning-system` (off `origin/main`)
**Owner:** firat

> This is the **reborn-native collapse** of the original design after a thermo-nuclear review of the
> architecture. It deletes the heavy Hermes-style machinery (per-turn background reflection *run*,
> dedicated run profile, trusted-submit orchestrator, transcript-readback host port, curator worker,
> supersede bookkeeping, a parallel memory model) in favor of: in-turn learning on the **existing**
> memory substrate, **decay-at-read**, and a **lightweight signal-triggered reflection** (one model
> call → deterministic write). Same behavior, far fewer moving parts.

## 1. Goal

Give the reborn agent Hermes-parity "it never makes the same mistake twice": it captures durable
learnings (facts, corrections, preferences, dismissed false-positives) with confidence + recency,
recalls them in later turns, and — when a turn reveals something worth keeping (a user correction or
a failure) — reflects once to persist it, without the user having to re-steer. Anti-poisoning is
first-class: store the *fix*, never the transient failure or a negative capability claim.

Behavioral reference (fold in, don't chase the harness): `nearai/benchmarks`
`datasets/ironclaw/v1/09-learning-system` — confidence-scoring, confidence-decay, dedup-correction,
fp-learning-loop, cross-project, learn-management. We implement equivalents **reborn-native** and
test them reborn-native.

## 2. Build on what already exists (recon, on `main`)

| Capability | Where |
|---|---|
| Memory docs (multi-tenant scoped) + repo | `crates/ironclaw_memory/` (`MemoryDocumentRepository`, filesystem + in-memory backends) |
| Memory tools | `crates/ironclaw_host_runtime/src/first_party_tools/memory.rs` (`builtin.memory_search/read/write/tree`) |
| Memory → prompt injection (recall path) | `crates/ironclaw_host_runtime/src/memory_context.rs` (`ProductionMemoryPromptContextService::load_memory_snippets`) |
| Identity/persona injection | `src/workspace/reborn_identity_context.rs` (`STABLE_IDENTITY_PATHS`, `HostIdentityContextSource`) |
| Write-safety policy | `crates/ironclaw_memory/src/safety.rs` (`PromptWriteSafetyPolicy`) |
| Turn-completed hook | `crates/ironclaw_turns/src/lifecycle.rs` `complete_run` → `TurnCommittedEventObserver`/`TurnEventSink` (`crates/ironclaw_turns/src/events.rs`); subscribe in `crates/ironclaw_reborn_composition` runtime |
| Part-1 failure incident signal | `LoopFailed`/failure categories on branch `claude/naughty-archimedes-d3ac3d` (available once merged; optional input) |
| LLM provider (for the reflection model call) | reborn runtime LLM provider used by the loop's model port |

**Net:** memory + recall + safety + persona injection + a turn-completed hook all exist. We add
(A) learning semantics + persona, and (B) one lightweight reflection service.

## 3. Design

### 3.1 The learning unit
A **learning is a memory document** (existing model — *no parallel `learnings/` format unless the
existing doc metadata genuinely can't carry these fields*) keyed by a **stable key** and carrying:
`confidence` (1–10), `created_at`, `category`, `source`. False-positives are learnings with
`category = fp`.

**Scope:** learnings are isolated by reborn's **existing memory scope `(tenant, user, agent)`** — no
new scoping. Reborn does **not** implement projects (`TurnScope.project_id` is an unused optional
slot, `None` in real runs), so there is **no project/`shared` dimension and no engine-level
cross-project enforcement** in v1. The benchmark's "cross-project" scenarios are prompt-level
behaviors (respect a stated scope, don't echo another context's secrets) — handled by the persona
prompt (WS-2) + secret redaction, not a memory-engine boundary.

- **Correction = overwrite.** Writing a learning with an existing stable key overwrites it. There is
  no second document, therefore **no ghost** — and no `superseded_by` field, no exclusion branch, no
  include-all variant. (Retaining superseded history is a deliberate non-goal for v1; the benchmark
  requires the old value to be *gone* from recall.)
- **Decay is read-time math, not a background job.** `memory_search` ranks by
  `f(confidence, age)`; aged/low-confidence learnings rank lower and are flagged, never deleted.
  No curator worker, no archive dance, no mutation.

### 3.0 Feature gate (A/B)
The whole learning *behavior* sits behind a single env flag, **default off**, so it can be A/B
tested by toggling: `IRONCLAW_LEARNING_ENABLED` (bool; read once via the reborn config layer, not
scattered `std::env::var` calls). When off: the learning persona is NOT injected and the reflection
service is NOT constructed/subscribed — the agent behaves exactly as today. When on: persona +
reflection are active. The WS-1 memory *mechanics* (frontmatter, overwrite-on-key, decay-at-read,
redaction) are always compiled but inert unless a learning is actually written; decay-at-read only
affects documents that carry confidence frontmatter, so non-learning memory ranking is unchanged
either way. (One gate for now; can split persona vs reflection later if A/B needs it.)

### 3.2 Layer A — in-turn learning (core)
When `IRONCLAW_LEARNING_ENABLED` is on, the agent uses the existing `memory_*` tools to
save/overwrite/search/report learnings, driven by a **learning persona** injected into the reborn
system prompt (so baseline behavior — empty per-scenario identity — already assigns confidence,
surfaces staleness, overwrites on correction, tracks FPs, supports `/learn`, and doesn't echo
secrets from another context). Recall is the **existing** memory-snippet injection — no second
injection path.

Minimal `ironclaw_memory` additions: the learning frontmatter fields, decay-at-read ranking, and a
**secret-redaction export** helper (`[REDACTED - sensitive]`). Scoping reuses the existing
`(tenant, user, agent)` memory scope as-is — no new scope code.

### 3.3 Layer B — lightweight reflection (core, reborn-native)
A small **reflection service** invoked by the turn-completed observer (best-effort, never blocks or
delays the user turn):

1. **Cheap deterministic gate** decides whether to reflect at all — default: the turn ended in a
   failure/incident, **or** the latest user message matches a lightweight correction cue. (Blind
   "every turn" cadence is off by default — it's cost with low precision.)
2. If gated in, **one model call** (reflection prompt + the just-finished conversation, which the
   observer already has) returns a **structured decision**: is there a durable learning here, and if
   so its `{key, category, content, confidence}`. The model only *judges/extracts*; it does not get
   a tool loop.
3. **Deterministic apply:** the service writes the learning via the safe memory path
   (`PromptWriteSafetyPolicy`). Overwrite-on-key handles dedup.

This is **not** a reborn "run": no run profile, no trusted-submit, no capability-surface narrowing,
no transcript-readback port. It's a bounded service: one model call + one safe write. Gated by the
same `IRONCLAW_LEARNING_ENABLED` env flag (default off) — when off it is not constructed/subscribed.
The reflection prompt carries the anti-poisoning rules
(never store env-dependent failures, transient errors that a retry fixed, or negative capability
claims; store the fix; declarative facts; user-correction is the top signal) — ported in spirit from
`crates/ironclaw_engine/prompts/mission_*.md`.

### 3.4 Invariants
- Reflection is best-effort and must never block, delay, or fail the user-facing turn.
- Memory writes (in-turn and reflection) go through `PromptWriteSafetyPolicy`; export redacts secrets.
- Scope isolation reuses the existing `(tenant, user, agent)` memory scope; no project dimension
  (reborn projects are unimplemented). Cross-context non-leakage is a persona + redaction behavior.
- Deterministic apply (Rule 5): the model judges/extracts; code performs the write.
- No `.unwrap()`/`.expect()` in prod; both memory backends stay at parity; new wire fields snake_case
  + `#[serde(default)]` + legacy round-trip.

## 4. Workstreams (sequential Codex; crate-scoped)

### WS-1 — Memory learning semantics (`crates/ironclaw_memory` + `…/first_party_tools/memory.rs`)
Learning frontmatter (confidence/created_at/category/key/source; tolerate docs without it);
stable-key overwrite (no-ghost); decay-at-read ranking (confidence × recency, flag-not-delete);
secret-redaction export. Reuse the existing `(tenant, user, agent)` memory scope as-is (no project
dimension). TDD mirroring dedup-correction/*, confidence-decay/*, learn-management/export-sanitizes-secrets.

### WS-2 — Learning persona + `/learn` surface (prompt file + `crates/ironclaw_host_runtime` + `crates/ironclaw_reborn_config`)
`IRONCLAW_LEARNING_ENABLED` env flag (default off) read via the reborn config layer. When on, inject
the learning preamble (prompt file, `include_str!`) as a stable identity candidate (baseline
behavior); when off, inject nothing. `/learn` stats/prune/search/export expressed over the existing
memory tools (add a host helper only if a behavior can't be expressed over the tools). TDD: flag off
⇒ preamble absent; flag on ⇒ present + reborn-tier behavior covering confidence-scoring/*,
learn-management/*, fp-learning-loop/*.

### WS-3 — Lightweight reflection service (`crates/ironclaw_reborn_composition` + `crates/ironclaw_reborn_config` + reflection prompt)
Turn-completed observer (best-effort) → cheap signal gate → one structured model call →
deterministic safe write. Gated by `IRONCLAW_LEARNING_ENABLED` (default off; not constructed/
subscribed when off). Anti-poisoning reflection prompt. TDD: flag off ⇒ no observer/no-op; flag on ⇒
gate fires only on signal; one write on a learnable turn; never blocks the turn (best-effort failure
swallowed + logged at debug).

### WS-4 — Never-repeat E2E + benchmark-equivalent tests + quality gate (NO PR)
Headline E2E: a turn with a user correction (or a failure) → reflection writes a learning → a fresh
turn recalls it and behaves correctly. Plus reborn-native ports of representative `09-learning-system`
scenarios. `cargo fmt`, `cargo clippy --all --tests --all-features` (zero warnings), `cargo test`.
**Leave on the local branch; do not open a PR.**

## 5. Acceptance criteria
1. Baseline agent saves a learning with a confidence score, reports it, recalls it later; old learnings surface as stale (low confidence), never deleted on decay.
2. Correcting a learning makes the old value unreachable via default search (overwrite = no-ghost).
3. A dismissed false-positive is not re-flagged for the same pattern; generalizes only on exact match.
4. Learnings isolated by the existing `(tenant, user, agent)` memory scope; the agent doesn't echo another context's secrets (persona) and `/learn export` redacts secrets.
5. On a learnable turn (correction/failure), the reflection service writes/updates a learning via one model call + deterministic safe write; disabled by config → no-op; never blocks the turn.
6. Never-repeat E2E: correction/failure in turn N → correct behavior in turn N+1 via the recalled learning.
7. Zero clippy warnings; tests green; both memory backends at parity.
8. The entire learning behavior (persona + reflection) is gated by `IRONCLAW_LEARNING_ENABLED`, default off: with the flag unset the agent is byte-for-byte the pre-learning agent (no persona injected, no reflection observer), enabling clean A/B.

## 6. Deleted from the prior design (and why)
- **Per-turn background reflection *run*** + dedicated run profile + trusted-submit orchestrator + transcript-readback port → replaced by a bounded reflection *service* (one model call + safe write); the observer already holds the conversation.
- **Curator worker** (decay/consolidate/archive) → decay is read-time math; dedup is overwrite-on-key.
- **`superseded_by` supersede bookkeeping** → stable-key overwrite.
- **Parallel `learnings/*.md` memory model** → learnings are existing memory docs + a category.
- **Pinned active-learnings prompt section** → reuse existing memory-snippet injection.

## 7. Out of scope (follow-ups)
- **Engine-level project scoping / cross-project sharing** — reborn does not implement projects; `TurnScope.project_id` is an unused optional slot. Revisit if/when reborn ships a real project feature.
- Matching the external benchmark harness (targets the v1 library).
- Superseded-history retention / audit log of overwritten learnings.
- Vector/embedding episodic search; FTS over full conversation history.
- DSPy/GEPA offline skill evolution.
- Skill (procedural) auto-generation/patching via reflection — v1 reflection writes *learnings* only.
