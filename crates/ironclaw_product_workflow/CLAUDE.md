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

## Dependencies

- `ironclaw_product_adapters` — trait definitions, envelope/ack types, `ProjectionStream` for SSE
- `ironclaw_turns` — turn coordinator, scope, IDs
- `ironclaw_threads` — session thread service contract
- `ironclaw_host_api` — canonical identifiers (TenantId, UserId, etc.)

## Boundary rules

Must NOT depend on: `ironclaw_dispatcher`, `ironclaw_extensions`,
`ironclaw_host_runtime`, `ironclaw_mcp`, `ironclaw_wasm`, `ironclaw_scripts`,
`ironclaw_network`, `ironclaw_engine`, `ironclaw_gateway`.

Agent-loop note: product-facing turns enter through workflow services and
canonical turn submission. Do not shortcut directly to `AgentLoopDriver`,
`PlannedDriver`, host runtime services, or loop host factories from adapters or
workflow callers.

WebUI-facing facade methods must bind browser thread ids through
`SessionThreadService` using a `ThreadScope` derived from the authenticated
caller before accepting messages, streaming events, canceling runs, or resolving
gates. Browser/session metadata is not authority by itself, and send-message
must not implicitly create missing threads.

## Test support

Enable `test-support` feature for in-memory fakes:
- `FakeConversationBindingService`
- `FakeIdempotencyLedger`
- `FakeInboundTurnService`
