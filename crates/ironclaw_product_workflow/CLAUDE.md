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

## Dependencies

- `ironclaw_product_adapters` — trait definitions, envelope/ack types
- `ironclaw_turns` — turn coordinator, scope, IDs
- `ironclaw_threads` — session thread service contract
- `ironclaw_host_api` — canonical identifiers (TenantId, UserId, etc.)

## Boundary rules

Must NOT depend on: `ironclaw_dispatcher`, `ironclaw_extensions`,
`ironclaw_host_runtime`, `ironclaw_mcp`, `ironclaw_wasm`, `ironclaw_scripts`,
`ironclaw_network`, `ironclaw_engine`, `ironclaw_gateway`.

## Test support

Enable `test-support` feature for in-memory fakes:
- `FakeConversationBindingService`
- `FakeIdempotencyLedger`
- `FakeInboundTurnService`

## Service ports

The workflow crate defines port traits for each non-`UserMessage` dispatch
arm. Production wiring (in `src/app.rs` / a composition crate) implements
these by wrapping existing host-layer services. Tests wire `Fake*` impls
from the `test-support` feature.

| Trait | Wrapped by production | Closes AC |
|-------|-----------------------|-----------|
| `BeforeInboundPolicy` | v1 `HookPoint::BeforeInbound` ported into `inbound_turn.rs` | #3280 AC #8, #9 |
| `ProductCommandRouter` | Reborn's `AgentCommandService` (#3286 owns the full matrix) | #3280 AC #10 |
| `ApprovalInteractionService` | `ironclaw_approvals::ApprovalResolver` (#3094) | #3280 AC #11 |
| `AuthInteractionService` | `ironclaw_authorization::CapabilityDispatchAuthorizer` (#3094) | #3280 AC #11 |
| `LinkedThreadActionService` | TBD per-product handler | #3280 AC #13 |
| `MissionService` | `ironclaw_engine` MissionManager once #3278 ships | #3280 AC #12, #13 |
| `SystemActionService` | TBD typed system action handlers | #3280 AC #15 |
| `ProjectionSubscriptionAuthority` | `ironclaw_outbound::OutboundPolicyService::authorize_subscription` (#3266 / PR #3542) | #3280 AC #14 |

Each port is **optional** on `DefaultProductWorkflow` (builder methods
`with_*`). When a port is unset, the corresponding dispatch arm returns a
redacted permanent `Rejected` ack — adapters must wire the service before
serving the matching action kind in production.

## No mission auto-attach

`DefaultProductWorkflow` does **not** route `ProductInboundPayload::UserMessage`
to `MissionService` under any condition. Missions only fire via explicit
`MissionActionPayload`. The v1 bug shape (`src/bridge/router.rs::fire_event_missions_for_message`
pattern-matched message text against `MissionCadence::OnEvent` regex and
side-fired missions) is closed by AC #12 of #3280 and locked in by the
`ordinary_user_message_text_never_reaches_mission_service` regression test
in `tests/product_workflow_contract.rs`.
