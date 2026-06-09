# Reborn OpenAI-Compatible API Contract

**Status:** contract, identity, and non-streaming Chat Completions workflow
slices (#4442, #4443, #4444)
**Parent:** #3283
**Crates:** `crates/ironclaw_reborn_openai_compat`,
`crates/ironclaw_reborn_openai_compat_storage`

## Purpose

The OpenAI-compatible API is a Reborn product/API ingress surface for clients
that speak Chat Completions or Responses. It is behavior-compatible at the HTTP
shape where practical, but it must not reuse the v1 gateway's stateless LLM
proxy code path.

These first slices are contract-first, with one narrow ProductWorkflow-backed
route. They define DTOs, host-owned ingress descriptors, a sanitized
OpenAI-style error envelope, route fragments, and the opaque ref/idempotency
vocabulary. `POST /v1/chat/completions` can submit non-streaming user-message
requests through ProductWorkflow when host composition injects the workflow
state. Responses routes, retrieve, cancel, and SSE translation remain
fail-closed.

## Route Surface

| Route | Method | Effect path | Streaming |
| --- | --- | --- | --- |
| `/v1/chat/completions` | POST | `ProductWorkflow` | SSE-capable |
| `/api/v1/responses` | POST | `ProductWorkflow` | SSE-capable |
| `/v1/responses` | POST | `ProductWorkflow` | SSE-capable |
| `/api/v1/responses/{response_id}` | GET | `ProjectionOnly` | none |
| `/v1/responses/{response_id}` | GET | `ProjectionOnly` | none |
| `/api/v1/responses/{response_id}/cancel` | POST | `ProductWorkflow` | none |
| `/v1/responses/{response_id}/cancel` | POST | `ProductWorkflow` | none |

All routes require bearer auth and authenticated-caller scope. Host composition
owns listener binding, bearer/session auth, CORS, body limits, rate limits,
audit, and mounting. Product/API crates expose descriptors only and must never
bind sockets or call `axum::serve`.

## Compatibility Rules

- Chat Completions and Responses request DTOs tolerate unknown fields so newer
  OpenAI-compatible clients do not fail during deserialization.
- Policy-relevant fields are modeled explicitly: `model`, `stream`, `tools`,
  `tool_choice`, prior response id, metadata, and message/input bodies.
- Client-supplied OpenAI tools are model-only compatibility data in this
  migration. They are not Reborn capabilities and must not execute through the
  capability host.
- External ids (`chatcmpl-*`, `resp_*`) are opaque product references. They must
  not encode tenant, user, thread, run, projection cursor, or host paths.
- Durable ref mappings are persisted behind `OpenAiCompatRefStore`; the
  contract crate defines the port and the storage crate provides
  filesystem-backed adapters under `/engine/openai_compat/refs/` with
  per-public-id mapping records plus per-scope idempotency index records.
- The in-memory ref store is bounded and evicts the oldest mappings when full.
  Durable filesystem retention and pruning are owned by host composition or the
  storage adapter lifecycle, not by route handlers.
- Client idempotency keys are scoped by authenticated actor scope, route
  surface, and request-body fingerprint. Same key + same fingerprint replays the
  same public ref; same key + different fingerprint is a sanitized conflict.
- Absence of an idempotency key always creates a fresh public ref/action
  mapping.
- Ref lookup for retrieve, stream resume, and cancel is actor/scope checked.
  Unauthorized and nonexistent refs must produce the same sanitized not-found
  response at the API boundary.
- Chat Completions projection reads must resolve through
  `ProductWorkflow::read_projection(...)` and the returned canonical
  actor/scope must match the authenticated caller before any projection reader
  is called.
- Ref mappings are two-stage: route code may reserve a pending public ref before
  ProductWorkflow side effects, then bind it to internal product-action,
  turn-run, and projection refs after those refs exist.
- Non-streaming Chat Completions wait timeout detaches from the wait, not from
  the underlying turn. The API response is a retryable sanitized service
  unavailable error.
- SSE translation is a later slice over `ironclaw_event_streams`; Reborn stream
  control frames must not leak into OpenAI-compatible SSE payloads.

## Non-Streaming Chat Completions

Host composition may mount `openai_compat_router_with_state(...)` with an
`OpenAiChatCompletionsWorkflow` for `POST /v1/chat/completions`.

The route:

- Requires verified bearer/session auth middleware to provide
  `OpenAiCompatAuthenticatedCaller`.
- Rejects `stream: true` before ProductWorkflow side effects.
- Reserves an actor-scoped opaque `chatcmpl-*` ref and idempotency mapping
  before submission.
- Converts OpenAI-compatible messages into a `UserMessagePayload` and submits it
  through `ProductWorkflow`.
- Resolves the canonical projection read request through
  `ProductWorkflow::read_projection(...)`, then waits through a
  composition-supplied projection reader and returns a sanitized Chat
  Completions response.
- Carries the requested public model string as a composition/policy hint for
  the projection reader; the route must not inject the model name into user
  transcript text.
- Preserves model-produced tool-call output shape in the response, while
  treating client-supplied tools as model-only hints rather than executable
  Reborn capabilities.

## Error Shape

Errors serialize as:

```json
{
  "error": {
    "message": "The request is invalid.",
    "type": "invalid_request_error",
    "param": "messages[0].content",
    "code": "invalid_request"
  }
}
```

Messages and codes come from a fixed sanitized vocabulary. Route code must not
surface raw provider/runtime diagnostics, host paths, backend details, raw
prompts, raw tool input/output, secrets, or user content in error payloads.

## Current Fail-Closed Behavior

With `openai-compat-beta`, the default route fragment can be mounted for
composition tests and returns `501` with code `unsupported`. Host composition
can inject the non-streaming Chat Completions workflow state. Other route
families keep returning fail-closed sanitized errors until their own
ProductWorkflow, projection, cancel, or event-stream slices land.
