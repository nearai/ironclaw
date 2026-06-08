# ironclaw_reborn_openai_compat

Reborn-native OpenAI-compatible API contract surface for #3283 / #4442 /
#4443.

## Boundary

This crate is a product/API route surface, not a host runtime:

- It may define DTOs, route descriptors, sanitized error envelopes, and
  feature-gated fail-closed axum handlers.
- It must not bind sockets, call `axum::serve`, read v1 gateway state, or proxy
  directly to `ironclaw_llm`.
- Host composition owns listener binding, bearer/session auth, CORS/origin,
  body/rate limits, mounting, audit, and product workflow wiring.
- Later slices should route through the channel-neutral `ProductWorkflow`
  surface rather than recreating v1 `/v1/chat/completions` LLM proxy behavior.

## Opaque Refs and Idempotency

The `refs` module owns the OpenAI-compatible identity contract used before
routes are wired to ProductWorkflow:

- Public ids are typed opaque refs: `chatcmpl-*` for Chat Completions and
  `resp_*` for Responses.
- Generated ids use host entropy and must not encode tenant, user, thread, run,
  product-action, projection, cursor, or host-path values.
- Client idempotency keys are scoped by actor scope + route surface +
  request-body fingerprint. Same key and same fingerprint replays the same
  mapping; same key with a different fingerprint returns a sanitized conflict.
- Missing idempotency keys create a new mapping on every POST.
- Lookup/cancel/stream-resume authorization checks use actor scope. Unauthorized
  and nonexistent refs are intentionally indistinguishable to API callers.
- Mappings start as pending and are later bound to internal product-action /
  turn-run / projection refs by ProductWorkflow wiring slices.
- Durable storage adapters live in `ironclaw_reborn_openai_compat_storage`;
  this crate defines only the side-effect-free `OpenAiCompatRefStore` port and
  ref vocabulary.

## Route Surface

The descriptor table covers:

- `POST /v1/chat/completions`
- `POST /api/v1/responses`
- `POST /v1/responses`
- `GET /api/v1/responses/{response_id}`
- `GET /v1/responses/{response_id}`
- `POST /api/v1/responses/{response_id}/cancel`
- `POST /v1/responses/{response_id}/cancel`

All routes require bearer auth and authenticated caller scope. Create routes
are declared as SSE-capable because the OpenAI-compatible request body may set
`stream: true`; non-streaming behavior is still handled by the same route.

## Fail-Closed Slice

The `openai-compat-beta` feature exposes an axum router and handlers, but every
handler currently returns a sanitized `501` OpenAI-compatible error. Do not wire
real turn submission, retrieval, cancel, or streaming in this slice.

## DTO Policy

Request DTOs intentionally tolerate unknown fields so OpenAI-compatible clients
with newer optional parameters do not fail during deserialization. Specific
fields that affect Reborn policy, such as `tools`, `tool_choice`, `stream`, and
`model`, are modeled explicitly so later slices can reject unsupported behavior
with stable errors.

Response and error DTOs are narrow. Error construction should use the helpers in
`src/error.rs`; do not surface raw backend messages, host paths, secrets,
provider/runtime diagnostics, or raw user content.
