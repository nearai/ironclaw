# Structured Multimodal Replies

## Problem

The attachment delivery path already persists structured attachment metadata and
delivers native files, but the current display fix parses finalized model prose
and matches Markdown destinations such as `/workspace/report.csv` against
attachment storage keys. That makes unstructured model text participate in a
feature contract and duplicates the reconciliation logic in Rust and the
WebUI.

Model prose must never grant attachment authority, select workspace files, or
serve as the identifier joining a message to an attachment.

## Decision

An explicit `builtin.attach_workspace_file_to_reply` capability call is the
only model action that registers an outbound attachment. Registration returns a
bounded, opaque attachment handle and safe presentation metadata. The finalized
assistant message persists ordinary text and durable `AttachmentRef` values as
separate fields. The host automatically includes every successfully registered
attachment when it finalizes the current run.

The model chooses wording. The host chooses and validates attachment bytes,
identity, ordering, limits, persistence, and delivery.

## Canonical contract

The existing canonical shape remains the foundation:

```text
MessageContent
├── text: String
└── attachments: Vec<AttachmentRef>
    ├── id: opaque stable handle
    ├── kind: image | audio | video | document | other
    ├── filename
    ├── mime_type
    ├── size_bytes
    └── storage_key: host-only workspace authority
```

`ReplyAttachmentIntent` remains run-scoped metadata. A stable opaque handle is
derived by host code from trusted run scope plus the validated workspace path.
The capability result exposes the handle, filename, MIME type, size, and
registration status; it does not expose the workspace storage key as the
user-facing reference.

Finalization seals the ordered intent set and converts each intent to an
`AttachmentRef` using the same handle derivation. The model cannot omit a
registered attachment by failing to mention it in its final text.

## Rendering and delivery

All surfaces consume the canonical message:

- WebUI renders attachment cards and authenticated previews/downloads from
  `AttachmentRef`.
- Slack receives text plus native file parts from the shared delivery
  coordinator.
- Telegram receives the same text plus native document/media parts.
- Future adapters such as Signal implement the same `OutboundPart::Text` and
  `OutboundPart::File` contract.

No adapter discovers attachments by parsing message text. No Markdown link can
cause file access or upload.

This change does not add inline attachment placement. If inline placement is
required later, it will use a typed `AttachmentMention { attachment_id, label
}` message part validated against the message's attachment set. It will not use
workspace paths or ad hoc Markdown tokens.

## User-visible text policy

The attachment feature does not rewrite finalized model prose. The
path-matching parsers in product delivery and WebUI history are removed.

The capability description and model-visible success result instruct the model
to refer to the safe filename or opaque handle and never present an internal
workspace path as a link. That instruction improves wording but is not an
authority boundary.

Generic Markdown safety remains a renderer responsibility. Unsupported local
URLs may be rendered as inert text by a shared presentation policy, but they
must never be reconciled against attachment metadata or interpreted as file
instructions.

## Security and failure behavior

- The capability validates `/workspace` scope, regular-file status, bounded
  reads, stable size, filename, MIME type, per-file limits, aggregate limits,
  and run scope before registration.
- Handles are opaque presentation identities, not filesystem capabilities.
- File bytes are read only after outbound policy authorization and immediately
  before adapter dispatch.
- Delivery revalidates durable metadata against the project filesystem.
- Unknown handles cannot be used to retrieve or deliver files.
- Registration, finalization, and delivery failures remain typed and
  fail-closed.

## Compatibility

The opaque handle is derived from already-persisted trusted intent fields, so
the reply attachment intent persistence schema does not change. Existing
canonical `AttachmentRef` wire fields remain compatible. The old path-matching
display behavior is removed rather than retained as a legacy fallback.

## Tests

The implementation must cover:

1. Capability success returns an opaque handle and omits the workspace path
   from its model-visible result.
2. Repeated registration of the same run/path is idempotent and returns the
   same handle.
3. Distinct files receive distinct handles and preserve registration order.
4. Finalization persists text unchanged plus durable attachment references.
5. Text containing `/workspace` Markdown cannot create, remove, or reorder
   attachments.
6. Product delivery materializes files only from durable references.
7. Slack and Telegram receive native file parts from the same canonical
   message contract.
8. WebUI timeline rendering uses attachment metadata without parsing prose.
9. Image, GIF, audio, video, document, and unknown MIME types retain the
   expected kind and native-delivery metadata.
10. Invalid paths, oversized files, duplicate conflicts, changed files,
    missing files, and unauthorized materialization fail closed.

Focused crate tests, the existing deterministic Reborn integration scenario,
channel adapter conformance tests, frontend tests, formatting, clippy, and the
architecture boundary test provide the merge evidence.
