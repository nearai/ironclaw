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
| `ProductConversationBindingService` | Adapter from product workflow bindings to `ironclaw_conversations` with trusted installation→tenant mapping |
| `StaticProductInstallationResolver` / `ProductInstallationScope` | Host-owned installation registry used by local-dev/tests to select tenant and default agent/project scope |
| `IdempotencyLedger` | Durable action deduplication port |
| `InMemoryIdempotencyLedger` | Local-dev/test ledger with in-flight lease recovery semantics |
| `ProductInboundAction` | Durable ledger record for inbound actions |
| `ProductCommandAdmissionService` | Source/auth-aware admission port that decides whether a typed product command may execute |
| `ProductCommandService` | Reborn-native product command execution port for already-admitted typed commands |
| `ApprovalInteractionService` / `DefaultApprovalInteractionService` | Approval-only product/WebUI boundary for listing redacted pending approval gates and resolving click approve/deny through canonical approval resolver + turn coordinator ports |
| `RunStateApprovalInteractionReadModel` | Canonical read model that returns status-bearing approval gates from scoped approval-request records plus the parked turn-run locator; `ApprovalInteractionService::list_pending` filters those records to pending UI DTOs |
| `RebornServicesApi` / `RebornServices` | Native WebChat v2 facade — stable surface beta WebUI route handlers consume in place of reaching into turn coordination, thread stores, runtime lanes, dispatchers, or capability hosts. Enforces caller ownership of the thread before any turn mutation; rejects stale or attacker-supplied `gate_ref` on denied/cancelled gate resolutions; refuses persistent (`always: true`) approvals until an approval-policy port lands |

## Dependencies

- `ironclaw_product_adapters` — trait definitions, envelope/ack types, `ProjectionStream` for SSE
- `ironclaw_approvals` / `ironclaw_authorization` — canonical approval resolution and scoped lease issue ports used by approval interactions
- `ironclaw_auth` — typed product-auth continuation events consumed by the workflow auth bridge
- `ironclaw_conversations` — canonical actor/conversation binding and thread route ownership
- `ironclaw_run_state` — approval request store contract surfaced through approval resolution/read-model ports
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

Product commands are not turns. Adapters may parse slash syntax at the edge, but
`ProductInboundPayload::Command` must enter the workflow as normalized command
payloads. The source/auth decision belongs to `ProductCommandAdmissionService`;
the source-agnostic command model must not know which product surface produced
the command. Admitted commands dispatch through `ProductCommandService`, not
`InboundTurnService`, v1 `SubmissionParser`, v1 command routers, or agent-loop
command handlers.

Approval interactions are click-approval only. Pending approval DTOs must be
redacted, scoped, and derived from canonical run-state/approval records or a
projection read model built from them. Approve/deny decisions must go through
`ApprovalResolutionPort` and `TurnCoordinator`; product/WebUI code must not
directly execute tools, mutate approval stores ad hoc, or implement
`AlwaysAllow` before a durable approval-policy port exists. High-value signing
and attested approvals require a separate service shape with canonical payload
attestation and must not be folded into this redacted click-approval DTO.

WebUI-facing facade methods must bind browser thread ids through
`SessionThreadService` using a `ThreadScope` derived from the authenticated
caller before accepting messages, streaming events, canceling runs, or resolving
gates. Browser/session metadata is not authority by itself, and send-message
must not implicitly create missing threads.

WebUI-facing facade errors must expose stable, sanitized taxonomy. Keep
`RebornServicesErrorCode` aligned with coarse transport/status shape and
`RebornServicesErrorKind` aligned with M1-renderable user-safe families such as
validation, duplicate, busy, participant denied, blocked approval/auth/resource,
replay/timeline unavailable, service unavailable, conflict, not found, and
internal. Do not surface backend strings, host paths, provider/runtime details,
raw prompts, tool args, or secrets through the facade error payload.

Product adapter bindings must choose `TenantId` only from trusted host
installation configuration, never from inbound adapter payloads. Default
`AgentId`/`ProjectId` for first-contact product turns are also trusted
installation configuration, not external hints, and must be persisted into the
canonical conversation binding on first bind rather than overlaid on every
resolve. Thread hints in subscription requests may narrow to the already
resolved binding only; they are not authority to switch threads or tenants.
Projection/subscription resolution is lookup-only and must not create bindings,
threads, or external-event route reservations.

## Test support

Enable `test-support` feature for in-memory fakes:
- `FakeConversationBindingService`
- `FakeIdempotencyLedger`
- `FakeInboundTurnService`
