# Reborn WebUI Ingress Agent Contract

This crate owns the host-side serve lifecycle for the Reborn WebChat v2
HTTP gateway. It is deliberately small: the product/API boundary is
held in `ironclaw_reborn_composition` (route descriptors + the
`Router`), and this crate's only job is to bind a listener and drive
the axum serve loop with the `Router` it gets handed.

## Boundaries

- Bind `tokio::net::TcpListener` and call `axum::serve`. This crate is
  intentionally outside the `reborn_product_api_crates_do_not_bind_http_ingress`
  forbidden list — that rule exists to keep product/API library crates
  from owning server lifecycle, and this crate is host-owned ingress
  code, not product/API.
- Provide concrete `WebuiAuthenticator` implementations the standalone
  `ironclaw-reborn` binary can wire (env-bearer first; DB / OIDC are
  follow-ups). Token comparison must be constant-time (`subtle::ConstantTimeEq`).
- Do not touch `ProductAdapter`, `ExternalActorRef`, `ProtocolAuthEvidence`,
  or other external-protocol shims — WebUI is a Path A native host
  surface (see `docs/reborn/how-to-port-channel-to-reborn.md`).
- Do not depend on v1's `src/`, `ironclaw_engine`, channel code, or
  v1 DB infrastructure.
- Do not store transcripts, threads, or any business state. Everything
  the gateway needs flows through `RebornServicesApi` from the
  `Router` the composition crate hands us.

## Allowed dependencies

- `ironclaw_reborn_composition` (consumes the composed `Router` +
  `WebuiAuthenticator` trait + `WebuiServeConfig`)
- `ironclaw_host_api` (identity types: `TenantId`, `UserId`)
- `axum`, `tokio`, `tracing`, `thiserror`, `async-trait`, `secrecy`,
  `subtle`

Any other workspace crate dependency requires an architecture-test
update + explicit PR rationale.

## Adding a new authenticator

1. Add the impl module under `src/`.
2. Implement `WebuiAuthenticator` from `ironclaw_reborn_composition`.
3. Use constant-time comparison for any secret material.
4. Add a unit test that exercises `authenticate` against a known
   token + a wrong token.
5. Add a caller-level test in `tests/` that spins up `serve_webui_v2`
   with the new authenticator on a random port and verifies bearer
   accept / reject through a real `reqwest::Client`.
