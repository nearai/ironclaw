# Agent Map — ironclaw_host_runtime

## Start Here

- Read `CLAUDE.md` first; it is the crate-local guardrail file.
- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these Reborn contracts as the source of truth before changing behavior:
- `docs/reborn/contracts/host-runtime.md`
- `docs/reborn/contracts/runtime-workflows.md`
- `docs/reborn/contracts/kernel-boundary.md`

## What This Crate Owns

- Host-side composition shared across Reborn runtime lanes and the kernel-facing services/adapters, currently:
- The production host runtime `DefaultHostRuntime` (`production`) and runtime-service composition/readiness: `HostRuntimeServices`, `ProductionWiring*` (component/config/issue/report), `RegisteredRuntimeHealth` (`services`).
- Capability surface: application of the host-API-owned `CapabilitySurfacePolicy`, plus `VisibleCapability`/`VisibleCapabilityAccess` (`surface`); the hot capability catalog `HotCapabilityCatalog`/`HotCapabilityRecord`/`publish_hot_capability_catalog` (`capability_catalog`).
- First-party capabilities: the `FirstPartyCapabilityRegistry`/handler/request/result (`first_party`) and the builtin tool set `BuiltinFirstPartyTools` with capability IDs (echo/time/json/http/shell/read_file/write_file/list_dir/glob/grep/apply_patch) and `builtin_first_party_handlers`/`_package` (`first_party_tools`). Several builtins keep only their manifest, registry wiring, and handler adapter here — their executor lives in `ironclaw_extension_support` (the coding tools, and since WS3 the skill-install source fetcher). See this crate's CLAUDE.md for which half goes where.
- Host-owned extension contract *discovery*: `discover_extensions_with_default_host_api_contracts*`, `discover_extensions_tolerant_bounded*` (`extension_contracts`) — the `RootFilesystem` binding, which is this crate's job. The two **default sets** those functions apply are **not** owned here (WS3 row 3, PROPOSAL §6.5.9): `ironclaw_host_api::host_port::default_host_port_catalog` and `ironclaw_extension_registry::default_host_api_contract_registry` each live with the vocabulary they enumerate. Do not re-add either one — or a `pub use` shim for them — to this crate. Product-specific manifest contracts are still added by the owning composition/product layer.
- Obligation handling (`obligations`), split along its **three chartered owners** so no single file fuses them again (PROPOSAL §6.5.9, CHECKLIST WS3). Put new obligation code in the owner it belongs to, never in `mod.rs`:
  - `obligations::handler` — which obligations apply and what each one does before/after dispatch: `BuiltinObligationHandler`, the `CapabilityObligationHandler` impl, and the audit / redaction / resource-ceiling / mount validation behind them.
  - `obligations::staged_handoffs` — material staged for a *later* consumer: `RuntimeSecretInjectionStore`, `NetworkObligationPolicyStore`, and the `RuntimeCredentialAccountResolver` port that feeds them.
  - `obligations::process_store` — `ProcessObligationLifecycleStore`: discarding staged handoffs and reconciling or releasing a prepared reservation once a capability process has started.
  - `obligations::mod` holds only `BuiltinObligationServices`, the assembly seam that binds the three for composition, and is deliberately the only place naming all three at once.
- The runtime process port `RuntimeProcessPort`/`HostProcessPort` + command execution types (`process_port`) and memory-context builders (`memory_context`).
- Production validation of the `TurnRunWakeNotifier` handle consumed by `ironclaw_turn_runner` (`ProductionWiringComponent::TurnRunWakeNotifier`, `src/services/production_wiring.rs`); scheduler/executor ownership lives in that runner-side crate.
- Low-level mediation by composing `ironclaw_network`/`ironclaw_secrets`/`ironclaw_resources` (egress, redaction, secret leases, accounting) — never duplicating that logic in runtime crates.
- Crate-local public API, tests, and fixtures needed to prove that ownership.

## Do Not Move In Here

- product loop strategy, prompt assembly, channel UX, migrations, or duplicated low-level network/secrets/resource logic.
- Secrets, raw host paths, backend error details, and unredacted user content in errors, events, snapshots, logs, or docs.

## Validation

- Fast local check: `cargo test -p ironclaw_host_runtime`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture_tests`
- If production persistence behavior changes, add/maintain PostgreSQL and libSQL parity tests.

## Agent Notes

- Keep edits inside this crate unless a contract explicitly requires a neighboring crate change.
- Prefer caller-level tests when a helper gates dispatch, persistence, network, secrets, approvals, resources, events, or process side effects.
- If the contract and code disagree, stop and treat the task as a contract-change request instead of silently changing ownership.
