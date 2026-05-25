# Reborn ProductAdapter contract

**Status:** Draft (first slice landing for #3285).
**Owner crate:** `ironclaw_product_adapters`.
**Host runtime:** `ironclaw_wasm_product_adapters`.
**First concrete adapter:** `ironclaw_telegram_v2_adapter`.
**Related issues:** #3269 (this contract), #3285 (Telegram tracer bullet),
#3266 (outbound policy), #3193 (conversation binding), #3094 (gate UX).

## Purpose

Define the boundary between channel/transport-specific code and the canonical
Reborn pipeline. Adapters parse external protocol payloads into structured
inbound envelopes and protocol-translate projection-derived outbound
envelopes back to the external surface. Adapters do NOT own canonical
thread/run/transcript state, do NOT call `TurnCoordinator` directly, and do
NOT expose raw protocol secrets to themselves or to WASM components.

## Layering

```text
protocol event (webhook / cookie / bearer / cli)
  -> host verifies protocol auth (mints ProtocolAuthEvidence::Verified)
  -> ProductAdapter::parse_inbound(raw_payload, evidence)
       -> ProductInboundEnvelope (or None for ambient/no-op events)
  -> ProductWorkflow::accept_inbound(envelope)
       -> ConversationBindingService -> SessionThreadService -> TurnCoordinator
  -> ProductInboundAck (Accepted / DeferredBusy / Rejected / Duplicate / NoOp)
  -> protocol layer maps ack to status code

projection update
  -> ProductOutboundEnvelope (FinalReply / Progress / GatePrompt / ...)
  -> ProductAdapter::render_outbound(envelope, &dyn ProtocolHttpEgress)
       -> ProtocolHttpEgress (declared host + credential handle)
  -> OutboundDeliverySink::record(DeliveryStatus)
```

## Frozen invariants

- ProductAdapter DTOs carry only structured external refs:
  `ExternalActorRef`, `ExternalConversationRef`, `ExternalEventId`,
  `ProductAttachmentDescriptor`. No raw bytes, source URLs, host paths, raw
  prompts, raw tool input, or backend diagnostics.
- `ProtocolAuthEvidence::Verified` is sealed. Only the host-glue helpers in
  `ironclaw_product_adapters::auth` (which take a crate-private
  `HostAuthSeal`) can construct one. WASM components and downstream adapters
  cannot fabricate verification.
- `ProtocolHttpEgress` is the only network capability. Adapters declare
  egress hosts up front (`DeclaredEgressHost`) and address credentials via
  opaque handles (`EgressCredentialHandle`). The host resolves credential
  material at request time and scans response bodies for leaks before
  returning them.
- Adapters MUST NOT depend on `ironclaw_dispatcher`, `ironclaw_capabilities`,
  `ironclaw_host_runtime`, `ironclaw_network`, `ironclaw_secrets`,
  `ironclaw_filesystem`, raw process spawning, or
  `ironclaw_turns::runner`. Boundary tests in
  `crates/ironclaw_product_adapters/tests/product_adapter_contract.rs`
  enforce this.
- Delivery failures are best-effort. They record a separate
  `DeliveryStatus` and never mutate canonical transcript/projection/turn
  state.

## Inbound

`ProductInboundEnvelope` fields:

- `adapter_id: ProductAdapterId`
- `installation_id: AdapterInstallationId`
- `external_event_id: ExternalEventId` — stable per-installation event id
  used for dedupe.
- `external_actor_ref: ExternalActorRef` — protocol's stable user id +
  optional display name.
- `external_conversation_ref: ExternalConversationRef` — protocol's
  conversation key (chat id + optional topic id) plus an optional
  reply-target message id (NOT part of the canonical key).
- `auth_evidence: ProtocolAuthEvidence`
- `received_at: DateTime<Utc>`
- `payload: ProductInboundPayload` — UserMessage / Command /
  ApprovalResolution / AuthResolution / SubscriptionRequest / NoOp.

`ProductInboundAck` outcomes:

- `Accepted { accepted_message_ref, submitted_run_id }`
- `DeferredBusy { accepted_message_ref, active_run_id }`
- `Rejected(ProductRejection { kind, reason })`
- `Duplicate { prior: Box<ProductInboundAck> }`
- `NoOp`

Webhook ack semantics:

| Outcome | Protocol response |
|--------|-------------------|
| `Accepted` / `DeferredBusy` / `Duplicate` / `NoOp` | 200 OK |
| `Rejected { BindingRequired \| AccessDenied \| UnknownInstallation }` | 403 |
| `Rejected { PolicyDenied }` | 403/422 (protocol-specific) |
| `Authentication` failure (host-side) | 401/403 |
| `WorkflowTransient` failure | 5xx/429 (retryable) |

## Outbound

`ProductOutboundEnvelope` fields:

- `adapter_id`, `installation_id`
- `target: ReplyTargetBindingRef`
- `projection_cursor: Option<ProjectionCursor>`
- `payload: ProductOutboundPayload` — FinalReply / Progress /
  CapabilityActivity / GatePrompt / AuthPrompt / ProjectionSnapshot /
  ProjectionUpdate
- `delivery_attempt_id: Uuid`

Capabilities (`ProductAdapterCapabilities`):

- `InboundMessages`, `InboundCommands`, `InboundAttachments`
- `ExternalFinalReplyPush`, `ExternalProgressPush` (opt-in per #3266),
  `ExternalGatePush` (deferred to #3094)
- `ProjectionSubscription`, `SynchronousWait`, `DeliveryStatusReporting`

`ProductAdapterCapabilities::external_channel_default()` is the safe
preset for chat channels: inbound messages/commands/attachments + final
reply push + delivery status reporting, without progress or gate push.

## Authentication evidence

Verifiers in `ironclaw_wasm_product_adapters::auth_verifier` provide
constant-time HMAC and shared-secret-header verification. The host calls a
verifier first and only constructs a `Verified` evidence (via one of the
public `mark_*_verified` helpers) when the digest matches.

Adapters never read the secret. Verifiers run with `subtle::ConstantTimeEq`
to avoid timing oracles.

## Egress

`ProtocolHttpEgress` requests carry:

- `host: DeclaredEgressHost`
- `method`, `path`, `headers`, `body`
- `credential_handle: Option<EgressCredentialHandle>`

Component runtime uses v1-style minimal WASI p2 for wasm32-wasip2 guest compatibility:
clock/random are available, but env, args, stdio, preopened directories, and
network are not inherited. The first WASM runtime slice is parse/render-only:
`render-outbound` returns a host-validated typed `EgressRequest`, while the
component `http-egress` import fails closed until follow-up host-runtime wiring
injects the production `ProtocolHttpEgress` path. Native adapters already use
`ProtocolHttpEgress` directly.

The production host egress path:

1. Validates the host against the adapter manifest's declared list.
2. Validates the credential handle against the per-installation
   allowlist (`EgressPolicy`).
3. Resolves the handle to an actual secret at request time (out-of-band).
4. Sends the request.
5. Scans the response for leaks before returning.
6. Reports `DeliveryStatus` to the registered `OutboundDeliverySink`.

## Default-off + cutover

V2 product adapters run in a **separate binary** from the v1 agent —
the `ironclaw-reborn` bin target in `crates/ironclaw_reborn_cli/`,
which pulls in each channel's host crate (e.g.
`crates/ironclaw_reborn_telegram_v2_host/`) behind a per-channel Cargo
feature (`telegram-v2`, future `slack-v2`, ...). The v1 agent binary
has no compile-time dependency on any Reborn product-layer crate. There
is no in-process feature flag, no v1/v2 exclusivity guard, and no
config-level cutover, because the two paths simply cannot coexist in
the same process. An operator running both v1 Telegram and v2 Telegram
must point them at distinct bot tokens / webhook URLs (operator-level
concern, not a code concern).

Future v2 product adapters (Slack, Discord, etc.) are expected to follow
the same pattern: their host crate lives next to
`ironclaw_reborn_telegram_v2_host`, links the same storage / adapter /
runner crates, exposes a library `serve_from_env` entry point, and
slots into `ironclaw-reborn run` behind its own Cargo feature on
`ironclaw_reborn_cli`.

## Status

| Item | Status |
|------|--------|
| Contract types | `[implemented slice]` (`ironclaw_product_adapters`) |
| Durable storage (libSQL + Postgres) | `[implemented slice]` (`ironclaw_product_workflow_storage`) — ledger with TOCTOU-safe insert + recovery-lease reclaim, conversation binding upsert, outbound delivery sink, Telegram HTTP egress shim |
| In-memory fakes (test-only) | `[implemented slice]` (`FakeProductWorkflow`, `FakeProtocolHttpEgress`, `FakeOutboundDeliverySink`, `FakeProjectionStream`) |
| Boundary / redaction tests | `[implemented slice]` |
| Webhook auth verifiers (HMAC, shared-secret-header) | `[implemented slice]` |
| Egress policy enforcement | `[implemented slice]` |
| `NativeProductAdapterRunner` | `[implemented slice]` |
| Telegram v2 native adapter | `[implemented slice]` (`ironclaw_telegram_v2_adapter`) |
| Telegram v2 production host | `[implemented slice, stub reply]` (`ironclaw_reborn_telegram_v2_host`, wired into the `ironclaw-reborn` binary behind the `telegram-v2` feature) — full inbound contract end-to-end; reply path stubbed pending outbound migration to `DefaultInboundTurnService` + `TurnCoordinator` (Reborn agent loop PRs #3544 / #3550 / #3586 have merged; the reply migration is a focused follow-up) |
| wasmtime component-model glue | `[implemented slice]` (`ProductAdapterComponentRuntime` loads `crates/ironclaw_wasm_product_adapters/wit/product_adapter.wit`; parse/render-only, component `http-egress` import fails closed until production egress wiring lands) |
| Web / Slack / Discord / WhatsApp / Feishu / Signal v2 adapters | `[not implemented]` |
| Production wiring of v2 webhook route | `[not implemented]` (default-off flag exists; route registration is a follow-up) |
