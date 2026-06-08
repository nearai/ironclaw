# ironclaw_reborn_openai_compat_storage

Durable storage adapters for the Reborn OpenAI-compatible ref/idempotency
contract from `ironclaw_reborn_openai_compat`.

## Boundary

This crate is storage-only:

- It implements `OpenAiCompatRefStore` over `ironclaw_filesystem::RootFilesystem`.
- It persists opaque public refs, actor scope, route surface, request
  fingerprint, optional client idempotency key, and opaque internal refs.
- It does not submit turns, inspect ProductWorkflow internals, bind listeners,
  call v1 gateway code, proxy LLM requests, or reach into Reborn composition.

## Storage Shape

The durable adapter stores CAS-protected JSON records under:

```text
/engine/openai_compat/refs/by_public_id/{chat_completions|responses}/{public_id}.json
/engine/openai_compat/refs/by_idempotency/{surface}/{digest}.json
```

Public-id records store one mapping each, so lookup/bind operations touch only
the requested ref. Idempotency records index authenticated actor scope, route
surface, and client idempotency key to the reserved public ref. The records store
metadata and opaque refs only; they must never contain raw prompts, response
payloads, event cursors, host paths, backend error details, secrets, or concrete
thread/run objects.

If this grows hot enough to need backend-native secondary indexes, preserve the
same `OpenAiCompatRefStore` behavior first and move indexing behind this crate
rather than changing the contract crate.

## Validation

Run targeted checks from the workspace root:

```bash
cargo test -p ironclaw_reborn_openai_compat_storage
cargo clippy -p ironclaw_reborn_openai_compat_storage --all-targets --all-features -- -D warnings
cargo test -p ironclaw_architecture reborn_crate_dependency_boundaries_hold
```
