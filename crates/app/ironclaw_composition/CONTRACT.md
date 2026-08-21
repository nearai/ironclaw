# ironclaw_composition guardrails

- Own only top-level Reborn composition for production/app startup.
- Expose service-shaped handles only: `HostRuntime`, `TurnCoordinator`, product-auth `RebornProductAuthServices`, WebUI `ProductSurface`, readiness.
- Keep lower substrate handles private to factories and owning crates.
- Substrate handles MAY be exposed via `#[cfg(any(test, feature = "test-support"))]` pub accessors on `RebornRuntime` when downstream integration tests need to drive production-shape state the service doesn't yet surface (e.g. seeding `TriggerRecord` rows, `pair_external_actor` calls). These seams ship zero bytes in production binaries. New test-support accessors must carry a doc-comment naming the production call site they mirror and an explicit note that the handle is for tests only.
- Outbound state stores are composition-owned; do not construct `ironclaw_outbound::OutboundStateStore` in consumer modules (lint-enforced — see the `disallowed-methods` entry in the workspace `clippy.toml`).
- Do not depend on the root `ironclaw` crate or `src/` modules.
- Do not add legacy bridge modes here until an accepted migration contract exists.
- Do not route live v1/product traffic here; callers must opt in through explicit Reborn adapters.
- Production and migration-dry-run profiles must fail closed on local-only or missing required handles.
- Product auth composition must only wire `RebornProductAuthServices` through
  the composition runtime entrypoint (`build_reborn_runtime`/`RebornRuntime`,
  `src/runtime.rs`) with
  configured stores, secrets, network clients, and generic runtime/product
  adapters. Product-auth contracts, durable services, continuations, refresh,
  cleanup, recipes, and fakes live in `ironclaw_auth`; HTTP route serving lives
  behind `ironclaw_webui::product_auth_route_mount`.
  Do not wire product auth through V1 OAuth routes, V1 pending maps, V1
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
- Setup-lane OAuth PKCE verifiers are durable (#6169 port):
  `start_setup_oauth_flow` writes the raw verifier to the injected
  `SecretStore` under `product-auth-setup-pkce-{flow_id}` (TTL = the flow's
  `expires_at`) BEFORE `create_flow`; the callback read is one-shot
  (`lease_once`/`consume`, setup handle first, blocked-turn gate store as
  fallback — mirroring `oauth_gate.rs`); terminal callback outcomes discard
  the durable copy, and `cleanup_credentials_for_lifecycle` eagerly drops
  verifiers for `SecretCleanupReport::canceled_flows`. The serve routes'
  bounded process-local verifier cache is a same-process fast path only —
  never treat it as the source of truth; restart/replica safety is pinned by
  `vendor_oauth_callback_completes_after_route_state_restart`. Early
  defensive cache evictions (unknown flow, cross-vendor path) must NOT
  discard the durable copy — the legitimate callback may still need it.
- Manual-token setup routes should call
  `RebornProductAuthServices::request_manual_token_setup` for the typed
  challenge and `RebornProductAuthServices::submit_manual_token` with a
  one-shot `RebornManualTokenSubmitRequest` for the dedicated secret-submit
  body. Build setup request scope from authenticated caller/session context,
  not a browser body; attach `CredentialAccountUpdateBinding` only after
  pre-authorizing the existing scoped account. Do not route raw token values
  through chat commands, model-visible messages, serializable DTOs,
  projections, or route-local pending maps.
- `RebornProductAuthServices::flow_record_source` is an optional WebUI/standalone
  read-projection port, not a required product-auth capability. Filesystem-backed
  standalone composition wires the durable product-auth service itself as this
  source so pending auth gates can be rendered from blocked turn state plus
  auth-flow records. If a supplied product-auth bundle
  omits it, runtime composition must expose the WebUI auth interaction surface
  as explicitly unavailable; do not fabricate a route-local or unscoped pending
  auth read model.
- Blocked run-state approval/auth gate rendering and resume belongs to #3094;
  keep this crate's #3811 auth seam reusable by that layer without implementing
  a second gate-resolution path.
- Standalone/WebUI capability result plumbing may stage results in memory, but any
  durable thread append keyed by `LoopRunContext` (for example capability display
  preview timeline messages) must resolve the thread scope through
  `ironclaw_loop_host::ThreadScopeResolver::resolve_for_turn` before
  calling `SessionThreadService`; do not append with the runtime/base
  `ThreadScope` directly in multi-user WebUI paths.

## WebUI v2 native surface

> **Moved (#6618).** The WebChat v2 HTTP gateway composition described in
> this section no longer lives in this crate — `webui_v2_app`, the middleware
> stack, and the serve loop are in **`crates/product/ironclaw_webui/src/`**. What
> composition still owns is the `RebornRuntime` that the WebUI host consumes.
> The section is kept because the seam contract below is still accurate; the
> paths have been repointed. Verify with
> `rg -n "fn webui_v2_app" crates/`.

It is the native host-owned surface that enters `ProductSurface` directly
(`docs/internal/reborn/how-to-port-channel-to-reborn.md`, "Native host surface").

### Surface

| Symbol | Role |
|---|---|
| `RebornRuntime::product_surface(event_stream)` | Build the v2 `Arc<dyn ProductSurface>` from an already-built `RebornRuntime`; reuses the runtime's thread service / turn coordinator, product-auth services, and runtime-owned `EventStreamManager` projection stream unless a caller supplies a custom stream |
| `RebornRuntime::product_auth_services()` / `RebornRuntime::readiness()` | Runtime-owned host handles consumed by the CLI/WebUI host when mounting auth routes and reporting readiness |
| `RebornProjectionServices` (in `crates/product/ironclaw_assistant/src/projection.rs`) | Runtime-owned projection/event-stream composition; owns the single standalone `EventStreamManager` and creates product-specific `ProjectionStream` adapters over it |
| `WebuiAuthenticator` trait | Host-supplied bearer-token verifier; returns `Option<WebuiAuthentication>` so identity and request-scoped WebUI capabilities travel together |
| `WebuiServeConfig { tenant_id, authenticator, max_body_bytes, allowed_origins, csp_header }` | Required config for `webui_v2_app`; no defaults that silently disable security |
| `webui_v2_app(product_surface, config) -> Result<Router, WebuiServeError>` | Build the fully-composed axum `Router`. This is the seam between this product/API crate and host-owned HTTP ingress: tests drive it via `tower::ServiceExt::oneshot`; the `ironclaw serve` subcommand hands it to `axum::serve` from a host-owned listener |
| `ProtectedRouteMount` | Host-supplied protected API route fragment merged inside the WebUI bearer-auth layer with descriptor-driven body/rate limits. Reborn OpenAI-compatible routes use this seam; do not use it for v1 gateway routers. |

### Middleware stack composed by `webui_v2_app`

Inbound order (outer → inner → handler) is the invariant; exact byte/rate
limits live in `descriptors.rs`, `webui_body_limit.rs`, and
`webui_rate_limit.rs` — read those, not this file, for today's numbers:

1. `SetResponseHeaderLayer` — static security headers (`nosniff`, `DENY`,
   CSP).
2. `CorsLayer` — allow-origin from `config.allowed_origins`; empty list
   means fail-closed (no echoing attacker-supplied origin).
3. `CatchPanicLayer` — panic boundary, logs truncated detail.
4. **Outer `RequestBodyLimitLayer`** (`config.max_body_bytes`) — defense in
   depth for paths that don't match any v2 descriptor (e.g. axum's 404
   fallback). v2 routes are additionally capped, strictly tighter, by the
   per-route limit below.
5. **Descriptor-driven per-route body limit**
   (`webui_body_limit::enforce_body_limit`) — reads each route's
   `BodyLimitPolicy` from `ironclaw_webui::webui_v2_routes()` and, when
   present, product-auth route descriptors at composition time, and
   enforces it **before auth runs** (so an oversized payload never spends a
   bearer-validation step). `BodyLimitPolicy` is an exhaustive `match`, so
   a new variant added upstream fails the build rather than silently
   disabling enforcement.
6. **WS same-origin enforcement** (`webui_ws_origin::enforce_websocket_origin`)
   — runs only on descriptors that declare a non-`NotApplicable`
   `WebSocketOriginPolicy`. The browser does not pre-flight WebSocket
   upgrades, so origin enforcement happens inline; absence or mismatch
   yields a `403` before the v2 handler executes the WS upgrade.
7. **Bearer auth + `?token=` shim** (`webui_serve::authenticate_request`)
   — `Authorization: Bearer <token>` for every route; `?token=` is
   honored ONLY on `GET /api/webchat/v2/threads/{id}/events` because
   the browser's `EventSource` cannot set headers. Mutations and
   timeline reads stay bearer-only. On success the middleware inserts
   a `ProductSurfaceCaller` extension built from
   `config.tenant_id` plus the authentication result's `UserId`, and a
   request-scoped `WebUiV2Capabilities` extension from the same
   authentication result.
   The same verified bearer result also
   inserts an `OpenAiCompatAuthenticatedCaller` extension with tenant-scoped
   verified auth evidence for protected OpenAI-compatible route mounts; route
   crates must not mint this evidence.
8. **Descriptor-driven per-route rate limit**
   (`webui_rate_limit::enforce_rate_limit`) — reads
   `ironclaw_webui::webui_v2_routes()` plus mounted product-auth
   descriptors at composition time and enforces the declared
   `RateLimitPolicy` with a sliding window. Authenticated WebUI/product
   auth start routes use `RateLimitScope::PerCaller`; the public OAuth
   callback uses `RateLimitScope::PerIp` backed by host-injected
   `ConnectInfo<SocketAddr>`, never `X-Forwarded-For` / `X-Real-IP`.
   Composition fails closed if a future descriptor declares an unsupported
   scope.
9. `webui_v2_router(WebUiV2State::new(bundle.product_surface))` — the v2
   handler set from `ironclaw_webui`; see that crate's router source for the
   current route list.

After the complete descriptor set is assembled, composition derives every
literal root namespace and supplies it to `static_router_with_config`. Exact
host routes still win through Axum routing, while unknown paths in any
host-owned namespace return 404 instead of the SPA shell. A descriptor whose
first segment is dynamic, percent-encoded/noncanonical, missing, or already
owned by a static asset or explicit static route fails composition: no finite
fail-closed reservation can represent those overlaps safely.

### Product-auth routes

When `bundle.product_auth` is present, `ironclaw_webui::webui_v2_app` also
mounts the Reborn-native product-auth surface:

- `POST /api/reborn/product-auth/oauth/start` is inside the existing
  bearer-auth layer. It derives `AuthProductScope` from the
  `ProductSurfaceCaller` inserted by host composition, hashes raw
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
  from `ProductSurfaceCaller`, validates the provider/account/token
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

### Slack personal OAuth setup

Slack host-beta normal personal setup is extension-card driven: the user
installs the Slack extension, clicks Configure, and the card starts the
provider-`slack` product-auth OAuth flow (the retired `slack_personal`
provider id survives only in the one-time forward data migration). The
successful callback binds the Slack `authed_user.id` to the authenticated
Reborn user through the host-owned identity binding store. Slack personal setup is OAuth-only; the old browser
manual-code redeem route and Slack command flow are not mounted.

✎ 2026-08-07: a subsection here described
`GET|PUT|DELETE /api/webchat/v2/channels/slack/routes`,
`GET|PUT /api/webchat/v2/channels/slack/allowed`, and
`GET /api/webchat/v2/channels/slack/subjects` admin routes carrying
`subject_user_id` assignments. Those routes were never shipped (zero
occurrences in the webui source), and the per-channel subject concept is
retired: a run acts as its invoker — a shared Slack conversation is one
canonical thread its paired participants share, each message running as its
sender, with no subject user to assign. Shared-channel admission is
presence-based and needs no configuration: `ironclaw_extension_host`'s
`PresenceSharedAdmission` admits exactly the conversations delivered through
the channel's verified ingress for its own installation (the bot being in
the channel is the admission), still consulted fail-closed through the
`SharedConversationAdmission` port on resolve, lookup, and reset.

### Host-supplied public route mount (#4116 — SSO login surface)

`WebuiServeConfig::with_public_route_mount(PublicRouteMount)`
attaches a host-supplied `{ router, descriptors }` pair that is
merged into the composed app OUTSIDE the bearer-auth layer but
INSIDE the outer security-header / CORS / global-body-limit
stack. The seam exists for the WebChat v2 SSO login surface
shipped by
`ironclaw_webui::webui_v2_auth_router`, which
mounts `/auth/providers`, `/auth/login/{provider}`,
`/auth/callback/{provider}`, and `/auth/logout`. The browser must
reach those routes without a session (the whole point of login),
so they cannot live behind the bearer middleware; they still
inherit the same defense-in-depth headers and CORS allow-list as
every other response.

The mount's `descriptors` are folded into the SAME descriptor
list the v2 service and the product-auth callback already use, so
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
Reborn-native auth router. (✎ The v1 gateway and its `src/channels/web/`
bearer/OAuth stack have since been deleted; the constraint stands as a
shape rule for any future public route mount.)

### Session transport decision (#4116)

The OAuth callback returns a short-lived, one-time login ticket to
the SPA via the URL query (`/?login_ticket=<ticket>`), not the
session bearer itself and not an `HttpOnly` cookie. The SPA
immediately POSTs that ticket to `/auth/session/exchange` and stores
the returned bearer in `sessionStorage`.
Rationale:

- **Matches the existing v2 SPA auth model.**
  `crates/product/ironclaw_webui/frontend/src/app/auth.ts`
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
  `SignedTokenSessionStore::revoke`; the regression in
  `crates/product/ironclaw_webui/tests/session_round_trip.rs`
  locks that a post-revoke bearer fails on `/api/webchat/v2/threads`.

This is a deliberate divergence from the v1 gateway, which sets a
`Set-Cookie: ironclaw_session=...; HttpOnly` on its OAuth
callback. v1 cookie code is NOT shared with the v2 listener.

### #3580 v1→v2 entrypoint migration: complete

✎ Every v1 gateway entrypoint (`src/channels/web/`, since deleted) has a
Reborn native-surface (`/api/webchat/v2/*`) counterpart; the one-off
engine-v1 auth-token/auth-cancel compatibility shim did not carry forward.
The full old-route → new-route mapping was a migration-tracking table, not
an ongoing invariant, and is retired now that #3580 is done — the current
route set lives in the router source (`ironclaw_webui::webui_v2_routes()`
and the auth/product-auth mounts below), not in this prose.

### Security invariants on every v2 route

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
  `ironclaw_webui::webui_v2_routes()` — `descriptors.rs` is the sole
  authoritative source; the figures below are a snapshot, re-derive with
  `rg -n 'body_limit_kib' crates/product/ironclaw_webui/src/webui_v2/descriptors.rs`
  before trusting them: `create_thread` 16 KiB, `send_message` 14 MiB,
  `cancel_run` / `resolve_gate` 4 KiB, `get_timeline` / `stream_events`
  `NoBody`. The outer `RequestBodyLimitLayer` at `config.max_body_bytes`
  (14 MiB default) is kept as defense in depth for paths that don't match
  any v2 descriptor.
- **Rate limit** — descriptor-driven; the v2 crate declares mutation
  60/60, read 120/60, stream 30/60 per `(tenant, user)` — snapshot,
  `webui_rate_limit.rs` and `descriptors.rs` are authoritative; re-derive
  with `rg -n 'rate_limit_per_caller' crates/product/ironclaw_webui/src/webui_v2/descriptors.rs`.
  Reading and enforcing happens in `webui_rate_limit::build_rate_limit_state`.
- **Static security headers** — `nosniff`, `DENY`, CSP applied via
  outer `SetResponseHeaderLayer`s; default CSP is
  `default-src 'self'; object-src 'none'; frame-ancestors 'none';
  base-uri 'self'`.
- **Connection limit (SSE + WS)** — bounded by `ironclaw_webui`'s own
  `SseCapacity` (3 streams per `(tenant, user)`, 5-minute max stream
  lifetime). The WebSocket stream (`stream_events_ws`) draws from the same
  shared `SseCapacity` pool as SSE (pinned by
  `stream_events_ws_shares_capacity_with_sse_streams` in
  `ironclaw_webui`'s `webui_v2_handlers_contract.rs`).
- **Caller construction** — `ProductSurfaceCaller` is built from
  `config.tenant_id` (trusted host installation) plus the
  authenticator's verified `UserId`. The browser body cannot influence
  either field; matches the rule in
  `crates/product/ironclaw_assistant/AGENTS.md`.

### What this composition deliberately does NOT do

Per Path A in `docs/internal/reborn/how-to-port-channel-to-reborn.md`:

- No `ProductAdapter` wrapper around browser sessions.
- No fake `ExternalActorRef` / `ProtocolAuthEvidence` /
  `OutboundDeliverySink` / declared egress.
- No shared middleware with the deleted v1 ✎ `src/channels/web/` stack —
  the v2 listener was built standalone and there is no v1 binary left to
  share with.

### How the standalone `ironclaw serve` consumes this

The `serve` subcommand builds a full standalone `RebornRuntime`, asks
`runtime.product_surface(None)` for the product surface, and hands
the resulting router to the host-owned `ironclaw_webui`
listener lifecycle. The default projection stream is backed by
the runtime-owned durable event log plus `EventStreamManager`, so
`/events` and `/ws` no longer advertise routes that only return
`Unavailable`. In standalone builds with `libsql` enabled, the log and
runtime state stores sit behind the composed standalone root filesystem
(`reborn-local-dev.db` for durable records, `/projects` for workspace
files). Production durable retention/live fanout still belongs in the
host runtime/event-store follow-up rather than WebUI ingress.

Live cumulative assistant-text projections are published at provider cadence.
The live-text publisher conflates only sub-frame microbursts within a 16 ms
window before they reach the event-stream manager's bounded subscriber.
Ordinary provider cadence remains incremental, while each microburst retains
its latest cumulative snapshot. Reasoning, capability, and lifecycle milestones
remain ordered. The in-memory source still records the latest projection for
replay when no browser subscriber is active.

The opaque WebUI projection cursor stamps its process-local live position with
the projection-services epoch. On an epoch mismatch, resume preserves durable
runtime and turn positions but clears the volatile live position so a browser
cursor retained across a deployment cannot suppress the restarted process's
interim updates. This is enforced by
`ProductRuntimeProjectionStream::runtime_subscription` in
`crates/product/ironclaw_assistant/src/projection.rs`.

```rust
// Inside a host-owned ingress crate / binary (NOT in this crate —
// `reborn_product_api_crates_do_not_bind_http_ingress` forbids
// product/API crates from owning server lifecycle).
let runtime = build_reborn_runtime(input).await?;
let product_surface = runtime.product_surface(None)?;
let config = WebuiServeConfig::new(
    TenantId::new(host_installation_tenant)?,
    Arc::new(MyHostAuthenticator::new(...)),
    same_origin_allowlist(bound_addr),
);
let app = webui_v2_app(product_surface, config)?;
let listener = tokio::net::TcpListener::bind(addr).await?;
axum::serve(listener, app).with_graceful_shutdown(shutdown).await?;
```

### Tests

Caller-level and unit coverage for the seams above — see each file for
current test names and assertions rather than a prose transcription here:

- `src/runtime/tests/core.rs` — standalone CLI/WebUI runtime regressions
  (saved model preference resolution, product surface reuses runtime
  turn/thread services).
- `crates/product/ironclaw_assistant/src/projection/tests/runtime_stream.rs`,
  `turn_stream.rs`, `live_progress_stream.rs` — product event-stream
  projection regressions (run-status drain, request-actor scoping, live-text
  microburst/cadence behavior).
- `tests/webui_v2_serve.rs` — the composed `Router` driven through
  `tower::ServiceExt::oneshot`: auth, SSE `?token=`, security headers, CORS,
  rate-limit, body-limit, and static-asset-serving cases.
- `crates/product/ironclaw_webui/src/webui_serve.rs`,
  `webui_route_match.rs`, `webui_rate_limit.rs`, `webui_body_limit.rs`
  (`::tests` modules in each) — matcher/extraction unit tests and
  composition-time tests that the rate-limit/body-limit state accepts every
  v2 descriptor and preserves its declared policy.
