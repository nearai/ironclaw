# ironclaw_composition

The crate's **module spec** is [`CONTRACT.md`](./CONTRACT.md) — named in the
root `AGENTS.md` Module Specs table; code follows spec, spec is the tiebreaker.
Working rules: [`AGENTS.md`](./AGENTS.md); orientation: [`README.md`](./README.md);
the family boundary: [`crates/app/AGENTS.md`](../AGENTS.md).

> **Why this file is a real file and not a symlink**, unlike every other crate's
> `CLAUDE.md`: `reborn_composition_boundaries.rs::composition_root_embeds_no_prompt_content`
> refuses a symlink anywhere under this crate — its ownership walks do not follow
> links, and stepping over one would let the gate report clean on a subtree it
> never read. Keep this a regular file; the gate fails loudly if it becomes a link.
