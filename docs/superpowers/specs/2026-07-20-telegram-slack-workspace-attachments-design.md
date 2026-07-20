# Telegram and Slack Workspace Attachments — Design Spec

- **Date:** 2026-07-20
- **Status:** Approved in conversation; implementation contract
- **Target:** Reborn WebUI, Telegram, and Slack channel paths

## Summary

Telegram and Slack attachments must use the attachment behavior IronClaw already
ships for WebUI. A file sent to IronClaw is not channel-local context: the host
downloads and validates it, the shared attachment lander writes it into the
project's `/workspace/attachments/...` mount, the durable user message stores an
`AttachmentRef`, and the normal model/context path consumes it. A workspace file
referenced by an assistant reply is likewise surfaced as a native Telegram or
Slack attachment alongside the text reply.

The provider integrations own only provider API translation and authenticated
byte transfer. They do not choose filesystem paths, write host files directly,
create alternate transcript shapes, or bypass the product workflow. Raw provider
URLs, bot tokens, upload tickets, and file bytes remain transient host data and
are never persisted in product-adapter DTOs, events, projections, logs, or
delivery records.

## Product contract

### Inbound user journey

1. A user attaches one or more supported files to a Telegram or Slack message.
   A caption/text body is optional.
2. The channel adapter authenticates and parses the event as it does today. It
   emits bounded `ProductAttachmentDescriptor` metadata containing provider file
   ids, names, MIME types, kinds, and declared sizes — never source URLs or bytes.
3. After duplicate-event replay and inbound policy checks, the host-side channel
   attachment materializer fetches each descriptor using the installation's
   mediated credential and network policy.
4. The materializer validates the descriptor and fetched bytes against the same
   shared product limits WebUI advertises: at most 10 files, 5 MiB per file, and
   10 MiB total. The shared supported-format registry remains authoritative for
   accepted MIME types.
5. The existing `InboundAttachmentLander` lands the bytes through the
   project-scoped filesystem authority under `/workspace/attachments/...` and
   returns durable `AttachmentRef` values.
6. The existing turn acceptance path stores those refs on the user message. The
   normal attachment-context path supplies images to vision-capable models and
   extracted/fallback context for supported documents, exactly as for WebUI.
7. The assistant reply is delivered in the originating channel. The attachment
   remains visible in WebUI history and readable by workspace tools and later
   turns.

An attachment-only message is a real user message. Multiple attachments preserve
provider order. A retry of the same Telegram `update_id` or Slack event id returns
the accepted outcome without downloading or landing a second copy.

### Outbound user journey

1. During a Telegram- or Slack-originated turn, the agent creates or selects a
   file in the scoped project workspace.
2. Its final text references the virtual workspace path using the existing WebUI
   convention, for example: `Here is the report: /workspace/report.pdf`.
3. The shared channel delivery layer extracts workspace file references with the
   same rules as the WebUI file chips: valid supported paths outside inline and
   fenced code spans, deduplicated in first-seen order.
4. A project-scoped reader resolves each path through filesystem authority,
   checks the shared count/size/total budgets, and returns transient bytes plus
   filename and MIME type. A host path is never accepted.
5. The native adapter delivers the text and files as one logical delivery:
   Telegram uses Bot API multipart upload (`sendPhoto` for supported images,
   otherwise `sendDocument`); Slack uses `files.getUploadURLExternal`, uploads
   bytes to the returned `files.slack.com` URL, then calls
   `files.completeUploadExternal` with the destination channel/thread and the
   reply text.
6. The delivery attempt is marked delivered only when every required provider
   operation succeeds. A partial provider send is terminal and reported as a
   partial permanent failure so automatic retry cannot duplicate already-sent
   files.

This is automatic parity with WebUI's existing workspace-path UX. It does not
introduce a new tool call, special attachment syntax, or provider-specific model
prompt.

## Selected architecture

### Shared limits and path recognition

Move the WebUI-only inline limits into an attachment-owned value contract and
have WebUI, channel materializers, outbound readers, and tests consume it. Keep
the current values and supported-format registry unchanged:

| Limit | Value |
| --- | ---: |
| Files per message/reply | 10 |
| Bytes per file | 5 MiB |
| Combined bytes | 10 MiB |

Add a channel-neutral Rust workspace-reference extractor to
`ironclaw_attachments`. Its fixtures pin parity with
`frontend/src/pages/chat/lib/project-file-paths.ts`: paths must start with
`/workspace/`, must have a supported filename/extension, are ignored inside code
spans, and are deduplicated without reordering. The browser helper stays a UI
presentation helper; the Rust helper is authoritative for host delivery.

### Inbound materialization port

Add an optional `InboundAttachmentMaterializer` port to
`ironclaw_product_workflow` and wire it into `DefaultInboundTurnService` next to
the existing `InboundAttachmentLander`.

```text
authenticated provider event
  -> ProductAttachmentDescriptor(s)
  -> replay lookup
  -> before-inbound policy
  -> InboundAttachmentMaterializer        provider API only
  -> InboundAttachmentLander              shared project filesystem
  -> AttachmentRef(s) on durable message
  -> existing thread/model attachment context
```

The workflow invokes the materializer only when the user-message descriptor list
is non-empty and no inline bytes were supplied. Inline bytes plus descriptors are
ambiguous and fail closed. A configured descriptor list without a materializer
also fails closed; the workflow never silently submits an attachment-less turn.
Materialization occurs after replay and policy so duplicate or rejected events do
not cause provider downloads.

The port accepts the authenticated envelope and bounded descriptors and returns
existing `InboundAttachment` values. Its error is typed as retryable or permanent
and carries only a sanitized user-safe reason. The product workflow maps it to a
submission failure without exposing provider URLs, credentials, or response
bodies.

Provider implementations:

- **Telegram:** for every descriptor, call `getFile(file_id)`, validate the
  returned `file_path`, then download
  `/file/bot{telegram_bot_token}/<file_path>` through mediated Telegram egress.
  The default Bot API's 20 MiB download ceiling is above IronClaw's 5 MiB file
  limit, so IronClaw rejects declared oversize before `getFile` and bounds the
  streamed response at the shared limit.
- **Slack:** call `files.info(file=<id>)` with `files:read`, validate that the
  returned id/name/MIME/size agree with the descriptor, accept only an HTTPS
  `files.slack.com` `url_private_download`, and download it with host-injected
  bearer authorization. The incoming event's `url_private` remains discarded.

### Transient outbound attachment rendering

Add a non-serializable `ProductOutboundAttachment` host value containing only
the validated virtual path, filename, MIME type, and bytes. Extend
`ProductAdapter` with a defaulted `render_outbound_with_attachments` method. The
default delegates to today's `render_outbound` when the list is empty and rejects
non-empty lists, preserving all existing adapters and preventing silent drops.

`ProductOutboundEnvelope`, `FinalReplyView`, WIT/component payloads, projections,
and persisted delivery records do not gain raw bytes. The channel delivery layer
alone reads workspace files after it has the authoritative thread scope, then
calls the transient method. Telegram and Slack native adapters override it and
own their provider-specific multi-request sequence plus one aggregate delivery
status. Existing text-only replies continue through the current method with no
behavior change.

```text
final assistant text + authoritative ThreadScope
  -> shared /workspace path extractor
  -> project-scoped bounded file reader
  -> Vec<ProductOutboundAttachment>       transient, host memory only
  -> native adapter aggregate renderer
  -> mediated provider egress
  -> one honest delivery outcome
```

The same extraction and read step is used for live inbound replies and triggered
delivery. It is channel-neutral and does not key on Telegram or Slack.

### Why this approach

Three approaches were evaluated:

1. **Selected: workflow materializer + transient adapter renderer.** This reuses
   the existing landing/model path, keeps provider work at explicit ports, avoids
   serialized bytes, and lets each adapter report one honest multipart outcome.
2. **Channel-specific pre-router landing and post-render uploads.** This is less
   code initially but duplicates storage/policy behavior and can mark the text
   delivered before a file upload fails. It is rejected.
3. **Add file bytes to `FinalReplyView` or `ProductOutboundEnvelope`.** This makes
   adapters simple but places workspace bytes into projection/WASM DTOs and risks
   persistence or disclosure through unrelated surfaces. It is rejected.

## Provider/API details

### Telegram

- Inbound metadata: existing photo/document/audio/video/voice/sticker descriptors.
- Inbound fetch: `getFile`, then the authenticated `/file/bot<token>/<file_path>`
  download route.
- Outbound image: `sendPhoto` multipart when MIME and provider constraints allow.
- Other outbound file: `sendDocument` multipart.
- Preserve current chat/topic/reply targeting. Multipart names, filenames, and
  captions are escaped and bounded; no raw token enters a constructed/logged URL.
- Provider error envelopes use the adapter's current retryable, unauthorized,
  permanent, and partial-delivery classifications.

### Slack

- Inbound lookup: `files.info` (`files:read`), followed by authenticated download
  from the validated private file URL.
- Outbound upload: `files.getUploadURLExternal` (`files:write`) for every file;
  POST bounded bytes to each validated upload URL; finalize all returned file ids
  once with `files.completeUploadExternal`, including `channel_id`, `thread_ts`
  when present, and the final reply as `initial_comment`.
- Do not use retired `files.upload`.
- Setup/install manifests and UI must request and explain both `files:read` and
  `files:write`. Existing installations missing `files:write` fail honestly with
  a reconnect/reinstall action rather than silently sending text only.

## Failure behavior

- Unsupported type, invalid filename, excessive count, declared oversize, or
  total oversize: reject before provider byte transfer when metadata permits.
- Missing size metadata: perform a response-bounded fetch and reject at the byte
  limit. Never buffer an unbounded body.
- Provider lookup/download failure: no landing and no turn submission. A retry of
  the same external event may retry while it is not yet accepted.
- Landing failure after download: no accepted message. Existing landing cleanup
  and idempotency rules apply; no channel-specific file remains.
- Missing or unauthorized outbound workspace path: do not send a misleading
  text-only success. Record a permanent delivery failure with a sanitized reason.
- Outbound provider failure before any part is visible: preserve the existing
  retryable/permanent classification. Failure after any visible part becomes
  permanent partial delivery to prevent duplicate sends.
- Empty assistant path list: unchanged text-only behavior.

## Security and isolation

- Provider file ids and URLs are untrusted. Resolve ids with the authenticated
  installation; validate returned scheme, host, path, size, MIME, and identity.
- Every network call goes through mediated host egress with an explicit host,
  method, request-size, response-size, timeout, and credential policy.
- Telegram token substitution remains a path-placeholder operation. Slack bearer
  tokens remain host-injected headers. Neither is adapter-visible text.
- Workspace paths are resolved through project-scoped filesystem authority. No
  host absolute path, `..`, symlink escape, cross-project scope, or user-provided
  `file://`/HTTP path is accepted.
- Attachment bytes and provider URLs are not logged. Errors carry bounded labels
  such as file index/id fingerprint and category, never response bodies.
- Duplicate external events must not repeat downloads, writes, turns, or sends.
- Attachment refs stay scoped to the original tenant/user/agent/project thread;
  WebUI download and later tool reads keep their existing authorization checks.

## Test-first acceptance suite

### Shared product-workflow and attachment contracts

1. Descriptor-backed inbound calls the materializer after policy, calls the
   existing lander, and persists returned refs on the accepted user message.
2. Duplicate accepted event replays without a second materialize or land call.
3. Rejected policy does not fetch; missing materializer and mixed descriptor plus
   inline bytes fail closed.
4. Count/per-file/total budgets and supported MIME registry are identical for
   WebUI and channel paths.
5. Rust workspace-path extraction matches WebUI fixtures for prose, punctuation,
   duplicates, unsupported extensions, inline code, fenced code, and traversal.
6. Outbound reader resolves only scoped `/workspace/...` paths and rejects
   missing, oversized, aggregate-oversized, and cross-scope files.
7. An adapter that does not override attachment rendering rejects non-empty
   attachments and still handles text-only replies unchanged.

### Telegram whole journey

Extend `reborn_integration_telegram_journey` through the existing production
composition and hermetic Bot API double:

1. Photo with caption -> verified webhook -> `getFile` -> bounded download ->
   `/workspace/attachments/...` -> transcript ref -> model sees image -> native
   reply.
2. Document-only update and multiple documents preserve order and reach the
   model/workspace without synthetic text.
3. Replayed `update_id` performs one download/landing/turn.
4. Unsupported, oversized, missing `file_path`, 401, 429/5xx, and truncated
   downloads produce honest failure and no attachment-less turn.
5. Assistant text referencing a valid workspace image/document performs native
   multipart upload and text delivery with the correct chat/topic/reply target.
6. Missing/oversized outbound file performs no misleading text-only success;
   partial provider delivery is terminal and redacted.
7. Restart/reopen proves the landed bytes and transcript ref remain available.

### Slack whole journey

Extend the production Slack serve/host integration suite with a hermetic Slack
API and file-host double:

1. File share -> verified event -> `files.info` -> authorized bounded private
   download -> shared landing -> transcript/model -> native reply.
2. Attachment-only and multiple-file events preserve order.
3. Replayed event id performs one lookup/download/landing/turn.
4. Forged download host, metadata mismatch, missing `files:read`, unsupported or
   oversized files, 401, 429/5xx, and truncated bodies fail without a turn.
5. Assistant workspace reference performs get-upload-URL -> bounded upload ->
   complete-upload in the right channel/thread with the reply comment.
6. Missing `files:write`, missing/oversized outbound paths, and partial uploads
   record honest non-duplicate-safe failures.
7. Two users/projects cannot read, deliver, or download each other's attachment.

### Existing WebUI regression

Keep `reborn_integration_attach` green and add a cross-surface fixture proving a
WebUI-uploaded file and a Telegram/Slack-uploaded file produce the same
`AttachmentRef`, workspace-read, transcript, model-context, and history behavior
after the provider materialization boundary.

## Compatibility, rollout, and rollback

- Text-only Telegram, Slack, WebUI, triggered delivery, prompts, and status
  messages retain their existing entrypoints.
- Existing Slack installations may need the additive `files:write` scope for
  outbound attachments; inbound already requires `files:read`. Surface this as a
  reconnect requirement, not a startup failure for text-only use.
- No persistence schema migration is required. Landed files and message
  `AttachmentRef` values use the existing format.
- Rollback removes the materializer and transient renderer wiring. Existing
  landed attachments remain ordinary workspace files and durable transcript
  refs; no cleanup or down-migration is necessary.

## Documentation updates

The implementation updates:

- `docs/reborn/contracts/telegram-v2.md` and Slack channel/setup documentation;
- `FEATURE_PARITY.md` attachment rows;
- `CHANGELOG.md` under the current unreleased section;
- Slack scope/setup copy for `files:read` and `files:write`;
- integration coverage comments/maps where the existing Telegram and Slack
  journey suites enumerate behavior.

## Non-goals

- No Telegram groups or new Slack admission behavior.
- No audio transcription, video understanding, OCR, or new MIME support. Such
  files may be landed only when already supported by the shared registry; model
  interpretation remains the existing attachment-context contract.
- No public file links, provider-to-provider forwarding, remote URL attachments,
  attachment deletion policy, or user-authored outbound destination selection.
- No alternate attachment syntax or explicit `send_file` tool.
