# ironclaw_reborn_openai_compat

Reborn-native OpenAI-compatible API contract surface for #3283 / #4442 /
#4443 / #4444 / #4445 / #4446 / #4447.
## Boundary

This crate is a product/API route surface, not a host runtime:

- It may define DTOs, route descriptors, sanitized error envelopes, and
  feature-gated axum route fragments for host composition.
- It must not bind sockets, call `axum::serve`, read v1 gateway state, or proxy
  directly to `ironclaw_llm`.
- Host composition owns listener binding, bearer/session auth, CORS/origin,
  body/rate limits, mounting, audit, and product workflow wiring.
- Chat, Responses, and streaming paths route through the channel-neutral
  `ProductWorkflow` plus projection-reader/streamer ports rather than
  recreating v1 `/v1/chat/completions` LLM proxy behavior.

## Opaque Refs and Idempotency

The `refs` module owns the OpenAI-compatible identity contract:

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
- The side-effect-free `OpenAiCompatRefStore` port and ref vocabulary are the
  default surface. The durable `FilesystemOpenAiCompatRefStore` adapter (folded
  in from the former `ironclaw_reborn_openai_compat_storage` crate) lives behind
  the `storage`/`libsql`/`postgres` features, so contract-only consumers pull no
  `ironclaw_filesystem` dependency.

## Chat Completions Workflow

With `openai-compat-beta`, the default router remains fail-closed unless host
composition injects `OpenAiCompatRouterState::with_chat_completions(...)`.
`ironclaw_reborn_composition::build_openai_compat_route_mount` performs that
host wiring for `ironclaw-reborn serve` by mounting the router inside the
protected Reborn route stack. The injected `OpenAiChatCompletionsWorkflow`
handles Chat Completions create and optional projection-backed SSE streaming:

- `POST /v1/chat/completions` parses the OpenAI-compatible DTO, reserves an
  opaque `chatcmpl-*` ref with actor-scoped idempotency, and submits the user
  message through the channel-neutral `ProductWorkflow` surface.
- The route resolves the canonical projection read request through
  `ProductWorkflow::read_projection(...)`, then waits through a
  composition-supplied `OpenAiChatCompletionProjectionReader`. Timeout returns
  a retryable sanitized API error and does not cancel or detach the underlying
  product turn.
- Detached waits must remain bounded by the shared Reborn turn-admission
  reservation held by `ProductWorkflow` / `TurnCoordinator`. Do not add a
  route-local OpenAI-compatible quota, and do not release admission capacity
  until the underlying turn reaches a terminal state.
- The canonical projection read actor/scope must match the authenticated caller
  before the projection reader is invoked.
- The requested public model string is carried as a composition/policy hint for
  the projection reader; do not inject it into the user transcript text.
- Client-supplied `tools` and `tool_choice` are model hints only. They are
  forwarded on the projection reader request as model-only metadata and must not
  execute as Reborn capabilities from this crate.
- `stream: true` is enabled only when host composition injects an
  `OpenAiCompatProjectionStreamer`. The route translates projection-safe
  outbound envelopes into OpenAI-compatible SSE without exposing projection
  cursors, product refs, or backend details.
- The route requires a verified `OpenAiCompatAuthenticatedCaller` extension
  minted by host auth middleware. Do not mint auth evidence in this crate's
  production feature set. The verified auth evidence must carry the same
  tenant id and user subject as `OpenAiCompatActorScope`; unscoped or
  cross-tenant claims fail closed before product workflow access.
- Streaming create consumes a composition-supplied projection streamer and must
  suppress keepalive/control frames, internal refs, projection cursors, and
  sanitized backend details.
- This crate still must not call v1 gateway handlers, raw `SseManager`/
  `AppEvent` streams, `ironclaw_llm`, `TurnCoordinator`, projection internals,
  listener APIs, secrets, DBs, or the host runtime directly.

## Models Listing

`GET /v1/models` (and its `/api/v1/models` alias) lists the deployment's
configured models for OpenAI-compatible clients (model pickers, etc.).

- The route authenticates the caller first: a missing
  `OpenAiCompatAuthenticatedCaller` fails closed with `401` before the catalog
  is consulted.
- The model source is the host-injected `OpenAiCompatModelCatalog` port
  (mirroring the projection reader/streamer ports). When no catalog is wired the
  route fails closed with `501`, exactly like the chat/responses surfaces before
  composition wiring.
- `ironclaw_reborn_composition::build_openai_compat_route_mount` wires a catalog
  backed by the operator `LlmConfigService` snapshot (the same configured-model
  source the operator WebUI uses), but only under the `root-llm-provider`
  feature; otherwise the route stays fail-closed.
- The crate maps catalog entries into the OpenAI list envelope
  (`{ object: "list", data: [{ id, object: "model", created, owned_by }] }`);
  it does not reach into `ironclaw_llm` or the runtime directly.

The `model` string on chat/responses create requests is validated at the parse
boundary (`validate_model_name`): non-empty, no surrounding whitespace, no
control characters, and at most 256 bytes (the #2673 bounded-resources bound).
Violations return a sanitized `400` naming the `model` param.

## Responses Workflow

With `openai-compat-beta`, host composition may also inject
`OpenAiCompatRouterState::with_responses(...)` for the non-streaming Responses
slice:

- `POST /api/v1/responses` and `POST /v1/responses` reserve opaque `resp_*`
  refs with actor-scoped idempotency, submit create requests through
  `ProductWorkflow`, and wait through a composition-supplied
  `OpenAiResponsesProjectionReader`.
- `GET /api/v1/responses/{id}` and `GET /v1/responses/{id}` read
  projection-backed state through an authorized opaque-ref lookup. They must not
  reconstruct state from legacy messages.
- `POST /api/v1/responses/{id}/cancel` and `POST /v1/responses/{id}/cancel`
  submit a typed ProductWorkflow control action for authorized, bound response
  refs. Unauthorized and nonexistent refs stay indistinguishable at the API
  boundary.
- Request `tools` / `tool_choice` remain unsupported in this slice, except that
  an empty `tools: []` is treated like an omitted field.
- Client-controlled Responses input is serialized as a structured
  `openai_compat.responses_input.v1` JSON payload inside `UserMessagePayload`
  text so CR/LF-delimited role spoofing cannot create synthetic transcript
  lines while `function_call` `call_id` and `arguments` remain available.
- `stream: true` uses the same ProductWorkflow submission and opaque ref
  reservation path, then drains a composition-supplied projection streamer into
  OpenAI-compatible Responses SSE events. Stalled streams are bounded by the
  workflow wait timeout and fail with a sanitized retryable service error.

## DTO Policy

Request DTOs intentionally tolerate unknown fields so OpenAI-compatible clients
with newer optional parameters do not fail during deserialization. Specific
fields that affect Reborn policy, such as `tools`, `tool_choice`, `stream`, and
`model`, are modeled explicitly so later slices can reject unsupported behavior
with stable errors.

Response and error DTOs are narrow. Error construction should use the helpers in
`src/error.rs`; do not surface raw backend messages, host paths, secrets,
provider/runtime diagnostics, or raw user content.
