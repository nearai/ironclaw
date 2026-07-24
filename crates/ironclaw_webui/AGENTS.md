# Agent Map — ironclaw_webui

The **WebUI host stack** for Reborn WebChat v2: route surface + SPA bundle +
gateway assembly/middleware + listener/serve loop + host authentication, all in
one `products`-layer crate above `ironclaw_reborn_composition`. Driven by the
`ironclaw` binary (`crates/ironclaw_reborn_cli`).

## Start here

- Read `CLAUDE.md` first — it is the module spec (full route table, streaming
  model, SSE caps, OAuth login security contract). Code follows the spec; the
  spec is the tiebreaker.
- Read `README.md` for the composed-crate map (what got folded in from where).
- Treat these as the source of truth before changing behavior:
  - `src/webui_v2/descriptors.rs` + `tests/webui_v2_descriptors_contract.rs` — the route contract.
  - `src/webui_v2/handlers.rs` + `tests/webui_v2_handlers_contract.rs` — handler dispatch.
  - `src/webui_serve.rs` — the `webui_v2_app` gateway assembly + middleware order.
  - `Cargo.toml` — feature gates (`openai-compat-beta`,
    `dev-in-memory-session`) and the (deliberately narrow) dependency shape.

## What this crate owns (composed subsystems)

1. **WebChat v2 route surface + SPA** (`src/webui_v2/`, folded from the former
   `ironclaw_webui_v2` crate): axum handlers dispatching to
   `ironclaw_host_api::ProductSurface`, the `webui_v2_router` builder,
   the `webui_v2_routes()` descriptor table, the `WebUiV2HttpError` redacted wire
   shape, SSE + WebSocket streaming with a shared `SseCapacity` budget, and the
   Vite SPA under `frontend/` (built by `build.rs`, served from
   `src/webui_v2/static_assets/`).
2. **Gateway assembly + middleware** (`src/webui_serve.rs`, `src/webui_*.rs`,
   folded from `ironclaw_reborn_composition::webui`): `webui_v2_app(bundle,
   config)` composes the full `axum::Router` and layers the fixed middleware
   stack — ws-origin → body limit → bearer auth → rate limit → handler — plus the
   `WebuiAuthenticator` / `WebuiAuthentication` host-auth vocabulary and the
   feature-gated OpenAI-compat mounts.
3. **Serve loop + host authentication** (`src/lib.rs`, `src/auth/`,
   `src/session.rs`, `src/oidc.rs`, `src/signed_session_login.rs`):
   `serve_webui_v2` (listener bind + `axum::serve` + graceful shutdown), the
   `Env` / `Session` / `Oidc` authenticators, `SignedTokenSessionStore`, and
   the `/auth/*` OAuth login surface (Google/GitHub via the `OAuthProvider`
   trait).

## Do not move in here

- **Product/API business logic.** Handlers consume only `ProductSurface`;
  the facade, projections, and domain services stay behind that seam in
  `ironclaw_product` / `ironclaw_reborn_composition`.
- **Product service or domain dependencies.** `ironclaw_product` is allowed here
  only for wire DTOs and product command/view descriptors. Do not import product
  workflow services, facades, lower substrates, runtime, or DB crates; reach
  execution through `ProductSurface` supplied by host assembly. The architecture
  boundary test enforces this DTO/descriptor-only edge.
- **v1 anything** — no `src/` (monolith) import, no `ironclaw_engine`, no v1
  channel code, no v1 secrets / settings / DB. This is a Path A native host
  surface (`docs/reborn/how-to-port-channel-to-reborn.md`).
- **Business/durable state.** Everything the gateway needs flows through
  `ProductSurface`; this crate stores no threads, transcripts, or projections.

## Allowed dependencies

`ironclaw_product` (wire DTOs and product command/view descriptors),
`ironclaw_host_api` (`ProductSurface`, caller/error vocabulary, identity
newtypes, and ingress descriptors), `ironclaw_host_ingress` (Axum route-mount
carriers), and `ironclaw_reborn_openai_compat`. Plus infra crates: `axum`, `tokio`, `tower*`,
`tracing`, `thiserror`, `async-trait`, `secrecy`, `subtle`, `jsonwebtoken`, etc.

Any other workspace-crate edge requires an `ironclaw_architecture` boundary-test
update (`tests/reborn_dependency_boundaries.rs`) plus explicit PR rationale.

## Agent notes

- **Adding a route** requires a handler **and** a matching entry in
  `webui_v2_routes()` — the descriptor contract test fails otherwise. Handlers
  receive `ProductSurfaceCaller` via `axum::Extension`; a missing extension
  surfaces `500` (locked by a regression test — do not hand-roll a fallback).
- **All HTTP errors travel through `WebUiV2HttpError`**, never hand-built
  `StatusCode` returns — that keeps the redacted-error vocabulary intact.
- **Operator-gated routes** (LLM config, operator setup/config/service, extension
  import) are mounted only when the authenticator advertises an operator config
  surface, and each handler still re-checks `operator_webui_config` on the
  matched token's `WebUiV2Capabilities`, failing closed with `403`.
- **Adding an authenticator:** implement `WebuiAuthenticator`, use constant-time
  comparison (`subtle::ConstantTimeEq`) for secret material, return both the
  `UserId` and the token's request-scoped capabilities, unit-test accept/reject +
  the capability shape, then add a caller-level `tests/` test that drives
  `serve_webui_v2` / `webui_v2_app` over a real client.
- **Adding an OAuth provider:** implement `OAuthProvider` in its own
  `src/auth/<provider>.rs` (providers must not depend on each other) and add
  caller-level route tests mirroring `tests/google_oauth_routes.rs`.
- **Streaming / events:** never broadcast durable-looking state directly from a
  handler; project through `ProductSurface` into the redacted
  `WebChatV2EventFrame` first (see `.claude/rules/gateway-events.md`).

## Validation

```bash
cargo test  -p ironclaw_webui --all-features
cargo clippy -p ironclaw_webui --all-features --all-targets -- -D warnings
cargo test  -p ironclaw_architecture reborn_crate_dependency_boundaries_hold
```
