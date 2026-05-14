# How to port a v1 channel to Reborn

This guide helps maintainers decide how an IronClaw v1 channel should move onto the Reborn product-surface path.

The important distinction is that v1 used one `Channel` abstraction for several different things. Reborn should keep those categories separate.

## Quick decision tree

Ask first: **is this channel an IronClaw-owned user surface, or an external protocol integration?**

| v1 source | Examples | Reborn target |
| --- | --- | --- |
| Host-owned native surface | TUI, Web Gateway, local REPL, built-in local control endpoints | Native Reborn surface that enters the product workflow directly |
| External protocol integration | Telegram, Slack, Discord, Signal-like integrations, external webhooks | WASM ProductAdapter component as the production artifact |
| Legacy WASM channel | `channels-src/telegram` style plugins | Port to WASM ProductAdapter; reuse protocol lessons, not v1 DTOs |
| Native external integration | `SignalChannel` style native protocol adapter | Extract pure core logic, then wrap as WASM ProductAdapter |

Rule of thumb:

- If the surface is IronClaw's own UI/API, keep it native.
- If the surface speaks a third-party protocol or should be installable at runtime, make the production path a WASM ProductAdapter.
- Native code is still useful for pure parse/render logic and tests, but not as the production boundary for installable external protocol adapters.

## Why Reborn keeps two boundary types

Reborn should converge on **one product workflow entrypoint**, not one adapter type for every surface.

```text
Native host surface                External protocol integration
(TUI/Web/REPL)                     (Telegram/Slack/Signal/etc.)
      │                                      │
      ▼                                      ▼
Native Reborn surface              WASM ProductAdapter component
      │                                      │
      └──────────── ProductWorkflow ─────────┘
                         │
                         ▼
                  Reborn turn/runtime path
```

`ProductAdapter` is the external-protocol contract. It carries host-visible protocol auth, declared egress, external actor/conversation refs, and delivery-status semantics. That fits Telegram/Slack/etc.

Host-owned surfaces already live inside the trusted IronClaw host. Web still needs auth, but it is **host auth** such as bearer tokens, sessions, OIDC, CSRF/origin checks, and tenancy. It should not be forced through fake protocol concepts like `ExternalActorRef`, protocol auth evidence, or delivery sinks unless it is actually crossing an external protocol boundary.

## Path A: port a native host-owned surface

Use this path for TUI, Web Gateway, REPL, and built-in local/control surfaces.

### Target shape

```text
host request / UI event
  → host auth/session/user resolution
  → native Reborn surface handler
  → ProductWorkflow or equivalent inbound facade
  → projection/outbound model
  → native UI/API response stream
```

### What to preserve

- Host auth remains host-owned.
- Session/user/thread binding stays in native Reborn host code.
- UI-specific streaming stays native: SSE, WebSocket, terminal redraws, local progress events.
- Route/body limits, CORS/origin, bearer tokens, and gateway security stay in gateway-owned code.

### What not to do

Do not invent external protocol wrappers just to reuse `ProductAdapter`:

- no fake `ExternalActorRef` for a browser session
- no fake `ProtocolAuthEvidence` for a host bearer token
- no fake delivery sink for SSE/WebSocket events
- no fake declared egress when the host is not calling a third-party API

### Native host-surface checklist

- [ ] Identify the v1 channel entrypoint and response path.
- [ ] Identify host auth/session/user resolution rules.
- [ ] Identify how the surface maps to a Reborn product workflow submission.
- [ ] Identify how replies/progress/projections flow back to the UI/API.
- [ ] Preserve existing listener/auth/body-limit/CORS/rate-limit invariants.
- [ ] Add caller-level tests at the handler/UI/service boundary, not just helper tests.
- [ ] Update feature parity docs if the user-visible surface changes status.

## Path B: port an external protocol integration

Use this path for Telegram, Slack, Discord, Signal-like protocol adapters, external webhook channels, and anything expected to be installable or sandboxed.

### Target shape

```text
external protocol payload
  → Reborn host verifies protocol auth
  → WASM ProductAdapter parses payload
  → ParsedProductInbound
  → host stamps TrustedInboundContext
  → ProductWorkflow
  → ProductOutboundEnvelope
  → WASM ProductAdapter renders outbound action
  → host-mediated ProtocolHttpEgress + OutboundDeliverySink
```

### Native core vs WASM component

Split responsibilities even if the exact paths move later into a grouped `product/` tree.

Native core logic may own:

- protocol serde types
- payload normalization
- trigger classification
- attachment descriptor extraction
- reply-target parsing
- outbound body rendering
- fast unit tests

Native core logic must not own:

- direct DB access
- direct filesystem access
- direct network clients
- raw secrets or bot tokens
- canonical user/thread binding
- legacy v1 channel dependencies

WASM component wrapper owns:

- ProductAdapter ABI export
- conversion between WIT/component types and native core types
- production adapter artifact build
- component smoke/contract tests

Host runtime owns:

- protocol auth verification
- `TrustedInboundContext` stamping
- constrained HTTP egress
- credential injection by handle
- delivery-status recording
- logging/tracing/clock capabilities
- resource limits and sandbox execution

### ProductAdapter implementation checklist

A concrete adapter must define:

- [ ] `adapter_id()`
- [ ] `installation_id()`
- [ ] `surface_kind()`
- [ ] `capabilities()`
- [ ] `auth_requirement()`
- [ ] `declared_egress()`
- [ ] `parse_inbound(raw_payload, auth_evidence)`
- [ ] `render_outbound(envelope, egress, delivery_sink)`
- [ ] `health()` if the default `Healthy` is not enough

Minimal Rust trait shape today:

```rust
#[async_trait::async_trait]
impl ProductAdapter for MyAdapter {
    fn adapter_id(&self) -> &ProductAdapterId;
    fn installation_id(&self) -> &AdapterInstallationId;
    fn surface_kind(&self) -> ProductSurfaceKind;
    fn capabilities(&self) -> &ProductAdapterCapabilities;
    fn auth_requirement(&self) -> &AuthRequirement;
    fn declared_egress(&self) -> &[DeclaredEgressTarget];

    fn parse_inbound(
        &self,
        raw_payload: &[u8],
        auth_evidence: &ProtocolAuthEvidence,
    ) -> Result<ParsedProductInbound, ProductAdapterError>;

    async fn render_outbound(
        &self,
        envelope: ProductOutboundEnvelope,
        egress: &dyn ProtocolHttpEgress,
        delivery_sink: &dyn OutboundDeliverySink,
    ) -> Result<ProductRenderOutcome, ProductAdapterError>;
}
```

A ProductAdapter WIT contract already exists at `crates/ironclaw_wasm_product_adapters/wit/product_adapter.wit`. It currently uses JSON-shim fields for Rust DTOs while the DTOs evolve. Keep native core logic clean enough to wrap behind that component boundary without pulling host internals across it.

## Path C: port a legacy WASM channel

Use this path for existing v1 WASM channels such as `channels-src/telegram`.

Do reuse:

- protocol payload fixtures
- trigger rules
- attachment metadata decisions
- auth requirements
- outbound API request shapes
- operational lessons from activation/setup

Do not reuse directly:

- `IncomingMessage`
- `OutgoingResponse`
- `StatusUpdate`
- v1 `Channel` / `ChannelManager` lifecycle
- v1 WASM host imports as the final ABI
- direct legacy channel storage assumptions

### Mapping table

| v1 channel concept | Reborn ProductAdapter concept |
| --- | --- |
| `IncomingMessage` text | `ProductInboundPayload::UserMessage` |
| ambient ignored event | `ProductInboundPayload::NoOp` |
| slash/bot command | `ProductInboundPayload::Command` / `InboundCommandPayload` |
| sender id | `ExternalActorRef` |
| chat/thread id | `ExternalConversationRef` |
| platform message/update id | `ExternalEventId` |
| attachment bytes/URLs | `ProductAttachmentDescriptor` only |
| webhook secret check | host-enforced `AuthRequirement` + `ProtocolAuthEvidence` |
| direct HTTP client | `ProtocolHttpEgress` |
| bot token | `EgressCredentialHandle` |
| send result/status update | `OutboundDeliverySink` + `DeliveryStatus` |
| v1 channel activation | Reborn adapter installation/runtime config |

## Inbound porting recipe

For each incoming protocol payload:

1. Deserialize only fields the adapter needs.
2. Validate protocol shape and reject malformed payloads with typed errors.
3. Convert platform user/sender into `ExternalActorRef`.
4. Convert chat/thread/topic into `ExternalConversationRef`.
5. Convert platform event/message/update id into `ExternalEventId` for dedupe.
6. Classify trigger reason: direct message, mention, reply, command, ambient/no-op, etc.
7. Convert user content into typed inbound payloads.
8. Convert files/media into bounded `ProductAttachmentDescriptor` values only.
9. Return `ProductInboundPayload::NoOp` for authenticated events that should be ignored.
10. Let the host stamp trusted fields; adapter code must not fabricate verified auth or canonical user/thread ids.

## Outbound porting recipe

For each outbound envelope:

1. Check the envelope belongs to this adapter installation.
2. Match `ProductOutboundPayload` by variant.
3. Render `FinalReplyView` into the protocol's send-message API.
4. Render progress only if the protocol supports it and capability is enabled.
5. Use `ProtocolHttpEgress`, never a direct client.
6. Use credential handles, never raw tokens.
7. Map API/egress failures to `DeliveryStatus`:
   - success -> `Delivered`
   - transient network/timeout/429/5xx -> `FailedRetryable`
   - bad auth/unknown credential -> `FailedUnauthorized`
   - malformed target/4xx permanent errors -> `FailedPermanent`
8. Record every push-channel attempt through `OutboundDeliverySink`.
9. Return `Deferred` for payloads the adapter intentionally does not render.

## Security checklist

- [ ] Adapter never receives or logs raw secrets.
- [ ] Adapter never constructs verification evidence; host does.
- [ ] Adapter declares every egress host and credential handle.
- [ ] Adapter uses host-mediated `ProtocolHttpEgress` only.
- [ ] Egress request paths/headers/bodies do not leak credentials.
- [ ] Inbound DTOs contain external refs and bounded descriptors, not raw bytes or local paths.
- [ ] Errors are redacted and do not include provider internals, raw tokens, host paths, or raw payloads.
- [ ] Wrong adapter/installation targets fail before egress.
- [ ] Delivery status reporting is best-effort and does not mutate transcript/turn state.
- [ ] Webhook/API auth remains fail-closed.

## Test checklist

For native core logic:

- [ ] parse valid fixtures
- [ ] reject malformed payloads
- [ ] classify triggers/no-ops
- [ ] normalize actor/conversation/event refs
- [ ] bound/redact attachments
- [ ] render outbound request bodies
- [ ] parse reply targets and reject extra/invalid segments

For ProductAdapter behavior:

- [ ] `auth_requirement()` is declared
- [ ] `declared_egress()` pairs host with credential handle
- [ ] `parse_inbound()` returns `ParsedProductInbound`, not host-stamped envelopes
- [ ] ignored authenticated events return `NoOp`
- [ ] render success records `Delivered`
- [ ] retryable/permanent/unauthorized failures record distinct statuses
- [ ] wrong adapter/installation ids do not egress
- [ ] redaction sentinels cover DTOs, errors, and egress shapes

For WASM component path, once available:

- [ ] component builds as production artifact
- [ ] host can load component and read metadata
- [ ] component contract tests cover parse/render through the WASM boundary
- [ ] resource/fuel/time/memory limits apply
- [ ] host capabilities are fail-closed by default

For native host surfaces:

- [ ] caller-level handler/UI tests prove ProductWorkflow submission
- [ ] auth/session/tenant checks remain enforced
- [ ] projection/reply streaming reaches the user-visible surface
- [ ] route/body/rate/origin security invariants are preserved

## Current dependencies and gaps

This guide can be used now for planning and native-core work, but the full production WASM ProductAdapter path still depends on follow-up work:

- ProductAdapter WIT exists at `crates/ironclaw_wasm_product_adapters/wit/product_adapter.wit`, but wasmtime component bindings/loader are not wired yet.
- Reborn WASM ProductAdapter host runtime that loads components and exposes egress/secrets/delivery/logging capabilities beyond the current native runner.
- Component-level ProductAdapter contract test harness.
- Production Reborn route/deployment documentation for selecting v1 or Reborn for a given external webhook.
- Final Telegram v2 component wrapper. Current Telegram v2 work is a native tracer bullet, not the final production component boundary.
- Future grouped `product/` source tree migration. Do not block this guide on the tree move.

## References

- `crates/ironclaw_product_adapters/CLAUDE.md`
- `crates/ironclaw_product_adapters/src/adapter.rs`
- `crates/ironclaw_product_adapters/src/inbound.rs`
- `crates/ironclaw_product_adapters/src/outbound.rs`
- `crates/ironclaw_product_adapters/src/egress.rs`
- `crates/ironclaw_wasm_product_adapters/wit/product_adapter.wit`
- `docs/reborn/contracts/wasm.md`
- `src/channels/mod.rs`
- `channels-src/telegram/` as a v1 WASM reference only
- Tracking architecture issue: <https://github.com/nearai/ironclaw/issues/3572>
