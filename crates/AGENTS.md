# IronClaw Crates Map

Instructions for AI coding assistants entering `crates/`, which contains the whole Reborn stack. The v1 monolith (root `src/`) and its crates (`ironclaw_engine`, `ironclaw_tui`, `ironclaw_gateway`, `ironclaw_oauth`) have been deleted — there is no legacy tier left in here.

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

Use targeted crate tests first. Add `ironclaw_architecture` when dependency edges or layer ownership change. Run Reborn e2e when turns, runtime lanes, host services, authorization, approvals, networking, secrets, ProductSurface behavior, or capability dispatch change. Note: `scripts/check-boundaries.sh` still targets the deleted v1 `src/` tree in four of its six checks (those now pass vacuously); only the test-tier and test-skip checks scan live files under `tests/`. For `crates/`, boundary enforcement is `cargo test -p ironclaw_architecture`.

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
  -> secrets / network / outbound / processes / authorization / approvals / runtime_policy / hooks
  -> host_runtime / processes / runtime lanes (scripts, mcp, wasm, wasm_limiter)
  -> turns / threads / agent_loop / loop_host / capabilities
  -> reborn composition / product adapters / product orchestration / CLI
  -> llm / webui / webui ingress / operator
```

This sketch is informal. The authoritative layer for a crate is the
`[package.metadata.ironclaw] layer` key in its own `Cargo.toml`, which is what
`cargo test -p ironclaw_architecture` enforces.

Boundary rule: if you need an upstream crate in a low-level crate, stop and check `crates/ironclaw_architecture` plus matching Reborn contract.

## Crate Map

### Foundation and substrate

| Crate | Load first | Owns / go here for | Avoid moving in |
| --- | --- | --- | --- |
| `ironclaw_common` | `ironclaw_common/AGENTS.md`, `Cargo.toml` | Low-dependency shared types/utilities: app events, identity, trust-boundary helpers, paths, platform/env/timezone, attachment helpers. | Runtime orchestration, persistence, clients, policy, product domain logic. |
| `ironclaw_host_api` | `ironclaw_host_api/AGENTS.md`, `ironclaw_host_api/CLAUDE.md`, `docs/reborn/contracts/host-api.md` | Neutral authority vocabulary: IDs, scopes, paths, actions, decisions, resources, approvals, audit, HTTP, dispatch, runtime-policy, trust types. | Runtime execution, persistence, HTTP clients, product workflow, policy engines. |
| `ironclaw_extension_contracts` | `ironclaw_extension_contracts/CLAUDE.md`, `docs/reborn/target-architecture/families/contracts.md` | The extension tier's contract: what an installable extension declares and exposes — `ChannelAdapter`/`ToolAdapter`/`RestrictedEgress` and their DTO families, channel egress transport vocabulary, the external vendor refs, the channel-rendered auth-prompt views, channel manifest-surface descriptors, channel-identity hooks, the `Extension` trait, the `[memory]` surface, the auth recipe schema, the installation state machine, `CapabilitySurfaceKind`, and the vendor-implemented `PreferenceTargetCodec`. | Any implementation of a port declared here, the registry or installation stores, lifecycle execution or ingress routing, product workflow, vendor names, any framework or driver crate. |
| `ironclaw_product_contracts` | `ironclaw_product_contracts/CLAUDE.md`, `docs/reborn/target-architecture/families/contracts.md` | The product tier's contract: the `ProductSurface`/`BoundProductSurface`/caller membrane and its invoke/query/stream DTOs, `ChannelInboundProductSurface`, the inbound/outbound/projection product wire DTOs, the interaction-reply grammar, the operator LLM menu vocabulary, and the package-lifecycle projection vocabulary. | The `ProductSurface` implementation or the frozen command/view inventory (those are `ironclaw_product`), any handler/admission/delivery logic, projection reducers, HTTP of any kind, vendor names, any framework or driver crate. |
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
| `ironclaw_projects` | `ironclaw_projects/CLAUDE.md` | Project entity + membership ACL (live `resolve_access`, never cached) + `ProjectRepository` over `RootFilesystem` with CAS create/delete. **W2 decision: keep standalone; do not fold into composition.** | Product workflow service logic. If revisited, `ironclaw_product` is the only acceptable consumer-side target. |

### Authority, policy, state

| Crate | Load first | Owns / go here for | Avoid moving in |
| --- | --- | --- | --- |
| `ironclaw_trust` | `ironclaw_trust/AGENTS.md`, `ironclaw_trust/CLAUDE.md`, `ironclaw_trust/CONTRACT.md` | Host-controlled trust classes, policy sources, requested-vs-effective trust, invalidation. | Authorization grants, runtime dispatch, product workflow. |
| `ironclaw_authorization` | `ironclaw_authorization/AGENTS.md`, `ironclaw_authorization/CLAUDE.md` | Grant matching, leases, dispatch/spawn authorization decisions, DB-backed auth state. | Execution, approvals, run-state persistence, prompting. |
| `ironclaw_approvals` | `ironclaw_approvals/AGENTS.md`, `ironclaw_approvals/CLAUDE.md` | Durable exact-invocation approval requests, gate records, resolution, leases, and approval policy. | Dispatch, runtime execution, process lifecycle. |
| `ironclaw_resources` | `ironclaw_resources/AGENTS.md`, `ironclaw_resources/CLAUDE.md` | Reservation, reconciliation, release, quota accounting. | Runtime dispatch, product workflow, hidden costed work without reservation. |
| `ironclaw_auth` | `ironclaw_auth/AGENTS.md`, `ironclaw_auth/CLAUDE.md`, `docs/reborn/contracts/auth-product.md` | Product-facing Reborn auth-flow, secure interaction, credential account, provider exchange, continuation, cleanup contracts and fakes. | V1 route handlers/pending maps, durable secret storage, raw provider HTTP, runtime injection, extension lifecycle mutation. |
| `ironclaw_runtime_policy` | `ironclaw_runtime_policy/AGENTS.md`, `ironclaw_runtime_policy/CLAUDE.md`, `docs/reborn/contracts/runtime-profiles.md` | Runtime profile resolver and runtime selection policy. | Runtime startup, action dispatch, product strategy outside selection. |
| `ironclaw_outbound` | `ironclaw_outbound/AGENTS.md`, `ironclaw_outbound/CLAUDE.md` | Metadata-only outbound egress policy, notification opt-in, projection subscription cursors, delivery attempt/status metadata. | Transport sends, concrete Slack/Telegram/Web payload validation, transcript/projection mutation. |
| `ironclaw_hooks` | `ironclaw_hooks/CLAUDE.md`, `Cargo.toml`, `src/lib.rs` | Reborn loop hook framework: trust-tiered hook contracts, sealed decision sinks, predicates, ordering, dispatch, telemetry, and failure policy. | Authority grants, runtime-policy bypasses, ambient secrets/network/filesystem handles, extension installation. |

### Host services and runtime lanes

| Crate | Load first | Owns / go here for | Avoid moving in |
| --- | --- | --- | --- |
| `ironclaw_secrets` | `ironclaw_secrets/AGENTS.md`, `ironclaw_secrets/CLAUDE.md` | Secret metadata, encrypted repositories, leases, one-shot consumption, legacy/db stores. | Raw secret exposure, provider HTTP, injection beyond mediated handoff. |
| `ironclaw_network` | `ironclaw_network/AGENTS.md`, `ironclaw_network/CLAUDE.md`, `docs/reborn/contracts/network.md` | Network policy boundary, URL targets, resolver, hardened transport, host/provider HTTP egress. | Runtime-lane behavior above boundary or manual credential injection. |
| `ironclaw_host_runtime` | `ironclaw_host_runtime/AGENTS.md`, `ironclaw_host_runtime/CLAUDE.md` | Host-side Reborn service composition: production services, obligations, HTTP egress, redaction, secrets/network/resource mediation. | Product workflow, runtime-specific request shapes, duplicate network/secret logic. |
| `ironclaw_processes` | `ironclaw_processes/AGENTS.md`, `ironclaw_processes/CLAUDE.md` | Process lifecycle, cancellation, stores, status/output helpers, `ProcessHost`, wrappers. | Authorization, approval policy, runtime lane internals beyond adapter contracts. |
| `ironclaw_scripts` | `ironclaw_scripts/AGENTS.md`, `ironclaw_scripts/CLAUDE.md` | Script runtime lane over host-mediated filesystem/events/resources/dispatcher/HTTP, Docker/backend output parsing. | Manual credentials, direct provider HTTP, duplicated dispatcher/process/resource policy. |
| `ironclaw_mcp` | `ironclaw_mcp/AGENTS.md`, `ironclaw_mcp/CLAUDE.md` | MCP runtime lane, execution request/result types, JSON-RPC exchange, client abstraction, HTTP adapter, resource accounting. | Direct outbound networking, ad-hoc credential injection, product workflow. |
| `ironclaw_wasm` | `ironclaw_wasm/AGENTS.md`, `ironclaw_wasm/CLAUDE.md`, `docs/reborn/contracts/wasm.md`, `wit/tool.wit` | WASM runtime lane, component/WIT bindings, folded `wasm_sandbox_core` primitives, store, host adapters, runtime config. | Privileged host effects outside mediated APIs; copied secrets/network/resource logic; product/runtime-specific dependencies inside `wasm_sandbox_core`. |
| `ironclaw_wasm_limiter` | `Cargo.toml`, `src/lib.rs` | Shared `wasmtime::ResourceLimiter` for WASM tool and hook runtimes. | Product adapter workflow, policy decisions, or runtime-specific side effects beyond limiter accounting. |
| `ironclaw_extensions` | `ironclaw_extensions/AGENTS.md`, `ironclaw_extensions/CLAUDE.md` | Declarative extension manifests (`src/v2.rs` and `src/v3.rs`; v3 is the current schema), capability descriptors, side-effect-free in-memory registry, installation records. | Execution of any kind (WASM/MCP/process), secrets, trust decisions. |
| `ironclaw_process_sandbox` | `ironclaw_process_sandbox/CLAUDE.md` | Typed `SandboxProcessPlan` contract and validation only: install/credentialed-run phase separation in plan types. No production execution backend is wired for this capability today. | Process lifecycle/stores (`ironclaw_processes`); raw Docker flags for extensions; adding an execution backend here. |

### Turns, threads, loops

| Crate | Load first | Owns / go here for | Avoid moving in |
| --- | --- | --- | --- |
| `ironclaw_turns` | `ironclaw_turns/AGENTS.md`, `ironclaw_turns/CLAUDE.md` | Host-layer turn coordination and admission *authority*: requests/responses, coordinator, runner claim projection, loop-exit **validation** (applier, policy, violation taxonomy), turn store projection. | Product adapter rendering, raw runtime lanes, UI behavior, and the loop-tier contract itself (`ironclaw_loop_contracts`). |
| `ironclaw_loop_contracts` | `ironclaw_loop_contracts/CLAUDE.md`, `docs/reborn/target-architecture/families/contracts.md` | The loop-tier contract: the eleven `Loop*Port` traits + `AgentLoopDriverHost`, `AgentLoopDriver`, run-profile vocabulary, prompt/model/skill/instruction/milestone contract types, the `LoopExit` claim DTOs, the redacted checkpoint payload. | Any implementation of a port declared here, the turn coordinator/state store/exit applier, a dependency on `ironclaw_turns` (the direction inverts), any framework or driver crate. |
| `ironclaw_threads` | `ironclaw_threads/AGENTS.md`, `ironclaw_threads/CLAUDE.md` | Canonical session thread/transcript service contracts, identifiers, tool-result references, db/in-memory stores. | Product delivery policy or model/provider behavior. |
| `ironclaw_conversations` | `ironclaw_conversations/AGENTS.md`, `ironclaw_conversations/CLAUDE.md` | Conversation binding, session thread contracts, inbound/state store, libSQL/Postgres conversation persistence. | Capability runtime internals or UI transport. |
| `ironclaw_agent_loop` | `ironclaw_agent_loop/AGENTS.md`, `ironclaw_agent_loop/CLAUDE.md` | Agent-loop framework state, planner/executor, strategy/family contracts, test support. | Product adapters, transport, concrete provider auth. |
| `ironclaw_loop_host` | `ironclaw_loop_host/AGENTS.md`, `ironclaw_loop_host/CLAUDE.md` | Loop host support services: capability/input ports, allow sets, input queue, identity/skill context, cancellation. | Owning core loop strategy or runtime lane execution. |
| `ironclaw_capabilities` | `ironclaw_capabilities/AGENTS.md`, `ironclaw_capabilities/CLAUDE.md` | Caller-facing `CapabilityHost` invoke/resume/spawn workflow, obligation seams, conformance helpers, and the host-private `ReplayPayloadStore` (raw gate/auth resume replay payload, never model-visible). | Process lifecycle APIs, direct concrete runtime dependencies. |

### Product, adapters, Reborn binary

| Crate | Load first | Owns / go here for | Avoid moving in |
| --- | --- | --- | --- |
| `ironclaw_runner` | `ironclaw_runner/AGENTS.md`, `ironclaw_runner/CLAUDE.md` | **Internal runner control plane and loop-runtime assembly** (sole production consumer: `ironclaw_reborn_composition`; test harnesses may use it directly): scheduler, per-run executor, driver registry, planned/text driver adapters, loop host factory, exit-applier wiring, home/profile/doctor support. | Treating it as a public composition root; V1 root runtime imports unless explicitly bridged. |
| `ironclaw_reborn_config` | `ironclaw_reborn_config/AGENTS.md`, `Cargo.toml`, `src/lib.rs` | Boot configuration contracts for standalone Reborn binary. | Runtime execution or product adapter behavior. |
| `ironclaw_reborn_composition` | `ironclaw_reborn_composition/AGENTS.md`, `ironclaw_reborn_composition/CLAUDE.md` | Service-shaped production composition root for Reborn. | Low-level policy internals that belong to service crates. |
| `ironclaw_reborn_openai_compat` | `ironclaw_reborn_openai_compat/AGENTS.md`, `ironclaw_reborn_openai_compat/CLAUDE.md` | Reborn-native OpenAI-compatible API route descriptors, Chat/Responses DTOs, sanitized error envelope, fail-closed route fragment, and the durable ref/idempotency storage adapters. The crate declares **no cargo features** — every one of those surfaces compiles unconditionally. | Direct LLM proxying, listener binding, ProductSurface internals/direct runtime wiring, or filesystem access outside `OpenAiCompatRefStore`. |
| `ironclaw_first_party_extensions` | `ironclaw_first_party_extensions/AGENTS.md`, `Cargo.toml` | Concrete first-party userland extension implementations and deterministic tool behavior behind scoped handles. | Host runtime composition, loop-facing ports, ambient runtime authority, dispatcher/network/secrets handles. |
| `ironclaw_first_party_extension_ports` | `ironclaw_first_party_extension_ports/AGENTS.md`, `Cargo.toml` | Loop-facing adapters for first-party extensions: skill activation/context/execution ports over loop-host and turn-run contracts. | Concrete tool behavior, host runtime composition, product workflow, raw host authority. |
| `ironclaw` | `ironclaw_reborn_cli/AGENTS.md` | Standalone Reborn CLI, command files, CLI context, shell completions, doctor/home/profile commands. | V1 runtime imports, root `ironclaw_legacy` deps, side effects in pure commands. |
| `ironclaw_product` | `ironclaw_product/AGENTS.md`, `ironclaw_product/CLAUDE.md` | Product contracts and orchestration: adapters, identity, inbound turns, bindings, idempotency, ProductSurface views/commands/capabilities, redaction, and the durable ledger adapters. Its only cargo feature is `test-support` — it is not an evidence minter, and WS1.5 deleted both of its re-export paths to the protocol-auth mint family. | Host runtime internals, specific runtime lanes, direct provider transports, or backend access outside typed ports. |
| `ironclaw_telegram_v2_adapter` | `ironclaw_telegram_v2_adapter/AGENTS.md`, `Cargo.toml`, `src/lib.rs` | Telegram Bot API **protocol engine only**: payload normalization (`payload.rs`) and outbound request rendering (`render.rs`). No I/O, no secrets. | The `ChannelAdapter` impl itself (that is `ironclaw_telegram_extension`); host verification/egress. |
| `ironclaw_telegram_extension` | `ironclaw_telegram_extension/AGENTS.md`, `Cargo.toml`, `src/lib.rs` | The Telegram **`ChannelAdapter`**: live inbound/outbound, webhook registration hooks, preference targets, attachment transfer — layered on `ironclaw_telegram_v2_adapter`'s protocol work. Stays free of raw token bytes. | Bot API payload/render logic; host signing secrets, admission, or egress credentials. |
| `ironclaw_webui` | `ironclaw_webui/AGENTS.md`, `ironclaw_webui/CLAUDE.md`, `ironclaw_webui/README.md` | The whole WebUI host stack for Reborn WebChat v2: the `webui_v2` route surface + axum handlers + descriptor table + redacted `WebUiV2HttpError` (folded up from the former `ironclaw_webui_v2` crate), the Vite SPA bundle (`frontend/`), the `webui_v2_app` gateway assembly + middleware stack, the listener/serve loop, and host authentication (Env/Session/OIDC authenticators, `SessionStore`, `/auth/*` OAuth login). | Product/API business logic, product services, lower substrates, transcript storage, and v1 channel code. Use `ProductSurface`; direct `ironclaw_product` imports are DTOs/descriptors only. |
| `ironclaw_extension_host` | `Cargo.toml`, `src/lib.rs` | Generic channel-host assembly binding installed extensions to inbound/outbound channel surfaces for the Reborn product surface: ingress registration, extension lifecycle command execution, delivery, and per-extension idempotency ledgers. | Host authority (signing secrets, bot tokens, network egress) and workflow admission; keep those in lower host crates and `ironclaw_reborn_composition`. |
| `ironclaw_slack_extension` | `ironclaw_slack_extension/AGENTS.md` | Slack `ChannelAdapter`: protocol parsing/rendering (payloads, mrkdwn, delivery DTOs, preference targets, attachment transfer). Host-side ingress — signature verification and delivery — is generic and lives in `ironclaw_extension_host` (`src/ingress/verifier.rs`, `src/channel_host.rs`), driven by the manifest recipe, not by Slack-specific host code. | Signing secrets, bot tokens, network, workflow admission — the boundary test bans host concerns here. |
| `ironclaw_reborn_identity` | `Cargo.toml`, `src/lib.rs` | Canonical identity mapping: every external identity (OAuth login, channel actor) → stable `UserId` before runtime state; filesystem-backed resolver fronted through composition. | Auth flows, session storage, provider HTTP. |

### LLM, skills, safety, UI, helpers

| Crate | Load first | Owns / go here for | Avoid moving in |
| --- | --- | --- | --- |
| `ironclaw_llm` | `ironclaw_llm/AGENTS.md`, `ironclaw_llm/CLAUDE.md`, `ironclaw_llm/Cargo.toml` | Multi-provider LLM integration: provider trait, auth, registry, retry/failover/circuit breaker/cache, tool schemas, reasoning, tracing, transcription/vision. | Engine loop ownership or product workflow. |
| `ironclaw_skills` | `ironclaw_skills/AGENTS.md` | Skill catalog, parser, gating, selector/scoring, registry, validation, v2 skill types, and pure skill-learning distillation/refinement logic. | Agent-loop execution, concrete LLM adapters, filesystem writes, or UI command routing. |
| `ironclaw_safety` | `ironclaw_safety/AGENTS.md`, `crates/ironclaw_safety/fuzz/README.md` | Prompt-injection detection, validation, sanitization, safety policy, sensitive paths, credential detection, leak scanning, fuzz/benches. | Sandbox execution, credential storage/injection, network allowlists, dispatch, UI decisions. |
| `ironclaw_silk_decoder` | `ironclaw_silk_decoder/AGENTS.md`, `ironclaw_silk_decoder/README.md`, `ironclaw_silk_decoder/Cargo.toml`, `ironclaw_silk_decoder/src/main.rs` | Excluded helper binary that decodes WeChat SILK v3 voice notes to WAV. | Main workspace build dependencies; keep libclang isolated. |

## Common Change Routes

- Host API shape: `ironclaw_host_api` -> matching `docs/reborn/contracts/*.md` -> affected service/runtime crates -> `ironclaw_architecture`.
- Storage and persistence: owning domain crate for schemas/queries; preserve libSQL/PostgreSQL parity where applicable. Product ledger adapters compile unconditionally in `ironclaw_product` (there are no `storage`/`libsql`/`postgres` features — backend choice is composition's, via the `RootFilesystem` mount catalog); event/audit store backends live in `ironclaw_reborn_event_store`.
- Files/memory: `ironclaw_filesystem` for mount/path authority; `ironclaw_memory` for memory documents/search/chunking/indexing.
- Events/projections/outbound: `ironclaw_events` for canonical redacted events; `ironclaw_event_projections` for projection model; `ironclaw_event_streams` for transport-neutral live/replay streams; `ironclaw_outbound` for metadata-only delivery/subscription policy; adapters for concrete delivery.
- Trust/auth/approval: `ironclaw_trust` -> `ironclaw_authorization` -> `ironclaw_approvals` -> `ironclaw_capabilities` as needed.
- Hooks and prompt context: `ironclaw_hooks` for hook registration/dispatch/failure policy; `ironclaw_prompt_envelope` for model-visible untrusted or trust-labeled snippet wrapping.
- Reborn runtime execution: lane crate (`scripts`, `mcp`, `wasm`) first; `ironclaw_capabilities` for the authorized dispatch path; `host_runtime` for secrets/network/resources/redaction; `processes` for background lifecycle; `ironclaw_wasm_limiter` only for shared limiter mechanics.
- Reborn turns/agent loop: `ironclaw_turns` for turn coordination; `ironclaw_agent_loop` for strategy/planner/executor contracts; `ironclaw_loop_host` for host support ports.
- Product adapter flow: `ironclaw_product` contracts and `adapter_registry` manifest projection -> `ironclaw_product` orchestration -> concrete adapter crate.
- Reborn binary/composition: `ironclaw_reborn_config` for boot config; `ironclaw_reborn_composition` for production wiring; the `ironclaw_reborn_cli/` directory (package `ironclaw`) for commands; `ironclaw_runner` for standalone adapters/driver registry; `ironclaw_webui` for host-owned WebChat v2 listener lifecycle.
- Model/provider behavior: `ironclaw_llm`; do not leak provider auth/cache/retry concerns into engine or product orchestration.
- UI presentation: `ironclaw_webui` owns the Reborn WebChat route surface, Vite SPA, serving, and auth. It is the only UI surface — the v1 TUI and gateway crates are gone.

## Testing

Prefer narrow tests during iteration:

```bash
cargo test -p ironclaw_host_api
cargo test -p ironclaw_network network_policy_contract
cargo test -p ironclaw_outbound --all-features
cargo test -p ironclaw_product
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
