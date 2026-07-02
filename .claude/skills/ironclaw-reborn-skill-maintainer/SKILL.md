---
name: ironclaw-reborn-skill-maintainer
description: Use when creating or editing agent guidance in this repo — anything under .claude/ (skills, commands, rules), AGENTS.md or CLAUDE.md files, or the runtime skills/ directory — or when guidance is found citing files, branches, or checks that no longer exist.
---

# IronClaw Guidance & Skill Maintenance

Every rule below counters a rot pattern this repo's guidance is prone to. Guidance is code: it has callers (agents), it breaks silently, and it needs the same review discipline.

## The two skill systems — never conflate them

- `.claude/skills/` + `.claude/commands/` + `.claude/rules/` → **developer-facing** (Claude Code; Codex reads only the AGENTS.md hierarchy).
- Top-level `skills/` → **product runtime skills, compiled into both shipping binaries** (`build.rs` → `src/skills/bundled.rs`; `crates/ironclaw_reborn_composition/src/bundled_skills.rs`). Editing `skills/` changes what users' agents do, not your workflow. Treat it as production code: test-first, product review.

## Authoring rules (each one earned)

1. **Recipes over maps.** Encode grep/verify *procedures* that re-derive facts, not hardcoded file:function maps. **When someone asks for a "key files list" in guidance, do not ship a bare list** — ship (a) the grep recipes that regenerate it, (b) at most a handful of stable anchor paths, and (c) beside *every* listed path, the one-line command that re-verifies it. A list without its regeneration recipe will drift.
   - Corollary: any flow/architecture map you write must respect the legacy enclave — `ironclaw_engine`, `ironclaw_tui`, `ironclaw_gateway`, `ironclaw_oauth`, `ironclaw_embeddings` are v1-only and never belong in a Reborn flow list (check `ironclaw-reborn-orientation` when unsure which side a crate is on).
2. **Verify every concrete reference at write time — and make it re-verifiable.** Branches die silently; prefer in-tree worked examples over branch/PR refs. For each cited path/symbol, keep a one-line grep a maintainer can re-run.
3. **No universal claims without a count.** Avoid absolute docs coverage claims. Write "most; fall back to Cargo.toml + lib.rs" or generate the list.
4. **Triggers live in frontmatter `description`, nowhere else.** The runtime surfaces only frontmatter for selection; a "when to use" buried in the body is invisible. Description = triggering conditions only — never a workflow summary (agents will follow the summary and skip the body).
5. **Never claim enforcement that doesn't exist.** Before writing "enforced by X", run X. If you add a check to `scripts/pre-commit-safety.sh` or `scripts/check-boundaries.sh`, add its self-test (`.claude/rules/review-discipline.md`: guardrail scripts are code) — and know there are two hook install paths (`.githooks/` vs `scripts/dev-setup.sh` symlink); a check is only real if both run it.
6. **After any extraction/move/rename, grep the guidance layer** for old paths in the same PR: `.claude/`, `AGENTS.md`, `CLAUDE.md`, `crates/AGENTS.md`, `docs/reborn/contracts/`, skill bodies.
7. **Runtime-skill spec must track the parser.** `crates/ironclaw_skills/src/types.rs` supports `requires.config`, `requires.skills`, `activation.setup_marker`, and silently truncates >20 keywords / >5 patterns (`enforce_limits`). If you use or change parser behavior, update `.claude/rules/skills.md` in the same PR.
8. **Codex parity check.** Codex cannot load Claude skills; it reads AGENTS.md. When a skill carries a rule Codex must also follow, the AGENTS.md hierarchy needs the pointer *with enough inline substance* ("read `.claude/skills/<name>/SKILL.md`" works — skills are plain markdown).
9. **Test skills like code (RED-GREEN).** Before shipping a new/edited skill: run the tempting scenario with a fresh subagent *without* the change (does the failure actually occur?), then with it (is it caught?). A skill nobody failed without is dead weight; a skill you didn't re-test may not bind. Record trap results in the skill PR.
10. **`crates/AGENTS.md` map hygiene**: when adding a crate, add its row. Absence from the routing map = agents misrouted.
11. **Two evidence layers, two lifecycles.** Skill `references/` files are the **living** teaching copies (worked examples, exemplar tests) — update them as code evolves. Audit/postmortem snapshots are **immutable** — never edit one; cite one only as dated provenance, and never make a skill's operative content depend on a document that isn't versioned alongside it.

## Definition of done for a guidance PR

- [ ] Every cited path/symbol/branch verified against HEAD (grep output in PR description)
- [ ] Frontmatter description = triggers only, third person
- [ ] Enforcement claims executed, not assumed
- [ ] Fresh-agent trap test run for behavior-changing edits (result noted)
- [ ] Old-path grep clean after moves
- [ ] Runtime `skills/` untouched unless the *product* change was intended
