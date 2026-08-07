# Agent Map — ironclaw_host_runtime

## Start Here

- Read `README.md` for orientation (charter, measured deps/consumers, the D-R
  egress rule, gates). This file is the canonical working-rules home;
  `CLAUDE.md` is a pointer here.
- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these Reborn contracts as the source of truth before changing behavior:
- `docs/reborn/contracts/host-runtime.md`
- `docs/reborn/contracts/runtime-workflows.md`
- `docs/reborn/contracts/kernel-boundary.md`

## What This Crate Owns

- Host-side composition shared across Reborn runtime lanes and the kernel-facing services/adapters, currently:
- The production host runtime `DefaultHostRuntime` (`production`) and runtime-service composition/readiness: `HostRuntimeServices`, `ProductionWiring*` (component/config/issue/report), `RegisteredRuntimeHealth` (`services`).
- Capability surface: application of the host-API-owned
  `CapabilitySurfacePolicy`, plus `VisibleCapability`/`VisibleCapabilityAccess`
  (`surface`); the hot capability catalog
  `HotCapabilityCatalog`/`HotCapabilityRecord`/`publish_hot_capability_catalog`
  (`capability_catalog`).
- First-party capabilities: the `FirstPartyCapabilityRegistry`/handler/request/result (`first_party`) and the builtin tool set `BuiltinFirstPartyTools` with capability IDs (echo/time/json/http/shell/read_file/write_file/list_dir/glob/grep/apply_patch) and `builtin_first_party_handlers`/`_package` (`first_party_tools`). Several builtins keep only their manifest, registry wiring, and handler adapter here — their executor lives in `ironclaw_extension_support` (the coding tools, and since WS3 the skill-install source fetcher). See "Agent-loop touch points" below for which half goes where.
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

## Guardrails

- Own host-side composition shared across Reborn runtime lanes.
- Keep runtime-specific request shapes in the runtime crates; adapters should
  translate into host API contracts and delegate here.
- Compose low-level services such as `ironclaw_network` and
  `ironclaw_secrets`; do not duplicate URL parsing, DNS checks, private-IP
  filtering, HTTP clients, secret stores, or redaction logic in runtime
  crates.
- Host HTTP egress lives under `src/egress/`: keep request
  validation/sanitization, credential-source resolution, staged
  network-policy lookup, staged secret injection, transport dispatch,
  response sanitization, and response-body storage as separate pipeline steps
  instead of rebuilding a monolithic service method.
- Obligation code lives under `src/obligations/` and stays in its owner:
  `handler` decides and executes obligations, `staged_handoffs` owns the
  secret/network material staged for a later consumer, `process_store` owns
  post-start cleanup and reservation reconciliation. `mod.rs` holds only
  `BuiltinObligationServices` — the assembly seam. A change that needs all
  three at once is a signal the split is being undone, not a reason to add
  code to `mod.rs`. Access needed **only** by a sibling owner is `pub(super)`
  — `RuntimeSecretInjectionStore::{has_for_capability, prune_expired}` and
  `NetworkObligationPolicyStore::contains` — so widening one of those is a
  deliberate edit. `pub(crate)` on the staged-handoff stores is *not* a
  violation of that rule: `insert`/`take`/`clone_material`/`get`/
  `discard_for_capability` are also called from `src/egress/**`, which is
  host-runtime composition outside `obligations/` and is the reason those
  stores exist. The test is what the caller set actually is, not which
  keyword appears: if a method's only callers are inside `obligations/`, it
  is `pub(super)`.
- Production host HTTP egress must be constructed with staged
  `NetworkObligationPolicyStore` and `RuntimeSecretInjectionStore` handoffs.
  Request-carried policy and direct `SecretStoreLease` sources are
  legacy/test compatibility paths only.
- Preserve the accounting invariant: `network_egress_bytes` is outbound
  request bytes only, with response bytes tracked separately.
- Keep raw secret material inside the narrow lease/injection path. Reject
  runtime-supplied manual credentials, scan raw and percent-decoded URL
  forms, redact leased values from runtime-visible errors and responses,
  strip sensitive response headers, and block credential-shaped runtime
  requests/responses before they reach external services or runtime callers.
- Credential injection requires HTTPS **or a literal loopback host** (D-R,
  PROPOSAL §12.13) — the predicate is the shared
  `ironclaw_trace_commons::onboarding::invite::is_loopback_host`
  (`src/egress/credential.rs:398-412`); both sides are pinned by the
  `host_http_egress_*` pair in `src/services/tests.rs`. Do not widen the
  exception without changing the shared predicate and both tests.
- Do not own product workflow, authorization/approval policy, persistence
  migrations, or event emission unless a later Reborn contract explicitly
  moves that composition here.

## Agent-loop touch points

- Production wiring validates the `TurnRunWakeNotifier` handle consumed by
  `ironclaw_turn_runner` (`ProductionWiringComponent::TurnRunWakeNotifier`);
  it does not construct or own the scheduler/executor.
- `surface.rs` owns host-runtime capability-surface shaping and versions.
- `production.rs` and `services.rs` compose runtime services and readiness
  evidence used by Reborn loop wiring.
- Production wiring must reject local-only runtime policy shapes, not just
  require that some `EffectiveRuntimePolicy` value is present.
- First-party runtime tools belong under `first_party_tools/`; do not append
  new built-ins to broad runtime files.
- What belongs there is the **host half**: the `CapabilityManifest`, the
  registry wiring, and a thin `FirstPartyCapabilityHandler` that translates
  this crate's dispatch types into the executor's own request/error pair. The
  **executor half** — parsing, network fetching through `RuntimeHttpEgress`,
  extraction, domain calls — belongs in `ironclaw_extension_support`, which
  may not name this crate (its `BoundaryRule` forbids it). WS3 is moving the
  existing families across that seam one at a time; the skill-install family
  (`extension_support::skills::{url_install, resolve_install_input}`) is the
  worked example.

## Adding code

- Add a new runtime service module when the service has its own authority,
  readiness, or resource accounting boundary.
- Add a first-party tool file per capability, except for tightly-coupled
  v1-compatible coding-tool families that share one legacy surface contract.
- Keep readiness checks near the runtime service they validate;
  driver/product readiness belongs in `ironclaw_turn_runner`.

## Common mistakes

- Do not call `AgentLoopDriver` or compose loop families here.
- Do not own product adapter routing or workflow idempotency.
- Do not bypass host API contracts with runtime-specific shortcuts.

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
