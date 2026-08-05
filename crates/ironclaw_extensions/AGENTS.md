# Agent Map — ironclaw_extensions

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
- The host-API manifest contract projection (`v2`): `HostApiContractRegistry`, `HostApiManifestContract`, `HostApiRefV2`, `HostApiManifestProjection`; plus the capability-provider host-API contract (`host_api/capability_provider`) and the **default registry** that enumerates it, `default_host_api_contract_registry` (`host_api/mod`, moved down from `ironclaw_host_runtime` in WS3 row 3, PROPOSAL §6.5.9). A new built-in manifest contract is registered *there*, beside the contracts it names — not in a kernel caller.
- Crate-local public API, tests, and fixtures needed to prove that ownership.

## Do Not Move In Here

- direct authority grants or runtime-specific execution logic; use capabilities/authorization/trust and lane crates.
- Secrets, raw host paths, backend error details, and unredacted user content in errors, events, snapshots, logs, or docs.

## Validation

- Fast local check: `cargo test -p ironclaw_extensions`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture`
- If production persistence behavior changes, add/maintain PostgreSQL and libSQL parity tests.

## Agent Notes

- Keep edits inside this crate unless a contract explicitly requires a neighboring crate change.
- Prefer caller-level tests when a helper gates dispatch, persistence, network, secrets, approvals, resources, events, or process side effects.
- If the contract and code disagree, stop and treat the task as a contract-change request instead of silently changing ownership.
