# ironclaw_extension_registry

The manifest grammar and the record store of the extension system: what a
manifest can say (v3 wire schema, v2 internal form, resolved+digested form) and
what is durably recorded about it (installation, membership,
credential-binding, and registered-definition records). It is a separate crate
because it is a record authority with a genuine persistence obligation and a
grammar many crates read — and keeping it apart from the host keeps a stateful,
CAS-mutated store out of the crate whose other job is verifying inbound trust.

- **Family / layer:** `extensions` / `substrates` · **Package:** `ironclaw_extension_registry` · **Manifest:** `crates/extensions/ironclaw_extension_registry/Cargo.toml`
- **Use this when:** changing manifest schema/validation/resolution, or what is durably recorded about installs and admitted definitions.
- **Don't use this when:** you need execution, verification, or lifecycle *transitions* (→ `ironclaw_extension_host` — the registry records, the host is the sole state-transition writer), management UX (→ `ironclaw_extension_manager`), or shared adapter vocabulary (→ `ironclaw_extension_contracts`).

## Public surface

`ExtensionRegistry` (in-memory catalog), `ExtensionManifestV2`/`v3`/
`ResolvedExtensionManifest` (+ digest), the installation-record store
(`installations.rs`, four record classes incl. registered package definitions
with `PackageDefinitionRetention`), lifecycle vocabulary
(`ExtensionLifecycleEvent`/`Sink`/`Service`), and the host-API manifest
contract projection (`host_api/`, incl. `host_api::product_adapter` — reached
by full path, deliberately not re-exported at the root).

## Depends on / consumed by

Depends only on `ironclaw_extension_contracts`, `ironclaw_host_api`, and
`ironclaw_filesystem` (contracts + substrates — the layer demands it). Consumed
widely: the host, the manager, composition, and kernel/lanes/loop/events crates
that read manifest vocabulary (`capabilities`, `host_runtime`, `turn_runner`,
`wasm`, `sandbox`, `mcp`, `event_projections`). Re-derive with
`rg -l 'ironclaw_extension_registry = ' Cargo.toml crates/*/*/Cargo.toml`.

## Invariants

- Declarative only — no execution, network, secrets, or payload inspection;
  side-effect-free registry half. Enforced by this crate's `BoundaryRule` in
  `reborn_dependency_boundaries.rs` and by review against `AGENTS.md`.
- No vendor names outside `#[cfg(test)]` (`reborn_extension_specificity.rs`).
- Hosted-MCP registration vocabulary confined to `src/hosted_mcp_`
  (`reborn_registration_pipeline_boundary.rs`).

## Tests

```bash
cargo test -p ironclaw_extension_registry     # incl. tests/: manifest_v2/v3, installations,
                                              # product_adapter contract + ingestion, discovery
cargo test -p ironclaw_architecture_tests     # boundary gates
```

## See also

[`AGENTS.md`](./AGENTS.md) — the canonical working rules ·
`crates/extensions/AGENTS.md` — the family model ·
`docs/internal/reborn/target-architecture/families/extensions.md` §"ironclaw_extension_registry".
