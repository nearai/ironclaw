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
- Capability planning and surface: `plan_capability`/`ExecutionPlan`/`PlannerError` (`planner`); the capability-surface policy `CapabilitySurfacePolicy`/`VisibleCapability`/`VisibleCapabilityAccess` (`surface`); the hot capability catalog `HotCapabilityCatalog`/`HotCapabilityRecord`/`publish_hot_capability_catalog` (`capability_catalog`).
- First-party capabilities: the `FirstPartyCapabilityRegistry`/handler/request/result (`first_party`) and the builtin tool set `BuiltinFirstPartyTools` with capability IDs (echo/time/json/http/shell/read_file/write_file/list_dir/glob/grep/apply_patch) and `builtin_first_party_handlers`/`_package` (`first_party_tools`).
- Extension contract discovery: `default_host_api_contract_registry`, `default_host_port_catalog`, `discover_extensions_with_default_host_api_contracts*` (`extension_contracts`).
- Obligation handling: `BuiltinObligationHandler`/`BuiltinObligationServices`, `ProcessObligationLifecycleStore`, and the secret-injection/network-policy stores (`obligations`).
- The runtime process port `RuntimeProcessPort`/`LocalHostProcessPort` + command execution types (`process_port`), the `TurnRunScheduler`/`TurnRunExecutor` scheduler-backed run concurrency (`turn_scheduler`), and memory-context builders (`memory_context`).
- Low-level mediation by composing `ironclaw_network`/`ironclaw_secrets`/`ironclaw_resources` (egress, redaction, secret leases, accounting) — never duplicating that logic in runtime crates.
- Crate-local public API, tests, and fixtures needed to prove that ownership.

## Do Not Move In Here

- product loop strategy, prompt assembly, channel UX, migrations, or duplicated low-level network/secrets/resource logic.
- Secrets, raw host paths, backend error details, and unredacted user content in errors, events, snapshots, logs, or docs.

## Validation

- Fast local check: `cargo test -p ironclaw_host_runtime`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture`
- If production persistence behavior changes, add/maintain PostgreSQL and libSQL parity tests.

## Agent Notes

- Keep edits inside this crate unless a contract explicitly requires a neighboring crate change.
- Prefer caller-level tests when a helper gates dispatch, persistence, network, secrets, approvals, resources, events, or process side effects.
- If the contract and code disagree, stop and treat the task as a contract-change request instead of silently changing ownership.
