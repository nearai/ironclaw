# Agent Map — ironclaw_memory

## Start Here

- Read `CLAUDE.md` first; it is the crate-local guardrail file.
- Read `Cargo.toml` for actual dependencies (the only internal IronClaw crate
  dependency should be `ironclaw_host_api`; neutral third-party crates such as
  `serde` / `serde_json` / `async-trait` / `chrono_tz` are fine).

## What This Crate Owns

- The provider-neutral `MemoryService` trait and its operation request/response
  DTOs (`service`).
- Memory document value types and the `/memory` path grammar: `MemoryDocumentScope`,
  `MemoryDocumentPath`, `MemoryContext` (`path`, `context`).
- Document metadata vocabulary (`metadata`) and content hashing helpers (`hash`).
- Prompt-write-safety contract vocabulary — operation, source, severity, reason
  codes, policy trait, event sink (`safety`).
- Memory significant-event / audit contracts (`events`).

## Do Not Move In Here

- Concrete providers, storage backends, filesystem adapters, chunking, search,
  indexers, or the prompt-safety enforcement engine — those belong in provider
  crates such as `ironclaw_memory_native`.
- Any internal IronClaw crate dependency beyond `ironclaw_host_api` (neutral
  third-party crates are fine).

## Validation

- Fast local check: `cargo test -p ironclaw_memory`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture`

## Agent Notes

- A provider crate depends on this crate, never the reverse.
- Keep value-type constructors validating at the boundary; do not add unchecked
  public constructors.
