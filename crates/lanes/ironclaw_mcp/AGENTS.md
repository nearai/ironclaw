# ironclaw_mcp — guidance pointer

Canonical guidance for this crate lives in:

- [`CLAUDE.md`](./CLAUDE.md) — the crate's working rules. **Gate-pinned:**
  `tests/module_charter.rs` requires it to keep naming the failure-string rule
  and the gate; edit it with `cargo test -p ironclaw_mcp` in hand.
- [`src/lib.rs`](./src/lib.rs) — the seven-module charter table (the
  authoritative "where does new code go" answer).
- [`README.md`](./README.md) — orientation: what the lane is, public surface,
  measured edges, tests.
- [`../AGENTS.md`](../AGENTS.md) — the `lanes/` family boundary (the lane
  contract) and its gates.

Contracts of record: `docs/reborn/contracts/mcp.md`,
`docs/reborn/contracts/runtime-workflows.md`,
`docs/reborn/contracts/processes.md`.

Consolidated 2026-08-05 per `docs/reborn/guidance-conventions.md` rule 1 (one
canonical home per fact); this file previously duplicated the charter table and
the guardrails' ownership content.
