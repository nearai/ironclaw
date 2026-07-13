# Reborn WebUI Ingress Agent Contract

This crate owns the host-side WebChat v2 HTTP gateway: the axum
`Router`, descriptor-driven middleware, auth contracts, route-mount
types, listener binding, and serve loop. Composition still builds the
runtime-backed `RebornServicesApi` and product-specific route mounts;
ingress owns how those are exposed over HTTP.

## Boundaries

- Bind `tokio::net::TcpListener` and call `axum::serve`. This crate is
  intentionally outside the `reborn_product_api_crates_do_not_bind_http_ingress`
  forbidden list — that rule exists to keep product/API library crates
  from owning server lifecycle, and this crate is host-owned ingress
  code, not product/API.
- Provide concrete `WebuiAuthenticator` implementations the standalone
  `ironclaw-reborn` binary can wire (env-bearer first; DB / OIDC are
  follow-ups). Each accepted token must return both the `UserId` and
  the request-scoped WebUI capabilities for that exact token. Token
  comparison must be constant-time (`subtle::ConstantTimeEq`).
- Do not touch `ProductAdapter`, `ExternalActorRef`, `ProtocolAuthEvidence`,
  or other external-protocol shims — WebUI is a Path A native host
  surface (see `docs/reborn/how-to-port-channel-to-reborn.md`).
- Do not depend on v1's `src/`, `ironclaw_engine`, channel code, or
  v1 DB infrastructure.
- Do not store transcripts, threads, or any business state. Everything
  the gateway needs flows through `RebornServicesApi` and typed route
  mounts supplied by composition/host code.

## Allowed dependencies

- `ironclaw_host_api` (identity types: `TenantId`, `UserId`)
- `ironclaw_product_adapters` (optional, `openai-compat-beta`
  authenticated-caller evidence only)
- `ironclaw_product_workflow` (`RebornServicesApi` and authenticated
  WebUI caller contract)
- `ironclaw_reborn_openai_compat` (optional, `openai-compat-beta`
  request-scope contract only)
- `ironclaw_webui_v2` (WebChat v2 route descriptors, static assets, and
  handler router)
- `async-trait`, `axum`, `base64`, `chrono`, `hex`, `hmac`,
  `jsonwebtoken`, `lru`, `parking_lot`, `rand`, `reqwest`, `secrecy`,
  `serde`, `serde_json`, `sha2`, `subtle`, `thiserror`, `tokio`,
  `tower`, `tower-http`, `tracing`, `url`, `urlencoding`, `uuid`

Any other workspace crate dependency requires an architecture-test
update + explicit PR rationale.

## Adding a new authenticator

1. Add the impl module under `src/`.
2. Implement `WebuiAuthenticator` from this crate.
3. Use constant-time comparison for any secret material.
4. Add a unit test that exercises `authenticate` against a known
   token + a wrong token, including the returned capability shape.
5. Add a caller-level test in `tests/` that spins up `serve_webui_v2`
   with the new authenticator on a random port and verifies bearer
   accept / reject through a real `reqwest::Client`.
