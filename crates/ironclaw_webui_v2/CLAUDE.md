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
   `WebUiAuthenticatedCaller` as an `axum::Extension` *before* the
   handler runs. The handlers fail closed (`500`) when this layer is
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
| `webui.v2.create_thread` | POST | `/api/webchat/v2/threads` | None | `ProductWorkflow` |
| `webui.v2.send_message` | POST | `/api/webchat/v2/threads/{thread_id}/messages` | None | `TurnCoordinator` |
| `webui.v2.get_timeline` | GET | `/api/webchat/v2/threads/{thread_id}/timeline` | None | `ProjectionOnly` |
| `webui.v2.stream_events` | GET | `/api/webchat/v2/threads/{thread_id}/events` | SSE | `ProjectionOnly` |
| `webui.v2.cancel_run` | POST | `/api/webchat/v2/threads/{thread_id}/runs/{run_id}/cancel` | None | `TurnCoordinator` |
| `webui.v2.resolve_gate` | POST | `/api/webchat/v2/threads/{thread_id}/runs/{run_id}/gates/{gate_ref}/resolve` | None | `TurnCoordinator` |

All six routes require `BearerToken` auth with `AuthenticatedCaller`
scope source. The host's bearer middleware is responsible for
constructing the `WebUiAuthenticatedCaller` and injecting it as an
axum `Extension` before the handler runs.

## Boundary rules

Handlers must consume only `RebornServicesApi`. They must NOT depend on
`ironclaw_dispatcher`, `ironclaw_extensions`, `ironclaw_host_runtime`,
`ironclaw_mcp`, `ironclaw_wasm`, `ironclaw_scripts`, `ironclaw_network`,
`ironclaw_engine`, `ironclaw_gateway`, `ironclaw_run_state`,
`ironclaw_capabilities`, or any DB/storage crate. The architecture
boundary test enforces this.

## Streaming model

`stream_events` is SSE. The facade is drain-only right now, so the
handler drains, emits each event with its projection cursor as the SSE
`id`, then polls again on a 1-second cadence. When
`RebornServicesApi::stream_events` gains a true subscription API the
handler can migrate without changing the descriptor.

The browser resumes via `Last-Event-ID` on auto-reconnect; the handler
prefers that header over the `?after_cursor=` query parameter, falling
back to the projection origin when neither is supplied.

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
