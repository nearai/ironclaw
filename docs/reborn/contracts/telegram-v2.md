# Telegram WASM v2 ProductAdapter

**Status:** First-slice tracer-bullet for #3285 (default off).
**Crate:** `ironclaw_telegram_v2_adapter`.
**Host runtime:** `ironclaw_wasm_product_adapters`.
**Contract:** `ironclaw_product_adapters` (see `product-adapters.md`).

## Goals

Prove the [`product-adapters.md`](product-adapters.md) contract end-to-end
against recorded Telegram payloads and fake Reborn services. The first
slice is intentionally narrow:

- The adapter is implemented natively in Rust today; the wasmtime
  component-model build of the same logic lives in a follow-up landing
  alongside the host runtime's full WIT bindings.
- All Reborn services below the workflow facade are fakes
  (`FakeProductWorkflow`, etc.).
- Production traffic is gated behind `REBORN_TELEGRAM_V2_ENABLED`
  (default off).

## Authentication

Telegram webhooks ship a shared secret in
`X-Telegram-Bot-Api-Secret-Token`. The host verifies the header in
constant time using
`ironclaw_wasm_product_adapters::SharedSecretHeaderAuth` and only then
constructs a `ProtocolAuthEvidence::Verified` via
`mark_shared_secret_header_verified`. Adapters refuse to parse a payload
whose evidence is not `Verified`.

## Reference normalization

| Telegram field | Reborn ref |
|----------------|-----------|
| `update_id` | `ExternalEventId` (formatted as `tg-<installation>-<update_id>`) |
| `message.from.id` | `ExternalActorRef.id` (kind = `telegram_user`) |
| `message.chat.id` | `ExternalConversationRef.conversation_id` |
| `message.message_thread_id` | `ExternalConversationRef.topic_id` |
| `message.message_id` | `ExternalConversationRef.reply_target_message_id` (NOT part of conversation key) |

Conversation fingerprint excludes `reply_target_message_id`. Two messages
in the same chat + topic but with different `message_id` produce identical
fingerprints — that is the canonical conversation key.

## Group/supergroup gating

In private chats every message creates an inbound envelope.

In groups/supergroups the adapter creates an envelope only when **one** of
the explicit triggers fires:

1. `mention` entity matching the configured `bot_username` (case-insensitive).
2. `reply_to_message.from.is_bot && from.id == bot_user_id`.
3. `bot_command` entity for a name in the configured
   `recognized_commands`. Bot commands of the form `/foo@botname` only
   match when the suffix matches the configured username.

Channel posts and edited messages are always `NoOp`.

## Attachments

`UserMessagePayload.attachments` carries `ProductAttachmentDescriptor`
values only:

- `external_file_id` (Telegram `file_id`)
- `mime_type`
- `filename` (when provided)
- `size_bytes` (when provided)
- `kind`: `Image` / `Audio` / `Video` / `Document` / `Voice` / `Sticker`

The adapter does **not** download files, **does not** include a
`source_url`, **does not** include any local filesystem path, and **does
not** include raw bytes. The workflow stages durable attachment refs
through the constrained egress capability before the turn coordinator
sees the message.

## Outbound rendering

Reply targets encode as `tg:<chat_id>:<topic_or_underscore>:<msg_or_underscore>`.

| Payload | Egress |
|---------|--------|
| `FinalReply` | `POST api.telegram.org/sendMessage` with `chat_id`, optional `message_thread_id`, optional `reply_to_message_id` |
| `Progress { Typing/Reflecting/ToolRunning }` | `POST api.telegram.org/sendChatAction { action: "typing" }` (only when `ExternalProgressPush` advertised) |
| `GatePrompt` / `AuthPrompt` | Deferred to #3094; first slice silently drops |
| `ProjectionSnapshot` / `ProjectionUpdate` | Telegram does not consume; silently dropped |

Egress targets only the declared `api.telegram.org` host. The bot token
travels as an opaque `EgressCredentialHandle` (`telegram_bot_token`); the
host resolves it at request time and never exposes the underlying secret
to the adapter.

## Idempotency

Dedupe key = `(adapter_installation_id, source_binding_ref,
external_event_id)`. The fake workflow returns
`ProductInboundAck::Duplicate { prior }` on second delivery of the same
`update_id`; the prior outcome is the one observed on first delivery.
Webhook responses for duplicates remain 200 OK with no side effects.

## Capabilities

`telegram_default_capabilities()` advertises:

- `InboundMessages`
- `InboundCommands`
- `InboundAttachments`
- `ExternalFinalReplyPush`
- `DeliveryStatusReporting`

`ExternalProgressPush` is opt-in via
`TelegramV2AdapterConfig::progress_push_enabled` (#3266 progress policy).
`ExternalGatePush` is intentionally absent until #3094 lands.

## Default-off behavior

`REBORN_TELEGRAM_V2_ENABLED=false` (default) keeps the legacy v1 Telegram
WASM channel (`channels-src/telegram`) running unchanged through the v1
channel manager.

`REBORN_TELEGRAM_V2_ENABLED=true` requires the legacy v1 Telegram channel
to be inactive for the same installation. The host calls
`ironclaw::config::validate_telegram_v1_v2_exclusivity` at startup and
fails closed when both are active.

## Test coverage (issue #3285 acceptance criteria)

Coverage today lives in the crate's per-module `mod tests` blocks
(`cargo test -p ironclaw_telegram_v2_adapter --lib`, 46 tests at the
time of writing). The tests are not yet named `ac<N>_*`; they are
organised by the source surface they exercise:

- `payload::tests` (~24 tests) — `parse_telegram_update` shape:
  private vs group routing, `/command` recognition (including media
  captions and mention+command), recognized-vs-unknown command
  classification, unauthenticated-payload fail-closed, malformed JSON,
  missing `from`, topic-keyed conversation refs, photo attachment
  descriptors, control-char and oversized-argument rejection through
  the shared validator.
- `render::tests` (~4 tests) — `parse_reply_target` round-trip,
  malformed-target typed error, `sendMessage` shape with topic and
  reply-to bindings, `sendChatAction` typing shape.
- `adapter::tests` (~15 tests) — capability default vs progress
  opt-in, declared egress host list + paired `(host, credential)`
  egress target, `parse_inbound` refusing unverified evidence,
  `render_outbound` install-scope guard (mismatched `adapter_id` /
  `installation_id` fail closed with no egress and no delivery
  record), and the full `DeliveryStatus` mapping for 2xx
  `Delivered` / 5xx + 429 `FailedRetryable` / 401 + 403
  `FailedUnauthorized` / other 4xx `FailedPermanent` / render-error
  `FailedPermanent` / non-final-reply `Deferred`.
- `payload::slice_tests` (~8 tests) — UTF-16 entity offset slicing
  used by `text_entity_windows`.

**Deferred:** the integration contract suite at
`crates/ironclaw_telegram_v2_adapter/tests/product_adapter_telegram_contract.rs`
(referenced in earlier revisions of this doc with `ac<N>_*`
acceptance-bullet test names) was removed pending a case-by-case
port to the post-#3352 product-adapter API
(`ProtocolAuthEvidence` enum→sealed-struct,
`ProductInboundEnvelope` private fields, 4-arg `render_outbound`
returning `ProductRenderOutcome`, `EgressRequest` builder API,
paired `(host, credential)` egress policy, and
`parse_inbound -> Result<ParsedProductInbound, _>`). The recorded
Telegram payload fixtures under
`crates/ironclaw_telegram_v2_adapter/tests/fixtures/*.json` are
retained for that followup. Once the port lands, each restored test
should carry an `ac<N>_*` name referencing the exact AC bullet from
issue #3285.
