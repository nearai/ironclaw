# LFD Brief: missions — Missions

**State**: partial — `ironclaw_triggers` fires synthetic inbound turns;
MissionManager/meta-prompt/progress/adaptation (design steps 1–6) not wired.
**Bar**: 0.90 holdout. **Profile**: `missions`.

## Outcome

Goal-oriented mission threads per `docs/plans/2026-03-24-missions.md`:
triggers (cron/event) spawn mission threads whose meta-prompt is built from
project memory; progress is tracked per run; missions persist across
restarts; repeated runs adapt (`next_focus` / `approach_history`) based on
prior outcomes; completed missions land results and stop firing.

## Spec sources

- `docs/plans/2026-03-24-missions.md` (authoritative; steps 1–2 NEW, 3–6 deferred)
- `crates/ironclaw_triggers/`, composition trigger-poller wiring
- `crates/ironclaw_engine/src/runtime/mission.rs` (engine-v2 shell —
  behavioral reference, do NOT extend engine v2)
- `docs/reborn/engine-v2-to-reborn-parity.md` missions section

## Stage 0 inner suite

`ironclaw_triggers` crate tests + `tests/reborn_group_triggers/` + new
MissionManager tests per spec. Green every cycle.

## Eval themes (dev ~35 / holdout ~12)

1. Spawn (6): cron trigger fires → mission thread created with mission
   metadata (state query mission/thread record: goal text, trigger ref,
   scope); manual + event trigger variants.
2. Meta-prompt data flow (8): project memory docs seeded in setup →
   mission's prompt envelope contains a digest derived from THOSE docs
   (reply/state contracts on envelope content; probe varies doc contents —
   a static meta-prompt fails probes). Docs absent → graceful minimal
   prompt (fail-soft, not crash).
3. Progress tracking (6): scripted run makes partial progress → progress
   record persists (state query), reflects step outcomes, never deleted
   (LLM-data retention invariant — forbidden: deletion events).
4. Persistence (5): mission survives process restart (group storage
   reload); next fire resumes the SAME mission identity, no duplicate
   mission per trigger (forbidden: second mission record).
5. Adaptation (6): case scripts a failed attempt (tool errors) → next fire's
   envelope carries `next_focus` referencing the failure category and
   `approach_history` grows (state_pred; probe perturbs the failure kind —
   canned next_focus text fails).
6. Completion (4): success evaluation met → mission marked complete, trigger
   quiesced (forbidden: post-completion fire), summary landed to memory.

## Feature-specific cheats → fences

- **Static meta-prompt** → theme-2 contracts key on seeded doc content via
  the probe map; memorized digests break under renames.
- **Mission that never executes** (record created, no run) → required
  Turn/step events per case, egress where the scripted plan demands it.
- **Canned adaptation strings** → next_focus contracts match the perturbed
  failure category, holdout uses failure kinds absent from dev.
- **Duplicate-spawn padding** (respawn fresh missions instead of resuming)
  → forbidden second-mission-record matcher; identity continuity state_eq.
- **Trigger-loop spam to inflate events** → forbidden: fire count > expected
  within scenario window.

## caps.json extras

Failure-category literal table: pattern of dev failure-kind strings in
`crates/**` diff, max 0. Mission goal-text literals from dev cases in diff,
max 0.

## Live mode

4 live cases: real model runs one mission end-to-end over stubbed tools
(HN-monitor-shaped, matching existing QA trace themes) — required: progress
record + completion summary written; structural contracts only (live text
varies).
