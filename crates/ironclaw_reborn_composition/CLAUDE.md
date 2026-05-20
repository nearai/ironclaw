# ironclaw_reborn_composition guardrails

- Own only top-level Reborn composition for production/app startup.
- Expose facade-shaped handles only: `HostRuntime`, `TurnCoordinator`, WebUI `RebornServicesApi`, readiness.
- Keep lower substrate handles private to factories and owning crates.
- Do not depend on the root `ironclaw` crate or `src/` modules.
- Do not add legacy bridge modes here until an accepted migration contract exists.
- Do not route live v1/product traffic here; callers must opt in through explicit Reborn adapters.
- Production and migration-dry-run profiles must fail closed on local-only or missing required handles.

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
| `build_webui_services(runtime, event_stream)` | Compose a `RebornWebuiBundle` from an already-built `RebornRuntime`; reuses the runtime's thread service / turn coordinator (no second turn loop) |
| `WebuiAuthenticator` trait | Host-supplied bearer-token verifier; returns `Option<UserId>` |
| `WebuiServeConfig { tenant_id, authenticator, max_body_bytes, allowed_origins, csp_header }` | Required config for `webui_v2_app` / `serve_webui_v2`; no defaults that silently disable security |
| `webui_v2_app(bundle, config) -> Router` | Build the fully-composed axum `Router`; useful for tests and any future ingress that wants its own listener loop |
| `serve_webui_v2(listener, bundle, config, shutdown)` | Bind the listener and `axum::serve` until shutdown resolves; the `ironclaw-reborn serve` subcommand calls this |

### Middleware stack composed by `webui_v2_app`

Inbound order (outer → inner → handler):

1. `SetResponseHeaderLayer` — static security headers
   (`X-Content-Type-Options: nosniff`, `X-Frame-Options: DENY`, CSP).
2. `CorsLayer` — allow-origin from `config.allowed_origins`; empty list
   means fail-closed (no echoing attacker-supplied origin).
3. `CatchPanicLayer` — panic boundary, logs truncated detail.
4. `RequestBodyLimitLayer` — `config.max_body_bytes` (14 MiB default).
5. **Bearer auth + `?token=` shim** (`webui_serve::authenticate_request`)
   — `Authorization: Bearer <token>` for every route; `?token=` is
   honored ONLY on `GET /api/webchat/v2/threads/{id}/events` because
   the browser's `EventSource` cannot set headers. Mutations and
   timeline reads stay bearer-only. On success the middleware inserts
   a `WebUiAuthenticatedCaller` extension built from
   `config.tenant_id` plus the authenticator's `UserId`.
6. **Descriptor-driven per-route rate limit**
   (`webui_rate_limit::enforce_rate_limit`) — reads
   `ironclaw_webui_v2::webui_v2_routes()` at composition time and
   enforces the declared `RateLimitPolicy` per `(route, caller)` with a
   sliding window. Today every v2 descriptor declares
   `RateLimitScope::PerCaller`; composition fails closed if a future
   descriptor declares an unsupported scope.
7. `webui_v2_router(WebUiV2State::new(bundle.api))` — the six v2
   handlers from `ironclaw_webui_v2`.

### Entrypoint inventory (#3580)

Mapping of every v1 gateway entrypoint to its Reborn native-surface
counterpart. "v1-only" means the v2 facade does not yet expose this
shape and a future native-surface ticket owns the migration — these
rows are inventoried here, not implemented in the current PR.

| Concern | v1 entrypoint (under `src/channels/web/`) | v2 native counterpart | Status |
|---|---|---|---|
| Send message | `POST /api/chat/send` | `POST /api/webchat/v2/threads/{thread_id}/messages` | Mapped |
| Create thread | `POST /api/chat/thread/new` | `POST /api/webchat/v2/threads` | Mapped |
| List threads | `GET /api/chat/threads` | (No v2 collection route; future ticket) | v1-only |
| Read history / timeline | `GET /api/chat/history` | `GET /api/webchat/v2/threads/{thread_id}/timeline` | Mapped |
| SSE stream | `GET /api/chat/events` | `GET /api/webchat/v2/threads/{thread_id}/events` | Mapped (incl. `?token=` shim) |
| WebSocket stream | `GET /api/chat/ws` | (`RebornServicesApi` exposes SSE only; no v2 WS) | v1-only |
| Cancel run | (engine v1 surface) | `POST /api/webchat/v2/threads/{tid}/runs/{run_id}/cancel` | Mapped |
| Resolve gate | `POST /api/chat/gate/resolve` | `POST /api/webchat/v2/threads/{tid}/runs/{run_id}/gates/{gate_ref}/resolve` | Mapped |
| Approval shim | `POST /api/chat/approval` | (Subsumed by `resolve_gate`) | Mapped |
| Auth-token / auth-cancel | `POST /api/chat/auth-{token,cancel}` | (Engine v1 compatibility shim; delete with v1) | v1-only (legacy) |
| Extensions onboarding | `GET\|POST /api/extensions/{name}/setup` | (No v2 onboarding route in `RebornServicesApi`) | v1-only |

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
- **Body limit** — `RequestBodyLimitLayer` at `config.max_body_bytes`
  (14 MiB default).
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

The `serve` subcommand on `feat/reborn-cli-serve` currently bails with
"composition not linked yet". The intended completion is one call
site:

```rust
// Inside `crates/ironclaw_reborn_cli/src/commands/serve.rs`
let runtime = build_reborn_runtime(input).await?;
let bundle = build_webui_services(&runtime, None)?;
let config = WebuiServeConfig::new(
    TenantId::new(host_installation_tenant)?,
    Arc::new(MyHostAuthenticator::new(...)),
    same_origin_allowlist(bound_addr),
);
let listener = tokio::net::TcpListener::bind(addr).await?;
serve_webui_v2(listener, bundle, config, shutdown).await?;
```

Until that PR lands, `webui_v2_app` is reachable through tests
(`crates/ironclaw_reborn_composition/tests/webui_v2_serve.rs`) and by
any other Reborn ingress crate that wants to mount the same routes
under a different listener.

### Tests

- `tests/webui_v2_serve.rs` — 12 caller-level tests driving the
  composed `Router` through `tower::ServiceExt::oneshot`: bearer
  happy path, missing/invalid bearer 401, SSE `?token=`, timeline
  rejects `?token=`, security headers, CORS allow + reject,
  malformed-id rejection, rate-limit 429 after descriptor budget
  exhausted, per-caller rate-limit independence.
- `src/webui_serve.rs::tests` — unit tests for `is_v2_sse_event_request`
  matcher and query-token extraction.
- `src/webui_rate_limit.rs::tests` — unit tests for the sliding-window
  policy resolver and pattern matcher, plus a regression test that
  `build_rate_limit_state` accepts every descriptor returned by
  `ironclaw_webui_v2::webui_v2_routes()`.
