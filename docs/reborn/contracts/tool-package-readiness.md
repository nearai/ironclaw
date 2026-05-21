# Tool Package Readiness Contract

**Status:** Draft implementation contract
**Date:** 2026-05-21
**Depends on:** [`extensions.md`](extensions.md), [`host-api.md`](host-api.md), [`capabilities.md`](capabilities.md), [`dispatcher.md`](dispatcher.md), [`runtime-profiles.md`](runtime-profiles.md), [`runtime-selection.md`](runtime-selection.md)

---

## 1. Purpose

This contract freezes the **gate** that any concrete Reborn tool package must pass before downstream lanes (Native Memory, Notion MCP, GitHub WASM, Google Suite, Slack, and any future package) can consider it "Reborn-ready."

It answers exactly one question:

```text
Given an extension package, is it model-ready, dispatch-ready, and lifecycle-ready?
```

It does **not** pick a runtime lane for any specific tool. Each downstream lane is free to choose `wasm`, `script`, `mcp`, or — for host-bundled packages only — `first_party` / `system`, provided the package passes every requirement below for the chosen lane.

Downstream issues `Depends on:` this document instead of redefining their own catalog/manifest requirements. If a downstream lane needs to relax a requirement here, that is a contract-change request against this file, not implementation work.

---

## 2. What this gate certifies (and what it does not)

This gate certifies that a package can be:

- discovered from `/system/extensions/<id>/manifest.toml`;
- installed through `ExtensionLifecycleService::install`;
- projected into the hot capability catalog through `publish_hot_capability_catalog`;
- dispatched through `CapabilityDispatcher::dispatch_json` for its declared runtime lane;
- removed through `ExtensionLifecycleService::remove` so its capabilities disappear from the next catalog publication.

This gate does **not** certify:

- effective trust class (owned by `ironclaw_trust` policy evaluation);
- credential issuance, rotation, or revocation (owned by Lane 3 — see §9);
- network egress allowlisting (owned by `ironclaw_network`);
- resource quotas beyond manifest-declared `resource_profile` (owned by `ironclaw_resources`);
- approval/lease flows (owned by `ironclaw_approvals`);
- product workflow binding (owned by `ironclaw_product_workflow`).

A package that passes this gate is **eligible** for production wiring. It is not yet **authorized** for it.

---

## 3. Per-package readiness checklist

For every capability the package declares, the following must hold at the manifest-source layer before any runtime wiring is considered:

1. **Manifest uses the host-API capability-provider shape.** For `ManifestSource::InstalledLocal` and `ManifestSource::RegistryInstalled`, capabilities live under `[[capability_provider.tools.capabilities]]` referenced by a `[[host_api]] id = "ironclaw.capability_provider/v1"` entry. Production discovery — `ExtensionDiscovery::discover_with_manifest_contracts`, which routes through `ExtensionManifestV2::parse_with_optional_host_api_contracts` — rejects top-level `[[capabilities]]` for these sources with `ManifestV2Error::LegacyTopLevelCapabilitiesForInstalledSource`. The bare `ExtensionManifestV2::parse` does not enforce this rejection and is reserved for legacy v1-envelope test fixtures and host-bundled tooling; package readiness is measured against the discovery entry point, not the bare parser.

2. **Input and output schemas resolve.** Both `input_schema_ref` and `output_schema_ref` resolve to package-local files containing JSON that validates as a JSON Schema. Resolution and validation happen during hot catalog publication. Unresolvable refs and structurally invalid schemas fail closed.

3. **Prompt documentation resolves when the capability is model-visible.** For `visibility = "model"`, `prompt_doc_ref` is mandatory, must resolve, and must be valid UTF-8 within the hot-catalog prompt size bound (`MAX_HOT_PROMPT_BYTES`). For `visibility = "host_internal"` or `visibility = "api"`, `prompt_doc_ref` is optional and ignored by the hot catalog.

4. **Runtime lane is declared and permitted for the manifest source.** `[runtime] kind` resolves to one of `wasm`, `script`, `mcp`, `first_party`, `system`. `first_party` and `system` are only permitted when `ManifestSource::HostBundled`; any other source declaring them fails at parse with `RuntimeForbiddenForSource`.

5. **Model-visible capabilities appear in the hot catalog after install.** Calling `publish_hot_capability_catalog(&fs, lifecycle.registry())` after `install` returns a `HotCapabilityRecord` keyed by the capability's `CapabilityId` for every capability declared with `visibility = "model"`, with `parameters_schema` replaced by the resolved input schema and `prompt_doc` populated. `host_internal` and `api` capabilities are validated by manifest parsing but are deliberately not projected into the hot catalog — see `crates/ironclaw_host_runtime/src/capability_catalog.rs::publish_package_capabilities`.

6. **Invocation routes through the host-runtime dispatch chain.** A `CapabilityDispatcher::dispatch_json` call with the capability's `CapabilityId` reaches a `RuntimeAdapter` registered for the capability's declared `RuntimeKind`, carrying the package's `ExtensionId` as the dispatched provider.

7. **Install/uninstall lifecycle is covered.** Calling `lifecycle.remove(extension_id)` followed by a fresh `publish_hot_capability_catalog` returns a catalog with the package's capabilities absent. Removal emits a redacted `ExtensionLifecycleEvent` to any composed `ExtensionLifecycleEventSink`; see §6 for the boundary with credential cleanup.

---

## 4. Per-runtime-lane requirements

The lane the package picks determines additional MUST/MUST-NOT requirements.

| Lane | MUST | MUST NOT |
|---|---|---|
| `wasm` | Declare a relative `module` asset path under the package root; pass `ExtensionAssetPath` validation (no absolute paths, no `..`, no host separators, no URLs). | Reference an absolute path, an HTTP URL, or any asset outside the package root. |
| `script` | Declare a semantic `runner`, `command`, and `args`. May declare a backend-specific `image` only when the chosen `runner` consumes it. | Embed raw Docker flags, host paths, or shell metacharacters that bypass the runner. |
| `mcp` | Declare `transport`. For stdio, declare `command` and optional `args`. For remote, declare `url`. | Connect, spawn, or open a transport during manifest parsing or registry insertion. |
| `first_party` | Declare a `service` name. Only valid when `ManifestSource::HostBundled`. | Be declared by installed third-party packages. |
| `system` | Declare a `service` name. Only valid when `ManifestSource::HostBundled`. Reserved for host-owned fixtures/services. | Be declared by installed third-party packages. Never user-installable. |

Lane choice does not by itself confer authority. A `wasm` capability is still subject to the same trust policy, capability access, approval lease, and resource reservation pipeline as a `script` capability.

---

## 5. Hot catalog projection requirements

The hot catalog is the model-facing surface. For every capability that passes §3, the projected `HotCapabilityRecord` must satisfy:

- **Stable tool name.** The capability ID is exactly the `CapabilityId` from the manifest, prefixed by the extension ID. Catalog publication does not rename, shorten, or namespace it further.
- **Resolved input schema.** `descriptor.parameters_schema` is the resolved JSON Schema object, not a `{ "$ref": "..." }` placeholder.
- **Resolved output schema.** `output_schema` is the resolved JSON Schema object retained alongside the descriptor.
- **Prompt doc present for model visibility.** `prompt_doc.is_some()` when `visibility == Model`. `None` is permitted for `HostInternal` and `Api`.
- **Description present.** The human description from the manifest is carried verbatim; empty descriptions are rejected by manifest validation.
- **No silent skips.** A capability that fails any of the above does not produce a degraded record — publication fails closed for the whole package.

---

## 6. Lifecycle invariants for ready packages

After install:

- the package is visible from `ExtensionRegistry::get_extension`;
- its capabilities are visible from `ExtensionRegistry::get_capability`;
- the next `publish_hot_capability_catalog` includes the model-visible subset.

After disable:

- the package remains in the registry;
- `ExtensionLifecycleService::is_enabled` returns `false`;
- runtime dispatch consumers may use this signal to refuse invocation, but the hot catalog projection is unchanged (disable is a soft state).

After enable:

- `is_enabled` returns `true`; the package is restored to the pre-disable state.

After remove:

- the package is absent from the registry;
- its capabilities are absent from the next `publish_hot_capability_catalog`;
- a redacted `ExtensionLifecycleEvent { operation: Remove, ... }` is delivered to any composed `ExtensionLifecycleEventSink`.

**Credential cleanup is out of scope for this gate.** The extensions crate does not delete secrets on remove. Credential lifecycle observers consume the `ExtensionLifecycleEvent` and decide their own retention policy; that wiring is owned by Lane 3 (production tool composition / secrets-egress substrate). A package passing this gate makes no promise about secret residency after removal.

---

## 7. Failure cases — all fail closed

Every case below must reject the package before any runtime side effect:

| Case | Stage | Error |
|---|---|---|
| Missing input/output schema file | hot catalog publication | `HostRuntimeError::invalid_request` ("missing input_schema_ref / output_schema_ref at …") |
| Schema file is structurally invalid JSON Schema | hot catalog publication | `HostRuntimeError::invalid_request` ("… must contain valid JSON schema") |
| Schema or prompt file exceeds size bound | hot catalog publication | `HostRuntimeError::invalid_request` ("… exceeds N bytes") |
| Model-visible capability missing `prompt_doc_ref` | manifest parse | `ManifestV2Error::MissingPromptDocRef` |
| Capability ID not prefixed with extension ID | manifest parse | `ManifestV2Error::CapabilityIdNotPrefixed` |
| Duplicate capability ID across packages | registry insert | `ExtensionError::DuplicateCapability` |
| `first_party` or `system` runtime declared by non-HostBundled source | manifest parse | `ManifestV2Error::RuntimeForbiddenForSource` |
| Legacy top-level `[[capabilities]]` from non-HostBundled source | production discovery (`parse_with_optional_host_api_contracts`) | `ManifestV2Error::LegacyTopLevelCapabilitiesForInstalledSource` |
| Unknown host port in `required_host_ports` | manifest parse | `ManifestV2Error::HostApiSectionRejected` ("unknown host port …") |
| Manifest asset path with `..`, absolute, URL, or control chars | manifest parse | `ExtensionError::InvalidAssetPath` |
| Manifest ID does not match package root directory | discovery | `ExtensionError::ManifestIdMismatch` |

A package fails this gate the moment any one of the above is reachable from its manifest. "Warn and continue" is not a permitted disposition.

---

## 8. Verification evidence

A package claiming readiness must have, at minimum:

- a unit test that parses its manifest through the production discovery entry point — `ExtensionManifestV2::parse_with_optional_host_api_contracts` (with the default `HostApiContractRegistry`) or via `ExtensionDiscovery::discover_with_manifest_contracts`. `parse_with_host_api_contracts` and the bare `parse` may be used for supplemental host-API or v1-envelope validation but are not the readiness evidence gate — a package can pass them with shape combinations that production discovery rejects;
- an integration test, per declared runtime lane, that exercises the full chain `discover → install → publish → dispatch → remove → publish` against a `LocalFilesystem`-mounted package fixture, with the dispatcher built from `lifecycle.registry()` (not the pre-install discovery registry) and `dispatch_json` reaching a recorded `RuntimeAdapter` for that lane. The post-remove publication must show the package's capabilities absent from the hot catalog;
- a negative test for at least one of §7's failure modes that is reachable from its manifest (most commonly missing schema or missing prompt doc).

The canonical reference fixture for the dispatch chain lives at:

```text
crates/ironclaw_host_runtime/tests/extension_v2_lifecycle_e2e.rs
```

That file demonstrates the chain for `script`, `wasm`, and `mcp` lanes and includes the `remove → catalog reflects` assertion. Downstream packages should mirror this shape rather than re-invent it.

---

## 9. Boundary — what is owned by adjacent lanes

This gate is intentionally narrow. The following are explicitly **not** part of readiness:

| Concern | Owner | Reference |
|---|---|---|
| Effective trust class assignment | `ironclaw_trust` policy evaluation | [`extensions.md`](extensions.md) §5, [`trust-boundary-hardening.md`](trust-boundary-hardening.md) |
| Credential issuance, leases, redaction | Lane 3 production tool composition; `ironclaw_secrets`; `ironclaw_host_runtime` | [`secrets.md`](secrets.md), [`capability-access.md`](capability-access.md) |
| Network egress allowlisting and policy | `ironclaw_network` | [`network.md`](network.md) |
| Resource reservation enforcement beyond declared profile | `ironclaw_resources` | [`resources.md`](resources.md) |
| Approval/lease state machines | `ironclaw_approvals`, `ironclaw_authorization` | [`approvals.md`](approvals.md), [`capability-access.md`](capability-access.md) |
| Hot-catalog event projection / streaming | Not specified in v1; see §10 | [`events-projections.md`](events-projections.md) |
| Product workflow binding (per-channel inbound) | `ironclaw_product_workflow`, `ironclaw_product_adapters` | [`product-adapters.md`](product-adapters.md) |

Downstream lane work that requires changes to any of these must coordinate with the owning contract, not amend this gate.

---

## 10. Non-goals (v1)

- **Catalog publication is not an event source in v1.** `publish_hot_capability_catalog` is an idempotent rebuild-on-demand function over an `ExtensionRegistry` snapshot. There is no `EventKind` for "catalog mutated"; downstream consumers (WebUI, agent loop) rebuild the catalog when they need a fresh view. Promoting catalog publication to a projection-eligible source log is explicitly out of scope here.
- **No per-tool shape lock-in.** This contract does not decide whether Native Memory is `first_party` vs host-bundled `[[capabilities]]`, whether GitHub ships as `wasm` vs `mcp`, or whether Google Calendar and Gmail share an extension. Those choices belong to each downstream lane and are evaluated against this gate.
- **No marketplace / install UX.** Discovery, install, and remove are exercised through `ExtensionLifecycleService`. End-user install flows, signature verification beyond manifest validation, and registry catalogs are downstream of this gate.
- **No upgrade / migration logic.** Replacing one validated package with another of the same ID is supported via `ExtensionLifecycleService::update`; cross-version data migration is not part of this contract.

---

## 11. Acceptance for downstream lanes

A downstream lane (Notion MCP, GitHub WASM, Native Memory, Google Suite, Slack, …) is "Lane 2-ready" when:

1. Its manifest parses against `ExtensionManifestV2::parse_with_host_api_contracts` using the default `HostApiContractRegistry`.
2. Its package satisfies every bullet in §3.
3. Its capability projects through `publish_hot_capability_catalog` per §5.
4. Its dispatch chain matches the reference fixture in §8.
5. Its tests cover at least one failure mode from §7.

Lane 2-ready does not imply production-ready. It implies the package is no longer blocked by the extensions/host-runtime substrate.
