# #3288 Reborn lifecycle UX realignment

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
| WebUI extension setup | `/api/webchat/v2/extensions/{extension_name}/setup` validates typed `ExtensionName` and returns a side-effect-free lifecycle projection. | `LifecycleProductFacade::project_package` over the extension package ref. | Projection wired; no setup/configure/activate side effects yet. |
| Skill search/install/remove | Existing first-party `builtin.skill_*` capability uses Reborn skill management filesystem logic; local-dev composition now wraps the same skill management implementation behind `LifecycleProductFacade`. | `LifecycleProductFacade` over skill management. | Local-dev wired; production lifecycle store/composition still follow-up. |
| Extension install/activate/remove | Local-dev composition wires extension search/install/activate/remove through `LifecycleProductFacade`; the Reborn CLI exposes that facade for operator testing, and the local-dev agent surface exposes model-visible `builtin.extension_search`, `builtin.extension_install`, `builtin.extension_activate`, and `builtin.extension_remove` capabilities over the same lifecycle port. | Extension lifecycle/config/package services behind facade. | Local-dev CLI and agent-loop surfaces wired; production lifecycle store/composition still follow-up. |
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
- WebUI setup now returns a Reborn lifecycle projection, but it intentionally
  does not configure, authenticate, activate, or migrate v1 setup behavior.
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
