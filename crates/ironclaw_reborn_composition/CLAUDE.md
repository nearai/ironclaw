# ironclaw_reborn_composition guardrails

- Own only top-level Reborn composition for production/app startup.
- Expose facade-shaped handles only: `HostRuntime`, `TurnCoordinator`, product-auth `RebornProductAuthServices`, WebUI `RebornServicesApi`, readiness.
- Keep lower substrate handles private to factories and owning crates.
- Substrate handles MAY be exposed via `#[cfg(any(test, feature = "test-support"))]` pub accessors on `RebornRuntime` when downstream integration tests need to drive production-shape state the facade doesn't yet surface (e.g. seeding `TriggerRecord` rows, `pair_external_actor` calls). These seams ship zero bytes in production binaries. New test-support accessors must carry a doc-comment naming the production call site they mirror and an explicit note that the handle is for tests only.
- Outbound state stores are composition-owned via `RebornLocalRuntimeServices`; do not construct `FilesystemOutboundStateStore` in consumer modules (lint-enforced via `clippy::disallowed-methods`).
- Do not depend on the root `ironclaw` crate or `src/` modules.
- Do not add legacy bridge modes here until an accepted migration contract exists.
- Do not route live v1/product traffic here; callers must opt in through explicit Reborn adapters.
- Production and migration-dry-run profiles must fail closed on local-only or missing required handles.
- Product auth composition must use `ironclaw_auth` trait-shaped ports. Do not
  wire product auth through V1 OAuth routes, V1 pending maps, V1
  `ExtensionManager`, V1 secret stores, or route-local raw HTTP clients.
- Product auth refresh and lifecycle cleanup callers should use
  `RebornProductAuthServices::refresh_credential_account` and
  `cleanup_credentials_for_lifecycle`. Do not reconstruct credential stores,
  provider clients, V1 extension cleanup, or route-local secret authority at
  the call site.
- OAuth callback routes should only parse/validate HTTP input and call
  `RebornProductAuthServices` for flow preflight/callback handling; the
  handler must claim the scoped flow/state/provider through `AuthFlowManager`
  before exchanging provider material through `AuthProviderClient`, then
  complete the flow and emit typed continuations.
- The first WebUI-mounted OAuth route keeps raw PKCE verifiers in a bounded,
  expiring process-local cache because `ironclaw_auth` durable records may
  store hashes only. Do not treat that route as multi-replica/restart-safe
  until a host-owned encrypted verifier store or equivalent sticky callback
  mechanism is wired.
- Manual-token setup routes should call
  `RebornProductAuthServices::request_manual_token_setup` for the typed
  challenge and `RebornProductAuthServices::submit_manual_token` with a
  one-shot `RebornManualTokenSubmitRequest` for the dedicated secret-submit
  body. Build setup request scope from authenticated caller/session context,
  not a browser body; attach `CredentialAccountUpdateBinding` only after
  pre-authorizing the existing scoped account. Do not route raw token values
  through chat commands, model-visible messages, serializable DTOs,
  projections, or route-local pending maps.
- `RebornProductAuthServices::flow_record_source` is an optional WebUI/local-dev
  read-projection port, not a required product-auth capability. Filesystem-backed
  local-dev composition wires the durable product-auth service itself as this
  source so pending auth gates can be rendered from blocked turn state plus
  auth-flow records. If a supplied product-auth bundle
  omits it, runtime composition must expose the WebUI auth interaction surface
  as explicitly unavailable; do not fabricate a route-local or unscoped pending
  auth read model.
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
| `RebornWebuiBundle` (in [`src/webui.rs`](src/webui.rs)) | `{ api: Arc<dyn RebornServicesApi>, product_auth: Option<Arc<RebornProductAuthServices>>, readiness }` — the v2 facade, optional product-auth route service, plus readiness snapshot |
| `build_webui_services(runtime, event_stream)` | Compose a `RebornWebuiBundle` from an already-built `RebornRuntime`; reuses the runtime's thread service / turn coordinator, product-auth services, and runtime-owned `EventStreamManager` projection stream unless a caller supplies a custom stream |
| `RebornProjectionServices` (in `src/projection.rs`) | Runtime-owned projection/event-stream composition; owns the single local-dev `EventStreamManager` and creates product-specific `ProjectionStream` adapters over it |
| `WebuiAuthenticator` trait | Host-supplied bearer-token verifier; returns `Option<WebuiAuthentication>` so identity and request-scoped WebUI capabilities travel together |
| `WebuiServeConfig { tenant_id, authenticator, max_body_bytes, allowed_origins, csp_header }` | Required config for `webui_v2_app`; no defaults that silently disable security |
| `webui_v2_app(bundle, config) -> Router` | Build the fully-composed axum `Router`. This is the seam between this product/API crate and host-owned HTTP ingress: tests drive it via `tower::ServiceExt::oneshot`; the `ironclaw-reborn serve` subcommand (follow-up PR) hands it to `axum::serve` from a host-owned listener |
| `ProtectedRouteMount` | Host-supplied protected API route fragment merged inside the WebUI bearer-auth layer with descriptor-driven body/rate limits. Reborn OpenAI-compatible routes use this seam; do not use it for v1 gateway routers. |

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
   `BodyLimitPolicy` from `ironclaw_webui_v2::webui_v2_routes()` and,
   when present, product-auth route descriptors at composition time and
   enforces it before auth runs (so an oversized payload never spends a
   bearer-validation step). Today: `create_thread`, product-auth OAuth
   start, manual-token setup/secret-submit, accounts list/select/recovery/
   refresh, and lifecycle cleanup — all 16 KiB; `send_message` 14 MiB
   (text + base64 inline attachments); `cancel_run` and `resolve_gate` 4 KiB; `get_timeline`,
   `stream_events`, and product-auth OAuth callback `NoBody`.
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
   `config.tenant_id` plus the authentication result's `UserId`, and a
   request-scoped `WebUiV2Capabilities` extension from the same
   authentication result.
   When `openai-compat-beta` is enabled, the same verified bearer result also
   inserts an `OpenAiCompatAuthenticatedCaller` extension with tenant-scoped
   verified auth evidence for protected OpenAI-compatible route mounts; route
   crates must not mint this evidence.
8. **Descriptor-driven per-route rate limit**
   (`webui_rate_limit::enforce_rate_limit`) — reads
   `ironclaw_webui_v2::webui_v2_routes()` plus mounted product-auth
   descriptors at composition time and enforces the declared
   `RateLimitPolicy` with a sliding window. Authenticated WebUI/product
   auth start routes use `RateLimitScope::PerCaller`; the public OAuth
   callback uses `RateLimitScope::PerIp` backed by host-injected
   `ConnectInfo<SocketAddr>`, never `X-Forwarded-For` / `X-Real-IP`.
   Composition fails closed if a future descriptor declares an unsupported
   scope.
9. `webui_v2_router(WebUiV2State::new(bundle.api))` — the v2
   handlers from `ironclaw_webui_v2` (create-thread, list-threads, delete-thread,
   send-message, get-timeline, stream-events SSE, stream-events WS,
   cancel-run, resolve-gate, setup-extension, list-automations).

### Product-auth routes

When `bundle.product_auth` is present, `webui_v2_app` also mounts the
Reborn-native product-auth surface:

- `POST /api/reborn/product-auth/oauth/start` is inside the existing
  bearer-auth layer. It derives `AuthProductScope` from the
  `WebUiAuthenticatedCaller` inserted by host composition, hashes raw
  state/PKCE once, rejects caller-supplied expiry beyond the route TTL,
  and creates an `AuthFlowRecord` through `AuthFlowManager::create_flow`.
  The response authorization URL is composed after flow creation so it can
  carry flow/callback metadata without serializing raw state or PKCE material.
  The route has a 16 KiB JSON body cap and per-caller rate limit in addition
  to the gateway-wide fallback cap. The browser cannot choose `AuthFlowKind` or
  `AuthContinuationRef`; this first route creates integration-credential
  setup flows with `AuthContinuationRef::SetupOnly`.
- `GET /api/reborn/product-auth/oauth/callback/{flow_id}` is a hosted
  callback route and is intentionally not behind WebUI bearer auth. It
  bounds the raw query string, reconstructs the scoped callback owner from
  host/callback metadata, hashes raw state/code/PKCE verifier material, and calls
  `RebornProductAuthServices` for flow preflight/callback handling. The route never
  exchanges provider tokens, activates extensions, resumes turns, or
  writes secrets directly. Its descriptor declares `NoBody` and a
  transport-peer-IP public callback rate limit.
- `POST /api/reborn/product-auth/manual-token/submit` is inside the
  same bearer-auth layer as OAuth start. It derives `AuthProductScope`
  from `WebUiAuthenticatedCaller`, validates the provider/account/token
  fields, creates a short-lived manual-token interaction with a
  `TurnGateResume` continuation, submits the raw token only to
  `RebornProductAuthServices`, and returns the resulting
  `credential_ref`. The browser must then call v2 `resolve_gate` with
  that `credential_ref`; raw token values never go through gate resolution.
  Setup, submit, and cleanup calls are timeout-bounded. If submit fails after
  the interaction is created, the route abandons that scoped interaction before
  returning a sanitized error.
- Raw `state`, OAuth authorization codes, PKCE verifiers, provider
  token handles, provider bodies, and host paths must not be logged or
  serialized by the route. Responses use the sanitized product-auth
  success/error DTOs only.

`webui_route_match` is the shared matcher both the body-limit and
rate-limit middlewares consume so the two enforcers cannot drift on
which request belongs to which descriptor.

### Extension pairing routes

When Slack host-beta personal binding is configured, `webui_v2_app`
mounts `POST /api/webchat/v2/extensions/pairing/redeem` inside the same
bearer-auth layer as the native WebUI v2 extension routes. The request
body carries `{ channel, code }`; the route validates the channel server-side
and currently resolves the supported Slack channel aliases to the Slack
personal-binding pairing service. The browser must not call provider-specific
pairing paths directly.

When Slack host-beta channel routing is configured, `webui_v2_app` also mounts
`GET|PUT|DELETE /api/webchat/v2/channels/slack/routes` and
`GET|PUT /api/webchat/v2/channels/slack/allowed` plus
`GET /api/webchat/v2/channels/slack/subjects` inside the same bearer auth
layer. The low-level `routes` API accepts `channel_id` plus
`subject_user_id`; the WebUI v2 channel picker uses the admin-managed
`allowed` API, reads the `subjects` catalog for named routable team agents,
and can save either legacy `channel_ids` or explicit per-channel
`{ channel_id, subject_user_id }` assignments. Allowed-channel responses also
include the backend-derived `subject_display_name` for each saved route so the
browser does not derive team-agent labels from raw subject ids. Missing explicit subjects are
deterministically assigned tenant-scoped Slack channel subjects, while existing
generated/current route subjects may be preserved for their same channel.
Tenant, adapter installation, and Slack team always come from host
configuration. These routes write to Slack host state so runtime assignments
are durable and are resolved before static TOML `channel_routes` fallback. In
admin-managed host-beta mode, new shared Slack conversations without a dynamic
or static channel route fail closed instead of falling back to the installation
default subject.

### Host-supplied public route mount (#4116 — SSO login surface)

`WebuiServeConfig::with_public_route_mount(PublicRouteMount)`
attaches a host-supplied `{ router, descriptors }` pair that is
merged into the composed app OUTSIDE the bearer-auth layer but
INSIDE the outer security-header / CORS / global-body-limit
stack. The seam exists for the WebChat v2 SSO login surface
shipped by
`ironclaw_reborn_webui_ingress::webui_v2_auth_router`, which
mounts `/auth/providers`, `/auth/login/{provider}`,
`/auth/callback/{provider}`, and `/auth/logout`. The browser must
reach those routes without a session (the whole point of login),
so they cannot live behind the bearer middleware; they still
inherit the same defense-in-depth headers and CORS allow-list as
every other response.

The mount's `descriptors` are folded into the SAME descriptor
list the v2 facade and the product-auth callback already use, so
the descriptor-driven per-route rate-limit and body-limit
middlewares apply to the host-supplied surface exactly like they
do to every other route — no side door. Today the SSO mount
declares its five routes as `LocalGateway`/`OAuthCallback` +
`IngressAuthPolicy::Public` + `RateLimitScope::PerIp` (60–120
req/min/IP, 60s window) + tight body limits, so a sustained
`/auth/login/*` flood is bounded by the same per-IP counter the
public OAuth callback uses.

This seam must NOT be used to re-introduce v1 `/auth/*` handlers
into the v2 listener; the only intended consumer is the
Reborn-native auth router. v1 gateway code remains untouched —
`src/channels/web/` keeps its own bearer/OAuth stack.

### Session transport decision (#4116)

The OAuth callback returns a short-lived, one-time login ticket to
the SPA via the URL query (`/v2?login_ticket=<ticket>`), not the
session bearer itself and not an `HttpOnly` cookie. The SPA
immediately POSTs that ticket to `/auth/session/exchange` and stores
the returned bearer in `sessionStorage`.
Rationale:

- **Matches the existing v2 SPA auth model.** `app/auth.js`
  already stores the bearer in `sessionStorage` and sends it as
  `Authorization: Bearer` on every API call; SSE / WS use the
  `?token=` query-string shim that the composition layer's bearer
  middleware accepts only on `GET /api/webchat/v2/threads/{id}/events`.
  Cookies would require a new auth path through the same middleware.
- **The bearer never appears in a redirect `Location` header.** A
  logged callback redirect can expose only the one-time ticket, not
  the long-lived bearer. Tickets are short-lived and consumed
  atomically by the exchange route. Composition also emits
  `Referrer-Policy: no-referrer` as defense in depth.
- **Logout actually revokes.** `POST /auth/logout` calls
  `SessionStore::revoke`; the regression in
  `crates/ironclaw_reborn_webui_ingress/tests/session_round_trip.rs`
  locks that a post-revoke bearer fails on `/api/webchat/v2/threads`.

This is a deliberate divergence from the v1 gateway, which sets a
`Set-Cookie: ironclaw_session=...; HttpOnly` on its OAuth
callback. v1 cookie code is NOT shared with the v2 listener.

### Entrypoint inventory (#3580)

Mapping of every v1 gateway entrypoint to its Reborn native-surface
counterpart. "v1-only" means the v2 facade does not yet expose this
shape and a future native-surface ticket owns the migration — these
rows are inventoried here, not implemented in the current PR.

| Concern | v1 entrypoint (under `src/channels/web/`) | v2 native counterpart | Status |
|---|---|---|---|
| Send message | `POST /api/chat/send` | `POST /api/webchat/v2/threads/{thread_id}/messages` | Mapped |
| Create thread | `POST /api/chat/thread/new` | `POST /api/webchat/v2/threads` | Mapped |
| Session/profile capabilities | `GET /api/profile` | `GET /api/webchat/v2/session` | Mapped to authenticated tenant/user plus request-scoped WebUI capabilities; `operator_webui_config` follows the matched bearer token, not just the deployment-wide route-mount decision |
| List threads | `GET /api/chat/threads` | `GET /api/webchat/v2/threads` | Mapped |
| Delete thread | (none) | `DELETE /api/webchat/v2/threads/{thread_id}` | Mapped |
| Read history / timeline | `GET /api/chat/history` | `GET /api/webchat/v2/threads/{thread_id}/timeline` | Mapped |
| SSE stream | `GET /api/chat/events` | `GET /api/webchat/v2/threads/{thread_id}/events` | Mapped (incl. `?token=` shim) |
| WebSocket stream | `GET /api/chat/ws` | `GET /api/webchat/v2/threads/{tid}/ws` | Mapped |
| Cancel run | (engine v1 surface) | `POST /api/webchat/v2/threads/{tid}/runs/{run_id}/cancel` | Mapped |
| Resolve gate | `POST /api/chat/gate/resolve` | `POST /api/webchat/v2/threads/{tid}/runs/{run_id}/gates/{gate_ref}/resolve` | Mapped |
| Approval shim | `POST /api/chat/approval` | (Subsumed by `resolve_gate`) | Mapped |
| Auth-token / auth-cancel | `POST /api/chat/auth-{token,cancel}` | (Engine v1 compatibility shim; delete with v1) | v1-only (legacy) |
| Extensions registry/list/install/activate/remove/setup | `GET\|POST /api/extensions/*` | `GET /api/webchat/v2/extensions`, `GET /api/webchat/v2/extensions/registry`, `POST /api/webchat/v2/extensions/install`, `POST /api/webchat/v2/extensions/{package_id}/{activate,remove,setup}` | Mapped to lifecycle package refs and registry projections; setup projects credential requirements and product-auth OAuth start is mounted under the extension setup surface |
| LLM provider config | v1 settings/provider config surface | `GET /api/webchat/v2/llm/providers`, `POST /api/webchat/v2/llm/providers`, `POST /api/webchat/v2/llm/providers/{provider_id}/delete`, `POST /api/webchat/v2/llm/active`, `POST /api/webchat/v2/llm/{test-connection,list-models}` | Mapped for trusted operator-token deployments; left unmounted for multi-user authenticators until an admin role boundary exists |
| Operator status/readiness | v1 doctor/readiness surfaces | `GET /api/webchat/v2/operator/status` | Mapped to Reborn readiness projection through the product facade; left unmounted with other operator routes for multi-user authenticators |
| Operator logs | `src/cli/logs.rs` command path | `GET /api/webchat/v2/operator/logs` | Mapped to the in-process operator log buffer with bounded query, tail, follow, filter, cursor, and redaction behavior |
| Operator service lifecycle | `src/cli/service.rs` command path | `POST /api/webchat/v2/operator/service` | Route and facade shell mapped; default runtime reports unsupported until a platform lifecycle backend is wired |
| SSO login (Google) | `GET /auth/providers`, `GET /auth/login/{p}`, `GET /auth/callback/{p}`, `POST /auth/logout` | Same paths on the v2 listener via `ironclaw_reborn_webui_ingress::webui_v2_auth_router`, merged into `webui_v2_app` through [`WebuiServeConfig::with_public_route_mount`] (typed `{ router, descriptors }` so the per-route body-limit / rate-limit middleware applies) | Mapped (Google); GitHub + NEAR follow under #4116 |

### Security invariants on every "Mapped" row

- **Bearer / OIDC / cookie auth** — none of these are shared with v1's
  `auth_middleware`. The Reborn binary owns its own
  `WebuiAuthenticator` impl (env tokens, DB-backed sessions, OIDC,
  whatever the host wires) and supplies it via `WebuiServeConfig`.
- **Operator WebUI config** — the `/api/webchat/v2/llm/*` routes and
  Slack channel-route admin mutate operator-wide provider settings,
  secrets, or channel ownership. `webui_v2_app` mounts them only when
  the host authenticator opts into
  `mounts_operator_webui_config_routes()`, then authorizes each request
  from the matched token's `WebUiV2Capabilities`. Multi-user
  session/OIDC authenticators must not grant
  `operator_webui_config` unless a real admin authorization boundary
  exists.
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
  `send_message` 14 MiB, `cancel_run` / `resolve_gate` 4 KiB,
  `get_timeline` / `stream_events` `NoBody`. The outer
  `RequestBodyLimitLayer` at `config.max_body_bytes` (14 MiB default)
  is kept as defense in depth for paths that don't match any v2
  descriptor.
- **Rate limit** — descriptor-driven; the v2 crate declares mutation
  60/60, read 120/60, stream 30/60 per `(tenant, user)`. Reading and
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
the runtime-owned durable event log plus `EventStreamManager`, so
`/events` and `/ws` no longer advertise routes that only return
`Unavailable`. In local-dev builds with `libsql` enabled, the log and
runtime state stores sit behind the composed local-dev root filesystem
(`reborn-local-dev.db` for durable records, `/projects` for workspace
files). Production durable retention/live fanout still belongs in the
host runtime/event-store follow-up rather than this composition facade.

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
  NoBody policy rejecting a non-empty body on a read route. The
  `static_*` cases additionally serve the embedded SPA bundle through
  the same composed router and assert asset content shape — including
  `static_i18n_module_guards_locale_race_and_clears_failed_pack_cache`
  (the `setLang` latest-request guard plus `pending[lang]` clear on
  both settle paths) and `static_typing_dot_animation_respects_reduced_motion`
  (typing dots animate by default, suppressed under
  `prefers-reduced-motion`). These lock source shape only; behavioral
  JS/`getComputedStyle` coverage needs a browser harness this workspace
  does not own and is deferred to the JS/e2e scaffold.
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
