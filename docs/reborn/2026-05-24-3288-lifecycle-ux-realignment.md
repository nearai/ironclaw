# #3288 Reborn lifecycle UX realignment

> **Historical note (superseded July 2026).** The shipping Reborn extension
> lifecycle is now `uninstalled` -> `setup_needed` or `active`: install joins
> caller membership and the host reconciles readiness automatically. There is
> no public extension Activate/Disable route, command, capability, or browser
> action. The current contract lives in
> [`extension-runtime/overview.md`](extension-runtime/overview.md) and its
> [`checklist.md`](extension-runtime/checklist.md); the inventory below records
> the earlier #3288 staging state only.

## Purpose

Issue #3288 is the lifecycle product-surface slice for extensions, skills, MCP,
and WASM. The current `reborn-integration` branch has advanced runtime,
composition, WebUI, model, and first-party tool work beyond that first slice.
This note re-anchors the lifecycle UX work so follow-up PRs do not treat those
vertical slices as a completed lifecycle migration.

## Contract boundary

Lifecycle product code owns package/install/readiness projection and routes
canonical lifecycle commands through `ProductWorkflow`.

It does not own auth, approval, pairing, policy, credential storage, MCP
processes, WASM runtimes, runtime execution, or capability publication. Those
remain with their existing Reborn services. Lifecycle projections use:

- `LifecyclePhase` for package/install state only;
- `LifecycleReadinessBlocker` for setup/auth/pairing/approval/policy/
  credential/runtime requirements;
- redacted refs only when pointing to an owning blocker service.

Do not add `auth_required`, `pairing_required`, or approval states as lifecycle
phases. Product surfaces may render blockers however they choose, but the source
of truth remains the owning service.

## Current behavior inventory

| Surface | Current implementation | Reborn lifecycle owner | Status |
| --- | --- | --- | --- |
| Chat/product commands | `ProductWorkflow` supports normalized command payloads and now recognizes #3288 extension/skill lifecycle command names. | `LifecycleProductFacade` | Contract wired; concrete production facade still follow-up. |
| WebUI extension setup | `/api/webchat/v2/extensions/{extension_name}/setup` validates typed `ExtensionName` and returns the caller's derived lifecycle projection plus the manifest-declared personal setup affordance. Completing that affordance reconciles readiness automatically. | `LifecycleProductFacade::project_package` over the extension package ref, with setup continuation owned by the declared auth/pairing recipe. | Shipping three-state lifecycle; there is no separate user activation action. |
| Skill search/install/remove | Existing first-party `builtin.skill_*` capability uses Reborn skill management filesystem logic; local-dev composition now wraps the same skill management implementation behind `LifecycleProductFacade`. | `LifecycleProductFacade` over skill management. | Local-dev wired; production lifecycle store/composition still follow-up. |
| Extension install/setup/remove | The staged branch exposed lifecycle operations that predated the final public contract. Shipping Reborn exposes search/install/remove plus setup continuations; install and setup completion trigger host-owned readiness reconciliation rather than a separate public activation command. | Extension lifecycle/config/package services behind facade. | Historical staging behavior; superseded by the current three-state lifecycle contract linked above. |
| MCP lifecycle | Manifest/runtime validation and runtime policy checks exist. | MCP lifecycle service behind facade. | Product lifecycle commands deferred. |
| WASM lifecycle | WASM runtime and adapter slices exist. | WASM package lifecycle service behind facade. | Product lifecycle commands deferred. |
| Capability publication | Host runtime hot capability catalog and visible surface exist. | CapabilityCatalog/ToolSurfaceService path, not product lifecycle. | Must remain separate from install/readiness UX. |

## Drift notes

- `builtin.skill_install` is a bridge, not the canonical long-term lifecycle UX
  surface. Local-dev lifecycle composition now calls the same underlying skill
  management implementation; keep the built-in bridge until production product
  lifecycle command routing replaces it.
- The standalone `ironclaw-reborn` CLI still reports several catalog surfaces as
  not wired; do not read CLI command availability as lifecycle migration
  completion.
- WebUI setup returns the caller's Reborn lifecycle projection and the
  manifest-declared personal auth/pairing affordance. Operator configuration
  remains a separate admin surface, while successful personal setup invokes
  the same host-owned readiness reconciliation used after install.
- Runtime/composition PRs on `reborn-integration` may prove execution paths, but
  #3288 still requires production lifecycle stores, cleanup plans, and concrete
  facade services before it is complete.
- Model-visible extension search may include the redacted installation phase
  when local lifecycle state already knows it. It must not invent auth phases,
  and configured or active search results must not repeat stale credential setup
  metadata or onboarding copy.

## Follow-ups

- Compose a production `LifecycleProductFacade` over extension package/config
  stores, skill management, MCP lifecycle, WASM package lifecycle, auth
  interaction, approval interaction, cleanup, and capability-surface services.
- Add MCP/WASM product commands only when their lifecycle services can return
  safe readiness projections without starting runtime protocol behavior.
- Add cleanup-plan enforcement for deactivate/remove before reporting clean
  success.
- Add architecture tests once production facade crates are selected, ensuring
  lifecycle product code still does not depend on v1 `ExtensionManager`, raw
  `ToolRegistry`, raw MCP clients/processes, raw WASM runtimes, raw secrets, or
  direct network clients.
