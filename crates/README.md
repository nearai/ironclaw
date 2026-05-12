# IronClaw crates

This directory contains the Rust crates that split IronClaw into smaller, reviewable boundaries. Most crates are Reborn system-service crates: they hold one slice of host authority, storage, policy, runtime composition, or product UI glue.

Use this page as a human map before opening individual crate docs or source files.

## Mental model

IronClaw Reborn keeps authority narrow and explicit:

1. **Contracts describe authority**: `ironclaw_host_api` and adjacent contract crates define scoped identities, policies, requests, decisions, and DTOs.
2. Policy gates decide: authorization, trust, runtime policy, resources, approvals, secrets, safety, filesystem, and network crates each own one kind of decision or side effect.
3. Capability hosts coordinate: capabilities, dispatcher, processes, scripts, MCP, WASM, and host-runtime crates compose validated requests into sandboxed execution.
4. State is durable and replayable: events, run state, threads, conversations, memory, outbound, and event projections keep the host observable without leaking secrets.
5. Product surfaces adapt: engine, loop support, gateway, TUI, skills, and product adapters turn those lower-level boundaries into agent and user experiences.

A good rule of thumb: if a change adds new authority or persistence, put it in the crate that owns that boundary instead of threading it through a UI or runtime crate.

## Crate groups

### Core vocabulary and shared contracts

| Crate directory | Package | Human context |
| --- | --- | --- |
| `ironclaw_common` | `ironclaw_common` | Shared workspace types and utilities that are not authority-bearing enough to belong in `ironclaw_host_api`. Keep this small. |
| `ironclaw_host_api` | `ironclaw_host_api` | Canonical Reborn authority vocabulary: actors, scopes, policies, capability requests, decisions, obligations, and host-facing data contracts. Runtime behavior belongs elsewhere. |
| `ironclaw_runtime_policy` | `ironclaw_runtime_policy` | Resolves runtime profiles from host configuration and policy inputs. Use it when choosing what runtime shape a capability may use. |
| `ironclaw_architecture` | `ironclaw_architecture` | Workspace architecture contract tests. It has no production role; it fails builds when crate dependency boundaries drift. |

### Authority, safety, and policy gates

| Crate directory | Package | Human context |
| --- | --- | --- |
| `ironclaw_authorization` | `ironclaw_authorization` | Evaluates host API authority contracts before capability execution. It should not execute work, reserve resources, or prompt users. |
| `ironclaw_approvals` | `ironclaw_approvals` | Resolves durable approval requests and issues scoped authorization leases. It does not own prompting UI or runtime execution. |
| `ironclaw_trust` | `ironclaw_trust` | Host-controlled trust-class policy engine. Use it for decisions about how much trust a runtime, extension, or input receives. |
| `ironclaw_resources` | `ironclaw_resources` | Resource reservation governor. Owns budget/reservation mechanics, not runtime dispatch. |
| `ironclaw_safety` | `safety_pipeline` | Prompt-injection defense, input validation, secret-leak detection, and safety policy enforcement. |
| `ironclaw_secrets` | `ironclaw_secrets` | Tenant-scoped secret storage and leasing behind opaque `SecretHandle` values. It stores/leases material; other crates decide when leases are allowed and where to inject them. |
| `ironclaw_network` | `ironclaw_network` | Network policy and HTTP egress boundary. Resolves DNS, rejects disallowed/private targets when configured, and owns host-mediated outbound HTTP. |
| `ironclaw_filesystem` | `ironclaw_filesystem` | Scoped filesystem service. Use it for host-controlled path access, not direct runtime path handling. |

### Capability execution and runtime lanes

| Crate directory | Package | Human context |
| --- | --- | --- |
| `ironclaw_capabilities` | `ironclaw_capabilities` | Caller-facing capability invocation host. Coordinates authorization, approvals, run-state transitions, and neutral runtime dispatch. |
| `ironclaw_dispatcher` | `ironclaw_dispatcher` | Composition-only runtime dispatch contracts. Wires validated extension descriptors to runtime lanes; it does not parse manifests or grant authority. |
| `ironclaw_processes` | `ironclaw_processes` | Host-tracked background process lifecycle. Owns lifecycle mechanics, not capability policy. |
| `ironclaw_scripts` | `ironclaw_scripts` | Script/CLI capability runner contracts. Executes declared commands through a host-selected backend. |
| `ironclaw_mcp` | `ironclaw_mcp` | Adapts manifest-declared MCP tools into IronClaw capabilities without granting ambient filesystem, secret, or network authority. |
| `ironclaw_wasm` | `ironclaw_wasm` | Reborn WASM component runtime lane. Owns component-model/WIT runtime surface and sandboxed WASM execution details. |
| `ironclaw_host_runtime` | `ironclaw_host_runtime` | Narrow facade upper Reborn services depend on. Provides `HostRuntime` plus production composition around capability hosting. |

### Durable state, eventing, and read models

| Crate directory | Package | Human context |
| --- | --- | --- |
| `ironclaw_events` | `ironclaw_events` | Redacted runtime/audit vocabulary plus durable append-log traits. Use it for observable history, not current state. |
| `ironclaw_reborn_event_store` | `ironclaw_reborn_event_store` | Concrete Reborn event/audit store backends and backend-profile validation. Depends on `ironclaw_events`; keeps storage adapters out of event vocabulary. |
| `ironclaw_event_projections` | `ironclaw_event_projections` | Product-facing read models over durable runtime and audit logs. Upper layers should consume these DTOs rather than parse event rows directly. |
| `ironclaw_run_state` | `ironclaw_run_state` | Current lifecycle state for host-managed invocations. Events are history; run state answers “what is happening now?” |
| `ironclaw_threads` | `ironclaw_threads` | Canonical session thread and transcript service contracts. Use it for durable thread/transcript ownership. |
| `ironclaw_conversations` | `ironclaw_conversations` | Conversation binding and session-thread contracts that connect product conversation concepts to Reborn threads. |
| `ironclaw_memory` | `ironclaw_memory` | Memory document service adapters. This is for workspace/memory document semantics, not arbitrary transcript deletion. |
| `ironclaw_outbound` | `ironclaw_outbound` | Metadata-only outbound state: notification policy, projection subscription cursors, and delivery status. It does not own transport delivery or payload content. |

### Product, agent loop, and user surfaces

| Crate directory | Package | Human context |
| --- | --- | --- |
| `ironclaw_reborn` | `llm_gateway` | Standalone Reborn composition and adapters. Despite the package name, this is the high-level Reborn composition crate. |
| `ironclaw_loop_support` | `ironclaw_loop_support` | Adapts durable Reborn support boundaries into the narrow agent-loop host port. It should not own provider clients or runtime dispatchers. |
| `ironclaw_turns` | `ironclaw_turns` | Host-layer turn coordination contracts. Use it for turn lifecycle boundaries between loop/product code and host services. |
| `ironclaw_product_adapters` | `ironclaw_product_adapters` | Product-adapter contracts for mapping Reborn state and events into product-facing shapes. |
| `ironclaw_product_workflow` | `ironclaw_product_workflow` | Product-facing workflow facade: inbound turn service, idempotency ledger, binding resolution. |
| `ironclaw_engine` | `ironclaw_engine` | Unified thread-capability-CodeAct execution engine. It is closer to product/agent orchestration than low-level host policy. |
| `ironclaw_skills` | `ironclaw_skills` | Skill selection, scoring, and management. |
| `ironclaw_gateway` | `ironclaw_gateway` | Browser gateway frontend assets, layout configuration, and widget extension system. |
| `ironclaw_tui` | `ironclaw_tui` | Modular Ratatui-based terminal UI. |

## Where to make common changes

- **New capability type or host API contract**: start in `ironclaw_host_api`, then update authorization/capability/runtime crates that consume it.
- **Authorization or approval behavior**: use `ironclaw_authorization` for policy decisions and `ironclaw_approvals` for approval lease resolution.
- **Secret storage or leasing**: use `ironclaw_secrets`; do not put SQL or crypto details in engine, gateway, or runtime lanes.
- **Network or filesystem access**: use `ironclaw_network` or `ironclaw_filesystem`; runtimes should ask host services instead of bypassing them.
- **WASM, MCP, or script execution**: use the corresponding runtime-lane crate plus `ironclaw_capabilities`/`ironclaw_dispatcher` for coordination.
- **Durable event history**: use `ironclaw_events` for contracts and `ironclaw_reborn_event_store` for backend adapters.
- **Current invocation state**: use `ironclaw_run_state`, not event logs.
- **User-visible read models**: prefer `ironclaw_event_projections` or `ironclaw_product_adapters` over parsing storage rows in UI code.
- **Agent loop/product orchestration**: use `ironclaw_loop_support`, `ironclaw_turns`, `ironclaw_engine`, or `ironclaw_reborn` depending on layer.
- **Web or terminal UI**: use `ironclaw_gateway` or `ironclaw_tui`; keep authority and persistence in lower crates.

## Boundary rules

- Keep crate-owned logic in the owning crate. Avoid reimplementing module-specific setup in `src/main.rs`, `src/app.rs`, gateway, or TUI code.
- Prefer extending existing traits and service boundaries over adding one-off integration paths.
- Do not give runtime lanes ambient access to secrets, filesystem, network, or process control. Route through host services.
- Treat `ironclaw_host_api` as the shared contract layer. It may define authority-bearing shapes; it should not perform side effects.
- Use `ironclaw_architecture` tests when dependency boundaries need to become enforceable.
- If behavior changes, check `../CLAUDE.md`, `../AGENTS.md`, and `../FEATURE_PARITY.md` for test/doc update expectations.

## Quick commands

From the repository root:

```bash
cargo fmt
cargo clippy --all --benches --tests --examples --all-features
cargo test
```

For targeted crate work, prefer the narrowest command first:

```bash
cargo test -p ironclaw_secrets --features libsql
cargo clippy -p ironclaw_network --tests -- -D warnings
```

Some crates are feature-gated or test backends conditionally. Read the crate-level docs and tests before assuming a command covers PostgreSQL, libSQL, WASM, or integration behavior.
