# ironclaw_memory — working rules

Orientation (what this crate is, surface, deps, tests) lives in
[`README.md`](./README.md); the family boundary in
[`../AGENTS.md`](../AGENTS.md). This file is the canonical crate-local rules —
consolidated 2026-08-05 from the former `CLAUDE.md` guardrails (now a pointer)
per `docs/internal/reborn/guidance-conventions.md` rule 1.

This is the **provider-neutral memory contract** crate. It owns the
host-facing memory vocabulary and nothing else: the `MemoryService` trait and
its operation DTOs, the memory document value types and `/memory` path grammar
(`MemoryDocumentScope`, `MemoryDocumentPath`, `MemoryContext`),
prompt-write-safety vocabulary, significant-event/audit contracts, and the
shared conformance suite (`test_support`).

## Rules

- Keep this crate provider-neutral. Do **not** add a concrete provider
  implementation, storage backend, filesystem adapter, chunking, search,
  indexer, or the prompt-safety enforcement engine here — those live in the
  provider extension packages
  (`crates/extensions/packages/memory-native/` = `ironclaw_memory_native`,
  `crates/extensions/packages/mem0/` = `ironclaw_memory_mem0`). A provider
  depends on this crate, never the reverse.
- Among internal IronClaw crates, the armed allowlist is
  `{ironclaw_host_api, ironclaw_prompt_envelope}` — enforced in
  `reborn_crate_dependency_boundaries_hold`
  (`crates/app/ironclaw_architecture_tests/tests/reborn_dependency_boundaries.rs`);
  today only `ironclaw_host_api` is actually used. Neutral third-party crates
  (`serde`, `serde_json`, `async-trait`, `chrono-tz`, `sha2`, `tracing`) are
  fine — they are the contract's serialization/async-trait/boundary-validation
  substrate. Do **not** depend on `ironclaw_filesystem`, `ironclaw_safety`,
  composition, dispatch, approvals, run-state, secrets, network, process,
  events, or extension crates.
- Value-type constructors validate at the boundary (e.g.
  `MemoryDocumentPath::from_scope` re-validates the relative path). Do not add
  unchecked public constructors that let another crate build a malformed
  value.
- Validation is fail-closed and stable: invalid scopes, paths, or context
  values must error rather than be silently coerced.
- Naming a memory *provider* from any other crate is gated:
  `only_the_sanctioned_residue_names_a_memory_provider` (shrink-only ledger);
  retired vocabulary is pinned by `reborn_memory_retired_vocabulary.rs`.

## Validation

- Fast local check: `cargo test -p ironclaw_memory`
- Boundary check after dependency/API changes:
  `cargo test -p ironclaw_architecture_tests`
