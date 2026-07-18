# ironclaw_webui

The **WebUI host stack** for IronClaw Reborn: the single crate that turns
`ironclaw_reborn_composition`'s product/API surface into a running WebChat v2
HTTP server a browser can talk to. It owns the route handlers, the single-page
app bundle, the gateway assembly + middleware, the listener/serve loop, and all
host-side authentication.

It sits in the `products` layer, one level **above** composition, and is driven
by the `ironclaw-reborn` binary (`crates/ironclaw_reborn_cli`). Composition
deliberately stops at the `reborn_product_api_crates_do_not_bind_http_ingress`
boundary — it returns a fully composed `axum::Router` but must never bind a
socket. This crate is the host-owned counterpart that does.

## What it composes

This crate was assembled by folding three previously-separate pieces into one,
so the whole WebUI host stack lives above composition instead of being smeared
across it:

| Composed piece | Was | Now lives in |
|---|---|---|
| **WebChat v2 route surface + SPA** | crate `ironclaw_webui_v2` | `src/webui_v2/` (public module) + `frontend/` |
| **Gateway assembly + middleware** | `ironclaw_reborn_composition::webui::webui_serve` + middleware | `src/webui_serve.rs` + `src/webui_*.rs` |
| **Serve loop + host auth** | crate `ironclaw_reborn_webui_ingress` (this crate's original scope) | `src/lib.rs`, `src/auth/`, `src/session.rs`, `src/oidc.rs`, `src/signed_session_login.rs` |

### 1. WebChat v2 route surface + SPA (`src/webui_v2/`)

The native WebChat v2 HTTP routes on top of
`ironclaw_product_workflow::RebornServicesApi`. Handlers are thin: they read the
`WebUiAuthenticatedCaller` + `WebUiV2Capabilities` injected as axum extensions,
dispatch to the facade, and render redacted responses through `WebUiV2HttpError`.

- `webui_v2_router(state)` / `webui_v2_router_with_options(state, opts)` — build
  the `axum::Router` from a `WebUiV2State`.
- `webui_v2_routes() -> Vec<IngressRouteDescriptor>` — the descriptor table
  (route id, method, pattern, auth, rate/body limits, streaming class). The
  descriptors are the contract host composition folds into the per-route policy
  stack; the table is locked by `tests/webui_v2_descriptors_contract.rs`.
- ~60 routes across sessions, threads/timeline, message send, SSE + WebSocket
  event streams, logs, automations, connectable channels, extensions
  (install/import/activate/setup lifecycle), LLM config, tool-approval settings,
  operator setup/config/diagnostics, admin user management, and trace credits.
  The full table lives in `CLAUDE.md`.
- **Streaming:** `stream_events` (SSE) and `stream_events_ws` (WebSocket) share
  one `SseCapacity` budget keyed by `(tenant, user)`; both render
  `ProductOutboundEnvelope`s into the redacted `WebChatV2EventFrame` schema and
  resume via `Last-Event-ID`. Slots are RAII and bounded by a max stream
  lifetime so a stuck client or facade cannot pin a slot.
- **SPA bundle:** the Vite/TypeScript frontend under `frontend/` is compiled by
  `build.rs` into Cargo's `OUT_DIR` and served from `src/webui_v2/static_assets/`.

### 2. Gateway assembly + middleware (`src/webui_serve.rs`, `src/webui_*.rs`)

`webui_v2_app(bundle, config)` takes composition's `RebornWebuiBundle` plus a
host-owned `WebuiServeConfig` and returns a `WebuiV2App` — the fully composed
`axum::Router` with the canonical middleware stack layered in a fixed order:

```
ws-origin  →  per-route body limit  →  bearer auth  →  rate limit  →  handler
```

Each middleware is its own module: `webui_ws_origin`, `webui_body_limit`,
`webui_operator_auth`, `webui_rate_limit`, `webui_route_match`. This is where
the descriptor-driven policy (from `webui_v2_routes()`) is turned into real
tower layers, where product-auth and OAuth `PublicRouteMount`s are merged
outside bearer auth, and where the Slack / OpenAI-compat host-beta route mounts
attach under their feature flags.

### 3. Serve loop + host authentication (`src/lib.rs`, `src/auth/`, `src/session.rs`, `src/oidc.rs`)

- `serve_webui_v2(RebornWebuiServeOptions)` — bind a `tokio::net::TcpListener`
  and run `axum::serve` with graceful shutdown.
- **Authenticators** (`WebuiAuthenticator` impls): `EnvBearerAuthenticator`
  (single operator token for the standalone binary / local dev),
  `SessionAuthenticator` (bearer → `SessionStore` lookup, non-operator),
  `OidcAuthenticator` (JWKS + standard-claim verifier, non-operator).
- **Sessions:** the `SessionStore` trait (durable impl is the host's;
  `InMemorySessionStore` behind `dev-in-memory-session` for dev/tests) plus the
  signed-token login surface (`build_signed_session_login`).
- **OAuth login surface:** `webui_v2_auth_router` mounts `/auth/*` and mints
  sessions from Google / GitHub logins. Providers plug in through the
  `OAuthProvider` trait. Full security model (PKCE, CSRF state, canonical host,
  one-time login tickets, redirect sanitization) is documented in `CLAUDE.md`.

## Layering & boundaries

- Reaches the rest of Reborn **only** through composition's facade
  (`RebornWebuiBundle`, product-auth mount builders, the
  `PublicRouteMount`/`ProtectedRouteMount` vocabulary) and
  `ironclaw_product_workflow::RebornServicesApi`.
- **No** direct dependency on `ironclaw_product_adapters` or any lower substrate
  crate; **no** v1 `src/` import; **no** v1 secrets / settings / DB. Host auth
  stays host-owned here (Path A of `docs/reborn/how-to-port-channel-to-reborn.md`).
- These edges are enforced by `crates/ironclaw_architecture` — see
  `tests/reborn_dependency_boundaries.rs`.

## Feature flags

| Feature | Effect |
|---|---|
| `default` | Route surface + SPA + serve loop + auth. |
| `dev-in-memory-session` | Compile in `InMemorySessionStore` + `EmailUserDirectory` for local dev / tests. |
| `slack-v2-host-beta` | Mount the Slack personal-OAuth setup + channel-route admin surface in `webui_v2_app` (forwards to composition). |
| `openai-compat-beta` | Stamp an `OpenAiCompatActorScope` onto verified callers for protected OpenAI-compatible mounts (forwards to composition + `ironclaw_reborn_openai_compat`). |

## Build & test

```bash
# Route surface + serve + auth (default features)
cargo test  -p ironclaw_webui
cargo clippy -p ironclaw_webui --all-targets -- -D warnings

# Everything, incl. Slack / OpenAI-compat host-beta and the relocated
# slack_host_beta_webui_v2 integration test
cargo test  -p ironclaw_webui --all-features
cargo clippy -p ironclaw_webui --all-features --all-targets -- -D warnings

# Dependency boundaries (this crate's allowed edges are asserted here)
cargo test  -p ironclaw_architecture reborn_crate_dependency_boundaries_hold
```

The SPA frontend builds through `build.rs` (Vite). `frontend/README.md` covers
the JS/TS toolchain; `Dockerfile.reborn` installs `frontend/` deps before the
`cargo build` so the release image bundles compiled assets.

## Where to read next

- **`CLAUDE.md`** — the authoritative module spec: full route table, streaming
  model, SSE caps, and the complete OAuth login security contract.
- **`AGENTS.md`** — the agent map: what this crate owns, what must not move in,
  and how to add a route / authenticator / OAuth provider.
