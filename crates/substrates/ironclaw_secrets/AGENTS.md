# ironclaw_secrets — guidance pointer

Canonical guidance for this crate lives in:

- [`CLAUDE.md`](./CLAUDE.md) — the crate's working rules (one-shot lease
  mechanics, no-raw-material rules, `put` as a trusted primitive).
- [`README.md`](./README.md) — orientation: what the crate is, public surface,
  measured edges (including the direct-consumer narrowing status), tests.
- [`../AGENTS.md`](../AGENTS.md) — the `substrates/` family boundary and its
  gates; secrets has the family's tightest mediation story.

Contracts of record: `docs/reborn/contracts/secrets.md`,
`docs/reborn/contracts/storage-placement.md`,
`docs/reborn/contracts/kernel-boundary.md`.

Consolidated 2026-08-05 per `docs/reborn/guidance-conventions.md` rule 1 (one
canonical home per fact); this file previously duplicated the guardrails'
ownership and validation content.
