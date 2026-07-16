# ironclaw_memory guardrails

This is the **provider-neutral memory contract** crate for IronClaw Reborn. It
owns the host-facing memory vocabulary and nothing else:

- The `MemoryService` trait and its operation request/response DTOs.
- Memory document value types: `MemoryDocumentScope`, `MemoryDocumentPath`,
  `MemoryContext`, and the `/memory` path grammar + validation.
- Prompt-write-safety vocabulary (operation, source, severity, reason codes,
  policy trait, event-sink contract).
- Memory significant-event / audit contracts.

Rules:

- Keep this crate provider-neutral. Do **not** add a concrete provider
  implementation, storage backend, filesystem adapter, chunking, search,
  indexer, or the prompt-safety enforcement engine here — those live in
  provider crates such as `ironclaw_memory_native`.
- Among **internal IronClaw crates**, depend only on `ironclaw_host_api`. Neutral
  third-party crates (`serde`, `serde_json`, `async-trait`, `chrono-tz`, `sha2`,
  `tracing`) are fine — they are the contract's serialization / async-trait /
  boundary-validation substrate, the same crates `ironclaw_host_api` itself
  depends on. Do **not** depend on `ironclaw_filesystem`, `ironclaw_safety`, host
  composition, dispatch, approvals, run-state, secrets, network, process, events,
  or extension crates. A provider crate depends on this crate, never the reverse.
- Value-type constructors validate at the boundary (e.g.
  `MemoryDocumentPath::from_scope` re-validates the relative path). Do not add
  unchecked public constructors that let a caller in another crate build a
  malformed value.
- Validation is fail-closed and stable: invalid scopes, paths, or context
  values must error rather than be silently coerced.
- Fast local check: `cargo test -p ironclaw_memory`. Boundary check after
  dependency/API changes: `cargo test -p ironclaw_architecture`.
