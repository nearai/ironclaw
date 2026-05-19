# ironclaw_product_workflow

Product-facing workflow facade for IronClaw Reborn (issue #3280).

## Purpose

Sits between product adapters and host-layer Reborn services. Owns the product
action orchestration so adapters (Web, API, CLI, Telegram, etc.) do not each
reimplement binding resolution, message staging, idempotency, busy/deferred
handling, gate routing, mission routing, and redacted acknowledgements.

## Key types

| Type | Role |
|------|------|
| `DefaultProductWorkflow` | Top-level orchestrator implementing `ProductWorkflow` trait |
| `InboundTurnService` / `DefaultInboundTurnService` | User-message turn submission path |
| `ConversationBindingService` | Resolves external adapter refs → canonical Reborn identifiers |
| `IdempotencyLedger` | Durable action deduplication port |
| `ProductInboundAction` | Durable ledger record for inbound actions |
| `RebornServicesApi` / `RebornServices` | Native WebChat v2 facade — stable surface beta WebUI route handlers consume in place of reaching into turn coordination, thread stores, runtime lanes, dispatchers, or capability hosts. Enforces caller ownership of the thread before any turn mutation; rejects stale or attacker-supplied `gate_ref` on denied/cancelled gate resolutions; refuses persistent (`always: true`) approvals until an approval-policy port lands |
| `MemoryProductFacade` | #3287 first-slice memory product boundary. Resolves product targets and relative paths into typed Reborn memory service requests, carries actor/surface/purpose authority, and keeps v1 `Workspace` out of Reborn product memory code |

## Dependencies

- `ironclaw_product_adapters` — trait definitions, envelope/ack types, `ProjectionStream` for SSE
- `ironclaw_turns` — turn coordinator, scope, IDs
- `ironclaw_threads` — session thread service contract
- `ironclaw_host_api` — canonical identifiers (TenantId, UserId, etc.)
- `ironclaw_memory` — focused Reborn memory service contracts and prompt-write safety vocabulary

## Boundary rules

Must NOT depend on: `ironclaw_dispatcher`, `ironclaw_extensions`,
`ironclaw_host_runtime`, `ironclaw_mcp`, `ironclaw_wasm`, `ironclaw_scripts`,
`ironclaw_network`, `ironclaw_engine`, `ironclaw_gateway`.

Agent-loop note: product-facing turns enter through workflow services and
canonical turn submission. Do not shortcut directly to `AgentLoopDriver`,
`PlannedDriver`, host runtime services, or loop host factories from adapters or
workflow callers.

Memory note: `MemoryProductFacade` is a contract boundary only in the first
#3287 slice. It may consume typed `ironclaw_memory` service traits, but must not
depend on v1 `Workspace`, `WorkspacePool`, `WorkspaceResolver`, raw memory
repositories, raw provider clients, secret stores, filesystem/process/network
handles, or HostRuntime internals. LLM-facing memory tools must eventually enter
through HostRuntime-mediated capability execution, not by directly calling
memory services from route/tool code.

## Test support

Enable `test-support` feature for in-memory fakes:
- `FakeConversationBindingService`
- `FakeIdempotencyLedger`
- `FakeInboundTurnService`
