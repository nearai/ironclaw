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
| `ProductConversationSubjectRouteResolver` | Host-owned dynamic shared-route subject resolver; product workflow consults it before static per-installation subject routes |
| `IdempotencyLedger` | Durable action deduplication port |
| `InMemoryIdempotencyLedger` | Local-dev/test ledger with in-flight lease recovery semantics |
| `ProductInboundAction` | Durable ledger record for inbound actions |
| `ProductCommandAdmissionService` | Source/auth-aware admission port that decides whether a typed product command may execute |
| `ProductCommandService` | Reborn-native product command execution port for already-admitted typed commands |
| `ApprovalInteractionService` / `DefaultApprovalInteractionService` | Approval-only product/WebUI boundary for listing redacted pending approval gates and resolving click approve/deny through canonical approval resolver + turn coordinator ports |
| `RunStateApprovalInteractionReadModel` | Canonical read model that returns status-bearing approval gates from scoped approval-request records plus the parked turn-run locator; `ApprovalInteractionService::list_pending` filters those records to pending UI DTOs |
| `AuthInteractionService` / `DefaultAuthInteractionService` | Auth-required product/WebUI boundary for listing redacted pending auth gates and resolving credential/callback/cancel decisions through typed auth-flow manager + turn coordinator ports |
| `RebornServicesApi` / `RebornServices` | Native WebChat v2 facade — stable surface beta WebUI route handlers consume in place of reaching into turn coordination, thread stores, runtime lanes, dispatchers, or capability hosts. Enforces caller ownership of the thread before any turn mutation; exposes connectable channel metadata for deterministic UI actions; rejects stale or attacker-supplied `gate_ref` on denied/cancelled gate resolutions; routes approval-gate `always: true` resolutions through the approval interaction policy path while keeping generic gate fallback one-shot only |

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
directly execute tools or mutate approval stores ad hoc. `AlwaysAllow` is
limited to approval gates backed by the durable persistent approval-policy port;
generic gate fallback remains one-shot only. Persistent approval policy checks
must be performed before approval/resume side effects and must fail closed when
the capability manifest does not allow durable reuse. High-value signing and
attested approvals require a separate service shape with canonical payload
attestation and must not be folded into this redacted click-approval DTO.

Auth interactions are auth-required gates only. Pending auth DTOs must be
redacted, scoped, and derived from typed auth-flow state plus the parked
turn-run locator. Credential/callback completion refs are opaque host-issued
references; raw tokens, OAuth codes, verifier material, provider errors, host
paths, or backend diagnostics must not enter product payloads or projection
DTOs. Resume/cancel decisions must go through `AuthFlowManager` and
`TurnCoordinator`; product/WebUI code must not handle raw credentials, mutate
auth-flow records directly, or resume blocked auth gates without the
`BlockedAuthGate` precondition.

WebUI gate resolution routing should use current run-state first: a
`BlockedApproval` run enters `ApprovalInteractionService`, a `BlockedAuth` run
enters `AuthInteractionService`, and generic fallback is only for non-typed
blocked gates or legacy/replay shapes. Do not let generic WebUI gate handling
resume/cancel auth-blocked runs.
Typed auth/approval interaction services intentionally re-read run-state through
`blocked_gate_state` immediately before resume/cancel side effects. Treat that
second read as a freshness/TOCTOU guard unless a future coordinator returns a
sealed gate grant that can safely replace it.

WebUI-facing facade methods must bind browser thread ids through
`SessionThreadService` using a `ThreadScope` derived from the authenticated
caller before accepting messages, streaming events, canceling runs, or resolving
gates. Browser/session metadata is not authority by itself, and send-message
must not implicitly create missing threads.

### Trigger-thread exception

Automation trigger-fired threads are stored by `record_trigger_prompt` with
`owner_user_id = Some(creator_user_id)` — the user who fired the trigger — not
the WebUI caller's session user. The caller-scoped `SessionThreadService` probe
therefore misses them.

When a thread lookup misses under the caller's session scope, `RebornServices`
falls back to `AutomationProductFacade::resolve_run_thread_scope`. That method
is caller-scoped: it scans only the triggers owned by the authenticated caller
(matched on tenant_id + creator_user_id + agent_id + project_id), so the
authorization check is embedded in the lookup. If a matching trigger run is
found, the service reconstructs a `TurnScope` with:

- `owner_user_id = Some(trigger creator_user_id)` — NOT the session caller
- `agent_id` / `project_id` from the trigger record

and substitutes the trigger creator as the `run_actor` for all downstream turn
operations (timeline, SSE stream, gate resolve, cancel, run-state).

**Visibility for listing and thread authorization are deliberately decoupled.**
The authorization model is caller ownership (tenant + user + agent + project),
not `list_automations` listing eligibility. The default `list_automations`
response excludes `Completed` triggers (soft-completed fire-once triggers) to
keep the active panel uncluttered. But completed triggers' run threads remain
authorized through this resolver — their history is retained user data and must
stay accessible. `resolve_run_thread_scope` does not filter on trigger state.

**No caching.** Every call revalidates automation visibility through the facade.
A caller that loses automation visibility mid-session cannot keep accessing the
trigger-owned thread after their access is revoked. Caching the authz result
is explicitly forbidden.

**Backend/timeout errors** from `resolve_run_thread_scope` surface as
`Unavailable` (503, retryable), never as 404. Only `Ok(None)` (thread not in
any caller-visible trigger) produces a 404 response.

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
Shared-route subject users are also first-bind scope, not a live overlay on
existing external conversation bindings. Route admin updates apply to new
bindings; existing Slack threads must continue resolving under the owner that
created their thread scope.

Outbound delivery orchestration starts only after `ironclaw_outbound` resolves
and validates a communication delivery candidate. `OutboundPolicyService`
remains the authority for reply-target validation and delivery-attempt metadata.
Product workflow may attach trusted product target metadata from conversation
binding and call `ProductAdapter::render_outbound`, but it must not choose a
different reply target, read outbound preferences itself, or render anything
before policy approval. Target metadata resolvers must be lookup-only and keyed
by the sealed validated reply-target binding.

## Test support

Enable `test-support` feature for in-memory fakes:
- `FakeConversationBindingService`
- `FakeIdempotencyLedger`
- `FakeInboundTurnService`
