# ProductAdapter Runtime Catalog Design

## Goal

Add a small runtime-facing read model that proves the ProductAdapter registry contract can drive runtime composition without expanding the registry crate's responsibilities.

## Scope

This PR adds a registry-backed ProductAdapter runtime catalog. The catalog reads from `ProductAdapterRegistryStore` and returns deterministic snapshots of enabled adapter installations for runtime consumers.

Out of scope:

- durable libSQL/PostgreSQL registry persistence
- webhook routing or inbound execution
- WASM component loading
- env-var adapter declarations
- raw secret material access

## Architecture

Create a new crate, `ironclaw_product_adapter_runtime_catalog`, because the registry crate must remain contracts-only and must not depend on runtime, WASM, dispatcher, or routing crates.

The crate exposes:

- `ProductAdapterRuntimeCatalog<S>`: generic wrapper over a `ProductAdapterRegistryStore`
- `ProductAdapterRuntimeEntry`: enabled installation plus its registered manifest data
- `ProductAdapterRuntimeCatalogError`: catalog-specific read/consistency errors

The catalog depends only on `ironclaw_product_adapter_registry` and product-adapter DTO crates. It does not load WASM or route webhooks.

## Data Flow

1. Caller provides a registry store.
2. Catalog calls `list_enabled_installations()`.
3. For each enabled installation, catalog fetches the corresponding manifest with `get_manifest()`.
4. Catalog returns sorted `ProductAdapterRuntimeEntry` values.
5. Missing manifests are reported as consistency errors instead of silently dropping entries.

## Invariants

- Only explicitly `Enabled` installations appear.
- Store remains the source of truth.
- Results are sorted by installation id for deterministic runtime wiring.
- Entries carry `SecretHandle` bindings only; no raw secret material is accepted or exposed.
- Registry crate remains free of runtime/WASM dependencies.

## Tests

Add integration/unit coverage for:

- empty registry yields empty catalog
- installed and disabled installations are excluded
- enabled installation appears with manifest/component/auth/egress data
- missing manifest for an enabled installation returns an error
- output order is deterministic
- credential validation remains enforced by registry before catalog reads

## Follow-ups

- Durable libSQL/PostgreSQL store with shared registry contract tests.
- Runtime/WASM bridge that consumes catalog entries.
- Webhook routing that uses catalog output as its activation source.
