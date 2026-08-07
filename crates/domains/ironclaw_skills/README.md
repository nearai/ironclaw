# ironclaw_skills

Skill parsing, validation, deterministic selection scoring, filesystem-backed
skill management, and the pure learning path (distillation/refinement) — the
content half of the prompt-level extension mechanism. Execution, hosting, and
trust *enforcement* live above; this crate owns what a skill *is* and which
ones deterministically match a request.

- **Family / layer:** `domains` / `substrates` · **Package:** `ironclaw_skills` · **Manifest:** `crates/domains/ironclaw_skills/Cargo.toml`
- **Use this when:** changing skill grammar (`SKILL.md` parsing/validation),
  selection scoring, install-metadata records, scoped management, or the
  learning prompts (`prompts/skill_extraction.md`, `prompts/skill_refinement.md`).
- **Don't use this when:** executing hooks or WASM → the loop/extensions
  tiers; deciding tool access for a skill → `ironclaw_authorization` /
  `ironclaw_capabilities`; wiring inference for learning →
  `ironclaw_composition` implements `SkillInferencePort`.

## Public surface

- Parsing/validation: `parse_skill_md`, `ParsedSkill`, `SkillParseError`,
  `validation` (attribute-safe version strings, requirement checks).
- Selection: `selector` — deterministic scoring (no ambient time, network, or
  filesystem effects).
- Management: `management` / `scoped_management` (mount-scoped installs),
  `install_metadata` records.
- Learning: `learning` — prompts, parsing, and `SkillInferencePort`, the
  inversion port the hosting tier implements.
- `types` — skill type definitions, incl. the `Installed < Trusted` trust
  ordering (what that trust gates is content exposure, decided by
  `ironclaw_loop_contracts::skill_context::SkillTrustLevel`).

## Depends on / consumed by

- **Normal deps (measured):** `ironclaw_filesystem`, `ironclaw_host_api`.
- **Consumed by (5):** `ironclaw_composition`, `ironclaw_extension_host`,
  `ironclaw_extension_manager`, `ironclaw_extension_support`,
  `ironclaw_loop_host`.

## Invariants

- Selection is deterministic and learning is pure — composition owns concrete
  inference adapters, scoped writes, and notifications (pinned by the crate's
  test suite; see [`AGENTS.md`](./AGENTS.md)).
- No reaching upward (kernel/loop/product/app) — enforced by the layer matrix
  (`reborn_workspace_crates_declare_layers_and_follow_layer_matrix`).
- Prompt templates live in `prompts/*.md`, loaded via `include_str!` — never
  inline Rust string constants (root `CLAUDE.md` code-style rule; the
  cross-crate include scan `reborn_cross_crate_include_scan.rs` polices
  reach-ins).

## Tests

```bash
cargo test -p ironclaw_skills
cargo test -p ironclaw_skills --test routing_corpus   # selection corpus (tests/fixtures)
```

## See also

- Working rules: [`AGENTS.md`](./AGENTS.md) (canonical crate guidance).
- Family boundary: [`../AGENTS.md`](../AGENTS.md).
- Runtime skills system rules: `.claude/rules/skills.md`; design record:
  `families/domains.md`, PROPOSAL §6.4.7.
