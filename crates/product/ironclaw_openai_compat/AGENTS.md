# Agent Map — ironclaw_openai_compat

Working rules for the OpenAI-compatible boundary. Orientation lives in
`README.md`; family rules in `crates/product/AGENTS.md`.

> **Product boundary (WS5 transport inversion, 2026-08-01).** This adapter
> speaks `ironclaw_product_contracts` — `surface` for the membrane,
> `inbound_requests` for the bodies it constructs, `inbound`/`outbound`/
> `projection`/`product_wire` for what crosses back — plus
> `ironclaw_extension_contracts` for the one channel-facing enum it stamps
> (`ProductTriggerReason`). Its whole remaining `ironclaw_assistant` surface is
> three command descriptor constants (`SUBMIT_TURN_COMMAND`,
> `CREATE_THREAD_COMMAND`, `CANCEL_RUN_COMMAND`) — product's frozen inventory
> per PROPOSAL §6.1.3. That list is pinned exactly and shrink-only by
> `ironclaw_architecture_tests/tests/reborn_transport_product_boundary.rs`; a new
> `ironclaw_assistant` import fails it.

## Start Here

- Read `src/descriptors.rs` before changing routes or ingress policy.
- Read `src/error.rs` before changing any HTTP error shape.
- Read `src/mount.rs` before changing what composition must supply.

## Boundary

This crate is a product/API route surface, not a host runtime:

- It may define DTOs, route descriptors, sanitized error envelopes, and
  axum route fragments for host composition.
- It must not bind sockets, call `axum::serve`, or proxy directly to
  `ironclaw_llm`.
- Host composition owns listener binding, bearer/session auth, CORS/origin,
  body/rate limits, audit, and the port *implementations* that reach runtime
  services.
- **Router assembly is this crate's, not composition's** (WS6 OpenAI-compat
  eviction, 2026-08-05). `mount.rs` takes an `OpenAiCompatRouteMountPorts` —
  product surface, ref store, the two projection readers, the external-tool
  store/resume pair, and an optional `LlmConfigService` — and returns a
  `ProtectedRouteMount`. Which workflow gets which port, the builder order, the
  shared projection streamer, and "no LLM config means `/v1/models` stays
  fail-closed at 501" are this surface's own rules and belong to its owner.
  Composition builds the port implementations (they name `ironclaw_threads` /
  `ironclaw_turns` / `ironclaw_event_streams`, all on this crate's forbidden
  list) and hands them over; it no longer knows the builder order.
- Chat, Responses, and streaming paths route through the channel-neutral
  `ProductSurface` plus projection-reader/streamer ports rather than
  recreating v1 `/v1/chat/completions` LLM proxy behavior.
- Do not execute client-supplied OpenAI tools as Reborn capabilities.

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
  turn-run / projection refs by ProductSurface wiring slices.
- The side-effect-free `OpenAiCompatRefStore` port and ref vocabulary are the
  default surface. The durable `OpenAiCompatRefStore` adapter compiles
  unconditionally — this crate declares no cargo features at all, and
  `ironclaw_filesystem` is a plain `[dependencies]` entry that every consumer
  pulls.

## Chat Completions Workflow

The default router remains fail-closed unless host composition injects
`OpenAiCompatRouterState::with_chat_completions(...)`.
`ironclaw_composition::build_openai_compat_route_mount` performs that
host wiring for `ironclaw serve` — since 2026-08-05 by filling in
`OpenAiCompatRouteMountPorts` and calling this crate's own
`openai_compat_route_mount` (`mount.rs`), which does the injection. The
injected `OpenAiChatCompletionsWorkflow` handles Chat Completions create and
optional projection-backed SSE streaming:

- `POST /v1/chat/completions` parses the OpenAI-compatible DTO, reserves an
  opaque `chatcmpl-*` ref with actor-scoped idempotency, and submits the user
  message through the channel-neutral `ProductSurface` service.
- The route builds a canonical projection read request from the authenticated
  caller and ProductSurface thread response, then waits through a
  composition-supplied `OpenAiChatCompletionProjectionReader`. Timeout returns
  a retryable sanitized API error and does not cancel or detach the underlying
  product turn.
- Detached waits must remain bounded by the shared Reborn turn-admission
  reservation held by `ProductSurface` / `TurnCoordinator`. Do not add a
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
  cross-tenant claims fail closed before product surface access.
- Streaming create consumes a composition-supplied projection streamer and must
  suppress keepalive/control frames, internal refs, projection cursors, and
  sanitized backend details.
- This crate still must not call `ironclaw_llm`, `TurnCoordinator`, projection
  internals, listener APIs, secrets, DBs, or the host runtime directly. The
  real streaming boundary is the projection stream; retired-name pins live in
  the architecture suite, not here.

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
- The catalog is `mount::LlmConfigModelCatalog`, this crate's own projection of
  the operator `LlmConfigService` snapshot (the same configured-model source the
  operator WebUI uses). Composition supplies the service —
  `OpenAiCompatRouteMountPorts::llm_config`, an `Option` — and `None` is what
  produces the fail-closed `501` above; the mapping itself, including the
  `LlmConfigServiceError` → status/retryability table, lives here.
- The crate maps catalog entries into the OpenAI list envelope
  (`{ object: "list", data: [{ id, object: "model", created, owned_by }] }`);
  it does not reach into `ironclaw_llm` or the runtime directly.

The `model` string on chat/responses create requests is validated at the parse
boundary (`validate_model_name`): non-empty, no surrounding whitespace, no
control characters, and at most 256 bytes (the #2673 bounded-resources bound).
Violations return a sanitized `400` naming the `model` param.

## Responses Workflow

Host composition may also inject
`OpenAiCompatRouterState::with_responses(...)` for the non-streaming Responses
slice:

- `POST /api/v1/responses` and `POST /v1/responses` reserve opaque `resp_*`
  refs with actor-scoped idempotency, submit create requests through
  `ProductSurface`, and wait through a composition-supplied
  `OpenAiResponsesProjectionReader`.
- `GET /api/v1/responses/{id}` and `GET /v1/responses/{id}` read
  projection-backed state through an authorized opaque-ref lookup. They must not
  reconstruct state from legacy messages.
- `POST /api/v1/responses/{id}/cancel` and `POST /v1/responses/{id}/cancel`
  submit a typed ProductSurface cancel action for authorized, bound response
  refs. Unauthorized and nonexistent refs stay indistinguishable at the API
  boundary.
- Request `tools` / `tool_choice` remain unsupported in this slice, except that
  an empty `tools: []` is treated like an omitted field.
- Client-controlled Responses input is serialized as a structured
  `openai_compat.responses_input.v1` JSON payload inside `UserMessagePayload`
  text so CR/LF-delimited role spoofing cannot create synthetic transcript
  lines while `function_call` `call_id` and `arguments` remain available.
- `stream: true` uses the same ProductSurface submission and opaque ref
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

## Validation

- `cargo test -p ironclaw_openai_compat`
- `cargo clippy -p ironclaw_openai_compat --all-targets --all-features -- -D warnings`
- `cargo test -p ironclaw_architecture_tests reborn_crate_dependency_boundaries_hold`
