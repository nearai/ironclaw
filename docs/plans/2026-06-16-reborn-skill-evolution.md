# Reborn Skill Extraction + Self-Evolution Prototype (Hermes-style)

**Date:** 2026-06-16  ·  **Target:** ~2-day demo-first prototype  ·  **Status:** plan approved, decisions locked

Goal: bring Hermes-style "learn skills from experience + self-evolve them to be
*measurably better*" to IronClaw Reborn, with **minimal coupling** and **zero edits
to the sealed `ironclaw_agent_loop` crate**. Demo value is the top priority.

## 0. Context (verified by research)

- Two parallel stacks: **engine v2** (`ironclaw_engine`) already has the full learning
  system but is **not** wired into the demo runtime; the demo binary `ironclaw-reborn`
  (crate `ironclaw_reborn_cli`) has **zero dependency on `ironclaw_engine`**. Engine v2 is a
  **blueprint to port**, not a runtime to enable.
- The runtime is the **Reborn stack** (`ironclaw_reborn` + `ironclaw_agent_loop` +
  `ironclaw_turns` + `ironclaw_webui_v2`), WebChat v2 @ `:3000/v2`, provider NEAR AI,
  live model `deepseek-ai/DeepSeek-V4-Flash`.
- The skill closed-loop already exists in Reborn: `ironclaw_skills::{install_skill,update_skill}`
  write to `/projects/tenants/{T}/users/{U}/skills`; the SAME path is listed by
  Settings→Skills (`GET /api/webchat/v2/skills`) and read into the next run's prompt by
  `FilesystemSkillBundleSource`. Reuse it; do not invent storage.

## 1. Locked decisions

| # | Decision | Choice |
|---|----------|--------|
| 1 | Extraction trigger | **Both** — explicit "Save as skill" (reliable spine + fallback) **and** autonomous auto-extract on substantive successful completion (stretch) |
| 2 | Evolution on stage | **Real code, pre-baked artifacts** — loop is real & runnable; on-stage diff/metrics pre-generated; lead with the *gate rejecting a regression* |
| 3 | Apply mode | **Stage + one-click approval + safety scan** — route writes through the WebUI facade for `validate_skill_content_safety`; learned skills go to a pending list |
| 4 | Models | **Strong-model "learning" slot** (Opus 4.8 / GPT-5.5 via NEAR AI) for extraction + evolve judge/mutation; live agent stays on DeepSeek-V4-Flash |

## 2. Non-negotiable correctness constraints (silent-failure guards)

1. **Scope from the run, never `owner_scope()`/`local_default`.** Derive `(tenant,user)` like
   `resource_scope_for_run` (run tenant + actor user). Wrong scope = green build, dead demo.
   Assert + loudly log the resolved tenant at write time.
2. **`install_skill` is create-only.** Re-learn/evolve must use `update_skill` with a
   **byte-stable frontmatter `name`** (slugified, deterministic). Name must pass
   `validate_skill_name` (alnum/`._-`, no leading dash/dot, no spaces).
3. **Module lives inside `ironclaw_reborn_composition`** (`RebornLocalSkillManagementPort`
   is `pub(crate)`).
4. **Post-run work is detached.** Inside `TurnEventSink::publish`, `tokio::spawn` and return
   `Ok(())` immediately — never await the LLM inline (would stall turn-completion publication).
5. **Safety:** route the write through the WebUI facade install path so
   `validate_skill_content_safety` (High/Critical injection block) applies; otherwise disclose
   that learned skills are unscanned trusted prompt text loaded into the next system prompt.

## 3. The integration seam (zero sealed-crate edits)

- **Trigger:** install `Arc<dyn TurnEventSink>` at `DefaultPlannedRuntimeParts.turn_event_sink`
  (composition `runtime.rs:2524`, currently `None`); gate on `kind==Completed`. (The
  `loop_completed`/EventTriggeredHook path is dead by default — do not use. `LoopExitApplier`
  is a concrete by-value struct — not decoratable.)
- **Read trace:** `SessionThreadService::list_thread_history(scope, thread_id)` →
  redacted transcript. "Substantive" gate = count `ThreadMessageRecord` where
  `kind==ToolResultReference && turn_run_id==run`. (Full tool args need `IRONCLAW_RECORD_TRACE`;
  redacted transcript is the default and is acceptable for the prototype.)
- **Write skill:** `ironclaw_skills::install_skill`/`update_skill` via the facade path.
- **Live UI:** reuse `SkillActivation.feedback: Vec<String>` (rendered at `useChatEvents.js:424-429`)
  for the "🎓 Learned / 🧬 Evolved" bubble — zero new wire variants; construct a
  `LiveProjectionPublisher` (`projection.rs:157-165`) and `publish_live_item` from the post-run
  path. Add one line `queryClient.invalidateQueries(['skills'])` for Settings auto-refresh.

## 4. Phase plan

### Phase 0 — Spike (first ~2h, de-risk the unknowns)
- Branch off a clean base (the prototype must not contaminate unrelated WIP).
- **Spike the strong-model one-shot completion** through the Reborn model layer
  (`model_gateway.rs` / per-slot `LlmSlot.model`). This is the #1 ergonomic unknown. If awkward,
  fall back to a direct NEAR client for extraction/judge calls.
- Add a `learning` model slot config pinned to a strong model.

### Phase 1 — Extraction loop (core; Day 1)
1. New `extraction` module in `ironclaw_reborn_composition` exposing
   `async fn extract_skill_from_run(scope, thread_id, run_id, models, skill_port)`.
2. Read transcript + substantive gate (≥N steps & ≥M tool results).
3. One strong-model call (port of `mission_skill_extraction.md`) → SKILL.md
   (frontmatter w/ deterministic name + When to Use / Procedure / Pitfalls / Verification).
4. Stage to a **pending list** (decision 3); one-click approve → write via facade (`install_skill`,
   or `update_skill` on Conflict).
5. **Explicit trigger:** `ironclaw-reborn skills extract` CLI + a "Save as skill" affordance
   (reliable, rehearsable spine).
6. **Autonomous trigger (stretch):** `TurnEventSink` filtered to `kind==Completed` calls the same
   `extract_skill_from_run`, detached. Verify in rehearsal; fall back to explicit if flaky.
7. **Demo Moment #1:** hard multi-step task → "🎓 Learned skill X" → open in Settings→Skills →
   new task uses `/X` (explicit; `ExplicitOnly` default) → qualitatively follows the Procedure /
   picks the right tool first try. (No live stopwatch claim.)

### Phase 2 — Evolution loop (GEPA-lite; Day 2)
- New `ironclaw-reborn skills evolve <name>` (in composition):
  1. **Frozen, committed eval fixture** (hand-authored / pre-generated then reviewed) — NOT
     self-generated at demo time. Report `n` explicitly.
  2. Baseline score: single-LLM-call proxy (skill-as-context + task → output), **judge returns
     score + textual feedback** (why it failed). Judge model ≠ mutation model; temp 0.
  3. Reflective mutation: skill + aggregate failure feedback → strong model → **1** mutated
     SKILL.md (the skill TEXT is the artifact — avoids Hermes issue #38).
  4. Score the candidate on the frozen set; **gate** (≤15KB, ≤+20% growth, valid frontmatter,
     must beat baseline by margin). Keep-best-if-improves.
  5. Emit before/after diff + `metrics.json`; stage for approval → `update_skill` version bump
     (parent_version preserved for rollback).
- Harden every LLM→JSON boundary: temp 0, fenced JSON, fence-strip + serde + one bounded retry +
  checked-in fallback fixture.
- **Demo Moment #2:** before/after diff + "frozen set n=N, self-judged, illustrative; cost ~$X;
  gates passed", and **show the gate rejecting a deliberately-worse variant**. Pre-baked on stage;
  one live judged call best-effort.

### Phase 3 — Demo polish (interleaved)
- "🎓 Learned / 🧬 Evolved vN→vN+1" bubble via `SkillActivation.feedback` reuse.
- One-line `['skills']` query invalidation for Settings auto-refresh.
- Scripted runbook + pre-baked fallback artifacts (SKILL.md, evolve diff, metrics) + a
  "reset skill dir" pre-demo step.

## 5. Explicitly out of scope (2-day cut)

Wiring engine v2 into Reborn; multi-objective Pareto frontier; executable skill **code** snippets
(skills stay pure prompt text); SessionDB trace mining; tool-description / system-prompt evolution
(Hermes Tiers 2–4); live eval-set generation; multi-variant (2–3) scoring; real agent-loop runs
for scoring (single-LLM-call proxy only — disclose the metric difference vs Phase-1's tool-call claim).

## 6. Honesty notes for the demo

- Phase-1 "better" = qualitative (follows Procedure / right tool first try), not a live stopwatch.
- Phase-2 metric is a **self-judged single-call proxy on a frozen n=N set** — present as
  *illustrative*, with `n` and the self-judged caveat on the same slide; the **gate rejecting a
  regression** is the most credible beat.

## 7. Implementation log

Branch `feat/skill-evolution` off `origin/main`.

### Increment 1 — turn-end seam (DONE, compiles clean)
- `crates/ironclaw_reborn_composition/src/skill_learning.rs`: `SkillLearningTurnEventSink`
  (on `TurnEventKind::Completed`, reads the run transcript via
  `SessionThreadService::load_context_window`, gates >=3 tool actions & >=5 messages) +
  `CompositeTurnEventSink` (fans the single `turn_event_sink` slot to both trace-capture and
  skill-learning — additive, no change to existing behavior).
- Wired at `runtime.rs` (composed at the trace-capture site; `turn_event_sink: Some(turn_event_sink)`);
  module registered in `lib.rs`. `cargo check -p ironclaw_reborn_composition` green.
- Modeled on `trace_capture.rs` (the existing non-run post-completion sink).

### Increment 2 — extraction LLM call (scoped, prompt ported)
- Prompt: `crates/ironclaw_reborn_composition/assets/prompts/skill_extraction.md` (one-shot
  transcript -> `SKILL.md` or `SKIP:`), to load via `include_str!`.
- LLM path (idiomatic, no raw provider plumbing): `SystemInferencePort::call_system_inference`
  (`crates/ironclaw_turns/src/run_profile/system_inference.rs`) via
  `ModelGatewayBackedSystemInferencePort::new(model_gateway, LoopRunContext)`
  (`ironclaw_loop_support::system_inference`). Template = the `failure_explanation` wiring at
  `runtime.rs:2694`. The `model_gateway` Arc is in scope at the composition site.
- Learning model = a model-id string; NEAR AI honours per-request model override
  (`CompletionRequest::model`), so no separate provider is constructed.
- TODO: thread a `SystemInferencePort` (+ learning model id + the extraction prompt) into
  `SkillLearningTurnEventSink`; build the request from the transcript; parse `SKILL.md`/`SKIP`.

### Key API references
- `LlmProvider::complete(CompletionRequest{messages, model, ...}) -> CompletionResponse{content}`
  (`ironclaw_llm/src/provider.rs:515`).
- Skill write (scoped): `SkillManagementContext::new(filesystem, mounts, scope)` +
  `install_skill`/`update_skill` (`ironclaw_skills/src/management.rs`); scope MUST come from the
  event (tenant + owner), mirroring `RebornLocalSkillManagementPort` in `lifecycle.rs`.
