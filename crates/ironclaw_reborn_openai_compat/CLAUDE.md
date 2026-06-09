# ironclaw_reborn_openai_compat

Reborn-native OpenAI-compatible API contract surface for #3283 / #4442 /
#4443 / #4444.

## Boundary

This crate is a product/API route surface, not a host runtime:

- It may define DTOs, route descriptors, sanitized error envelopes, and
  feature-gated axum route fragments for host composition.
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

## Chat Completions Workflow

With `openai-compat-beta`, the default router remains fail-closed unless host
composition injects `OpenAiCompatRouterState::with_chat_completions(...)`.
`ironclaw_reborn_composition::build_openai_compat_route_mount` performs that
host wiring for `ironclaw-reborn serve` by mounting the router inside the
protected Reborn route stack. The injected `OpenAiChatCompletionsWorkflow` is
the non-streaming Chat Completions slice:

- `POST /v1/chat/completions` parses the OpenAI-compatible DTO, reserves an
  opaque `chatcmpl-*` ref with actor-scoped idempotency, and submits the user
  message through the channel-neutral `ProductWorkflow` surface.
- The route resolves the canonical projection read request through
  `ProductWorkflow::read_projection(...)`, then waits through a
  composition-supplied `OpenAiChatCompletionProjectionReader`. Timeout returns
  a retryable sanitized API error and does not cancel or detach the underlying
  product turn.
- The canonical projection read actor/scope must match the authenticated caller
  before the projection reader is invoked.
- The requested public model string is carried as a composition/policy hint for
  the projection reader; do not inject it into the user transcript text.
- Client-supplied `tools` and `tool_choice` are model hints only. They are
  forwarded on the projection reader request as model-only metadata and must not
  execute as Reborn capabilities from this crate.
- The route requires a verified `OpenAiCompatAuthenticatedCaller` extension
  minted by host auth middleware. Do not mint auth evidence in this crate's
  production feature set.
- This crate still must not call v1 gateway handlers, `ironclaw_llm`,
  `TurnCoordinator`, projection internals, listener APIs, secrets, DBs, or the
  host runtime directly.

## DTO Policy

Request DTOs intentionally tolerate unknown fields so OpenAI-compatible clients
with newer optional parameters do not fail during deserialization. Specific
fields that affect Reborn policy, such as `tools`, `tool_choice`, `stream`, and
`model`, are modeled explicitly so later slices can reject unsupported behavior
with stable errors.

Response and error DTOs are narrow. Error construction should use the helpers in
`src/error.rs`; do not surface raw backend messages, host paths, secrets,
provider/runtime diagnostics, or raw user content.
