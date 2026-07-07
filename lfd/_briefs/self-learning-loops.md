# LFD Brief: self-learning-loops — Self-learning loops

**State**: partial — `ironclaw_skill_learning` distill/refine is
production-ready; the post-run auto-fire (four learning-mission types from
engine-v2's `ensure_learning_missions`) is not wired. **Bar**: 0.90 holdout.
**Profile**: `learning`.

Prior art note: branch `claude/reborn-learning-system` holds earlier
Hermes-parity work — the dev agent should read it for design context but
build on ITS OWN branch against current main-line code.

## Outcome

Completed runs feed a learning loop automatically: failures fire
error-diagnosis missions; transcripts distill into skills
(`distill_skill`) that validate, install, and are SELECTED on subsequent
matching runs; refinement (`refine_skill`) evolves rather than duplicates
existing skills; successful uneventful runs don't spam learning missions.

## Spec sources

- `crates/ironclaw_skill_learning/` (+ its prompts/*.md),
  `crates/ironclaw_skills/` (selection pipeline, parse_skill_md, install)
- `crates/ironclaw_engine/src/runtime/mission.rs::ensure_learning_missions`
  (the four types — behavioral reference)
- `docs/plans/2026-03-24-missions.md` step 4, `docs/plans/2026-06-16-reborn-skill-evolution.md`
- Parity doc "Learning Missions: Partial"

## Stage 0 inner suite

`ironclaw_skill_learning` + `ironclaw_skills` crate tests + new auto-fire
integration tests per spec. Green every cycle. (Missions LFD is a sibling —
this loop may stub the mission-manager seam if that LFD hasn't landed;
spec.md must define the seam explicitly.)

## Eval themes (dev ~30 / holdout ~10)

1. Auto-fire on failure (6): scripted run with tool errors → error-diagnosis
   learning mission fires exactly once (state query; forbidden: duplicate
   fire, forbidden: fire on the clean-run control cases).
2. Distillation validity (8): scripted distillation output → lands as a
   skill that `parse_skill_md` accepts, installed in registry (state
   query), frontmatter triggers reference the scenario's actual context
   (probe renames transcript entities — canned skills fail).
3. Selection delta (6): case = two-phase scenario; phase 2 (same task
   shape) must include the learned skill in the envelope (state/envelope
   contract) and phase-2-without-learning control must not.
4. Refinement (5): existing similar skill + new transcript → refine path
   updates it (state_eq on skill identity, version bump) instead of
   installing a near-duplicate (forbidden: second skill with overlapping
   triggers).
5. Retention invariant (5): learning inputs (transcripts, traces) are
   never deleted or truncated by the loop (forbidden: deletion events;
   LLM-data retention rule).

## Feature-specific cheats → fences

- **Canned skill emission** (same skill text every time) → theme-2
  trigger-context contracts keyed to probe-perturbed entities; holdout
  transcripts are new domains.
- **Fire-and-forget** (mission fires, installs nothing) → paired required
  matchers: fire event AND registry state change.
- **Copy eval transcripts into skills** → answer-literal overlap lint
  (sealed contract literals); dev transcripts are visible inputs so
  copying them is caught by probe (renamed entities) instead.
- **Selection delta via always-include** (stuff every skill into envelope)
  → control cases forbid the skill when task shape doesn't match; skills
  budget (SKILLS_MAX_TOKENS) state_pred.
- **Suppress fires to dodge duplicate-fire penalties** → clean-run
  forbidden cases are paired with failure-run REQUIRED fires; both
  directions priced.

## caps.json extras

Dev transcript entity literals in `crates/**` diff: max 0. New prompt
strings inline in Rust (must be prompts/*.md per repo rule): lint pattern
for multiline string constants in `crates/ironclaw_skill_learning/**` diff,
max 0.

## Live mode

4 live cases (this feature benefits most from live eval): real model
distills a skill from a scripted failure transcript → structural contracts
(parse-valid, trigger references transcript context, installs). Distillation
quality beyond structure is reviewed by the human at acceptance, not scored.
