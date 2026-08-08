# ironclaw_memory

The provider-neutral memory contract: the single `MemoryService` trait every
memory provider implements and every memory caller depends on, plus the
document path/scope grammar, prompt-write-safety vocabulary, audit-event
contracts, and the shared conformance suite that proves providers
interchangeable. Deliberately the family's thinnest crate — the seam is the
point.

- **Family / layer:** `domains` / `substrates` · **Package:** `ironclaw_memory` · **Manifest:** `crates/domains/ironclaw_memory/Cargo.toml`
- **Use this when:** changing what memory *means* — the service operations,
  the `/memory` path grammar, write-safety vocabulary, or the conformance
  suite; or when a caller needs to talk to memory without caring which
  provider serves it.
- **Don't use this when:** implementing or fixing a concrete provider → the
  extension packages `crates/extensions/packages/memory-native/`
  (`ironclaw_memory_native`) and `crates/extensions/packages/mem0/`
  (`ironclaw_memory_mem0`); wiring a provider into a binary →
  `ironclaw_composition` / the binary.

## Public surface

- `MemoryService` (in `service`) — the provider seam and its operation
  request/response DTOs; `ironclaw.memory.*` is the naming convention for
  tools built on it.
- Path/scope grammar: `MemoryDocumentScope`, `MemoryDocumentPath` (validating
  constructors; fail-closed), `MemoryContext`.
- Prompt-write-safety vocabulary (`safety`): operation, source, severity,
  reason codes, policy trait, event sink — the *shape* of what a provider must
  enforce; the enforcement engine lives in the providers.
- Metadata + hashing helpers (`metadata`, `hash`); significant-event/audit
  contracts (`events`).
- `test_support` — the shared conformance suite every provider wires against.

## Depends on / consumed by

- **Normal deps (measured):** `ironclaw_host_api` — and nothing else internal.
  The armed allowlist also permits `ironclaw_prompt_envelope`, which the crate
  does not currently use; anything further is a gate failure, not a choice.
- **Consumed by (6):** `ironclaw_composition`, `ironclaw_host_runtime`,
  `ironclaw_loop_host`, `ironclaw_memory_mem0`, `ironclaw_memory_native`,
  `ironclaw_turn_runner`. Providers depend on this crate, never the reverse.

## Invariants

- **Allowlist-enforced neutrality:** `reborn_crate_dependency_boundaries_hold`
  forbids every internal crate outside `{ironclaw_host_api,
  ironclaw_prompt_envelope}`.
- **No crate outside the provider packages and the binary names a provider:**
  `only_the_sanctioned_residue_names_a_memory_provider`
  (`reborn_dependency_boundaries.rs`, shrink-only residue ledger).
- Retired memory vocabulary stays retired:
  `reborn_memory_retired_vocabulary.rs`.
- Value-type constructors validate at the boundary; no unchecked public
  constructors (see [`AGENTS.md`](./AGENTS.md)).

## Tests

```bash
cargo test -p ironclaw_memory
```

Provider conformance runs inside each provider package's suite via this
crate's `test_support`.

## See also

- Working rules: [`AGENTS.md`](./AGENTS.md) (canonical crate guidance).
- Family boundary: [`../AGENTS.md`](../AGENTS.md).
- Design record: `families/domains.md`, PROPOSAL §6.4.4 (provider packages:
  `families/extensions.md`).
