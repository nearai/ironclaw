# Reflection-fork convergence: declarative learnings + procedural skills

**Status:** draft for discussion · **Date:** 2026-06-17
**Reconciles:** learning stack #4937→#4938→#4975→#4994 (declarative) · #5061 skill extraction/evolution (procedural) · #2590 SkillClaw (skill direction)

## 1. Problem

Two independent efforts implement halves of the same Hermes-style "learn from
experience" loop, and both hook the **same** turn-completed seam in the Reborn
runtime. Left uncoordinated they become two parallel post-turn pipelines with
divergent gates, flags, and cost — the opposite of the single "reflection fork"
the learning design intended.

| | Declarative (learning stack) | Procedural (#5061) |
|---|---|---|
| Learns from | failures / user corrections | substantive successes |
| Artifact | memory doc (frontmatter learning) | `SKILL.md` |
| Persist | overwrite-on-key safe memory write | extract → dedup/refine SKILL.md |
| Trigger | `LearningReflectionEventSink` (TurnEventSink) | `SkillLearningTurnEventSink` (TurnEventSink) |
| Gate | failure / correction-cue | substantive-success eligibility |
| Recall | persona + memory-snippet injection | keyword auto-activation |
| Flag | `IRONCLAW_LEARNING_ENABLED` (A/B, default off) | per-skill + global auto-activation toggles |

These are **complementary**, not competing: the learning design doc §7 explicitly
deferred skill auto-generation ("reflection writes *learnings* only"). #5061 fills
that gap. The risk is purely in the **mechanism**.

## 2. Convergence risks

1. **Single-slot collision.** Both write `DefaultPlannedRuntimeParts.turn_event_sink`.
   #5061 already introduced `CompositeTurnEventSink` to fan the one slot out to
   multiple sinks (trace-capture + skill-learning); the learning stack sets the slot
   to `LearningReflectionEventSink`. Whichever merges second must compose into the
   other's sink or one silently wins the slot.
2. **Two forks, not one.** Each sink independently `tokio::spawn`s a post-turn job +
   model call. Hermes (and this design's "reflection fork" collapse) is *one* gated
   post-turn pass that decides what to persist. Two spawns ⇒ double background model
   cost and divergent eligibility on the same event.
3. **Two master switches** for one "learning" story ⇒ incoherent operator UX.
4. **Duplicated extraction infra.** Both reimplement transcript-readback → one model
   call → safety-scanned write. Different artifacts (memory doc vs SKILL.md), but the
   plumbing is the same.

## 3. Proposed shape

Keep both writers; unify the **trigger + gate + composition**.

- **One composition seam.** Adopt `CompositeTurnEventSink` as the canonical fan-out
  for the single `turn_event_sink` slot. Runtime composes:
  `[trace_capture?, learning_reflection?, skill_learning?]` — each gated by its own
  enable flag, each best-effort/off-turn. (Minimal step; unblocks both PRs.)
- **One cheap gate, then route.** Promote a single `reflection_signal(event, transcript)`
  classifier that yields `{ Failure, CorrectionCue, SubstantiveSuccess, None }`.
  Declarative writer consumes Failure/CorrectionCue; procedural writer consumes
  SubstantiveSuccess. One transcript read, one gate evaluation, fan to ≤1 model call
  per writer that actually fires. (Removes the double-spawn-per-turn waste.)
- **Shared extraction primitive.** Factor the "off-turn: read transcript → one
  model call → safety-scanned scoped write" into a small reusable helper both
  writers call with their own prompt + write target.
- **One umbrella flag** (or an explicit, documented relationship) so "learning
  behavior" has a single A/B story; per-skill/per-category toggles hang under it.

## 4. Minimal integration step (do this regardless)

At merge, `runtime.rs` must build `turn_event_sink` as a `CompositeTurnEventSink`
containing **both** `LearningReflectionEventSink` (when `IRONCLAW_LEARNING_ENABLED`)
and the skill-learning sink (when its flag is on), plus trace-capture. Neither PR
may overwrite the slot with only its own sink.

## 5. Open questions

- **#5061 vs #2590 (SkillClaw).** Both target post-task skill evolution. Confirm
  they aren't duplicating the procedural side before either lands; this note assumes
  #5061 is the chosen skill track.
- **Ownership of the shared gate/primitive.** Likely a small new module in
  `ironclaw_reborn_composition` (or a neutral helper crate) consumed by both the
  reflection service and skill-learning.
- **Sequencing.** Easiest path: land the learning stack's reflection sink + #5061's
  `CompositeTurnEventSink` first (the seam), then refactor to the shared gate as a
  fast-follow rather than blocking either PR on the full unification.
