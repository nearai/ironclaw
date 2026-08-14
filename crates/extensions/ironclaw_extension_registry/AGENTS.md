# Agent Map — ironclaw_extension_registry

Canonical working-rules file for this crate (`CLAUDE.md` here is a pointer;
consolidated 2026-08-05 per `docs/internal/reborn/guidance-conventions.md` rule 1).

## Start Here

- Read `README.md` for orientation, `Cargo.toml` for actual dependencies.
- Use these Reborn contracts as the source of truth before changing behavior:
  - `docs/internal/reborn/contracts/extensions.md`
  - `docs/internal/reborn/contracts/kernel-boundary.md`
  - `docs/internal/reborn/contracts/capability-access.md`

## What This Crate Owns

- Declarative extension manifest, registry, lifecycle vocabulary, and trust
  inputs — no execution, network, secrets, or WASM/script/MCP inspection.
- **Layer `substrates`** (WS2, PROPOSAL §6.8.1). Its dependencies are therefore
  restricted to `contracts` and `substrates`: `ironclaw_extension_contracts`,
  `ironclaw_host_api`, `ironclaw_filesystem`. `ExtensionPackage::trust_policy_input`
  builds an `ironclaw_host_api::trust::TrustPolicyInput` — this crate does
  **not** depend on `ironclaw_trust` and must not regain that dependency; the
  policy engine that consumes the input sits above it.
- Manifest discovery/validation and asset-path containment: `ExtensionError`,
  `ExtensionAssetPath` (`lib.rs`); the in-memory `ExtensionRegistry` (`registry`).
- Lifecycle vocabulary: `ExtensionLifecycleEvent`, `ExtensionLifecycleEventSink`,
  `ExtensionLifecycleService` (`lifecycle`).
- The v2 manifest schema (`v2`): `ExtensionManifestV2`, `CapabilityDeclV2`,
  `ExtensionRuntimeV2`, `ManifestSource`, `CapabilityVisibility`,
  `ManifestV2Error`, and the schema-version/size constants. The v3 wire schema
  (`v3`) and the resolved+digested form (`resolved.rs`) the rest of the system
  reads instead of re-parsing TOML.
- The durable records (`installations.rs`): installation, membership,
  credential-binding, and **registered package definitions** — rows admitted to
  the catalog with zero installations, carrying their own retention policy
  (`definition_admission.rs`). Four record classes, CAS-mutated.
- The host-API manifest contract projection (`host_api/`):
  `HostApiContractRegistry`, `HostApiManifestContract`, `HostApiRefV2`,
  `HostApiManifestProjection`; the built-in contracts
  (`host_api/capability_provider`, `host_api/product_adapter`) and
  `default_host_api_contract_registry`. A new built-in manifest contract is
  registered *there*, beside the contracts it names — not in a kernel caller.
- `host_api/product_adapter` (arrived with WS5 from
  `ironclaw_assistant::adapter_registry`): the `ironclaw.product_adapter/v1`
  contract, `parse_product_adapter_manifest_record`/`product_adapter_sections`,
  the raw-TOML inline-secret guard, and `ProductAdapterHostApiSection`. The
  declared section **schema** is not here — it is
  `ironclaw_extension_contracts::product_adapter_section` (§6.1.2). Reached at
  `ironclaw_extension_registry::host_api::product_adapter::…`, deliberately not
  re-exported from the crate root (§11.2.4's one-import-path rule).
- Crate-local public API, tests, and fixtures needed to prove that ownership.

## Guardrails

- Keep this crate declarative. Do not execute tools, resolve authorization,
  perform network I/O, read secrets, spawn processes, or inspect
  WASM/script/MCP payloads here (`hosted_mcp_discovery.rs` transforms an
  already-fetched tool slice; the fetch lives in `ironclaw_extension_host`).
- Preserve manifest validation as fail-closed and stable: unknown/invalid
  capability ids, provider mismatches, malformed paths, duplicate capabilities,
  or unsupported runtime shapes are errors, never papered over.
- Keep package roots virtual-path based; no raw host paths or product-specific
  workspace assumptions. Registry lookups stay deterministic and side-effect
  free; callers own trust, visibility filtering, and execution policy.
- `ResolvedExtensionManifest.root_binding: PackageRootBinding` (`src/resolved.rs`)
  is persisted inside `WireManifestRecord`. `Materialized(root)` is the ordinary
  filesystem-backed shape; `Virtual` is reserved for validated remote-only
  packages such as user-registered hosted MCP servers; `FabricateOnLoad` exists
  only while migrating legacy rows and must be resolved before filesystem or
  trust consumers run. `ExtensionManifestRecord::from_toml_with_root_binding`
  forces admission callers to select that shape explicitly. Trust identity comes
  from `ExtensionPackage::trust_policy_source()` so publication and invocation
  cannot derive different package sources.

## Do Not Move In Here

- Direct authority grants or runtime-specific execution logic; use
  capabilities/authorization/trust and lane crates.
- Secrets, raw host paths, backend error details, or unredacted user content in
  errors, events, snapshots, logs, or docs.

## Validation

- Fast local check: `cargo test -p ironclaw_extension_registry`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture_tests`
- Registration-pipeline vocabulary containment (this crate's `src/hosted_mcp_`
  scope is one of the gate's two): `reborn_registration_pipeline_boundary.rs`.
- If production persistence behavior changes, add/maintain PostgreSQL and
  libSQL parity tests.

## Agent Notes

- Keep edits inside this crate unless a contract explicitly requires a
  neighboring crate change.
- Prefer caller-level tests when a helper gates dispatch, persistence, network,
  secrets, approvals, resources, events, or process side effects.
- If the contract and code disagree, stop and treat the task as a
  contract-change request instead of silently changing ownership.
