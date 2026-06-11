# ironclaw_webui_v2

Reborn WebChat v2 HTTP route surface. Off by default — compile in with
the `webui-v2-beta` Cargo feature. The descriptors and handlers in this
crate are the route-layer; the gateway-layer (see "Host composition
still owes" below) is a separate piece host composition must land.

## Purpose

Owns the minimal native WebUI v2 route set on top of
`ironclaw_product_workflow::RebornServicesApi`. Handlers are the only
public surface; host composition consumes the
`IngressRouteDescriptor`s returned by `webui_v2_routes()` and mounts
each handler under the matching pattern after running its own bearer
auth, CORS, body-limit, and rate-limit middleware.

## Host composition still owes

Compiling this crate into a binary is not enough to expose the v2
routes to a browser. Host composition (gateway / app startup) still
owns:

1. **Mounting the router.** Call `webui_v2_router(state)` and merge
   the resulting `axum::Router` into the gateway's main router under
   the same path prefix the descriptors declare.
2. **Bearer-token middleware.** Authenticate `Authorization: Bearer
   …` (or the matching session form) and inject a
   `WebUiAuthenticatedCaller` and request-scoped
   `WebUiV2Capabilities` as `axum::Extension`s *before* the handler
   runs. The handlers fail closed (`500`) when this layer is
   missing — verified by
   `missing_caller_extension_returns_500`.
3. **Query-token path for the SSE route.** The browser's
   `EventSource` cannot set request headers, so
   `/api/webchat/v2/threads/{thread_id}/events` must additionally
   accept `?token=…` (the existing WebUI v1 gateway allowlists
   `/api/chat/events`, `/api/logs/events`, `/api/chat/ws` for the
   same reason — see `src/channels/web/CLAUDE.md`). The route
   descriptor is bearer-only at the protocol layer; the gateway's
   query-token handler converts `?token=` to the same bearer-style
   identity before this crate's handler sees the request.
4. **Static security headers + CORS.** Declared at the descriptor
   policy level (`CorsPolicy::SameOriginOnly`) but enforced in the
   gateway's middleware stack.

Until those four steps land, the routes here compile and lock the
contract host composition will mount against, but they are not yet
browser-reachable.

## Route table

| Route ID | Method | Pattern | Streaming | Effect path |
|---|---|---|---|---|
| `webui.v2.get_session` | GET | `/api/webchat/v2/session` | None | `ProjectionOnly` |
| `webui.v2.create_thread` | POST | `/api/webchat/v2/threads` | None | `ProductWorkflow` |
| `webui.v2.list_threads` | GET | `/api/webchat/v2/threads` (optional `?limit=N&cursor=...`) | None | `ProjectionOnly` |
| `webui.v2.delete_thread` | DELETE | `/api/webchat/v2/threads/{thread_id}` | None | `ProductWorkflow` |
| `webui.v2.send_message` | POST | `/api/webchat/v2/threads/{thread_id}/messages` | None | `TurnCoordinator` |
| `webui.v2.get_timeline` | GET | `/api/webchat/v2/threads/{thread_id}/timeline` (optional `?limit=N&cursor=...`) | None | `ProjectionOnly` |
| `webui.v2.stream_events` | GET | `/api/webchat/v2/threads/{thread_id}/events` | SSE | `ProjectionOnly` |
| `webui.v2.stream_events_ws` | GET | `/api/webchat/v2/threads/{thread_id}/ws` | WebSocket | `ProjectionOnly` |
| `webui.v2.cancel_run` | POST | `/api/webchat/v2/threads/{thread_id}/runs/{run_id}/cancel` | None | `TurnCoordinator` |
| `webui.v2.resolve_gate` | POST | `/api/webchat/v2/threads/{thread_id}/runs/{run_id}/gates/{gate_ref}/resolve` | None | `TurnCoordinator` |
| `webui.v2.list_automations` | GET | `/api/webchat/v2/automations` (optional `?limit=N&run_limit=N`) | None | `ProductWorkflow` |
| `webui.v2.list_connectable_channels` | GET | `/api/webchat/v2/channels/connectable` | None | `ProjectionOnly` |
| `webui.v2.list_extensions` | GET | `/api/webchat/v2/extensions` | None | `ProjectionOnly` |
| `webui.v2.list_extension_registry` | GET | `/api/webchat/v2/extensions/registry` | None | `ProjectionOnly` |
| `webui.v2.install_extension` | POST | `/api/webchat/v2/extensions/install` | None | `ProductWorkflow` |
| `webui.v2.activate_extension` | POST | `/api/webchat/v2/extensions/{package_id}/activate` | None | `ProductWorkflow` |
| `webui.v2.remove_extension` | POST | `/api/webchat/v2/extensions/{package_id}/remove` | None | `ProductWorkflow` |
| `webui.v2.get_extension_setup` | GET | `/api/webchat/v2/extensions/{package_id}/setup` | None | `ProjectionOnly` |
| `webui.v2.setup_extension` | POST | `/api/webchat/v2/extensions/{package_id}/setup` | None | `ProductWorkflow` |
| `webui.v2.get_llm_config` | GET | `/api/webchat/v2/llm/providers` | None | `ProjectionOnly` |
| `webui.v2.upsert_llm_provider` | POST | `/api/webchat/v2/llm/providers` | None | `ProductWorkflow` |
| `webui.v2.delete_llm_provider` | POST | `/api/webchat/v2/llm/providers/{provider_id}/delete` | None | `ProductWorkflow` |
| `webui.v2.set_active_llm` | POST | `/api/webchat/v2/llm/active` | None | `ProductWorkflow` |
| `webui.v2.test_llm_connection` | POST | `/api/webchat/v2/llm/test-connection` | None | `ProductWorkflow` |
| `webui.v2.list_llm_models` | POST | `/api/webchat/v2/llm/list-models` | None | `ProductWorkflow` |
| `webui.v2.operator.get_setup` | GET | `/api/webchat/v2/operator/setup` | None | `ProjectionOnly` |
| `webui.v2.operator.run_setup` | POST | `/api/webchat/v2/operator/setup` | None | `ProductWorkflow` |
| `webui.v2.operator.list_config` | GET | `/api/webchat/v2/operator/config` | None | `ProjectionOnly` |
| `webui.v2.operator.get_config_key` | GET | `/api/webchat/v2/operator/config/{key}` | None | `ProjectionOnly` |
| `webui.v2.operator.set_config_key` | POST | `/api/webchat/v2/operator/config/{key}` | None | `ProductWorkflow` |
| `webui.v2.operator.validate_config` | POST | `/api/webchat/v2/operator/config/validate` | None | `ProductWorkflow` |
| `webui.v2.operator.diagnostics` | GET | `/api/webchat/v2/operator/diagnostics` | None | `ProjectionOnly` |
| `webui.v2.operator.status` | GET | `/api/webchat/v2/operator/status` | None | `ProjectionOnly` |
| `webui.v2.operator.logs` | GET | `/api/webchat/v2/operator/logs` | None | `ProjectionOnly` |
| `webui.v2.operator.service_lifecycle` | POST | `/api/webchat/v2/operator/service` | None | `ProductWorkflow` |

All routes require `BearerToken` auth with `AuthenticatedCaller`
scope source. The host's bearer middleware is responsible for
constructing the `WebUiAuthenticatedCaller`, carrying the matched
token's `WebUiV2Capabilities`, and injecting both as axum
`Extension`s before the handler runs.

The LLM configuration and operator command-plane routes are operator-wide. Host
composition mounts them only when the authenticator says the deployment
has an operator configuration surface, and must still authorize each
request from the matched token's `operator_webui_config` capability.
Multi-user session/OIDC authenticators should leave those routes
unmounted or return non-operator capabilities until an admin role
boundary exists. The route handlers also reject mounted operator
requests with `403` when the injected `WebUiV2Capabilities` lacks
`operator_webui_config`, so host composition and handler dispatch share
the same fail-closed capability boundary.
Unwired operator command-plane write, setup, log, and
service-control methods fail closed with sanitized `503 service_unavailable`
responses. Config validation plus read-only config, status, and diagnostics
surfaces may instead return unavailable command-plane payloads with redacted
diagnostics so operators can see why a setting is ignored. Stable
unsupported-config reason codes currently include
`operator_config_service_not_wired`, `operator_config_secret_not_wired`,
`operator_config_deprecated`, `operator_config_immutable`,
`operator_config_not_wired`, and `operator_config_unknown_key`.
`POST /api/webchat/v2/operator/setup` uses the typed LLM config service
for provider/model setup; profile and WebUI access setup return redacted
not-yet-wired diagnostics until those owning services are exposed.

### List-threads

`list_threads` is the v2 native counterpart to v1's
`GET /api/chat/threads`. The facade scopes the enumeration to the
caller's `(tenant, agent, project, owner_user_id)` triple — never
the body, never a query parameter — so a caller cannot enumerate
threads owned by other users in the same `(tenant, agent, project)`
triple. Pagination uses the same `?limit=N&cursor=...` shape as
`get_timeline`.

The underlying backend port is
`SessionThreadService::list_threads_for_scope`. The trait's default
impl returns `SessionThreadError::Backend(...)` — backends that do
not implement enumeration surface a retryable
`service_unavailable` (HTTP 503) at the gateway rather than
silently returning an empty list. The contract is locked by
`list_threads_unimplemented_backend_returns_service_unavailable` in
`crates/ironclaw_product_workflow/tests/reborn_services_contract.rs`.

### Delete-thread

`delete_thread` removes a caller-owned thread and transcript via
`SessionThreadService::delete_thread`. The facade constructs the same
owner-bound `(tenant, agent, project, owner_user_id)` scope used by timeline,
stream, cancel, and gate-resolution probes. Missing and cross-owner thread ids
both surface as `404 not_found` so callers cannot use deletion attempts as an
existence oracle.

### Stream-events (WebSocket)

`stream_events_ws` is the WebSocket transport variant of
`stream_events`. It drains the same `RebornServicesApi::stream_events`
facade and emits each `ProductOutboundEnvelope` as a JSON text frame.
The descriptor declares
`WebSocketOriginPolicy::SameOriginRequired`; host composition runs
the same-origin check before the upgrade reaches this crate's
handler.

The same `(tenant, user)` `SseCapacity` pool gates both transports —
WS and SSE share one budget. A caller cannot bypass the cap by
opening `cap` SSE streams *and* `cap` WS streams in parallel. The
pre-upgrade `try_acquire` returns `429 rate_limited` if the budget
is exhausted; the regression is locked by
`stream_events_ws_shares_capacity_with_sse_streams`.

Every `socket.send` is bounded by the remaining
`SSE_MAX_LIFETIME` budget via `ws_send_with_timeout`, so a TCP-
backpressuring client cannot pin the slot past the configured
stream lifetime.

### Setup-extension lifecycle projection

`setup_extension` is the v2 entrypoint for extension onboarding.
The native facade exposes the route surface as a lifecycle
projection: responses carry `phase`, `blockers`, optional
payload, and the lifecycle `package_ref`. Auth, pairing, approval,
policy, credential, and runtime requirements must be represented
as blockers owned by their dedicated services, not as legacy
setup status aliases or lifecycle phases. The route still does
not perform production setup/configure/activate side effects.

Extension lifecycle side effects use `LifecyclePackageRef`
end-to-end. Install accepts a JSON `package_ref` body; activate,
remove, and setup lift `{package_id}` path segments into
`LifecyclePackageRef { kind: extension, id: ... }` at the
handler/facade boundary. A malformed package id returns
`400 invalid_request` with `field: "package_id"` and
`validation_code: "invalid_id"` before the facade is called.
Browsers should render registry `display_name` for users and send
`package_ref` for lifecycle operations.

## Boundary rules

Handlers must consume only `RebornServicesApi`. They must NOT depend on
`ironclaw_dispatcher`, `ironclaw_extensions`, `ironclaw_host_runtime`,
`ironclaw_mcp`, `ironclaw_wasm`, `ironclaw_scripts`, `ironclaw_network`,
`ironclaw_engine`, `ironclaw_gateway`, `ironclaw_run_state`,
`ironclaw_capabilities`, or any DB/storage crate. The architecture
boundary test enforces this.

## Streaming model

`stream_events` is SSE. The facade is drain-only right now, so the
handler drains, renders each `ProductOutboundEnvelope` into the
browser-visible `WebChatV2EventFrame` schema with its projection cursor
as the SSE `id`, then polls again on a 1-second cadence. The frame
intentionally excludes adapter routing/delivery metadata. When
`RebornServicesApi::stream_events` gains a true subscription API the
handler can migrate without changing the descriptor or browser event
schema.

The per-poll ownership probe goes through `SessionThreadService::read_thread`
(metadata-only) rather than `list_thread_history`, so an active stream does
not reload the full message transcript every second.

`capability_activity` SSE frames are projection-derived lifecycle metadata for
tool/capability execution. They expose only the safe activity DTO
(`invocation_id`, `capability_id`, status, provider/runtime/process metadata,
byte counts, sanitized error kind, timestamp) and must not carry raw tool
arguments, raw results, command strings, host paths, or provider payloads.

`capability_display_preview` SSE frames are separate sanitized display artifacts
for WebUI tool blocks. They may carry bounded summaries/previews only: summaries
are capped at 2 KiB and output previews at 16 KiB. They are not source-of-truth
tool results. Full output remains behind the
scoped `result_ref` fetch path; SSE must never carry raw unbounded args/results.
Preview generation belongs in the Reborn product/composition layer, using
staged input/result-ref or transcript evidence where available, not in low-level
capability ports.

Snapshot/replay drains bound activity fan-out per projection item so every
emitted SSE cursor remains resumable through `Last-Event-ID`; when the folded
activity set is larger than the bound, the stream splits the overflow across
resumable projection cursors. Partial cursors carry the runtime item watermark
and delivered payload count, so reconnect drains continue from the same folded
item when it is stable and restart that item when the folded head changes.

The browser resumes via `Last-Event-ID` on auto-reconnect; the handler
prefers that header over the `?after_cursor=` query parameter, falling
back to the projection origin when neither is supplied.

## Timeline pagination

`get_timeline` accepts two optional query parameters:

- `limit=N` — maximum number of messages returned in one response. The
  facade clamps to `[1, 200]` so a caller cannot widen the response by
  passing a huge value.
- `cursor=<opaque>` — round-tripped value from the previous response's
  `next_cursor`. The browser does not interpret the cursor; it just
  echoes it back to load the page preceding the current one.

The response carries `next_cursor: Option<String>`. `None` means the
caller has reached the start of the thread and there are no older
pages.

### SSE resource caps

Two ceilings sit in front of `stream_events`, on top of the route
descriptor's per-caller request rate limit:

- **Per-caller concurrency cap** — `WebUiV2State` carries an
  `SseCapacity` keyed by `(tenant, user)`. New opens beyond the cap
  return `429 Too Many Requests` with `retryable: true`. The default
  cap is 3 streams per `(tenant, user)`; host composition can override
  via `WebUiV2State::with_sse_concurrency_limit`.
- **Max stream lifetime** — every stream is closed after 5 minutes so
  the browser must reconnect with `Last-Event-ID`. Bounds cursor drift
  and recycles slots even under leaked client connections. The drain
  await is wrapped in `tokio::time::timeout(remaining, ...)` so a
  stuck/never-resolving facade `stream_events` call cannot pin the
  slot past the budget — covered by
  `stream_events_releases_slot_when_facade_drain_stalls_past_max_lifetime`.

Slots are RAII: the SSE generator owns an `SseSlot` guard that
decrements the per-caller count on drop, so a client disconnect,
lifetime expiry, or facade error all release the slot automatically.

## Test support

- `tests/webui_v2_descriptors_contract.rs` — locks the descriptor table
  (count / methods / patterns / auth / rate limits / SSE).
- `tests/webui_v2_handlers_contract.rs` — drives a real axum router
  built from `webui_v2_router` against a stub `RebornServicesApi`, per
  `.claude/rules/testing.md` "Test Through the Caller".

## Validation

```bash
cargo test -p ironclaw_webui_v2 --features webui-v2-beta
cargo clippy -p ironclaw_webui_v2 --all-features --tests -- -D warnings
cargo test -p ironclaw_architecture reborn_crate_dependency_boundaries_hold
```
