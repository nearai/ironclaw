# ironclaw_webui guardrails

The **WebUI host stack** for Reborn WebChat v2 — the single `products`-layer
crate, above `ironclaw_reborn_composition`, that turns composition's product/API
surface into a running HTTP server a browser can talk to. It owns three
subsystems that used to live apart (see `README.md` for the fold-in map):

1. **WebChat v2 route surface + SPA** (`src/webui_v2/`, from the former
   `ironclaw_webui_v2` crate) — the axum handlers over `ProductSurface`, the
   `webui_v2_routes()` descriptor table, the `WebUiV2HttpError` redacted wire
   shape, SSE/WebSocket streaming, and the Vite SPA bundle.
2. **Gateway assembly + middleware** (`src/webui_serve.rs`, `src/webui_*.rs`,
   from `ironclaw_reborn_composition::webui`) — `webui_v2_app(bundle, config)`
   composes the full `Router` and layers the fixed middleware stack; owns the
   `WebuiAuthenticator` / `WebuiAuthentication` host-auth vocabulary and the
   feature-gated OpenAI-compat mounts.
3. **Serve loop + host authentication** (`src/lib.rs`, `src/auth/`,
   `src/session.rs`, `src/oidc.rs`) — `serve_webui_v2` binds the listener and
   runs `axum::serve`; the `Env`/`Session`/`Oidc` authenticators, the
   signed-token session store, and the `/auth/*` OAuth login surface that mints
   sessions.

Composition deliberately stops at the
`reborn_product_api_crates_do_not_bind_http_ingress` boundary — it returns a
fully composed `Router` but must never bind a socket. This crate is the
host-owned counterpart that binds the `TcpListener` and drives the serve loop.

Path A of `docs/reborn/how-to-port-channel-to-reborn.md` rules apply: host auth
stays host-owned in this crate, no `src/` (v1) imports, no v1 secrets / settings
/ DB, and no direct `ironclaw_product` edge (reach it through
composition's facade). Enforced by `ironclaw_architecture`
(`tests/reborn_dependency_boundaries.rs`).

## Surface

### Route surface + gateway assembly

| Symbol | Role |
|---|---|
| `webui_v2_router(state)` / `webui_v2_router_with_options(state, opts)` | Build the WebChat v2 `axum::Router` from a `WebUiV2State`. |
| `webui_v2_routes() -> Vec<IngressRouteDescriptor>` | The route descriptor table (id, method, pattern, auth, rate/body limit, streaming). Locked by `tests/webui_v2_descriptors_contract.rs`. |
| `WebUiV2State` | Handler state: the `ProductSurface` facade + `SseCapacity` + route options. |
| `WebUiV2HttpError` / `WebUiV2HttpErrorBody` | The only path handlers return HTTP errors through — keeps the redacted-error vocabulary intact. |
| `webui_v2_app(product_surface, config) -> WebuiV2App` | Compose a host-supplied `ProductSurface` + `WebuiServeConfig` into the full middleware-wrapped `Router` (also `webui_v2_app_with_lifecycle`). |
| `WebuiServeConfig` | Host-owned serve config (tenant, authenticator, default agent/project, public/protected mounts, Google OAuth). |
| `WebuiAuthenticator` trait / `WebuiAuthentication` | Host-auth vocabulary the bearer middleware resolves each token through. |

Middleware modules (`src/webui_*.rs`) layer in a fixed order —
**ws-origin → per-route body limit → bearer auth → rate limit → handler** —
turning the `webui_v2_routes()` descriptors into tower layers.

### Serve loop + host authentication

| Symbol | Role |
|---|---|
| `serve_webui_v2(opts)` | Bind a `TcpListener` + run `axum::serve` with graceful shutdown |
| `RebornWebuiServeOptions` | Owner-supplied input (addr, router, shutdown receiver) |
| `EnvBearerAuthenticator` | Single-token `WebuiAuthenticator` for the standalone CLI / local dev; accepted tokens map to operator WebUI capabilities |
| `SignedTokenSessionStore` | HMAC-signed bearer mint/lookup with a bounded process-local logout denylist |
| `SessionAuthenticator` | `WebuiAuthenticator` that resolves bearer tokens through `SignedTokenSessionStore` |
| `OidcAuthenticator` | OIDC bearer-token verifier (JWKS + standard claims); accepted tokens map to non-operator WebUI capabilities |
| `webui_v2_auth_router(config) -> PublicRouteMount` | OAuth login router + route descriptors. The descriptors travel with the router so composition can fold them into the descriptor-driven per-route rate-limit / body-limit middleware — same machinery the v2 facade and product-auth callback already use, no side door. |
| `PublicRouteMount` | `{ router, descriptors }` pair handed to `WebuiServeConfig::with_public_route_mount` |
| `OAuthProvider` trait (in `auth/provider.rs`) | Extension point for per-provider URL / code-exchange logic. Deliberately lives in its own module so each provider does not depend on the others. `GoogleProvider` and `GitHubProvider` ship today. |
| `GoogleProvider` (in `auth/google.rs`) | Google OIDC provider (scopes `openid email profile`, PKCE S256, optional `hd` hosted-domain restriction). Built from `GoogleOAuthConfig`. |
| `GitHubProvider` (in `auth/github.rs`) | GitHub OAuth App provider (scopes `read:user user:email`, no PKCE, verified-email preference). Built from `GitHubOAuthConfig`. |
| `OAuthRouterConfig` | Tenant + `SignedTokenSessionStore` + `UserDirectory` + provider list + base URL |
| `UserDirectory` trait | Host-supplied mapping from `(provider, OAuthUserProfile)` to `UserId` |
| `EmailUserDirectory` | Local-dev default impl (verified email → `UserId`); gated on `test-support` |

## WebChat v2 route surface (folded from `ironclaw_webui_v2`)

Handlers consume only `ironclaw_host_api::ProductSurface`. The bearer
middleware (in this crate's `webui_v2_app`) constructs the
`ProductSurfaceCaller`, carries the matched token's `WebUiV2Capabilities`,
and injects both as axum `Extension`s before the handler runs; handlers fail
closed (`500`) if that layer is missing (locked by
`missing_caller_extension_returns_500`).

| Route ID | Method | Pattern | Streaming | Effect path |
|---|---|---|---|---|
| `webui.v2.get_session` | GET | `/api/webchat/v2/session` | — | `ProjectionOnly` |
| `webui.v2.create_thread` | POST | `/api/webchat/v2/threads` | — | `ProductWorkflow` |
| `webui.v2.list_threads` | GET | `/api/webchat/v2/threads` (`?limit&cursor`) | — | `ProjectionOnly` |
| `webui.v2.delete_thread` | DELETE | `/api/webchat/v2/threads/{thread_id}` | — | `ProductWorkflow` |
| `webui.v2.send_message` | POST | `/api/webchat/v2/threads/{thread_id}/messages` | — | `TurnCoordinator` |
| `webui.v2.get_timeline` | GET | `/api/webchat/v2/threads/{thread_id}/timeline` (`?limit&cursor`) | — | `ProjectionOnly` |
| `webui.v2.get_run_artifact` | GET | `/api/webchat/v2/threads/{thread_id}/runs/{run_id}/artifact` | — | `ProjectionOnly` |
| `webui.v2.logs` | GET | `/api/webchat/v2/logs` | — | `ProjectionOnly` |
| `webui.v2.stream_events` | GET | `/api/webchat/v2/threads/{thread_id}/events` | **SSE** | `ProjectionOnly` |
| `webui.v2.stream_events_ws` | GET | `/api/webchat/v2/threads/{thread_id}/ws` | **WebSocket** | `ProjectionOnly` |
| `webui.v2.cancel_run` / `retry_run` / `resolve_gate` | POST | `…/runs/{run_id}/…` | — | `TurnCoordinator` |
| `webui.v2.list/pause/resume/rename/delete_automation` | GET/POST/DELETE | `/api/webchat/v2/automations…` | — | `ProductWorkflow` |
| `webui.v2.list/install/import/remove/get_setup/setup_extension` | GET/POST | `/api/webchat/v2/extensions…` | — | `ProjectionOnly` / `ProductWorkflow` |
| `webui.v2.*_llm_*` | GET/POST | `/api/webchat/v2/llm/…` | — | `ProjectionOnly` / `ProductWorkflow` |
| `webui.v2.settings.list_tools` / `set_tools_auto_approve` / `set_tool_permission` | GET/POST | `/api/webchat/v2/settings/tools…` | — | `ProjectionOnly` / `ProductWorkflow` |
| `webui.v2.operator.*` (setup, config, config/{key}, validate, diagnostics, status, logs, service) | GET/POST | `/api/webchat/v2/operator/…` | — | `ProjectionOnly` / `ProductWorkflow` |
| `webui.v2.admin.*` (users CRUD, status, role, secrets) | GET/POST/PATCH/PUT/DELETE | `/api/webchat/v2/admin/users…` | — | `ProductWorkflow` |
| `webui.v2.trace_*` (credit, account, account-login-link, holds/authorize) | GET/POST | `/api/webchat/v2/traces/…` | — | `ProductWorkflow` |

The exact per-route set (methods, query params, auth, rate/body limits) is the
descriptor table in `src/webui_v2/descriptors.rs`; the count/shape is locked by
`tests/webui_v2_descriptors_contract.rs`. Add a route → add a handler **and** a
`webui_v2_routes()` entry, or that test fails.

`webui.v2.get_run_artifact` exports one exact caller-owned run as the versioned
`ironclaw.run_artifact.v1` evidence schema. The facade authorizes the thread
from authenticated tenant/user scope before selecting records by `turn_run_id`,
reconstructs provider tool-call metadata through the model-context read path,
and applies deterministic trace redaction before serialization. Its logs are a
bounded process-local diagnostic sidecar: `logs.complete` is always false and
availability/truncation are explicit. Deployment-wide logs are not exposed
through this caller route.

**Operator-gating.** LLM config, operator setup/config/service-control, and
extension zip-import routes are operator-wide: `webui_v2_app` mounts them only
when the authenticator advertises an operator config surface, and each handler
still rejects with `403` when the injected `WebUiV2Capabilities` lacks
`operator_webui_config`. Multi-user session/OIDC authenticators return
non-operator capabilities. `webui.v2.admin.*` user management is
admin/operator-gated server-side in `ProductSurface` (`AdminUserService`,
last-admin protection); `create_user` returns the one-time API bearer exactly
once in `api_token`. `webui.v2.settings.tools` is a normal authenticated-caller
route (tenant/user-scoped tool-approval settings), not an operator route.

### Streaming model (SSE + WebSocket)

- `stream_events` (SSE) and `stream_events_ws` (WebSocket) render each
  `ProductOutboundEnvelope` into the redacted `WebChatV2EventFrame` schema
  (never raw adapter routing/delivery metadata) with the projection cursor as
  the SSE `id`; the browser resumes via `Last-Event-ID` (preferred over
  `?after_cursor=`).
- Both transports share **one** `SseCapacity` budget keyed by `(tenant, user)`
  (default 3 concurrent; override via `WebUiV2State::with_sse_concurrency_limit`)
  — a caller cannot bypass the cap by mixing SSE and WS. Exhaustion returns
  `429` with `retryable: true`.
- The SPA also sends a bounded, random `connection_id` that is stable for one
  loaded browser tab plus a monotonically increasing `connection_generation`
  for each EventSource it creates. A same-caller, same-id stream supersedes its
  prior generation without consuming another slot; a delayed older generation
  receives `204` and cannot cancel the current stream. This prevents
  proxy-reordered closes/opens during thread navigation from stranding the
  replacement stream behind the cap; distinct tabs still consume distinct
  slots.
- A successful facade subscription emits an application-level `keep_alive`
  frame immediately after admission. Browser connection state uses that frame
  as proof that the projection tail is ready instead of waiting for a model
  delta or the periodic transport keep-alive.
- `after_cursor` is retained only within one mounted Chat route (including
  native EventSource retries and visibility recovery). A route/thread remount
  starts at the projection origin so the server returns durable state plus the
  compacted current live state; it does not persist process-local live cursors
  across SPA navigation.
- Every stream is closed after a max lifetime (5 min) and every `socket.send` /
  drain await is `timeout`-bounded, so a back-pressuring client or a stalled
  facade cannot pin a slot past the budget. Slots are RAII (`SseSlot`), released
  on disconnect / expiry / error. Regressions locked by
  `stream_events_ws_shares_capacity_with_sse_streams` and
  `stream_events_releases_slot_when_facade_drain_stalls_past_max_lifetime`.
- `capability_activity` / `capability_display_preview` frames carry only
  bounded, secret-redacted input/output *summaries* (host paths rejected, URLs
  stripped, byte-bounded) — never raw args/results. Full output stays behind the
  scoped `result_ref` fetch path. See `.claude/rules/gateway-events.md`.

### SPA bundle

The Vite/TypeScript frontend under `frontend/` is compiled by `build.rs` into
Cargo's `OUT_DIR` and served from `src/webui_v2/static_assets/`.
`Dockerfile.reborn` installs `frontend/` deps before the `cargo build` so the
release image bundles compiled assets; `frontend/README.md` covers the JS
toolchain.

## Why the OAuth login router lives here

The crate already owns `WebuiAuthenticator` impls, `SignedTokenSessionStore`,
and the session lifecycle types. The OAuth callback's job is exactly that
— turn a provider profile into a signed session `create_session` call
— so the login mint path belongs in the same host-owned crate, not
behind the product/API seam in `ironclaw_reborn_composition`.

SSO sessions are user identity only. They must not inherit operator
WebUI configuration privileges from the deployment. When the CLI
composes SSO plus the env bearer token, the env token remains the
separate operator credential and session/OIDC bearers remain
non-operator.

Composition merges the `PublicRouteMount` supplied by
`webui_v2_auth_router` through
`WebuiServeConfig::with_public_route_mount`. The router merges
outside bearer auth (the user has no session yet); the
descriptors fold into the same per-route policy stack the rest of
the WebChat v2 surface already rides on. That keeps the
product/API boundary intact: composition never sees provider
secrets, never speaks to Google, never parses a signed session token.

## WebChat v2 OAuth login surface (#4116)

Routes mounted by `webui_v2_auth_router`:

- `GET  /auth/providers` — list configured provider names.
- `GET  /auth/login/{provider}` — redirect non-canonical hosts to
  the configured `base_url`, then mint a pending flow (CSRF state +
  PKCE verifier + sanitized `redirect_after`) and redirect the
  browser to the provider's authorization URL.
- `GET  /auth/callback/{provider}` — single-use state lookup,
  cross-provider replay guard, code exchange via the matching
  `OAuthProvider`, user resolution via `UserDirectory`, session
  mint via `SignedTokenSessionStore`, and redirect to
  `{redirect_after}?login_ticket=<ticket>` (default `/`). The
  ticket is short-lived and single-use; the SPA redeems it over
  same-origin JSON so the bearer never appears in a redirect
  `Location` header.
- `POST /auth/session/exchange` — consume the one-time login ticket
  and return `{ token }`.
- `POST /auth/logout` — bearer-protected; calls
  `SignedTokenSessionStore::revoke` and returns `204` with or without
  a bearer, so the SPA's local clear stays unconditional.

### Provider trait

`OAuthProvider` is the seam new providers plug into:

```rust
#[async_trait]
pub trait OAuthProvider: Send + Sync + 'static {
    fn name(&self) -> &OAuthProviderName;
    fn authorization_url(&self, callback_url: &str, state: &str, code_challenge: &str) -> String;
    async fn exchange_code(&self, code: &str, callback_url: &str, code_verifier: &str)
        -> Result<OAuthUserProfile, OAuthError>;
}
```

- `GoogleProvider` ships today (OIDC scopes `openid email profile`,
  PKCE S256, optional `hd=` Workspace hint + server-side `hd`
  claim check, audience+issuer validation; signature verification
  is disabled because the `id_token` arrived over TLS directly
  from Google).
- `GitHubProvider` ships today. It uses GitHub's
  OAuth App flow with scopes `read:user user:email`, ignores the
  PKCE challenge the router computes (GitHub does not support PKCE —
  CSRF is the `state` param only), and after the token exchange
  reads `/user` + `/user/emails`, preferring the primary verified
  email, then any verified email, then the unverified profile email
  flagged `email_verified = false` so the `UserDirectory` fails
  closed. Built from `GitHubOAuthConfig` (client id + secret); no
  hosted-domain analogue.
- NEAR wallet login does NOT fit OAuth code flow and will get its
  own pair of endpoints (`/auth/near/challenge` +
  `/auth/near/verify`) plus its own sub-module under `auth/near/`.
  The signed session store + `UserDirectory` + composition seam stay the
  same.

### Security invariants

- **Pending-flow store** is process-local, bounded (1024 entries +
  5-min TTL), and single-use on `take`. A replayed callback cannot
  re-use a state token; cross-provider replay (state minted for
  Google arriving on the GitHub callback) fails closed.
- **Canonical login host** is the configured `base_url`. Login
  requests received on any other `Host` redirect to that base URL
  before a pending-flow entry is created, so preview/custom domains
  cannot mint state that the registered provider callback host will
  never see.
- **Session exchange tickets** are process-local, bounded (1024
  entries + 60-sec TTL), and single-use on `take`. The OAuth
  callback puts only the ticket in the redirect `Location`; the SPA
  redeems it via `POST /auth/session/exchange` to receive the real
  bearer over a same-origin JSON response.
- **CSRF state** is 32 random bytes (hex). **PKCE verifier** is 32
  random bytes (base64url-no-pad → 43 chars). S256 challenge is
  `base64url_no_pad(sha256(verifier))`.
- **Redirect target** (`?redirect_after=`) is sanitized: must start
  with `/`, must not start with `//` or `/\`, must contain only
  RFC-3986 path chars; the percent-decoded form must also pass so
  smuggled sequences like `%2f%2f` (→ `//`) are rejected.
- **Hosted-domain restriction** is enforced server-side from the
  ID token's `hd` claim, not from the `hd=` URL hint.
- **Error mapping**: every failure path redirects to
  `/?login_error=<code>` where `<code>` is an opaque enum
  (`invalid_state`, `provider_mismatch`, `denied`,
  `unauthorized`, `exchange_failed`, `server_error`,
  `invalid_request`). Provider error bodies, JWT decode messages,
  and signed-session errors are logged via `tracing` and never
  echoed back to the client.
- **Session transport** is one-time login ticket in the callback
  redirect (`?login_ticket=<ticket>`) followed by same-origin
  exchange for the bearer — see
  `ironclaw_reborn_composition/CLAUDE.md` → "Session transport
  decision" for the rationale.

### What the SSO router deliberately does NOT do

- No cookie writes (the SPA stores the exchanged bearer in
  `sessionStorage`).
- No DB schema. `UserDirectory` is host-supplied; the crate ships
  only the local-dev `EmailUserDirectory`.
- No retry / refresh-token handling. The callback is one-shot:
  exchange code, mint session, done. Token refresh is the host's
  job if it wants it.
- No v1 `/auth/*` reuse. The crate has zero `src/`-tier dependency
  by contract; that constraint is what lets WebChat v2 declare a
  hard non-goal on v1 routes (issue #3886).

## Test layout

**Route surface + gateway assembly** (folded from `ironclaw_webui_v2` +
composition):

- `tests/webui_v2_descriptors_contract.rs` — locks the descriptor table
  (count / methods / patterns / auth / rate limits / SSE).
- `tests/webui_v2_handlers_contract.rs` — drives a real axum router from
  `webui_v2_router` against a stub `ProductSurface` (test-through-the-caller).
- `tests/webui_v2_schema_contract.rs`, `tests/webui_v2_operator_config_key_contract.rs`,
  `tests/webui_v2_operator_route_predicate_contract.rs` — wire schema + operator
  gating.
- `tests/headers_errors_contract.rs`, `tests/network_limits_contract.rs`,
  `tests/auth_route_contract.rs` — middleware stack (security headers, body/rate
  limits, bearer auth) over the composed `webui_v2_app`.
- `tests/serve_loop.rs` — listener bind + graceful shutdown.

**Host authentication:**

- `src/{auth, oidc, session}/tests` — unit tests per module
  (provider URL building, PKCE math, ID-token decode, pending
  store, redirect sanitization, session lookup).
- `tests/google_oauth_routes.rs` — caller-level tests on
  `webui_v2_auth_router` covering provider discovery, login
  redirect, callback success, state replay, open-redirect bypass,
  provider error, hd denial, ticket exchange, logout revocation.
- `tests/github_oauth_routes.rs` — caller-level tests driving the
  REAL `GitHubProvider` against a local mock GitHub token/user/emails
  server: discovery, login redirect (state + scope, no PKCE),
  callback success minting a session for the primary verified email,
  an all-unverified login minting a provider-sub (`github:<id>`)
  session rather than an email identity, ticket exchange + single-use
  replay, provider-error and exchange-failure redirects, and logout
  revocation.
- `tests/session_round_trip.rs` — end-to-end test composing
  `webui_v2_app` with `SessionAuthenticator` + the OAuth router;
  drives an OAuth callback, exchanges the resulting ticket, uses the bearer on
  `POST /api/webchat/v2/threads`, then revokes and verifies the
  bearer is rejected. This locks the contract called out in
  #4116's acceptance criteria ("session use on a protected
  WebChat v2 route").
- `tests/oidc_e2e.rs` — pre-existing JWKS-signed ID-token e2e
  for the OIDC authenticator path.
- `tests/serve_loop.rs` — listener bind + graceful shutdown.

## Validation

```bash
cargo test -p ironclaw_webui --all-features
cargo clippy -p ironclaw_webui --all-features --tests -- -D warnings
```
