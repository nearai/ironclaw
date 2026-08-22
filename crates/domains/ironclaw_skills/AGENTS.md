# Agent Map — ironclaw_skills

## Start Here

- This file is the canonical crate-local rules; orientation, measured
  surface/deps, and test commands are in [`README.md`](./README.md), and the
  family boundary is [`../AGENTS.md`](../AGENTS.md).
- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these sources of truth before changing behavior:
- `.claude/rules/skills.md`
- `docs/internal/reborn/contracts/extensions.md`

## What This Crate Owns

- Skill metadata parsing (`parser`), validation (`validation`), deterministic scoring/selection (`selector`), pluggable activation-scoring strategy binding (`activation_strategy`), filesystem management and its mount-scoped port (`management`, `scoped_management`), installed-skill records (`install_metadata`), pure learning distillation/refinement logic (`learning`), requirement gating for skill activation (`gating`), and the skill type definitions (`types`).
- Crate-local public API, tests, and fixtures needed to prove that ownership.

> ✎ **Corrected 2026-08-04 (WS6 domain-internal cleanups), re-corrected since.**
> This section previously claimed modules `gating`, `registry` and `catalog`, and
> a `v2` module exporting `V2SkillMetadata` / `CodeSnippet` / `SkillMetrics` /
> `SkillRevision` / `SkillRepairRecord` "serialized into `MemoryDoc.metadata` by
> the engine crate". `registry`, `catalog`, and the `v2` module never came back
> and still do not exist (`ironclaw_engine` stays deleted). **`gating` is back**,
> though: `crates/domains/ironclaw_skills/src/gating.rs` was restored after main
> deleted it as verified-dead (#6943) — the deletion was itself the bug, since
> `requires` was parsed and never consulted, so a skill declaring a missing
> binary activated cleanly and failed later in the shell. Today's `gating` is a
> different, smaller module than the one this note originally described: it
> owns only `GatingResult`/`binary_exists`/`check_requirements_sync` gating a
> skill's declared `requires.bins`/`requires.env`/`requires.config`, not the
> registry/catalog/v2 system the 2026-08-04 correction was refuting. Re-verify
> module claims against `src/` before trusting either version of this note:
> `ls crates/domains/ironclaw_skills/src/`.

## Do Not Move In Here

- Concrete prompt execution, LLM/runtime adapters, tool authorization, extension runtime dispatch, credential handling, channel UI, or ClawHub server behavior.
- Compatibility shims for unsupported legacy skill metadata unless the parser contract explicitly changes.
- Secrets, raw host paths, backend error details, and unredacted user content in errors, events, snapshots, logs, or docs.

## Validation

- Fast local check: `cargo test -p ironclaw_skills`
- Feature-shape check after catalog/registry changes: `cargo test -p ironclaw_skills --all-features`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture_tests`

## Agent Notes

- Skill selection must stay deterministic: no ambient time, network, or filesystem effects in scoring.
- Skill learning must stay pure: `learning` owns prompts, parsing, and the `SkillInferencePort` abstraction only; composition owns concrete inference adapters, scoped writes, and notifications.
- Installed skills are lower-trust than user/workspace skills. What that trust gates is **content exposure** (prompt body vs. safe description only), decided by `ironclaw_loop_contracts::skill_context::SkillTrustLevel` — not tool access, which `ironclaw_authorization` / `ironclaw_capabilities` own. Preserve the `Installed < Trusted` ordering `SkillTrust` derives.
- Add caller-level tests when parser or selection changes affect prompt assembly or skill-content exposure.
