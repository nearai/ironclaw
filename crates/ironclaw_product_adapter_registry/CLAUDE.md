# ironclaw_product_adapter_registry guardrails

Owns ProductAdapter host-api projection contracts for IronClaw Reborn.

- ProductAdapter declarations live in the single Extension Manifest v2 as
  `ironclaw.product_adapter/v1` host-api sections. This crate owns the typed
  ProductAdapter projection; generic extension manifests, installation state,
  activation state, credential-handle bindings, and health snapshots live in
  `ironclaw_extensions`. This crate must not introduce a second ProductAdapter
  TOML manifest format.
- Do **not** load WASM components, perform HTTP egress, route webhooks,
  resolve secret material, or write durable database state from this crate.
- Do **not** introduce an env-var adapter list (no `REBORN_PRODUCT_ADAPTERS`
  primary declaration path). Registry state is the source of truth.
- Do **not** depend on legacy `ChannelsConfig`, `ExtensionManager`,
  v1 WASM channel storage, or any runtime/dispatcher crate. The architecture
  boundary test in `crates/ironclaw_architecture` enforces this.
- Credential bindings store `ironclaw_host_api::SecretHandle` only. Raw
  secret material must never be stored, accepted, serialized, or logged.
- ProductAdapter host-api section projection must:
  - reject unknown TOML fields (`#[serde(deny_unknown_fields)]`),
  - reject inline secret material (key denylist + value heuristics),
  - validate every egress credential handle is declared in
    `required_credentials`,
  - keep `(host, credential_handle)` pairs distinct.
- ProductAdapter runtime projection must keep the cross-write invariant at
  read time: every surfaced installation must remain valid against its
  registered manifest and current ProductAdapter sections.
- Health updates are generic extension installation state and use
  `ironclaw_extensions::ExtensionHealthMessage` for redacted debug output.

## Tests

- Unit tests in `src/**/mod tests {}` cover validation helpers.
- Integration tests in `tests/registry_contract.rs` pin projection over the
  generic extension store: default-empty, explicit activation, undeclared
  ProductAdapter credential binding, egress pair preservation, manifest hash
  mismatch, redacted health, and cross-write invariant maintenance.
- Integration tests in `tests/manifest_ingestion.rs` cover manifest
  parsing, unknown-field rejection, inline-secret rejection, and egress
  credential validation.
- `cargo test -p ironclaw_architecture reborn_crate_dependency_boundaries_hold`
  pins crate dependency boundary.
