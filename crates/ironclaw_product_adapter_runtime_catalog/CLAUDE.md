# ironclaw_product_adapter_runtime_catalog guardrails

Owns a narrow runtime-facing catalog/read model for ProductAdapter activation.

- The registry remains the source of truth. This crate reads `ProductAdapterRegistryStore`; it does not own persistence.
- Surface only enabled installations. Installed or disabled adapters must not appear in runtime catalog output.
- Keep output deterministic so startup/runtime wiring is stable across runs.
- Do not load WASM, route webhooks, perform HTTP egress, resolve secret material, or own durable DB behavior from this crate.
- Keep credential bindings as opaque `SecretHandle`s only.
- Missing manifests for enabled installations are consistency errors, not silent skips.
- Prefer small DTOs and explicit errors over stringly-typed runtime state.

## Tests

- Unit/integration tests should drive the catalog API, not private helpers.
- Cover default-empty, default-off, explicit enabled visibility, deterministic ordering, missing-manifest error, and registry validation pass-through expectations.
