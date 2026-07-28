# Agent Map — ironclaw_skills

## Start Here

- No crate-local `CLAUDE.md` exists yet; use this map plus the skills rules below.
- Read `Cargo.toml` for actual dependencies, feature flags, and catalog/registry shape.
- Use these sources of truth before changing behavior:
- `.claude/rules/skills.md`
- `CLAUDE.md`
- `docs/reborn/contracts/extensions.md`

## What This Crate Owns

- Skill metadata parsing (`parser`), validation (`validation`), deterministic gating/scoring/selection (`gating`, `selector`), registry operations (`registry`), catalog lookup (`catalog`), pure learning distillation/refinement logic (`learning`), and trust-aware v1 skill type definitions (`types`).
- V2 engine skill types (`v2`): `V2SkillMetadata`, `CodeSnippet`, `SkillMetrics`, `SkillRevision`/`SkillRepairRecord` — serialized into `MemoryDoc.metadata` by the engine crate.
- Crate-local public API, tests, and fixtures needed to prove that ownership.

## Do Not Move In Here

- Concrete prompt execution, LLM/runtime adapters, tool authorization, extension runtime dispatch, credential handling, channel UI, or ClawHub server behavior.
- Compatibility shims for unsupported legacy skill metadata unless the parser contract explicitly changes.
- Secrets, raw host paths, backend error details, and unredacted user content in errors, events, snapshots, logs, or docs.

## Validation

- Fast local check: `cargo test -p ironclaw_skills`
- Feature-shape check after catalog/registry changes: `cargo test -p ironclaw_skills --all-features`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture`

## Agent Notes

- Skill selection must stay deterministic: no ambient time, network, or filesystem effects in scoring.
- Skill learning must stay pure: `learning` owns prompts, parsing, and the `SkillInferencePort` abstraction only; composition owns concrete inference adapters, scoped writes, and notifications.
- Installed skills are lower-trust than user/workspace skills; preserve tool-ceiling attenuation.
- Add caller-level tests when parser or gating changes affect prompt assembly or tool exposure.
