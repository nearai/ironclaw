# memory-native — the default `[memory]` provider

The bundled memory provider: a filesystem-backed implementation of the
provider-neutral `ironclaw_memory::MemoryService` contract, shipped and
installed by default so memory is always available. Extension id:
`ironclaw.memory`.

- **Surfaces:** `[memory]` provider + 5 memory tools (`ironclaw.memory.read` / `.write` / `.search` / `.tree` / `.profile_set`)
- **Vendor (credential authority):** none — no `[auth.*]` recipe
- **Runtime:** `first_party`
- **Code:** crate `ironclaw_memory_native` (this directory: `Cargo.toml`, `src/`, `tests/`, `manifest.toml`, `prompts/`, `schemas/`)
- **Linked by:** the binary only, like every package crate — plus one recorded
  kernel residue: `ironclaw_host_runtime` holds a normal dep (its bundled-memory
  package builder, `crates/kernel/ironclaw_host_runtime/src/memory_native_extension.rs`),
  an exception to §8.2's "only the provider packages and the binary name a
  memory provider" rule that PROPOSAL §6.8.4 records as a pending port
  inversion, not a move
- **Tests:** `cargo test -p ironclaw_memory_native` — includes the shared
  `MemoryService` conformance suite the mem0 package also runs

Exactly one `[memory]` provider is active per deployment; the alternative is
[`../mem0`](../mem0). Working rules: [`AGENTS.md`](./AGENTS.md). Family model:
`crates/extensions/AGENTS.md`.
