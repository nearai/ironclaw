# ironclaw_sandbox — guidance pointer

Canonical guidance for this crate lives in:

- [`CLAUDE.md`](./CLAUDE.md) — the crate's working rules, **wiring status**
  (three production paths, no production execution backend — read it before
  deleting "dead" code), and known debt (`script.rs` direct spawn, the inert
  Docker fail-closed switch #7081).
- [`README.md`](./README.md) — orientation: what the lane is, public surface,
  measured edges (including the three substrate deps carried in by the WS3
  merge), tests.
- [`../AGENTS.md`](../AGENTS.md) — the `lanes/` family boundary (the lane
  contract) and its gates.

Contracts of record: `docs/reborn/contracts/scripts.md`,
`docs/reborn/contracts/processes.md`,
`docs/reborn/contracts/runtime-workflows.md`,
`docs/reborn/contracts/network.md`.

Consolidated 2026-08-05 per `docs/reborn/guidance-conventions.md` rule 1 (one
canonical home per fact); this file previously duplicated the guardrails'
ownership content, and its "script runtime lane over host-mediated …" claim
overstated what `CLAUDE.md`'s known-debt section documents accurately.
