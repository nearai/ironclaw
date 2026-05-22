# IronClaw Crates Map

Instructions for AI coding assistants entering `crates/` on `reborn-integration`.

This file is a routing map, not a full architecture spec. Pick the crate(s) that match the change, then read crate-local guidance before editing:

1. `crates/<crate>/AGENTS.md` (every crate has one).
2. `crates/<crate>/CLAUDE.md` if present.
3. `crates/<crate>/CONTRACT.md` or `README.md` if present.
4. Matching `docs/reborn/contracts/*.md` when behavior crosses crate boundaries.

Do **not** eagerly load every crate guide. Use this map to choose.

## Branch and Workspace

This map was refreshed from `reborn-integration` after inspecting every crate directory, manifest, source layout, tests, and crate-local docs. Every crate with `Cargo.toml` now has a crate-local `AGENTS.md`.

Run crate work from repo root unless crate-local docs say otherwise.

```bash
cargo test -p <crate_name>
cargo clippy -p <crate_name> --all-targets --all-features -- -D warnings
cargo test -p ironclaw_architecture
scripts/check-boundaries.sh
scripts/reborn-e2e-rust.sh
```

Use targeted crate tests first. Add `ironclaw_architecture` when dependency edges or layer ownership change. Run Reborn e2e when turns, runtime lanes, host services, authorization, approvals, networking, secrets, product workflow, or capability dispatch change.

## Guidance Files

- `AGENTS.md` — crate-local agent entrypoint; read first.
- `CLAUDE.md` — crate guardrails/spec; read before changing behavior.
- `CONTRACT.md` — public cross-crate contract; update with semantic changes.
- `README.md` — helper/user/operator details.
- `docs/reborn/contracts/*.md` — Reborn source-of-truth contracts.
- `crates/ironclaw_architecture` — mechanical dependency-boundary enforcement.

Every crate has `AGENTS.md`; treat it as first file to load even if this table becomes stale.

## Dependency Mental Model

Keep lower layers neutral. Product and runtime composition flows downward through typed contracts, not concrete shortcuts.

```text
common / host_api / storage
  -> filesystem / memory / events / event_projections / extensions / trust / resources
  -> secrets / network / outbound / run_state / authorization / approvals / runtime_policy
  -> host_runtime / processes / dispatcher / runtime lanes (scripts, mcp, wasm)
  -> turns / threads / agent_loop / loop_support / capabilities
  -> reborn composition / product adapters / product workflow / CLI
  -> engine / llm / gateway / tui / root product integration
```

Boundary rule: if you need an upstream crate in a low-level crate, stop and check `crates/ironclaw_architecture` plus matching Reborn contract.

## Crate Map

### Foundation and substrate

| Crate | Load first | Owns / go here for | Avoid moving in |
| --- | --- | --- | --- |
| `ironclaw_common` | `ironclaw_common/AGENTS.md`, `Cargo.toml` | Low-dependency shared types/utilities: app events, identity, trust-boundary helpers, paths, platform/env/timezone, attachment helpers. | Runtime orchestration, persistence, clients, policy, product domain logic. |
| `ironclaw_host_api` | `ironclaw_host_api/AGENTS.md`, `ironclaw_host_api/CLAUDE.md`, `docs/reborn/contracts/host-api.md` | Neutral authority vocabulary: IDs, scopes, paths, actions, decisions, resources, approvals, audit, HTTP, dispatch, runtime-policy, trust types. | Runtime execution, persistence, HTTP clients, product workflow, policy engines. |
| `ironclaw_storage` | `ironclaw_storage/AGENTS.md`, `ironclaw_storage/CLAUDE.md` | Generic storage substrate: backend identity, redacted errors, migration descriptors, pagination, serialization, primitive blob/record store traits. | Domain schemas/semantics for turns, threads, outbound, secrets, events. |
| `ironclaw_architecture` | `ironclaw_architecture/AGENTS.md`, `ironclaw_architecture/CLAUDE.md` | Workspace architecture tests, Reborn dependency boundaries, composition-boundary checks. | Production runtime code or production deps. |

### Files, memory, events, projections

| Crate | Load first | Owns / go here for | Avoid moving in |
| --- | --- | --- | --- |
| `ironclaw_filesystem` | `ironclaw_filesystem/AGENTS.md`, `ironclaw_filesystem/CLAUDE.md`, `docs/reborn/contracts/filesystem.md` | Root/scoped/composite filesystem, catalog, virtual path authority, backend containment, mount routing. | Memory-domain grammar, network/secrets/dispatcher/product workflow. |
| `ironclaw_memory` | `ironclaw_memory/AGENTS.md`, `ironclaw_memory/CLAUDE.md`, `docs/reborn/contracts/memory.md` | Memory docs, `/memory` paths, metadata/schema, chunking, embeddings, search, indexer hooks, memory filesystem adapter, backend contracts. | Generic mount/catalog logic or product workflow. |
| `ironclaw_events` | `ironclaw_events/AGENTS.md`, `ironclaw_events/CLAUDE.md`, `docs/reborn/contracts/events.md` | Typed redacted event/audit substrate, event envelopes, sinks/log traits, durable adapters. | SSE/WebSocket/product transport or projection policy. |
| `ironclaw_event_projections` | `ironclaw_event_projections/AGENTS.md`, `ironclaw_event_projections/CLAUDE.md`, `docs/reborn/contracts/events-projections.md` | Event projection model, cursor/visibility contracts, product-facing projection boundaries. | Canonical event storage or transport delivery. |
| `ironclaw_reborn_event_store` | `ironclaw_reborn_event_store/AGENTS.md`, `docs/reborn/contracts/events.md` | Reborn-owned durable event/audit store backends and fixtures. | Product projections, transport fanout, workflow policy. |

### Authority, policy, state

| Crate | Load first | Owns / go here for | Avoid moving in |
| --- | --- | --- | --- |
| `ironclaw_trust` | `ironclaw_trust/AGENTS.md`, `ironclaw_trust/CLAUDE.md`, `ironclaw_trust/CONTRACT.md` | Host-controlled trust classes, policy sources, requested-vs-effective trust, invalidation. | Authorization grants, runtime dispatch, product workflow. |
| `ironclaw_authorization` | `ironclaw_authorization/AGENTS.md`, `ironclaw_authorization/CLAUDE.md` | Grant matching, leases, dispatch/spawn authorization decisions, DB-backed auth state. | Execution, approvals, run-state persistence, prompting. |
| `ironclaw_approvals` | `ironclaw_approvals/AGENTS.md`, `ironclaw_approvals/CLAUDE.md` | Exact-invocation approval requests, leases, resume coordination, approval events. | Reusable broad approvals or dispatch before fingerprinted lease claim. |
| `ironclaw_run_state` | `ironclaw_run_state/AGENTS.md`, `ironclaw_run_state/CLAUDE.md` | Durable invocation state and approval request records. | Authorization policy, approval resolution, dispatch, runtime execution, process lifecycle. |
| `ironclaw_resources` | `ironclaw_resources/AGENTS.md`, `ironclaw_resources/CLAUDE.md` | Reservation, reconciliation, release, quota accounting. | Runtime dispatch, product workflow, hidden costed work without reservation. |
| `ironclaw_runtime_policy` | `ironclaw_runtime_policy/AGENTS.md`, `ironclaw_runtime_policy/CLAUDE.md`, `docs/reborn/contracts/runtime-profiles.md` | Runtime profile resolver and runtime selection policy. | Runtime startup, action dispatch, product strategy outside selection. |
| `ironclaw_outbound` | `ironclaw_outbound/AGENTS.md`, `ironclaw_outbound/CLAUDE.md` | Metadata-only outbound egress policy, notification opt-in, projection subscription cursors, delivery attempt/status metadata. | Transport sends, concrete Slack/Telegram/Web payload validation, transcript/projection mutation. |

### Host services and runtime lanes

| Crate | Load first | Owns / go here for | Avoid moving in |
| --- | --- | --- | --- |
| `ironclaw_secrets` | `ironclaw_secrets/AGENTS.md`, `ironclaw_secrets/CLAUDE.md` | Secret metadata, encrypted repositories, leases, one-shot consumption, legacy/db stores. | Raw secret exposure, provider HTTP, injection beyond mediated handoff. |
| `ironclaw_network` | `ironclaw_network/AGENTS.md`, `ironclaw_network/CLAUDE.md`, `docs/reborn/contracts/network.md` | Network policy boundary, URL targets, resolver, hardened transport, host/provider HTTP egress. | Runtime-lane behavior above boundary or manual credential injection. |
| `ironclaw_host_runtime` | `ironclaw_host_runtime/AGENTS.md`, `ironclaw_host_runtime/CLAUDE.md` | Host-side Reborn service composition: production services, obligations, HTTP egress, redaction, secrets/network/resource mediation. | Product workflow, runtime-specific request shapes, duplicate network/secret logic. |
| `ironclaw_processes` | `ironclaw_processes/AGENTS.md`, `ironclaw_processes/CLAUDE.md` | Process lifecycle, cancellation, stores, status/output helpers, `ProcessHost`, wrappers. | Authorization, approval policy, runtime lane internals beyond adapter contracts. |
| `ironclaw_dispatcher` | `ironclaw_dispatcher/AGENTS.md`, `ironclaw_dispatcher/CLAUDE.md` | Already-authorized runtime routing through `RuntimeAdapter`, redacted dispatch results, event dispatch contracts. | Authorization, approvals, run-state, concrete runtime deps, product workflow. |
| `ironclaw_scripts` | `ironclaw_scripts/AGENTS.md`, `ironclaw_scripts/CLAUDE.md` | Script runtime lane over host-mediated filesystem/events/resources/dispatcher/HTTP, Docker/backend output parsing. | Manual credentials, direct provider HTTP, duplicated dispatcher/process/resource policy. |
| `ironclaw_mcp` | `ironclaw_mcp/AGENTS.md`, `ironclaw_mcp/CLAUDE.md` | MCP runtime lane, execution request/result types, JSON-RPC exchange, client abstraction, HTTP adapter, resource accounting. | Direct outbound networking, ad-hoc credential injection, product workflow. |
| `ironclaw_wasm` | `ironclaw_wasm/AGENTS.md`, `ironclaw_wasm/CLAUDE.md`, `docs/reborn/contracts/wasm.md`, `wit/tool.wit` | WASM runtime lane, component/WIT bindings, limiter, store, host adapters, runtime config. | Privileged host effects outside mediated APIs; copied secrets/network/resource logic. |
| `ironclaw_wasm_sandbox_core` | `ironclaw_wasm_sandbox_core/AGENTS.md`, `ironclaw_wasm_sandbox_core/CLAUDE.md` | Shared WASM sandbox core primitives used below product adapters/runtime. | Product adapter workflow or host product policy. |

### Turns, threads, loops, engine

| Crate | Load first | Owns / go here for | Avoid moving in |
| --- | --- | --- | --- |
| `ironclaw_turns` | `ironclaw_turns/AGENTS.md`, `ironclaw_turns/CLAUDE.md` | Host-layer turn coordination: requests/responses, coordinator, runner, run profiles, loop exit, memory/context handoff, turn store. | Product adapter rendering, raw runtime lanes, UI behavior. |
| `ironclaw_threads` | `ironclaw_threads/AGENTS.md`, `ironclaw_threads/CLAUDE.md` | Canonical session thread/transcript service contracts, identifiers, tool-result references, db/in-memory stores. | Product delivery policy or model/provider behavior. |
| `ironclaw_conversations` | `ironclaw_conversations/AGENTS.md`, `ironclaw_conversations/CLAUDE.md` | Conversation binding, session thread contracts, inbound/state store, libSQL/Postgres conversation persistence. | Capability runtime internals or UI transport. |
| `ironclaw_agent_loop` | `ironclaw_agent_loop/AGENTS.md`, `ironclaw_agent_loop/CLAUDE.md` | Agent-loop framework state, planner/executor, strategy/family contracts, test support. | Product adapters, transport, concrete provider auth. |
| `ironclaw_loop_support` | `ironclaw_loop_support/AGENTS.md`, `ironclaw_loop_support/CLAUDE.md` | Loop host support services: capability/input ports, allow sets, input queue, identity/skill context, cancellation. | Owning core loop strategy or runtime lane execution. |
| `ironclaw_capabilities` | `ironclaw_capabilities/AGENTS.md`, `ironclaw_capabilities/CLAUDE.md` | Caller-facing `CapabilityHost` invoke/resume/spawn workflow, obligation seams, conformance helpers. | Process lifecycle APIs, direct concrete runtime dependencies. |
| `ironclaw_engine` | `ironclaw_engine/AGENTS.md`, `ironclaw_engine/CLAUDE.md`, `ironclaw_engine/MONTY.md` | Thread/capability/CodeAct engine: runtime manager, executor, gates, leases, memory retrieval, workspace mounts, traits/types. | Product transport, provider-specific auth, lower-layer host policy shortcuts. |

### Product, adapters, Reborn binary

| Crate | Load first | Owns / go here for | Avoid moving in |
| --- | --- | --- | --- |
| `ironclaw_reborn` | `ironclaw_reborn/AGENTS.md`, `ironclaw_reborn/CLAUDE.md` | Standalone Reborn composition/adapters: driver registry, home/profile/doctor support, runtime composition seams. | V1 root runtime imports unless explicitly bridged. |
| `ironclaw_reborn_config` | `ironclaw_reborn_config/AGENTS.md`, `Cargo.toml`, `src/lib.rs` | Boot configuration contracts for standalone Reborn binary. | Runtime execution or product adapter behavior. |
| `ironclaw_reborn_composition` | `ironclaw_reborn_composition/AGENTS.md`, `ironclaw_reborn_composition/CLAUDE.md` | Facade-shaped production composition root for Reborn. | Low-level policy internals that belong to service crates. |
| `ironclaw_first_party_extensions` | `ironclaw_first_party_extensions/AGENTS.md`, `Cargo.toml` | First-party userland Reborn extensions with explicit scoped handles and narrow loop-facing ports. | Ambient runtime authority, dispatcher/network/secrets handles, or Reborn composition wiring. |
| `ironclaw_reborn_cli` | `ironclaw_reborn_cli/AGENTS.md` | Standalone Reborn CLI, command files, CLI context, shell completions, doctor/home/profile commands. | V1 runtime imports, root `ironclaw` deps, side effects in pure commands. |
| `ironclaw_product_adapters` | `ironclaw_product_adapters/AGENTS.md`, `ironclaw_product_adapters/CLAUDE.md` | Product-adapter contracts: adapter trait, auth, egress, identity, workflow, external/projection/inbound, redaction, fakes. | Host runtime internals or specific WASM runner implementation. |
| `ironclaw_product_adapter_registry` | `ironclaw_product_adapter_registry/AGENTS.md`, `ironclaw_product_adapter_registry/CLAUDE.md` | ProductAdapter host-api projection and installation registry. | Adapter execution or product workflow orchestration. |
| `ironclaw_product_workflow` | `ironclaw_product_workflow/AGENTS.md`, `ironclaw_product_workflow/CLAUDE.md` | Product-facing workflow facade: inbound turns, bindings, ledger, workflow/errors, Reborn service bridges. | Low-level runtime lane internals or direct provider-specific transports. |
| `ironclaw_wasm_product_adapters` | `ironclaw_wasm_product_adapters/AGENTS.md`, `ironclaw_wasm_product_adapters/CLAUDE.md` | WASM v2 ProductAdapter runtime: component runner, egress policy, auth verifier, bindings, store. | Generic WASM lane semantics or product workflow decisions. |
| `ironclaw_telegram_v2_adapter` | `ironclaw_telegram_v2_adapter/AGENTS.md`, `Cargo.toml`, `src/lib.rs` | Telegram WASM v2 ProductAdapter tracer bullet: payload parsing, rendering, adapter implementation. | Shared adapter contracts or registry semantics. |

### LLM, skills, safety, UI, helpers

| Crate | Load first | Owns / go here for | Avoid moving in |
| --- | --- | --- | --- |
| `ironclaw_llm` | `ironclaw_llm/AGENTS.md`, `ironclaw_llm/CLAUDE.md`, `ironclaw_llm/Cargo.toml` | Multi-provider LLM integration: provider trait, auth, registry, retry/failover/circuit breaker/cache, tool schemas, reasoning, tracing, transcription/vision. | Engine loop ownership or product workflow. |
| `ironclaw_skills` | `ironclaw_skills/AGENTS.md` | Skill catalog, parser, gating, selector/scoring, registry, validation, v2 skill types. | Agent-loop execution or UI command routing. |
| `ironclaw_safety` | `ironclaw_safety/AGENTS.md`, `crates/ironclaw_safety/fuzz/README.md` | Prompt-injection detection, validation, sanitization, safety policy, sensitive paths, credential detection, leak scanning, fuzz/benches. | Sandbox execution, credential storage/injection, network allowlists, dispatch, UI decisions. |
| `ironclaw_gateway` | `ironclaw_gateway/AGENTS.md` | Gateway frontend assets, layout config, bundle metadata, widget extension system. | Browser API/web channel runtime (`src/channels/web/`) or product workflow. |
| `ironclaw_tui` | `ironclaw_tui/AGENTS.md`, `ironclaw_tui/CLAUDE.md` | Ratatui app, widgets, layout, render, theme, event/input loop, spinner. | Main crate channel bridge (`src/channels/tui.rs`) or backend workflow. |
| `ironclaw_silk_decoder` | `ironclaw_silk_decoder/AGENTS.md`, `ironclaw_silk_decoder/README.md`, `ironclaw_silk_decoder/Cargo.toml`, `ironclaw_silk_decoder/src/main.rs` | Excluded helper binary that decodes WeChat SILK v3 voice notes to WAV. | Main workspace build dependencies; keep libclang isolated. |

## Common Change Routes

- Host API shape: `ironclaw_host_api` -> matching `docs/reborn/contracts/*.md` -> affected service/runtime crates -> `ironclaw_architecture`.
- Storage abstraction: `ironclaw_storage` for generic mechanics; owning domain crate for schemas/queries; preserve libSQL/PostgreSQL parity.
- Files/memory: `ironclaw_filesystem` for mount/path authority; `ironclaw_memory` for memory documents/search/chunking/indexing.
- Events/projections/outbound: `ironclaw_events` for canonical redacted events; `ironclaw_event_projections` for projection model; `ironclaw_outbound` for metadata-only delivery/subscription policy; adapters for concrete delivery.
- Trust/auth/approval: `ironclaw_trust` -> `ironclaw_authorization` -> `ironclaw_run_state`/`ironclaw_approvals` -> `ironclaw_capabilities` as needed.
- Runtime execution: lane crate (`scripts`, `mcp`, `wasm`) first; `dispatcher` for routing; `host_runtime` for secrets/network/resources/redaction; `processes` for background lifecycle.
- Turns/agent loop: `ironclaw_turns` for turn coordination; `ironclaw_agent_loop` for strategy/planner/executor contracts; `ironclaw_loop_support` for host support ports; `ironclaw_engine` for CodeAct/thread runtime.
- Product adapter flow: `ironclaw_product_adapters` contracts -> `ironclaw_product_adapter_registry` installation/projection -> `ironclaw_product_workflow` orchestration -> concrete adapter crate.
- Reborn binary/composition: `ironclaw_reborn_config` for boot config; `ironclaw_reborn_composition` for production wiring; `ironclaw_reborn_cli` for commands; `ironclaw_reborn` for standalone adapters/driver registry.
- Model/provider behavior: `ironclaw_llm`; do not leak provider auth/cache/retry concerns into engine or product workflow.
- UI presentation: `ironclaw_tui` or `ironclaw_gateway`; backend API/web channel code remains under root `src/`.

## Testing

Prefer narrow tests during iteration:

```bash
cargo test -p ironclaw_host_api
cargo test -p ironclaw_network network_policy_contract
cargo test -p ironclaw_outbound --all-features
cargo test -p ironclaw_product_workflow
cargo test -p ironclaw_wasm --test wit_tool_runtime_contract
```

Then expand by risk:

```bash
cargo test -p ironclaw_architecture
scripts/check-boundaries.sh
scripts/reborn-e2e-rust.sh
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Persistence behavior must support PostgreSQL and libSQL where applicable. If local Postgres is unavailable, follow crate-local skip flags only when docs/tests explicitly permit them.

## Guardrails

- Avoid `.unwrap()` / `.expect()` in production; use typed errors with context.
- Preserve tenant/user/agent/project/mission/thread scope on authority, state, memory, process, network, outbound, resource, and event records.
- Fail closed for auth, approvals, trust, filesystem containment, network policy, secret leases, runtime selection, and adapter identity.
- Do not expose raw secrets, backend paths, private URLs, transport internals, raw SQL/backend errors, or unredacted runtime/user content across public surfaces.
- Keep runtime crates untrusted: host-runtime mediates secrets/network/redaction/accounting.
- Keep declarative crates declarative: manifests, contracts, registries, and policy descriptions should not perform execution side effects.
- Use existing traits/ports/registries; avoid hardcoded cross-crate shortcuts.
- Test through caller when a helper gates dispatch, persistence, network, secrets, approvals, resources, events, process, adapter, or UI side effects.

## Docs / Parity Checklist

Behavior changes may require updates to:

- crate-local `AGENTS.md`, `CLAUDE.md`, `CONTRACT.md`, or `README.md`
- `docs/reborn/contracts/*.md`
- `FEATURE_PARITY.md`
- crate changelogs for packages that publish independently
- architecture boundary tests in `crates/ironclaw_architecture`
