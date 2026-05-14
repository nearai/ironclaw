# ironclaw_product_adapter_registry guardrails

Owns ProductAdapter installation registry contracts for IronClaw Reborn.

- This crate is the typed Reborn analog of v1 channel manifest + activation
  state. It models adapter manifests, installations, activation state,
  credential-handle bindings, and health snapshots; nothing else.
- Do **not** load WASM components, perform HTTP egress, route webhooks,
  resolve secret material, or write durable database state from this crate.
- Do **not** introduce an env-var adapter list (no `REBORN_PRODUCT_ADAPTERS`
  primary declaration path). Registry state is the source of truth.
- Do **not** depend on legacy `ChannelsConfig`, `ExtensionManager`,
  v1 WASM channel storage, or any runtime/dispatcher crate. The architecture
  boundary test in `crates/ironclaw_architecture` enforces this.
- Credential bindings store `ironclaw_host_api::SecretHandle` only. Raw
  secret material must never be stored, accepted, serialized, or logged.
- ProductAdapter manifest parsing must:
  - reject unknown TOML fields (`#[serde(deny_unknown_fields)]`),
  - reject inline secret material (key denylist + value heuristics),
  - validate every egress credential handle is declared in
    `required_credentials`,
  - keep `(host, credential_handle)` pairs distinct.
- Registry mutations must keep the cross-write invariant: every stored
  installation must remain valid against its registered manifest. When a
  manifest is replaced, existing installations must be re-validated; when
  an installation is enabled, it must be re-validated against the current
  manifest at the time of the activation transition.
- Health updates must redact provider/runtime internals through
  `ironclaw_product_adapters::RedactedString`.

## Tests

- Unit tests in `src/**/mod tests {}` cover validation helpers.
- Integration tests in `tests/registry_contract.rs` pin store invariants:
  default-empty, explicit activation, undeclared credential binding,
  egress pair preservation, manifest hash mismatch, redacted health, and
  cross-write invariant maintenance.
- Integration tests in `tests/manifest_ingestion.rs` cover manifest
  parsing, unknown-field rejection, inline-secret rejection, and egress
  credential validation.
- `cargo test -p ironclaw_architecture reborn_crate_dependency_boundaries_hold`
  pins crate dependency boundary.
