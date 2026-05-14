# ironclaw_product_adapter_runtime_catalog Agent Notes

- This crate owns the runtime-facing read model over `ProductAdapterRegistryStore`.
- It may read registry manifests/installations and shape enabled adapter snapshots for runtime composition.
- Do not persist registry state here; durable storage belongs to a later registry-store backend.
- Do not load WASM components, route webhooks, perform HTTP egress, resolve raw secret material, or depend on dispatcher/runtime crates in this slice.
- Only explicitly `Enabled` installations may be surfaced.
- Credential bindings must remain opaque `SecretHandle`s; never add raw secret fields.
- Validation runs:
  - `cargo test -p ironclaw_product_adapter_runtime_catalog`
  - `cargo clippy -p ironclaw_product_adapter_runtime_catalog --all-targets -- -D warnings`
  - `cargo test -p ironclaw_architecture reborn_crate_dependency_boundaries_hold`
