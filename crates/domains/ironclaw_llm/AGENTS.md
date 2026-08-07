# ironclaw_llm

Canonical crate guidance lives in [`CLAUDE.md`](./CLAUDE.md) — the module spec
named in the root `CLAUDE.md` Module Specs table, carrying the file map, the
**enforced sub-owner map** (`cargo test -p ironclaw_llm --test module_charter`
reads that file's `## Sub-owner map` section), provider selection, per-provider
gotchas, and the decorator chain. Orientation, measured surface/deps, and test
commands are in [`README.md`](./README.md); the family boundary is
[`../AGENTS.md`](../AGENTS.md).

This file was reduced to a pointer on 2026-08-05
(`docs/reborn/guidance-conventions.md`, rule 1): its buckets and notes
duplicated the spec, and where the two disagreed the map had already been
declared the winner. Two facts worth keeping visible at the door:

- `reasoning.rs` is **not legacy and not dead** — the v1 engine half was
  deleted (#6964); the three surviving public functions have five production
  call sites in `crates/loop/ironclaw_loop_host/src/model_gateway.rs`.
- `complete_with_tools()` is never cached (tool calls can have side effects);
  the full rule set is in the spec.
