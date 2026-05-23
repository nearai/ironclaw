# ironclaw_reborn_composition guardrails

- Own only top-level Reborn composition for production/app startup.
- Expose facade-shaped handles only: `HostRuntime`, `TurnCoordinator`, product-auth `RebornProductAuthServices`, WebUI `RebornServicesApi`, readiness.
- Keep lower substrate handles private to factories and owning crates.
- Do not depend on the root `ironclaw` crate or `src/` modules.
- Do not add legacy bridge modes here until an accepted migration contract exists.
- Do not route live v1/product traffic here; callers must opt in through explicit Reborn adapters.
- Production and migration-dry-run profiles must fail closed on local-only or missing required handles.
- Product auth composition must use `ironclaw_auth` trait-shaped ports. Do not
  wire product auth through V1 OAuth routes, V1 pending maps, V1
  `ExtensionManager`, V1 secret stores, or route-local raw HTTP clients.
- OAuth callback routes should only parse/validate HTTP input and call
  `RebornProductAuthServices::handle_oauth_callback`; the handler must claim
  the scoped flow/state/provider through `AuthFlowManager` before exchanging
  provider material through `AuthProviderClient`, then complete the flow and
  emit typed continuations.
- Blocked run-state approval/auth gate rendering and resume belongs to #3094;
  keep this crate's #3811 auth seam reusable by that layer without implementing
  a second gate-resolution path.

## WebUI v2 native surface (`webui-v2-beta` feature)

The Reborn-side host composition for the WebChat v2 HTTP gateway lives
in this crate. Implements Path A of
`docs/reborn/how-to-port-channel-to-reborn.md` (native host-owned
surface entering `ProductWorkflow` directly) without sharing any
middleware with v1's `src/channels/web/`.

### Surface

| Symbol | Role |
|---|---|
| `RebornWebuiBundle` (in [`src/webui.rs`](src/webui.rs)) | `{ api: Arc<dyn RebornServicesApi>, readiness }` — the v2 facade plus readiness snapshot |
| `build_webui_services(runtime, event_stream)` | Compose a `RebornWebuiBundle` from an already-built `RebornRuntime`; reuses the runtime's thread service / turn coordinator and attaches the runtime-owned `EventStreamManager` projection stream unless a caller supplies a custom stream |
| `RebornProjectionServices` (in `src/projection.rs`) | Runtime-owned projection/event-stream composition; owns the single local-dev `EventStreamManager` and creates product-specific `ProjectionStream` adapters over it |
| `WebuiAuthenticator` trait | Host-supplied bearer-token verifier; returns `Option<UserId>` |
| `WebuiServeConfig { tenant_id, authenticator, max_body_bytes, allowed_origins, csp_header }` | Required config for `webui_v2_app`; no defaults that silently disable security |
| `webui_v2_app(bundle, config) -> Router` | Build the fully-composed axum `Router`. This is the seam between this product/API crate and host-owned HTTP ingress: tests drive it via `tower::ServiceExt::oneshot`; the `ironclaw-reborn serve` subcommand (follow-up PR) hands it to `axum::serve` from a host-owned listener |

### Middleware stack composed by `webui_v2_app`

Inbound order (outer → inner → handler):

1. `SetResponseHeaderLayer` — static security headers
   (`X-Content-Type-Options: nosniff`, `X-Frame-Options: DENY`, CSP).
2. `CorsLayer` — allow-origin from `config.allowed_origins`; empty list
   means fail-closed (no echoing attacker-supplied origin).
3. `CatchPanicLayer` — panic boundary, logs truncated detail.
4. **Outer `RequestBodyLimitLayer`** — `config.max_body_bytes` (14 MiB
   default). Defense in depth for paths that don't match any v2
   descriptor (e.g. axum's 404 fallback). v2 routes are additionally
   capped, strictly tighter, by the per-route limit below.
5. **Descriptor-driven per-route body limit**
   (`webui_body_limit::enforce_body_limit`) — reads each route's
   `BodyLimitPolicy` from `ironclaw_webui_v2::webui_v2_routes()` at
   composition time and enforces it before auth runs (so an oversized
   payload never spends a bearer-validation step). Today:
   `create_thread` 16 KiB, `send_message` 1 MiB, `cancel_run` and
   `resolve_gate` 4 KiB, `get_timeline` and `stream_events` `NoBody`.
   `BodyLimitPolicy` is an exhaustive `match`, so a new variant added
   upstream fails the build rather than silently disabling
   enforcement.
6. **WS same-origin enforcement** (`webui_ws_origin::enforce_websocket_origin`)
   — runs only on descriptors that declare a non-`NotApplicable`
   `WebSocketOriginPolicy`. The browser does not pre-flight WebSocket
   upgrades, so origin enforcement happens inline; absence or mismatch
   yields a `403` before the v2 handler executes the WS upgrade.
   `SameOriginRequired` (today's `stream_events_ws` descriptor)
   matches `Origin` against `Host`; `HostConfiguredAllowlist` /
   `LocalhostAllowed` are additional shapes future descriptors can
   opt into.
7. **Bearer auth + `?token=` shim** (`webui_serve::authenticate_request`)
   — `Authorization: Bearer <token>` for every route; `?token=` is
   honored ONLY on `GET /api/webchat/v2/threads/{id}/events` because
   the browser's `EventSource` cannot set headers. Mutations and
   timeline reads stay bearer-only. On success the middleware inserts
   a `WebUiAuthenticatedCaller` extension built from
   `config.tenant_id` plus the authenticator's `UserId`.
8. **Descriptor-driven per-route rate limit**
   (`webui_rate_limit::enforce_rate_limit`) — reads
   `ironclaw_webui_v2::webui_v2_routes()` at composition time and
   enforces the declared `RateLimitPolicy` per `(route, caller)` with a
   sliding window. Today every v2 descriptor declares
   `RateLimitScope::PerCaller`; composition fails closed if a future
   descriptor declares an unsupported scope.
9. `webui_v2_router(WebUiV2State::new(bundle.api))` — the nine v2
   handlers from `ironclaw_webui_v2` (create-thread, list-threads,
   send-message, get-timeline, stream-events SSE, stream-events WS,
   cancel-run, resolve-gate, setup-extension).

`webui_route_match` is the shared matcher both the body-limit and
rate-limit middlewares consume so the two enforcers cannot drift on
which request belongs to which descriptor.

### Entrypoint inventory (#3580)

Mapping of every v1 gateway entrypoint to its Reborn native-surface
counterpart. "v1-only" means the v2 facade does not yet expose this
shape and a future native-surface ticket owns the migration — these
rows are inventoried here, not implemented in the current PR.

| Concern | v1 entrypoint (under `src/channels/web/`) | v2 native counterpart | Status |
|---|---|---|---|
| Send message | `POST /api/chat/send` | `POST /api/webchat/v2/threads/{thread_id}/messages` | Mapped |
| Create thread | `POST /api/chat/thread/new` | `POST /api/webchat/v2/threads` | Mapped |
| List threads | `GET /api/chat/threads` | `GET /api/webchat/v2/threads` | Mapped |
| Read history / timeline | `GET /api/chat/history` | `GET /api/webchat/v2/threads/{thread_id}/timeline` | Mapped |
| SSE stream | `GET /api/chat/events` | `GET /api/webchat/v2/threads/{thread_id}/events` | Mapped (incl. `?token=` shim) |
| WebSocket stream | `GET /api/chat/ws` | `GET /api/webchat/v2/threads/{tid}/ws` | Mapped |
| Cancel run | (engine v1 surface) | `POST /api/webchat/v2/threads/{tid}/runs/{run_id}/cancel` | Mapped |
| Resolve gate | `POST /api/chat/gate/resolve` | `POST /api/webchat/v2/threads/{tid}/runs/{run_id}/gates/{gate_ref}/resolve` | Mapped |
| Approval shim | `POST /api/chat/approval` | (Subsumed by `resolve_gate`) | Mapped |
| Auth-token / auth-cancel | `POST /api/chat/auth-{token,cancel}` | (Engine v1 compatibility shim; delete with v1) | v1-only (legacy) |
| Extensions onboarding | `GET\|POST /api/extensions/{name}/setup` | `POST /api/webchat/v2/extensions/{name}/setup` | Mapped (skeleton — facade returns `NotImplemented` until v2-aware extension lifecycle lands) |

### Security invariants on every "Mapped" row

- **Bearer / OIDC / cookie auth** — none of these are shared with v1's
  `auth_middleware`. The Reborn binary owns its own
  `WebuiAuthenticator` impl (env tokens, DB-backed sessions, OIDC,
  whatever the host wires) and supplies it via `WebuiServeConfig`.
- **`?token=` exception** — only `GET /api/webchat/v2/threads/{id}/events`;
  any other v2 route receiving a `?token=` query parameter ignores it
  and falls through to bearer-header check (so a stale referer link
  cannot authenticate a state change).
- **CORS** — `CorsLayer` allow-origin = `config.allowed_origins`. The
  Reborn `serve` subcommand should set this to the bound listener's
  same-origin URL set; an empty allowlist rejects every cross-origin
  preflight.
- **Body limit** — descriptor-driven per-route via
  `webui_body_limit::enforce_body_limit`. Caps come from
  `ironclaw_webui_v2::webui_v2_routes()`: `create_thread` 16 KiB,
  `send_message` 1 MiB, `cancel_run` / `resolve_gate` 4 KiB,
  `get_timeline` / `stream_events` `NoBody`. The outer
  `RequestBodyLimitLayer` at `config.max_body_bytes` (14 MiB default)
  is kept as defense in depth for paths that don't match any v2
  descriptor.
- **Rate limit** — descriptor-driven; the v2 crate declares mutation
  60/60, read 120/60, stream 12/60 per `(tenant, user)`. Reading and
  enforcing happens in `webui_rate_limit::build_rate_limit_state`.
- **Static security headers** — `nosniff`, `DENY`, CSP applied via
  outer `SetResponseHeaderLayer`s; default CSP is
  `default-src 'self'; object-src 'none'; frame-ancestors 'none';
  base-uri 'self'`.
- **Connection limit (SSE)** — bounded by `ironclaw_webui_v2`'s own
  `SseCapacity` (3 streams per `(tenant, user)`, 5-minute max stream
  lifetime). No WS surface to bound.
- **Caller construction** — `WebUiAuthenticatedCaller` is built from
  `config.tenant_id` (trusted host installation) plus the
  authenticator's verified `UserId`. The browser body cannot influence
  either field; matches the rule in
  `crates/ironclaw_product_workflow/CLAUDE.md`.

### What this composition deliberately does NOT do

Per Path A in `docs/reborn/how-to-port-channel-to-reborn.md`:

- No `ProductAdapter` wrapper around browser sessions.
- No fake `ExternalActorRef` / `ProtocolAuthEvidence` /
  `OutboundDeliverySink` / declared egress.
- No shared middleware with v1's `src/channels/web/` —
  `feat/webui-v2-gateway-composition-3580` deliberately keeps the v1
  binary untouched.

### How the standalone `ironclaw-reborn serve` consumes this

The `serve` subcommand builds a full local-dev `RebornRuntime`, asks
`build_webui_services(&runtime, None)` for the WebUI bundle, and hands
the resulting router to the host-owned `ironclaw_reborn_webui_ingress`
listener lifecycle. The bundle's default projection stream is backed by
the runtime's process-local in-memory durable event log plus
`EventStreamManager`, so `/events` and `/ws` no longer advertise routes
that only return `Unavailable`. That log is intentionally local-dev
scaffolding for this slice; production durable retention/live fanout
belongs in the host runtime/event-store follow-up rather than this
composition facade.

```rust
// Inside a host-owned ingress crate / binary (NOT in this crate —
// `reborn_product_api_crates_do_not_bind_http_ingress` forbids
// product/API crates from owning server lifecycle).
let runtime = build_reborn_runtime(input).await?;
let bundle = build_webui_services(&runtime, None)?;
let config = WebuiServeConfig::new(
    TenantId::new(host_installation_tenant)?,
    Arc::new(MyHostAuthenticator::new(...)),
    same_origin_allowlist(bound_addr),
);
let app = webui_v2_app(bundle, config)?;
let listener = tokio::net::TcpListener::bind(addr).await?;
axum::serve(listener, app).with_graceful_shutdown(shutdown).await?;
```

### Tests

- `src/runtime.rs::tests::local_dev_runtime_webui_bundle_reuses_thread_and_turn_facades`
  — regression guard that the WebUI bundle reuses the runtime turn/thread
  facades.
- `src/projection.rs::tests::webui_event_stream_drains_run_status_projection_from_event_stream_manager`
  — regression guard that the WebUI projection stream drains the current
  run-status projection slice from a real `EventStreamManager` snapshot
  into product outbound envelopes.
- `src/projection.rs::tests::webui_event_stream_uses_request_actor_for_projection_scope`
  — regression guard that the WebUI projection adapter uses the facade
  request actor when selecting the runtime event stream, rather than a
  hidden runtime owner actor.
- `tests/webui_v2_serve.rs` — caller-level tests driving the composed
  `Router` through `tower::ServiceExt::oneshot`: bearer happy path,
  missing/invalid bearer 401, SSE `?token=`, timeline rejects `?token=`,
  security headers, CORS allow + reject, malformed-id rejection,
  rate-limit 429 after descriptor budget exhausted, per-caller
  rate-limit independence, descriptor-driven body-limit 413 on
  oversized mutation payload, in-budget mutation reaches facade, and
  NoBody policy rejecting a non-empty body on a read route.
- `src/webui_serve.rs::tests` — unit tests for `is_v2_sse_event_request`
  matcher and query-token extraction.
- `src/webui_route_match.rs::tests` — unit tests for the pattern
  parser and segment matcher shared by both descriptor-driven
  middlewares.
- `src/webui_rate_limit.rs::tests` — unit tests for the sliding-window
  policy resolver, a regression test that `build_rate_limit_state`
  accepts every descriptor returned by
  `ironclaw_webui_v2::webui_v2_routes()`, and
  `unsupported_scope_is_rejected_at_composition` locking the
  fail-closed branch for non-`PerCaller` scopes.
- `src/webui_body_limit.rs::tests` — composition-time tests that
  `build_body_limit_state` accepts every v2 descriptor and preserves
  the per-route caps (regression guard against silently widening the
  `send_message` cap or relaxing a `NoBody` policy).
