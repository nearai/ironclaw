# ironclaw_auth

Canonical crate guidance lives in [`CLAUDE.md`](./CLAUDE.md) — the guardrails
and the **enforced sub-owner map** (`cargo test -p ironclaw_auth --test
module_charter` reads that file's `## Sub-owner map` section, so it stays the
rules' one home). Orientation, measured surface/deps, and test commands are in
[`README.md`](./README.md); the family boundary is
[`../AGENTS.md`](../AGENTS.md).

This file was reduced to a pointer on 2026-08-05
(`docs/reborn/guidance-conventions.md`, rule 1): its charter summary,
ownership lists, and validation commands duplicated `CLAUDE.md` and the
README. Sources of truth beyond the crate:
`docs/reborn/contracts/auth-product.md` and PROPOSAL §6.4.8.
