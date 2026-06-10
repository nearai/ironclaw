# Reborn OpenAI-Compatible API Contract

**Status:** contract, identity, ProductWorkflow-backed Chat Completions,
Responses create/retrieve/cancel, idempotency/opaque-ref, and projection-backed
SSE streaming slices (#4442, #4443, #4444, #4445, #4446, #4447)
**Parent:** #3283
**Crates:** `crates/ironclaw_reborn_openai_compat`,
`crates/ironclaw_reborn_openai_compat_storage`

## Purpose

The OpenAI-compatible API is a Reborn product/API ingress surface for clients
that speak Chat Completions or Responses. It is behavior-compatible at the HTTP
shape where practical, but it must not reuse the v1 gateway's stateless LLM
proxy code path.

These first slices are contract-first, with narrow ProductWorkflow-backed
routes. They define DTOs, host-owned ingress descriptors, a sanitized
OpenAI-style error envelope, route fragments, and the opaque ref/idempotency
vocabulary. `POST /v1/chat/completions` can submit non-streaming user-message
requests through ProductWorkflow when host composition injects the workflow
state. `POST /api/v1/responses`, `POST /v1/responses`, Responses retrieve,
and Responses cancel can use the same injected ProductWorkflow-backed Responses
service. Projection-backed SSE translation is owned by this route crate through a
composition-supplied projection-stream port; Reborn keepalive/control frames and
projection cursors stay internal.

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
  Reborn local-dev host composition places the production route's tenant-owned
  ref store under `/tenants/{tenant}/shared/openai_compat/refs` on the root
  filesystem; route handlers still access it only through `OpenAiCompatRefStore`.
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
- Timed-out create requests remain bounded by the shared Reborn turn-admission
  reservation held by ProductWorkflow / TurnCoordinator. OpenAI-compatible
  wrappers must not add route-local quota, route-local cancellation, or any
  admission release separate from the underlying turn's terminal transition.
- SSE translation consumes a composition-supplied projection stream port and
  emits OpenAI-compatible events from `ProductProjectionItem` state. Reborn
  keepalive/control frames, projection cursors, internal refs, provider
  diagnostics, and runtime details must not leak into SSE ids or payloads.

## Non-Streaming Chat Completions

With the `openai-compat-beta` feature, `ironclaw-reborn serve` mounts
`openai_compat_router_with_state(...)` inside the Reborn protected route stack
with an `OpenAiChatCompletionsWorkflow` for `POST /v1/chat/completions`.
Default routers remain fail-closed unless host composition injects that
workflow state.

The route:

- Requires verified bearer/session auth middleware to provide
  `OpenAiCompatAuthenticatedCaller`.
- Routes `stream: true` through the projection-backed SSE translator when a
  projection streamer is injected; otherwise rejects it before ProductWorkflow
  side effects.
- Reserves an actor-scoped opaque `chatcmpl-*` ref and idempotency mapping
  before submission.
- Converts OpenAI-compatible messages into a `UserMessagePayload` and submits it
  through `ProductWorkflow`.
- Resolves the canonical projection read request through
  `ProductWorkflow::read_projection(...)`, then waits through a
  composition-supplied projection reader. The local-dev Reborn composition
  reader polls `SessionThreadService::finalized_assistant_message_by_run` for
  the accepted run's finalized assistant message and returns a sanitized Chat
  Completions response.
- Inherits the shared turn-admission policy from the configured
  `TurnCoordinator` / turn-state store. A wait timeout must not release
  admission capacity while the underlying run remains queued, running, blocked,
  cancel-requested, or otherwise non-terminal.
- Carries the requested public model string as a composition/policy hint for
  the projection reader; the route must not inject the model name into user
  transcript text.
- Preserves model-produced tool-call output shape in the response, while
  treating client-supplied tools as model-only hints rather than executable
  Reborn capabilities.

## Non-Streaming Responses

Host composition may inject `OpenAiResponsesWorkflow` for:

- `POST /api/v1/responses`
- `POST /v1/responses`
- `GET /api/v1/responses/{response_id}`
- `GET /v1/responses/{response_id}`
- `POST /api/v1/responses/{response_id}/cancel`
- `POST /v1/responses/{response_id}/cancel`

The route:

- Requires verified bearer/session auth middleware to provide
  `OpenAiCompatAuthenticatedCaller`.
- Routes `stream: true` through the projection-backed SSE translator when a
  projection streamer is injected; otherwise rejects it before ProductWorkflow
  side effects. Request `tools` and `tool_choice` are explicitly rejected
  before ProductWorkflow side effects until a dedicated capability-view
  contract exists.
- Authorizes `previous_response_id` against the caller scope before using it as
  conversation context.
- Reserves actor-scoped opaque `resp_*` refs and idempotency mappings before
  create submission. Same-key/same-body replays the same projection-backed
  response; same-key/different-body returns a sanitized conflict.
- Submits create requests through `ProductWorkflow` as user-message payloads and
  waits through a composition-supplied projection reader. Wait timeout detaches
  the HTTP waiter and returns a retryable sanitized service-unavailable error.
- Retrieves current state through an authorized opaque-ref lookup and the
  projection reader, not raw legacy conversation messages.
- Cancels only when the opaque ref is authorized and bound to a Reborn run ref,
  then submits a typed ProductWorkflow control action. Nonexistent, unauthorized,
  and unbound refs all return the same sanitized not-found envelope.

## Streaming Chat And Responses

When host composition injects `with_projection_streamer(...)`, `stream: true`
Chat and Responses create requests submit through `ProductWorkflow` and then
translate projection updates into OpenAI-compatible SSE at the route boundary.
Chat emits `chat.completion.chunk` events plus `[DONE]`; Responses emits
`response.created`, `response.output_text.delta`, terminal `response.completed`,
`response.failed`, or `response.cancelled` events as appropriate. Terminal
`RunStatus` projection items complete or fail streams even when no synthetic
final-reply projection item is present.

Streaming idempotency follows the same actor-scope, route-surface, and
fingerprint rules as non-streaming create. Same-key/same-body replays reuse the
recorded accepted ProductWorkflow ack and must not resubmit after an accepted
turn; pending mappings left by a non-accepted submit may be retried. Same-key/
different-body conflicts remain sanitized.

The SSE translator must suppress Reborn keepalive/control frames, raw
projection cursors, internal product-action/turn-run/projection refs, provider
messages, host paths, secrets, raw tool input, and runtime details. Translation
errors emit sanitized OpenAI-compatible stream errors.

Responses request `tools` and `tool_choice` remain intentionally unsupported on
both streaming and non-streaming creates until Reborn has a capability-view
contract for exposing executable tool affordances through this API. Chat
Completions may pass client-supplied tools as model-only metadata, but route
code must not execute them as Reborn capabilities.

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
composition tests and returns `501` with code `unsupported` until host
composition injects workflow state. Host composition can inject Chat
Completions and Responses workflows, with optional projection streamers for
OpenAI-compatible SSE. Without a projection streamer, `stream: true` remains a
sanitized fail-closed invalid request rather than falling back to v1 gateway SSE
or raw `AppEvent` streams.
