# mem0 — the alternative `[memory]` provider

The second memory provider: maps the provider-neutral
`ironclaw_memory::MemoryService` contract onto an external, self-hosted mem0
service's REST surface, behind a hardened transport seam (bounded timeout,
redirects disabled, target URL validated before any request leaves the
process). Installed per deployment *in place of* the native provider.
Extension id: `mem0.local.memory`.

- **Surfaces:** `[memory]` provider + 5 memory tools (the same `ironclaw.memory.*` ids the native provider declares)
- **Vendor (credential authority):** none — no `[auth.*]` recipe; endpoint configuration is deployment-side
- **Runtime:** `first_party`
- **Code:** crate `ironclaw_memory_mem0` (this directory: `Cargo.toml`, `src/`, `tests/`, `manifest.toml`)
- **Depends on:** `ironclaw_memory` + `ironclaw_host_api` only; its HTTP cone stays isolated to this package
- **Tests:** `cargo test -p ironclaw_memory_mem0` — the shared `MemoryService`
  conformance suite over a mock transport; `tests/live_local_mem0.rs` drives a
  locally running mem0 stack when one is available

The second independent implementation is what keeps the memory contract's
conformance suite honest. Family model: `crates/extensions/AGENTS.md`.
