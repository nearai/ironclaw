# Generic Cross-Channel Attachments Design

**Date:** 2026-07-29
**Status:** Implemented and verified
**Target:** PR #6364 (`codex/telegram-slack-attachments`)

## Summary

Make attachments one product contract across WebUI, Telegram, Slack, and future
channels:

- inbound files are validated against one host policy, fetched before
  admission, committed as one atomic message batch, and stored as durable
  attachment references;
- the agent can read or write those workspace files through the existing
  mediated filesystem tools;
- outbound files require an explicit structured attachment intent and never
  result from scanning reply prose for `/workspace/...` paths;
- the finalized assistant message is the durable source for both WebUI
  attachment presentation and channel file delivery;
- channel adapters own only provider transfer and rendering, through restricted
  egress and host-injected credentials.

The implementation also fixes the production channel attachment lander, which
currently receives a read-only project mount even though attachment landing
requires write permission.

## Approved Product Decisions

1. **Outbound attachment selection is explicit.** The model must invoke a
   structured host capability to attach a workspace file to its reply.
   Mentioning a path in natural language, Markdown, or a code block does not
   attach or send that file.
2. **Inbound multi-file ingestion is all-or-nothing.** No user message,
   attachment reference, run, or committed workspace attachment becomes
   visible unless every file in the message succeeds.
3. **The host owns attachment policy.** Count, per-file bytes, aggregate bytes,
   and MIME support are uniform across product surfaces. A manifest or provider
   adapter may narrow the effective policy but cannot widen it.
4. **One durable message shape serves every surface.** WebUI does not receive a
   special assistant-file DTO and channels do not maintain a parallel
   attachment store.

## Current-State Findings

The branch already contains most of the neutral vocabulary:

- `ironclaw_attachments::DEFAULT_ATTACHMENT_BUDGETS` supplies the current
  10-file, 10 MiB-per-file, 10 MiB-total policy.
- WebUI decodes inline uploads, the channel workflow fetches provider bytes,
  and both paths call the shared attachment lander.
- `MessageContent` already carries durable attachment references.
- `OutboundPart::File` already carries transient, host-materialized bytes to a
  channel adapter.
- Telegram implements provider download and `sendDocument`.

Four current behaviors prevent the approved product contract:

1. production composition wires `ProjectScopedAttachmentLander` to a read-only
   workspace filesystem, causing live Telegram landing to fail;
2. `land_inbound_attachments` writes sequentially, and its current regression
   test explicitly leaves an earlier file behind when a later file fails;
3. WebUI file chips and channel outbound materialization infer files by parsing
   assistant reply text for workspace paths;
4. Slack preserves attachment descriptors but implements neither
   `fetch_attachment` nor `OutboundPart::File` delivery.

## Goals

- Accept, store, reference, inspect, and download inbound attachments from
  WebUI, Telegram, and Slack through the same product workflow.
- Allow an agent to write a workspace file and attach it explicitly to the
  final reply on any outbound-capable surface.
- Preserve attachment identity, filename, normalized MIME, byte length, and
  scoped storage reference without persisting raw bytes in events, delivery
  attempts, logs, or provider DTOs.
- Make replay, retry, and restart behavior deterministic.
- Enforce identical host limits before expensive extraction, persistence, or
  provider delivery.
- Prove the production composition path, both database backends, browser
  behavior, and live Telegram/Slack behavior.

## Non-Goals

- Arbitrary host-file attachment. Only files under the current thread's
  project workspace are eligible.
- Inferring attachment intent from prose, Markdown links, code blocks, tool
  output text, or filenames.
- Atomic multi-file delivery at a third-party provider. Telegram and Slack do
  not expose a transaction spanning several messages/files. Partial outbound
  acceptance is recorded honestly and completed parts are not duplicated.
- Changing the existing model-facing file read/write tools beyond documenting
  the new reply-attachment capability.
- Migrating old inbound attachment paths. Existing durable storage references
  remain readable.
- Adding Slack user-token file tools. Channel file transfer uses the
  administrator-configured bot identity only.

## Options Considered

### 1. Run-scoped attachment registration capability

Add a provider-neutral built-in capability,
`builtin.attach_workspace_file_to_reply`, that validates and registers a
workspace file for the current run. Finalization consumes the registered
intents and writes them into the assistant message's existing
`MessageContent.attachments`.

This is the selected option. It is explicit, works with every model provider
that supports tools, leaves an audit trail, and reuses the durable message and
delivery contracts already present.

### 2. Extend every model provider's final-response schema

Each LLM wrapper could return text plus attachment references. This is
type-clean at the model boundary but forces provider-specific structured-output
support, changes the complete provider trait wrapper chain, and makes ordinary
tool-capable providers unnecessarily difficult to support.

### 3. Keep path inference and only finish adapter transfer

Fixing the mount and adding Slack transfer would be the smallest patch. It is
rejected because path mentions are ambiguous, can be produced accidentally,
and cannot provide a reliable durable attachment contract to WebUI or retries.

## Architecture

### 1. Shared policy

`ironclaw_attachments` remains the owner of neutral attachment policy:

```text
AttachmentPolicy
  budgets:
    max_count
    max_file_bytes
    max_total_bytes
  supported MIME registry
```

The current numeric defaults remain unchanged. WebUI staging configuration,
product-surface inline decode, channel descriptor preflight, fetched-byte
validation, atomic landing, outbound intent registration, and outbound
materialization all consume this policy.

Static channel-specific narrowing, when needed, is manifest-declared rather
than adapter-reported. Resolved policy is the intersection of the host policy
and the manifest declaration. Assembly rejects any declaration that attempts
to exceed a host maximum or add a MIME type outside the host registry.
Provider runtime constraints may reject an otherwise host-valid transfer, but
can never cause the host to accept bytes outside host policy.

All arithmetic is checked or saturating as appropriate. Declared provider sizes
are a preflight hint only; actual downloaded or read bytes are authoritative.

### 2. Atomic inbound landing

The generic inbound flow is:

```text
bounded surface request
  -> descriptor/count/MIME/declared-size preflight
  -> fetch or decode every file into bounded memory
  -> actual-size and aggregate validation
  -> extraction/metadata preparation for every file
  -> one atomic filesystem attachment-batch commit
  -> one finalized user message with all attachment refs
  -> turn admission
```

The filesystem boundary gains an atomic batch operation suitable for files
under one attachment-message directory:

- the local backend writes every file into a unique sibling staging directory,
  flushes the completed batch, and atomically renames that directory to its
  deterministic committed directory;
- LibSQL and PostgreSQL write all file rows and directory metadata in one
  backend transaction;
- the in-memory and fault-injecting implementations preserve the same
  all-or-nothing contract for tests;
- scoped filesystem permission checks cover the complete batch before a
  backend starts the mutation.

New attachments use a message-directory layout such as:

```text
/workspace/attachments/<date>/<message-id>/<index>-<safe-filename>
```

Old flat attachment paths remain readable. A deterministic message directory
plus idempotent commit makes webhook replay return the existing committed
batch. A conflicting replay fails closed.

If fetch, validation, extraction, permission checking, staging, or commit
fails, the product surface accepts no message and submits no run. Staging
artifacts are removed best-effort immediately and are never addressable through
a durable attachment reference. Startup or periodic maintenance removes stale
uncommitted staging directories left by process termination.

If the file batch commits but later message acceptance fails, the workflow
compensates by deleting the unreferenced committed batch. The same maintenance
pass removes committed attachment directories that have no accepted-message
reference after the bounded reconciliation window, covering process termination
between file commit and message acceptance. It never removes a batch referenced
by an accepted or finalized message.

### 3. Explicit outbound attachment intent

The model-visible capability accepts one thread-scoped path:

```json
{
  "path": "/workspace/reports/final.pdf"
}
```

It returns bounded public metadata and an opaque intent identifier, never raw
bytes or a host path. The capability:

1. resolves the path through the current run's project scope;
2. verifies it is a regular readable file;
3. normalizes MIME and filename through the shared registry;
4. checks the per-file limit;
5. registers an idempotent run-scoped `ReplyAttachmentIntent`.

Registration uses a neutral port; the capability implementation does not
depend on product or channel crates. The durable implementation is co-located
with thread-message persistence and keys intents by tenant, project, thread,
run, and normalized scoped path. Repeating the same path for the same run is
idempotent. Registering distinct files beyond count or aggregate limits fails
before finalization.

Registration does not send bytes. It is an explicit part of composing the
current final reply and follows the capability authorization, approval,
obligation, and audit path. Cross-thread, non-workspace, directory, missing,
oversized, unsupported, or unreadable paths fail closed with a model-visible
correctable outcome.

### 4. Assistant finalization and durable references

`ThreadBackedLoopTranscriptPort::finalize_assistant_message` loads the current
run's registered intents, revalidates metadata, and finalizes one
`MessageContent` containing text plus attachment references.

The finalize operation is idempotent:

- a retry with the same text and attachment set returns the existing finalized
  message;
- a retry whose attachment set differs is a conflict;
- registered intents cannot be mutated after successful finalization;
- a failed or cancelled run never exposes its pending intents as a finalized
  reply.

The message stores scoped references and metadata, not attachment bytes.
Existing thread history and product projection paths then expose the same
assistant attachments WebUI already understands for user messages.

### 5. WebUI behavior

Inbound staging continues to use the current attachment picker, but server
policy remains authoritative. A multi-file send receives one failure and
creates no message if any file fails.

Assistant message chips are rendered from durable attachment references.
Preview and download continue through the authorized
thread/message/attachment routes. New messages never call
`extractWorkspaceFilePaths`.

For compatibility, an old assistant message without attachment metadata may
retain a display-only legacy workspace-path chip. That fallback cannot add a
delivery part, modify a message, or cause network egress.

### 6. Channel delivery

The final-reply delivery observer loads the finalized assistant message, not
only its text. The delivery attempt persists semantic text plus attachment
references. Immediately before provider dispatch, the delivery coordinator:

1. rejects any caller-supplied byte-bearing file part;
2. validates the attachment set and current thread scope;
3. stats every referenced file and checks the complete declared budget;
4. reads every file with bounded reads and checks the actual aggregate budget;
5. appends transient `OutboundPart::File` values;
6. hands the normalized envelope to the pinned channel adapter.

No raw bytes are written to a delivery attempt, event, projection, transcript,
or log. Retry uses the same durable attachment references. A path in the text
has no delivery effect.

### 7. Telegram transfer

Telegram keeps its existing protocol ownership:

- inbound `getFile` metadata and bounded download through restricted egress;
- outbound multipart `sendDocument`;
- Telegram-specific provider limits and error mapping;
- provider message/file identifiers as delivery evidence.

Production composition supplies the attachment lander a project mount that has
the exact write permissions required by atomic landing. General model
filesystem permissions and unrelated mounts remain unchanged.

### 8. Slack transfer

Inbound Slack file descriptors retain only bounded public metadata and the
opaque Slack file ID. `fetch_attachment`:

1. calls `files.info` with the bot credential through restricted egress;
2. validates returned identity, size, MIME, and private-download URL;
3. downloads only from the manifest-allowlisted Slack file host;
4. supplies bounded bytes to the generic workflow.

Outbound `OutboundPart::File` uses Slack's supported external upload sequence:

1. pre-validate the complete ordered file batch;
2. call `files.getUploadURLExternal` and upload bounded bytes for every file
   without sharing any of them;
3. call `files.completeUploadExternal` once with the complete ordered file-ID
   array plus the resolved channel and thread;
4. perform bounded `files.info` read-back for every file before reporting
   verified success.

Only the read-back step is retried after Slack accepts completion. This covers
the provider's short destination-indexing delay without replaying tickets,
bytes, or completion. Exhausted read-back remains terminal so a file-only
envelope cannot duplicate an already accepted provider side effect. Slack does
not accept a zero-byte external-upload ticket, so that provider-specific
narrowing fails before egress with an explicit permanent outcome.

The channel manifest declares the exact API/upload hosts, methods, path
prefixes, body limits, and bot credential handle needed by those calls. The
workspace bot requires `files:read` and `files:write`; setup documentation and
the minimal Slack app manifest are updated. Reinstalling or expanding a live
Slack app's scopes is an explicit operator action outside repository mutation.

No private Slack download URL, bearer token, response body containing
credentials, or raw file bytes are persisted or logged. Vendor and egress
failures map to retryable or permanent attachment-transfer outcomes without
provider details leaking to the model or user.

## Error and Recovery Semantics

### Inbound

- malformed descriptors, unsupported MIME, invalid filenames, excessive
  count, or declared oversize: permanent rejection before fetch;
- provider timeout/rate limit or inconsistent retrievable metadata: retryable
  transfer failure where safe;
- provider-retrieved MIME or actual byte-length mismatch: fail closed, with
  retryability based on whether a stable provider retry can correct it;
- any landing failure: no committed batch, message, or run;
- duplicate webhook after a committed batch: idempotent replay;
- duplicate webhook after a failed batch: a fresh bounded attempt.

### Outbound

- missing or unreadable registered file: final delivery fails honestly;
- path mention without registered intent: text-only delivery;
- provider accepts some parts and then fails: the delivery report records the
  exact accepted prefix and terminal partial failure;
- retries do not resend provider-confirmed parts;
- a provider-issued upload ID plus successful read-back is verified evidence;
  if a provider makes read-back unavailable, the result is explicitly
  unverified rather than reported as completed.

## Security Invariants

- Every public request is verified and bounded before attachment processing.
- Vendor adapters never receive filesystem services or arbitrary network
  clients.
- Network calls use restricted egress and host-side credential injection.
- Scoped project paths are authoritative; display strings and provider
  metadata never derive tenant, project, thread, turn, or run identity.
- Filenames are presentation metadata only and are sanitized before storage.
- MIME is normalized and checked against the shared host registry before
  extraction, persistence, model context, preview, or delivery.
- Raw bytes and provider-private references are transient and redact from
  `Debug`, logs, events, state, and errors.
- The reply-attachment capability cannot bypass normal authorization,
  approvals, obligations, or capability visibility.
- No production code introduces `.unwrap()` or `.expect()`.

## Testing Strategy

Tests are written red-first and reach the production caller or wrapper chain
that owns each contract.

### Unit and contract

- shared policy intersection cannot widen count, byte, or MIME limits;
- path mentions in prose, Markdown, and code blocks do not create attachment
  intents or file parts;
- reply attachment registration is scoped, bounded, idempotent, and sealed
  after finalization;
- assistant finalization preserves text plus the exact registered references;
- atomic landing fault injection before, during, and after each file leaves no
  committed partial batch;
- legacy flat references remain readable;
- Slack/Telegram provider parsing, transfer request construction, bounds,
  redaction, and failure mapping;
- WebUI history mapping renders assistant attachment refs without parsing text.

### Filesystem backend

- local atomic directory commit and interrupted-staging cleanup;
- LibSQL and PostgreSQL transaction rollback after a later-file fault;
- in-memory and fault-injecting parity;
- permission denial occurs before mutation;
- concurrent duplicate commits converge on one identical batch, while
  conflicting contents fail closed.

### Reborn integration

Run the existing extension-delivery suites through production composition for
both LibSQL and PostgreSQL:

- WebUI, Telegram, and Slack single-file inbound;
- multi-file inbound and duplicate filenames;
- second-file fetch and second-file landing failures;
- model reads an inbound attachment;
- model writes a file, explicitly registers it, and returns it through each
  surface;
- text-only path mention produces no file;
- restart/replay preserves references and does not duplicate messages,
  batches, or provider sends;
- production channel host assembly proves the lander mount is writable without
  broadening unrelated workspace grants.

### Browser E2E

- select multiple files, inspect staged chips, send once;
- inspect user and assistant attachment chips;
- preview supported image/text/PDF content and download exact bytes;
- one invalid file rejects the whole batch with no optimistic message;
- a plain assistant path mention remains ordinary text.

### Live canaries

- Telegram: inbound document, model read/reference, generated-file explicit
  reply, exact downloaded bytes;
- Slack: inbound file, threaded model reply, generated-file upload, exact
  downloaded bytes;
- WebUI: bearer-authenticated browser flow covering inbound and outbound;
- inspect service logs and durable state for exact run/message correlation,
  delivery evidence, retry behavior, and absence of secrets or raw bytes.

Live canaries supplement but do not replace deterministic integration tests.

## Compatibility

- Existing numeric attachment limits remain unchanged.
- Existing inbound `AttachmentRef` wire fields remain readable.
- Old flat storage references remain supported; only new batches use the
  message-directory layout.
- Adding attachment fields to assistant projection shapes is additive and
  defaults to an empty list for old persisted data.
- Legacy workspace-path chips may remain display-only for old messages.
- Automatic outbound delivery from reply text is intentionally removed. A
  workspace path without an explicit intent becomes text-only.
- Existing plain-text Slack and Telegram behavior remains unchanged.

## Rollback

The change ships as reviewable commits with the generic contract before
provider enablement:

1. atomic filesystem/landing contract and production mount fix;
2. reply attachment intent registration and assistant-message finalization;
3. WebUI projection/presentation cutover;
4. Telegram production regression coverage;
5. Slack inbound and outbound transfer;
6. deterministic and live E2E evidence.

If provider transfer must be disabled, the Slack or Telegram manifest can
remove the corresponding egress capability while generic WebUI behavior and
durable messages remain intact. Reverting the explicit-intent feature does not
invalidate existing attachment references because they remain ordinary scoped
storage keys. New message-directory paths remain readable by the existing
project filesystem.

## Completion Criteria

The feature is complete only when all of the following are true:

- all three surfaces use the same durable attachment-reference contract;
- inbound multi-file failure leaves no committed partial files, accepted
  message, or run;
- outbound files require explicit structured intent;
- Slack and Telegram both transfer inbound and outbound bytes through
  restricted egress;
- production composition, both database backends, browser E2E, and live
  canaries pass;
- changed crates are formatted and clippy-clean with warnings denied;
- architecture, security, compatibility, rollback, and final-diff audits pass;
- the PR Test Strategy records exact commands and evidence for every applicable
  tier and does not claim readiness while review or CI gates remain unresolved.
