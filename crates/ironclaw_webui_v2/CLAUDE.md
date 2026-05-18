# ironclaw_webui_v2

Reborn WebChat v2 HTTP route surface (#3611). Off by default — compile in
with the `webui-v2-beta` Cargo feature.

## Purpose

Owns the minimal native WebUI v2 route set on top of
`ironclaw_product_workflow::RebornServicesApi`. Handlers are the only
public surface; host composition consumes the
`IngressRouteDescriptor`s returned by `webui_v2_routes()` and mounts
each handler under the matching pattern after running its own bearer
auth, CORS, body-limit, and rate-limit middleware.

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
  and recycles slots even under leaked client connections.

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
