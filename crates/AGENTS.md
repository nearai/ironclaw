# IronClaw Crates Map

Instructions for AI coding assistants entering `crates/`, which contains Reborn crates plus legacy v1 support crates.

This file is a routing map, not a full architecture spec. Pick the crate(s) that match the change, then read crate-local guidance before editing:

1. `crates/<crate>/AGENTS.md` when present.
2. `crates/<crate>/CLAUDE.md` if present.
3. `crates/<crate>/CONTRACT.md` or `README.md` if present.
4. Matching `docs/reborn/contracts/*.md` when behavior crosses crate boundaries.

Do **not** eagerly load every crate guide. Use this map to choose.

## Branch and Workspace

This map was last refreshed 2026-07-02 against the workspace crate manifests, source layout, tests, and crate-local docs. Most crates have a crate-local `AGENTS.md`; when one is missing, load `CLAUDE.md`, `CONTRACT.md` or `README.md` if present, `Cargo.toml`, and the crate's primary `src/` entrypoint instead.

Run crate work from repo root unless crate-local docs say otherwise.

```bash
cargo test -p <crate_name>
cargo clippy -p <crate_name> --all-targets --all-features -- -D warnings
cargo test -p ironclaw_architecture
scripts/check-boundaries.sh
scripts/reborn-e2e-rust.sh
```

Use targeted crate tests first. Add `ironclaw_architecture` when dependency edges or layer ownership change. Run Reborn e2e when turns, runtime lanes, host services, authorization, approvals, networking, secrets, product workflow, or capability dispatch change. Note: `scripts/check-boundaries.sh` inspects the v1 `src/` tree only — for `crates/`, boundary enforcement is `cargo test -p ironclaw_architecture`.

## Guidance Files

- `AGENTS.md` — crate-local agent entrypoint; read first.
- `CLAUDE.md` — crate guardrails/spec; read before changing behavior.
- `CONTRACT.md` — public cross-crate contract; update with semantic changes.
- `README.md` — helper/user/operator details.
- `docs/reborn/contracts/*.md` — Reborn source-of-truth contracts.
- `crates/ironclaw_architecture` — mechanical dependency-boundary enforcement.

Treat crate-local `AGENTS.md` as the first file to load when it exists. Several crates lack one — don't rely on a hand-maintained list; find them with `for d in crates/ironclaw_*/; do [ -f "$d/AGENTS.md" ] || echo "$d"; done` and fall back to `CLAUDE.md`, `CONTRACT.md` or `README.md` if present, `Cargo.toml`, and the crate's primary `src/` entrypoint (`src/lib.rs` for libraries, `src/main.rs` for binaries).

## Dependency Mental Model

Keep lower layers neutral. Product and runtime composition flows downward through typed contracts, not concrete shortcuts.

```text
common / host_api / prompt_envelope
  -> filesystem / memory / events / event_projections / event_streams / extensions / trust / resources
  -> secrets / network / outbound / channel_host / channel_delivery / run_state / authorization / approvals / runtime_policy / hooks
  -> host_runtime / processes / dispatcher / runtime lanes (scripts, mcp, wasm, wasm_limiter)
  -> turns / threads / agent_loop / loop_host / capabilities
  -> reborn composition / product adapters / product workflow / product workflow storage / CLI
  -> engine / llm / gateway / webui_v2 / webui_ingress / tui / root product integration
```

Boundary rule: if you need an upstream crate in a low-level crate, stop and check `crates/ironclaw_architecture` plus matching Reborn contract.

## Crate Map

### Foundation and substrate

| Crate | Load first | Owns / go here for | Avoid moving in |
| --- | --- | --- | --- |
| `ironclaw_common` | `ironclaw_common/AGENTS.md`, `Cargo.toml` | Low-dependency shared types/utilities: app events, identity, trust-boundary helpers, paths, platform/env/timezone, attachment helpers. | Runtime orchestration, persistence, clients, policy, product domain logic. |
| `ironclaw_host_api` | `ironclaw_host_api/AGENTS.md`, `ironclaw_host_api/CLAUDE.md`, `docs/reborn/contracts/host-api.md` | Neutral authority vocabulary: IDs, scopes, paths, actions, decisions, resources, approvals, audit, HTTP, dispatch, runtime-policy, trust types. | Runtime execution, persistence, HTTP clients, product workflow, policy engines. |
| `ironclaw_prompt_envelope` | `Cargo.toml`, `src/lib.rs` | Leaf prompt-envelope helper: wraps model-visible snippets with closed-vocabulary source/trust labels, size limits, and instruction-hijack rejection. | Runtime orchestration, model routing, policy decisions, or free-form source labels. |
| `ironclaw_architecture` | `ironclaw_architecture/AGENTS.md`, `ironclaw_architecture/CLAUDE.md` | Workspace architecture tests, Reborn dependency boundaries, composition-boundary checks. | Production runtime code or production deps. |
| `ironclaw_observability` | `Cargo.toml`, `src/lib.rs` | Shared latency-tracing macros (`live_latency_trace*`) over the `ironclaw_latency` tracing target. | Policy, state, or runtime behavior. |

### Files, memory, events, projections

| Crate | Load first | Owns / go here for | Avoid moving in |
| --- | --- | --- | --- |
| `ironclaw_filesystem` | `ironclaw_filesystem/AGENTS.md`, `ironclaw_filesystem/CLAUDE.md`, `docs/reborn/contracts/filesystem.md` | Root/scoped/composite filesystem, catalog, virtual path authority, backend containment, mount routing. | Memory-domain grammar, network/secrets/dispatcher/product workflow. |
| `ironclaw_memory` | `ironclaw_memory/AGENTS.md`, `ironclaw_memory/CLAUDE.md`, `docs/reborn/contracts/memory.md` | Memory docs, `/memory` paths, metadata/schema, chunking, embeddings, search, indexer hooks, memory filesystem adapter, backend contracts. | Generic mount/catalog logic or product workflow. |
| `ironclaw_events` | `ironclaw_events/AGENTS.md`, `ironclaw_events/CLAUDE.md`, `docs/reborn/contracts/events.md` | Typed redacted event/audit substrate, event envelopes, sinks/log traits, durable adapters. | SSE/WebSocket/product transport or projection policy. |
| `ironclaw_event_projections` | `ironclaw_event_projections/AGENTS.md`, `ironclaw_event_projections/CLAUDE.md`, `docs/reborn/contracts/events-projections.md` | Event projection model, cursor/visibility contracts, product-facing projection boundaries. | Canonical event storage or transport delivery. |
| `ironclaw_event_streams` | `ironclaw_event_streams/AGENTS.md`, `ironclaw_event_streams/CLAUDE.md`, `docs/reborn/contracts/events-projections.md` | Transport-neutral projection stream manager: admission, bounded subscription buffers, live/replay update delivery, lag/rebase signals, redaction validation. | Axum/SSE/WebSocket framing, product workflow submission, durable event-store adapters, raw runtime payloads. |
| `ironclaw_reborn_event_store` | `ironclaw_reborn_event_store/AGENTS.md`, `docs/reborn/contracts/events.md` | Reborn-owned durable event/audit store backends and fixtures. | Product projections, transport fanout, workflow policy. |
| `ironclaw_reborn_traces` | `Cargo.toml`, `src/lib.rs` | Trace Commons / TraceDAO client surface: contribution pipeline, trace client, redaction helpers, conversation-message compatibility, and trace preview re-exports. | Reborn CLI command behavior, LLM provider routing, unredacted trace submission. |
| `ironclaw_memory_native` | `ironclaw_memory_native/AGENTS.md`, `ironclaw_memory_native/CLAUDE.md` | Native filesystem memory provider: `NativeMemoryService`, document repos, chunking, hybrid search, indexer, prompt-write-safety engine. | Provider-neutral memory contracts (`ironclaw_memory`) or product workflow. |
| `ironclaw_attachments` | `Cargo.toml`, `src/lib.rs` | The single inbound-attachment landing routine, writing through project-scoped `ScopedFilesystem` (fail-closed on read-only mounts). | Per-channel persistence paths; text extraction (that's `ironclaw_extractors`). |
| `ironclaw_extractors` | `Cargo.toml`, `src/lib.rs` | Pure bytes→text extraction by MIME (PDF/OOXML/legacy Office) with decompression-bomb caps; no I/O. | Network fetches, storage, channel logic. |
| `ironclaw_triggers` | `ironclaw_triggers/AGENTS.md`, `docs/reborn/contracts/triggers.md` | Scheduled-trigger substrate: records, cron/timezone validation, deterministic fire identity, poller core, durable libSQL/Postgres repos, trusted-submit request minting. | Poller lifecycle/composition (composition owns it); any parallel agent loop. |
| `ironclaw_projects` | `ironclaw_projects/CLAUDE.md` | Project entity + membership ACL (live `resolve_access`, never cached) + `ProjectRepository` over `RootFilesystem` with CAS create/delete. **W2 decision: keep standalone; do not fold into composition.** | The legacy engine `Project` type; product workflow facade logic. If revisited, `ironclaw_product_workflow` is the only acceptable consumer-side target. |

### Authority, policy, state

| Crate | Load first | Owns / go here for | Avoid moving in |
| --- | --- | --- | --- |
| `ironclaw_trust` | `ironclaw_trust/AGENTS.md`, `ironclaw_trust/CLAUDE.md`, `ironclaw_trust/CONTRACT.md` | Host-controlled trust classes, policy sources, requested-vs-effective trust, invalidation. | Authorization grants, runtime dispatch, product workflow. |
| `ironclaw_authorization` | `ironclaw_authorization/AGENTS.md`, `ironclaw_authorization/CLAUDE.md` | Grant matching, leases, dispatch/spawn authorization decisions, DB-backed auth state. | Execution, approvals, run-state persistence, prompting. |
| `ironclaw_approvals` | `ironclaw_approvals/AGENTS.md`, `ironclaw_approvals/CLAUDE.md` | Exact-invocation approval requests, leases, resume coordination, approval events. | Reusable broad approvals or dispatch before fingerprinted lease claim. |
| `ironclaw_run_state` | `ironclaw_run_state/AGENTS.md`, `ironclaw_run_state/CLAUDE.md` | Durable invocation state and approval request records. | Authorization policy, approval resolution, dispatch, runtime execution, process lifecycle. |
| `ironclaw_resources` | `ironclaw_resources/AGENTS.md`, `ironclaw_resources/CLAUDE.md` | Reservation, reconciliation, release, quota accounting. | Runtime dispatch, product workflow, hidden costed work without reservation. |
| `ironclaw_auth` | `ironclaw_auth/AGENTS.md`, `ironclaw_auth/CLAUDE.md`, `docs/reborn/contracts/auth-product.md` | Product-facing Reborn auth-flow, secure interaction, credential account, provider exchange, continuation, cleanup contracts and fakes. | V1 route handlers/pending maps, durable secret storage, raw provider HTTP, runtime injection, extension lifecycle mutation. |
| `ironclaw_runtime_policy` | `ironclaw_runtime_policy/AGENTS.md`, `ironclaw_runtime_policy/CLAUDE.md`, `docs/reborn/contracts/runtime-profiles.md` | Runtime profile resolver and runtime selection policy. | Runtime startup, action dispatch, product strategy outside selection. |
| `ironclaw_outbound` | `ironclaw_outbound/AGENTS.md`, `ironclaw_outbound/CLAUDE.md` | Metadata-only outbound egress policy, notification opt-in, projection subscription cursors, delivery attempt/status metadata. | Transport sends, concrete Slack/Telegram/Web payload validation, transcript/projection mutation. |
| `ironclaw_hooks` | `ironclaw_hooks/CLAUDE.md`, `Cargo.toml`, `src/lib.rs` | Reborn loop hook framework: trust-tiered hook contracts, sealed decision sinks, predicates, ordering, dispatch, telemetry, and failure policy. | Authority grants, runtime-policy bypasses, ambient secrets/network/filesystem handles, extension installation. |
| `ironclaw_hooks_postgres` | `ironclaw_hooks_postgres/AGENTS.md` | Durable PostgreSQL `PredicateStateBackend` (advisory-lock concurrency, deadlock-free eviction). | Window math (canonical helper lives in `ironclaw_hooks`); diverging from the parity contract. |
| `ironclaw_hooks_libsql` | `Cargo.toml`, `src/lib.rs` | Durable libSQL `PredicateStateBackend` (single-writer mutex, `BEGIN IMMEDIATE`). | Window math; diverging from the parity contract. |
| `ironclaw_hooks_parity` | `Cargo.toml`, `src/lib.rs` | Test-only cross-backend adversarial parity suite proving in-memory/Postgres/libSQL backends behaviorally interchangeable (its `src/` is doc-only by design). | Production code or deps. |

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
| `ironclaw_wasm` | `ironclaw_wasm/AGENTS.md`, `ironclaw_wasm/CLAUDE.md`, `docs/reborn/contracts/wasm.md`, `wit/tool.wit` | WASM runtime lane, component/WIT bindings, folded `wasm_sandbox_core` primitives, store, host adapters, runtime config. | Privileged host effects outside mediated APIs; copied secrets/network/resource logic; product/runtime-specific dependencies inside `wasm_sandbox_core`. |
| `ironclaw_wasm_limiter` | `Cargo.toml`, `src/lib.rs` | Shared `wasmtime::ResourceLimiter` for WASM tool and hook runtimes. | Product adapter workflow, policy decisions, or runtime-specific side effects beyond limiter accounting. |
| `ironclaw_extensions` | `ironclaw_extensions/AGENTS.md`, `ironclaw_extensions/CLAUDE.md` | Declarative extension manifests (v2), capability descriptors, side-effect-free in-memory registry, installation records. | Execution of any kind (WASM/MCP/process), secrets, trust decisions. |
| `ironclaw_process_sandbox` | `ironclaw_process_sandbox/CLAUDE.md` | Docker process-sandbox backend behind `ironclaw_processes::ProcessExecutor`: typed sandbox plans, install/credentialed-run phase separation, mount roots. | Process lifecycle/stores (`ironclaw_processes`); raw Docker flags for extensions. |

### Turns, threads, loops, engine

| Crate | Load first | Owns / go here for | Avoid moving in |
| --- | --- | --- | --- |
| `ironclaw_turns` | `ironclaw_turns/AGENTS.md`, `ironclaw_turns/CLAUDE.md` | Host-layer turn coordination: requests/responses, coordinator, runner, run profiles, loop exit, memory/context handoff, turn store. | Product adapter rendering, raw runtime lanes, UI behavior. |
| `ironclaw_threads` | `ironclaw_threads/AGENTS.md`, `ironclaw_threads/CLAUDE.md` | Canonical session thread/transcript service contracts, identifiers, tool-result references, db/in-memory stores. | Product delivery policy or model/provider behavior. |
| `ironclaw_conversations` | `ironclaw_conversations/AGENTS.md`, `ironclaw_conversations/CLAUDE.md` | Conversation binding, session thread contracts, inbound/state store, libSQL/Postgres conversation persistence. | Capability runtime internals or UI transport. |
| `ironclaw_agent_loop` | `ironclaw_agent_loop/AGENTS.md`, `ironclaw_agent_loop/CLAUDE.md` | Agent-loop framework state, planner/executor, strategy/family contracts, test support. | Product adapters, transport, concrete provider auth. |
| `ironclaw_loop_host` | `ironclaw_loop_host/AGENTS.md`, `ironclaw_loop_host/CLAUDE.md` | Loop host support services: capability/input ports, allow sets, input queue, identity/skill context, cancellation. | Owning core loop strategy or runtime lane execution. |
| `ironclaw_capabilities` | `ironclaw_capabilities/AGENTS.md`, `ironclaw_capabilities/CLAUDE.md` | Caller-facing `CapabilityHost` invoke/resume/spawn workflow, obligation seams, conformance helpers, and the host-private `ReplayPayloadStore` (raw gate/auth resume replay payload, never model-visible). | Process lifecycle APIs, direct concrete runtime dependencies. |
| `ironclaw_engine` | `ironclaw_engine/AGENTS.md`, `ironclaw_engine/CLAUDE.md`, `ironclaw_engine/MONTY.md` | **v1-only (legacy — retires with the monolith).** The root crate's engine v2 (thread/capability/CodeAct: runtime manager, executor, gates, leases). Reborn crates are boundary-test-forbidden from importing it. | **Any new Reborn behavior.** Maintenance of existing v1 behavior only. |

### Product, adapters, Reborn binary

| Crate | Load first | Owns / go here for | Avoid moving in |
| --- | --- | --- | --- |
| `ironclaw_runner` | `ironclaw_runner/AGENTS.md`, `ironclaw_runner/CLAUDE.md` | **Internal runner control plane and loop-runtime assembly** (sole production consumer: `ironclaw_reborn_composition`; test harnesses may use it directly): scheduler, per-run executor, driver registry, planned/text driver adapters, loop host factory, exit-applier wiring, home/profile/doctor support. | Treating it as a public composition root; V1 root runtime imports unless explicitly bridged. |
| `ironclaw_reborn_config` | `ironclaw_reborn_config/AGENTS.md`, `Cargo.toml`, `src/lib.rs` | Boot configuration contracts for standalone Reborn binary. | Runtime execution or product adapter behavior. |
| `ironclaw_reborn_composition` | `ironclaw_reborn_composition/AGENTS.md`, `ironclaw_reborn_composition/CLAUDE.md` | Facade-shaped production composition root for Reborn. | Low-level policy internals that belong to service crates. |
| `ironclaw_reborn_openai_compat` | `ironclaw_reborn_openai_compat/AGENTS.md`, `ironclaw_reborn_openai_compat/CLAUDE.md` | Reborn-native OpenAI-compatible API route descriptors, Chat/Responses DTOs, sanitized error envelope, fail-closed route fragment, and feature-gated durable ref/idempotency storage adapters. | V1 gateway handlers, direct LLM proxying, listener binding, ProductWorkflow internals/direct runtime wiring, or filesystem access outside `OpenAiCompatRefStore`. |
| `ironclaw_first_party_extensions` | `ironclaw_first_party_extensions/AGENTS.md`, `Cargo.toml` | Concrete first-party userland extension implementations and deterministic tool behavior behind scoped handles. | Host runtime composition, loop-facing ports, ambient runtime authority, dispatcher/network/secrets handles. |
| `ironclaw_first_party_extension_ports` | `ironclaw_first_party_extension_ports/AGENTS.md`, `Cargo.toml` | Loop-facing adapters for first-party extensions: skill activation/context/execution ports over loop-host and turn-run contracts. | Concrete tool behavior, host runtime composition, product workflow, raw host authority. |
| `ironclaw_reborn_cli` | `ironclaw_reborn_cli/AGENTS.md` | Standalone Reborn CLI, command files, CLI context, shell completions, doctor/home/profile commands. | V1 runtime imports, root `ironclaw` deps, side effects in pure commands. |
| `ironclaw_product_adapters` | `ironclaw_product_adapters/AGENTS.md`, `ironclaw_product_adapters/CLAUDE.md` | Product-adapter contracts: adapter trait, auth, egress, identity, workflow, external/projection/inbound, redaction, fakes. | Host runtime internals or specific WASM runner implementation. |
| `ironclaw_product_adapter_registry` | `ironclaw_product_adapter_registry/AGENTS.md`, `ironclaw_product_adapter_registry/CLAUDE.md` | ProductAdapter host-api projection and installation registry. | Adapter execution or product workflow orchestration. |
| `ironclaw_product_workflow` | `ironclaw_product_workflow/AGENTS.md`, `ironclaw_product_workflow/CLAUDE.md` | Product-facing workflow facade: inbound turns, bindings, ledger, workflow/errors, Reborn service bridges, and feature-gated durable ledger adapters. | Low-level runtime lane internals, direct provider-specific transports, or durable ledger access outside the `IdempotencyLedger` port. |
| `ironclaw_wasm_product_adapters` | `ironclaw_wasm_product_adapters/AGENTS.md`, `ironclaw_wasm_product_adapters/CLAUDE.md` | Product-layer WASM v2 ProductAdapter host glue: component runner, egress policy, auth verifier, bindings, store. | Generic WASM lane semantics or product workflow decisions. |
| `ironclaw_channel_host` | `ironclaw_channel_host/AGENTS.md` | Vendor-neutral channel-host contracts/helpers: identity lookup, `ChannelDeliveryProtocol`, outbound-target provider, ingress projection/rate/error helpers, host-state JSON records, auth continuation. | Concrete channels, delivery algorithms, mount assembly, server lifecycle. |
| `ironclaw_channel_delivery` | `ironclaw_channel_delivery/AGENTS.md` | Product-neutral live/triggered channel-delivery engine: observation, bounded admission, actionable prompts, route tracking, honest outcomes, keyed hook fan-out. | Channel-specific protocol/rendering, composition/global registries, WebUI/CLI. |
| `ironclaw_telegram_extension` | `ironclaw_telegram_extension/AGENTS.md`, `docs/reborn/contracts/telegram-v2.md` | Concrete Telegram host domain and host builder: setup/Bot API, filesystem state, pairing, DM ingress, revision workflows, protocol/targets/trigger hook, facades/routes. | `RebornRuntime`, global mount/registry ownership, other channels. |
| `ironclaw_telegram_v2_adapter` | `ironclaw_telegram_v2_adapter/AGENTS.md`, `Cargo.toml`, `src/lib.rs` | Telegram WASM v2 ProductAdapter tracer bullet: payload parsing, rendering, adapter implementation. | Shared adapter contracts or registry semantics. |
| `ironclaw_webui` | `ironclaw_webui/AGENTS.md`, `ironclaw_webui/CLAUDE.md`, `ironclaw_webui/README.md` | The whole WebUI host stack for Reborn WebChat v2: the `webui_v2` route surface + axum handlers + descriptor table + redacted `WebUiV2HttpError` (folded up from the former `ironclaw_webui_v2`), the Vite SPA bundle (`frontend/`), the `webui_v2_app` gateway assembly + middleware stack, the listener/serve loop, and host authentication (Env/Session/OIDC authenticators, `SessionStore`, `/auth/*` OAuth login). | Product/API business logic (consume `RebornServicesApi` only), a direct `ironclaw_product_adapters` edge, transcript storage, v1 channel code. |
| `ironclaw_slack_v2_adapter` | `ironclaw_slack_v2_adapter/AGENTS.md` | Slack v2 ProductAdapter: protocol parsing/rendering only (payloads, mrkdwn, delivery DTOs). Host-side Slack (signature verify, delivery fan-out, setup/secrets) currently lives in `ironclaw_reborn_composition`. | Signing secrets, bot tokens, network, workflow admission — the boundary test bans host concerns here. |
| `ironclaw_product_context` | `ironclaw_product_context/AGENTS.md` | Single owner of turn-origin/surface trust classification at ingress; only its `TrustedTrigger` arm mints `ScheduledTrigger`. Deliberately tiny and dependency-light (host_api + turns only). | Adding anything — its value is staying small enough for every ingress layer to call without cycles. |
| `ironclaw_reborn_identity` | `Cargo.toml`, `src/lib.rs` | Canonical identity mapping: every external identity (OAuth login, channel actor) → stable `UserId` before runtime state; filesystem-backed resolver fronted through composition. | Auth flows, session storage, provider HTTP. |

### LLM, skills, safety, UI, helpers

| Crate | Load first | Owns / go here for | Avoid moving in |
| --- | --- | --- | --- |
| `ironclaw_llm` | `ironclaw_llm/AGENTS.md`, `ironclaw_llm/CLAUDE.md`, `ironclaw_llm/Cargo.toml` | Multi-provider LLM integration: provider trait, auth, registry, retry/failover/circuit breaker/cache, tool schemas, reasoning, tracing, transcription/vision. | Engine loop ownership or product workflow. |
| `ironclaw_skills` | `ironclaw_skills/AGENTS.md` | Skill catalog, parser, gating, selector/scoring, registry, validation, v2 skill types, and pure skill-learning distillation/refinement logic. | Agent-loop execution, concrete LLM adapters, filesystem writes, or UI command routing. |
| `ironclaw_safety` | `ironclaw_safety/AGENTS.md`, `crates/ironclaw_safety/fuzz/README.md` | Prompt-injection detection, validation, sanitization, safety policy, sensitive paths, credential detection, leak scanning, fuzz/benches. | Sandbox execution, credential storage/injection, network allowlists, dispatch, UI decisions. |
| `ironclaw_gateway` | `ironclaw_gateway/AGENTS.md` | **v1-only (legacy).** v1 gateway frontend assets, layout config, bundle metadata, widget extension system. WebChat v2 assets live in `ironclaw_webui_v2_static`. | Browser API/web channel runtime (`src/channels/web/`) or product workflow; Reborn WebChat v2 work. |
| `ironclaw_tui` | `ironclaw_tui/AGENTS.md`, `ironclaw_tui/CLAUDE.md` | **v1-only today** (sole consumer: the root crate). Ratatui app, widgets, layout, render, theme, event/input loop, spinner. | Main crate channel bridge (`src/channels/tui.rs`) or backend workflow; new Reborn surfaces. |
| `ironclaw_silk_decoder` | `ironclaw_silk_decoder/AGENTS.md`, `ironclaw_silk_decoder/README.md`, `ironclaw_silk_decoder/Cargo.toml`, `ironclaw_silk_decoder/src/main.rs` | Excluded helper binary that decodes WeChat SILK v3 voice notes to WAV. | Main workspace build dependencies; keep libclang isolated. |
| `ironclaw_webui_v2_static` | `frontend/README.md`, `src/lib.rs` | WebChat v2 static SPA bundle: a thin Rust embedding harness (`static_router`) over the `static/` JS app; zero workspace deps by design. | Route semantics / listener / auth — all now in `ironclaw_webui`. |
| `ironclaw_embeddings` | `ironclaw_embeddings/AGENTS.md` | `EmbeddingProvider` trait + OpenAI/NearAI/Ollama/Bedrock impls + caching decorator. **v1-only today** (sole consumer: the root crate; not yet wired Reborn-side). | Reborn runtime wiring assumptions; memory-native's same-named local port (deliberately separate). |

## Common Change Routes

- Host API shape: `ironclaw_host_api` -> matching `docs/reborn/contracts/*.md` -> affected service/runtime crates -> `ironclaw_architecture`.
- Storage and persistence: owning domain crate for schemas/queries; preserve libSQL/PostgreSQL parity where applicable. Product workflow ledger adapters live behind `ironclaw_product_workflow`'s `storage`/`libsql`/`postgres` features; event/audit store backends live in `ironclaw_reborn_event_store`.
- Files/memory: `ironclaw_filesystem` for mount/path authority; `ironclaw_memory` for memory documents/search/chunking/indexing.
- Events/projections/outbound: `ironclaw_events` for canonical redacted events; `ironclaw_event_projections` for projection model; `ironclaw_event_streams` for transport-neutral live/replay streams; `ironclaw_outbound` for metadata-only delivery/subscription policy; adapters for concrete delivery.
- Trust/auth/approval: `ironclaw_trust` -> `ironclaw_authorization` -> `ironclaw_run_state`/`ironclaw_approvals` -> `ironclaw_capabilities` as needed.
- Hooks and prompt context: `ironclaw_hooks` for hook registration/dispatch/failure policy; `ironclaw_prompt_envelope` for model-visible untrusted or trust-labeled snippet wrapping.
- Reborn runtime execution: lane crate (`scripts`, `mcp`, `wasm`) first; `dispatcher` for routing; `host_runtime` for secrets/network/resources/redaction; `processes` for background lifecycle; `ironclaw_wasm_limiter` only for shared limiter mechanics. Use `ironclaw_engine` only for existing v1 engine maintenance.
- Reborn turns/agent loop: `ironclaw_turns` for turn coordination; `ironclaw_agent_loop` for strategy/planner/executor contracts; `ironclaw_loop_host` for host support ports. Use `ironclaw_engine` only for existing v1 CodeAct/thread runtime maintenance.
- Product adapter flow: `ironclaw_product_adapters` contracts -> `ironclaw_product_adapter_registry` installation/projection -> `ironclaw_product_workflow` orchestration -> concrete adapter crate.
- Reborn binary/composition: `ironclaw_reborn_config` for boot config; `ironclaw_reborn_composition` for production wiring; `ironclaw_reborn_cli` for commands; `ironclaw_runner` for standalone adapters/driver registry; `ironclaw_webui` for host-owned WebChat v2 listener lifecycle.
- Model/provider behavior: `ironclaw_llm`; do not leak provider auth/cache/retry concerns into engine or product workflow.
- UI presentation: `ironclaw_webui` (route surface + serve + auth) and `ironclaw_webui_v2_static` for Reborn WebChat v2; `ironclaw_tui` and `ironclaw_gateway` only for existing v1 UI maintenance. Backend API/web channel code remains under root `src/` unless the surface is the Reborn WebChat v2 host crate.

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
