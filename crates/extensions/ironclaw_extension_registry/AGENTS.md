# Agent Map — ironclaw_extension_registry

## Start Here

- Read `CLAUDE.md` first; it is the crate-local guardrail file.
- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these Reborn contracts as the source of truth before changing behavior:
- `docs/reborn/contracts/extensions.md`
- `docs/reborn/contracts/kernel-boundary.md`
- `docs/reborn/contracts/capability-access.md`

## What This Crate Owns

- Declarative extension manifest, registry, lifecycle, and trust inputs (no execution, network, secrets, or WASM/script/MCP inspection), currently:
- **Layer `substrates`** (WS2, PROPOSAL §6.8.1). Its dependencies are therefore restricted to `contracts` and `substrates`: `ironclaw_extension_contracts`, `ironclaw_host_api`, `ironclaw_filesystem`. `ExtensionPackage::trust_policy_input` builds an `ironclaw_host_api::trust::TrustPolicyInput` — this crate does **not** depend on `ironclaw_trust` and must not regain that dependency; the policy engine that consumes the input sits above it.
- Manifest discovery/validation and asset-path containment: `ExtensionError`, `ExtensionAssetPath` (`lib.rs`); the in-memory `ExtensionRegistry` (`registry`).
- Lifecycle: `ExtensionLifecycleEvent`, `ExtensionLifecycleEventSink`, `ExtensionLifecycleService` (`lifecycle`).
- The v2 manifest schema (`v2`): `ExtensionManifestV2`, `CapabilityDeclV2`, `ExtensionRuntimeV2`, `ManifestSource`, `CapabilityVisibility`, `ManifestV2Error`, and the schema-version/size constants.
- The host-API manifest contract projection (`v2`): `HostApiContractRegistry`, `HostApiManifestContract`, `HostApiRefV2`, `HostApiManifestProjection`; plus the built-in host-API contracts (`host_api/capability_provider`, `host_api/product_adapter`) and the **default registry** that enumerates the capability-provider one, `default_host_api_contract_registry` (`host_api/mod`, moved down from `ironclaw_host_runtime` in WS3 row 3, PROPOSAL §6.5.9). A new built-in manifest contract is registered *there*, beside the contracts it names — not in a kernel caller.
- `host_api/product_adapter` (arrived with WS5 from `ironclaw_assistant::adapter_registry`, PROPOSAL §6.8.1): the `ironclaw.product_adapter/v1` contract, `parse_product_adapter_manifest_record`/`product_adapter_sections`, the raw-TOML inline-secret guard, and `ProductAdapterHostApiSection` — a resolved section paired with the `ManifestSectionPath` it was declared at. The declared section **schema** is not here: it is `ironclaw_extension_contracts::product_adapter_section` (§6.1.2), the same split `[channel]` already has. This module is reached at `ironclaw_extension_registry::host_api::product_adapter::…` and is deliberately **not** re-exported from the crate root — §11.2.4's one-import-path rule.
- Crate-local public API, tests, and fixtures needed to prove that ownership.

## Do Not Move In Here

- direct authority grants or runtime-specific execution logic; use capabilities/authorization/trust and lane crates.
- Secrets, raw host paths, backend error details, and unredacted user content in errors, events, snapshots, logs, or docs.

## Validation

- Fast local check: `cargo test -p ironclaw_extension_registry`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture_tests`
- If production persistence behavior changes, add/maintain PostgreSQL and libSQL parity tests.

## Agent Notes

- Keep edits inside this crate unless a contract explicitly requires a neighboring crate change.
- Prefer caller-level tests when a helper gates dispatch, persistence, network, secrets, approvals, resources, events, or process side effects.
- If the contract and code disagree, stop and treat the task as a contract-change request instead of silently changing ownership.
