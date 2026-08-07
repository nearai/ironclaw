# ironclaw_extension_manager

The **product face** of extensions — everything a user or operator does to
discover, install, configure, and pair one: lifecycle commands and
agent-callable capabilities, the lifecycle product service the WebUI routes
through, admin/operator capability handlers, credential setup views, and the
IronHub extension hub. Split out of `ironclaw_extension_host` in WS2.4 so the
host could drop to the `loops` layer: the manager owns UX semantics, the host
keeps lifecycle *authority*, and the edge between them is one-way.

- **Family / layer:** `extensions` / `products` · **Package:** `ironclaw_extension_manager` · **Manifest:** `crates/extensions/ironclaw_extension_manager/Cargo.toml`
- **Use this when:** changing what extension management looks like or exposes — commands, capabilities, views, the hub.
- **Don't use this when:** you need to *change* lifecycle behavior (→ `ironclaw_extension_host::ExtensionLifecycleManager` — this crate never mutates installation state itself), record shapes (→ `ironclaw_extension_registry`), or conversation UX (→ `ironclaw_assistant`).

## Public surface

`ExtensionHostLifecycleProductService` (the `LifecycleProductService` port),
`RebornChannelConfigProductService` (product projection over the host's
`ChannelConfigService`), the lifecycle capabilities/command
(`extension_lifecycle_capabilities`, `extension_lifecycle_command`),
admin/operator/skill-auto-activate capability handlers, credential views
(`webui_extension_credentials`), and `ironhub/` (search / info / install). The
module table lives in [`CLAUDE.md`](./CLAUDE.md).

## Depends on / consumed by

Depends on the family (`extension_host`, `extension_registry`; dev + optional
`extension_support`), contracts, and a transitional tail the split could not
drain (`ironclaw_assistant` — seven files of DTOs/constants/port residues,
frozen shrink-only; direct `auth`, `host_runtime`, `secrets`, `skills`, and
optional fixture deps). The target set is four crates
(`product_contracts`, `extension_contracts`, `extension_registry`,
`extension_host`); the gap list is honest in
`docs/reborn/target-architecture/families/extensions.md` §"ironclaw_extension_manager".
Consumed by `ironclaw_composition`, `ironclaw_cli`, and the root
integration-test package (dev). Re-derive with
`rg -l 'ironclaw_extension_manager = ' Cargo.toml crates/*/*/Cargo.toml`.

## Invariants

- **The manager calls the host; the host never depends on the manager** — any
  dependency kind, dev-dependencies included
  (`reborn_extension_manager_split.rs`, manifest *and* source level).
- The `ironclaw_assistant` file list is exact-match and shrink-only (same
  gate). Moving a symbol to `ironclaw_product_contracts` is the fix; adding a
  row is not.
- Never write installation state here — every mutation goes through the host's
  `ExtensionLifecycleManager`.

## Tests

```bash
cargo test -p ironclaw_extension_manager
cargo test -p ironclaw_architecture_tests reborn_extension_manager_split
```

## See also

[`CLAUDE.md`](./CLAUDE.md) — the canonical working rules for this crate (module
table, what deliberately stayed in the host, conventions) ·
`crates/extensions/AGENTS.md` — the family model.
