# Agent Map — ironclaw_event_projections

## Start Here

- Read `CLAUDE.md` first; it is the crate-local guardrail file.
- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these Reborn contracts before changing behavior:
  - `docs/reborn/contracts/events.md`
  - `docs/reborn/contracts/events-projections.md`
  - `crates/ironclaw_events/AGENTS.md`
  - `crates/ironclaw_outbound/AGENTS.md`

## What This Crate Owns

- Replay/materialization-agnostic projection traits and DTOs.
- Metadata-only projection outputs for product/read-model consumers.
- Scoped reads with explicit stream and read-scope filters.
- Projection contract tests for extension lifecycle, memory prompt safety, significant memory events, and replay behavior.

## Do Not Move In Here

- Backend rows or direct JSONL/PostgreSQL/libSQL adapter dependencies.
- Mutations to durable logs or kernel state from projection failures.
- Raw inputs, raw outputs, host paths, secrets, approval reasons, invocation fingerprints, backend details, or unredacted payloads.
- Transport delivery or outbound delivery status persistence.

## Validation

- Fast local check: `cargo test -p ironclaw_event_projections`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture`
- Run outbound/product workflow tests when projection shape changes affect delivery candidates or UI-visible feeds.

## Agent Notes

- Keep projections backend-independent and replayable.
- Add new DTO fields only when they remain metadata-only and explicitly scoped.
- Projection failures should be observable but non-mutating.
