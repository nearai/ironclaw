# ironclaw_extension_support

The shared support crate for the bundled packages — **not itself a package**.
It holds the `PACKAGES` inventory (which package directories ship, embedded via
`include_str!`/`include_bytes!`) and the native tool *executors* that serve
many packages at once (`gsuite`, `web_access`, `coding`, `skills`). It is a
separate crate because it is the one sanctioned, scan-exempt home for vendor
names and a heavy native-tool dependency surface — keeping both out of the
vendor-blind host.

- **Family / layer:** `extensions` / `runtimes` · **Package:** `ironclaw_extension_support` · **Manifest:** `crates/extensions/ironclaw_extension_support/Cargo.toml`
- **Use this when:** adding native (non-WASM) tool logic for a data-only package, or shipping a new package's embed module + `PACKAGES` row.
- **Don't use this when:** the code is a capability handler, manifest declaration, or registry insertion (→ the host side of the seam: `ironclaw_host_runtime::first_party_tools` or the binary's `FirstPartyHandlerRegistrar`), a channel adapter or provider implementation (→ the package's own crate), or generic hosting (→ `ironclaw_extension_host`).

## Public surface

Executor entry points behind narrow request types this crate defines
(`GsuiteDispatchRequest`, `WebAccessDispatchRequest`, `SkillUrlFetchContext`,
…), returning this crate's own error types; and `packages::PACKAGES` — the
package inventory (11 entries; `nearai`'s embed module is deliberately outside
the table — the host patches its `[mcp].server` from operator configuration;
read `src/packages/nearai.rs`'s header). Executors reach the world only through
contracts-layer ports handed in per invocation (`RuntimeHttpEgress`,
`RootFilesystem`, `ResourceScope`, `CapabilityId`).

## Depends on / consumed by

Depends on `auth`, `extractors`, `filesystem`, `host_api`, `observability`,
`safety`, `skills` — all contracts/substrates, which is what lets a kernel
consumer exist. Consumed by `ironclaw_host_runtime` (a **designed** kernel
edge — the executor/adapter seam leaves handlers there), `ironclaw_extension_host`,
`ironclaw_extension_manager` (dev + optional), `ironclaw_composition`,
`ironclaw_cli`, and the root integration-test package (dev). Re-derive with
`rg -l 'ironclaw_extension_support = ' Cargo.toml crates/*/*/Cargo.toml`.

## Invariants

- **Executor, never handler:** may not name `ironclaw_host_runtime` or
  `ironclaw_extension_registry` (this crate's `BoundaryRule`,
  `reborn_dependency_boundaries.rs`) nor `ironclaw_assistant`/`ironclaw_product_contracts`.
- The `runtimes` demotion's consumer set is frozen by a `DowngradePin`
  (`reborn_same_layer_edge_inventory.rs`).
- First-party status raises no authority: every tool call crosses the same
  authorization/approval stages as any capability invocation.
- Committed WASM guests it embeds are keyed to their `wasm-src/` source:
  `python3 scripts/ci/check-wasm-artifact-freshness.py` (rebuild via
  `./scripts/build-wasm-extensions.sh --first-party`, then `--update`).

## Tests

```bash
cargo test -p ironclaw_extension_support
cargo test -p ironclaw_host_runtime --test first_party_coding_tools   # caller-side check
cargo test -p ironclaw_architecture_tests reborn_crate_dependency_boundaries_hold
```

## See also

[`AGENTS.md`](./AGENTS.md) — canonical working rules (the executor/adapter seam
in full, the packages-live-elsewhere note) · `crates/extensions/AGENTS.md` —
the family model and the package catalog.
